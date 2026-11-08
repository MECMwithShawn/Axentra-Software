//! The backup store as one open vault uses it. Each backup goes through a fallback chain: the
//! root the user chose, then the default root under `%LOCALAPPDATA%`, then, for ordinary saves
//! only, `<db>.bak` beside the database. If nothing works the caller fails its save and the
//! database stays as it was. A credential change never falls back to `<db>.bak`: a copy there
//! syncs along with the database, and the key change needs the store anyway.
//!
//! A fallback raises one notice per session, plus a status line that stays until a backup
//! reaches the first-choice root again. A `<db>.bak` written as a fallback is moved into the
//! store (copy, verify, delete) once a store write works again.

use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::SystemTime;

use crate::backups::{
    self, BackupConfig, BackupSettings, Folder, Kind, KnownRoot, RootKind, RootSetting,
    WriteOutcome,
};
use crate::fsops;
use crate::limits::{DATABASE_MAX, read_capped};
use crate::vault::with_suffix;

/// Serializes read-modify-write of the settings file within this process (the app runs once
/// per user).
static SETTINGS_LOCK: Mutex<()> = Mutex::new(());

/// Where a backup went.
#[derive(Debug, Clone)]
pub enum Written {
    /// Into a store folder; `Duplicate` when its newest backup already held these bytes.
    Store {
        folder: Folder,
        outcome: WriteOutcome,
    },
    /// To `<db>.bak`, because no store root worked.
    Bak(PathBuf),
}

impl Written {
    pub fn path(&self) -> &Path {
        match self {
            Written::Store { outcome, .. } => outcome.path(),
            Written::Bak(path) => path,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    /// Every place tried, and why each failed.
    #[error("{}", list(.0))]
    Failed(Vec<(PathBuf, String)>),
    /// The vault has no store, and the backup may not go to `<db>.bak`.
    #[error("no backup folder is set up for this vault")]
    NoStore,
}

fn list(failed: &[(PathBuf, String)]) -> String {
    failed
        .iter()
        .map(|(place, why)| format!("{}: {why}", place.display()))
        .collect::<Vec<_>>()
        .join("; ")
}

/// One backup to write.
pub(crate) struct Request<'a> {
    pub bytes: &'a [u8],
    pub kind: Kind,
    /// The version's own modification time, which names the backup.
    pub source_mtime: Option<SystemTime>,
    /// The credential generation known to open the bytes, and whether it is committed.
    pub generation: Option<&'a str>,
    pub committed: bool,
    pub db_path: &'a Path,
    /// The database's size, for the free-space check.
    pub db_len: u64,
    pub vault_id: &'a str,
    /// `<db>.bak` may be the last resort: an ordinary save, not a credential change.
    pub allow_bak: bool,
}

/// A vault's view of the backup store: its config, the notices fallbacks raised this session
/// and the `<db>.bak` it may have had to write.
pub(crate) struct Store {
    config: BackupConfig,
    /// The settings when the config names no settings file.
    settings: BackupSettings,
    notice: Option<String>,
    notice_given: bool,
    status: Option<String>,
    /// The `<db>.bak` written as a fallback this session: its hash and the generation that
    /// opens it. Also recorded in the default root's index when that can be reached.
    fallback_bak: Option<(String, Option<String>)>,
    /// The last retention warning (retention deleted nothing in a folder).
    warning: Option<String>,
}

impl Store {
    pub(crate) fn new(config: BackupConfig) -> Store {
        Store {
            config,
            settings: BackupSettings::default(),
            notice: None,
            notice_given: false,
            status: None,
            fallback_bak: None,
            warning: None,
        }
    }

    pub(crate) fn config(&self) -> &BackupConfig {
        &self.config
    }

    /// The fallback notice, given once per session.
    pub(crate) fn take_notice(&mut self) -> Option<String> {
        self.notice.take()
    }

    pub(crate) fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }

    pub(crate) fn warning(&self) -> Option<&str> {
        self.warning.as_deref()
    }

    /// Runs `f` on the settings: the settings file re-read and, if `f` changed them, written
    /// back (unless it couldn't be read, so a damaged or locked file never loses its roots), or
    /// the in-memory settings.
    fn with_settings<R>(&mut self, f: impl FnOnce(&BackupConfig, &mut BackupSettings) -> R) -> R {
        let Some(file) = self.config.settings_file.clone() else {
            return f(&self.config, &mut self.settings);
        };
        let _guard = SETTINGS_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        let (mut settings, writable) = match BackupSettings::read(&file) {
            Ok(found) => (found.unwrap_or_default(), true),
            Err(_) => (BackupSettings::default(), false),
        };
        let before = settings.clone();
        let result = f(&self.config, &mut settings);
        if writable && settings != before {
            // Losing this only costs a search of the root next time.
            let _ = settings.save(&file);
        }
        result
    }

    fn open_folder(
        &mut self,
        root: &KnownRoot,
        db_path: &Path,
        vault_id: &str,
        create: bool,
    ) -> Result<Option<Folder>, backups::BackupError> {
        // Checked before the settings lock is taken, so a slow root never holds up another
        // thread's use of the settings; the check inside then reuses this answer.
        if root.kind != RootKind::Default {
            self.config
                .check_root(root)
                .map_err(backups::BackupError::Unavailable)?;
        }
        self.with_settings(|config, settings| {
            config.open_folder(root, db_path, vault_id, settings, create)
        })
    }

    /// Writes one backup through the fallback chain; see the module docs.
    pub(crate) fn write(&mut self, req: &Request<'_>) -> Result<Written, StoreError> {
        let home = self.config.home();
        let mut roots = vec![home.clone()];
        if home.kind != RootKind::Default {
            roots.push(self.config.default_known_root());
        }
        let mut failed: Vec<(PathBuf, String)> = Vec::new();
        for root in &roots {
            match self.try_root(root, req) {
                Ok((folder, outcome)) => {
                    if failed.is_empty() {
                        self.status = None;
                    } else {
                        self.fell_back(format!(
                            "The backup folder {} can't be used: {}. Backups go to {} meanwhile.",
                            failed[0].0.display(),
                            failed[0].1,
                            root.path.display()
                        ));
                    }
                    // Not from a credential change's own copy: the `.bak` holds the old
                    // credentials, and the key change re-encrypts and removes it itself.
                    if req.kind != Kind::BeforeKeyChange {
                        self.import_fallback_bak(&folder, req.db_path, req.vault_id);
                    }
                    return Ok(Written::Store { folder, outcome });
                }
                Err(why) => failed.push((root.path.clone(), why)),
            }
        }
        if req.allow_bak {
            let bak = with_suffix(req.db_path, ".bak");
            match write_bak(&bak, req.bytes, req.source_mtime) {
                Ok(()) => {
                    self.record_fallback_bak(req);
                    self.fell_back(format!(
                        "The backup folder {} can't be used: {}. The previous version was kept as {}.",
                        failed[0].0.display(),
                        failed[0].1,
                        bak.display()
                    ));
                    return Ok(Written::Bak(bak));
                }
                Err(e) => failed.push((bak, e.to_string())),
            }
        }
        let error = StoreError::Failed(failed);
        self.fell_back(format!("No backup could be written ({error})."));
        Err(error)
    }

    /// Makes `root` the backup root (None: the default root). A chosen folder gets its marker
    /// first (one it already carries is kept); the old root joins the previous roots, whose
    /// backups stay where they are and are still listed and pruned. Unlike other settings
    /// updates this fails, changing nothing, when the settings file can't be read or written.
    pub(crate) fn set_root(&mut self, root: Option<&Path>) -> io::Result<()> {
        let setting = match root {
            Some(path) => Some(RootSetting {
                path: path.to_owned(),
                marker: backups::ensure_marker(path)?,
            }),
            None => None,
        };
        let settings = match self.config.settings_file.clone() {
            Some(file) => {
                let _guard = SETTINGS_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
                let mut settings = BackupSettings::read(&file)?.unwrap_or_default();
                settings.set_root(setting);
                settings.save(&file)?;
                settings
            }
            None => {
                self.settings.set_root(setting);
                self.settings.clone()
            }
        };
        self.config.root = settings.root;
        self.config.previous_roots = settings.previous_roots;
        // A new first choice: fallback notices start over.
        self.status = None;
        Ok(())
    }

    /// Stops using a previous root (see [`BackupSettings::forget_root`]). Like `set_root`, this
    /// fails, changing nothing, when the settings file can't be read or written.
    pub(crate) fn forget_root(&mut self, root: &Path) -> io::Result<bool> {
        let (settings, forgotten) = match self.config.settings_file.clone() {
            Some(file) => {
                let _guard = SETTINGS_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
                let mut settings = BackupSettings::read(&file)?.unwrap_or_default();
                let forgotten = settings.forget_root(root);
                if forgotten {
                    settings.save(&file)?;
                }
                (settings, forgotten)
            }
            None => {
                let forgotten = self.settings.forget_root(root);
                (self.settings.clone(), forgotten)
            }
        };
        self.config.root = settings.root;
        self.config.previous_roots = settings.previous_roots;
        Ok(forgotten)
    }

    /// Whether the owner answered "Don't ask again" to moving this vault's old files.
    pub(crate) fn migration_declined(&mut self, vault_id: &str) -> bool {
        self.with_settings(|_, settings| {
            settings.migration_declined.iter().any(|id| id == vault_id)
        })
    }

    pub(crate) fn decline_migration(&mut self, vault_id: &str) {
        self.with_settings(|_, settings| {
            if !settings.migration_declined.iter().any(|id| id == vault_id) {
                settings.migration_declined.push(vault_id.to_owned());
            }
        });
    }

    /// A second view of the same store for a worker thread: the same config and settings, its
    /// own notices.
    pub(crate) fn detached(&self) -> Store {
        Store {
            settings: self.settings.clone(),
            ..Store::new(self.config.clone())
        }
    }

    /// This vault's existing folders in every known root that answers, and the roots that
    /// don't. Creates nothing.
    pub(crate) fn existing_folders(
        &mut self,
        db_path: &Path,
        vault_id: &str,
    ) -> (Vec<Folder>, Vec<(KnownRoot, backups::Unavailable)>) {
        // As in `open_folder`: every root is checked before the settings lock is taken.
        for root in self.config.known_roots() {
            if root.kind != RootKind::Default {
                let _ = self.config.check_root(&root);
            }
        }
        self.with_settings(|config, settings| config.existing_folders(db_path, vault_id, settings))
    }

    /// The folder a copy of `len` bytes goes to now: the first-choice root's, else the default
    /// root's (created if needed). Never `<db>.bak`.
    pub(crate) fn writable_folder(
        &mut self,
        db_path: &Path,
        vault_id: &str,
        len: u64,
    ) -> Result<Folder, StoreError> {
        let home = self.config.home();
        let mut roots = vec![home.clone()];
        if home.kind != RootKind::Default {
            roots.push(self.config.default_known_root());
        }
        let mut failed = Vec::new();
        for root in &roots {
            match self.folder_for(root, db_path, vault_id, len, 0) {
                Ok(folder) => return Ok(folder),
                Err(why) => failed.push((root.path.clone(), why)),
            }
        }
        Err(StoreError::Failed(failed))
    }

    /// One root: its folder for this vault (created if needed) and the free-space check.
    fn folder_for(
        &mut self,
        root: &KnownRoot,
        db_path: &Path,
        vault_id: &str,
        len: u64,
        db_len: u64,
    ) -> Result<Folder, String> {
        let folder = self
            .open_folder(root, db_path, vault_id, true)
            .map_err(|e| match e {
                // Just why: the message around it says it is about the backup folder.
                backups::BackupError::Unavailable(why) => why.to_string(),
                backups::BackupError::Io(e) => e.to_string(),
            })?
            .ok_or_else(|| "no backup folder".to_owned())?;
        if !self
            .config
            .has_room(&root.path, len, Some((db_path, db_len)))
        {
            return Err("not enough free space".into());
        }
        Ok(folder)
    }

    /// One root: its folder for this vault (created if needed), the free-space check, the write.
    fn try_root(
        &mut self,
        root: &KnownRoot,
        req: &Request<'_>,
    ) -> Result<(Folder, WriteOutcome), String> {
        let folder = self.folder_for(
            root,
            req.db_path,
            req.vault_id,
            req.bytes.len() as u64,
            req.db_len,
        )?;
        let outcome = folder
            .write_backup(req.bytes, req.kind, req.source_mtime, req.generation)
            .map_err(|e| e.to_string())?;
        if let Some(generation) = req.generation {
            // A cache too: failing to note it only loses the generation's date.
            let _ = folder.with_index(|index| index.note_generation(generation, req.committed));
        }
        Ok((folder, outcome))
    }

    fn fell_back(&mut self, message: String) {
        if !self.notice_given {
            self.notice_given = true;
            self.notice = Some(message.clone());
        }
        self.status = Some(message);
    }

    /// This vault's existing folder in the default root, if it is reachable.
    fn default_folder(&mut self, db_path: &Path, vault_id: &str) -> Option<Folder> {
        let root = self.config.default_known_root();
        self.open_folder(&root, db_path, vault_id, false)
            .ok()
            .flatten()
    }

    fn record_fallback_bak(&mut self, req: &Request<'_>) {
        let sha256 = backups::sha256_hex(req.bytes);
        if let Some(folder) = self.default_folder(req.db_path, req.vault_id) {
            let _ = folder.with_index(|index| index.fallback_bak_sha256 = Some(sha256.clone()));
        }
        self.fallback_bak = Some((sha256, req.generation.map(str::to_owned)));
    }

    /// Forgets the `<db>.bak` a fallback wrote, here and in the default root's index, so it is
    /// never moved into the store: a key change re-encrypted it, and the file left on the old
    /// credentials is only to be deleted.
    pub(crate) fn forget_fallback_bak(&mut self, db_path: &Path, vault_id: &str) {
        self.fallback_bak = None;
        if let Some(folder) = self.default_folder(db_path, vault_id) {
            let _ = folder.with_index(|index| index.fallback_bak_sha256 = None);
        }
    }

    /// Moves the `<db>.bak` a fallback wrote into `folder`: a copy, read back and compared,
    /// then the original goes. Only those exact bytes are moved; any other `.bak` is left alone.
    fn import_fallback_bak(&mut self, folder: &Folder, db_path: &Path, vault_id: &str) {
        let bak = with_suffix(db_path, ".bak");
        if !bak.is_file() {
            self.fallback_bak = None;
            return;
        }
        let (expected, generation) = match self.fallback_bak.clone() {
            Some(known) => known,
            None => {
                let recorded = self
                    .default_folder(db_path, vault_id)
                    .and_then(|f| f.index().ok())
                    .and_then(|index| index.fallback_bak_sha256);
                match recorded {
                    Some(sha256) => (sha256, None),
                    None => return,
                }
            }
        };
        let Ok(bytes) = read_capped(&bak, DATABASE_MAX) else {
            return;
        };
        if backups::sha256_hex(&bytes) != expected {
            return;
        }
        let mtime = fs::metadata(&bak).and_then(|m| m.modified()).ok();
        let Ok(outcome) = folder.write_backup(&bytes, Kind::Imported, mtime, generation.as_deref())
        else {
            return;
        };
        if backups::sha256_file(outcome.path()).ok().as_deref() != Some(expected.as_str()) {
            return;
        }
        // The copy is in the store: forget the hash even if the original can't go yet, so it
        // is never imported twice.
        self.fallback_bak = None;
        if let Some(default) = self.default_folder(db_path, vault_id) {
            let _ = default.with_index(|index| {
                if index.fallback_bak_sha256.as_deref() == Some(expected.as_str()) {
                    index.fallback_bak_sha256 = None;
                }
            });
        }
        let _ = fsops::remove_file(&bak);
    }

    /// Marks `generation` as replaced by a credential change in this vault's folder in every
    /// known root that answers (see [`backups::Index::retire`]). A cache: failures are ignored.
    pub(crate) fn retire_generation(&mut self, db_path: &Path, vault_id: &str, generation: &str) {
        let (folders, _) = self.existing_folders(db_path, vault_id);
        for folder in &folders {
            let _ = folder.with_index(|index| index.retire(generation));
        }
    }

    /// Applies retention to this vault's folder in every known root that answers, keeping ring
    /// snapshots of `session_gen` only (all of them without one; see [`Folder::prune`]). Never
    /// fails: a folder where retention deleted nothing leaves a warning.
    pub(crate) fn prune(&mut self, db_path: &Path, vault_id: &str, session_gen: Option<&str>) {
        let (folders, _) = self.existing_folders(db_path, vault_id);
        let mut warning = None;
        for folder in &folders {
            if let Some(w) = folder.prune(session_gen).warning() {
                warning.get_or_insert(w);
            }
        }
        self.warning = warning;
    }
}

/// Replaces `<db>.bak` all at once: a synced temp file beside it, then a rename that replaces
/// it and returns once that is on disk. It gets the version's own time, as a store backup does.
pub(crate) fn write_bak(bak: &Path, bytes: &[u8], mtime: Option<SystemTime>) -> io::Result<()> {
    let tmp = fsops::write_temp_beside(bak, bytes)?;
    fsops::move_replace(&tmp, bak).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })?;
    if let Some(mtime) = mtime.filter(|t| *t <= SystemTime::now()) {
        // Only the name of a later import depends on it.
        let _ = OpenOptions::new()
            .write(true)
            .open(bak)
            .and_then(|f| f.set_modified(mtime));
    }
    Ok(())
}
