//! The OneDrive mock server through the real `GraphSource`, mirrored into a
//! `MemDestination`: planning reads only, a run reaches the round trip, and
//! later edits, renames and deletions on OneDrive leave every file already
//! at the destination exactly as it was.

use std::sync::Arc;

use axentra_clouddrive_mirror::{
    plan, plan_with_progress, Action, ExecOptions, Executor, ItemState, Manifest, PlanOptions,
    PlanProgress, Readiness, RunOutcome,
};
use axentra_clouddrive_provider::{GraphSource, MemDestination, VerifyLevel};
use axentra_onedrive_graph::{AuthConfig, Graph, TokenStore};
use axentra_onedrive_mock::MockServer;
use tokio::sync::watch;

struct Env {
    mock: MockServer,
    source: GraphSource,
    dir: tempfile::TempDir,
}

impl Env {
    async fn new() -> Self {
        let mock = MockServer::start().await;
        let dir = tempfile::tempdir().unwrap();
        let tokens = dir.path().join("tokens.json");
        std::fs::write(&tokens, mock.tokens_json(false)).unwrap();
        let cfg = AuthConfig {
            client_id: "test".into(),
            tenant: "common".into(),
            authority: mock.origin.clone(),
        };
        let tokens = Arc::new(TokenStore::open(tokens, cfg));
        let graph = Arc::new(Graph::new(&mock.graph_base, tokens));
        Env {
            mock,
            source: GraphSource::new(graph),
            dir,
        }
    }

    fn manifest(&self) -> Manifest {
        Manifest::open(&self.dir.path().join("mirror.sqlite")).unwrap()
    }

    fn opts(&self) -> ExecOptions {
        let mut o = ExecOptions::new(self.dir.path().join("staging"));
        o.min_free_bytes = 0;
        o
    }

    /// Only GET requests reached the server.
    fn assert_read_only(&self) {
        for method in ["DELETE", "PATCH", "PUT", "POST"] {
            assert_eq!(self.mock.requests(&format!("{method} ")), 0);
        }
    }
}

fn opts() -> PlanOptions {
    PlanOptions {
        round_trip: true,
        staging_cap: 1 << 20,
        ..Default::default()
    }
}

#[tokio::test]
async fn onedrive_mock_to_memory_destination() {
    let env = Env::new().await;
    env.mock.with(|d| d.page_size = 3);
    env.mock.add_folder("/", "CloudDrive Mirror Test");
    env.mock.add_folder("/CloudDrive Mirror Test", "Sub");
    env.mock
        .add_file("/CloudDrive Mirror Test", "a.txt", b"alpha");
    env.mock
        .add_file("/CloudDrive Mirror Test/Sub", "b.txt", b"bravo");
    env.mock
        .add_file("/CloudDrive Mirror Test/Sub", "c.txt", b"charlie");
    env.mock.add_file("/", "outside.txt", b"not mirrored");
    let dest = MemDestination::new();
    let mut o = opts();
    o.source_prefix = Some("/CloudDrive Mirror Test".into());

    // Dry run: counts, and no change anywhere.
    let p = plan(&env.source, &dest, &env.manifest().snapshot().unwrap(), &o)
        .await
        .unwrap();
    assert_eq!(p.count(Action::CreateFolder), 2);
    assert_eq!(p.count(Action::UploadNew), 3);
    assert_eq!(p.entries.len(), 5);
    assert_eq!(dest.mutating_calls(), 0);
    env.assert_read_only();

    let mut m = env.manifest();
    let sum = Executor::new(&env.source, &dest, &mut m, env.opts())
        .run(&p, watch::channel(false).1)
        .await
        .unwrap();
    assert_eq!(sum.outcome, RunOutcome::Completed);
    assert_eq!(sum.files_mirrored, 3);
    assert_eq!(
        dest.content("OneDrive/CloudDrive Mirror Test/Sub/c.txt")
            .unwrap(),
        b"charlie"
    );
    let rep = Readiness::from_manifest(&m).unwrap();
    assert_eq!(rep.measured_round_trip.items, 3);
    drop(m);

    // Edit, rename and delete on OneDrive; the destination keeps it all.
    let snapshot: Vec<(String, Vec<u8>)> = dest
        .paths()
        .into_iter()
        .filter_map(|p| dest.content(&p).map(|c| (p, c)))
        .collect();
    let calls = dest.mutating_calls();
    env.mock
        .modify("/CloudDrive Mirror Test/a.txt", b"alpha, edited");
    env.mock
        .rename("/CloudDrive Mirror Test/Sub/b.txt", "b-renamed.txt");
    env.mock.delete("/CloudDrive Mirror Test/Sub/c.txt");
    let p = plan(&env.source, &dest, &env.manifest().snapshot().unwrap(), &o)
        .await
        .unwrap();
    assert_eq!(p.count(Action::Diverged), 1);
    assert_eq!(p.count(Action::SourceMoved), 1);
    assert_eq!(p.count(Action::SourceDeleted), 1);
    let mut m = env.manifest();
    Executor::new(&env.source, &dest, &mut m, env.opts())
        .run(&p, watch::channel(false).1)
        .await
        .unwrap();
    assert_eq!(dest.mutating_calls(), calls, "{:?}", dest.calls());
    for (path, content) in &snapshot {
        assert_eq!(dest.content(path).as_ref(), Some(content), "{path}");
        assert_eq!(dest.revisions(path).len(), 1);
    }
    let states: Vec<(ItemState, VerifyLevel)> = m
        .items()
        .unwrap()
        .into_iter()
        .filter(|r| r.source_path.ends_with(".txt"))
        .map(|r| (r.state, r.verify_level))
        .collect();
    assert!(states.contains(&(ItemState::Diverged, VerifyLevel::L4RoundTrip)));
    assert!(states.contains(&(ItemState::SourceDeleted, VerifyLevel::L4RoundTrip)));
    assert!(states.contains(&(ItemState::Verified, VerifyLevel::L4RoundTrip)));
    env.assert_read_only();
}

#[tokio::test]
async fn a_folder_plan_lists_only_that_folder_and_keeps_no_cursor() {
    let env = Env::new().await;
    env.mock.with(|d| d.page_size = 2);
    env.mock.add_folder("/", "CloudDrive Mirror Test");
    env.mock.add_folder("/CloudDrive Mirror Test", "Sub");
    env.mock
        .add_file("/CloudDrive Mirror Test", "a.txt", b"alpha");
    env.mock
        .add_file("/CloudDrive Mirror Test/Sub", "b.txt", b"bravo");
    env.mock
        .add_file("/CloudDrive Mirror Test/Sub", "c.txt", b"charlie");
    env.mock.add_folder("/", "Elsewhere");
    env.mock.add_file("/", "outside.txt", b"not mirrored");
    let dest = MemDestination::new();
    let mut o = opts();
    o.source_prefix = Some("/CloudDrive Mirror Test".into());

    let events = std::sync::Mutex::new(Vec::new());
    let p = plan_with_progress(
        &env.source,
        &dest,
        &env.manifest().snapshot().unwrap(),
        &o,
        &|e| events.lock().unwrap().push(e),
    )
    .await
    .unwrap();
    assert_eq!(
        env.mock.requests("/delta"),
        0,
        "{:#?}",
        env.mock.with(|d| d.log.clone())
    );
    assert!(env.mock.requests("/children") > 0);
    assert_eq!(p.count(Action::CreateFolder), 2);
    assert_eq!(p.count(Action::UploadNew), 3);
    assert_eq!(p.entries.len(), 5);
    assert!(p.entries.iter().all(|e| !e.source_path.contains("outside")));
    assert!(p.source_cursor.is_none());
    // Counts that only go up, then the destination.
    let events = events.into_inner().unwrap();
    assert_eq!(events.last(), Some(&PlanProgress::ListingDestination));
    let counts: Vec<u64> = events
        .iter()
        .filter_map(|e| match e {
            PlanProgress::SourceItems(n) => Some(*n),
            PlanProgress::ListingDestination => None,
        })
        .collect();
    // The folder and the four items in it.
    assert_eq!(counts.last(), Some(&5));
    assert!(counts.windows(2).all(|w| w[0] <= w[1]));
    env.assert_read_only();

    // A run of it leaves no change-feed cursor behind, so a later
    // whole-drive run can't take it to mean everything was seen.
    let mut m = env.manifest();
    let sum = Executor::new(&env.source, &dest, &mut m, env.opts())
        .run(&p, watch::channel(false).1)
        .await
        .unwrap();
    assert_eq!(sum.outcome, RunOutcome::Completed);
    assert_eq!(m.kv_get("source_cursor").unwrap(), None);
    drop(m);

    // Moved out of the folder: still on OneDrive, so recorded as moved,
    // not deleted. Deleted: recorded as deleted.
    env.mock
        .move_to("/CloudDrive Mirror Test/Sub/b.txt", "/Elsewhere");
    env.mock.delete("/CloudDrive Mirror Test/Sub/c.txt");
    let p = plan(&env.source, &dest, &env.manifest().snapshot().unwrap(), &o)
        .await
        .unwrap();
    assert_eq!(p.count(Action::SourceMoved), 1);
    assert_eq!(p.count(Action::SourceDeleted), 1);
    assert!(p.source_cursor.is_none());
    assert_eq!(env.mock.requests("/delta"), 0);

    // A whole-drive plan still reads the change feed and keeps its cursor.
    let whole = plan(
        &env.source,
        &dest,
        &env.manifest().snapshot().unwrap(),
        &opts(),
    )
    .await
    .unwrap();
    assert!(env.mock.requests("/delta") > 0);
    assert!(whole.source_cursor.is_some());
    let mut m = env.manifest();
    Executor::new(&env.source, &dest, &mut m, env.opts())
        .run(&whole, watch::channel(false).1)
        .await
        .unwrap();
    assert!(m.kv_get("source_cursor").unwrap().is_some());
    env.assert_read_only();
}

#[tokio::test]
async fn a_folder_plan_for_a_missing_folder_is_an_error() {
    let env = Env::new().await;
    env.mock.add_folder("/", "CloudDrive Mirror Test");
    let dest = MemDestination::new();
    let mut o = opts();
    o.source_prefix = Some("/No Such Folder".into());
    let e = plan(&env.source, &dest, &env.manifest().snapshot().unwrap(), &o)
        .await
        .unwrap_err();
    assert!(e.to_string().contains("No Such Folder"), "{e}");
    assert_eq!(env.mock.requests("/delta"), 0);
    assert_eq!(dest.mutating_calls(), 0);
    env.assert_read_only();
}
