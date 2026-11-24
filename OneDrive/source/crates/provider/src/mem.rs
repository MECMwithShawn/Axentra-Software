//! In-memory fakes of both sides of a mirror, for tests.
//!
//! Every trait call is logged as `"<op> <argument>"` before anything else
//! happens, and any call can be made to fail: queue errors with `fail_next`
//! and each call of that operation takes the next one. A destination can
//! also fail a call *after* doing it (`fail_after`), like an upload that
//! was stored but timed out before the reply.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::io::Write;
use std::path::Path;

use chrono::Utc;
use parking_lot::Mutex;

use crate::digests::ContentDigests;
use crate::error::{PResult, ProviderError};
use crate::path::RelPath;
use crate::staging::{parent_dir, verify_and_publish, StagedTemp};
use crate::traits::{
    Account, Capabilities, Capability, ChangePage, DestEntry, ItemKind, MirrorDestination,
    RemoteStat, SourceItem, SourceProvider,
};

/// The id of [`MemSource`]'s root folder.
pub const MEM_ROOT: &str = "ROOT";

/// A trait operation of either fake.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Op {
    Account,
    ChangePage,
    Item,
    Download,
    List,
    EnsureFolder,
    UploadNew,
    UploadRevision,
    Stat,
}

impl Op {
    pub fn name(self) -> &'static str {
        match self {
            Op::Account => "account",
            Op::ChangePage => "change_page",
            Op::Item => "item",
            Op::Download => "download_to",
            Op::List => "list",
            Op::EnsureFolder => "ensure_folder",
            Op::UploadNew => "upload_new",
            Op::UploadRevision => "upload_revision",
            Op::Stat => "stat",
        }
    }

    /// Adds something to the destination. None of the operations take
    /// anything away.
    pub fn is_mutating(self) -> bool {
        matches!(self, Op::EnsureFolder | Op::UploadNew | Op::UploadRevision)
    }
}

/// Errors queued per operation, each used once.
#[derive(Debug, Default)]
struct Faults(HashMap<Op, VecDeque<ProviderError>>);

impl Faults {
    fn take(&mut self, op: Op) -> PResult<()> {
        match self.0.get_mut(&op).and_then(VecDeque::pop_front) {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

/// Log the call, then fail it if an error is queued for it.
fn begin(log: &mut Vec<String>, faults: &mut Faults, op: Op, arg: &str) -> PResult<()> {
    log.push(format!("{} {arg}", op.name()));
    faults.take(op)
}

fn write_temp(dir: &Path, data: &[u8]) -> PResult<StagedTemp> {
    let mut tmp = StagedTemp::create_in(dir)?;
    tmp.file().write_all(data)?;
    Ok(tmp)
}

// ----- source -----------------------------------------------------------------

/// A source drive held in memory, with a change feed.
///
/// Cursors look like `mem:<epoch>:full:<skip>:<mark>` while a full
/// enumeration is being paged and `mem:<epoch>:delta:<pos>` afterwards;
/// [`MemSource::expire_cursors`] makes every earlier one stale.
pub struct MemSource {
    state: Mutex<SourceState>,
}

struct SourceState {
    items: BTreeMap<String, (SourceItem, Vec<u8>)>,
    /// Every change, in order, as the item was right after it.
    journal: Vec<SourceItem>,
    epoch: u64,
    page_size: usize,
    next_id: u64,
    faults: Faults,
    log: Vec<String>,
    corrupt_downloads: usize,
}

impl SourceState {
    fn record(&mut self, item: SourceItem, data: Vec<u8>) {
        self.journal.push(item.clone());
        if item.deleted {
            self.items.remove(&item.id);
        } else {
            self.items.insert(item.id.clone(), (item, data));
        }
    }

    fn get(&self, id: &str) -> SourceItem {
        self.items
            .get(id)
            .unwrap_or_else(|| panic!("MemSource: no item {id}"))
            .0
            .clone()
    }
}

impl Default for MemSource {
    fn default() -> Self {
        Self::new()
    }
}

impl MemSource {
    pub fn new() -> Self {
        let mut state = SourceState {
            items: BTreeMap::new(),
            journal: Vec::new(),
            epoch: 1,
            page_size: 200,
            next_id: 1,
            faults: Faults::default(),
            log: Vec::new(),
            corrupt_downloads: 0,
        };
        let root = SourceItem {
            id: MEM_ROOT.into(),
            name: "root".into(),
            parent_id: None,
            kind: ItemKind::Folder,
            deleted: false,
            special: None,
            digests: ContentDigests::default(),
            modified: Some(Utc::now()),
            version: Some("1".into()),
        };
        state.record(root, Vec::new());
        Self {
            state: Mutex::new(state),
        }
    }

    fn add(&self, parent_id: &str, name: &str, kind: ItemKind, content: &[u8]) -> String {
        let mut s = self.state.lock();
        assert!(s.items.contains_key(parent_id), "MemSource: no {parent_id}");
        let id = format!("M{:04}", s.next_id);
        s.next_id += 1;
        let digests = match kind {
            ItemKind::File => ContentDigests::of_bytes(content),
            ItemKind::Folder => ContentDigests::default(),
        };
        let item = SourceItem {
            id: id.clone(),
            name: name.into(),
            parent_id: Some(parent_id.into()),
            kind,
            deleted: false,
            special: None,
            digests,
            modified: Some(Utc::now()),
            version: Some("1".into()),
        };
        s.record(item, content.to_vec());
        id
    }

    pub fn add_folder(&self, parent_id: &str, name: &str) -> String {
        self.add(parent_id, name, ItemKind::Folder, &[])
    }

    pub fn add_file(&self, parent_id: &str, name: &str, content: &[u8]) -> String {
        self.add(parent_id, name, ItemKind::File, content)
    }

    /// New content for a file, as if edited online.
    pub fn modify(&self, id: &str, content: &[u8]) {
        let mut s = self.state.lock();
        let mut item = s.get(id);
        item.digests = ContentDigests::of_bytes(content);
        item.modified = Some(Utc::now());
        let v: u64 = item.version.as_deref().unwrap_or("0").parse().unwrap_or(0);
        item.version = Some((v + 1).to_string());
        s.record(item, content.to_vec());
    }

    /// A new name, as if renamed online. The content stays.
    pub fn set_name(&self, id: &str, name: &str) {
        let mut s = self.state.lock();
        let mut item = s.get(id);
        let data = s.items[id].1.clone();
        item.name = name.into();
        s.record(item, data);
    }

    /// Gone from the source, as if deleted online.
    pub fn mark_deleted(&self, id: &str) {
        let mut s = self.state.lock();
        let mut item = s.get(id);
        item.deleted = true;
        s.record(item, Vec::new());
    }

    pub fn set_page_size(&self, n: usize) {
        self.state.lock().page_size = n.max(1);
    }

    /// Make every cursor handed out so far stale, like OneDrive's 410.
    pub fn expire_cursors(&self) {
        self.state.lock().epoch += 1;
    }

    pub fn fail_next(&self, op: Op, err: ProviderError) {
        self.state
            .lock()
            .faults
            .0
            .entry(op)
            .or_default()
            .push_back(err);
    }

    /// Flip a byte in what the next download writes, as a transfer that
    /// corrupted it would.
    pub fn corrupt_next_download(&self) {
        self.state.lock().corrupt_downloads += 1;
    }

    pub fn calls(&self) -> Vec<String> {
        self.state.lock().log.clone()
    }

    pub fn count(&self, op: Op) -> usize {
        let prefix = format!("{} ", op.name());
        self.state
            .lock()
            .log
            .iter()
            .filter(|l| l.starts_with(&prefix))
            .count()
    }

    fn page(s: &SourceState, cursor: Option<&str>) -> PResult<ChangePage> {
        let bad = || ProviderError::Other(format!("MemSource: bad cursor {cursor:?}"));
        let (full, from, mark) = match cursor {
            None => (true, 0, s.journal.len()),
            Some(c) => {
                let parts: Vec<&str> = c.split(':').collect();
                let num = |i: usize| parts.get(i).and_then(|p| p.parse::<usize>().ok());
                if parts.first() != Some(&"mem") {
                    return Err(bad());
                }
                if num(1) != Some(s.epoch as usize) {
                    return Err(ProviderError::ResyncRequired);
                }
                match parts.get(2) {
                    Some(&"full") => (true, num(3).ok_or_else(bad)?, num(4).ok_or_else(bad)?),
                    Some(&"delta") => (false, num(3).ok_or_else(bad)?, 0),
                    _ => return Err(bad()),
                }
            }
        };
        let epoch = s.epoch;
        let n = s.page_size;
        let mut page = ChangePage::default();
        if full {
            // Every page of a round lists the drive as it was at `mark`, so
            // a change mid-round shifts nothing; it comes in the delta after.
            let mut at_mark: BTreeMap<&str, &SourceItem> = BTreeMap::new();
            for i in &s.journal[..mark.min(s.journal.len())] {
                at_mark.insert(&i.id, i);
            }
            at_mark.retain(|_, i| !i.deleted);
            let depth = |id: &str| {
                let mut d = 0;
                let mut cur = at_mark.get(id).and_then(|i| i.parent_id.as_deref());
                while let Some(p) = cur {
                    d += 1;
                    cur = at_mark.get(p).and_then(|i| i.parent_id.as_deref());
                }
                d
            };
            let mut all: Vec<&SourceItem> = at_mark.values().copied().collect();
            all.sort_by_key(|i| (depth(&i.id), i.id.clone()));
            page.items = all
                .iter()
                .skip(from)
                .take(n)
                .map(|i| (*i).clone())
                .collect();
            if from + n < all.len() {
                page.next = Some(format!("mem:{epoch}:full:{}:{mark}", from + n));
            } else {
                page.resume = Some(format!("mem:{epoch}:delta:{mark}"));
            }
        } else {
            let from = from.min(s.journal.len());
            page.items = s.journal[from..].iter().take(n).cloned().collect();
            if from + n < s.journal.len() {
                page.next = Some(format!("mem:{epoch}:delta:{}", from + n));
            } else {
                page.resume = Some(format!("mem:{epoch}:delta:{}", s.journal.len()));
            }
        }
        Ok(page)
    }
}

impl SourceProvider for MemSource {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            change_feed: true,
            quick_xor_hash: true,
            sha256: true,
            ..Default::default()
        }
    }

    async fn account(&self) -> PResult<Account> {
        let mut s = self.state.lock();
        let s = &mut *s;
        begin(&mut s.log, &mut s.faults, Op::Account, "")?;
        Ok(Account {
            provider: "mem".into(),
            id: "MEMSOURCE".into(),
            display_name: Some("Test Source".into()),
            email: None,
            kind: None,
        })
    }

    async fn change_page(&self, cursor: Option<&str>) -> PResult<ChangePage> {
        let mut s = self.state.lock();
        let s = &mut *s;
        begin(
            &mut s.log,
            &mut s.faults,
            Op::ChangePage,
            cursor.unwrap_or("-"),
        )?;
        Self::page(s, cursor)
    }

    async fn item(&self, id: &str) -> PResult<SourceItem> {
        let mut s = self.state.lock();
        let s = &mut *s;
        begin(&mut s.log, &mut s.faults, Op::Item, id)?;
        match s.items.get(id) {
            Some((item, _)) => Ok(item.clone()),
            None => Err(ProviderError::NotFound(id.into())),
        }
    }

    async fn download_to(&self, item: &SourceItem, dest: &Path) -> PResult<ContentDigests> {
        let data = {
            let mut s = self.state.lock();
            let s = &mut *s;
            begin(&mut s.log, &mut s.faults, Op::Download, &item.id)?;
            if item.kind != ItemKind::File || item.deleted || item.special.is_some() {
                return Err(ProviderError::Other(format!(
                    "{} is not a plain file",
                    item.name
                )));
            }
            let Some((_, data)) = s.items.get(&item.id) else {
                return Err(ProviderError::NotFound(item.id.clone()));
            };
            let mut data = data.clone();
            if s.corrupt_downloads > 0 {
                s.corrupt_downloads -= 1;
                match data.first_mut() {
                    Some(b) => *b ^= 0x01,
                    None => data.push(0),
                }
            }
            data
        };
        let tmp = write_temp(parent_dir(dest), &data)?;
        verify_and_publish(tmp, Some(&item.digests), dest).await
    }
}

// ----- destination ------------------------------------------------------------

/// A destination drive held in memory. Starts with an empty root folder.
pub struct MemDestination {
    state: Mutex<DestState>,
}

struct Revision {
    data: Vec<u8>,
    claimed: ContentDigests,
}

enum Node {
    Folder {
        uid: String,
    },
    File {
        uid: String,
        revisions: Vec<Revision>,
    },
}

struct DestState {
    nodes: BTreeMap<RelPath, Node>,
    next_uid: u64,
    caps: Capabilities,
    faults: Faults,
    /// Errors returned after the call has taken effect.
    faults_after: Faults,
    log: Vec<String>,
}

impl DestState {
    fn uid(&mut self) -> String {
        let uid = format!("D{:04}", self.next_uid);
        self.next_uid += 1;
        uid
    }

    fn stat_of(&self, file: &RelPath) -> PResult<RemoteStat> {
        match self.nodes.get(file) {
            Some(Node::File { uid, revisions }) => {
                let last = revisions.last().expect("a file has a revision");
                Ok(RemoteStat {
                    uid: uid.clone(),
                    claimed: last.claimed.clone(),
                    revision: Some(revisions.len().to_string()),
                })
            }
            Some(Node::Folder { .. }) => Err(ProviderError::Other(format!("{file} is a folder"))),
            None => Err(ProviderError::NotFound(file.to_string())),
        }
    }
}

/// What the fake says it stored: the size and SHA-1 of the bytes it was
/// given and nothing else, as Proton Drive reports.
fn claim(data: &[u8]) -> ContentDigests {
    let all = ContentDigests::of_bytes(data);
    ContentDigests {
        size: all.size,
        sha1: all.sha1,
        ..Default::default()
    }
}

impl Default for MemDestination {
    fn default() -> Self {
        Self::new()
    }
}

impl MemDestination {
    pub fn new() -> Self {
        Self::with_capabilities(Capabilities {
            sha1: true,
            create_folder: true,
            revisions: true,
            round_trip: true,
            ..Default::default()
        })
    }

    /// A destination that can do only `caps`. Anything else it is asked
    /// gets [`ProviderError::Unsupported`].
    pub fn with_capabilities(caps: Capabilities) -> Self {
        let mut nodes = BTreeMap::new();
        nodes.insert(
            RelPath::root(),
            Node::Folder {
                uid: "D0000".into(),
            },
        );
        Self {
            state: Mutex::new(DestState {
                nodes,
                next_uid: 1,
                caps,
                faults: Faults::default(),
                faults_after: Faults::default(),
                log: Vec::new(),
            }),
        }
    }

    pub fn fail_next(&self, op: Op, err: ProviderError) {
        self.state
            .lock()
            .faults
            .0
            .entry(op)
            .or_default()
            .push_back(err);
    }

    /// Fail the next call of `op` with `err` only after it has done its
    /// work, as when the destination stored something but the reply was
    /// lost.
    pub fn fail_after(&self, op: Op, err: ProviderError) {
        self.state
            .lock()
            .faults_after
            .0
            .entry(op)
            .or_default()
            .push_back(err);
    }

    pub fn calls(&self) -> Vec<String> {
        self.state.lock().log.clone()
    }

    /// Calls that would add something, failed ones included.
    pub fn mutating_calls(&self) -> usize {
        let ops = [Op::EnsureFolder, Op::UploadNew, Op::UploadRevision];
        let s = self.state.lock();
        s.log
            .iter()
            .filter(|l| {
                ops.iter()
                    .any(|op| l.starts_with(&format!("{} ", op.name())))
            })
            .count()
    }

    /// Every path held, folders and files, root excluded.
    pub fn paths(&self) -> Vec<String> {
        let s = self.state.lock();
        s.nodes
            .keys()
            .filter(|p| !p.is_root())
            .map(RelPath::to_string)
            .collect()
    }

    /// The latest revision of `path`.
    pub fn content(&self, path: &str) -> Option<Vec<u8>> {
        let path = RelPath::parse(path).ok()?;
        match self.state.lock().nodes.get(&path)? {
            Node::File { revisions, .. } => revisions.last().map(|r| r.data.clone()),
            Node::Folder { .. } => None,
        }
    }

    /// Every revision of `path`, oldest first.
    pub fn revisions(&self, path: &str) -> Vec<Vec<u8>> {
        let Ok(path) = RelPath::parse(path) else {
            return Vec::new();
        };
        match self.state.lock().nodes.get(&path) {
            Some(Node::File { revisions, .. }) => {
                revisions.iter().map(|r| r.data.clone()).collect()
            }
            _ => Vec::new(),
        }
    }

    /// Flip a byte of what is stored for `path` without changing what is
    /// claimed for it: only a round trip notices.
    pub fn corrupt_stored(&self, path: &str) {
        let path = RelPath::parse(path).expect("a valid path");
        let mut s = self.state.lock();
        match s.nodes.get_mut(&path) {
            Some(Node::File { revisions, .. }) => {
                let data = &mut revisions.last_mut().unwrap().data;
                match data.first_mut() {
                    Some(b) => *b ^= 0x01,
                    None => data.push(0),
                }
            }
            _ => panic!("MemDestination: no file {path}"),
        }
    }
}

impl MirrorDestination for MemDestination {
    fn capabilities(&self) -> Capabilities {
        self.state.lock().caps
    }

    async fn account(&self) -> PResult<Account> {
        let mut s = self.state.lock();
        let s = &mut *s;
        begin(&mut s.log, &mut s.faults, Op::Account, "")?;
        Ok(Account {
            provider: "mem".into(),
            id: "MEMDEST".into(),
            display_name: Some("Test Destination".into()),
            email: None,
            kind: None,
        })
    }

    async fn list(&self, folder: &RelPath) -> PResult<Vec<DestEntry>> {
        let mut s = self.state.lock();
        let s = &mut *s;
        begin(&mut s.log, &mut s.faults, Op::List, &folder.to_string())?;
        match s.nodes.get(folder) {
            Some(Node::Folder { .. }) => {}
            Some(Node::File { .. }) => {
                return Err(ProviderError::Other(format!("{folder} is not a folder")))
            }
            None => return Err(ProviderError::NotFound(folder.to_string())),
        }
        Ok(s.nodes
            .iter()
            .filter(|(p, _)| p.parent().as_ref() == Some(folder))
            .map(|(p, node)| match node {
                Node::Folder { uid } => DestEntry {
                    name: p.name().unwrap_or_default().into(),
                    uid: uid.clone(),
                    kind: ItemKind::Folder,
                    size: None,
                },
                Node::File { uid, revisions } => DestEntry {
                    name: p.name().unwrap_or_default().into(),
                    uid: uid.clone(),
                    kind: ItemKind::File,
                    size: revisions.last().map(|r| r.data.len() as u64),
                },
            })
            .collect())
    }

    async fn ensure_folder(&self, folder: &RelPath) -> PResult<String> {
        let mut s = self.state.lock();
        let s = &mut *s;
        begin(
            &mut s.log,
            &mut s.faults,
            Op::EnsureFolder,
            &folder.to_string(),
        )?;
        s.caps.require(Capability::CreateFolder)?;
        // Check the whole way down before creating anything.
        let mut at = RelPath::root();
        let mut missing = Vec::new();
        for seg in folder.segments() {
            at = at.join(seg)?;
            match s.nodes.get(&at) {
                Some(Node::Folder { .. }) => {}
                Some(Node::File { .. }) => {
                    return Err(ProviderError::Conflict(format!("{at} is a file")))
                }
                None => missing.push(at.clone()),
            }
        }
        for p in missing {
            let uid = s.uid();
            s.nodes.insert(p, Node::Folder { uid });
        }
        s.faults_after.take(Op::EnsureFolder)?;
        match &s.nodes[folder] {
            Node::Folder { uid } => Ok(uid.clone()),
            Node::File { .. } => unreachable!("checked above"),
        }
    }

    async fn upload_new(&self, folder: &RelPath, name: &str, local: &Path) -> PResult<RemoteStat> {
        {
            let mut s = self.state.lock();
            let s = &mut *s;
            let arg = format!("{folder}/{name}");
            begin(
                &mut s.log,
                &mut s.faults,
                Op::UploadNew,
                arg.trim_start_matches('/'),
            )?;
        }
        let file = folder.join(name)?;
        let data = tokio::fs::read(local).await?;
        let mut s = self.state.lock();
        match s.nodes.get(folder) {
            Some(Node::Folder { .. }) => {}
            Some(Node::File { .. }) => {
                return Err(ProviderError::Conflict(format!("{folder} is a file")))
            }
            None => return Err(ProviderError::NotFound(folder.to_string())),
        }
        if s.nodes.contains_key(&file) {
            return Err(ProviderError::Conflict(format!("{file} already exists")));
        }
        let uid = s.uid();
        let claimed = claim(&data);
        s.nodes.insert(
            file.clone(),
            Node::File {
                uid,
                revisions: vec![Revision { data, claimed }],
            },
        );
        s.faults_after.take(Op::UploadNew)?;
        s.stat_of(&file)
    }

    async fn upload_revision(&self, file: &RelPath, local: &Path) -> PResult<RemoteStat> {
        {
            let mut s = self.state.lock();
            let s = &mut *s;
            begin(
                &mut s.log,
                &mut s.faults,
                Op::UploadRevision,
                &file.to_string(),
            )?;
            s.caps.require(Capability::Revisions)?;
        }
        let data = tokio::fs::read(local).await?;
        let mut s = self.state.lock();
        match s.nodes.get_mut(file) {
            Some(Node::File { revisions, .. }) => {
                let claimed = claim(&data);
                revisions.push(Revision { data, claimed });
            }
            Some(Node::Folder { .. }) => {
                return Err(ProviderError::Conflict(format!("{file} is a folder")))
            }
            None => return Err(ProviderError::NotFound(file.to_string())),
        }
        s.faults_after.take(Op::UploadRevision)?;
        s.stat_of(file)
    }

    async fn stat(&self, file: &RelPath) -> PResult<RemoteStat> {
        let mut s = self.state.lock();
        let s = &mut *s;
        begin(&mut s.log, &mut s.faults, Op::Stat, &file.to_string())?;
        s.stat_of(file)
    }

    async fn download_to(&self, file: &RelPath, dest: &Path) -> PResult<ContentDigests> {
        let data = {
            let mut s = self.state.lock();
            let s = &mut *s;
            begin(&mut s.log, &mut s.faults, Op::Download, &file.to_string())?;
            s.caps.require(Capability::RoundTrip)?;
            match s.nodes.get(file) {
                Some(Node::File { revisions, .. }) => revisions.last().unwrap().data.clone(),
                Some(Node::Folder { .. }) => {
                    return Err(ProviderError::Other(format!("{file} is a folder")))
                }
                None => return Err(ProviderError::NotFound(file.to_string())),
            }
        };
        let tmp = write_temp(parent_dir(dest), &data)?;
        verify_and_publish(tmp, None, dest).await
    }
}
