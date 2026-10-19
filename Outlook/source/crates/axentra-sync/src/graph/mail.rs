//! Mail over Graph (§4.6–4.8): folder tree, per-folder message delta,
//! attachments, and pushing read / flag / Focused / categories / move /
//! delete changes.

use std::collections::{HashMap, HashSet};

use axentra_core::{Account, Attachment, FolderId, FolderRole, InferenceClass, Message, MessageId, Store};
use base64::Engine as _;
use chrono::Utc;
use serde_json::{Value, json};

use super::client::{Graph, seg};
use super::map::{GraphMessage, flag_patch, message_from_graph};
use crate::config::AccountConfig;
use crate::diff::{
    FolderOp, FolderState, KIND_FOLDER, KIND_MESSAGE, MessageOp, MessageState, Shadow, folder_ops, merge_server_message, message_ops, sent_drafts,
};
use crate::error::{SyncError, SyncResult};
use crate::imap::folders::{FolderCursor, RemoteFolder, apply_to_store};
use crate::notify::Notifier;

/// Message properties synced (one constant, §4.7).
pub const MESSAGE_SELECT: &str = "id,conversationId,internetMessageId,subject,bodyPreview,from,sender,toRecipients,ccRecipients,\
bccRecipients,replyTo,receivedDateTime,sentDateTime,isRead,isDraft,flag,importance,hasAttachments,categories,\
inferenceClassification,parentFolderId,changeKey,body";

/// Well-known folder names and the roles they back.
const WELL_KNOWN: &[(&str, FolderRole)] = &[
    ("inbox", FolderRole::Inbox),
    ("drafts", FolderRole::Drafts),
    ("sentitems", FolderRole::Sent),
    ("deleteditems", FolderRole::Deleted),
    ("junkemail", FolderRole::Junk),
    ("archive", FolderRole::Archive),
    ("conversationhistory", FolderRole::ConversationHistory),
];

/// Attachments larger than this keep metadata only.
const MAX_ATTACHMENT: i64 = 10 * 1024 * 1024;

pub struct GraphMail<'a> {
    pub graph: &'a Graph,
    pub store: &'a Store,
    pub account: &'a Account,
    pub cfg: &'a AccountConfig,
    pub notifier: &'a Notifier,
    /// Local folder → Graph folder id.
    pub remote_of: HashMap<FolderId, String>,
    local_of: HashMap<String, FolderId>,
    locals: HashMap<MessageId, Message>,
}

impl<'a> GraphMail<'a> {
    pub fn new(graph: &'a Graph, store: &'a Store, account: &'a Account, cfg: &'a AccountConfig, notifier: &'a Notifier) -> SyncResult<Self> {
        let locals = store.account_messages(account.id)?.into_iter().map(|m| (m.id, m)).collect();
        let mut s = GraphMail { graph, store, account, cfg, notifier, remote_of: HashMap::new(), local_of: HashMap::new(), locals };
        for (fid, remote, _) in store.sync_folder_list::<FolderCursor>(account.id)? {
            s.remote_of.insert(fid, remote.clone());
            s.local_of.insert(remote, fid);
        }
        Ok(s)
    }

    /// Ids of the well-known folders (stable; cached in a cursor).
    async fn well_known(&self) -> SyncResult<HashMap<String, FolderRole>> {
        let key = "graph:wellknown";
        if let Some(cached) = self.store.sync_cursor(self.account.id, key)?
            && let Ok(map) = serde_json::from_str::<HashMap<String, String>>(&cached)
            && !map.is_empty()
        {
            return Ok(map.into_iter().map(|(id, role)| (id, FolderRole::parse(&role))).collect());
        }
        let mut out = HashMap::new();
        for (name, role) in WELL_KNOWN {
            match self.graph.get(&format!("/me/mailFolders/{name}?$select=id")).await {
                Ok(v) => {
                    if let Some(id) = v["id"].as_str() {
                        out.insert(id.to_string(), *role);
                    }
                }
                Err(SyncError::NotFound) => {}
                Err(e) => return Err(e),
            }
        }
        let json: HashMap<&String, &str> = out.iter().map(|(id, r)| (id, r.as_str())).collect();
        self.store.set_sync_cursor(self.account.id, key, Some(&serde_json::to_string(&json).unwrap_or_default()))?;
        Ok(out)
    }

    /// Mirror the folder tree (recursive `childFolders` listing).
    pub async fn sync_folders(&mut self) -> SyncResult<()> {
        let roles = self.well_known().await?;
        let select = "$select=id,displayName,parentFolderId,childFolderCount,isHidden&$top=100";
        let mut all: Vec<Value> = self.graph.get_all(&format!("/me/mailFolders?{select}")).await?;
        let mut i = 0;
        while i < all.len() {
            if all[i]["childFolderCount"].as_u64().unwrap_or(0) > 0
                && let Some(id) = all[i]["id"].as_str().map(str::to_string)
            {
                let children = self.graph.get_all(&format!("/me/mailFolders/{}/childFolders?{select}", seg(&id))).await?;
                all.extend(children);
            }
            i += 1;
            if all.len() > 5000 {
                break;
            }
        }
        let ids: HashSet<String> = all.iter().filter_map(|f| f["id"].as_str().map(str::to_string)).collect();
        let remote: Vec<RemoteFolder> = all
            .iter()
            .filter(|f| !f["isHidden"].as_bool().unwrap_or(false))
            .filter_map(|f| {
                let id = f["id"].as_str()?.to_string();
                let parent = f["parentFolderId"].as_str().filter(|p| ids.contains(*p)).map(str::to_string);
                Some(RemoteFolder {
                    role: roles.get(&id).copied(),
                    name: id,
                    delimiter: None,
                    display: f["displayName"].as_str().unwrap_or_default().to_string(),
                    parent,
                    attributes: Vec::new(),
                    special: None,
                    selectable: true,
                })
            })
            .collect();
        let mapped = apply_to_store(self.store, self.account.id, &remote)?;
        self.remote_of = mapped.iter().map(|m| (m.folder_id, m.remote.name.clone())).collect();
        self.local_of = mapped.iter().map(|m| (m.remote.name.clone(), m.folder_id)).collect();
        self.notifier.bump();
        Ok(())
    }

    /// Whether folders were created, renamed, moved or deleted here.
    pub fn folders_pending(&self) -> SyncResult<bool> {
        crate::imap::folders::changes_pending(self.store, self.account.id)
    }

    /// Push folder creates, renames, moves and deletes (§4.6). Ids are
    /// stable across renames and moves, so only the shadow changes.
    pub async fn push_folders(&mut self) -> SyncResult<usize> {
        let account_id = self.account.id;
        let locals = self.store.folders(Some(account_id))?;
        let shadows = self.store.sync_list::<Shadow>(account_id, KIND_FOLDER, None)?;
        let ops = folder_ops(&locals, &shadows);
        if ops.is_empty() {
            return Ok(0);
        }
        let mut remote_of: HashMap<FolderId, String> = locals.iter().filter_map(|f| f.remote_id.clone().map(|r| (f.id, r))).collect();
        let by_id: HashMap<FolderId, &axentra_core::Folder> = locals.iter().map(|f| (f.id, f)).collect();
        let shadow_of: HashMap<FolderId, Shadow> = shadows.into_iter().collect();
        let mut pushed = 0;
        for op in ops {
            match op {
                FolderOp::Create(id) => {
                    let Some(f) = by_id.get(&id) else { continue };
                    let parent = match f.parent_id {
                        Some(p) => match remote_of.get(&p) {
                            Some(r) => Some(r.clone()),
                            None => continue,
                        },
                        None => None,
                    };
                    let path = match &parent {
                        Some(p) => format!("/me/mailFolders/{}/childFolders", seg(p)),
                        None => "/me/mailFolders".to_string(),
                    };
                    let rid = match self.graph.post(&path, json!({ "displayName": f.name })).await {
                        Ok(v) => v["id"].as_str().map(str::to_string),
                        // Already there (409 ErrorFolderExists): adopt the server's folder.
                        Err(SyncError::Protocol(m)) if m.contains("ErrorFolderExists") || m.contains("409") => {
                            let list = self.graph.get_all(&format!("{path}?$select=id,displayName&$top=100")).await?;
                            list.iter()
                                .find(|v| v["displayName"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(&f.name)))
                                .and_then(|v| v["id"].as_str().map(str::to_string))
                        }
                        Err(e) => return Err(e),
                    };
                    let Some(rid) = rid else { return Err(SyncError::protocol("Microsoft 365 didn't return the new folder's id.")) };
                    let mut folder = (*f).clone();
                    folder.remote_id = Some(rid.clone());
                    self.store.save_synced_folder(&mut folder)?;
                    let shadow = Shadow { remote_id: rid.clone(), state: FolderState::of(f).to_value(), ..Default::default() };
                    self.store.sync_put(account_id, KIND_FOLDER, id, &rid, "", &shadow)?;
                    self.store.sync_folder_put(id, account_id, &rid, &FolderCursor::default())?;
                    self.remote_of.insert(id, rid.clone());
                    self.local_of.insert(rid.clone(), id);
                    remote_of.insert(id, rid);
                    pushed += 1;
                }
                FolderOp::Update(id) => {
                    let (Some(f), Some(sh)) = (by_id.get(&id), shadow_of.get(&id)) else { continue };
                    let last = FolderState::from_value(&sh.state);
                    let path = format!("/me/mailFolders/{}", seg(&sh.remote_id));
                    let result: SyncResult<()> = async {
                        if f.name != last.name {
                            self.graph.patch(&path, json!({ "displayName": f.name })).await?;
                        }
                        if f.parent_id != last.parent {
                            let dest = match f.parent_id {
                                Some(p) => remote_of.get(&p).cloned(),
                                None => Some("msgfolderroot".to_string()),
                            };
                            if let Some(dest) = dest {
                                self.graph.post(&format!("{path}/move"), json!({ "destinationId": dest })).await?;
                            }
                        }
                        Ok(())
                    }
                    .await;
                    match result {
                        Ok(()) => {}
                        Err(SyncError::NotFound) => {
                            self.store.sync_remove(account_id, KIND_FOLDER, id)?;
                            continue;
                        }
                        Err(e) => return Err(e),
                    }
                    let shadow = Shadow { state: FolderState::of(f).to_value(), ..sh.clone() };
                    self.store.sync_put(account_id, KIND_FOLDER, id, &sh.remote_id, "", &shadow)?;
                    pushed += 1;
                }
                FolderOp::Delete { local_id, remote } => {
                    match self.graph.delete(&format!("/me/mailFolders/{}", seg(&remote))).await {
                        Ok(()) | Err(SyncError::NotFound) => {}
                        Err(e) => return Err(e),
                    }
                    self.store.sync_remove(account_id, KIND_FOLDER, local_id)?;
                    self.store.sync_remove_container(account_id, KIND_MESSAGE, &remote)?;
                    pushed += 1;
                }
            }
        }
        if pushed > 0 {
            self.notifier.bump();
        }
        Ok(pushed)
    }

    /// Delete the server copies of drafts that were sent from here
    /// (`sendMail` doesn't consume the draft; Graph files its own Sent copy).
    pub async fn delete_sent_drafts(&mut self) -> SyncResult<usize> {
        let account_id = self.account.id;
        let Some(drafts) = self.store.folder_by_role(account_id, FolderRole::Drafts)? else { return Ok(0) };
        let Some(container) = self.remote_of.get(&drafts.id).cloned() else { return Ok(0) };
        let sent_like: HashSet<FolderId> = [FolderRole::Outbox, FolderRole::Sent]
            .into_iter()
            .filter_map(|r| self.store.folder_by_role(account_id, r).ok().flatten().map(|f| f.id))
            .collect();
        let shadows = self.store.sync_list::<Shadow>(account_id, KIND_MESSAGE, Some(&container))?;
        let doomed = sent_drafts(&self.locals, &shadows, &sent_like);
        for (id, sh) in &doomed {
            match self.graph.delete(&format!("/me/messages/{}", seg(&sh.remote_id))).await {
                Ok(()) | Err(SyncError::NotFound) => {}
                Err(e) => return Err(e),
            }
            self.store.sync_remove(account_id, KIND_MESSAGE, *id)?;
        }
        Ok(doomed.len())
    }

    /// Delta-sync messages of the given folders (all mapped ones if `None`).
    pub async fn sync_messages(&mut self, only: Option<&[FolderRole]>) -> SyncResult<usize> {
        let mut folders: Vec<(FolderId, String, FolderRole)> = Vec::new();
        for (fid, remote) in &self.remote_of {
            let role = self.store.folder(*fid)?.map(|f| f.role).unwrap_or_default();
            if only.is_some_and(|o| !o.contains(&role)) {
                continue;
            }
            folders.push((*fid, remote.clone(), role));
        }
        // Inbox first so the list fills in quickly.
        folders.sort_by_key(|(_, _, r)| (*r != FolderRole::Inbox, *r == FolderRole::Custom));
        let mut changed = 0;
        let mut removed: Vec<(String, String)> = Vec::new();
        for (fid, remote, _) in folders {
            changed += match self.sync_folder(fid, &remote, &mut removed).await {
                Err(SyncError::ResyncRequired) => {
                    let mut c: FolderCursor = self.store.sync_folder_get(fid)?.unwrap_or_default();
                    c.delta_link = None;
                    self.store.sync_folder_put(fid, self.account.id, &remote, &c)?;
                    self.sync_folder(fid, &remote, &mut removed).await?
                }
                Err(SyncError::NotFound) => 0,
                other => other?,
            };
        }
        // "@removed" means "gone from that folder": only delete what didn't
        // reappear in another folder during this pass (moves keep their id).
        for (id, folder) in removed {
            if let Some((lid, sh)) = self.store.sync_find::<Shadow>(self.account.id, KIND_MESSAGE, &id)?
                && sh.container == folder
            {
                self.store.delete_messages_permanently(&[lid])?;
                self.store.sync_remove(self.account.id, KIND_MESSAGE, lid)?;
                self.locals.remove(&lid);
                changed += 1;
            }
        }
        if changed > 0 {
            self.notifier.bump();
        }
        Ok(changed)
    }

    async fn sync_folder(&mut self, folder_id: FolderId, remote: &str, removed: &mut Vec<(String, String)>) -> SyncResult<usize> {
        let mut cursor: FolderCursor = self.store.sync_folder_get(folder_id)?.unwrap_or_default();
        let url = match &cursor.delta_link {
            Some(link) => link.clone(),
            None => {
                let since = (Utc::now() - chrono::Duration::days(self.cfg.days_to_sync.max(1) as i64)).format("%Y-%m-%dT00:00:00Z");
                format!(
                    "/me/mailFolders/{}/messages/delta?$select={MESSAGE_SELECT}&$filter=receivedDateTime+ge+{since}",
                    seg(remote)
                )
            }
        };
        let (items, delta) = self.graph.delta(&url).await?;
        let mut changed = 0;
        for item in &items {
            let Some(g) = message_from_graph(item) else { continue };
            if g.removed {
                removed.push((g.id, remote.to_string()));
                continue;
            }
            if self.upsert(folder_id, remote, g, item).await? {
                changed += 1;
            }
            if changed % 25 == 24 {
                self.notifier.bump();
            }
        }
        cursor.delta_link = delta;
        self.store.sync_folder_put(folder_id, self.account.id, remote, &cursor)?;
        Ok(changed)
    }

    async fn upsert(&mut self, folder_id: FolderId, remote: &str, g: GraphMessage, raw: &Value) -> SyncResult<bool> {
        let account_id = self.account.id;
        let target_folder = self.local_of.get(&g.folder).copied().unwrap_or(folder_id);
        let container = if g.folder.is_empty() { remote.to_string() } else { g.folder.clone() };
        let server = MessageState { folder_id: target_folder, ..MessageState::of(&g.message) };
        if let Some((lid, mut sh)) = self.store.sync_find::<Shadow>(account_id, KIND_MESSAGE, &g.id)? {
            let mut last = MessageState::from_value(&sh.state);
            let mut changed = false;
            if let Some(local) = self.locals.get_mut(&lid) {
                changed = merge_server_message(local, &mut last, &server);
                if local.is_draft && sh.etag != g.change_key {
                    local.subject = g.message.subject.clone();
                    local.to = g.message.to.clone();
                    local.cc = g.message.cc.clone();
                    local.preview = g.message.preview.clone();
                    self.store.set_message_body(lid, g.body.html.as_deref(), &g.body.text)?;
                    changed = true;
                }
                if changed {
                    self.store.update_message(local)?;
                }
            } else {
                last = server;
            }
            sh.state = last.to_value();
            sh.etag = g.change_key.clone();
            sh.container = container.clone();
            self.store.sync_put(account_id, KIND_MESSAGE, lid, &g.id, &container, &sh)?;
            return Ok(changed);
        }
        // Our own sent / moved copy (no shadow yet): adopt it.
        if let Some(local) = self.store.folder_message_by_internet_id(target_folder, &g.message.internet_message_id)?
            && self.store.sync_get::<Shadow>(account_id, KIND_MESSAGE, local.id)?.is_none()
        {
            let mut m = local;
            m.remote_id = Some(g.id.clone());
            m.conversation_id = g.message.conversation_id.clone();
            m.is_read = g.message.is_read;
            self.store.update_message(&m)?;
            let sh = Shadow { remote_id: g.id.clone(), container: container.clone(), etag: g.change_key.clone(), state: MessageState::of(&m).to_value(), ..Default::default() };
            self.store.sync_put(account_id, KIND_MESSAGE, m.id, &g.id, &container, &sh)?;
            self.locals.insert(m.id, m);
            return Ok(true);
        }
        let mut m = g.message.clone();
        m.account_id = account_id;
        m.folder_id = target_folder;
        if m.meeting_kind.is_none() && raw["@odata.type"].as_str() == Some("#microsoft.graph.eventMessageRequest") {
            m.meeting_kind = Some(axentra_core::MeetingMessageKind::Request);
        }
        let wants_attachments = m.has_attachments || g.body.html.as_deref().is_some_and(|h| h.contains("cid:"));
        let attachments = if wants_attachments { self.attachments(&g.id).await.unwrap_or_default() } else { Vec::new() };
        let id = self.store.insert_synced_message(&mut m, &g.body, &attachments)?;
        let sh = Shadow { remote_id: g.id.clone(), container: container.clone(), etag: g.change_key.clone(), state: MessageState::of(&m).to_value(), ..Default::default() };
        self.store.sync_put(account_id, KIND_MESSAGE, id, &g.id, &container, &sh)?;
        self.locals.insert(id, m);
        Ok(true)
    }

    /// Attachment metadata, with bytes for files up to 10 MB (§4.8).
    async fn attachments(&self, message_id: &str) -> SyncResult<Vec<(Attachment, Option<Vec<u8>>)>> {
        let base = format!("/me/messages/{}/attachments", seg(message_id));
        let list = self.graph.get_all(&format!("{base}?$select=id,name,contentType,size,isInline")).await?;
        let mut out = Vec::new();
        for a in list {
            let Some(id) = a["id"].as_str() else { continue };
            let size = a["size"].as_i64().unwrap_or(0);
            let mut att = Attachment {
                name: a["name"].as_str().unwrap_or("attachment").to_string(),
                content_type: a["contentType"].as_str().unwrap_or("application/octet-stream").to_string(),
                size,
                is_inline: a["isInline"].as_bool().unwrap_or(false),
                ..Default::default()
            };
            let mut data = None;
            if size <= MAX_ATTACHMENT {
                let full = self.graph.get(&format!("{base}/{}", seg(id))).await?;
                if full["@odata.type"].as_str() == Some("#microsoft.graph.fileAttachment") {
                    att.content_id = full["contentId"].as_str().map(|c| c.trim_matches(['<', '>']).to_string()).filter(|c| !c.is_empty());
                    data = full["contentBytes"].as_str().and_then(|b| base64::engine::general_purpose::STANDARD.decode(b).ok());
                }
            }
            out.push((att, data));
        }
        Ok(out)
    }

    /// Push local changes for every synced message of the account.
    pub async fn push(&mut self) -> SyncResult<usize> {
        let account_id = self.account.id;
        let shadows = self.store.sync_list::<Shadow>(account_id, KIND_MESSAGE, None)?;
        let mut pushed = 0;
        for (lid, mut sh) in shadows {
            let last = MessageState::from_value(&sh.state);
            let local = self.locals.get(&lid).cloned();
            let ops = message_ops(local.as_ref(), &last);
            if ops.is_empty() {
                continue;
            }
            let path = format!("/me/messages/{}", seg(&sh.remote_id));
            let result: SyncResult<()> = async {
                let mut patch = serde_json::Map::new();
                for op in &ops {
                    match op {
                        MessageOp::Delete => {
                            self.graph.delete(&path).await?;
                        }
                        MessageOp::Move(target) => {
                            if let Some(dest) = self.remote_of.get(target) {
                                let moved = self.graph.post(&format!("{path}/move"), json!({"destinationId": dest})).await?;
                                if let Some(new_id) = moved["id"].as_str() {
                                    sh.remote_id = new_id.to_string();
                                }
                                sh.container = dest.clone();
                            }
                        }
                        MessageOp::Read(r) => {
                            patch.insert("isRead".into(), json!(r));
                        }
                        MessageOp::Flag(status, due) => {
                            if let Value::Object(f) = flag_patch(*status, *due) {
                                patch.extend(f);
                            }
                        }
                        MessageOp::Inference(class) => {
                            patch.insert("inferenceClassification".into(), json!(if *class == InferenceClass::Other { "other" } else { "focused" }));
                        }
                        MessageOp::Categories(c) => {
                            patch.insert("categories".into(), json!(c));
                        }
                    }
                }
                if !patch.is_empty() {
                    self.graph.patch(&format!("/me/messages/{}", seg(&sh.remote_id)), Value::Object(patch)).await?;
                }
                Ok(())
            }
            .await;
            match result {
                Ok(()) => {}
                Err(SyncError::NotFound) => {
                    // Gone on the server: drop the pending change and the local copy.
                    self.store.sync_remove(account_id, KIND_MESSAGE, lid)?;
                    if local.is_some() {
                        self.store.delete_messages_permanently(&[lid])?;
                        self.locals.remove(&lid);
                    }
                    pushed += 1;
                    continue;
                }
                Err(e) => return Err(e),
            }
            match local {
                None => self.store.sync_remove(account_id, KIND_MESSAGE, lid)?,
                Some(m) => {
                    let moved_ok = self.remote_of.contains_key(&m.folder_id);
                    let state = MessageState { folder_id: if moved_ok { m.folder_id } else { last.folder_id }, ..MessageState::of(&m) };
                    sh.state = state.to_value();
                    self.store.sync_put(account_id, KIND_MESSAGE, lid, &sh.remote_id.clone(), &sh.container.clone(), &sh)?;
                }
            }
            pushed += 1;
        }
        if pushed > 0 {
            self.notifier.bump();
        }
        Ok(pushed)
    }
}
