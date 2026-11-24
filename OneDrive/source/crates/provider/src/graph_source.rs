//! OneDrive as a read-only [`SourceProvider`], over the existing [`Graph`]
//! client.
//!
//! Only `Graph`'s reading calls are used (`me`, `drive`, `delta_page`,
//! `root`, `child`, `children_page`, `item`, `download`), so this can never change anything on OneDrive. The
//! sync engine is untouched: a mirror gets its own `Graph`, sharing the
//! engine's `TokenStore`.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::Arc;

use axentra_onedrive_graph::{DriveItem, Graph, GraphError};

use crate::digests::ContentDigests;
use crate::error::{PResult, ProviderError};
use crate::path::RelPath;
use crate::staging::{parent_dir, verify_and_publish, StagedTemp};
use crate::traits::{Account, Capabilities, ChangePage, ItemKind, SourceItem, SourceProvider};

pub struct GraphSource {
    graph: Arc<Graph>,
}

impl GraphSource {
    pub fn new(graph: Arc<Graph>) -> Self {
        Self { graph }
    }
}

impl From<GraphError> for ProviderError {
    fn from(e: GraphError) -> Self {
        if e.is_offline() {
            return ProviderError::Offline(e.to_string());
        }
        match e {
            GraphError::NotSignedIn => ProviderError::AuthRequired,
            GraphError::Api { status: 401, .. } => ProviderError::AuthRequired,
            GraphError::Api { status: 404, .. } => ProviderError::NotFound(e.to_string()),
            GraphError::Api {
                status: 409 | 412, ..
            } => ProviderError::Conflict(e.to_string()),
            GraphError::Api { status: 410, .. } => ProviderError::ResyncRequired,
            // `Graph` already waited out any Retry-After it was given.
            GraphError::Api { status: 429, .. } => ProviderError::RateLimited { retry_after: None },
            // 408 is a timeout and 509 OneDrive's bandwidth limit: both pass.
            GraphError::Api {
                status: 408 | 500 | 502 | 503 | 504 | 509,
                ..
            } => ProviderError::Transient(e.to_string()),
            GraphError::Network(_) => ProviderError::Transient(e.to_string()),
            GraphError::Io(io) => ProviderError::Local(io),
            other => ProviderError::Other(other.to_string()),
        }
    }
}

fn source_item(it: DriveItem) -> SourceItem {
    let hashes = it.file.as_ref().and_then(|f| f.hashes.as_ref());
    let special = if it.package.is_some() {
        Some("package".to_string())
    } else if it.remote_item.is_some() {
        Some("remoteItem".to_string())
    } else {
        None
    };
    SourceItem {
        kind: if it.is_folder() {
            ItemKind::Folder
        } else {
            ItemKind::File
        },
        deleted: it.is_deleted(),
        special,
        digests: ContentDigests {
            size: it.size.unwrap_or(0),
            qxh: it.quick_xor_hash().map(str::to_string),
            sha1: hashes
                .and_then(|h| h.sha1_hash.as_deref())
                .map(str::to_ascii_lowercase),
            sha256: hashes
                .and_then(|h| h.sha256_hash.as_deref())
                .map(str::to_ascii_lowercase),
        },
        modified: it.modified(),
        parent_id: it.parent_id().map(str::to_string),
        name: it.name.unwrap_or_default(),
        version: it.c_tag,
        id: it.id,
    }
}

impl SourceProvider for GraphSource {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            change_feed: true,
            quick_xor_hash: true,
            ..Default::default()
        }
    }

    async fn account(&self) -> PResult<Account> {
        let me = self.graph.me().await?;
        let drive = self.graph.drive().await?;
        Ok(Account {
            provider: "onedrive".into(),
            id: drive.id,
            display_name: me.display_name,
            email: me.mail.or(me.user_principal_name),
            kind: drive.drive_type,
        })
    }

    async fn change_page(&self, cursor: Option<&str>) -> PResult<ChangePage> {
        let page = self.graph.delta_page(cursor).await?;
        Ok(ChangePage {
            items: page.items.into_iter().map(source_item).collect(),
            next: page.next_link,
            resume: page.delta_link,
        })
    }

    async fn item(&self, id: &str) -> PResult<SourceItem> {
        Ok(source_item(self.graph.item(id).await?))
    }

    /// The folder is found one name at a time from the root (which also
    /// gives the folders above it), then listed breadth-first, page by
    /// page. GET requests only, and no change-feed cursor is involved.
    async fn enumerate_subtree(
        &self,
        path: &RelPath,
        progress: &(dyn Fn(u64) + Sync),
    ) -> PResult<Vec<SourceItem>> {
        let not_found =
            || ProviderError::NotFound(format!("no folder or file at /{path} on OneDrive"));
        let mut out = Vec::new();
        let mut at = self.graph.root().await?;
        for name in path.segments() {
            if !at.is_folder() || at.package.is_some() || at.remote_item.is_some() {
                return Err(not_found());
            }
            let child = match self.graph.child(&at.id, name).await? {
                // OneDrive looks names up ignoring case; the mirror's paths
                // don't, so only the exact name will do.
                Some(c) if c.name.as_deref() == Some(name.as_str()) => c,
                _ => return Err(not_found()),
            };
            out.push(source_item(std::mem::replace(&mut at, child)));
        }
        let mut folders = VecDeque::new();
        if at.is_folder() && at.package.is_none() && at.remote_item.is_none() {
            folders.push_back(at.id.clone());
        }
        out.push(source_item(at));
        let mut seen = 1u64;
        progress(seen);
        while let Some(folder) = folders.pop_front() {
            let mut link: Option<String> = None;
            loop {
                let page = self.graph.children_page(&folder, link.as_deref()).await?;
                seen += page.items.len() as u64;
                progress(seen);
                for it in page.items {
                    if it.is_deleted() {
                        continue;
                    }
                    // What is inside a package or a shortcut is not listed:
                    // the planner skips such items with everything in them.
                    if it.is_folder() && it.package.is_none() && it.remote_item.is_none() {
                        folders.push_back(it.id.clone());
                    }
                    out.push(source_item(it));
                }
                match page.next_link {
                    Some(next) => link = Some(next),
                    None => break,
                }
            }
        }
        Ok(out)
    }

    async fn download_to(&self, item: &SourceItem, dest: &Path) -> PResult<ContentDigests> {
        if item.kind != ItemKind::File || item.deleted || item.special.is_some() {
            return Err(ProviderError::Other(format!(
                "{} is not a plain file",
                item.name
            )));
        }
        // Without OneDrive's hash nothing could be verified: fail closed.
        if item.digests.qxh.is_none() {
            return Err(ProviderError::Integrity(format!(
                "OneDrive gave no QuickXorHash for {}",
                item.name
            )));
        }
        let mut tmp = StagedTemp::create_in(parent_dir(dest))?;
        // Written through the handle `tmp` opened, never by path, so nothing
        // swapped in at the temporary name is followed or truncated.
        // Dropping `tmp` on an error removes it.
        let mut file = tokio::fs::File::from_std(tmp.file().try_clone()?);
        self.graph
            .download_into(&item.id, &mut file, &|_| {})
            .await?;
        drop(file);
        verify_and_publish(tmp, Some(&item.digests), dest).await
    }
}
