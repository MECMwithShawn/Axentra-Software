//! The sync engine: pulls OneDrive's change feed into the local state,
//! pushes local changes back, and downloads file contents on demand.
//!
//! Conflict rules follow the Windows client: when both sides changed a file,
//! both copies are kept and the local one is renamed with this computer's
//! name; files deleted online but edited here are kept and re-uploaded.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
use crate::db::{Activity, Db, Item, Store, Tombstone, ROOT};
use crate::names::{self, IgnoreRules};
use crate::notify::Notifier;
use crate::status::{human_size, Account, Issue, Status, Transfer};
use crate::thumbs::{self, Bucket};

const ACCOUNT_REFRESH: Duration = Duration::from_secs(600);
const BIG_DOWNLOAD_NOTICE: u64 = 50 * 1024 * 1024;

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
    opens: Mutex<HashMap<u64, OpenCount>>,
    transfers: Mutex<HashMap<u64, Transfer>>,
    signed_in: AtomicBool,
    cycle_running: AtomicBool,
    offline: AtomicBool,
    last_error: Mutex<Option<String>>,
    last_sync: AtomicI64,
    paused_until: AtomicI64,
    pinned_running: AtomicBool,
    account: Mutex<Account>,
    account_due: Mutex<Option<tokio::time::Instant>>,
    notifier: Arc<Notifier>,
    kernel: OnceLock<fuser::Notifier>,
    pub(crate) thumbs: CloudThumbs,
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
        // Thumbnails are named after the files' paths, so a new OneDrive
        // folder location orphans them all.
        let mount = cfg.mount_path().display().to_string();
        if db.with(|s| s.kv("thumbs_mount")).ok().flatten().as_deref() != Some(mount.as_str()) {
            if let Ok(names) = db.with(|s| s.take_all_thumbs()) {
                thumbs::remove(&paths.thumbnail_dir, &names);
            }
            let _ = db.with(|s| s.set_kv("thumbs_mount", &mount));
        }
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
            opens: Mutex::new(HashMap::new()),
            transfers: Mutex::new(HashMap::new()),
            cycle_running: AtomicBool::new(false),
            offline: AtomicBool::new(false),
            last_error: Mutex::new(None),
            last_sync: AtomicI64::new(last_sync),
            paused_until: AtomicI64::new(paused_until),
            pinned_running: AtomicBool::new(false),
            account: Mutex::new(account),
            account_due: Mutex::new(None),
            notifier: Arc::new(Notifier::new(opts.notifications)),
            kernel: OnceLock::new(),
            thumbs: CloudThumbs::default(),
        }))
    }

    // ----- configuration & signals ----------------------------------------

    pub fn cfg(&self) -> Config {
        self.cfg.read().clone()
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
        self.changed();
        self.wake();
        Ok(())
    }

    // ----- transfers ------------------------------------------------------------

    fn begin_transfer(&self, ino: u64, name: &str, upload: bool, total: u64) {
        self.transfers.lock().insert(
            ino,
            Transfer {
                name: name.to_string(),
                upload,
                done: 0,
                total,
            },
        );
        self.changed();
    }

    fn progress(&self, ino: u64, done: u64) {
        if let Some(t) = self.transfers.lock().get_mut(&ino) {
            t.done = done;
        }
    }

    fn end_transfer(&self, ino: u64) {
        self.transfers.lock().remove(&ino);
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
        self.changed();
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
            let at = Local
                .timestamp_opt(t, 0)
                .single()
                .map(|d| d.format("%-I:%M %p").to_string())
                .unwrap_or_default();
            (
                "paused",
                "Syncing is paused".into(),
                format!("Syncing will resume at {at}"),
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
                format!("{pending} changes waiting to sync"),
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

    async fn refresh_account(&self) -> Result<()> {
        let me = self.graph.me().await?;
        let drive = self.graph.drive().await?;
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
            email: me.mail.or(me.user_principal_name).unwrap_or_default(),
            drive_type: drive.drive_type.unwrap_or_default(),
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
        let result = async {
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
        }
        .await;
        match result {
            Ok(()) => {
                self.offline.store(false, Ordering::Relaxed);
                *self.last_error.lock() = None;
                let now = Utc::now().timestamp();
                self.last_sync.store(now, Ordering::Relaxed);
                let _ = self.db.with(|s| s.set_kv("last_sync", &now.to_string()));
            }
            Err(e) => match graph_err(&e) {
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
        self.spawn_pinned_downloads();
        if let Ok(names) = self.db.with(|s| s.take_orphan_thumbs()) {
            thumbs::remove(&self.paths.thumbnail_dir, &names);
        }
        self.changed();
    }

    // ----- remote → local ---------------------------------------------------------

    async fn pull_remote(&self) -> Result<()> {
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
            let mut out = PullOutcome {
                initial: self.db.with(|s| s.kv("synced_once"))?.is_none(),
                ..Default::default()
            };
            let mut link = saved.or(resume.clone());
            let mut orphans: Vec<DriveItem> = Vec::new();
            loop {
                let page = match self.graph.delta_page(link.as_deref()).await {
                    Ok(p) => p,
                    Err(e) if e.status() == Some(410) && restarts < 3 => {
                        restarts += 1;
                        tracing::info!("change feed expired; doing a full resync");
                        self.db.with(|s| -> Result<()> {
                            s.del_kv("delta_link")?;
                            s.del_kv("scan_next")
                        })?;
                        continue 'restart;
                    }
                    Err(e) if resume.is_some() && e.status() == Some(400) && restarts < 3 => {
                        restarts += 1;
                        tracing::info!(
                            "saved scan position is no longer valid; starting the scan again"
                        );
                        self.db.with(|s| s.del_kv("scan_next"))?;
                        continue 'restart;
                    }
                    Err(e) => return Err(e.into()),
                };
                let mut items = std::mem::take(&mut orphans);
                items.extend(page.items);
                orphans = self.db.tx(|s| self.apply_items(s, items, &mut out))?;
                if let Some(next) = page.next_link {
                    if full {
                        self.db.with(|s| s.set_kv("scan_next", &next))?;
                    }
                    link = Some(next);
                    continue;
                }
                if !orphans.is_empty() {
                    let left = self
                        .db
                        .tx(|s| self.apply_items(s, std::mem::take(&mut orphans), &mut out))?;
                    for o in left {
                        tracing::debug!("skipping {} ({:?}): parent not synced", o.id, o.name);
                    }
                }
                if full {
                    self.db.tx(|s| {
                        for it in s.unseen()? {
                            if let Some(row) = s.get(it.ino)? {
                                self.remote_deleted(s, row, &mut out)?;
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
                    s.set_kv("synced_once", "1")
                })?;
                break;
            }
            self.after_pull(out);
            return Ok(());
        }
    }

    fn after_pull(&self, out: PullOutcome) {
        for ino in &out.uncache {
            self.uncache(*ino);
        }
        self.invalidate(&out);
        let cfg = self.cfg();
        let n = out.deleted_online.len();
        if cfg.notify_deletes && n > 0 && n as u32 >= cfg.delete_warning_threshold.max(1) {
            self.notify(
                format!("{n} files were deleted from your OneDrive"),
                "They were removed from this device too. You can restore them from the OneDrive recycle bin."
                    .into(),
            );
        }
        if cfg.notify_conflicts {
            match out.conflicts.as_slice() {
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
        if !out.uncache.is_empty() || !out.conflicts.is_empty() || n > 0 {
            self.changed();
        }
    }

    /// Apply a batch of delta items; returns the ones whose parent is unknown.
    fn apply_items(
        &self,
        s: &Store<'_>,
        items: Vec<DriveItem>,
        out: &mut PullOutcome,
    ) -> Result<Vec<DriveItem>> {
        let mut orphans = Vec::new();
        for it in items {
            if !self.apply_one(s, &it, out)? {
                orphans.push(it);
            }
        }
        Ok(orphans)
    }

    fn apply_one(&self, s: &Store<'_>, it: &DriveItem, out: &mut PullOutcome) -> Result<bool> {
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
            if let Some(row) = existing {
                self.remote_deleted(s, row, out)?;
            }
            return Ok(true);
        }
        let Some(parent_id) = it.parent_id() else {
            return Ok(true);
        };
        let Some(parent) = s.by_id(parent_id)? else {
            return Ok(false);
        };
        let name = it.name.clone().unwrap_or_default();
        if name.is_empty() {
            return Ok(true);
        }
        match existing {
            Some(row) => self.remote_updated(s, row, &parent, &name, it, out)?,
            None => self.remote_added(s, &parent, &name, it, out)?,
        }
        Ok(true)
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

    fn remote_updated(
        &self,
        s: &Store<'_>,
        mut row: Item,
        parent: &Item,
        name: &str,
        it: &DriveItem,
        out: &mut PullOutcome,
    ) -> Result<()> {
        let local_edit = row.dirty || self.has_writers(row.ino);
        let changed = Self::content_changed(&row, it);
        if changed && local_edit {
            // Both sides changed: keep both, ours under a new name.
            self.split_conflict(s, &mut row, out)?;
            return self.remote_added(s, parent, name, it, out);
        }
        if (row.parent != parent.ino || row.name != name) && !row.moved {
            self.make_room(s, parent.ino, name, row.ino, out)?;
            out.stale_entries.push((row.parent, row.name.clone()));
            out.stale_thumbs.extend(s.take_thumbs_under(row.ino)?);
            row.parent = parent.ino;
            row.name = name.to_string();
            if parent.pinned {
                row.pinned = true;
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
        s.update(&row)
    }

    fn remote_added(
        &self,
        s: &Store<'_>,
        parent: &Item,
        name: &str,
        it: &DriveItem,
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
        } else {
            // A synced item whose own rename or delete is later in the feed.
            out.stale_entries.push((other.parent, other.name.clone()));
            out.stale_thumbs.extend(s.take_thumbs_under(other.ino)?);
            other.name = format!(".axentra-onedrive-pending-{}-{}", other.ino, other.name);
            s.update(&other)
        }
    }

    /// Rename a local item to the conflict-copy name and detach it from its
    /// OneDrive item so it uploads as a new file.
    fn split_conflict(&self, s: &Store<'_>, row: &mut Item, out: &mut PullOutcome) -> Result<()> {
        let mut n = 1;
        let new_name = loop {
            let cand = names::conflict_name(&row.name, &self.host, n);
            if s.child(row.parent, &cand)?.is_none() {
                break cand;
            }
            n += 1;
        };
        let original = std::mem::replace(&mut row.name, new_name);
        out.stale_entries.push((row.parent, original.clone()));
        out.stale_thumbs.extend(s.take_thumbs_under(row.ino)?);
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

    fn remote_deleted(&self, s: &Store<'_>, row: Item, out: &mut PullOutcome) -> Result<()> {
        let tree = s.subtree(row.ino)?;
        let mut kept_any = false;
        // Children first, so a folder knows whether anything inside survived.
        for it in tree.iter().rev() {
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
            s.add_activity(
                "conflict",
                &row,
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
        self.push_deletes().await?;
        self.push_folders().await?;
        self.push_moves().await?;
        self.push_uploads().await
    }

    async fn push_deletes(&self) -> Result<()> {
        for t in self.db.with(|s| s.tombstones())? {
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

    async fn push_folders(&self) -> Result<()> {
        loop {
            let dirs = self.db.with(|s| s.folders_to_create())?;
            if dirs.is_empty() {
                return Ok(());
            }
            let mut progressed = false;
            for d in dirs {
                let Some(parent_id) = self.db.with(|s| s.get(d.parent))?.and_then(|p| p.id) else {
                    continue;
                };
                let created = match self.graph.create_folder(&parent_id, &d.name).await {
                    Ok(it) => Some(it),
                    Err(e) if e.status() == Some(409) => {
                        match self.graph.child(&parent_id, &d.name).await? {
                            Some(it) if it.is_folder() => Some(it),
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
                                None
                            }
                        }
                    }
                    Err(e) if e.is_offline() || matches!(e, GraphError::NotSignedIn) => {
                        return Err(e.into())
                    }
                    Err(e) => {
                        self.db.with(|s| s.set_error(d.ino, Some(&e.to_string())))?;
                        None
                    }
                };
                if let Some(it) = created {
                    progressed = true;
                    self.db.with(|s| -> Result<()> {
                        let Some(mut row) = s.get(d.ino)? else {
                            // Deleted while we were creating it.
                            return s.add_tombstone(&Tombstone {
                                id: it.id.clone(),
                                name: d.name.clone(),
                                path: String::new(),
                                is_dir: true,
                            });
                        };
                        if s.by_id(&it.id)?.is_some_and(|other| other.ino != row.ino) {
                            return Ok(());
                        }
                        row.id = Some(it.id.clone());
                        row.etag = it.e_tag.clone();
                        row.ctag = it.c_tag.clone();
                        row.web_url = it.web_url.clone();
                        row.error = None;
                        row.moved = row.name != d.name || row.parent != d.parent;
                        s.update(&row)
                    })?;
                }
            }
            if !progressed {
                return Ok(());
            }
        }
    }

    async fn push_moves(&self) -> Result<()> {
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
                    // Gone online: upload what we have as new items.
                    self.db.tx(|s| {
                        for mut it in s.subtree(m.ino)? {
                            it.id = None;
                            it.etag = None;
                            it.ctag = None;
                            it.moved = false;
                            it.dirty = !it.is_dir;
                            if !it.is_dir && !it.cached {
                                s.delete(it.ino)?;
                            } else {
                                s.update(&it)?;
                            }
                        }
                        Ok(())
                    })?;
                }
                Err(e) if e.status() == Some(409) => {
                    // The new name is taken online: pick a "-PCNAME" name instead.
                    let out = self.db.tx(|s| {
                        let mut out = PullOutcome::default();
                        if let Some(mut row) = s.get(m.ino)? {
                            let keep = row.clone();
                            self.split_conflict(s, &mut row, &mut out)?;
                            // Same item online; only its name changes.
                            row.id = keep.id;
                            row.etag = keep.etag;
                            row.ctag = keep.ctag;
                            row.qxh = keep.qxh;
                            row.web_url = keep.web_url;
                            row.shared = keep.shared;
                            row.dirty = keep.dirty;
                            row.moved = true;
                            s.update(&row)?;
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
        let uploads: Vec<Item> = self
            .db
            .with(|s| s.uploads())?
            .into_iter()
            .filter(|i| !self.has_writers(i.ino))
            .collect();
        let n = self.cfg().max_parallel_transfers.max(1);
        let errors: Vec<anyhow::Error> = futures_util::stream::iter(uploads)
            .map(|it| {
                let this = self.clone();
                async move { this.upload_one(it).await.err() }
            })
            .buffer_unordered(n)
            .filter_map(|e| async move { e })
            .collect()
            .await;
        for e in errors {
            if graph_err(&e).is_some_and(|g| g.is_offline() || matches!(g, GraphError::NotSignedIn))
            {
                return Err(e);
            }
            tracing::warn!("upload failed: {e:#}");
        }
        Ok(())
    }

    async fn upload_one(self: Arc<Self>, it: Item) -> Result<()> {
        let Some(parent_id) = self.db.with(|s| s.get(it.parent))?.and_then(|p| p.id) else {
            return Ok(());
        };
        let path = self.cache_path(it.ino);
        if !path.exists() {
            self.db
                .with(|s| s.set_error(it.ino, Some("The local copy of this file is missing")))?;
            return Ok(());
        }
        let target = match &it.id {
            Some(id) => UploadTarget::Existing(id),
            None => UploadTarget::New {
                parent_id: &parent_id,
                name: &it.name,
            },
        };
        self.begin_transfer(it.ino, &it.name, true, it.size);
        let progress = |done: u64| self.progress(it.ino, done);
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
        self.end_transfer(it.ino);

        match result {
            Ok(remote) => self.db.tx(|s| {
                let Some(mut row) = s.get(it.ino)? else {
                    // Deleted locally while uploading.
                    return s.add_tombstone(&Tombstone {
                        id: remote.id.clone(),
                        name: it.name.clone(),
                        path: String::new(),
                        is_dir: false,
                    });
                };
                row.id = Some(remote.id.clone());
                row.etag = remote.e_tag.clone();
                row.ctag = remote.c_tag.clone();
                row.qxh = remote.quick_xor_hash().map(str::to_string);
                row.web_url = remote.web_url.clone();
                row.error = None;
                if row.gen == it.gen {
                    row.dirty = false;
                }
                if row.name != it.name || row.parent != it.parent {
                    row.moved = true;
                }
                s.update(&row)?;
                s.add_activity("uploaded", &row, "")
            })?,
            Err(e) if e.status() == Some(412) => {
                // Changed online since we last looked.
                let id = it.id.clone().unwrap_or_default();
                let remote = self.graph.item(&id).await?;
                let out = self.db.tx(|s| {
                    let mut out = PullOutcome::default();
                    let Some(mut row) = s.get(it.ino)? else {
                        return Ok(out);
                    };
                    if Self::content_changed(&row, &remote) {
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
                // Something online already has this name.
                let out = self.db.tx(|s| {
                    let mut out = PullOutcome::default();
                    if let Some(mut row) = s.get(it.ino)? {
                        self.split_conflict(s, &mut row, &mut out)?;
                    }
                    Ok(out)
                })?;
                self.after_pull_conflicts(&out);
                self.wake();
            }
            Err(e) if e.is_not_found() && it.id.is_some() => {
                // Deleted online meanwhile: upload as a new file.
                self.db.with(|s| -> Result<()> {
                    if let Some(mut row) = s.get(it.ino)? {
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
                let msg = e.to_string();
                self.db.with(|s| -> Result<()> {
                    s.set_error(it.ino, Some(&msg))?;
                    if let Some(row) = s.get(it.ino)? {
                        s.add_activity("error", &row, &msg)?;
                    }
                    Ok(())
                })?;
                if self.cfg().notify_errors {
                    self.notify(format!("Couldn't upload \"{}\"", it.name), msg);
                }
            }
        }
        self.changed();
        Ok(())
    }

    fn after_pull_conflicts(&self, out: &PullOutcome) {
        self.invalidate(out);
        if self.cfg().notify_conflicts {
            for name in &out.conflicts {
                self.notify(
                    "Kept both copies of a file".into(),
                    format!("It changed here and online, so your version is saved as \"{name}\"."),
                );
            }
        }
    }

    // ----- Files On-Demand ----------------------------------------------------------

    fn hydrate_lock(&self, ino: u64) -> Arc<tokio::sync::Mutex<()>> {
        self.hydrate_locks.lock().entry(ino).or_default().clone()
    }

    /// Make a file's content available locally, downloading it if needed.
    pub async fn hydrate(&self, ino: u64) -> Result<()> {
        let lock = self.hydrate_lock(ino);
        let _guard = lock.lock().await;
        let it = self
            .db
            .with(|s| s.get(ino))?
            .ok_or_else(|| anyhow!("no such file"))?;
        if it.cached || it.is_dir {
            return Ok(());
        }
        let id = it
            .id
            .clone()
            .ok_or_else(|| anyhow!("file has no content"))?;
        if it.size >= BIG_DOWNLOAD_NOTICE {
            self.notify(
                format!("Downloading \"{}\"", it.name),
                format!("{} from OneDrive", human_size(it.size)),
            );
        }
        let tmp = self.paths.cache_dir.join(format!("{ino}.part"));
        self.begin_transfer(ino, &it.name, false, it.size);
        let progress = |done: u64| self.progress(ino, done);
        let result = self.graph.download(&id, &tmp, &progress).await;
        self.end_transfer(ino);
        if let Err(e) = result {
            let _ = std::fs::remove_file(&tmp);
            if e.is_offline() {
                self.offline.store(true, Ordering::Relaxed);
                self.changed();
            }
            return Err(e.into());
        }

        if let Some(expected) = it.qxh.clone() {
            let p = tmp.clone();
            let actual = tokio::task::spawn_blocking(move || QuickXorHash::of_file(&p)).await??;
            if actual != expected {
                let _ = std::fs::remove_file(&tmp);
                return Err(anyhow!(
                    "downloaded content of {} didn't match its hash",
                    it.name
                ));
            }
        }
        if let Ok(f) = std::fs::File::options().write(true).open(&tmp) {
            let _ = f.set_modified(ms_to_system(it.mtime));
        }
        std::fs::rename(&tmp, self.cache_path(ino))?;

        let stored = self.db.with(|s| -> Result<bool> {
            let Some(mut row) = s.get(ino)? else {
                return Ok(false);
            };
            if row.ctag != it.ctag {
                return Ok(false); // changed online while downloading
            }
            row.cached = true;
            s.update(&row)?;
            s.add_activity("downloaded", &row, "")?;
            Ok(true)
        })?;
        if !stored {
            self.uncache(ino);
            return Err(anyhow!(
                "{} changed online while downloading; try again",
                it.name
            ));
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

    fn spawn_pinned_downloads(self: &Arc<Self>) {
        if self.pinned_running.swap(true, Ordering::AcqRel) {
            return;
        }
        let this = self.clone();
        self.rt.spawn(async move {
            let todo = this.db.with(|s| s.pinned_uncached()).unwrap_or_default();
            let n = this.cfg().max_parallel_transfers.max(1);
            futures_util::stream::iter(todo)
                .for_each_concurrent(n, |it| {
                    let this = this.clone();
                    async move {
                        if let Err(e) = this.hydrate(it.ino).await {
                            tracing::warn!("couldn't download {}: {e:#}", it.name);
                        }
                    }
                })
                .await;
            this.pinned_running.store(false, Ordering::Release);
        });
    }

    /// "Always keep on this device" (true) or undo it (false).
    pub fn set_pinned(self: &Arc<Self>, ino: u64, pinned: bool) -> Result<()> {
        self.db.tx(|s| {
            for mut it in s.subtree(ino)? {
                it.pinned = pinned;
                s.update(&it)?;
            }
            Ok(())
        })?;
        if pinned {
            self.spawn_pinned_downloads();
        }
        self.changed();
        Ok(())
    }

    /// "Free up space": drop local copies that are safely online.
    /// Returns how many files had to stay (unsynced changes or open).
    pub fn free_up_space(&self, ino: u64) -> Result<u32> {
        let mut kept = 0;
        let mut freed = Vec::new();
        self.db.tx(|s| {
            for mut it in s.subtree(ino)? {
                it.pinned = false;
                if !it.is_dir && it.cached {
                    if it.dirty || it.id.is_none() || self.is_open(it.ino) {
                        kept += 1;
                    } else {
                        it.cached = false;
                        freed.push(it.ino);
                    }
                }
                s.update(&it)?;
            }
            Ok(())
        })?;
        for ino in freed {
            self.uncache(ino);
        }
        self.changed();
        Ok(kept)
    }

    // ----- cloud thumbnails ------------------------------------------------------

    /// A file manager or file chooser listed a folder: prepare thumbnails of
    /// its online-only photos in the background.
    pub fn folder_listed(self: &Arc<Self>, dir: u64) {
        self.thumbs.folder_listed(self, dir);
    }

    /// A thumbnail of a file in the freedesktop cache, fetched from OneDrive
    /// if the file is online-only. `None` if there isn't one.
    pub async fn thumbnail(self: &Arc<Self>, ino: u64, size: Bucket) -> Result<Option<PathBuf>> {
        self.thumbs.get(self, ino, size).await
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
        Ok(self
            .graph
            .create_link(&id, if edit { "edit" } else { "view" }, scope)
            .await?)
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
        *self.account.lock() = Account::default();
        self.last_sync.store(0, Ordering::Relaxed);
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

fn folder_of(path: &str) -> String {
    let parent = path.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
    match parent.rsplit('/').next() {
        Some(name) if !name.is_empty() => name.to_string(),
        _ => "OneDrive".into(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn digits_are_grouped() {
        assert_eq!(super::group_digits(7), "7");
        assert_eq!(super::group_digits(1000), "1,000");
        assert_eq!(super::group_digits(256599), "256,599");
        assert_eq!(super::group_digits(1234567), "1,234,567");
    }
}
