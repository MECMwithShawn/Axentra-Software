//! Cloud thumbnails: OneDrive's previews of online-only photos and videos,
//! fetched into the freedesktop thumbnail cache (see [`crate::thumbs`]) so
//! the file is never downloaded just to be shown as a picture.
//!
//! Work starts lazily: when a file manager, image viewer or file chooser lists
//! a folder, its photos are queued (the folder listed last goes first) and
//! fetched a few at a time in the background; when a thumbnailer or the file
//! chooser portal opens an online-only photo, that one is fetched right away.
//! Files that are on this device, or always kept on it, are left to the
//! system's thumbnailers.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use parking_lot::Mutex;
use tokio::sync::Semaphore;

use crate::db::{Item, Thumb};
use crate::engine::Engine;
use crate::thumbs::{self, Bucket, Original};

/// Listing the same folder again within this long doesn't look again.
const RELIST: Duration = Duration::from_secs(60);
/// Photos queued from one folder listing (the first ones by name).
const PER_FOLDER: usize = 1000;
const MAX_QUEUE: usize = 4000;
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
    queued: HashSet<u64>,
    running: bool,
}

pub struct CloudThumbs {
    queue: Mutex<Queue>,
    listed: Mutex<HashMap<u64, Instant>>,
    locks: Mutex<HashMap<u64, Arc<tokio::sync::Mutex<()>>>>,
    background: Arc<Semaphore>,
    direct: Semaphore,
}

/// A file as the mount shows it, and its cache entry name.
struct Target {
    item: Item,
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
            listed: Mutex::new(HashMap::new()),
            locks: Mutex::new(HashMap::new()),
            background: Arc::new(Semaphore::new(BACKGROUND)),
            direct: Semaphore::new(DIRECT),
        }
    }
}

impl CloudThumbs {
    /// A folder was listed to be shown: queue its photos that need a thumbnail.
    pub fn folder_listed(&self, engine: &Arc<Engine>, dir: u64) {
        if !engine.cfg().cloud_thumbnails || !engine.can_reach_onedrive() {
            return;
        }
        {
            let now = Instant::now();
            let mut listed = self.listed.lock();
            if listed
                .get(&dir)
                .is_some_and(|t| now.duration_since(*t) < RELIST)
            {
                return;
            }
            listed.retain(|_, t| now.duration_since(*t) < RELIST);
            listed.insert(dir, now);
        }
        let engine = engine.clone();
        engine.rt.clone().spawn(async move {
            let e = engine.clone();
            let todo = tokio::task::spawn_blocking(move || scan(&e, dir))
                .await
                .unwrap_or_default();
            if todo.is_empty() {
                return;
            }
            tracing::debug!("queueing {} cloud thumbnails for folder {dir}", todo.len());
            {
                let mut q = engine.thumbs.queue.lock();
                for ino in todo.into_iter().rev() {
                    if q.queued.insert(ino) {
                        q.order.push_front(ino);
                    } else if let Some(i) = q.order.iter().position(|x| *x == ino) {
                        // Already waiting: move it up with the rest of this folder.
                        q.order.remove(i);
                        q.order.push_front(ino);
                    }
                }
                while q.order.len() > MAX_QUEUE {
                    if let Some(old) = q.order.pop_back() {
                        q.queued.remove(&old);
                    }
                }
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
        self.listed.lock().clear();
    }

    /// The thumbnail of `ino` at `bucket`, fetched from OneDrive if the file
    /// is online-only. `None` when there isn't one.
    pub async fn get(
        &self,
        engine: &Arc<Engine>,
        ino: u64,
        bucket: Bucket,
    ) -> Result<Option<PathBuf>> {
        let _permit = self.direct.acquire().await?;
        self.fetch(engine, ino, bucket).await
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
        let same = |r: &Thumb| r.md5 == t.name && r.mtime == t.mtime;
        if row.as_ref().is_some_and(|r| same(r) && r.sizes == 0) {
            return Ok(None); // OneDrive has no preview of it
        }
        let id = t.item.id.clone().unwrap_or_default();
        let px = bucket.px();
        let data = match engine.graph.thumbnail(&id, &format!("c{px}x{px}")).await {
            // No custom sizes here: take the 800 px one and scale it.
            Err(e) if e.status() == Some(400) => engine.graph.thumbnail(&id, "large").await?,
            r => r?,
        };
        let mut sizes = row.as_ref().filter(|r| same(r)).map_or(0, |r| r.sizes);
        if let Some(data) = data {
            let (uri, mtime, size, dir) = (t.uri.clone(), t.mtime, t.size, root.clone());
            sizes |= tokio::task::spawn_blocking(move || -> Result<u32> {
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
            .await??;
        }
        if let Some(old) = row.filter(|r| r.md5 != t.name) {
            thumbs::remove(&root, &[old.md5]); // written under a name it no longer has
        }
        engine.db.with(|s| {
            s.set_thumb(&Thumb {
                ino,
                md5: t.name.clone(),
                mtime: t.mtime,
                sizes,
            })
        })?;
        if sizes & bucket.bit() == 0 {
            return Ok(None);
        }
        thumbs::clear_failure(&root, &t.abs);
        engine.nudge(t.abs.clone());
        Ok(Some(want))
    }
}

/// The photos in a folder that need a thumbnail, in name order.
fn scan(engine: &Engine, dir: u64) -> Vec<u64> {
    let root = &engine.paths.thumbnail_dir;
    let Ok((rel, mut kids)) = engine
        .db
        .with(|s| -> Result<_> { Ok((s.path(dir)?, s.children(dir)?)) })
    else {
        return vec![];
    };
    kids.retain(wanted);
    kids.sort_by_cached_key(|k| k.name.to_lowercase());
    kids.truncate(PER_FOLDER);
    let base = engine.cfg().mount_path().join(rel.trim_start_matches('/'));
    kids.into_iter()
        .filter(|k| {
            let uri = thumbs::file_uri(&base.join(&k.name));
            let name = thumbs::cache_name(&uri);
            let mtime = k.mtime.max(0) / 1000;
            let path = thumbs::path_in(root, LISTING_SIZE, &name);
            if let Ok(Some(t)) = engine.db.with(|s| s.thumb(k.ino)) {
                if t.md5 == name && t.mtime == mtime {
                    if t.sizes == 0 {
                        return false; // OneDrive has none
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
        .map(|k| k.ino)
        .collect()
}
