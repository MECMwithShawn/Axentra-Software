//! Per-folder IMAP sync (§5.5) and pushing local changes back (§5.7).
//!
//! For every selectable folder: SELECT, check UIDVALIDITY, list the UIDs in
//! the sync window, download the ones we don't have (`BODY.PEEK[]`, so
//! `\Seen` is never set by syncing), refresh flags of the ones we do
//! (CONDSTORE `CHANGEDSINCE` when available), drop the ones that vanished,
//! then push local read/flag/move/delete changes for that folder.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use async_imap::imap_proto::{AttributeValue, Response};
use async_imap::types::Flag;
use axentra_core::{Account, FlagStatus, FolderId, FolderRole, Message, MessageId, Store};
use chrono::{DateTime, Utc};

use super::folders::{self, FolderCursor, MappedFolder, RemoteFolder, Special};
use super::{Imap, Transport, drain_unsolicited, imap_err, quote, raw_command, raw_command_timeout, uid_set};
use crate::config::AccountConfig;
use crate::diff::{KIND_MESSAGE, MessageOp, MessageState, Shadow, merge_server_message, message_ops, sent_drafts};
use crate::error::{SyncError, SyncResult};
use crate::mime::parse::{ParsedMail, parse_rfc822};
use crate::mime::thread::conversation_id;
use crate::notify::Notifier;

/// Newest messages kept per folder (§5.5 step 3, estimate).
const MAX_WINDOW: usize = 5000;
/// Messages per body FETCH.
const FETCH_BATCH: usize = 20;
/// Bodies above this size are not downloaded (header only).
const MAX_BODY: u32 = 25 * 1024 * 1024;
const FETCH_TIMEOUT: Duration = Duration::from_secs(300);
/// Axentra-private keyword for completed follow-up flags (§5.6).
pub const COMPLETED_KEYWORD: &str = "$Completed";

/// Account-wide state for one sync pass.
pub struct MailSync<'a> {
    pub store: &'a Store,
    pub account: &'a Account,
    pub cfg: &'a AccountConfig,
    pub notifier: &'a Notifier,
    /// Local folder id → server mailbox name.
    pub remote_of: HashMap<FolderId, String>,
    /// Gmail `\All` mailbox, the target of Archive.
    pub all_mail: Option<String>,
    /// Every local message of the account, kept current during the pass.
    pub locals: HashMap<MessageId, Message>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FolderReport {
    pub new: usize,
    pub updated: usize,
    pub deleted: usize,
    pub pushed: usize,
}

impl std::ops::AddAssign for FolderReport {
    fn add_assign(&mut self, o: Self) {
        self.new += o.new;
        self.updated += o.updated;
        self.deleted += o.deleted;
        self.pushed += o.pushed;
    }
}

/// One `FETCH` response.
#[derive(Clone, Debug, Default)]
pub struct Fetched {
    pub uid: u32,
    pub flags: Vec<String>,
    pub internal_date: Option<DateTime<Utc>>,
    pub size: Option<u32>,
    pub body: Option<Vec<u8>>,
    pub header: Option<Vec<u8>>,
    pub modseq: Option<u64>,
    pub gmail_thread: Option<u64>,
}

/// Parse an IMAP INTERNALDATE (" 7-Jul-1996 02:44:25 -0700").
pub fn parse_internal_date(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_str(s.trim(), "%d-%b-%Y %H:%M:%S %z").ok().map(|d| d.with_timezone(&Utc))
}

/// `SINCE` date argument ("19-Oct-2026").
pub fn search_date(d: chrono::NaiveDate) -> String {
    d.format("%-d-%b-%Y").to_string()
}

/// Read flags into (is_read, flag, is_draft, deleted).
pub fn read_flags(flags: &[String]) -> (bool, FlagStatus, bool, bool) {
    let has = |f: &str| flags.iter().any(|x| x.eq_ignore_ascii_case(f));
    let flag = if has("\\Flagged") {
        FlagStatus::Flagged
    } else if has(COMPLETED_KEYWORD) {
        FlagStatus::Complete
    } else {
        FlagStatus::NotFlagged
    };
    (has("\\Seen"), flag, has("\\Draft"), has("\\Deleted"))
}

fn message_key(uidvalidity: u32, uid: u32, mailbox: &str) -> String {
    format!("{uidvalidity}:{uid}:{mailbox}")
}

pub async fn fetch<T: Transport>(imap: &mut Imap<T>, set: &str, items: &str) -> SyncResult<Vec<Fetched>> {
    if set.is_empty() {
        return Ok(Vec::new());
    }
    let mut out: Vec<Fetched> = Vec::new();
    raw_command_timeout(&mut imap.session, &format!("UID FETCH {set} {items}"), FETCH_TIMEOUT, |r| {
        if let Response::Fetch(_, attrs) = r {
            let mut f = Fetched::default();
            for a in attrs {
                match a {
                    AttributeValue::Uid(u) => f.uid = *u,
                    AttributeValue::Flags(fl) => f.flags = fl.iter().map(|s| s.to_string()).collect(),
                    AttributeValue::InternalDate(d) => f.internal_date = parse_internal_date(d),
                    AttributeValue::Rfc822Size(s) => f.size = Some(*s),
                    AttributeValue::ModSeq(m) => f.modseq = Some(*m),
                    AttributeValue::GmailThrId(t) => f.gmail_thread = Some(*t),
                    AttributeValue::Rfc822(Some(b)) => f.body = Some(b.to_vec()),
                    AttributeValue::Rfc822Header(Some(b)) => f.header = Some(b.to_vec()),
                    AttributeValue::BodySection { section: None, data: Some(b), .. } => f.body = Some(b.to_vec()),
                    AttributeValue::BodySection { section: Some(_), data: Some(b), .. } => f.header = Some(b.to_vec()),
                    _ => {}
                }
            }
            if f.uid != 0 {
                out.push(f);
            }
        }
    })
    .await?;
    drain_unsolicited(&mut imap.session);
    Ok(out)
}

async fn uid_search<T: Transport>(imap: &mut Imap<T>, query: &str) -> SyncResult<HashSet<u32>> {
    let r = tokio::time::timeout(super::COMMAND_TIMEOUT, imap.session.uid_search(query))
        .await
        .map_err(|_| SyncError::Offline(crate::error::copy::OFFLINE.into()))?
        .map_err(|e| imap_err(e, "server"));
    drain_unsolicited(&mut imap.session);
    r
}

impl MailSync<'_> {
    pub fn new<'a>(store: &'a Store, account: &'a Account, cfg: &'a AccountConfig, notifier: &'a Notifier) -> SyncResult<MailSync<'a>> {
        let locals = store.account_messages(account.id)?.into_iter().map(|m| (m.id, m)).collect();
        Ok(MailSync { store, account, cfg, notifier, remote_of: HashMap::new(), all_mail: None, locals })
    }

    /// Server mailboxes holding messages with unpushed read / flag / move /
    /// delete changes.
    pub fn pending_containers(&self) -> SyncResult<HashSet<String>> {
        let mut out = HashSet::new();
        for (lid, sh) in self.store.sync_list::<Shadow>(self.account.id, KIND_MESSAGE, None)? {
            if sh.uid.is_none() {
                continue;
            }
            let last = MessageState::from_value(&sh.state);
            let ops = message_ops(self.locals.get(&lid), &last);
            if ops.iter().any(|o| !matches!(o, MessageOp::Inference(_) | MessageOp::Categories(_))) {
                out.insert(sh.container);
            }
        }
        Ok(out)
    }

    /// Select a mailbox and push its pending changes (no pull).
    pub async fn push_only<T: Transport>(&mut self, imap: &mut Imap<T>, mailbox: &str) -> SyncResult<usize> {
        let mbox = match imap.session.select(mailbox).await {
            Ok(m) => m,
            Err(async_imap::error::Error::No(_)) => return Ok(0),
            Err(e) => return Err(imap_err(e, "server")),
        };
        drain_unsolicited(&mut imap.session);
        let can_keywords = mbox.permanent_flags.is_empty()
            || mbox.permanent_flags.iter().any(|f| matches!(f, Flag::MayCreate) || matches!(f, Flag::Custom(c) if c.eq_ignore_ascii_case(COMPLETED_KEYWORD)));
        self.push_container(imap, mailbox, mbox.uid_validity.unwrap_or(0), can_keywords).await
    }

    /// Mirror the folder list and remember the mailbox of every local folder.
    pub fn map_folders(&mut self, remote: &[RemoteFolder]) -> SyncResult<Vec<MappedFolder>> {
        let mapped = folders::apply_to_store(self.store, self.account.id, remote)?;
        self.remote_of = mapped.iter().map(|m| (m.folder_id, m.remote.name.clone())).collect();
        self.all_mail = remote.iter().find(|r| r.special == Some(Special::All)).map(|r| r.name.clone());
        self.notifier.bump();
        Ok(mapped)
    }

    fn conversation(&self, parsed: &ParsedMail) -> String {
        let since = Utc::now() - chrono::Duration::days(60);
        conversation_id(
            &parsed.thread,
            |ids| self.store.conversation_for_message_ids(self.account.id, ids).ok().flatten(),
            |subject| self.store.conversation_for_subject(self.account.id, subject, since).ok().flatten(),
        )
    }

    /// Sync one folder: pull, then push.
    pub async fn sync_folder<T: Transport>(&mut self, imap: &mut Imap<T>, folder_id: FolderId, mailbox: &str) -> SyncResult<FolderReport> {
        let mut report = FolderReport::default();
        let account_id = self.account.id;
        let condstore = imap.has("CONDSTORE");
        let selected = if condstore { imap.session.select_condstore(mailbox).await } else { imap.session.select(mailbox).await };
        drain_unsolicited(&mut imap.session);
        let mbox = match selected {
            Ok(m) => m,
            // Deleted between LIST and SELECT, or not selectable after all.
            Err(async_imap::error::Error::No(_)) => return Ok(report),
            Err(e) => return Err(imap_err(e, "server")),
        };
        let can_keywords = mbox.permanent_flags.is_empty()
            || mbox.permanent_flags.iter().any(|f| matches!(f, Flag::MayCreate) || matches!(f, Flag::Custom(c) if c.eq_ignore_ascii_case(COMPLETED_KEYWORD)));
        let uidvalidity = mbox.uid_validity.unwrap_or(0);
        let mut cursor: FolderCursor = self.store.sync_folder_get(folder_id)?.unwrap_or_default();

        // UIDVALIDITY changed: everything we know about this folder is void.
        if cursor.uidvalidity.is_some() && cursor.uidvalidity != Some(uidvalidity) {
            let stale = self.store.sync_list::<Shadow>(account_id, KIND_MESSAGE, Some(mailbox))?;
            let ids: Vec<MessageId> = stale.iter().map(|(id, _)| *id).collect();
            self.store.delete_messages_permanently(&ids)?;
            for id in &ids {
                self.locals.remove(id);
            }
            self.store.sync_remove_container(account_id, KIND_MESSAGE, mailbox)?;
            cursor = FolderCursor { delimiter: cursor.delimiter.clone(), ..Default::default() };
            report.deleted += ids.len();
        }

        // UIDs inside the sync window.
        let mut window: Vec<u32> = match cursor.synced_min_uid {
            None => {
                let since = Utc::now().date_naive() - chrono::Duration::days(self.cfg.days_to_sync.max(1) as i64);
                uid_search(imap, &format!("SINCE {}", search_date(since))).await?.into_iter().collect()
            }
            Some(min) => uid_search(imap, &format!("UID {min}:*")).await?.into_iter().filter(|u| *u >= min).collect(),
        };
        window.sort_unstable();
        let capped = window.len() > MAX_WINDOW;
        if capped {
            window.drain(..window.len() - MAX_WINDOW);
        }
        // The window floor stays put between syncs (so expunged UIDs below the
        // lowest remaining one are still noticed), unless the cap moves it up.
        let min_uid = match (cursor.synced_min_uid, capped) {
            (Some(m), false) => m,
            _ => window.first().copied().or(mbox.uid_next).unwrap_or(1),
        };
        let in_window: HashSet<u32> = window.iter().copied().collect();

        let shadows = self.store.sync_list::<Shadow>(account_id, KIND_MESSAGE, Some(mailbox))?;
        let mut known: HashMap<u32, (MessageId, Shadow)> = HashMap::new();
        let mut adoptable: HashMap<String, (MessageId, Shadow)> = HashMap::new();
        for (lid, sh) in shadows {
            match sh.uid {
                Some(u) => {
                    known.insert(u, (lid, sh));
                }
                None => {
                    if let Some(m) = self.locals.get(&lid)
                        && !m.internet_message_id.is_empty()
                    {
                        adoptable.insert(m.internet_message_id.clone(), (lid, sh));
                    }
                }
            }
        }

        // 1. New mail, newest first so the top of the list fills in quickly.
        let mut new_uids: Vec<u32> = window.iter().copied().filter(|u| !known.contains_key(u)).collect();
        new_uids.reverse();
        let gmail = imap.has("X-GM-EXT-1");
        for batch in new_uids.chunks(FETCH_BATCH) {
            let sizes = fetch(imap, &uid_set(batch), "(UID RFC822.SIZE)").await?;
            let mut normal_uids = Vec::new();
            let mut big_uids = Vec::new();
            for f in &sizes {
                if f.size.unwrap_or(0) > MAX_BODY {
                    big_uids.push(f.uid);
                } else {
                    normal_uids.push(f.uid);
                }
            }
            let extra = if gmail { " X-GM-THRID" } else { "" };
            let mut fetched = fetch(imap, &uid_set(&normal_uids), &format!("(UID FLAGS INTERNALDATE RFC822.SIZE BODY.PEEK[]{extra})")).await?;
            fetched.extend(fetch(imap, &uid_set(&big_uids), &format!("(UID FLAGS INTERNALDATE RFC822.SIZE BODY.PEEK[HEADER]{extra})")).await?);
            for f in fetched {
                if self.ingest(folder_id, mailbox, uidvalidity, f, &mut adoptable)? {
                    report.new += 1;
                }
            }
            self.notifier.bump();
        }

        // 2. Flag changes for messages we already have.
        let tracked: Vec<u32> = known.keys().copied().filter(|u| in_window.contains(u)).collect();
        if !tracked.is_empty() {
            let unchanged = condstore && cursor.highest_modseq.is_some() && cursor.highest_modseq == mbox.highest_modseq;
            if !unchanged {
                let query = match (condstore, cursor.highest_modseq) {
                    (true, Some(modseq)) => format!("(UID FLAGS) (CHANGEDSINCE {modseq})"),
                    _ => "(UID FLAGS)".to_string(),
                };
                for f in fetch(imap, &format!("{min_uid}:*"), &query).await? {
                    let Some((lid, sh)) = known.get_mut(&f.uid) else { continue };
                    let (is_read, mut flag, _, deleted) = read_flags(&f.flags);
                    if deleted {
                        continue; // treated like an expunge below
                    }
                    let mut last = MessageState::from_value(&sh.state);
                    if !can_keywords && last.flag == FlagStatus::Complete && flag == FlagStatus::NotFlagged {
                        flag = FlagStatus::Complete;
                    }
                    let server = MessageState { is_read, flag, ..last.clone() };
                    if server == last && f.modseq.map(|m| m.to_string()) == sh.etag {
                        continue;
                    }
                    if let Some(local) = self.locals.get_mut(lid)
                        && merge_server_message(local, &mut last, &server)
                    {
                        self.store.update_message(local)?;
                        report.updated += 1;
                    }
                    sh.state = last.to_value();
                    sh.etag = f.modseq.map(|m| m.to_string()).or(sh.etag.take());
                    self.store.sync_put(account_id, KIND_MESSAGE, *lid, &sh.remote_id, mailbox, sh)?;
                }
            }
            // Messages flagged \Deleted by another client count as gone.
            let deleted_on_server = uid_search(imap, &format!("UID {min_uid}:* DELETED")).await.unwrap_or_default();
            // 3. Deletions: tracked UIDs no longer in the window.
            for (uid, (lid, _)) in known.iter() {
                if *uid >= min_uid && (!in_window.contains(uid) || deleted_on_server.contains(uid)) {
                    self.store.delete_messages_permanently(&[*lid])?;
                    self.store.sync_remove(account_id, KIND_MESSAGE, *lid)?;
                    self.locals.remove(lid);
                    report.deleted += 1;
                }
            }
            if report.updated + report.deleted > 0 {
                self.notifier.bump();
            }
        }

        // 4. Push local changes made in this folder.
        report.pushed += self.push_container(imap, mailbox, uidvalidity, can_keywords).await?;

        cursor.uidvalidity = Some(uidvalidity);
        cursor.uidnext = mbox.uid_next;
        cursor.highest_modseq = mbox.highest_modseq;
        cursor.synced_min_uid = Some(min_uid);
        self.store.sync_folder_put(folder_id, account_id, mailbox, &cursor)?;
        Ok(report)
    }

    /// Store one fetched message, or match it with a local copy the app put
    /// there itself (a sent message, a moved message). Returns true if new.
    fn ingest(
        &mut self,
        folder_id: FolderId,
        mailbox: &str,
        uidvalidity: u32,
        f: Fetched,
        adoptable: &mut HashMap<String, (MessageId, Shadow)>,
    ) -> SyncResult<bool> {
        let (is_read, flag, is_draft, deleted) = read_flags(&f.flags);
        if deleted {
            return Ok(false);
        }
        let raw = f.body.as_deref().or(f.header.as_deref()).unwrap_or_default();
        let mut parsed = parse_rfc822(raw).unwrap_or_default();
        if f.body.is_none() && f.header.is_some() {
            parsed.body.text = "This message is too large to download. Open it in your provider's web mail.".into();
        }
        parsed.thread.gmail_thread_id = f.gmail_thread;
        let key = message_key(uidvalidity, f.uid, mailbox);
        let account_id = self.account.id;
        let mid = parsed.message.internet_message_id.clone();

        // A copy we already have locally (moved or appended by us).
        let existing = adoptable.remove(&mid).map(|(lid, _)| lid).or_else(|| {
            self.store
                .folder_message_by_internet_id(folder_id, &mid)
                .ok()
                .flatten()
                .filter(|m| self.store.sync_get::<Shadow>(account_id, KIND_MESSAGE, m.id).ok().flatten().is_none())
                .map(|m| m.id)
        });
        if let Some(lid) = existing.filter(|lid| self.locals.contains_key(lid)) {
            let local = self.locals.get_mut(&lid).expect("checked");
            if local.folder_id == folder_id {
                local.is_read = is_read;
                local.flag = flag;
                self.store.update_message(local)?;
            }
            let shadow = Shadow {
                remote_id: key.clone(),
                container: mailbox.to_string(),
                uid: Some(f.uid),
                etag: f.modseq.map(|m| m.to_string()),
                state: MessageState { folder_id, ..MessageState::of(local) }.to_value(),
                extra: serde_json::Value::Null,
            };
            self.store.sync_put(account_id, KIND_MESSAGE, lid, &key, mailbox, &shadow)?;
            return Ok(false);
        }

        let mut m = parsed.message.clone();
        m.account_id = account_id;
        m.folder_id = folder_id;
        m.remote_id = None;
        if let Some(d) = f.internal_date {
            m.received_at = d;
        }
        m.is_read = is_read;
        m.flag = flag;
        m.is_draft = is_draft;
        m.conversation_id = self.conversation(&parsed);
        let id = self.store.insert_synced_message(&mut m, &parsed.body, &parsed.attachments)?;
        let shadow = Shadow {
            remote_id: key.clone(),
            container: mailbox.to_string(),
            uid: Some(f.uid),
            etag: f.modseq.map(|m| m.to_string()),
            state: MessageState::of(&m).to_value(),
            extra: serde_json::Value::Null,
        };
        self.store.sync_put(account_id, KIND_MESSAGE, id, &key, mailbox, &shadow)?;
        self.locals.insert(id, m);
        Ok(true)
    }

    /// Mailbox a local folder maps to, creating a missing role mailbox
    /// (Archive, Junk…) on first use. `None` for local-only folders.
    async fn target_mailbox<T: Transport>(&mut self, imap: &mut Imap<T>, folder_id: FolderId) -> SyncResult<Option<String>> {
        if let Some(name) = self.remote_of.get(&folder_id) {
            return Ok(Some(name.clone()));
        }
        let Some(mut folder) = self.store.folder(folder_id)? else { return Ok(None) };
        if folder.account_id != self.account.id {
            return Ok(None);
        }
        if folder.role == FolderRole::Archive
            && let Some(all) = &self.all_mail
        {
            return Ok(Some(all.clone()));
        }
        if !matches!(folder.role, FolderRole::Archive | FolderRole::Junk | FolderRole::Deleted | FolderRole::Sent | FolderRole::Drafts) {
            return Ok(None);
        }
        let name = folders::create_role_folder(imap, folder.role).await?;
        folder.remote_id = Some(name.clone());
        self.store.save_synced_folder(&mut folder)?;
        self.store.sync_folder_put(folder_id, self.account.id, &name, &FolderCursor::default())?;
        self.remote_of.insert(folder_id, name.clone());
        Ok(Some(name))
    }

    /// Push read / flag / move / delete changes for messages whose server
    /// copy lives in `mailbox` (which must be selected).
    pub async fn push_container<T: Transport>(&mut self, imap: &mut Imap<T>, mailbox: &str, uidvalidity: u32, can_keywords: bool) -> SyncResult<usize> {
        let account_id = self.account.id;
        let shadows = self.store.sync_list::<Shadow>(account_id, KIND_MESSAGE, Some(mailbox))?;
        let mut stores: HashMap<&'static str, Vec<u32>> = HashMap::new();
        let mut moves: HashMap<FolderId, Vec<(MessageId, u32)>> = HashMap::new();
        let mut deletes: Vec<(MessageId, u32)> = Vec::new();
        let mut settled: Vec<(MessageId, Shadow)> = Vec::new();
        for (lid, sh) in shadows {
            let Some(uid) = sh.uid else { continue };
            let last = MessageState::from_value(&sh.state);
            let local = self.locals.get(&lid);
            let ops = message_ops(local, &last);
            if ops.is_empty() {
                continue;
            }
            for op in &ops {
                match op {
                    MessageOp::Delete => deletes.push((lid, uid)),
                    MessageOp::Move(target) => moves.entry(*target).or_default().push((lid, uid)),
                    MessageOp::Read(true) => stores.entry("+FLAGS.SILENT (\\Seen)").or_default().push(uid),
                    MessageOp::Read(false) => stores.entry("-FLAGS.SILENT (\\Seen)").or_default().push(uid),
                    MessageOp::Flag(FlagStatus::Flagged, _) => {
                        stores.entry("+FLAGS.SILENT (\\Flagged)").or_default().push(uid);
                        if can_keywords && last.flag == FlagStatus::Complete {
                            stores.entry("-FLAGS.SILENT ($Completed)").or_default().push(uid);
                        }
                    }
                    MessageOp::Flag(FlagStatus::Complete, _) => {
                        stores.entry("-FLAGS.SILENT (\\Flagged)").or_default().push(uid);
                        if can_keywords {
                            stores.entry("+FLAGS.SILENT ($Completed)").or_default().push(uid);
                        }
                    }
                    MessageOp::Flag(FlagStatus::NotFlagged, _) => {
                        stores.entry("-FLAGS.SILENT (\\Flagged)").or_default().push(uid);
                        if can_keywords && last.flag == FlagStatus::Complete {
                            stores.entry("-FLAGS.SILENT ($Completed)").or_default().push(uid);
                        }
                    }
                    // Not representable on IMAP: settle locally.
                    MessageOp::Inference(_) | MessageOp::Categories(_) => {}
                }
            }
            if let Some(m) = local {
                let mut s = sh.clone();
                // A move settles separately (it changes container and UID).
                s.state = MessageState { folder_id: last.folder_id, ..MessageState::of(m) }.to_value();
                settled.push((lid, s));
            }
        }
        let mut pushed = 0;
        for (op, uids) in &stores {
            raw_command(&mut imap.session, &format!("UID STORE {} {op}", uid_set(uids))).await?;
            pushed += uids.len();
        }
        drain_unsolicited(&mut imap.session);
        for (lid, s) in &settled {
            self.store.sync_put(account_id, KIND_MESSAGE, *lid, &s.remote_id, mailbox, s)?;
        }

        for (target_folder, items) in moves {
            let Some(target) = self.target_mailbox(imap, target_folder).await? else { continue };
            if target == mailbox {
                continue;
            }
            let uids: Vec<u32> = items.iter().map(|(_, u)| *u).collect();
            let set = uid_set(&uids);
            let reply = if imap.has("MOVE") {
                raw_command(&mut imap.session, &format!("UID MOVE {set} {}", quote(&target))).await?
            } else {
                let r = raw_command(&mut imap.session, &format!("UID COPY {set} {}", quote(&target))).await?;
                raw_command(&mut imap.session, &format!("UID STORE {set} +FLAGS.SILENT (\\Deleted)")).await?;
                if imap.has("UIDPLUS") {
                    raw_command(&mut imap.session, &format!("UID EXPUNGE {set}")).await?;
                }
                r
            };
            drain_unsolicited(&mut imap.session);
            let mapping: HashMap<u32, u32> = match &reply.copy_uid {
                Some((_, src, dst)) if src.len() == dst.len() => src.iter().copied().zip(dst.iter().copied()).collect(),
                _ => HashMap::new(),
            };
            let dest_validity = reply.copy_uid.as_ref().map(|(v, _, _)| *v);
            for (lid, uid) in items {
                let Some(state) = self.locals.get(&lid).map(MessageState::of) else { continue };
                let new_uid = mapping.get(&uid).copied();
                let key = match (dest_validity, new_uid) {
                    (Some(v), Some(u)) => message_key(v, u, &target),
                    _ => format!("moved:{lid}:{target}"),
                };
                // Gmail: the message already existed in "All Mail"; drop our duplicate.
                if let Some((other, _)) = self.store.sync_find::<Shadow>(account_id, KIND_MESSAGE, &key)?
                    && other != lid
                {
                    self.store.delete_messages_permanently(&[other])?;
                    self.locals.remove(&other);
                }
                let shadow = Shadow {
                    remote_id: key.clone(),
                    container: target.clone(),
                    uid: new_uid,
                    etag: None,
                    state: state.to_value(),
                    extra: serde_json::Value::Null,
                };
                self.store.sync_put(account_id, KIND_MESSAGE, lid, &key, &target, &shadow)?;
                pushed += 1;
            }
        }

        if !deletes.is_empty() {
            let uids: Vec<u32> = deletes.iter().map(|(_, u)| *u).collect();
            let set = uid_set(&uids);
            raw_command(&mut imap.session, &format!("UID STORE {set} +FLAGS.SILENT (\\Deleted)")).await?;
            if imap.has("UIDPLUS") {
                raw_command(&mut imap.session, &format!("UID EXPUNGE {set}")).await?;
            } else {
                // Plain EXPUNGE only when no one else's \Deleted messages would go too.
                let flagged = uid_search(imap, "DELETED").await?;
                if flagged.iter().all(|u| uids.contains(u)) {
                    raw_command(&mut imap.session, "EXPUNGE").await?;
                }
            }
            drain_unsolicited(&mut imap.session);
            for (lid, _) in &deletes {
                self.store.sync_remove(account_id, KIND_MESSAGE, *lid)?;
            }
            pushed += deletes.len();
        }
        let _ = uidvalidity;
        if pushed > 0 {
            self.notifier.bump();
        }
        Ok(pushed)
    }

    /// Delete the server copies of drafts that were sent from here (they
    /// would otherwise be moved into Sent next to the real Sent copy).
    pub async fn delete_sent_drafts<T: Transport>(&mut self, imap: &mut Imap<T>) -> SyncResult<usize> {
        let account_id = self.account.id;
        let Some(drafts) = self.store.folder_by_role(account_id, FolderRole::Drafts)? else { return Ok(0) };
        let Some(mailbox) = self.remote_of.get(&drafts.id).cloned() else { return Ok(0) };
        let sent_like: HashSet<FolderId> = [FolderRole::Outbox, FolderRole::Sent]
            .into_iter()
            .filter_map(|r| self.store.folder_by_role(account_id, r).ok().flatten().map(|f| f.id))
            .collect();
        let shadows = self.store.sync_list::<Shadow>(account_id, KIND_MESSAGE, Some(&mailbox))?;
        let doomed = sent_drafts(&self.locals, &shadows, &sent_like);
        let uids: Vec<u32> = doomed.iter().filter_map(|(_, s)| s.uid).collect();
        if !uids.is_empty() {
            match imap.session.select(&mailbox).await {
                Ok(_) => {}
                Err(async_imap::error::Error::No(_)) => return Ok(0),
                Err(e) => return Err(imap_err(e, "server")),
            }
            drain_unsolicited(&mut imap.session);
            let set = uid_set(&uids);
            raw_command(&mut imap.session, &format!("UID STORE {set} +FLAGS.SILENT (\\Deleted)")).await?;
            if imap.has("UIDPLUS") {
                raw_command(&mut imap.session, &format!("UID EXPUNGE {set}")).await?;
            } else {
                let flagged = uid_search(imap, "DELETED").await?;
                if flagged.iter().all(|u| uids.contains(u)) {
                    raw_command(&mut imap.session, "EXPUNGE").await?;
                }
            }
            drain_unsolicited(&mut imap.session);
        }
        for (id, _) in &doomed {
            self.store.sync_remove(account_id, KIND_MESSAGE, *id)?;
        }
        Ok(doomed.len())
    }

    /// Append a sent message to Sent (policy `client_append` / `detect`).
    pub async fn append_sent<T: Transport>(&mut self, imap: &mut Imap<T>, sent_folder: FolderId, bytes: &[u8]) -> SyncResult<()> {
        let Some(mailbox) = self.target_mailbox(imap, sent_folder).await? else { return Ok(()) };
        tokio::time::timeout(FETCH_TIMEOUT, imap.session.append(&mailbox, Some("(\\Seen)"), None, bytes))
            .await
            .map_err(|_| SyncError::Offline(crate::error::copy::OFFLINE.into()))?
            .map_err(|e| imap_err(e, "server"))?;
        drain_unsolicited(&mut imap.session);
        Ok(())
    }

    /// Whether Sent already holds a message with this Message-ID.
    pub async fn sent_has<T: Transport>(&mut self, imap: &mut Imap<T>, sent_folder: FolderId, message_id: &str) -> SyncResult<bool> {
        let Some(mailbox) = self.remote_of.get(&sent_folder).cloned() else { return Ok(false) };
        imap.session.examine(&mailbox).await.map_err(|e| imap_err(e, "server"))?;
        let found = uid_search(imap, &format!("HEADER Message-ID {}", quote(&format!("<{message_id}>")))).await?;
        Ok(!found.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_and_flags() {
        assert_eq!(parse_internal_date(" 7-Jul-1996 02:44:25 -0700").unwrap().to_rfc3339(), "1996-07-07T09:44:25+00:00");
        assert_eq!(search_date(chrono::NaiveDate::from_ymd_opt(2026, 10, 9).unwrap()), "9-Oct-2026");
        let f = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(read_flags(&f(&["\\Seen", "\\Flagged"])), (true, FlagStatus::Flagged, false, false));
        assert_eq!(read_flags(&f(&["$Completed", "\\Draft"])), (false, FlagStatus::Complete, true, false));
        assert_eq!(read_flags(&f(&["\\Deleted"])), (false, FlagStatus::NotFlagged, false, true));
        assert_eq!(message_key(7, 42, "INBOX"), "7:42:INBOX");
    }
}
