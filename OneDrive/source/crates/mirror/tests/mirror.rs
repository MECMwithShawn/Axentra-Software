//! End to end: `MemSource` -> planner -> executor -> `MemDestination`, with
//! faults injected on both sides.

use std::collections::HashSet;
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axentra_clouddrive_mirror::{
    plan, Action, ChangedSourcePolicy, ExecOptions, Executor, ItemState, Manifest, Plan,
    PlanOptions, Readiness, RunOutcome, RunSummary,
};
use axentra_clouddrive_provider::{
    Account, Capabilities, ChangePage, ContentDigests, DestEntry, ItemKind, MemDestination,
    MemSource, MirrorDestination, Op, PResult, ProviderError, RelPath, RemoteStat, SourceItem,
    SourceProvider, VerifyLevel, MEM_ROOT,
};
use chrono::{DateTime, TimeZone, Utc};
use tokio::sync::{watch, Notify};

// ----- harness ----------------------------------------------------------------

struct Env {
    _dir: tempfile::TempDir,
    db: PathBuf,
    staging: PathBuf,
    /// Seconds added to the fake clock.
    offset: Arc<AtomicI64>,
}

impl Env {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        Self {
            db: dir.path().join("data/mirror.sqlite"),
            staging: dir.path().join("cache/mirror-staging"),
            _dir: dir,
            offset: Arc::new(AtomicI64::new(0)),
        }
    }

    fn manifest(&self) -> Manifest {
        std::fs::create_dir_all(self.db.parent().unwrap()).unwrap();
        Manifest::open(&self.db).unwrap()
    }

    fn opts(&self) -> ExecOptions {
        let mut o = ExecOptions::new(self.staging.clone());
        o.min_free_bytes = 0;
        o.free_space = |_| Ok(1 << 40);
        let offset = self.offset.clone();
        o.clock = Arc::new(move || {
            base_time() + chrono::Duration::seconds(offset.load(Ordering::SeqCst))
        });
        o
    }

    fn advance(&self, secs: i64) {
        self.offset.fetch_add(secs, Ordering::SeqCst);
    }

    fn staged_entries(&self) -> usize {
        std::fs::read_dir(&self.staging).map_or(0, |d| d.count())
    }
}

fn base_time() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 29, 12, 0, 0).unwrap()
}

fn popts() -> PlanOptions {
    PlanOptions {
        source_prefix: None,
        dest_root: "OneDrive".into(),
        staging_cap: 1 << 20,
        round_trip: false,
        on_source_change: ChangedSourcePolicy::MarkDiverged,
    }
}

fn no_stop() -> watch::Receiver<bool> {
    // The sender is dropped: never stops.
    watch::channel(false).1
}

async fn plan_with<S: SourceProvider, D: MirrorDestination>(
    src: &S,
    dest: &D,
    env: &Env,
    o: &PlanOptions,
) -> Plan {
    let m = env.manifest();
    plan(src, dest, &m.snapshot().unwrap(), o).await.unwrap()
}

/// Plan and run once.
async fn mirror<S: SourceProvider, D: MirrorDestination>(
    src: &S,
    dest: &D,
    env: &Env,
    o: &PlanOptions,
) -> (Plan, RunSummary) {
    let p = plan_with(src, dest, env, o).await;
    let mut m = env.manifest();
    let sum = Executor::new(src, dest, &mut m, env.opts())
        .run(&p, no_stop())
        .await
        .unwrap();
    (p, sum)
}

fn row(env: &Env, id: &str) -> axentra_clouddrive_mirror::ItemRow {
    env.manifest().get(id).unwrap().expect("a manifest row")
}

fn entry<'p>(p: &'p Plan, id: &str) -> &'p axentra_clouddrive_mirror::PlanEntry {
    p.entries
        .iter()
        .find(|e| e.source_id == id)
        .unwrap_or_else(|| panic!("no plan entry for {id}"))
}

fn uploads(dest: &MemDestination) -> Vec<String> {
    dest.calls()
        .into_iter()
        .filter(|c| c.starts_with("upload_"))
        .collect()
}

/// A small tree:
/// Docs/a.txt, Docs/Deep/b.bin, Ünïcødé/日本語/файл 📄.txt, top.txt
struct Tree {
    docs: String,
    a: String,
    deep: String,
    b: String,
    uni_file: String,
    top: String,
}

fn tree(src: &MemSource) -> Tree {
    let docs = src.add_folder(MEM_ROOT, "Docs");
    let a = src.add_file(&docs, "a.txt", b"alpha");
    let deep = src.add_folder(&docs, "Deep");
    let b = src.add_file(&deep, "b.bin", &[7u8; 300]);
    let uni = src.add_folder(MEM_ROOT, "Ünïcødé");
    let jp = src.add_folder(&uni, "日本語");
    let uni_file = src.add_file(&jp, "файл 📄.txt", "привет".as_bytes());
    let top = src.add_file(MEM_ROOT, "top.txt", b"top level");
    Tree {
        docs,
        a,
        deep,
        b,
        uni_file,
        top,
    }
}

// ----- wrappers ---------------------------------------------------------------

/// A destination that "crashes" after storing its `after`-th upload: it
/// signals and then never replies.
struct CrashAfter<'a> {
    inner: &'a MemDestination,
    after: usize,
    seen: AtomicUsize,
    crashed: Notify,
}

impl MirrorDestination for CrashAfter<'_> {
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }
    async fn account(&self) -> PResult<Account> {
        self.inner.account().await
    }
    async fn list(&self, folder: &RelPath) -> PResult<Vec<DestEntry>> {
        self.inner.list(folder).await
    }
    async fn ensure_folder(&self, folder: &RelPath) -> PResult<String> {
        self.inner.ensure_folder(folder).await
    }
    async fn upload_new(&self, folder: &RelPath, name: &str, local: &Path) -> PResult<RemoteStat> {
        let r = self.inner.upload_new(folder, name, local).await;
        if self.seen.fetch_add(1, Ordering::SeqCst) + 1 == self.after {
            self.crashed.notify_one();
            std::future::pending::<()>().await;
        }
        r
    }
    async fn upload_revision(&self, file: &RelPath, local: &Path) -> PResult<RemoteStat> {
        self.inner.upload_revision(file, local).await
    }
    async fn stat(&self, file: &RelPath) -> PResult<RemoteStat> {
        self.inner.stat(file).await
    }
    async fn download_to(&self, file: &RelPath, dest: &Path) -> PResult<ContentDigests> {
        self.inner.download_to(file, dest).await
    }
}

/// A destination that asks to stop after its first upload.
struct StopAfterFirst<'a> {
    inner: &'a MemDestination,
    stop: watch::Sender<bool>,
}

impl MirrorDestination for StopAfterFirst<'_> {
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }
    async fn account(&self) -> PResult<Account> {
        self.inner.account().await
    }
    async fn list(&self, folder: &RelPath) -> PResult<Vec<DestEntry>> {
        self.inner.list(folder).await
    }
    async fn ensure_folder(&self, folder: &RelPath) -> PResult<String> {
        self.inner.ensure_folder(folder).await
    }
    async fn upload_new(&self, folder: &RelPath, name: &str, local: &Path) -> PResult<RemoteStat> {
        let r = self.inner.upload_new(folder, name, local).await;
        let _ = self.stop.send(true);
        r
    }
    async fn upload_revision(&self, file: &RelPath, local: &Path) -> PResult<RemoteStat> {
        self.inner.upload_revision(file, local).await
    }
    async fn stat(&self, file: &RelPath) -> PResult<RemoteStat> {
        self.inner.stat(file).await
    }
    async fn download_to(&self, file: &RelPath, dest: &Path) -> PResult<ContentDigests> {
        self.inner.download_to(file, dest).await
    }
}

/// A source that can hide SHA-1/SHA-256 (as OneDrive Personal does) and
/// mark items as not plain content.
struct Tweaked<'a> {
    inner: &'a MemSource,
    qxh_only: bool,
    special: HashSet<String>,
}

impl Tweaked<'_> {
    fn tweak(&self, mut it: SourceItem) -> SourceItem {
        if self.qxh_only {
            it.digests.sha1 = None;
            it.digests.sha256 = None;
        }
        if self.special.contains(&it.id) {
            it.special = Some("package".into());
        }
        it
    }
}

impl SourceProvider for Tweaked<'_> {
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }
    async fn account(&self) -> PResult<Account> {
        self.inner.account().await
    }
    async fn change_page(&self, cursor: Option<&str>) -> PResult<ChangePage> {
        let mut p = self.inner.change_page(cursor).await?;
        p.items = p.items.into_iter().map(|i| self.tweak(i)).collect();
        Ok(p)
    }
    async fn item(&self, id: &str) -> PResult<SourceItem> {
        Ok(self.tweak(self.inner.item(id).await?))
    }
    async fn download_to(&self, item: &SourceItem, dest: &Path) -> PResult<ContentDigests> {
        self.inner.download_to(item, dest).await
    }
}

// ----- tests ------------------------------------------------------------------

#[tokio::test]
async fn a_dry_run_reads_only_and_counts_everything() {
    let env = Env::new();
    let src = MemSource::new();
    src.set_page_size(2);
    let t = tree(&src);
    let dest = MemDestination::new();

    let p = plan_with(&src, &dest, &env, &popts()).await;
    assert_eq!(dest.mutating_calls(), 0, "{:?}", dest.calls());
    assert_eq!(src.count(Op::Download), 0);
    assert!(!p.dest_root_exists);
    assert_eq!(p.totals.folders, 4);
    assert_eq!(p.totals.files, 4);
    assert_eq!(p.count(Action::CreateFolder), 4);
    assert_eq!(p.count(Action::UploadNew), 4);
    assert_eq!(p.totals.bytes_to_upload, 5 + 300 + 12 + 9);
    assert_eq!(p.totals.staging_needed, 300);
    // Parents come before children, folders before files.
    let pos = |id: &str| p.entries.iter().position(|e| e.source_id == id).unwrap();
    assert!(pos(&t.docs) < pos(&t.deep));
    assert!(pos(&t.deep) < pos(&t.a));
    assert_eq!(entry(&p, &t.b).dest_path, "OneDrive/Docs/Deep/b.bin");
    assert_eq!(
        entry(&p, &t.uni_file).dest_path,
        "OneDrive/Ünïcødé/日本語/файл 📄.txt"
    );
    assert!(p.source_cursor.is_some());

    // JSON both ways, and a readable summary.
    let back = Plan::from_json(&p.to_json().unwrap()).unwrap();
    assert_eq!(back, p);
    let text = p.render_text();
    assert!(text.contains("dry run"), "{text}");
    assert!(text.contains("upload_new"), "{text}");

    // Recording the dry run touches only mirror_runs.
    let mut m = env.manifest();
    p.record_dry_run(&mut m).unwrap();
    assert!(m.items().unwrap().is_empty());
    assert_eq!(m.runs(1).unwrap()[0].mode, "dry");
    assert_eq!(dest.mutating_calls(), 0);
}

#[tokio::test]
async fn a_full_run_with_round_trip_reaches_l4() {
    let env = Env::new();
    let src = MemSource::new();
    let t = tree(&src);
    let dest = MemDestination::new();
    let mut o = popts();
    o.round_trip = true;

    let (_, sum) = mirror(&src, &dest, &env, &o).await;
    assert_eq!(sum.outcome, RunOutcome::Completed);
    assert_eq!(sum.files_mirrored, 4);
    assert_eq!(sum.folders_created, 4);
    assert_eq!(sum.bytes_uploaded, 326);
    assert_eq!(
        dest.content("OneDrive/Docs/Deep/b.bin").unwrap(),
        vec![7u8; 300]
    );
    assert_eq!(
        dest.content("OneDrive/Ünïcødé/日本語/файл 📄.txt").unwrap(),
        "привет".as_bytes()
    );
    for id in [&t.a, &t.b, &t.uni_file, &t.top] {
        let r = row(&env, id);
        assert_eq!(r.state, ItemState::Verified, "{r:?}");
        assert_eq!(r.verify_level, VerifyLevel::L4RoundTrip);
        assert!(r.sha256.is_some() && r.dest_claimed_sha1 == r.sha1);
        assert!(!r.upload_pending);
    }
    assert_eq!(row(&env, &t.deep).state, ItemState::Verified);
    assert_eq!(env.staged_entries(), 0, "staging is emptied");

    let rep = Readiness::from_manifest(&env.manifest()).unwrap();
    assert_eq!(rep.measured_round_trip.items, 4);
    assert_eq!(rep.not_verified.items, 0);
    assert!(rep.render_text().contains("Measured"));
    assert_eq!(
        rep.last_run.as_ref().unwrap().outcome.as_deref(),
        Some("completed")
    );

    // Again: nothing left to do, and nothing is touched.
    let before = dest.mutating_calls();
    let (p, sum) = mirror(&src, &dest, &env, &o).await;
    assert_eq!(p.count(Action::AlreadyMirrored), 8);
    assert_eq!(sum.files_mirrored, 0);
    assert_eq!(dest.mutating_calls(), before);
}

#[tokio::test]
async fn without_round_trip_files_stop_at_remote_claimed() {
    let env = Env::new();
    let src = MemSource::new();
    let t = tree(&src);
    let dest = MemDestination::new();
    let (_, sum) = mirror(&src, &dest, &env, &popts()).await;
    assert_eq!(sum.files_mirrored, 4);
    let r = row(&env, &t.top);
    assert_eq!(r.verify_level, VerifyLevel::L3RemoteClaimed);
    assert_eq!(r.state, ItemState::Verified);
    let rep = Readiness::from_manifest(&env.manifest()).unwrap();
    assert_eq!(rep.claimed_only.items, 4);
    assert_eq!(rep.measured_round_trip.items, 0);

    // Turning round trip on later only verifies; nothing is uploaded again.
    let mut o = popts();
    o.round_trip = true;
    let (p, sum) = mirror(&src, &dest, &env, &o).await;
    assert_eq!(p.count(Action::Resume), 4);
    assert_eq!(sum.files_verified, 4);
    assert_eq!(uploads(&dest).len(), 4);
    assert_eq!(row(&env, &t.top).verify_level, VerifyLevel::L4RoundTrip);
}

#[tokio::test]
async fn a_crash_mid_upload_resumes_without_duplicates() {
    let env = Env::new();
    let src = MemSource::new();
    let t = tree(&src);
    let dest = MemDestination::new();
    let o = popts();
    let p = plan_with(&src, &dest, &env, &o).await;

    // The second upload is stored, then the process "dies" before the reply.
    let crashing = CrashAfter {
        inner: &dest,
        after: 2,
        seen: AtomicUsize::new(0),
        crashed: Notify::new(),
    };
    {
        let mut m = env.manifest();
        let mut ex = Executor::new(&src, &crashing, &mut m, env.opts());
        tokio::select! {
            r = ex.run(&p, no_stop()) => panic!("the run should have hung: {r:?}"),
            () = crashing.crashed.notified() => {}
        }
        // Executor, run and manifest connection all dropped here.
    }
    assert_eq!(uploads(&dest).len(), 2);
    let pending: Vec<_> = env
        .manifest()
        .items()
        .unwrap()
        .into_iter()
        .filter(|r| r.upload_pending)
        .collect();
    assert_eq!(pending.len(), 1, "the interrupted upload is marked");
    assert!(env.staged_entries() > 0, "the crash left a staged file");

    // A new process, a new plan and executor on the same manifest.
    let (p2, sum) = mirror(&src, &dest, &env, &o).await;
    assert_eq!(sum.outcome, RunOutcome::Completed);
    assert!(p2.count(Action::Resume) >= 1);
    let ups = uploads(&dest);
    let distinct: HashSet<&String> = ups.iter().collect();
    assert_eq!(ups.len(), 4, "every file uploaded exactly once: {ups:?}");
    assert_eq!(distinct.len(), 4);
    let r = row(&env, &pending[0].source_id);
    assert_eq!(r.verify_level, VerifyLevel::L3RemoteClaimed);
    assert!(!r.upload_pending && !r.adopted);
    assert!(r.detail.unwrap().contains("interrupted"));
    for id in [&t.a, &t.b, &t.uni_file, &t.top] {
        assert_eq!(row(&env, id).state, ItemState::Verified);
    }
    assert_eq!(env.staged_entries(), 0, "leftovers cleaned");
}

#[tokio::test]
async fn a_lost_reply_is_checked_before_retrying() {
    let env = Env::new();
    let src = MemSource::new();
    let id = src.add_file(MEM_ROOT, "one.txt", b"one");
    let dest = MemDestination::new();
    dest.fail_after(Op::UploadNew, ProviderError::Transient("timed out".into()));

    let (_, sum) = mirror(&src, &dest, &env, &popts()).await;
    assert_eq!(sum.files_failed, 1);
    let r = row(&env, &id);
    assert!(r.upload_pending);
    assert_eq!(r.attempts, 1);
    assert_eq!(r.next_attempt_at, Some(base_time().timestamp() + 60));

    env.advance(61);
    let (_, sum) = mirror(&src, &dest, &env, &popts()).await;
    assert_eq!(sum.files_mirrored, 1);
    assert_eq!(uploads(&dest).len(), 1, "not uploaded a second time");
    let r = row(&env, &id);
    assert_eq!(r.verify_level, VerifyLevel::L3RemoteClaimed);
    assert_eq!(
        (r.attempts, r.last_error, r.next_attempt_at),
        (0, None, None)
    );
}

#[tokio::test]
async fn a_stop_request_ends_the_run_and_a_later_run_finishes() {
    let env = Env::new();
    let src = MemSource::new();
    tree(&src);
    let dest = MemDestination::new();
    let p = plan_with(&src, &dest, &env, &popts()).await;

    // Stopped before it starts: nothing is uploaded.
    let (tx, rx) = watch::channel(true);
    let mut m = env.manifest();
    let sum = Executor::new(&src, &dest, &mut m, env.opts())
        .run(&p, rx)
        .await
        .unwrap();
    drop(tx);
    assert_eq!(sum.outcome, RunOutcome::Cancelled);
    assert!(uploads(&dest).is_empty());

    // Stopped after the first upload.
    let (tx, rx) = watch::channel(false);
    let stopping = StopAfterFirst {
        inner: &dest,
        stop: tx,
    };
    let sum = Executor::new(&src, &stopping, &mut m, env.opts())
        .run(&p, rx)
        .await
        .unwrap();
    assert_eq!(sum.outcome, RunOutcome::Cancelled);
    assert_eq!(uploads(&dest).len(), 1);
    assert_eq!(m.runs(1).unwrap()[0].outcome.as_deref(), Some("cancelled"));
    drop(m);

    let (_, sum) = mirror(&src, &dest, &env, &popts()).await;
    assert_eq!(sum.outcome, RunOutcome::Completed);
    assert_eq!(uploads(&dest).len(), 4);
}

#[tokio::test]
async fn a_source_change_after_mirroring_diverges_and_leaves_the_destination() {
    let env = Env::new();
    let src = MemSource::new();
    let t = tree(&src);
    let dest = MemDestination::new();
    mirror(&src, &dest, &env, &popts()).await;
    let before = dest.mutating_calls();

    src.modify(&t.a, b"alpha, edited");
    let (p, sum) = mirror(&src, &dest, &env, &popts()).await;
    assert_eq!(entry(&p, &t.a).action, Action::Diverged);
    assert_eq!(sum.files_diverged, 1);
    assert_eq!(dest.mutating_calls(), before, "{:?}", dest.calls());
    assert_eq!(
        dest.revisions("OneDrive/Docs/a.txt"),
        vec![b"alpha".to_vec()]
    );
    let r = row(&env, &t.a);
    assert_eq!(r.state, ItemState::Diverged);
    assert_eq!(r.size, 5, "the row still describes what was mirrored");
    assert_eq!(r.verify_level, VerifyLevel::L3RemoteClaimed);
    let rep = Readiness::from_manifest(&env.manifest()).unwrap();
    assert_eq!(rep.diverged.len(), 1);

    // It stays diverged on later runs.
    let (p, _) = mirror(&src, &dest, &env, &popts()).await;
    assert_eq!(entry(&p, &t.a).action, Action::Diverged);
    assert_eq!(dest.mutating_calls(), before);
}

#[tokio::test]
async fn a_change_between_plan_and_run_is_caught_by_the_executor() {
    let env = Env::new();
    let src = MemSource::new();
    let id = src.add_file(MEM_ROOT, "x.txt", b"first");
    let dest = MemDestination::new();
    mirror(&src, &dest, &env, &popts()).await;
    // Planned as already mirrored... then a round trip makes it a resume,
    // and the source changes before the run.
    let mut o = popts();
    o.round_trip = true;
    let p = plan_with(&src, &dest, &env, &o).await;
    assert_eq!(entry(&p, &id).action, Action::Resume);
    src.modify(&id, b"second!");
    let mut m = env.manifest();
    let sum = Executor::new(&src, &dest, &mut m, env.opts())
        .run(&p, no_stop())
        .await
        .unwrap();
    assert_eq!(sum.files_diverged, 1);
    assert_eq!(dest.revisions("x.txt").len(), 0);
    assert_eq!(dest.revisions("OneDrive/x.txt"), vec![b"first".to_vec()]);
}

#[tokio::test]
async fn the_revision_policy_keeps_the_old_revision() {
    let env = Env::new();
    let src = MemSource::new();
    let id = src.add_file(MEM_ROOT, "r.txt", b"v1");
    let dest = MemDestination::new();
    let mut o = popts();
    o.on_source_change = ChangedSourcePolicy::CreateNewRevision;
    o.round_trip = true;
    mirror(&src, &dest, &env, &o).await;
    src.modify(&id, b"version 2");
    let (p, sum) = mirror(&src, &dest, &env, &o).await;
    assert_eq!(entry(&p, &id).action, Action::UploadRevision);
    assert_eq!(sum.files_mirrored, 1);
    assert_eq!(
        dest.revisions("OneDrive/r.txt"),
        vec![b"v1".to_vec(), b"version 2".to_vec()]
    );
    let r = row(&env, &id);
    assert_eq!(r.verify_level, VerifyLevel::L4RoundTrip);
    assert_eq!(r.size, 9);
}

#[tokio::test]
async fn deletes_and_moves_on_the_source_are_recorded_only() {
    let env = Env::new();
    let src = MemSource::new();
    let t = tree(&src);
    let dest = MemDestination::new();
    mirror(&src, &dest, &env, &popts()).await;
    let paths = dest.paths();
    let before = dest.mutating_calls();

    src.mark_deleted(&t.a);
    src.set_name(&t.top, "renamed.txt");
    let (p, sum) = mirror(&src, &dest, &env, &popts()).await;
    assert_eq!(entry(&p, &t.a).action, Action::SourceDeleted);
    assert_eq!(entry(&p, &t.top).action, Action::SourceMoved);
    assert_eq!(sum.source_deleted, 1);
    assert_eq!(dest.mutating_calls(), before);
    assert_eq!(dest.paths(), paths, "the destination is unchanged");
    assert_eq!(dest.content("OneDrive/Docs/a.txt").unwrap(), b"alpha");
    let r = row(&env, &t.a);
    assert_eq!(r.state, ItemState::SourceDeleted);
    assert_eq!(r.verify_level, VerifyLevel::L3RemoteClaimed);
    let r = row(&env, &t.top);
    assert_eq!(r.source_path, "top.txt");
    assert!(r.detail.unwrap().contains("renamed.txt"));
}

#[tokio::test]
async fn a_different_file_at_the_destination_diverges_and_both_are_kept() {
    let env = Env::new();
    let src = MemSource::new();
    let diff_size = src.add_file(MEM_ROOT, "size.txt", b"source");
    let same_size = src.add_file(MEM_ROOT, "hash.txt", b"source");
    let same = src.add_file(MEM_ROOT, "same.txt", b"identical");
    let dir_clash = src.add_file(MEM_ROOT, "clash", b"file");
    let dest = MemDestination::new();
    let stage = tempfile::tempdir().unwrap();
    dest.ensure_folder(&RelPath::parse("OneDrive/clash").unwrap())
        .await
        .unwrap();
    let root = RelPath::parse("OneDrive").unwrap();
    for (name, data) in [
        ("size.txt", &b"other content"[..]),
        ("hash.txt", b"SOURCE"),
        ("same.txt", b"identical"),
    ] {
        let f = stage.path().join(name);
        std::fs::write(&f, data).unwrap();
        dest.upload_new(&root, name, &f).await.unwrap();
    }
    let before = dest.mutating_calls();

    let p = plan_with(&src, &dest, &env, &popts()).await;
    assert_eq!(dest.mutating_calls(), before);
    assert!(p.dest_root_exists);
    assert_eq!(entry(&p, &diff_size).action, Action::Diverged);
    assert_eq!(entry(&p, &same_size).action, Action::Diverged);
    assert_eq!(entry(&p, &same).action, Action::AdoptExisting);
    assert_eq!(entry(&p, &dir_clash).action, Action::Diverged);

    let mut m = env.manifest();
    let sum = Executor::new(&src, &dest, &mut m, env.opts())
        .run(&p, no_stop())
        .await
        .unwrap();
    assert_eq!(sum.files_diverged, 3);
    assert_eq!(sum.files_adopted, 1);
    assert_eq!(sum.files_mirrored, 0);
    assert_eq!(dest.content("OneDrive/size.txt").unwrap(), b"other content");
    assert_eq!(dest.content("OneDrive/hash.txt").unwrap(), b"SOURCE");
    assert_eq!(dest.revisions("OneDrive/same.txt").len(), 1);
    let r = m.get(&same).unwrap().unwrap();
    assert!(r.adopted);
    assert_eq!(r.verify_level, VerifyLevel::L3RemoteClaimed);
    assert_eq!(
        m.get(&diff_size).unwrap().unwrap().state,
        ItemState::Diverged
    );
    // Only the existing folder was "ensured"; no upload was attempted.
    assert!(uploads(&dest)
        .iter()
        .all(|u| u.starts_with("upload_new OneDrive/")));
    assert_eq!(uploads(&dest).len(), 3, "only the three set up above");
}

#[tokio::test]
async fn without_a_source_sha1_a_same_named_file_is_compared_after_download() {
    let env = Env::new();
    let mem = MemSource::new();
    let same = mem.add_file(MEM_ROOT, "same.txt", b"identical");
    let other = mem.add_file(MEM_ROOT, "other.txt", b"source!");
    let src = Tweaked {
        inner: &mem,
        qxh_only: true,
        special: HashSet::new(),
    };
    let dest = MemDestination::new();
    let stage = tempfile::tempdir().unwrap();
    let root = RelPath::parse("OneDrive").unwrap();
    dest.ensure_folder(&root).await.unwrap();
    for (name, data) in [("same.txt", &b"identical"[..]), ("other.txt", b"SOURCE!")] {
        let f = stage.path().join(name);
        std::fs::write(&f, data).unwrap();
        dest.upload_new(&root, name, &f).await.unwrap();
    }
    let (p, sum) = mirror(&src, &dest, &env, &popts()).await;
    assert_eq!(entry(&p, &same).action, Action::CheckExisting);
    assert_eq!(entry(&p, &other).action, Action::CheckExisting);
    assert_eq!(sum.files_adopted, 1);
    assert_eq!(sum.files_diverged, 1);
    assert_eq!(dest.content("OneDrive/other.txt").unwrap(), b"SOURCE!");
    assert_eq!(uploads(&dest).len(), 2, "nothing uploaded by the mirror");
    assert!(row(&env, &same).adopted);
    assert_eq!(row(&env, &other).state, ItemState::Diverged);
}

#[tokio::test]
async fn names_that_differ_only_in_case_stay_distinct_and_are_reported() {
    let env = Env::new();
    let src = MemSource::new();
    let f = src.add_folder(MEM_ROOT, "Folder");
    let upper = src.add_file(&f, "Report.txt", b"upper");
    let lower = src.add_file(&f, "report.txt", b"lower");
    let nfc = src.add_file(&f, "caf\u{e9}.txt", b"nfc");
    let nfd = src.add_file(&f, "cafe\u{301}.txt", b"nfd");
    let dest = MemDestination::new();
    let (p, sum) = mirror(&src, &dest, &env, &popts()).await;
    assert_eq!(
        p.case_collisions,
        vec![vec![
            "Folder/Report.txt".to_string(),
            "Folder/report.txt".into()
        ]]
    );
    assert!(p.render_text().contains("only in case"));
    assert_eq!(sum.files_mirrored, 4);
    assert_eq!(
        dest.content("OneDrive/Folder/Report.txt").unwrap(),
        b"upper"
    );
    assert_eq!(
        dest.content("OneDrive/Folder/report.txt").unwrap(),
        b"lower"
    );
    assert_eq!(
        dest.content("OneDrive/Folder/caf\u{e9}.txt").unwrap(),
        b"nfc"
    );
    assert_eq!(
        dest.content("OneDrive/Folder/cafe\u{301}.txt").unwrap(),
        b"nfd"
    );
    for id in [&upper, &lower, &nfc, &nfd] {
        assert_eq!(row(&env, id).state, ItemState::Verified);
    }
}

#[tokio::test]
async fn the_staging_cap_is_respected_and_large_files_are_skipped() {
    let env = Env::new();
    let src = MemSource::new();
    let small = src.add_file(MEM_ROOT, "small", &[1u8; 50]);
    let exact = src.add_file(MEM_ROOT, "exact", &[2u8; 100]);
    let big = src.add_file(MEM_ROOT, "big", &[3u8; 150]);
    let dest = MemDestination::new();
    let mut o = popts();
    o.staging_cap = 100;
    o.round_trip = true;
    let (p, sum) = mirror(&src, &dest, &env, &o).await;
    assert_eq!(entry(&p, &big).action, Action::SkipTooLarge);
    assert_eq!(p.totals.staging_needed, 100);
    assert_eq!(sum.files_mirrored, 2);
    assert_eq!(sum.files_skipped, 1);
    assert!(
        sum.staging_peak > 0 && sum.staging_peak <= 100,
        "{}",
        sum.staging_peak
    );
    assert!(dest.content("OneDrive/big").is_none());
    assert_eq!(row(&env, &big).state, ItemState::Skipped);
    assert_eq!(row(&env, &small).verify_level, VerifyLevel::L4RoundTrip);
    assert_eq!(row(&env, &exact).verify_level, VerifyLevel::L4RoundTrip);
    let rep = Readiness::from_manifest(&env.manifest()).unwrap();
    assert_eq!(rep.skipped.len(), 1);
    assert_eq!(rep.eligible.items, 2);
}

#[tokio::test]
async fn too_little_free_space_stops_before_staging() {
    let env = Env::new();
    let src = MemSource::new();
    src.add_file(MEM_ROOT, "f", &[0u8; 64]);
    let dest = MemDestination::new();
    let p = plan_with(&src, &dest, &env, &popts()).await;
    let mut opts = env.opts();
    opts.free_space = |_| Ok(10);
    let mut m = env.manifest();
    let sum = Executor::new(&src, &dest, &mut m, opts)
        .run(&p, no_stop())
        .await
        .unwrap();
    assert_eq!(
        sum.outcome,
        RunOutcome::LowDiskSpace {
            free: 10,
            needed: 64
        }
    );
    assert_eq!(src.count(Op::Download), 0);
    assert!(uploads(&dest).is_empty());
}

#[tokio::test]
async fn a_transient_fault_backs_off_then_succeeds() {
    let env = Env::new();
    let src = MemSource::new();
    let id = src.add_file(MEM_ROOT, "t.txt", b"data");
    let dest = MemDestination::new();
    dest.fail_next(Op::UploadNew, ProviderError::Transient("503".into()));

    let (_, sum) = mirror(&src, &dest, &env, &popts()).await;
    assert_eq!(sum.files_failed, 1);
    let r = row(&env, &id);
    assert_eq!(r.state, ItemState::Failed);
    assert_eq!(r.attempts, 1);
    assert!(r.last_error.as_deref().unwrap().contains("503"));
    let rep = Readiness::from_manifest(&env.manifest()).unwrap();
    assert_eq!(rep.failed.len(), 1);

    // Too soon: not tried.
    env.advance(30);
    let (_, sum) = mirror(&src, &dest, &env, &popts()).await;
    assert_eq!(sum.files_waiting, 1);
    assert_eq!(dest.count_calls("upload_new"), 1);

    // Fails again: the wait doubles.
    dest.fail_next(Op::UploadNew, ProviderError::Transient("503".into()));
    env.advance(31);
    mirror(&src, &dest, &env, &popts()).await;
    let r = row(&env, &id);
    assert_eq!(r.attempts, 2);
    assert_eq!(r.next_attempt_at, Some(base_time().timestamp() + 61 + 120));

    env.advance(121);
    let (_, sum) = mirror(&src, &dest, &env, &popts()).await;
    assert_eq!(sum.files_mirrored, 1);
    let r = row(&env, &id);
    assert_eq!(r.state, ItemState::Verified);
    assert_eq!(r.attempts, 0);
}

#[tokio::test]
async fn rate_limiting_stops_the_run_until_the_given_time() {
    let env = Env::new();
    let src = MemSource::new();
    let a = src.add_file(MEM_ROOT, "a", b"a");
    src.add_file(MEM_ROOT, "b", b"b");
    let dest = MemDestination::new();
    dest.fail_next(
        Op::UploadNew,
        ProviderError::RateLimited {
            retry_after: Some(Duration::from_secs(600)),
        },
    );
    let (_, sum) = mirror(&src, &dest, &env, &popts()).await;
    let at = base_time().timestamp() + 600;
    assert_eq!(sum.outcome, RunOutcome::RateLimited { retry_at: at });
    assert_eq!(dest.count_calls("upload_new"), 1, "nothing more was tried");
    assert_eq!(row(&env, &a).next_attempt_at, Some(at));
}

#[tokio::test]
async fn an_auth_failure_stops_the_run_cleanly() {
    let env = Env::new();
    let src = MemSource::new();
    let t = tree(&src);
    let dest = MemDestination::new();
    dest.fail_next(Op::UploadNew, ProviderError::AuthRequired);
    let (_, sum) = mirror(&src, &dest, &env, &popts()).await;
    assert_eq!(sum.outcome, RunOutcome::AuthRequired);
    assert_eq!(dest.count_calls("upload_new"), 1);
    assert_eq!(env.staged_entries(), 0);
    let m = env.manifest();
    assert_eq!(
        m.runs(1).unwrap()[0].outcome.as_deref(),
        Some("auth_required")
    );
    let failed: Vec<_> = m
        .items()
        .unwrap()
        .into_iter()
        .filter(|r| r.last_error.is_some())
        .collect();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].attempts, 0, "not counted against the file");
    assert_eq!(failed[0].next_attempt_at, None);
    drop(m);

    let (_, sum) = mirror(&src, &dest, &env, &popts()).await;
    assert_eq!(sum.outcome, RunOutcome::Completed);
    assert_eq!(row(&env, &t.a).state, ItemState::Verified);
    assert_eq!(dest.count_calls("upload_new"), 5);
}

#[tokio::test]
async fn a_corrupt_destination_byte_is_caught_at_l4() {
    let env = Env::new();
    let src = MemSource::new();
    let id = src.add_file(MEM_ROOT, "c.bin", b"precious bytes");
    let dest = MemDestination::new();
    mirror(&src, &dest, &env, &popts()).await;
    assert_eq!(row(&env, &id).verify_level, VerifyLevel::L3RemoteClaimed);

    dest.corrupt_stored("OneDrive/c.bin");
    let mut o = popts();
    o.round_trip = true;
    let (_, sum) = mirror(&src, &dest, &env, &o).await;
    assert_eq!(sum.files_failed, 1);
    let r = row(&env, &id);
    assert_eq!(r.state, ItemState::Failed);
    assert_eq!(
        r.verify_level,
        VerifyLevel::L3RemoteClaimed,
        "not rounded up"
    );
    assert!(
        r.last_error.as_deref().unwrap().contains("SHA-256"),
        "{r:?}"
    );
    assert_eq!(
        dest.revisions("OneDrive/c.bin").len(),
        1,
        "nothing re-uploaded"
    );
    let rep = Readiness::from_manifest(&env.manifest()).unwrap();
    assert_eq!(rep.measured_round_trip.items, 0);
    assert_eq!(rep.claimed_only.items, 1);
    assert_eq!(rep.failed.len(), 1);
}

#[tokio::test]
async fn a_corrupt_source_download_is_caught_at_l1() {
    let env = Env::new();
    let src = MemSource::new();
    let id = src.add_file(MEM_ROOT, "s.bin", b"source bytes");
    let dest = MemDestination::new();
    src.corrupt_next_download();
    let (_, sum) = mirror(&src, &dest, &env, &popts()).await;
    assert_eq!(sum.files_failed, 1);
    assert!(uploads(&dest).is_empty());
    let r = row(&env, &id);
    assert_eq!(r.verify_level, VerifyLevel::L0Planned);
    assert!(
        r.last_error.as_deref().unwrap().contains("integrity"),
        "{r:?}"
    );
    env.advance(3600);
    let (_, sum) = mirror(&src, &dest, &env, &popts()).await;
    assert_eq!(sum.files_mirrored, 1);
}

#[tokio::test]
async fn unsupported_items_and_their_contents_are_skipped() {
    let env = Env::new();
    let mem = MemSource::new();
    let pkg = mem.add_folder(MEM_ROOT, "Notebook");
    let inside = mem.add_file(&pkg, "section.one", b"x");
    let plain = mem.add_file(MEM_ROOT, "plain.txt", b"p");
    let src = Tweaked {
        inner: &mem,
        qxh_only: false,
        special: [pkg.clone()].into_iter().collect(),
    };
    let dest = MemDestination::new();
    let (p, sum) = mirror(&src, &dest, &env, &popts()).await;
    assert_eq!(entry(&p, &pkg).action, Action::SkipUnsupported);
    assert_eq!(entry(&p, &inside).action, Action::SkipUnsupported);
    assert_eq!(entry(&p, &plain).action, Action::UploadNew);
    assert_eq!(sum.files_mirrored, 1);
    assert_eq!(sum.files_skipped, 1);
    assert_eq!(dest.paths(), ["OneDrive", "OneDrive/plain.txt"]);
}

#[tokio::test]
async fn a_source_folder_prefix_limits_the_mirror() {
    let env = Env::new();
    let src = MemSource::new();
    let test = src.add_folder(MEM_ROOT, "CloudDrive Mirror Test");
    let inner = src.add_file(&test, "in.txt", b"in");
    let outside = src.add_file(MEM_ROOT, "out.txt", b"out");
    let lookalike = src.add_folder(MEM_ROOT, "CloudDrive Mirror Test 2");
    let dest = MemDestination::new();
    let mut o = popts();
    o.source_prefix = Some("/CloudDrive Mirror Test".into());
    let (p, sum) = mirror(&src, &dest, &env, &o).await;
    let ids: HashSet<&str> = p.entries.iter().map(|e| e.source_id.as_str()).collect();
    assert_eq!(ids, [test.as_str(), inner.as_str()].into_iter().collect());
    assert!(!ids.contains(outside.as_str()) && !ids.contains(lookalike.as_str()));
    assert_eq!(sum.files_mirrored, 1);
    assert_eq!(
        dest.paths(),
        [
            "OneDrive",
            "OneDrive/CloudDrive Mirror Test",
            "OneDrive/CloudDrive Mirror Test/in.txt"
        ]
    );

    // Deleting something outside the prefix is not this mirror's business.
    src.mark_deleted(&outside);
    let p = plan_with(&src, &dest, &env, &o).await;
    assert_eq!(p.count(Action::SourceDeleted), 0);
}

#[tokio::test]
async fn a_file_where_a_folder_goes_diverges_with_its_contents() {
    let env = Env::new();
    let src = MemSource::new();
    let f = src.add_folder(MEM_ROOT, "Photos");
    let child = src.add_file(&f, "p.jpg", b"jpeg");
    let dest = MemDestination::new();
    let stage = tempfile::tempdir().unwrap();
    let root = RelPath::parse("OneDrive").unwrap();
    dest.ensure_folder(&root).await.unwrap();
    let local = stage.path().join("Photos");
    std::fs::write(&local, b"not a folder").unwrap();
    dest.upload_new(&root, "Photos", &local).await.unwrap();

    let (p, sum) = mirror(&src, &dest, &env, &popts()).await;
    assert_eq!(entry(&p, &f).action, Action::Diverged);
    assert_eq!(entry(&p, &child).action, Action::Diverged);
    assert_eq!(sum.files_diverged, 1);
    assert_eq!(dest.content("OneDrive/Photos").unwrap(), b"not a folder");
    assert_eq!(row(&env, &f).state, ItemState::Diverged);
}

#[tokio::test]
async fn round_trip_needs_a_destination_that_can_download() {
    let env = Env::new();
    let src = MemSource::new();
    src.add_file(MEM_ROOT, "f", b"f");
    let dest = MemDestination::with_capabilities(Capabilities {
        sha1: true,
        create_folder: true,
        ..Default::default()
    });
    let mut o = popts();
    o.round_trip = true;
    let p = plan_with(&src, &dest, &env, &o).await;
    let mut m = env.manifest();
    let err = Executor::new(&src, &dest, &mut m, env.opts())
        .run(&p, no_stop())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("downloading back"), "{err}");
    assert_eq!(dest.mutating_calls(), 0);
    assert!(m.runs(1).unwrap().is_empty());
}

/// A destination with Proton's name rules for `[`, that can hide one name
/// from listings, and whose `stat` of one name fails.
struct Picky<'a> {
    inner: &'a MemDestination,
    hide: Option<&'static str>,
    bad_stat: Option<&'static str>,
}

impl MirrorDestination for Picky<'_> {
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }
    fn name_problem(&self, name: &str, _: ItemKind) -> Option<&'static str> {
        name.contains('[').then_some("it contains [")
    }
    async fn account(&self) -> PResult<Account> {
        self.inner.account().await
    }
    async fn list(&self, folder: &RelPath) -> PResult<Vec<DestEntry>> {
        let mut l = self.inner.list(folder).await?;
        l.retain(|e| Some(e.name.as_str()) != self.hide);
        Ok(l)
    }
    async fn ensure_folder(&self, folder: &RelPath) -> PResult<String> {
        self.inner.ensure_folder(folder).await
    }
    async fn upload_new(&self, folder: &RelPath, name: &str, local: &Path) -> PResult<RemoteStat> {
        assert!(self.name_problem(name, ItemKind::File).is_none(), "{name}");
        self.inner.upload_new(folder, name, local).await
    }
    async fn upload_revision(&self, file: &RelPath, local: &Path) -> PResult<RemoteStat> {
        self.inner.upload_revision(file, local).await
    }
    async fn stat(&self, file: &RelPath) -> PResult<RemoteStat> {
        if file.name().is_some() && file.name() == self.bad_stat {
            return Err(ProviderError::Other("Proton claims no size".into()));
        }
        self.inner.stat(file).await
    }
    async fn download_to(&self, file: &RelPath, dest: &Path) -> PResult<ContentDigests> {
        self.inner.download_to(file, dest).await
    }
}

#[tokio::test]
async fn names_the_destination_refuses_are_skipped_not_failed() {
    let env = Env::new();
    let src = MemSource::new();
    let bad = src.add_file(MEM_ROOT, "Report [final].txt", b"r");
    let folder = src.add_folder(MEM_ROOT, "Old [2019]");
    let inside = src.add_file(&folder, "fine.txt", b"f");
    let ok = src.add_file(MEM_ROOT, "ok.txt", b"ok");
    let mem = MemDestination::new();
    let dest = Picky {
        inner: &mem,
        hide: None,
        bad_stat: None,
    };
    let (p, sum) = mirror(&src, &dest, &env, &popts()).await;
    for id in [&bad, &folder, &inside] {
        assert_eq!(entry(&p, id).action, Action::SkipUnsupported, "{id}");
    }
    assert_eq!(entry(&p, &ok).action, Action::UploadNew);
    assert_eq!(
        (sum.files_mirrored, sum.files_skipped, sum.files_failed),
        (1, 2, 0)
    );
    let r = row(&env, &bad);
    assert_eq!(r.state, ItemState::Skipped);
    assert!(!r.upload_pending);
    // And stays skipped, not retried.
    let (p, sum) = mirror(&src, &dest, &env, &popts()).await;
    assert_eq!(entry(&p, &bad).action, Action::SkipUnsupported);
    assert_eq!(sum.files_failed, 0);
    assert_eq!(mem.paths(), ["OneDrive", "OneDrive/ok.txt"]);
}

#[tokio::test]
async fn a_name_taken_after_planning_is_adopted_not_claimed() {
    let env = Env::new();
    let src = MemSource::new();
    let id = src.add_file(MEM_ROOT, "late.txt", b"same bytes");
    let mem = MemDestination::new();
    let root = RelPath::parse("OneDrive").unwrap();
    mem.ensure_folder(&root).await.unwrap();
    let stage = tempfile::tempdir().unwrap();
    let f = stage.path().join("late.txt");
    std::fs::write(&f, b"same bytes").unwrap();
    mem.upload_new(&root, "late.txt", &f).await.unwrap();
    // The listing hides it, as if it arrived between plan and run.
    let dest = Picky {
        inner: &mem,
        hide: Some("late.txt"),
        bad_stat: None,
    };
    let (p, sum) = mirror(&src, &dest, &env, &popts()).await;
    assert_eq!(entry(&p, &id).action, Action::UploadNew);
    assert_eq!((sum.files_mirrored, sum.files_adopted), (0, 1));
    let r = row(&env, &id);
    assert!(r.adopted && !r.upload_pending);
    assert!(r.detail.unwrap().contains("adopted"));
    assert_eq!(mem.revisions("OneDrive/late.txt").len(), 1);
}

#[tokio::test]
async fn a_destination_file_that_cant_be_compared_diverges_alone() {
    let env = Env::new();
    let src = MemSource::new();
    let odd = src.add_file(MEM_ROOT, "odd.txt", b"odd");
    let ok = src.add_file(MEM_ROOT, "ok.txt", b"ok");
    let mem = MemDestination::new();
    let root = RelPath::parse("OneDrive").unwrap();
    mem.ensure_folder(&root).await.unwrap();
    let stage = tempfile::tempdir().unwrap();
    let f = stage.path().join("odd.txt");
    std::fs::write(&f, b"odd").unwrap();
    mem.upload_new(&root, "odd.txt", &f).await.unwrap();
    let dest = Picky {
        inner: &mem,
        hide: None,
        bad_stat: Some("odd.txt"),
    };
    let (p, sum) = mirror(&src, &dest, &env, &popts()).await;
    assert_eq!(entry(&p, &odd).action, Action::Diverged);
    assert!(entry(&p, &odd)
        .reason
        .as_deref()
        .unwrap()
        .contains("can't be compared"));
    assert_eq!(entry(&p, &ok).action, Action::UploadNew);
    assert_eq!((sum.files_mirrored, sum.files_diverged), (1, 1));
}

#[tokio::test]
async fn a_staging_folder_the_mirror_didnt_make_is_left_alone() {
    let env = Env::new();
    let src = MemSource::new();
    src.add_file(MEM_ROOT, "a.txt", b"a");
    let dest = MemDestination::new();
    std::fs::create_dir_all(&env.staging).unwrap();
    std::fs::set_permissions(&env.staging, std::fs::Permissions::from_mode(0o755)).unwrap();
    let keep = env.staging.join("m-keep.txt");
    std::fs::write(&keep, b"the user's").unwrap();
    std::fs::create_dir(env.staging.join("m-folder")).unwrap();

    let p = plan_with(&src, &dest, &env, &popts()).await;
    let mut m = env.manifest();
    let err = Executor::new(&src, &dest, &mut m, env.opts())
        .run(&p, no_stop())
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("wasn't made by the mirror"),
        "{err}"
    );
    assert_eq!(std::fs::read(&keep).unwrap(), b"the user's");
    assert!(env.staging.join("m-folder").is_dir());
    let mode = std::fs::metadata(&env.staging)
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o755, "permissions untouched");
    assert_eq!(dest.mutating_calls(), 0);
    assert!(m.runs(1).unwrap().is_empty());

    // An empty one is taken over, and from then on is the mirror's.
    std::fs::remove_file(&keep).unwrap();
    std::fs::remove_dir(env.staging.join("m-folder")).unwrap();
    let sum = Executor::new(&src, &dest, &mut m, env.opts())
        .run(&p, no_stop())
        .await
        .unwrap();
    assert_eq!(sum.files_mirrored, 1);
    std::fs::create_dir(env.staging.join("m-left-by-a-crash")).unwrap();
    Executor::new(&src, &dest, &mut m, env.opts())
        .run(&p, no_stop())
        .await
        .unwrap();
    assert_eq!(env.staged_entries(), 0);
}

#[tokio::test]
async fn only_one_run_at_a_time() {
    let env = Env::new();
    let src = MemSource::new();
    src.add_file(MEM_ROOT, "a.txt", b"a");
    let dest = MemDestination::new();
    let p = plan_with(&src, &dest, &env, &popts()).await;
    let mut m = env.manifest();
    // The first run makes the folder; then another run "holds" it.
    Executor::new(&src, &dest, &mut m, env.opts())
        .run(&p, no_stop())
        .await
        .unwrap();
    let held = std::fs::File::open(&env.staging).unwrap();
    assert_eq!(
        unsafe { libc::flock(held.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
    let busy = env.staging.join("m-in-flight");
    std::fs::create_dir(&busy).unwrap();
    let err = Executor::new(&src, &dest, &mut m, env.opts())
        .run(&p, no_stop())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("another mirror run"), "{err}");
    assert!(busy.is_dir(), "the other run's staging is untouched");
    drop(held);
    Executor::new(&src, &dest, &mut m, env.opts())
        .run(&p, no_stop())
        .await
        .unwrap();
    assert!(!busy.exists());
}

// ----- helpers ----------------------------------------------------------------

trait CountCalls {
    fn count_calls(&self, op: &str) -> usize;
}

impl CountCalls for MemDestination {
    fn count_calls(&self, op: &str) -> usize {
        let prefix = format!("{op} ");
        self.calls()
            .iter()
            .filter(|c| c.starts_with(&prefix))
            .count()
    }
}

// ----- fetching ahead, and a local folder ----------------------------------------

#[tokio::test]
async fn fetching_ahead_gives_the_same_result_in_plan_order() {
    let env = Env::new();
    let src = MemSource::new();
    let docs = src.add_folder(MEM_ROOT, "Docs");
    let ids: Vec<String> = (0..25)
        .map(|i| src.add_file(&docs, &format!("f{i:02}.txt"), format!("file {i}").as_bytes()))
        .collect();
    // One download fails: only that file waits for a retry.
    src.fail_next(Op::Download, ProviderError::Transient("dropped".into()));
    let dest = MemDestination::new();
    let p = plan_with(&src, &dest, &env, &popts()).await;
    let mut m = env.manifest();
    let mut o = env.opts();
    o.parallel = 6;
    let sum = Executor::new(&src, &dest, &mut m, o)
        .run(&p, no_stop())
        .await
        .unwrap();
    assert_eq!(sum.outcome, RunOutcome::Completed);
    assert_eq!(sum.files_mirrored, 24);
    assert_eq!(sum.files_failed, 1);
    assert_eq!(env.staged_entries(), 0, "staging is emptied");
    // Uploads still happen one at a time, in plan order.
    let up = uploads(&dest);
    let mut sorted = up.clone();
    sorted.sort();
    assert_eq!(up, sorted);
    for (i, id) in ids.iter().enumerate() {
        let r = row(&env, id);
        if r.state == ItemState::Failed {
            continue;
        }
        assert_eq!(r.verify_level, VerifyLevel::L3RemoteClaimed);
        assert_eq!(
            dest.content(&format!("OneDrive/Docs/f{i:02}.txt")).unwrap(),
            format!("file {i}").as_bytes()
        );
    }

    // After the backoff, the last one goes too, and nothing twice.
    env.advance(3600);
    let p = plan_with(&src, &dest, &env, &popts()).await;
    let mut m = env.manifest();
    let mut o = env.opts();
    o.parallel = 6;
    let sum = Executor::new(&src, &dest, &mut m, o)
        .run(&p, no_stop())
        .await
        .unwrap();
    assert_eq!(sum.files_mirrored, 1);
    assert_eq!(uploads(&dest).len(), 25);
}

#[tokio::test]
async fn a_stop_while_fetching_ahead_leaves_nothing_staged() {
    let env = Env::new();
    let src = MemSource::new();
    for i in 0..10 {
        src.add_file(MEM_ROOT, &format!("f{i}"), b"data");
    }
    let dest = MemDestination::new();
    let p = plan_with(&src, &dest, &env, &popts()).await;
    let (tx, rx) = watch::channel(true);
    let mut m = env.manifest();
    let mut o = env.opts();
    o.parallel = 4;
    let sum = Executor::new(&src, &dest, &mut m, o)
        .run(&p, rx)
        .await
        .unwrap();
    drop(tx);
    assert_eq!(sum.outcome, RunOutcome::Cancelled);
    assert_eq!(uploads(&dest).len(), 0);
    assert_eq!(env.staged_entries(), 0);
}

#[tokio::test]
async fn a_local_folder_gets_the_files_with_their_times() {
    use axentra_clouddrive_provider::LocalFolderDestination;
    let env = Env::new();
    let src = MemSource::new();
    let t = tree(&src);
    let drive = tempfile::tempdir().unwrap();
    let dest = LocalFolderDestination::new(drive.path()).unwrap();
    let mut o = popts();
    o.dest_root = "OneDrive Mirror".into();
    o.round_trip = true;
    let p = plan_with(&src, &dest, &env, &o).await;
    let mut m = env.manifest();
    let mut x = env.opts();
    x.parallel = 3;
    let sum = Executor::new(&src, &dest, &mut m, x)
        .run(&p, no_stop())
        .await
        .unwrap();
    assert_eq!(sum.outcome, RunOutcome::Completed);
    assert_eq!(sum.files_mirrored, 4);
    let b = drive.path().join("OneDrive Mirror/Docs/Deep/b.bin");
    assert_eq!(std::fs::read(&b).unwrap(), vec![7u8; 300]);
    let want = src.item(&t.b).await.unwrap().modified.unwrap();
    let got: DateTime<Utc> = std::fs::metadata(&b).unwrap().modified().unwrap().into();
    assert_eq!(got.timestamp(), want.timestamp());
    assert_eq!(row(&env, &t.top).verify_level, VerifyLevel::L4RoundTrip);

    // Again: nothing is written.
    let (p, sum) = mirror(&src, &dest, &env, &o).await;
    assert_eq!(p.count(Action::AlreadyMirrored), 8);
    assert_eq!(sum.files_mirrored, 0);

    // A file already there with other content is left alone.
    let other = src.add_file(MEM_ROOT, "clash.txt", b"from onedrive");
    std::fs::write(drive.path().join("OneDrive Mirror/clash.txt"), b"mine").unwrap();
    let (_, sum) = mirror(&src, &dest, &env, &o).await;
    assert_eq!(sum.files_diverged, 1);
    assert_eq!(row(&env, &other).state, ItemState::Diverged);
    assert_eq!(
        std::fs::read(drive.path().join("OneDrive Mirror/clash.txt")).unwrap(),
        b"mine"
    );
}

/// A source whose downloads take several network round trips, like
/// OneDrive's (look up, redirect, body).
struct SlowSource(MemSource);

impl SourceProvider for SlowSource {
    fn capabilities(&self) -> Capabilities {
        self.0.capabilities()
    }
    async fn account(&self) -> PResult<Account> {
        self.0.account().await
    }
    async fn change_page(&self, cursor: Option<&str>) -> PResult<ChangePage> {
        self.0.change_page(cursor).await
    }
    async fn item(&self, id: &str) -> PResult<SourceItem> {
        self.0.item(id).await
    }
    async fn download_to(&self, item: &SourceItem, dest: &Path) -> PResult<ContentDigests> {
        for _ in 0..5 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        self.0.download_to(item, dest).await
    }
}

/// A destination that takes a while to write each file.
struct SlowDest(MemDestination);

impl MirrorDestination for SlowDest {
    fn capabilities(&self) -> Capabilities {
        self.0.capabilities()
    }
    async fn account(&self) -> PResult<Account> {
        self.0.account().await
    }
    async fn list(&self, folder: &RelPath) -> PResult<Vec<DestEntry>> {
        self.0.list(folder).await
    }
    async fn ensure_folder(&self, folder: &RelPath) -> PResult<String> {
        self.0.ensure_folder(folder).await
    }
    async fn upload_new(&self, folder: &RelPath, name: &str, local: &Path) -> PResult<RemoteStat> {
        tokio::time::sleep(Duration::from_millis(30)).await;
        self.0.upload_new(folder, name, local).await
    }
    async fn upload_revision(&self, file: &RelPath, local: &Path) -> PResult<RemoteStat> {
        self.0.upload_revision(file, local).await
    }
    async fn stat(&self, file: &RelPath) -> PResult<RemoteStat> {
        self.0.stat(file).await
    }
    async fn download_to(&self, file: &RelPath, dest: &Path) -> PResult<ContentDigests> {
        self.0.download_to(file, dest).await
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn downloads_ahead_keep_going_while_a_file_is_written() {
    let env = Env::new();
    let src = SlowSource(MemSource::new());
    for i in 0..20 {
        src.0.add_file(MEM_ROOT, &format!("f{i:02}"), b"data");
    }
    let dest = SlowDest(MemDestination::new());
    let p = plan_with(&src, &dest, &env, &popts()).await;
    let mut m = env.manifest();
    let mut o = env.opts();
    o.parallel = 8;
    let t = std::time::Instant::now();
    let sum = Executor::new(&src, &dest, &mut m, o)
        .run(&p, no_stop())
        .await
        .unwrap();
    let took = t.elapsed();
    assert_eq!(sum.files_mirrored, 20);
    // Writing alone takes 20 x 30 ms; each download (5 x 10 ms) should
    // overlap it. Done one after the other it would be 20 x 80 ms.
    assert!(took < Duration::from_millis(1100), "took {took:?}");
}

#[tokio::test]
async fn trusting_the_plan_skips_lookups_and_still_catches_changes() {
    let env = Env::new();
    let src = MemSource::new();
    let same = src.add_file(MEM_ROOT, "same.txt", b"unchanged");
    let changed = src.add_file(MEM_ROOT, "changed.txt", b"first");
    let gone = src.add_file(MEM_ROOT, "gone.txt", b"bye");
    let dest = MemDestination::new();
    let p = plan_with(&src, &dest, &env, &popts()).await;
    // Changed and removed after planning.
    src.modify(&changed, b"second version");
    src.mark_deleted(&gone);
    let lookups = src.count(Op::Item);
    let mut m = env.manifest();
    let mut o = env.opts();
    o.parallel = 4;
    o.trust_plan = true;
    let sum = Executor::new(&src, &dest, &mut m, o)
        .run(&p, no_stop())
        .await
        .unwrap();
    assert_eq!(src.count(Op::Item), lookups, "no lookups when fetching ahead");
    assert_eq!(sum.files_mirrored, 1);
    assert_eq!(row(&env, &same).verify_level, VerifyLevel::L3RemoteClaimed);
    // Neither stale version reached the destination.
    assert_eq!(dest.content("OneDrive/changed.txt"), None);
    assert_eq!(dest.content("OneDrive/gone.txt"), None);
    assert_eq!(row(&env, &changed).state, ItemState::Failed);

    // The next run plans afresh and gets the new version.
    env.advance(3600);
    let (_, sum) = mirror(&src, &dest, &env, &popts()).await;
    assert_eq!(sum.files_mirrored, 1);
    assert_eq!(
        dest.content("OneDrive/changed.txt").unwrap(),
        b"second version"
    );
    assert_eq!(dest.content("OneDrive/gone.txt"), None);
}

#[tokio::test]
async fn a_rate_limit_can_be_waited_out_without_ending_the_run() {
    let env = Env::new();
    let src = MemSource::new();
    for i in 0..5 {
        src.add_file(MEM_ROOT, &format!("f{i}"), b"data");
    }
    src.fail_next(Op::Download, ProviderError::RateLimited { retry_after: None });
    let dest = MemDestination::new();
    let p = plan_with(&src, &dest, &env, &popts()).await;
    let mut m = env.manifest();
    let mut o = env.opts();
    o.parallel = 2;
    o.backoff_base = Duration::from_secs(1);
    o.wait_out_rate_limits = true;
    let sum = Executor::new(&src, &dest, &mut m, o)
        .run(&p, no_stop())
        .await
        .unwrap();
    assert_eq!(sum.outcome, RunOutcome::Completed);
    assert_eq!(sum.files_mirrored, 4);
    assert_eq!(sum.files_waiting, 1);
}

#[tokio::test]
async fn files_already_on_this_computer_are_taken_instead_of_downloaded() {
    use axentra_clouddrive_mirror::SeedIndex;
    let env = Env::new();
    let src = MemSource::new();
    let docs = src.add_folder(MEM_ROOT, "Docs");
    src.add_file(&docs, "a.js", b"module a");
    src.add_file(&docs, "b.js", b"module b");
    src.add_file(&docs, "c.js", b"module c");
    // A local copy laid out differently, with one stale file.
    let seed = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(seed.path().join("old/Docs")).unwrap();
    std::fs::write(seed.path().join("old/Docs/a.js"), b"module a").unwrap();
    std::fs::write(seed.path().join("b.js"), b"module b").unwrap();
    std::fs::write(seed.path().join("old/Docs/c.js"), b"module X").unwrap();
    let index = SeedIndex::scan(&[seed.path().to_path_buf()], &|_| {}).unwrap();

    let dest = MemDestination::new();
    let p = plan_with(&src, &dest, &env, &popts()).await;
    let downloads = src.count(Op::Download);
    let mut m = env.manifest();
    let mut o = env.opts();
    o.parallel = 3;
    o.trust_plan = true;
    o.seed = Some(Arc::new(index));
    let sum = Executor::new(&src, &dest, &mut m, o)
        .run(&p, no_stop())
        .await
        .unwrap();
    assert_eq!(sum.files_mirrored, 3);
    assert_eq!(sum.files_seeded, 2);
    assert_eq!(src.count(Op::Download) - downloads, 1, "only the stale one is downloaded");
    assert_eq!(dest.content("OneDrive/Docs/c.js").unwrap(), b"module c");
    assert_eq!(dest.content("OneDrive/Docs/a.js").unwrap(), b"module a");
    assert_eq!(std::fs::read(seed.path().join("old/Docs/c.js")).unwrap(), b"module X");
}
