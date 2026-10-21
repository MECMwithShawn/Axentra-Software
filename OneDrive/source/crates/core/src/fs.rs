//! Files On-Demand: the OneDrive folder as a FUSE filesystem.
//!
//! Every file is listed from local state, so browsing never touches the
//! network. Opening an online-only file downloads it into the cache first
//! (the reply is deferred, so other operations keep flowing); writes go to
//! the cached copy and are uploaded when the file is closed. Status is
//! exposed as `user.axentra.onedrive.*` extended attributes for file-manager badges.
//!
//! Thumbnailers get OneDrive's thumbnail instead of the file (see
//! [`crate::cloud_thumbs`]), and so does the file chooser portal.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::File;
use std::os::unix::fs::FileExt;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use anyhow::Result;
use fuser::{
    BsdFileFlags, Errno, FileAttr, FileHandle, FileType, Filesystem, FopenFlags, Generation,
    INodeNo, InitFlags, KernelConfig, LockOwner, MountOption, OpenAccMode, OpenFlags, RenameFlags,
    ReplyAttr, ReplyCreate, ReplyData, ReplyDirectory, ReplyEmpty, ReplyEntry, ReplyOpen,
    ReplyStatfs, ReplyWrite, ReplyXattr, Request, TimeOrNow, WriteFlags,
};
use parking_lot::Mutex;

use crate::db::{Item, Store, Tombstone, ROOT};
use crate::engine::{ms_to_system, now_ms, Engine};
use crate::names;
use crate::thumbs::{self, Bucket};

const TTL: Duration = Duration::from_secs(1);
/// How long a thumbnailer or the portal waits for OneDrive's thumbnail.
const THUMBNAIL_WAIT: Duration = Duration::from_secs(10);
const PINNED_XATTR: &str = "user.axentra.onedrive.pinned";
/// What `listxattr` offers. "pinned" is readable and settable by name but not
/// listed: copy tools (`cp -a`, `rsync -X`) would carry it to their copies,
/// and setting it on a copy back in here pins it or frees up its space.
const LISTED_XATTRS: [&str; 4] = [
    "user.axentra.onedrive.status",
    "user.axentra.onedrive.shared",
    "user.axentra.onedrive.url",
    "user.axentra.onedrive.error",
];
/// Folder entries read from the database at a time while listing.
const LISTING_PAGE: u32 = 512;

struct Handle {
    ino: u64,
    file: Arc<File>,
    write: bool,
    wrote: bool,
}

struct Inner {
    engine: Arc<Engine>,
    handles: Mutex<HashMap<u64, Handle>>,
    next_fh: AtomicU64,
    uid: u32,
    gid: u32,
}

pub struct AxentraOneDriveFs(Arc<Inner>);

fn eio(e: impl std::fmt::Display) -> Errno {
    tracing::warn!("filesystem error: {e}");
    Errno::EIO
}

/// Failures inside a transaction that should reach the caller as an errno.
#[derive(Debug)]
struct FsErr(Errno);

impl std::fmt::Display for FsErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

impl std::error::Error for FsErr {}

fn fail<T>(e: Errno) -> Result<T> {
    Err(FsErr(e).into())
}

fn to_errno(e: anyhow::Error) -> Errno {
    match e.downcast_ref::<FsErr>() {
        Some(FsErr(errno)) => *errno,
        None => eio(e),
    }
}

impl Inner {
    fn db<R>(&self, f: impl FnOnce(&Store<'_>) -> Result<R>) -> Result<R, Errno> {
        self.engine.db.with(f).map_err(to_errno)
    }

    fn tx<R>(&self, f: impl FnOnce(&Store<'_>) -> Result<R>) -> Result<R, Errno> {
        self.engine.db.tx(f).map_err(to_errno)
    }

    fn item(&self, ino: u64) -> Result<Item, Errno> {
        self.db(|s| s.get(ino))?.ok_or(Errno::ENOENT)
    }

    fn attr(&self, it: &Item) -> FileAttr {
        let mut size = it.size;
        let mut mtime = ms_to_system(it.mtime);
        if !it.is_dir && it.cached {
            if let Ok(m) = std::fs::metadata(self.engine.cache_path(it.ino)) {
                size = m.len();
                mtime = m.modified().unwrap_or(mtime);
            }
        }
        FileAttr {
            ino: INodeNo(it.ino),
            size: if it.is_dir { 4096 } else { size },
            // Online-only files take no space here, which `du` and the file
            // manager's "size on disk" both reflect.
            blocks: if it.cached && !it.is_dir {
                size.div_ceil(512)
            } else {
                0
            },
            atime: mtime,
            mtime,
            ctime: mtime,
            crtime: mtime,
            kind: if it.is_dir {
                FileType::Directory
            } else {
                FileType::RegularFile
            },
            perm: if it.is_dir { 0o755 } else { 0o644 },
            nlink: if it.is_dir { 2 } else { 1 },
            uid: self.uid,
            gid: self.gid,
            rdev: 0,
            blksize: 4096,
            flags: 0,
        }
    }

    fn status(&self, it: &Item) -> String {
        let base = it.status();
        if it.is_dir && base == "online" {
            if let Ok((total, cached)) = self.engine.db.with(|s| s.dir_availability(it.ino)) {
                if total > 0 && cached == total {
                    return "local".into();
                }
            }
        }
        base.into()
    }

    /// The value of one of our extended attributes, if `it` has one.
    fn xattr(&self, it: &Item, name: &str) -> Option<String> {
        match name {
            "user.axentra.onedrive.status" => Some(self.status(it)),
            PINNED_XATTR => Some(if it.pinned { "1" } else { "0" }.to_string()),
            "user.axentra.onedrive.shared" => it.shared.then(|| "1".to_string()),
            "user.axentra.onedrive.url" => it.web_url.clone(),
            "user.axentra.onedrive.error" => it.error.clone(),
            _ => None,
        }
    }

    /// Whether a new name in a folder is synced, and why not.
    fn classify(&self, in_local_only: bool, name: &str) -> (bool, Option<String>) {
        if in_local_only {
            return (true, None);
        }
        if self.engine.is_ignored(name) {
            return (true, None);
        }
        match names::invalid_reason(name) {
            Some(reason) => (true, Some(reason)),
            None => (false, None),
        }
    }

    /// Open the local copy. `truncate` empties it, creating it if the file
    /// is online-only.
    fn open_handle(&self, ino: u64, write: bool, truncate: bool) -> Result<u64, Errno> {
        // Counted as open before the content changes, so a pull never takes
        // the file for unmodified and closed in between.
        self.engine.opened(ino, write);
        let file = if truncate {
            self.overwrite(ino)
        } else {
            File::options()
                .read(true)
                .write(write)
                .open(self.engine.cache_path(ino))
                .map_err(eio)
        };
        let file = match file {
            Ok(f) => f,
            Err(e) => {
                self.engine.closed(ino, write);
                return Err(e);
            }
        };
        let fh = self.next_fh.fetch_add(1, Ordering::Relaxed);
        self.handles.lock().insert(
            fh,
            Handle {
                ino,
                file: Arc::new(file),
                write,
                wrote: truncate,
            },
        );
        Ok(fh)
    }

    /// The first change through a handle queues the file for upload right
    /// away rather than at close, so it isn't lost if the service stops
    /// while the file is still open. Uploads wait for the file to close.
    fn mark_written(&self, fh: u64) {
        let ino = match self.handles.lock().get_mut(&fh) {
            Some(h) if !h.wrote => {
                h.wrote = true;
                h.ino
            }
            _ => return,
        };
        if let Err(e) = self.engine.local_content_changed(ino) {
            tracing::warn!("couldn't record change to {ino}: {e:#}");
        }
    }

    fn handle_file(&self, fh: u64) -> Option<(Arc<File>, bool)> {
        self.handles
            .lock()
            .get(&fh)
            .map(|h| (h.file.clone(), h.write))
    }

    /// Empty the local copy, creating it if the file is online-only: it's
    /// being overwritten, so nothing is downloaded. The file is marked as
    /// changed here and then made under the database lock, so a download or
    /// "Free up space" finishing meanwhile leaves the new copy alone; if it
    /// can't be made (disk full, out of file handles) the mark is undone.
    fn overwrite(&self, ino: u64) -> Result<File, Errno> {
        let file = self.db(|s| {
            let was = s.get(ino)?.ok_or(FsErr(Errno::ENOENT))?;
            s.update(&Item {
                cached: true,
                dirty: true,
                gen: was.gen + 1,
                ..was.clone()
            })?;
            let file = File::options()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(self.engine.cache_path(ino));
            file.or_else(|e| {
                s.update(&was)?;
                Err(e.into())
            })
        })?;
        // Then its size, thumbnail and upload.
        if let Err(e) = self.engine.local_content_changed(ino) {
            tracing::warn!("couldn't record change to {ino}: {e:#}");
        }
        Ok(file)
    }

    /// Run `then` once the file's content is local, downloading if needed.
    fn with_content(
        self: &Arc<Self>,
        it: &Item,
        then: impl FnOnce(Result<(), Errno>) + Send + 'static,
    ) {
        if it.cached {
            return then(Ok(()));
        }
        let this = self.clone();
        let ino = it.ino;
        self.engine.rt.spawn(async move {
            let r = this.engine.hydrate(ino).await;
            then(r.map_err(|e| {
                tracing::warn!("couldn't download file {ino}: {e:#}");
                Errno::EIO
            }));
        });
    }

    /// Open a file whose content is (or is about to be made) local.
    fn finish_open(self: &Arc<Self>, it: Item, write: bool, trunc: bool, reply: ReplyOpen) {
        let ino = it.ino;
        let it = Item {
            cached: it.cached || trunc,
            ..it
        };
        let inner = self.clone();
        self.with_content(&it, move |ready| {
            if let Err(e) = ready {
                return reply.error(e);
            }
            match inner.open_handle(ino, write, trunc) {
                Ok(fh) => reply.opened(FileHandle(fh), FopenFlags::empty()),
                Err(e) => reply.error(e),
            }
        });
    }

    fn remove_thumbnails(&self, names: &[String]) {
        thumbs::remove(&self.engine.paths.thumbnail_dir, names);
    }

    fn tombstone(&self, s: &Store<'_>, it: &Item) -> Result<()> {
        if let Some(id) = &it.id {
            s.add_tombstone(&Tombstone {
                id: id.clone(),
                name: it.name.clone(),
                path: s.path(it.ino)?,
                is_dir: it.is_dir,
            })?;
        }
        Ok(())
    }

    /// Make `ino` local-only (or synced), and everything inside it whatever
    /// it would be if created there now: in a synced folder, ignored and
    /// invalid names still stay on this device.
    fn set_local_only(
        &self,
        s: &Store<'_>,
        ino: u64,
        local_only: bool,
        error: Option<&str>,
    ) -> Result<()> {
        let mut settled = HashMap::new();
        // Parents come before their children.
        for (i, mut it) in s.subtree(ino)?.into_iter().enumerate() {
            let (stays, reason) = if i == 0 {
                (local_only, error.map(str::to_string))
            } else {
                let in_local_only = settled.get(&it.parent).copied().unwrap_or(true);
                self.classify(in_local_only, &it.name)
            };
            settled.insert(it.ino, stays);
            if !stays && it.local_only {
                it.dirty = !it.is_dir;
            }
            it.local_only = stays;
            it.error = reason;
            s.update(&it)?;
        }
        Ok(())
    }
}

impl AxentraOneDriveFs {
    pub fn new(engine: Arc<Engine>) -> Self {
        Self(Arc::new(Inner {
            engine,
            handles: Mutex::new(HashMap::new()),
            next_fh: AtomicU64::new(1),
            uid: unsafe { libc::getuid() },
            gid: unsafe { libc::getgid() },
        }))
    }
}

fn utf8(name: &OsStr) -> Result<&str, Errno> {
    name.to_str().ok_or(Errno::EINVAL)
}

impl Filesystem for AxentraOneDriveFs {
    fn init(&mut self, _req: &Request, config: &mut KernelConfig) -> std::io::Result<()> {
        // Without this the kernel drops O_TRUNC from open() and truncates
        // afterwards, so overwriting an online-only file would download it
        // first (and fail offline).
        if let Err(missing) = config.add_capabilities(InitFlags::FUSE_ATOMIC_O_TRUNC) {
            tracing::debug!("kernel lacks {missing:?}");
        }
        Ok(())
    }

    fn lookup(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        let fs = &self.0;
        let Ok(name) = utf8(name) else {
            return reply.error(Errno::ENOENT);
        };
        // Names are unique ignoring case (create, mkdir and rename refuse a
        // second "readme.md"), but only the exact name is found: otherwise
        // the kernel takes "Report.pdf" for the "report.pdf" being renamed
        // to it and never sends the rename.
        match fs.db(|s| Ok(s.child(parent.0, name)?.filter(|it| it.name == name))) {
            Ok(Some(it)) => reply.entry(&TTL, &fs.attr(&it), Generation(0)),
            Ok(None) => reply.error(Errno::ENOENT),
            Err(e) => reply.error(e),
        }
    }

    fn getattr(&self, _req: &Request, ino: INodeNo, _fh: Option<FileHandle>, reply: ReplyAttr) {
        match self.0.item(ino.0) {
            Ok(it) => reply.attr(&TTL, &self.0.attr(&it)),
            Err(e) => reply.error(e),
        }
    }

    fn setattr(
        &self,
        _req: &Request,
        ino: INodeNo,
        _mode: Option<u32>,
        _uid: Option<u32>,
        _gid: Option<u32>,
        size: Option<u64>,
        _atime: Option<TimeOrNow>,
        mtime: Option<TimeOrNow>,
        _ctime: Option<SystemTime>,
        _fh: Option<FileHandle>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        _flags: Option<BsdFileFlags>,
        reply: ReplyAttr,
    ) {
        let fs = self.0.clone();
        let it = match fs.item(ino.0) {
            Ok(it) => it,
            Err(e) => return reply.error(e),
        };
        if size.is_some() && it.is_dir {
            return reply.error(Errno::EISDIR);
        }
        if size == Some(0) && !it.cached {
            if let Err(e) = fs.overwrite(it.ino) {
                return reply.error(e);
            }
        }
        // Only a truncate needs the content; a timestamp change doesn't.
        let fresh = Item {
            cached: it.cached || size.is_none() || size == Some(0),
            ..it
        };
        let inner = fs.clone();
        fs.with_content(&fresh.clone(), move |ready| {
            let fs = inner;
            if let Err(e) = ready {
                return reply.error(e);
            }
            let path = fs.engine.cache_path(fresh.ino);
            if let Some(size) = size {
                let r = File::options()
                    .write(true)
                    .open(&path)
                    .and_then(|f| f.set_len(size));
                if let Err(e) = r {
                    return reply.error(eio(e));
                }
                if let Err(e) = fs.engine.local_content_changed(fresh.ino) {
                    return reply.error(eio(e));
                }
            }
            if let Some(t) = mtime {
                let t = match t {
                    TimeOrNow::Now => SystemTime::now(),
                    TimeOrNow::SpecificTime(t) => t,
                };
                if let Ok(f) = File::options().write(true).open(&path) {
                    let _ = f.set_modified(t);
                }
                let ms = t
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or_else(|_| now_ms());
                let _ = fs.db(|s| {
                    if let Some(mut it) = s.get(fresh.ino)? {
                        it.mtime = ms;
                        s.update(&it)?;
                    }
                    Ok(())
                });
            }
            match fs.item(fresh.ino) {
                Ok(it) => reply.attr(&TTL, &fs.attr(&it)),
                Err(e) => reply.error(e),
            }
        });
    }

    fn mknod(
        &self,
        _req: &Request,
        _parent: INodeNo,
        _name: &OsStr,
        _mode: u32,
        _umask: u32,
        _rdev: u32,
        reply: ReplyEntry,
    ) {
        reply.error(Errno::EPERM);
    }

    fn mkdir(
        &self,
        _req: &Request,
        parent: INodeNo,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        reply: ReplyEntry,
    ) {
        let fs = &self.0;
        let Ok(name) = utf8(name) else {
            return reply.error(Errno::EINVAL);
        };
        if parent.0 == ROOT && names::is_trash_dir(name) {
            return reply.error(Errno::EPERM);
        }
        let result = fs.tx(|s| {
            let p = s.get(parent.0)?.ok_or(FsErr(Errno::ENOENT))?;
            if !p.is_dir {
                return fail(Errno::ENOTDIR);
            }
            if s.child(p.ino, name)?.is_some() {
                return fail(Errno::EEXIST);
            }
            let (local_only, error) = fs.classify(p.local_only, name);
            let mut it = Item {
                parent: p.ino,
                name: name.to_string(),
                is_dir: true,
                mtime: now_ms(),
                pinned: p.pinned,
                local_only,
                error,
                ..Default::default()
            };
            it.ino = s.insert(&it)?;
            Ok(it)
        });
        match result {
            Ok(it) => {
                fs.engine.wake();
                fs.engine.changed();
                reply.entry(&TTL, &fs.attr(&it), Generation(0));
            }
            Err(e) => reply.error(e),
        }
    }

    fn unlink(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        let fs = &self.0;
        let Ok(name) = utf8(name) else {
            return reply.error(Errno::ENOENT);
        };
        let result = fs.tx(|s| {
            let it = s.child(parent.0, name)?.ok_or(FsErr(Errno::ENOENT))?;
            if it.is_dir {
                return fail(Errno::EISDIR);
            }
            fs.tombstone(s, &it)?;
            s.delete(it.ino)?;
            Ok((it.ino, s.take_thumb(it.ino)?))
        });
        match result {
            Ok((ino, stale)) => {
                fs.remove_thumbnails(&stale);
                // Not left fetching what nobody can open any more.
                fs.engine.stop_download(ino);
                // Open handles keep the unlinked content readable.
                let _ = std::fs::remove_file(fs.engine.cache_path(ino));
                fs.engine.wake();
                fs.engine.changed();
                reply.ok();
            }
            Err(e) => reply.error(e),
        }
    }

    fn rmdir(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        let fs = &self.0;
        let Ok(name) = utf8(name) else {
            return reply.error(Errno::ENOENT);
        };
        let result = fs.tx(|s| {
            let it = s.child(parent.0, name)?.ok_or(FsErr(Errno::ENOENT))?;
            if !it.is_dir {
                return fail(Errno::ENOTDIR);
            }
            if s.child_count(it.ino)? > 0 {
                return fail(Errno::ENOTEMPTY);
            }
            fs.tombstone(s, &it)?;
            s.delete(it.ino)
        });
        match result {
            Ok(()) => {
                fs.engine.wake();
                fs.engine.changed();
                reply.ok();
            }
            Err(e) => reply.error(e),
        }
    }

    fn symlink(
        &self,
        _req: &Request,
        _parent: INodeNo,
        _link_name: &OsStr,
        _target: &Path,
        reply: ReplyEntry,
    ) {
        // OneDrive has no symbolic links.
        reply.error(Errno::EPERM);
    }

    fn link(
        &self,
        _req: &Request,
        _ino: INodeNo,
        _newparent: INodeNo,
        _newname: &OsStr,
        reply: ReplyEntry,
    ) {
        reply.error(Errno::EPERM);
    }

    fn rename(
        &self,
        _req: &Request,
        parent: INodeNo,
        name: &OsStr,
        newparent: INodeNo,
        newname: &OsStr,
        flags: RenameFlags,
        reply: ReplyEmpty,
    ) {
        let fs = &self.0;
        let (Ok(name), Ok(newname)) = (utf8(name), utf8(newname)) else {
            return reply.error(Errno::EINVAL);
        };
        if flags.contains(RenameFlags::RENAME_EXCHANGE)
            || flags.contains(RenameFlags::RENAME_WHITEOUT)
        {
            return reply.error(Errno::EINVAL);
        }
        if newparent.0 == ROOT && names::is_trash_dir(newname) {
            return reply.error(Errno::EPERM);
        }
        let mut drop_cache = Vec::new();
        let mut stale_thumbs = Vec::new();
        let mut pin = None;
        let result = fs.tx(|s| {
            let mut src = s.child(parent.0, name)?.ok_or(FsErr(Errno::ENOENT))?;
            // Thumbnails are named after the path, which is about to change.
            stale_thumbs = s.take_thumbs_under(src.ino)?;
            let np = s.get(newparent.0)?.ok_or(FsErr(Errno::ENOENT))?;
            if !np.is_dir {
                return fail(Errno::ENOTDIR);
            }
            if src.is_dir && s.is_inside(np.ino, src.ino)? {
                return fail(Errno::EINVAL);
            }
            let (local_only, error) = fs.classify(np.local_only, newname);
            let mut keep_moved = src.moved || src.id.is_some();
            if let Some(dest) = s.child(np.ino, newname)?.filter(|d| d.ino != src.ino) {
                // Another item whose name differs only in case can't be
                // there too, but isn't replaced: lookup doesn't find it, so
                // programs that ask before replacing a file never see it.
                if flags.contains(RenameFlags::RENAME_NOREPLACE) || dest.name != newname {
                    return fail(Errno::EEXIST);
                }
                if dest.is_dir != src.is_dir {
                    return fail(if dest.is_dir {
                        Errno::EISDIR
                    } else {
                        Errno::ENOTDIR
                    });
                }
                if dest.is_dir && s.child_count(dest.ino)? > 0 {
                    return fail(Errno::ENOTEMPTY);
                }
                let can_take_over = !dest.is_dir && dest.id.is_some() && !local_only && src.cached;
                if can_take_over {
                    // Save-by-rename (editors write a temp file, then rename it
                    // over the original): upload as a new version of the
                    // original so its sharing and version history survive.
                    if let Some(old) = src.id.take() {
                        s.add_tombstone(&Tombstone {
                            id: old,
                            name: src.name.clone(),
                            path: s.path(src.ino)?,
                            is_dir: false,
                        })?;
                    }
                    src.id = dest.id.clone();
                    src.etag = dest.etag.clone();
                    src.ctag = dest.ctag.clone();
                    src.qxh = dest.qxh.clone();
                    src.web_url = dest.web_url.clone();
                    src.shared = dest.shared;
                    src.pinned |= dest.pinned;
                    src.dirty = true;
                    src.gen += 1;
                    keep_moved = false;
                } else {
                    fs.tombstone(s, &dest)?;
                }
                s.delete(dest.ino)?;
                stale_thumbs.extend(s.take_thumb(dest.ino)?);
                drop_cache.push(dest.ino);
            }

            src.parent = np.ino;
            src.name = newname.to_string();
            src.moved = keep_moved && src.id.is_some();
            if np.pinned && !src.pinned {
                src.pinned = true;
                pin = Some(src.ino);
            }
            if local_only && !src.local_only {
                if src.id.is_some() && !src.is_dir && src.cached {
                    // Renamed to something that doesn't sync: remove it online.
                    fs.tombstone(s, &src)?;
                    src.id = None;
                    src.moved = false;
                    s.update(&src)?;
                    fs.set_local_only(s, src.ino, true, error.as_deref())?;
                } else if src.id.is_none() {
                    s.update(&src)?;
                    fs.set_local_only(s, src.ino, true, error.as_deref())?;
                } else {
                    // Can't drop a synced folder or online-only file locally;
                    // report the problem instead.
                    src.error = error;
                    s.update(&src)?;
                }
            } else if !local_only && src.local_only {
                s.update(&src)?;
                fs.set_local_only(s, src.ino, false, None)?;
            } else {
                if !local_only {
                    src.error = None;
                }
                s.update(&src)?;
            }
            Ok(())
        });
        match result {
            Ok(()) => {
                fs.remove_thumbnails(&stale_thumbs);
                for ino in drop_cache {
                    fs.engine.stop_download(ino);
                    let _ = std::fs::remove_file(fs.engine.cache_path(ino));
                }
                if let Some(ino) = pin {
                    // Moved into an "Always keep on this device" folder: so is
                    // everything inside it, and it all downloads now. Queued
                    // before replying, so what's asked for after the move
                    // comes after it too.
                    fs.engine.pin_moved_in(ino);
                }
                fs.engine.wake();
                fs.engine.changed();
                reply.ok();
            }
            Err(e) => reply.error(e),
        }
    }

    fn open(&self, req: &Request, ino: INodeNo, flags: OpenFlags, reply: ReplyOpen) {
        let fs = self.0.clone();
        let it = match fs.item(ino.0) {
            Ok(it) => it,
            Err(e) => return reply.error(e),
        };
        if it.is_dir {
            return reply.error(Errno::EISDIR);
        }
        // With FUSE_ATOMIC_O_TRUNC truncating is ours to do, even for O_RDONLY.
        let trunc = flags.0 & libc::O_TRUNC != 0;
        let write = trunc || flags.acc_mode() != OpenAccMode::O_RDONLY;
        // A file being overwritten needs nothing downloaded.
        if !it.cached && !trunc {
            let pid = req.pid();
            if thumbs::is_background_reader(pid) {
                if thumbs::is_thumbnailer_pid(pid) && fs.engine.may_fetch_thumbnail(&it) {
                    // Save OneDrive's thumbnail where the thumbnailer would
                    // have, then turn it away: file managers look for the
                    // thumbnail again when a thumbnailer gives up.
                    let size = thumbs::requested_size(pid).map_or(Bucket::Large, Bucket::fitting);
                    let engine = fs.engine.clone();
                    let who = thumbs::describe(pid);
                    fs.engine.rt.spawn(async move {
                        let got =
                            tokio::time::timeout(THUMBNAIL_WAIT, engine.thumbnail(ino.0, size))
                                .await;
                        match got {
                            Ok(Ok(Some(_))) => {
                                tracing::debug!("gave {who} OneDrive's thumbnail of {}", it.name)
                            }
                            Ok(Ok(None)) => {
                                tracing::debug!("not downloading {} for {who}", it.name)
                            }
                            Ok(Err(e)) => tracing::debug!(
                                "no cloud thumbnail of {} for {who}: {e:#}",
                                it.name
                            ),
                            Err(_) => {
                                tracing::debug!("OneDrive's thumbnail of {} took too long", it.name)
                            }
                        }
                        reply.error(Errno::EACCES);
                    });
                    return;
                }
                tracing::debug!("not downloading {} for {}", it.name, thumbs::describe(pid));
                return reply.error(Errno::EACCES);
            } else if thumbs::is_portal(pid) && fs.engine.may_fetch_thumbnail(&it) {
                // The file chooser reads photos for its previews; it can have
                // the thumbnail instead. Anything else it reads downloads.
                let who = thumbs::describe(pid);
                let inner = fs.clone();
                fs.engine.rt.spawn(async move {
                    let got = tokio::time::timeout(THUMBNAIL_WAIT, inner.engine.thumbnail(ino.0, Bucket::Large)).await;
                    if let Ok(Ok(Some(_))) = got {
                        tracing::info!("showing \"{}\" to {who} as OneDrive's thumbnail instead of downloading it", it.name);
                        return reply.error(Errno::EACCES);
                    }
                    tracing::info!("downloading \"{}\" on demand for {who}", it.name);
                    inner.finish_open(it, write, false, reply);
                });
                return;
            } else {
                tracing::info!(
                    "downloading \"{}\" on demand for {}",
                    it.name,
                    thumbs::describe(pid)
                );
            }
        }
        fs.finish_open(it, write, trunc, reply);
    }

    fn read(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        size: u32,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyData,
    ) {
        let Some((file, _)) = self.0.handle_file(fh.0) else {
            return reply.error(Errno::EBADF);
        };
        let mut buf = vec![0u8; size as usize];
        let mut filled = 0;
        while filled < buf.len() {
            match file.read_at(&mut buf[filled..], offset + filled as u64) {
                Ok(0) => break,
                Ok(n) => filled += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return reply.error(eio(e)),
            }
        }
        reply.data(&buf[..filled]);
    }

    fn write(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        data: &[u8],
        _write_flags: WriteFlags,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyWrite,
    ) {
        let Some((file, writable)) = self.0.handle_file(fh.0) else {
            return reply.error(Errno::EBADF);
        };
        if !writable {
            return reply.error(Errno::EBADF);
        }
        match file.write_all_at(data, offset) {
            Ok(()) => {
                self.0.mark_written(fh.0);
                reply.written(data.len() as u32);
            }
            Err(e) => reply.error(eio(e)),
        }
    }

    fn flush(
        &self,
        _req: &Request,
        _ino: INodeNo,
        _fh: FileHandle,
        _lock_owner: LockOwner,
        reply: ReplyEmpty,
    ) {
        reply.ok();
    }

    fn release(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        let fs = &self.0;
        let Some(h) = fs.handles.lock().remove(&fh.0) else {
            return reply.ok();
        };
        // Size and time are final now. Recorded before the handle stops
        // counting as a writer, so a pull never sees the file unmodified and
        // closed in between and drops the new content.
        if h.wrote {
            if let Err(e) = fs.engine.local_content_changed(h.ino) {
                tracing::warn!("couldn't record change to {}: {e:#}", h.ino);
            }
        }
        fs.engine.closed(h.ino, h.write);
        if !h.wrote && h.write {
            // A new file closed without writes (e.g. `touch`) still uploads.
            fs.engine.wake();
        }
        reply.ok();
    }

    fn fsync(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        datasync: bool,
        reply: ReplyEmpty,
    ) {
        let Some((file, _)) = self.0.handle_file(fh.0) else {
            return reply.ok();
        };
        let r = if datasync {
            file.sync_data()
        } else {
            file.sync_all()
        };
        match r {
            Ok(()) => reply.ok(),
            Err(e) => reply.error(eio(e)),
        }
    }

    fn readdir(
        &self,
        req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        mut reply: ReplyDirectory,
    ) {
        let came = Instant::now();
        let fs = &self.0;
        let dir = match fs.item(ino.0) {
            Ok(d) if d.is_dir => d,
            Ok(_) => return reply.error(Errno::ENOTDIR),
            Err(e) => return reply.error(e),
        };
        // Offsets are 1 for ".", 2 for ".." and inode + 1 for a child (their
        // inodes start at 2), so each call reads only the page it returns and
        // a listing continues after its last entry even if others came or
        // went meanwhile.
        let parent = if dir.ino == ROOT { ROOT } else { dir.parent };
        let mut full = (offset < 1 && reply.add(INodeNo(dir.ino), 1, FileType::Directory, "."))
            || (offset < 2 && reply.add(INodeNo(parent), 2, FileType::Directory, ".."));
        let mut after = offset.saturating_sub(1);
        while !full {
            let page = match fs.db(|s| s.listing(dir.ino, after, LISTING_PAGE)) {
                Ok(p) => p,
                Err(e) => return reply.error(e),
            };
            for (child, is_dir, name) in &page {
                let kind = if *is_dir {
                    FileType::Directory
                } else {
                    FileType::RegularFile
                };
                full = reply.add(INodeNo(*child), child + 1, kind, name);
                if full {
                    break;
                }
                after = *child;
            }
            if page.len() < LISTING_PAGE as usize {
                break;
            }
        }
        // Before the reply, while the caller waits for it: see
        // `CloudThumbs::folder_listed`.
        fs.engine
            .folder_listed(&dir, req.pid(), offset, came.elapsed());
        reply.ok();
    }

    fn statfs(&self, _req: &Request, _ino: INodeNo, reply: ReplyStatfs) {
        const BS: u64 = 4096;
        let a = self.0.engine.account();
        let (total, free) = if a.total > 0 {
            (a.total, a.remaining)
        } else {
            (1 << 40, 1 << 40)
        };
        reply.statfs(
            total / BS,
            free / BS,
            free / BS,
            1 << 24,
            1 << 24,
            BS as u32,
            255,
            BS as u32,
        );
    }

    fn setxattr(
        &self,
        _req: &Request,
        ino: INodeNo,
        name: &OsStr,
        value: &[u8],
        _flags: i32,
        _position: u32,
        reply: ReplyEmpty,
    ) {
        if name == thumbs::NUDGE_XATTR {
            return reply.ok(); // the change notification is the point
        }
        if name != PINNED_XATTR {
            return reply.error(Errno::EOPNOTSUPP);
        }
        let engine = &self.0.engine;
        let r = match value {
            b"1" | b"true" => engine.set_pinned(ino.0, true),
            b"0" | b"false" => engine.free_up_space(ino.0).map(|_| ()),
            _ => return reply.error(Errno::EINVAL),
        };
        match r {
            Ok(()) => reply.ok(),
            Err(e) => reply.error(eio(e)),
        }
    }

    fn getxattr(&self, _req: &Request, ino: INodeNo, name: &OsStr, size: u32, reply: ReplyXattr) {
        let fs = &self.0;
        let it = match fs.item(ino.0) {
            Ok(it) => it,
            Err(e) => return reply.error(e),
        };
        let Some(value) = fs.xattr(&it, name.to_str().unwrap_or("")) else {
            return reply.error(Errno::NO_XATTR);
        };
        let bytes = value.as_bytes();
        if size == 0 {
            reply.size(bytes.len() as u32);
        } else if (size as usize) < bytes.len() {
            reply.error(Errno::ERANGE);
        } else {
            reply.data(bytes);
        }
    }

    fn listxattr(&self, _req: &Request, ino: INodeNo, size: u32, reply: ReplyXattr) {
        let fs = &self.0;
        let it = match fs.item(ino.0) {
            Ok(it) => it,
            Err(e) => return reply.error(e),
        };
        // Only names that have a value: `rsync -X` fails on any it can't read.
        let mut buf = Vec::new();
        for n in LISTED_XATTRS {
            if fs.xattr(&it, n).is_some() {
                buf.extend_from_slice(n.as_bytes());
                buf.push(0);
            }
        }
        if size == 0 {
            reply.size(buf.len() as u32);
        } else if (size as usize) < buf.len() {
            reply.error(Errno::ERANGE);
        } else {
            reply.data(&buf);
        }
    }

    fn removexattr(&self, _req: &Request, _ino: INodeNo, _name: &OsStr, reply: ReplyEmpty) {
        reply.error(Errno::EOPNOTSUPP);
    }

    fn access(&self, _req: &Request, _ino: INodeNo, _mask: fuser::AccessFlags, reply: ReplyEmpty) {
        reply.ok();
    }

    fn create(
        &self,
        _req: &Request,
        parent: INodeNo,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        _flags: i32,
        reply: ReplyCreate,
    ) {
        let fs = &self.0;
        let Ok(name) = utf8(name) else {
            return reply.error(Errno::EINVAL);
        };
        let result = fs.tx(|s| {
            let p = s.get(parent.0)?.ok_or(FsErr(Errno::ENOENT))?;
            if !p.is_dir {
                return fail(Errno::ENOTDIR);
            }
            if s.child(p.ino, name)?.is_some() {
                return fail(Errno::EEXIST);
            }
            let (local_only, error) = fs.classify(p.local_only, name);
            let mut it = Item {
                parent: p.ino,
                name: name.to_string(),
                mtime: now_ms(),
                pinned: p.pinned,
                cached: true,
                dirty: !local_only,
                local_only,
                error,
                ..Default::default()
            };
            it.ino = s.insert(&it)?;
            File::create(fs.engine.cache_path(it.ino))?;
            Ok(it)
        });
        let it = match result {
            Ok(it) => it,
            Err(e) => return reply.error(e),
        };
        match fs.open_handle(it.ino, true, false) {
            Ok(fh) => {
                fs.engine.changed();
                reply.created(
                    &TTL,
                    &fs.attr(&it),
                    Generation(0),
                    FileHandle(fh),
                    FopenFlags::empty(),
                );
            }
            Err(e) => reply.error(e),
        }
    }
}

/// Mount the OneDrive folder; it stays mounted until the session is dropped.
pub fn mount(engine: Arc<Engine>, mountpoint: &Path) -> Result<fuser::BackgroundSession> {
    prepare_mountpoint(mountpoint)?;
    let mut cfg = fuser::Config::default();
    cfg.mount_options = vec![
        MountOption::FSName("axentra-onedrive".into()),
        MountOption::Subtype("axentra-onedrive".into()),
        MountOption::DefaultPermissions,
        MountOption::NoAtime,
        MountOption::NoDev,
        MountOption::NoSuid,
    ];
    cfg.n_threads = Some(4);
    Ok(fuser::spawn_mount(
        AxentraOneDriveFs::new(engine),
        mountpoint,
        &cfg,
    )?)
}

/// Refuse to hide existing files, and clear a mount left by a crash.
fn prepare_mountpoint(mountpoint: &Path) -> Result<()> {
    match std::fs::read_dir(mountpoint) {
        Ok(mut entries) => {
            if entries.next().is_some() {
                return Err(crate::config::NeedsUser(format!(
                    "{} already has files in it. Move them somewhere else (you can copy them \
                     back in once Axentra OneDrive is running), or choose another folder with \
                     `axentra-onedrive setup --folder`.",
                    mountpoint.display()
                ))
                .into());
            }
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir_all(mountpoint)?;
            Ok(())
        }
        Err(e) if e.raw_os_error() == Some(libc::ENOTCONN) => {
            let _ = std::process::Command::new("fusermount3")
                .arg("-uz")
                .arg(mountpoint)
                .status();
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}
