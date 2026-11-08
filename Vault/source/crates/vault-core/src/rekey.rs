//! Re-encrypting the backup store during a key change (first passkey setup or a revocation).
//!
//! Once the database commits to a new key, the old one is gone for good, so every backup is
//! re-encrypted before that commit, while both keys are known ([`RekeyPlan::run`], on a worker):
//! 0. An earlier key change's manifest still in a folder is finished first (or thrown away, see
//!    [`recover`]), and the old-key originals it couldn't delete then go.
//! 1. Copies of the database beside it (`<db>.bak`, and the `.tmp` files earlier versions or an
//!    interrupted save left; see [`database_copies_beside`]) are copied into the store first
//!    (`imported`) and so re-keyed along with the rest; the files beside it go only after the
//!    commit, and the manifest records them until they have.
//! 2. Every database backup in every reachable known folder whose generation isn't the new one
//!    is read (capped) and checked before any key derivation: KDBX 4.1 (the only version this
//!    app's writer saves), and key derivation within the budget and at most twice the cost of
//!    the database's own.
//! 3. It is opened with the old credentials, or, if they had a password and a key file and
//!    don't open it, with the password alone (copies from before the key file was added).
//! 4. It is saved under the new credentials, with the database's current KDF settings and the
//!    new credential record, to a temp file in its folder.
//! 5. The two are compared ([`fidelity::compare`]): a backup this writer can't save without
//!    losing something is left as it was.
//! 6. The temp is listed in the folder's `rekey.json` with both hashes.
//!
//! A backup that fails any step keeps its old credentials and is marked `failed: <why>` once the
//! change commits. Cancelling (checked before every key derivation) deletes the temps, the
//! manifests and the copies made in step 1: nothing has changed. After the commit
//! [`Staged::publish`] deletes the copies beside the database, moves each staged copy to its
//! original's name with the next revision (`~n`) and deletes the original. If the app stops in
//! between, the next unlock finishes that publish when the database carries the manifest's
//! generation and throws the copies away otherwise ([`recover`]). About four key derivations
//! per backup.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use keepass::Database;
use keepass::DatabaseKey;
use keepass::config::KdfConfig;
use keepass::db::DatabaseOpenError;
use keepass::error::DatabaseKeyError;
use zeroize::Zeroizing;

use crate::backups::{
    self, Backup, BesideCopy, Folder, Kind, RekeyFailure, RekeyItem, RekeyManifest, RekeyPublished,
    StagingGuard,
};
use crate::envelope;
use crate::fidelity;
use crate::fsops;
use crate::generation::{self, CredentialRecord};
use crate::header::{self, HeaderError, Kdf, KdfBudget};
use crate::limits::{DATABASE_MAX, read_capped};
use crate::store::Store;
use crate::vault::{Credentials, with_suffix};

/// How far staging has got: backups done of the total (0 until they are counted).
#[derive(Debug, Default)]
pub struct Progress {
    done: AtomicUsize,
    total: AtomicUsize,
}

impl Progress {
    pub fn done(&self) -> usize {
        self.done.load(Ordering::Relaxed)
    }

    pub fn total(&self) -> usize {
        self.total.load(Ordering::Relaxed)
    }
}

/// Why a backup kept its old credentials.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    /// This app's writer would lose something in it (written by another app or plugin).
    Lossy,
    /// Not KDBX 4.1, which is all this app's writer saves.
    Format,
    /// Its key derivation is over the budget or more than twice the database's.
    Kdf,
    /// The old credentials (or the password alone) don't open it, or it can't be read.
    Open,
}

impl Failure {
    /// The index's `rekey` value.
    pub fn label(self) -> &'static str {
        match self {
            Failure::Lossy => "failed: lossy",
            Failure::Format => "failed: format",
            Failure::Kdf => "failed: kdf",
            Failure::Open => "failed: open",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StageError {
    #[error("the key change was cancelled")]
    Cancelled,
    #[error("{0}")]
    Io(#[from] io::Error),
}

/// Everything staging needs, taken from the open vault (see `Vault::plan_key_change`), to run on
/// a worker thread.
pub struct RekeyPlan {
    pub(crate) store: Store,
    pub(crate) db_path: PathBuf,
    pub(crate) vault_id: String,
    pub(crate) old: Credentials,
    pub(crate) new: Credentials,
    /// The new generation's record, written into every re-encrypted copy and then into the
    /// database.
    pub(crate) record: CredentialRecord,
    pub(crate) kdf_config: KdfConfig,
    /// The database's own key derivation; a backup may ask for at most twice its cost.
    pub(crate) current_kdf: Option<Kdf>,
    pub(crate) budget: KdfBudget,
    /// The session's generation if the database carries it: an earlier key change's manifest
    /// for it is published before staging, any other is thrown away.
    pub(crate) committed: Option<String>,
}

/// What the per-backup steps share.
struct Context {
    old: Credentials,
    /// The old password alone, when the old credentials also had a key file.
    password_only: Option<Credentials>,
    new_key: DatabaseKey,
    record: CredentialRecord,
    kdf_config: KdfConfig,
    max_cost: Option<u128>,
    budget: KdfBudget,
}

impl RekeyPlan {
    /// The generation the copies are staged under.
    pub fn generation(&self) -> &str {
        self.record.generation()
    }

    /// Stages every backup (see the module docs). `cancel` is checked before every key
    /// derivation; once it is set, everything staged so far is deleted and `Cancelled` returned.
    /// Dropping the result without publishing it deletes it the same way.
    pub fn run(self, cancel: &AtomicBool, progress: &Progress) -> Result<Staged, StageError> {
        let RekeyPlan {
            mut store,
            db_path,
            vault_id,
            old,
            new,
            record,
            kdf_config,
            current_kdf,
            budget,
            committed,
        } = self;
        let new_key = new.to_key().map_err(io::Error::other)?;
        let mut staged = Staged {
            to_gen: record.generation().to_owned(),
            record: record.clone(),
            db_path: db_path.clone(),
            old: old.clone(),
            new,
            folders: Vec::new(),
            imported: Vec::new(),
            unreachable: Vec::new(),
            kdf_runs: 0,
            armed: true,
        };
        let (folders, unreachable) = store.existing_folders(&db_path, &vault_id);
        staged.unreachable = unreachable.into_iter().map(|(root, _)| root.path).collect();

        // 0. This change's manifest must not replace an earlier one's record.
        recover(&folders, committed.as_deref(), &db_path);
        for folder in &folders {
            let _ = folder.delete_superseded();
        }
        for folder in folders {
            staged.add_folder(folder);
        }

        // 1.
        let mut moved: Option<(Folder, Vec<String>)> = None;
        for (path, suffix) in database_copies_beside(&db_path) {
            let bytes = read_capped(&path, DATABASE_MAX)?;
            let sha256 = backups::sha256_hex(&bytes);
            if moved.is_none() {
                let folder = store
                    .writable_folder(&db_path, &vault_id, bytes.len() as u64)
                    .map_err(io::Error::other)?;
                staged.add_folder(folder.clone());
                moved = Some((folder, Vec::new()));
            }
            let (folder, copies) = moved.as_mut().expect("just set");
            // Two files with the same contents need one copy.
            if !copies.contains(&sha256) {
                let mtime = fs::metadata(&path).and_then(|m| m.modified()).ok();
                let copy = folder.write_backup(&bytes, Kind::Imported, mtime, None)?;
                let copy = copy.path().to_owned();
                staged.imported.push((folder.clone(), copy.clone()));
                if backups::sha256_file(&copy)? != sha256 {
                    return Err(io::Error::other(format!(
                        "the copy of {} didn't read back the same",
                        path.display()
                    ))
                    .into());
                }
                copies.push(sha256.clone());
            }
            let folder = folder.clone();
            staged
                .manifest_of(&folder)
                .beside
                .push(BesideCopy { suffix, sha256 });
            folder.write_manifest(staged.manifest_of(&folder))?;
        }

        // 2.–6.
        let mut work: Vec<(usize, Backup)> = Vec::new();
        for (i, f) in staged.folders.iter().enumerate() {
            for backup in f.folder.list()? {
                if backup.generation.as_deref() != Some(staged.to_gen.as_str()) {
                    work.push((i, backup));
                }
            }
        }
        progress.total.store(work.len(), Ordering::Relaxed);
        let password_only = match (&old.password, &old.key_file) {
            (Some(password), Some(_)) => Some(Credentials {
                password: Some(password.clone()),
                key_file: None,
            }),
            _ => None,
        };
        let max_cost = current_kdf.map(|k| k.cost().saturating_mul(2));
        let context = Context {
            old,
            password_only,
            new_key,
            record,
            kdf_config,
            max_cost,
            budget,
        };
        for (i, backup) in work {
            let folder = staged.folders[i].folder.clone();
            let outcome = stage_one(&folder, &backup, &context, cancel, &mut staged.kdf_runs)?;
            let manifest = &mut staged.folders[i].manifest;
            match outcome {
                Ok(item) => manifest.items.push(item),
                Err(failure) => manifest.failed.push(RekeyFailure {
                    original: backup.file_name.clone(),
                    why: failure.label().to_owned(),
                }),
            }
            folder.write_manifest(manifest)?;
            progress.done.fetch_add(1, Ordering::Relaxed);
        }
        Ok(staged)
    }
}

/// Counts a key derivation about to run, unless the change was cancelled.
fn before_kdf(cancel: &AtomicBool, runs: &mut usize) -> Result<(), StageError> {
    if cancel.load(Ordering::SeqCst) {
        return Err(StageError::Cancelled);
    }
    *runs += 1;
    Ok(())
}

fn wrong_key(e: &DatabaseOpenError) -> bool {
    matches!(e, DatabaseOpenError::Key(DatabaseKeyError::IncorrectKey))
}

/// Steps 2–5 for one backup, writing its temp file: the manifest item, or why it stays as it is.
fn stage_one(
    folder: &Folder,
    backup: &Backup,
    cx: &Context,
    cancel: &AtomicBool,
    runs: &mut usize,
) -> Result<Result<RekeyItem, Failure>, StageError> {
    let Ok(data) = read_capped(&backup.path, DATABASE_MAX) else {
        return Ok(Err(Failure::Open));
    };
    // 2. Nothing is derived for a file the writer can't save or that asks for too much.
    if header::version(&data) != Some((4, 1)) {
        return Ok(Err(Failure::Format));
    }
    let kdf = match header::parse_kdf(&data) {
        Ok(kdf) => kdf,
        Err(HeaderError::UnknownKdf) => return Ok(Err(Failure::Kdf)),
        Err(_) => return Ok(Err(Failure::Format)),
    };
    if !cx.budget.allows(&kdf) || cx.max_cost.is_some_and(|max| kdf.cost() > max) {
        return Ok(Err(Failure::Kdf));
    }

    // 3.
    let Ok(old_key) = cx.old.to_key() else {
        return Ok(Err(Failure::Open));
    };
    before_kdf(cancel, runs)?;
    let (mut db, opened_with) = match Database::parse(&data, old_key.clone()) {
        Ok(db) => (db, old_key),
        Err(e) if wrong_key(&e) && cx.password_only.is_some() => {
            let Some(Ok(key)) = cx.password_only.as_ref().map(Credentials::to_key) else {
                return Ok(Err(Failure::Open));
            };
            before_kdf(cancel, runs)?;
            match Database::parse(&data, key.clone()) {
                Ok(db) => (db, key),
                Err(_) => return Ok(Err(Failure::Open)),
            }
        }
        Err(_) => return Ok(Err(Failure::Open)),
    };

    // 4.
    cx.record.write_to(&mut db);
    db.config.kdf_config = cx.kdf_config.clone();
    let mut copy = Zeroizing::new(Vec::new());
    let saved = before_kdf(cancel, runs).map(|()| db.save(&mut *copy, cx.new_key.clone()));
    generation::wipe_record(&mut db);
    drop(db);
    if saved?.is_err() {
        return Ok(Err(Failure::Format));
    }

    // 5.
    before_kdf(cancel, runs)?;
    let Ok(before) = Database::get_xml(&mut &data[..], opened_with) else {
        return Ok(Err(Failure::Lossy));
    };
    let before = Zeroizing::new(before);
    before_kdf(cancel, runs)?;
    let Ok(after) = Database::get_xml(&mut &copy[..], cx.new_key.clone()) else {
        return Ok(Err(Failure::Lossy));
    };
    let after = Zeroizing::new(after);
    if !fidelity::compare(&before, &after).is_ok_and(|r| r.is_clean()) {
        return Ok(Err(Failure::Lossy));
    }

    // 6. (The caller lists it.)
    let tmp = fsops::write_temp_in(folder.path(), &copy)?;
    Ok(Ok(RekeyItem {
        tmp: tmp
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        original: backup.file_name.clone(),
        original_sha256: backups::sha256_hex(&data),
        tmp_sha256: backups::sha256_hex(&copy),
    }))
}

/// One folder's share of a staged key change. Pruning stays off it until the change is published
/// or thrown away.
struct StagedFolder {
    folder: Folder,
    manifest: RekeyManifest,
    _guard: StagingGuard,
}

/// A key change's re-encrypted backups, staged and not yet published. Dropped without
/// [`Staged::publish`], it deletes them again (and the copies of files beside the database):
/// nothing has changed.
pub struct Staged {
    pub(crate) to_gen: String,
    pub(crate) record: CredentialRecord,
    pub(crate) db_path: PathBuf,
    pub(crate) old: Credentials,
    pub(crate) new: Credentials,
    folders: Vec<StagedFolder>,
    /// Copies of files beside the database written into the store (step 1).
    imported: Vec<(Folder, PathBuf)>,
    unreachable: Vec<PathBuf>,
    kdf_runs: usize,
    armed: bool,
}

/// What publishing a staged key change did.
#[derive(Debug, Default)]
pub(crate) struct Published {
    /// Old-key originals another program held open; deleted at a later unlock or prune.
    pub(crate) held: usize,
    /// Copies of the database beside it, still there on the old credentials (held open); the
    /// next unlock tries again.
    pub(crate) left_beside: Vec<PathBuf>,
}

impl Staged {
    fn add_folder(&mut self, folder: Folder) {
        if self
            .folders
            .iter()
            .any(|f| f.folder.path() == folder.path())
        {
            return;
        }
        self.folders.push(StagedFolder {
            _guard: folder.begin_staging(),
            manifest: RekeyManifest::new(&self.to_gen),
            folder,
        });
    }

    /// The manifest of a folder added with [`Staged::add_folder`].
    fn manifest_of(&mut self, folder: &Folder) -> &mut RekeyManifest {
        let i = self
            .folders
            .iter()
            .position(|f| f.folder.path() == folder.path())
            .expect("the folder was added");
        &mut self.folders[i].manifest
    }

    /// The new credentials' generation.
    pub fn generation(&self) -> &str {
        &self.to_gen
    }

    /// Backups staged, re-encrypted to the new credentials.
    pub fn rekeyed(&self) -> usize {
        self.folders.iter().map(|f| f.manifest.items.len()).sum()
    }

    /// Backups that will keep their old credentials.
    pub fn failed(&self) -> usize {
        self.folders.iter().map(|f| f.manifest.failed.len()).sum()
    }

    /// Roots that didn't answer: their backups keep the old credentials.
    pub fn unreachable(&self) -> &[PathBuf] {
        &self.unreachable
    }

    /// Key derivations run (or started) while staging.
    pub fn kdf_runs(&self) -> usize {
        self.kdf_runs
    }

    /// Deletes everything staged; see the type docs.
    pub fn discard(mut self) {
        self.discard_now();
    }

    fn discard_now(&mut self) {
        if !std::mem::take(&mut self.armed) {
            return;
        }
        for f in &self.folders {
            let _ = f.folder.discard_rekey(&f.manifest);
        }
        for (folder, copy) in std::mem::take(&mut self.imported) {
            if let Some(name) = copy.file_name().and_then(|n| n.to_str()) {
                let _ = folder.delete(&[name]);
            }
        }
    }

    /// Test support: what the app stopping leaves behind: the files stay, the guards go.
    #[cfg(test)]
    pub(crate) fn abandon(mut self) {
        self.armed = false;
    }

    /// Publishes every folder's copies (see [`finish`]); for after the database has committed
    /// to the new credentials. A folder that fails keeps its manifest for the next unlock.
    pub(crate) fn publish(mut self) -> Published {
        self.armed = false;
        let mut out = Published::default();
        for f in &self.folders {
            let (report, left) = finish(&f.folder, &f.manifest, &self.db_path);
            out.held += report.map_or(0, |r| r.held.len());
            out.left_beside.extend(left);
        }
        out
    }
}

/// Publishes one folder's manifest of a committed key change: first the copies of the database
/// beside it that the manifest moved into the store are deleted (each only while it still holds
/// what was moved), then [`Folder::publish_rekey`]. Copies that couldn't be deleted (held open)
/// stay listed in a manifest, so the next unlock tries again. Returns the publish report and the
/// copies still there.
fn finish(
    folder: &Folder,
    manifest: &RekeyManifest,
    db_path: &Path,
) -> (io::Result<RekeyPublished>, Vec<PathBuf>) {
    let mut left = Vec::new();
    for copy in manifest.beside.iter().filter(|c| c.valid()) {
        let path = with_suffix(db_path, &copy.suffix);
        let ours = backups::sha256_file(&path).is_ok_and(|h| h.eq_ignore_ascii_case(&copy.sha256));
        if ours && !matches!(fsops::remove_file(&path), Ok(true)) && path.exists() {
            left.push(copy.clone());
        }
    }
    let report = folder.publish_rekey(manifest);
    if !left.is_empty() && report.as_ref().is_ok_and(|r| !r.pending) {
        // Published: what remains to do is these deletions.
        let _ = folder.write_manifest(&RekeyManifest {
            beside: left.clone(),
            ..RekeyManifest::new(&manifest.to_gen)
        });
    }
    let left = left
        .iter()
        .map(|c| with_suffix(db_path, &c.suffix))
        .collect();
    (report, left)
}

/// Copies of the database beside it that a key change moves into the store, each with its
/// suffix (see [`backups::is_copy_suffix`]): `<db>.bak`, `<db>.tmp` and `<db>.bak.tmp` from
/// earlier versions, and the temps an interrupted save or fallback `.bak` left, each holding a
/// KeePass file (at least 512 bytes, with the KDBX signature). Links are not followed.
pub(crate) fn database_copies_beside(db_path: &Path) -> Vec<(PathBuf, String)> {
    let (Some(dir), Some(db_name)) = (db_path.parent(), db_path.file_name()) else {
        return Vec::new();
    };
    let db_name = db_name.to_string_lossy().to_lowercase();
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<(PathBuf, String)> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            let suffix = name.strip_prefix(&db_name)?;
            let meta = fs::symlink_metadata(entry.path()).ok()?;
            (backups::is_copy_suffix(suffix)
                && meta.is_file()
                && !fsops::is_reparse_point(&meta)
                && backups::looks_like_kdbx(&entry.path()))
            .then(|| (entry.path(), suffix.to_owned()))
        })
        .collect();
    found.sort();
    found
}

/// Deletes the passkey files earlier versions or an interrupted ring write left beside the
/// database: `<db>.axvault.old`, `<db>.axvault.tmp` and `<db>.axvault.~axv-<hex>.tmp`. They
/// may wrap a key that was just replaced. Returns those that couldn't be deleted.
pub(crate) fn remove_old_rings(db_path: &Path) -> Vec<PathBuf> {
    let ring = envelope::path_for(db_path);
    let (Some(dir), Some(ring_name)) = (ring.parent(), ring.file_name()) else {
        return Vec::new();
    };
    let ring_name = ring_name.to_string_lossy().to_lowercase();
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut left = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        let old = name
            .strip_prefix(&ring_name)
            .is_some_and(|suffix| matches!(suffix, ".old" | ".tmp") || fsops::is_temp_name(suffix));
        let path = entry.path();
        if old && !matches!(fsops::remove_file(&path), Ok(true)) && path.exists() {
            left.push(path);
        }
    }
    left
}

impl Drop for Staged {
    fn drop(&mut self) {
        self.discard_now();
    }
}

/// What unlock-time recovery did with interrupted key changes.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Recovered {
    /// Folders whose staged copies were published: the database carries their generation.
    pub published: Vec<PathBuf>,
    /// Folders whose staged copies were thrown away: the key change never committed.
    pub discarded: Vec<PathBuf>,
}

/// Recovery of interrupted key changes, at unlock and before a key change stages: in each folder
/// with a `rekey.json`, the staged copies are published if `committed` (the database's committed
/// generation) is the manifest's (see [`finish`]: the copies beside the database it moved in go
/// first; the manifest's hashes are used and no key derivation runs), and ring snapshots of
/// other generations and old passkey files beside the database are deleted with them; otherwise
/// they are deleted. A folder being staged right now, or with a manifest that can't be read, is
/// left alone.
pub(crate) fn recover(folders: &[Folder], committed: Option<&str>, db_path: &Path) -> Recovered {
    let mut done = Recovered::default();
    for folder in folders {
        if folder.staging_active() {
            continue;
        }
        let Ok(Some(manifest)) = folder.read_manifest() else {
            continue;
        };
        if committed == Some(manifest.to_gen.as_str()) {
            if finish(folder, &manifest, db_path).0.is_ok() {
                let _ = folder.delete_rings_except(&manifest.to_gen);
                remove_old_rings(db_path);
                done.published.push(folder.path().to_owned());
            }
        } else if folder.discard_rekey(&manifest).is_ok() {
            done.discarded.push(folder.path().to_owned());
        }
    }
    done
}

/// Whether `path` is the database's own leftover save temp, `<db>.~axv-<16 hex>.tmp`.
pub(crate) fn is_leftover_temp(db_path: &Path, path: &Path) -> bool {
    let (Some(db_name), Some(name)) = (db_path.file_name(), path.file_name()) else {
        return false;
    };
    let (db_name, name) = (
        db_name.to_string_lossy().to_lowercase(),
        name.to_string_lossy().to_lowercase(),
    );
    path.parent() == db_path.parent()
        && name.strip_prefix(&db_name).is_some_and(fsops::is_temp_name)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use std::time::SystemTime;

    use keepass::db::{CustomDataItem, CustomDataValue, Times, fields};

    use super::*;
    use crate::backups::{BackupConfig, BackupName, BackupSettings, Item};
    use crate::envelope::{Envelope, KeyRing, RingKey};
    use crate::random_bytes;
    use crate::vault::{RecoveryReport, Vault};

    pub(crate) fn temp_dir() -> PathBuf {
        let mut n = [0u8; 8];
        random_bytes(&mut n);
        let dir = std::env::temp_dir().join(format!("axvault-rekey-{}", hex::encode(n)));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Clears read-only everywhere first: backups are read-only.
    pub(crate) fn cleanup(dir: &Path) {
        fn clear(dir: &Path) {
            for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if entry.file_type().is_ok_and(|t| t.is_dir()) {
                    clear(&path);
                } else {
                    let _ = fsops::set_readonly(&path, false);
                }
            }
        }
        clear(dir);
        let _ = fs::remove_dir_all(dir);
    }

    pub(crate) fn creds(password: &str, key: Option<[u8; 32]>) -> Credentials {
        Credentials {
            password: Some(Zeroizing::new(password.to_owned())),
            key_file: key.map(|k| Zeroizing::new(k.to_vec())),
        }
    }

    /// A database with one entry, a fast KDF and no compression (so tests can rewrite its XML).
    fn db_bytes(c: &Credentials, title: &str) -> Vec<u8> {
        let mut db = Database::new();
        db.config.kdf_config = KdfConfig::Aes { rounds: 1000 };
        db.config.compression_config = keepass::config::CompressionConfig::None;
        db.root_mut().add_entry().edit(|e| {
            e.set_unprotected(fields::TITLE, title);
            e.set_protected(fields::PASSWORD, "hunter2");
        });
        let mut out = Vec::new();
        db.save(&mut out, c.to_key().unwrap()).unwrap();
        out
    }

    fn store_config(dir: &Path) -> BackupConfig {
        BackupConfig::new(dir.join("Backups")).with_free_space(|_| None)
    }

    /// A vault at `<dir>\data\v.kdbx` with `saves` saves behind it, so its store holds that many
    /// backups (the first is the file as created).
    fn vault_with_backups(dir: &Path, c: &Credentials, saves: usize) -> Vault {
        let path = dir.join("data").join("v.kdbx");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, db_bytes(c, "Bank")).unwrap();
        let mut v = Vault::open(&path, c.clone()).unwrap();
        v.attach_store(store_config(dir));
        for i in 0..saves {
            v.db.root_mut()
                .add_entry()
                .set_unprotected(fields::TITLE, format!("entry {i}"));
            v.mark_dirty();
            v.save().unwrap();
        }
        v
    }

    fn folder_of(v: &Vault) -> Folder {
        let config = v.backup_config().unwrap();
        config
            .open_folder(
                &config.home(),
                v.path(),
                &backups::vault_id_of(&v.db),
                &mut BackupSettings::default(),
                false,
            )
            .unwrap()
            .unwrap()
    }

    /// Puts `bytes` into the folder as a backup another run (or app) left, after every other.
    fn put(folder: &Folder, bytes: &[u8], kind: Kind) -> String {
        let seq = folder
            .list_all()
            .unwrap()
            .iter()
            .map(|b| b.name.seq)
            .max()
            .unwrap_or(0)
            + 1;
        let name = BackupName {
            stem: folder.stem().to_owned(),
            date: "2026-01-02".into(),
            time: "03-04-05".into(),
            seq,
            rev: 0,
            item: Item::Kdbx(kind),
        }
        .file_name();
        fs::write(folder.path().join(&name), bytes).unwrap();
        name
    }

    /// Every file under `dir` and its SHA-256.
    fn hashes(dir: &Path) -> BTreeMap<PathBuf, String> {
        let mut out = BTreeMap::new();
        let mut stack = vec![dir.to_owned()];
        while let Some(d) = stack.pop() {
            for entry in fs::read_dir(&d).unwrap().flatten() {
                let p = entry.path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    out.insert(p.clone(), backups::sha256_file(&p).unwrap());
                }
            }
        }
        out
    }

    fn ring(key: &[u8; 32]) -> KeyRing {
        KeyRing {
            keys: vec![RingKey {
                label: "Phone passkey".into(),
                envelope: Envelope::seal("localhost", &[1], [2; 32], &[3; 32], key),
            }],
        }
    }

    fn stage(v: &mut Vault, new: &Credentials) -> Staged {
        v.plan_key_change(new.clone())
            .unwrap()
            .run(&AtomicBool::new(false), &Progress::default())
            .unwrap()
    }

    /// Test support: `kdbx` (saved by keepass with no compression, AES-256 and AES-KDF) with
    /// `extra` put into its XML right after `<Meta>`, encrypted again under the same key: a file
    /// another app wrote with an element this app's writer doesn't keep.
    pub(crate) fn with_unknown_element(kdbx: &[u8], c: &Credentials, extra: &str) -> Vec<u8> {
        use aes::cipher::{
            BlockCipherEncrypt, BlockModeDecrypt, BlockModeEncrypt, KeyInit, KeyIvInit,
            block_padding::Pkcs7,
        };
        use hmac::{Hmac, Mac};
        use sha2::{Digest, Sha256, Sha512};

        let u32_at = |b: &[u8], at: usize| u32::from_le_bytes(b[at..at + 4].try_into().unwrap());
        let (mut master_seed, mut iv, mut kdf_seed, mut rounds) =
            (Vec::new(), Vec::new(), Vec::new(), 0);
        let mut pos = 12;
        loop {
            let (id, len) = (kdbx[pos], u32_at(kdbx, pos + 1) as usize);
            let data = &kdbx[pos + 5..pos + 5 + len];
            pos += 5 + len;
            match id {
                0 => break,
                4 => master_seed = data.to_vec(),
                7 => iv = data.to_vec(),
                11 => {
                    let mut at = 2;
                    while data[at] != 0 {
                        let klen = u32_at(data, at + 1) as usize;
                        let key = &data[at + 5..at + 5 + klen];
                        let vlen = u32_at(data, at + 5 + klen) as usize;
                        let value = &data[at + 9 + klen..at + 9 + klen + vlen];
                        match key {
                            b"S" => kdf_seed = value.to_vec(),
                            b"R" => rounds = u64::from_le_bytes(value.try_into().unwrap()),
                            _ => {}
                        }
                        at += 9 + klen + vlen;
                    }
                }
                _ => {}
            }
        }
        let blocks_at = pos + 64;

        // AES-KDF, then the master and HMAC keys, as KDBX 4 derives them.
        let composite = generation::composite_key(c);
        let aes = aes::Aes256::new_from_slice(&kdf_seed).unwrap();
        let mut halves: [aes::Block; 2] = [
            composite[..16].try_into().unwrap(),
            composite[16..].try_into().unwrap(),
        ];
        for _ in 0..rounds {
            aes.encrypt_block(&mut halves[0]);
            aes.encrypt_block(&mut halves[1]);
        }
        let transformed = Sha256::new()
            .chain_update(halves[0])
            .chain_update(halves[1])
            .finalize();
        let master_key = Sha256::new()
            .chain_update(&master_seed)
            .chain_update(transformed)
            .finalize();
        let hmac_key = Sha512::new()
            .chain_update(&master_seed)
            .chain_update(transformed)
            .chain_update([1u8])
            .finalize();
        let block_mac = |index: u64, data: &[u8]| {
            let key = Sha512::new()
                .chain_update(index.to_le_bytes())
                .chain_update(hmac_key)
                .finalize();
            let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(&key).unwrap();
            mac.update(&index.to_le_bytes());
            mac.update(&(data.len() as u32).to_le_bytes());
            mac.update(data);
            mac.finalize().into_bytes()
        };

        // The HMAC block stream, decrypted.
        let mut encrypted = Vec::new();
        let mut at = blocks_at;
        loop {
            let size = u32_at(kdbx, at + 32) as usize;
            if size == 0 {
                break;
            }
            encrypted.extend_from_slice(&kdbx[at + 36..at + 36 + size]);
            at += 36 + size;
        }
        let mut plain = encrypted.clone();
        let plain = cbc::Decryptor::<aes::Aes256>::new_from_slices(&master_key, &iv)
            .unwrap()
            .decrypt_padded::<Pkcs7>(&mut plain)
            .unwrap()
            .to_vec();
        // The inner header, then the XML.
        let mut xml_at = 0;
        loop {
            let (id, len) = (plain[xml_at], u32_at(&plain, xml_at + 1) as usize);
            xml_at += 5 + len;
            if id == 0 {
                break;
            }
        }
        let xml = std::str::from_utf8(&plain[xml_at..]).unwrap();
        let meta = xml.find("<Meta>").unwrap() + "<Meta>".len();
        let changed = format!("{}{extra}{}", &xml[..meta], &xml[meta..]);
        let mut payload = plain[..xml_at].to_vec();
        payload.extend_from_slice(changed.as_bytes());

        let mut out_buf = vec![0u8; payload.len() + 16];
        let encrypted = cbc::Encryptor::<aes::Aes256>::new_from_slices(&master_key, &iv)
            .unwrap()
            .encrypt_padded_b2b::<Pkcs7>(&payload, &mut out_buf)
            .unwrap()
            .to_vec();
        let mut out = kdbx[..blocks_at].to_vec();
        out.extend_from_slice(&block_mac(0, &encrypted));
        out.extend_from_slice(&(encrypted.len() as u32).to_le_bytes());
        out.extend_from_slice(&encrypted);
        out.extend_from_slice(&block_mac(1, &[]));
        out.extend_from_slice(&0u32.to_le_bytes());
        out
    }

    #[test]
    fn unknown_elements_open_but_do_not_survive_a_save() {
        let c = creds("pw", Some([1; 32]));
        let plain = db_bytes(&c, "Bank");
        let foreign = with_unknown_element(&plain, &c, "<OtherAppSetting>on</OtherAppSetting>");
        let db = Database::parse(&foreign, c.to_key().unwrap()).unwrap();
        assert_eq!(db.num_entries(), 1);
        let xml = Database::get_xml(&mut &foreign[..], c.to_key().unwrap()).unwrap();
        assert!(
            String::from_utf8(xml)
                .unwrap()
                .contains("<OtherAppSetting>")
        );
        let report = fidelity::check(&foreign, c.to_key().unwrap()).unwrap();
        assert!(!report.is_clean());
        assert!(
            fidelity::check(&plain, c.to_key().unwrap())
                .unwrap()
                .is_clean()
        );
    }

    #[test]
    fn rekey_stage_then_publish_new_then_delete() {
        let dir = temp_dir();
        let old = creds("pw", Some([1; 32]));
        let new = creds("pw", Some([2; 32]));
        let mut v = vault_with_backups(&dir, &old, 3);
        let folder = folder_of(&v);
        let before = folder.list().unwrap();
        assert_eq!(before.len(), 3);
        let mtimes: Vec<SystemTime> = before
            .iter()
            .map(|b| fs::metadata(&b.path).unwrap().modified().unwrap())
            .collect();

        let progress = Progress::default();
        let staged = v
            .plan_key_change(new.clone())
            .unwrap()
            .run(&AtomicBool::new(false), &progress)
            .unwrap();
        assert_eq!((staged.rekeyed(), staged.failed()), (3, 0));
        assert_eq!((progress.done(), progress.total()), (3, 3));
        assert_eq!(staged.kdf_runs(), 12, "four key derivations per backup");
        // Staged, not published: the originals are as they were, the copies wait in temps.
        let manifest = folder.read_manifest().unwrap().unwrap();
        assert_eq!(manifest.to_gen, staged.generation());
        assert_eq!(manifest.items.len(), 3);
        for item in &manifest.items {
            assert!(folder.path().join(&item.tmp).is_file());
        }
        assert_eq!(folder.list().unwrap(), before);
        let generation = staged.generation().to_owned();

        let report = v
            .commit_key_change(staged, &ring(&[2; 32]), Some("v-backup 2026-10-14.keyx"))
            .unwrap();
        assert_eq!((report.rekeyed, report.kept_old), (3, 0));
        assert!(report.ring_error.is_none());
        assert_eq!(v.generation(), generation);
        assert!(v.generation_committed());
        assert!(Vault::open(v.path(), old.clone()).is_err());
        assert!(Vault::open(v.path(), new.clone()).is_ok());

        let after = folder.list().unwrap();
        for (original, mtime) in before.iter().zip(&mtimes) {
            assert!(
                !original.path.exists(),
                "{} was deleted",
                original.file_name
            );
            let copy = after
                .iter()
                .find(|b| b.name.seq == original.name.seq)
                .expect("a re-encrypted copy");
            assert_eq!(copy.name.rev, 1);
            assert_eq!(copy.name.kind(), original.name.kind());
            assert_eq!(copy.generation.as_deref(), Some(&*generation));
            let meta = fs::metadata(&copy.path).unwrap();
            assert!(meta.permissions().readonly());
            assert_eq!(
                meta.modified().unwrap(),
                *mtime,
                "keeps its original's time"
            );
            assert!(Vault::open(&copy.path, old.clone()).is_err());
            let reopened = Vault::open(&copy.path, new.clone()).unwrap();
            assert_eq!(reopened.generation(), generation, "carries the new record");
            assert!(reopened.generation_committed() && !reopened.outside_change());
        }
        // Plus the commit's own copy of the version it replaced.
        assert_eq!(after.len(), 4);
        assert_eq!(after[0].name.kind(), Some(Kind::BeforeKeyChange));
        assert!(folder.read_manifest().unwrap().is_none());
        assert!(
            !fs::read_dir(folder.path())
                .unwrap()
                .flatten()
                .any(|e| fsops::is_temp_name(&e.file_name().to_string_lossy())),
            "no temps left"
        );
        let index = folder.index().unwrap();
        assert!(index.gens[&generation].committed);
        assert_eq!(
            index.gens[&generation].keyx.as_deref(),
            Some("v-backup 2026-10-14.keyx")
        );
        drop(v);
        cleanup(&dir);
    }

    #[test]
    fn rekey_cancel_leaves_store_identical() {
        let dir = temp_dir();
        let old = creds("pw", None);
        let new = creds("pw", Some([2; 32]));
        let mut v = vault_with_backups(&dir, &old, 6);
        // A `.bak` too, which staging copies into the store first.
        let bak = with_suffix(v.path(), ".bak");
        fs::write(&bak, db_bytes(&old, "From the .bak")).unwrap();
        let store = dir.join("Backups");
        let snapshot = hashes(&store);
        let identical = |v: &Vault| {
            assert_eq!(hashes(&store), snapshot, "every file as it was");
            assert!(bak.is_file());
            assert!(Vault::open(v.path(), old.clone()).is_ok());
        };

        // Cancelled before the first key derivation.
        let cancel = AtomicBool::new(true);
        let plan = v.plan_key_change(new.clone()).unwrap();
        assert!(matches!(
            plan.run(&cancel, &Progress::default()),
            Err(StageError::Cancelled)
        ));
        identical(&v);

        // Cancelled from another thread while staging runs (or, if staging wins the race,
        // its result is dropped unpublished, which is the same).
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(Progress::default());
        let plan = v.plan_key_change(new.clone()).unwrap();
        let (c, p) = (cancel.clone(), progress.clone());
        let worker = std::thread::spawn(move || plan.run(&c, &p).map(drop));
        while progress.done() == 0 && !worker.is_finished() {
            std::thread::yield_now();
        }
        cancel.store(true, Ordering::SeqCst);
        let _ = worker.join().unwrap();
        identical(&v);

        // Staged in full, then thrown away.
        let staged = stage(&mut v, &new);
        assert_eq!(staged.rekeyed(), 7);
        assert!(folder_of(&v).read_manifest().unwrap().is_some());
        staged.discard();
        identical(&v);
        drop(v);
        cleanup(&dir);
    }

    #[test]
    fn rekey_skips_lossy_unsupported_and_over_budget() {
        let dir = temp_dir();
        let old = creds("pw", Some([1; 32]));
        let new = creds("pw", Some([2; 32]));
        let mut v = vault_with_backups(&dir, &old, 1);
        let folder = folder_of(&v);

        // Another app's file: plugin CustomData (kept) and an element this writer drops.
        let mut other = Database::new();
        other.config.kdf_config = KdfConfig::Aes { rounds: 1000 };
        other.config.compression_config = keepass::config::CompressionConfig::None;
        other.meta.custom_data.insert(
            "KeePassHttpSettings".into(),
            CustomDataItem {
                value: Some(CustomDataValue::String("{\"Version\":1}".into())),
                last_modification_time: Some(Times::now()),
            },
        );
        let mut written = Vec::new();
        other.save(&mut written, old.to_key().unwrap()).unwrap();
        let lossy = with_unknown_element(
            &written,
            &old,
            "<KeePassXCPluginState>enabled</KeePassXCPluginState>",
        );
        // KDBX 4.0, which this writer can't save.
        let kdbx40 = header::patch_minor_version(&db_bytes(&old, "Old format"), 0);
        // A header asking Argon2 for 4 GiB, over the budget.
        let argon2 = |memory| {
            let mut db = Database::new();
            db.config.kdf_config = KdfConfig::Argon2id {
                iterations: 2,
                memory,
                parallelism: 1,
                version: argon2::Version::Version13,
            };
            let mut out = Vec::new();
            db.save(&mut out, old.to_key().unwrap()).unwrap();
            out
        };
        let huge = header::patch_kdf_param(&argon2(1 << 20), "M", &(4u64 << 30).to_le_bytes());
        // Within the budget but far heavier than the vault's own AES-KDF.
        let heavier = argon2(4 << 20);
        let cases = [
            (
                put(&folder, &lossy, Kind::Manual),
                lossy.clone(),
                "failed: lossy",
            ),
            (
                put(&folder, &kdbx40, Kind::Imported),
                kdbx40.clone(),
                "failed: format",
            ),
            (put(&folder, &huge, Kind::Auto), huge.clone(), "failed: kdf"),
            (
                put(&folder, &heavier, Kind::Opened),
                heavier.clone(),
                "failed: kdf",
            ),
        ];

        let started = std::time::Instant::now();
        let staged = stage(&mut v, &new);
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        assert_eq!((staged.rekeyed(), staged.failed()), (1, 4));
        assert_eq!(
            staged.kdf_runs(),
            4 + 4,
            "the vault's own backup and the lossy file; no key derivation for the others"
        );
        v.commit_key_change(staged, &ring(&[2; 32]), None).unwrap();

        let listed = folder.list().unwrap();
        for (name, bytes, why) in &cases {
            assert_eq!(
                &fs::read(folder.path().join(name)).unwrap(),
                bytes,
                "{name} untouched"
            );
            let entry = listed.iter().find(|b| &b.file_name == name).unwrap();
            assert_eq!(entry.rekey.as_deref(), Some(*why), "{name}");
        }
        assert!(Vault::open(&folder.path().join(&cases[0].0), old.clone()).is_ok());
        drop(v);
        cleanup(&dir);
    }

    #[test]
    fn rekey_covers_unknown_and_password_only() {
        let dir = temp_dir();
        let password_only = creds("pw", None);
        let k1 = creds("pw", Some([1; 32]));
        let k2 = creds("pw", Some([2; 32]));
        let mut v = vault_with_backups(&dir, &password_only, 2);
        // First passkey setup: the backups move to K1 with it.
        let staged = stage(&mut v, &k1);
        v.commit_key_change(staged, &ring(&[1; 32]), None).unwrap();
        let folder = folder_of(&v);
        // Password-only copies the store never re-keyed (from a root that was unplugged then).
        let strays = [
            put(&folder, &db_bytes(&password_only, "stray one"), Kind::Auto),
            put(
                &folder,
                &db_bytes(&password_only, "stray two"),
                Kind::Imported,
            ),
        ];
        // Nothing in the index: every generation is unknown.
        fs::remove_file(folder.path().join(backups::INDEX_FILE)).unwrap();
        let total = folder.list().unwrap().len();
        assert!(
            folder
                .list()
                .unwrap()
                .iter()
                .all(|b| b.generation.is_none())
        );

        // A revocation re-keys all of them, the password-only ones too.
        let staged = stage(&mut v, &k2);
        assert_eq!((staged.rekeyed(), staged.failed()), (total, 0));
        v.commit_key_change(staged, &ring(&[2; 32]), None).unwrap();
        for b in folder.list().unwrap() {
            for c in [&password_only, &k1] {
                assert!(Vault::open(&b.path, c.clone()).is_err(), "{}", b.file_name);
            }
            assert!(Vault::open(&b.path, k2.clone()).is_ok(), "{}", b.file_name);
        }
        for stray in &strays {
            assert!(!folder.path().join(stray).exists());
        }
        drop(v);
        cleanup(&dir);
    }

    #[test]
    fn manifest_recovery_publishes_or_discards() {
        let dir = temp_dir();
        let old = creds("pw", None);
        let new = creds("pw", Some([2; 32]));

        // The app stops after staging, before the database commits: the copies go.
        let mut v = vault_with_backups(&dir, &old, 2);
        let folder = folder_of(&v);
        let before = hashes(folder.path());
        stage(&mut v, &new).abandon();
        assert!(folder.read_manifest().unwrap().is_some());
        drop(v);
        let mut v = reopen_at(&dir, &old);
        let report = v.recover_backups();
        assert_eq!(report.rekey.discarded, [folder.path().to_owned()]);
        assert!(report.rekey.published.is_empty());
        assert_eq!(hashes(folder.path()), before, "temps and manifest gone");

        // The app stops after the database committed, before publishing: the copies go in.
        let staged = stage(&mut v, &new);
        let generation = staged.generation().to_owned();
        v.commit_without_publishing(staged);
        let manifest = folder.read_manifest().unwrap().unwrap();
        // The file is on the new key now, and its record names the manifest's generation.
        drop(v);
        let mut v = reopen_at(&dir, &new);
        assert_eq!(v.generation(), generation);
        assert!(v.generation_committed());
        let report = v.recover_backups();
        assert_eq!(report.rekey.published, [folder.path().to_owned()]);
        assert!(folder.read_manifest().unwrap().is_none());
        for item in &manifest.items {
            assert!(!folder.path().join(&item.original).exists());
            assert!(!folder.path().join(&item.tmp).exists());
        }
        for b in folder.list().unwrap() {
            assert!(Vault::open(&b.path, new.clone()).is_ok(), "{}", b.file_name);
            assert_eq!(b.generation.as_deref(), Some(&*generation));
        }
        // Running it again finds nothing to do.
        assert_eq!(v.recover_backups(), RecoveryReport::default());
        drop(v);
        cleanup(&dir);
    }

    fn reopen_at(dir: &Path, c: &Credentials) -> Vault {
        let mut v = Vault::open(&dir.join("data").join("v.kdbx"), c.clone()).unwrap();
        v.attach_store(store_config(dir));
        v
    }

    #[test]
    fn key_change_imports_bak_and_rekeys_it() {
        let dir = temp_dir();
        let old = creds("pw", Some([1; 32]));
        let new = creds("pw", Some([2; 32]));
        let mut v = vault_with_backups(&dir, &old, 1);
        let bak = with_suffix(v.path(), ".bak");
        let bak_bytes = db_bytes(&old, "From the .bak");
        fs::write(&bak, &bak_bytes).unwrap();

        let staged = stage(&mut v, &new);
        assert_eq!(staged.rekeyed(), 2, "the backup and the .bak");
        assert!(bak.is_file(), "kept until the commit");
        v.commit_key_change(staged, &ring(&[2; 32]), None).unwrap();
        assert!(!bak.exists(), "moved into the store");
        let listed = folder_of(&v).list().unwrap();
        let imported: Vec<_> = listed
            .iter()
            .filter(|b| b.name.kind() == Some(Kind::Imported))
            .collect();
        assert_eq!(imported.len(), 1);
        assert_eq!(imported[0].name.rev, 1, "re-encrypted");
        assert!(Vault::open(&imported[0].path, old.clone()).is_err());
        let restored = Vault::open(&imported[0].path, new.clone()).unwrap();
        assert!(
            restored
                .db
                .iter_all_entries()
                .any(|e| e.get_title() == Some("From the .bak"))
        );
        drop((v, restored));
        cleanup(&dir);
    }

    /// A save that found no backup folder kept the previous version as `<db>.bak` (and noted its
    /// hash to move it in later). The key change that follows re-encrypts that `.bak` like any
    /// backup; its own save must not then move the old-key `.bak` into the store as it is.
    #[test]
    fn key_change_after_a_fallback_bak_keeps_no_old_copy() {
        let dir = temp_dir();
        let old = creds("pw", None);
        let new = creds("pw", Some([2; 32]));
        let mut v = vault_with_backups(&dir, &old, 1);
        let store = dir.join("Backups");
        let away = dir.join("Backups.away");
        fs::rename(&store, &away).unwrap();
        fs::write(&store, b"not a folder").unwrap();
        v.db.root_mut()
            .add_entry()
            .set_unprotected(fields::TITLE, "saved while the folder was gone");
        v.mark_dirty();
        v.save().unwrap();
        let bak = with_suffix(v.path(), ".bak");
        assert!(bak.is_file(), "the fallback wrote a .bak");
        let bak_bytes = fs::read(&bak).unwrap();
        fs::remove_file(&store).unwrap();
        fs::rename(&away, &store).unwrap();

        let staged = stage(&mut v, &new);
        let report = v.commit_key_change(staged, &ring(&[2; 32]), None).unwrap();
        assert_eq!(report.kept_old, 0);
        assert!(!bak.exists());
        let all = folder_of(&v).list_all().unwrap();
        for b in all.iter().filter(|b| b.name.is_kdbx()) {
            assert!(
                Vault::open(&b.path, old.clone()).is_err(),
                "{} opens with the replaced credentials",
                b.file_name
            );
        }
        let imported: Vec<_> = all
            .iter()
            .filter(|b| b.name.kind() == Some(Kind::Imported))
            .collect();
        assert_eq!(imported.len(), 1, "{all:#?}");
        assert_eq!(imported[0].name.rev, 1, "the re-encrypted copy only");

        // Had the old `.bak` survived the commit (held open elsewhere), no later save moves it
        // into the store as it is: the key change forgot it.
        fs::write(&bak, &bak_bytes).unwrap();
        v.mark_dirty();
        v.save().unwrap();
        assert!(bak.exists());
        assert!(
            folder_of(&v)
                .list_all()
                .unwrap()
                .iter()
                .filter(|b| b.name.is_kdbx())
                .all(|b| Vault::open(&b.path, old.clone()).is_err())
        );
        drop(v);
        cleanup(&dir);
    }

    /// Every copy of the database beside it moves into the store and is re-encrypted with the
    /// backups: `.bak`, `.tmp` and `.bak.tmp` from earlier versions and the temps a crash left.
    /// Old passkey files beside it go at the commit. Nothing beside it opens with the old
    /// credentials afterwards.
    #[test]
    fn key_change_moves_every_copy_beside_the_database() {
        let dir = temp_dir();
        let old = creds("pw", None);
        let new = creds("pw", Some([2; 32]));
        let mut v = vault_with_backups(&dir, &old, 1);
        let db_name = v.path().file_name().unwrap().to_string_lossy().into_owned();
        let beside = |suffix: &str| v.path().with_file_name(format!("{db_name}{suffix}"));
        let copies: Vec<PathBuf> = [
            ".bak".to_owned(),
            ".tmp".to_owned(),
            ".bak.tmp".to_owned(),
            fsops::temp_name(),
            format!(".bak{}", fsops::temp_name()),
        ]
        .iter()
        .map(|s| beside(s))
        .collect();
        for (i, path) in copies.iter().enumerate() {
            fs::write(path, db_bytes(&old, &format!("copy {i}"))).unwrap();
        }
        // The same contents twice need one copy in the store.
        fs::copy(&copies[0], &copies[2]).unwrap();
        let not_a_database = beside(".tmp.txt");
        fs::write(&not_a_database, db_bytes(&old, "not ours")).unwrap();
        let rings: Vec<PathBuf> = [".axvault.old", ".axvault.tmp"]
            .iter()
            .map(|s| beside(s))
            .chain([beside(&format!(".axvault{}", fsops::temp_name()))])
            .collect();
        for path in &rings {
            fs::write(path, b"AxentraVault/2\n").unwrap();
        }

        let mut preview = v.key_change_preview();
        preview.beside.sort();
        let mut expected = copies.clone();
        expected.sort();
        assert_eq!(preview.beside, expected);

        let staged = stage(&mut v, &new);
        assert_eq!(
            staged.rekeyed(),
            1 + 4,
            "the backup and four distinct copies"
        );
        assert!(copies.iter().all(|p| p.exists()), "kept until the commit");
        let report = v.commit_key_change(staged, &ring(&[2; 32]), None).unwrap();
        assert_eq!((report.kept_old, report.held), (0, 0));
        assert!(report.left_beside.is_empty());
        for path in copies.iter().chain(&rings) {
            assert!(!path.exists(), "{} was removed", path.display());
        }
        assert!(not_a_database.exists(), "other files stay");
        let folder = folder_of(&v);
        assert!(folder.read_manifest().unwrap().is_none());
        for b in folder
            .list_all()
            .unwrap()
            .iter()
            .filter(|b| b.name.is_kdbx())
        {
            assert!(
                Vault::open(&b.path, old.clone()).is_err(),
                "{}",
                b.file_name
            );
            assert!(Vault::open(&b.path, new.clone()).is_ok(), "{}", b.file_name);
        }
        drop(v);
        cleanup(&dir);
    }

    /// A copy beside the database that another program holds open at the commit stays, and the
    /// report says so; the manifest keeps it listed, and the next unlock deletes it. The same
    /// after the app stopped between the database's commit and publishing.
    #[cfg(windows)]
    #[test]
    fn a_copy_beside_held_at_the_commit_goes_at_the_next_unlock() {
        #[cfg(windows)]
    use std::os::windows::fs::OpenOptionsExt;

        let dir = temp_dir();
        let old = creds("pw", None);
        let new = creds("pw", Some([2; 32]));
        let mut v = vault_with_backups(&dir, &old, 1);
        let bak = with_suffix(v.path(), ".bak");
        fs::write(&bak, db_bytes(&old, "From the .bak")).unwrap();
        let staged = stage(&mut v, &new);
        // FILE_SHARE_READ only: no deleting meanwhile.
        let held = fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&bak)
            .unwrap();
        let report = v.commit_key_change(staged, &ring(&[2; 32]), None).unwrap();
        assert_eq!(report.left_beside, std::slice::from_ref(&bak));
        let folder = folder_of(&v);
        let manifest = folder.read_manifest().unwrap().expect("still listed");
        assert!(manifest.items.is_empty());
        assert_eq!(manifest.beside.len(), 1);
        drop((held, v));

        let mut v = reopen_at(&dir, &new);
        assert_eq!(
            v.recover_backups().rekey.published,
            [folder.path().to_owned()]
        );
        assert!(!bak.exists());
        assert!(folder.read_manifest().unwrap().is_none());

        // The app stops after the database committed: the next unlock deletes the copy too.
        fs::write(&bak, db_bytes(&new, "Another .bak")).unwrap();
        let newer = creds("pw", Some([3; 32]));
        let staged = stage(&mut v, &newer);
        v.commit_without_publishing(staged);
        assert!(bak.exists());
        drop(v);
        let mut v = reopen_at(&dir, &newer);
        v.recover_backups();
        assert!(!bak.exists());
        for b in folder
            .list_all()
            .unwrap()
            .iter()
            .filter(|b| b.name.is_kdbx())
        {
            assert!(
                Vault::open(&b.path, new.clone()).is_err(),
                "{}",
                b.file_name
            );
        }
        drop(v);
        cleanup(&dir);
    }

    /// An original another program holds open when its re-encrypted copy is published stays for
    /// a while, but never for good: the report counts it, a second key change before the next
    /// unlock doesn't lose track of it, and once it is free the next unlock deletes it. Nothing
    /// in the store then opens with either replaced key.
    #[cfg(windows)]
    #[test]
    fn an_original_held_at_publishing_is_never_orphaned() {
        #[cfg(windows)]
    use std::os::windows::fs::OpenOptionsExt;

        let dir = temp_dir();
        let (k1, k2, k3) = (
            creds("pw", Some([1; 32])),
            creds("pw", Some([2; 32])),
            creds("pw", Some([3; 32])),
        );
        let mut v = vault_with_backups(&dir, &k1, 2);
        let folder = folder_of(&v);
        let original = folder.list().unwrap()[1].path.clone();
        let staged = stage(&mut v, &k2);
        let held = fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&original)
            .unwrap();
        let report = v.commit_key_change(staged, &ring(&[2; 32]), None).unwrap();
        assert_eq!((report.rekeyed, report.held), (2, 1));
        assert!(original.exists());
        assert!(
            folder.list().unwrap().iter().all(|b| b.path != original),
            "a lower revision"
        );

        // Revoked again in the same session, while it is still held.
        let staged = stage(&mut v, &k3);
        v.commit_key_change(staged, &ring(&[3; 32]), None).unwrap();
        assert!(original.exists());
        drop(held);
        drop(v);

        let mut v = reopen_at(&dir, &k3);
        let report = v.recover_backups();
        assert_eq!(
            report.superseded,
            [original.file_name().unwrap().to_str().unwrap()]
        );
        assert!(!original.exists());
        for b in folder
            .list_all()
            .unwrap()
            .iter()
            .filter(|b| b.name.is_kdbx())
        {
            for c in [&k1, &k2] {
                assert!(Vault::open(&b.path, c.clone()).is_err(), "{}", b.file_name);
            }
        }
        drop(v);
        cleanup(&dir);
    }

    #[test]
    fn leftover_save_temps_are_imported_or_swept() {
        let dir = temp_dir();
        let c = creds("pw", None);
        let mut v = vault_with_backups(&dir, &c, 1);
        let name = v.path().file_name().unwrap().to_string_lossy().into_owned();
        let data = v.path().parent().unwrap().to_owned();
        let complete = data.join(format!("{name}{}", fsops::temp_name()));
        fs::write(&complete, db_bytes(&c, "Unsaved elsewhere")).unwrap();
        let young = data.join(format!("{name}{}", fsops::temp_name()));
        fs::write(&young, b"partial").unwrap();
        let old = data.join(format!("{name}{}", fsops::temp_name()));
        fs::write(&old, b"partial").unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(SystemTime::now() - std::time::Duration::from_secs(2 * 3600))
            .unwrap();
        let foreign = data.join(format!("other.kdbx{}", fsops::temp_name()));
        fs::write(&foreign, db_bytes(&c, "Another vault")).unwrap();

        let report = v.recover_backups();
        assert_eq!(report.imported, [complete.as_path()]);
        assert_eq!(report.removed, [old.as_path()]);
        assert!(!complete.exists() && !old.exists());
        assert!(
            young.exists() && foreign.exists(),
            "young and other files stay"
        );
        let imported: Vec<_> = folder_of(&v)
            .list()
            .unwrap()
            .into_iter()
            .filter(|b| b.name.kind() == Some(Kind::Imported))
            .collect();
        assert_eq!(imported.len(), 1);
        assert!(Vault::open(&imported[0].path, c.clone()).is_ok());
        drop(v);
        cleanup(&dir);
    }
}
