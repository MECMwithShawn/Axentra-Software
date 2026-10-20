//! Encrypted account backups (Settings › Accounts › Backup).
//!
//! A backup is one file, `axentra-outlook-backup-YYYYMMDD-HHMMSS.axobk`
//! (UTC time), in `<dest>/Axentra/Outlook Backups/`, where `<dest>` is
//! normally the OneDrive folder that the separate Axentra OneDrive app syncs
//! to the cloud ([`dest`]). It holds the account setups — never passwords
//! or tokens, which stay in the OS keyring — the settings, and the data
//! that exists only on this device ([`collect()`]). The file is encrypted
//! with a key derived from the backup password ([`format`]).
//!
//! Restoring ([`restore()`]) merges a backup into the current store and
//! skips what is already there, so restoring twice adds nothing.

mod collect;
pub mod dest;
mod format;
mod payload;
mod restore;
#[cfg(test)]
mod tests;

use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::{AccountKind, Store};

pub use collect::{ACCOUNT_KV, KV_ALLOWLIST, KV_STRUCTURAL, collect, kv_allowed, looks_secret, strip_secrets};
pub use format::{FORMAT_VERSION, KdfParams};
pub use payload::*;
pub use restore::{Preview, PreviewAccount, RestoreOptions, RestoreSummary, preview, restore};

/// File names: `axentra-outlook-backup-YYYYMMDD-HHMMSS.axobk`.
pub const FILE_PREFIX: &str = "axentra-outlook-backup-";
pub const FILE_EXT: &str = ".axobk";
/// How often automatic backups run.
pub const AUTO_INTERVAL_DAYS: i64 = 30;
/// Default number of backups kept.
pub const DEFAULT_KEEP: u32 = 12;
pub const MIN_PASSWORD_LEN: usize = 8;

const KV_LAST: &str = "backup.last";
const KV_FAILURE: &str = "backup.last_failure";
/// Paths of the backup files this device wrote: retention only ever
/// deletes these, so backups made by another computer sharing the OneDrive
/// folder (or before a reinstall) are left alone.
const KV_WRITTEN: &str = "backup.written";
/// At most this many paths are remembered.
const WRITTEN_MAX: usize = 400;
/// A remembered backup time further in the future than this can't be right
/// (the clock was wrong when it was recorded), so it's treated as unknown.
const CLOCK_SLACK_HOURS: i64 = 24;

#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    /// Wrong password, or the file was changed or damaged (AES-GCM can't
    /// tell these apart).
    #[error("This backup couldn't be opened. Check the password.")]
    CantOpen,
    #[error("This backup was made by a newer version of Axentra Outlook.")]
    NewerVersion,
    #[error("This file isn't an Axentra Outlook backup.")]
    NotABackup,
    #[error("Use a backup password with at least 8 characters.")]
    WeakPassword,
    /// A complete sentence about a file system problem.
    #[error("{0}")]
    Io(String),
    #[error("{0}")]
    Store(#[from] crate::Error),
}

pub type BackupResult<T> = std::result::Result<T, BackupError>;

/// The file name for a backup made at `at`.
pub fn file_name(at: DateTime<Utc>) -> String {
    format!("{FILE_PREFIX}{}{FILE_EXT}", at.format("%Y%m%d-%H%M%S"))
}

/// When a backup was made, if `name` is exactly a backup file name.
pub fn parse_file_name(name: &str) -> Option<DateTime<Utc>> {
    let stamp = name.strip_prefix(FILE_PREFIX)?.strip_suffix(FILE_EXT)?;
    let b = stamp.as_bytes();
    let shape_ok = b.len() == 15 && b[8] == b'-' && b.iter().enumerate().all(|(i, c)| i == 8 || c.is_ascii_digit());
    if !shape_ok {
        return None;
    }
    NaiveDateTime::parse_from_str(stamp, "%Y%m%d-%H%M%S").ok().map(|n| n.and_utc())
}

/// A backup file found in the backup folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackupFile {
    pub path: PathBuf,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub size: u64,
}

/// Backups in `dir`, newest first. Only regular files whose names match
/// the backup pattern are listed; a missing folder lists nothing.
pub fn list_backups(dir: &Path) -> BackupResult<Vec<BackupFile>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(BackupError::Io(format!("Couldn't open {}: {e}", dir.display()))),
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(created_at) = parse_file_name(&name) else { continue };
        let Ok(meta) = std::fs::symlink_metadata(entry.path()) else { continue };
        if !meta.is_file() {
            continue;
        }
        out.push(BackupFile { path: entry.path(), name, created_at, size: meta.len() });
    }
    out.sort_by(|a, b| b.name.cmp(&a.name));
    Ok(out)
}

/// After writing `just_written`, keep it and the newest of the other
/// backups in `dir` that `ours` accepts, `keep` in all, and delete the
/// older ones. The file just written is never deleted, even when other
/// names sort after it (a clock that was ahead), and nothing but files
/// matching the backup name pattern is ever touched. Returns the deleted
/// paths.
pub fn apply_retention(dir: &Path, keep: u32, just_written: &Path, ours: impl Fn(&BackupFile) -> bool) -> BackupResult<Vec<PathBuf>> {
    let mut deleted = Vec::new();
    let older = list_backups(dir)?.into_iter().filter(|b| b.path != just_written && ours(b));
    for old in older.skip(keep.max(1) as usize - 1) {
        std::fs::remove_file(&old.path).map_err(|e| BackupError::Io(format!("Couldn't remove the old backup {}: {e}", old.name)))?;
        deleted.push(old.path);
    }
    Ok(deleted)
}

/// The backup files this device wrote (see [`KV_WRITTEN`]).
fn written_paths(store: &Store) -> Vec<String> {
    store.kv_get(KV_WRITTEN).ok().flatten().and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default()
}

/// Remember `path` as written here and forget files that are gone.
fn remember_written(store: &Store, mut paths: Vec<String>, path: &Path) -> crate::Result<()> {
    paths.retain(|p| Path::new(p).exists());
    let p = path.display().to_string();
    if !paths.contains(&p) {
        paths.push(p);
    }
    let excess = paths.len().saturating_sub(WRITTEN_MAX);
    paths.drain(..excess);
    store.kv_set(KV_WRITTEN, &serde_json::to_string(&paths)?)
}

/// Delete one backup file (only a regular file with a backup file name).
pub fn delete_backup(path: &Path) -> BackupResult<()> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let meta = std::fs::symlink_metadata(path).map_err(|e| BackupError::Io(format!("Couldn't find {name}: {e}")))?;
    if parse_file_name(&name).is_none() || !meta.is_file() {
        return Err(BackupError::NotABackup);
    }
    std::fs::remove_file(path).map_err(|e| BackupError::Io(format!("Couldn't delete {name}: {e}")))
}

/// Write `bytes` as a new backup file in `dir`: first `<name>.partial`,
/// flushed to disk, then renamed. The partial file is removed on error.
fn write_atomically(dir: &Path, at: DateTime<Utc>, bytes: &[u8]) -> BackupResult<PathBuf> {
    let io = |e: std::io::Error| BackupError::Io(format!("Couldn't save the backup in {}: {e}", dir.display()));
    std::fs::create_dir_all(dir).map_err(io)?;
    // Two backups in the same second get the next free second.
    let (path, partial) = (0..120)
        .map(|i| {
            let name = file_name(at + Duration::seconds(i));
            (dir.join(&name), dir.join(format!("{name}.partial")))
        })
        .find(|(p, t)| !p.exists() && !t.exists())
        .ok_or_else(|| BackupError::Io(format!("Couldn't find a free file name in {}.", dir.display())))?;
    let write = || -> std::io::Result<()> {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        let mut f = opts.open(&partial)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&partial, &path)?;
        // Make the rename itself durable (best effort).
        if let Ok(d) = std::fs::File::open(dir) {
            let _ = d.sync_all();
        }
        Ok(())
    };
    if let Err(e) = write() {
        let _ = std::fs::remove_file(&partial);
        return Err(io(e));
    }
    Ok(path)
}

/// A finished backup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Written {
    pub path: PathBuf,
    pub size: u64,
    pub created_at: DateTime<Utc>,
    pub counts: Counts,
    /// Old backups removed by retention.
    pub removed: usize,
}

/// Back up the store into `dir` with the default key-derivation cost, then
/// keep only the newest `keep` backups.
pub fn create_backup(store: &Store, dir: &Path, password: &str, now: DateTime<Utc>, keep: u32) -> BackupResult<Written> {
    create_backup_with(store, dir, password, now, keep, KdfParams::DEFAULT)
}

/// [`create_backup`] with explicit Argon2id parameters.
pub fn create_backup_with(store: &Store, dir: &Path, password: &str, now: DateTime<Utc>, keep: u32, kdf: KdfParams) -> BackupResult<Written> {
    if password.chars().count() < MIN_PASSWORD_LEN {
        return Err(BackupError::WeakPassword);
    }
    let payload = collect(store, now)?;
    let counts = payload.manifest.counts.clone();
    let json = Zeroizing::new(serde_json::to_vec(&payload).map_err(crate::Error::from)?);
    drop(payload);
    let sealed = format::seal(&json, password, kdf)?;
    drop(json);
    let path = write_atomically(dir, now, &sealed)?;
    let mine = written_paths(store);
    let removed = match apply_retention(dir, keep, &path, |b| mine.iter().any(|p| Path::new(p) == b.path)) {
        Ok(d) => d.len(),
        Err(e) => {
            tracing::warn!("backup retention: {e}");
            0
        }
    };
    if let Err(e) = remember_written(store, mine, &path) {
        tracing::warn!("backup list: {e}");
    }
    Ok(Written { path, size: sealed.len() as u64, created_at: now, counts, removed })
}

/// Read and decrypt a backup file.
pub fn open_backup(path: &Path, password: &str) -> BackupResult<Payload> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let bytes = std::fs::read(path).map_err(|e| BackupError::Io(format!("Couldn't read {name}: {e}")))?;
    open_bytes(&bytes, password)
}

/// Decrypt a backup held in memory.
pub fn open_bytes(bytes: &[u8], password: &str) -> BackupResult<Payload> {
    let plain = format::open(bytes, password)?;
    let payload: Payload = serde_json::from_slice(&plain).map_err(|_| BackupError::CantOpen)?;
    if payload.manifest.format != FORMAT_NAME {
        return Err(BackupError::NotABackup);
    }
    if payload.manifest.format_version > FORMAT_VERSION {
        return Err(BackupError::NewerVersion);
    }
    Ok(payload)
}

/// The plaintext JSON a backup of `store` would contain (for tests and
/// diagnostics; never written to disk unencrypted).
pub fn plaintext(store: &Store, now: DateTime<Utc>) -> BackupResult<Zeroizing<Vec<u8>>> {
    let payload = collect(store, now)?;
    Ok(Zeroizing::new(serde_json::to_vec(&payload).map_err(crate::Error::from)?))
}

// ---------------------------------------------------------------- schedule

/// The last successful backup (store key/value, never backed up itself).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastBackup {
    pub at: DateTime<Utc>,
    pub size: u64,
    pub path: String,
}

/// The last failed attempt, and the day its error toast was shown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastFailure {
    pub at: DateTime<Utc>,
    pub message: String,
    #[serde(default)]
    pub notified_on: Option<NaiveDate>,
}

pub fn last_backup(store: &Store) -> Option<LastBackup> {
    store.kv_get(KV_LAST).ok().flatten().and_then(|j| serde_json::from_str(&j).ok())
}

pub fn last_failure(store: &Store) -> Option<LastFailure> {
    store.kv_get(KV_FAILURE).ok().flatten().and_then(|j| serde_json::from_str(&j).ok())
}

/// Remember a successful backup and clear the last failure.
pub fn record_success(store: &Store, written: &Written) -> crate::Result<()> {
    let last = LastBackup { at: written.created_at, size: written.size, path: written.path.display().to_string() };
    store.kv_set(KV_LAST, &serde_json::to_string(&last)?)?;
    store.kv_set(KV_FAILURE, "null")
}

/// Remember a failed attempt. Returns whether to show an error toast: at
/// most one per day (`today` is the local date).
pub fn record_failure(store: &Store, at: DateTime<Utc>, message: &str, today: NaiveDate) -> crate::Result<bool> {
    let notify = last_failure(store).and_then(|f| f.notified_on) != Some(today);
    let notified_on = if notify { Some(today) } else { last_failure(store).and_then(|f| f.notified_on) };
    let f = LastFailure { at, message: message.to_string(), notified_on };
    store.kv_set(KV_FAILURE, &serde_json::to_string(&f)?)?;
    Ok(notify)
}

/// `last` unless it lies more than a day after `now`: a time recorded
/// while the clock was ahead would otherwise hold off backups for as long.
fn plausible(last: Option<DateTime<Utc>>, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    last.filter(|t| *t <= now + Duration::hours(CLOCK_SLACK_HOURS))
}

/// Whether an automatic backup should run now: automatic backups are on,
/// the password is available without asking, and this device's last
/// success is at least [`AUTO_INTERVAL_DAYS`] old (or there is none, or it
/// is dated in the future).
pub fn is_due(auto: bool, password_available: bool, last_success: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    auto && password_available && plausible(last_success, now).is_none_or(|t| now - t >= Duration::days(AUTO_INTERVAL_DAYS))
}

/// When the next automatic backup is due.
pub fn next_due(last_success: Option<DateTime<Utc>>, now: DateTime<Utc>) -> DateTime<Utc> {
    plausible(last_success, now).map(|t| t + Duration::days(AUTO_INTERVAL_DAYS)).unwrap_or(now)
}

// ---------------------------------------------------------------- password

/// Why a new backup password can't be used yet, if it can't.
pub fn password_problem(password: &str, confirm: &str) -> Option<&'static str> {
    if password.chars().count() < MIN_PASSWORD_LEN {
        Some("Use at least 8 characters.")
    } else if password != confirm {
        Some("The passwords don't match.")
    } else {
        None
    }
}

/// Rough strength: 0 too short, 1 weak, 2 fair, 3 strong.
pub fn password_strength(password: &str) -> u8 {
    let len = password.chars().count();
    if len < MIN_PASSWORD_LEN {
        return 0;
    }
    let classes = [
        password.chars().any(|c| c.is_lowercase()),
        password.chars().any(|c| c.is_uppercase()),
        password.chars().any(|c| c.is_ascii_digit()),
        password.chars().any(|c| !c.is_alphanumeric()),
    ]
    .iter()
    .filter(|b| **b)
    .count();
    let distinct = password.chars().collect::<std::collections::HashSet<_>>().len();
    if distinct < 4 {
        1
    } else if len >= 16 || (len >= 12 && classes >= 3) {
        3
    } else if len >= 10 || classes >= 3 {
        2
    } else {
        1
    }
}

/// Short label for an account kind in the restore preview.
pub fn kind_label(kind: AccountKind) -> &'static str {
    match kind {
        AccountKind::Local => "On this device",
        AccountKind::Imap => "IMAP",
        AccountKind::Microsoft => "Microsoft",
    }
}
