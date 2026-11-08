//! What the Backups section in Settings does with a vault's backup store:
//! - an overview of the vault's backups in every known root: how many, the newest, and which
//!   credentials each one needs as far as the index knows ([`BackupsTask::overview`]);
//! - deleting the backups that need older credentials, or all of them;
//! - moving files earlier versions left beside the database (`<db>.bak`, `<db>.tmp`,
//!   `<db>.bak.tmp`, `<db>.axvault.old`) into the store ([`old_files`], `Vault::migrate_old_files`);
//! - checking a folder the user picks as the backup root ([`check_new_root`]).
//!
//! Listing and deleting may wait on a slow or unplugged root, so they run on a worker thread
//! from a [`BackupsTask`]; the rest runs where the vault is (see the `Vault` methods).

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use chrono::{DateTime, Local};

use crate::backups::{self, Backup, BackupConfig, Folder, GenInfo, Kind, RootKind};
use crate::envelope;
use crate::fsops;
use crate::limits::{DATABASE_MAX, read_capped};
use crate::store::Store;
use crate::vault::with_suffix;

/// Which credentials a backup needs, as far as the store's index knows. Labels are hints: they
/// follow the content hashes this app opened or saved itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyLabel {
    /// The session's credentials open it.
    Current,
    /// Credentials the vault had before a key change: the backup key file made for them (its
    /// date and name, when known), or the credentials it used from that date. A backup a key
    /// change couldn't re-encrypt counts here too.
    Older {
        since: Option<String>,
        key_file: Option<String>,
    },
    /// Not known: a version another app saved, or an imported file.
    Unknown,
}

impl KeyLabel {
    pub fn text(&self) -> String {
        match self {
            KeyLabel::Current => "Current key".into(),
            KeyLabel::Older {
                since: Some(date),
                key_file: Some(name),
            } => format!("Older key – needs your backup key file from {date} ({name})"),
            // No backup key file is known for it (a password-only vault, a change made in
            // KeePass): it may never have had one.
            KeyLabel::Older {
                since: Some(date),
                key_file: None,
            } => format!("Older key – needs the credentials the vault used from {date}"),
            KeyLabel::Older {
                since: None,
                key_file: Some(name),
            } => format!("Older key – needs your backup key file {name}"),
            KeyLabel::Older {
                since: None,
                key_file: None,
            } => "Older key – needs the vault's earlier credentials".into(),
            KeyLabel::Unknown => "Unknown".into(),
        }
    }
}

/// `created` (UTC, RFC 3339) as a local date.
fn local_date(created: &str) -> Option<String> {
    DateTime::parse_from_rfc3339(created)
        .ok()
        .map(|t| t.with_timezone(&Local).format("%Y-%m-%d").to_string())
}

/// A backup's label: the session's generation is current; another generation a save committed
/// and a credential change then replaced is older; so is a backup a key change marked as not
/// re-encrypted. Anything else is unknown, including a committed generation nothing replaced:
/// a lost record (a file restored from before it) starts a new generation with the same
/// credentials, so such backups usually open with the current ones.
fn label(backup: &Backup, gens: &BTreeMap<String, GenInfo>, session: &str) -> KeyLabel {
    let generation = backup.generation.as_deref();
    if generation == Some(session) {
        return KeyLabel::Current;
    }
    if let Some(info) = generation
        .and_then(|g| gens.get(g))
        .filter(|info| info.committed && info.retired)
    {
        return KeyLabel::Older {
            since: local_date(&info.created),
            key_file: info.keyx.clone(),
        };
    }
    if backup.rekey.is_some() {
        return KeyLabel::Older {
            since: None,
            key_file: None,
        };
    }
    KeyLabel::Unknown
}

/// One database backup, as the Backups section lists it.
#[derive(Debug, Clone)]
pub struct Listed {
    pub backup: Backup,
    pub key: KeyLabel,
}

impl Listed {
    pub fn kind(&self) -> Kind {
        self.backup.name.kind().unwrap_or(Kind::Auto)
    }

    /// `YYYY-MM-DD HH:MM`: the version's own time, from its name.
    pub fn when(&self) -> String {
        let name = &self.backup.name;
        let hm = name.time.get(..5).unwrap_or(&name.time).replace('-', ":");
        format!("{} {hm}", name.date)
    }

    pub fn needs_older(&self) -> bool {
        matches!(self.key, KeyLabel::Older { .. })
    }
}

/// The vault's backups across every known root, for the Backups section.
#[derive(Debug, Clone, Default)]
pub struct Overview {
    /// Where new backups go: the chosen root, else the default one.
    pub location: PathBuf,
    /// Whether `location` is a folder the user chose (else the default under %LOCALAPPDATA%).
    pub chosen: bool,
    /// Why the chosen root can't be used right now; backups go to the default root meanwhile.
    pub unavailable: Option<String>,
    /// What "Open folder" shows: this vault's folder where new backups go now, else that root.
    pub open: Option<PathBuf>,
    /// The database backups in every reachable root, newest first.
    pub backups: Vec<Listed>,
    /// Other known roots (or folders) that couldn't be read, and why.
    pub unreachable: Vec<(PathBuf, String)>,
}

impl Overview {
    pub fn newest(&self) -> Option<&Listed> {
        self.backups.first()
    }

    /// Backups that need older credentials than the session's.
    pub fn older(&self) -> usize {
        self.backups.iter().filter(|b| b.needs_older()).count()
    }
}

/// What deleting backups did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeleteReport {
    pub deleted: usize,
    /// Still there: another program has them open. Tried again next time.
    pub left: usize,
    pub errors: Vec<String>,
}

/// The Backups section's work for a worker thread: a detached view of the vault's store (see
/// `Vault::backups_task`).
pub struct BackupsTask {
    pub(crate) store: Store,
    pub(crate) db_path: PathBuf,
    pub(crate) vault_id: String,
    /// The session's credential generation: its backups are current.
    pub(crate) generation: String,
}

impl BackupsTask {
    /// Lists the vault's backups in every known root that answers (creating nothing).
    pub fn overview(&mut self) -> Overview {
        let config = self.store.config().clone();
        let home = config.home();
        let (folders, unreachable) = self.store.existing_folders(&self.db_path, &self.vault_id);
        let mut overview = Overview {
            location: home.path.clone(),
            chosen: home.kind == RootKind::Configured,
            ..Overview::default()
        };
        for (root, why) in unreachable {
            if root.kind == RootKind::Configured {
                overview.unavailable = Some(why.to_string());
            } else {
                overview.unreachable.push((root.path, why.to_string()));
            }
        }
        let folder_in = |root: &Path| -> Option<PathBuf> {
            folders
                .iter()
                .find(|f| f.root() == root)
                .map(|f| f.path().to_owned())
        };
        overview.open = match overview.unavailable {
            None => folder_in(&home.path).or_else(|| home.path.is_dir().then(|| home.path.clone())),
            Some(_) => folder_in(&config.default_root),
        };
        for folder in &folders {
            let gens = folder.index().map(|i| i.gens).unwrap_or_default();
            match folder.list() {
                Ok(listed) => overview
                    .backups
                    .extend(listed.into_iter().map(|backup| Listed {
                        key: label(&backup, &gens, &self.generation),
                        backup,
                    })),
                Err(e) => overview
                    .unreachable
                    .push((folder.path().to_owned(), e.to_string())),
            }
        }
        // Across roots by the version's time; within a folder that agrees with `seq`.
        overview.backups.sort_by(|a, b| {
            let key = |l: &Listed| {
                let n = &l.backup.name;
                (n.date.clone(), n.time.clone(), n.seq)
            };
            key(b).cmp(&key(a))
        });
        overview
    }

    /// Deletes every backup that needs older credentials (see [`KeyLabel::Older`]) in every
    /// reachable folder, the old-key originals a key change re-encrypted but couldn't delete
    /// then (see [`Folder::superseded`]), and the ring snapshots of other generations (they wrap
    /// older keys).
    pub fn delete_older(&mut self) -> DeleteReport {
        let generation = self.generation.clone();
        self.delete_in_folders(|folder| {
            let gens = folder.index()?.gens;
            let mut names: Vec<String> = folder
                .list()?
                .into_iter()
                .filter(|b| matches!(label(b, &gens, &generation), KeyLabel::Older { .. }))
                .map(|b| b.file_name)
                .collect();
            names.extend(folder.superseded()?);
            Ok(names)
        })
    }

    /// Deletes every database backup of the vault in every reachable folder (all revisions),
    /// a key change's staged copies, and the ring snapshots of other generations. Snapshots of
    /// the current passkey file stay: they hold nothing the file beside the database doesn't.
    pub fn delete_all(&mut self) -> DeleteReport {
        self.delete_in_folders(|folder| {
            // Staged copies would otherwise be published again at the next unlock.
            if let Some(manifest) = folder.read_manifest()? {
                folder.discard_rekey(&manifest)?;
            }
            Ok(folder
                .list_all()?
                .into_iter()
                .filter(|b| b.name.is_kdbx())
                .map(|b| b.file_name)
                .collect())
        })
    }

    /// Deletes the files `pick` names in each reachable folder, then other generations' ring
    /// snapshots. A folder a key change is staging in is left alone.
    fn delete_in_folders(
        &mut self,
        pick: impl Fn(&Folder) -> io::Result<Vec<String>>,
    ) -> DeleteReport {
        let (folders, _) = self.store.existing_folders(&self.db_path, &self.vault_id);
        let mut report = DeleteReport::default();
        for folder in &folders {
            if folder.staging_active() {
                continue;
            }
            let done = pick(folder).and_then(|names| {
                let refs: Vec<&str> = names.iter().map(String::as_str).collect();
                let deleted = folder.delete(&refs)?;
                report.deleted += deleted.len();
                report.left += names
                    .iter()
                    .filter(|n| folder.path().join(n).exists())
                    .count();
                folder.delete_rings_except(&self.generation).map(drop)
            });
            if let Err(e) = done {
                report
                    .errors
                    .push(format!("{}: {e}", folder.path().display()));
            }
        }
        report
    }
}

// ---------------------------------------------------------------------------------------------
// Old files beside the database

/// A file beside the database that belongs in the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OldFile {
    pub path: PathBuf,
    /// A copy of the database (imported into the store); else an old passkey file (deleted:
    /// it may wrap a revoked key).
    pub database: bool,
}

/// The old files beside `db_path`: `<db>.bak`, `<db>.tmp` and `<db>.bak.tmp` that hold a
/// KeePass database (at least 512 bytes, with the KDBX signature), and `<db>.axvault.old`.
pub fn old_files(db_path: &Path) -> Vec<OldFile> {
    // Links are not followed.
    let is_file = |p: &Path| fs::symlink_metadata(p).is_ok_and(|m| m.is_file());
    let mut found: Vec<OldFile> = [".bak", ".tmp", ".bak.tmp"]
        .iter()
        .map(|suffix| with_suffix(db_path, suffix))
        .filter(|p| is_file(p) && backups::looks_like_kdbx(p))
        .map(|path| OldFile {
            path,
            database: true,
        })
        .collect();
    let old_ring = envelope::legacy_old_path_for(db_path);
    if is_file(&old_ring) {
        found.push(OldFile {
            path: old_ring,
            database: false,
        });
    }
    found
}

/// What moving the old files did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MigrationReport {
    /// Copied into the store, read back and compared, then deleted.
    pub moved: Vec<PathBuf>,
    /// Copied and verified, but not deleted (another program has it open): still there.
    pub kept: Vec<(PathBuf, String)>,
    /// Old passkey files deleted.
    pub removed: Vec<PathBuf>,
    /// Left as they were, and why.
    pub failed: Vec<(PathBuf, String)>,
}

const IN_USE: &str = "another program has it open";

/// Moves the old files into the store: each database copy is written there as `imported`
/// (named after its own time, with no generation: nothing says which credentials open it), read
/// back and compared, and only then deleted. Old passkey files are deleted. A file that can't be
/// deleted stays where it is, and the report says so.
pub(crate) fn migrate(store: &mut Store, db_path: &Path, vault_id: &str) -> MigrationReport {
    let mut report = MigrationReport::default();
    for old in old_files(db_path) {
        if old.database
            && let Err(why) = import(store, db_path, vault_id, &old.path)
        {
            report.failed.push((old.path, why));
            continue;
        }
        let removed = match fsops::remove_file(&old.path) {
            Ok(true) => Ok(()),
            Ok(false) if !old.path.exists() => Ok(()),
            Ok(false) => Err(IN_USE.to_owned()),
            Err(e) => Err(e.to_string()),
        };
        match (removed, old.database) {
            (Ok(()), true) => report.moved.push(old.path),
            (Ok(()), false) => report.removed.push(old.path),
            (Err(why), true) => report.kept.push((old.path, why)),
            (Err(why), false) => report.failed.push((old.path, why)),
        }
    }
    report
}

/// Copies one old database file into the store as `imported`, unless a backup with the same
/// contents is already there (an earlier move that couldn't delete its source), and checks that
/// the copy reads back the same.
fn import(store: &mut Store, db_path: &Path, vault_id: &str, path: &Path) -> Result<(), String> {
    let bytes = read_capped(path, DATABASE_MAX).map_err(|e| e.to_string())?;
    let sha256 = backups::sha256_hex(&bytes);
    let folder = store
        .writable_folder(db_path, vault_id, bytes.len() as u64)
        .map_err(|e| e.to_string())?;
    let existing = folder
        .list_all()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|b| b.name.is_kdbx() && b.sha256.as_deref() == Some(sha256.as_str()))
        .map(|b| b.path);
    let copy = match existing {
        Some(copy) => copy,
        None => {
            let mtime = fs::metadata(path).and_then(|m| m.modified()).ok();
            folder
                .write_backup(&bytes, Kind::Imported, mtime, None)
                .map_err(|e| e.to_string())?
                .path()
                .to_owned()
        }
    };
    match backups::sha256_file(&copy) {
        Ok(h) if h == sha256 => Ok(()),
        Ok(_) => Err("the copy in the backup folder didn't read back the same".into()),
        Err(e) => Err(e.to_string()),
    }
}

// ---------------------------------------------------------------------------------------------
// Choosing the backup root

/// A reason to think twice about a folder as the backup root; the user may go ahead anyway.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootWarning {
    /// Inside a folder a sync service uploads: the backups would leave this PC.
    Synced(&'static str),
    /// FAT, FAT32 or exFAT: no file permissions.
    NoPermissions(String),
    /// A network share.
    Network,
    /// The folder already holds KeePass databases (one is named), such as vaults on a USB drive.
    HoldsDatabases(String),
}

impl RootWarning {
    pub fn text(&self) -> String {
        match self {
            RootWarning::Synced(service) => format!(
                "That folder syncs to {service}. Backups there leave this PC, along with every version they hold."
            ),
            RootWarning::NoPermissions(fs) => format!(
                "That drive uses {fs}, which has no file permissions: anyone with the drive can copy the backups."
            ),
            RootWarning::Network => "That is a network folder: backups there can be slow or unavailable, and other people with access to the share may copy them.".into(),
            RootWarning::HoldsDatabases(name) => format!(
                "That folder already holds KeePass databases (such as {name}). They stay ordinary vaults, but whoever has this folder would have the backups too; a folder of its own is easier to keep apart."
            ),
        }
    }
}

/// A KeePass database (`.kdbx`) in `folder` or one level below, outside a vault's backup folder
/// (one with a `vault.json`): its name. Looks at no more than a couple of thousand entries.
fn database_in(folder: &Path) -> Option<String> {
    const MAX_ENTRIES: usize = 2000;
    let is_kdbx = |name: &str| name.to_lowercase().ends_with(".kdbx");
    let mut seen = 0;
    let mut subfolders = Vec::new();
    for entry in fs::read_dir(folder).ok()?.flatten() {
        seen += 1;
        if seen > MAX_ENTRIES {
            return None;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        match entry.file_type() {
            Ok(t) if t.is_file() && is_kdbx(&name) => return Some(name),
            Ok(t) if t.is_dir() => subfolders.push(entry.path()),
            _ => {}
        }
    }
    for sub in subfolders {
        if sub.join(backups::VAULT_FILE).is_file() {
            continue;
        }
        for entry in fs::read_dir(&sub).into_iter().flatten().flatten() {
            seen += 1;
            if seen > MAX_ENTRIES {
                return None;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if entry.file_type().is_ok_and(|t| t.is_file()) && is_kdbx(&name) {
                return Some(name);
            }
        }
    }
    None
}

/// The sync service whose folder holds `path`, if any: one of `onedrive_roots` (the app passes
/// the OneDrive folders from the environment), or a folder named like OneDrive's, Dropbox's or
/// Google Drive's.
fn sync_service(path: &Path, onedrive_roots: &[PathBuf]) -> Option<&'static str> {
    if onedrive_roots
        .iter()
        .any(|root| !root.as_os_str().is_empty() && backups::is_within(path, root))
    {
        return Some("OneDrive");
    }
    path.components().find_map(|c| {
        let Component::Normal(name) = c else {
            return None;
        };
        let name = name.to_string_lossy().to_lowercase();
        if name == "onedrive" || name.starts_with("onedrive - ") {
            Some("OneDrive")
        } else if name == "dropbox" || name.starts_with("dropbox (") {
            Some("Dropbox")
        } else if matches!(name.as_str(), "google drive" | "googledrive" | "my drive") {
            Some("Google Drive")
        } else {
            None
        }
    })
}

/// Checks a folder the user picked as the backup root. Refused, with the reason: a folder that
/// isn't there; one holding the database (its own folder or any folder above it); one inside a
/// backup root already known. Otherwise the warnings to confirm first: a sync folder, a file
/// system without permissions, a network share, a folder that already holds KeePass databases.
pub fn check_new_root(
    path: &Path,
    db_path: &Path,
    config: &BackupConfig,
    onedrive_roots: &[PathBuf],
) -> Result<Vec<RootWarning>, String> {
    if !path.is_dir() {
        return Err("That folder isn't there.".into());
    }
    if backups::is_within(db_path, path) {
        return Err(
            "That folder holds the vault itself. Choose a folder outside the vault's own folder."
                .into(),
        );
    }
    if config
        .known_roots()
        .iter()
        .any(|r| backups::is_within(path, &r.path) && !backups::is_within(&r.path, path))
    {
        return Err("That folder is inside another backup folder. Choose the backup folder itself or one elsewhere.".into());
    }
    let mut warnings = Vec::new();
    if let Some(service) = sync_service(path, onedrive_roots) {
        warnings.push(RootWarning::Synced(service));
    }
    if let Some(info) = fsops::volume_info(path)
        && ["fat", "fat12", "fat16", "fat32", "exfat"]
            .contains(&info.file_system.to_lowercase().as_str())
    {
        warnings.push(RootWarning::NoPermissions(info.file_system));
    }
    if fsops::is_network(path) {
        warnings.push(RootWarning::Network);
    }
    if let Some(name) = database_in(path) {
        warnings.push(RootWarning::HoldsDatabases(name));
    }
    Ok(warnings)
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use std::os::windows::fs::OpenOptionsExt;

    use keepass::Database;
    use keepass::db::fields;
    use zeroize::Zeroizing;

    use super::*;
    use crate::backups::{BackupSettings, sha256_hex};
    use crate::random_bytes;
    use crate::vault::{Credentials, Vault, VaultError};

    fn temp_dir() -> PathBuf {
        let mut n = [0u8; 8];
        random_bytes(&mut n);
        let dir = std::env::temp_dir().join(format!("axvault-manage-{}", hex::encode(n)));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Backups are read-only; clear that before removing the test folder.
    fn cleanup(dir: &Path) {
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

    fn creds(password: &str) -> Credentials {
        Credentials {
            password: Some(Zeroizing::new(password.to_owned())),
            key_file: None,
        }
    }

    fn new_db(path: &Path, c: &Credentials) {
        let mut db = Database::new();
        db.config.kdf_config = keepass::config::KdfConfig::Aes { rounds: 100 };
        db.root_mut()
            .add_entry()
            .set_unprotected(fields::TITLE, "Bank");
        db.save(&mut fs::File::create(path).unwrap(), c.to_key().unwrap())
            .unwrap();
    }

    fn config(dir: &Path) -> BackupConfig {
        BackupConfig::new(dir.join("Backups")).with_free_space(|_| None)
    }

    fn vault(dir: &Path) -> Vault {
        let path = dir.join("v.kdbx");
        let c = creds("pw");
        new_db(&path, &c);
        let mut v = Vault::open(&path, c).unwrap();
        v.attach_store(config(dir));
        v
    }

    fn save_change(v: &mut Vault, title: &str) {
        v.db.root_mut()
            .add_entry()
            .set_unprotected(fields::TITLE, title);
        v.mark_dirty();
        v.save().unwrap();
    }

    fn overview(v: &Vault) -> Overview {
        v.backups_task().expect("a store").overview()
    }

    /// "Back up now" pins exactly what is on disk, even when the newest backup already holds it;
    /// it is refused with unsaved changes and when another app changed the file.
    #[test]
    fn back_up_now_is_pinned_and_refused_while_dirty() {
        let dir = temp_dir();
        let mut v = vault(&dir);
        save_change(&mut v, "one");
        let on_disk = fs::read(v.path()).unwrap();
        for _ in 0..2 {
            let written = v.back_up_now().unwrap();
            assert_eq!(fs::read(written.path()).unwrap(), on_disk);
        }
        let listed = overview(&v);
        let manual: Vec<_> = listed
            .backups
            .iter()
            .filter(|b| b.kind() == Kind::Manual)
            .collect();
        assert_eq!(manual.len(), 2, "never skipped as a duplicate");
        assert!(manual.iter().all(|b| b.key == KeyLabel::Current));
        assert!(listed.newest().is_some_and(|b| b.kind() == Kind::Manual));

        v.mark_dirty();
        assert!(matches!(v.back_up_now(), Err(VaultError::Unsaved)));
        drop(v);

        // Another app saved since the session opened: what it shows is no longer on disk.
        let open = |dir: &Path| {
            let mut v = Vault::open(&dir.join("v.kdbx"), creds("pw")).unwrap();
            v.attach_store(config(dir));
            v
        };
        let mut v = open(&dir);
        save_change(&mut open(&dir), "elsewhere");
        assert!(matches!(v.back_up_now(), Err(VaultError::ChangedOnDisk)));
        drop(v);
        cleanup(&dir);
    }

    /// After a key change the backups made before it need the older credentials: they are
    /// counted, labelled with the backup key file's name, and deleted on request, while the
    /// current ones and unknown ones stay.
    #[test]
    fn older_credentials_are_counted_labelled_and_deleted() {
        let dir = temp_dir();
        let mut v = vault(&dir);
        save_change(&mut v, "one");
        save_change(&mut v, "two");
        let mut task = v.backups_task().unwrap();
        let before = task.overview();
        assert_eq!(before.backups.len(), 2);
        assert_eq!(before.older(), 0);
        assert_eq!(before.location, dir.join("Backups"));
        assert!(!before.chosen && before.unavailable.is_none());
        assert!(
            before
                .open
                .as_ref()
                .is_some_and(|p| p.starts_with(dir.join("Backups")) && p.is_dir())
        );
        let cfg = v.backup_config().unwrap().clone();
        let folder = cfg
            .open_folder(
                &cfg.default_known_root(),
                v.path(),
                &backups::vault_id_of(&v.db),
                &mut BackupSettings::default(),
                false,
            )
            .unwrap()
            .unwrap();
        let old_gen = v.generation().to_owned();
        folder
            .with_index(|index| {
                index.gens.get_mut(&old_gen).unwrap().keyx = Some("old-backup.keyx".into())
            })
            .unwrap();

        // A credential change saved directly (no re-encryption): the earlier copies stay on the
        // old password.
        v.set_credentials(creds("new password"));
        v.save().unwrap();
        // Another app's copy: unknown.
        let foreign = folder
            .path()
            .join("v 2026-01-01 10-00-00 #000900 imported.kdbx");
        fs::copy(dir.join("v.kdbx"), &foreign).unwrap();

        let mut task = v.backups_task().unwrap();
        let after = task.overview();
        assert_eq!(after.older(), 2, "{:#?}", after.backups);
        let older: Vec<_> = after.backups.iter().filter(|b| b.needs_older()).collect();
        assert!(
            older.iter().all(|b| b
                .key
                .text()
                .starts_with("Older key – needs your backup key file from")
                && b.key.text().ends_with("(old-backup.keyx)")),
            "{:?}",
            older[0].key.text()
        );
        let unknown: Vec<_> = after
            .backups
            .iter()
            .filter(|b| b.key == KeyLabel::Unknown)
            .collect();
        assert_eq!(unknown.len(), 1);
        assert!(
            after
                .backups
                .iter()
                .any(|b| b.kind() == Kind::BeforeKeyChange && b.key == KeyLabel::Current)
        );

        let report = task.delete_older();
        assert_eq!((report.deleted, report.left), (2, 0), "{report:?}");
        assert!(report.errors.is_empty());
        let left = task.overview();
        assert_eq!(left.older(), 0);
        assert_eq!(left.backups.len(), 2, "current and unknown stay");

        let report = task.delete_all();
        assert_eq!(report.deleted, 2);
        assert!(task.overview().backups.is_empty());
        assert!(v.path().is_file(), "the vault itself is never touched");
        drop(v);
        cleanup(&dir);
    }

    /// Saves `path` again under `to`, the way another KeePass app does (the record carried over
    /// unless `drop_record`).
    fn resave_elsewhere(path: &Path, from: &Credentials, to: &Credentials, drop_record: bool) {
        let mut db = Database::parse(&fs::read(path).unwrap(), from.to_key().unwrap()).unwrap();
        if drop_record {
            db.meta.custom_data.remove(crate::generation::RECORD_KEY);
        }
        let mut out = Vec::new();
        db.save(&mut out, to.to_key().unwrap()).unwrap();
        fs::write(path, out).unwrap();
    }

    /// A record lost while the credentials stayed the same (the file restored from before it,
    /// or saved by an app that drops it) starts a new generation at the next unlock; the
    /// earlier generation's backups still open with the current credentials, so they are not
    /// offered as older ones. A key change made outside Axentra does make backups older, and
    /// their label doesn't point at a backup key file nobody made.
    #[test]
    fn only_replaced_credentials_make_backups_older() {
        let dir = temp_dir();
        let mut v = vault(&dir);
        save_change(&mut v, "one");
        save_change(&mut v, "two");
        let path = v.path().to_owned();
        assert_eq!(overview(&v).backups.len(), 2);
        drop(v);
        let reopen = |c: Credentials| {
            let mut v = Vault::open(&path, c).unwrap();
            v.attach_store(config(&dir));
            v.recover_backups();
            v
        };

        resave_elsewhere(&path, &creds("pw"), &creds("pw"), true);
        let mut v = reopen(creds("pw"));
        assert!(!v.outside_change() && !v.generation_committed());
        save_change(&mut v, "three");
        let listed = overview(&v);
        assert_eq!(listed.older(), 0, "{:#?}", listed.backups);
        assert_eq!(
            listed
                .backups
                .iter()
                .filter(|b| b.key == KeyLabel::Unknown)
                .count(),
            2
        );
        let mut task = v.backups_task().unwrap();
        assert_eq!(task.delete_older().deleted, 0);
        drop(v);

        // The master password is changed in KeePass.
        resave_elsewhere(&path, &creds("pw"), &creds("new pw"), false);
        let v = reopen(creds("new pw"));
        assert!(v.outside_change());
        let listed = overview(&v);
        let older: Vec<&Listed> = listed.backups.iter().filter(|b| b.needs_older()).collect();
        assert_eq!(older.len(), 1, "the one from before the change");
        let text = older[0].key.text();
        assert!(
            text.starts_with("Older key – needs the credentials the vault used from"),
            "{text}"
        );
        assert!(!text.contains("backup key file"), "{text}");
        drop(v);
        cleanup(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn migrate_imports_verified_copies_and_removes_old_rings() {
        let dir = temp_dir();
        let mut v = vault(&dir);
        let db = v.path().to_owned();
        let bak = with_suffix(&db, ".bak");
        let bak_tmp = with_suffix(&db, ".bak.tmp");
        fs::copy(&db, &bak).unwrap();
        fs::copy(&db, &bak_tmp).unwrap();
        let bak_bytes = fs::read(&bak).unwrap();
        // Not a database: never offered, never touched.
        fs::write(with_suffix(&db, ".tmp"), b"half a file").unwrap();
        fs::write(envelope::legacy_old_path_for(&db), b"AxentraVault/2\n").unwrap();
        let found = v.old_files();
        assert_eq!(
            found,
            [
                OldFile {
                    path: bak.clone(),
                    database: true
                },
                OldFile {
                    path: bak_tmp.clone(),
                    database: true
                },
                OldFile {
                    path: envelope::legacy_old_path_for(&db),
                    database: false
                },
            ]
        );

        // Another program holds the .bak.tmp open: it is copied, but stays.
        let held = fs::OpenOptions::new()
            .read(true)
            .share_mode(1) // FILE_SHARE_READ: no deleting meanwhile.
            .open(&bak_tmp)
            .unwrap();
        let report = v.migrate_old_files().unwrap();
        assert_eq!(report.moved, [bak.as_path()]);
        assert_eq!(report.removed, [envelope::legacy_old_path_for(&db)]);
        assert_eq!(report.kept.len(), 1);
        assert_eq!(report.kept[0].0, bak_tmp);
        assert!(report.failed.is_empty());
        assert!(!bak.exists() && bak_tmp.exists());
        assert!(with_suffix(&db, ".tmp").exists());
        let listed = overview(&v);
        let imported: Vec<_> = listed
            .backups
            .iter()
            .filter(|b| b.kind() == Kind::Imported)
            .collect();
        assert_eq!(imported.len(), 1, "one copy of the identical contents");
        assert_eq!(
            imported[0].backup.sha256.as_deref(),
            Some(&*sha256_hex(&bak_bytes))
        );
        assert_eq!(imported[0].key, KeyLabel::Unknown);

        // Once it is free, a second move only deletes it: the copy is already there.
        drop(held);
        let report = v.migrate_old_files().unwrap();
        assert_eq!(report.moved, [bak_tmp.as_path()]);
        assert!(v.old_files().is_empty());
        assert_eq!(
            overview(&v)
                .backups
                .iter()
                .filter(|b| b.kind() == Kind::Imported)
                .count(),
            1
        );

        assert!(!v.migration_declined());
        v.decline_migration();
        assert!(v.migration_declined());
        drop(v);
        cleanup(&dir);
    }

    #[test]
    fn new_root_is_checked_and_set_with_a_marker() {
        let dir = temp_dir();
        let mut v = vault(&dir);
        let db = v.path().to_owned();
        let cfg = v.backup_config().unwrap().clone();
        let none: &[PathBuf] = &[];
        assert!(check_new_root(&dir.join("missing"), &db, &cfg, none).is_err());
        let own = check_new_root(&dir, &db, &cfg, none).unwrap_err();
        assert!(own.contains("holds the vault"), "{own}");
        let inside = dir.join("Backups").join("inner");
        fs::create_dir_all(&inside).unwrap();
        assert!(check_new_root(&inside, &db, &cfg, none).is_err());

        let usb = temp_dir();
        assert_eq!(check_new_root(&usb, &db, &cfg, none), Ok(Vec::new()));
        let synced = usb.join("Dropbox");
        fs::create_dir_all(&synced).unwrap();
        assert_eq!(
            check_new_root(&synced, &db, &cfg, none),
            Ok(vec![RootWarning::Synced("Dropbox")])
        );
        assert_eq!(
            check_new_root(&usb, &db, &cfg, std::slice::from_ref(&usb)),
            Ok(vec![RootWarning::Synced("OneDrive")])
        );
        // Other vaults on the drive: they stay vaults, but the choice is confirmed first.
        let portable = temp_dir();
        fs::create_dir_all(portable.join("KeePass")).unwrap();
        fs::write(portable.join("KeePass").join("Portable.kdbx"), b"db").unwrap();
        assert_eq!(
            check_new_root(&portable, &db, &cfg, none),
            Ok(vec![RootWarning::HoldsDatabases("Portable.kdbx".into())])
        );
        cleanup(&portable);

        v.set_backup_root(Some(&usb)).unwrap();
        assert!(backups::read_marker(&usb).is_some());
        let cfg = v.backup_config().unwrap().clone();
        assert_eq!(cfg.home().path, usb);
        save_change(&mut v, "goes to the chosen folder");
        let listed = overview(&v);
        assert!(listed.chosen && listed.unavailable.is_none());
        assert_eq!(listed.backups.len(), 1);
        assert!(listed.backups[0].backup.path.starts_with(&usb));

        // Back to the default: the chosen folder becomes a previous root, still listed.
        v.set_backup_root(Some(&dir.join("Backups"))).unwrap();
        let cfg = v.backup_config().unwrap().clone();
        assert!(cfg.root.is_none());
        assert_eq!(cfg.previous_roots.len(), 1);
        save_change(&mut v, "goes to the default folder");
        assert_eq!(overview(&v).backups.len(), 2);

        // Forgotten, the earlier folder is no longer listed; its backups stay where they are.
        assert!(v.forget_backup_root(&usb).unwrap());
        assert!(!v.forget_backup_root(&usb).unwrap());
        assert!(v.backup_config().unwrap().previous_roots.is_empty());
        let listed = overview(&v);
        assert_eq!(listed.backups.len(), 1);
        assert!(listed.unreachable.is_empty());
        assert!(
            fs::read_dir(&usb)
                .unwrap()
                .flatten()
                .any(|e| e.path().is_dir())
        );

        // With a settings file, the choice is written there.
        let file = dir.join(backups::SETTINGS_FILE);
        let mut w = Vault::open(&db, creds("pw")).unwrap();
        w.attach_store(BackupConfig::load(dir.join("Backups"), &file));
        w.set_backup_root(Some(&usb)).unwrap();
        let saved = BackupSettings::load(&file);
        assert_eq!(saved.root.map(|r| r.path), Some(usb.clone()));
        drop((v, w));
        cleanup(&usb);
        cleanup(&dir);
    }
}
