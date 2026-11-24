//! `GraphSource` against the mock Graph server: paging the change feed, a
//! stale cursor, verified downloads, and never a request that changes
//! anything online.

use std::path::Path;
use std::sync::Arc;

use axentra_clouddrive_provider::{
    ContentDigests, GraphSource, ItemKind, ProviderError, RelPath, SourceItem, SourceProvider,
};
use axentra_onedrive_graph::{AuthConfig, Graph, TokenStore};
use axentra_onedrive_mock::MockServer;

struct Env {
    mock: MockServer,
    source: GraphSource,
    /// Staging folder for downloads.
    stage: tempfile::TempDir,
    _dir: tempfile::TempDir,
}

impl Env {
    async fn new() -> Self {
        Self::signed_in(true).await
    }

    async fn signed_in(signed_in: bool) -> Self {
        let mock = MockServer::start().await;
        let dir = tempfile::tempdir().unwrap();
        let tokens = dir.path().join("tokens.json");
        if signed_in {
            std::fs::write(&tokens, mock.tokens_json(false)).unwrap();
        }
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
            stage: tempfile::tempdir().unwrap(),
            _dir: dir,
        }
    }

    /// Every page of one round of the feed: the items and how many pages.
    async fn round(&self, cursor: Option<&str>) -> (Vec<SourceItem>, usize, String) {
        let mut items = Vec::new();
        let mut pages = 0;
        let mut cursor = cursor.map(str::to_string);
        loop {
            let page = self.source.change_page(cursor.as_deref()).await.unwrap();
            pages += 1;
            items.extend(page.items);
            match (page.next, page.resume) {
                (Some(next), None) => cursor = Some(next),
                (None, Some(resume)) => return (items, pages, resume),
                other => panic!("a page needs exactly one of next and resume: {other:?}"),
            }
        }
    }

    async fn item_at(&self, path: &str) -> SourceItem {
        let id = self.mock.id(path).unwrap();
        self.source.item(&id).await.unwrap()
    }

    fn staged(&self) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(self.stage.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    /// Only GET requests reached the server.
    fn assert_read_only(&self) {
        let log = self.mock.with(|d| d.log.clone());
        assert!(!log.is_empty());
        for method in ["DELETE", "PATCH", "PUT", "POST"] {
            assert_eq!(self.mock.requests(&format!("{method} ")), 0, "{log:#?}");
        }
        assert!(log.iter().all(|l| l.starts_with("GET ")), "{log:#?}");
    }
}

fn names(items: &[SourceItem]) -> Vec<&str> {
    let mut v: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
    v.sort();
    v
}

#[tokio::test]
async fn account_comes_from_the_drive_and_user() {
    let env = Env::new().await;
    let acct = env.source.account().await.unwrap();
    assert_eq!(acct.provider, "onedrive");
    assert_eq!(acct.id, "DRIVE1");
    assert_eq!(acct.email.as_deref(), Some("test.user@example.com"));
    assert_eq!(acct.kind.as_deref(), Some("personal"));
    env.assert_read_only();
}

#[tokio::test]
async fn the_change_feed_pages_and_resumes() {
    let env = Env::new().await;
    env.mock.with(|d| d.page_size = 2);
    env.mock.add_folder("/", "Docs");
    for n in 0..4 {
        env.mock
            .add_file("/Docs", &format!("{n}.txt"), format!("file {n}").as_bytes());
    }

    let (items, pages, resume) = env.round(None).await;
    // Root, folder and four files, two to a page.
    assert_eq!(items.len(), 6);
    assert_eq!(pages, 3);
    assert_eq!(
        names(&items),
        ["0.txt", "1.txt", "2.txt", "3.txt", "Docs", "root"]
    );
    let root = items.iter().find(|i| i.parent_id.is_none()).unwrap();
    assert_eq!(root.kind, ItemKind::Folder);
    let docs = items.iter().find(|i| i.name == "Docs").unwrap();
    let file = items.iter().find(|i| i.name == "2.txt").unwrap();
    assert_eq!(file.kind, ItemKind::File);
    assert_eq!(file.parent_id.as_deref(), Some(docs.id.as_str()));
    assert_eq!(file.digests.size, 6);
    assert_eq!(
        file.digests.qxh,
        ContentDigests::of_bytes(b"file 2").qxh,
        "the hash OneDrive claims is carried through"
    );
    assert!(file.version.is_some());

    // Nothing new: an empty round.
    let (items, _, resume) = env.round(Some(&resume)).await;
    assert!(items.is_empty(), "{items:?}");

    // Only what changed since, a deletion included.
    let one = env.mock.id("/Docs/1.txt").unwrap();
    env.mock.add_file("/Docs", "new.txt", b"new");
    env.mock.modify("/Docs/0.txt", b"edited");
    env.mock.delete("/Docs/1.txt");
    let (items, _, _) = env.round(Some(&resume)).await;
    assert_eq!(items.len(), 3, "{items:?}");
    let gone: Vec<&SourceItem> = items.iter().filter(|i| i.deleted).collect();
    assert_eq!(gone.len(), 1);
    assert_eq!(gone[0].id, one);
    assert!(items.iter().any(|i| i.name == "new.txt" && !i.deleted));
    let edited = items.iter().find(|i| i.name == "0.txt").unwrap();
    assert_eq!(edited.digests.size, 6);
    env.assert_read_only();
}

#[tokio::test]
async fn a_stale_cursor_asks_for_a_resync() {
    let env = Env::new().await;
    env.mock.add_file("/", "a.txt", b"a");
    let (_, _, resume) = env.round(None).await;
    env.mock.add_file("/", "b.txt", b"b");
    env.mock.expire_delta_tokens();

    let err = env.source.change_page(Some(&resume)).await.unwrap_err();
    assert!(matches!(err, ProviderError::ResyncRequired), "{err}");
    assert!(!err.is_retryable());
    // Starting over works and sees everything.
    let (items, _, _) = env.round(None).await;
    assert_eq!(names(&items), ["a.txt", "b.txt", "root"]);
    env.assert_read_only();
}

#[tokio::test]
async fn a_download_is_verified_then_put_in_place() {
    let env = Env::new().await;
    let data: Vec<u8> = (0..3 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
    env.mock.add_file("/", "big.bin", &data);
    let item = env.item_at("/big.bin").await;

    let dest = env.stage.path().join("Ünïcødé 日本語.bin");
    let got = env.source.download_to(&item, &dest).await.unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), data);
    assert_eq!(got.size, data.len() as u64);
    assert_eq!(got.qxh, item.digests.qxh);
    assert_eq!(got, ContentDigests::of_bytes(&data));
    // No temporary file is left behind.
    assert_eq!(env.staged(), ["Ünïcødé 日本語.bin"]);

    // Never over something already there.
    let err = env.source.download_to(&item, &dest).await.unwrap_err();
    assert!(matches!(err, ProviderError::Conflict(_)), "{err}");
    assert_eq!(env.staged(), ["Ünïcødé 日本語.bin"]);
    env.assert_read_only();
}

#[tokio::test]
async fn a_corrupted_byte_is_rejected_and_nothing_is_kept() {
    let env = Env::new().await;
    let data = b"the quick brown fox jumps over the lazy dog".to_vec();
    env.mock.add_file("/", "fox.txt", &data);
    let item = env.item_at("/fox.txt").await;

    // The bytes that arrive differ from the hash OneDrive gave by one bit,
    // at the same size. (The mock serves exactly what it hashes, so the
    // stored content is changed after the item was looked up.)
    let mut bad = data.clone();
    bad[10] ^= 0x01;
    env.mock.modify("/fox.txt", &bad);
    let dest = env.stage.path().join("fox.txt");
    let err = env.source.download_to(&item, &dest).await.unwrap_err();
    assert!(matches!(err, ProviderError::Integrity(_)), "{err}");
    assert!(!err.is_retryable());
    assert!(!dest.exists());
    assert!(env.staged().is_empty(), "{:?}", env.staged());
    assert_eq!(env.mock.downloads(), 1);

    // The same when the expected hash is wrong instead.
    let mut item = env.item_at("/fox.txt").await;
    item.digests.qxh = ContentDigests::of_bytes(&data).qxh;
    let err = env.source.download_to(&item, &dest).await.unwrap_err();
    assert!(matches!(err, ProviderError::Integrity(_)), "{err}");
    assert!(env.staged().is_empty());

    // With the right hash it goes through.
    let item = env.item_at("/fox.txt").await;
    env.source.download_to(&item, &dest).await.unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), bad);
    env.assert_read_only();
}

#[tokio::test]
async fn no_hash_means_no_download() {
    let env = Env::new().await;
    env.mock.add_file("/", "a.txt", b"a");
    let mut item = env.item_at("/a.txt").await;
    item.digests.qxh = None;
    let dest = env.stage.path().join("a.txt");
    let err = env.source.download_to(&item, &dest).await.unwrap_err();
    assert!(matches!(err, ProviderError::Integrity(_)), "{err}");
    assert_eq!(env.mock.downloads(), 0, "fails before asking for the bytes");
    assert!(env.staged().is_empty());

    // Folders aren't downloaded at all.
    let root = env.source.item(&env.mock.id("/").unwrap()).await.unwrap();
    assert!(env
        .source
        .download_to(&root, &env.stage.path().join("root"))
        .await
        .is_err());
    assert!(env.staged().is_empty());
}

#[tokio::test]
async fn graph_errors_map_to_what_the_caller_can_do() {
    let env = Env::new().await;
    let err = env.source.item("NOSUCHITEM").await.unwrap_err();
    assert!(matches!(err, ProviderError::NotFound(_)), "{err}");

    // Past Graph's own retries, a server fault is temporary, and so are a
    // request timeout and OneDrive's bandwidth limit.
    for status in [503, 408, 509] {
        env.mock.with(|d| d.fail_next = Some((status, 10)));
        let err = env.source.change_page(None).await.unwrap_err();
        assert!(
            matches!(err, ProviderError::Transient(_)),
            "{status}: {err}"
        );
        assert!(err.is_retryable());
    }
    env.mock.with(|d| d.fail_next = Some((429, 10)));
    let err = env.source.account().await.unwrap_err();
    assert!(matches!(err, ProviderError::RateLimited { .. }), "{err}");
    env.mock.with(|d| d.fail_next = None);

    // A download that fails leaves no temporary file.
    env.mock.add_file("/", "a.txt", b"a");
    let item = env.item_at("/a.txt").await;
    env.mock.delete("/a.txt");
    let err = env
        .source
        .download_to(&item, &env.stage.path().join("a.txt"))
        .await
        .unwrap_err();
    assert!(matches!(err, ProviderError::NotFound(_)), "{err}");
    assert!(env.staged().is_empty());
    env.assert_read_only();
}

#[tokio::test]
async fn signed_out_is_auth_required() {
    let env = Env::signed_in(false).await;
    let err = env.source.change_page(None).await.unwrap_err();
    assert!(matches!(err, ProviderError::AuthRequired), "{err}");
}

#[tokio::test]
async fn a_whole_read_session_sends_only_gets() {
    let env = Env::new().await;
    env.mock.with(|d| d.page_size = 3);
    env.mock.add_folder("/", "A");
    for n in 0..5 {
        env.mock
            .add_file("/A", &format!("{n}.bin"), &vec![n as u8; 1000 + n]);
    }
    env.source.account().await.unwrap();
    let (items, _, resume) = env.round(None).await;
    for item in items.iter().filter(|i| i.kind == ItemKind::File) {
        let fresh = env.source.item(&item.id).await.unwrap();
        let dest = env.stage.path().join(&item.name);
        env.source.download_to(&fresh, &dest).await.unwrap();
    }
    env.mock.delete("/A/0.bin");
    env.mock.expire_delta_tokens();
    let _ = env.source.change_page(Some(&resume)).await;
    let _ = env.source.item("NOSUCHITEM").await;
    assert_eq!(env.mock.downloads(), 5);
    assert_eq!(env.staged().len(), 5);
    assert!(Path::new(&env.stage.path().join("4.bin")).exists());
    env.assert_read_only();
}

#[tokio::test]
async fn a_subtree_is_listed_page_by_page_without_the_change_feed() {
    let env = Env::new().await;
    env.mock.with(|d| d.page_size = 2);
    env.mock.add_folder("/", "Mirror Test");
    env.mock.add_folder("/Mirror Test", "Sub");
    env.mock.add_folder("/Mirror Test/Sub", "Deeper");
    for n in 0..5 {
        env.mock.add_file(
            "/Mirror Test",
            &format!("{n}.txt"),
            format!("file {n}").as_bytes(),
        );
    }
    env.mock.add_file("/Mirror Test/Sub", "b.txt", b"bravo");
    env.mock
        .add_file("/Mirror Test/Sub/Deeper", "c.txt", b"charlie");
    env.mock.add_folder("/", "Elsewhere");
    env.mock.add_file("/Elsewhere", "x.txt", b"not listed");

    let seen = std::sync::Mutex::new(Vec::new());
    let path = RelPath::parse("Mirror Test").unwrap();
    let items = env
        .source
        .enumerate_subtree(&path, &|n| seen.lock().unwrap().push(n))
        .await
        .unwrap();
    // The root above it, the folder, and everything inside, at any depth.
    assert_eq!(
        names(&items),
        [
            "0.txt",
            "1.txt",
            "2.txt",
            "3.txt",
            "4.txt",
            "Deeper",
            "Mirror Test",
            "Sub",
            "b.txt",
            "c.txt",
            "root"
        ]
    );
    // Parents are named as the change feed names them.
    let (feed, _, _) = env.round(None).await;
    for it in &items {
        let same = feed.iter().find(|f| f.id == it.id).unwrap();
        assert_eq!(same, it);
    }
    let root = items.iter().find(|i| i.parent_id.is_none()).unwrap();
    assert_eq!(root.name, "root");
    let c = env.item_at("/Mirror Test/Sub/Deeper/c.txt").await;
    assert_eq!(items.iter().find(|i| i.id == c.id), Some(&c));
    assert_eq!(c.digests.size, 7);
    assert!(c.digests.qxh.is_some());
    // Seven items under the folder at two to a page: more than one page.
    assert!(env.mock.requests("children?skip=") >= 2);
    let seen = seen.into_inner().unwrap();
    assert_eq!(seen.last(), Some(&10));
    assert!(seen.windows(2).all(|w| w[0] <= w[1]), "{seen:?}");
    env.assert_read_only();
}

#[tokio::test]
async fn a_subtree_listing_never_reads_the_change_feed() {
    let env = Env::new().await;
    env.mock.add_folder("/", "A");
    env.mock.add_folder("/A", "B");
    env.mock.add_file("/A/B", "f.txt", b"f");
    let items = env
        .source
        .enumerate_subtree(&RelPath::parse("A/B").unwrap(), &|_| {})
        .await
        .unwrap();
    assert_eq!(names(&items), ["A", "B", "f.txt", "root"]);
    assert_eq!(env.mock.requests("/delta"), 0);

    // One file on its own.
    let items = env
        .source
        .enumerate_subtree(&RelPath::parse("A/B/f.txt").unwrap(), &|_| {})
        .await
        .unwrap();
    assert_eq!(names(&items), ["A", "B", "f.txt", "root"]);
    assert_eq!(env.mock.requests("/delta"), 0);
    env.assert_read_only();
}

#[tokio::test]
async fn a_missing_subtree_is_an_error_not_an_empty_listing() {
    let env = Env::new().await;
    env.mock.add_folder("/", "Mirror Test");
    env.mock.add_file("/Mirror Test", "f.txt", b"f");
    // OneDrive finds names ignoring case; the mirror wants the exact one.
    for missing in [
        "Nope",
        "Mirror Test/Nope",
        "mirror test",
        "Mirror Test/f.txt/inside",
    ] {
        let e = env
            .source
            .enumerate_subtree(&RelPath::parse(missing).unwrap(), &|_| {})
            .await
            .unwrap_err();
        assert!(matches!(e, ProviderError::NotFound(_)), "{missing}: {e}");
        assert!(e.to_string().contains(missing), "{e}");
    }
    assert_eq!(env.mock.requests("/delta"), 0);
    env.assert_read_only();
}
