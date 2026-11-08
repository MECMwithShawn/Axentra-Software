//! The versioned backup store. Before a save replaces the database, the previous version is
//! copied into a per-vault folder under a backup root, so earlier versions survive bad edits,
//! sync conflicts and lost devices without a `.bak` beside the database (which would sync along
//! with it).
//!
//! Layout: `<root>\<stem>[ (n)]\` holds `vault.json` (which vault the folder belongs to; no path
//! is stored in the store), `index.json` (a rebuildable cache of hashes and credential
//! generations), `rekey.json` (only during or after an interrupted key change), the backups and
//! ring snapshots. Backups are named
//! `<stem> YYYY-MM-DD HH-MM-SS #<seq>[~<rev>][ <kind>].kdbx` and ordered by `seq`, never by the
//! clock. Files are published without ever replacing one and are then marked read-only. Files
//! whose names don't parse are foreign and never touched.
//!
//! Roots: the default root under `%LOCALAPPDATA%` (which the app may create), a root the user
//! chose (never created by the app; it must carry a marker file matching the settings), and any
//! previous root, which is still listed and pruned.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::time::{Duration, Instant, SystemTime};

use chrono::{DateTime, Local, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::fsops;
use crate::limits::read_capped;
use crate::random_bytes;

/// Marks a folder the user chose as an Axentra backup root: `{"marker":"<uuid>"}`.
pub const MARKER_FILE: &str = ".axentra-backups";
pub const VAULT_FILE: &str = "vault.json";
pub const INDEX_FILE: &str = "index.json";
pub const MANIFEST_FILE: &str = "rekey.json";
/// The settings file's name; the app keeps it in `%APPDATA%\Axentra Vault`.
pub const SETTINGS_FILE: &str = "backups.json";
/// The first 8 bytes of every KDBX file.
pub const KDBX_SIGNATURE: [u8; 8] = [0x03, 0xD9, 0xA2, 0x9A, 0x67, 0xFB, 0x4B, 0xB5];
/// Smaller than any real KeePass database.
pub const MIN_KDBX_LEN: u64 = 512;
/// How long a custom root may take to answer before it counts as unavailable.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

const JSON_MAX: u64 = 4 * 1024 * 1024;
const SMALL_JSON_MAX: u64 = 64 * 1024;
const MIB: u64 = 1024 * 1024;
/// Folders looked at when searching a root for a vault's folder.
const MAX_FOLDER_SCAN: usize = 256;
/// Names tried when a backup name is taken between choosing it and publishing.
const PUBLISH_ATTEMPTS: u32 = 16;
/// Temp files younger than this may still be in use.
const TEMP_MAX_AGE: Duration = Duration::from_secs(60 * 60);

/// Retention: the newest backup, the newest 20 regular backups, the newest of each of the last
/// 30 distinct days and 12 distinct months with regular backups, and every pinned backup.
const KEEP_RECENT: usize = 20;
const KEEP_DAYS: usize = 30;
const KEEP_MONTHS: usize = 12;
/// Ring snapshots kept for the current credential generation.
const KEEP_RINGS: usize = 3;

/// `hex(SHA-256("axentra-vault/store-id/v1" ‖ root group UUID))[..32]`: names the vault, not
/// its path, so the folder survives a rename or move, and reveals nothing about the path.
pub fn vault_id(root_group_uuid: &[u8; 16]) -> String {
    let mut h = Sha256::new();
    h.update(b"axentra-vault/store-id/v1");
    h.update(root_group_uuid);
    hex::encode(h.finalize())[..32].to_owned()
}

pub fn vault_id_of(db: &keepass::Database) -> String {
    vault_id(db.root().id().uuid().as_bytes())
}

/// `%LOCALAPPDATA%\Axentra Vault\Backups`, given `%LOCALAPPDATA%`.
pub fn default_root_in(local_app_data: &Path) -> PathBuf {
    local_app_data.join("Axentra Vault").join("Backups")
}

// ---------------------------------------------------------------------------------------------
// Names

/// Why a backup was taken. The word appears in the file name; `Auto` has none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// The previous version, written before a save.
    Auto,
    /// The version found on disk at unlock.
    Opened,
    /// Another device's version, written before "Save anyway" overwrites it.
    ReplacedExternal,
    /// "Back up now".
    Manual,
    /// The version a restore replaced.
    BeforeRestore,
    /// The previous version during a key change, re-encrypted to the new key.
    BeforeKeyChange,
    /// Migrated or recovered `.bak` and `.tmp` files.
    Imported,
}

impl Kind {
    pub const ALL: [Kind; 7] = [
        Kind::Auto,
        Kind::Opened,
        Kind::ReplacedExternal,
        Kind::Manual,
        Kind::BeforeRestore,
        Kind::BeforeKeyChange,
        Kind::Imported,
    ];

    pub fn word(self) -> Option<&'static str> {
        match self {
            Kind::Auto => None,
            Kind::Opened => Some("opened"),
            Kind::ReplacedExternal => Some("replaced-external"),
            Kind::Manual => Some("manual"),
            Kind::BeforeRestore => Some("before-restore"),
            Kind::BeforeKeyChange => Some("before-key-change"),
            Kind::Imported => Some("imported"),
        }
    }

    fn from_word(word: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.word() == Some(word))
    }

    /// Pinned backups are never removed by retention.
    pub fn is_pinned(self) -> bool {
        matches!(
            self,
            Kind::Manual | Kind::BeforeRestore | Kind::BeforeKeyChange | Kind::Imported
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Item {
    /// A copy of the database.
    Kdbx(Kind),
    /// A copy of the passkey file (`.axvault`).
    Ring,
}

/// A parsed backup file name. [`BackupName::parse`] accepts exactly the names
/// [`BackupName::file_name`] writes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BackupName {
    /// Any non-empty text: the database may have been renamed since.
    pub stem: String,
    /// `YYYY-MM-DD` and `HH-MM-SS`: the local time of the version's own modification time. For
    /// display and day/month retention only; order comes from `seq`.
    pub date: String,
    pub time: String,
    pub seq: u64,
    /// 0 for the original, n for its n-th re-encrypted copy (`~n`).
    pub rev: u32,
    pub item: Item,
}

impl BackupName {
    /// Parses from the right, so stems with spaces, dates, `#` or `~` are fine. Anything that
    /// doesn't match exactly (an unknown kind, a `(2)` copy suffix, extra zeros) is foreign.
    pub fn parse(file_name: &str) -> Option<BackupName> {
        let (base, ring) = match file_name.strip_suffix(".kdbx") {
            Some(base) => (base, false),
            None => (file_name.strip_suffix(".axvault")?, true),
        };
        let (rest, last) = base.rsplit_once(' ')?;
        let (rest, seq_token, word) = if last.starts_with('#') {
            (rest, last, None)
        } else {
            let (rest, seq_token) = rest.rsplit_once(' ')?;
            (rest, seq_token, Some(last))
        };
        let (rest, time) = rest.rsplit_once(' ')?;
        let (stem, date) = rest.rsplit_once(' ')?;
        if stem.is_empty() || !shaped(date, "dddd-dd-dd") || !shaped(time, "dd-dd-dd") {
            return None;
        }
        let token = seq_token.strip_prefix('#')?;
        let (seq, rev) = match token.split_once('~') {
            Some((seq, rev)) => (seq, Some(rev)),
            None => (token, None),
        };
        let seq = canonical_number(seq, 6)?;
        let rev = match rev {
            None => 0,
            Some(rev) => u32::try_from(canonical_number(rev, 1)?)
                .ok()
                .filter(|&r| r > 0)?,
        };
        let item = match (ring, word) {
            (false, None) => Item::Kdbx(Kind::Auto),
            (false, Some(word)) => Item::Kdbx(Kind::from_word(word)?),
            (true, Some("passkeys")) if rev == 0 => Item::Ring,
            _ => return None,
        };
        Some(BackupName {
            stem: stem.to_owned(),
            date: date.to_owned(),
            time: time.to_owned(),
            seq,
            rev,
            item,
        })
    }

    pub fn file_name(&self) -> String {
        let mut name = format!("{} {} {} #{:06}", self.stem, self.date, self.time, self.seq);
        if self.rev > 0 {
            name.push_str(&format!("~{}", self.rev));
        }
        match self.item {
            Item::Kdbx(kind) => {
                if let Some(word) = kind.word() {
                    name.push(' ');
                    name.push_str(word);
                }
                name.push_str(".kdbx");
            }
            Item::Ring => name.push_str(" passkeys.axvault"),
        }
        name
    }

    /// `YYYY-MM`.
    pub fn month(&self) -> &str {
        &self.date[..7]
    }

    /// The kind of a database backup; None for a ring snapshot.
    pub fn kind(&self) -> Option<Kind> {
        match self.item {
            Item::Kdbx(kind) => Some(kind),
            Item::Ring => None,
        }
    }

    pub fn is_kdbx(&self) -> bool {
        matches!(self.item, Item::Kdbx(_))
    }
}

/// `pattern` with `d` for any ASCII digit.
fn shaped(s: &str, pattern: &str) -> bool {
    s.len() == pattern.len()
        && s.bytes().zip(pattern.bytes()).all(|(c, p)| match p {
            b'd' => c.is_ascii_digit(),
            _ => c == p,
        })
}

/// Digits written exactly as `{:0min$}` writes their value: at least `min` of them and no
/// extra leading zeros, so every number has one spelling.
fn canonical_number(s: &str, min: usize) -> Option<u64> {
    if s.len() < min || s.len() > 20 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: u64 = s.parse().ok()?;
    (format!("{n:0min$}") == s).then_some(n)
}

/// The database file's stem made safe for file and folder names: `<>:"/\|?*` and control
/// characters become `_`, trailing dots and spaces go, at most 64 characters, and a Windows
/// device name (`CON`, `NUL`, ...) gets a leading `_`.
pub fn sanitize_stem(stem: &str) -> String {
    let replaced: String = stem
        .chars()
        .map(|c| {
            if c.is_control() || r#"<>:"/\|?*"#.contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let capped: String = replaced
        .trim_end_matches(['.', ' '])
        .chars()
        .take(64)
        .collect();
    let mut stem = capped.trim_end_matches(['.', ' ']).to_owned();
    if stem.is_empty() {
        stem = "Vault".to_owned();
    }
    let device = stem.split('.').next().unwrap_or_default().trim_end();
    let upper = device.to_ascii_uppercase();
    let reserved = matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.as_bytes()[3].is_ascii_digit());
    if reserved {
        stem.insert(0, '_');
    }
    stem
}

fn stem_of(db_path: &Path) -> String {
    sanitize_stem(
        &db_path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
    )
}

/// The time a backup is named after: the source version's modification time, or now if that
/// is unknown or in the future.
fn name_time(source_mtime: Option<SystemTime>) -> SystemTime {
    let now = SystemTime::now();
    match source_mtime {
        Some(t) if t <= now => t,
        _ => now,
    }
}

fn local_date_time(t: SystemTime) -> (String, String) {
    let local: DateTime<Local> = t.into();
    (
        local.format("%Y-%m-%d").to_string(),
        local.format("%H-%M-%S").to_string(),
    )
}

fn utc_string(t: SystemTime) -> String {
    DateTime::<Utc>::from(t).to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

// ---------------------------------------------------------------------------------------------
// Paths

/// Case-insensitive comparison key for a Windows path.
fn path_key(path: &Path) -> String {
    #[cfg(target_os="linux")]
    return path.to_string_lossy().trim_end_matches('/').to_owned();
    #[cfg(windows)]
    let mut key = path.to_string_lossy().replace('/', "\\").to_lowercase();
    #[cfg(windows)]
    while key.len() > 3 && key.ends_with('\\') {
        key.pop();
    }
    #[cfg(windows)]
    key
}

fn same_path(a: &Path, b: &Path) -> bool {
    path_key(a) == path_key(b)
}

/// Whether `path` is `root` or lies under it (compared by name, case-insensitively).
pub fn is_within(path: &Path, root: &Path) -> bool {
    if cfg!(target_os="linux") { return path.starts_with(root); }
    let (path, root) = (path_key(path), path_key(root));
    path == root
        || (path.starts_with(&root)
            && (root.ends_with('\\') || path[root.len()..].starts_with('\\')))
}

/// A plain folder name inside a root, nothing that could point elsewhere.
fn valid_folder_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['\\', '/', ':'])
        && !name.chars().any(char::is_control)
}

// ---------------------------------------------------------------------------------------------
// Settings

/// A backup root the user chose, and the marker that must be in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootSetting {
    pub path: PathBuf,
    pub marker: String,
}

/// Which folders hold a database's backups: one in each root it has used, so a root change or
/// a fallback never makes another database with the same vault id (a copy, or a "Save As")
/// take this one's folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "VaultSettingJson")]
pub struct VaultSetting {
    pub id: String,
    pub folders: Vec<RootFolder>,
}

/// A database's folder in one root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootFolder {
    pub root: PathBuf,
    /// The folder's name inside `root`.
    pub folder: String,
}

/// [`VaultSetting`] as stored, also reading the earlier form with one root:
/// `{"id","root","folder"}`.
#[derive(Deserialize)]
struct VaultSettingJson {
    id: String,
    #[serde(default)]
    folders: Vec<RootFolder>,
    #[serde(default)]
    root: Option<PathBuf>,
    #[serde(default)]
    folder: Option<String>,
}

impl From<VaultSettingJson> for VaultSetting {
    fn from(json: VaultSettingJson) -> Self {
        let mut setting = VaultSetting {
            id: json.id,
            folders: Vec::new(),
        };
        let legacy = json
            .root
            .zip(json.folder)
            .map(|(root, folder)| RootFolder { root, folder });
        for f in json.folders.into_iter().chain(legacy) {
            if setting.folder_in(&f.root).is_none() {
                setting.folders.push(f);
            }
        }
        setting
    }
}

impl VaultSetting {
    pub fn new(id: &str) -> VaultSetting {
        VaultSetting {
            id: id.to_owned(),
            folders: Vec::new(),
        }
    }

    /// The folder's name in `root`, if one is recorded.
    pub fn folder_in(&self, root: &Path) -> Option<&str> {
        self.folders
            .iter()
            .find(|f| same_path(&f.root, root))
            .map(|f| f.folder.as_str())
    }

    fn set_folder(&mut self, root: &Path, folder: &str) {
        self.folders.retain(|f| !same_path(&f.root, root));
        self.folders.push(RootFolder {
            root: root.to_owned(),
            folder: folder.to_owned(),
        });
    }
}

/// `backups.json`. Holds no secrets: paths, folder names, vault ids and markers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupSettings {
    /// The chosen root; None means the default root.
    #[serde(default)]
    pub root: Option<RootSetting>,
    /// Roots used before, whose backups are still listed and pruned.
    #[serde(default)]
    pub previous_roots: Vec<RootSetting>,
    /// Lower-cased database path → its folders.
    #[serde(default)]
    pub vaults: BTreeMap<String, VaultSetting>,
    /// Vault ids whose owner answered "Don't ask again" to moving old `.bak` files.
    #[serde(default)]
    pub migration_declined: Vec<String>,
}

impl BackupSettings {
    /// Missing or unreadable settings give the defaults.
    pub fn load(path: &Path) -> BackupSettings {
        Self::read(path).ok().flatten().unwrap_or_default()
    }

    /// The settings in `path`: Ok(None) when there is no file, an error when it can't be read
    /// or parsed (a file that must then not be replaced, or its roots would be forgotten).
    pub fn read(path: &Path) -> io::Result<Option<BackupSettings>> {
        match read_capped(path, JSON_MAX) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Replaces the file all at once, creating its folder if needed.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        fsops::write_replacing(path, &serde_json::to_vec_pretty(self)?)
    }

    /// Chooses a new root (None: the default root). The old one joins `previous_roots`, so its
    /// backups are still found where they are.
    pub fn set_root(&mut self, root: Option<RootSetting>) {
        if let Some(old) = self.root.take()
            && root
                .as_ref()
                .is_none_or(|new| !same_path(&new.path, &old.path))
            && !self
                .previous_roots
                .iter()
                .any(|p| same_path(&p.path, &old.path))
        {
            self.previous_roots.push(old);
        }
        if let Some(new) = &root {
            self.previous_roots
                .retain(|p| !same_path(&p.path, &new.path));
        }
        self.root = root;
    }

    /// Stops using a previous root: its backups stay where they are but are no longer listed,
    /// cleaned up or re-encrypted, and its folder records go. Returns whether it was one.
    pub fn forget_root(&mut self, path: &Path) -> bool {
        let before = self.previous_roots.len();
        self.previous_roots.retain(|p| !same_path(&p.path, path));
        if self.previous_roots.len() == before {
            return false;
        }
        for setting in self.vaults.values_mut() {
            setting.folders.retain(|f| !same_path(&f.root, path));
        }
        self.vaults.retain(|_, s| !s.folders.is_empty());
        true
    }
}

// ---------------------------------------------------------------------------------------------
// Roots

#[derive(Serialize, Deserialize)]
struct MarkerJson {
    marker: String,
}

/// The marker in `root`, if it has a readable one.
pub fn read_marker(root: &Path) -> Option<String> {
    let bytes = read_capped(&root.join(MARKER_FILE), SMALL_JSON_MAX).ok()?;
    let marker = serde_json::from_slice::<MarkerJson>(&bytes).ok()?.marker;
    (!marker.is_empty()).then_some(marker)
}

/// The marker of the existing folder `root`, writing a new one if it has none. An unreadable
/// marker file is not replaced (it may belong to a root in use elsewhere): that is an error.
pub fn ensure_marker(root: &Path) -> io::Result<String> {
    if let Some(marker) = read_marker(root) {
        return Ok(marker);
    }
    let mut b = [0u8; 16];
    random_bytes(&mut b);
    // A version 4 UUID.
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = hex::encode(b);
    let marker = format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    );
    let json = serde_json::to_vec(&MarkerJson {
        marker: marker.clone(),
    })?;
    fsops::write_new(&root.join(MARKER_FILE), &json)?;
    Ok(marker)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootKind {
    /// The root the user chose.
    Configured,
    /// `%LOCALAPPDATA%\Axentra Vault\Backups`.
    Default,
    /// A root chosen before; read, re-keyed and pruned, never written to.
    Previous,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownRoot {
    pub kind: RootKind,
    pub path: PathBuf,
    /// The marker a custom root must carry; None for the default root.
    pub marker: Option<String>,
}

/// Why a root can't be used right now.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Unavailable {
    #[error("the folder isn't there (is the drive connected?)")]
    Missing,
    #[error("the folder isn't marked as an Axentra backup folder")]
    NoMarker,
    #[error("the folder is marked for a different backup location")]
    WrongMarker,
    #[error("the folder didn't respond in time")]
    TimedOut,
    #[error("{0}")]
    Io(String),
}

#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("the backup folder is unavailable: {0}")]
    Unavailable(Unavailable),
    #[error("{0}")]
    Io(#[from] io::Error),
}

/// Bytes free for this user in a folder; None when unknown.
pub type FreeSpaceFn = Arc<dyn Fn(&Path) -> Option<u64> + Send + Sync>;

/// Where backups go. Built by the app from `backups.json` and `%LOCALAPPDATA%`; tests build
/// their own, so nothing here reads the environment.
#[derive(Clone)]
pub struct BackupConfig {
    pub default_root: PathBuf,
    pub root: Option<RootSetting>,
    pub previous_roots: Vec<RootSetting>,
    /// Injectable so tests can simulate a full disk.
    pub free_space: FreeSpaceFn,
    pub probe_timeout: Duration,
    /// `backups.json`, re-read and updated as folders are found or created. None keeps the
    /// settings in memory for the life of the vault that uses this config.
    pub settings_file: Option<PathBuf>,
    /// Recent checks of custom roots: one that answered counts as available for
    /// [`PROBE_REUSE`], one that didn't answer in time as unavailable for [`PROBE_RETRY`], so a
    /// save or unlock checks each root once instead of at every step (and a hung network
    /// drive costs one wait and one stuck thread, not one per step). Shared by clones.
    checked: Arc<Mutex<HashMap<String, RootCheck>>>,
}

/// When a custom root was checked, and the answer.
type RootCheck = (Instant, Result<(), Unavailable>);

/// How long a custom root that answered counts as available without asking again.
pub const PROBE_REUSE: Duration = Duration::from_secs(5);
/// How long a custom root that didn't answer in time counts as unavailable without asking again.
pub const PROBE_RETRY: Duration = Duration::from_secs(30);

impl std::fmt::Debug for BackupConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BackupConfig")
            .field("default_root", &self.default_root)
            .field("root", &self.root)
            .field("previous_roots", &self.previous_roots)
            .field("probe_timeout", &self.probe_timeout)
            .field("settings_file", &self.settings_file)
            .finish_non_exhaustive()
    }
}

impl BackupConfig {
    pub fn new(default_root: impl Into<PathBuf>) -> BackupConfig {
        BackupConfig {
            default_root: default_root.into(),
            root: None,
            previous_roots: Vec::new(),
            free_space: Arc::new(fsops::free_space),
            probe_timeout: PROBE_TIMEOUT,
            settings_file: None,
            checked: Arc::default(),
        }
    }

    /// The config the settings file describes, kept in step with that file.
    pub fn load(default_root: impl Into<PathBuf>, settings_file: impl Into<PathBuf>) -> Self {
        let settings_file = settings_file.into();
        BackupConfig {
            settings_file: Some(settings_file.clone()),
            ..BackupConfig::from_settings(default_root, &BackupSettings::load(&settings_file))
        }
    }

    pub fn from_settings(default_root: impl Into<PathBuf>, settings: &BackupSettings) -> Self {
        BackupConfig {
            root: settings.root.clone(),
            previous_roots: settings.previous_roots.clone(),
            ..BackupConfig::new(default_root)
        }
    }

    pub fn with_free_space(
        mut self,
        free_space: impl Fn(&Path) -> Option<u64> + Send + Sync + 'static,
    ) -> Self {
        self.free_space = Arc::new(free_space);
        self
    }

    /// The root backups are written to when it is available: the chosen one, else the default.
    pub fn home(&self) -> KnownRoot {
        match &self.root {
            Some(root) => KnownRoot {
                kind: RootKind::Configured,
                path: root.path.clone(),
                marker: Some(root.marker.clone()),
            },
            None => self.default_known_root(),
        }
    }

    /// `%LOCALAPPDATA%\Axentra Vault\Backups`, as a known root.
    pub fn default_known_root(&self) -> KnownRoot {
        KnownRoot {
            kind: RootKind::Default,
            path: self.default_root.clone(),
            marker: None,
        }
    }

    /// Every root that may hold backups: the chosen one, the default, then previous ones, each
    /// path once.
    pub fn known_roots(&self) -> Vec<KnownRoot> {
        let mut roots = Vec::new();
        if self.root.is_some() {
            roots.push(self.home());
        }
        roots.push(self.default_known_root());
        for p in &self.previous_roots {
            roots.push(KnownRoot {
                kind: RootKind::Previous,
                path: p.path.clone(),
                marker: Some(p.marker.clone()),
            });
        }
        let mut seen = HashSet::new();
        roots.retain(|r| seen.insert(path_key(&r.path)));
        roots
    }

    /// The known root `path` lies in (at any depth), if any.
    pub fn root_containing(&self, path: &Path) -> Option<KnownRoot> {
        self.known_roots()
            .into_iter()
            .find(|r| is_within(path, &r.path))
    }

    /// The known root a backup at `path` belongs to: a file named as a database backup, in a
    /// vault's folder (one with a `vault.json`) directly inside a known root. Such a file opens
    /// as a backup (read-only, no store of its own). Anything else under a root, such as
    /// another vault on the same USB drive, is not a backup.
    pub fn backup_root_of(&self, path: &Path) -> Option<KnownRoot> {
        let name = path.file_name()?.to_str()?;
        if !BackupName::parse(name).is_some_and(|n| n.is_kdbx()) {
            return None;
        }
        let folder = path.parent()?;
        let root_path = folder.parent()?;
        let root = self
            .known_roots()
            .into_iter()
            .find(|r| same_path(&r.path, root_path))?;
        read_vault_json(folder).is_some().then_some(root)
    }

    /// Whether `root` can be used now. Creates nothing. A custom root must exist and carry the
    /// marker from the settings, checked on a worker thread so a hung network drive costs at
    /// most `probe_timeout`; a recent answer is reused (see [`PROBE_REUSE`], [`PROBE_RETRY`]).
    pub fn check_root(&self, root: &KnownRoot) -> Result<(), Unavailable> {
        let Some(marker) = &root.marker else {
            return if root.path.is_dir() {
                Ok(())
            } else {
                Err(Unavailable::Missing)
            };
        };
        let key = format!("{}|{marker}", path_key(&root.path));
        let recent = |(at, result): &RootCheck| match result {
            Ok(()) => at.elapsed() < PROBE_REUSE,
            Err(Unavailable::TimedOut) => at.elapsed() < PROBE_RETRY,
            Err(_) => false,
        };
        let lock = || self.checked.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((_, result)) = lock().get(&key).filter(|c| recent(c)) {
            return result.clone();
        }
        let result = probe(&root.path, marker, self.probe_timeout);
        lock().insert(key, (Instant::now(), result.clone()));
        result
    }

    /// Whether `root` has room for a backup of `backup_len` bytes: that plus 64 MiB, plus twice
    /// the database and 256 MiB more when the root shares the database's volume (the next save
    /// needs room too). Unknown free space counts as enough; the write itself still fails
    /// cleanly on a full disk.
    pub fn has_room(&self, root: &Path, backup_len: u64, database: Option<(&Path, u64)>) -> bool {
        let Some(free) = (self.free_space)(root) else {
            return true;
        };
        let mut need = backup_len.saturating_add(64 * MIB);
        if let Some((db_path, db_len)) = database
            && fsops::same_volume(root, db_path)
        {
            need = need
                .saturating_add(db_len.saturating_mul(2))
                .saturating_add(256 * MIB);
        }
        free >= need
    }

    /// Opens this vault's folder in `root`, found as follows (settings map each path to its
    /// folder in each root):
    /// 1. this path's folder in this root, if its `vault.json` names this vault;
    /// 2. the folders of an entry for this vault whose path no longer exists (moved or
    ///    renamed), all taken over;
    /// 3. a folder for this vault that no other path's entry has in this root (settings lost,
    ///    e.g. a reinstall);
    /// 4. with `create`, a new `<stem>` or `<stem> (n)` folder. A copy of the database (same
    ///    vault, both paths exist) gets its own folder this way, in every root.
    ///
    /// The folder found is recorded for this root. The default root is created when needed; a
    /// custom root never is, and a previous root never gets a new folder. Leftover temp files
    /// older than an hour are swept. Ok(None): no folder and `create` was false.
    pub fn open_folder(
        &self,
        root: &KnownRoot,
        db_path: &Path,
        vault_id: &str,
        settings: &mut BackupSettings,
        create: bool,
    ) -> Result<Option<Folder>, BackupError> {
        let create = create && root.kind != RootKind::Previous;
        if root.kind == RootKind::Default {
            if create {
                fs::create_dir_all(&root.path)?;
            } else if !root.path.is_dir() {
                return Ok(None);
            }
        } else {
            self.check_root(root).map_err(BackupError::Unavailable)?;
        }
        let stem = stem_of(db_path);
        let Some(name) = resolve_folder(root, db_path, &stem, vault_id, settings, create)? else {
            return Ok(None);
        };
        let folder = Folder::new(root.path.join(name), &root.path, vault_id, stem);
        let _ = folder.sweep_temps();
        Ok(Some(folder))
    }

    /// This vault's existing folders in every known root that answers, and the roots that
    /// don't. Creates nothing.
    pub fn existing_folders(
        &self,
        db_path: &Path,
        vault_id: &str,
        settings: &mut BackupSettings,
    ) -> (Vec<Folder>, Vec<(KnownRoot, Unavailable)>) {
        let mut folders = Vec::new();
        let mut unavailable = Vec::new();
        for root in self.known_roots() {
            match self.open_folder(&root, db_path, vault_id, settings, false) {
                Ok(Some(folder)) => folders.push(folder),
                Ok(None) => {}
                Err(BackupError::Unavailable(why)) => unavailable.push((root, why)),
                Err(BackupError::Io(e)) => unavailable.push((root, Unavailable::Io(e.to_string()))),
            }
        }
        (folders, unavailable)
    }

    /// The ring snapshots of the database at `db_path`, newest first, from the folder the
    /// settings file maps its path to. For putting back a passkey file before the database (and
    /// so its vault id) is open: whatever is found here is used only if its key opens the
    /// database. Empty without a settings file.
    pub fn ring_snapshots_for(&self, db_path: &Path) -> Vec<PathBuf> {
        let Some(file) = &self.settings_file else {
            return Vec::new();
        };
        let settings = BackupSettings::load(file);
        let Some(entry) = settings.vaults.get(&path_key(db_path)) else {
            return Vec::new();
        };
        // The first known root, in order, whose folder answers and holds snapshots.
        for root in self.known_roots() {
            let Some(name) = entry.folder_in(&root.path) else {
                continue;
            };
            if self.check_root(&root).is_err() || !folder_holds(&root.path, name, &entry.id) {
                continue;
            }
            let folder = Folder::new(
                root.path.join(name),
                &root.path,
                &entry.id,
                stem_of(db_path),
            );
            let rings: Vec<PathBuf> = folder
                .list_rings()
                .map(|rings| rings.into_iter().map(|b| b.path).collect())
                .unwrap_or_default();
            if !rings.is_empty() {
                return rings;
            }
        }
        Vec::new()
    }
}

fn probe(root: &Path, marker: &str, timeout: Duration) -> Result<(), Unavailable> {
    let (tx, rx) = std::sync::mpsc::channel();
    let (root, marker) = (root.to_owned(), marker.to_owned());
    std::thread::Builder::new()
        .name("backup root check".into())
        .spawn(move || {
            let _ = tx.send(marker_matches(&root, &marker));
        })
        .map_err(|e| Unavailable::Io(e.to_string()))?;
    match rx.recv_timeout(timeout) {
        Ok(result) => result,
        Err(RecvTimeoutError::Timeout) => Err(Unavailable::TimedOut),
        Err(RecvTimeoutError::Disconnected) => Err(Unavailable::Io("the check failed".into())),
    }
}

fn marker_matches(root: &Path, expected: &str) -> Result<(), Unavailable> {
    match fs::metadata(root) {
        Ok(m) if m.is_dir() => {}
        Ok(_) => return Err(Unavailable::Missing),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(Unavailable::Missing),
        Err(e) => return Err(Unavailable::Io(e.to_string())),
    }
    match read_marker(root) {
        Some(m) if m.eq_ignore_ascii_case(expected) => Ok(()),
        Some(_) => Err(Unavailable::WrongMarker),
        None => Err(Unavailable::NoMarker),
    }
}

#[derive(Serialize, Deserialize)]
struct VaultJson {
    v: u32,
    id: String,
    stem: String,
}

fn read_vault_json(folder: &Path) -> Option<VaultJson> {
    let bytes = read_capped(&folder.join(VAULT_FILE), SMALL_JSON_MAX).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn folder_holds(root: &Path, folder: &str, vault_id: &str) -> bool {
    valid_folder_name(folder)
        && read_vault_json(&root.join(folder)).is_some_and(|v| v.id == vault_id)
}

/// Folder lookup; see [`BackupConfig::open_folder`]. Returns the folder's name in `root`.
fn resolve_folder(
    root: &KnownRoot,
    db_path: &Path,
    stem: &str,
    vault_id: &str,
    settings: &mut BackupSettings,
    create: bool,
) -> io::Result<Option<String>> {
    let key = path_key(db_path);
    let live = |k: &str| Path::new(k).exists();
    let holds = |e: &VaultSetting| {
        e.folder_in(&root.path)
            .filter(|f| e.id == vault_id && folder_holds(&root.path, f, vault_id))
            .map(str::to_owned)
    };
    // This path's entry, replaced if it was for another vault.
    let entry = |settings: &mut BackupSettings| {
        let entry = settings
            .vaults
            .entry(key.clone())
            .or_insert_with(|| VaultSetting::new(vault_id));
        if entry.id != vault_id {
            *entry = VaultSetting::new(vault_id);
        }
    };

    let mut found = settings.vaults.get(&key).and_then(holds);
    if found.is_none() {
        let moved = settings
            .vaults
            .iter()
            .find(|(k, e)| **k != key && !live(k) && holds(e).is_some())
            .map(|(k, _)| k.clone());
        if let Some(old_key) = moved {
            let old = settings.vaults.remove(&old_key).expect("just found");
            found = holds(&old);
            entry(settings);
            let this = settings.vaults.get_mut(&key).expect("just made");
            for f in old.folders {
                if this.folder_in(&f.root).is_none() {
                    this.folders.push(f);
                }
            }
        }
    }
    if found.is_none() {
        // A folder another path has in this root is that database's, even while its path is
        // gone (a drive not plugged in) or it is a copy of this one.
        let claimed: HashSet<String> = settings
            .vaults
            .iter()
            .filter(|(k, _)| **k != key)
            .filter_map(|(_, e)| e.folder_in(&root.path))
            .map(str::to_lowercase)
            .collect();
        found = find_unclaimed(&root.path, vault_id, &claimed);
    }
    if found.is_none() && create {
        found = Some(create_folder(root, stem, vault_id)?);
    }
    if let Some(folder) = &found {
        entry(settings);
        if let Some(this) = settings.vaults.get_mut(&key) {
            this.set_folder(&root.path, folder);
        }
    }
    Ok(found)
}

/// The first (by name) of the root's first 256 folders whose `vault.json` names this vault and
/// that no other path's settings entry has.
fn find_unclaimed(root: &Path, vault_id: &str, claimed: &HashSet<String>) -> Option<String> {
    let mut matches = Vec::new();
    let mut seen = 0;
    for entry in fs::read_dir(root).ok()?.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_dir() || fsops::is_reparse_point(&meta) {
            continue;
        }
        seen += 1;
        if seen > MAX_FOLDER_SCAN {
            break;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !claimed.contains(&name.to_lowercase()) && folder_holds(root, &name, vault_id) {
            matches.push(name);
        }
    }
    matches.sort();
    matches.into_iter().next()
}

fn create_folder(root: &KnownRoot, stem: &str, vault_id: &str) -> io::Result<String> {
    // Folders under a custom root get a private access list where the file system keeps one;
    // the default root under %LOCALAPPDATA% is already private to the user.
    let private = root.kind != RootKind::Default
        && fsops::volume_info(&root.path).is_some_and(|v| v.persistent_acls);
    for n in 1..=1000u32 {
        let name = match n {
            1 => stem.to_owned(),
            n => format!("{stem} ({n})"),
        };
        let path = root.path.join(&name);
        let made = if private {
            fsops::create_dir_private(&path)
        } else {
            fs::create_dir(&path)
        };
        match made {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
        let json = serde_json::to_vec(&VaultJson {
            v: 1,
            id: vault_id.to_owned(),
            stem: stem.to_owned(),
        })?;
        if let Err(e) = fsops::write_new(&path.join(VAULT_FILE), &json) {
            let _ = fs::remove_dir(&path);
            return Err(e);
        }
        return Ok(name);
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "no free backup folder name",
    ))
}

// ---------------------------------------------------------------------------------------------
// Index and manifest

/// `index.json`: a cache. A missing or corrupt index is rebuilt from the file names with the
/// hashes recomputed and no generation; deletion of backups never depends on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Index {
    pub v: u32,
    /// Credential generations: when each was created, whether a save under it committed, and
    /// the name of the backup key file written for it.
    #[serde(default)]
    pub gens: BTreeMap<String, GenInfo>,
    /// File name → what is known about it.
    #[serde(default)]
    pub entries: BTreeMap<String, IndexEntry>,
    /// The hash of a `<db>.bak` written while no root worked, to import it later.
    #[serde(default)]
    pub fallback_bak_sha256: Option<String>,
}

impl Default for Index {
    fn default() -> Self {
        Index {
            v: 1,
            gens: BTreeMap::new(),
            entries: BTreeMap::new(),
            fallback_bak_sha256: None,
        }
    }
}

impl Index {
    /// Notes a credential generation (created now, if it is new) and marks it committed once a
    /// save under it has replaced the database. A committed generation stays committed.
    pub fn note_generation(&mut self, generation: &str, committed: bool) {
        let info = self
            .gens
            .entry(generation.to_owned())
            .or_insert_with(|| GenInfo {
                created: utc_string(SystemTime::now()),
                committed: false,
                keyx: None,
                retired: false,
            });
        info.committed |= committed;
    }

    /// Marks a generation as replaced by a credential change (one Axentra made, or one found at
    /// unlock that was made elsewhere): backups labelled with it need older credentials.
    pub fn retire(&mut self, generation: &str) {
        if let Some(info) = self.gens.get_mut(generation) {
            info.retired = true;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenInfo {
    pub created: String,
    pub committed: bool,
    #[serde(default)]
    pub keyx: Option<String>,
    /// A credential change replaced it. Without one, another committed generation may just
    /// mean the record was lost (a file restored from before it, or saved by an app that drops
    /// it) while the credentials stayed the same.
    #[serde(default)]
    pub retired: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexEntry {
    pub sha256: String,
    pub size: u64,
    /// The credential generation that opens it, as far as this app knows (a hint only).
    #[serde(default, rename = "gen")]
    pub generation: Option<String>,
    /// When the copy was made (UTC).
    #[serde(default)]
    pub taken: String,
    /// Set when a key change could not re-encrypt it: `failed: lossy|format|kdf|open`.
    #[serde(default)]
    pub rekey: Option<String>,
}

/// `rekey.json`: re-encrypted copies staged during a key change, not yet published, the backups
/// that couldn't be re-encrypted (marked in the index once the change commits), and the copies
/// beside the database the change moved into this folder (deleted once it commits).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RekeyManifest {
    pub to_gen: String,
    pub items: Vec<RekeyItem>,
    #[serde(default)]
    pub failed: Vec<RekeyFailure>,
    #[serde(default)]
    pub beside: Vec<BesideCopy>,
}

impl RekeyManifest {
    pub fn new(to_gen: &str) -> RekeyManifest {
        RekeyManifest {
            to_gen: to_gen.to_owned(),
            items: Vec::new(),
            failed: Vec::new(),
            beside: Vec::new(),
        }
    }
}

/// A copy of the database beside it (`<db><suffix>`) that a key change moved into the store, by
/// the hash of what was moved: once the change commits, the file is deleted if it still holds
/// exactly that.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BesideCopy {
    pub suffix: String,
    pub sha256: String,
}

impl BesideCopy {
    /// One of the suffixes such copies have (see [`is_copy_suffix`]): the manifest can be
    /// written by anyone who can write the store, so it may name nothing else.
    pub fn valid(&self) -> bool {
        is_copy_suffix(&self.suffix)
    }
}

/// Whether `<db><suffix>` names a copy of the database earlier versions or an interrupted save
/// left beside it: `.bak`, `.tmp`, `.bak.tmp`, a save temp `.~axv-<hex>.tmp` or a fallback
/// `.bak` temp `.bak.~axv-<hex>.tmp` (lower case).
pub fn is_copy_suffix(suffix: &str) -> bool {
    matches!(suffix, ".bak" | ".tmp" | ".bak.tmp")
        || fsops::is_temp_name(suffix)
        || suffix.strip_prefix(".bak").is_some_and(fsops::is_temp_name)
}

/// File names are relative to the folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RekeyItem {
    pub tmp: String,
    pub original: String,
    pub original_sha256: String,
    pub tmp_sha256: String,
}

impl RekeyItem {
    /// A temp name this store makes and a database backup's name, both plain names in the
    /// folder: the manifest can be written by anyone who can write the store, so nothing in it
    /// may point elsewhere.
    fn valid(&self) -> bool {
        fsops::is_temp_name(&self.tmp)
            && !self.original.contains(['\\', '/', ':'])
            && BackupName::parse(&self.original).is_some_and(|n| n.is_kdbx())
    }
}

/// A backup a key change left on its old credentials, and why: `failed: lossy|format|kdf|open`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RekeyFailure {
    pub original: String,
    pub why: String,
}

/// What publishing a key change's manifest did in one folder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RekeyPublished {
    /// The re-encrypted copies now in place.
    pub published: Vec<String>,
    /// The originals they replaced, deleted.
    pub removed: Vec<String>,
    /// Items left as they were, and why.
    pub skipped: Vec<(String, String)>,
    /// Originals another program holds open, still there on the old credentials.
    pub held: Vec<String>,
    /// An original couldn't be deleted yet: `rekey.json` stays for the next unlock to finish.
    pub pending: bool,
}

// ---------------------------------------------------------------------------------------------
// Folders

/// State shared by every handle on one folder in this process: the mutex that covers seq
/// allocation, publishing, index writes and pruning, and the count of staging jobs (a key
/// change re-encrypting backups), during which pruning is skipped. The app runs once per user,
/// so no cross-process lock is needed: names never replace each other and pruning tolerates
/// files vanishing.
#[derive(Debug, Default)]
struct Shared {
    lock: Mutex<()>,
    staging: AtomicUsize,
}

fn shared_for(folder: &Path) -> Arc<Shared> {
    static FOLDERS: OnceLock<Mutex<HashMap<String, Weak<Shared>>>> = OnceLock::new();
    let mut folders = FOLDERS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    folders.retain(|_, w| w.strong_count() > 0);
    let key = path_key(folder);
    if let Some(shared) = folders.get(&key).and_then(Weak::upgrade) {
        return shared;
    }
    let shared = Arc::new(Shared::default());
    folders.insert(key, Arc::downgrade(&shared));
    shared
}

/// Keeps pruning off a folder while a key change stages re-encrypted copies in it.
#[derive(Debug)]
pub struct StagingGuard(Arc<Shared>);

impl Drop for StagingGuard {
    fn drop(&mut self) {
        self.0.staging.fetch_sub(1, Ordering::SeqCst);
    }
}

/// A file in the folder whose name parses.
#[derive(Debug, Clone)]
struct Found {
    file_name: String,
    name: BackupName,
    path: PathBuf,
    len: u64,
    /// Creation (else modification) time, for an index entry rebuilt from the file.
    time: Option<SystemTime>,
    reparse: bool,
}

/// One backup or ring snapshot, as listed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backup {
    pub path: PathBuf,
    pub file_name: String,
    pub name: BackupName,
    pub size: u64,
    /// None while the file couldn't be read to hash it.
    pub sha256: Option<String>,
    pub generation: Option<String>,
    pub rekey: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteOutcome {
    Written(PathBuf),
    /// The newest backup already holds these bytes; nothing was written.
    Duplicate(PathBuf),
}

impl WriteOutcome {
    pub fn path(&self) -> &Path {
        match self {
            WriteOutcome::Written(p) | WriteOutcome::Duplicate(p) => p,
        }
    }
}

/// Why retention deleted no database backup.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PruneAbort {
    #[error("the folder's vault.json is missing or names another vault")]
    ForeignFolder,
    #[error("the newest backup isn't a complete KeePass file")]
    SuspectNewest,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PruneReport {
    /// A key change is staging in this folder; nothing was looked at (not a warning).
    pub skipped: bool,
    pub aborted: Option<PruneAbort>,
    /// File names, oldest first.
    pub deleted: Vec<String>,
    /// Backups retention would delete that wait for a later run: one run deletes at most
    /// max(5, a quarter) of the backups, so a backlog goes over a few runs and a mistake can't
    /// take everything at once.
    pub deferred: usize,
    /// Old-key originals a key change re-encrypted but couldn't delete then.
    pub superseded: Vec<String>,
    pub rings_deleted: Vec<String>,
    /// Problems that didn't stop the rest (never fatal to a save).
    pub errors: Vec<String>,
}

impl PruneReport {
    pub fn warning(&self) -> Option<String> {
        self.aborted
            .as_ref()
            .map(|why| format!("Old backups were not cleaned up: {why}."))
    }
}

/// What a prune will delete.
#[derive(Debug, Default)]
struct PrunePlan {
    /// Why no database backup is deleted, if none is.
    abort: Option<PruneAbort>,
    delete: Vec<Found>,
    deferred: usize,
    superseded: Vec<Found>,
    rings: Vec<Found>,
    index: Index,
}

/// Retention over representatives sorted newest `seq` first: which to keep. Pure (no clock, no
/// files), so a wrong clock costs at most one day slot and one month slot.
fn retention_keep(reps: &[(Kind, &str)]) -> Vec<bool> {
    let mut keep = vec![false; reps.len()];
    if let Some(newest) = keep.first_mut() {
        *newest = true;
    }
    let mut recent = 0;
    let mut days: Vec<&str> = Vec::new();
    let mut months: Vec<&str> = Vec::new();
    for (i, &(kind, date)) in reps.iter().enumerate() {
        if kind.is_pinned() {
            keep[i] = true;
            continue;
        }
        if recent < KEEP_RECENT {
            recent += 1;
            keep[i] = true;
        }
        if !days.contains(&date) && days.len() < KEEP_DAYS {
            days.push(date);
            keep[i] = true;
        }
        let month = date.get(..7).unwrap_or(date);
        if !months.contains(&month) && months.len() < KEEP_MONTHS {
            months.push(month);
            keep[i] = true;
        }
    }
    keep
}

/// The newest revision of each database backup, in the order given (highest (seq, rev, name)
/// first). Lower revisions exist only during or after a key change (see
/// [`Folder::superseded`]). Backups are told apart by their name without the revision, not by
/// `seq` alone: two PCs writing to one shared root can each pick the same number.
fn representatives<'a>(found: impl IntoIterator<Item = &'a Found>) -> Vec<&'a Found> {
    let mut seen = HashSet::new();
    found
        .into_iter()
        .filter(|f| f.name.is_kdbx() && seen.insert(base_name(&f.name)))
        .collect()
}

/// A backup's name without its revision: what its re-encrypted copies share.
fn base_name(name: &BackupName) -> String {
    BackupName {
        rev: 0,
        ..name.clone()
    }
    .file_name()
}

fn next_seq(found: &[Found]) -> u64 {
    found
        .iter()
        .map(|f| f.name.seq)
        .max()
        .map_or(1, |max| max.saturating_add(1))
}

/// At least 512 bytes and the KDBX signature.
pub fn looks_like_kdbx(path: &Path) -> bool {
    let check = || -> io::Result<bool> {
        let mut file = fs::File::open(path)?;
        let mut head = [0u8; 8];
        file.read_exact(&mut head)?;
        Ok(file.metadata()?.len() >= MIN_KDBX_LEN && head == KDBX_SIGNATURE)
    };
    check().unwrap_or(false)
}

/// Gives a just-published backup the source version's time, flushes it and makes it read-only.
fn finish_published(path: &Path, mtime: SystemTime) -> io::Result<()> {
    let file = OpenOptions::new().write(true).open(path)?;
    file.set_modified(mtime)?;
    file.sync_all()?;
    drop(file);
    fsops::set_readonly(path, true)
}

/// One vault's folder in one root.
#[derive(Debug, Clone)]
pub struct Folder {
    path: PathBuf,
    root: PathBuf,
    vault_id: String,
    /// The current database's sanitized stem, used for new file names.
    stem: String,
    shared: Arc<Shared>,
}

impl Folder {
    fn new(path: PathBuf, root: &Path, vault_id: &str, stem: String) -> Folder {
        Folder {
            shared: shared_for(&path),
            path,
            root: root.to_owned(),
            vault_id: vault_id.to_owned(),
            stem,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn vault_id(&self) -> &str {
        &self.vault_id
    }

    pub fn stem(&self) -> &str {
        &self.stem
    }

    fn lock(&self) -> MutexGuard<'_, ()> {
        self.shared
            .lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Marks a staging job active until the guard is dropped.
    pub fn begin_staging(&self) -> StagingGuard {
        self.shared.staging.fetch_add(1, Ordering::SeqCst);
        StagingGuard(self.shared.clone())
    }

    /// Whether a key change is staging re-encrypted copies in this folder right now.
    pub fn staging_active(&self) -> bool {
        self.shared.staging.load(Ordering::SeqCst) > 0
    }

    /// Every file in the folder whose name parses, highest (seq, rev) first. Links are skipped;
    /// other reparse points (cloud placeholders) are listed but flagged, and never pruned.
    fn scan(&self) -> io::Result<Vec<Found>> {
        let mut found = Vec::new();
        for entry in fs::read_dir(&self.path)?.flatten() {
            // Not followed: a link reports as a link, not a file.
            let Ok(meta) = entry.metadata() else { continue };
            if !meta.is_file() {
                continue;
            }
            let os_name = entry.file_name();
            let Some(file_name) = os_name.to_str() else {
                continue;
            };
            let Some(name) = BackupName::parse(file_name) else {
                continue;
            };
            found.push(Found {
                file_name: file_name.to_owned(),
                name,
                path: entry.path(),
                len: meta.len(),
                time: meta.created().or_else(|_| meta.modified()).ok(),
                reparse: fsops::is_reparse_point(&meta),
            });
        }
        found.sort_by(|a, b| {
            (b.name.seq, b.name.rev, &b.file_name).cmp(&(a.name.seq, a.name.rev, &a.file_name))
        });
        Ok(found)
    }

    /// The index, with entries added (hash recomputed, no generation) for files it doesn't know
    /// or whose size changed. Also says whether anything changed.
    fn load_index(&self, found: &[Found]) -> (Index, bool) {
        let parsed = read_capped(&self.path.join(INDEX_FILE), JSON_MAX)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Index>(&bytes).ok())
            .filter(|index| index.v == 1);
        let mut changed = parsed.is_none();
        let mut index = parsed.unwrap_or_default();
        for f in found {
            let known = index
                .entries
                .get(&f.file_name)
                .is_some_and(|e| e.size == f.len);
            // Reading a cloud placeholder would download it; it is hashed once it is local.
            if known || f.reparse {
                continue;
            }
            let Ok(sha256) = sha256_file(&f.path) else {
                continue;
            };
            index.entries.insert(
                f.file_name.clone(),
                IndexEntry {
                    sha256,
                    size: f.len,
                    generation: None,
                    taken: f.time.map(utc_string).unwrap_or_default(),
                    rekey: None,
                },
            );
            changed = true;
        }
        (index, changed)
    }

    fn save_index(&self, index: &Index) -> io::Result<()> {
        fsops::write_replacing(
            &self.path.join(INDEX_FILE),
            &serde_json::to_vec_pretty(index)?,
        )
    }

    /// Reads the index (rebuilding what is missing), applies `f` and writes it back.
    pub fn with_index<R>(&self, f: impl FnOnce(&mut Index) -> R) -> io::Result<R> {
        let _guard = self.lock();
        let found = self.scan()?;
        let (mut index, _) = self.load_index(&found);
        let result = f(&mut index);
        self.save_index(&index)?;
        Ok(result)
    }

    /// The index, with what was missing rebuilt (and cached), for reading.
    pub fn index(&self) -> io::Result<Index> {
        let _guard = self.lock();
        let found = self.scan()?;
        let (index, changed) = self.load_index(&found);
        if changed {
            let _ = self.save_index(&index);
        }
        Ok(index)
    }

    fn listing(&self, pick: impl FnOnce(&[Found]) -> Vec<&Found>) -> io::Result<Vec<Backup>> {
        let _guard = self.lock();
        let found = self.scan()?;
        let (index, changed) = self.load_index(&found);
        if changed {
            // A cache: failing to store it only costs the hashing again next time.
            let _ = self.save_index(&index);
        }
        Ok(pick(&found)
            .into_iter()
            .map(|f| {
                let entry = index.entries.get(&f.file_name);
                Backup {
                    path: f.path.clone(),
                    file_name: f.file_name.clone(),
                    name: f.name.clone(),
                    size: f.len,
                    sha256: entry.map(|e| e.sha256.clone()),
                    generation: entry.and_then(|e| e.generation.clone()),
                    rekey: entry.and_then(|e| e.rekey.clone()),
                }
            })
            .collect())
    }

    /// The database backups (the newest revision of each), newest first.
    pub fn list(&self) -> io::Result<Vec<Backup>> {
        self.listing(|found| representatives(found))
    }

    /// The ring snapshots, newest first.
    pub fn list_rings(&self) -> io::Result<Vec<Backup>> {
        self.listing(|found| found.iter().filter(|f| f.name.item == Item::Ring).collect())
    }

    /// Every backup file, all revisions and ring snapshots, highest (seq, rev) first.
    pub fn list_all(&self) -> io::Result<Vec<Backup>> {
        self.listing(|found| found.iter().collect())
    }

    /// Publishes `bytes` under the first free name from `first_seq` on: a synced temp file, a
    /// rename that never replaces (a taken name moves on to seq + 1, up to 16 times), then the
    /// final file is synced, given `mtime` and made read-only. Returns the path and file name.
    fn publish(
        &self,
        bytes: &[u8],
        first_seq: u64,
        mtime: SystemTime,
        name_for: impl Fn(u64) -> BackupName,
    ) -> io::Result<(PathBuf, String)> {
        let tmp = fsops::write_temp_in(&self.path, bytes)?;
        let mut seq = first_seq;
        for _ in 0..PUBLISH_ATTEMPTS {
            let file_name = name_for(seq).file_name();
            let dest = self.path.join(&file_name);
            match fsops::move_no_replace(&tmp, &dest) {
                Ok(()) => {
                    finish_published(&dest, mtime)?;
                    return Ok((dest, file_name));
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => seq = seq.saturating_add(1),
                Err(e) => {
                    let _ = fs::remove_file(&tmp);
                    return Err(e);
                }
            }
        }
        let _ = fs::remove_file(&tmp);
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "no free backup file name",
        ))
    }

    /// Writes a database backup named after `source_mtime` (the version's own modification
    /// time; now if unknown or in the future), which also becomes the backup's modification
    /// time. `Auto` and `Opened` copies identical to the newest backup are skipped.
    /// `generation` labels it with the credential generation known to open it.
    pub fn write_backup(
        &self,
        bytes: &[u8],
        kind: Kind,
        source_mtime: Option<SystemTime>,
        generation: Option<&str>,
    ) -> io::Result<WriteOutcome> {
        let _guard = self.lock();
        let found = self.scan()?;
        let (mut index, _) = self.load_index(&found);
        let sha256 = sha256_hex(bytes);
        if matches!(kind, Kind::Auto | Kind::Opened)
            && let Some(newest) = representatives(&found).first()
            && index
                .entries
                .get(&newest.file_name)
                .is_some_and(|e| e.sha256 == sha256)
        {
            return Ok(WriteOutcome::Duplicate(newest.path.clone()));
        }
        let mtime = name_time(source_mtime);
        let (date, time) = local_date_time(mtime);
        let (path, file_name) = self.publish(bytes, next_seq(&found), mtime, |seq| BackupName {
            stem: self.stem.clone(),
            date: date.clone(),
            time: time.clone(),
            seq,
            rev: 0,
            item: Item::Kdbx(kind),
        })?;
        index.entries.insert(
            file_name,
            IndexEntry {
                sha256,
                size: bytes.len() as u64,
                generation: generation.map(str::to_owned),
                taken: utc_string(SystemTime::now()),
                rekey: None,
            },
        );
        let _ = self.save_index(&index);
        Ok(WriteOutcome::Written(path))
    }

    /// Snapshots the passkey file, named after now. Skipped when the newest snapshot holds the
    /// same bytes under the same generation, or under one that never committed (each unlock of
    /// a file without a matching record starts a new provisional generation): that snapshot is
    /// labelled with `generation` instead.
    pub fn write_ring_snapshot(
        &self,
        bytes: &[u8],
        generation: Option<&str>,
    ) -> io::Result<WriteOutcome> {
        let _guard = self.lock();
        let found = self.scan()?;
        let (mut index, _) = self.load_index(&found);
        let sha256 = sha256_hex(bytes);
        if let Some(newest) = found.iter().find(|f| f.name.item == Item::Ring)
            && let Some(entry) = index.entries.get(&newest.file_name)
            && entry.sha256 == sha256
        {
            let labelled = entry.generation.clone();
            let provisional = labelled
                .as_deref()
                .is_none_or(|g| !index.gens.get(g).is_some_and(|info| info.committed));
            if labelled.as_deref() == generation {
                return Ok(WriteOutcome::Duplicate(newest.path.clone()));
            }
            if provisional {
                if let Some(entry) = index.entries.get_mut(&newest.file_name) {
                    entry.generation = generation.map(str::to_owned);
                }
                // A cache: failing to store the label costs a second snapshot at most.
                let _ = self.save_index(&index);
                return Ok(WriteOutcome::Duplicate(newest.path.clone()));
            }
        }
        let now = SystemTime::now();
        let (date, time) = local_date_time(now);
        let (path, file_name) = self.publish(bytes, next_seq(&found), now, |seq| BackupName {
            stem: self.stem.clone(),
            date: date.clone(),
            time: time.clone(),
            seq,
            rev: 0,
            item: Item::Ring,
        })?;
        index.entries.insert(
            file_name,
            IndexEntry {
                sha256,
                size: bytes.len() as u64,
                generation: generation.map(str::to_owned),
                taken: utc_string(now),
                rekey: None,
            },
        );
        let _ = self.save_index(&index);
        Ok(WriteOutcome::Written(path))
    }

    /// Deletes backups or ring snapshots of this folder by file name (clearing read-only; files
    /// already gone or held open elsewhere are skipped) and drops them from the index. Returns
    /// the names actually deleted. Names that aren't this store's file names are refused.
    pub fn delete(&self, file_names: &[&str]) -> io::Result<Vec<String>> {
        for name in file_names {
            if BackupName::parse(name).is_none() || name.contains(['\\', '/', ':']) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("not a backup file name: {name}"),
                ));
            }
        }
        let _guard = self.lock();
        let mut deleted = Vec::new();
        for name in file_names {
            if fsops::remove_file(&self.path.join(name))? {
                deleted.push((*name).to_owned());
            }
        }
        let found = self.scan()?;
        let (mut index, _) = self.load_index(&found);
        index
            .entries
            .retain(|name, _| found.iter().any(|f| &f.file_name == name));
        self.save_index(&index)?;
        Ok(deleted)
    }

    /// Applies retention (see [`retention_keep`] and [`Folder::plan_prune`]), deletes the
    /// old-key originals a key change left (see [`Folder::superseded`]), then keeps the newest
    /// three ring snapshots of `session_gen` and deletes every other one (old rings may hold
    /// revoked passkey envelopes). Without a session generation, or while no snapshot of it
    /// exists yet, ring snapshots are left alone. Takes no clock. Never fails: problems are
    /// reported.
    pub fn prune(&self, session_gen: Option<&str>) -> PruneReport {
        let skipped = PruneReport {
            skipped: true,
            ..PruneReport::default()
        };
        if self.staging_active() {
            return skipped;
        }
        let _guard = self.lock();
        if self.staging_active() {
            return skipped;
        }
        match self.plan_prune(session_gen) {
            Ok(plan) => self.execute_prune(plan),
            Err(e) => PruneReport {
                errors: vec![e.to_string()],
                ..PruneReport::default()
            },
        }
    }

    /// Decides what a prune deletes. Nothing at all if `vault.json` is missing or names another
    /// vault. No database backup (the rest goes ahead) if the newest backup is under 512 bytes
    /// or lacks the KDBX signature. At most max(5, a quarter) of the backups in one run, the
    /// oldest first; the rest wait for later runs, so a backlog (many `opened` copies from
    /// unlocks without a save) drains over a few runs instead of blocking clean-up for good.
    fn plan_prune(&self, session_gen: Option<&str>) -> io::Result<PrunePlan> {
        if read_vault_json(&self.path).is_none_or(|v| v.id != self.vault_id) {
            return Ok(PrunePlan {
                abort: Some(PruneAbort::ForeignFolder),
                ..PrunePlan::default()
            });
        }
        let found = self.scan()?;
        let regular: Vec<&Found> = found.iter().filter(|f| !f.reparse).collect();
        let (index, _) = self.load_index(&found);
        let mut plan = PrunePlan {
            superseded: self.superseded_in(&found).into_iter().cloned().collect(),
            ..PrunePlan::default()
        };

        if let Some(session_gen) = session_gen {
            let rings: Vec<&Found> = regular
                .iter()
                .copied()
                .filter(|f| f.name.item == Item::Ring)
                .collect();
            let current = |f: &Found| {
                index
                    .entries
                    .get(&f.file_name)
                    .and_then(|e| e.generation.as_deref())
                    == Some(session_gen)
            };
            // Older rings go only once the current one is kept: they are what passkey unlock
            // falls back to.
            if rings.iter().any(|f| current(f)) {
                let mut kept = 0;
                for f in rings {
                    if current(f) && kept < KEEP_RINGS {
                        kept += 1;
                    } else {
                        plan.rings.push(f.clone());
                    }
                }
            }
        }

        let reps = representatives(regular.iter().copied());
        if let Some(newest) = reps.first()
            && !looks_like_kdbx(&newest.path)
        {
            plan.abort = Some(PruneAbort::SuspectNewest);
        } else {
            let slots: Vec<(Kind, &str)> = reps
                .iter()
                // Representatives are database backups.
                .map(|f| (f.name.kind().unwrap_or(Kind::Auto), f.name.date.as_str()))
                .collect();
            let keep = retention_keep(&slots);
            // Oldest first.
            let mut delete: Vec<Found> = reps
                .iter()
                .zip(&keep)
                .filter(|(_, keep)| !**keep)
                .map(|(f, _)| (*f).clone())
                .rev()
                .collect();
            let limit = (reps.len() / 4).max(5);
            plan.deferred = delete.len().saturating_sub(limit);
            delete.truncate(limit);
            plan.delete = delete;
        }
        plan.index = index;
        Ok(plan)
    }

    fn execute_prune(&self, plan: PrunePlan) -> PruneReport {
        let mut report = PruneReport {
            aborted: plan.abort,
            deferred: plan.deferred,
            ..PruneReport::default()
        };
        let remove = |f: &Found, into: &mut Vec<String>, errors: &mut Vec<String>| {
            match fsops::remove_file(&f.path) {
                Ok(true) => into.push(f.file_name.clone()),
                // Already gone, or held open elsewhere: retried at the next prune.
                Ok(false) => {}
                Err(e) => errors.push(format!("{}: {e}", f.file_name)),
            }
        };
        for f in &plan.delete {
            remove(f, &mut report.deleted, &mut report.errors);
        }
        for f in &plan.superseded {
            remove(f, &mut report.superseded, &mut report.errors);
        }
        for f in &plan.rings {
            remove(f, &mut report.rings_deleted, &mut report.errors);
        }
        let mut index = plan.index;
        let before = index.entries.len();
        index
            .entries
            .retain(|name, _| fs::symlink_metadata(self.path.join(name)).is_ok());
        if index.entries.len() != before
            && let Err(e) = self.save_index(&index)
        {
            report.errors.push(format!("{INDEX_FILE}: {e}"));
        }
        report
    }

    pub fn read_manifest(&self) -> io::Result<Option<RekeyManifest>> {
        match read_capped(&self.path.join(MANIFEST_FILE), JSON_MAX) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn write_manifest(&self, manifest: &RekeyManifest) -> io::Result<()> {
        fsops::write_replacing(
            &self.path.join(MANIFEST_FILE),
            &serde_json::to_vec_pretty(manifest)?,
        )
    }

    pub fn remove_manifest(&self) -> io::Result<()> {
        fsops::remove_file(&self.path.join(MANIFEST_FILE)).map(drop)
    }

    /// Deletes `.~axv-<hex>.tmp` files older than an hour that the manifest doesn't list. With
    /// an unreadable manifest nothing is deleted (its temps may be among them). Returns how
    /// many went.
    pub fn sweep_temps(&self) -> io::Result<usize> {
        let _guard = self.lock();
        let listed: HashSet<String> = match self.read_manifest() {
            Ok(Some(manifest)) => manifest
                .items
                .iter()
                .map(|item| item.tmp.to_lowercase())
                .collect(),
            Ok(None) => HashSet::new(),
            Err(_) => return Ok(0),
        };
        let now = SystemTime::now();
        let mut removed = 0;
        for entry in fs::read_dir(&self.path)?.flatten() {
            let os_name = entry.file_name();
            let Some(name) = os_name.to_str() else {
                continue;
            };
            if !fsops::is_temp_name(name) || listed.contains(&name.to_lowercase()) {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            let old = meta
                .modified()
                .ok()
                .and_then(|m| now.duration_since(m).ok())
                .is_some_and(|age| age > TEMP_MAX_AGE);
            if meta.is_file()
                && !fsops::is_reparse_point(&meta)
                && old
                && matches!(fsops::remove_file(&entry.path()), Ok(true))
            {
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// Publishes a key change's staged copies, once the database has committed to the
    /// manifest's generation. For each item:
    /// 1. the temp moves (never replacing anything) to the original's name with the next
    ///    revision, `~n`, gets the original's time and becomes read-only, labelled with the new
    ///    generation;
    /// 2. then the original goes, unless it changed since it was staged.
    ///
    /// Then the backups that couldn't be re-encrypted are marked in the index and `rekey.json`
    /// is deleted, unless an original couldn't be deleted yet (another program holds it open):
    /// then it stays for the next unlock to finish. Hashes from the manifest are checked; no key
    /// derivation runs. Safe to repeat: an item already moved is recognised by its hash.
    pub fn publish_rekey(&self, manifest: &RekeyManifest) -> io::Result<RekeyPublished> {
        let _guard = self.lock();
        let found = self.scan()?;
        let (mut index, _) = self.load_index(&found);
        let mut report = RekeyPublished::default();
        for item in &manifest.items {
            let skip = |report: &mut RekeyPublished, why: &str| {
                report.skipped.push((item.original.clone(), why.to_owned()))
            };
            if !item.valid() {
                skip(&mut report, "not a backup in this folder");
                continue;
            }
            let Some(mut name) = BackupName::parse(&item.original) else {
                continue;
            };
            let Some(rev) = name.rev.checked_add(1) else {
                skip(&mut report, "no revision number left");
                continue;
            };
            name.rev = rev;
            let target_name = name.file_name();
            let target = self.path.join(&target_name);
            let tmp = self.path.join(&item.tmp);
            let original = self.path.join(&item.original);
            let holds_copy = |path: &Path| {
                sha256_file(path).is_ok_and(|h| h.eq_ignore_ascii_case(&item.tmp_sha256))
            };

            // 1.
            if tmp.is_file() {
                if !holds_copy(&tmp) {
                    let _ = fsops::remove_file(&tmp);
                    skip(
                        &mut report,
                        "the re-encrypted copy changed after it was staged",
                    );
                    continue;
                }
                match fsops::move_no_replace(&tmp, &target) {
                    Ok(()) => {}
                    // Published before an interruption, its temp written again: keep the one
                    // in place.
                    Err(e) if e.kind() == io::ErrorKind::AlreadyExists && holds_copy(&target) => {
                        let _ = fsops::remove_file(&tmp);
                    }
                    Err(e) => {
                        let _ = fsops::remove_file(&tmp);
                        skip(&mut report, &e.to_string());
                        continue;
                    }
                }
            } else if !holds_copy(&target) {
                skip(&mut report, "the re-encrypted copy is missing");
                continue;
            }
            let meta = fs::metadata(&target)?;
            if !meta.permissions().readonly() {
                let mtime = fs::metadata(&original)
                    .and_then(|m| m.modified())
                    .ok()
                    .or_else(|| meta.modified().ok())
                    .unwrap_or_else(SystemTime::now);
                finish_published(&target, mtime)?;
            }
            let sha256 = item.tmp_sha256.to_lowercase();
            let taken = index
                .entries
                .get(&target_name)
                .filter(|e| e.sha256 == sha256)
                .map(|e| e.taken.clone())
                .unwrap_or_else(|| utc_string(SystemTime::now()));
            index.entries.insert(
                target_name.clone(),
                IndexEntry {
                    sha256,
                    size: meta.len(),
                    generation: Some(manifest.to_gen.clone()),
                    taken,
                    rekey: None,
                },
            );
            report.published.push(target_name.clone());

            // 2.
            match sha256_file(&original) {
                Ok(h) if h.eq_ignore_ascii_case(&item.original_sha256) => {
                    if fsops::remove_file(&original)? {
                        report.removed.push(item.original.clone());
                    }
                    if original.exists() {
                        // Held open elsewhere: the manifest stays, and the next unlock tries
                        // again (as does every prune: see `Folder::superseded`).
                        report.pending = true;
                        report.held.push(item.original.clone());
                    } else {
                        index.entries.remove(&item.original);
                    }
                }
                Ok(_) => {
                    // What was re-encrypted is no longer what the original holds: the copy goes
                    // again, and the original stays the backup, marked as not re-encrypted.
                    fsops::remove_file(&target)?;
                    if target.exists() {
                        // Held open: tried again with the manifest, which keeps both meanwhile.
                        report.pending = true;
                    } else {
                        index.entries.remove(&target_name);
                    }
                    report.published.pop();
                    if let Some(entry) = index.entries.get_mut(&item.original) {
                        entry.rekey = Some("failed: changed".into());
                    }
                    skip(
                        &mut report,
                        "the original changed after it was staged; it keeps its credentials",
                    );
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    index.entries.remove(&item.original);
                }
                Err(e) => {
                    report.pending = true;
                    skip(&mut report, &e.to_string());
                }
            }
        }
        for failed in &manifest.failed {
            if let Some(entry) = index.entries.get_mut(&failed.original) {
                entry.rekey = Some(failed.why.clone());
            }
        }
        index.note_generation(&manifest.to_gen, true);
        self.save_index(&index)?;
        if !report.pending {
            self.remove_manifest()?;
        }
        Ok(report)
    }

    /// Throws a key change's staged copies away: the manifest's temps, then the manifest.
    pub fn discard_rekey(&self, manifest: &RekeyManifest) -> io::Result<()> {
        let _guard = self.lock();
        for item in &manifest.items {
            if fsops::is_temp_name(&item.tmp) {
                fsops::remove_file(&self.path.join(&item.tmp))?;
            }
        }
        self.remove_manifest()
    }

    /// Database backups a key change re-encrypted whose original it couldn't delete then
    /// (another program held it open): lower revisions of a backup that has a higher one. Their
    /// contents are in the higher revision, on the new credentials. None while a key change's
    /// manifest with staged copies is here (it owns lower revisions until it is published or
    /// thrown away; an unreadable one might) or one is being staged.
    pub fn superseded(&self) -> io::Result<Vec<String>> {
        let _guard = self.lock();
        Ok(self
            .superseded_in(&self.scan()?)
            .into_iter()
            .map(|f| f.file_name.clone())
            .collect())
    }

    fn superseded_in<'a>(&self, found: &'a [Found]) -> Vec<&'a Found> {
        let owned = match self.read_manifest() {
            Ok(manifest) => manifest.is_some_and(|m| !m.items.is_empty()),
            Err(_) => true,
        };
        if self.staging_active() || owned {
            return Vec::new();
        }
        let mut newest: HashMap<String, u32> = HashMap::new();
        for f in found.iter().filter(|f| f.name.is_kdbx()) {
            let rev = newest.entry(base_name(&f.name)).or_default();
            *rev = (*rev).max(f.name.rev);
        }
        found
            .iter()
            .filter(|f| f.name.is_kdbx() && !f.reparse && f.name.rev < newest[&base_name(&f.name)])
            .collect()
    }

    /// Deletes [`Folder::superseded`] backups; returns the names deleted.
    pub fn delete_superseded(&self) -> io::Result<Vec<String>> {
        let names = self.superseded()?;
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        if refs.is_empty() {
            return Ok(Vec::new());
        }
        self.delete(&refs)
    }

    /// Deletes every ring snapshot not labelled `generation`: older rings may wrap a revoked key.
    /// Returns the names deleted.
    pub fn delete_rings_except(&self, generation: &str) -> io::Result<Vec<String>> {
        let _guard = self.lock();
        let found = self.scan()?;
        let (mut index, _) = self.load_index(&found);
        let mut deleted = Vec::new();
        for f in found
            .iter()
            .filter(|f| f.name.item == Item::Ring && !f.reparse)
        {
            let current = index
                .entries
                .get(&f.file_name)
                .and_then(|e| e.generation.as_deref())
                == Some(generation);
            if !current && fsops::remove_file(&f.path)? {
                index.entries.remove(&f.file_name);
                deleted.push(f.file_name.clone());
            }
        }
        if !deleted.is_empty() {
            self.save_index(&index)?;
        }
        Ok(deleted)
    }
}

#[cfg(test)]
mod tests {
    use std::cmp::Reverse;
    use std::collections::BTreeSet;

    use chrono::{Days, NaiveDate, TimeZone};

    use super::*;

    const ID: &str = "0123456789abcdef0123456789abcdef";

    fn temp_dir() -> PathBuf {
        let mut n = [0u8; 8];
        random_bytes(&mut n);
        let dir = std::env::temp_dir().join(format!("axvault-backups-{}", hex::encode(n)));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Clears read-only everywhere first: backups are read-only.
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

    struct Fx {
        dir: PathBuf,
        config: BackupConfig,
        settings: BackupSettings,
        db: PathBuf,
        folder: Folder,
    }

    impl Drop for Fx {
        fn drop(&mut self) {
            cleanup(&self.dir);
        }
    }

    /// A database file and its folder in a default root, all under a fresh temp folder.
    fn fx() -> Fx {
        let dir = temp_dir();
        let db = dir.join("data").join("My Vault.kdbx");
        fs::create_dir_all(db.parent().unwrap()).unwrap();
        fs::write(&db, kdbx_bytes(0)).unwrap();
        let config = BackupConfig::new(dir.join("Backups"));
        let mut settings = BackupSettings::default();
        let folder = config
            .open_folder(&config.home(), &db, ID, &mut settings, true)
            .unwrap()
            .unwrap();
        Fx {
            dir,
            config,
            settings,
            db,
            folder,
        }
    }

    /// 600 bytes that pass the newest-backup check, distinct per `tag`.
    fn kdbx_bytes(tag: u64) -> Vec<u8> {
        let mut bytes = KDBX_SIGNATURE.to_vec();
        bytes.extend(tag.to_le_bytes());
        bytes.resize(600, 0xAB);
        bytes
    }

    fn name(stem: &str, date: &str, time: &str, seq: u64, item: Item) -> BackupName {
        BackupName {
            stem: stem.into(),
            date: date.into(),
            time: time.into(),
            seq,
            rev: 0,
            item,
        }
    }

    /// Creates a backup file directly, as another run of the app would have left it.
    fn put(folder: &Folder, date: &str, time: &str, seq: u64, kind: Kind) -> String {
        let file_name = name(folder.stem(), date, time, seq, Item::Kdbx(kind)).file_name();
        fs::write(folder.path().join(&file_name), kdbx_bytes(seq)).unwrap();
        file_name
    }

    fn kdbx_seqs(folder: &Folder) -> BTreeSet<u64> {
        folder
            .scan()
            .unwrap()
            .iter()
            .filter(|f| f.name.is_kdbx())
            .map(|f| f.name.seq)
            .collect()
    }

    fn day(start: NaiveDate, n: u64) -> String {
        start
            .checked_add_days(Days::new(n))
            .unwrap()
            .format("%Y-%m-%d")
            .to_string()
    }

    fn local(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> SystemTime {
        Local
            .with_ymd_and_hms(y, mo, d, h, mi, s)
            .earliest()
            .unwrap()
            .into()
    }

    fn set_mtime(path: &Path, t: SystemTime) {
        OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(t)
            .unwrap();
    }

    fn hours_ago(h: u64) -> SystemTime {
        SystemTime::now() - Duration::from_secs(h * 3600)
    }

    #[test]
    fn name_parse_roundtrip_any_stem() {
        let stems = [
            "My Vault",
            "Vault 2026-10-14 12-00-00",
            "a #000001",
            "x~1",
            "Stem #12~3 manual",
            "passkeys",
            "名前 ü",
            "a  b",
            "t",
        ];
        let items = Kind::ALL.map(Item::Kdbx).into_iter().chain([Item::Ring]);
        for stem in stems {
            for item in items.clone() {
                let revs: &[u32] = if item == Item::Ring {
                    &[0]
                } else {
                    &[0, 1, 12]
                };
                for &rev in revs {
                    for seq in [1, 42, 999_999, 1_000_000, u64::MAX] {
                        let n = BackupName {
                            rev,
                            ..name(stem, "2026-10-14", "09-05-00", seq, item)
                        };
                        let file_name = n.file_name();
                        assert_eq!(BackupName::parse(&file_name), Some(n), "{file_name}");
                    }
                }
            }
        }
        assert_eq!(
            name("V", "2026-10-14", "09-05-00", 7, Item::Kdbx(Kind::Manual)).file_name(),
            "V 2026-10-14 09-05-00 #000007 manual.kdbx"
        );
        assert_eq!(
            name("V", "2026-10-14", "09-05-00", 7, Item::Ring).file_name(),
            "V 2026-10-14 09-05-00 #000007 passkeys.axvault"
        );

        for foreign in [
            "X 12-00-00 (2).kdbx",
            "Stem 2026-10-14 12-00-00 (2).kdbx",
            "Stem 2026-10-14 12-00-00 #000001 (2).kdbx",
            "Stem 2026-10-14 12-00-00 #000001 auto.kdbx",
            "Stem 2026-10-14 12-00-00 #000001 weekly.kdbx",
            "Stem 2026-10-14 12-00-00 #000001 Manual.kdbx",
            "Stem 2026-10-14 12-00-00 #00001.kdbx",
            "Stem 2026-10-14 12-00-00 #0000001.kdbx",
            "Stem 2026-10-14 12-00-00 #000001~0.kdbx",
            "Stem 2026-10-14 12-00-00 #000001~01.kdbx",
            "Stem 2026-10-14 12-00-00 #000001~.kdbx",
            "Stem 2026-10-14 12-00-00 #000001 passkeys.kdbx",
            "Stem 2026-10-14 12-00-00 #000001.axvault",
            "Stem 2026-10-14 12-00-00 #000001 manual.axvault",
            "Stem 2026-10-14 12-00-00 #000001~1 passkeys.axvault",
            " 2026-10-14 12-00-00 #000001.kdbx",
            "2026-10-14 12-00-00 #000001.kdbx",
            "Stem 2026-1-14 12-00-00 #000001.kdbx",
            "Stem 2026-10-14 12:00:00 #000001.kdbx",
            "Stem 2026-10-14 12-00-00 000001.kdbx",
            "Stem 2026-10-14 12-00-00 #000001.KDBX",
            "Stem 2026-10-14 12-00-00 #000001.kdbx.tmp",
            "Stem 2026-10-14 12-00-00 #99999999999999999999.kdbx",
            "Stem 2026-10-14 12-00-00 #-00001.kdbx",
            "My Vault.kdbx",
            "notes.txt",
            ".~axv-0123456789abcdef.tmp",
        ] {
            assert_eq!(BackupName::parse(foreign), None, "{foreign}");
        }
    }

    #[test]
    fn sanitized_stems_are_safe_names() {
        assert_eq!(sanitize_stem("My Vault"), "My Vault");
        assert_eq!(
            sanitize_stem(r#"a<b>c:d"e/f\g|h?i*j"#),
            "a_b_c_d_e_f_g_h_i_j"
        );
        assert_eq!(sanitize_stem("tab\there"), "tab_here");
        assert_eq!(sanitize_stem("trailing. . "), "trailing");
        assert_eq!(sanitize_stem("...  "), "Vault");
        assert_eq!(sanitize_stem(""), "Vault");
        assert_eq!(sanitize_stem("con"), "_con");
        assert_eq!(sanitize_stem("LPT1.backup"), "_LPT1.backup");
        assert_eq!(sanitize_stem("console"), "console");
        let long = "é".repeat(70) + " x";
        assert_eq!(sanitize_stem(&long).chars().count(), 64);
        // Capping never leaves a trailing space.
        let spaced = format!("{} y", "a".repeat(63));
        assert_eq!(sanitize_stem(&spaced), "a".repeat(63));
    }

    #[test]
    fn vault_id_names_the_root_group() {
        let uuid = [0x5A; 16];
        let id = vault_id(&uuid);
        let mut h = Sha256::new();
        h.update(b"axentra-vault/store-id/v1");
        h.update(uuid);
        assert_eq!(id, hex::encode(h.finalize())[..32]);
        assert_eq!(id.len(), 32);
        assert_ne!(id, vault_id(&[0x5B; 16]));

        let db = keepass::Database::new();
        assert_eq!(vault_id_of(&db), vault_id(db.root().id().uuid().as_bytes()));
    }

    #[test]
    fn foreign_files_are_never_touched() {
        let f = fx();
        for seq in 1..=25 {
            put(
                &f.folder,
                "2026-03-01",
                &format!("10-00-{seq:02}"),
                seq,
                Kind::Auto,
            );
        }
        let old = local(2024, 1, 1, 12, 0, 0);
        let foreign = [
            "notes.txt",
            "My Vault.kdbx",
            "X 12-00-00 (2).kdbx",
            "My Vault 2020-01-01 00-00-00 #000001 auto.kdbx",
            "My Vault 2020-01-01 00-00-00 #0000002.kdbx",
            "My Vault 2020-01-01 00-00-00 #000003 passkeys.kdbx",
            ".~axv-hello.tmp",
            ".~axv-0123456789ABCDEF.tmp",
        ];
        for (i, name) in foreign.iter().enumerate() {
            let path = f.folder.path().join(name);
            fs::write(&path, format!("foreign {i}")).unwrap();
            set_mtime(&path, old);
        }
        let young = f.folder.path().join(fsops::temp_name());
        fs::write(&young, b"in progress").unwrap();
        let dir_like = f
            .folder
            .path()
            .join("My Vault 2020-01-01 00-00-00 #000004.kdbx");
        fs::create_dir(&dir_like).unwrap();

        let report = f.folder.prune(None);
        assert_eq!(report.deleted.len(), 5, "{report:?}");
        assert_eq!(f.folder.sweep_temps().unwrap(), 0);

        for (i, name) in foreign.iter().enumerate() {
            assert_eq!(
                fs::read_to_string(f.folder.path().join(name)).unwrap(),
                format!("foreign {i}"),
                "{name}"
            );
        }
        assert_eq!(fs::read(&young).unwrap(), b"in progress");
        assert!(dir_like.is_dir());
        // Nor listed as backups.
        assert_eq!(f.folder.list().unwrap().len(), 20);
    }

    #[test]
    fn publish_never_replaces() {
        let f = fx();
        let mk = |seq| {
            name(
                f.folder.stem(),
                "2026-03-01",
                "10-00-00",
                seq,
                Item::Kdbx(Kind::Manual),
            )
        };
        let taken = f.folder.path().join(mk(1).file_name());
        fs::write(&taken, b"existing").unwrap();

        let mtime = local(2026, 3, 1, 10, 0, 0);
        let (path, file_name) = f.folder.publish(&kdbx_bytes(9), 1, mtime, mk).unwrap();
        assert_eq!(file_name, mk(2).file_name());
        assert_eq!(fs::read(&taken).unwrap(), b"existing");
        assert_eq!(fs::read(&path).unwrap(), kdbx_bytes(9));
        assert!(fs::metadata(&path).unwrap().permissions().readonly());
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);

        // Every name taken: an error, nothing replaced, no temp left.
        for seq in 3..=18 {
            fs::write(f.folder.path().join(mk(seq).file_name()), b"existing").unwrap();
        }
        let err = f.folder.publish(&kdbx_bytes(10), 3, mtime, mk).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        for seq in 3..=18 {
            assert_eq!(
                fs::read(f.folder.path().join(mk(seq).file_name())).unwrap(),
                b"existing"
            );
        }
        let temps = fs::read_dir(f.folder.path())
            .unwrap()
            .flatten()
            .filter(|e| fsops::is_temp_name(&e.file_name().to_string_lossy()))
            .count();
        assert_eq!(temps, 0);
    }

    #[test]
    fn written_backup_is_read_only_and_keeps_the_source_time() {
        let f = fx();
        let source = local(2025, 7, 4, 15, 30, 9) + Duration::from_millis(250);
        let out = f
            .folder
            .write_backup(&kdbx_bytes(1), Kind::Auto, Some(source), Some("g1"))
            .unwrap();
        let path = match out {
            WriteOutcome::Written(path) => path,
            other => panic!("not written: {other:?}"),
        };
        assert_eq!(
            path.file_name().unwrap().to_str().unwrap(),
            "My Vault 2025-07-04 15-30-09 #000001.kdbx"
        );
        let meta = fs::metadata(&path).unwrap();
        assert!(meta.permissions().readonly());
        assert_eq!(meta.modified().unwrap(), source);

        // A future time is not believed.
        let future = SystemTime::now() + Duration::from_secs(400 * 24 * 3600);
        let before = SystemTime::now() - Duration::from_secs(2);
        let out = f
            .folder
            .write_backup(&kdbx_bytes(2), Kind::Manual, Some(future), None)
            .unwrap();
        let mtime = fs::metadata(out.path()).unwrap().modified().unwrap();
        assert!(mtime >= before && mtime <= SystemTime::now());

        let listed = f.folder.list().unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[1].generation.as_deref(), Some("g1"));
        assert_eq!(
            listed[1].sha256.as_deref(),
            Some(&*sha256_hex(&kdbx_bytes(1)))
        );
    }

    #[test]
    fn seq_spans_kdbx_and_axvault() {
        let f = fx();
        let seq_of = |out: WriteOutcome| {
            BackupName::parse(out.path().file_name().unwrap().to_str().unwrap())
                .unwrap()
                .seq
        };
        let write = |tag| {
            f.folder
                .write_backup(&kdbx_bytes(tag), Kind::Auto, None, None)
                .unwrap()
        };
        let ring = |tag: u8| f.folder.write_ring_snapshot(&[tag; 40], None).unwrap();
        assert_eq!(seq_of(write(1)), 1);
        assert_eq!(seq_of(ring(1)), 2);
        assert_eq!(seq_of(write(2)), 3);
        let other = name("Old Name", "2026-01-01", "00-00-00", 10, Item::Ring).file_name();
        fs::write(f.folder.path().join(other), b"ring").unwrap();
        assert_eq!(seq_of(write(3)), 11);
        assert_eq!(seq_of(ring(2)), 12);
        // An identical ring snapshot is not written twice.
        assert!(matches!(ring(2), WriteOutcome::Duplicate(_)));
    }

    #[test]
    fn dedupe_skips_identical_newest_but_not_manual() {
        let f = fx();
        let (x, y) = (kdbx_bytes(1), kdbx_bytes(2));
        let write = |bytes: &[u8], kind| f.folder.write_backup(bytes, kind, None, None).unwrap();

        let WriteOutcome::Written(first) = write(&x, Kind::Auto) else {
            panic!("first copy skipped");
        };
        assert_eq!(
            write(&x, Kind::Auto),
            WriteOutcome::Duplicate(first.clone())
        );
        assert_eq!(write(&x, Kind::Opened), WriteOutcome::Duplicate(first));
        let WriteOutcome::Written(manual) = write(&x, Kind::Manual) else {
            panic!("manual copies are always written");
        };
        assert_eq!(write(&x, Kind::Auto), WriteOutcome::Duplicate(manual));
        assert!(matches!(write(&y, Kind::Auto), WriteOutcome::Written(_)));
        assert!(matches!(
            write(&y, Kind::Opened),
            WriteOutcome::Duplicate(_)
        ));
        // Only the newest counts.
        assert!(matches!(write(&x, Kind::Opened), WriteOutcome::Written(_)));
        assert_eq!(f.folder.list().unwrap().len(), 4);
    }

    /// The keep set computed another way: dates rise with seq in these tests, so "the last N
    /// distinct days" are simply the N latest dates.
    fn expected_keep(entries: &[(u64, String, Kind)]) -> BTreeSet<u64> {
        let mut keep = BTreeSet::new();
        keep.insert(entries.iter().map(|e| e.0).max().unwrap());
        keep.extend(entries.iter().filter(|e| e.2.is_pinned()).map(|e| e.0));
        let mut regular: Vec<_> = entries.iter().filter(|e| !e.2.is_pinned()).collect();
        regular.sort_by_key(|e| Reverse(e.0));
        keep.extend(regular.iter().take(KEEP_RECENT).map(|e| e.0));
        let mut by_day: BTreeMap<&str, u64> = BTreeMap::new();
        let mut by_month: BTreeMap<&str, u64> = BTreeMap::new();
        for e in &regular {
            let d = by_day.entry(&e.1).or_default();
            *d = (*d).max(e.0);
            let m = by_month.entry(&e.1[..7]).or_default();
            *m = (*m).max(e.0);
        }
        keep.extend(by_day.values().rev().take(KEEP_DAYS));
        keep.extend(by_month.values().rev().take(KEEP_MONTHS));
        keep
    }

    fn planned_keep(entries: &[(u64, String, Kind)]) -> BTreeSet<u64> {
        let mut sorted: Vec<_> = entries.iter().collect();
        sorted.sort_by_key(|e| Reverse(e.0));
        let slots: Vec<(Kind, &str)> = sorted.iter().map(|e| (e.2, e.1.as_str())).collect();
        sorted
            .iter()
            .zip(retention_keep(&slots))
            .filter(|(_, keep)| *keep)
            .map(|(e, _)| e.0)
            .collect()
    }

    /// Adds the entries one by one, pruning after each, as saves do.
    fn add_pruning(folder: &Folder, entries: &[(u64, String, Kind)]) {
        for (seq, date, kind) in entries {
            put(folder, date, "12-00-00", *seq, *kind);
            let report = folder.prune(None);
            assert!(
                report.aborted.is_none() && report.errors.is_empty(),
                "{report:?}"
            );
        }
    }

    #[test]
    fn prune_keep_set_exact() {
        let start = NaiveDate::from_ymd_opt(2024, 10, 1).unwrap();
        let entries: Vec<(u64, String, Kind)> = (0..400u64)
            .map(|i| {
                // About one save every 1.8 days over two years; every 7th on the day before's.
                let d = if i % 7 == 0 && i > 0 {
                    (i - 1) * 9 / 5
                } else {
                    i * 9 / 5
                };
                let kind = match i {
                    _ if i % 25 == 3 => Kind::Manual,
                    _ if i % 41 == 5 => Kind::Imported,
                    _ if i % 53 == 7 => Kind::BeforeKeyChange,
                    _ if i % 67 == 11 => Kind::BeforeRestore,
                    _ if i % 13 == 1 => Kind::Opened,
                    _ if i % 17 == 2 => Kind::ReplacedExternal,
                    _ => Kind::Auto,
                };
                (i + 1, day(start, d), kind)
            })
            .collect();
        let expected = expected_keep(&entries);
        let pinned = entries.iter().filter(|e| e.2.is_pinned()).count();
        assert!(pinned >= 25);
        let regular_kept = expected.len() - pinned;
        assert!(regular_kept <= 1 + KEEP_RECENT + KEEP_DAYS + KEEP_MONTHS);
        // Days and months reach past the 20 most recent.
        assert!(
            regular_kept > KEEP_RECENT + KEEP_MONTHS / 2,
            "{regular_kept}"
        );
        assert_eq!(planned_keep(&entries), expected);

        let f = fx();
        add_pruning(&f.folder, &entries);
        assert_eq!(kdbx_seqs(&f.folder), expected);
    }

    #[test]
    fn prune_has_no_clock_dependence() {
        let f = fx();
        let start = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
        let entries: Vec<(u64, String, Kind)> = (0..150u64)
            .map(|i| {
                // The newest six share a day, so moving five of them frees a day slot.
                let d = i.min(144) * 3 / 2;
                let kind = if i % 10 == 3 {
                    Kind::Manual
                } else {
                    Kind::Auto
                };
                (i + 1, day(start, d), kind)
            })
            .collect();
        add_pruning(&f.folder, &entries);
        let before = kdbx_seqs(&f.folder);

        // The newest five get dates 80 years ahead, as after a wildly wrong clock.
        for found in f.folder.scan().unwrap().iter().take(5) {
            let mut n = found.name.clone();
            let year: i32 = n.date[..4].parse().unwrap();
            n.date = format!("{}{}", year + 80, &n.date[4..]);
            fs::rename(&found.path, f.folder.path().join(n.file_name())).unwrap();
        }
        let report = f.folder.prune(None);
        assert!(report.aborted.is_none(), "{report:?}");
        let after = kdbx_seqs(&f.folder);
        assert!(after.is_subset(&before));
        assert!(
            before.len() - after.len() <= 2,
            "at most one day slot and one month slot: {:?}",
            report.deleted
        );
        // Here the shift does take a day slot (#145 now heads its day), so the case is real.
        assert!(!report.deleted.is_empty());
        assert!(after.contains(&150), "the newest is kept");
        for (seq, _, kind) in &entries {
            if kind.is_pinned() {
                assert!(after.contains(seq), "pinned #{seq}");
            }
        }
    }

    #[test]
    fn prune_after_13_month_gap_keeps_monthly_history() {
        let f = fx();
        let mut entries = Vec::new();
        // Fifteen months with three saves each: 2023-01 … 2024-03.
        for m in 0..15u32 {
            let (y, mo) = (2023 + (m / 12) as i32, m % 12 + 1);
            for d in [5, 15, 25] {
                entries.push((0, format!("{y}-{mo:02}-{d:02}"), Kind::Auto));
            }
        }
        // Nothing for 13 months, then a save a day for 40 days from 2025-05-01.
        let back = NaiveDate::from_ymd_opt(2025, 5, 1).unwrap();
        for d in 0..40 {
            entries.push((0, day(back, d), Kind::Auto));
        }
        for (i, e) in entries.iter_mut().enumerate() {
            e.0 = i as u64 + 1;
        }
        add_pruning(&f.folder, &entries);

        let left = kdbx_seqs(&f.folder);
        let before_gap: BTreeSet<&str> = entries
            .iter()
            .filter(|e| left.contains(&e.0) && e.1.as_str() < "2025")
            .map(|e| e.1.as_str())
            .collect();
        // The newest of each of the ten latest months before the gap: 12 months minus the two
        // after it.
        let monthly: BTreeSet<String> = (5..15u32)
            .map(|m| format!("{}-{:02}-25", 2023 + (m / 12) as i32, m % 12 + 1))
            .collect();
        assert_eq!(
            before_gap,
            monthly.iter().map(String::as_str).collect::<BTreeSet<_>>()
        );
        assert_eq!(left, expected_keep(&entries));
    }

    #[test]
    fn prune_orders_by_seq_not_name_time() {
        let f = fx();
        // 25 saves on one day whose name times run backwards (a clock set back, or the hour
        // repeated when daylight saving ends), with some times repeated exactly.
        for seq in 1..=25u64 {
            let minute = if seq == 13 { 60 - 12 } else { 60 - seq };
            put(
                &f.folder,
                "2026-11-01",
                &format!("01-{minute:02}-00"),
                seq,
                Kind::Auto,
            );
        }
        let report = f.folder.prune(None);
        assert!(report.aborted.is_none(), "{report:?}");
        let gone: Vec<u64> = report
            .deleted
            .iter()
            .map(|n| BackupName::parse(n).unwrap().seq)
            .collect();
        assert_eq!(
            gone,
            [1, 2, 3, 4, 5],
            "lowest seq first, whatever the times"
        );
        assert_eq!(kdbx_seqs(&f.folder), (6..=25).collect::<BTreeSet<_>>());

        // The highest seq is the newest even when its date is the oldest.
        let mut entries = vec![(30, "2001-01-01".to_owned(), Kind::Auto)];
        entries.extend((1..30).map(|s| (s, "2026-01-01".to_owned(), Kind::Auto)));
        let kept = planned_keep(&entries);
        assert!(kept.contains(&30));
        assert!(!kept.contains(&1));
    }

    #[test]
    fn prune_never_deletes_pinned() {
        let f = fx();
        let pinned_kinds = [
            Kind::Manual,
            Kind::BeforeRestore,
            Kind::BeforeKeyChange,
            Kind::Imported,
        ];
        let start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let entries: Vec<(u64, String, Kind)> = (0..300u64)
            .map(|i| {
                let kind = match i % 3 {
                    0 => pinned_kinds[(i / 3 % 4) as usize],
                    1 => Kind::Auto,
                    _ => Kind::Opened,
                };
                (i + 1, day(start, i), kind)
            })
            .collect();
        add_pruning(&f.folder, &entries);
        let left = kdbx_seqs(&f.folder);
        for (seq, _, kind) in &entries {
            if kind.is_pinned() {
                assert!(left.contains(seq), "pinned #{seq}");
            }
        }
        assert!(left.len() < entries.len());

        let mut many: Vec<(u64, String, Kind)> = (1..=1000)
            .map(|s| (s, day(start, s), Kind::Manual))
            .collect();
        many.extend((1001..=1030).map(|s| (s, day(start, s), Kind::Auto)));
        let kept = planned_keep(&many);
        assert!((1..=1000).all(|s| kept.contains(&s)));
    }

    /// A backlog (here 60 `opened` copies from unlocks without a save) is cleaned up a quarter
    /// at a time, oldest first, and never all at once; it drains instead of blocking clean-up.
    #[test]
    fn prune_works_through_a_backlog_a_quarter_at_a_time() {
        let f = fx();
        for seq in 1..=60 {
            put(
                &f.folder,
                "2026-03-01",
                &format!("10-{:02}-00", seq % 60),
                seq,
                Kind::Opened,
            );
        }
        let mut next_oldest = 1;
        let mut runs = 0;
        loop {
            let total = kdbx_seqs(&f.folder).len();
            let report = f.folder.prune(None);
            assert!(report.aborted.is_none() && report.warning().is_none());
            assert!(report.deleted.len() <= (total / 4).max(5), "{report:?}");
            for name in &report.deleted {
                assert_eq!(BackupName::parse(name).unwrap().seq, next_oldest);
                next_oldest += 1;
            }
            runs += 1;
            if report.deferred == 0 {
                break;
            }
            assert!(runs < 10, "drains");
        }
        assert_eq!(runs, 4, "15, 11, 8, then the last 6");
        assert_eq!(kdbx_seqs(&f.folder), (41..=60).collect::<BTreeSet<_>>());
        assert_eq!(f.folder.prune(None).deleted.len(), 0);
    }

    #[test]
    fn prune_aborts_on_corrupt_newest() {
        let f = fx();
        let names: Vec<String> = (1..=25)
            .map(|seq| {
                put(
                    &f.folder,
                    "2026-03-01",
                    &format!("10-00-{seq:02}"),
                    seq,
                    Kind::Auto,
                )
            })
            .collect();
        let newest = f.folder.path().join(&names[24]);
        fs::OpenOptions::new()
            .write(true)
            .open(&newest)
            .unwrap()
            .set_len(100)
            .unwrap();
        let report = f.folder.prune(None);
        assert_eq!(report.aborted, Some(PruneAbort::SuspectNewest));
        assert_eq!(kdbx_seqs(&f.folder).len(), 25);

        fs::write(&newest, [0u8; 600]).unwrap();
        assert_eq!(
            f.folder.prune(None).aborted,
            Some(PruneAbort::SuspectNewest)
        );
        assert_eq!(kdbx_seqs(&f.folder).len(), 25);

        fs::write(&newest, kdbx_bytes(25)).unwrap();
        assert_eq!(f.folder.prune(None).deleted.len(), 5);
    }

    #[test]
    fn prune_aborts_on_foreign_vault_json() {
        let f = fx();
        for seq in 1..=25 {
            put(
                &f.folder,
                "2026-03-01",
                &format!("10-00-{seq:02}"),
                seq,
                Kind::Auto,
            );
        }
        let vault_json = f.folder.path().join(VAULT_FILE);
        let ours = fs::read(&vault_json).unwrap();
        fs::write(
            &vault_json,
            br#"{"v":1,"id":"ffffffffffffffffffffffffffffffff","stem":"Other"}"#,
        )
        .unwrap();
        assert_eq!(
            f.folder.prune(None).aborted,
            Some(PruneAbort::ForeignFolder)
        );
        fs::remove_file(&vault_json).unwrap();
        assert_eq!(
            f.folder.prune(None).aborted,
            Some(PruneAbort::ForeignFolder)
        );
        assert_eq!(kdbx_seqs(&f.folder).len(), 25);

        fs::write(&vault_json, ours).unwrap();
        assert_eq!(f.folder.prune(None).deleted.len(), 5);
    }

    /// Two PCs writing to one shared root can pick the same number for different backups. Both
    /// are backups of their own (listed, kept or pruned by the usual rules), and clean-up goes
    /// on.
    #[test]
    fn a_number_used_twice_is_two_backups() {
        let f = fx();
        for seq in 1..=25 {
            put(
                &f.folder,
                "2026-03-01",
                &format!("10-00-{seq:02}"),
                seq,
                Kind::Auto,
            );
        }
        let twin = put(&f.folder, "2026-03-02", "10-00-00", 3, Kind::Manual);
        let late_twin = put(&f.folder, "2026-03-02", "11-00-00", 25, Kind::Auto);
        assert_eq!(f.folder.list().unwrap().len(), 27);
        let report = f.folder.prune(None);
        assert!(report.aborted.is_none(), "{report:?}");
        assert_eq!(report.deleted.len(), 6, "{report:?}");
        let left: Vec<String> = f
            .folder
            .list()
            .unwrap()
            .into_iter()
            .map(|b| b.file_name)
            .collect();
        assert_eq!(left.len(), 21);
        assert!(left.contains(&twin), "pinned");
        assert!(left.contains(&late_twin));
    }

    /// Ring snapshots of other generations go even when no database backup may be deleted, but
    /// only once a snapshot of the session's generation exists.
    #[test]
    fn rings_are_cleaned_up_whatever_retention_decides() {
        let f = fx();
        for seq in 1..=25 {
            put(
                &f.folder,
                "2026-03-01",
                &format!("10-00-{seq:02}"),
                seq,
                Kind::Auto,
            );
        }
        let old = f
            .folder
            .write_ring_snapshot(b"old ring", Some("g1"))
            .unwrap();
        // The newest backup is damaged: retention stops, rings don't.
        put(&f.folder, "2026-03-02", "10-00-00", 30, Kind::Auto);
        fs::write(f.folder.list().unwrap()[0].path.clone(), b"short").unwrap();

        let report = f.folder.prune(Some("g2"));
        assert_eq!(report.aborted, Some(PruneAbort::SuspectNewest));
        assert!(report.rings_deleted.is_empty(), "no g2 snapshot yet");
        assert!(old.path().exists());

        f.folder
            .write_ring_snapshot(b"new ring", Some("g2"))
            .unwrap();
        let report = f.folder.prune(Some("g2"));
        assert_eq!(report.aborted, Some(PruneAbort::SuspectNewest));
        assert!(report.deleted.is_empty());
        assert_eq!(report.rings_deleted.len(), 1);
        assert!(!old.path().exists());
        assert_eq!(kdbx_seqs(&f.folder).len(), 26);
    }

    /// The same ring, snapshotted again under a generation that never committed (each unlock
    /// of a file without a matching record starts a new one), is relabelled, not copied again.
    #[test]
    fn ring_snapshot_of_a_provisional_generation_is_relabelled() {
        let f = fx();
        let first = f.folder.write_ring_snapshot(b"ring", Some("p1")).unwrap();
        assert!(matches!(first, WriteOutcome::Written(_)));
        assert!(matches!(
            f.folder.write_ring_snapshot(b"ring", Some("p2")).unwrap(),
            WriteOutcome::Duplicate(_)
        ));
        let rings = f.folder.list_rings().unwrap();
        assert_eq!(rings.len(), 1);
        assert_eq!(rings[0].generation.as_deref(), Some("p2"));
        // A committed generation's snapshot is kept as it is.
        f.folder
            .with_index(|index| index.note_generation("p2", true))
            .unwrap();
        assert!(matches!(
            f.folder.write_ring_snapshot(b"ring", Some("g3")).unwrap(),
            WriteOutcome::Written(_)
        ));
        assert_eq!(f.folder.list_rings().unwrap().len(), 2);
    }

    #[test]
    fn prune_skips_a_folder_being_staged() {
        let f = fx();
        for seq in 1..=25 {
            put(
                &f.folder,
                "2026-03-01",
                &format!("10-00-{seq:02}"),
                seq,
                Kind::Auto,
            );
        }
        let guard = f.folder.begin_staging();
        // Another handle on the same folder sees the job too.
        let other = f
            .config
            .open_folder(&f.config.home(), &f.db, ID, &mut f.settings.clone(), false)
            .unwrap()
            .unwrap();
        let report = other.prune(None);
        assert!(report.skipped && report.warning().is_none());
        assert_eq!(kdbx_seqs(&f.folder).len(), 25);
        drop(guard);
        assert_eq!(other.prune(None).deleted.len(), 5);
    }

    #[test]
    fn prune_clears_readonly_and_tolerates_missing() {
        let f = fx();
        for i in 0..25u32 {
            let t = local(2026, 3, 1, 10, 0, i);
            f.folder
                .write_backup(&kdbx_bytes(i.into()), Kind::Auto, Some(t), None)
                .unwrap();
        }
        let report = f.folder.prune(None);
        assert_eq!(report.deleted.len(), 5, "{report:?}");
        assert!(report.errors.is_empty(), "{report:?}");
        assert_eq!(kdbx_seqs(&f.folder), (6..=25).collect::<BTreeSet<_>>());

        // A file that vanishes between planning and deleting is no error.
        f.folder
            .write_backup(
                &kdbx_bytes(99),
                Kind::Auto,
                Some(local(2026, 3, 1, 10, 1, 0)),
                None,
            )
            .unwrap();
        let plan = f.folder.plan_prune(None).unwrap();
        assert_eq!(plan.delete.len(), 1);
        let doomed = plan.delete[0].clone();
        assert!(fsops::remove_file(&doomed.path).unwrap());
        let report = f.folder.execute_prune(plan);
        assert!(
            report.deleted.is_empty() && report.errors.is_empty(),
            "{report:?}"
        );
        let index: Index =
            serde_json::from_slice(&fs::read(f.folder.path().join(INDEX_FILE)).unwrap()).unwrap();
        assert!(!index.entries.contains_key(&doomed.file_name));
        assert_eq!(index.entries.len(), 20);
    }

    #[test]
    fn ring_snapshots_keep_three_of_session_gen() {
        let f = fx();
        f.folder
            .write_backup(&kdbx_bytes(1), Kind::Auto, None, Some("g2"))
            .unwrap();
        let ring = |tag: u8, generation| {
            f.folder
                .write_ring_snapshot(&[tag; 64], generation)
                .unwrap()
                .path()
                .to_owned()
        };
        let old: Vec<PathBuf> = (1..=2).map(|t| ring(t, Some("g1"))).collect();
        let unknown = f
            .folder
            .path()
            .join(name(f.folder.stem(), "2026-01-01", "00-00-00", 50, Item::Ring).file_name());
        fs::write(&unknown, b"ring from elsewhere").unwrap();
        let current: Vec<PathBuf> = (10..15).map(|t| ring(t, Some("g2"))).collect();

        assert!(f.folder.prune(None).rings_deleted.is_empty());
        assert_eq!(f.folder.list_rings().unwrap().len(), 8);

        let report = f.folder.prune(Some("g2"));
        assert_eq!(report.rings_deleted.len(), 5, "{report:?}");
        let left: Vec<PathBuf> = f
            .folder
            .list_rings()
            .unwrap()
            .into_iter()
            .map(|b| b.path)
            .collect();
        assert_eq!(left, current[2..].iter().rev().cloned().collect::<Vec<_>>());
        assert!(old.iter().chain([&unknown]).all(|p| !p.exists()));
        assert_eq!(f.folder.list().unwrap().len(), 1);
    }

    #[test]
    fn index_rebuilt_when_missing_or_corrupt() {
        let f = fx();
        let bytes: Vec<Vec<u8>> = (1..=3).map(kdbx_bytes).collect();
        for b in &bytes {
            f.folder
                .write_backup(b, Kind::Auto, None, Some("g"))
                .unwrap();
        }
        f.folder.write_ring_snapshot(b"ring", Some("g")).unwrap();
        let listed = f.folder.list().unwrap();
        assert!(listed.iter().all(|b| b.generation.as_deref() == Some("g")));

        let index = f.folder.path().join(INDEX_FILE);
        for damage in [None, Some(&b"{not json"[..])] {
            match damage {
                None => fs::remove_file(&index).unwrap(),
                Some(junk) => fs::write(&index, junk).unwrap(),
            }
            let listed = f.folder.list().unwrap();
            assert_eq!(listed.len(), 3);
            for (b, expected) in listed.iter().zip(bytes.iter().rev()) {
                assert_eq!(b.generation, None);
                assert_eq!(b.sha256.as_deref(), Some(&*sha256_hex(expected)));
                assert_eq!(b.size, expected.len() as u64);
            }
            let rings = f.folder.list_rings().unwrap();
            assert_eq!(rings.len(), 1);
            assert_eq!(rings[0].generation, None);
            let rebuilt: Index = serde_json::from_slice(&fs::read(&index).unwrap()).unwrap();
            assert_eq!(rebuilt.entries.len(), 4);
        }
    }

    #[test]
    fn sweep_respects_age_and_manifest() {
        let f = fx();
        let temp = |age_hours: u64| {
            let path = f.folder.path().join(fsops::temp_name());
            fs::write(&path, b"temp").unwrap();
            set_mtime(&path, hours_ago(age_hours));
            path
        };
        let old = temp(2);
        let young = temp(0);
        let listed = temp(5);
        let name_of = |p: &Path| p.file_name().unwrap().to_str().unwrap().to_owned();
        f.folder
            .write_manifest(&RekeyManifest {
                to_gen: "g2".into(),
                items: vec![RekeyItem {
                    tmp: name_of(&listed),
                    original: "x".into(),
                    original_sha256: "0".into(),
                    tmp_sha256: "0".into(),
                }],
                ..RekeyManifest::new("g2")
            })
            .unwrap();
        assert_eq!(f.folder.sweep_temps().unwrap(), 1);
        assert!(!old.exists() && young.exists() && listed.exists());

        // An unreadable manifest might own any temp.
        let another = temp(3);
        fs::write(f.folder.path().join(MANIFEST_FILE), b"{broken").unwrap();
        assert!(f.folder.read_manifest().is_err());
        assert_eq!(f.folder.sweep_temps().unwrap(), 0);
        assert!(another.exists());

        f.folder.remove_manifest().unwrap();
        assert_eq!(f.folder.sweep_temps().unwrap(), 2);
        assert!(!another.exists() && !listed.exists() && young.exists());
    }

    /// Publishing moves each copy to `~rev+1` with its original's time, read-only and labelled,
    /// then deletes the original; it stays inside the folder whatever the manifest says, and
    /// running it again after an interruption changes nothing.
    #[test]
    fn publish_rekey_is_repeatable_and_stays_in_the_folder() {
        let f = fx();
        let original = put(&f.folder, "2026-03-01", "10-00-00", 7, Kind::Manual);
        let original_path = f.folder.path().join(&original);
        set_mtime(&original_path, local(2026, 3, 1, 10, 0, 0));
        fsops::set_readonly(&original_path, true).unwrap();
        let failed = put(&f.folder, "2026-03-02", "10-00-00", 8, Kind::Auto);
        let copy = kdbx_bytes(70);
        let tmp = fsops::write_temp_in(f.folder.path(), &copy).unwrap();
        let tmp_name = tmp.file_name().unwrap().to_str().unwrap().to_owned();
        let outside = f.dir.join("outside.kdbx");
        fs::write(&outside, b"not in the store").unwrap();
        let manifest = RekeyManifest {
            to_gen: "g2".into(),
            items: vec![
                RekeyItem {
                    tmp: tmp_name.clone(),
                    original: original.clone(),
                    original_sha256: sha256_hex(&kdbx_bytes(7)),
                    tmp_sha256: sha256_hex(&copy),
                },
                RekeyItem {
                    tmp: tmp_name.clone(),
                    original: r"..\outside.kdbx".into(),
                    original_sha256: sha256_hex(b"not in the store"),
                    tmp_sha256: sha256_hex(&copy),
                },
            ],
            failed: vec![RekeyFailure {
                original: failed.clone(),
                why: "failed: lossy".into(),
            }],
            beside: Vec::new(),
        };
        f.folder.write_manifest(&manifest).unwrap();

        let report = f.folder.publish_rekey(&manifest).unwrap();
        let published = "My Vault 2026-03-01 10-00-00 #000007~1 manual.kdbx";
        assert_eq!(report.published, [published]);
        assert_eq!(report.removed, [original.as_str()]);
        assert_eq!(report.skipped.len(), 1, "{:?}", report.skipped);
        assert!(outside.exists() && !original_path.exists() && !tmp.exists());
        let path = f.folder.path().join(published);
        let meta = fs::metadata(&path).unwrap();
        assert!(meta.permissions().readonly());
        assert_eq!(meta.modified().unwrap(), local(2026, 3, 1, 10, 0, 0));
        assert_eq!(fs::read(&path).unwrap(), copy);
        let listed = f.folder.list().unwrap();
        let entry = |name: &str| listed.iter().find(|b| b.file_name == name).unwrap();
        assert_eq!(entry(published).generation.as_deref(), Some("g2"));
        assert_eq!(entry(&failed).rekey.as_deref(), Some("failed: lossy"));
        assert!(f.folder.index().unwrap().gens["g2"].committed);
        assert!(f.folder.read_manifest().unwrap().is_none());

        // Interrupted before the manifest went: the same publish again changes nothing.
        let index_before = f.folder.index().unwrap();
        let again = f.folder.publish_rekey(&manifest).unwrap();
        assert_eq!(again.published, [published]);
        assert!(again.removed.is_empty());
        assert_eq!(fs::read(&path).unwrap(), copy);
        assert_eq!(f.folder.index().unwrap().entries, index_before.entries);

        // Discarding deletes only the listed temps (in this folder) and the manifest.
        let tmp = fsops::write_temp_in(f.folder.path(), b"staged").unwrap();
        let discard = RekeyManifest {
            items: vec![
                RekeyItem {
                    tmp: tmp.file_name().unwrap().to_str().unwrap().to_owned(),
                    ..manifest.items[0].clone()
                },
                RekeyItem {
                    tmp: r"..\outside.kdbx".into(),
                    ..manifest.items[0].clone()
                },
            ],
            ..manifest.clone()
        };
        f.folder.write_manifest(&discard).unwrap();
        f.folder.discard_rekey(&discard).unwrap();
        assert!(!tmp.exists() && outside.exists() && path.exists());
        assert!(f.folder.read_manifest().unwrap().is_none());
    }

    /// An original another program holds open stays, and so does the manifest, until a later
    /// publish (the next unlock) can delete it.
    #[cfg(windows)]
    #[test]
    fn publish_rekey_retries_an_original_held_open() {
        #[cfg(windows)]
    use std::os::windows::fs::OpenOptionsExt;

        let f = fx();
        let original = put(&f.folder, "2026-03-01", "10-00-00", 3, Kind::Auto);
        let copy = kdbx_bytes(30);
        let tmp = fsops::write_temp_in(f.folder.path(), &copy).unwrap();
        let manifest = RekeyManifest {
            items: vec![RekeyItem {
                tmp: tmp.file_name().unwrap().to_str().unwrap().to_owned(),
                original: original.clone(),
                original_sha256: sha256_hex(&kdbx_bytes(3)),
                tmp_sha256: sha256_hex(&copy),
            }],
            ..RekeyManifest::new("g2")
        };
        f.folder.write_manifest(&manifest).unwrap();
        let held = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(f.folder.path().join(&original))
            .unwrap();
        let report = f.folder.publish_rekey(&manifest).unwrap();
        assert!(report.pending && report.removed.is_empty());
        assert_eq!(report.published.len(), 1);
        assert!(
            f.folder.read_manifest().unwrap().is_some(),
            "kept for later"
        );

        drop(held);
        let report = f.folder.publish_rekey(&manifest).unwrap();
        assert!(!report.pending);
        assert_eq!(report.removed, [original.as_str()]);
        assert!(f.folder.read_manifest().unwrap().is_none());
        let listed = f.folder.list_all().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name.rev, 1);
    }

    /// An original whose contents changed after staging stays the backup (marked as not
    /// re-encrypted) and the copy of its old contents goes, so no lower revision is left that
    /// clean-up would take for a re-encrypted backup's leftover.
    #[test]
    fn publish_rekey_keeps_an_original_that_changed() {
        let f = fx();
        let original = put(&f.folder, "2026-03-01", "10-00-00", 4, Kind::Auto);
        let copy = kdbx_bytes(40);
        let tmp = fsops::write_temp_in(f.folder.path(), &copy).unwrap();
        let manifest = RekeyManifest {
            items: vec![RekeyItem {
                tmp: tmp.file_name().unwrap().to_str().unwrap().to_owned(),
                original: original.clone(),
                original_sha256: sha256_hex(&kdbx_bytes(4)),
                tmp_sha256: sha256_hex(&copy),
            }],
            ..RekeyManifest::new("g2")
        };
        f.folder.write_manifest(&manifest).unwrap();
        fs::write(f.folder.path().join(&original), kdbx_bytes(99)).unwrap();
        let report = f.folder.publish_rekey(&manifest).unwrap();
        assert!(report.published.is_empty() && !report.pending, "{report:?}");
        assert_eq!(report.skipped.len(), 1);
        let all = f.folder.list_all().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].file_name, original);
        assert_eq!(all[0].rekey.as_deref(), Some("failed: changed"));
        assert_eq!(fs::read(&all[0].path).unwrap(), kdbx_bytes(99));
        assert!(f.folder.superseded().unwrap().is_empty());
    }

    /// Lower revisions are left alone while a manifest owns them, and are otherwise the
    /// leftovers of a key change: listed by `superseded` and deleted.
    #[test]
    fn superseded_lower_revisions_wait_for_the_manifest() {
        let f = fx();
        let original = put(&f.folder, "2026-03-01", "10-00-00", 5, Kind::Manual);
        let mut n = BackupName::parse(&original).unwrap();
        n.rev = 1;
        fs::write(f.folder.path().join(n.file_name()), kdbx_bytes(50)).unwrap();
        let other = put(&f.folder, "2026-03-02", "10-00-00", 6, Kind::Auto);
        let staged = RekeyManifest {
            items: vec![RekeyItem {
                tmp: fsops::temp_name(),
                original: original.clone(),
                original_sha256: sha256_hex(&kdbx_bytes(5)),
                tmp_sha256: sha256_hex(&kdbx_bytes(50)),
            }],
            ..RekeyManifest::new("g2")
        };
        f.folder.write_manifest(&staged).unwrap();
        assert!(f.folder.superseded().unwrap().is_empty());
        fs::write(f.folder.path().join(MANIFEST_FILE), b"{broken").unwrap();
        assert!(f.folder.superseded().unwrap().is_empty(), "might own them");
        let guard = f.folder.begin_staging();
        // Only copies beside the database left to delete: it owns no revision.
        f.folder
            .write_manifest(&RekeyManifest {
                beside: vec![BesideCopy {
                    suffix: ".bak".into(),
                    sha256: "0".into(),
                }],
                ..RekeyManifest::new("g2")
            })
            .unwrap();
        assert!(f.folder.superseded().unwrap().is_empty(), "while staging");
        drop(guard);
        assert_eq!(
            f.folder.delete_superseded().unwrap(),
            std::slice::from_ref(&original)
        );
        let left: Vec<String> = f
            .folder
            .list_all()
            .unwrap()
            .into_iter()
            .map(|b| b.file_name)
            .collect();
        assert_eq!(left, [other, n.file_name()]);
    }

    #[test]
    fn ring_snapshots_of_other_generations_are_deleted() {
        let f = fx();
        f.folder
            .write_ring_snapshot(b"old ring", Some("g1"))
            .unwrap();
        f.folder.write_ring_snapshot(b"unknown ring", None).unwrap();
        let kept = f
            .folder
            .write_ring_snapshot(b"new ring", Some("g2"))
            .unwrap();
        let deleted = f.folder.delete_rings_except("g2").unwrap();
        assert_eq!(deleted.len(), 2);
        let rings = f.folder.list_rings().unwrap();
        assert_eq!(rings.len(), 1);
        assert_eq!(rings[0].path, kept.path());
        assert!(f.folder.delete_rings_except("g2").unwrap().is_empty());
    }

    #[test]
    fn folder_identity_move_vs_copy() {
        let dir = temp_dir();
        let config = BackupConfig::new(dir.join("Backups"));
        let home = config.home();
        let mut settings = BackupSettings::default();
        let db_at = |sub: &str, file: &str| {
            let p = dir.join(sub).join(file);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            p
        };
        let open = |db: &Path, settings: &mut BackupSettings| {
            config
                .open_folder(&home, db, ID, settings, true)
                .unwrap()
                .unwrap()
                .path()
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned()
        };

        let a = db_at("a", "Stem.kdbx");
        fs::write(&a, b"db").unwrap();
        assert_eq!(open(&a, &mut settings), "Stem");
        let json: serde_json::Value = serde_json::from_slice(
            &fs::read(config.default_root.join("Stem").join(VAULT_FILE)).unwrap(),
        )
        .unwrap();
        assert_eq!(json["id"], ID);
        assert!(json.get("path").is_none(), "no path in the store");

        // Moved: the old path is gone, so its folder is taken over.
        let b = db_at("b", "Stem.kdbx");
        fs::rename(&a, &b).unwrap();
        assert_eq!(open(&b, &mut settings), "Stem");
        assert!(!settings.vaults.contains_key(&path_key(&a)));
        assert_eq!(
            settings.vaults[&path_key(&b)].folder_in(&home.path),
            Some("Stem")
        );

        // Renamed in place: the same.
        let renamed = db_at("b", "Renamed.kdbx");
        fs::rename(&b, &renamed).unwrap();
        assert_eq!(open(&renamed, &mut settings), "Stem");

        // Copied: both paths exist, so the copy gets its own folder.
        let copy = db_at("c", "Stem.kdbx");
        fs::copy(&renamed, &copy).unwrap();
        assert_eq!(open(&copy, &mut settings), "Stem (2)");
        assert_eq!(open(&renamed, &mut settings), "Stem");
        assert_eq!(settings.vaults.len(), 2);

        // Settings lost (a reinstall): the existing folders are found again by id.
        let mut fresh = BackupSettings::default();
        assert_eq!(open(&renamed, &mut fresh), "Stem");
        assert_eq!(open(&copy, &mut fresh), "Stem (2)");

        // Another vault with the same file name gets a folder of its own.
        let other = config
            .open_folder(&home, &copy, "f".repeat(32).as_str(), &mut fresh, true)
            .unwrap()
            .unwrap();
        assert!(other.path().ends_with("Stem (3)"));
        cleanup(&dir);
    }

    /// A vault and its copy (the same vault id, a "Save As") each keep their own folder in every
    /// root: choosing another root, or falling back to the default one while it is unplugged,
    /// never makes them share one. Even with only the earlier one-root settings, the second to
    /// come along takes the next folder, not the first one's.
    #[test]
    fn folders_are_kept_per_root_and_copies_never_share() {
        let dir = temp_dir();
        let original = dir.join("OneDrive").join("Vault.kdbx");
        let copy = dir.join("Travel").join("Vault.kdbx");
        for db in [&original, &copy] {
            fs::create_dir_all(db.parent().unwrap()).unwrap();
            fs::write(db, b"db").unwrap();
        }
        let mut settings = BackupSettings::default();
        let name_in = |config: &BackupConfig, root: &KnownRoot, db: &Path, s: &mut _| {
            config
                .open_folder(root, db, ID, s, true)
                .unwrap()
                .unwrap()
                .path()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        };
        let config = BackupConfig::new(dir.join("Backups"));
        let default = config.default_known_root();
        assert_eq!(
            name_in(&config, &default, &original, &mut settings),
            "Vault"
        );
        assert_eq!(
            name_in(&config, &default, &copy, &mut settings),
            "Vault (2)"
        );

        // A USB root is chosen: each gets a folder there too.
        let usb = dir.join("usb");
        fs::create_dir(&usb).unwrap();
        settings.set_root(Some(RootSetting {
            path: usb.clone(),
            marker: ensure_marker(&usb).unwrap(),
        }));
        let config = BackupConfig::from_settings(dir.join("Backups"), &settings);
        let home = config.home();
        assert_eq!(name_in(&config, &home, &copy, &mut settings), "Vault");
        assert_eq!(
            name_in(&config, &home, &original, &mut settings),
            "Vault (2)"
        );

        // Unplugged: both fall back to the default root, each to its own folder.
        for (db, expected) in [(&copy, "Vault (2)"), (&original, "Vault")] {
            assert_eq!(name_in(&config, &default, db, &mut settings), expected);
            let entry = &settings.vaults[&path_key(db)];
            assert_eq!(entry.folders.len(), 2, "{entry:?}");
        }

        // Settings from before, one root per database (the USB folder), lost the default
        // root's: whichever comes first takes a folder, and the other never shares it.
        let mut one_root = settings.clone();
        for entry in one_root.vaults.values_mut() {
            entry.folders.retain(|f| same_path(&f.root, &usb));
        }
        let first = name_in(&config, &default, &copy, &mut one_root);
        let second = name_in(&config, &default, &original, &mut one_root);
        assert_ne!(first, second);
        assert_eq!(
            [first, second].into_iter().collect::<BTreeSet<_>>(),
            ["Vault".to_owned(), "Vault (2)".to_owned()].into()
        );
        cleanup(&dir);
    }

    /// Only a file named as a backup, in a vault's folder directly inside a known root, is a
    /// backup; another vault on the same drive is not.
    #[test]
    fn only_files_in_a_vaults_folder_are_backups() {
        let f = fx();
        let backup = f
            .folder
            .path()
            .join(put(&f.folder, "2026-03-01", "10-00-00", 1, Kind::Auto));
        assert_eq!(
            f.config.backup_root_of(&backup).map(|r| r.kind),
            Some(RootKind::Default)
        );
        let root = f.config.default_root.clone();
        let loose_backup_name = root.join(backup.file_name().unwrap());
        let other_vault = root.join("Vault.kdbx");
        let nested = root.join("Personal").join("Vault.kdbx");
        let no_vault_json = root.join("Other").join(backup.file_name().unwrap());
        fs::create_dir_all(nested.parent().unwrap()).unwrap();
        fs::create_dir_all(no_vault_json.parent().unwrap()).unwrap();
        for path in [&loose_backup_name, &other_vault, &nested, &no_vault_json] {
            fs::write(path, kdbx_bytes(1)).unwrap();
            assert!(f.config.root_containing(path).is_some());
            assert_eq!(f.config.backup_root_of(path), None, "{}", path.display());
        }
        assert_eq!(f.config.backup_root_of(&f.db), None);
    }

    #[test]
    fn custom_root_without_marker_is_unavailable_and_not_created() {
        let dir = temp_dir();
        let db = dir.join("v.kdbx");
        fs::write(&db, b"db").unwrap();
        let usb = dir.join("usb");
        let mut settings = BackupSettings::default();
        settings.set_root(Some(RootSetting {
            path: usb.clone(),
            marker: "00000000-0000-4000-8000-000000000000".into(),
        }));
        let config = BackupConfig::from_settings(dir.join("Backups"), &settings);
        let home = config.home();
        assert_eq!(home.kind, RootKind::Configured);
        let open = |settings: &mut BackupSettings| {
            config
                .open_folder(&home, &db, ID, settings, true)
                .map(|f| f.is_some())
        };

        assert_eq!(config.check_root(&home), Err(Unavailable::Missing));
        assert!(matches!(
            open(&mut settings),
            Err(BackupError::Unavailable(Unavailable::Missing))
        ));
        assert!(!usb.exists(), "a custom root is never created");

        fs::create_dir(&usb).unwrap();
        assert!(matches!(
            open(&mut settings),
            Err(BackupError::Unavailable(Unavailable::NoMarker))
        ));
        assert_eq!(
            fs::read_dir(&usb).unwrap().count(),
            0,
            "nothing created inside"
        );

        let written = ensure_marker(&usb).unwrap();
        assert_eq!(
            ensure_marker(&usb).unwrap(),
            written,
            "an existing marker is kept"
        );
        assert!(matches!(
            open(&mut settings),
            Err(BackupError::Unavailable(Unavailable::WrongMarker))
        ));
        assert_eq!(fs::read_dir(&usb).unwrap().count(), 1);
        assert!(settings.vaults.is_empty());
        assert!(
            !config.default_root.exists(),
            "no silent switch to the default root"
        );

        // With the marker from the settings it works, and the folder is created.
        settings.set_root(Some(RootSetting {
            path: usb.clone(),
            marker: written,
        }));
        let config = BackupConfig::from_settings(dir.join("Backups"), &settings);
        let folder = config
            .open_folder(&config.home(), &db, ID, &mut settings, true)
            .unwrap()
            .unwrap();
        assert!(folder.path().starts_with(&usb));
        assert!(folder.path().join(VAULT_FILE).exists());
        folder
            .write_backup(&kdbx_bytes(1), Kind::Manual, None, None)
            .unwrap();
        assert_eq!(folder.list().unwrap().len(), 1);
        cleanup(&dir);
    }

    /// A save or unlock checks a custom root once: an answer is reused for a few seconds (by
    /// every clone of the config), so the checks at each step don't each cost a wait.
    #[test]
    fn root_checks_are_reused_briefly() {
        let dir = temp_dir();
        let usb = dir.join("usb");
        fs::create_dir(&usb).unwrap();
        let mut settings = BackupSettings::default();
        settings.set_root(Some(RootSetting {
            path: usb.clone(),
            marker: ensure_marker(&usb).unwrap(),
        }));
        let config = BackupConfig::from_settings(dir.join("Backups"), &settings);
        let home = config.home();
        assert_eq!(config.check_root(&home), Ok(()));
        fs::remove_file(usb.join(MARKER_FILE)).unwrap();
        assert_eq!(config.clone().check_root(&home), Ok(()), "reused");
        let fresh = BackupConfig::from_settings(dir.join("Backups"), &settings);
        assert_eq!(fresh.check_root(&home), Err(Unavailable::NoMarker));
        // A failure other than a timeout is asked again each time (a drive just plugged in).
        ensure_marker(&usb).unwrap();
        assert_eq!(fresh.check_root(&home), Err(Unavailable::WrongMarker));
        cleanup(&dir);
    }

    #[test]
    fn known_roots_and_settings_round_trip() {
        let dir = temp_dir();
        let root = |n: &str| RootSetting {
            path: dir.join(n),
            marker: format!("marker-{n}"),
        };
        let mut settings = BackupSettings::default();
        settings.set_root(Some(root("one")));
        settings.set_root(Some(root("two")));
        settings.set_root(Some(root("one")));
        assert_eq!(settings.root, Some(root("one")));
        assert_eq!(settings.previous_roots, [root("two")]);
        settings.set_root(None);
        assert_eq!(settings.previous_roots, [root("two"), root("one")]);
        settings.set_root(Some(root("two")));
        settings.migration_declined.push(ID.into());

        let config = BackupConfig::from_settings(dir.join("Backups"), &settings);
        let kinds: Vec<RootKind> = config.known_roots().iter().map(|r| r.kind).collect();
        assert_eq!(
            kinds,
            [RootKind::Configured, RootKind::Default, RootKind::Previous]
        );
        let inside = dir.join(if cfg!(windows) { "ONE" } else { "one" }).join("Stem").join("x.kdbx");
        assert_eq!(
            config.root_containing(&inside).map(|r| r.kind),
            Some(RootKind::Previous)
        );
        assert_eq!(config.root_containing(&dir.join("oneself.kdbx")), None);

        let file = dir.join("settings").join(SETTINGS_FILE);
        let db = dir.join("v.kdbx");
        fs::write(&db, b"db").unwrap();
        let default = KnownRoot {
            kind: RootKind::Default,
            path: config.default_root.clone(),
            marker: None,
        };
        // Every root's folder is recorded, not only the home root's.
        config
            .open_folder(&default, &db, ID, &mut settings, true)
            .unwrap()
            .unwrap();
        assert_eq!(
            settings.vaults[&path_key(&db)].folder_in(&config.default_root),
            Some("v")
        );
        settings
            .vaults
            .get_mut(&path_key(&db))
            .unwrap()
            .set_folder(&dir.join("two"), "v (2)");
        settings.save(&file).unwrap();
        assert_eq!(BackupSettings::load(&file), settings);

        // The earlier form, one root per database, still reads.
        let legacy = serde_json::json!({
            "vaults": { path_key(&db): { "id": ID, "root": dir.join("two"), "folder": "v" } }
        });
        fs::write(&file, serde_json::to_vec(&legacy).unwrap()).unwrap();
        let read = BackupSettings::load(&file);
        assert_eq!(
            read.vaults[&path_key(&db)].folder_in(&dir.join(if cfg!(windows){"TWO"}else{"two"})),
            Some("v")
        );
        assert_eq!(read.vaults[&path_key(&db)].folders.len(), 1);

        // Forgetting a previous root drops it and its folder records; the others stay.
        settings.set_root(None);
        assert!(settings.forget_root(&dir.join(if cfg!(windows){"ONE"}else{"one"})));
        assert!(!settings.forget_root(&dir.join("one")), "only once");
        assert!(
            !settings.forget_root(&config.default_root),
            "not a previous root"
        );
        assert_eq!(settings.previous_roots, [root("two")]);
        assert!(settings.forget_root(&dir.join("two")));
        assert!(settings.previous_roots.is_empty());
        assert_eq!(settings.vaults[&path_key(&db)].folders.len(), 1);
        fs::write(&file, b"{bad").unwrap();
        assert_eq!(BackupSettings::load(&file), BackupSettings::default());
        assert_eq!(
            BackupSettings::load(&dir.join("missing.json")),
            BackupSettings::default()
        );
        cleanup(&dir);
    }

    #[test]
    fn existing_folders_cover_every_known_root() {
        let dir = temp_dir();
        let db = dir.join("v.kdbx");
        fs::write(&db, b"db").unwrap();
        let mut settings = BackupSettings::default();
        let config = BackupConfig::new(dir.join("Backups"));
        assert_eq!(
            config.existing_folders(&db, ID, &mut settings).0.len(),
            0,
            "nothing is created"
        );
        assert!(!config.default_root.exists());
        let old = config
            .open_folder(&config.home(), &db, ID, &mut settings, true)
            .unwrap()
            .unwrap();

        // Move to a marked root, then away from it to one that is unplugged later.
        let usb = dir.join("usb");
        fs::create_dir(&usb).unwrap();
        let marker = ensure_marker(&usb).unwrap();
        settings.set_root(Some(RootSetting {
            path: usb.clone(),
            marker,
        }));
        let config = BackupConfig::from_settings(dir.join("Backups"), &settings);
        let on_usb = config
            .open_folder(&config.home(), &db, ID, &mut settings, true)
            .unwrap()
            .unwrap();
        settings.set_root(Some(RootSetting {
            path: dir.join("nas"),
            marker: "gone".into(),
        }));
        let config = BackupConfig::from_settings(dir.join("Backups"), &settings);
        let (folders, unavailable) = config.existing_folders(&db, ID, &mut settings);
        let paths: Vec<&Path> = folders.iter().map(Folder::path).collect();
        assert_eq!(paths, [old.path(), on_usb.path()]);
        assert_eq!(unavailable.len(), 1);
        assert_eq!(unavailable[0].0.kind, RootKind::Configured);
        assert_eq!(unavailable[0].1, Unavailable::Missing);
        // A previous root never gets a new folder.
        let previous = &config.known_roots()[2];
        assert_eq!(previous.kind, RootKind::Previous);
        assert!(
            config
                .open_folder(previous, &dir.join("other.kdbx"), ID, &mut settings, true)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            fs::read_dir(&usb).unwrap().count(),
            2,
            "marker and one folder"
        );
        cleanup(&dir);
    }

    #[test]
    fn free_space_rule() {
        let dir = temp_dir();
        let db = dir.join("v.kdbx");
        fs::write(&db, b"db").unwrap();
        let free = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let reported = free.clone();
        let config = BackupConfig::new(dir.join("Backups"))
            .with_free_space(move |_| Some(reported.load(Ordering::SeqCst)));
        let set = |n: u64| free.store(n, Ordering::SeqCst);

        // Another volume (no database given): the backup plus 64 MiB.
        set(10 * MIB + 64 * MIB);
        assert!(config.has_room(&dir, 10 * MIB, None));
        set(10 * MIB + 64 * MIB - 1);
        assert!(!config.has_room(&dir, 10 * MIB, None));
        // The database's volume: also twice the database plus 256 MiB.
        let need = 10 * MIB + 64 * MIB + 2 * 10 * MIB + 256 * MIB;
        set(need);
        assert!(config.has_room(&dir, 10 * MIB, Some((db.as_path(), 10 * MIB))));
        set(need - 1);
        assert!(!config.has_room(&dir, 10 * MIB, Some((db.as_path(), 10 * MIB))));

        let unknown = BackupConfig::new(dir.join("Backups")).with_free_space(|_| None);
        assert!(unknown.has_room(&dir, u64::MAX, Some((db.as_path(), u64::MAX))));
        cleanup(&dir);
    }

    #[test]
    fn delete_clears_readonly_and_refuses_other_names() {
        let f = fx();
        let out = f
            .folder
            .write_backup(&kdbx_bytes(1), Kind::Manual, None, None)
            .unwrap();
        let file_name = out.path().file_name().unwrap().to_str().unwrap().to_owned();
        assert!(fs::metadata(out.path()).unwrap().permissions().readonly());
        assert!(matches!(
            f.folder.delete(&[VAULT_FILE]),
            Err(e) if e.kind() == io::ErrorKind::InvalidInput
        ));
        assert!(matches!(
            f.folder.delete(&[r"..\x 2026-01-01 00-00-00 #000001.kdbx"]),
            Err(e) if e.kind() == io::ErrorKind::InvalidInput
        ));
        assert_eq!(
            f.folder.delete(&[file_name.as_str()]).unwrap(),
            [file_name.as_str()]
        );
        assert!(!out.path().exists());
        assert!(f.folder.delete(&[file_name.as_str()]).unwrap().is_empty());
        assert!(f.folder.list().unwrap().is_empty());
        assert!(f.folder.path().join(VAULT_FILE).exists());
    }

    #[test]
    fn generations_are_noted_once_and_stay_committed() {
        let f = fx();
        f.folder
            .with_index(|index| index.note_generation("g1", false))
            .unwrap();
        let created = f
            .folder
            .with_index(|index| index.gens["g1"].clone())
            .unwrap();
        assert!(!created.committed);
        f.folder
            .with_index(|index| {
                index.note_generation("g1", true);
                index.note_generation("g1", false);
                index.note_generation("g2", true);
            })
            .unwrap();
        let index: Index =
            serde_json::from_slice(&fs::read(f.folder.path().join(INDEX_FILE)).unwrap()).unwrap();
        assert_eq!(index.gens.len(), 2);
        assert!(index.gens["g1"].committed && index.gens["g2"].committed);
        assert_eq!(index.gens["g1"].created, created.created);
    }
}
