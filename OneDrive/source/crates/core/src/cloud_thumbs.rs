//! Cloud thumbnails: OneDrive's previews of online-only photos and videos,
//! fetched into the freedesktop thumbnail cache (see [`crate::thumbs`]) so
//! the file is never downloaded just to be shown as a picture.
//!
//! Work starts lazily: when a file manager, image viewer or file chooser lists
//! a folder to show it, its photos are queued (the folder listed last goes
//! first) and fetched a few at a time in the background; when a thumbnailer or
//! the file chooser portal opens an online-only photo, that one is fetched
//! right away (and its folder queued, if its listing looked like counting).
//! Photos a file manager gave up on aren't asked for again, so those in the
//! OneDrive folder itself, whose listings always look like counting, are
//! queued whenever it's listed. Files that are on this device, or always
//! kept on it, are left to the system's thumbnailers.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use parking_lot::Mutex;
use tokio::sync::Semaphore;

use crate::db::{Item, Thumb, ROOT};
use crate::engine::Engine;
use crate::thumbs::{self, Bucket, Original};

/// Listing the same folder again within this long doesn't look again.
const RELIST: Duration = Duration::from_secs(60);
/// A program that read a folder this recently is still busy listing.
const BUSY: Duration = Duration::from_secs(1);
/// What a program listed is forgotten after this long.
const FORGET: Duration = Duration::from_secs(60);
/// Photos queued from one folder listing (the first ones by name that need one).
const PER_FOLDER: usize = 1000;
const MAX_QUEUE: usize = 4000;
/// OneDrive makes previews a while after upload: after it had none, ask again
/// as long after as the file was old then, within these.
const RETRY: Duration = Duration::from_secs(120);
const MAX_RETRY: Duration = Duration::from_secs(24 * 60 * 60);
/// What folder listings prepare; the smaller sizes come from the same download.
const LISTING_SIZE: Bucket = Bucket::Large;
/// Background fetches at a time, and the pause between starting them.
const BACKGROUND: usize = 3;
const PACING: Duration = Duration::from_millis(50);
/// Requests someone is waiting for (a thumbnailer, the Explorer over D-Bus).
const DIRECT: usize = 6;

/// Whether a file gets a cloud thumbnail: an online-only photo or video.
pub fn wanted(it: &Item) -> bool {
    !it.is_dir
        && it.id.is_some()
        && !it.cached
        && !it.pinned
        && !it.local_only
        && !it.dirty
        && it.size > 0
        && thumbs::is_media(&it.name)
}

#[derive(Default)]
struct Queue {
    order: VecDeque<u64>,
    /// The folder each waiting file was queued from.
    queued: HashMap<u64, u64>,
    running: bool,
}

/// What one program listed lately. File managers list a folder to show it
/// and then each folder in it to count its items (Nautilus's "N items",
/// Properties' total), on the same worker threads.
struct Lister {
    /// When its last read came, or was answered if that's later: the time
    /// spent answering isn't a pause of its own.
    last: Instant,
    /// Folders it listed so far.
    listings: u64,
    /// Which of those was the last listing of each folder, and when it was.
    seen: HashMap<u64, (u64, Instant)>,
}

impl Lister {
    fn busy(&self, now: Instant) -> bool {
        now.duration_since(self.last) < BUSY
    }
}

#[derive(Default)]
struct Listers(HashMap<u32, Lister>);

impl Listers {
    /// Whether process `pid` listing `dir` (in `parent`) is to show it. While
    /// it's busy listing, a first look at a folder in one it listed, or another
    /// look at one whose parent it listed again since (a refresh, a recount),
    /// is counting. Any other look is someone opening the folder, except one
    /// at the OneDrive folder itself: it's counted from the home folder, which
    /// isn't on this mount, so there's no telling. Either way, a folder that's
    /// on screen after all is queued once one of its photos is asked for
    /// (see `CloudThumbs::get`).
    ///
    /// Which came first goes by the order of the listings, not their times,
    /// which a coarse clock can make the same.
    fn shows(&mut self, pid: u32, dir: u64, parent: u64, now: Instant) -> bool {
        self.forget(now);
        let l = self.0.entry(pid).or_insert_with(|| Lister {
            last: now,
            listings: 0,
            seen: HashMap::new(),
        });
        let busy = l.busy(now);
        let counting = dir == ROOT
            || (busy
                && match (l.seen.get(&dir), l.seen.get(&parent)) {
                    (None, Some(_)) => true,
                    (Some((d, _)), Some((p, _))) => p > d,
                    _ => false,
                });
        if !busy {
            l.seen.retain(|_, (_, t)| now.duration_since(*t) < FORGET);
        }
        l.last = l.last.max(now);
        l.listings += 1;
        l.seen.insert(dir, (l.listings, now));
        !counting
    }

    /// Whether any program is busy listing folders.
    fn busy(&mut self, now: Instant) -> bool {
        self.forget(now);
        self.0.values().any(|l| l.busy(now))
    }

    /// Whether a program listed `dir` lately, to show it or not.
    fn listed(&mut self, dir: u64, now: Instant) -> bool {
        self.forget(now);
        self.0.values().any(|l| {
            l.seen
                .get(&dir)
                .is_some_and(|(_, t)| now.duration_since(*t) < FORGET)
        })
    }

    /// `pid` read further into a folder: a big one takes a while. Whether
    /// that kept it busy.
    fn reading(&mut self, pid: u32, now: Instant) -> bool {
        match self.0.get_mut(&pid).filter(|l| l.busy(now)) {
            Some(l) => {
                l.last = l.last.max(now);
                true
            }
            None => false,
        }
    }

    /// `pid`'s read, taken in by `shows` or `reading`, was answered at `now`:
    /// its pause before the next one counts from then, however long
    /// answering took.
    fn answered(&mut self, pid: u32, now: Instant) {
        if let Some(l) = self.0.get_mut(&pid) {
            l.last = l.last.max(now);
        }
    }

    fn forget(&mut self, now: Instant) {
        self.0.retain(|_, l| now.duration_since(l.last) < FORGET);
    }
}

/// Whether it's too soon, at `now` (Unix seconds), to ask OneDrive again for
/// a preview of the version of a file `t` records. Previews of new files
/// come within minutes; one it hasn't made of a file that was old already
/// (a video it can't preview) isn't coming soon. But a file's date is when
/// it was taken or edited, not when OneDrive got it (a video the phone kept
/// for Wi-Fi, old photos copied in from another PC), so the wait is as long
/// as the file was old when it had none, and no more than twice as long as
/// it has had none: `RETRY` after the first time, up to `MAX_RETRY`. A file
/// dated after it had none (by a camera whose clock was ahead) is of no
/// known age: the wait is twice as long as it has had none. A miss dated
/// after `now` was kept while the clock was ahead, and is over.
fn held(t: &Thumb, now: i64) -> bool {
    let grown = 2 * (t.missed - t.first_missed);
    let wait = match t.missed - t.mtime {
        age if age < 0 => grown,
        age => age.min(grown),
    };
    let wait = wait.clamp(RETRY.as_secs() as i64, MAX_RETRY.as_secs() as i64);
    t.missed != 0 && (0..wait).contains(&(now - t.missed))
}

/// Which of a folder's photos that need a thumbnail to queue.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Which {
    All,
    /// Those a file manager gave up on (see `thumbs::failed`).
    GivenUp,
}

/// Folders whose photos were queued lately, and when.
#[derive(Default)]
struct Listed {
    at: HashMap<(u64, Which), Instant>,
    /// `thumbs::removals()` then: thumbnails removed since, of a folder
    /// renamed or moved for one, are needed again.
    removals: u64,
}

pub struct CloudThumbs {
    queue: Mutex<Queue>,
    listed: Mutex<Listed>,
    listers: Mutex<Listers>,
    locks: Mutex<HashMap<u64, Arc<tokio::sync::Mutex<()>>>>,
    background: Arc<Semaphore>,
    direct: Arc<Semaphore>,
    clock: Mutex<Clock>,
}

/// The time cloud thumbnails go by: the system's, moved on by `pass`.
#[derive(Default)]
struct Clock {
    /// Where tests hold the time listings go by, so that a busy machine's
    /// pauses between their steps, or its time spent answering a read, don't
    /// change what a listing looks like.
    held: Option<Instant>,
    /// How far `pass` moved it on.
    passed: Duration,
}

/// A file as the mount shows it, and its cache entry name.
struct Target {
    item: Item,
    /// Its path in the mount, starting with '/'.
    rel: String,
    abs: PathBuf,
    uri: String,
    name: String,
    mtime: i64,
    size: u64,
}

impl Target {
    fn original(&self) -> Original<'_> {
        Original {
            uri: &self.uri,
            mtime: self.mtime,
            size: self.size,
        }
    }
}

fn target(engine: &Engine, ino: u64) -> Result<Option<Target>> {
    let found = engine.db.with(|s| -> Result<Option<(Item, String)>> {
        let Some(it) = s.get(ino)? else {
            return Ok(None);
        };
        Ok(Some((it, s.path(ino)?)))
    })?;
    let Some((item, rel)) = found else {
        return Ok(None);
    };
    if item.is_dir {
        return Ok(None);
    }
    let abs = engine.cfg().mount_path().join(rel.trim_start_matches('/'));
    let uri = thumbs::file_uri(&abs);
    // What `stat` shows through the mount (see `fs::Inner::attr`).
    let (mut size, mut mtime) = (item.size, item.mtime.max(0) / 1000);
    if item.cached {
        if let Ok(m) = std::fs::metadata(engine.cache_path(ino)) {
            size = m.len();
            if let Ok(d) = m
                .modified()
                .map(|t| t.duration_since(std::time::UNIX_EPOCH))
            {
                mtime = d.map(|d| d.as_secs() as i64).unwrap_or(0);
            }
        }
    }
    Ok(Some(Target {
        name: thumbs::cache_name(&uri),
        item,
        rel,
        abs,
        uri,
        mtime,
        size,
    }))
}

impl Default for CloudThumbs {
    fn default() -> Self {
        Self {
            queue: Mutex::new(Queue::default()),
            listed: Mutex::new(Listed::default()),
            listers: Mutex::new(Listers::default()),
            locks: Mutex::new(HashMap::new()),
            background: Arc::new(Semaphore::new(BACKGROUND)),
            direct: Arc::new(Semaphore::new(DIRECT)),
            clock: Mutex::new(Clock::default()),
        }
    }
}

impl CloudThumbs {
    /// Thread `tid` read folder `dir` from `offset` on, and is about to be
    /// answered, which took `took`. When that's a file manager, image viewer
    /// or file chooser showing the folder, queue its photos that need a
    /// thumbnail.
    ///
    /// Called before the answer, while the thread waits for it: once
    /// answered, it may finish and be gone before it's told from a script,
    /// and the next folder it lists, on another of the mount's threads,
    /// could be taken in before this one.
    pub fn folder_listed(
        &self,
        engine: &Arc<Engine>,
        dir: &Item,
        tid: u32,
        offset: u64,
        took: Duration,
    ) {
        // When the read came: a program waiting on a slow answer isn't idle.
        let (came, now) = self.read_times(took);
        if offset > 0 {
            // Every program's every chunk comes here: look no further unless
            // a file manager is busy listing.
            if self.listers.lock().busy(came) {
                if let Some(pid) = thumbs::process_of(tid) {
                    let mut listers = self.listers.lock();
                    if listers.reading(pid, came) {
                        listers.answered(pid, now);
                    }
                }
            }
            return;
        }
        if !engine.cloud_thumbnails() || !engine.can_reach_onedrive() {
            return;
        }
        if !thumbs::lists_for_display(tid) {
            return;
        }
        let Some(pid) = thumbs::process_of(tid) else {
            return;
        };
        let shows = {
            let mut listers = self.listers.lock();
            let shows = listers.shows(pid, dir.ino, dir.parent, came);
            listers.answered(pid, now);
            shows
        };
        if shows {
            self.queue_folder(engine, dir.ino, Which::All, now);
        } else if dir.ino == ROOT {
            // On screen or not, its photos a file manager gave up on (it was
            // offline, or OneDrive had no preview yet) won't be asked for.
            self.queue_folder(engine, ROOT, Which::GivenUp, now);
        }
    }

    /// Queue the photos in folder `dir` that need a thumbnail (`which` of
    /// them), unless that was done lately.
    fn queue_folder(&self, engine: &Arc<Engine>, dir: u64, which: Which, now: Instant) {
        {
            let mut listed = self.listed.lock();
            let removals = thumbs::removals();
            if listed.removals != removals {
                listed.at.clear();
                listed.removals = removals;
            }
            let lately = |w| {
                listed
                    .at
                    .get(&(dir, w))
                    .is_some_and(|t| now.duration_since(*t) < RELIST)
            };
            if lately(Which::All) || lately(which) {
                return;
            }
            listed.at.retain(|_, t| now.duration_since(*t) < RELIST);
            listed.at.insert((dir, which), now);
        }
        let engine = engine.clone();
        engine.rt.clone().spawn(async move {
            let e = engine.clone();
            let todo = tokio::task::spawn_blocking(move || scan(&e, dir, which))
                .await
                .unwrap_or_default();
            if todo.is_empty() {
                return;
            }
            tracing::debug!("queueing {} cloud thumbnails for folder {dir}", todo.len());
            {
                let mut q = engine.thumbs.queue.lock();
                for ino in todo.into_iter().rev() {
                    if q.queued.insert(ino, dir).is_none() {
                        q.order.push_front(ino);
                    } else if let Some(i) = q.order.iter().position(|x| *x == ino) {
                        // Already waiting: move it up with the rest of this folder.
                        q.order.remove(i);
                        q.order.push_front(ino);
                    }
                }
                let mut dropped = HashSet::new();
                while q.order.len() > MAX_QUEUE {
                    if let Some(old) = q.order.pop_back() {
                        dropped.extend(q.queued.remove(&old));
                    }
                }
                // Listing those folders again queues them again.
                let mut listed = engine.thumbs.listed.lock();
                listed.at.retain(|(d, _), _| !dropped.contains(d));
            }
            engine.thumbs.start(&engine);
        });
    }

    fn start(&self, engine: &Arc<Engine>) {
        {
            let mut q = self.queue.lock();
            if q.running || q.order.is_empty() {
                return;
            }
            q.running = true;
        }
        let engine = engine.clone();
        engine
            .rt
            .clone()
            .spawn(async move { engine.thumbs.work(&engine).await });
    }

    async fn work(&self, engine: &Arc<Engine>) {
        loop {
            let permit = self
                .background
                .clone()
                .acquire_owned()
                .await
                .expect("the semaphore is never closed");
            let next = {
                let mut q = self.queue.lock();
                let next = q.order.pop_front();
                match next {
                    Some(ino) => {
                        q.queued.remove(&ino);
                    }
                    None => q.running = false,
                }
                next
            };
            let Some(ino) = next else { return };
            let e = engine.clone();
            engine.rt.spawn(async move {
                let _permit = permit;
                match e.thumbs.fetch(&e, ino, LISTING_SIZE).await {
                    Ok(_) => {}
                    Err(err) => {
                        let offline = err
                            .downcast_ref::<axentra_onedrive_graph::GraphError>()
                            .is_some_and(|g| {
                                g.is_offline()
                                    || matches!(g, axentra_onedrive_graph::GraphError::NotSignedIn)
                            });
                        if offline {
                            e.thumbs.clear_queue();
                        } else {
                            tracing::debug!("couldn't get the cloud thumbnail of {ino}: {err:#}");
                        }
                    }
                }
            });
            tokio::time::sleep(PACING).await;
        }
    }

    fn clear_queue(&self) {
        let mut q = self.queue.lock();
        q.order.clear();
        q.queued.clear();
        self.listed.lock().at.clear();
    }

    /// The thumbnail of `ino` at `bucket`, fetched from OneDrive if the file
    /// is online-only. `None` when there isn't one.
    pub async fn get(
        &self,
        engine: &Arc<Engine>,
        ino: u64,
        bucket: Bucket,
    ) -> Result<Option<PathBuf>> {
        self.asked_for(engine, ino);
        let permit = self.direct.clone().acquire_owned().await?;
        // In a task of its own, so a caller that stops waiting doesn't leave
        // the thumbnail half written and unrecorded.
        let e = engine.clone();
        engine
            .rt
            .spawn(async move {
                let _permit = permit;
                e.thumbs.fetch(&e, ino, bucket).await
            })
            .await?
    }

    async fn fetch(
        &self,
        engine: &Arc<Engine>,
        ino: u64,
        bucket: Bucket,
    ) -> Result<Option<PathBuf>> {
        let lock = self.locks.lock().entry(ino).or_default().clone();
        let result = {
            let _guard = lock.lock().await;
            self.fetch_locked(engine, ino, bucket).await
        };
        let mut locks = self.locks.lock();
        if locks.get(&ino).is_some_and(|l| Arc::strong_count(l) <= 2) {
            locks.remove(&ino);
        }
        result
    }

    async fn fetch_locked(
        &self,
        engine: &Arc<Engine>,
        ino: u64,
        bucket: Bucket,
    ) -> Result<Option<PathBuf>> {
        let root = engine.paths.thumbnail_dir.clone();
        let Some(t) = target(engine, ino)? else {
            return Ok(None);
        };
        let want = thumbs::path_in(&root, bucket, &t.name);
        if thumbs::is_valid(&want, &t.original()) {
            return Ok(Some(want));
        }
        if !wanted(&t.item) || !engine.cfg().cloud_thumbnails {
            return Ok(None);
        }
        let row = engine.db.with(|s| s.thumb(ino))?;
        let same = row
            .as_ref()
            .filter(|r| r.md5 == t.name && r.mtime == t.mtime);
        if same.is_some_and(|r| held(r, self.now())) {
            return Ok(None); // OneDrive had none a moment ago
        }
        let id = t.item.id.clone().unwrap_or_default();
        let px = bucket.px();
        let data = match engine.graph.thumbnail(&id, &format!("c{px}x{px}")).await {
            // No custom sizes here: take the 800 px one and scale it.
            Err(e) if e.status() == Some(400) => engine.graph.thumbnail(&id, "large").await?,
            r => r?,
        };
        let now = self.now();
        let missed = if data.is_some() { 0 } else { now };
        let mut rec = Thumb {
            ino,
            md5: t.name.clone(),
            mtime: t.mtime,
            sizes: same.map_or(0, |r| r.sizes),
            missed,
            // Still none since then, unless that was while the clock was ahead.
            first_missed: same
                .map(|r| r.first_missed)
                .filter(|f| missed != 0 && (1..=now).contains(f))
                .unwrap_or(missed),
        };
        // Recorded before anything is written, while the file is still where
        // it was: whatever takes its thumbnails from now on (a rename,
        // unlinking this PC) takes these too. Unlinking marks this PC signed
        // out before it takes them all and forgets the files.
        let recorded = engine.db.with(|s| -> Result<bool> {
            let here = engine.account().signed_in
                && s.get(ino)?.is_some_and(|it| it.id == t.item.id)
                && s.path(ino)? == t.rel;
            if here {
                s.set_thumb(&rec)?;
            }
            Ok(here)
        })?;
        if !recorded {
            return Ok(None);
        }
        if let Some(old) = row.filter(|r| r.md5 != t.name) {
            thumbs::remove(&root, &[old.md5]); // written under a name it no longer has
        }
        let Some(data) = data else {
            return Ok(None);
        };
        let (uri, mtime, size, dir) = (t.uri.clone(), t.mtime, t.size, root.clone());
        let written = tokio::task::spawn_blocking(move || -> Result<u32> {
            let img = image::load_from_memory(&data).context("reading OneDrive's thumbnail")?;
            let of = Original {
                uri: &uri,
                mtime,
                size,
            };
            let mut bits = 0;
            for b in Bucket::ALL.into_iter().filter(|b| *b <= bucket) {
                thumbs::write(&dir, b, &of, &img)?;
                bits |= b.bit();
            }
            Ok(bits)
        })
        .await?;
        let kept = engine.db.with(|s| -> Result<bool> {
            let ours = engine.account().signed_in
                && s.thumb(ino)?
                    .is_some_and(|r| r.md5 == rec.md5 && r.mtime == rec.mtime);
            if let (true, Ok(bits)) = (ours, &written) {
                rec.sizes |= bits;
                s.set_thumb(&rec)?;
            }
            Ok(ours)
        })?;
        if !kept {
            // Taken meanwhile (or about to be), to be removed.
            thumbs::remove(&root, std::slice::from_ref(&t.name));
            return Ok(None);
        }
        written?;
        thumbs::clear_failure(&root, &t.abs);
        engine.nudge(t.abs.clone());
        Ok(Some(want))
    }

    /// Someone wants the thumbnail of `ino`. When a program listed its folder
    /// lately but that looked like counting (a folder opened while others
    /// were being counted, or the OneDrive folder itself), the folder is on
    /// screen after all: queue the rest of its photos too.
    fn asked_for(&self, engine: &Arc<Engine>, ino: u64) {
        if !engine.cloud_thumbnails() || !engine.can_reach_onedrive() {
            return;
        }
        let Ok(Some(it)) = engine.db.with(|s| s.get(ino)) else {
            return;
        };
        let now = self.instant();
        if wanted(&it) && self.listers.lock().listed(it.parent, now) {
            self.queue_folder(engine, it.parent, Which::All, now);
        }
    }

    /// The time listings go by, moved on by `pass`.
    fn instant(&self) -> Instant {
        let c = self.clock.lock();
        c.held.unwrap_or_else(Instant::now) + c.passed
    }

    /// When a read that is answered now came, and now, by the time listings
    /// go by. Answering it took `took`, unless the clock is held: then time
    /// passes only as `pass` says, however long that took.
    fn read_times(&self, took: Duration) -> (Instant, Instant) {
        let c = self.clock.lock();
        match c.held {
            Some(t) => (t + c.passed, t + c.passed),
            None => {
                let now = Instant::now() + c.passed;
                (now.checked_sub(took).unwrap_or(now), now)
            }
        }
    }

    /// Unix seconds, moved on by `pass`.
    fn now(&self) -> i64 {
        chrono::Utc::now().timestamp() + self.clock.lock().passed.as_secs() as i64
    }

    /// Act as if `by` had passed since anything was listed or missed.
    pub(crate) fn pass(&self, by: Duration) {
        self.clock.lock().passed += by;
    }

    /// From now on, time passes for listings only as `pass` says.
    pub(crate) fn hold_clock(&self) {
        self.clock.lock().held.get_or_insert_with(Instant::now);
    }
}

/// The photos in a folder that need a thumbnail (`which` of them), in name
/// order.
fn scan(engine: &Engine, dir: u64, which: Which) -> Vec<u64> {
    let root = &engine.paths.thumbnail_dir;
    let fail_dirs = match which {
        Which::All => vec![],
        Which::GivenUp => thumbs::fail_dirs(root),
    };
    let Ok((rel, mut kids)) = engine
        .db
        .with(|s| -> Result<_> { Ok((s.path(dir)?, s.children(dir)?)) })
    else {
        return vec![];
    };
    kids.retain(wanted);
    kids.sort_by_cached_key(|k| k.name.to_lowercase());
    let base = engine.cfg().mount_path().join(rel.trim_start_matches('/'));
    let now = engine.thumbs.now();
    kids.into_iter()
        .filter(|k| {
            let uri = thumbs::file_uri(&base.join(&k.name));
            let name = thumbs::cache_name(&uri);
            if which == Which::GivenUp && !thumbs::failed(&fail_dirs, &name) {
                return false;
            }
            let mtime = k.mtime.max(0) / 1000;
            let path = thumbs::path_in(root, LISTING_SIZE, &name);
            if let Ok(Some(t)) = engine.db.with(|s| s.thumb(k.ino)) {
                if t.md5 == name && t.mtime == mtime {
                    if held(&t, now) {
                        return false; // OneDrive had none a moment ago
                    }
                    if t.sizes & LISTING_SIZE.bit() != 0 && path.exists() {
                        return false;
                    }
                }
            }
            // Made earlier by a thumbnailer, while the file was on this device.
            !thumbs::is_valid(
                &path,
                &Original {
                    uri: &uri,
                    mtime,
                    size: k.size,
                },
            )
        })
        .take(PER_FOLDER)
        .map(|k| k.ino)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counting_folders_is_not_showing_them() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut l = Listers::default();
        // Nautilus opens folder 2 (in 1), then counts what's in each folder
        // there; Properties counts a whole tree the same way.
        assert!(l.shows(10, 2, 1, at(0)));
        assert!(!l.shows(10, 20, 2, at(30)));
        assert!(!l.shows(10, 21, 2, at(60)));
        assert!(!l.shows(10, 200, 20, at(90)));
        // Still reading a big folder.
        assert!(l.reading(10, at(900)));
        assert!(!l.shows(10, 22, 2, at(1800)));
        // Another program.
        assert!(l.shows(11, 20, 2, at(1850)));
        // After a pause, someone opens a folder: counted before or not.
        assert!(l.shows(10, 21, 2, at(3000)));
        assert!(!l.shows(10, 210, 21, at(3050)));
        assert!(l.shows(10, 23, 2, at(5000)));
        // Somewhere else, even while busy.
        assert!(l.shows(10, 7, 6, at(5100)));
        // Programs that stopped listing are forgotten.
        assert!(l.shows(10, 2, 1, at(70_000)));
        assert_eq!(l.0.len(), 1);
    }

    #[test]
    fn opening_a_folder_while_others_are_counted_shows_it() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut l = Listers::default();
        // Pictures (2) is shown and its big folders counted, a chunk at a time.
        assert!(l.shows(10, 2, 1, at(0)));
        assert!(!l.shows(10, 20, 2, at(30)));
        assert!(!l.shows(10, 21, 2, at(60)));
        for ms in (100..3000).step_by(100) {
            l.reading(10, at(ms));
        }
        // 2020 (20) is opened meanwhile, and the folder in it counted.
        assert!(l.shows(10, 20, 2, at(3050)));
        assert!(!l.shows(10, 200, 20, at(3080)));

        // F5 in Pictures: it and its folders are listed again, and counted again.
        assert!(l.shows(10, 2, 1, at(10_000)));
        assert!(!l.shows(10, 20, 2, at(10_030)));
        assert!(!l.shows(10, 21, 2, at(10_060)));
        // Then 2021 is opened right away.
        assert!(l.shows(10, 21, 2, at(10_200)));

        // The location bar lists the folder typed so far, then Enter opens it.
        assert!(!l.shows(10, 22, 2, at(10_400)));
        assert!(l.shows(10, 22, 2, at(10_700)));
    }

    #[test]
    fn a_deep_count_in_one_window_doesnt_hide_folders_opened_in_another() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut l = Listers::default();
        // Properties of Pictures (2): every folder under it, for seconds.
        assert!(l.shows(10, 2, 1, at(0)));
        for (i, (dir, parent)) in [(20, 2), (200, 20), (21, 2), (210, 21), (22, 2)]
            .into_iter()
            .enumerate()
        {
            assert!(!l.shows(10, dir, parent, at(400 * i as u64 + 400)));
        }
        // Another window of the same Nautilus opens 2021 (21) and 2022 (22).
        assert!(l.shows(10, 21, 2, at(2100)));
        assert!(l.shows(10, 22, 2, at(2500)));
    }

    #[test]
    fn reading_on_only_matters_to_a_busy_program() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut l = Listers::default();
        assert!(!l.busy(at(0)));
        l.shows(10, 2, 1, at(0));
        assert!(l.busy(at(500)));
        l.reading(10, at(900));
        assert!(l.busy(at(1500)));
        // Idle a while: reading on doesn't make it busy again.
        assert!(!l.reading(10, at(5000)));
        assert!(!l.busy(at(5000)));
        // Long idle: forgotten.
        assert!(!l.busy(at(70_000)));
        assert!(l.0.is_empty());
    }

    #[test]
    fn a_slow_answer_doesnt_make_a_counting_program_look_idle() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut l = Listers::default();
        // Pictures (2) is shown, though answering took a while (a sync pass
        // had the database), and the folders in it are counted as soon as
        // Nautilus has it.
        assert!(l.shows(10, 2, 1, at(0)));
        l.answered(10, at(1500));
        assert!(!l.shows(10, 20, 2, at(1550)));
        assert!(l.reading(10, at(1600)));
        l.answered(10, at(3000));
        assert!(!l.shows(10, 21, 2, at(3050)));
        // After a pause, someone opens one.
        assert!(l.shows(10, 22, 2, at(5000)));
    }

    #[test]
    fn listings_go_by_their_order_on_a_clock_too_coarse_to_tell() {
        let t0 = Instant::now();
        let mut l = Listers::default();
        assert!(l.shows(10, 2, 1, t0));
        assert!(!l.shows(10, 20, 2, t0));
        // Refreshed and its folders counted again, then 2020 (20) opened.
        assert!(l.shows(10, 2, 1, t0));
        assert!(!l.shows(10, 20, 2, t0));
        assert!(l.shows(10, 20, 2, t0));
    }

    #[test]
    fn a_held_clock_moves_on_only_as_told() {
        let thumbs = CloudThumbs::default();
        thumbs.hold_clock();
        let t = thumbs.instant();
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(thumbs.instant(), t);
        thumbs.pass(Duration::from_millis(1200));
        assert_eq!(thumbs.instant(), t + Duration::from_millis(1200));
        // However long answering a read took.
        let t = thumbs.instant();
        assert_eq!(thumbs.read_times(Duration::from_secs(5)), (t, t));
    }

    #[test]
    fn a_read_came_as_long_ago_as_answering_it_took() {
        let thumbs = CloudThumbs::default();
        let before = Instant::now();
        let (came, now) = thumbs.read_times(Duration::from_millis(1500));
        assert!(now >= before);
        assert_eq!(now - came, Duration::from_millis(1500));
    }

    #[test]
    fn the_onedrive_folder_is_not_taken_for_shown_by_its_listing_alone() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut l = Listers::default();
        // Counted from the home folder, or opened: either way.
        assert!(!l.shows(10, ROOT, 0, at(0)));
        assert!(!l.shows(10, ROOT, 0, at(5000)));
        // Its folders are counted, then one is opened.
        assert!(!l.shows(10, 2, ROOT, at(5030)));
        assert!(l.shows(10, 2, ROOT, at(9000)));
        assert!(l.listed(ROOT, at(9000)));
        assert!(!l.listed(3, at(9000)));
        assert!(!l.listed(ROOT, at(70_000)));
    }

    #[test]
    fn asking_again_for_a_preview_waits_as_long_as_the_file_was_old() {
        let miss = |mtime: i64, first_missed: i64, missed: i64| Thumb {
            ino: 9,
            md5: "a".into(),
            mtime,
            sizes: 0,
            missed,
            first_missed,
        };
        let (retry, day) = (RETRY.as_secs() as i64, MAX_RETRY.as_secs() as i64);
        // Just uploaded: OneDrive makes one in a minute or two.
        let new = miss(1000, 1030, 1030);
        assert!(held(&new, 1030 + 60));
        assert!(!held(&new, 1030 + retry));
        // Still none 5 minutes after it was made: 5 minutes more, and so on.
        let later = miss(1000, 1030, 1300);
        assert!(held(&later, 1300 + 299));
        assert!(!held(&later, 1300 + 300));
        // Taken long ago, uploaded just now (the phone waited for Wi-Fi):
        // asked about again as soon as a new one.
        let up = 1000 + 400 * day;
        let uploaded = miss(1000, up, up);
        assert!(held(&uploaded, up + retry - 1));
        assert!(!held(&uploaded, up + retry));
        // Still none: twice as long as since the first time, and so on.
        let again = miss(1000, up, up + retry);
        assert!(held(&again, up + 3 * retry - 1));
        assert!(!held(&again, up + 3 * retry));
        // None for a long while, such as a video OneDrive can't preview: the
        // next day.
        let old = miss(1000, up, up + day);
        assert!(held(&old, old.missed + day - 1));
        assert!(!held(&old, old.missed + day));
        // Dated in the future by a wrong clock: as a new one at first, then
        // twice as long as since the first time, and so on.
        let ahead = 1000 + 400 * day;
        assert!(held(&miss(ahead, 1000, 1000), 1000 + retry - 1));
        assert!(!held(&miss(ahead, 1000, 1000), 1000 + retry));
        let again = miss(ahead, 1000, 1000 + retry);
        assert!(held(&again, again.missed + 2 * retry - 1));
        assert!(!held(&again, again.missed + 2 * retry));
        let long = miss(ahead, 1000, 1000 + 30 * day);
        assert!(held(&long, long.missed + day - 1));
        assert!(!held(&long, long.missed + day));
        // Missed while this PC's clock was ahead: over once it's right.
        assert!(!held(&miss(1000, 5000, 5000), 1000));
        assert!(!held(&miss(1000, 1000, 5000), 1000));
        // It had one.
        assert!(!held(&miss(1000, 0, 0), 1000));
    }
}
