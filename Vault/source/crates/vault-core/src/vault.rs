//! An open database plus the credentials needed to save it again. Saving is defensive: the new
//! file is checked before it replaces the old one, the exact previous version is first kept in
//! the backup store (see [`crate::store`]), and a file changed on disk since it was opened (e.g.
//! by a OneDrive sync) is not silently overwritten.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use keepass::db::DatabaseOpenError;
use keepass::error::DatabaseKeyError;
use keepass::{Database, DatabaseKey};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::backups::{self, BackupConfig, Kind};
use crate::envelope::{self, KeyRing};
use crate::fsops;
use crate::generation::{self, CredentialRecord, RecordAtOpen, SealedRecord};
use crate::header::{self, HeaderError, Kdf, KdfBudget};
use crate::limits::{DATABASE_MAX, SIDECAR_MAX, read_capped};
use crate::manage::{self, BackupsTask, MigrationReport, OldFile};
use crate::rekey::{self, RekeyPlan, Staged};
use crate::store::{self, Request, Store, StoreError, Written};

#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("wrong master password or key, or the file is damaged")]
    Open(#[source] keepass::db::DatabaseOpenError),
    /// The header asks for more key derivation than the budget allows; nothing was derived.
    #[error(
        "the database asks for unusually heavy key derivation ({}); it was not opened",
        .0.details()
    )]
    KdfOverBudget(Kdf),
    #[error("{0}")]
    Header(#[from] HeaderError),
    #[error("could not write the database: {0}")]
    Save(#[from] keepass::db::DatabaseSaveError),
    #[error("the saved file failed verification ({0}); the original was left untouched")]
    Verify(String),
    #[error(
        "the database file changed on disk since it was opened (another device or app saved it)"
    )]
    ChangedOnDisk,
    /// "Save anyway" found a file the session's credentials don't open: another device changed
    /// them, and overwriting would put the old ones back.
    #[error("Lock and reopen; saving would put back credentials changed on another device")]
    CredentialsChangedElsewhere,
    /// "Save anyway" found a file on disk that isn't a KeePass database this app can read (a
    /// damaged or cut-off file, a header it doesn't know, key derivation over the budget).
    /// [`Vault::save_replacing_unreadable`] replaces it once the user confirms.
    #[error("the file on disk can't be read as a KeePass database ({0})")]
    UnreadableOnDisk(Box<VaultError>),
    /// No copy of the version being replaced could be kept; nothing was replaced.
    #[error(
        "no backup of the previous version could be written ({0}); the database was not changed"
    )]
    Backup(#[from] StoreError),
    #[error("this is a backup opened read-only; changes to it are not saved")]
    ReadOnly,
    #[error("This backup needs older credentials; restoring those comes in a later version.")]
    NeedsOlderCredentials,
    #[error("save your changes first")]
    Unsaved,
    #[error("a password or key is required")]
    EmptyKey,
}

impl VaultError {
    /// The credentials don't open the file (as opposed to a damaged or unsupported file).
    pub fn is_wrong_key(&self) -> bool {
        matches!(self, VaultError::Open(e) if wrong_key(e))
    }
}

/// What a key change will do to the backups, for the confirmation shown before it starts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyChangePreview {
    /// Backups in reachable folders, all to be re-encrypted to the new key.
    pub backups: usize,
    /// Copies of the database beside it (`<db>.bak` and the like), which move into the backup
    /// folder.
    pub beside: Vec<PathBuf>,
    /// Known backup roots that don't answer: their backups keep the old credentials.
    pub unreachable: Vec<PathBuf>,
}

/// How a committed key change went.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyChangeReport {
    /// Backups now on the new credentials.
    pub rekeyed: usize,
    /// Backups that kept their old credentials (marked in the index).
    pub kept_old: usize,
    /// Old-key originals of re-encrypted backups that another program held open: still there,
    /// deleted at a later unlock or clean-up.
    pub held: usize,
    /// Copies of the database beside it, moved into the store but still there (held open) on
    /// the old credentials; the next unlock deletes them.
    pub left_beside: Vec<PathBuf>,
    /// Roots that didn't answer while staging.
    pub unreachable: Vec<PathBuf>,
    /// The database is on the new key but the passkey file couldn't be replaced: the new ring
    /// stays as `<db>.axvault.~new`, which the next passkey unlock puts in place.
    pub ring_error: Option<String>,
}

/// What unlock-time repairs of the store did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecoveryReport {
    pub rekey: crate::rekey::Recovered,
    /// Old-key originals of re-encrypted backups, left by a key change that couldn't delete
    /// them then, deleted now.
    pub superseded: Vec<String>,
    /// Leftover save temps copied into the store (`imported`) and then deleted.
    pub imported: Vec<PathBuf>,
    /// Leftover save temps too small to be a database, deleted.
    pub removed: Vec<PathBuf>,
}

/// The parts of a composite key, kept so the database can be saved under the same key.
#[derive(Default, Clone, PartialEq, Eq)]
pub struct Credentials {
    pub password: Option<Zeroizing<String>>,
    /// Raw contents of a key file, or the 32 bytes unwrapped from a passkey (a 32-byte key file
    /// is used directly, exactly as KeePass does).
    pub key_file: Option<Zeroizing<Vec<u8>>>,
}

impl Credentials {
    pub fn to_key(&self) -> Result<DatabaseKey, VaultError> {
        if self.password.is_none() && self.key_file.is_none() {
            return Err(VaultError::EmptyKey);
        }
        let mut key = DatabaseKey::new();
        if let Some(p) = &self.password {
            key = key.with_password(p);
        }
        if let Some(k) = &self.key_file {
            key = key.with_keyfile(&mut &k[..])?;
        }
        Ok(key)
    }
}

/// Which save: see [`Vault::save`], [`Vault::save_overwriting`] and
/// [`Vault::save_replacing_unreadable`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SaveMode {
    Normal,
    Overwrite,
    ReplaceUnreadable,
}

/// A version of the database file: its modification time, length and content hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    modified: Option<SystemTime>,
    len: u64,
    sha256: [u8; 32],
}

/// Modification time and length, from metadata alone.
fn metadata_stamp(path: &Path) -> io::Result<(Option<SystemTime>, u64)> {
    let m = fs::metadata(path)?;
    Ok((m.modified().ok(), m.len()))
}

/// Reads the whole file (capped) and stamps it. The modification time and length from before
/// and after the read must agree with each other and with what was read; a file caught being
/// written is read once more before giving up.
fn read_stamped(path: &Path) -> io::Result<(Zeroizing<Vec<u8>>, FileStamp)> {
    for _ in 0..2 {
        let before = metadata_stamp(path)?;
        let data = read_capped(path, DATABASE_MAX)?;
        let after = metadata_stamp(path)?;
        if before == after && after.1 == data.len() as u64 {
            let stamp = FileStamp {
                modified: after.0,
                len: after.1,
                sha256: Sha256::digest(&data[..]).into(),
            };
            return Ok((data, stamp));
        }
    }
    Err(io::Error::other(
        "the database file kept changing while it was being read; try again",
    ))
}

/// A wrong key, as opposed to a damaged or unsupported file.
fn wrong_key(e: &DatabaseOpenError) -> bool {
    matches!(e, DatabaseOpenError::Key(DatabaseKeyError::IncorrectKey))
}

/// The exact bytes a vault was opened from, for its `opened` snapshot (see
/// [`Vault::snapshot_opened`]).
pub struct Opened {
    data: Zeroizing<Vec<u8>>,
    modified: Option<SystemTime>,
}

/// A locked recovery record. Only standard encrypted KDBX bytes and file metadata survive;
/// reopening requires the complete original credentials. It is intentionally memory-only.
#[derive(Clone)]
pub struct SealedVault {
    encrypted: Vec<u8>,
    path: PathBuf,
    on_disk: Option<FileStamp>,
    dirty: bool,
    kdf_budget: KdfBudget,
    /// The session's generation without its check (that would test passwords without the KDF).
    generation: SealedRecord,
    generation_committed: bool,
    outside_change: bool,
    read_only: bool,
    /// Content hashes and generations (see `Vault::known`): file hashes, no key material.
    known: HashMap<[u8; 32], String>,
    restore_pending: bool,
}

pub struct Vault {
    pub db: Database,
    path: PathBuf,
    credentials: Credentials,
    // The on-disk file still uses these until a credential-change save commits.
    previous_credentials: Option<Credentials>,
    on_disk: Option<FileStamp>,
    dirty: bool,
    /// What re-reading this file may cost: the default, or raised to what the user approved.
    kdf_budget: KdfBudget,
    /// The credential generation of `credentials`, written into the database at every save.
    generation: CredentialRecord,
    /// The database file carries `generation`: it did at open, or a save under it committed.
    generation_committed: bool,
    /// Restored along with `previous_credentials` when a credential change is rolled back.
    previous_generation: Option<(CredentialRecord, bool)>,
    /// At open, the database's record named other credentials: its key was changed outside
    /// Axentra.
    outside_change: bool,
    /// The generation that record named, which that change replaced (marked so in the store at
    /// unlock, see `recover_backups`).
    outside_generation: Option<String>,
    /// Where copies of replaced versions go. Without a store, an ordinary save keeps the
    /// previous version as `<db>.bak` and a credential change is refused.
    store: Option<Store>,
    /// A backup opened as a vault: it is never saved and has no store.
    read_only: bool,
    /// Content hashes of the versions this session opened or saved, or parsed with its own
    /// credentials before overwriting, each with the generation that opens it. A backup of
    /// exactly those bytes is labelled with it; anything else is unknown.
    known: HashMap<[u8; 32], String>,
    /// The database was replaced by a restored backup; the next save keeps the version it
    /// replaces as `before-restore`.
    restore_pending: bool,
    /// Runs just before the check that precedes the rename (step 6 of a save).
    #[cfg(test)]
    before_commit: Option<BeforeCommit>,
}

#[cfg(test)]
type BeforeCommit = Box<dyn FnMut(&Path) + Send>;

impl Drop for Vault {
    fn drop(&mut self) {
        generation::wipe_record(&mut self.db);
    }
}

/// The KDF budget check. It runs on the exact bytes about to be parsed, so the settings checked
/// are the settings used, and before any key derivation.
fn check_kdf(data: &[u8], budget: &KdfBudget) -> Result<(), VaultError> {
    let kdf = header::parse_kdf(data)?;
    if budget.allows(&kdf) {
        Ok(())
    } else {
        Err(VaultError::KdfOverBudget(kdf))
    }
}

impl Vault {
    /// Opens with the default KDF budget.
    pub fn open(path: &Path, credentials: Credentials) -> Result<Self, VaultError> {
        Self::open_with_budget(path, credentials, KdfBudget::DEFAULT)
    }

    /// Opens if the header's key derivation fits `budget` (refused with `KdfOverBudget`, before
    /// deriving anything, otherwise). Later re-reads of this file, such as a re-key, use the
    /// same budget.
    pub fn open_with_budget(
        path: &Path,
        credentials: Credentials,
        kdf_budget: KdfBudget,
    ) -> Result<Self, VaultError> {
        Self::open_read(path, credentials, kdf_budget).map(|(vault, _)| vault)
    }

    /// [`Vault::open_with_budget`], also returning the bytes it parsed for
    /// [`Vault::snapshot_opened`]. The file is read once, with its modification time and length
    /// taken before and after (a file caught mid-write is read again).
    pub fn open_read(
        path: &Path,
        credentials: Credentials,
        kdf_budget: KdfBudget,
    ) -> Result<(Self, Opened), VaultError> {
        let (data, on_disk) = read_stamped(path)?;
        check_kdf(&data, &kdf_budget)?;
        let db = Database::parse(&data, credentials.to_key()?).map_err(VaultError::Open)?;
        let (generation, found) = generation::at_open(&db, &credentials);
        let outside_generation = (found == RecordAtOpen::Mismatch)
            .then(|| CredentialRecord::read(&db).map(|r| r.generation().to_owned()))
            .flatten();
        let known = HashMap::from([(on_disk.sha256, generation.generation().to_owned())]);
        let vault = Vault {
            db,
            path: path.to_owned(),
            credentials,
            previous_credentials: None,
            on_disk: Some(on_disk),
            dirty: false,
            kdf_budget,
            generation,
            generation_committed: found == RecordAtOpen::Matches,
            previous_generation: None,
            outside_change: found == RecordAtOpen::Mismatch,
            outside_generation,
            store: None,
            read_only: false,
            known,
            restore_pending: false,
            #[cfg(test)]
            before_commit: None,
        };
        let opened = Opened {
            data,
            modified: on_disk.modified,
        };
        Ok((vault, opened))
    }

    /// Checks that `credentials` open the database at `path`, keeping nothing (one KDF run,
    /// within `kdf_budget`).
    pub fn verify_credentials(
        path: &Path,
        credentials: &Credentials,
        kdf_budget: &KdfBudget,
    ) -> Result<(), VaultError> {
        let data = read_capped(path, DATABASE_MAX)?;
        check_kdf(&data, kdf_budget)?;
        Database::parse(&data, credentials.to_key()?)
            .map(|mut db| generation::wipe_record(&mut db))
            .map_err(VaultError::Open)
    }

    pub fn kdf_budget(&self) -> KdfBudget {
        self.kdf_budget
    }

    /// Preserve unsaved changes without disk access or retaining the composite key.
    pub fn seal(&self) -> Result<SealedVault, VaultError> {
        // Recovery must not forget how to rekey an old on-disk backup. The app commits
        // or rolls back credential changes before returning to normal session handling.
        if self.previous_credentials.is_some() {
            return Err(VaultError::Verify(
                "commit or roll back the credential change before locking".into(),
            ));
        }
        let key = self.credentials.to_key()?;
        let mut encrypted = Vec::new();
        self.db.save(&mut encrypted, key.clone())?;
        Self::verify_db(&self.db, &encrypted, key)?;
        Ok(SealedVault {
            encrypted,
            path: self.path.clone(),
            on_disk: self.on_disk,
            dirty: self.dirty,
            kdf_budget: self.kdf_budget,
            generation: self.generation.seal(),
            generation_committed: self.generation_committed,
            outside_change: self.outside_change,
            read_only: self.read_only,
            known: self.known.clone(),
            restore_pending: self.restore_pending,
        })
    }

    /// The sealed bytes were written by this app with the session's own KDF settings, so they
    /// need no budget check; the budget is kept for later reads of the file on disk. The session
    /// keeps its generation: the credentials that open the sealed bytes are the ones it was made
    /// for, so its check is recomputed from them. The store is attached again by the caller.
    pub fn open_sealed(sealed: &SealedVault, credentials: Credentials) -> Result<Self, VaultError> {
        let db =
            Database::parse(&sealed.encrypted, credentials.to_key()?).map_err(VaultError::Open)?;
        Ok(Self {
            db,
            path: sealed.path.clone(),
            generation: sealed.generation.unseal(&credentials),
            credentials,
            previous_credentials: None,
            on_disk: sealed.on_disk,
            dirty: sealed.dirty,
            kdf_budget: sealed.kdf_budget,
            generation_committed: sealed.generation_committed,
            previous_generation: None,
            outside_change: sealed.outside_change,
            // Marked in the store at the unlock that found it.
            outside_generation: None,
            store: None,
            read_only: sealed.read_only,
            known: sealed.known.clone(),
            restore_pending: sealed.restore_pending,
            #[cfg(test)]
            before_commit: None,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub fn credentials(&self) -> &Credentials {
        &self.credentials
    }

    /// The credential generation of the session's credentials: 32 hex digits, random, recorded
    /// in the database at every save (see [`crate::generation`]).
    pub fn generation(&self) -> &str {
        self.generation.generation()
    }

    /// Whether the database file carries the session's generation: its record matched at open,
    /// or a save under it has replaced the file. Otherwise the generation is provisional.
    pub fn generation_committed(&self) -> bool {
        self.generation_committed
    }

    /// Whether the database's record at open named other credentials than the ones that opened
    /// it: its key was changed outside Axentra (KeePass, KeePassXC, the plugin).
    pub fn outside_change(&self) -> bool {
        self.outside_change
    }

    /// Keeps copies of replaced versions in the store `config` describes from now on. A
    /// read-only vault gets none.
    pub fn attach_store(&mut self, config: BackupConfig) {
        if !self.read_only {
            self.store = Some(Store::new(config));
        }
    }

    /// The store's config, if one is attached.
    pub fn backup_config(&self) -> Option<&BackupConfig> {
        self.store.as_ref().map(Store::config)
    }

    /// Makes this a backup opened as a vault: saving is refused and no store is kept.
    pub fn set_read_only(&mut self) {
        self.read_only = true;
        self.store = None;
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// The notice for the first backup this session that had to fall back (another root,
    /// `<db>.bak`, or nothing); given once.
    pub fn take_backup_notice(&mut self) -> Option<String> {
        self.store.as_mut().and_then(Store::take_notice)
    }

    /// While backups fall back: what happened last. Cleared once a backup reaches the
    /// first-choice root again.
    pub fn backup_status(&self) -> Option<&str> {
        self.store.as_ref().and_then(Store::status)
    }

    /// Why retention last deleted nothing in a backup folder, if it didn't.
    pub fn backup_warning(&self) -> Option<&str> {
        self.store.as_ref().and_then(Store::warning)
    }

    /// Re-keys the database. Takes effect on the next save. New credentials start a new,
    /// provisional generation (the same key in another key-file form keeps it); going back to
    /// the previous credentials before the save restores theirs.
    pub fn set_credentials(&mut self, credentials: Credentials) {
        if self.previous_credentials.as_ref() == Some(&credentials) {
            self.previous_credentials = None;
            if let Some((record, committed)) = self.previous_generation.take() {
                self.generation = record;
                self.generation_committed = committed;
            }
        } else if self.previous_credentials.is_none() && credentials != self.credentials {
            self.previous_credentials = Some(self.credentials.clone());
            self.previous_generation = Some((self.generation.clone(), self.generation_committed));
        }
        if !self.generation.matches(&credentials) {
            self.generation = CredentialRecord::new(&credentials);
            self.generation_committed = false;
        }
        self.credentials = credentials;
        self.dirty = true;
    }

    /// A quick check by modification time and size. Saving compares the contents.
    pub fn changed_on_disk(&self) -> bool {
        match (self.on_disk, metadata_stamp(&self.path)) {
            (Some(then), Ok(now)) => (then.modified, then.len) != now,
            _ => true,
        }
    }

    /// Writes the bytes the vault was opened from to the store as an `opened` backup, labelled
    /// with the session's generation, unless the newest backup already holds them. It never
    /// falls back to `<db>.bak`. Ok(None) without a store.
    pub fn snapshot_opened(&mut self, opened: &Opened) -> Result<Option<Written>, StoreError> {
        let vault_id = backups::vault_id_of(&self.db);
        let Some(store) = self.store.as_mut() else {
            return Ok(None);
        };
        store
            .write(&Request {
                bytes: &opened.data,
                kind: Kind::Opened,
                source_mtime: opened.modified,
                generation: Some(self.generation.generation()),
                committed: self.generation_committed,
                db_path: &self.path,
                db_len: opened.data.len() as u64,
                vault_id: &vault_id,
                allow_bak: false,
            })
            .map(Some)
    }

    /// Saves unless the file changed on disk since it was opened or last saved.
    pub fn save(&mut self) -> Result<(), VaultError> {
        self.save_inner(SaveMode::Normal)
    }

    /// Saves even if another app changed the file meanwhile, provided the session's credentials
    /// (the old ones while a change is pending) still open it; the replaced version is kept as
    /// `replaced-external`. A file that isn't a KeePass database this app can read is refused
    /// with `UnreadableOnDisk`, for [`Vault::save_replacing_unreadable`] once confirmed.
    pub fn save_overwriting(&mut self) -> Result<(), VaultError> {
        self.save_inner(SaveMode::Overwrite)
    }

    /// "Save anyway" over a file on disk that isn't a KeePass database this app can read (see
    /// `UnreadableOnDisk`): damaged, cut off by a failed sync, or asking for key derivation
    /// over the budget. Its bytes are kept as `replaced-external`, as they are. Otherwise only
    /// the session's copy of the vault would be left. Still refused for a database the
    /// session's credentials don't open (`CredentialsChangedElsewhere`) and while a credential
    /// change is pending (its previous version must be re-encrypted, so it must be readable).
    pub fn save_replacing_unreadable(&mut self) -> Result<(), VaultError> {
        self.save_inner(SaveMode::ReplaceUnreadable)
    }

    /// The save, in this order:
    /// 1. the new file is serialized and verified;
    /// 2. the file on disk is read once and stamped (a normal save requires it unchanged since
    ///    open or the last save; overwriting requires that its own credentials open it, or,
    ///    once confirmed, that it isn't a KeePass database this app can read at all);
    /// 3. its backup is chosen: those exact bytes, or during a credential change their contents
    ///    re-encrypted to the new credentials and KDF;
    /// 4. the new file goes to a unique temp beside the database;
    /// 5. the backup goes through the fallback chain, and without one nothing is replaced;
    /// 6. the file must still hold the bytes read in step 2;
    /// 7. the temp replaces it (`MoveFileExW` with write-through);
    /// 8. the session records the commit;
    /// 9. retention runs, which never fails the save.
    ///
    /// Something can still write the file between 6 and 7; the backup from 5 holds what was
    /// there before.
    fn save_inner(&mut self, mode: SaveMode) -> Result<(), VaultError> {
        if self.read_only {
            return Err(VaultError::ReadOnly);
        }
        let overwrite = mode != SaveMode::Normal;
        let key = self.credentials.to_key()?;
        // 1. The record names the credentials the file is about to be saved under.
        self.generation.write_to(&mut self.db);
        let mut buf = Zeroizing::new(Vec::new());
        self.db.save(&mut *buf, key.clone())?;
        Self::verify_db(&self.db, &buf, key.clone())?;
        let generation = self.generation.generation().to_owned();

        // 2.
        let previous = match read_stamped(&self.path) {
            Ok(found) => Some(found),
            // "Save anyway" may recreate a file that went away; a normal save may not.
            Err(e) if e.kind() == io::ErrorKind::NotFound && overwrite => None,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(VaultError::ChangedOnDisk),
            Err(e) => return Err(e.into()),
        };
        let mut parsed = None;
        if let Some((prev, stamp)) = &previous {
            if !overwrite {
                if self.on_disk.map(|s| s.sha256) != Some(stamp.sha256) {
                    return Err(VaultError::ChangedOnDisk);
                }
            } else {
                // The file may have been replaced since it was opened, possibly re-keyed by
                // another device: only a version its credentials open is replaced, or, once
                // confirmed, one that isn't a database this app can read at all.
                let on_disk = self
                    .previous_credentials
                    .as_ref()
                    .unwrap_or(&self.credentials);
                let key = on_disk.to_key()?;
                let read = check_kdf(prev, &self.kdf_budget)
                    .and_then(|()| Database::parse(prev, key).map_err(VaultError::Open));
                match read {
                    Ok(db) => {
                        parsed = Some(db);
                        if self.previous_credentials.is_none() {
                            self.known.insert(stamp.sha256, generation.clone());
                        }
                    }
                    Err(e) if e.is_wrong_key() => {
                        return Err(VaultError::CredentialsChangedElsewhere);
                    }
                    Err(_)
                        if mode == SaveMode::ReplaceUnreadable
                            && self.previous_credentials.is_none() => {}
                    Err(e) => return Err(VaultError::UnreadableOnDisk(Box::new(e))),
                }
            }
        }

        // 3. Never publish a new, weaker backup when adding or changing a factor.
        let mut rekeyed = None;
        let backup = match &previous {
            None => None,
            Some((prev, stamp)) => Some(if let Some(old) = &self.previous_credentials {
                let mut db = match parsed.take() {
                    Some(db) => db,
                    None => {
                        check_kdf(prev, &self.kdf_budget)?;
                        Database::parse(prev, old.to_key()?).map_err(VaultError::Open)?
                    }
                };
                // Re-encrypted to the new credentials and KDF, so it carries their generation.
                self.generation.write_to(&mut db);
                db.config.kdf_config = self.db.config.kdf_config.clone();
                let mut bytes = Zeroizing::new(Vec::new());
                let saved = db
                    .save(&mut *bytes, key.clone())
                    .map_err(VaultError::from)
                    .and_then(|()| Self::verify_db(&db, &bytes, key.clone()));
                generation::wipe_record(&mut db);
                saved?;
                rekeyed = Some(bytes);
                (
                    Kind::BeforeKeyChange,
                    Some(generation.clone()),
                    stamp.modified,
                )
            } else {
                let kind = if self.restore_pending {
                    Kind::BeforeRestore
                } else if overwrite {
                    Kind::ReplacedExternal
                } else {
                    Kind::Auto
                };
                (kind, self.known.get(&stamp.sha256).cloned(), stamp.modified)
            }),
        };
        if let Some(mut db) = parsed.take() {
            generation::wipe_record(&mut db);
        }
        let vault_id = backups::vault_id_of(&self.db);

        // 4.
        let tmp = fsops::write_temp_beside(&self.path, &buf)?;
        let discard = |e: VaultError| {
            let _ = fs::remove_file(&tmp);
            e
        };

        // 5.
        let written = match (&backup, &previous) {
            (Some((kind, label, mtime)), Some((prev, _))) => {
                let bytes: &[u8] = rekeyed.as_deref().map_or(&prev[..], |b| &b[..]);
                let db_len = buf.len().max(prev.len()) as u64;
                let kept =
                    self.keep_previous(bytes, *kind, *mtime, label.as_deref(), &vault_id, db_len);
                Some(kept.map_err(discard)?)
            }
            _ => None,
        };

        // 6.
        self.run_before_commit();
        let now = match read_stamped(&self.path) {
            Ok((_, stamp)) => Some(stamp.sha256),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(discard(e.into())),
        };
        if now != previous.as_ref().map(|(_, stamp)| stamp.sha256) {
            return Err(discard(VaultError::ChangedOnDisk));
        }

        // 7.
        fsops::move_replace(&tmp, &self.path).map_err(|e| discard(e.into()))?;

        // 8. Replacement has committed. A later metadata error must not tell callers to
        // restore old credentials; it requires a reopen before any subsequent save.
        let sha256: [u8; 32] = Sha256::digest(&buf[..]).into();
        self.on_disk = fs::metadata(&self.path).ok().map(|m| FileStamp {
            modified: m.modified().ok(),
            len: m.len(),
            sha256,
        });
        self.known.insert(sha256, generation.clone());
        self.previous_credentials = None;
        // A credential change committed: the generation it replaced is retired.
        let replaced = self
            .previous_generation
            .take()
            .map(|(record, _)| record.generation().to_owned())
            .filter(|g| *g != generation);
        self.generation_committed = true;
        self.dirty = false;
        self.restore_pending = false;

        // 9.
        if let Some(Written::Store { folder, .. }) = &written {
            let _ = folder.with_index(|index| index.note_generation(&generation, true));
        }
        if let Some(store) = self.store.as_mut() {
            if let Some(replaced) = &replaced {
                store.retire_generation(&self.path, &vault_id, replaced);
            }
            store.prune(&self.path, &vault_id, Some(&generation));
        }
        Ok(())
    }

    /// Step 5 of a save: the previous version through the store's fallback chain. Without a
    /// store an ordinary save keeps it as `<db>.bak`, and a credential change is refused.
    fn keep_previous(
        &mut self,
        bytes: &[u8],
        kind: Kind,
        source_mtime: Option<SystemTime>,
        label: Option<&str>,
        vault_id: &str,
        db_len: u64,
    ) -> Result<Written, VaultError> {
        let allow_bak = self.previous_credentials.is_none();
        let committed = label == Some(self.generation.generation()) && self.generation_committed;
        let Some(store) = self.store.as_mut() else {
            if !allow_bak {
                return Err(StoreError::NoStore.into());
            }
            let bak = with_suffix(&self.path, ".bak");
            store::write_bak(&bak, bytes, source_mtime)?;
            return Ok(Written::Bak(bak));
        };
        Ok(store.write(&Request {
            bytes,
            kind,
            source_mtime,
            generation: label,
            committed,
            db_path: &self.path,
            db_len,
            vault_id,
            allow_bak,
        })?)
    }

    #[cfg(test)]
    fn run_before_commit(&mut self) {
        if let Some(hook) = self.before_commit.as_mut() {
            hook(&self.path);
        }
    }

    #[cfg(not(test))]
    fn run_before_commit(&mut self) {}

    /// Replaces the database with the backup at `path`, then saves it under the session's
    /// credentials, KDF and generation (a copy of the replaced version is kept as
    /// `before-restore`). Never a byte copy: that could bring back a weaker or revoked key and
    /// break passkey unlock.
    ///
    /// Refused while there are unsaved changes, and for a backup the session's credentials
    /// don't open. On `ChangedOnDisk` the restored database stays in the session, unsaved, for
    /// "Save anyway"; any other failure leaves the session as it was.
    pub fn restore(&mut self, path: &Path) -> Result<(), VaultError> {
        if self.read_only {
            return Err(VaultError::ReadOnly);
        }
        if self.dirty {
            return Err(VaultError::Unsaved);
        }
        let data = read_capped(path, DATABASE_MAX)?;
        check_kdf(&data, &self.kdf_budget)?;
        let mut restored = Database::parse(&data, self.credentials.to_key()?).map_err(|e| {
            if wrong_key(&e) {
                VaultError::NeedsOlderCredentials
            } else {
                VaultError::Open(e)
            }
        })?;
        // The format, ciphers and KDF of the current database; the save writes its record.
        restored.config = self.db.config.clone();
        generation::wipe_record(&mut restored);
        let mut replaced = std::mem::replace(&mut self.db, restored);
        self.dirty = true;
        self.restore_pending = true;
        match self.save() {
            Err(e) if !matches!(e, VaultError::ChangedOnDisk) => {
                let mut restored = std::mem::replace(&mut self.db, replaced);
                generation::wipe_record(&mut restored);
                self.dirty = false;
                self.restore_pending = false;
                Err(e)
            }
            result => {
                generation::wipe_record(&mut replaced);
                result
            }
        }
    }

    /// "Back up now": a pinned `manual` copy of the database file, which is what the session
    /// shows: refused while there are unsaved changes (`Unsaved`) and when another app changed
    /// the file since it was opened or saved (`ChangedOnDisk`). Never skipped as a duplicate and
    /// never written to `<db>.bak`.
    pub fn back_up_now(&mut self) -> Result<Written, VaultError> {
        if self.read_only {
            return Err(VaultError::ReadOnly);
        }
        if self.dirty {
            return Err(VaultError::Unsaved);
        }
        let (data, stamp) = read_stamped(&self.path)?;
        if self.on_disk.map(|s| s.sha256) != Some(stamp.sha256) {
            return Err(VaultError::ChangedOnDisk);
        }
        let vault_id = backups::vault_id_of(&self.db);
        let label = self.known.get(&stamp.sha256).cloned();
        let committed =
            label.as_deref() == Some(self.generation.generation()) && self.generation_committed;
        let store = self.store.as_mut().ok_or(StoreError::NoStore)?;
        Ok(store.write(&Request {
            bytes: &data,
            kind: Kind::Manual,
            source_mtime: stamp.modified,
            generation: label.as_deref(),
            committed,
            db_path: &self.path,
            db_len: data.len() as u64,
            vault_id: &vault_id,
            allow_bak: false,
        })?)
    }

    /// The Backups section's listing and deleting, for a worker thread (see [`BackupsTask`]).
    /// None without a store.
    pub fn backups_task(&self) -> Option<BackupsTask> {
        Some(BackupsTask {
            store: self.store.as_ref()?.detached(),
            db_path: self.path.clone(),
            vault_id: backups::vault_id_of(&self.db),
            generation: self.generation.generation().to_owned(),
        })
    }

    /// Files beside the database that belong in the store (see [`manage::old_files`]). None for
    /// a backup opened read-only.
    pub fn old_files(&self) -> Vec<OldFile> {
        if self.read_only {
            return Vec::new();
        }
        manage::old_files(&self.path)
    }

    /// Moves the files [`Vault::old_files`] lists into the store: each database copy is copied,
    /// read back and compared, then deleted; old passkey files are deleted.
    pub fn migrate_old_files(&mut self) -> Result<MigrationReport, VaultError> {
        if self.read_only {
            return Err(VaultError::ReadOnly);
        }
        let vault_id = backups::vault_id_of(&self.db);
        let store = self.store.as_mut().ok_or(StoreError::NoStore)?;
        Ok(manage::migrate(store, &self.path, &vault_id))
    }

    /// Whether the owner answered "Don't ask again" to moving this vault's old files (stored by
    /// vault id, so it follows the vault through a rename).
    pub fn migration_declined(&mut self) -> bool {
        let vault_id = backups::vault_id_of(&self.db);
        self.store
            .as_mut()
            .is_some_and(|store| store.migration_declined(&vault_id))
    }

    pub fn decline_migration(&mut self) {
        let vault_id = backups::vault_id_of(&self.db);
        if let Some(store) = self.store.as_mut() {
            store.decline_migration(&vault_id);
        }
    }

    /// Makes `root` the backup root from now on (None, or the default root itself: the default
    /// root). Run [`manage::check_new_root`] and confirm its warnings first. The marker goes into
    /// the chosen folder, and earlier roots stay known.
    pub fn set_backup_root(&mut self, root: Option<&Path>) -> Result<(), VaultError> {
        if self.read_only {
            return Err(VaultError::ReadOnly);
        }
        let store = self.store.as_mut().ok_or(StoreError::NoStore)?;
        let default = store.config().default_root.clone();
        let root =
            root.filter(|r| !(backups::is_within(r, &default) && backups::is_within(&default, r)));
        Ok(store.set_root(root)?)
    }

    /// Stops using `root`, a previous backup root: its backups stay where they are but are no
    /// longer listed, cleaned up or re-encrypted, and it is no longer checked at every save.
    /// Ok(false) if it wasn't a previous root.
    pub fn forget_backup_root(&mut self, root: &Path) -> Result<bool, VaultError> {
        if self.read_only {
            return Err(VaultError::ReadOnly);
        }
        let store = self.store.as_mut().ok_or(StoreError::NoStore)?;
        Ok(store.forget_root(root)?)
    }

    /// What a key change would do to the backups now (reads folders; derives no key).
    pub fn key_change_preview(&mut self) -> KeyChangePreview {
        let mut preview = KeyChangePreview {
            beside: rekey::database_copies_beside(&self.path)
                .into_iter()
                .map(|(path, _)| path)
                .collect(),
            ..KeyChangePreview::default()
        };
        let vault_id = backups::vault_id_of(&self.db);
        if let Some(store) = self.store.as_mut() {
            let (folders, unreachable) = store.existing_folders(&self.path, &vault_id);
            preview.backups = folders
                .iter()
                .map(|f| f.list().map_or(0, |l| l.len()))
                .sum();
            preview.unreachable = unreachable.into_iter().map(|(r, _)| r.path).collect();
        }
        preview
    }

    /// Prepares a key change to `new`: the work [`RekeyPlan::run`] does on a worker thread before
    /// [`Vault::commit_key_change`]. A key change needs the store.
    pub fn plan_key_change(&mut self, new: Credentials) -> Result<RekeyPlan, VaultError> {
        if self.read_only {
            return Err(VaultError::ReadOnly);
        }
        if self.previous_credentials.is_some() {
            return Err(VaultError::Verify(
                "another credential change is waiting to be saved".into(),
            ));
        }
        new.to_key()?;
        let store = self.store.as_ref().ok_or(StoreError::NoStore)?;
        Ok(RekeyPlan {
            store: store.detached(),
            db_path: self.path.clone(),
            vault_id: backups::vault_id_of(&self.db),
            old: self.credentials.clone(),
            record: CredentialRecord::new(&new),
            new,
            kdf_config: self.db.config.kdf_config.clone(),
            current_kdf: Kdf::of_config(&self.db.config.kdf_config)
                .or_else(|| header::kdf_of(&self.path)),
            budget: self.kdf_budget,
            committed: self
                .generation_committed
                .then(|| self.generation.generation().to_owned()),
        })
    }

    /// Commits a staged key change, in this order:
    /// 1. the session takes the new credentials, with the generation the copies were staged
    ///    under;
    /// 2. `ring` is staged beside the database as `<db>.axvault.~new`;
    /// 3. the database is saved (its previous version goes to the store as `before-key-change`;
    ///    the store is required). On failure the credentials go back, and the staged ring and
    ///    copies are deleted: nothing changed;
    /// 4. the staged ring replaces the passkey file;
    /// 5. the copies of the database beside it that staging moved into the store are deleted
    ///    (each only if it still holds what was moved), and the staged copies are published;
    /// 6. ring snapshots of other generations and old passkey files beside the database
    ///    (`<db>.axvault.old` and leftover temps) are deleted (they may wrap a revoked key), the
    ///    new ring is snapshotted, and the generation is recorded as committed with
    ///    `key_file_name`, the new backup key file.
    pub fn commit_key_change(
        &mut self,
        staged: Staged,
        ring: &KeyRing,
        key_file_name: Option<&str>,
    ) -> Result<KeyChangeReport, VaultError> {
        if staged.old != self.credentials || staged.db_path != self.path {
            staged.discard();
            return Err(VaultError::Verify(
                "the vault changed while its backups were being re-encrypted".into(),
            ));
        }
        // 1.
        let (old, was_dirty) = (self.credentials.clone(), self.dirty);
        self.set_credentials(staged.new.clone());
        if staged.record.matches(&self.credentials) {
            self.generation = staged.record.clone();
            self.generation_committed = false;
        }
        let roll_back = |vault: &mut Vault| {
            vault.set_credentials(old.clone());
            vault.dirty = was_dirty;
        };
        // 2.
        let ring_text = match envelope::stage_ring(&self.path, ring) {
            Ok(text) => text,
            Err(e) => {
                roll_back(self);
                staged.discard();
                return Err(e.into());
            }
        };
        // 3.
        let staged_ring = envelope::staged_path_for(&self.path);
        if let Err(e) = self.save() {
            roll_back(self);
            let _ = fsops::remove_file(&staged_ring);
            staged.discard();
            return Err(e);
        }
        // From here on the database is on the new key. A `<db>.bak` a fallback wrote holds the
        // old one: staging re-encrypted its copy, so it is never moved in as it is.
        let vault_id = backups::vault_id_of(&self.db);
        if let Some(store) = self.store.as_mut() {
            store.forget_fallback_bak(&self.path, &vault_id);
        }
        // 4.
        let ring_error = fsops::move_replace(&staged_ring, &envelope::path_for(&self.path))
            .err()
            .map(|e| e.to_string());
        // 5.
        let generation = self.generation().to_owned();
        let mut report = KeyChangeReport {
            rekeyed: staged.rekeyed(),
            kept_old: staged.failed(),
            unreachable: staged.unreachable().to_vec(),
            ring_error,
            ..KeyChangeReport::default()
        };
        // A folder that fails here keeps its manifest; the next unlock publishes it.
        let published = staged.publish();
        report.held = published.held;
        report.left_beside = published.left_beside;
        // 6.
        if let Some(store) = self.store.as_mut() {
            let (folders, _) = store.existing_folders(&self.path, &vault_id);
            for folder in &folders {
                let _ = folder.delete_rings_except(&generation);
            }
        }
        rekey::remove_old_rings(&self.path);
        let _ = self.snapshot_ring(ring_text.as_bytes());
        if let Some(store) = self.store.as_mut() {
            let (folders, _) = store.existing_folders(&self.path, &vault_id);
            for folder in &folders {
                let _ = folder.with_index(|index| {
                    index.note_generation(&generation, true);
                    if let (Some(info), Some(name)) =
                        (index.gens.get_mut(&generation), key_file_name)
                    {
                        info.keyx = Some(name.to_owned());
                    }
                });
            }
            store.prune(&self.path, &vault_id, Some(&generation));
        }
        Ok(report)
    }

    /// Test support: a key change whose database save commits, after which the app stops before
    /// replacing the passkey file or publishing anything.
    #[cfg(test)]
    pub(crate) fn commit_without_publishing(&mut self, staged: Staged) {
        self.set_credentials(staged.new.clone());
        self.generation = staged.record.clone();
        self.generation_committed = false;
        self.save().unwrap();
        staged.abandon();
    }

    /// Unlock-time repairs of the store:
    /// - a key change made outside Axentra (the record named other credentials): the generation
    ///   it replaced is retired, so its backups count as needing older credentials;
    /// - a key change interrupted after staging: its copies are published if the database
    ///   carries (committed) the generation they were staged under, and deleted otherwise (see
    ///   `rekey::recover`); old-key originals a published key change couldn't delete go too;
    /// - save temps beside the database (`<db>.~axv-<hex>.tmp`) left by a save that never
    ///   finished: one holding a complete KeePass file is copied into the store as `imported`
    ///   (copy, read back, compare) and then deleted; anything smaller is deleted once it is an
    ///   hour old.
    ///
    /// Never fails the unlock; does nothing without a store.
    pub fn recover_backups(&mut self) -> RecoveryReport {
        let mut report = RecoveryReport::default();
        let vault_id = backups::vault_id_of(&self.db);
        let committed = self
            .generation_committed
            .then(|| self.generation.generation().to_owned());
        let Some(store) = self.store.as_mut() else {
            return report;
        };
        if let Some(replaced) = &self.outside_generation {
            store.retire_generation(&self.path, &vault_id, replaced);
        }
        let (folders, _) = store.existing_folders(&self.path, &vault_id);
        report.rekey = rekey::recover(&folders, committed.as_deref(), &self.path);
        for folder in &folders {
            if let Ok(deleted) = folder.delete_superseded() {
                report.superseded.extend(deleted);
            }
        }

        let Some(dir) = self.path.parent() else {
            return report;
        };
        let Ok(entries) = fs::read_dir(dir) else {
            return report;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else { continue };
            if !meta.is_file()
                || fsops::is_reparse_point(&meta)
                || !rekey::is_leftover_temp(&self.path, &path)
            {
                continue;
            }
            if backups::looks_like_kdbx(&path) {
                let Ok(bytes) = read_capped(&path, DATABASE_MAX) else {
                    continue;
                };
                let sha256 = backups::sha256_hex(&bytes);
                let copied = store
                    .writable_folder(&self.path, &vault_id, bytes.len() as u64)
                    .ok()
                    .and_then(|folder| {
                        folder
                            .write_backup(&bytes, Kind::Imported, meta.modified().ok(), None)
                            .ok()
                    })
                    .is_some_and(|written| {
                        backups::sha256_file(written.path()).is_ok_and(|h| h == sha256)
                    });
                if copied && matches!(fsops::remove_file(&path), Ok(true)) {
                    report.imported.push(path);
                }
            } else {
                let old = meta
                    .modified()
                    .ok()
                    .and_then(|m| SystemTime::now().duration_since(m).ok())
                    .is_some_and(|age| age > std::time::Duration::from_secs(60 * 60));
                if old && matches!(fsops::remove_file(&path), Ok(true)) {
                    report.removed.push(path);
                }
            }
        }
        report
    }

    /// Copies a passkey file's contents into the store as a ring snapshot, labelled with the
    /// session's generation, unless the newest snapshot already holds them. Never to
    /// `<db>.bak`; Ok(None) without a store, or for text that isn't a key ring.
    pub fn snapshot_ring(&mut self, bytes: &[u8]) -> Result<Option<Written>, StoreError> {
        let parses = std::str::from_utf8(bytes).is_ok_and(|t| KeyRing::parse(t).is_ok());
        let vault_id = backups::vault_id_of(&self.db);
        let generation = self.generation.generation().to_owned();
        let Some(store) = self.store.as_mut() else {
            return Ok(None);
        };
        if !parses {
            return Ok(None);
        }
        let folder = store.writable_folder(&self.path, &vault_id, bytes.len() as u64)?;
        let outcome = folder
            .write_ring_snapshot(bytes, Some(&generation))
            .map_err(|e| StoreError::Failed(vec![(folder.path().to_owned(), e.to_string())]))?;
        Ok(Some(Written::Store { folder, outcome }))
    }

    /// The unlock-time ring snapshot: the passkey file beside the database, if any (see
    /// [`Vault::snapshot_ring`]). Skipped while a key change's staged ring (`<db>.axvault.~new`)
    /// is waiting: until passkey unlock settles which ring is current, neither is labelled.
    pub fn snapshot_sidecar(&mut self) -> Result<Option<Written>, StoreError> {
        if envelope::staged_path_for(&self.path).exists() {
            return Ok(None);
        }
        match read_capped(&envelope::path_for(&self.path), SIDECAR_MAX) {
            Ok(bytes) => self.snapshot_ring(&bytes),
            Err(_) => Ok(None),
        }
    }

    /// Applies retention to the vault's backup folders, as a save does afterwards; for unlock,
    /// after its `opened` copy and ring snapshot, so a PC that mostly reads never builds up
    /// copies. Ring snapshots of other generations go only once the session's generation is
    /// committed (a provisional one may be gone at the next unlock). Never fails; nothing
    /// without a store.
    pub fn prune_backups(&mut self) {
        let vault_id = backups::vault_id_of(&self.db);
        let committed = self
            .generation_committed
            .then(|| self.generation.generation().to_owned());
        if let Some(store) = self.store.as_mut() {
            store.prune(&self.path, &vault_id, committed.as_deref());
        }
    }

    fn verify_db(db: &Database, saved: &[u8], key: DatabaseKey) -> Result<(), VaultError> {
        let mut reread =
            Database::parse(saved, key).map_err(|e| VaultError::Verify(e.to_string()))?;
        generation::wipe_record(&mut reread);
        let counts = |d: &Database| {
            (
                d.num_groups(),
                d.num_entries(),
                d.num_attachments(),
                d.num_custom_icons(),
            )
        };
        if counts(&reread) != counts(db) {
            return Err(VaultError::Verify(format!(
                "groups/entries/attachments/icons {:?} after save, expected {:?}",
                counts(&reread),
                counts(db)
            )));
        }
        Ok(())
    }
}

/// The files beside a database that belong to it, by suffix: its fallback backup, the passkey
/// file and its staged and legacy copies, and temp names earlier versions used.
const DATABASE_FILE_SUFFIXES: &[&str] = &[
    "",
    ".bak",
    ".bak.tmp",
    ".tmp",
    envelope::EXTENSION,
    ".axvault.tmp",
    ".axvault.old",
    ".axvault.~new",
];

/// Whether `candidate` is one of the database's own files: the database, a file listed in
/// [`DATABASE_FILE_SUFFIXES`], or a temp a save writes beside one of them
/// (`<file>.~axv-<hex>.tmp`). Compared by name, case-insensitively, in the same folder (both
/// folders resolved). A backup key file saved under such a name would be replaced or deleted by
/// the next save or key change.
pub fn is_database_file(db_path: &Path, candidate: &Path) -> bool {
    let folder = |p: &Path| {
        p.parent().map(|d| {
            let d = if d.as_os_str().is_empty() {
                Path::new(".")
            } else {
                d
            };
            fs::canonicalize(d)
                .unwrap_or_else(|_| d.to_owned())
                .to_string_lossy()
                .to_lowercase()
        })
    };
    if folder(db_path) != folder(candidate) {
        return false;
    }
    let (Some(db_name), Some(name)) = (db_path.file_name(), candidate.file_name()) else {
        return false;
    };
    let (db_name, name) = (
        db_name.to_string_lossy().to_lowercase(),
        name.to_string_lossy().to_lowercase(),
    );
    DATABASE_FILE_SUFFIXES.iter().any(|suffix| {
        let own = format!("{db_name}{suffix}");
        name == own
            || name
                .strip_prefix(&own)
                .is_some_and(crate::fsops::is_temp_name)
    })
}

pub fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use keepass::config::KdfConfig;
    use keepass::db::{CustomDataItem, CustomDataValue, Times, fields};

    use super::*;
    use crate::backups::{Backup, BackupName, BackupSettings, Item, KnownRoot, RootSetting};
    use crate::random_bytes;

    fn temp_dir() -> PathBuf {
        let mut n = [0u8; 8];
        random_bytes(&mut n);
        let dir = std::env::temp_dir().join(format!("axvault-test-{}", hex::encode(n)));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn creds(password: &str, key: Option<[u8; 32]>) -> Credentials {
        Credentials {
            password: Some(Zeroizing::new(password.to_owned())),
            key_file: key.map(|k| Zeroizing::new(k.to_vec())),
        }
    }

    fn new_db(path: &Path, c: &Credentials) {
        let mut db = Database::new();
        // Fast KDF so tests do not spend seconds in Argon2.
        db.config.kdf_config = keepass::config::KdfConfig::Aes { rounds: 1000 };
        db.root_mut().add_entry().edit(|e| {
            e.set_unprotected(fields::TITLE, "Bank");
            e.set_protected(fields::PASSWORD, "hunter2");
        });
        let mut f = fs::File::create(path).unwrap();
        db.save(&mut f, c.to_key().unwrap()).unwrap();
    }

    /// A store whose default root is `<dir>\Backups`, with free space reported as unknown
    /// (enough) so tests don't depend on the disk.
    fn store_config(dir: &Path) -> BackupConfig {
        BackupConfig::new(dir.join("Backups")).with_free_space(|_| None)
    }

    /// A chosen root that is unplugged: it never existed.
    fn unplugged(dir: &Path) -> RootSetting {
        RootSetting {
            path: dir.join("usb"),
            marker: "00000000-0000-4000-8000-000000000000".into(),
        }
    }

    fn with_root(dir: &Path, root: RootSetting) -> BackupConfig {
        let mut settings = BackupSettings::default();
        settings.set_root(Some(root));
        BackupConfig::from_settings(dir.join("Backups"), &settings).with_free_space(|_| None)
    }

    fn open_stored(path: &Path, c: &Credentials, config: &BackupConfig) -> Vault {
        let mut v = Vault::open(path, c.clone()).unwrap();
        v.attach_store(config.clone());
        v
    }

    fn add_entry(v: &mut Vault, title: &str) {
        v.db.root_mut()
            .add_entry()
            .set_unprotected(fields::TITLE, title);
        v.mark_dirty();
    }

    /// The vault's backups in one root, newest first.
    fn backups_in(config: &BackupConfig, root: &KnownRoot, path: &Path, v: &Vault) -> Vec<Backup> {
        let id = backups::vault_id_of(&v.db);
        let mut settings = BackupSettings::default();
        match config.open_folder(root, path, &id, &mut settings, false) {
            Ok(Some(folder)) => folder.list().unwrap(),
            _ => Vec::new(),
        }
    }

    /// The vault's backups in every known root that answers.
    fn store_backups(config: &BackupConfig, path: &Path, v: &Vault) -> Vec<Backup> {
        config
            .known_roots()
            .iter()
            .flat_map(|root| backups_in(config, root, path, v))
            .collect()
    }

    fn of_kind(listed: &[Backup], kind: Kind) -> Vec<&Backup> {
        listed
            .iter()
            .filter(|b| b.name.kind() == Some(kind))
            .collect()
    }

    /// Temp files left in `dir` by saves (`<name>.~axv-<hex>.tmp`).
    fn leftover_temps(dir: &Path) -> Vec<String> {
        fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".~axv-"))
            .collect()
    }

    #[test]
    fn save_writes_exact_previous_bytes_to_store() {
        let dir = temp_dir();
        let path = dir.join("v.kdbx");
        let c = creds("pw", Some([7u8; 32]));
        new_db(&path, &c);
        let previous = fs::read(&path).unwrap();
        let source_mtime = fs::metadata(&path).unwrap().modified().unwrap();
        let config = store_config(&dir);

        let mut v = open_stored(&path, &c, &config);
        add_entry(&mut v, "Email");
        v.save().unwrap();
        assert!(!v.is_dirty());
        assert!(!with_suffix(&path, ".bak").exists(), "no .bak beside it");
        assert!(leftover_temps(&dir).is_empty());
        assert!(v.backup_status().is_none() && v.take_backup_notice().is_none());

        let listed = store_backups(&config, &path, &v);
        assert_eq!(listed.len(), 1);
        let copy = &listed[0];
        assert_eq!(copy.name.kind(), Some(Kind::Auto));
        assert_eq!(fs::read(&copy.path).unwrap(), previous);
        assert_eq!(
            copy.sha256.as_deref(),
            Some(&*backups::sha256_hex(&previous))
        );
        assert_eq!(
            fs::metadata(&copy.path).unwrap().modified().unwrap(),
            source_mtime,
            "the backup keeps the version's own time"
        );
        assert!(fs::metadata(&copy.path).unwrap().permissions().readonly());

        let v2 = Vault::open(&path, c).unwrap();
        assert_eq!(v2.db.num_entries(), 2);
        drop((v, v2));
        clear_readonly_and_remove(&dir);
    }

    #[test]
    fn fallback_order_configured_default_bak() {
        let dir = temp_dir();
        fs::create_dir_all(dir.join("data")).unwrap();
        let path = dir.join("data").join("v.kdbx");
        let c = creds("pw", None);
        new_db(&path, &c);
        let config = with_root(&dir, unplugged(&dir));
        let default_root = dir.join("Backups");
        let mut v = open_stored(&path, &c, &config);

        // The chosen root is unplugged: the default root takes the backup.
        add_entry(&mut v, "one");
        v.save().unwrap();
        let notice = v.take_backup_notice().expect("a notice");
        assert!(
            notice.contains(&default_root.display().to_string()),
            "{notice}"
        );
        // Said once, plainly: which folder, why, and where backups go instead.
        assert_eq!(
            notice,
            format!(
                "The backup folder {} can't be used: the folder isn't there (is the drive connected?). Backups go to {} meanwhile.",
                dir.join("usb").display(),
                default_root.display()
            )
        );
        assert!(v.backup_status().is_some());
        assert!(!dir.join("usb").exists(), "a chosen root is never created");
        let in_default = backups_in(&config, &config.default_known_root(), &path, &v);
        assert_eq!(in_default.len(), 1);

        // The default root can't be written either: `<db>.bak`, and the status says so.
        let away = dir.join("Backups.away");
        fs::rename(&default_root, &away).unwrap();
        fs::write(&default_root, b"not a folder").unwrap();
        let previous = fs::read(&path).unwrap();
        add_entry(&mut v, "two");
        v.save().unwrap();
        let bak = with_suffix(&path, ".bak");
        assert_eq!(fs::read(&bak).unwrap(), previous);
        let status = v.backup_status().expect("status").to_owned();
        assert!(status.contains(&bak.display().to_string()), "{status}");
        assert!(v.take_backup_notice().is_none(), "one notice per session");

        // Once the default root is back, the next save moves that `.bak` into the store.
        fs::remove_file(&default_root).unwrap();
        fs::rename(&away, &default_root).unwrap();
        add_entry(&mut v, "three");
        v.save().unwrap();
        assert!(!bak.exists(), "the fallback .bak was moved");
        let listed = backups_in(&config, &config.default_known_root(), &path, &v);
        let imported = of_kind(&listed, Kind::Imported);
        assert_eq!(imported.len(), 1);
        assert_eq!(fs::read(&imported[0].path).unwrap(), previous);
        assert_eq!(of_kind(&listed, Kind::Auto).len(), 2);
        assert_eq!(Vault::open(&path, c).unwrap().db.num_entries(), 4);
        assert!(leftover_temps(&dir.join("data")).is_empty());
        drop(v);
        clear_readonly_and_remove(&dir);
    }

    #[test]
    fn credential_change_never_falls_back_to_bak() {
        let dir = temp_dir();
        let path = dir.join("rekey.kdbx");
        let old = creds("pw", None);
        let new = creds("pw", Some([9; 32]));
        new_db(&path, &old);
        let config = with_root(&dir, unplugged(&dir));
        // The default root is blocked as well.
        fs::write(dir.join("Backups"), b"in the way").unwrap();
        let before = fs::read(&path).unwrap();

        let mut v = open_stored(&path, &old, &config);
        add_entry(&mut v, "new unsaved entry");
        v.set_credentials(new.clone());
        let err = v.save().unwrap_err();
        assert!(matches!(err, VaultError::Backup(_)), "{err}");
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(Vault::open(&path, old.clone()).unwrap().db.num_entries(), 1);
        assert!(!with_suffix(&path, ".bak").exists());
        assert!(leftover_temps(&dir).is_empty());

        // Without any store, the same.
        let mut bare = Vault::open(&path, old.clone()).unwrap();
        bare.set_credentials(new);
        assert!(matches!(
            bare.save(),
            Err(VaultError::Backup(StoreError::NoStore))
        ));
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(!with_suffix(&path, ".bak").exists());
        drop((v, bare));
        clear_readonly_and_remove(&dir);
    }

    #[test]
    fn credential_change_protects_previous_contents_and_can_fail_safely() {
        let dir = temp_dir();
        let path = dir.join("rekey.kdbx");
        let old = creds("pw", None);
        let new = creds("pw", Some([9; 32]));
        new_db(&path, &old);
        let config = store_config(&dir);
        let mut vault = open_stored(&path, &old, &config);
        add_entry(&mut vault, "new unsaved entry");
        vault.set_credentials(new.clone());
        // A failed backup commit must leave the original usable.
        let blocker = dir.join("Backups");
        fs::write(&blocker, b"in the way").unwrap();
        assert!(vault.save().is_err());
        assert_eq!(Vault::open(&path, old.clone()).unwrap().db.num_entries(), 1);
        fs::remove_file(&blocker).unwrap();
        vault.save().unwrap();
        assert!(Vault::open(&path, old.clone()).is_err());
        assert_eq!(Vault::open(&path, new.clone()).unwrap().db.num_entries(), 2);

        let copies = store_backups(&config, &path, &vault);
        assert_eq!(copies.len(), 1);
        let copy = &copies[0];
        assert_eq!(copy.name.kind(), Some(Kind::BeforeKeyChange));
        assert_eq!(copy.generation.as_deref(), Some(vault.generation()));
        assert!(Vault::open(&copy.path, old).is_err());
        assert_eq!(Vault::open(&copy.path, new).unwrap().db.num_entries(), 1);
        assert!(!with_suffix(&path, ".bak").exists());
        drop(vault);
        clear_readonly_and_remove(&dir);
    }

    #[test]
    fn save_fails_without_durable_previous_copy() {
        let dir = temp_dir();
        let path = dir.join("v.kdbx");
        let c = creds("pw", None);
        new_db(&path, &c);
        let config = with_root(&dir, unplugged(&dir));
        fs::write(dir.join("Backups"), b"in the way").unwrap();
        // Nothing can be renamed over a folder either.
        let bak = with_suffix(&path, ".bak");
        fs::create_dir(&bak).unwrap();
        let before = fs::read(&path).unwrap();

        let mut v = open_stored(&path, &c, &config);
        add_entry(&mut v, "unsaved");
        let err = v.save().unwrap_err();
        assert!(
            matches!(&err, VaultError::Backup(StoreError::Failed(tried)) if tried.len() == 3),
            "{err}"
        );
        assert_eq!(
            fs::read(&path).unwrap(),
            before,
            "the main file is untouched"
        );
        assert!(v.is_dirty());
        assert!(leftover_temps(&dir).is_empty());
        assert!(bak.is_dir());
        assert!(v.backup_status().is_some());
        drop(v);
        clear_readonly_and_remove(&dir);
    }

    #[test]
    fn external_write_between_backup_and_rename_is_detected() {
        let dir = temp_dir();
        let path = dir.join("v.kdbx");
        let c = creds("pw", None);
        new_db(&path, &c);
        let config = store_config(&dir);
        let mut v = open_stored(&path, &c, &config);
        add_entry(&mut v, "unsaved");

        // Same length, modification time put back: only the contents differ.
        let written = Arc::new(Mutex::new(None));
        let outside = written.clone();
        v.before_commit = Some(Box::new(move |p: &Path| {
            let mtime = fs::metadata(p).unwrap().modified().unwrap();
            let mut bytes = fs::read(p).unwrap();
            let last = bytes.len() - 1;
            bytes[last] ^= 0xFF;
            fs::write(p, &bytes).unwrap();
            fs::OpenOptions::new()
                .write(true)
                .open(p)
                .unwrap()
                .set_modified(mtime)
                .unwrap();
            *outside.lock().unwrap() = Some(bytes);
        }));
        assert!(matches!(v.save(), Err(VaultError::ChangedOnDisk)));
        let outside = written.lock().unwrap().take().unwrap();
        assert_eq!(
            fs::read(&path).unwrap(),
            outside,
            "the outside bytes are intact"
        );
        assert!(leftover_temps(&dir).is_empty(), "no temp left behind");
        assert!(v.is_dirty());
        // The copy written before the check stays.
        assert_eq!(store_backups(&config, &path, &v).len(), 1);
        drop(v);
        clear_readonly_and_remove(&dir);
    }

    #[test]
    fn overwrite_refuses_other_credentials() {
        let dir = temp_dir();
        let path = dir.join("v.kdbx");
        let k1 = creds("pw", Some([1; 32]));
        let k2 = creds("pw", Some([2; 32]));
        new_db(&path, &k1);
        let config = store_config(&dir);
        let mut ours = open_stored(&path, &k1, &config);
        add_entry(&mut ours, "unsaved");

        // Another device rotates the key to K2.
        let mut other = open_stored(&path, &k1, &config);
        other.set_credentials(k2.clone());
        other.save().unwrap();
        let rotated = fs::read(&path).unwrap();
        let stored = store_backups(&config, &path, &ours).len();

        assert!(matches!(ours.save(), Err(VaultError::ChangedOnDisk)));
        let err = ours.save_overwriting().unwrap_err();
        assert!(
            matches!(err, VaultError::CredentialsChangedElsewhere),
            "{err}"
        );
        assert!(err.to_string().starts_with("Lock and reopen"), "{err}");
        assert_eq!(fs::read(&path).unwrap(), rotated, "still on K2");
        assert!(Vault::open(&path, k2).is_ok());
        assert_eq!(store_backups(&config, &path, &ours).len(), stored);
        assert!(leftover_temps(&dir).is_empty());
        drop((ours, other));
        clear_readonly_and_remove(&dir);
    }

    /// A failed sync or another app's crashed write leaves the vault file cut off or damaged.
    /// "Save anyway" says the file can't be read, and once that is confirmed the session's
    /// version replaces it (the damaged bytes kept as a backup), so what is open here is never
    /// left only in memory. A database on other credentials is still never replaced.
    #[test]
    fn save_anyway_replaces_an_unreadable_file_once_confirmed() {
        let dir = temp_dir();
        let path = dir.join("v.kdbx");
        let c = creds("pw", None);
        new_db(&path, &c);
        let config = store_config(&dir);
        let mut v = open_stored(&path, &c, &config);
        add_entry(&mut v, "saved");
        v.save().unwrap();
        let good = fs::read(&path).unwrap();
        for damaged in [Vec::new(), good[..200].to_vec(), {
            let mut bytes = good.clone();
            bytes[0] ^= 0xFF;
            bytes
        }] {
            fs::write(&path, &damaged).unwrap();
            add_entry(&mut v, "unsaved");
            assert!(matches!(v.save(), Err(VaultError::ChangedOnDisk)));
            let err = v.save_overwriting().unwrap_err();
            assert!(matches!(err, VaultError::UnreadableOnDisk(_)), "{err}");
            assert_eq!(fs::read(&path).unwrap(), damaged, "not yet");
            v.save_replacing_unreadable().unwrap();
            assert!(!v.is_dirty());
            let reopened = Vault::open(&path, c.clone()).unwrap();
            assert_eq!(reopened.db.num_entries(), v.db.num_entries());
            let listed = store_backups(&config, &path, &v);
            let replaced = of_kind(&listed, Kind::ReplacedExternal);
            assert_eq!(fs::read(&replaced[0].path).unwrap(), damaged);
            assert_eq!(replaced[0].generation, None, "nothing known opens it");
        }

        // A readable database on another key is never replaced, even when confirmed.
        let other = creds("pw", Some([3; 32]));
        let mut elsewhere = open_stored(&path, &c, &config);
        elsewhere.set_credentials(other.clone());
        elsewhere.save().unwrap();
        let rotated = fs::read(&path).unwrap();
        add_entry(&mut v, "unsaved");
        assert!(matches!(
            v.save_replacing_unreadable(),
            Err(VaultError::CredentialsChangedElsewhere)
        ));
        assert_eq!(fs::read(&path).unwrap(), rotated);
        drop((v, elsewhere));
        clear_readonly_and_remove(&dir);
    }

    #[test]
    fn low_free_space_skips_root() {
        use std::sync::atomic::{AtomicU64, Ordering};

        let dir = temp_dir();
        let path = dir.join("v.kdbx");
        let c = creds("pw", None);
        new_db(&path, &c);
        let usb = dir.join("usb");
        fs::create_dir(&usb).unwrap();
        let marker = backups::ensure_marker(&usb).unwrap();
        let free = Arc::new(AtomicU64::new(1024));
        let reported = free.clone();
        let low = usb.clone();
        let config = with_root(&dir, RootSetting { path: usb, marker }).with_free_space(move |p| {
            backups::is_within(p, &low).then(|| reported.load(Ordering::SeqCst))
        });
        let mut v = open_stored(&path, &c, &config);

        add_entry(&mut v, "one");
        v.save().unwrap();
        assert!(backups_in(&config, &config.home(), &path, &v).is_empty());
        assert_eq!(
            backups_in(&config, &config.default_known_root(), &path, &v).len(),
            1
        );
        let notice = v.take_backup_notice().unwrap();
        assert!(notice.contains("free space"), "{notice}");

        // With room again, backups go to the chosen root and the status clears.
        free.store(u64::MAX, Ordering::SeqCst);
        add_entry(&mut v, "two");
        v.save().unwrap();
        assert_eq!(backups_in(&config, &config.home(), &path, &v).len(), 1);
        assert!(v.backup_status().is_none());
        drop(v);
        clear_readonly_and_remove(&dir);
    }

    #[test]
    fn labels_follow_content_hash() {
        let dir = temp_dir();
        let path = dir.join("v.kdbx");
        let c = creds("pw", None);
        new_db(&path, &c);
        let config = store_config(&dir);
        let (mut v, opened) = Vault::open_read(&path, c.clone(), KdfBudget::DEFAULT).unwrap();
        v.attach_store(config.clone());
        let generation = v.generation().to_owned();
        let written = v.snapshot_opened(&opened).unwrap().unwrap();
        assert!(matches!(
            written,
            Written::Store {
                outcome: backups::WriteOutcome::Written(_),
                ..
            }
        ));

        // The first `auto` copy equals the `opened` one and is skipped; the second holds bytes
        // this session saved.
        add_entry(&mut v, "one");
        v.save().unwrap();
        add_entry(&mut v, "two");
        v.save().unwrap();
        // Another app saves under the same credentials; "Save anyway" parses its version
        // with the session's credentials first.
        rekey_elsewhere(&path, &c, &c);
        add_entry(&mut v, "three");
        assert!(matches!(v.save(), Err(VaultError::ChangedOnDisk)));
        v.save_overwriting().unwrap();

        // Bytes nobody in this session opened, saved or parsed.
        let folder = match &written {
            Written::Store { folder, .. } => folder.clone(),
            Written::Bak(_) => unreachable!(),
        };
        let foreign = BackupName {
            stem: folder.stem().to_owned(),
            date: "2020-01-01".into(),
            time: "00-00-00".into(),
            seq: 1000,
            rev: 0,
            item: Item::Kdbx(Kind::Manual),
        };
        let mut external = Vec::new();
        let mut db = Database::new();
        db.config.kdf_config = KdfConfig::Aes { rounds: 1000 };
        db.save(&mut external, c.to_key().unwrap()).unwrap();
        fs::write(folder.path().join(foreign.file_name()), &external).unwrap();

        let listed = folder.list().unwrap();
        let kinds: Vec<_> = listed.iter().map(|b| b.name.kind().unwrap()).collect();
        assert_eq!(
            kinds,
            [
                Kind::Manual,
                Kind::ReplacedExternal,
                Kind::Auto,
                Kind::Opened
            ]
        );
        assert_eq!(listed[0].generation, None, "external bytes are unknown");
        for b in &listed[1..] {
            assert_eq!(
                b.generation.as_deref(),
                Some(&*generation),
                "{}",
                b.file_name
            );
        }
        let index = folder.index().unwrap();
        assert!(index.gens[&generation].committed);
        drop(v);
        clear_readonly_and_remove(&dir);
    }

    #[test]
    fn restore_resaves_under_current_credentials_and_kdf() {
        let dir = temp_dir();
        let path = dir.join("v.kdbx");
        let c = creds("pw", Some([4; 32]));
        // The vault uses a (cheap) Argon2id...
        let mut db = Database::new();
        db.config.kdf_config = KdfConfig::Argon2id {
            iterations: 2,
            memory: 64 * 1024,
            parallelism: 1,
            version: argon2::Version::Version13,
        };
        db.root_mut()
            .add_entry()
            .set_unprotected(fields::TITLE, "Current");
        db.save(&mut fs::File::create(&path).unwrap(), c.to_key().unwrap())
            .unwrap();
        let current_kdf = header::kdf_of(&path).unwrap();
        // ...and the backup is an earlier version of it with a weak AES KDF.
        let mut earlier = Database::parse(&fs::read(&path).unwrap(), c.to_key().unwrap()).unwrap();
        earlier
            .root_mut()
            .add_entry()
            .set_unprotected(fields::TITLE, "Restored");
        earlier.config.kdf_config = KdfConfig::Aes { rounds: 1000 };
        let backup = dir.join("earlier.kdbx");
        earlier
            .save(&mut fs::File::create(&backup).unwrap(), c.to_key().unwrap())
            .unwrap();
        assert_ne!(header::kdf_of(&backup).unwrap(), current_kdf);
        let replaced = fs::read(&path).unwrap();

        let config = store_config(&dir);
        let mut v = open_stored(&path, &c, &config);
        v.restore(&backup).unwrap();
        assert!(!v.is_dirty());
        assert_eq!(v.db.num_entries(), 2);
        assert_eq!(header::kdf_of(&path).unwrap(), current_kdf);
        let reopened = Vault::open(&path, c.clone()).unwrap();
        assert_eq!(reopened.db.num_entries(), 2);
        assert_eq!(reopened.generation(), v.generation());
        assert!(reopened.generation_committed());

        let copies = store_backups(&config, &path, &v);
        let before = of_kind(&copies, Kind::BeforeRestore);
        assert_eq!(before.len(), 1);
        assert_eq!(fs::read(&before[0].path).unwrap(), replaced);
        assert!(Kind::BeforeRestore.is_pinned());

        // Unsaved changes are never thrown away by a restore.
        add_entry(&mut v, "unsaved");
        assert!(matches!(v.restore(&backup), Err(VaultError::Unsaved)));
        assert_eq!(v.db.num_entries(), 3);
        drop((v, reopened));
        clear_readonly_and_remove(&dir);
    }

    #[test]
    fn restore_rejects_backup_needing_older_credentials() {
        let dir = temp_dir();
        let path = dir.join("v.kdbx");
        let old = creds("pw", None);
        let new = creds("pw", Some([5; 32]));
        new_db(&path, &old);
        let earlier = dir.join("earlier.kdbx");
        fs::copy(&path, &earlier).unwrap();
        let config = store_config(&dir);
        let mut v = open_stored(&path, &old, &config);
        v.set_credentials(new.clone());
        v.save().unwrap();
        let current = fs::read(&path).unwrap();

        let err = v.restore(&earlier).unwrap_err();
        assert!(matches!(err, VaultError::NeedsOlderCredentials), "{err}");
        assert!(err.to_string().contains("older credentials"), "{err}");
        assert_eq!(fs::read(&path).unwrap(), current);
        assert!(!v.is_dirty());
        assert_eq!(v.db.num_entries(), 1);
        assert!(Vault::open(&path, new.clone()).is_ok());

        // The key change's own copy, re-encrypted to the new key, restores.
        let copies = store_backups(&config, &path, &v);
        let rekeyed = of_kind(&copies, Kind::BeforeKeyChange);
        v.restore(&rekeyed[0].path).unwrap();
        assert!(Vault::open(&path, new).is_ok());
        drop(v);
        clear_readonly_and_remove(&dir);
    }

    #[test]
    fn read_only_vault_never_saves() {
        let dir = temp_dir();
        let path = dir.join("v.kdbx");
        let c = creds("pw", None);
        new_db(&path, &c);
        let before = fs::read(&path).unwrap();
        let mut v = Vault::open(&path, c).unwrap();
        v.set_read_only();
        v.attach_store(store_config(&dir));
        assert!(v.is_read_only() && v.backup_config().is_none());
        add_entry(&mut v, "unsaved");
        assert!(matches!(v.save(), Err(VaultError::ReadOnly)));
        assert!(matches!(v.save_overwriting(), Err(VaultError::ReadOnly)));
        let sealed = v.seal().unwrap();
        assert!(
            Vault::open_sealed(&sealed, creds("pw", None))
                .unwrap()
                .is_read_only()
        );
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(!dir.join("Backups").exists());
        drop(v);
        clear_readonly_and_remove(&dir);
    }

    #[test]
    fn wrong_key_fails() {
        let dir = temp_dir();
        let path = dir.join("w.kdbx");
        new_db(&path, &creds("pw", Some([7u8; 32])));
        assert!(matches!(
            Vault::open(&path, creds("pw", Some([8u8; 32]))),
            Err(VaultError::Open(_))
        ));
        assert!(matches!(
            Vault::open(&path, creds("pw", None)),
            Err(VaultError::Open(_))
        ));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn verify_credentials_checks_without_opening() {
        let dir = temp_dir();
        let path = dir.join("check.kdbx");
        new_db(&path, &creds("pw", Some([7u8; 32])));
        let verify = |path: &Path, c| Vault::verify_credentials(path, &c, &KdfBudget::DEFAULT);
        assert!(verify(&path, creds("pw", Some([7u8; 32]))).is_ok());
        assert!(matches!(
            verify(&path, creds("pw", Some([8u8; 32]))),
            Err(VaultError::Open(_))
        ));
        assert!(matches!(
            verify(&dir.join("missing.kdbx"), creds("pw", None)),
            Err(VaultError::Io(_))
        ));
        fs::remove_dir_all(dir).ok();
    }

    /// Saved with a cheap Argon2id whose 101 passes are just over the budget, so approving it
    /// can be exercised without a slow key derivation.
    fn new_heavy_db(path: &Path, c: &Credentials) -> Kdf {
        let mut db = Database::new();
        db.config.kdf_config = KdfConfig::Argon2id {
            iterations: 101,
            memory: 64 * 1024,
            parallelism: 1,
            version: argon2::Version::Version13,
        };
        db.save(&mut fs::File::create(path).unwrap(), c.to_key().unwrap())
            .unwrap();
        header::kdf_of(path).unwrap()
    }

    /// The audit's case: a synced file whose header asks Argon2 for 64 GiB, with the (unkeyed)
    /// header hash fixed up so keepass would accept it and start deriving.
    #[test]
    fn header_patched_to_64_gib_fails_fast_before_key_derivation() {
        const MEMORY: u64 = 64 << 30;
        let dir = temp_dir();
        let path = dir.join("bomb.kdbx");
        let c = creds("pw", None);
        let mut db = Database::new();
        db.config.kdf_config = KdfConfig::Argon2id {
            iterations: 2,
            memory: 1 << 20,
            parallelism: 1,
            version: argon2::Version::Version13,
        };
        let mut bytes = Vec::new();
        db.save(&mut bytes, c.to_key().unwrap()).unwrap();
        fs::write(
            &path,
            header::patch_kdf_param(&bytes, "M", &MEMORY.to_le_bytes()),
        )
        .unwrap();

        let started = std::time::Instant::now();
        let over_budget = |r: Result<(), VaultError>| matches!(r, Err(VaultError::KdfOverBudget(Kdf::Argon2 { memory_bytes, .. })) if memory_bytes == MEMORY);
        assert!(over_budget(Vault::open(&path, c.clone()).map(drop)));
        assert!(over_budget(Vault::verify_credentials(
            &path,
            &c,
            &KdfBudget::DEFAULT
        )));
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "no key derivation ran"
        );
        let Err(err) = Vault::open(&path, c) else {
            panic!("opened");
        };
        assert!(err.to_string().contains("65536 MB"), "{err}");
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn unknown_kdf_fails_closed() {
        let dir = temp_dir();
        let path = dir.join("unknown.kdbx");
        let c = creds("pw", None);
        let mut db = Database::new();
        db.config.kdf_config = KdfConfig::Aes { rounds: 1000 };
        let mut bytes = Vec::new();
        db.save(&mut bytes, c.to_key().unwrap()).unwrap();
        fs::write(&path, header::patch_kdf_param(&bytes, "$UUID", &[0x11; 16])).unwrap();
        assert!(matches!(
            Vault::open(&path, c),
            Err(VaultError::Header(HeaderError::UnknownKdf))
        ));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn approved_kdf_opens_and_bounds_later_reads() {
        let dir = temp_dir();
        let path = dir.join("heavy.kdbx");
        let old = creds("pw", None);
        let kdf = new_heavy_db(&path, &old);
        let Err(VaultError::KdfOverBudget(found)) = Vault::open(&path, old.clone()) else {
            panic!("an over-budget header opened without approval");
        };
        assert_eq!(found, kdf);

        let approved = KdfBudget::DEFAULT.allowing(&kdf);
        let mut v = Vault::open_with_budget(&path, old.clone(), approved).unwrap();
        let config = store_config(&dir);
        v.attach_store(config.clone());
        assert_eq!(v.kdf_budget(), approved);
        assert!(Vault::verify_credentials(&path, &old, &approved).is_ok());
        // Recovery keeps the approval for the file on disk.
        let sealed = v.seal().unwrap();
        let recovered = Vault::open_sealed(&sealed, old.clone()).unwrap();
        assert_eq!(recovered.kdf_budget(), approved);

        // A credential change re-reads the file on disk within the approved budget.
        let new = creds("pw", Some([5; 32]));
        v.set_credentials(new.clone());
        v.save().unwrap();
        assert!(Vault::open_with_budget(&path, new.clone(), approved).is_ok());
        let copies = store_backups(&config, &path, &v);
        assert_eq!(copies.len(), 1);
        assert!(Vault::open_with_budget(&copies[0].path, new, approved).is_ok());
        drop((v, recovered));
        clear_readonly_and_remove(&dir);
    }

    #[test]
    fn rekey_read_refuses_an_over_budget_file_put_in_its_place() {
        let dir = temp_dir();
        let path = dir.join("swap.kdbx");
        let old = creds("pw", None);
        new_db(&path, &old);
        let config = store_config(&dir);
        let mut v = open_stored(&path, &old, &config);
        v.set_credentials(creds("pw", Some([6; 32])));
        // Another device (or anyone who can write the synced file) replaces it.
        let kdf = new_heavy_db(&path, &old);
        let swapped = fs::read(&path).unwrap();
        assert!(matches!(
            v.save_overwriting(),
            Err(VaultError::UnreadableOnDisk(e)) if matches!(*e, VaultError::KdfOverBudget(ref k) if *k == kdf)
        ));
        // Not even once confirmed: the previous version of a credential change must be read.
        assert!(matches!(
            v.save_replacing_unreadable(),
            Err(VaultError::UnreadableOnDisk(_))
        ));
        assert_eq!(fs::read(&path).unwrap(), swapped, "nothing was written");
        assert!(!with_suffix(&path, ".bak").exists());
        assert!(leftover_temps(&dir).is_empty());
        assert!(store_backups(&config, &path, &v).is_empty());
        drop(v);
        clear_readonly_and_remove(&dir);
    }

    #[test]
    fn oversized_database_is_refused_before_reading() {
        let dir = temp_dir();
        let path = dir.join("huge.kdbx");
        // Sized without writing: the length check refuses it before any byte is read.
        fs::File::create(&path)
            .unwrap()
            .set_len(DATABASE_MAX + 1)
            .unwrap();
        let too_large = |r: Result<(), VaultError>| matches!(r, Err(VaultError::Io(e)) if e.kind() == std::io::ErrorKind::FileTooLarge);
        assert!(too_large(Vault::open(&path, creds("pw", None)).map(drop)));
        assert!(too_large(Vault::verify_credentials(
            &path,
            &creds("pw", None),
            &KdfBudget::DEFAULT
        )));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn refuses_to_clobber_external_change() {
        let dir = temp_dir();
        let path = dir.join("c.kdbx");
        let c = creds("pw", None);
        new_db(&path, &c);
        let mut v = Vault::open(&path, c.clone()).unwrap();

        // Another app saves the file.
        std::thread::sleep(std::time::Duration::from_millis(20));
        new_db(&path, &c);
        let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
        std::io::Write::write_all(&mut f, b"x").unwrap();

        assert!(matches!(v.save(), Err(VaultError::ChangedOnDisk)));
        fs::remove_dir_all(dir).ok();
    }

    /// The audit's M3 case: two sessions on one file. The second save is refused.
    #[test]
    fn second_vault_on_the_same_file_gets_changed_on_disk() {
        let dir = temp_dir();
        let path = dir.join("two.kdbx");
        let c = creds("pw", None);
        new_db(&path, &c);
        let config = store_config(&dir);
        let mut first = open_stored(&path, &c, &config);
        let mut second = open_stored(&path, &c, &config);
        add_entry(&mut first, "first");
        add_entry(&mut second, "second");
        first.save().unwrap();
        let saved = fs::read(&path).unwrap();
        assert!(matches!(second.save(), Err(VaultError::ChangedOnDisk)));
        assert_eq!(fs::read(&path).unwrap(), saved);
        drop((first, second));
        clear_readonly_and_remove(&dir);
    }

    #[test]
    fn rekey_with_passkey_bytes_matches_backup_keyfile() {
        let dir = temp_dir();
        let path = dir.join("r.kdbx");
        new_db(&path, &creds("pw", None));

        let key = [42u8; 32];
        let mut v = open_stored(&path, &creds("pw", None), &store_config(&dir));
        v.set_credentials(creds("pw", Some(key)));
        v.save().unwrap();

        // The backup .keyx (XML 2.0) must open what the raw passkey bytes locked.
        let xml = crate::keyfile::build_v2(&key);
        let via_keyfile = Credentials {
            password: Some(Zeroizing::new("pw".into())),
            key_file: Some(Zeroizing::new(xml.into_bytes())),
        };
        assert_eq!(Vault::open(&path, via_keyfile).unwrap().db.num_entries(), 1);
        drop(v);
        clear_readonly_and_remove(&dir);
    }

    fn with_key_file(password: &str, file: Vec<u8>) -> Credentials {
        Credentials {
            password: Some(Zeroizing::new(password.to_owned())),
            key_file: Some(Zeroizing::new(file)),
        }
    }

    /// The credential record in the file, as KeePass stores Meta CustomData.
    fn record_value(path: &Path, c: &Credentials) -> Option<String> {
        let db = Database::parse(&fs::read(path).unwrap(), c.to_key().unwrap()).unwrap();
        match db
            .meta
            .custom_data
            .get(generation::RECORD_KEY)?
            .value
            .as_ref()?
        {
            CustomDataValue::String(s) => Some(s.clone()),
            CustomDataValue::Binary(_) => None,
        }
    }

    fn record_generation(path: &Path, c: &Credentials) -> Option<String> {
        record_value(path, c)?.split(';').nth(1).map(str::to_owned)
    }

    /// Re-keys the file the way another KeePass app does: everything, the record included, is
    /// carried over unchanged under the new key.
    fn rekey_elsewhere(path: &Path, old: &Credentials, new: &Credentials) {
        let db = Database::parse(&fs::read(path).unwrap(), old.to_key().unwrap()).unwrap();
        let mut out = Vec::new();
        db.save(&mut out, new.to_key().unwrap()).unwrap();
        fs::write(path, out).unwrap();
    }

    #[test]
    fn credential_check_detects_outside_change() {
        let dir = temp_dir();
        let cases = [
            // KeePass changes the master password, adds a key file, replaces the key file.
            (creds("pw", None), creds("changed in KeePass", None)),
            (creds("pw", None), creds("pw", Some([3; 32]))),
            (creds("pw", Some([3; 32])), creds("pw", Some([4; 32]))),
        ];
        for (i, (old, new)) in cases.into_iter().enumerate() {
            let path = dir.join(format!("outside-{i}.kdbx"));
            new_db(&path, &old);
            // Never saved by Axentra: no record, a provisional generation and no notice.
            let mut v = Vault::open(&path, old.clone()).unwrap();
            assert!(!v.generation_committed() && !v.outside_change(), "case {i}");
            let first = v.generation().to_owned();
            assert_eq!(first.len(), 32);
            v.mark_dirty();
            v.save().unwrap();
            assert!(v.generation_committed());
            assert_eq!(v.generation(), first);
            drop(v);
            let v = Vault::open(&path, old.clone()).unwrap();
            assert_eq!(v.generation(), first);
            assert!(v.generation_committed() && !v.outside_change(), "case {i}");
            drop(v);

            rekey_elsewhere(&path, &old, &new);
            assert_eq!(record_generation(&path, &new).as_deref(), Some(&*first));
            let mut v = Vault::open(&path, new.clone()).unwrap();
            assert!(v.outside_change(), "case {i}");
            assert!(!v.generation_committed());
            let second = v.generation().to_owned();
            assert_ne!(second, first);
            // Recovery keeps the notice and the provisional generation.
            v.mark_dirty();
            let recovered = Vault::open_sealed(&v.seal().unwrap(), new.clone()).unwrap();
            assert!(recovered.outside_change() && !recovered.generation_committed());
            assert_eq!(recovered.generation(), second);
            drop(recovered);
            // The next save records the new credentials; after that the file opens clean.
            v.save().unwrap();
            assert!(v.generation_committed());
            drop(v);
            let v = Vault::open(&path, new).unwrap();
            assert_eq!(v.generation(), second);
            assert!(v.generation_committed() && !v.outside_change(), "case {i}");
        }
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn credential_check_is_form_independent() {
        let dir = temp_dir();
        let path = dir.join("forms.kdbx");
        let key: [u8; 32] = core::array::from_fn(|i| (i as u8).wrapping_mul(11).wrapping_add(5));
        let raw = with_key_file("pw", key.to_vec());
        let keyx = with_key_file("pw", crate::keyfile::build_v2(&key).into_bytes());
        let hex_file = with_key_file("pw", hex::encode_upper(key).into_bytes());
        assert_eq!(
            generation::composite_key(&raw)[..],
            generation::composite_key(&keyx)[..]
        );

        // Locked with a passkey's raw bytes...
        new_db(&path, &creds("pw", None));
        let mut v = open_stored(&path, &creds("pw", None), &store_config(&dir));
        v.set_credentials(raw.clone());
        v.save().unwrap();
        let generation = v.generation().to_owned();
        let record = record_value(&path, &raw).unwrap();
        drop(v);
        // ...then opened with the backup .keyx or a hex key file: same check, same generation,
        // and saving in that form leaves the record as it is.
        for c in [&raw, &keyx, &hex_file] {
            let mut v = Vault::open(&path, c.clone()).unwrap();
            assert_eq!(v.generation(), generation);
            assert!(v.generation_committed() && !v.outside_change());
            v.mark_dirty();
            v.save().unwrap();
            assert_eq!(record_value(&path, c).unwrap(), record);
        }
        // Switching to another form of the same key keeps the generation and its commit.
        let mut v = Vault::open(&path, keyx).unwrap();
        v.set_credentials(raw);
        assert_eq!(v.generation(), generation);
        assert!(v.generation_committed());
        drop(v);
        clear_readonly_and_remove(&dir);
    }

    #[test]
    fn generation_is_provisional_until_the_main_rename_commits() {
        let dir = temp_dir();
        let path = dir.join("commit.kdbx");
        let old = creds("pw", None);
        let new = creds("pw", Some([9; 32]));
        new_db(&path, &old);
        let config = store_config(&dir);
        let mut v = open_stored(&path, &old, &config);
        v.mark_dirty();
        v.save().unwrap();
        let g1 = v.generation().to_owned();

        // A key change starts a new, provisional generation; rolling it back before the save
        // restores the committed one.
        v.set_credentials(new.clone());
        assert_ne!(v.generation(), g1);
        assert!(!v.generation_committed());
        v.set_credentials(old.clone());
        assert_eq!(v.generation(), g1);
        assert!(v.generation_committed());

        // A save that fails before the rename (no backup folder) leaves it provisional and the
        // file on g1.
        v.set_credentials(new.clone());
        let g2 = v.generation().to_owned();
        assert_ne!(g2, g1);
        let blocker = dir.join("Backups.blocked");
        fs::rename(dir.join("Backups"), &blocker).unwrap();
        fs::write(dir.join("Backups"), b"in the way").unwrap();
        assert!(v.save().is_err());
        assert!(!v.generation_committed());
        assert_eq!(record_generation(&path, &old), Some(g1.clone()));
        fs::remove_file(dir.join("Backups")).unwrap();
        fs::rename(&blocker, dir.join("Backups")).unwrap();

        v.save().unwrap();
        assert_eq!(v.generation(), g2);
        assert!(v.generation_committed());
        assert_eq!(record_generation(&path, &new), Some(g2.clone()));
        // The previous version, re-encrypted to the new key, carries the new generation too.
        let copies = store_backups(&config, &path, &v);
        let copy = of_kind(&copies, Kind::BeforeKeyChange)[0];
        let previous = Vault::open(&copy.path, new.clone()).unwrap();
        assert_eq!(previous.generation(), g2);
        assert!(previous.generation_committed() && !previous.outside_change());
        assert_eq!(previous.db.num_entries(), 1);
        drop(previous);

        // Recovery keeps the generation and its commit.
        v.mark_dirty();
        let sealed = v.seal().unwrap();
        drop(v);
        let recovered = Vault::open_sealed(&sealed, new).unwrap();
        assert_eq!(recovered.generation(), g2);
        assert!(recovered.generation_committed() && !recovered.outside_change());
        drop(recovered);
        clear_readonly_and_remove(&dir);
    }

    /// The record is ordinary KDBX 4.1 Meta CustomData (Key, Value, LastModificationTime), which
    /// KeePass and other apps keep without interpreting it. Other items are untouched, and a
    /// save by an app that only keeps CustomData keeps the generation.
    #[test]
    fn credential_record_round_trips_as_keepass_custom_data() {
        let dir = temp_dir();
        let path = dir.join("meta.kdbx");
        let c = creds("pw", Some([5; 32]));
        let mut db = Database::new();
        db.config.kdf_config = KdfConfig::Aes { rounds: 1000 };
        db.meta.custom_data.insert(
            "Plugin.Setting".into(),
            CustomDataItem {
                value: Some(CustomDataValue::String("a value; with spaces".into())),
                last_modification_time: Some(Times::now()),
            },
        );
        db.save(&mut fs::File::create(&path).unwrap(), c.to_key().unwrap())
            .unwrap();

        let mut v = Vault::open(&path, c.clone()).unwrap();
        v.mark_dirty();
        v.save().unwrap();
        let generation = v.generation().to_owned();
        drop(v);

        let data = fs::read(&path).unwrap();
        let xml =
            String::from_utf8(Database::get_xml(&mut &data[..], c.to_key().unwrap()).unwrap())
                .unwrap();
        let meta = &xml[xml.find("<Meta>").unwrap()..xml.find("</Meta>").unwrap()];
        let custom = &meta[meta.find("<CustomData>").unwrap()..];
        let item_of = |key: &str| {
            let start = custom.find(&format!("<Key>{key}</Key>")).unwrap();
            &custom[start..start + custom[start..].find("</Item>").unwrap()]
        };
        let ours = item_of(generation::RECORD_KEY);
        assert!(ours.contains(&format!("<Value>v1;{generation};")), "{ours}");
        assert!(ours.contains("<LastModificationTime>"), "{ours}");
        assert!(item_of("Plugin.Setting").contains("<Value>a value; with spaces</Value>"));

        // Another writer keeps everything, the record included.
        let report = crate::fidelity::check(&data, c.to_key().unwrap()).unwrap();
        assert!(report.is_clean(), "{:?}", report.differences);
        rekey_elsewhere(&path, &c, &c);
        let v = Vault::open(&path, c).unwrap();
        assert_eq!(v.generation(), generation);
        assert!(v.generation_committed() && !v.outside_change());
        drop(v);
        fs::remove_dir_all(dir).ok();
    }

    /// Nothing derived from key material leaves the encrypted database: not the password or its
    /// hash, the raw key, the key file's hash, the composite key or the check. The store only
    /// knows random generations. Goes through what the store holds around an unlock, a save and
    /// a key change (the `opened` copy, the previous versions, the generations, the settings)
    /// and searches every file and name.
    #[test]
    fn no_key_material_outside_kdbx() {
        let dir = temp_dir();
        fs::create_dir_all(dir.join("data")).unwrap();
        let path = dir.join("data").join("Secrets.kdbx");
        let password = "correct horse battery staple";
        let old_key: [u8; 32] =
            core::array::from_fn(|i| (i as u8).wrapping_mul(13).wrapping_add(7));
        let new_key: [u8; 32] =
            core::array::from_fn(|i| (i as u8).wrapping_mul(17).wrapping_add(1));
        let keyx = crate::keyfile::build_v2(&old_key).into_bytes();
        let old = with_key_file(password, keyx.clone());
        let new = with_key_file(password, new_key.to_vec());
        new_db(&path, &old);

        let mut config = store_config(&dir);
        config.settings_file = Some(dir.join(backups::SETTINGS_FILE));
        let (mut v, opened) = Vault::open_read(&path, old.clone(), KdfBudget::DEFAULT).unwrap();
        v.attach_store(config.clone());
        v.snapshot_opened(&opened).unwrap();
        v.mark_dirty();
        v.save().unwrap();
        let old_record = record_value(&path, &old).unwrap();
        v.set_credentials(new.clone());
        v.save().unwrap();
        let new_record = record_value(&path, &new).unwrap();
        let copies = store_backups(&config, &path, &v);
        assert_eq!(
            copies.len(),
            2,
            "the `opened` and `before-key-change` copies"
        );
        let index = fs::read_to_string(copies[0].path.with_file_name(backups::INDEX_FILE)).unwrap();
        assert!(index.contains(v.generation()), "the index labels copies");
        assert!(dir.join(backups::SETTINGS_FILE).is_file());

        let check_of = |record: &str| record.rsplit(';').next().unwrap().to_owned();
        let digest = |bytes: &[u8]| Sha256::digest(bytes).to_vec();
        let mut secrets: Vec<Vec<u8>> = vec![
            digest(password.as_bytes()),
            old_key.to_vec(),
            new_key.to_vec(),
            digest(&keyx),
            generation::composite_key(&old).to_vec(),
            generation::composite_key(&new).to_vec(),
            hex::decode(check_of(&old_record)).unwrap(),
            hex::decode(check_of(&new_record)).unwrap(),
        ];
        // Each as raw bytes and as hex in either case; the password as itself.
        let mut needles: Vec<Vec<u8>> = vec![password.as_bytes().to_vec()];
        for s in secrets.drain(..) {
            needles.push(hex::encode(&s).into_bytes());
            needles.push(hex::encode_upper(&s).into_bytes());
            needles.push(s);
        }
        let contains = |hay: &[u8], needle: &[u8]| hay.windows(needle.len()).any(|w| w == needle);
        let mut files = 0;
        let mut stack = vec![dir.clone()];
        while let Some(d) = stack.pop() {
            for entry in fs::read_dir(&d).unwrap().flatten() {
                let p = entry.path();
                let name = entry.file_name().to_string_lossy().into_owned();
                for n in &needles {
                    assert!(
                        !contains(name.as_bytes(), n),
                        "key material in the name {name}"
                    );
                }
                if p.is_dir() {
                    stack.push(p);
                    continue;
                }
                files += 1;
                let bytes = fs::read(&p).unwrap();
                for n in &needles {
                    assert!(!contains(&bytes, n), "key material in {}", p.display());
                }
            }
        }
        // The database, two copies (the `auto` one equals the `opened` one, so it is skipped),
        // vault.json, index.json and backups.json; no `.bak`.
        assert_eq!(files, 6, "files searched");
        drop(v);
        clear_readonly_and_remove(&dir);
    }

    /// A backup key file may not take a name the database's own files use.
    #[test]
    fn database_files_are_recognised_by_name() {
        let dir = temp_dir();
        let db = dir.join("Vault.kdbx");
        for own in [
            "Vault.kdbx",
            "vault.KDBX",
            "Vault.kdbx.bak",
            "Vault.kdbx.bak.tmp",
            "Vault.kdbx.tmp",
            "Vault.kdbx.axvault",
            "Vault.kdbx.axvault.tmp",
            "Vault.kdbx.axvault.old",
            "Vault.kdbx.axvault.~new",
            "Vault.kdbx.~axv-0123456789abcdef.tmp",
            "Vault.kdbx.axvault.~axv-0123456789abcdef.tmp",
            "Vault.kdbx.bak.~axv-0123456789abcdef.tmp",
        ] {
            assert!(is_database_file(&db, &dir.join(own)), "{own}");
        }
        // Through another spelling of the same folder.
        let sub = dir.join("sub");
        fs::create_dir(&sub).unwrap();
        assert!(is_database_file(
            &db,
            &sub.join("..").join("Vault.kdbx.bak")
        ));
        for other in [
            "Vault-backup 2026-10-14.keyx",
            "Vault.kdbx.keyx",
            "Vault.kdbx.~axv-hello.tmp",
            "Other.kdbx.bak",
        ] {
            assert!(!is_database_file(&db, &dir.join(other)), "{other}");
        }
        assert!(!is_database_file(&db, &sub.join("Vault.kdbx.bak")));
        fs::remove_dir_all(dir).ok();
    }

    fn clear_readonly_and_remove(dir: &Path) {
        fn clear(dir: &Path) {
            for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if entry.file_type().is_ok_and(|t| t.is_dir()) {
                    clear(&path);
                } else {
                    let _ = crate::fsops::set_readonly(&path, false);
                }
            }
        }
        clear(dir);
        let _ = fs::remove_dir_all(dir);
    }
}
