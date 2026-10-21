//! The sync engine: pulls OneDrive's change feed into the local state,
//! pushes local changes back, and downloads file contents on demand.
//!
//! Conflict rules follow the Windows client: when both sides changed a file,
//! both copies are kept and the local one is renamed with this computer's
//! name; files deleted online but edited here are kept and re-uploaded.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Result};
use axentra_onedrive_graph::{
    DriveItem, DriveItemVersion, Graph, GraphError, QuickXorHash, TokenStore, UploadTarget,
};
use chrono::{DateTime, Local, TimeZone, Utc};
use futures_util::StreamExt;
use parking_lot::{Mutex, RwLock};
use tokio::sync::{broadcast, watch, Notify};

use crate::cloud_thumbs::CloudThumbs;
use crate::config::{Config, Paths};
use crate::db::{Activity, Db, Item, Store, Tombstone, MAX_DEPTH, ROOT};
use crate::names::{self, IgnoreRules};
use crate::notify::Notifier;
use crate::status::{human_size, Account, Issue, Status, Transfer};
use crate::thumbs::{self, Bucket};

const ACCOUNT_REFRESH: Duration = Duration::from_secs(600);
const BIG_DOWNLOAD_NOTICE: u64 = 50 * 1024 * 1024;
/// Rows per transaction when a whole folder tree changes, so the mount
/// keeps answering while a large drive is done.
const CHUNK: usize = 2000;
/// Name prefix of a synced item set aside while the change feed is applied.
const PENDING_PREFIX: &str = ".axentra-onedrive-pending-";
/// How many items [`Engine::take_rebadged`] names at most; past that, all.
const REBADGED_MAX: usize = 500;

pub fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

pub fn to_ms(t: Option<DateTime<Utc>>) -> i64 {
    t.map(|t| t.timestamp_millis()).unwrap_or_else(now_ms)
}

pub fn ms_to_system(ms: i64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(ms.max(0) as u64)
}

pub fn ms_to_utc(ms: i64) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(ms)
        .single()
        .unwrap_or_else(Utc::now)
}

fn graph_err(e: &anyhow::Error) -> Option<&GraphError> {
    e.downcast_ref::<GraphError>()
}

#[derive(Debug, Clone)]
pub struct Options {
    /// Show desktop notifications (off in tests).
    pub notifications: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            notifications: true,
        }
    }
}

#[derive(Default, Clone, Copy)]
struct OpenCount {
    all: u32,
    writers: u32,
}

/// What a pass over the change feed did, for side effects after the commit.
#[derive(Default)]
struct PullOutcome {
    initial: bool,
    uncache: Vec<u64>,
    deleted_online: Vec<String>,
    conflicts: Vec<String>,
    /// Kernel dentries and inodes to drop so the change shows immediately.
    stale_entries: Vec<(u64, String)>,
    stale_inodes: Vec<u64>,
    /// Cloud thumbnails of items that changed, moved or went away.
    stale_thumbs: Vec<String>,
    /// Items whose badge changed with no activity entry to say so.
    rebadged: Vec<String>,
}

/// Items whose last attempt failed for a reason that retrying right away
/// won't fix (OneDrive is full, no permission), and when to try each again:
/// after a minute, doubling up to half an hour.
#[derive(Default)]
struct Backoff(Mutex<HashMap<u64, (Instant, u32)>>);

impl Backoff {
    fn due(&self, ino: u64) -> bool {
        self.0
            .lock()
            .get(&ino)
            .is_none_or(|(at, _)| Instant::now() >= *at)
    }

    fn failed(&self, ino: u64) {
        let mut m = self.0.lock();
        let (at, n) = m.entry(ino).or_insert((Instant::now(), 0));
        *n += 1;
        *at = Instant::now() + Duration::from_secs((60 << (*n - 1).min(5)).min(1800));
    }

    fn clear(&self, ino: u64) {
        self.0.lock().remove(&ino);
    }

    fn reset(&self) {
        self.0.lock().clear();
    }
}

/// A transfer in the flyout's list, removed when it ends or is abandoned
/// (a stopped pass drops its uploads mid-way).
struct TransferGuard<'a> {
    engine: &'a Engine,
    key: (u64, bool),
}

impl Drop for TransferGuard<'_> {
    fn drop(&mut self) {
        self.engine.end_transfer(self.key);
    }
}

/// A download under way. Its partial file goes when it ends or is
/// abandoned (paused, signed out), and writing over the file here stops it.
struct Downloading<'a> {
    engine: &'a Engine,
    ino: u64,
    part: PathBuf,
    stop: Arc<Notify>,
}

/// A download under way, as [`Engine::downloads`] keeps it.
struct Running {
    /// Notified to stop it: the file was written over or deleted here, or,
    /// fetched in the background, isn't to be kept on this device any more.
    stop: Arc<Notify>,
    /// For "Always keep on this device", with no program waiting for it.
    background: bool,
}

impl Drop for Downloading<'_> {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.part);
        self.engine.downloads.lock().remove(&self.ino);
    }
}

/// OneDrive's parent of each item looked up while pushing local changes,
/// `None` at the top (or gone).
type Parents = HashMap<String, Option<String>>;

/// What a page of the change feed is applied with: found out beforehand,
/// as it takes the network or reading files.
#[derive(Default)]
struct PageContext {
    /// Moved here but not online yet, inside a folder deleted online, and
    /// gone with it there (rather than moved in from somewhere else).
    gone: HashSet<String>,
    /// QuickXorHashes of local copies with changes that the page has
    /// changed online too, and the generation hashed, by inode.
    hashes: HashMap<u64, (i64, String)>,
}

pub struct Engine {
    pub paths: Paths,
    cfg: RwLock<Config>,
    pub db: Db,
    pub graph: Graph,
    pub rt: tokio::runtime::Handle,
    host: String,
    ignore: RwLock<IgnoreRules>,
    wake: Notify,
    changed_tx: broadcast::Sender<()>,
    hydrate_locks: Mutex<HashMap<u64, Arc<tokio::sync::Mutex<()>>>>,
    /// Downloads under way, by inode.
    downloads: Mutex<HashMap<u64, Running>>,
    opens: Mutex<HashMap<u64, OpenCount>>,
    /// By inode and whether it's an upload: a file overwritten while it
    /// downloads uploads at the same time.
    transfers: Mutex<HashMap<(u64, bool), Transfer>>,
    signed_in: AtomicBool,
    cycle_running: AtomicBool,
    /// Held for the length of a sync pass.
    pass: tokio::sync::Mutex<()>,
    /// Bumped to stop the pass in progress (pause, sign-out).
    stop: watch::Sender<u64>,
    upload_retry: Backoff,
    pinned_retry: Backoff,
    offline: AtomicBool,
    last_error: Mutex<Option<String>>,
    last_sync: AtomicI64,
    paused_until: AtomicI64,
    pinned_running: AtomicBool,
    /// More to keep on this device was asked for since the background
    /// downloads made their list.
    pinned_again: AtomicBool,
    /// Trees moved into an "Always keep on this device" folder, still to
    /// be kept too: see [`Engine::pin_moved_in`].
    moved_in: Mutex<Vec<u64>>,
    /// Held while what's kept on this device changes, so the changes go in
    /// the order they were asked for.
    pinning: Mutex<()>,
    account: Mutex<Account>,
    account_due: Mutex<Option<tokio::time::Instant>>,
    notifier: Arc<Notifier>,
    kernel: OnceLock<fuser::Notifier>,
    pub(crate) thumbs: CloudThumbs,
    /// Paths of items whose badge changed with nothing in the activity feed
    /// to say so (shared or unshared online, say); "/" once there are too
    /// many to name.
    rebadged: Mutex<BTreeSet<String>>,
}

impl Engine {
    pub fn new(paths: Paths, cfg: Config, opts: Options) -> Result<Arc<Self>> {
        paths.ensure()?;
        let db = Db::open(&paths.db_file)?;
        let tokens = Arc::new(TokenStore::open(&paths.tokens_file, cfg.auth()));
        let graph = Graph::new(&cfg.graph_base, tokens);
        graph.upload_limit.set_rate(cfg.upload_limit_kbps * 1024);
        graph
            .download_limit
            .set_rate(cfg.download_limit_kbps * 1024);
        let account: Account = db
            .with(|s| s.kv("account"))
            .ok()
            .flatten()
            .and_then(|j| serde_json::from_str(&j).ok())
            .unwrap_or_default();
        let paused_until = db
            .with(|s| s.kv("paused_until"))
            .ok()
            .flatten()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let last_sync = db
            .with(|s| s.kv("last_sync"))
            .ok()
            .flatten()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let (changed_tx, _) = broadcast::channel(64);
        Ok(Arc::new(Self {
            signed_in: AtomicBool::new(paths.tokens_file.exists()),
            ignore: RwLock::new(IgnoreRules::new(&cfg.ignore)),
            cfg: RwLock::new(cfg),
            paths,
            db,
            graph,
            rt: tokio::runtime::Handle::current(),
            host: names::hostname(),
            wake: Notify::new(),
            changed_tx,
            hydrate_locks: Mutex::new(HashMap::new()),
            downloads: Mutex::new(HashMap::new()),
            opens: Mutex::new(HashMap::new()),
            transfers: Mutex::new(HashMap::new()),
            cycle_running: AtomicBool::new(false),
            pass: tokio::sync::Mutex::new(()),
            stop: watch::channel(0).0,
            upload_retry: Backoff::default(),
            pinned_retry: Backoff::default(),
            offline: AtomicBool::new(false),
            last_error: Mutex::new(None),
            last_sync: AtomicI64::new(last_sync),
            paused_until: AtomicI64::new(paused_until),
            pinned_running: AtomicBool::new(false),
            pinned_again: AtomicBool::new(false),
            moved_in: Mutex::new(Vec::new()),
            pinning: Mutex::new(()),
            account: Mutex::new(account),
            account_due: Mutex::new(None),
            notifier: Arc::new(Notifier::new(opts.notifications)),
            kernel: OnceLock::new(),
            thumbs: CloudThumbs::default(),
            rebadged: Mutex::new(BTreeSet::new()),
        }))
    }

    /// Clear what an earlier run left behind. Only once this process is the
    /// service (it owns the bus name): the files are whichever one's runs.
    pub fn tidy_up(&self) {
        // Thumbnails are named after the files' paths, so a new OneDrive
        // folder location orphans them all.
        let mount = self.cfg().mount_path().display().to_string();
        if self
            .db
            .with(|s| s.kv("thumbs_mount"))
            .ok()
            .flatten()
            .as_deref()
            != Some(mount.as_str())
        {
            if let Ok(names) = self.db.with(|s| s.take_all_thumbs()) {
                thumbs::remove(&self.paths.thumbnail_dir, &names);
            }
            let _ = self.db.with(|s| s.set_kv("thumbs_mount", &mount));
        }
        // Left by a pass that stopped part-way: copies of items gone since,
        // and unfinished downloads.
        self.sweep_cache();
    }

    /// Remove files in the cache that no item counts as its local copy.
    fn sweep_cache(&self) {
        let Ok(entries) = std::fs::read_dir(&self.paths.cache_dir) else {
            return;
        };
        for e in entries.flatten() {
            let name = e.file_name();
            let Some(name) = name.to_str() else { continue };
            let stray = if let Some(ino) = name.strip_suffix(".part") {
                // Unless this run is downloading it already.
                !ino.parse()
                    .is_ok_and(|ino: u64| self.downloads.lock().contains_key(&ino))
            } else if let Ok(ino) = name.parse::<u64>() {
                self.db
                    .with(|s| s.get(ino))
                    .is_ok_and(|row| row.is_none_or(|r| !r.cached && !r.dirty))
            } else {
                false
            };
            if stray {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }

    // ----- configuration & signals ----------------------------------------

    pub fn cfg(&self) -> Config {
        self.cfg.read().clone()
    }

    /// `cfg().cloud_thumbnails` without copying the config, for every readdir.
    pub(crate) fn cloud_thumbnails(&self) -> bool {
        self.cfg.read().cloud_thumbnails
    }

    pub fn set_config(&self, cfg: Config) {
        self.graph.set_base(&cfg.graph_base);
        self.graph.tokens().set_config(cfg.auth());
        self.graph
            .upload_limit
            .set_rate(cfg.upload_limit_kbps * 1024);
        self.graph
            .download_limit
            .set_rate(cfg.download_limit_kbps * 1024);
        *self.ignore.write() = IgnoreRules::new(&cfg.ignore);
        *self.cfg.write() = cfg;
        self.changed();
        self.wake();
    }

    /// Lets remote changes invalidate the kernel's caches of the mount.
    pub fn attach_kernel(&self, n: fuser::Notifier) {
        let _ = self.kernel.set(n);
    }

    /// Drop what a change made stale: thumbnails, and the kernel's caches of the mount.
    fn invalidate(&self, out: &PullOutcome) {
        thumbs::remove(&self.paths.thumbnail_dir, &out.stale_thumbs);
        let Some(k) = self.kernel.get() else { return };
        for (parent, name) in &out.stale_entries {
            let _ = k.inval_entry(fuser::INodeNo(*parent), OsStr::new(name));
        }
        for ino in &out.stale_inodes {
            let _ = k.inval_inode(fuser::INodeNo(*ino), 0, 0);
        }
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn is_ignored(&self, name: &str) -> bool {
        self.ignore.read().matches(name)
    }

    /// Ask the engine to sync soon (local change, "Sync now").
    pub fn wake(&self) {
        self.wake.notify_one();
    }

    /// Tell listeners (D-Bus, tray) that status or activity changed.
    pub fn changed(&self) {
        let _ = self.changed_tx.send(());
    }

    pub fn subscribe(&self) -> broadcast::Receiver<()> {
        self.changed_tx.subscribe()
    }

    /// Note items whose badge changed when the activity feed doesn't name
    /// them, so file managers look at them again.
    fn rebadge(&self, paths: impl IntoIterator<Item = String>) {
        let mut named = self.rebadged.lock();
        if named.contains("/") {
            return;
        }
        named.extend(paths);
        if named.len() > REBADGED_MAX {
            named.clear();
            named.insert("/".into());
        }
    }

    /// The items whose badge changed since the last call besides those in
    /// the activity feed, by path from the top of the OneDrive folder ("/"
    /// for anything in it).
    pub fn take_rebadged(&self) -> Vec<String> {
        std::mem::take(&mut *self.rebadged.lock())
            .into_iter()
            .collect()
    }

    fn notify(&self, summary: String, body: String) {
        let n = self.notifier.clone();
        self.rt.spawn(async move { n.send(&summary, &body).await });
    }

    pub fn cache_path(&self, ino: u64) -> PathBuf {
        self.paths.cache_dir.join(ino.to_string())
    }

    fn uncache(&self, ino: u64) {
        let _ = std::fs::remove_file(self.cache_path(ino));
    }

    /// Remove local copies the database stopped counting as on this device.
    /// Checked again under the lock: a copy downloaded or written since stays.
    fn drop_stale_copies(&self, inos: &[u64]) {
        for &ino in inos {
            let _ = self.db.with(|s| -> Result<()> {
                if s.get(ino)?.is_none_or(|r| !r.cached && !r.dirty) {
                    self.uncache(ino);
                }
                Ok(())
            });
        }
    }

    fn notify_conflicts(&self, names: &[String]) {
        if !self.cfg().notify_conflicts {
            return;
        }
        match names {
            [] => {}
            [one] => self.notify(
                "Kept both copies of a file".into(),
                format!("It changed here and online, so your version is saved as \"{one}\"."),
            ),
            many => self.notify(
                format!("Kept both copies of {} files", many.len()),
                "They changed here and online. Your versions have this PC's name added.".into(),
            ),
        }
    }

    // ----- open-file tracking (called by the filesystem) -------------------

    pub fn opened(&self, ino: u64, write: bool) {
        let mut opens = self.opens.lock();
        let c = opens.entry(ino).or_default();
        c.all += 1;
        if write {
            c.writers += 1;
        }
    }

    pub fn closed(&self, ino: u64, write: bool) {
        let mut opens = self.opens.lock();
        if let Some(c) = opens.get_mut(&ino) {
            c.all = c.all.saturating_sub(1);
            if write {
                c.writers = c.writers.saturating_sub(1);
            }
            if c.all == 0 {
                opens.remove(&ino);
            }
        }
    }

    pub fn is_open(&self, ino: u64) -> bool {
        self.opens.lock().contains_key(&ino)
    }

    pub fn has_writers(&self, ino: u64) -> bool {
        self.opens.lock().get(&ino).is_some_and(|c| c.writers > 0)
    }

    /// Content of a cached file changed locally: queue it for upload.
    pub fn local_content_changed(&self, ino: u64) -> Result<()> {
        // A new version: worth trying again even if the last one failed.
        self.upload_retry.clear(ino);
        let meta = std::fs::metadata(self.cache_path(ino)).ok();
        let stale = self.db.with(|s| -> Result<Vec<String>> {
            let stale = s.take_thumb(ino)?;
            if let Some(mut it) = s.get(ino)? {
                it.dirty = true;
                it.cached = true;
                it.gen += 1;
                if let Some(m) = &meta {
                    it.size = m.len();
                    it.mtime = m
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                        .map(|d| d.as_millis() as i64)
                        .unwrap_or_else(now_ms);
                }
                s.update(&it)?;
            }
            Ok(stale)
        })?;
        thumbs::remove(&self.paths.thumbnail_dir, &stale);
        // Overwritten while it was downloading: what was coming is old now.
        self.stop_download(ino);
        self.changed();
        self.wake();
        Ok(())
    }

    /// Stop the download of `ino` under way, if there is one: what it would
    /// bring is no use any more (overwritten or deleted here).
    pub fn stop_download(&self, ino: u64) {
        if let Some(d) = self.downloads.lock().get(&ino) {
            d.stop.notify_one();
        }
    }

    /// Stop what "Always keep on this device" is fetching in the background
    /// of `ino` and everything inside it, no longer to be kept here.
    fn stop_kept_downloads(&self, ino: u64) {
        let running: Vec<(u64, Arc<Notify>)> = self
            .downloads
            .lock()
            .iter()
            .filter(|(_, d)| d.background)
            .map(|(&i, d)| (i, d.stop.clone()))
            .collect();
        for (i, stop) in running {
            if self.db.with(|s| s.is_inside(i, ino)).unwrap_or(false) {
                stop.notify_one();
            }
        }
    }

    // ----- transfers ------------------------------------------------------------

    fn begin_transfer(&self, ino: u64, name: &str, upload: bool, total: u64) -> TransferGuard<'_> {
        self.transfers.lock().insert(
            (ino, upload),
            Transfer {
                name: name.to_string(),
                upload,
                done: 0,
                total,
            },
        );
        self.changed();
        TransferGuard {
            engine: self,
            key: (ino, upload),
        }
    }

    fn progress(&self, ino: u64, upload: bool, done: u64) {
        if let Some(t) = self.transfers.lock().get_mut(&(ino, upload)) {
            t.done = done;
        }
    }

    fn end_transfer(&self, key: (u64, bool)) {
        self.transfers.lock().remove(&key);
        self.changed();
    }

    // ----- status -----------------------------------------------------------------

    pub fn paused_until(&self) -> Option<i64> {
        let t = self.paused_until.load(Ordering::Relaxed);
        (t > Utc::now().timestamp()).then_some(t)
    }

    pub fn pause(&self, secs: u64) {
        let until = Utc::now().timestamp() + secs as i64;
        self.paused_until.store(until, Ordering::Relaxed);
        let _ = self
            .db
            .with(|s| s.set_kv("paused_until", &until.to_string()));
        // Like the Windows client, transfers stop now, not when the pass ends.
        self.interrupt();
        self.changed();
    }

    /// Stop the sync pass in progress, if there is one.
    fn interrupt(&self) {
        self.stop.send_modify(|n| *n = n.wrapping_add(1));
    }

    pub fn resume(&self) {
        self.paused_until.store(0, Ordering::Relaxed);
        let _ = self.db.with(|s| s.del_kv("paused_until"));
        self.changed();
        self.wake();
    }

    pub fn status(&self) -> Status {
        let transfers: Vec<Transfer> = self.transfers.lock().values().cloned().collect();
        let (pending, issues) = self
            .db
            .with(|s| -> Result<(u64, Vec<Issue>)> {
                let issues = s
                    .errors()?
                    .into_iter()
                    .map(|i| Issue {
                        path: s.path(i.ino).unwrap_or_default(),
                        message: i.error.unwrap_or_default(),
                    })
                    .collect();
                Ok((s.pending_count()?, issues))
            })
            .unwrap_or_default();
        let uploading = transfers.iter().filter(|t| t.upload).count() as u32;
        let downloading = transfers.len() as u32 - uploading;
        let bytes_done = transfers.iter().map(|t| t.done).sum();
        let bytes_total = transfers.iter().map(|t| t.total).sum();
        let paused_until = self.paused_until();
        let last_sync = Some(self.last_sync.load(Ordering::Relaxed)).filter(|t| *t > 0);
        let last_error = self.last_error.lock().clone();

        let files = |n: u32| {
            if n == 1 {
                "1 file".to_string()
            } else {
                format!("{n} files")
            }
        };
        let (state, message, detail) = if !self.signed_in.load(Ordering::Relaxed) {
            (
                "signed_out",
                "You're not signed in".into(),
                "Sign in to start syncing your files".into(),
            )
        } else if let Some(t) = paused_until {
            (
                "paused",
                "Syncing is paused".into(),
                format!("Syncing will resume {}", resume_when(Local::now(), t)),
            )
        } else if uploading + downloading > 0 {
            let headline = match (uploading, downloading) {
                (u, 0) => format!("Uploading {}", files(u)),
                (0, d) => format!("Downloading {}", files(d)),
                (u, d) => format!("Syncing {}", files(u + d)),
            };
            (
                "syncing",
                headline,
                format!("{} of {}", human_size(bytes_done), human_size(bytes_total)),
            )
        } else if self.offline.load(Ordering::Relaxed) {
            (
                "offline",
                "You're offline".into(),
                "Your files will sync when you're back online".into(),
            )
        } else if !issues.is_empty() {
            let n = issues.len();
            (
                "error",
                if n == 1 {
                    "1 sync issue".into()
                } else {
                    format!("{n} sync issues")
                },
                issues[0].message.clone(),
            )
        } else if let Some(e) = last_error {
            ("error", "Axentra OneDrive couldn't sync".into(), e)
        } else if pending > 0 {
            (
                "syncing",
                "Processing changes".into(),
                if pending == 1 {
                    "1 change waiting to sync".into()
                } else {
                    format!("{pending} changes waiting to sync")
                },
            )
        } else if last_sync.is_none() {
            let listed = self.db.with(|s| s.item_count()).unwrap_or(0);
            let detail = if listed > 0 {
                format!(
                    "Listing your OneDrive: {} items so far. Files download only when you open them.",
                    group_digits(listed)
                )
            } else {
                String::new()
            };
            ("starting", "Getting your files ready".into(), detail)
        } else {
            ("up_to_date", "Your files are synced".into(), String::new())
        };
        Status {
            state: state.into(),
            message,
            detail,
            uploading,
            downloading,
            bytes_done,
            bytes_total,
            pending,
            issues,
            paused_until,
            last_sync,
            mount_point: self.cfg().mount_path().display().to_string(),
            transfers,
        }
    }

    pub fn account(&self) -> Account {
        let mut a = self.account.lock().clone();
        a.signed_in = self.signed_in.load(Ordering::Relaxed);
        a
    }

    pub fn activity(&self, limit: u32) -> Vec<Activity> {
        self.db.with(|s| s.activity(limit)).unwrap_or_default()
    }

    /// Refuse to sync another account's OneDrive into this PC's state: its
    /// item ids, delta link and unsynced changes belong to one drive.
    /// Unlinking this PC forgets the drive, so any account can sign in after.
    pub fn check_account(&self, drive_id: &str, drive_type: &str, email: &str) -> Result<()> {
        let linked = self.db.with(|s| s.kv("drive_id"))?;
        let account = self.account.lock().clone();
        let name = &account.email;
        // A personal and a work or school account can share an email address.
        let kind_differs = !account.drive_type.is_empty()
            && !drive_type.is_empty()
            && !account.drive_type.eq_ignore_ascii_case(drive_type);
        let other = match linked {
            // Graph doesn't keep to one case for personal drive ids, so a
            // case change is still the same drive.
            Some(linked) => !linked.eq_ignore_ascii_case(drive_id),
            // Linked before the drive was recorded: go by the account.
            None => !name.is_empty() && (!name.eq_ignore_ascii_case(email) || kind_differs),
        };
        if !other {
            return Ok(());
        }
        let (synced, wanted) = if name.is_empty() {
            ("a different OneDrive".to_string(), email.to_string())
        } else if kind_differs && name.eq_ignore_ascii_case(email) {
            (
                format!("the {} OneDrive of {name}", kind(&account.drive_type)),
                format!("your {} one", kind(drive_type)),
            )
        } else {
            (format!("the OneDrive of {name}"), email.to_string())
        };
        Err(anyhow!(
            "You're already syncing {synced} on this PC. Sign in with that account, \
             or unlink this PC to use {wanted} instead."
        ))
    }

    async fn refresh_account(&self) -> Result<()> {
        let me = self.graph.me().await?;
        let drive = self.graph.drive().await?;
        let email = me.mail.or(me.user_principal_name).unwrap_or_default();
        let drive_type = drive.drive_type.unwrap_or_default();
        self.check_account(&drive.id, &drive_type, &email)?;
        let _ = self.db.with(|s| s.set_kv("drive_id", &drive.id));
        let q = drive.quota.clone().unwrap_or_default();
        let a = Account {
            signed_in: true,
            name: me
                .display_name
                .or_else(|| {
                    drive
                        .owner
                        .and_then(|o| o.user)
                        .and_then(|u| u.display_name)
                })
                .unwrap_or_default(),
            email,
            drive_type,
            used: q.used.unwrap_or(0),
            total: q.total.unwrap_or(0),
            remaining: q.remaining.unwrap_or(0),
            quota_state: q.state.unwrap_or_else(|| "normal".into()),
            web_url: drive.web_url.unwrap_or_default(),
        };
        let _ = self
            .db
            .with(|s| s.set_kv("account", &serde_json::to_string(&a).unwrap_or_default()));
        *self.account.lock() = a;
        Ok(())
    }

    // ----- main loop ------------------------------------------------------------

    pub async fn run(self: Arc<Self>, mut shutdown: watch::Receiver<bool>) {
        loop {
            if *shutdown.borrow() {
                break;
            }
            tokio::select! {
                _ = self.cycle() => {}
                // Stop mid-pass rather than finishing a long first scan: every
                // database change is its own transaction, so nothing is left half-done.
                _ = shutdown.changed() => break,
            }
            let interval = Duration::from_secs(self.cfg().poll_interval_secs.max(2));
            tokio::select! {
                _ = tokio::time::sleep(interval) => {}
                _ = self.wake.notified() => {
                    // Let bursts of local changes settle into one pass.
                    tokio::time::sleep(Duration::from_millis(600)).await;
                }
                _ = shutdown.changed() => {}
            }
        }
    }

    /// One full sync pass. Public so tests can drive the engine step by step.
    pub async fn cycle(self: &Arc<Self>) {
        // Listening before taking the lock, so a stop sent just as this pass
        // gets it isn't missed.
        let mut stop = self.stop.subscribe();
        // One pass at a time; sign_out waits here for the pass it stopped.
        let _pass = self.pass.lock().await;
        let signed_in = self.graph.tokens().is_signed_in().await;
        self.signed_in.store(signed_in, Ordering::Relaxed);
        if self.paused_until.load(Ordering::Relaxed) != 0 && self.paused_until().is_none() {
            self.resume(); // the pause ran out
        }
        if !signed_in || self.paused_until().is_some() {
            self.changed();
            return;
        }
        self.cycle_running.store(true, Ordering::Relaxed);
        self.changed();
        let work = async {
            let due = self
                .account_due
                .lock()
                .is_none_or(|t| tokio::time::Instant::now() >= t);
            if due {
                self.refresh_account().await?;
                *self.account_due.lock() = Some(tokio::time::Instant::now() + ACCOUNT_REFRESH);
            }
            self.push_local().await?;
            self.pull_remote().await?;
            // Conflicts found while pulling produce new uploads.
            self.push_local().await?;
            Ok::<_, anyhow::Error>(())
        };
        // Pausing or signing out stops the pass where it is. Every database
        // change is its own transaction, so the next pass picks up from there.
        let result = tokio::select! {
            biased;
            _ = stop.changed() => None,
            r = work => Some(r),
        };
        let stopped = result.is_none();
        match result {
            None => tracing::info!("sync pass stopped"),
            Some(Ok(())) => {
                self.offline.store(false, Ordering::Relaxed);
                *self.last_error.lock() = None;
                let now = Utc::now().timestamp();
                self.last_sync.store(now, Ordering::Relaxed);
                let _ = self.db.with(|s| s.set_kv("last_sync", &now.to_string()));
            }
            Some(Err(e)) => match graph_err(&e) {
                Some(g) if g.is_offline() => {
                    self.offline.store(true, Ordering::Relaxed);
                }
                Some(GraphError::NotSignedIn) => {
                    self.signed_in.store(false, Ordering::Relaxed);
                    self.notify(
                        "Sign in to OneDrive".into(),
                        "Your session ended. Sign in again to keep syncing.".into(),
                    );
                }
                _ => {
                    tracing::warn!("sync pass failed: {e:#}");
                    self.offline.store(false, Ordering::Relaxed);
                    *self.last_error.lock() = Some(format!("{e:#}"));
                }
            },
        }
        self.cycle_running.store(false, Ordering::Relaxed);
        if !stopped {
            self.spawn_pinned_downloads();
        }
        if let Ok(names) = self.db.with(|s| s.take_orphan_thumbs()) {
            thumbs::remove(&self.paths.thumbnail_dir, &names);
        }
        self.changed();
    }

    // ----- remote → local ---------------------------------------------------------

    async fn pull_remote(&self) -> Result<()> {
        let mut out = PullOutcome::default();
        let result = self.pull_pages(&mut out).await;
        // Pages applied before a failure still get their notifications.
        self.after_pull(out);
        result
    }

    async fn pull_pages(&self, out: &mut PullOutcome) -> Result<()> {
        let mut restarts = 0;
        'restart: loop {
            let saved = self.db.with(|s| s.kv("delta_link"))?;
            let full = saved.is_none();
            // A first scan of a large drive takes many pages; resume it after
            // a restart instead of listing everything again.
            let resume = if full {
                self.db.with(|s| s.kv("scan_next"))?
            } else {
                None
            };
            if full && resume.is_none() {
                self.db.with(|s| s.mark_all_unseen())?;
            }
            out.initial = self.db.with(|s| s.kv("synced_once"))?.is_none();
            let mut link = saved.or(resume.clone());
            // Items whose parent hasn't come yet. The pages they came in
            // aren't sent again, so a scan resumed later needs them too.
            let mut orphans: Vec<DriveItem> = match &resume {
                Some(_) => self
                    .db
                    .with(|s| s.kv("scan_orphans"))?
                    .and_then(|j| serde_json::from_str(&j).ok())
                    .unwrap_or_default(),
                None => Vec::new(),
            };
            // Items that don't sync (OneNote notebooks, say) and everything
            // in them, so what's inside isn't kept waiting for its folder.
            let mut skipped: HashSet<String> = match &resume {
                Some(_) => self
                    .db
                    .with(|s| s.kv("scan_skipped"))?
                    .and_then(|j| serde_json::from_str(&j).ok())
                    .unwrap_or_default(),
                None => HashSet::new(),
            };
            loop {
                let page = match self.graph.delta_page(link.as_deref()).await {
                    Ok(p) => p,
                    Err(e) if e.status() == Some(410) && restarts < 3 => {
                        restarts += 1;
                        tracing::info!("change feed expired; doing a full resync");
                        self.db.with(|s| -> Result<()> {
                            s.del_kv("delta_link")?;
                            s.del_kv("scan_next")?;
                            s.del_kv("scan_orphans")?;
                            s.del_kv("scan_skipped")
                        })?;
                        continue 'restart;
                    }
                    Err(e) if resume.is_some() && e.status() == Some(400) && restarts < 3 => {
                        restarts += 1;
                        tracing::info!(
                            "saved scan position is no longer valid; starting the scan again"
                        );
                        self.db.with(|s| -> Result<()> {
                            s.del_kv("scan_next")?;
                            s.del_kv("scan_orphans")?;
                            s.del_kv("scan_skipped")
                        })?;
                        continue 'restart;
                    }
                    Err(e) => return Err(e.into()),
                };
                let mut items = std::mem::take(&mut orphans);
                items.extend(page.items);
                let ctx = self.prepare_page(&items).await?;
                let known = skipped.len();
                orphans = self
                    .db
                    .tx(|s| self.apply_items(s, items, &ctx, &mut skipped, out))?;
                // Now rather than at the end of the feed, which a pass that
                // stops part-way never reaches.
                self.settle(out);
                if let Some(next) = page.next_link {
                    if full {
                        for o in &mut orphans {
                            o.download_url = None; // pre-authenticated, and short-lived
                        }
                        self.db.tx(|s| -> Result<()> {
                            s.set_kv("scan_next", &next)?;
                            // Only grows, and most pages add nothing to it.
                            if skipped.len() != known {
                                s.set_kv("scan_skipped", &serde_json::to_string(&skipped)?)?;
                            }
                            if orphans.is_empty() {
                                s.del_kv("scan_orphans")
                            } else {
                                s.set_kv("scan_orphans", &serde_json::to_string(&orphans)?)
                            }
                        })?;
                    }
                    link = Some(next);
                    continue;
                }
                for o in &orphans {
                    tracing::debug!("skipping {} ({:?}): parent not synced", o.id, o.name);
                }
                if full {
                    self.db.tx(|s| {
                        let unseen = s.unseen()?;
                        // Moved here and not listed: gone online too.
                        let ctx = PageContext {
                            gone: unseen
                                .iter()
                                .filter(|i| i.moved)
                                .filter_map(|i| i.id.clone())
                                .collect(),
                            ..Default::default()
                        };
                        for it in unseen {
                            if let Some(row) = s.get(it.ino)? {
                                self.remote_deleted(s, row, &ctx, out)?;
                            }
                        }
                        Ok(())
                    })?;
                }
                self.db.with(|s| -> Result<()> {
                    if let Some(d) = &page.delta_link {
                        s.set_kv("delta_link", d)?;
                    }
                    s.del_kv("scan_next")?;
                    s.del_kv("scan_orphans")?;
                    s.del_kv("scan_skipped")?;
                    s.set_kv("synced_once", "1")
                })?;
                break;
            }
            return Ok(());
        }
    }

    /// What applying `items` needs to know from OneDrive and local files.
    async fn prepare_page(&self, items: &[DriveItem]) -> Result<PageContext> {
        let (moved, edited) = self.db.with(|s| -> Result<(Vec<String>, Vec<Item>)> {
            // Moved here, inside folders the page deletes.
            let mut moved = Vec::new();
            for it in items.iter().filter(|i| i.is_deleted()) {
                if let Some(dir) = s.by_id(&it.id)?.filter(|d| d.is_dir) {
                    let tree = s.subtree(dir.ino)?.into_iter().skip(1);
                    moved.extend(tree.filter(|d| d.moved).filter_map(|d| d.id));
                }
            }
            // Changed here, and by the page online too.
            let mut edited = Vec::new();
            if s.pending_count()? == 0 {
                return Ok((moved, edited));
            }
            for it in items.iter().filter(|i| !i.is_folder() && !i.is_deleted()) {
                let Some(online) = it.quick_xor_hash() else {
                    continue;
                };
                let row = match s.by_id(&it.id)? {
                    Some(row) => Some(row),
                    // Or a new file here by its name.
                    None => match (it.parent_id(), it.name.as_deref()) {
                        (Some(p), Some(name)) => match s.by_id(p)? {
                            Some(parent) => s.child(parent.ino, name)?.filter(|r| r.id.is_none()),
                            None => None,
                        },
                        _ => None,
                    },
                };
                if let Some(row) = row.filter(|r| {
                    !r.is_dir
                        && r.cached
                        && !r.local_only
                        && (r.dirty || r.id.is_none())
                        && r.qxh.as_deref() != Some(online)
                }) {
                    edited.push(row);
                }
            }
            Ok((moved, edited))
        })?;
        let mut ctx = PageContext::default();
        for id in moved {
            match self.graph.item(&id).await {
                Err(e) if e.is_not_found() => {
                    ctx.gone.insert(id);
                }
                Ok(it) if it.is_deleted() => {
                    ctx.gone.insert(id);
                }
                Err(e) if e.is_offline() || matches!(e, GraphError::NotSignedIn) => {
                    return Err(e.into())
                }
                // Still there, moved in from somewhere else (or can't tell).
                _ => {}
            }
        }
        for row in edited.into_iter().filter(|r| !self.has_writers(r.ino)) {
            if let Some(h) = local_hash(&self.cache_path(row.ino)).await {
                ctx.hashes.insert(row.ino, (row.gen, h));
            }
        }
        Ok(ctx)
    }

    /// Side effects of changes just committed: drop the local copies that
    /// went stale and what the kernel and thumbnailers cached of them.
    fn settle(&self, out: &mut PullOutcome) {
        let done = PullOutcome {
            uncache: std::mem::take(&mut out.uncache),
            stale_entries: std::mem::take(&mut out.stale_entries),
            stale_inodes: std::mem::take(&mut out.stale_inodes),
            stale_thumbs: std::mem::take(&mut out.stale_thumbs),
            ..Default::default()
        };
        let rebadged = std::mem::take(&mut out.rebadged);
        self.drop_stale_copies(&done.uncache);
        self.invalidate(&done);
        if !done.uncache.is_empty() || !rebadged.is_empty() {
            self.rebadge(rebadged);
            self.changed();
        }
    }

    fn after_pull(&self, mut out: PullOutcome) {
        self.settle(&mut out);
        let cfg = self.cfg();
        let n = out.deleted_online.len();
        if cfg.notify_deletes && n > 0 && n as u32 >= cfg.delete_warning_threshold.max(1) {
            self.notify(
                format!("{n} files were deleted from your OneDrive"),
                "They were removed from this device too. You can restore them from the OneDrive recycle bin."
                    .into(),
            );
        }
        self.notify_conflicts(&out.conflicts);
        if !out.conflicts.is_empty() || n > 0 {
            self.changed();
        }
    }

    /// Apply a batch of delta items; returns the ones whose parent is
    /// unknown. `skipped`: the ids of items that don't sync, and of
    /// everything in them, so far.
    fn apply_items(
        &self,
        s: &Store<'_>,
        mut items: Vec<DriveItem>,
        ctx: &PageContext,
        skipped: &mut HashSet<String>,
        out: &mut PullOutcome,
    ) -> Result<Vec<DriveItem>> {
        loop {
            let n = items.len();
            let mut orphans = Vec::new();
            for it in items {
                if !self.apply_one(s, &it, ctx, skipped, out)? {
                    orphans.push(it);
                }
            }
            // A parent later in the batch lets its children in now.
            if orphans.is_empty() || orphans.len() == n {
                return Ok(orphans);
            }
            items = orphans;
        }
    }

    fn apply_one(
        &self,
        s: &Store<'_>,
        it: &DriveItem,
        ctx: &PageContext,
        skipped: &mut HashSet<String>,
        out: &mut PullOutcome,
    ) -> Result<bool> {
        if it.root.is_some() {
            let mut root = s.get(ROOT)?.ok_or_else(|| anyhow!("root row missing"))?;
            root.id = Some(it.id.clone());
            root.etag = it.e_tag.clone();
            root.ctag = it.c_tag.clone();
            root.web_url = it.web_url.clone();
            root.mtime = to_ms(it.modified());
            s.update(&root)?;
            return Ok(true);
        }
        let existing = s.by_id(&it.id)?;
        if it.is_deleted() || it.package.is_some() || it.remote_item.is_some() {
            if !it.is_deleted() {
                skipped.insert(it.id.clone());
            }
            if let Some(row) = existing {
                self.remote_deleted(s, row, ctx, out)?;
            }
            return Ok(true);
        }
        let Some(parent_id) = it.parent_id() else {
            return Ok(true);
        };
        let Some(parent) = s.by_id(parent_id)? else {
            if !skipped.contains(parent_id) {
                return Ok(false);
            }
            // Inside something that doesn't sync (a OneNote notebook's
            // sections): neither does this, nor what's in it.
            skipped.insert(it.id.clone());
            if let Some(row) = existing {
                self.remote_deleted(s, row, ctx, out)?;
            }
            return Ok(true);
        };
        let name = it.name.clone().unwrap_or_default();
        if name.is_empty() {
            return Ok(true);
        }
        match existing {
            Some(row) => self.remote_updated(s, row, &parent, &name, it, ctx, out)?,
            None => self.remote_added(s, &parent, &name, it, ctx, out)?,
        }
        Ok(true)
    }

    /// `row` has changes here, and `it` changed online. That's no conflict
    /// when online is this PC's own version, an earlier upload of
    /// `target` whose answer was lost, or holds the same content anyway.
    /// Then: whether the local copy is that version (else it has newer
    /// changes still to go).
    fn own_version(
        &self,
        ctx: &PageContext,
        row: &Item,
        target: UploadTarget<'_>,
        it: &DriveItem,
    ) -> Option<bool> {
        let online = it.quick_xor_hash()?;
        let same = !self.has_writers(row.ino)
            && ctx
                .hashes
                .get(&row.ino)
                .is_some_and(|(gen, h)| *gen == row.gen && h == online);
        (same || self.graph.sent_before(target, online)).then_some(same)
    }

    /// Take `it` online as what `row` builds on; `same`: as its own version.
    fn adopt(&self, row: &mut Item, it: &DriveItem, same: bool) {
        Self::rebase(row, it);
        if same {
            row.dirty = false;
        }
        self.upload_retry.clear(row.ino);
    }

    fn content_changed(row: &Item, it: &DriveItem) -> bool {
        if row.is_dir {
            return false;
        }
        match (row.qxh.as_deref(), it.quick_xor_hash()) {
            (Some(a), Some(b)) => a != b,
            _ => row.ctag != it.c_tag,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn remote_updated(
        &self,
        s: &Store<'_>,
        mut row: Item,
        parent: &Item,
        name: &str,
        it: &DriveItem,
        ctx: &PageContext,
        out: &mut PullOutcome,
    ) -> Result<()> {
        let local_edit = row.dirty || self.has_writers(row.ino);
        let mut changed = Self::content_changed(&row, it);
        let badge = (row.shared, row.pinned);
        if changed && local_edit {
            match self.own_version(ctx, &row, UploadTarget::Existing(&it.id), it) {
                Some(same) => {
                    self.adopt(&mut row, it, same);
                    changed = false;
                }
                None => {
                    // Both sides changed: keep both, ours under a new name.
                    self.split_conflict(s, &mut row, out)?;
                    return self.remote_added(s, parent, name, it, ctx, out);
                }
            }
        }
        if (row.parent != parent.ino || row.name != name) && !row.moved {
            if row.parent != parent.ino && s.is_inside(parent.ino, row.ino)? {
                self.untangle(s, &row, parent.ino, out)?;
            }
            self.make_room(s, parent.ino, name, row.ino, out)?;
            out.stale_entries.push((row.parent, row.name.clone()));
            out.stale_thumbs.extend(s.take_thumbs_under(row.ino)?);
            row.parent = parent.ino;
            row.name = name.to_string();
            if parent.pinned && !row.pinned {
                row.pinned = true;
                // And everything inside; it downloads at the end of the pass.
                for mut d in s.subtree(row.ino)?.into_iter().skip(1) {
                    d.pinned = true;
                    s.update(&d)?;
                }
            }
        }
        row.etag = it.e_tag.clone();
        row.web_url = it.web_url.clone();
        row.shared = it.shared.is_some();
        if !local_edit {
            row.ctag = it.c_tag.clone();
            row.qxh = it.quick_xor_hash().map(str::to_string);
            if !row.is_dir {
                row.size = it.size.unwrap_or(0);
            }
            row.mtime = to_ms(it.modified());
        }
        if changed {
            out.stale_inodes.push(row.ino);
            out.stale_thumbs.extend(s.take_thumb(row.ino)?);
            if row.cached {
                row.cached = false;
                out.uncache.push(row.ino);
            }
            s.add_activity("updated_online", &row, "")?;
        }
        s.update(&row)?;
        if !changed && (row.shared, row.pinned) != badge {
            // Shared or unshared online, say: nothing in the activity feed
            // tells file managers to look at its badge again.
            out.rebadged.push(s.path(row.ino)?);
        }
        Ok(())
    }

    fn remote_added(
        &self,
        s: &Store<'_>,
        parent: &Item,
        name: &str,
        it: &DriveItem,
        ctx: &PageContext,
        out: &mut PullOutcome,
    ) -> Result<()> {
        if let Some(mut local) = s.child(parent.ino, name)? {
            // A folder created here and online with the same name: merge them.
            if local.id.is_none() && local.is_dir && it.is_folder() && !local.local_only {
                local.id = Some(it.id.clone());
                local.etag = it.e_tag.clone();
                local.ctag = it.c_tag.clone();
                local.web_url = it.web_url.clone();
                local.moved = local.name != name;
                return s.update(&local);
            }
            // A file created here that's online already: uploaded without
            // an answer, or the same file made there too.
            if local.id.is_none() && !local.is_dir && !it.is_folder() && !local.local_only {
                let own = parent.id.as_deref().and_then(|parent_id| {
                    let target = UploadTarget::New {
                        parent_id,
                        name: &local.name,
                    };
                    self.own_version(ctx, &local, target, it)
                });
                if let Some(same) = own {
                    self.adopt(&mut local, it, same);
                    local.moved = local.name != name;
                    return s.update(&local);
                }
            }
        }
        self.make_room(s, parent.ino, name, 0, out)?;
        let row = Item {
            id: Some(it.id.clone()),
            parent: parent.ino,
            name: name.to_string(),
            is_dir: it.is_folder(),
            size: if it.is_folder() {
                0
            } else {
                it.size.unwrap_or(0)
            },
            mtime: to_ms(it.modified()),
            etag: it.e_tag.clone(),
            ctag: it.c_tag.clone(),
            qxh: it.quick_xor_hash().map(str::to_string),
            web_url: it.web_url.clone(),
            shared: it.shared.is_some(),
            pinned: parent.pinned || !self.cfg().files_on_demand,
            local_only: parent.local_only,
            ..Default::default()
        };
        let ino = s.insert(&row)?;
        if !out.initial {
            let row = Item { ino, ..row };
            s.add_activity("added_online", &row, "")?;
        }
        Ok(())
    }

    /// Make sure no other row holds `name` in `parent`.
    fn make_room(
        &self,
        s: &Store<'_>,
        parent: u64,
        name: &str,
        except: u64,
        out: &mut PullOutcome,
    ) -> Result<()> {
        let Some(mut other) = s.child(parent, name)? else {
            return Ok(());
        };
        if other.ino == except {
            return Ok(());
        }
        if other.id.is_none() || other.local_only {
            // Created here, not uploaded yet: it becomes the "-PCNAME" copy.
            self.split_conflict(s, &mut other, out)
        } else if other.moved {
            // Renamed here to a name that is now taken online.
            self.rename_aside(s, &mut other, out)
        } else {
            // A synced item whose own rename or delete is later in the feed.
            out.stale_entries.push((other.parent, other.name.clone()));
            out.stale_thumbs.extend(s.take_thumbs_under(other.ino)?);
            other.name = format!("{PENDING_PREFIX}{}-{}", other.ino, other.name);
            s.update(&other)
        }
    }

    /// `row` is moving online into `dest`, which here is inside it because of
    /// a local move not pushed yet. The online move wins: the folder moved
    /// here on the way down to `dest` goes where `row` was, so the two don't
    /// end up inside each other. With none moved here, a folder's own move
    /// out of `row` is later in the feed, and puts it where it really is.
    fn untangle(&self, s: &Store<'_>, row: &Item, dest: u64, out: &mut PullOutcome) -> Result<()> {
        let mut chain = Vec::new();
        let mut cur = dest;
        while cur != row.ino && chain.len() < MAX_DEPTH {
            let Some(it) = s.get(cur)? else { break };
            cur = it.parent;
            chain.push(it);
        }
        let Some(mut top) = chain
            .iter()
            .rev()
            .find(|i| i.moved)
            .or(chain.last())
            .cloned()
        else {
            return Ok(());
        };
        out.stale_entries.push((top.parent, top.name.clone()));
        out.stale_thumbs.extend(s.take_thumbs_under(top.ino)?);
        top.parent = row.parent;
        if s.child(top.parent, &top.name)?.is_some() {
            self.take_conflict_name(s, &mut top, out)?;
        }
        s.update(&top)
    }

    /// The first conflict-copy name ("name-PCNAME") of `name` that is free in
    /// the folder `parent`.
    fn free_conflict_name(&self, s: &Store<'_>, parent: u64, name: &str) -> Result<String> {
        let mut n = 1;
        loop {
            let cand = names::conflict_name(name, &self.host, n);
            if s.child(parent, &cand)?.is_none() {
                return Ok(cand);
            }
            n += 1;
        }
    }

    /// Give `row` the first free conflict-copy name in its folder. Returns
    /// the name it had.
    fn take_conflict_name(
        &self,
        s: &Store<'_>,
        row: &mut Item,
        out: &mut PullOutcome,
    ) -> Result<String> {
        let new_name = self.free_conflict_name(s, row.parent, &row.name)?;
        let original = std::mem::replace(&mut row.name, new_name);
        out.stale_entries.push((row.parent, original.clone()));
        out.stale_thumbs.extend(s.take_thumbs_under(row.ino)?);
        Ok(original)
    }

    /// The name `row` was renamed to here is taken online: it gets the
    /// "-PCNAME" name instead and stays the same OneDrive item.
    fn rename_aside(&self, s: &Store<'_>, row: &mut Item, out: &mut PullOutcome) -> Result<()> {
        let original = self.take_conflict_name(s, row, out)?;
        row.moved = true;
        s.update(row)?;
        s.add_activity(
            "conflict",
            row,
            &format!("\"{original}\" is already taken online, so this item was renamed."),
        )
    }

    /// Rename a local item to the conflict-copy name and detach it from its
    /// OneDrive item so it uploads as a new file.
    fn split_conflict(&self, s: &Store<'_>, row: &mut Item, out: &mut PullOutcome) -> Result<()> {
        let original = self.take_conflict_name(s, row, out)?;
        row.id = None;
        row.etag = None;
        row.ctag = None;
        row.qxh = None;
        row.web_url = None;
        row.shared = false;
        row.moved = false;
        row.dirty = !row.is_dir;
        s.update(row)?;
        s.add_activity(
            "conflict",
            row,
            &format!("\"{original}\" changed here and online. Kept both copies."),
        )?;
        out.conflicts.push(row.name.clone());
        Ok(())
    }

    fn remote_deleted(
        &self,
        s: &Store<'_>,
        row: Item,
        ctx: &PageContext,
        out: &mut PullOutcome,
    ) -> Result<()> {
        let tree = s.subtree(row.ino)?;
        // Moved in here but not online yet: online they're somewhere else,
        // untouched by this delete, and the move goes on into the folders
        // kept for them. Not those only renamed or moved within it here,
        // which went with it online.
        let mut elsewhere = HashSet::new();
        for it in &tree {
            let moved_in = it.moved && it.id.as_ref().is_some_and(|id| !ctx.gone.contains(id));
            if it.ino != row.ino && (moved_in || elsewhere.contains(&it.parent)) {
                elsewhere.insert(it.ino);
            }
        }
        let mut kept_any = false;
        // Children first, so a folder knows whether anything inside survived.
        for it in tree.iter().rev() {
            if elsewhere.contains(&it.ino) {
                kept_any = true;
                continue;
            }
            let keep = if it.is_dir {
                s.child_count(it.ino)? > 0
            } else {
                it.dirty || it.id.is_none() || it.local_only || self.has_writers(it.ino)
            };
            if keep {
                kept_any = true;
                let mut k = it.clone();
                k.id = None;
                k.etag = None;
                k.ctag = None;
                k.web_url = None;
                k.moved = false;
                k.dirty = !k.is_dir && !k.local_only;
                if let Some(name) = pending_original(&k.name) {
                    // Set aside for a rename that turned out to be a delete:
                    // ours stays as the "-PCNAME" copy of its old name.
                    k.name = name.to_string();
                    self.take_conflict_name(s, &mut k, out)?;
                }
                s.update(&k)?;
            } else {
                s.delete(it.ino)?;
                out.stale_thumbs.extend(s.take_thumb(it.ino)?);
                out.stale_entries.push((it.parent, it.name.clone()));
                if it.cached {
                    out.uncache.push(it.ino);
                }
                if !it.is_dir {
                    out.deleted_online.push(it.name.clone());
                }
            }
        }
        if kept_any {
            let kept = s.get(row.ino)?.unwrap_or(row);
            s.add_activity(
                "conflict",
                &kept,
                "Deleted online, but changed here. Kept your version.",
            )?;
        } else {
            let path = s.path(row.parent)?;
            let folder = s.folder_name(row.parent)?;
            let full = format!("{}/{}", path.trim_end_matches('/'), row.name);
            s.add_activity_raw("deleted_online", &row.name, &folder, &full, row.is_dir, "")?;
        }
        Ok(())
    }

    // ----- local → remote ---------------------------------------------------------

    async fn push_local(self: &Arc<Self>) -> Result<()> {
        // Looked up once for all the steps, kept up to date by the moves.
        let mut parents = Parents::new();
        // Deletes first, so names freed here are free online too. Folders
        // wait while moves are pending, though: things may have moved out of them.
        self.push_deletes(false, &mut parents).await?;
        self.push_folders(&mut parents).await?;
        self.push_moves(&mut parents).await?;
        self.push_deletes(true, &mut parents).await?;
        self.push_uploads().await
    }

    /// Deleting a folder online takes along everything still in it there,
    /// including items moved out of it here whose move isn't online yet. So
    /// while anything is waiting to move, folders wait for `after_moves`, and
    /// even then for the moves that failed out of them.
    async fn push_deletes(&self, after_moves: bool, parents: &mut Parents) -> Result<()> {
        let tombstones = self.db.with(|s| s.tombstones())?;
        if tombstones.is_empty() {
            return Ok(());
        }
        let moving = self.db.with(|s| s.moved_ids())?;
        for t in tombstones {
            if t.is_dir
                && !moving.is_empty()
                && (!after_moves || self.holds_any(&t.id, &moving, parents).await?)
            {
                continue;
            }
            match self.graph.delete(&t.id).await {
                Ok(()) => {
                    let folder = folder_of(&t.path);
                    self.db.with(|s| -> Result<()> {
                        s.remove_tombstone(&t.id)?;
                        s.add_activity_raw("deleted", &t.name, &folder, &t.path, t.is_dir, "")
                    })?;
                    self.changed();
                }
                Err(e) if e.is_offline() || matches!(e, GraphError::NotSignedIn) => {
                    return Err(e.into())
                }
                Err(e) => tracing::warn!("couldn't delete {} online: {e}", t.path),
            }
        }
        Ok(())
    }

    /// Whether any of the items `ids` is, online, somewhere inside the folder
    /// `folder`. `parents` remembers online parents across calls.
    async fn holds_any(&self, folder: &str, ids: &[String], parents: &mut Parents) -> Result<bool> {
        // Where each item is: a few at a time, as for a reorganised photo
        // library that's thousands.
        let unknown: Vec<String> = ids
            .iter()
            .filter(|id| !parents.contains_key(*id))
            .cloned()
            .collect();
        let n = self.cfg().max_parallel_transfers.max(1);
        let found: Vec<Option<(String, Option<String>)>> = futures_util::stream::iter(unknown)
            .map(|id| async move {
                match self.graph.item(&id).await {
                    Ok(it) => {
                        let parent = it.parent_id().map(str::to_string);
                        Some((id, parent))
                    }
                    Err(e) if e.is_not_found() => Some((id, None)),
                    // Looked up again on the way up below, errors and all.
                    Err(_) => None,
                }
            })
            .buffer_unordered(n)
            .collect()
            .await;
        parents.extend(found.into_iter().flatten());
        for id in ids {
            let mut cur = id.clone();
            for _ in 0..MAX_DEPTH {
                let parent = match parents.get(&cur) {
                    Some(p) => p.clone(),
                    None => {
                        let p = match self.graph.item(&cur).await {
                            Ok(it) => it.parent_id().map(str::to_string),
                            Err(e) if e.is_not_found() => None,
                            Err(e) if e.is_offline() || matches!(e, GraphError::NotSignedIn) => {
                                return Err(e.into())
                            }
                            Err(e) => {
                                // Can't tell: keep the folder for now.
                                tracing::warn!("couldn't look up {cur} online: {e}");
                                return Ok(true);
                            }
                        };
                        parents.insert(cur.clone(), p.clone());
                        p
                    }
                };
                match parent {
                    Some(p) if p == folder => return Ok(true),
                    Some(p) => cur = p,
                    None => break,
                }
            }
        }
        Ok(false)
    }

    /// Whether `holder`, deleted here, stays online past this pass's moves
    /// because something moved out of it here is still in it there.
    async fn delete_must_wait(&self, holder: &DriveItem, parents: &mut Parents) -> Result<bool> {
        if !holder.is_folder() {
            return Ok(false);
        }
        let moving = self.db.with(|s| s.moved_ids())?;
        Ok(!moving.is_empty() && self.holds_any(&holder.id, &moving, parents).await?)
    }

    async fn push_folders(&self, parents: &mut Parents) -> Result<()> {
        loop {
            let dirs = self.db.with(|s| s.folders_to_create())?;
            if dirs.is_empty() {
                return Ok(());
            }
            // Only a folder that got its OneDrive item (or a new name) counts:
            // otherwise the same ones would be tried again forever.
            let mut progressed = false;
            for d in dirs {
                let Some(parent_id) = self.db.with(|s| s.get(d.parent))?.and_then(|p| p.id) else {
                    continue;
                };
                // The name it's made under online.
                let mut name = d.name.clone();
                let made = loop {
                    match self.graph.create_folder(&parent_id, &name).await {
                        Ok(it) => break Some((it, true)),
                        Err(e) if e.status() == Some(409) && name == d.name => {
                            match self.graph.child(&parent_id, &d.name).await? {
                                Some(h) if self.db.with(|s| s.is_tombstoned(&h.id))? => {
                                    // Deleted here, not online yet: the name is
                                    // free once that's pushed after the moves.
                                    if !self.delete_must_wait(&h, parents).await? {
                                        break None;
                                    }
                                    // It can't go before things move out of it,
                                    // perhaps into this folder: made under a free
                                    // name for now, renamed once that one's gone.
                                    name = self
                                        .db
                                        .with(|s| self.free_conflict_name(s, d.parent, &d.name))?;
                                }
                                Some(it) if it.is_folder() => break Some((it, false)),
                                _ => {
                                    // A file online has this name: keep ours as the "-PCNAME" copy.
                                    let out = self.db.tx(|s| {
                                        let mut out = PullOutcome::default();
                                        if let Some(mut row) = s.get(d.ino)? {
                                            self.split_conflict(s, &mut row, &mut out)?;
                                        }
                                        Ok(out)
                                    })?;
                                    self.invalidate(&out);
                                    progressed = true;
                                    break None;
                                }
                            }
                        }
                        Err(e) if e.is_offline() || matches!(e, GraphError::NotSignedIn) => {
                            return Err(e.into())
                        }
                        Err(e) => {
                            self.db.with(|s| s.set_error(d.ino, Some(&e.to_string())))?;
                            break None;
                        }
                    }
                };
                let Some((it, made_here)) = made else {
                    continue;
                };
                let mut out = PullOutcome::default();
                progressed |= self.db.tx(|s| -> Result<bool> {
                    let Some(mut row) = s.get(d.ino)? else {
                        // Deleted while we were creating it. A folder that was
                        // there already isn't ours to delete.
                        if made_here {
                            s.add_tombstone(&Tombstone {
                                id: it.id.clone(),
                                name: d.name.clone(),
                                path: String::new(),
                                is_dir: true,
                            })?;
                        }
                        return Ok(false);
                    };
                    if let Some(other) = s.by_id(&it.id)?.filter(|o| o.ino != row.ino) {
                        if other.moved {
                            // The folder online by that name is another item
                            // here, renamed but not online yet: ours comes after.
                            return Ok(false);
                        }
                        // Another folder here that OneDrive counts as the same
                        // name (case differs beyond ASCII): ours is the "-PCNAME" copy.
                        self.split_conflict(s, &mut row, &mut out)?;
                        return Ok(true);
                    }
                    row.id = Some(it.id.clone());
                    row.etag = it.e_tag.clone();
                    row.ctag = it.c_tag.clone();
                    row.web_url = it.web_url.clone();
                    row.error = None;
                    row.moved = row.name != name || row.parent != d.parent;
                    s.update(&row)?;
                    Ok(true)
                })?;
                self.invalidate(&out);
            }
            if !progressed {
                return Ok(());
            }
        }
    }

    async fn push_moves(&self, parents: &mut Parents) -> Result<()> {
        for m in self.db.with(|s| s.moves())? {
            let (Some(id), Some(parent_id)) = (
                m.id.clone(),
                self.db.with(|s| s.get(m.parent))?.and_then(|p| p.id),
            ) else {
                continue;
            };
            match self
                .graph
                .update(&id, Some(&m.name), Some(&parent_id))
                .await
            {
                Ok(it) => {
                    parents.insert(id.clone(), Some(parent_id.clone()));
                    self.db.with(|s| -> Result<()> {
                        if let Some(mut row) = s.get(m.ino)? {
                            row.etag = it.e_tag.clone();
                            row.web_url = it.web_url.clone();
                            row.error = None;
                            row.moved = row.name != m.name || row.parent != m.parent;
                            s.update(&row)?;
                            let kind = if m.parent == row.parent && m.name != row.name {
                                "renamed"
                            } else {
                                "moved"
                            };
                            s.add_activity(kind, &row, "")?;
                        }
                        Ok(())
                    })?;
                }
                Err(e) if e.is_not_found() => {
                    // The item is gone online, or the folder it's moving into is.
                    match self.graph.item(&id).await {
                        Ok(_) => self.db.with(|s| {
                            s.set_error(
                                m.ino,
                                Some("The folder it was moved to isn't on OneDrive anymore"),
                            )
                        })?,
                        Err(g) if g.is_not_found() => {
                            // Upload what we have as new items.
                            let out = self.db.tx(|s| {
                                let mut out = PullOutcome::default();
                                for mut it in s.subtree(m.ino)? {
                                    it.id = None;
                                    it.etag = None;
                                    it.ctag = None;
                                    it.moved = false;
                                    it.dirty = !it.is_dir;
                                    if !it.is_dir && !it.cached {
                                        // No copy here to upload: say where it went.
                                        s.add_activity("deleted_online", &it, "")?;
                                        out.stale_entries.push((it.parent, it.name.clone()));
                                        s.delete(it.ino)?;
                                    } else {
                                        s.update(&it)?;
                                    }
                                }
                                Ok(out)
                            })?;
                            self.invalidate(&out);
                        }
                        Err(g) if g.is_offline() || matches!(g, GraphError::NotSignedIn) => {
                            return Err(g.into())
                        }
                        Err(g) => self.db.with(|s| s.set_error(m.ino, Some(&g.to_string())))?,
                    }
                }
                Err(e) if e.status() == Some(409) => {
                    // The new name is taken online. By an item deleted here, it's
                    // free once that delete is pushed; otherwise ours is renamed.
                    let holder = match self.graph.child(&parent_id, &m.name).await {
                        Ok(h) => h,
                        Err(g) if g.is_offline() || matches!(g, GraphError::NotSignedIn) => {
                            return Err(g.into())
                        }
                        Err(_) => None,
                    };
                    if let Some(h) = holder.filter(|h| h.id != id) {
                        if self.db.with(|s| s.is_tombstoned(&h.id))? {
                            if self.delete_must_wait(&h, parents).await? {
                                // That delete waits for this very move (the
                                // item is still inside it online, say): move
                                // under a free name for now, and to the name
                                // wanted once the delete is through.
                                let free = self
                                    .db
                                    .with(|s| self.free_conflict_name(s, m.parent, &m.name))?;
                                match self.graph.update(&id, Some(&free), Some(&parent_id)).await {
                                    Ok(it) => {
                                        parents.insert(id.clone(), Some(parent_id.clone()));
                                        self.db.with(|s| -> Result<()> {
                                            if let Some(mut row) = s.get(m.ino)? {
                                                row.etag = it.e_tag.clone();
                                                row.web_url = it.web_url.clone();
                                                s.update(&row)?;
                                            }
                                            Ok(())
                                        })?
                                    }
                                    Err(g)
                                        if g.is_offline()
                                            || matches!(g, GraphError::NotSignedIn) =>
                                    {
                                        return Err(g.into())
                                    }
                                    Err(g) => {
                                        tracing::warn!("couldn't move {} online: {g}", m.name)
                                    }
                                }
                            }
                            continue;
                        }
                    }
                    let out = self.db.tx(|s| {
                        let mut out = PullOutcome::default();
                        if let Some(mut row) = s.get(m.ino)? {
                            self.rename_aside(s, &mut row, &mut out)?;
                        }
                        Ok(out)
                    })?;
                    self.invalidate(&out);
                }
                Err(e) if e.is_offline() || matches!(e, GraphError::NotSignedIn) => {
                    return Err(e.into())
                }
                Err(e) => self.db.with(|s| s.set_error(m.ino, Some(&e.to_string())))?,
            }
        }
        Ok(())
    }

    async fn push_uploads(self: &Arc<Self>) -> Result<()> {
        let waiting = self.db.with(|s| s.uploads())?;
        // What's kept for uploads to anything else (renamed, deleted or
        // split off since) is no use any more. A file whose folder isn't
        // online yet still goes to the same item, if it has one.
        let wanted: Vec<UploadTarget<'_>> = waiting
            .iter()
            .filter_map(|(it, parent_id)| upload_target(it, parent_id.as_deref()))
            .collect();
        self.graph.forget_uploads(&wanted);
        drop(wanted);
        let uploads: Vec<Item> = waiting
            .into_iter()
            // Its folder goes online first.
            .filter(|(_, parent_id)| parent_id.is_some())
            .map(|(it, _)| it)
            .filter(|i| !self.has_writers(i.ino) && self.upload_retry.due(i.ino))
            .collect();
        let n = self.cfg().max_parallel_transfers.max(1);
        let results: Vec<Result<Option<(String, String)>>> = futures_util::stream::iter(uploads)
            .map(|it| {
                let this = self.clone();
                async move { this.upload_one(it).await }
            })
            .buffer_unordered(n)
            .collect()
            .await;
        let mut failed = Vec::new();
        let mut stop = None;
        for r in results {
            match r {
                Ok(Some(f)) => failed.push(f),
                Ok(None) => {}
                Err(e)
                    if graph_err(&e).is_some_and(|g| {
                        g.is_offline() || matches!(g, GraphError::NotSignedIn)
                    }) =>
                {
                    stop.get_or_insert(e);
                }
                Err(e) => tracing::warn!("upload failed: {e:#}"),
            }
        }
        self.notify_upload_failures(&failed);
        stop.map_or(Ok(()), Err)
    }

    /// One notification for the files that newly couldn't upload in a pass.
    fn notify_upload_failures(&self, failed: &[(String, String)]) {
        if !self.cfg().notify_errors {
            return;
        }
        match failed {
            [] => {}
            [(name, msg)] => self.notify(format!("Couldn't upload \"{name}\""), msg.clone()),
            [(_, msg), ..] => self.notify(
                format!("Couldn't upload {} files", failed.len()),
                msg.clone(),
            ),
        }
    }

    /// Take OneDrive's item `remote` as what `row`'s changes build on.
    fn rebase(row: &mut Item, remote: &DriveItem) {
        row.id = Some(remote.id.clone());
        row.etag = remote.e_tag.clone();
        row.ctag = remote.c_tag.clone();
        row.qxh = remote.quick_xor_hash().map(str::to_string);
        row.web_url = remote.web_url.clone();
        row.error = None;
    }

    /// Record that OneDrive's item `remote` holds `row`'s content as of
    /// generation `gen`.
    fn uploaded(row: &mut Item, remote: &DriveItem, gen: i64) {
        Self::rebase(row, remote);
        if row.gen == gen {
            row.dirty = false;
        }
    }

    /// Whether `row` still stands for the OneDrive item an upload of `it`
    /// went to: not once a save-by-rename gave it another file's item, or
    /// it was renamed to a name that doesn't sync.
    fn same_item(row: &Item, it: &Item) -> bool {
        row.id == it.id && !row.local_only
    }

    /// `remote` holds `it`'s content, from an upload that went through
    /// without an answer or was stopped while its date was set: it gets
    /// the date the file has here, as an upload gives it. Only while it's
    /// still that version: one saved over it since (from another PC, say)
    /// keeps its own date, and what comes back is `remote` as it was, so
    /// the next pull brings the other one down instead of it being taken
    /// for ours.
    async fn keep_local_date(&self, remote: DriveItem, it: &Item) -> DriveItem {
        let here = ms_to_utc(it.mtime);
        if remote
            .modified()
            .is_some_and(|t| t.timestamp() == here.timestamp())
        {
            return remote;
        }
        self.graph
            .set_modified(&remote.id, here, remote.e_tag.as_deref())
            .await
            .ok()
            .filter(|dated| dated.same_content(&remote))
            .unwrap_or(remote)
    }

    /// Returns the file's name and the problem when it newly failed to upload.
    async fn upload_one(self: Arc<Self>, it: Item) -> Result<Option<(String, String)>> {
        // As it is now: the list was made when the pass began, and a long
        // one takes a while to get through.
        let Some(it) = self
            .db
            .with(|s| s.get(it.ino))?
            .filter(|r| !r.is_dir && r.cached && !r.local_only && (r.dirty || r.id.is_none()))
        else {
            return Ok(None);
        };
        // Opened for writing since: it goes once it's closed.
        if self.has_writers(it.ino) {
            return Ok(None);
        }
        let Some(parent_id) = self.db.with(|s| s.get(it.parent))?.and_then(|p| p.id) else {
            return Ok(None);
        };
        let path = self.cache_path(it.ino);
        if !path.exists() {
            self.db
                .with(|s| s.set_error(it.ino, Some("The local copy of this file is missing")))?;
            return Ok(None);
        }
        let target = match &it.id {
            Some(id) => UploadTarget::Existing(id),
            None => UploadTarget::New {
                parent_id: &parent_id,
                name: &it.name,
            },
        };
        let transfer = self.begin_transfer(it.ino, &it.name, true, it.size);
        let progress = |done: u64| self.progress(it.ino, true, done);
        let result = self
            .graph
            .upload(
                target,
                &path,
                Some(ms_to_utc(it.mtime)),
                it.etag.as_deref(),
                &progress,
            )
            .await;
        drop(transfer);

        let mut failure = None;
        match result {
            Ok(remote) => {
                self.upload_retry.clear(it.ino);
                let stray = self.db.tx(|s| -> Result<Option<String>> {
                    let Some(mut row) = s.get(it.ino)? else {
                        if let Some(mut heir) = s.by_id(&remote.id)? {
                            // Saved again (by rename) meanwhile: the file that
                            // took over the item builds on what was just
                            // uploaded, so its own upload goes on from there.
                            if heir.etag == it.etag {
                                heir.etag = remote.e_tag.clone();
                                heir.ctag = remote.c_tag.clone();
                                heir.qxh = remote.quick_xor_hash().map(str::to_string);
                                s.update(&heir)?;
                            }
                            return Ok(None);
                        }
                        // Deleted locally while uploading.
                        s.add_tombstone(&Tombstone {
                            id: remote.id.clone(),
                            name: it.name.clone(),
                            path: String::new(),
                            is_dir: false,
                        })?;
                        return Ok(None);
                    };
                    if !Self::same_item(&row, &it) {
                        // Saved over another file (by rename), or renamed to
                        // a name that doesn't sync, while uploading: it goes
                        // up as what it is now. Its old item was left to be
                        // deleted then; a new one made for it is nobody's.
                        if it.id.is_none() {
                            s.add_tombstone(&Tombstone {
                                id: remote.id.clone(),
                                name: it.name.clone(),
                                path: String::new(),
                                is_dir: false,
                            })?;
                            return Ok(Some(remote.id.clone()));
                        }
                        return Ok(None);
                    }
                    Self::uploaded(&mut row, &remote, it.gen);
                    if row.name != it.name || row.parent != it.parent {
                        row.moved = true;
                    }
                    s.update(&row)?;
                    s.add_activity("uploaded", &row, "")?;
                    Ok(None)
                })?;
                // Only now that it's recorded: a pass stopped before then
                // still knows what landed for this PC's own.
                self.graph.forget_sent(target);
                if let Some(id) = stray {
                    // Now, before the pull takes it for a file added online.
                    if self.graph.delete(&id).await.is_ok() {
                        self.db.with(|s| s.remove_tombstone(&id))?;
                    }
                }
            }
            Err(e) if e.status() == Some(412) => {
                // Changed online since we last looked, or an earlier attempt
                // went through without us hearing back.
                let id = it.id.clone().unwrap_or_default();
                let mut remote = self.graph.item(&id).await?;
                let ours = local_hash(&path).await;
                let same = ours.is_some() && remote.quick_xor_hash() == ours.as_deref();
                if same {
                    // Ours, perhaps still without its date.
                    remote = self.keep_local_date(remote, &it).await;
                }
                let sent = remote
                    .quick_xor_hash()
                    .is_some_and(|h| self.graph.sent_before(UploadTarget::Existing(&id), h));
                let out = self.db.tx(|s| {
                    let mut out = PullOutcome::default();
                    let Some(mut row) = s.get(it.ino)?.filter(|r| Self::same_item(r, &it)) else {
                        return Ok(out);
                    };
                    if same {
                        Self::uploaded(&mut row, &remote, it.gen);
                        s.update(&row)?;
                    } else if sent {
                        // That was an earlier version from here, saved over
                        // since: this one goes up on top of it.
                        Self::rebase(&mut row, &remote);
                        s.update(&row)?;
                    } else if Self::content_changed(&row, &remote) {
                        self.split_conflict(s, &mut row, &mut out)?;
                    } else {
                        // Only metadata changed (e.g. renamed online): retry with the new tag.
                        row.etag = remote.e_tag.clone();
                        s.update(&row)?;
                    }
                    Ok(out)
                })?;
                // Kernel cache invalidation must happen with the database unlocked.
                self.after_pull_conflicts(&out);
                self.wake();
            }
            Err(e) if e.status() == Some(409) && it.id.is_none() => {
                // Something online already has this name. Perhaps this very
                // file, from an attempt that went through without us hearing
                // back: only different content there is a conflict.
                let mut online = self.graph.child(&parent_id, &it.name).await?;
                let ours = local_hash(&path).await;
                let same = ours.is_some()
                    && online.as_ref().and_then(|o| o.quick_xor_hash()) == ours.as_deref();
                let sent = online
                    .as_ref()
                    .and_then(|o| o.quick_xor_hash())
                    .is_some_and(|h| self.graph.sent_before(target, h));
                if let Some(o) = online.as_ref().filter(|o| same && !o.is_folder()) {
                    // Ours, perhaps still without its date, unless another
                    // item here stands for it (see below).
                    let unclaimed = self.db.with(|s| -> Result<bool> {
                        Ok(s.by_id(&o.id)?.is_none() && !s.is_tombstoned(&o.id)?)
                    })?;
                    if unclaimed {
                        online = Some(self.keep_local_date(o.clone(), &it).await);
                    }
                }
                let out = self.db.tx(|s| {
                    let mut out = PullOutcome::default();
                    let Some(mut row) = s.get(it.ino)?.filter(|r| Self::same_item(r, &it)) else {
                        return Ok(out);
                    };
                    let Some(o) = &online else {
                        return Ok(out); // gone again: try once more
                    };
                    let holder = s.by_id(&o.id)?;
                    if s.is_tombstoned(&o.id)? || holder.as_ref().is_some_and(|h| h.moved) {
                        // An item deleted or renamed here, not online yet;
                        // those go first next time.
                    } else if holder.is_none() && !o.is_folder() && (same || sent) {
                        if same {
                            Self::uploaded(&mut row, o, it.gen);
                        } else {
                            // An earlier version from here: this one goes on top.
                            Self::rebase(&mut row, o);
                        }
                        if row.name != it.name || row.parent != it.parent {
                            row.moved = true;
                        }
                        s.update(&row)?;
                        s.add_activity("uploaded", &row, "")?;
                    } else {
                        // Other content, or an item here that stays where it
                        // is (its name differs only in a case OneDrive doesn't
                        // tell apart), so the name never comes free.
                        self.split_conflict(s, &mut row, &mut out)?;
                    }
                    Ok(out)
                })?;
                self.after_pull_conflicts(&out);
                if !out.conflicts.is_empty() || sent {
                    self.wake();
                }
            }
            Err(e) if e.is_not_found() && it.id.is_some() => {
                // Deleted online meanwhile: upload as a new file.
                self.db.with(|s| -> Result<()> {
                    if let Some(mut row) = s.get(it.ino)?.filter(|r| Self::same_item(r, &it)) {
                        row.id = None;
                        row.etag = None;
                        row.ctag = None;
                        s.update(&row)?;
                    }
                    Ok(())
                })?;
                self.wake();
            }
            Err(e) if e.is_offline() || matches!(e, GraphError::NotSignedIn) => {
                return Err(e.into())
            }
            Err(e) => {
                // Changed (or deleted) here while it went up: no failure,
                // that version goes next.
                let superseded = matches!(e, GraphError::Changed)
                    || self
                        .db
                        .with(|s| s.get(it.ino))?
                        .is_none_or(|r| r.gen != it.gen);
                if superseded {
                    self.wake();
                } else {
                    let msg = e.to_string();
                    self.upload_retry.failed(it.ino);
                    // Reported when it starts, not again on every retry.
                    let new = it.error.as_deref() != Some(msg.as_str());
                    self.db.with(|s| -> Result<()> {
                        s.set_error(it.ino, Some(&msg))?;
                        if new {
                            if let Some(row) = s.get(it.ino)? {
                                s.add_activity("error", &row, &msg)?;
                            }
                        }
                        Ok(())
                    })?;
                    if new {
                        failure = Some((it.name.clone(), msg));
                    }
                }
            }
        }
        self.changed();
        Ok(failure)
    }

    fn after_pull_conflicts(&self, out: &PullOutcome) {
        self.invalidate(out);
        self.notify_conflicts(&out.conflicts);
    }

    // ----- Files On-Demand ----------------------------------------------------------

    fn hydrate_lock(&self, ino: u64) -> Arc<tokio::sync::Mutex<()>> {
        self.hydrate_locks.lock().entry(ino).or_default().clone()
    }

    /// Make a file's content available locally, downloading it if needed.
    pub async fn hydrate(&self, ino: u64) -> Result<()> {
        self.download(ino, true).await
    }

    /// `opening`: a program is waiting to open the file, rather than "Always
    /// keep on this device" fetching it in the background, so a big one gets
    /// a notification.
    async fn download(&self, ino: u64, opening: bool) -> Result<()> {
        let lock = self.hydrate_lock(ino);
        let _guard = lock.lock().await;
        // Before looking at the row, so an overwrite, or freeing up space,
        // just after still stops it.
        let dl = Downloading {
            engine: self,
            ino,
            part: self.paths.cache_dir.join(format!("{ino}.part")),
            stop: Arc::new(Notify::new()),
        };
        self.downloads.lock().insert(
            ino,
            Running {
                stop: dl.stop.clone(),
                background: !opening,
            },
        );
        let it = self
            .db
            .with(|s| s.get(ino))?
            .ok_or_else(|| anyhow!("no such file"))?;
        // In the background, only while it's still to be kept here: the
        // list of what's to fetch was made a while ago.
        if it.cached || it.is_dir || (!opening && !it.pinned) {
            return Ok(());
        }
        let id = it
            .id
            .clone()
            .ok_or_else(|| anyhow!("file has no content"))?;
        let announce = opening && it.size >= BIG_DOWNLOAD_NOTICE;
        let announced = AtomicBool::new(false);
        let tmp = &dl.part;
        let transfer = self.begin_transfer(ino, &it.name, false, it.size);
        let progress = |done: u64| {
            self.progress(ino, false, done);
            // Once it's actually coming, so an attempt while offline says nothing.
            if announce && done > 0 && !announced.swap(true, Ordering::Relaxed) {
                self.notify(
                    format!("Downloading \"{}\"", it.name),
                    format!("{} from OneDrive", human_size(it.size)),
                );
            }
        };
        let result = tokio::select! {
            r = self.graph.download(&id, tmp, &progress) => Some(r),
            // Overwritten here meanwhile: the new content stands, and the
            // rest of the old isn't worth fetching. Or deleted, or freed up.
            () = dl.stop.notified() => None,
        };
        drop(transfer);
        match result {
            Some(Err(e)) => {
                if e.is_offline() {
                    self.offline.store(true, Ordering::Relaxed);
                    self.changed();
                }
                return Err(e.into());
            }
            Some(Ok(_)) => {
                if let Some(expected) = it.qxh.clone() {
                    let p = tmp.clone();
                    let actual =
                        tokio::task::spawn_blocking(move || QuickXorHash::of_file(&p)).await??;
                    if actual != expected {
                        return Err(anyhow!(
                            "downloaded content of {} didn't match its hash",
                            it.name
                        ));
                    }
                }
                if let Ok(f) = std::fs::File::options().write(true).open(tmp) {
                    let _ = f.set_modified(ms_to_system(it.mtime));
                }
            }
            None => {}
        }

        // Swapped in under the lock, and only if the file wasn't written here
        // meanwhile (emptied and refilled without a download) or changed
        // online, and, in the background, is still to be kept here.
        let row = self.db.with(|s| -> Result<Option<Item>> {
            let Some(mut row) = s.get(ino)? else {
                return Ok(None);
            };
            let wanted = opening || row.pinned;
            if result.is_some() && !row.cached && !row.dirty && row.ctag == it.ctag && wanted {
                std::fs::rename(tmp, self.cache_path(ino))?;
                row.cached = true;
                s.update(&row)?;
                s.add_activity("downloaded", &row, "")?;
            }
            Ok(Some(row))
        })?;
        match row {
            Some(r) if r.cached => {}
            // Freed up, no longer kept here or deleted meanwhile: nothing
            // was asked for, so nothing failed.
            r if !opening && !r.as_ref().is_some_and(|r| r.pinned) => return Ok(()),
            _ => {
                return Err(anyhow!(
                    "{} changed online while downloading; try again",
                    it.name
                ))
            }
        }
        if let Ok(rel) = self.db.with(|s| s.path(ino)) {
            thumbs::clear_failure(
                &self.paths.thumbnail_dir,
                &self.cfg().mount_path().join(rel.trim_start_matches('/')),
            );
        }
        self.changed();
        Ok(())
    }

    /// Whether background downloads ("Always keep on this device") may run.
    fn may_download_pinned(&self) -> bool {
        self.can_reach_onedrive() && self.paused_until().is_none()
    }

    fn spawn_pinned_downloads(self: &Arc<Self>) {
        // Offline or paused: the next pass after that tries again.
        if !self.may_download_pinned() {
            return;
        }
        // A run under way makes its list again when it's through this one.
        self.pinned_again.store(true, Ordering::SeqCst);
        if self.pinned_running.swap(true, Ordering::AcqRel) {
            return;
        }
        // Pausing or signing out stops these too, those running and the rest.
        let stop = self.stop.subscribe();
        let this = self.clone();
        self.rt.spawn(async move {
            while this.pinned_again.swap(false, Ordering::SeqCst)
                && !stop.has_changed().unwrap_or(true)
            {
                let todo: Vec<Item> = this
                    .db
                    .with(|s| s.pinned_uncached())
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|it| this.pinned_retry.due(it.ino))
                    .collect();
                let n = this.cfg().max_parallel_transfers.max(1);
                futures_util::stream::iter(todo)
                    .for_each_concurrent(n, |it| {
                        let this = this.clone();
                        let mut stop = stop.clone();
                        async move {
                            if !this.may_download_pinned() {
                                return;
                            }
                            // Freed up or deleted since the list was made
                            // (after freeing up a whole drive, most of it).
                            let row = this.db.with(|s| s.get(it.ino)).ok().flatten();
                            if !row.is_some_and(|r| r.pinned && !r.cached) {
                                return;
                            }
                            let result = tokio::select! {
                                biased;
                                _ = stop.changed() => return,
                                r = this.download(it.ino, false) => r,
                            };
                            match result {
                                Ok(()) => this.pinned_retry.clear(it.ino),
                                Err(e) => {
                                    // Offline waits for the connection; anything
                                    // else for a while, not every pass.
                                    if !graph_err(&e).is_some_and(|g| g.is_offline()) {
                                        this.pinned_retry.failed(it.ino);
                                    }
                                    tracing::warn!("couldn't download {}: {e:#}", it.name);
                                }
                            }
                        }
                    })
                    .await;
            }
            this.pinned_running.store(false, Ordering::Release);
            // Asked for just as this run ended.
            if this.pinned_again.load(Ordering::SeqCst) && !stop.has_changed().unwrap_or(true) {
                this.spawn_pinned_downloads();
            }
        });
    }

    /// "Always keep on this device" (true) or undo it (false).
    pub fn set_pinned(self: &Arc<Self>, ino: u64, pinned: bool) -> Result<()> {
        self.pin_change(|| {
            self.pin_subtree(ino, pinned)?;
            if !pinned {
                // What hasn't come yet stays online, as in the Windows client.
                self.stop_kept_downloads(ino);
            }
            Ok(())
        })?;
        if pinned {
            // Asked again: the ones that failed before get another try now.
            self.pinned_retry.reset();
            self.spawn_pinned_downloads();
        }
        self.changed();
        Ok(())
    }

    /// Moved into an "Always keep on this device" folder: so is everything
    /// inside, and it downloads now. Files that failed to before still
    /// wait their turn: nobody asked for them again.
    ///
    /// A whole tree takes a while, so it's done in the background. It's
    /// queued now, though, so a change asked for after the move (freeing
    /// up space in it, say) waits for it rather than being undone by it.
    pub fn pin_moved_in(self: &Arc<Self>, ino: u64) {
        self.moved_in.lock().push(ino);
        let this = self.clone();
        self.rt.spawn_blocking(move || {
            // Done already if a change asked for since got there first.
            let _ = this.pin_change(|| Ok(()));
            this.spawn_pinned_downloads();
            this.changed();
        });
    }

    /// Make `change` to what's kept on this device once the changes asked
    /// for before it are made.
    fn pin_change<R>(&self, change: impl FnOnce() -> Result<R>) -> Result<R> {
        let _turn = self.pinning.lock();
        let moved_in = std::mem::take(&mut *self.moved_in.lock());
        for ino in moved_in {
            if let Err(e) = self.pin_subtree(ino, true) {
                let path = self.db.with(|s| s.path(ino)).unwrap_or_default();
                tracing::warn!("couldn't keep {path} on this device: {e:#}");
            }
        }
        change()
    }

    /// Set "always keep" on `ino` and everything below it, a chunk at a time
    /// so the mount keeps answering while a whole drive is done.
    fn pin_subtree(&self, ino: u64, pinned: bool) -> Result<()> {
        let inos = self.db.with(|s| s.subtree_inos(ino))?;
        for chunk in inos.chunks(CHUNK) {
            self.db.tx(|s| {
                for &i in chunk {
                    s.set_pinned(i, pinned)?;
                }
                Ok(())
            })?;
        }
        Ok(())
    }

    /// "Free up space": drop local copies that are safely online.
    /// Returns how many files had to stay (unsynced changes or open).
    pub fn free_up_space(&self, ino: u64) -> Result<u32> {
        let (kept, freed) = self.pin_change(|| {
            self.pin_subtree(ino, false)?;
            // What "Always keep" hasn't fetched yet stays online too.
            self.stop_kept_downloads(ino);
            let files = self.db.with(|s| s.cached_files_under(ino))?;
            let mut kept = 0;
            let mut freed = Vec::new();
            for chunk in files.chunks(CHUNK) {
                self.db.tx(|s| {
                    for &f in chunk {
                        if !self.is_open(f) && s.mark_online_only(f)? {
                            freed.push(f);
                        } else {
                            kept += 1;
                        }
                    }
                    Ok(())
                })?;
            }
            Ok((kept, freed))
        })?;
        self.drop_stale_copies(&freed);
        self.changed();
        Ok(kept)
    }

    // ----- cloud thumbnails ------------------------------------------------------

    /// Thread `tid` read a folder from `offset` on, and is about to be
    /// answered, which took `took`: if a file manager or file chooser is
    /// showing it, prepare thumbnails of its online-only photos in the
    /// background.
    pub fn folder_listed(self: &Arc<Self>, dir: &Item, tid: u32, offset: u64, took: Duration) {
        self.thumbs.folder_listed(self, dir, tid, offset, took);
    }

    /// A thumbnail of a file in the freedesktop cache, fetched from OneDrive
    /// if the file is online-only. `None` if there isn't one.
    pub async fn thumbnail(self: &Arc<Self>, ino: u64, size: Bucket) -> Result<Option<PathBuf>> {
        self.thumbs.get(self, ino, size).await
    }

    /// Tests: act as if `by` had passed for what cloud thumbnails remember
    /// (folders listed, previews OneDrive didn't have).
    #[doc(hidden)]
    pub fn age_thumbnail_state(&self, by: Duration) {
        self.thumbs.pass(by);
    }

    /// Tests: from now on, time passes for what folder listings look like
    /// only as `age_thumbnail_state` says, however long a busy machine takes
    /// between the steps of a test or to answer a read.
    #[doc(hidden)]
    pub fn hold_thumbnail_clock(&self) {
        self.thumbs.hold_clock();
    }

    /// Whether OneDrive's thumbnail can stand in for downloading `it`.
    pub fn may_fetch_thumbnail(&self, it: &Item) -> bool {
        self.can_reach_onedrive() && self.cfg().cloud_thumbnails && crate::cloud_thumbs::wanted(it)
    }

    /// Signed in and not known to be offline.
    pub fn can_reach_onedrive(&self) -> bool {
        self.signed_in.load(Ordering::Relaxed) && !self.offline.load(Ordering::Relaxed)
    }

    /// Change an attribute of `path` through the mount, so programs watching
    /// its folder look at the file again (and find its new thumbnail).
    pub(crate) fn nudge(&self, path: PathBuf) {
        if self.kernel.get().is_none() {
            return;
        }
        // The kernel calls back into the mount: never from a filesystem thread.
        self.rt.spawn_blocking(move || {
            use std::os::unix::ffi::OsStrExt;
            let (Ok(p), Ok(n)) = (
                std::ffi::CString::new(path.as_os_str().as_bytes()),
                std::ffi::CString::new(thumbs::NUDGE_XATTR),
            ) else {
                return;
            };
            unsafe { libc::setxattr(p.as_ptr(), n.as_ptr(), b"1".as_ptr().cast(), 1, 0) };
        });
    }

    // ----- sharing, versions, account -------------------------------------------

    fn remote_id(&self, ino: u64) -> Result<String> {
        self.db
            .with(|s| s.get(ino))?
            .and_then(|i| i.id)
            .ok_or_else(|| anyhow!("This item hasn't finished uploading yet"))
    }

    pub async fn share_link(&self, ino: u64, edit: bool) -> Result<String> {
        let id = self.remote_id(ino)?;
        let business = self.account.lock().drive_type == "business";
        let scope = if business {
            "organization"
        } else {
            "anonymous"
        };
        let link = self
            .graph
            .create_link(&id, if edit { "edit" } else { "view" }, scope)
            .await?;
        // Shared now: its badge says so without waiting for the next pass.
        let newly = self.db.with(|s| -> Result<Option<String>> {
            s.set_shared(ino, true)?.then(|| s.path(ino)).transpose()
        });
        if let Ok(Some(path)) = newly {
            self.rebadge([path]);
            self.changed();
        }
        Ok(link)
    }

    pub fn web_url(&self, ino: u64) -> Result<String> {
        let it = self
            .db
            .with(|s| s.get(ino))?
            .ok_or_else(|| anyhow!("no such item"))?;
        if ino == ROOT {
            let a = self.account.lock().web_url.clone();
            if !a.is_empty() {
                return Ok(a);
            }
        }
        it.web_url
            .ok_or_else(|| anyhow!("This item hasn't finished uploading yet"))
    }

    pub async fn versions(&self, ino: u64) -> Result<Vec<DriveItemVersion>> {
        let id = self.remote_id(ino)?;
        Ok(self.graph.versions(&id).await?)
    }

    pub async fn restore_version(&self, ino: u64, version_id: &str) -> Result<()> {
        let id = self.remote_id(ino)?;
        self.graph.restore_version(&id, version_id).await?;
        self.wake();
        Ok(())
    }

    /// Reload tokens written by `axentra-onedrive login` and sync right away.
    pub async fn reload_tokens(&self) {
        self.graph.tokens().reload().await;
        *self.account_due.lock() = None;
        self.wake();
    }

    /// "Unlink this PC": forget the account and every local copy.
    pub async fn sign_out(&self) -> Result<()> {
        // Stop the pass in progress and wait for it, so nothing it was in the
        // middle of (an upload finishing, a page of the first scan) lands in
        // the fresh state, where it would be taken for the next account's.
        self.interrupt();
        let _pass = self.pass.lock().await;
        // Uploads it kept to carry on with later, with their upload URLs.
        self.graph.forget_uploads(&[]);
        self.graph.tokens().clear().await?;
        self.signed_in.store(false, Ordering::Relaxed);
        if let Ok(names) = self.db.with(|s| s.take_all_thumbs()) {
            thumbs::remove(&self.paths.thumbnail_dir, &names);
        }
        self.db.with(|s| s.reset())?;
        if let Ok(entries) = std::fs::read_dir(&self.paths.cache_dir) {
            for e in entries.flatten() {
                let _ = std::fs::remove_file(e.path());
            }
        }
        self.upload_retry.reset();
        self.pinned_retry.reset();
        *self.account.lock() = Account::default();
        self.last_sync.store(0, Ordering::Relaxed);
        // Its saved copy went with the reset.
        self.paused_until.store(0, Ordering::Relaxed);
        self.changed();
        Ok(())
    }

    /// For the status line in Settings ("Using 1.2 GB on this device").
    pub fn local_usage(&self) -> u64 {
        self.db.with(|s| s.cached_bytes()).unwrap_or(0)
    }
}

/// 256599 -> "256,599".
fn group_digits(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The kind of account, as Microsoft's account picker tells them apart.
fn kind(drive_type: &str) -> &'static str {
    if drive_type.eq_ignore_ascii_case("business") {
        "work or school"
    } else {
        "personal"
    }
}

fn folder_of(path: &str) -> String {
    let parent = path.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
    match parent.rsplit('/').next() {
        Some(name) if !name.is_empty() => name.to_string(),
        _ => "OneDrive".into(),
    }
}

/// When a pause ends, as the flyout says it: "at 4:30 PM", "tomorrow at
/// 4:30 PM", "on Friday at 4:30 PM".
fn resume_when(now: DateTime<Local>, until: i64) -> String {
    let Some(at) = Local.timestamp_opt(until, 0).single() else {
        return "later".into();
    };
    let time = at.format("%-I:%M %p");
    match (at.date_naive() - now.date_naive()).num_days() {
        ..=0 => format!("at {time}"),
        1 => format!("tomorrow at {time}"),
        2..=6 => format!("on {} at {time}", at.format("%A")),
        _ => format!("on {} at {time}", at.format("%B %-d")),
    }
}

/// The name an item had before `make_room` set it aside.
fn pending_original(name: &str) -> Option<&str> {
    let (_, original) = name.strip_prefix(PENDING_PREFIX)?.split_once('-')?;
    Some(original)
}

/// Where `it` uploads to, its folder being the OneDrive item `parent_id`.
fn upload_target<'a>(it: &'a Item, parent_id: Option<&'a str>) -> Option<UploadTarget<'a>> {
    Some(match &it.id {
        Some(id) => UploadTarget::Existing(id),
        None => UploadTarget::New {
            parent_id: parent_id?,
            name: &it.name,
        },
    })
}

/// QuickXorHash of a local file, `None` if it can't be read.
async fn local_hash(path: &Path) -> Option<String> {
    let p = path.to_path_buf();
    tokio::task::spawn_blocking(move || QuickXorHash::of_file(&p))
        .await
        .ok()?
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digits_are_grouped() {
        assert_eq!(super::group_digits(7), "7");
        assert_eq!(super::group_digits(1000), "1,000");
        assert_eq!(super::group_digits(256599), "256,599");
        assert_eq!(super::group_digits(1234567), "1,234,567");
    }

    #[test]
    fn resume_time_says_the_day() {
        let now = Local
            .with_ymd_and_hms(2026, 6, 10, 16, 30, 0)
            .single()
            .unwrap();
        let after = |h: i64| (now + chrono::Duration::hours(h)).timestamp();
        assert_eq!(resume_when(now, after(2)), "at 6:30 PM");
        assert_eq!(resume_when(now, after(24)), "tomorrow at 4:30 PM");
        assert_eq!(resume_when(now, after(72)), "on Saturday at 4:30 PM");
        assert_eq!(resume_when(now, after(24 * 30)), "on July 10 at 4:30 PM");
    }

    #[test]
    fn set_aside_names_give_back_the_original() {
        assert_eq!(
            pending_original(".axentra-onedrive-pending-42-a-b.txt"),
            Some("a-b.txt")
        );
        assert_eq!(pending_original("a.txt"), None);
    }

    #[test]
    fn failures_wait_longer_each_time() {
        let b = Backoff::default();
        assert!(b.due(7));
        b.failed(7);
        assert!(!b.due(7));
        assert!(b.due(8));
        let first = b.0.lock()[&7].0;
        b.failed(7);
        assert!(b.0.lock()[&7].0 > first + Duration::from_secs(60));
        b.clear(7);
        assert!(b.due(7));
    }

    #[tokio::test]
    async fn stray_cache_files_are_swept_at_start() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        paths.ensure().unwrap();
        let (kept, edited, stale, gone) = Db::open(&paths.db_file).unwrap().with(|s| {
            let file = |name: &str, cached: bool, dirty: bool| {
                s.insert(&Item {
                    parent: ROOT,
                    name: name.into(),
                    cached,
                    dirty,
                    ..Default::default()
                })
                .unwrap()
            };
            let gone = file("gone", true, false);
            s.delete(gone).unwrap();
            (
                file("kept", true, false),
                file("edited", false, true),
                file("stale", false, false),
                gone,
            )
        });
        let cache = |name: String| paths.cache_dir.join(name);
        for ino in [kept, edited, stale, gone] {
            std::fs::write(cache(ino.to_string()), b"x").unwrap();
        }
        std::fs::write(cache(format!("{stale}.part")), b"x").unwrap();
        std::fs::write(cache("notes".into()), b"x").unwrap();

        let engine = Engine::new(
            paths.clone(),
            Config::default(),
            Options {
                notifications: false,
            },
        )
        .unwrap();
        // Not before this process is known to be the only one.
        assert!(cache(format!("{stale}.part")).exists());
        engine.tidy_up();
        let mut left: Vec<String> = std::fs::read_dir(&paths.cache_dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        let mut want = vec![kept.to_string(), edited.to_string(), "notes".into()];
        want.sort();
        assert_eq!(left, want);
    }

    /// Online C moved out of A into X, then A into C; the feed has A first.
    #[tokio::test]
    async fn a_folder_untangled_ahead_of_its_own_move_lands_where_it_is_online() {
        let dir = tempfile::tempdir().unwrap();
        let engine = Engine::new(
            Paths::under(dir.path()),
            Config::default(),
            Options {
                notifications: false,
            },
        )
        .unwrap();
        let (a, c, x) = engine.db.with(|s| {
            let mut root = s.get(ROOT).unwrap().unwrap();
            root.id = Some("root".into());
            s.update(&root).unwrap();
            let folder = |id: &str, parent: u64, name: &str| {
                s.insert(&Item {
                    id: Some(id.into()),
                    parent,
                    name: name.into(),
                    is_dir: true,
                    ..Default::default()
                })
                .unwrap()
            };
            let a = folder("a", ROOT, "A");
            let x = folder("x", ROOT, "X");
            (a, folder("c", a, "C"), x)
        });
        let online = |id: &str, name: &str, parent: &str| -> DriveItem {
            serde_json::from_value(serde_json::json!({
                "id": id, "name": name, "folder": {}, "parentReference": { "id": parent },
            }))
            .unwrap()
        };
        let mut out = PullOutcome::default();
        engine
            .db
            .tx(|s| {
                engine.apply_items(
                    s,
                    vec![online("a", "A", "c"), online("c", "C", "x")],
                    &PageContext::default(),
                    &mut HashSet::new(),
                    &mut out,
                )
            })
            .unwrap();

        let (a, c) = engine
            .db
            .with(|s| (s.get(a).unwrap().unwrap(), s.get(c).unwrap().unwrap()));
        assert_eq!((c.parent, c.moved), (x, false));
        assert_eq!((a.parent, a.moved), (c.ino, false));
    }

    /// A tree moved into an "Always keep on this device" folder is kept in
    /// the background, but before a change asked for after the move, which
    /// it would otherwise undo a chunk at a time.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn keeping_a_moved_tree_does_not_undo_what_was_asked_after() {
        let dir = tempfile::tempdir().unwrap();
        let engine = Engine::new(
            Paths::under(dir.path()),
            Config::default(),
            Options {
                notifications: false,
            },
        )
        .unwrap();
        let (big, sub) = engine.db.with(|s| {
            let item = |parent: u64, name: String, is_dir: bool| {
                s.insert(&Item {
                    parent,
                    name,
                    is_dir,
                    ..Default::default()
                })
                .unwrap()
            };
            let big = item(ROOT, "Big".into(), true);
            let sub = item(big, "Sub".into(), true);
            for i in 0..3 {
                item(sub, format!("{i}.txt"), false);
            }
            // More than is kept in one go, ahead of what's in Sub.
            for i in 0..CHUNK + 100 {
                item(big, i.to_string(), true);
            }
            (big, sub)
        });
        engine.pin_moved_in(big);
        engine.free_up_space(sub).unwrap();
        // Nor once the background pass has run.
        tokio::time::sleep(Duration::from_millis(300)).await;
        engine.db.with(|s| {
            let (tree, sub) = (s.subtree(big).unwrap(), s.subtree(sub).unwrap());
            assert_eq!(
                tree.iter().filter(|i| i.pinned).count(),
                tree.len() - sub.len()
            );
            assert!(sub.iter().all(|i| !i.pinned));
        });
    }

    /// A OneNote notebook doesn't sync, so neither does what's in it, and
    /// that isn't kept (and saved, page after page) waiting for it.
    #[tokio::test]
    async fn whats_inside_a_notebook_is_not_kept_waiting_for_it() {
        let dir = tempfile::tempdir().unwrap();
        let engine = Engine::new(
            Paths::under(dir.path()),
            Config::default(),
            Options {
                notifications: false,
            },
        )
        .unwrap();
        let moved = engine.db.with(|s| {
            let mut root = s.get(ROOT).unwrap().unwrap();
            root.id = Some("root".into());
            s.update(&root).unwrap();
            s.insert(&Item {
                id: Some("moved".into()),
                parent: ROOT,
                name: "moved.txt".into(),
                ..Default::default()
            })
            .unwrap()
        });
        let online = |v: serde_json::Value| -> DriveItem { serde_json::from_value(v).unwrap() };
        let file = |id: &str, parent: &str| {
            online(serde_json::json!({
                "id": id, "name": format!("{id}.one"), "file": {},
                "parentReference": { "id": parent },
            }))
        };
        let apply = |items: Vec<DriveItem>, skipped: &mut HashSet<String>| {
            let mut out = PullOutcome::default();
            engine
                .db
                .tx(|s| engine.apply_items(s, items, &PageContext::default(), skipped, &mut out))
                .unwrap()
        };
        let mut skipped = HashSet::new();
        // A section listed before its notebook, then the notebook, a
        // section group in it and what's in that, including a file that
        // was moved there online; and one item whose folder is still to come.
        let orphans = apply(
            vec![
                file("section", "notebook"),
                online(serde_json::json!({
                    "id": "notebook", "name": "Notes", "package": { "type": "oneNote" },
                    "parentReference": { "id": "root" },
                })),
                online(serde_json::json!({
                    "id": "group", "name": "Work", "folder": {},
                    "parentReference": { "id": "notebook" },
                })),
                file("plans", "group"),
                file("moved", "group"),
                file("waiting", "later"),
            ],
            &mut skipped,
        );
        let ids: Vec<&str> = orphans.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(ids, vec!["waiting"]);
        // And in a later page.
        assert!(apply(vec![file("ideas", "group")], &mut skipped).is_empty());
        assert_eq!(skipped.len(), 6);
        assert!(engine.db.with(|s| s.get(moved)).unwrap().is_none());
        assert_eq!(engine.db.with(|s| s.item_count()).unwrap(), 0);
    }
}
