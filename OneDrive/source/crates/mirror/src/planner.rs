//! The dry run: what a mirror run would do, worked out by reading only.
//!
//! The source is enumerated in full from no cursor, or, for a plan limited
//! to one folder, only that folder is listed
//! ([`SourceProvider::enumerate_subtree`]), and the destination is
//! listed folder by folder (plus a `stat` where a file of the same name is
//! already there). Both are reads: planning makes no call that adds or
//! changes anything, and writes nothing to the manifest either, so a plan
//! can be thrown away freely.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;

use axentra_clouddrive_provider::{
    ContentDigests, DestEntry, ItemKind, MirrorDestination, ProviderError, RelPath, SourceItem,
    SourceProvider, VerifyLevel,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{MResult, MirrorError};
use crate::manifest::{ItemRow, ItemState};

/// What to do when a file already mirrored changes on the source.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangedSourcePolicy {
    /// Record it as diverged and leave the destination's copy as it is.
    #[default]
    MarkDiverged,
    /// Upload the new content as a new revision of the destination's file,
    /// which keeps the earlier ones. Off unless chosen.
    CreateNewRevision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanOptions {
    /// Only this source folder and what is in it, such as
    /// `/CloudDrive Mirror Test`. `None` for the whole source.
    pub source_prefix: Option<String>,
    /// The folder under the destination's root that mirrors the source's
    /// root, such as `OneDrive`.
    pub dest_root: String,
    /// Files larger than this are skipped as too large.
    pub staging_cap: u64,
    /// Whether runs download each file back to check what was stored.
    pub round_trip: bool,
    pub on_source_change: ChangedSourcePolicy,
}

impl Default for PlanOptions {
    fn default() -> Self {
        Self {
            source_prefix: None,
            dest_root: crate::DEFAULT_DEST_ROOT.into(),
            staging_cap: crate::DEFAULT_STAGING_CAP,
            round_trip: false,
            on_source_change: ChangedSourcePolicy::default(),
        }
    }
}

impl PlanOptions {
    pub(crate) fn prefix(&self) -> MResult<RelPath> {
        let p = self.source_prefix.as_deref().unwrap_or("");
        RelPath::parse(p.trim_start_matches('/'))
            .map_err(|e| MirrorError::Invalid(format!("source folder: {e}")))
    }

    pub(crate) fn dest_root(&self) -> MResult<RelPath> {
        RelPath::parse(self.dest_root.trim_start_matches('/'))
            .map_err(|e| MirrorError::Invalid(format!("destination folder: {e}")))
    }

    /// The level a file has to reach to count as done.
    pub fn target_level(&self) -> VerifyLevel {
        if self.round_trip {
            VerifyLevel::L4RoundTrip
        } else {
            VerifyLevel::L3RemoteClaimed
        }
    }
}

/// What a run would do with one item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// A folder to create at the destination.
    CreateFolder,
    /// A folder the destination already has.
    FolderExists,
    /// A file to upload; the destination has nothing of that name.
    UploadNew,
    /// The destination has a file of that name whose claimed SHA-1 and size
    /// match the source's: it is checked and recorded, not uploaded.
    AdoptExisting,
    /// The destination has a file of that name and the source gave no SHA-1
    /// to compare: the run downloads it, compares, and adopts it or marks
    /// it diverged. It never overwrites it.
    CheckExisting,
    /// Changed on the source after it was mirrored, and new revisions are
    /// allowed: uploaded as a new revision, keeping the old one.
    UploadRevision,
    /// Partly done: the run carries on from the level already reached.
    Resume,
    AlreadyMirrored,
    /// Source and destination disagree; both are left alone.
    Diverged,
    SkipTooLarge,
    /// Not plain content (a OneNote package, a shortcut into another
    /// drive), a name the destination can't take, or inside such an item.
    SkipUnsupported,
    /// Gone from the source. Recorded only; the destination keeps its copy.
    SourceDeleted,
    /// Moved or renamed on the source after it was mirrored. Recorded only:
    /// nothing is moved or copied at the destination.
    SourceMoved,
}

impl Action {
    pub fn as_str(self) -> &'static str {
        match self {
            Action::CreateFolder => "create_folder",
            Action::FolderExists => "folder_exists",
            Action::UploadNew => "upload_new",
            Action::AdoptExisting => "adopt_existing",
            Action::CheckExisting => "check_existing",
            Action::UploadRevision => "upload_revision",
            Action::Resume => "resume",
            Action::AlreadyMirrored => "already_mirrored",
            Action::Diverged => "diverged",
            Action::SkipTooLarge => "skip_too_large",
            Action::SkipUnsupported => "skip_unsupported",
            Action::SourceDeleted => "source_deleted",
            Action::SourceMoved => "source_moved",
        }
    }

    /// The run downloads the file from the source (or back from the
    /// destination) for this, so it needs staging space.
    pub fn stages(self) -> bool {
        matches!(
            self,
            Action::UploadNew
                | Action::AdoptExisting
                | Action::CheckExisting
                | Action::UploadRevision
                | Action::Resume
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanEntry {
    pub source_id: String,
    /// Relative to the source's root.
    pub source_path: String,
    /// Relative to the destination's root.
    pub dest_path: String,
    pub folder: bool,
    /// What the source claims for the current version.
    pub digests: ContentDigests,
    pub version: Option<String>,
    pub modified: Option<DateTime<Utc>>,
    pub action: Action,
    pub reason: Option<String>,
    /// The level the manifest records for it now.
    pub level: VerifyLevel,
}

impl PlanEntry {
    pub fn kind(&self) -> ItemKind {
        if self.folder {
            ItemKind::Folder
        } else {
            ItemKind::File
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Count {
    pub items: u64,
    pub bytes: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanTotals {
    pub folders: u64,
    pub files: u64,
    pub by_action: BTreeMap<Action, Count>,
    /// Bytes to be uploaded (new files and new revisions).
    pub bytes_to_upload: u64,
    /// Bytes to be downloaded from the source into staging.
    pub bytes_to_stage: u64,
    /// The most staging space a run needs at once: files are staged one at
    /// a time, so this is the largest one.
    pub staging_needed: u64,
    pub staging_cap: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub created: DateTime<Utc>,
    pub options: PlanOptions,
    /// Whether the destination already has the mirror's root folder.
    pub dest_root_exists: bool,
    /// Folders first, parents before children, then files.
    pub entries: Vec<PlanEntry>,
    /// Source paths whose names differ only in letter case within one
    /// folder. They are kept as distinct files (Proton Drive's names are
    /// case-sensitive) but listed, since a case-insensitive filesystem
    /// could not hold them side by side.
    pub case_collisions: Vec<Vec<String>>,
    /// The source's change-feed cursor at the end of the enumeration.
    /// `None` for a plan limited to one folder: that folder was listed, not
    /// the whole change feed, so no cursor can say what has been seen.
    pub source_cursor: Option<String>,
    pub totals: PlanTotals,
}

/// How far planning has got, for showing to people. Carries counts only,
/// never names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanProgress {
    /// This many source items seen so far.
    SourceItems(u64),
    /// The source is read; the destination is being listed.
    ListingDestination,
}

/// Work out what a run would do, reading only.
///
/// `known` is the manifest as it is now ([`crate::Manifest::snapshot`]).
pub async fn plan<S, D>(
    source: &S,
    dest: &D,
    known: &HashMap<String, ItemRow>,
    opts: &PlanOptions,
) -> MResult<Plan>
where
    S: SourceProvider,
    D: MirrorDestination,
{
    plan_with_progress(source, dest, known, opts, &|_| {}).await
}

/// [`plan`], telling `progress` how far it has got now and then.
pub async fn plan_with_progress<S, D>(
    source: &S,
    dest: &D,
    known: &HashMap<String, ItemRow>,
    opts: &PlanOptions,
    progress: &(dyn Fn(PlanProgress) + Sync),
) -> MResult<Plan>
where
    S: SourceProvider,
    D: MirrorDestination,
{
    let prefix = opts.prefix()?;
    let dest_root = opts.dest_root()?;
    let source_progress = |n: u64| progress(PlanProgress::SourceItems(n));

    let mut all: HashMap<String, SourceItem> = HashMap::new();
    let subtree = !prefix.is_root();
    let resume = if subtree {
        // Only the folder asked for (and the folders above it). This is no
        // round of the change feed, so it gives no cursor to keep.
        for it in source.enumerate_subtree(&prefix, &source_progress).await? {
            all.insert(it.id.clone(), it);
        }
        None
    } else {
        // Full enumeration from no cursor.
        let mut cursor: Option<String> = None;
        let mut seen = 0u64;
        loop {
            let page = source.change_page(cursor.as_deref()).await?;
            seen += page.items.len() as u64;
            source_progress(seen);
            for it in page.items {
                if it.deleted {
                    all.remove(&it.id);
                } else {
                    all.insert(it.id.clone(), it);
                }
            }
            match (page.next, page.resume) {
                (Some(next), _) => cursor = Some(next),
                (None, resume) => break resume,
            }
        }
    };

    let tree = Tree::build(&all);
    progress(PlanProgress::ListingDestination);
    let mut listings = Listings::new(dest_root.clone());
    let dest_root_exists = listings.get(dest, &dest_root).await?.is_some();

    // In scope: under the prefix, the prefix itself included, never the root.
    let mut scope: Vec<(&SourceItem, &Node)> = all
        .values()
        .filter_map(|it| Some((it, tree.node(&it.id)?)))
        .filter(|(_, n)| !n.segs.is_empty() && n.segs.starts_with(prefix.segments()))
        .collect();
    scope.sort_by(|(a, na), (b, nb)| {
        let ka = (a.kind == ItemKind::File, na.segs.len(), &na.segs);
        let kb = (b.kind == ItemKind::File, nb.segs.len(), &nb.segs);
        ka.cmp(&kb)
    });

    let mut entries = Vec::new();
    // Folders whose contents can't be mirrored where they are, and why.
    let mut blocked: HashMap<String, (Action, String)> = HashMap::new();
    for (it, node) in &scope {
        let row = known.get(&it.id);
        let shown = node.segs.join("/");
        let mut e = PlanEntry {
            source_id: it.id.clone(),
            source_path: shown.clone(),
            dest_path: format!("{}/{shown}", dest_root)
                .trim_start_matches('/')
                .to_string(),
            folder: it.kind == ItemKind::Folder,
            digests: it.digests.clone(),
            version: it.version.clone(),
            modified: it.modified,
            action: Action::UploadNew,
            reason: None,
            level: row.map_or(VerifyLevel::L0Planned, |r| r.verify_level),
        };
        let parent_block = it.parent_id.as_ref().and_then(|p| blocked.get(p)).cloned();
        let bad_name = dest.name_problem(&it.name, it.kind);
        let (action, reason) = match (&node.path, &it.special, parent_block) {
            (_, Some(special), _) => (
                Action::SkipUnsupported,
                Some(format!("not plain content ({special})")),
            ),
            (Err(why), None, _) => (Action::SkipUnsupported, Some(why.clone())),
            (Ok(_), None, Some((action, why))) => (action, Some(why)),
            (Ok(_), None, None) if bad_name.is_some() => (
                Action::SkipUnsupported,
                bad_name.map(|why| format!("its name can't be used at the destination: {why}")),
            ),
            (Ok(path), None, None) => {
                let dest_path = under(&dest_root, path);
                e.dest_path = dest_path.to_string();
                classify(it, path, row, &dest_path, dest, &mut listings, opts).await?
            }
        };
        e.action = action;
        e.reason = reason;
        // What is inside a folder that isn't mirrored isn't either. Children
        // of a moved folder are judged on their own: each either moved with
        // it (recorded) or is new.
        if e.folder {
            let why = match action {
                Action::Diverged => Some("its folder diverged at the destination"),
                Action::SkipUnsupported => Some("it is inside an item that is not mirrored"),
                _ => None,
            };
            if let Some(why) = why {
                blocked.insert(it.id.clone(), (action, why.to_string()));
            }
        }
        entries.push(e);
    }

    // Recorded items no longer found where they were.
    let mut gone: Vec<&ItemRow> = known
        .values()
        .filter(|r| r.state != ItemState::SourceDeleted)
        .filter(|r| match RelPath::parse(&r.source_path) {
            Ok(p) => starts_with(&p, &prefix),
            Err(_) => false,
        })
        .collect();
    gone.sort_by(|a, b| a.source_path.cmp(&b.source_path));
    for r in gone {
        let now_at = tree.node(&r.source_id).map(|n| &n.segs);
        // Only one folder was listed: something not in it may still be
        // elsewhere on the source, so ask for it by id.
        let elsewhere = match now_at {
            None if subtree => match source.item(&r.source_id).await {
                Ok(it) => !it.deleted,
                Err(ProviderError::NotFound(_)) => false,
                Err(e) => return Err(e.into()),
            },
            _ => false,
        };
        let (action, reason) = match now_at {
            Some(segs) if segs.starts_with(prefix.segments()) => continue,
            Some(segs) => (
                Action::SourceMoved,
                format!(
                    "moved on the source to {}, outside this mirror; recorded only",
                    segs.join("/")
                ),
            ),
            None if elsewhere => (
                Action::SourceMoved,
                "moved on the source to somewhere outside this mirror; recorded only".to_string(),
            ),
            None => (
                Action::SourceDeleted,
                "gone from the source; the destination keeps its copy".to_string(),
            ),
        };
        entries.push(PlanEntry {
            source_id: r.source_id.clone(),
            source_path: r.source_path.clone(),
            dest_path: r.dest_path.clone().unwrap_or_default(),
            folder: r.kind == ItemKind::Folder,
            digests: ContentDigests {
                size: r.size,
                qxh: r.qxh.clone(),
                ..Default::default()
            },
            version: r.ctag.clone(),
            modified: None,
            action,
            reason: Some(reason),
            level: r.verify_level,
        });
    }

    let case_collisions = case_collisions(&scope);
    let totals = totals(&entries, opts);
    Ok(Plan {
        created: Utc::now(),
        options: opts.clone(),
        dest_root_exists,
        entries,
        case_collisions,
        source_cursor: resume,
        totals,
    })
}

async fn classify<D: MirrorDestination>(
    it: &SourceItem,
    path: &RelPath,
    row: Option<&ItemRow>,
    dest_path: &RelPath,
    dest: &D,
    listings: &mut Listings,
    opts: &PlanOptions,
) -> MResult<(Action, Option<String>)> {
    let some = |s: &str| Some(s.to_string());

    // Recorded before, and something of it may be at the destination.
    if let Some(row) = row.filter(|r| r.reached_destination()) {
        if row.source_path != path.to_string() {
            return Ok((
                Action::SourceMoved,
                Some(format!(
                    "moved on the source from {} to {path}; recorded only, nothing moved at \
                     the destination",
                    row.source_path
                )),
            ));
        }
        if row.state == ItemState::Diverged {
            return Ok((Action::Diverged, row.detail.clone()));
        }
        if it.kind == ItemKind::File && content_changed(row, &it.digests, it.version.as_deref()) {
            return Ok(match opts.on_source_change {
                ChangedSourcePolicy::MarkDiverged => (
                    Action::Diverged,
                    some("changed on the source after it was mirrored; destination left as it was"),
                ),
                ChangedSourcePolicy::CreateNewRevision => (
                    Action::UploadRevision,
                    some("changed on the source after it was mirrored"),
                ),
            });
        }
        let done = match it.kind {
            ItemKind::Folder => row.state == ItemState::Verified,
            ItemKind::File => {
                row.state == ItemState::Verified && row.verify_level >= opts.target_level()
            }
        };
        if done {
            return Ok((Action::AlreadyMirrored, None));
        }
        let why = match &row.last_error {
            Some(e) => format!(
                "continuing from {} after {} failed attempt(s): {e}",
                row.verify_level.as_str(),
                row.attempts
            ),
            None => format!("continuing from {}", row.verify_level.as_str()),
        };
        return Ok((Action::Resume, Some(why)));
    }

    if it.kind == ItemKind::File && it.digests.size > opts.staging_cap {
        return Ok((
            Action::SkipTooLarge,
            Some(format!(
                "{} bytes is more than the staging cap of {} bytes",
                it.digests.size, opts.staging_cap
            )),
        ));
    }

    // What the destination already has under that name.
    let name = dest_path.name().unwrap_or_default();
    let parent = dest_path.parent().unwrap_or_default();
    let existing: Option<DestEntry> = listings
        .get(dest, &parent)
        .await?
        .and_then(|l| l.iter().find(|e| e.name == name).cloned());
    let retry_note = row
        .filter(|r| r.attempts > 0)
        .map(|r| format!("retrying after {} failed attempt(s)", r.attempts));

    match (it.kind, existing) {
        (ItemKind::Folder, None) => Ok((Action::CreateFolder, retry_note)),
        (ItemKind::Folder, Some(e)) if e.kind == ItemKind::Folder => {
            Ok((Action::FolderExists, None))
        }
        (ItemKind::Folder, Some(_)) => Ok((
            Action::Diverged,
            some("a file of the same name is at the destination; both kept"),
        )),
        (ItemKind::File, None) => Ok((Action::UploadNew, retry_note)),
        (ItemKind::File, Some(e)) if e.kind == ItemKind::Folder => Ok((
            Action::Diverged,
            some("a folder of the same name is at the destination; both kept"),
        )),
        (ItemKind::File, Some(e)) => {
            if e.size.is_some_and(|s| s != it.digests.size) {
                return Ok((
                    Action::Diverged,
                    some("a different file of the same name is at the destination; both kept"),
                ));
            }
            let st = match dest.stat(dest_path).await {
                Ok(st) => st,
                Err(ProviderError::NotFound(_)) => return Ok((Action::UploadNew, retry_note)),
                // About this one file (no claimed size, an unreadable
                // digest): it can't be compared, so it is left alone.
                Err(
                    e @ (ProviderError::Other(_)
                    | ProviderError::Conflict(_)
                    | ProviderError::Integrity(_)),
                ) => {
                    return Ok((
                        Action::Diverged,
                        Some(format!(
                            "the destination's file of the same name can't be compared ({e}); \
                             both kept"
                        )),
                    ))
                }
                Err(e) => return Err(e.into()),
            };
            match (&it.digests.sha1, &st.claimed.sha1) {
                (Some(ours), Some(theirs)) => {
                    if ours.eq_ignore_ascii_case(theirs) && st.claimed.size == it.digests.size {
                        Ok((
                            Action::AdoptExisting,
                            some("the destination already has this content under this name"),
                        ))
                    } else {
                        Ok((
                            Action::Diverged,
                            some("a different file of the same name is at the destination; both kept"),
                        ))
                    }
                }
                (_, None) => Ok((
                    Action::Diverged,
                    some("a file of the same name is at the destination and it reports no SHA-1 to compare; both kept"),
                )),
                (None, Some(_)) => Ok((
                    Action::CheckExisting,
                    some("a file of the same name is at the destination; compared after download"),
                )),
            }
        }
    }
}

/// The source's current version differs from the one recorded.
pub(crate) fn content_changed(row: &ItemRow, now: &ContentDigests, version: Option<&str>) -> bool {
    if row.size != now.size {
        return true;
    }
    match (&row.qxh, &now.qxh) {
        (Some(a), Some(b)) => a != b,
        // No hash on one side: fall back to the version tag.
        _ => match (row.ctag.as_deref(), version) {
            (Some(a), Some(b)) => a != b,
            _ => false,
        },
    }
}

fn starts_with(p: &RelPath, prefix: &RelPath) -> bool {
    p.segments().starts_with(prefix.segments())
}

fn under(root: &RelPath, p: &RelPath) -> RelPath {
    let mut segs = root.segments().to_vec();
    segs.extend(p.segments().iter().cloned());
    // Every segment was checked when it was joined.
    RelPath::parse(&segs.join("/")).expect("already-checked segments")
}

fn case_collisions(scope: &[(&SourceItem, &Node)]) -> Vec<Vec<String>> {
    let mut groups: BTreeMap<(Vec<String>, String), Vec<String>> = BTreeMap::new();
    for (_, n) in scope {
        let Some((name, parent)) = n.segs.split_last() else {
            continue;
        };
        groups
            .entry((parent.to_vec(), name.to_lowercase()))
            .or_default()
            .push(n.segs.join("/"));
    }
    groups
        .into_values()
        .filter(|g| g.len() > 1)
        .map(|mut g| {
            g.sort();
            g
        })
        .collect()
}

fn totals(entries: &[PlanEntry], opts: &PlanOptions) -> PlanTotals {
    let mut t = PlanTotals {
        staging_cap: opts.staging_cap,
        ..Default::default()
    };
    for e in entries {
        if matches!(e.action, Action::SourceDeleted) || e.source_path.is_empty() {
            // Counted by action only.
        } else if e.folder {
            t.folders += 1;
        } else {
            t.files += 1;
        }
        let size = if e.folder { 0 } else { e.digests.size };
        let c = t.by_action.entry(e.action).or_default();
        c.items += 1;
        c.bytes += size;
        if !e.folder && e.action.stages() {
            t.bytes_to_stage += size;
            t.staging_needed = t.staging_needed.max(size);
        }
        if !e.folder && matches!(e.action, Action::UploadNew | Action::UploadRevision) {
            t.bytes_to_upload += size;
        }
    }
    t
}

/// The source as a tree: every reachable item's path, worked out once.
struct Tree {
    nodes: HashMap<String, Node>,
}

#[derive(Clone)]
struct Node {
    /// The names from the root down, usable or not.
    segs: Vec<String>,
    /// The checked path, or why a name on the way can't be used.
    path: Result<RelPath, String>,
}

impl Tree {
    fn build(all: &HashMap<String, SourceItem>) -> Self {
        let mut nodes: HashMap<String, Node> = HashMap::new();
        for id in all.keys() {
            // Walk up to something already known (or the root), then back
            // down. Items whose parents are missing or loop are left out.
            let mut chain = Vec::new();
            let mut seen = HashSet::new();
            let mut cur = id.as_str();
            let mut base = loop {
                if let Some(n) = nodes.get(cur) {
                    break Some(n.clone());
                }
                if !seen.insert(cur) {
                    break None;
                }
                let Some(it) = all.get(cur) else {
                    break None;
                };
                match it.parent_id.as_deref() {
                    None => {
                        let root = Node {
                            segs: Vec::new(),
                            path: Ok(RelPath::root()),
                        };
                        nodes.insert(cur.to_string(), root.clone());
                        break Some(root);
                    }
                    Some(p) => {
                        chain.push(cur);
                        cur = p;
                    }
                }
            };
            for c in chain.into_iter().rev() {
                let Some(b) = base else { break };
                let name = &all[c].name;
                let mut segs = b.segs;
                segs.push(name.clone());
                let path = match b.path {
                    Ok(p) => p
                        .join(name)
                        .map_err(|e| format!("its name can't be used at the destination: {e}")),
                    Err(_) => Err("it is inside a folder whose name can't be used".into()),
                };
                let n = Node { segs, path };
                nodes.insert(c.to_string(), n.clone());
                base = Some(n);
            }
        }
        Self { nodes }
    }

    fn node(&self, id: &str) -> Option<&Node> {
        self.nodes.get(id)
    }
}

/// Destination folder listings, fetched once each and only for folders
/// known to exist.
struct Listings {
    root: RelPath,
    cache: HashMap<RelPath, Option<Vec<DestEntry>>>,
}

impl Listings {
    fn new(root: RelPath) -> Self {
        Self {
            root,
            cache: HashMap::new(),
        }
    }

    /// What is in `folder` (at or under the root), or `None` if the
    /// destination has no such folder.
    async fn get<D: MirrorDestination>(
        &mut self,
        dest: &D,
        folder: &RelPath,
    ) -> MResult<Option<&Vec<DestEntry>>> {
        if !starts_with(folder, &self.root) {
            return Err(MirrorError::Invalid(format!(
                "{folder} is outside the mirror's destination folder"
            )));
        }
        let segs = folder.segments();
        let mut at = self.root.clone();
        for depth in self.root.segments().len()..=segs.len() {
            if depth > self.root.segments().len() {
                at = at.join(&segs[depth - 1]).map_err(ProviderError::from)?;
            }
            if self.cache.contains_key(&at) {
                continue;
            }
            let exists = match at.parent() {
                _ if at == self.root => true,
                Some(parent) => match self.cache.get(&parent) {
                    Some(Some(list)) => list
                        .iter()
                        .any(|e| e.kind == ItemKind::Folder && Some(e.name.as_str()) == at.name()),
                    _ => false,
                },
                None => true,
            };
            let listing = if exists {
                match dest.list(&at).await {
                    Ok(l) => Some(l),
                    Err(ProviderError::NotFound(_)) => None,
                    Err(e) => return Err(e.into()),
                }
            } else {
                None
            };
            self.cache.insert(at.clone(), listing);
        }
        Ok(self.cache.get(folder).and_then(|l| l.as_ref()))
    }
}

impl Plan {
    pub fn to_json(&self) -> MResult<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    pub fn from_json(s: &str) -> MResult<Self> {
        Ok(serde_json::from_str(s)?)
    }

    pub fn count(&self, action: Action) -> u64 {
        self.totals.by_action.get(&action).map_or(0, |c| c.items)
    }

    /// A short, plain-text summary for people.
    pub fn render_text(&self) -> String {
        self.render_text_for(&format!(
            "/{}",
            self.options.dest_root.trim_start_matches('/')
        ))
    }

    /// [`render_text`](Self::render_text) with the destination folder
    /// shown as `dest_shown`, the way the destination writes its paths
    /// (such as `/my-files/OneDrive`).
    pub fn render_text_for(&self, dest_shown: &str) -> String {
        let t = &self.totals;
        let mut s = String::new();
        let scope = self
            .options
            .source_prefix
            .as_deref()
            .unwrap_or("/ (everything)");
        let _ = writeln!(s, "Mirror plan (dry run: nothing was changed)");
        let _ = writeln!(s, "  Source folder:      {scope}");
        let _ = writeln!(
            s,
            "  Destination folder: {dest_shown}{}",
            if self.dest_root_exists {
                ""
            } else {
                " (will be created)"
            }
        );
        let _ = writeln!(s, "  {} folders, {} files", t.folders, t.files);
        for (action, c) in &t.by_action {
            let _ = writeln!(
                s,
                "  {:<18} {:>7} item(s) {:>12}",
                action.as_str(),
                c.items,
                human_bytes(c.bytes)
            );
        }
        let _ = writeln!(
            s,
            "  To upload:          {}",
            human_bytes(t.bytes_to_upload)
        );
        let _ = writeln!(s, "  To stage:           {}", human_bytes(t.bytes_to_stage));
        let _ = writeln!(
            s,
            "  Staging needed:     {} at once (cap {})",
            human_bytes(t.staging_needed),
            human_bytes(t.staging_cap)
        );
        let _ = writeln!(
            s,
            "  Verification:       up to {}",
            self.options.target_level().as_str()
        );
        let notable = [
            Action::Diverged,
            Action::SkipTooLarge,
            Action::SkipUnsupported,
            Action::SourceDeleted,
            Action::SourceMoved,
        ];
        for action in notable {
            let list: Vec<&PlanEntry> =
                self.entries.iter().filter(|e| e.action == action).collect();
            if list.is_empty() {
                continue;
            }
            let _ = writeln!(s, "{}:", action.as_str());
            for e in list.iter().take(20) {
                let _ = writeln!(
                    s,
                    "  {} ({})",
                    e.source_path,
                    e.reason.as_deref().unwrap_or("")
                );
            }
            if list.len() > 20 {
                let _ = writeln!(s, "  ... and {} more", list.len() - 20);
            }
        }
        if !self.case_collisions.is_empty() {
            let _ = writeln!(
                s,
                "Names differing only in case ({} group(s), kept as distinct files):",
                self.case_collisions.len()
            );
            for g in self.case_collisions.iter().take(20) {
                let _ = writeln!(s, "  {}", g.join("  |  "));
            }
        }
        s
    }
}

pub(crate) fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

impl Plan {
    /// Record this dry run in `mirror_runs` (mode `dry`). Nothing else in
    /// the manifest changes. Returns the run's id.
    pub fn record_dry_run(&self, m: &mut crate::Manifest) -> MResult<i64> {
        let stamp = |t: DateTime<Utc>| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let id = m.begin_run("dry", &stamp(self.created))?;
        let files = |a: Action| {
            self.entries
                .iter()
                .filter(|e| !e.folder && e.action == a)
                .count() as u64
        };
        m.finish_run(&crate::RunRow {
            id,
            mode: "dry".into(),
            started: stamp(self.created),
            finished: Some(stamp(Utc::now())),
            outcome: Some("planned".into()),
            folders_created: self.count(Action::CreateFolder),
            files_mirrored: files(Action::UploadNew) + files(Action::UploadRevision),
            files_adopted: files(Action::AdoptExisting),
            files_diverged: files(Action::Diverged),
            files_skipped: files(Action::SkipTooLarge) + files(Action::SkipUnsupported),
            files_failed: 0,
            bytes_uploaded: 0,
            staging_peak: 0,
        })?;
        Ok(id)
    }
}
