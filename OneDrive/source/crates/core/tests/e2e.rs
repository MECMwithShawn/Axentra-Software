//! End-to-end: a real FUSE mount backed by the engine, syncing with the mock
//! Graph server. Each test gets its own mount in a temp directory.

use axentra_onedrive_core::db::ROOT;
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axentra_onedrive_core::config::{Config, Paths};
use axentra_onedrive_core::engine::{Engine, Options};
use axentra_onedrive_mock::MockServer;

struct Env {
    mock: MockServer,
    engine: Arc<Engine>,
    mnt: PathBuf,
    session: Option<fuser::BackgroundSession>,
    _dir: tempfile::TempDir,
}

impl Drop for Env {
    fn drop(&mut self) {
        if let Some(s) = self.session.take() {
            if s.umount_and_join().is_err() {
                // Busy (e.g. a desktop volume monitor peeking at the new mount).
                let _ = std::process::Command::new("fusermount3")
                    .arg("-uz")
                    .arg(&self.mnt)
                    .status();
            }
        }
    }
}

impl Env {
    async fn new() -> Self {
        Self::with(|_| {}, false).await
    }

    async fn with(tweak: impl FnOnce(&mut Config), expired_token: bool) -> Self {
        let _ = tracing_subscriber::fmt()
            .with_test_writer()
            .with_env_filter("axentra_onedrive_core=debug")
            .try_init();
        let dir = tempfile::tempdir().unwrap();
        let mock = MockServer::start().await;
        let paths = Paths::under(&dir.path().join("state"));
        paths.ensure().unwrap();
        std::fs::write(&paths.tokens_file, mock.tokens_json(expired_token)).unwrap();
        let mnt = dir.path().join("OneDrive");
        let mut cfg = Config {
            client_id: "test".into(),
            authority: mock.origin.clone(),
            graph_base: mock.graph_base.clone(),
            mount_point: mnt.display().to_string(),
            poll_interval_secs: 3600,
            ..Default::default()
        };
        tweak(&mut cfg);
        let engine = Engine::new(
            paths,
            cfg,
            Options {
                notifications: false,
            },
        )
        .unwrap();
        // What a folder listing looks like (shown, or counted) doesn't hang
        // on how long a busy machine takes between a test's steps: time
        // passes for it only with `age_thumbnail_state`.
        engine.hold_thumbnail_clock();
        let e = engine.clone();
        let session =
            tokio::task::spawn_blocking(move || axentra_onedrive_core::daemon::start_mounted(e))
                .await
                .unwrap()
                .unwrap();
        Env {
            mock,
            engine,
            mnt,
            session: Some(session),
            _dir: dir,
        }
    }

    fn p(&self, rel: &str) -> PathBuf {
        self.mnt.join(rel.trim_start_matches('/'))
    }

    /// Wait for files written through the mount to be closed there: the
    /// kernel passes on a close after close() returns, and uploads wait for
    /// files that are still open for writing.
    async fn settle(&self) {
        eventually("files to close", || {
            let items = self.engine.db.with(|s| s.subtree(ROOT)).unwrap();
            items.iter().all(|it| !self.engine.has_writers(it.ino))
        })
        .await;
    }

    async fn sync(&self) {
        self.settle().await;
        self.engine.cycle().await;
        let st = self.engine.status();
        assert!(
            !matches!(st.state.as_str(), "error" | "offline" | "signed_out"),
            "sync failed: {st:?}"
        );
    }

    fn ino(&self, rel: &str) -> u64 {
        self.engine
            .db
            .with(|s| s.resolve(rel))
            .unwrap()
            .unwrap_or_else(|| panic!("no local item {rel}"))
            .ino
    }
}

/// Filesystem calls go through FUSE, which may need the async runtime to
/// download, so they must not run on a runtime thread.
async fn fsop<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(f).await.unwrap()
}

fn xattr(path: &Path, name: &str) -> Option<String> {
    let p = CString::new(path.as_os_str().as_bytes()).unwrap();
    let n = CString::new(name).unwrap();
    let mut buf = vec![0u8; 512];
    let r = unsafe { libc::getxattr(p.as_ptr(), n.as_ptr(), buf.as_mut_ptr().cast(), buf.len()) };
    if r < 0 {
        return None;
    }
    buf.truncate(r as usize);
    String::from_utf8(buf).ok()
}

async fn status_of(path: PathBuf) -> String {
    fsop(move || xattr(&path, "user.axentra.onedrive.status").unwrap_or_default()).await
}

async fn read(path: PathBuf) -> std::io::Result<Vec<u8>> {
    fsop(move || std::fs::read(path)).await
}

async fn write(path: PathBuf, data: impl Into<Vec<u8>>) {
    let data = data.into();
    fsop(move || std::fs::write(&path, data).unwrap()).await
}

const MIB: usize = 1024 * 1024;

/// `len` bytes that differ with `seed`: big enough ones upload in fragments.
fn bytes(len: usize, seed: u8) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8 ^ seed).collect()
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

async fn list(dir: PathBuf) -> Vec<String> {
    fsop(move || names(&dir)).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn online_files_are_listed_without_downloading() {
    let env = Env::new().await;
    env.mock.add_folder("/", "Documents");
    env.mock.add_file("/Documents", "a.txt", b"hello");
    env.mock.add_file("/", "big.bin", &vec![9u8; 5_000_000]);
    env.sync().await;

    assert_eq!(list(env.mnt.clone()).await, vec!["Documents", "big.bin"]);
    let big = env.p("big.bin");
    let meta = fsop(move || std::fs::metadata(big).unwrap()).await;
    assert_eq!(meta.len(), 5_000_000);
    assert_eq!(meta.blocks(), 0, "online-only files take no local space");
    assert_eq!(status_of(env.p("big.bin")).await, "online");
    assert_eq!(env.mock.requests("/content"), 0, "nothing downloaded yet");

    // The Files extension rereads badges through GIO, which names them so.
    if std::process::Command::new("gio")
        .arg("--version")
        .output()
        .is_ok()
    {
        let big = env.p("big.bin");
        let out = fsop(move || {
            std::process::Command::new("gio")
                .args(["info", "-a"])
                .arg("xattr::axentra.onedrive.status,xattr::axentra.onedrive.shared")
                .arg(&big)
                .output()
                .unwrap()
        })
        .await;
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(
            text.contains("xattr::axentra.onedrive.status: online"),
            "{text}"
        );
        assert!(!text.contains("xattr::axentra.onedrive.shared"), "{text}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn opening_an_online_file_downloads_it_once() {
    let env = Env::new().await;
    env.mock.add_folder("/", "Documents");
    env.mock.add_file("/Documents", "a.txt", b"hello");
    env.sync().await;

    assert_eq!(read(env.p("Documents/a.txt")).await.unwrap(), b"hello");
    assert_eq!(read(env.p("Documents/a.txt")).await.unwrap(), b"hello");
    assert_eq!(env.mock.requests("/content"), 1);
    assert_eq!(status_of(env.p("Documents/a.txt")).await, "local");
    // Every file in the folder is now local, so the folder shows as local too.
    assert_eq!(status_of(env.p("Documents")).await, "local");
    let kinds: Vec<String> = env
        .engine
        .activity(10)
        .into_iter()
        .map(|a| a.kind)
        .collect();
    assert!(kinds.contains(&"downloaded".to_string()), "{kinds:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn local_changes_upload() {
    let env = Env::new().await;
    env.sync().await;

    write(env.p("new.txt"), b"abc").await;
    let projects = env.p("Projects/sub");
    fsop(move || std::fs::create_dir_all(projects).unwrap()).await;
    write(env.p("Projects/sub/x.md"), b"# notes").await;
    assert_eq!(status_of(env.p("new.txt")).await, "syncing");
    env.sync().await;
    assert_eq!(env.mock.content("/new.txt").unwrap(), b"abc");
    assert_eq!(env.mock.content("/Projects/sub/x.md").unwrap(), b"# notes");
    assert_eq!(status_of(env.p("new.txt")).await, "local");

    // Edit in place: a new version of the same item.
    let id = env.mock.id("/new.txt").unwrap();
    let p = env.p("new.txt");
    fsop(move || {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().append(true).open(p).unwrap();
        f.write_all(b"def").unwrap();
    })
    .await;
    // The kernel reports the close in the background, after close() returns.
    eventually("the edit to reach the engine", || {
        env.engine
            .db
            .with(|s| s.resolve("/new.txt"))
            .unwrap()
            .is_some_and(|it| it.dirty)
    })
    .await;
    env.sync().await;
    assert_eq!(env.mock.content("/new.txt").unwrap(), b"abcdef");
    assert_eq!(env.mock.version_count("/new.txt"), 2);

    // Rename keeps the item's identity.
    let (a, b) = (env.p("new.txt"), env.p("Projects/renamed.txt"));
    fsop(move || std::fs::rename(a, b).unwrap()).await;
    env.sync().await;
    assert!(!env.mock.exists("/new.txt"));
    assert_eq!(env.mock.id("/Projects/renamed.txt").unwrap(), id);

    // Large files go through an upload session.
    let big: &'static [u8] = Box::leak(vec![42u8; 9 * 1024 * 1024 + 123].into_boxed_slice());
    write_closed(&env, "Projects/video.mp4", big).await;
    env.sync().await;
    assert_eq!(env.mock.content("/Projects/video.mp4").unwrap(), big);
    assert!(env.mock.requests("createUploadSession") >= 1);

    // Deletes go to the online recycle bin.
    let gone = env.p("Projects/renamed.txt");
    fsop(move || std::fs::remove_file(gone).unwrap()).await;
    let sub = env.p("Projects/sub");
    fsop(move || std::fs::remove_dir_all(sub).unwrap()).await;
    env.sync().await;
    assert!(!env.mock.exists("/Projects/renamed.txt"));
    assert!(!env.mock.exists("/Projects/sub"));
    assert_eq!(env.mock.list("/Projects"), vec!["video.mp4"]);
    assert_eq!(env.engine.status().state, "up_to_date");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn online_changes_come_down() {
    let env = Env::new().await;
    env.mock.add_folder("/", "Docs");
    env.mock.add_file("/Docs", "r.txt", b"v1");
    env.mock.add_file("/Docs", "old-name.txt", b"x");
    env.mock.add_file("/Docs", "doomed.txt", b"y");
    env.sync().await;
    assert_eq!(read(env.p("Docs/r.txt")).await.unwrap(), b"v1");

    env.mock.modify("/Docs/r.txt", b"version two");
    env.mock.rename("/Docs/old-name.txt", "new-name.txt");
    env.mock.delete("/Docs/doomed.txt");
    env.mock.add_file("/Docs", "fresh.txt", b"z");
    env.sync().await;

    assert_eq!(
        list(env.p("Docs")).await,
        vec!["fresh.txt", "new-name.txt", "r.txt"]
    );
    assert_eq!(read(env.p("Docs/r.txt")).await.unwrap(), b"version two");
    let kinds: Vec<String> = env
        .engine
        .activity(20)
        .into_iter()
        .map(|a| a.kind)
        .collect();
    assert!(
        kinds.contains(&"deleted_online".into()) && kinds.contains(&"added_online".into()),
        "{kinds:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn conflicting_edits_keep_both_copies() {
    let env = Env::new().await;
    env.mock.add_file("/", "c.txt", b"base");
    env.sync().await;
    assert_eq!(read(env.p("c.txt")).await.unwrap(), b"base");

    write(env.p("c.txt"), b"local edit").await;
    env.mock.modify("/c.txt", b"remote edit");
    env.sync().await;

    let copy = format!("c-{}.txt", env.engine.host());
    let mut expected = vec!["c.txt".to_string(), copy.clone()];
    expected.sort();
    assert_eq!(list(env.mnt.clone()).await, expected);
    assert_eq!(read(env.p("c.txt")).await.unwrap(), b"remote edit");
    assert_eq!(read(env.p(&copy)).await.unwrap(), b"local edit");
    assert_eq!(
        env.mock.content(&format!("/{copy}")).unwrap(),
        b"local edit"
    );
    assert_eq!(env.mock.content("/c.txt").unwrap(), b"remote edit");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn deleted_online_but_edited_here_is_kept() {
    let env = Env::new().await;
    env.mock.add_file("/", "keep.txt", b"old");
    env.sync().await;
    write(env.p("keep.txt"), b"my changes").await;
    env.mock.delete("/keep.txt");
    env.sync().await;
    assert_eq!(read(env.p("keep.txt")).await.unwrap(), b"my changes");
    assert_eq!(env.mock.content("/keep.txt").unwrap(), b"my changes");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn free_up_space_and_always_keep() {
    let env = Env::new().await;
    env.mock.add_folder("/", "Photos");
    for i in 0..5 {
        env.mock
            .add_file("/Photos", &format!("p{i}.jpg"), &vec![i as u8; 100_000]);
    }
    env.sync().await;

    assert_eq!(read(env.p("Photos/p0.jpg")).await.unwrap().len(), 100_000);
    let ino = env.ino("/Photos/p0.jpg");
    // The kernel reports the close in the background; until then it's open,
    // and an open file keeps its local copy.
    eventually("the file to be closed", || !env.engine.is_open(ino)).await;
    assert_eq!(env.engine.free_up_space(ino).unwrap(), 0);
    let p0 = env.p("Photos/p0.jpg");
    assert_eq!(
        fsop(move || std::fs::metadata(p0).unwrap().blocks()).await,
        0
    );
    assert_eq!(status_of(env.p("Photos/p0.jpg")).await, "online");

    // "Always keep on this device" downloads everything in the folder.
    env.engine.set_pinned(env.ino("/Photos"), true).unwrap();
    for _ in 0..100 {
        if env
            .engine
            .db
            .with(|s| s.pinned_uncached())
            .unwrap()
            .is_empty()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    for i in 0..5 {
        assert_eq!(
            status_of(env.p(&format!("Photos/p{i}.jpg"))).await,
            "pinned"
        );
    }
    // New files online in a pinned folder are pinned (and so downloaded) too.
    env.mock.add_file("/Photos", "p9.jpg", b"new");
    env.sync().await;
    eventually("the new file to download", || {
        env.engine
            .db
            .with(|s| s.pinned_uncached())
            .unwrap()
            .is_empty()
    })
    .await;
    assert_eq!(status_of(env.p("Photos/p9.jpg")).await, "pinned");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn save_by_rename_keeps_version_history() {
    let env = Env::new().await;
    env.mock.add_file("/", "doc.txt", b"first");
    env.sync().await;
    let id = env.mock.id("/doc.txt").unwrap();
    assert_eq!(read(env.p("doc.txt")).await.unwrap(), b"first");

    // What gedit does: write a temp file, rename it over the original.
    write(env.p(".goutputstream-7XK2Q1"), b"second").await;
    let (a, b) = (env.p(".goutputstream-7XK2Q1"), env.p("doc.txt"));
    fsop(move || std::fs::rename(a, b).unwrap()).await;
    env.sync().await;

    assert_eq!(env.mock.list("/"), vec!["doc.txt"]);
    assert_eq!(env.mock.id("/doc.txt").unwrap(), id);
    assert_eq!(env.mock.content("/doc.txt").unwrap(), b"second");
    assert_eq!(env.mock.version_count("/doc.txt"), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn paging_and_expired_change_feed() {
    let env = Env::new().await;
    env.mock.with(|d| d.page_size = 2);
    env.mock.add_folder("/", "A");
    for i in 0..6 {
        env.mock.add_file("/A", &format!("{i}.txt"), b"x");
    }
    env.sync().await;
    assert_eq!(list(env.p("A")).await.len(), 6);

    // While we "weren't looking": a delete, then the feed expires.
    env.mock.delete("/A/3.txt");
    env.mock.add_file("/A", "late.txt", b"y");
    env.mock.expire_delta_tokens();
    env.sync().await;
    let names = list(env.p("A")).await;
    assert!(names.contains(&"late.txt".to_string()));
    assert!(!names.contains(&"3.txt".to_string()), "{names:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn expired_access_token_is_refreshed() {
    let env = Env::with(|_| {}, true).await;
    env.mock.add_file("/", "a.txt", b"1");
    env.sync().await;
    assert_eq!(list(env.mnt.clone()).await, vec!["a.txt"]);
    let saved = std::fs::read_to_string(&env.engine.paths.tokens_file).unwrap();
    assert!(saved.contains("mock-access-1"), "{saved}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unreachable_sign_in_service_means_offline() {
    // Starting before the network is up: the token refresh can't connect.
    let env = Env::with(|c| c.authority = "http://127.0.0.1:9".into(), true).await;
    env.mock.add_file("/", "a.txt", b"1");
    env.engine.cycle().await;
    let st = env.engine.status();
    assert_eq!(st.state, "offline", "{st:?}");

    // Back online: the next pass refreshes and syncs.
    let mut cfg = env.engine.cfg();
    cfg.authority = env.mock.origin.clone();
    env.engine.set_config(cfg);
    env.sync().await;
    assert_eq!(list(env.mnt.clone()).await, vec!["a.txt"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unmount_does_not_wait_for_open_folders() {
    let mut env = Env::new().await;
    env.mock.add_folder("/", "A");
    env.mock.add_file("/A", "a.txt", b"1");
    env.sync().await;
    // Something (a folder window, a search) keeps a folder open.
    let dir = env.p("A");
    let held = fsop(move || std::fs::File::open(dir).unwrap()).await;

    let session = env.session.take().unwrap();
    let started = std::time::Instant::now();
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        axentra_onedrive_core::daemon::unmount(
            session,
            &env.mnt,
            std::time::Duration::from_millis(500),
        ),
    )
    .await
    .expect("unmount waited for the open folder");
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    let mounts = std::fs::read_to_string("/proc/self/mountinfo").unwrap();
    assert!(
        !mounts.contains(&format!(" {} ", env.mnt.display())),
        "still mounted"
    );
    fsop(move || drop(held)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn names_onedrive_cannot_hold_stay_local() {
    let env = Env::new().await;
    env.sync().await;
    write(env.p("~$Report.docx"), b"lock").await;
    write(env.p("bad:name.txt"), b"x").await;
    write(env.p("fine.txt"), b"y").await;
    env.settle().await;
    env.engine.cycle().await; // reports a sync issue, which is the point
    assert_eq!(env.mock.list("/"), vec!["fine.txt"]);
    assert_eq!(status_of(env.p("~$Report.docx")).await, "local-only");
    assert_eq!(status_of(env.p("bad:name.txt")).await, "error");
    assert_eq!(env.engine.status().state, "error");

    // Renaming to a valid name fixes it.
    let (a, b) = (env.p("bad:name.txt"), env.p("good name.txt"));
    fsop(move || std::fs::rename(a, b).unwrap()).await;
    env.sync().await;
    assert_eq!(env.mock.content("/good name.txt").unwrap(), b"x");
    assert_eq!(env.engine.status().state, "up_to_date");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn trash_folder_is_refused_and_names_are_case_insensitive() {
    let env = Env::new().await;
    env.sync().await;
    let trash = env.p(".Trash-1000");
    let err = fsop(move || std::fs::create_dir(trash).unwrap_err()).await;
    assert_eq!(err.raw_os_error(), Some(libc::EPERM));

    write(env.p("Readme.md"), b"x").await;
    let dup = env.p("README.md");
    let err = fsop(move || {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dup)
            .unwrap_err()
    })
    .await;
    assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn throttling_is_retried() {
    let env = Env::new().await;
    env.mock.add_file("/", "a.txt", b"1");
    env.mock.with(|d| d.fail_next = Some((429, 2)));
    env.sync().await;
    assert_eq!(list(env.mnt.clone()).await, vec!["a.txt"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sharing_and_versions() {
    let env = Env::new().await;
    env.mock.add_file("/", "s.txt", b"one");
    env.mock.modify("/s.txt", b"two");
    env.sync().await;
    let ino = env.ino("/s.txt");
    let link = env.engine.share_link(ino, false).await.unwrap();
    assert!(link.starts_with("https://1drv.ms/"), "{link}");
    let versions = env.engine.versions(ino).await.unwrap();
    assert_eq!(versions.len(), 2);
    env.engine
        .restore_version(ino, &versions[1].id)
        .await
        .unwrap();
    env.sync().await;
    assert_eq!(read(env.p("s.txt")).await.unwrap(), b"one");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sharing_changes_the_badge_without_an_activity_entry() {
    let env = Env::new().await;
    env.mock.add_folder("/", "Docs");
    env.mock.add_file("/Docs", "s.txt", b"one");
    env.mock.add_file("/Docs", "t.txt", b"two");
    env.sync().await;
    env.engine.take_rebadged();
    let shared = |rel: &str| {
        let p = env.p(rel);
        fsop(move || xattr(&p, "user.axentra.onedrive.shared"))
    };
    let entries = env.engine.activity(500).len();

    // Shared on the website: the next pass names it, for its badge.
    env.mock.set_shared("/Docs/s.txt", true);
    env.sync().await;
    assert_eq!(shared("Docs/s.txt").await.as_deref(), Some("1"));
    assert_eq!(env.engine.take_rebadged(), ["/Docs/s.txt"]);
    // A pass that changes nothing names nothing.
    env.sync().await;
    assert!(env.engine.take_rebadged().is_empty());
    // And unshared.
    env.mock.set_shared("/Docs/s.txt", false);
    env.sync().await;
    assert_eq!(shared("Docs/s.txt").await, None);
    assert_eq!(env.engine.take_rebadged(), ["/Docs/s.txt"]);
    // None of it is in the activity feed.
    assert_eq!(env.engine.activity(500).len(), entries);

    // Shared from here: badged at once, before the next pass.
    let ino = env.ino("/Docs/t.txt");
    env.engine.share_link(ino, false).await.unwrap();
    assert_eq!(shared("Docs/t.txt").await.as_deref(), Some("1"));
    assert_eq!(env.engine.take_rebadged(), ["/Docs/t.txt"]);
    // The pass that hears of it online has nothing more to say.
    env.sync().await;
    assert!(env.engine.take_rebadged().is_empty());
}

fn set_xattr(path: &Path, name: &str, value: &[u8]) -> i32 {
    let p = CString::new(path.as_os_str().as_bytes()).unwrap();
    let n = CString::new(name).unwrap();
    unsafe {
        libc::setxattr(
            p.as_ptr(),
            n.as_ptr(),
            value.as_ptr().cast(),
            value.len(),
            0,
        )
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pinning_through_extended_attributes() {
    let env = Env::new().await;
    env.mock.add_file("/", "x.bin", &vec![1u8; 4096]);
    env.sync().await;
    let p = env.p("x.bin");
    assert_eq!(
        fsop(move || set_xattr(&p, "user.axentra.onedrive.pinned", b"1")).await,
        0
    );
    for _ in 0..100 {
        if status_of(env.p("x.bin")).await == "pinned" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(status_of(env.p("x.bin")).await, "pinned");
    let p = env.p("x.bin");
    assert_eq!(
        fsop(move || set_xattr(&p, "user.axentra.onedrive.pinned", b"0")).await,
        0
    );
    assert_eq!(status_of(env.p("x.bin")).await, "online");
    let p = env.p("x.bin");
    let other = fsop(move || set_xattr(&p, "user.something", b"1")).await;
    assert_eq!(other, -1, "other attributes aren't stored");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn interrupted_first_scan_resumes_where_it_stopped() {
    let env = Env::new().await;
    env.mock.with(|d| d.page_size = 2);
    env.mock.add_folder("/", "A");
    for i in 0..7 {
        env.mock.add_file("/A", &format!("{i}.txt"), b"x");
    }
    // The scan is cut off after two pages (as if the service stopped).
    env.mock.with(|d| d.fail_delta_from = Some(4));
    env.engine.cycle().await;
    // Two pages were applied (the drive root and A, then two files) and the
    // position of the third was saved.
    assert!(env.engine.db.with(|s| s.kv("scan_next")).unwrap().is_some());
    assert_eq!(env.engine.db.with(|s| s.item_count()).unwrap(), 3);
    let fresh_starts = |env: &Env| {
        env.mock.with(|d| {
            d.log
                .iter()
                .filter(|l| l.as_str() == "GET /me/drive/root/delta")
                .count()
        })
    };
    assert_eq!(fresh_starts(&env), 1);

    env.mock.with(|d| d.fail_delta_from = None);
    env.sync().await;
    assert_eq!(
        fresh_starts(&env),
        1,
        "the scan continued instead of starting over"
    );
    assert_eq!(list(env.p("A")).await.len(), 7);
    assert_eq!(env.engine.status().state, "up_to_date");
}

// ----- passes stopped part-way, and changes made while one runs ----------------

/// Start a sync pass that waits at the first request matching `hold`, and
/// return once it's waiting there. `release` lets it go on.
async fn pass_held_at(env: &Env, hold: &str) -> tokio::task::JoinHandle<()> {
    env.mock.with(|d| {
        d.hold = Some(hold.into());
        d.held = 0;
    });
    let engine = env.engine.clone();
    let pass = tokio::spawn(async move { engine.cycle().await });
    eventually("the pass to reach the held request", || {
        env.mock.with(|d| d.held > 0)
    })
    .await;
    pass
}

fn release(env: &Env) {
    env.mock.with(|d| d.hold = None);
}

/// Write a file and wait until the mount has seen it closed, which the
/// kernel reports in the background after close() returns.
async fn write_closed(env: &Env, rel: &str, data: impl Into<Vec<u8>>) {
    write(env.p(rel), data).await;
    let ino = env.ino(rel);
    eventually("the file to be closed", || !env.engine.is_open(ino)).await;
}

/// What gedit does: write a temp file, then rename it over the original.
async fn save_by_rename(env: &Env, tmp: &str, rel: &str, data: &'static [u8]) {
    write_closed(env, tmp, data).await;
    let (a, b) = (env.p(tmp), env.p(rel));
    fsop(move || std::fs::rename(a, b).unwrap()).await;
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pausing_stops_the_upload_in_progress() {
    let env = Env::new().await;
    env.sync().await;
    write_closed(&env, "new.txt", b"abc").await;
    assert_eq!(env.engine.status().detail, "1 change waiting to sync");
    let pass = pass_held_at(&env, ":/new.txt:/content").await;
    assert_eq!(env.engine.status().uploading, 1);

    env.engine.pause(3600);
    tokio::time::timeout(Duration::from_secs(5), pass)
        .await
        .expect("the pass went on after pausing")
        .unwrap();
    let st = env.engine.status();
    assert_eq!(st.state, "paused");
    assert!(st.transfers.is_empty(), "{st:?}");

    release(&env);
    env.engine.resume();
    env.sync().await;
    assert_eq!(env.mock.content("/new.txt").unwrap(), b"abc");
    assert_eq!(list(env.mnt.clone()).await, vec!["new.txt"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn signing_out_stops_the_pass_before_forgetting_everything() {
    let env = Env::new().await;
    env.mock.add_file("/", "old.txt", b"o");
    env.sync().await;
    write_closed(&env, "new.txt", b"abc").await;
    let pass = pass_held_at(&env, ":/new.txt:/content").await;

    tokio::time::timeout(Duration::from_secs(5), env.engine.sign_out())
        .await
        .expect("signing out waited for the upload")
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), pass)
        .await
        .expect("the pass went on after signing out")
        .unwrap();
    release(&env);
    tokio::time::sleep(Duration::from_millis(300)).await;
    // Nothing of the old pass landed in the fresh state.
    let (tombstones, kv): (usize, i64) = env.engine.db.with(|s| {
        (
            s.tombstones().unwrap().len(),
            s.0.query_row("SELECT COUNT(*) FROM kv", [], |r| r.get(0))
                .unwrap(),
        )
    });
    assert_eq!((tombstones, kv), (0, 0));

    // Signing in again deletes nothing online.
    std::fs::write(&env.engine.paths.tokens_file, env.mock.tokens_json(false)).unwrap();
    env.engine.reload_tokens().await;
    env.sync().await;
    assert_eq!(env.mock.requests("DELETE"), 0);
    assert!(env.mock.exists("/old.txt"));
    assert_eq!(
        sorted(list(env.mnt.clone()).await),
        sorted(env.mock.list("/"))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn files_moved_out_of_a_folder_that_is_then_deleted_survive() {
    let env = Env::new().await;
    for d in ["D", "E", "F"] {
        env.mock.add_folder("/", d);
    }
    env.mock.add_file("/D", "a.txt", b"a");
    env.mock.add_file("/D", "b.txt", b"b");
    env.sync().await;
    let a = env.mock.id("/D/a.txt").unwrap();
    let f = env.mock.id("/F").unwrap();

    let (d, e) = (env.p("D"), env.p("E"));
    fsop(move || {
        for n in ["a.txt", "b.txt"] {
            std::fs::rename(d.join(n), e.join(n)).unwrap();
        }
        std::fs::remove_dir(d).unwrap();
    })
    .await;
    // A folder deleted and made again is a new folder online too.
    let fp = env.p("F");
    fsop(move || {
        std::fs::remove_dir(&fp).unwrap();
        std::fs::create_dir(&fp).unwrap();
    })
    .await;
    env.sync().await;

    assert!(!env.mock.exists("/D"));
    assert_eq!(env.mock.list("/E"), vec!["a.txt", "b.txt"]);
    assert_eq!(
        env.mock.id("/E/a.txt").unwrap(),
        a,
        "moved, not re-uploaded"
    );
    assert_eq!(list(env.p("E")).await, vec!["a.txt", "b.txt"]);
    assert_eq!(read(env.p("E/b.txt")).await.unwrap(), b"b");
    let new_f = env.mock.id("/F").unwrap();
    assert_ne!(new_f, f);
    let local_f = env.engine.db.with(|s| s.resolve("/F")).unwrap().unwrap();
    assert_eq!(local_f.id, Some(new_f));
    assert_eq!(env.engine.status().state, "up_to_date");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_new_folder_can_take_the_name_of_one_renamed_here() {
    let env = Env::new().await;
    env.mock.add_folder("/", "Photos");
    env.mock.add_file("/Photos", "a.jpg", b"a");
    env.sync().await;
    let id = env.mock.id("/Photos").unwrap();

    let (old, new) = (env.p("Photos"), env.p("Photos 2023"));
    fsop(move || {
        std::fs::rename(&old, new).unwrap();
        std::fs::create_dir(&old).unwrap();
    })
    .await;
    write_closed(&env, "Photos/b.jpg", b"b").await;
    tokio::time::timeout(Duration::from_secs(20), env.sync())
        .await
        .expect("the pass never finished");

    assert_eq!(env.mock.id("/Photos 2023").unwrap(), id);
    assert_eq!(env.mock.list("/Photos 2023"), vec!["a.jpg"]);
    assert_ne!(env.mock.id("/Photos").unwrap(), id);
    assert_eq!(env.mock.content("/Photos/b.jpg").unwrap(), b"b");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn saving_again_while_the_last_save_uploads_keeps_the_file() {
    let env = Env::new().await;
    env.mock.add_file("/", "doc.txt", b"first");
    env.sync().await;
    let id = env.mock.id("/doc.txt").unwrap();
    assert_eq!(read(env.p("doc.txt")).await.unwrap(), b"first");

    save_by_rename(&env, ".goutputstream-1", "doc.txt", b"second").await;
    let pass = pass_held_at(&env, &format!("PUT /v1.0/me/drive/items/{id}/content")).await;
    save_by_rename(&env, ".goutputstream-2", "doc.txt", b"third").await;
    release(&env);
    pass.await.unwrap();
    env.sync().await;

    assert_eq!(env.mock.list("/"), vec!["doc.txt"]);
    assert_eq!(env.mock.id("/doc.txt").unwrap(), id);
    assert_eq!(env.mock.content("/doc.txt").unwrap(), b"third");
    assert_eq!(list(env.mnt.clone()).await, vec!["doc.txt"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_upload_that_went_through_unheard_is_not_a_conflict() {
    let env = Env::new().await;
    env.sync().await;
    write_closed(&env, "report.pdf", b"the report").await;
    write_closed(&env, "notes.txt", b"mine").await;
    // Already online: the first as if an earlier attempt went through
    // without an answer, the second with other content.
    env.mock.add_file("/", "report.pdf", b"the report");
    env.mock.add_file("/", "notes.txt", b"theirs");
    env.sync().await;

    let copy = format!("notes-{}.txt", env.engine.host());
    let want = sorted(vec!["notes.txt".into(), copy.clone(), "report.pdf".into()]);
    assert_eq!(list(env.mnt.clone()).await, want);
    assert_eq!(env.mock.list("/"), want);
    assert_eq!(read(env.p(&copy)).await.unwrap(), b"mine");
    assert_eq!(status_of(env.p("report.pdf")).await, "local");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_file_opened_and_edited_during_a_long_pull_keeps_the_edit() {
    let env = Env::new().await;
    env.mock.with(|d| d.page_size = 1);
    env.mock.add_file("/", "x.txt", b"v1");
    env.sync().await;
    assert_eq!(read(env.p("x.txt")).await.unwrap(), b"v1");
    // Changed online, with enough else to take several pages.
    env.mock.modify("/x.txt", b"v2");
    for i in 0..3 {
        env.mock.add_file("/", &format!("{i}.txt"), b"n");
    }

    // The pass applies the first page (x.txt), then waits ...
    let pass = pass_held_at(&env, "skip=1").await;
    // ... while x.txt is opened, which downloads the new version, and edited.
    assert_eq!(read(env.p("x.txt")).await.unwrap(), b"v2");
    write_closed(&env, "x.txt", b"my edit").await;
    release(&env);
    pass.await.unwrap();

    assert_eq!(read(env.p("x.txt")).await.unwrap(), b"my edit");
    env.sync().await;
    assert_eq!(env.mock.content("/x.txt").unwrap(), b"my edit");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_file_rewritten_while_it_downloads_keeps_the_new_content() {
    let env = Env::new().await;
    env.mock.add_file("/", "x.pdf", b"old content");
    env.sync().await;
    let id = env.mock.id("/x.pdf").unwrap();
    env.mock.with(|d| {
        d.hold = Some(format!("GET /v1.0/me/drive/items/{id}/content"));
        d.held = 0;
    });
    env.engine.set_pinned(env.ino("/x.pdf"), true).unwrap();
    eventually("the download to start", || env.mock.with(|d| d.held > 0)).await;

    // Emptied and written while that download is under way.
    let p = env.p("x.pdf");
    fsop(move || {
        let c = CString::new(p.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::truncate(c.as_ptr(), 0) }, 0);
    })
    .await;
    write_closed(&env, "x.pdf", b"new").await;
    release(&env);
    eventually("the download to end", || {
        env.engine.status().downloading == 0
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    assert_eq!(read(env.p("x.pdf")).await.unwrap(), b"new");
    env.sync().await;
    assert_eq!(env.mock.content("/x.pdf").unwrap(), b"new");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn renamed_here_to_a_name_just_taken_online() {
    let env = Env::new().await;
    env.mock.add_file("/", "X.txt", b"x");
    env.sync().await;
    let id = env.mock.id("/X.txt").unwrap();

    let pass = pass_held_at(&env, "/root/delta?token=").await;
    let (a, b) = (env.p("X.txt"), env.p("A.txt"));
    fsop(move || std::fs::rename(a, b).unwrap()).await;
    env.mock.add_file("/", "A.txt", b"a");
    release(&env);
    pass.await.unwrap();
    env.sync().await;

    let ours = format!("A-{}.txt", env.engine.host());
    let want = sorted(vec!["A.txt".into(), ours.clone()]);
    assert_eq!(env.mock.list("/"), want);
    assert_eq!(env.mock.id(&format!("/{ours}")).unwrap(), id);
    assert_eq!(list(env.mnt.clone()).await, want);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn edited_here_while_replaced_online_keeps_a_visible_copy() {
    let env = Env::new().await;
    env.mock.add_file("/", "a.txt", b"old");
    env.sync().await;
    assert_eq!(read(env.p("a.txt")).await.unwrap(), b"old");

    let pass = pass_held_at(&env, "/root/delta?token=").await;
    write_closed(&env, "a.txt", b"my edit").await;
    // Another PC replaced it: deleted, then made again.
    env.mock.delete("/a.txt");
    env.mock.add_file("/", "a.txt", b"new");
    release(&env);
    pass.await.unwrap();
    env.sync().await;

    let ours = format!("a-{}.txt", env.engine.host());
    let want = sorted(vec!["a.txt".into(), ours.clone()]);
    assert_eq!(env.mock.list("/"), want);
    assert_eq!(env.mock.content(&format!("/{ours}")).unwrap(), b"my edit");
    assert_eq!(env.mock.content("/a.txt").unwrap(), b"new");
    assert_eq!(list(env.mnt.clone()).await, want);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn folders_moved_into_each_other_here_and_online() {
    let env = Env::new().await;
    env.mock.add_folder("/", "A");
    env.mock.add_folder("/", "B");
    env.mock.add_file("/B", "b.txt", b"b");
    env.sync().await;

    let pass = pass_held_at(&env, "/root/delta?token=").await;
    // Here B goes into A; online, A goes into B.
    let (from, to) = (env.p("B"), env.p("A/B"));
    fsop(move || std::fs::rename(from, to).unwrap()).await;
    env.mock.move_to("/A", "/B");
    release(&env);
    tokio::time::timeout(Duration::from_secs(20), pass)
        .await
        .expect("the pass never finished")
        .unwrap();
    tokio::time::timeout(Duration::from_secs(20), env.sync())
        .await
        .expect("the pass never finished");

    // The online move won.
    assert_eq!(env.mock.list("/"), vec!["B"]);
    assert_eq!(env.mock.list("/B"), vec!["A", "b.txt"]);
    assert_eq!(list(env.mnt.clone()).await, vec!["B"]);
    assert_eq!(list(env.p("B")).await, vec!["A", "b.txt"]);
    assert_eq!(env.engine.status().state, "up_to_date");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_folder_flattened_into_its_own_name() {
    let env = Env::new().await;
    env.mock.add_folder("/", "Foo");
    env.mock.add_folder("/Foo", "Foo");
    env.mock.add_file("/Foo/Foo", "a.txt", b"a");
    env.sync().await;
    let inner = env.mock.id("/Foo/Foo").unwrap();

    // What tidying up an extracted archive looks like.
    let root = env.mnt.clone();
    fsop(move || {
        std::fs::rename(root.join("Foo/Foo"), root.join("Foo.tmp")).unwrap();
        std::fs::remove_dir(root.join("Foo")).unwrap();
        std::fs::rename(root.join("Foo.tmp"), root.join("Foo")).unwrap();
    })
    .await;
    tokio::time::timeout(Duration::from_secs(20), env.sync())
        .await
        .expect("the pass never finished");

    assert_eq!(env.mock.list("/"), vec!["Foo"]);
    assert_eq!(env.mock.id("/Foo").unwrap(), inner);
    assert_eq!(env.mock.list("/Foo"), vec!["a.txt"]);
    assert_eq!(list(env.mnt.clone()).await, vec!["Foo"]);
    assert_eq!(read(env.p("Foo/a.txt")).await.unwrap(), b"a");
    assert_eq!(env.engine.status().state, "up_to_date");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_folder_emptied_into_a_new_one_of_its_name() {
    let env = Env::new().await;
    env.mock.add_folder("/", "F");
    env.mock.add_file("/F", "a.txt", b"a");
    env.mock.add_file("/F", "b.txt", b"b");
    env.sync().await;
    let old = env.mock.id("/F").unwrap();
    let a = env.mock.id("/F/a.txt").unwrap();

    let root = env.mnt.clone();
    fsop(move || {
        std::fs::rename(root.join("F"), root.join("F.old")).unwrap();
        std::fs::create_dir(root.join("F")).unwrap();
        for n in ["a.txt", "b.txt"] {
            std::fs::rename(root.join("F.old").join(n), root.join("F").join(n)).unwrap();
        }
        std::fs::remove_dir(root.join("F.old")).unwrap();
    })
    .await;
    tokio::time::timeout(Duration::from_secs(20), env.sync())
        .await
        .expect("the pass never finished");

    assert_eq!(env.mock.list("/"), vec!["F"]);
    assert_ne!(env.mock.id("/F").unwrap(), old);
    assert_eq!(env.mock.list("/F"), vec!["a.txt", "b.txt"]);
    assert_eq!(
        env.mock.id("/F/a.txt").unwrap(),
        a,
        "moved, not re-uploaded"
    );
    assert_eq!(list(env.p("F")).await, vec!["a.txt", "b.txt"]);
    assert_eq!(env.engine.status().state, "up_to_date");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_new_file_named_like_another_but_for_case_beyond_ascii() {
    let env = Env::new().await;
    env.mock.add_file("/", "Übersicht.docx", b"theirs");
    env.mock.add_folder("/", "Ärger");
    env.sync().await;

    // Different names here, one name to OneDrive.
    write_closed(&env, "übersicht.docx", b"mine").await;
    let p = env.p("ärger");
    fsop(move || std::fs::create_dir(p).unwrap()).await;
    env.sync().await;

    let host = env.engine.host().to_string();
    let file = format!("übersicht-{host}.docx");
    let folder = format!("ärger-{host}");
    let want = sorted(vec![
        "Übersicht.docx".into(),
        "Ärger".into(),
        file.clone(),
        folder.clone(),
    ]);
    assert_eq!(env.mock.list("/"), want);
    assert_eq!(env.mock.content(&format!("/{file}")).unwrap(), b"mine");
    assert_eq!(env.mock.content("/Übersicht.docx").unwrap(), b"theirs");
    assert_eq!(list(env.mnt.clone()).await, want);
    assert_eq!(env.engine.status().state, "up_to_date");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn moved_into_a_folder_deleted_online_meanwhile() {
    let env = Env::new().await;
    env.mock.add_folder("/", "S");
    env.mock.add_folder("/", "D");
    env.mock.add_file("/S", "a.txt", b"a");
    env.sync().await;
    let a = env.mock.id("/S/a.txt").unwrap();

    let (from, to) = (env.p("S/a.txt"), env.p("D/a.txt"));
    fsop(move || std::fs::rename(from, to).unwrap()).await;
    env.mock.delete("/D");
    env.sync().await;

    // The folder is made again for what was moved into it.
    assert_eq!(env.mock.list("/"), vec!["D", "S"]);
    assert_eq!(env.mock.list("/D"), vec!["a.txt"]);
    assert_eq!(env.mock.id("/D/a.txt").unwrap(), a);
    assert!(env.mock.list("/S").is_empty());
    assert_eq!(list(env.p("D")).await, vec!["a.txt"]);
    assert_eq!(read(env.p("D/a.txt")).await.unwrap(), b"a");
    assert_eq!(env.engine.status().state, "up_to_date");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_upload_problem_is_reported_once_and_retried_later() {
    let env = Env::new().await;
    env.sync().await;
    let errors = |env: &Env| {
        env.engine
            .activity(50)
            .into_iter()
            .filter(|a| a.kind == "error")
            .count()
    };
    // The kernel tells the mount about a close in the background, after
    // close() returns: wait until the engine has taken in the change and
    // the file counts as closed (each edit raises `gen`, at least once).
    let taken_in = |edits: i64| {
        let engine = env.engine.clone();
        eventually("the change to reach the engine", move || {
            engine
                .db
                .with(|s| s.resolve("/a.txt"))
                .unwrap()
                .is_some_and(|it| it.gen >= edits && it.dirty && !engine.is_open(it.ino))
        })
    };
    let append = |data: &'static [u8]| {
        let p = env.p("a.txt");
        fsop(move || {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new().append(true).open(p).unwrap();
            f.write_all(data).unwrap();
        })
    };
    write(env.p("a.txt"), b"1").await;
    taken_in(1).await;
    env.mock.with(|d| d.fail_next = Some((403, 1)));
    env.engine.cycle().await;
    assert_eq!(errors(&env), 1);
    assert_eq!(status_of(env.p("a.txt")).await, "error");

    // Not tried again on every pass ...
    let tried = env.mock.requests("PUT ");
    env.engine.cycle().await;
    assert_eq!(env.mock.requests("PUT "), tried);
    // ... but after another change, and the same problem isn't reported again.
    append(b"2").await;
    taken_in(2).await;
    env.mock.with(|d| d.fail_next = Some((403, 1)));
    env.engine.cycle().await;
    assert_eq!(env.mock.requests("PUT "), tried + 1);
    assert_eq!(errors(&env), 1);

    append(b"3").await;
    taken_in(3).await;
    env.sync().await;
    assert_eq!(env.mock.content("/a.txt").unwrap(), b"123");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failed_background_download_is_not_retried_every_pass() {
    let env = Env::new().await;
    env.mock.add_file("/", "v.bin", &vec![1u8; 4096]);
    env.sync().await;
    let downloads = |env: &Env| env.mock.requests("/content");

    env.mock.with(|d| d.fail_next = Some((403, 1)));
    env.engine.set_pinned(env.ino("/v.bin"), true).unwrap();
    eventually("the download to be tried", || downloads(&env) == 1).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    env.sync().await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(downloads(&env), 1);

    // Asking again tries again.
    env.engine.set_pinned(env.ino("/v.bin"), true).unwrap();
    eventually("the download", || {
        env.engine
            .db
            .with(|s| s.pinned_uncached())
            .unwrap()
            .is_empty()
    })
    .await;
    assert_eq!(status_of(env.p("v.bin")).await, "pinned");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pausing_a_first_scan_keeps_what_waits_for_its_folder() {
    let env = Env::new().await;
    env.mock.add_folder("/", "A");
    env.mock.add_file("/A", "x.txt", b"x");
    // An item a page, and the file a page before its folder.
    env.mock.with(|d| {
        d.page_size = 1;
        d.children_first = true;
    });
    let pass = pass_held_at(&env, "skip=2").await;
    env.engine.pause(3600);
    tokio::time::timeout(Duration::from_secs(5), pass)
        .await
        .expect("the pass went on after pausing")
        .unwrap();
    release(&env);
    env.engine.resume();
    env.sync().await;
    assert_eq!(list(env.p("A")).await, vec!["x.txt"]);
    assert_eq!(read(env.p("A/x.txt")).await.unwrap(), b"x");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pausing_keeps_a_big_upload_where_it_got_to() {
    let env = Env::new().await;
    env.sync().await;
    let data = bytes(12 * MIB, 1);
    write_closed(&env, "big.bin", data.clone()).await;
    // Paused while the second fragment goes up ...
    let pass = pass_held_at(&env, &format!("bytes {}-", 10 * MIB)).await;
    env.engine.pause(3600);
    tokio::time::timeout(Duration::from_secs(5), pass)
        .await
        .expect("the pass went on after pausing")
        .unwrap();
    // ... which never arrives.
    env.mock.with(|d| {
        d.fail_fragments_from = Some(10 * MIB as u64);
        d.hold = None;
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    env.mock.with(|d| d.fail_fragments_from = None);
    assert_eq!(env.mock.upload_sessions(), 1);

    env.engine.resume();
    env.sync().await;
    assert_eq!(env.mock.content("/big.bin").unwrap(), data);
    assert_eq!(env.mock.requests("createUploadSession"), 1);
    assert_eq!(
        env.mock.with(|d| d.upload_bytes),
        data.len() as u64,
        "nothing went up twice"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pausing_stops_background_downloads() {
    let env = Env::new().await;
    env.mock.add_file("/", "v.bin", &[1u8; 4096]);
    env.sync().await;
    let id = env.mock.id("/v.bin").unwrap();
    let ino = env.ino("/v.bin");
    env.mock.with(|d| {
        d.hold = Some(format!("GET /v1.0/me/drive/items/{id}/content"));
        d.held = 0;
    });
    env.engine.set_pinned(ino, true).unwrap();
    eventually("the download to start", || env.mock.with(|d| d.held > 0)).await;
    assert_eq!(env.engine.status().downloading, 1);

    env.engine.pause(3600);
    eventually("the download to stop", || {
        env.engine.status().downloading == 0
    })
    .await;
    release(&env);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(status_of(env.p("v.bin")).await, "online");
    assert!(!env.engine.cache_path(ino).with_extension("part").exists());

    env.engine.resume();
    env.sync().await;
    eventually("the download after resuming", || {
        env.engine
            .db
            .with(|s| s.pinned_uncached())
            .unwrap()
            .is_empty()
    })
    .await;
    assert_eq!(status_of(env.p("v.bin")).await, "pinned");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_new_file_saved_over_another_while_it_uploads() {
    let env = Env::new().await;
    env.mock.add_file("/", "cover.txt", b"old cover");
    env.sync().await;
    let id = env.mock.id("/cover.txt").unwrap();
    write_closed(&env, "draft.txt", b"new cover").await;
    let pass = pass_held_at(&env, ":/draft.txt:/content").await;
    let (a, b) = (env.p("draft.txt"), env.p("cover.txt"));
    fsop(move || std::fs::rename(a, b).unwrap()).await;
    release(&env);
    pass.await.unwrap();
    env.sync().await;

    // A new version of the file it replaced, and nothing else.
    assert_eq!(env.mock.list("/"), vec!["cover.txt"]);
    assert_eq!(env.mock.id("/cover.txt").unwrap(), id);
    assert_eq!(env.mock.content("/cover.txt").unwrap(), b"new cover");
    assert_eq!(list(env.mnt.clone()).await, vec!["cover.txt"]);
    assert_eq!(status_of(env.p("cover.txt")).await, "local");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_file_opened_for_writing_while_uploads_wait_their_turn_waits_for_it() {
    use std::io::Write;
    let env = Env::with(|c| c.max_parallel_transfers = 1, false).await;
    env.sync().await;
    write_closed(&env, "a.txt", b"a").await;
    write_closed(&env, "b.txt", b"b").await;
    let pass = pass_held_at(&env, ":/a.txt:/content").await;
    // Opened again before its turn came.
    let p = env.p("b.txt");
    let file = fsop(move || {
        let mut f = std::fs::OpenOptions::new().append(true).open(p).unwrap();
        f.write_all(b" and more").unwrap();
        f
    })
    .await;
    release(&env);
    pass.await.unwrap();
    assert!(!env.mock.exists("/b.txt"), "uploaded while being written");

    fsop(move || drop(file)).await;
    env.sync().await;
    assert_eq!(env.mock.content("/b.txt").unwrap(), b"b and more");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_file_rewritten_while_it_uploads_is_not_a_failure() {
    let env = Env::new().await;
    env.sync().await;
    write_closed(&env, "big.bin", bytes(12 * MIB, 2)).await;
    // Saved again, smaller, while the first fragment goes up.
    let pass = pass_held_at(&env, "bytes 0-").await;
    write_closed(&env, "big.bin", b"short").await;
    release(&env);
    pass.await.unwrap();
    let errors = env
        .engine
        .activity(50)
        .into_iter()
        .filter(|a| a.kind == "error")
        .count();
    assert_eq!(errors, 0);
    assert_ne!(env.engine.status().state, "error");
    // Not held back for a while, as a failure is.
    env.sync().await;
    assert_eq!(env.mock.content("/big.bin").unwrap(), b"short");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_upload_that_landed_unheard_is_not_a_conflict_when_it_comes_down() {
    let env = Env::new().await;
    env.sync().await;
    write_closed(&env, "big.bin", bytes(12 * MIB, 3)).await;
    // The last fragment lands but its answer is lost, and so is every
    // attempt to look for the file after it.
    let pass = pass_held_at(&env, &format!("bytes {}-", 10 * MIB)).await;
    env.mock.with(|d| {
        d.lose_replies_at = vec![10 * MIB as u64];
        d.fail_next = Some((503, 6));
        d.hold = None;
    });
    pass.await.unwrap();
    env.sync().await;

    assert_eq!(env.mock.list("/"), vec!["big.bin"]);
    assert_eq!(list(env.mnt.clone()).await, vec!["big.bin"]);
    assert_eq!(env.mock.requests("createUploadSession"), 1, "uploaded once");
    assert_eq!(status_of(env.p("big.bin")).await, "local");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn saved_again_after_an_upload_landed_unheard_is_not_a_conflict() {
    let env = Env::new().await;
    env.mock.add_file("/", "big.bin", &bytes(12 * MIB, 4));
    env.sync().await;
    assert_eq!(read(env.p("big.bin")).await.unwrap().len(), 12 * MIB);
    let id = env.mock.id("/big.bin").unwrap();
    write_closed(&env, "big.bin", bytes(12 * MIB, 5)).await;
    // The last fragment lands but its answer is lost, and syncing is
    // paused while the upload looks for the file ...
    env.mock.with(|d| d.lose_replies_at = vec![10 * MIB as u64]);
    let pass = pass_held_at(&env, &format!("GET /v1.0/me/drive/items/{id}")).await;
    env.engine.pause(3600);
    tokio::time::timeout(Duration::from_secs(5), pass)
        .await
        .expect("the pass went on after pausing")
        .unwrap();
    release(&env);
    // ... and saved again meanwhile.
    write_closed(&env, "big.bin", b"edited while paused").await;
    env.engine.resume();
    env.sync().await;

    assert_eq!(env.mock.list("/"), vec!["big.bin"]);
    assert_eq!(env.mock.id("/big.bin").unwrap(), id);
    assert_eq!(
        env.mock.content("/big.bin").unwrap(),
        b"edited while paused"
    );
    assert_eq!(list(env.mnt.clone()).await, vec!["big.bin"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn kept_upload_sessions_are_cancelled_once_not_wanted() {
    use std::io::Write;
    let env = Env::new().await;
    env.sync().await;
    let interrupted = |env: &Env| {
        env.mock
            .with(|d| d.fail_fragments_from = Some(10 * MIB as u64));
        let engine = env.engine.clone();
        async move {
            engine.cycle().await;
        }
    };
    write_closed(&env, "a.bin", bytes(12 * MIB, 6)).await;
    interrupted(&env).await;
    env.mock.with(|d| d.fail_fragments_from = None);
    assert_eq!(env.mock.upload_sessions(), 1, "kept for the next attempt");
    // Renamed before then: it's a new file of another name online.
    let (a, b) = (env.p("a.bin"), env.p("b.bin"));
    fsop(move || std::fs::rename(a, b).unwrap()).await;
    env.engine.cycle().await;
    eventually("the session to be cancelled", || {
        env.mock.upload_sessions() == 0
    })
    .await;

    // And by signing out.
    let p = env.p("b.bin");
    fsop(move || {
        let mut f = std::fs::OpenOptions::new().append(true).open(p).unwrap();
        f.write_all(b"more").unwrap();
    })
    .await;
    env.settle().await;
    interrupted(&env).await;
    assert_eq!(env.mock.upload_sessions(), 1, "kept for the next attempt");
    env.engine.sign_out().await.unwrap();
    eventually("the session to be cancelled", || {
        env.mock.upload_sessions() == 0
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn moved_items_are_looked_up_once_a_pass() {
    let env = Env::new().await;
    for d in ["P", "Q", "T1", "T2", "T3"] {
        env.mock.add_folder("/", d);
    }
    for i in 0..5 {
        env.mock.add_file("/P", &format!("{i}.jpg"), b"x");
    }
    env.sync().await;
    let moved: Vec<String> = (0..5)
        .map(|i| env.mock.id(&format!("/P/{i}.jpg")).unwrap())
        .collect();
    let old = env.mock.id("/T1").unwrap();

    let root = env.mnt.clone();
    fsop(move || {
        for i in 0..5 {
            let n = format!("{i}.jpg");
            std::fs::rename(root.join("P").join(&n), root.join("Q").join(&n)).unwrap();
        }
        // Folders replaced by new ones of their names, each waiting for the
        // moves to know it can go.
        for t in ["T1", "T2", "T3"] {
            std::fs::remove_dir(root.join(t)).unwrap();
            std::fs::create_dir(root.join(t)).unwrap();
        }
    })
    .await;
    env.sync().await;

    for id in &moved {
        let get = format!("GET /me/drive/items/{id}");
        let n = env
            .mock
            .with(|d| d.log.iter().filter(|l| **l == get).count());
        assert_eq!(n, 1, "{id} looked up {n} times");
    }
    assert_eq!(env.mock.list("/Q").len(), 5);
    assert_ne!(env.mock.id("/T1").unwrap(), old);
    assert_eq!(env.engine.status().state, "up_to_date");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_folder_deleted_online_with_a_file_renamed_in_it_here_stays_deleted() {
    let env = Env::new().await;
    env.mock.add_folder("/", "X");
    env.mock.add_file("/X", "a.txt", b"a");
    env.sync().await;
    env.mock.with(|d| d.deleted_folders_only = true);

    // Moved into a new folder in it here, which can't be made online ...
    let x = env.p("X");
    fsop(move || {
        std::fs::create_dir(x.join("new")).unwrap();
        std::fs::rename(x.join("a.txt"), x.join("new/a.txt")).unwrap();
    })
    .await;
    // ... as it's deleted there meanwhile.
    env.mock.delete("/X");
    env.sync().await;
    env.sync().await;

    assert!(!env.mock.exists("/X"), "made again online");
    assert!(list(env.mnt.clone()).await.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn overwriting_a_file_stops_its_download() {
    use std::io::Write;
    let env = Env::new().await;
    env.mock.add_file("/", "x.pdf", b"old content");
    env.sync().await;
    let id = env.mock.id("/x.pdf").unwrap();
    env.mock.with(|d| {
        d.hold = Some(format!("GET /v1.0/me/drive/items/{id}/content"));
        d.held = 0;
    });
    env.engine.set_pinned(env.ino("/x.pdf"), true).unwrap();
    eventually("the download to start", || env.mock.with(|d| d.held > 0)).await;
    assert_eq!(env.engine.status().downloading, 1);

    let p = env.p("x.pdf");
    let file = fsop(move || {
        let mut f = std::fs::File::create(p).unwrap();
        f.write_all(b"new").unwrap();
        f
    })
    .await;
    // Not left running while OneDrive sends what's old now.
    eventually("the download to stop", || {
        env.engine.status().downloading == 0
    })
    .await;
    release(&env);
    fsop(move || drop(file)).await;
    env.sync().await;
    assert_eq!(env.mock.content("/x.pdf").unwrap(), b"new");
    assert_eq!(read(env.p("x.pdf")).await.unwrap(), b"new");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn moving_into_an_always_kept_folder_leaves_failed_downloads_waiting() {
    let env = Env::new().await;
    env.mock.add_folder("/", "Offline");
    env.mock.add_file("/Offline", "blocked.bin", b"b");
    env.mock.add_file("/", "x.txt", b"x");
    env.sync().await;
    // A download that keeps failing (blocked as malware, say).
    env.mock.with(|d| d.fail_next = Some((403, 1)));
    env.engine.set_pinned(env.ino("/Offline"), true).unwrap();
    eventually("the download to be tried", || env.mock.downloads() == 1).await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    let (a, b) = (env.p("x.txt"), env.p("Offline/x.txt"));
    fsop(move || std::fs::rename(a, b).unwrap()).await;
    eventually("the moved file to download", || {
        env.engine
            .db
            .with(|s| s.resolve("/Offline/x.txt"))
            .unwrap()
            .is_some_and(|it| it.cached)
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(env.mock.downloads(), 2, "the failed one was tried again");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn what_always_keep_was_fetching_stops_once_not_wanted() {
    let env = Env::with(|c| c.max_parallel_transfers = 1, false).await;
    env.mock.add_folder("/", "Videos");
    for i in 0..3 {
        env.mock
            .add_file("/Videos", &format!("{i}.bin"), &[i as u8; 4096]);
    }
    env.mock.add_file("/", "x.bin", &[9u8; 4096]);
    env.sync().await;
    let fetches = |env: &Env| env.mock.with(|d| d.held);
    let downloading = |env: &Env| env.engine.status().downloading;
    // Each download waits online until released.
    let start = |env: &Env, rel: &str| {
        env.mock.with(|d| {
            d.hold = Some("/content".into());
            d.held = 0;
        });
        env.engine.set_pinned(env.ino(rel), true).unwrap();
    };

    // "Download all files", then Files On-Demand back on while the first comes.
    start(&env, "/Videos");
    eventually("a download to start", || fetches(&env) == 1).await;
    assert_eq!(env.engine.free_up_space(env.ino("/Videos")).unwrap(), 0);
    eventually("the download to stop", || downloading(&env) == 0).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(fetches(&env), 1, "the others were fetched too");
    release(&env);

    // "Always keep on this device" unticked.
    start(&env, "/Videos");
    eventually("a download to start", || fetches(&env) == 1).await;
    env.engine.set_pinned(env.ino("/Videos"), false).unwrap();
    eventually("the download to stop", || downloading(&env) == 0).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(fetches(&env), 1, "the others were fetched too");
    release(&env);
    tokio::time::sleep(Duration::from_millis(300)).await;
    for i in 0..3 {
        let rel = format!("Videos/{i}.bin");
        assert_eq!(status_of(env.p(&rel)).await, "online", "{rel}");
    }
    assert_eq!(env.engine.local_usage(), 0);

    // Deleted.
    start(&env, "/x.bin");
    eventually("the download to start", || fetches(&env) == 1).await;
    let p = env.p("x.bin");
    fsop(move || std::fs::remove_file(p).unwrap()).await;
    eventually("the download to stop", || downloading(&env) == 0).await;
    release(&env);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn space_freed_up_right_after_moving_into_an_always_kept_folder_stays_free() {
    let env = Env::new().await;
    env.mock.add_folder("/", "Offline");
    env.mock.add_folder("/", "Big");
    env.mock.add_file("/Big", "x.txt", b"x");
    env.mock.add_folder("/Big", "Sub");
    for i in 0..3 {
        env.mock.add_file("/Big/Sub", &format!("{i}.txt"), b"x");
    }
    env.sync().await;
    env.engine.set_pinned(env.ino("/Offline"), true).unwrap();
    // Paused, so nothing downloads until both are done: in between, what's
    // in Sub is to be kept for a moment, and could start downloading before
    // freeing up space stops it.
    env.engine.pause(3600);

    // `mv Big Offline/ && setfattr -n user.axentra.onedrive.pinned -v 0 Offline/Big/Sub`
    let (a, b, sub) = (env.p("Big"), env.p("Offline/Big"), env.p("Offline/Big/Sub"));
    let freed = fsop(move || {
        std::fs::rename(a, b).unwrap();
        set_xattr(&sub, "user.axentra.onedrive.pinned", b"0")
    })
    .await;
    assert_eq!(freed, 0);
    env.engine.resume();
    env.sync().await;
    let x = env.ino("/Offline/Big/x.txt");
    eventually("the rest to download", || {
        env.engine
            .db
            .with(|s| s.get(x))
            .unwrap()
            .is_some_and(|it| it.cached)
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let kept = env.engine.db.with(|s| {
        let sub = s.resolve("/Offline/Big/Sub").unwrap().unwrap();
        let tree = s.subtree(sub.ino).unwrap();
        tree.iter().filter(|i| i.pinned || i.cached).count()
    });
    assert_eq!(kept, 0, "kept on this device after all");
    assert_eq!(env.mock.downloads(), 1);
}

/// Pause the pass uploading the small file `id` once its content is up,
/// while its date is being set, so the pass never hears it went through.
/// The date doesn't arrive.
async fn paused_while_dated(env: &Env, id: &str) {
    let pass = pass_held_at(env, &format!("PATCH /v1.0/me/drive/items/{id}")).await;
    env.engine.pause(3600);
    tokio::time::timeout(Duration::from_secs(5), pass)
        .await
        .expect("the pass went on after pausing")
        .unwrap();
    env.mock.with(|d| {
        d.fail_next = Some((503, 1));
        d.hold = None;
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    env.mock.with(|d| d.fail_next = None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pausing_while_an_upload_is_dated_is_no_conflict() {
    let env = Env::new().await;
    env.mock.add_file("/", "doc.txt", b"first");
    env.sync().await;
    assert_eq!(read(env.p("doc.txt")).await.unwrap(), b"first");
    let id = env.mock.id("/doc.txt").unwrap();

    // Saved, with a date long ago (copied in from elsewhere, say).
    write_closed(&env, "doc.txt", b"second").await;
    let then = std::time::UNIX_EPOCH + Duration::from_secs(1_600_000_000);
    let p = env.p("doc.txt");
    fsop(move || std::fs::File::open(p).unwrap().set_modified(then).unwrap()).await;
    paused_while_dated(&env, &id).await;
    env.engine.resume();
    env.sync().await;
    assert_eq!(env.mock.content("/doc.txt").unwrap(), b"second");
    let online = env.engine.graph.item(&id).await.unwrap();
    assert_eq!(online.modified().unwrap().timestamp(), 1_600_000_000);
    assert_eq!(status_of(env.p("doc.txt")).await, "local");

    // Saved again while paused there.
    write_closed(&env, "doc.txt", b"third").await;
    paused_while_dated(&env, &id).await;
    write_closed(&env, "doc.txt", b"fourth").await;
    env.engine.resume();
    env.sync().await;
    assert_eq!(env.mock.list("/"), vec!["doc.txt"]);
    assert_eq!(env.mock.content("/doc.txt").unwrap(), b"fourth");
    assert_eq!(list(env.mnt.clone()).await, vec!["doc.txt"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn what_an_upload_sent_is_remembered_while_its_new_folder_cannot_be_made() {
    let env = Env::new().await;
    env.mock.add_file("/", "doc.txt", b"first");
    env.sync().await;
    assert_eq!(read(env.p("doc.txt")).await.unwrap(), b"first");
    let id = env.mock.id("/doc.txt").unwrap();
    write_closed(&env, "doc.txt", b"second").await;
    paused_while_dated(&env, &id).await;
    // Meanwhile moved into a new folder and saved again there ...
    let (dir, a, b) = (env.p("Archive"), env.p("doc.txt"), env.p("Archive/doc.txt"));
    fsop(move || {
        std::fs::create_dir(dir).unwrap();
        std::fs::rename(a, b).unwrap();
    })
    .await;
    write_closed(&env, "Archive/doc.txt", b"third").await;
    // ... which can't be made online at first.
    env.mock.with(|d| d.fail_next = Some((403, 1)));
    env.engine.resume();
    env.sync().await;

    assert_eq!(env.mock.list("/"), vec!["Archive"]);
    assert_eq!(env.mock.list("/Archive"), vec!["doc.txt"]);
    assert_eq!(env.mock.id("/Archive/doc.txt").unwrap(), id);
    assert_eq!(env.mock.content("/Archive/doc.txt").unwrap(), b"third");
    assert_eq!(list(env.p("Archive")).await, vec!["doc.txt"]);
}

/// Resume, and save `theirs` over the file at `path` online, as another
/// PC would, while the pass gives the content of ours it finds there the
/// local date.
async fn saved_over_while_dated(env: &Env, path: &str, theirs: &[u8]) {
    env.mock.with(|d| {
        d.hold = Some("PATCH /v1.0/me/drive/items/".into());
        d.held = 0;
    });
    env.engine.resume();
    let engine = env.engine.clone();
    let pass = tokio::spawn(async move { engine.cycle().await });
    eventually("the pass to date the file", || {
        env.mock.with(|d| d.held > 0)
    })
    .await;
    env.mock.modify(path, theirs);
    release(env);
    pass.await.unwrap();
    env.sync().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_version_saved_online_while_ours_is_dated_comes_down() {
    let env = Env::new().await;
    env.mock.add_file("/", "doc.txt", b"first");
    env.sync().await;
    assert_eq!(read(env.p("doc.txt")).await.unwrap(), b"first");
    let id = env.mock.id("/doc.txt").unwrap();
    let then = std::time::UNIX_EPOCH + Duration::from_secs(1_600_000_000);
    let date = |p: PathBuf| fsop(move || std::fs::File::open(p).unwrap().set_modified(then));

    // A new version, found online by the next attempt (HTTP 412) ...
    write_closed(&env, "doc.txt", b"second").await;
    date(env.p("doc.txt")).await.unwrap();
    paused_while_dated(&env, &id).await;
    saved_over_while_dated(&env, "/doc.txt", b"theirs").await;
    assert_eq!(read(env.p("doc.txt")).await.unwrap(), b"theirs");
    assert_eq!(env.mock.content("/doc.txt").unwrap(), b"theirs");
    assert_eq!(list(env.mnt.clone()).await, vec!["doc.txt"]);
    let online = env.engine.graph.item(&id).await.unwrap();
    assert_ne!(online.modified().unwrap().timestamp(), 1_600_000_000);

    // ... and a new file (HTTP 409); it has no id here yet.
    write_closed(&env, "new.txt", b"ours").await;
    date(env.p("new.txt")).await.unwrap();
    paused_while_dated(&env, "").await;
    saved_over_while_dated(&env, "/new.txt", b"theirs").await;
    assert_eq!(read(env.p("new.txt")).await.unwrap(), b"theirs");
    assert_eq!(env.mock.content("/new.txt").unwrap(), b"theirs");
    assert_eq!(list(env.mnt.clone()).await, vec!["doc.txt", "new.txt"]);
    let id = env.mock.id("/new.txt").unwrap();
    let online = env.engine.graph.item(&id).await.unwrap();
    assert_ne!(online.modified().unwrap().timestamp(), 1_600_000_000);
}

// ----- cloud thumbnails ---------------------------------------------------------

use axentra_onedrive_core::thumbs::{self, Bucket, Original};

/// Run `f` on a thread with this name, which is what the mount sees as the
/// program (GLib's worker threads are "pool-<program>").
async fn as_program<T: Send + 'static>(name: &str, f: impl FnOnce() -> T + Send + 'static) -> T {
    let name = name.to_string();
    fsop(move || {
        std::thread::Builder::new()
            .name(name)
            .spawn(f)
            .unwrap()
            .join()
            .unwrap()
    })
    .await
}

fn thumb_of(env: &Env, rel: &str, size: Bucket) -> PathBuf {
    let uri = thumbs::file_uri(&env.p(rel));
    thumbs::path_in(
        &env.engine.paths.thumbnail_dir,
        size,
        &thumbs::cache_name(&uri),
    )
}

/// Whether the thumbnail is one GIO would use for the file as the mount shows it.
async fn thumb_is_valid(env: &Env, rel: &str, size: Bucket) -> bool {
    let file = env.p(rel);
    let meta = fsop(move || std::fs::metadata(file).unwrap()).await;
    let uri = thumbs::file_uri(&env.p(rel));
    thumbs::is_valid(
        &thumb_of(env, rel, size),
        &Original {
            uri: &uri,
            mtime: meta.mtime(),
            size: meta.len(),
        },
    )
}

async fn eventually(what: &str, cond: impl Fn() -> bool) {
    for _ in 0..200 {
        if cond() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for {what}");
}

fn thumbnail_requests(env: &Env, rel: &str) -> usize {
    let id = env.mock.id(rel).unwrap();
    env.mock.requests(&format!("/items/{id}/thumbnails"))
}

async fn photo_env() -> Env {
    let env = Env::new().await;
    env.mock.add_folder("/", "Photos");
    for name in [
        "IMG_0001.JPG",
        "IMG_0002.jpg",
        "clip.mov",
        "kept.jpg",
        "seen.jpg",
    ] {
        env.mock.add_file("/Photos", name, &vec![7u8; 50_000]);
    }
    env.mock.add_file("/Photos", "IMG_0001.AAE", b"<plist/>");
    env.mock.add_file("/Photos", "notes.txt", b"hello");
    env.sync().await;
    env
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn listing_a_photo_folder_fills_the_thumbnail_cache_without_downloading() {
    let env = photo_env().await;
    // On this device already: left to the system's thumbnailers.
    assert_eq!(read(env.p("Photos/seen.jpg")).await.unwrap().len(), 50_000);
    env.engine
        .set_pinned(env.ino("/Photos/kept.jpg"), true)
        .unwrap();
    eventually("the pinned photo to download", || {
        env.engine
            .db
            .with(|s| s.pinned_uncached())
            .unwrap()
            .is_empty()
    })
    .await;
    let downloads = env.mock.downloads();
    let used = env.engine.local_usage();

    // A script or `find` walking the tree fetches nothing.
    assert_eq!(list(env.p("Photos")).await.len(), 7);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(env.mock.requests("/thumbnails"), 0);

    // Watch the folder like a file manager does.
    let photos = env.p("Photos");
    let watch = fsop(move || {
        let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK) };
        let p = CString::new(photos.as_os_str().as_bytes()).unwrap();
        assert!(unsafe { libc::inotify_add_watch(fd, p.as_ptr(), libc::IN_ATTRIB) } >= 0);
        fd
    })
    .await;

    // Nautilus lists folders on GLib worker threads.
    let photos = env.p("Photos");
    assert_eq!(
        as_program("pool-org.gnome.", move || names(&photos))
            .await
            .len(),
        7
    );
    for rel in [
        "Photos/IMG_0001.JPG",
        "Photos/IMG_0002.jpg",
        "Photos/clip.mov",
    ] {
        let (normal, large) = (
            thumb_of(&env, rel, Bucket::Normal),
            thumb_of(&env, rel, Bucket::Large),
        );
        eventually(rel, || normal.exists() && large.exists()).await;
        assert!(thumb_is_valid(&env, rel, Bucket::Large).await, "{rel}");
        assert!(thumb_is_valid(&env, rel, Bucket::Normal).await, "{rel}");
        // Mock photos are 4:3.
        assert_eq!(image::image_dimensions(&large).unwrap(), (256, 192));
        assert_eq!(image::image_dimensions(&normal).unwrap(), (128, 96));
    }
    for rel in [
        "/Photos/kept.jpg",
        "/Photos/seen.jpg",
        "/Photos/notes.txt",
        "/Photos/IMG_0001.AAE",
    ] {
        assert_eq!(thumbnail_requests(&env, rel), 0, "{rel}");
    }
    assert_eq!(env.mock.downloads(), downloads, "no photo was downloaded");
    assert_eq!(
        env.engine.local_usage(),
        used,
        "\"on this device\" didn't change"
    );
    assert_eq!(status_of(env.p("Photos/IMG_0001.JPG")).await, "online");

    // Each new thumbnail was announced, so open windows show it (just after
    // it's written).
    let events = std::cell::RefCell::new(String::new());
    eventually("a change notification for each photo", || {
        let mut buf = [0u8; 4096];
        let n = unsafe { libc::read(watch, buf.as_mut_ptr().cast(), buf.len()) };
        let mut events = events.borrow_mut();
        events.push_str(&String::from_utf8_lossy(&buf[..n.max(0) as usize]));
        ["IMG_0001.JPG", "IMG_0002.jpg", "clip.mov"]
            .iter()
            .all(|name| events.contains(name))
    })
    .await;
    unsafe { libc::close(watch) };

    // GIO itself finds and accepts them, as file managers and choosers ask it.
    if std::process::Command::new("gio")
        .arg("--version")
        .output()
        .is_ok()
    {
        let file = env.p("Photos/IMG_0002.jpg");
        let cache_home = env
            .engine
            .paths
            .thumbnail_dir
            .parent()
            .unwrap()
            .to_path_buf();
        let out = fsop(move || {
            std::process::Command::new("gio")
                .args(["info", "-a", "thumbnail::path,thumbnail::is-valid"])
                .arg(&file)
                .env("XDG_CACHE_HOME", cache_home)
                .output()
                .unwrap()
        })
        .await;
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("thumbnail::is-valid: TRUE"), "{text}");
        assert!(
            text.contains(&*thumb_of(&env, "Photos/IMG_0002.jpg", Bucket::Large).to_string_lossy()),
            "{text}"
        );
    }

    // Listing again soon doesn't fetch again.
    let before = env.mock.requests("/thumbnails");
    let photos = env.p("Photos");
    as_program("pool-org.gnome.", move || names(&photos)).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(env.mock.requests("/thumbnails"), before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn thumbnailers_get_the_cloud_thumbnail_instead_of_the_file() {
    let env = photo_env().await;
    let photo = env.p("Photos/IMG_0001.JPG");
    let err = as_program("x-thumbnailer", move || std::fs::read(photo).unwrap_err()).await;
    assert_eq!(err.raw_os_error(), Some(libc::EACCES));
    assert!(thumb_is_valid(&env, "Photos/IMG_0001.JPG", Bucket::Large).await);
    // Indexers want the file, not a picture of it: refused, nothing fetched.
    let photo = env.p("Photos/IMG_0002.jpg");
    let err = as_program("tracker-extract", move || std::fs::read(photo).unwrap_err()).await;
    assert_eq!(err.raw_os_error(), Some(libc::EACCES));
    assert_eq!(thumbnail_requests(&env, "/Photos/IMG_0002.jpg"), 0);

    // The real GNOME thumbnailer, asked for a 512 px thumbnail.
    if std::path::Path::new("/usr/bin/gdk-pixbuf-thumbnailer").exists() {
        let uri = thumbs::file_uri(&env.p("Photos/clip.mov"));
        let out = env._dir.path().join("out.png");
        let o = out.clone();
        let status = fsop(move || {
            std::process::Command::new("/usr/bin/gdk-pixbuf-thumbnailer")
                .args(["-s", "512", &uri])
                .arg(&o)
                .output()
                .unwrap()
                .status
        })
        .await;
        assert!(!status.success() && !out.exists(), "it read the file");
        let xl = thumb_of(&env, "Photos/clip.mov", Bucket::XLarge);
        assert!(thumb_is_valid(&env, "Photos/clip.mov", Bucket::XLarge).await);
        assert_eq!(image::image_dimensions(&xl).unwrap(), (512, 384));
    }
    // A thumbnailer whose helper does the reading (like `xapp-raw-thumbnailer`
    // running `dcraw`), asked for 128 px.
    let script = env._dir.path().join("raw-thumbnailer");
    std::fs::write(
        &script,
        "#!/bin/sh\ncat \"$3\" >/dev/null 2>&1\necho $? > \"$4\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let (photo, result) = (
        env.p("Photos/IMG_0002.jpg"),
        env._dir.path().join("cat-status"),
    );
    let r = result.clone();
    fsop(move || {
        for _ in 0..200 {
            let run = std::process::Command::new(&script)
                .args(["-s", "128"])
                .arg(&photo)
                .arg(&r)
                .status();
            match run {
                // A program another test started while the script was
                // being written holds it open for writing (as it does every
                // file open here) until it starts running itself.
                Err(e) if e.raw_os_error() == Some(libc::ETXTBSY) => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                run => return run.unwrap(),
            }
        }
        panic!("the script stayed busy");
    })
    .await;
    assert_ne!(
        std::fs::read_to_string(&result).unwrap().trim(),
        "0",
        "the helper read the photo"
    );
    assert!(thumb_is_valid(&env, "Photos/IMG_0002.jpg", Bucket::Normal).await);
    assert!(!thumb_of(&env, "Photos/IMG_0002.jpg", Bucket::Large).exists());
    assert_eq!(env.mock.downloads(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_file_chooser_portal_is_shown_thumbnails_but_can_still_read_other_files() {
    let env = photo_env().await;
    let photo = env.p("Photos/IMG_0002.jpg");
    let err = as_program("pool-xdg-deskto", move || std::fs::read(photo).unwrap_err()).await;
    assert_eq!(err.raw_os_error(), Some(libc::EACCES));
    assert!(thumb_is_valid(&env, "Photos/IMG_0002.jpg", Bucket::Large).await);
    assert_eq!(env.mock.downloads(), 0);

    // No thumbnail possible: the portal gets the file.
    let notes = env.p("Photos/notes.txt");
    let data = as_program("pool-xdg-deskto", move || std::fs::read(notes).unwrap()).await;
    assert_eq!(data, b"hello");
    assert_eq!(env.mock.downloads(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cloud_thumbnails_go_with_their_items() {
    let env = photo_env().await;
    let get = |rel: &str| {
        let ino = env.ino(rel);
        let engine = env.engine.clone();
        async move { engine.thumbnail(ino, Bucket::Normal).await.unwrap() }
    };
    for rel in [
        "/Photos/IMG_0001.JPG",
        "/Photos/IMG_0002.jpg",
        "/Photos/clip.mov",
        "/Photos/kept.jpg",
    ] {
        assert!(get(rel).await.is_some(), "{rel}");
    }
    let old = |rel: &str| thumb_of(&env, rel, Bucket::Normal);
    let (changed, renamed, deleted, moved) = (
        old("Photos/IMG_0001.JPG"),
        old("Photos/IMG_0002.jpg"),
        old("Photos/clip.mov"),
        old("Photos/kept.jpg"),
    );

    // Renamed here: the old path's thumbnail goes.
    let (a, b) = (env.p("Photos/IMG_0002.jpg"), env.p("Photos/beach.jpg"));
    fsop(move || std::fs::rename(a, b).unwrap()).await;
    assert!(!renamed.exists());
    // Changed, deleted or moved online.
    env.mock.modify("/Photos/IMG_0001.JPG", b"new picture");
    env.mock.delete("/Photos/clip.mov");
    env.mock.add_folder("/", "Album");
    env.mock.move_to("/Photos/kept.jpg", "/Album");
    env.sync().await;
    assert!(!changed.exists());
    assert!(!deleted.exists());
    assert!(!moved.exists());
    let rows: i64 = env
        .engine
        .db
        .with(|s| {
            s.0.query_row("SELECT COUNT(*) FROM thumbs", [], |r| r.get(0))
        })
        .unwrap();
    assert_eq!(rows, 0);

    // Fetched again on request, for the new name and content.
    assert!(get("/Photos/beach.jpg").await.is_some());
    assert!(thumb_is_valid(&env, "Photos/beach.jpg", Bucket::Normal).await);
    assert!(get("/Photos/IMG_0001.JPG").await.is_some());
    assert!(thumb_is_valid(&env, "Photos/IMG_0001.JPG", Bucket::Normal).await);
    // A file OneDrive has no thumbnail of.
    assert!(get("/Photos/notes.txt").await.is_none());
    // Downloaded here: no cloud thumbnail is made for it.
    assert_eq!(read(env.p("Photos/seen.jpg")).await.unwrap().len(), 50_000);
    assert!(get("/Photos/seen.jpg").await.is_none());
    assert_eq!(thumbnail_requests(&env, "/Photos/seen.jpg"), 0);

    // Unlinking this PC removes them all.
    let beach = thumb_of(&env, "Photos/beach.jpg", Bucket::Normal);
    env.engine.sign_out().await.unwrap();
    assert!(!beach.exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn thumbnails_are_scaled_here_when_onedrive_has_no_custom_sizes() {
    let env = photo_env().await;
    env.mock.with(|d| d.no_custom_thumbnails = true);
    let ino = env.ino("/Photos/IMG_0001.JPG");
    let p = env
        .engine
        .thumbnail(ino, Bucket::Large)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(image::image_dimensions(&p).unwrap(), (256, 192));
    assert!(env.mock.requests("/thumbnails/0/large/content") == 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn overwriting_an_online_file_does_not_download_it() {
    let env = Env::new().await;
    env.mock.add_file("/", "old.pdf", &vec![1u8; 2_000_000]);
    env.sync().await;
    let id = env.mock.id("/old.pdf").unwrap();

    // Even offline, where downloading it first would fail.
    let mut cfg = env.engine.cfg();
    let online = std::mem::replace(&mut cfg.graph_base, "http://127.0.0.1:9".into());
    env.engine.set_config(cfg);
    write(env.p("old.pdf"), b"new content").await;
    assert_eq!(read(env.p("old.pdf")).await.unwrap(), b"new content");
    assert_eq!(status_of(env.p("old.pdf")).await, "syncing");

    let mut cfg = env.engine.cfg();
    cfg.graph_base = online;
    env.engine.set_config(cfg);
    env.sync().await;
    assert_eq!(env.mock.content("/old.pdf").unwrap(), b"new content");
    assert_eq!(env.mock.id("/old.pdf").unwrap(), id);
    assert_eq!(env.mock.version_count("/old.pdf"), 2);
    assert_eq!(env.mock.downloads(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_file_overwritten_while_it_downloads_keeps_the_new_content() {
    use std::io::Write;
    let env = Env::new().await;
    env.mock.add_file("/", "x.pdf", b"old content");
    env.sync().await;
    let id = env.mock.id("/x.pdf").unwrap();
    let ino = env.ino("/x.pdf");
    env.mock.with(|d| {
        d.hold = Some(format!("GET /v1.0/me/drive/items/{id}/content"));
        d.held = 0;
    });
    env.engine.set_pinned(ino, true).unwrap();
    eventually("the download to start", || env.mock.with(|d| d.held > 0)).await;

    // Overwritten (`cp`, an in-place save) while that download is under way,
    // and still being written once it has finished.
    let p = env.p("x.pdf");
    let mut file = fsop(move || {
        let mut f = std::fs::File::create(p).unwrap();
        f.write_all(b"new").unwrap();
        f
    })
    .await;
    env.mock.with(|d| d.hold = None);
    let part = env.engine.cache_path(ino).with_extension("part");
    eventually("the download to end", || {
        env.engine.status().downloading == 0 && !part.exists()
    })
    .await;
    fsop(move || file.write_all(b" content").unwrap()).await;

    assert_eq!(read(env.p("x.pdf")).await.unwrap(), b"new content");
    env.sync().await;
    assert_eq!(env.mock.content("/x.pdf").unwrap(), b"new content");
    assert_eq!(read(env.p("x.pdf")).await.unwrap(), b"new content");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_overwrite_that_cannot_be_saved_leaves_the_file_online() {
    if unsafe { libc::geteuid() } == 0 {
        return; // root writes anywhere
    }
    let env = Env::new().await;
    env.mock.add_file("/", "old.pdf", b"old content");
    env.sync().await;

    // No room for the local copy (here: no permission to make it).
    let cache = env.engine.cache_path(ROOT).parent().unwrap().to_path_buf();
    let before = std::fs::metadata(&cache).unwrap().permissions();
    let mut read_only = before.clone();
    read_only.set_readonly(true);
    std::fs::set_permissions(&cache, read_only).unwrap();
    let p = env.p("old.pdf");
    let r = fsop(move || std::fs::write(p, b"new content")).await;
    std::fs::set_permissions(&cache, before).unwrap();
    assert!(r.is_err());

    // Still online-only, and still opens.
    assert_eq!(status_of(env.p("old.pdf")).await, "online");
    assert_eq!(read(env.p("old.pdf")).await.unwrap(), b"old content");
    env.sync().await;
    assert_eq!(env.mock.version_count("/old.pdf"), 1);
    assert_eq!(status_of(env.p("old.pdf")).await, "local");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn edits_are_recorded_while_the_file_is_still_open() {
    let env = Env::new().await;
    env.mock.add_file("/", "library.db", b"one");
    env.sync().await;
    assert_eq!(read(env.p("library.db")).await.unwrap(), b"one");
    let ino = env.ino("/library.db");

    // An app that keeps its database open.
    let p = env.p("library.db");
    let file = fsop(move || {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().append(true).open(p).unwrap();
        f.write_all(b" two").unwrap();
        f
    })
    .await;
    // Queued already, so the edit isn't lost if the service stops first.
    let row = env.engine.db.with(|s| s.get(ino)).unwrap().unwrap();
    assert!(row.dirty && row.cached, "{row:?}");
    assert_eq!(status_of(env.p("library.db")).await, "syncing");
    env.engine.cycle().await;
    assert_eq!(
        env.mock.content("/library.db").unwrap(),
        b"one",
        "uploaded while still being written"
    );

    fsop(move || drop(file)).await;
    env.sync().await;
    assert_eq!(env.mock.content("/library.db").unwrap(), b"one two");
    assert_eq!(status_of(env.p("library.db")).await, "local");
}

fn rename_noreplace(from: &Path, to: &Path) -> i32 {
    let (a, b) = (
        CString::new(from.as_os_str().as_bytes()).unwrap(),
        CString::new(to.as_os_str().as_bytes()).unwrap(),
    );
    unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            a.as_ptr(),
            libc::AT_FDCWD,
            b.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn renaming_only_the_case_of_a_name() {
    let env = Env::new().await;
    env.mock.add_file("/", "report.pdf", b"x");
    env.mock.add_folder("/", "photos");
    env.sync().await;
    let id = env.mock.id("/report.pdf").unwrap();

    let (a, b) = (env.p("report.pdf"), env.p("Report.pdf"));
    fsop(move || std::fs::rename(a, b).unwrap()).await;
    // What `mv` does first.
    let (a, b) = (env.p("photos"), env.p("Photos"));
    assert_eq!(fsop(move || rename_noreplace(&a, &b)).await, 0);
    assert_eq!(list(env.mnt.clone()).await, vec!["Photos", "Report.pdf"]);
    env.sync().await;
    assert_eq!(env.mock.list("/"), vec!["Photos", "Report.pdf"]);
    assert_eq!(env.mock.id("/Report.pdf").unwrap(), id);

    // Names still can't differ only in case, as on OneDrive.
    let dup = env.p("REPORT.pdf");
    let err = fsop(move || std::fs::write(dup, b"y").unwrap_err()).await;
    assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
    // Nor does a rename replace what differs only in case: programs that
    // ask before replacing a file (Files, `mv -i`) can't see it there.
    write_closed(&env, "notes.txt", b"n").await;
    for (from, to) in [("notes.txt", "report.PDF"), ("Report.pdf", "photos")] {
        let (a, b) = (env.p(from), env.p(to));
        let err = fsop(move || std::fs::rename(a, b).unwrap_err()).await;
        assert_eq!(err.raw_os_error(), Some(libc::EEXIST), "{from} to {to}");
    }
    assert_eq!(
        list(env.mnt.clone()).await,
        vec!["Photos", "Report.pdf", "notes.txt"]
    );
    env.sync().await;
    assert_eq!(env.mock.id("/Report.pdf").unwrap(), id);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn big_folders_list_fully_even_while_being_emptied() {
    let env = Env::new().await;
    env.mock.add_folder("/", "Camera Roll");
    for i in 0..1500 {
        env.mock
            .add_file("/Camera Roll", &format!("IMG_{i:04}.jpg"), b"x");
    }
    env.sync().await;
    assert_eq!(list(env.p("Camera Roll")).await.len(), 1500);

    // A recursive delete removes entries while it's still reading the folder.
    let dir = env.p("Camera Roll");
    let removed = fsop(move || {
        let mut n = 0;
        for e in std::fs::read_dir(&dir).unwrap() {
            std::fs::remove_file(e.unwrap().path()).unwrap();
            n += 1;
        }
        std::fs::remove_dir(&dir).unwrap();
        n
    })
    .await;
    assert_eq!(removed, 1500);
    env.sync().await;
    assert!(!env.mock.exists("/Camera Roll"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_folder_that_starts_syncing_keeps_bad_names_inside_local() {
    let env = Env::new().await;
    env.sync().await;
    let dir = env.p("Q3: drafts/aux");
    fsop(move || std::fs::create_dir_all(dir).unwrap()).await;
    for rel in [
        "Q3: drafts/budget.ods",
        "Q3: drafts/.~lock.budget.ods#",
        "Q3: drafts/notes?.txt",
        "Q3: drafts/aux/inside.txt",
    ] {
        write(env.p(rel), b"x").await;
    }
    assert_eq!(
        status_of(env.p("Q3: drafts/budget.ods")).await,
        "local-only"
    );

    let (a, b) = (env.p("Q3: drafts"), env.p("Q3 drafts"));
    fsop(move || std::fs::rename(a, b).unwrap()).await;
    env.settle().await;
    env.engine.cycle().await; // reports the invalid names, which is the point
    assert_eq!(env.mock.list("/Q3 drafts"), vec!["budget.ods"]);
    for (rel, status) in [
        ("Q3 drafts", "local"),
        ("Q3 drafts/budget.ods", "local"),
        ("Q3 drafts/.~lock.budget.ods#", "local-only"),
        ("Q3 drafts/notes?.txt", "error"),
        ("Q3 drafts/aux", "error"),
        ("Q3 drafts/aux/inside.txt", "local-only"),
    ] {
        assert_eq!(status_of(env.p(rel)).await, status, "{rel}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn folders_moved_into_an_always_kept_folder_are_kept_too() {
    let env = Env::new().await;
    env.mock.add_folder("/", "Offline");
    env.mock.add_folder("/", "Trip");
    env.mock.add_folder("/Trip", "Day 1");
    env.mock.add_file("/Trip/Day 1", "a.jpg", b"a");
    env.mock.add_folder("/", "Later");
    env.mock.add_file("/Later", "b.jpg", b"b");
    env.sync().await;
    env.engine.set_pinned(env.ino("/Offline"), true).unwrap();
    let all_local = || {
        env.engine
            .db
            .with(|s| s.pinned_uncached())
            .unwrap()
            .is_empty()
    };

    // Moved here...
    let (a, b) = (env.p("Trip"), env.p("Offline/Trip"));
    fsop(move || std::fs::rename(a, b).unwrap()).await;
    env.sync().await;
    eventually("the moved folder to download", all_local).await;
    // ...and online.
    env.mock.move_to("/Later", "/Offline");
    env.sync().await;
    eventually("the folder moved online to download", all_local).await;
    for rel in [
        "Offline/Trip",
        "Offline/Trip/Day 1",
        "Offline/Trip/Day 1/a.jpg",
        "Offline/Later",
        "Offline/Later/b.jpg",
    ] {
        assert_eq!(status_of(env.p(rel)).await, "pinned", "{rel}");
    }
}

fn xattr_names(path: &Path) -> Vec<String> {
    let p = CString::new(path.as_os_str().as_bytes()).unwrap();
    let mut buf = vec![0u8; 1024];
    let n = unsafe { libc::listxattr(p.as_ptr(), buf.as_mut_ptr().cast(), buf.len()) };
    assert!(n >= 0, "listxattr failed");
    buf.truncate(n as usize);
    buf.split(|b| *b == 0)
        .filter(|n| !n.is_empty())
        .map(|n| String::from_utf8_lossy(n).into_owned())
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_listed_attribute_can_be_read() {
    let env = Env::new().await;
    env.mock.add_file("/", "synced.txt", b"x");
    env.sync().await;
    write(env.p("new.txt"), b"y").await;
    for (rel, expected) in [
        (
            "synced.txt",
            vec!["user.axentra.onedrive.status", "user.axentra.onedrive.url"],
        ),
        ("new.txt", vec!["user.axentra.onedrive.status"]),
    ] {
        let p = env.p(rel);
        let (names, values) = fsop(move || {
            let names = xattr_names(&p);
            let values: Vec<_> = names.iter().map(|n| xattr(&p, n)).collect();
            (names, values)
        })
        .await;
        // `rsync -X` fails on a listed name it can't read; "pinned" isn't
        // listed so copies don't carry it.
        assert_eq!(names, expected, "{rel}");
        assert!(values.iter().all(Option::is_some), "{rel}: {values:?}");
    }
    let p = env.p("synced.txt");
    assert_eq!(
        fsop(move || xattr(&p, "user.axentra.onedrive.pinned")).await,
        Some("0".into())
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn counting_what_is_in_folders_fetches_nothing_until_one_is_opened() {
    let env = Env::new().await;
    env.mock.add_folder("/", "Pictures");
    env.mock.add_file("/Pictures", "a.jpg", &[7u8; 1000]);
    env.mock.add_folder("/Pictures", "2023");
    env.mock.add_file("/Pictures/2023", "b.jpg", &[7u8; 1000]);
    env.mock.add_folder("/Pictures/2023", "Trip");
    env.mock
        .add_file("/Pictures/2023/Trip", "c.jpg", &[7u8; 1000]);
    env.mock.add_folder("/Pictures", "2024");
    env.mock.add_file("/Pictures/2024", "d.jpg", &[7u8; 1000]);
    env.sync().await;
    let untouched = |rels: &[&str]| {
        for rel in rels {
            assert_eq!(thumbnail_requests(&env, rel), 0, "{rel}");
        }
    };

    // Nautilus shows Pictures, then counts what's in each folder there (and
    // Properties goes on through the whole tree), on the same threads.
    let p = env.p("Pictures");
    as_program("pool-org.gnome.", move || {
        for d in ["", "2023", "2024", "2023/Trip"] {
            names(&p.join(d));
        }
    })
    .await;
    let a = thumb_of(&env, "Pictures/a.jpg", Bucket::Large);
    eventually("the shown folder's photo", || a.exists()).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    untouched(&[
        "/Pictures/2023/b.jpg",
        "/Pictures/2023/Trip/c.jpg",
        "/Pictures/2024/d.jpg",
    ]);

    // Pictures is refreshed and its folders counted again; 2023 is opened
    // straight after, while Nautilus is still busy, and the folder in it counted.
    let p = env.p("Pictures");
    as_program("pool-org.gnome.", move || {
        for d in ["", "2023", "2024", "2023", "2023/Trip"] {
            names(&p.join(d));
        }
    })
    .await;
    let b = thumb_of(&env, "Pictures/2023/b.jpg", Bucket::Large);
    eventually("the opened folder's photo", || b.exists()).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    untouched(&["/Pictures/2023/Trip/c.jpg", "/Pictures/2024/d.jpg"]);

    // A moment later 2024 is opened.
    env.engine.age_thumbnail_state(Duration::from_millis(1200));
    let p = env.p("Pictures/2024");
    as_program("pool-org.gnome.", move || names(&p)).await;
    let d = thumb_of(&env, "Pictures/2024/d.jpg", Bucket::Large);
    eventually("the folder opened later", || d.exists()).await;
    untouched(&["/Pictures/2023/Trip/c.jpg"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_thumbnail_is_finished_when_its_caller_stops_waiting() {
    let env = photo_env().await;
    let ino = env.ino("/Photos/IMG_0001.JPG");
    // Like a thumbnailer's open or a D-Bus call that timed out.
    let _ = tokio::time::timeout(Duration::ZERO, env.engine.thumbnail(ino, Bucket::Large)).await;
    eventually("the thumbnail to be recorded", || {
        let row = env.engine.db.with(|s| s.thumb(ino)).unwrap();
        row.is_some_and(|t| t.sizes & Bucket::Large.bit() != 0)
    })
    .await;
    assert!(thumb_is_valid(&env, "Photos/IMG_0001.JPG", Bucket::Large).await);
    let partial: Vec<_> = std::fs::read_dir(env.engine.paths.thumbnail_dir.join("large"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with('.'))
        .collect();
    assert!(partial.is_empty(), "{partial:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_preview_onedrive_makes_later_is_fetched_when_asked_again() {
    let env = photo_env().await;
    // Just uploaded: OneDrive hasn't made previews of them yet.
    env.mock.with(|d| d.no_thumbnails_yet = true);
    let (one, two) = (
        env.ino("/Photos/IMG_0001.JPG"),
        env.ino("/Photos/IMG_0002.jpg"),
    );
    let get = || {
        let engine = env.engine.clone();
        async move { engine.thumbnail(one, Bucket::Large).await.unwrap() }
    };
    let list = || {
        let photos = env.p("Photos");
        as_program("pool-org.gnome.", move || names(&photos))
    };
    assert!(get().await.is_none());
    list().await;
    eventually("the listed photo to be looked up", || {
        env.engine.db.with(|s| s.thumb(two)).unwrap().is_some()
    })
    .await;
    // A thumbnailer gave up on it meanwhile.
    let uri = thumbs::file_uri(&env.p("Photos/IMG_0001.JPG"));
    let failed = env
        .engine
        .paths
        .thumbnail_dir
        .join("fail/gnome-thumbnail-factory")
        .join(format!("{}.png", thumbs::cache_name(&uri)));
    std::fs::create_dir_all(failed.parent().unwrap()).unwrap();
    std::fs::write(&failed, b"").unwrap();

    // OneDrive has made them now, but isn't asked again right away.
    env.mock.with(|d| d.no_thumbnails_yet = false);
    assert!(get().await.is_none());
    env.engine.age_thumbnail_state(Duration::from_secs(61));
    list().await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(thumbnail_requests(&env, "/Photos/IMG_0001.JPG"), 1);
    assert_eq!(thumbnail_requests(&env, "/Photos/IMG_0002.jpg"), 1);

    // A couple of minutes later it is.
    env.engine.age_thumbnail_state(Duration::from_secs(61));
    let p = get().await.expect("asked again");
    assert_eq!(p, thumb_of(&env, "Photos/IMG_0001.JPG", Bucket::Large));
    assert!(thumb_is_valid(&env, "Photos/IMG_0001.JPG", Bucket::Large).await);
    let row = env.engine.db.with(|s| s.thumb(one)).unwrap().unwrap();
    assert_ne!(row.sizes & Bucket::Large.bit(), 0);
    assert!(!failed.exists(), "GNOME would still not show it");
    list().await;
    let listed = thumb_of(&env, "Photos/IMG_0002.jpg", Bucket::Large);
    eventually("the listed photo's thumbnail", || listed.exists()).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_big_folder_moves_on_to_the_photos_still_without_a_thumbnail() {
    let env = Env::new().await;
    env.mock.add_folder("/", "Camera");
    for i in 1..=1001 {
        env.mock
            .add_file("/Camera", &format!("IMG_{i:04}.jpg"), &[7u8; 100]);
    }
    env.sync().await;
    // The first thousand by name have thumbnails already.
    let camera = env.p("Camera");
    let done = fsop(move || {
        (1..=1000)
            .map(|i| {
                let p = camera.join(format!("IMG_{i:04}.jpg"));
                let m = std::fs::metadata(&p).unwrap();
                (thumbs::file_uri(&p), m.mtime(), m.len())
            })
            .collect::<Vec<_>>()
    })
    .await;
    let img = image::DynamicImage::ImageRgb8(image::RgbImage::new(4, 3));
    for (uri, mtime, size) in &done {
        let of = Original {
            uri,
            mtime: *mtime,
            size: *size,
        };
        thumbs::write(&env.engine.paths.thumbnail_dir, Bucket::Large, &of, &img).unwrap();
    }

    let camera = env.p("Camera");
    as_program("pool-org.gnome.", move || names(&camera)).await;
    let last = thumb_of(&env, "Camera/IMG_1001.jpg", Bucket::Large);
    eventually("the 1001st photo's thumbnail", || last.exists()).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(thumbnail_requests(&env, "/Camera/IMG_1001.jpg"), 1);
    assert_eq!(env.mock.requests("/thumbnails"), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_folder_opened_right_after_its_parent_follows_its_first_photo() {
    let env = Env::new().await;
    env.mock.add_folder("/", "Pictures");
    env.mock.add_file("/Pictures", "a.jpg", &[7u8; 1000]);
    env.mock.add_folder("/Pictures", "2024");
    for name in ["d1.jpg", "d2.jpg", "d3.jpg"] {
        env.mock.add_file("/Pictures/2024", name, &[7u8; 1000]);
    }
    env.sync().await;

    // Pictures is shown and 2024 opened straight after, with no counts in
    // between: that looks just like Nautilus counting it.
    let p = env.p("Pictures");
    as_program("pool-org.gnome.", move || {
        names(&p);
        names(&p.join("2024"));
    })
    .await;
    // It's on screen, so its first photo goes to a thumbnailer...
    let photo = env.p("Pictures/2024/d1.jpg");
    let err = as_program("x-thumbnailer", move || std::fs::read(photo).unwrap_err()).await;
    assert_eq!(err.raw_os_error(), Some(libc::EACCES));
    // ...and the rest of the folder follows without being asked for.
    for rel in ["Pictures/2024/d2.jpg", "Pictures/2024/d3.jpg"] {
        let t = thumb_of(&env, rel, Bucket::Large);
        eventually(rel, || t.exists()).await;
    }
    assert_eq!(env.mock.downloads(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn counting_the_onedrive_folder_from_home_fetches_nothing() {
    let env = Env::new().await;
    for name in ["r1.jpg", "r2.jpg"] {
        env.mock.add_file("/", name, &[7u8; 1000]);
    }
    env.sync().await;

    // Home in list view, or Properties, counts what's in the OneDrive folder.
    let root = env.mnt.clone();
    as_program("pool-org.gnome.", move || names(&root)).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(env.mock.requests("/thumbnails"), 0);

    // It was opened: once a thumbnailer is asked for one of its photos, the
    // rest follow.
    let photo = env.p("r1.jpg");
    as_program("x-thumbnailer", move || std::fs::read(photo).unwrap_err()).await;
    let r2 = thumb_of(&env, "r2.jpg", Bucket::Large);
    eventually("the other photo's thumbnail", || r2.exists()).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn photos_in_the_onedrive_folder_files_gave_up_on_come_when_it_is_listed_again() {
    let env = Env::new().await;
    for name in ["r1.jpg", "r2.jpg", "r3.jpg", "r4.jpg"] {
        env.mock.add_file("/", name, &[7u8; 1000]);
    }
    env.sync().await;

    // Files showed the OneDrive folder just after r1 and r2 were uploaded,
    // before OneDrive had made previews of them (or while offline): the
    // thumbnailers it ran were turned away, and GNOME marked them failed.
    env.mock.with(|d| d.no_thumbnails_yet = true);
    let mut failed = vec![];
    for rel in ["r1.jpg", "r2.jpg"] {
        let photo = env.p(rel);
        let err = as_program("x-thumbnailer", move || std::fs::read(photo).unwrap_err()).await;
        assert_eq!(err.raw_os_error(), Some(libc::EACCES));
        let uri = thumbs::file_uri(&env.p(rel));
        let marker = env
            .engine
            .paths
            .thumbnail_dir
            .join("fail/gnome-thumbnail-factory")
            .join(format!("{}.png", thumbs::cache_name(&uri)));
        std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
        std::fs::write(&marker, b"").unwrap();
        failed.push(marker);
    }

    // A few minutes later the folder is shown again. GNOME doesn't ask for
    // photos it gave up on, and the listing looks like a count from Home;
    // those two are fetched all the same, and nothing else.
    env.mock.with(|d| d.no_thumbnails_yet = false);
    env.engine.age_thumbnail_state(Duration::from_secs(121));
    let root = env.mnt.clone();
    as_program("pool-org.gnome.", move || names(&root)).await;
    for (rel, marker) in ["r1.jpg", "r2.jpg"].into_iter().zip(&failed) {
        let t = thumb_of(&env, rel, Bucket::Large);
        eventually(rel, || t.exists() && !marker.exists()).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    for rel in ["/r3.jpg", "/r4.jpg"] {
        assert_eq!(thumbnail_requests(&env, rel), 0, "{rel}");
    }

    // It is on screen: once a thumbnailer is asked for another of its
    // photos, the rest follow.
    let photo = env.p("r3.jpg");
    as_program("x-thumbnailer", move || std::fs::read(photo).unwrap_err()).await;
    let r4 = thumb_of(&env, "r4.jpg", Bucket::Large);
    eventually("the photo not asked for", || r4.exists()).await;
    assert_eq!(env.mock.downloads(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_folder_renamed_and_shown_again_soon_gets_its_thumbnails_back() {
    let env = Env::new().await;
    env.mock.add_folder("/", "Trip");
    for name in ["a.jpg", "b.jpg", "c.jpg"] {
        env.mock.add_file("/Trip", name, &[7u8; 1000]);
    }
    env.sync().await;
    let show = |rel: &str| {
        let p = env.p(rel);
        as_program("pool-org.gnome.", move || names(&p))
    };
    show("Trip").await;
    for name in ["a.jpg", "b.jpg", "c.jpg"] {
        let t = thumb_of(&env, &format!("Trip/{name}"), Bucket::Large);
        eventually(name, || t.exists()).await;
    }

    // Renamed, which takes its thumbnails (they're named after the path),
    // and opened again within the minute.
    let (a, b) = (env.p("Trip"), env.p("Trip 2024"));
    fsop(move || std::fs::rename(a, b).unwrap()).await;
    assert!(!thumb_of(&env, "Trip/a.jpg", Bucket::Large).exists());
    show("Trip 2024").await;
    for name in ["a.jpg", "b.jpg", "c.jpg"] {
        let rel = format!("Trip 2024/{name}");
        let t = thumb_of(&env, &rel, Bucket::Large);
        eventually(&rel, || t.exists()).await;
        assert!(thumb_is_valid(&env, &rel, Bucket::Large).await, "{rel}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_thumbnail_arriving_after_unlinking_this_pc_is_not_kept() {
    let env = photo_env().await;
    let ino = env.ino("/Photos/IMG_0001.JPG");
    let id = env.mock.id("/Photos/IMG_0001.JPG").unwrap();
    env.mock.with(|d| {
        d.hold = Some(format!("/items/{id}/thumbnails"));
        d.held = 0;
    });
    let engine = env.engine.clone();
    let fetch = tokio::spawn(async move { engine.thumbnail(ino, Bucket::Large).await });
    eventually("the thumbnail to be on its way", || {
        env.mock.with(|d| d.held > 0)
    })
    .await;
    env.engine.sign_out().await.unwrap();
    env.mock.with(|d| d.hold = None);
    let got = fetch.await.unwrap();
    assert!(!matches!(got, Ok(Some(_))), "{got:?}");

    let left: Vec<_> = Bucket::ALL
        .into_iter()
        .filter_map(|b| std::fs::read_dir(env.engine.paths.thumbnail_dir.join(b.dir())).ok())
        .flat_map(|d| d.map(|e| e.unwrap().file_name()))
        .collect();
    assert!(left.is_empty(), "{left:?}");
    let rows: i64 = env
        .engine
        .db
        .with(|s| {
            s.0.query_row("SELECT COUNT(*) FROM thumbs", [], |r| r.get(0))
        })
        .unwrap();
    assert_eq!(rows, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_preview_onedrive_had_none_of_is_not_asked_for_again_after_a_restart() {
    let env = photo_env().await;
    env.mock.with(|d| d.no_thumbnails_yet = true);
    let ino = env.ino("/Photos/clip.mov");
    assert!(env
        .engine
        .thumbnail(ino, Bucket::Large)
        .await
        .unwrap()
        .is_none());
    assert_eq!(thumbnail_requests(&env, "/Photos/clip.mov"), 1);

    // The service starts again, at the next login.
    let again = Engine::new(
        env.engine.paths.clone(),
        env.engine.cfg(),
        Options {
            notifications: false,
        },
    )
    .unwrap();
    env.mock.with(|d| d.no_thumbnails_yet = false);
    assert!(again.thumbnail(ino, Bucket::Large).await.unwrap().is_none());
    assert_eq!(thumbnail_requests(&env, "/Photos/clip.mov"), 1);
    // Just uploaded, so asked about again a couple of minutes on.
    again.age_thumbnail_state(Duration::from_secs(121));
    assert!(again.thumbnail(ino, Bucket::Large).await.unwrap().is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_old_video_uploaded_just_now_is_asked_about_again_within_minutes() {
    let env = Env::new().await;
    env.mock.add_folder("/", "Videos");
    env.mock.add_file("/Videos", "clip.mov", &vec![7u8; 50_000]);
    // Recorded a year ago; the phone uploads it only now.
    env.mock
        .backdate("/Videos/clip.mov", Duration::from_secs(400 * 24 * 60 * 60));
    env.sync().await;
    env.mock.with(|d| d.no_thumbnails_yet = true);
    let ino = env.ino("/Videos/clip.mov");
    let get = || {
        let engine = env.engine.clone();
        async move { engine.thumbnail(ino, Bucket::Large).await.unwrap() }
    };
    assert!(get().await.is_none());

    // Asked again a couple of minutes on, as a new video would be.
    env.engine.age_thumbnail_state(Duration::from_secs(121));
    assert!(get().await.is_none());
    assert_eq!(thumbnail_requests(&env, "/Videos/clip.mov"), 2);
    // Still none: then less and less often.
    env.engine.age_thumbnail_state(Duration::from_secs(121));
    assert!(get().await.is_none());
    assert_eq!(thumbnail_requests(&env, "/Videos/clip.mov"), 2);
    env.mock.with(|d| d.no_thumbnails_yet = false);
    env.engine.age_thumbnail_state(Duration::from_secs(121));
    assert!(get().await.is_some());
    assert_eq!(thumbnail_requests(&env, "/Videos/clip.mov"), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn another_account_isnt_synced_into_this_pcs_onedrive() {
    let env = Env::new().await;
    env.sync().await;
    assert_eq!(env.engine.account().email, "test.user@example.com");
    let check = |drive_id, drive_type, email| {
        env.engine
            .check_account(drive_id, drive_type, email)
            .map_err(|e| e.to_string())
    };
    assert!(check("DRIVE1", "personal", "test.user@example.com").is_ok());

    // Graph sends the same drive's id back in another case: still this PC's.
    env.engine
        .db
        .with(|s| s.set_kv("drive_id", "drive1"))
        .unwrap();
    env.mock.add_file("/", "mine.txt", b"yours");
    env.engine.reload_tokens().await;
    env.sync().await;
    assert!(env
        .engine
        .db
        .with(|s| s.resolve("/mine.txt"))
        .unwrap()
        .is_some());

    // This PC's files came from another OneDrive than the one signed in now.
    env.engine
        .db
        .with(|s| s.set_kv("drive_id", "DRIVE2"))
        .unwrap();
    env.mock.add_file("/", "theirs.txt", b"not yours");
    env.engine.reload_tokens().await;
    env.engine.cycle().await;
    let st = env.engine.status();
    assert_eq!(st.state, "error");
    assert!(st.detail.contains("unlink this PC"), "{st:?}");
    assert!(env
        .engine
        .db
        .with(|s| s.resolve("/theirs.txt"))
        .unwrap()
        .is_none());
    let e = check("DRIVE1", "personal", "someone@example.com").unwrap_err();
    assert!(e.contains("test.user@example.com"), "{e}");

    // Linked before the drive was recorded: the account decides, and a work
    // account can have the same address as a personal one.
    env.engine.db.with(|s| s.del_kv("drive_id")).unwrap();
    assert!(check("DRIVE9", "personal", "someone@example.com").is_err());
    assert!(check("DRIVE9", "personal", "Test.User@example.com").is_ok());
    let e = check("DRIVE9", "business", "test.user@example.com").unwrap_err();
    assert!(
        e.contains("the personal OneDrive of test.user@example.com"),
        "{e}"
    );
    assert!(e.contains("your work or school one"), "{e}");

    // Unlinking forgets it, so any account can sign in after.
    env.engine.sign_out().await.unwrap();
    assert!(check("DRIVE1", "business", "someone@example.com").is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_service_stops_on_problems_only_the_user_can_fix() {
    use axentra_onedrive_core::config::NeedsUser;
    use axentra_onedrive_core::daemon::{run, DaemonOptions};
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::under(&dir.path().join("state"));
    let opts = || DaemonOptions {
        dbus: false,
        notifications: false,
    };
    let needs_user = |e: &anyhow::Error| e.downcast_ref::<NeedsUser>().map(|n| n.0.clone());

    // Files already in the folder it would be mounted over.
    let mnt = dir.path().join("OneDrive");
    std::fs::create_dir_all(&mnt).unwrap();
    std::fs::write(mnt.join("notes.txt"), "mine").unwrap();
    let cfg = Config {
        mount_point: mnt.display().to_string(),
        ..Default::default()
    };
    cfg.save(&paths.config_file).unwrap();
    let e = run(paths.clone(), opts()).await.unwrap_err();
    let why = needs_user(&e).unwrap_or_else(|| panic!("{e:#}"));
    assert!(why.contains("setup --folder"), "{why}");
    assert_eq!(std::fs::read(mnt.join("notes.txt")).unwrap(), b"mine");

    // A mistake in the settings file.
    std::fs::write(&paths.config_file, "mount_point = [").unwrap();
    let e = run(paths, opts()).await.unwrap_err();
    assert!(needs_user(&e).is_some(), "{e:#}");
}
