//! The trait contracts, through the in-memory fakes: paging and cursors,
//! verified downloads, add-only destinations, capability refusals, fault
//! injection and call logs, and that no destructive method exists.

use std::path::Path;
use std::time::Duration;

use axentra_clouddrive_provider::{
    Account, Capabilities, Capability, ChangePage, ContentDigests, DestEntry, ItemKind,
    MemDestination, MemSource, MirrorDestination, Op, PResult, ProviderError, RelPath, RemoteStat,
    SourceItem, SourceProvider, VerifyLevel, MEM_ROOT,
};

fn rel(s: &str) -> RelPath {
    RelPath::parse(s).unwrap()
}

fn listing(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

/// Every page of one round.
async fn round(
    src: &impl SourceProvider,
    cursor: Option<&str>,
) -> (Vec<SourceItem>, usize, String) {
    let mut items = Vec::new();
    let mut pages = 0;
    let mut cursor = cursor.map(str::to_string);
    loop {
        let page = src.change_page(cursor.as_deref()).await.unwrap();
        pages += 1;
        items.extend(page.items);
        match (page.next, page.resume) {
            (Some(next), None) => cursor = Some(next),
            (None, Some(resume)) => return (items, pages, resume),
            other => panic!("a page needs exactly one of next and resume: {other:?}"),
        }
    }
}

// ----- source -----------------------------------------------------------------

#[tokio::test]
async fn the_source_feed_pages_resumes_and_expires() {
    let src = MemSource::new();
    src.set_page_size(2);
    let docs = src.add_folder(MEM_ROOT, "Docs");
    let ids: Vec<String> = (0..3)
        .map(|n| src.add_file(&docs, &format!("{n}.txt"), b"x"))
        .collect();

    let (items, pages, resume) = round(&src, None).await;
    assert_eq!(items.len(), 5);
    assert_eq!(pages, 3);
    assert_eq!(items[0].id, MEM_ROOT, "parents come first");
    assert_eq!(items[1].name, "Docs");

    let (items, _, resume) = round(&src, Some(&resume)).await;
    assert!(items.is_empty());

    src.modify(&ids[0], b"changed");
    src.set_name(&ids[1], "renamed.txt");
    src.mark_deleted(&ids[2]);
    let (items, _, resume) = round(&src, Some(&resume)).await;
    assert_eq!(items.len(), 3);
    assert_eq!(items[0].digests, ContentDigests::of_bytes(b"changed"));
    assert_eq!(items[1].name, "renamed.txt");
    assert!(items[2].deleted);
    assert!(matches!(
        src.item(&ids[2]).await,
        Err(ProviderError::NotFound(_))
    ));

    src.expire_cursors();
    let err = src.change_page(Some(&resume)).await.unwrap_err();
    assert!(matches!(err, ProviderError::ResyncRequired), "{err}");
    let (items, _, _) = round(&src, None).await;
    assert_eq!(items.len(), 4, "root, Docs and the two files left");
}

#[tokio::test]
async fn a_change_mid_round_loses_nothing() {
    let src = MemSource::new();
    src.set_page_size(2);
    let ids: Vec<String> = ["a", "b", "c", "d"]
        .iter()
        .map(|n| src.add_file(MEM_ROOT, n, n.as_bytes()))
        .collect();

    let first = src.change_page(None).await.unwrap();
    let mut seen: Vec<SourceItem> = first.items;
    // Deleted and added between pages: the rest of the round is unmoved.
    src.mark_deleted(&ids[0]);
    src.add_file(MEM_ROOT, "e", b"e");
    let (rest, _, resume) = round(&src, first.next.as_deref()).await;
    seen.extend(rest);
    let mut names: Vec<&str> = seen.iter().map(|i| i.name.as_str()).collect();
    names.sort();
    assert_eq!(names, ["a", "b", "c", "d", "root"]);
    // The changes come in the delta after.
    let (delta, _, _) = round(&src, Some(&resume)).await;
    assert_eq!(delta.len(), 2);
    assert!(delta[0].deleted && delta[0].id == ids[0]);
    assert_eq!(delta[1].name, "e");
}

#[tokio::test]
async fn source_faults_are_used_once_each_and_every_call_is_logged() {
    let src = MemSource::new();
    src.fail_next(Op::ChangePage, ProviderError::Offline("no network".into()));
    src.fail_next(
        Op::ChangePage,
        ProviderError::RateLimited {
            retry_after: Some(Duration::from_secs(7)),
        },
    );
    let err = src.change_page(None).await.unwrap_err();
    assert!(matches!(err, ProviderError::Offline(_)) && err.is_retryable());
    match src.change_page(None).await.unwrap_err() {
        ProviderError::RateLimited { retry_after } => {
            assert_eq!(retry_after, Some(Duration::from_secs(7)))
        }
        other => panic!("{other}"),
    }
    src.change_page(None).await.unwrap();
    src.fail_next(Op::Account, ProviderError::AuthRequired);
    assert!(matches!(
        src.account().await,
        Err(ProviderError::AuthRequired)
    ));
    assert_eq!(src.account().await.unwrap().provider, "mem");
    assert_eq!(src.count(Op::ChangePage), 3);
    assert_eq!(
        src.calls(),
        [
            "change_page -",
            "change_page -",
            "change_page -",
            "account ",
            "account "
        ]
    );
}

#[tokio::test]
async fn source_downloads_are_verified_before_they_appear() {
    let src = MemSource::new();
    let id = src.add_file(MEM_ROOT, "a.bin", b"abcdef");
    let item = src.item(&id).await.unwrap();
    let stage = tempfile::tempdir().unwrap();

    // Corrupted in transit: refused, nothing left.
    src.corrupt_next_download();
    let dest = stage.path().join("a.bin");
    let err = src.download_to(&item, &dest).await.unwrap_err();
    assert!(matches!(err, ProviderError::Integrity(_)), "{err}");
    assert!(listing(stage.path()).is_empty());

    // Changed since it was listed: the bytes don't match what was expected.
    src.modify(&id, b"abcdeg");
    let err = src.download_to(&item, &dest).await.unwrap_err();
    assert!(matches!(err, ProviderError::Integrity(_)), "{err}");
    assert!(listing(stage.path()).is_empty());

    // Fresh metadata: it goes through, once.
    let item = src.item(&id).await.unwrap();
    let got = src.download_to(&item, &dest).await.unwrap();
    assert_eq!(got, ContentDigests::of_bytes(b"abcdeg"));
    assert_eq!(std::fs::read(&dest).unwrap(), b"abcdeg");
    let err = src.download_to(&item, &dest).await.unwrap_err();
    assert!(matches!(err, ProviderError::Conflict(_)), "{err}");
    assert_eq!(listing(stage.path()), ["a.bin"]);

    // A transport failure leaves nothing either.
    src.fail_next(Op::Download, ProviderError::Transient("reset".into()));
    let other = stage.path().join("b.bin");
    assert!(src.download_to(&item, &other).await.is_err());
    assert_eq!(listing(stage.path()), ["a.bin"]);
    assert_eq!(src.count(Op::Download), 5);
}

// ----- destination ------------------------------------------------------------

#[tokio::test]
async fn the_destination_only_adds() {
    let dst = MemDestination::new();
    let local = tempfile::tempdir().unwrap();
    let v1 = local.path().join("v1");
    let v2 = local.path().join("v2");
    std::fs::write(&v1, b"version one").unwrap();
    std::fs::write(&v2, b"version two").unwrap();

    let uid = dst
        .ensure_folder(&rel("Mirror/Ünïcødé/日本語"))
        .await
        .unwrap();
    assert_eq!(
        dst.ensure_folder(&rel("Mirror/Ünïcødé/日本語"))
            .await
            .unwrap(),
        uid
    );
    assert_eq!(
        dst.paths(),
        ["Mirror", "Mirror/Ünïcødé", "Mirror/Ünïcødé/日本語"]
    );

    let folder = rel("Mirror/Ünïcødé/日本語");
    let stat = dst.upload_new(&folder, "файл.txt", &v1).await.unwrap();
    // Like Proton Drive: a size and a SHA-1, nothing more.
    assert_eq!(stat.claimed.size, 11);
    assert_eq!(
        stat.claimed.sha1,
        ContentDigests::of_bytes(b"version one").sha1
    );
    assert_eq!((stat.claimed.qxh, stat.claimed.sha256), (None, None));
    assert_eq!(stat.revision.as_deref(), Some("1"));

    // A taken name is a conflict; nothing is overwritten.
    let err = dst.upload_new(&folder, "файл.txt", &v2).await.unwrap_err();
    assert!(matches!(err, ProviderError::Conflict(_)), "{err}");
    let file = folder.join("файл.txt").unwrap();
    assert_eq!(dst.content(&file.to_string()).unwrap(), b"version one");

    // A new revision keeps the old one.
    let stat2 = dst.upload_revision(&file, &v2).await.unwrap();
    assert_eq!(stat2.uid, stat.uid);
    assert_eq!(stat2.revision.as_deref(), Some("2"));
    assert_eq!(
        dst.revisions(&file.to_string()),
        [b"version one".to_vec(), b"version two".to_vec()]
    );
    assert_eq!(dst.stat(&file).await.unwrap(), stat2);

    let entries = dst.list(&folder).await.unwrap();
    assert_eq!(
        entries,
        [DestEntry {
            name: "файл.txt".into(),
            uid: stat.uid.clone(),
            kind: ItemKind::File,
            size: Some(11),
        }]
    );
    let top = dst.list(&RelPath::root()).await.unwrap();
    assert_eq!(top.len(), 1);
    assert_eq!(top[0].kind, ItemKind::Folder);

    // Missing parents and files in the way.
    let err = dst.upload_new(&rel("Nope"), "a", &v1).await.unwrap_err();
    assert!(matches!(err, ProviderError::NotFound(_)), "{err}");
    let err = dst
        .ensure_folder(&file.join("sub").unwrap())
        .await
        .unwrap_err();
    assert!(matches!(err, ProviderError::Conflict(_)), "{err}");
    let err = dst
        .upload_revision(&rel("Mirror/missing"), &v1)
        .await
        .unwrap_err();
    assert!(matches!(err, ProviderError::NotFound(_)), "{err}");
    // An unsafe name never reaches the destination's tree.
    let err = dst.upload_new(&folder, "../escape", &v1).await.unwrap_err();
    assert!(matches!(err, ProviderError::InvalidPath(_)), "{err}");
    assert_eq!(dst.paths().len(), 4);
}

#[tokio::test]
async fn a_round_trip_catches_what_a_claimed_digest_cannot() {
    let dst = MemDestination::new();
    let local = tempfile::tempdir().unwrap();
    let stage = tempfile::tempdir().unwrap();
    let src = local.path().join("src");
    std::fs::write(&src, b"precious bytes").unwrap();
    let ours = ContentDigests::of_bytes(b"precious bytes");

    let stat = dst
        .upload_new(&RelPath::root(), "p.bin", &src)
        .await
        .unwrap();
    // L3: the claim matches...
    assert!(stat.claimed.verify_against(&ours).is_ok());
    // ...and L4 agrees while the stored bytes are intact.
    let back = dst
        .download_to(&rel("p.bin"), &stage.path().join("1"))
        .await
        .unwrap();
    assert_eq!(back.sha256, ours.sha256);

    // The stored bytes rot; the claim still matches, only a round trip shows it.
    dst.corrupt_stored("p.bin");
    let stat = dst.stat(&rel("p.bin")).await.unwrap();
    assert!(stat.claimed.verify_against(&ours).is_ok());
    let back = dst
        .download_to(&rel("p.bin"), &stage.path().join("2"))
        .await
        .unwrap();
    assert_ne!(back.sha256, ours.sha256);
    assert!(back.verify_against(&ours).is_err());
    assert!(VerifyLevel::L3RemoteClaimed < VerifyLevel::L4RoundTrip);
    assert_eq!(listing(stage.path()), ["1", "2"]);
}

#[tokio::test]
async fn missing_capabilities_are_refused_explicitly() {
    let dst = MemDestination::with_capabilities(Capabilities {
        create_folder: true,
        ..Default::default()
    });
    let local = tempfile::tempdir().unwrap();
    let f = local.path().join("f");
    std::fs::write(&f, b"x").unwrap();
    dst.upload_new(&RelPath::root(), "f", &f).await.unwrap();

    for (err, cap) in [
        (
            dst.upload_revision(&rel("f"), &f).await.unwrap_err(),
            Capability::Revisions,
        ),
        (
            dst.download_to(&rel("f"), &local.path().join("back"))
                .await
                .unwrap_err(),
            Capability::RoundTrip,
        ),
    ] {
        match err {
            ProviderError::Unsupported(c) => assert_eq!(c, cap),
            other => panic!("{other}"),
        }
    }
    assert_eq!(dst.revisions("f").len(), 1, "nothing changed");
    assert!(!local.path().join("back").exists());
    assert!(!dst.capabilities().supports(Capability::Sha1));
    assert!(matches!(
        dst.capabilities().require(Capability::Sha1),
        Err(ProviderError::Unsupported(Capability::Sha1))
    ));
    assert_eq!(
        ProviderError::Unsupported(Capability::Revisions).to_string(),
        "not supported by this provider: file revisions"
    );
}

#[tokio::test]
async fn destination_faults_leave_nothing_half_done() {
    let dst = MemDestination::new();
    let local = tempfile::tempdir().unwrap();
    let f = local.path().join("f");
    std::fs::write(&f, b"x").unwrap();

    dst.fail_next(
        Op::UploadNew,
        ProviderError::RateLimited { retry_after: None },
    );
    dst.fail_next(Op::EnsureFolder, ProviderError::Offline("down".into()));
    assert!(dst.upload_new(&RelPath::root(), "f", &f).await.is_err());
    assert!(dst.ensure_folder(&rel("A")).await.is_err());
    assert!(dst.paths().is_empty());

    dst.upload_new(&RelPath::root(), "f", &f).await.unwrap();
    dst.ensure_folder(&rel("A")).await.unwrap();
    assert_eq!(dst.paths(), ["A", "f"]);
    assert_eq!(dst.mutating_calls(), 4, "failed calls are logged too");
    assert_eq!(
        dst.calls(),
        [
            "upload_new f",
            "ensure_folder A",
            "upload_new f",
            "ensure_folder A"
        ]
    );
}

#[tokio::test]
async fn a_call_can_fail_after_it_took_effect() {
    let dst = MemDestination::new();
    let local = tempfile::tempdir().unwrap();
    let (f, g) = (local.path().join("f"), local.path().join("g"));
    std::fs::write(&f, b"first").unwrap();
    std::fs::write(&g, b"other").unwrap();

    // Stored, but the reply was lost.
    dst.fail_after(Op::UploadNew, ProviderError::Transient("timeout".into()));
    dst.fail_after(Op::EnsureFolder, ProviderError::Offline("down".into()));
    let err = dst.upload_new(&RelPath::root(), "f", &f).await.unwrap_err();
    assert!(err.is_retryable(), "{err}");
    assert!(dst.ensure_folder(&rel("A")).await.is_err());
    assert_eq!(dst.paths(), ["A", "f"]);

    // Retrying finds it there: the folder is fine, the file a conflict,
    // and what was stored first is untouched.
    dst.ensure_folder(&rel("A")).await.unwrap();
    let err = dst.upload_new(&RelPath::root(), "f", &g).await.unwrap_err();
    assert!(matches!(err, ProviderError::Conflict(_)), "{err}");
    assert_eq!(dst.revisions("f"), [b"first".to_vec()]);
}

#[tokio::test]
async fn reading_the_destination_mutates_nothing() {
    let dst = MemDestination::new();
    let stage = tempfile::tempdir().unwrap();
    dst.account().await.unwrap();
    dst.list(&RelPath::root()).await.unwrap();
    let _ = dst.stat(&rel("nothing")).await;
    let _ = dst
        .download_to(&rel("nothing"), &stage.path().join("x"))
        .await;
    assert_eq!(dst.calls().len(), 4);
    assert_eq!(dst.mutating_calls(), 0);
    assert!(listing(stage.path()).is_empty());
}

// ----- both sides, through the traits only -------------------------------------

/// Copy every file at the top of `src`'s root to `dst`, verifying each to
/// L4. Uses nothing but the two traits, so it shows they're enough.
async fn mirror_top_level(
    src: &impl SourceProvider,
    dst: &impl MirrorDestination,
    stage: &Path,
) -> PResult<Vec<(String, VerifyLevel)>> {
    let mut done = Vec::new();
    let mut cursor = None;
    loop {
        let page = src.change_page(cursor.as_deref()).await?;
        for item in page.items {
            if item.kind != ItemKind::File || item.parent_id.as_deref() != Some(MEM_ROOT) {
                continue;
            }
            let staged = stage.join(&item.id);
            let ours = src.download_to(&item, &staged).await?;
            let stat = dst
                .upload_new(&RelPath::root(), &item.name, &staged)
                .await?;
            let mut level = VerifyLevel::L2Uploaded;
            if stat.claimed.verify_against(&ours).is_ok() {
                level = VerifyLevel::L3RemoteClaimed;
                let back = stage.join(format!("{}.back", item.id));
                let theirs = dst
                    .download_to(&RelPath::root().join(&item.name)?, &back)
                    .await?;
                if theirs.sha256.is_some() && theirs.sha256 == ours.sha256 {
                    level = VerifyLevel::L4RoundTrip;
                }
            }
            done.push((item.name, level));
        }
        match page.next {
            Some(next) => cursor = Some(next),
            None => return Ok(done),
        }
    }
}

#[tokio::test]
async fn the_traits_are_enough_to_mirror_and_their_futures_are_send() {
    let src = std::sync::Arc::new(MemSource::new());
    src.set_page_size(1);
    src.add_file(MEM_ROOT, "a.txt", b"alpha");
    src.add_file(MEM_ROOT, "b.txt", b"beta");
    let dst = std::sync::Arc::new(MemDestination::new());
    let stage = tempfile::tempdir().unwrap();

    // Spawning needs the futures to be Send.
    let (s, d, p) = (src.clone(), dst.clone(), stage.path().to_path_buf());
    let done = tokio::spawn(async move { mirror_top_level(&*s, &*d, &p).await })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        done,
        [
            ("a.txt".to_string(), VerifyLevel::L4RoundTrip),
            ("b.txt".to_string(), VerifyLevel::L4RoundTrip)
        ]
    );
    assert_eq!(dst.content("a.txt").unwrap(), b"alpha");
    assert_eq!(dst.content("b.txt").unwrap(), b"beta");
}

// ----- no destructive method ----------------------------------------------------

/// Implements both traits with exactly the methods they have today. Adding a
/// required method to either (a `delete`, say) stops this compiling, so it
/// can't happen without someone reading this.
struct Exhaustive;

impl SourceProvider for Exhaustive {
    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }
    async fn account(&self) -> PResult<Account> {
        Err(ProviderError::Unsupported(Capability::ChangeFeed))
    }
    async fn change_page(&self, _: Option<&str>) -> PResult<ChangePage> {
        Err(ProviderError::Unsupported(Capability::ChangeFeed))
    }
    async fn item(&self, _: &str) -> PResult<SourceItem> {
        Err(ProviderError::Unsupported(Capability::ChangeFeed))
    }
    async fn download_to(&self, _: &SourceItem, _: &Path) -> PResult<ContentDigests> {
        Err(ProviderError::Unsupported(Capability::ChangeFeed))
    }
}

impl MirrorDestination for Exhaustive {
    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }
    async fn account(&self) -> PResult<Account> {
        Err(ProviderError::Unsupported(Capability::RoundTrip))
    }
    async fn list(&self, _: &RelPath) -> PResult<Vec<DestEntry>> {
        Err(ProviderError::Unsupported(Capability::RoundTrip))
    }
    async fn ensure_folder(&self, _: &RelPath) -> PResult<String> {
        Err(ProviderError::Unsupported(Capability::CreateFolder))
    }
    async fn upload_new(&self, _: &RelPath, _: &str, _: &Path) -> PResult<RemoteStat> {
        Err(ProviderError::Unsupported(Capability::RoundTrip))
    }
    async fn upload_revision(&self, _: &RelPath, _: &Path) -> PResult<RemoteStat> {
        Err(ProviderError::Unsupported(Capability::Revisions))
    }
    async fn stat(&self, _: &RelPath) -> PResult<RemoteStat> {
        Err(ProviderError::Unsupported(Capability::RoundTrip))
    }
    async fn download_to(&self, _: &RelPath, _: &Path) -> PResult<ContentDigests> {
        Err(ProviderError::Unsupported(Capability::RoundTrip))
    }
}

/// The names of every `fn` in the trait definitions, read from the source.
/// Catches a destructive method added with a default body, which
/// [`Exhaustive`] would not.
fn trait_methods() -> Vec<String> {
    let src = include_str!("../src/traits.rs");
    let mut names = Vec::new();
    let mut in_trait = false;
    for line in src.lines() {
        let t = line.trim();
        if t.starts_with("pub trait ") {
            in_trait = true;
        } else if line.starts_with('}') {
            in_trait = false;
        } else if in_trait {
            if let Some(rest) = t
                .strip_prefix("fn ")
                .or_else(|| t.strip_prefix("async fn "))
            {
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                names.push(name);
            }
        }
    }
    names
}

#[test]
fn no_destructive_method_exists() {
    assert_eq!(
        SourceProvider::capabilities(&Exhaustive),
        Capabilities::default()
    );
    assert_eq!(
        MirrorDestination::capabilities(&Exhaustive),
        Capabilities::default()
    );
    let methods = trait_methods();
    assert_eq!(
        methods,
        [
            // SourceProvider
            "capabilities",
            "account",
            "change_page",
            "item",
            // Reads only: lists one folder and what is in it.
            "enumerate_subtree",
            "download_to",
            // MirrorDestination
            "capabilities",
            // Reads only: says which names can't be put there.
            "name_problem",
            "account",
            "list",
            "ensure_folder",
            "upload_new",
            "upload_revision",
            "stat",
            "download_to",
        ],
        "a trait method was added or removed: if it deletes, moves, renames, \
         trashes or replaces anything, it must not exist"
    );
    for m in &methods {
        for bad in [
            "delete", "remove", "trash", "move", "rename", "purge", "unlink", "replace", "empty",
            "restore",
        ] {
            assert!(!m.contains(bad), "{m}");
        }
    }
    // The fakes' operations are the trait methods; none takes anything away.
    for op in [
        Op::Account,
        Op::ChangePage,
        Op::Item,
        Op::Download,
        Op::List,
        Op::EnsureFolder,
        Op::UploadNew,
        Op::UploadRevision,
        Op::Stat,
    ] {
        // Exhaustive: a new Op has to be placed here.
        let adds = match op {
            Op::EnsureFolder | Op::UploadNew | Op::UploadRevision => true,
            Op::Account | Op::ChangePage | Op::Item | Op::Download | Op::List | Op::Stat => false,
        };
        assert_eq!(op.is_mutating(), adds, "{op:?}");
        assert!(methods.iter().any(|m| m == op.name()), "{op:?}");
    }
}

#[tokio::test]
async fn the_default_subtree_listing_reads_the_feed_and_keeps_the_folder() {
    let src = MemSource::new();
    src.set_page_size(2);
    let docs = src.add_folder(MEM_ROOT, "Docs");
    let deep = src.add_folder(&docs, "Deep");
    let a = src.add_file(&deep, "a.txt", b"alpha");
    let b = src.add_file(&docs, "b.txt", b"bravo");
    let gone = src.add_file(&deep, "gone.txt", b"gone");
    src.mark_deleted(&gone);
    let other = src.add_folder(MEM_ROOT, "Other");
    src.add_file(&other, "c.txt", b"charlie");

    let seen = std::sync::Mutex::new(Vec::new());
    let items = src
        .enumerate_subtree(&rel("Docs/Deep"), &|n| seen.lock().unwrap().push(n))
        .await
        .unwrap();
    let mut ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
    ids.sort();
    // The folder, what is in it, and the folders above it; nothing else,
    // and nothing deleted.
    let mut want = vec![MEM_ROOT, docs.as_str(), deep.as_str(), a.as_str()];
    want.sort();
    assert_eq!(ids, want);
    assert!(!ids.contains(&b.as_str()));
    let seen = seen.into_inner().unwrap();
    assert!(!seen.is_empty());
    assert!(seen.windows(2).all(|w| w[0] <= w[1]), "{seen:?}");

    // The whole folder from one level up, and the root is everything.
    let items = src.enumerate_subtree(&rel("Docs"), &|_| {}).await.unwrap();
    assert_eq!(items.len(), 5);
    let items = src.enumerate_subtree(&rel(""), &|_| {}).await.unwrap();
    assert_eq!(items.len(), 7);

    // Names must match exactly, and something missing is an error.
    for missing in ["Nope", "docs/Deep", "Docs/Deep/gone.txt"] {
        let e = src
            .enumerate_subtree(&rel(missing), &|_| {})
            .await
            .unwrap_err();
        assert!(matches!(e, ProviderError::NotFound(_)), "{missing}: {e}");
    }
    assert_eq!(src.count(Op::Download), 0);
}
