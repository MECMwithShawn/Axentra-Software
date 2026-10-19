//! End-to-end: a real FUSE mount backed by the engine, syncing with the mock
//! Graph server. Each test gets its own mount in a temp directory.

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

    async fn sync(&self) {
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

async fn write(path: PathBuf, data: &'static [u8]) {
    fsop(move || std::fs::write(&path, data).unwrap()).await
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
    write(env.p("Projects/video.mp4"), big).await;
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
    tokio::time::sleep(Duration::from_millis(500)).await;
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
async fn names_onedrive_cannot_hold_stay_local() {
    let env = Env::new().await;
    env.sync().await;
    write(env.p("~$Report.docx"), b"lock").await;
    write(env.p("bad:name.txt"), b"x").await;
    write(env.p("fine.txt"), b"y").await;
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
