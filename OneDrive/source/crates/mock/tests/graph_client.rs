//! The Graph client against the mock: resuming upload sessions and keeping
//! the sign-in fresh.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axentra_onedrive_graph::{AuthConfig, DriveItem, GResult, Graph, TokenStore, UploadTarget};
use axentra_onedrive_mock::MockServer;

const MIB: usize = 1024 * 1024;

fn auth(mock: &MockServer) -> AuthConfig {
    AuthConfig {
        client_id: "test".into(),
        tenant: "common".into(),
        authority: mock.origin.clone(),
    }
}

/// `len` bytes that differ with `seed`, so a mix-up of two versions shows.
fn content(len: usize, seed: u8) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8 ^ seed).collect()
}

struct Env {
    mock: MockServer,
    graph: Graph,
    dir: tempfile::TempDir,
}

impl Env {
    async fn new() -> Self {
        let mock = MockServer::start().await;
        let dir = tempfile::tempdir().unwrap();
        let tokens = dir.path().join("tokens.json");
        std::fs::write(&tokens, mock.tokens_json(false)).unwrap();
        let tokens = Arc::new(TokenStore::open(tokens, auth(&mock)));
        let graph = Graph::new(&mock.graph_base, tokens);
        Env { mock, graph, dir }
    }

    fn write(&self, data: &[u8]) -> PathBuf {
        let path = self.dir.path().join("big.bin");
        std::fs::write(&path, data).unwrap();
        path
    }

    async fn upload_new(&self, path: &Path) -> GResult<DriveItem> {
        let root = self.mock.id("/").unwrap();
        let target = UploadTarget::New {
            parent_id: &root,
            name: "big.bin",
        };
        self.graph.upload(target, path, None, None, &|_| {}).await
    }

    /// A new version of `item`, made against the eTag it had then.
    async fn upload_over(&self, item: &DriveItem, path: &Path) -> GResult<DriveItem> {
        let target = UploadTarget::Existing(&item.id);
        let tag = item.e_tag.as_deref();
        self.graph.upload(target, path, None, tag, &|_| {}).await
    }

    fn uploaded_bytes(&self) -> usize {
        self.mock.with(|d| d.upload_bytes) as usize
    }

    /// Start a 25 MiB upload that fails after its first 10 MiB fragment.
    async fn interrupted(&self, data: &[u8]) -> PathBuf {
        let path = self.write(data);
        self.cut_short(self.upload_new(&path)).await;
        path
    }

    /// Run a 25 MiB `upload` that fails after its first 10 MiB fragment.
    async fn cut_short(&self, upload: impl Future<Output = GResult<DriveItem>>) {
        self.mock
            .with(|d| d.fail_fragments_from = Some(10 * MIB as u64));
        let err = upload.await.unwrap_err();
        assert_eq!(err.status(), Some(503), "{err}");
        assert_eq!(self.mock.upload_sessions(), 1, "the session is kept");
        self.mock.with(|d| d.fail_fragments_from = None);
    }
}

#[tokio::test]
async fn a_lost_reply_mid_upload_resumes_from_what_the_session_has() {
    let env = Env::new().await;
    let data = content(25 * MIB, 1);
    let path = env.write(&data);
    env.mock.with(|d| d.lose_replies_at = vec![10 * MIB as u64]);
    let item = env.upload_new(&path).await.unwrap();
    assert_eq!(item.size, Some(data.len() as u64));
    assert_eq!(env.mock.content("/big.bin").unwrap(), data);
    // One session, and no byte sent twice.
    assert_eq!(env.mock.requests("createUploadSession"), 1);
    assert_eq!(env.uploaded_bytes(), data.len());
}

#[tokio::test]
async fn a_lost_reply_to_the_last_fragment_finds_the_finished_file() {
    let env = Env::new().await;
    let data = content(5 * MIB, 2);
    let path = env.write(&data);
    env.mock.with(|d| d.lose_replies_at = vec![0]);
    let item = env.upload_new(&path).await.unwrap();
    assert_eq!(env.mock.list("/"), vec!["big.bin"]);
    assert_eq!(env.mock.content("/big.bin").unwrap(), data);
    assert_eq!(env.mock.requests("createUploadSession"), 1);

    // The same for a new version of an existing file.
    let data = content(6 * MIB, 3);
    std::fs::write(&path, &data).unwrap();
    env.mock.with(|d| d.lose_replies_at = vec![0]);
    let target = UploadTarget::Existing(&item.id);
    let again = env
        .graph
        .upload(target, &path, None, item.e_tag.as_deref(), &|_| {})
        .await
        .unwrap();
    assert_eq!(again.id, item.id);
    assert_eq!(env.mock.content("/big.bin").unwrap(), data);
    assert_eq!(env.mock.version_count("/big.bin"), 2);
    assert_eq!(env.mock.requests("createUploadSession"), 2);
}

#[tokio::test]
async fn an_interrupted_upload_resumes_on_the_next_attempt() {
    let env = Env::new().await;
    let data = content(25 * MIB, 4);
    let path = env.interrupted(&data).await;
    env.upload_new(&path).await.unwrap();
    assert_eq!(env.mock.content("/big.bin").unwrap(), data);
    assert_eq!(env.mock.requests("createUploadSession"), 1);
    assert_eq!(env.uploaded_bytes(), data.len());
}

#[tokio::test]
async fn an_expired_upload_session_starts_over() {
    let env = Env::new().await;
    let data = content(25 * MIB, 5);
    let path = env.interrupted(&data).await;
    env.mock.expire_upload_sessions();
    env.upload_new(&path).await.unwrap();
    assert_eq!(env.mock.content("/big.bin").unwrap(), data);
    assert_eq!(env.mock.requests("createUploadSession"), 2);
    assert_eq!(env.uploaded_bytes(), 10 * MIB + data.len());
}

#[tokio::test]
async fn a_file_changed_since_the_interruption_starts_over() {
    let env = Env::new().await;
    let path = env.interrupted(&content(25 * MIB, 6)).await;
    // Same size and no mtime given: only the change time tells. It comes
    // from a coarse clock, so let it tick first.
    tokio::time::sleep(Duration::from_millis(20)).await;
    let changed = content(25 * MIB, 7);
    std::fs::write(&path, &changed).unwrap();
    env.upload_new(&path).await.unwrap();
    assert_eq!(env.mock.content("/big.bin").unwrap(), changed);
    assert_eq!(env.mock.requests("createUploadSession"), 2);
    assert_eq!(
        env.mock.upload_sessions(),
        0,
        "the stale session is cancelled"
    );
}

#[tokio::test]
async fn an_interrupted_new_version_resumes_on_the_next_attempt() {
    let env = Env::new().await;
    let path = env.write(&content(5 * MIB, 8));
    let item = env.upload_new(&path).await.unwrap();
    let data = content(25 * MIB, 9);
    std::fs::write(&path, &data).unwrap();
    env.cut_short(env.upload_over(&item, &path)).await;
    let before = env.uploaded_bytes();
    env.upload_over(&item, &path).await.unwrap();
    assert_eq!(env.mock.content("/big.bin").unwrap(), data);
    assert_eq!(env.mock.requests("createUploadSession"), 2);
    assert_eq!(env.uploaded_bytes() - before, data.len() - 10 * MIB);
}

#[tokio::test]
async fn an_edit_made_online_since_the_interruption_is_a_conflict() {
    let env = Env::new().await;
    let path = env.write(&content(5 * MIB, 10));
    let item = env.upload_new(&path).await.unwrap();
    std::fs::write(&path, content(25 * MIB, 11)).unwrap();
    env.cut_short(env.upload_over(&item, &path)).await;
    // Someone edits the file online before the upload can carry on.
    let theirs = content(1000, 12);
    env.mock.modify("/big.bin", &theirs);
    let err = env.upload_over(&item, &path).await.unwrap_err();
    assert_eq!(err.status(), Some(412), "{err}");
    assert_eq!(env.mock.content("/big.bin").unwrap(), theirs);
    assert_eq!(
        env.mock.upload_sessions(),
        0,
        "the stale session is cancelled"
    );
}

#[tokio::test]
async fn a_kept_session_whose_last_fragment_landed_is_not_a_conflict() {
    let env = Env::new().await;
    let path = env.write(&content(5 * MIB, 13));
    let item = env.upload_new(&path).await.unwrap();
    let data = content(25 * MIB, 14);
    std::fs::write(&path, &data).unwrap();
    // The last fragment lands but its reply is lost, and so is every
    // attempt to look at the item after it: the session is kept.
    env.mock.with(|d| d.lose_replies_at = vec![20 * MIB as u64]);
    let progress = |done: u64| {
        if done == 20 * MIB as u64 {
            env.mock.with(|d| d.fail_next = Some((503, 6)));
        }
    };
    let target = UploadTarget::Existing(&item.id);
    let err = env
        .graph
        .upload(target, &path, None, item.e_tag.as_deref(), &progress)
        .await
        .unwrap_err();
    assert_eq!(err.status(), Some(503), "{err}");
    // The changed eTag is our own upload, not someone else's edit.
    let again = env.upload_over(&item, &path).await.unwrap();
    assert_eq!(again.id, item.id);
    assert_eq!(env.mock.content("/big.bin").unwrap(), data);
    assert_eq!(env.mock.requests("createUploadSession"), 2);
    assert_eq!(env.mock.version_count("/big.bin"), 2);
}

#[tokio::test]
async fn a_cancelled_upload_resumes_on_the_next_attempt() {
    let env = Env::new().await;
    let data = content(25 * MIB, 15);
    let path = env.write(&data);
    // Cancelled (syncing paused, say) while its second fragment goes up.
    env.mock
        .with(|d| d.hold = Some(format!("bytes {}-", 10 * MIB)));
    tokio::select! {
        r = env.upload_new(&path) => panic!("not held: {r:?}"),
        () = held(&env) => {}
    }
    // The fragment held then isn't taken once let go.
    env.mock.with(|d| {
        d.fail_fragments_from = Some(10 * MIB as u64);
        d.hold = None;
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    env.mock.with(|d| d.fail_fragments_from = None);
    assert_eq!(env.mock.upload_sessions(), 1, "the session is kept");

    env.upload_new(&path).await.unwrap();
    assert_eq!(env.mock.content("/big.bin").unwrap(), data);
    assert_eq!(env.mock.requests("createUploadSession"), 1);
    assert_eq!(env.uploaded_bytes(), data.len());
}

#[tokio::test]
async fn uploads_no_longer_wanted_cancel_their_sessions() {
    let env = Env::new().await;
    let path = env.interrupted(&content(25 * MIB, 16)).await;
    let root = env.mock.id("/").unwrap();
    let kept = UploadTarget::New {
        parent_id: &root,
        name: "big.bin",
    };
    env.graph.forget_uploads(&[kept]);
    assert_eq!(env.mock.upload_sessions(), 1);
    // Renamed or deleted since, say.
    env.graph.forget_uploads(&[]);
    for _ in 0..100 {
        if env.mock.upload_sessions() == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(env.mock.upload_sessions(), 0, "cancelled online");
    env.upload_new(&path).await.unwrap();
    assert_eq!(env.mock.requests("createUploadSession"), 2);
}

#[tokio::test]
async fn what_an_upload_sent_is_known_until_one_goes_through() {
    let env = Env::new().await;
    let root = env.mock.id("/").unwrap();
    let target = UploadTarget::New {
        parent_id: &root,
        name: "big.bin",
    };
    let hash = |data: &[u8]| {
        let mut h = axentra_onedrive_graph::QuickXorHash::new();
        h.update(data);
        h.base64()
    };
    // Small files in one request, big ones in fragments.
    for (len, seed) in [(1000, 17), (25 * MIB, 18)] {
        let data = content(len, seed);
        let path = env.write(&data);
        env.mock.with(|d| d.fail_next = Some((403, 1)));
        env.upload_new(&path).await.unwrap_err();
        let sent = env.graph.sent_before(target, &hash(&data));
        // A session sends nothing before it's made.
        assert_eq!(sent, len < 4 * MIB, "{len}");
    }
    let data = content(25 * MIB, 19);
    let path = env.interrupted(&data).await;
    assert!(
        !env.graph.sent_before(target, &hash(&data)),
        "only part of it"
    );
    // Known once the last fragment is on its way.
    env.mock
        .with(|d| d.hold = Some(format!("bytes {}-", 20 * MIB)));
    let upload = env.upload_new(&path);
    tokio::pin!(upload);
    tokio::select! {
        r = &mut upload => panic!("not held: {r:?}"),
        () = held(&env) => {}
    }
    assert!(env.graph.sent_before(target, &hash(&data)));
    env.mock.with(|d| d.hold = None);
    upload.await.unwrap();
    // Until the caller has recorded that it went through: one stopped
    // before then (while the date of a small file is set, say) has landed.
    assert!(env.graph.sent_before(target, &hash(&data)));
    env.graph.forget_sent(target);
    assert!(
        !env.graph.sent_before(target, &hash(&data)),
        "it went through"
    );
}

/// Once a request is waiting on `hold`.
async fn held(env: &Env) {
    while env.mock.with(|d| d.held) == 0 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn a_version_saved_over_an_upload_while_it_is_dated_is_not_taken_for_it() {
    let env = Env::new().await;
    let data = content(1000, 20);
    let path = env.write(&data);
    let root = env.mock.id("/").unwrap();
    let target = UploadTarget::New {
        parent_id: &root,
        name: "big.bin",
    };
    let then = chrono::DateTime::from_timestamp(1_600_000_000, 0).unwrap();
    env.mock.with(|d| d.hold = Some("PATCH ".into()));
    let upload = env.graph.upload(target, &path, Some(then), None, &|_| {});
    tokio::pin!(upload);
    tokio::select! {
        r = &mut upload => panic!("not held: {r:?}"),
        () = held(&env) => {}
    }
    // Saved from another PC while the date is on its way.
    env.mock.modify("/big.bin", b"theirs");
    env.mock.with(|d| d.hold = None);
    let item = upload.await.unwrap();
    let mut ours = axentra_onedrive_graph::QuickXorHash::new();
    ours.update(&data);
    assert_eq!(item.quick_xor_hash(), Some(ours.base64().as_str()));
    let online = env.graph.item(&item.id).await.unwrap();
    assert_ne!(online.c_tag, item.c_tag, "theirs is taken for ours");
}

#[tokio::test]
async fn refreshed_tokens_are_kept_when_they_cannot_be_saved() {
    let mock = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    std::fs::create_dir(&state).unwrap();
    let path = state.join("tokens.json");
    std::fs::write(&path, mock.tokens_json(true)).unwrap();
    let tokens = TokenStore::open(&path, auth(&mock));
    // Nothing can be saved there any more, as when the disk is full.
    std::fs::remove_dir_all(&state).unwrap();
    std::fs::write(&state, b"").unwrap();
    assert_eq!(tokens.access_token().await.unwrap(), "mock-access-1");
    // Kept in memory, so no second refresh.
    assert_eq!(tokens.access_token().await.unwrap(), "mock-access-1");
}

#[tokio::test]
async fn a_late_401_for_a_replaced_token_does_not_refresh_again() {
    let mock = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tokens.json");
    std::fs::write(&path, mock.tokens_json(false)).unwrap();
    let tokens = TokenStore::open(&path, auth(&mock));
    assert_eq!(tokens.access_token().await.unwrap(), "mock-access-0");
    // Two requests sent with token 0 are refused. The first one's refresh...
    tokens.invalidate("mock-access-0").await;
    assert_eq!(tokens.access_token().await.unwrap(), "mock-access-1");
    // ...covers the second.
    tokens.invalidate("mock-access-0").await;
    assert_eq!(tokens.access_token().await.unwrap(), "mock-access-1");
    tokens.invalidate("mock-access-1").await;
    assert_eq!(tokens.access_token().await.unwrap(), "mock-access-2");
}
