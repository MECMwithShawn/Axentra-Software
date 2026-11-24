//! The two sides of a mirror: a source read from and a destination copied to.
//!
//! Neither trait has a method that deletes, moves, renames or trashes
//! anything, on purpose: code that only holds these traits cannot express a
//! destructive call. Keep it that way. A source that loses a file, or
//! renames it, is the mirror's to record, never to repeat on the destination.
//!
//! Methods return `impl Future + Send` (native async in traits, no boxing),
//! so the traits are used through generics rather than `dyn`.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::future::Future;
use std::path::Path;

use chrono::{DateTime, Utc};

use crate::digests::ContentDigests;
use crate::error::{PResult, ProviderError};
use crate::path::RelPath;

/// Who a provider is signed in as.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Account {
    /// Which provider, such as `onedrive` or `mem`.
    pub provider: String,
    /// The provider's stable id for the drive or account.
    pub id: String,
    pub display_name: Option<String>,
    pub email: Option<String>,
    /// The provider's own word for the account type (OneDrive's
    /// `personal` or `business`).
    pub kind: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ItemKind {
    File,
    Folder,
}

/// One entry of a source's change feed, or a looked-up item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceItem {
    pub id: String,
    pub name: String,
    /// `None` for the root.
    pub parent_id: Option<String>,
    pub kind: ItemKind,
    /// Gone from the source. Only ever recorded, never acted on.
    pub deleted: bool,
    /// Not plain content, such as a OneNote `package` or a `remoteItem`
    /// shortcut into someone else's drive.
    pub special: Option<String>,
    /// What the source claims for this version of the content.
    pub digests: ContentDigests,
    pub modified: Option<DateTime<Utc>>,
    /// Changes when the content does (OneDrive's cTag).
    pub version: Option<String>,
}

/// One page of a source's change feed.
#[derive(Debug, Clone, Default)]
pub struct ChangePage {
    pub items: Vec<SourceItem>,
    /// More pages follow in this round: pass this to get the next one.
    pub next: Option<String>,
    /// The round is complete: save this and pass it next time to get only
    /// what changed since.
    pub resume: Option<String>,
}

/// An entry in a destination folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DestEntry {
    pub name: String,
    /// The destination's own id for it.
    pub uid: String,
    pub kind: ItemKind,
    pub size: Option<u64>,
}

/// What a destination says it holds for a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteStat {
    pub uid: String,
    /// Claimed by the destination, not computed here: matching them is
    /// [`VerifyLevel::L3RemoteClaimed`](crate::VerifyLevel::L3RemoteClaimed)
    /// at best.
    pub claimed: ContentDigests,
    /// The destination's id for the current revision, if it has one.
    pub revision: Option<String>,
}

/// Something a provider may or may not be able to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    /// A resumable change feed, not just listings.
    ChangeFeed,
    /// Reports a QuickXorHash for files.
    QuickXorHash,
    /// Reports a SHA-1 for files.
    Sha1,
    /// Reports a SHA-256 for files.
    Sha256,
    /// Creates folders.
    CreateFolder,
    /// Keeps earlier revisions when a file is uploaded again.
    Revisions,
    /// Can download what it stores, for round-trip verification.
    RoundTrip,
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Capability::ChangeFeed => "change feed",
            Capability::QuickXorHash => "QuickXorHash",
            Capability::Sha1 => "SHA-1",
            Capability::Sha256 => "SHA-256",
            Capability::CreateFolder => "creating folders",
            Capability::Revisions => "file revisions",
            Capability::RoundTrip => "downloading back",
        })
    }
}

/// What a provider says it can do. Asking for anything else gets
/// [`ProviderError::Unsupported`], never a silent fallback.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Capabilities {
    pub change_feed: bool,
    pub quick_xor_hash: bool,
    pub sha1: bool,
    pub sha256: bool,
    pub create_folder: bool,
    pub revisions: bool,
    pub round_trip: bool,
}

impl Capabilities {
    pub fn supports(&self, cap: Capability) -> bool {
        match cap {
            Capability::ChangeFeed => self.change_feed,
            Capability::QuickXorHash => self.quick_xor_hash,
            Capability::Sha1 => self.sha1,
            Capability::Sha256 => self.sha256,
            Capability::CreateFolder => self.create_folder,
            Capability::Revisions => self.revisions,
            Capability::RoundTrip => self.round_trip,
        }
    }

    pub fn require(&self, cap: Capability) -> PResult<()> {
        if self.supports(cap) {
            Ok(())
        } else {
            Err(ProviderError::Unsupported(cap))
        }
    }
}

/// Where files are mirrored from. Read-only.
pub trait SourceProvider: Send + Sync {
    fn capabilities(&self) -> Capabilities;

    fn account(&self) -> impl Future<Output = PResult<Account>> + Send;

    /// One page of changes. `None` starts a full enumeration; otherwise pass
    /// a [`ChangePage::next`] or a saved [`ChangePage::resume`]. A cursor
    /// too old to use gives [`ProviderError::ResyncRequired`].
    fn change_page(&self, cursor: Option<&str>)
        -> impl Future<Output = PResult<ChangePage>> + Send;

    fn item(&self, id: &str) -> impl Future<Output = PResult<SourceItem>> + Send;

    /// Everything at and under `path`, plus the folders above it up to the
    /// root (so each item's place can be worked out from `parent_id`s, as
    /// with [`change_page`](Self::change_page)), without deleted items.
    /// Reads only, and leaves no cursor behind: it is a listing, not a
    /// round of the change feed. Nothing at `path` is
    /// [`ProviderError::NotFound`]. `progress` is told how many items have
    /// been seen so far, now and then.
    ///
    /// The default reads the whole change feed from no cursor and keeps
    /// what is in scope; a provider that can list one folder should do
    /// that instead.
    fn enumerate_subtree(
        &self,
        path: &RelPath,
        progress: &(dyn Fn(u64) + Sync),
    ) -> impl Future<Output = PResult<Vec<SourceItem>>> + Send {
        async move {
            let mut all: HashMap<String, SourceItem> = HashMap::new();
            let mut cursor: Option<String> = None;
            let mut seen = 0u64;
            loop {
                let page = self.change_page(cursor.as_deref()).await?;
                seen += page.items.len() as u64;
                progress(seen);
                for it in page.items {
                    if it.deleted {
                        all.remove(&it.id);
                    } else {
                        all.insert(it.id.clone(), it);
                    }
                }
                match page.next {
                    Some(next) => cursor = Some(next),
                    None => break,
                }
            }
            subtree_of(all, path)
        }
    }

    /// Download `item`'s content to `dest`, which must not exist yet. The
    /// bytes go to a temporary file beside it first and are checked against
    /// `item.digests`; only a match is put in place. A mismatch gives
    /// [`ProviderError::Integrity`] and leaves nothing behind. Returns the
    /// digests computed from the bytes.
    fn download_to(
        &self,
        item: &SourceItem,
        dest: &Path,
    ) -> impl Future<Output = PResult<ContentDigests>> + Send;
}

/// What of `all` (one whole enumeration) is at or under `path`, plus the
/// folders above it. Items whose parents are missing or loop are left out.
fn subtree_of(all: HashMap<String, SourceItem>, path: &RelPath) -> PResult<Vec<SourceItem>> {
    /// The names from the root down to `id`, remembered in `segs`.
    fn walk(
        id: &str,
        all: &HashMap<String, SourceItem>,
        segs: &mut HashMap<String, Option<Vec<String>>>,
    ) -> Option<Vec<String>> {
        let mut chain = Vec::new();
        let mut seen = HashSet::new();
        let mut cur = id.to_string();
        let mut base = loop {
            if let Some(known) = segs.get(&cur) {
                break known.clone();
            }
            if !seen.insert(cur.clone()) {
                break None;
            }
            let Some(it) = all.get(&cur) else {
                break None;
            };
            match &it.parent_id {
                None => {
                    segs.insert(cur.clone(), Some(Vec::new()));
                    break Some(Vec::new());
                }
                Some(p) => {
                    chain.push(cur.clone());
                    cur = p.clone();
                }
            }
        };
        for c in chain.into_iter().rev() {
            base = base.map(|mut s| {
                s.push(all[&c].name.clone());
                s
            });
            segs.insert(c, base.clone());
        }
        base
    }

    let want = path.segments();
    let mut segs = HashMap::new();
    let mut found = false;
    let mut out = Vec::new();
    for (id, it) in &all {
        let Some(s) = walk(id, &all, &mut segs) else {
            continue;
        };
        found |= s.as_slice() == want;
        if s.starts_with(want) || want.starts_with(&s) {
            out.push(it.clone());
        }
    }
    if !found {
        return Err(ProviderError::NotFound(format!("nothing at /{path}")));
    }
    Ok(out)
}

/// Where files are mirrored to. Adds and reads only.
pub trait MirrorDestination: Send + Sync {
    fn capabilities(&self) -> Capabilities;

    /// Why an item of `kind` called `name` can't be put at this
    /// destination, or `None` if it can. Planning skips such items (and
    /// what is inside such folders) instead of failing them on every run.
    fn name_problem(&self, name: &str, kind: ItemKind) -> Option<&'static str> {
        let _ = (name, kind);
        None
    }

    fn account(&self) -> impl Future<Output = PResult<Account>> + Send;

    /// What is directly inside `folder`.
    fn list(&self, folder: &RelPath) -> impl Future<Output = PResult<Vec<DestEntry>>> + Send;

    /// Make `folder` and any missing parents; fine if it exists. Returns
    /// its uid. A file in the way is a [`ProviderError::Conflict`].
    fn ensure_folder(&self, folder: &RelPath) -> impl Future<Output = PResult<String>> + Send;

    /// Upload `local` as a new file `name` in `folder`. A name already taken
    /// is a [`ProviderError::Conflict`]: nothing is overwritten.
    fn upload_new(
        &self,
        folder: &RelPath,
        name: &str,
        local: &Path,
    ) -> impl Future<Output = PResult<RemoteStat>> + Send;

    /// Upload `local` as a new revision of the existing `file`, keeping the
    /// earlier ones. [`ProviderError::Unsupported`] without
    /// [`Capability::Revisions`].
    fn upload_revision(
        &self,
        file: &RelPath,
        local: &Path,
    ) -> impl Future<Output = PResult<RemoteStat>> + Send;

    /// What the destination claims for `file`.
    fn stat(&self, file: &RelPath) -> impl Future<Output = PResult<RemoteStat>> + Send;

    /// Download `file` to `dest` (which must not exist yet) for round-trip
    /// verification. Returns digests computed here from the bytes received,
    /// never ones echoed from the destination.
    fn download_to(
        &self,
        file: &RelPath,
        dest: &Path,
    ) -> impl Future<Output = PResult<ContentDigests>> + Send;
}
