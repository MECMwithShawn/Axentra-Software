//! Merge a backup into the current store.
//!
//! Accounts are matched by (kind, email); unmatched ones are added without
//! credentials (synced accounts get a fresh keyring id and show "Sign-in
//! needed"). Folders, calendars and lists are matched by role / remote id
//! / name and otherwise created, so every id is remapped. In an account
//! this restore created, the default calendar, task list and well-known
//! folders that came with it take the backed-up names, colours and
//! favourites. Items that are already there are skipped by a stable key:
//!
//! | item     | key                                          |
//! |----------|----------------------------------------------|
//! | message  | account + Message-ID (subject + time if none) |
//! | draft    | account + subject + created time              |
//! | event    | calendar + title + start                      |
//! | contact  | account + primary email, else display name    |
//! | list     | account + name                                |
//! | task     | list + title + created time                   |
//!
//! Keys are compared as multisets: two identical copies in the backup are
//! both restored into an empty store, and nothing is added when the same
//! backup is restored again. Restored drafts are never scheduled or queued
//! to send: a backup may be older than the send, or restored on a second
//! device. Keyring entries are never touched.

use std::collections::{BTreeMap, HashMap};
use std::hash::Hash;

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};

use super::collect::{ACCOUNT_KV, KV_STRUCTURAL, folder_ref, kv_allowed, looks_secret, parents_first, strip_secrets};
use super::format::FORMAT_VERSION;
use super::payload::*;
use super::{BackupError, BackupResult};
use crate::model::*;
use crate::{Result, Store};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RestoreOptions {
    /// Also restore settings and preferences (replacing the current ones,
    /// except where backups are saved).
    pub settings: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RestoreSummary {
    pub added: Counts,
    /// Already present, so left alone.
    pub skipped: Counts,
    /// Accounts that were added ("name <email>").
    pub accounts_added: Vec<String>,
    /// Added accounts that need the user to sign in before they sync.
    pub needs_sign_in: Vec<String>,
    pub settings_restored: bool,
    /// Restored drafts that were scheduled (or waiting to send) when the
    /// backup was made; they came back as ordinary drafts.
    pub unscheduled: usize,
}

/// One account in the restore preview.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviewAccount {
    pub display_name: String,
    pub email: String,
    pub kind: AccountKind,
    /// All of the account's data is in the backup (local accounts).
    pub complete: bool,
    pub counts: Counts,
    /// The account is already here, so its items are merged into it.
    pub exists: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preview {
    pub created_at: DateTime<Utc>,
    pub app_version: String,
    pub counts: Counts,
    pub accounts: Vec<PreviewAccount>,
}

fn account_key(kind: AccountKind, email: &str) -> (AccountKind, String) {
    (kind, email.trim().to_lowercase())
}

/// What a backup holds, and which accounts already exist in `store`.
pub fn preview(payload: &Payload, store: Option<&Store>) -> Preview {
    let existing: Vec<(AccountKind, String)> = store
        .and_then(|s| s.accounts().ok())
        .unwrap_or_default()
        .iter()
        .map(|a| account_key(a.kind, &a.email))
        .collect();
    let accounts = payload
        .accounts
        .iter()
        .map(|a| PreviewAccount {
            display_name: a.account.display_name.clone(),
            email: a.account.email.clone(),
            kind: a.account.kind,
            complete: a.complete,
            counts: a.counts(),
            exists: existing.contains(&account_key(a.account.kind, &a.account.email)),
        })
        .collect();
    Preview {
        created_at: payload.manifest.created_at,
        app_version: payload.manifest.app_version.clone(),
        counts: payload.manifest.counts.clone(),
        accounts,
    }
}

/// Multiset de-duplication by a stable key: the n-th occurrence of a key
/// in the backup matches the n-th existing item with that key.
struct Dedupe<K> {
    known: HashMap<K, Vec<i64>>,
    seen: HashMap<K, usize>,
}

impl<K: Eq + Hash + Clone> Dedupe<K> {
    fn new() -> Self {
        Self { known: HashMap::new(), seen: HashMap::new() }
    }

    fn with(items: impl IntoIterator<Item = (K, i64)>) -> Self {
        let mut d = Self::new();
        for (k, id) in items {
            d.known.entry(k).or_default().push(id);
        }
        d
    }

    /// The existing item this occurrence of `key` stands for, if any.
    fn find(&mut self, key: &K) -> Option<i64> {
        let n = self.seen.entry(key.clone()).or_insert(0);
        let i = *n;
        *n += 1;
        self.known.get(key).and_then(|ids| ids.get(i)).copied()
    }

    /// Record an item inserted after [`Dedupe::find`] returned `None`.
    fn inserted(&mut self, key: K, id: i64) {
        self.known.entry(key).or_default().push(id);
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum MsgKey {
    Id(String),
    /// Subject and the time the draft was first saved (its last-save time
    /// for drafts saved before that was recorded).
    Draft(String, i64),
    Untitled(String, i64),
}

/// `at` is the message's `received_at` (for a draft, its last save);
/// `draft_created` the draft's first save, when known.
fn msg_key(is_draft: bool, internet_id: &str, subject: &str, at: i64, draft_created: Option<i64>) -> MsgKey {
    if is_draft {
        MsgKey::Draft(subject.to_string(), draft_created.unwrap_or(at))
    } else if !internet_id.trim().is_empty() {
        MsgKey::Id(internet_id.trim().to_string())
    } else {
        MsgKey::Untitled(subject.to_string(), at)
    }
}

fn contact_key(c: &Contact) -> String {
    match c.primary_email().map(|e| e.trim().to_lowercase()).filter(|e| !e.is_empty()) {
        Some(e) => format!("email:{e}"),
        None => format!("name:{}", c.display_name.trim().to_lowercase()),
    }
}

fn rows<T>(conn: &Connection, sql: &str, p: impl rusqlite::Params, f: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>) -> Result<Vec<T>> {
    let mut stmt = conn.prepare(sql)?;
    let out = stmt.query_map(p, f)?.collect::<rusqlite::Result<Vec<T>>>()?;
    Ok(out)
}

fn kv_put(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO kv (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

/// Merge a JSON list of names into the list stored under `key`.
fn union_merge(conn: &Connection, key: &str, incoming: &[String], sort: bool) -> Result<()> {
    let current: Option<String> = conn.query_row("SELECT value FROM kv WHERE key = ?1", [key], |r| r.get(0)).optional()?;
    let mut list: Vec<String> = current.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default();
    let before = list.len();
    for name in incoming {
        if !name.trim().is_empty() && !list.iter().any(|x| x.eq_ignore_ascii_case(name)) {
            list.push(name.clone());
        }
    }
    if list.len() == before {
        return Ok(());
    }
    if sort {
        list.sort_by_key(|f| f.to_lowercase());
    }
    kv_put(conn, key, &serde_json::to_string(&list)?)
}

/// A new account for a backed-up one: no secrets; synced accounts get a
/// fresh keyring id (so no existing keyring entry is ever reused) and
/// start in the "Sign-in needed" state.
fn new_account(src: &Account, sort_order: i64, now: DateTime<Utc>) -> Account {
    let mut config = strip_secrets(&src.config);
    if src.kind != AccountKind::Local {
        if !config.is_object() {
            config = serde_json::json!({});
        }
        if let Some(obj) = config.as_object_mut() {
            obj.insert("uuid".into(), uuid::Uuid::new_v4().to_string().into());
            obj.insert("state".into(), "needs_reauth".into());
            obj.insert("state_detail".into(), serde_json::Value::Null);
            obj.insert("state_since".into(), serde_json::to_value(now).unwrap_or_default());
            obj.insert("bar_dismissed".into(), false.into());
        }
    }
    Account {
        id: 0,
        kind: src.kind,
        display_name: src.display_name.clone(),
        email: src.email.clone(),
        color: src.color.clone(),
        config,
        sort_order,
        created_at: src.created_at,
        last_sync_at: None,
        sync_error: None,
        nickname: src.nickname.clone(),
    }
}

/// Restore `payload` into `store`. Account creation happens first; all
/// other changes are one transaction.
pub fn restore(store: &Store, payload: &Payload, opts: RestoreOptions, now: DateTime<Utc>) -> BackupResult<RestoreSummary> {
    if payload.manifest.format != FORMAT_NAME {
        return Err(BackupError::NotABackup);
    }
    if payload.manifest.format_version > FORMAT_VERSION {
        return Err(BackupError::NewerVersion);
    }
    let mut summary = RestoreSummary::default();
    let mut known: Vec<((AccountKind, String), AccountId)> =
        store.accounts()?.iter().map(|a| (account_key(a.kind, &a.email), a.id)).collect();
    // (target account, created by this restore)
    let mut targets: Vec<(AccountId, bool)> = Vec::with_capacity(payload.accounts.len());
    let mut created: Vec<AccountId> = Vec::new();
    for data in &payload.accounts {
        let key = account_key(data.account.kind, &data.account.email);
        if let Some((_, id)) = known.iter().find(|(k, _)| *k == key) {
            summary.skipped.accounts += 1;
            targets.push((*id, created.contains(id)));
            continue;
        }
        let mut account = new_account(&data.account, store.next_account_sort_order()?, now);
        let id = store.add_account(&mut account)?;
        let label = if account.display_name.trim().is_empty() {
            account.email.clone()
        } else {
            format!("{} <{}>", account.display_name, account.email)
        };
        if account.kind != AccountKind::Local {
            summary.needs_sign_in.push(account.email.clone());
        }
        summary.accounts_added.push(label);
        summary.added.accounts += 1;
        known.push((key, id));
        created.push(id);
        targets.push((id, true));
    }
    let current = store.settings();
    store.tx(|tx| {
        let mut contact_folders: Vec<String> = Vec::new();
        let mut ids = IdMaps::default();
        for (data, &(target, fresh)) in payload.accounts.iter().zip(&targets) {
            ids.accounts.insert(data.account.id, target);
            restore_account(tx, data, target, fresh, opts, &mut summary, &mut contact_folders, &mut ids)?;
        }
        restore_shared(tx, payload, opts, &current, &ids, &mut summary)?;
        union_merge(tx, "people.folders", &contact_folders, true)?;
        Ok(())
    })?;
    Ok(summary)
}

/// Old (backup) id → id in this store, for preferences that refer to ids.
#[derive(Default)]
struct IdMaps {
    accounts: HashMap<AccountId, AccountId>,
    task_lists: HashMap<TaskListId, TaskListId>,
}

/// The account's folder with `role`, created when missing.
fn role_folder(conn: &Connection, folders: &mut Vec<Folder>, account: AccountId, role: FolderRole) -> Result<FolderId> {
    if let Some(f) = folders.iter().find(|f| f.role == role) {
        return Ok(f.id);
    }
    let mut f = Folder {
        account_id: account,
        name: role.default_name().into(),
        role,
        sort_order: folders.iter().map(|f| f.sort_order).max().unwrap_or(0) + 1,
        is_expanded: true,
        ..Default::default()
    };
    Store::insert_doc_in(conn, &mut f)?;
    folders.push(f.clone());
    Ok(f.id)
}

/// Merge one account's data into account `target`. `fresh`: this restore
/// created the account, so its default containers are the ones
/// `add_account` made and take the backed-up presentation.
#[allow(clippy::too_many_arguments)]
fn restore_account(
    tx: &Connection,
    data: &AccountData,
    target: AccountId,
    fresh: bool,
    opts: RestoreOptions,
    summary: &mut RestoreSummary,
    contact_folders: &mut Vec<String>,
    ids: &mut IdMaps,
) -> Result<()> {
    let complete = data.complete;
    let keep_remote = |r: &Option<String>| if complete { r.clone() } else { None };

    // ---- folders
    let mut folders: Vec<Folder> = Store::query_docs_in(tx, "WHERE account_id = ?1 ORDER BY id", [target])?;
    let mut folder_map: HashMap<FolderId, FolderId> = HashMap::new();
    for f in parents_first(data.folders.clone()) {
        let parent = f.parent_id.and_then(|p| folder_map.get(&p).copied());
        let found = if f.role != FolderRole::Custom {
            folders.iter().find(|e| e.role == f.role)
        } else {
            folders.iter().find(|e| e.role == FolderRole::Custom && e.parent_id == parent && e.name.eq_ignore_ascii_case(&f.name))
        }
        .map(|e| e.id);
        match found {
            Some(id) => {
                if fresh && let Some(e) = folders.iter_mut().find(|e| e.id == id) {
                    *e = Folder { name: f.name.clone(), is_favorite: f.is_favorite, is_expanded: f.is_expanded, sort_order: f.sort_order, ..e.clone() };
                    Store::update_doc_in(tx, e)?;
                }
                folder_map.insert(f.id, id);
                summary.skipped.folders += 1;
            }
            None => {
                let mut nf = Folder {
                    id: 0,
                    account_id: target,
                    parent_id: parent,
                    remote_id: keep_remote(&f.remote_id),
                    unread_count: 0,
                    total_count: 0,
                    ..f.clone()
                };
                Store::insert_doc_in(tx, &mut nf)?;
                folder_map.insert(f.id, nf.id);
                folders.push(nf);
                summary.added.folders += 1;
            }
        }
    }
    for (kind, folder) in &data.special_folders {
        if let Some(mapped) = folder_map.get(folder) {
            tx.execute(
                "INSERT OR IGNORE INTO mail_special_folders (account_id, kind, folder_id) VALUES (?1, ?2, ?3)",
                params![target, kind, mapped],
            )?;
        }
    }

    // ---- calendars and events (before mail: invitations point at events)
    let mut calendars: Vec<Calendar> = Store::query_docs_in(tx, "WHERE account_id = ?1 ORDER BY id", [target])?;
    let mut event_map: HashMap<EventId, EventId> = HashMap::new();
    let mut series_links: Vec<(EventId, EventId)> = Vec::new();
    for cd in &data.calendars {
        let c = &cd.calendar;
        let found = c
            .remote_id
            .as_ref()
            .and_then(|r| calendars.iter().find(|e| e.remote_id.as_ref() == Some(r)))
            .or_else(|| if c.is_default { calendars.iter().find(|e| e.is_default) } else { None })
            .or_else(|| calendars.iter().find(|e| e.name.trim().eq_ignore_ascii_case(c.name.trim())))
            .map(|e| e.id);
        let cal_id = match found {
            Some(id) => {
                if fresh && let Some(e) = calendars.iter_mut().find(|e| e.id == id) {
                    *e = Calendar {
                        name: c.name.clone(),
                        color: c.color.clone(),
                        is_visible: c.is_visible,
                        group: c.group.clone(),
                        sort_order: c.sort_order,
                        ..e.clone()
                    };
                    Store::update_doc_in(tx, e)?;
                }
                summary.skipped.calendars += 1;
                id
            }
            None => {
                let has_default = calendars.iter().any(|e| e.is_default);
                let mut nc = Calendar {
                    id: 0,
                    account_id: target,
                    remote_id: keep_remote(&c.remote_id),
                    is_default: c.is_default && !has_default,
                    ..c.clone()
                };
                Store::insert_doc_in(tx, &mut nc)?;
                calendars.push(nc.clone());
                summary.added.calendars += 1;
                nc.id
            }
        };
        let mut dedupe = Dedupe::with(rows(
            tx,
            "SELECT id, COALESCE(json_extract(json, '$.title'), ''), start_ts FROM events WHERE calendar_id = ?1 ORDER BY id",
            [cal_id],
            |r| Ok(((r.get::<_, String>(1)?, r.get::<_, i64>(2)?), r.get::<_, i64>(0)?)),
        )?);
        for e in &cd.events {
            let key = (e.title.clone(), e.start.timestamp());
            if let Some(id) = dedupe.find(&key) {
                event_map.insert(e.id, id);
                summary.skipped.events += 1;
                continue;
            }
            let mut ne = Event { id: 0, calendar_id: cal_id, series_id: None, remote_id: keep_remote(&e.remote_id), ..e.clone() };
            Store::insert_doc_in(tx, &mut ne)?;
            dedupe.inserted(key, ne.id);
            event_map.insert(e.id, ne.id);
            if let Some(s) = e.series_id {
                series_links.push((ne.id, s));
            }
            for rs in cd.reminder_states.iter().filter(|r| r.event_id == e.id) {
                tx.execute(
                    "INSERT OR IGNORE INTO reminder_state (event_id, occurrence_start, dismissed, snoozed_until) VALUES (?1, ?2, ?3, ?4)",
                    params![ne.id, rs.occurrence_start, rs.dismissed, rs.snoozed_until],
                )?;
            }
            summary.added.events += 1;
        }
    }
    for (event, old_series) in series_links {
        if let Some(&series) = event_map.get(&old_series)
            && let Some(mut e) = Store::get_doc_in::<Event>(tx, event)?
        {
            e.series_id = Some(series);
            Store::update_doc_in(tx, &e)?;
        }
    }

    // ---- mail
    let mut msg_map: HashMap<MessageId, MessageId> = HashMap::new();
    let mut dedupe = Dedupe::with(rows(
        tx,
        "SELECT m.id, m.internet_message_id, COALESCE(json_extract(m.json, '$.subject'), ''), m.received_at, m.is_draft,
                json_extract(d.json, '$.created_at')
         FROM messages m LEFT JOIN drafts d ON d.message_id = m.id WHERE m.account_id = ?1 ORDER BY m.id",
        [target],
        |r| {
            let id: i64 = r.get(0)?;
            let imid: Option<String> = r.get(1)?;
            let created = r.get::<_, Option<String>>(5)?.as_deref().and_then(parse_time);
            Ok((msg_key(r.get(4)?, imid.as_deref().unwrap_or(""), &r.get::<_, String>(2)?, r.get(3)?, created), id))
        },
    )?);
    for md in &data.messages {
        let m = &md.message;
        let created = md.draft.as_ref().and_then(|d| d.created_at).map(|t| t.timestamp());
        let key = msg_key(m.is_draft, &m.internet_message_id, &m.subject, m.received_at.timestamp(), created);
        if let Some(id) = dedupe.find(&key) {
            msg_map.insert(m.id, id);
            if m.is_draft { summary.skipped.drafts += 1 } else { summary.skipped.messages += 1 }
            continue;
        }
        let role_of = |id: FolderId| folders.iter().find(|f| f.id == id).map(|f| f.role);
        let folder = match folder_map.get(&m.folder_id) {
            // A draft waiting out "Undo send" sat in Outbox; it comes back
            // as an ordinary draft.
            Some(f) if complete && !(m.is_draft && role_of(*f) == Some(FolderRole::Outbox)) => *f,
            _ => role_folder(tx, &mut folders, target, if m.is_draft { FolderRole::Drafts } else { FolderRole::Inbox })?,
        };
        // Never restore a pending send: the backup may be older than the
        // send, or be restored on a second device that would send it again.
        let was_pending = m.scheduled_send_at.is_some()
            || md.draft.as_ref().is_some_and(|d| d.scheduled_send_at.is_some() || d.send_pending_until.is_some());
        if was_pending && m.is_draft {
            summary.unscheduled += 1;
        }
        let mut nm = Message {
            id: 0,
            account_id: target,
            folder_id: folder,
            remote_id: keep_remote(&m.remote_id),
            meeting_event_id: m.meeting_event_id.and_then(|e| event_map.get(&e).copied()),
            scheduled_send_at: None,
            ..m.clone()
        };
        Store::insert_doc_in(tx, &mut nm)?;
        if let Some(b) = &md.body {
            tx.execute(
                "INSERT OR REPLACE INTO message_bodies (message_id, html, text) VALUES (?1, ?2, ?3)",
                params![nm.id, b.html, b.text],
            )?;
        }
        let mut attachment_map: HashMap<AttachmentId, AttachmentId> = HashMap::new();
        for a in &md.attachments {
            let mut na = Attachment { id: 0, message_id: nm.id, ..a.attachment.clone() };
            Store::insert_doc_in(tx, &mut na)?;
            if let Some(bytes) = &a.data {
                tx.execute("UPDATE attachments SET data = ?1 WHERE id = ?2", params![bytes, na.id])?;
            }
            attachment_map.insert(a.attachment.id, na.id);
            summary.added.attachments += 1;
        }
        if let Some(d) = &md.draft {
            let mut nd = d.clone();
            nd.id = Some(nm.id);
            nd.account_id = target;
            nd.scheduled_send_at = None;
            nd.send_pending_until = None;
            nd.source_message_id = match d.source_message_id.and_then(|s| msg_map.get(&s).copied()) {
                Some(s) => Some(s),
                None => match &md.draft_source_message_id {
                    Some(imid) => tx
                        .query_row(
                            "SELECT id FROM messages WHERE account_id = ?1 AND internet_message_id = ?2 ORDER BY id LIMIT 1",
                            params![target, imid],
                            |r| r.get(0),
                        )
                        .optional()?,
                    None => None,
                },
            };
            for att in nd.attachments.iter_mut() {
                let old = att.path.strip_prefix("attachment:").and_then(|s| s.parse::<AttachmentId>().ok());
                if let Some(new) = old.and_then(|o| attachment_map.get(&o)) {
                    att.path = format!("attachment:{new}");
                }
            }
            tx.execute(
                "INSERT OR REPLACE INTO drafts (message_id, json) VALUES (?1, ?2)",
                params![nm.id, serde_json::to_string(&nd)?],
            )?;
        }
        if complete && let Some(origin) = md.origin_folder.and_then(|o| folder_map.get(&o)) {
            tx.execute(
                "INSERT OR REPLACE INTO mail_origins (message_id, folder_id) VALUES (?1, ?2)",
                params![nm.id, origin],
            )?;
        }
        dedupe.inserted(key, nm.id);
        msg_map.insert(m.id, nm.id);
        if m.is_draft { summary.added.drafts += 1 } else { summary.added.messages += 1 }
    }

    // ---- sweep rules, matched by folder role / path
    let by_id: BTreeMap<FolderId, Folder> = folders.iter().map(|f| (f.id, f.clone())).collect();
    let lookup: HashMap<FolderId, Folder> = by_id.iter().map(|(k, v)| (*k, v.clone())).collect();
    let find = |r: &FolderRef| -> Option<FolderId> {
        by_id.keys().copied().find(|&id| {
            folder_ref(&lookup, id).is_some_and(|fr| {
                if r.role != FolderRole::Custom {
                    fr.role == r.role
                } else {
                    fr.role == FolderRole::Custom
                        && fr.path.len() == r.path.len()
                        && fr.path.iter().zip(&r.path).all(|(a, b)| a.eq_ignore_ascii_case(b))
                }
            })
        })
    };
    for rule in &data.sweep_rules {
        let (Some(folder), Some(target_folder)) = (find(&rule.folder), find(&rule.target)) else { continue };
        let exists = tx
            .query_row(
                "SELECT 1 FROM mail_sweep_rules WHERE folder_id = ?1 AND sender = lower(?2)",
                params![folder, rule.sender],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !exists {
            tx.execute(
                "INSERT INTO mail_sweep_rules (account_id, folder_id, sender, mode, target_folder_id, created_at, last_run)
                 VALUES (?1, ?2, lower(?3), ?4, ?5, ?6, ?7)",
                params![target, folder, rule.sender, rule.mode, target_folder, rule.created_at, rule.last_run],
            )?;
        }
    }

    // ---- people
    let existing: Vec<Contact> = Store::query_docs_in(tx, "WHERE account_id = ?1 ORDER BY id", [target])?;
    let mut dedupe = Dedupe::with(existing.iter().map(|c| (contact_key(c), c.id)));
    for c in &data.contacts {
        let key = contact_key(c);
        if dedupe.find(&key).is_some() {
            summary.skipped.contacts += 1;
            continue;
        }
        let mut nc = Contact { id: 0, account_id: target, remote_id: keep_remote(&c.remote_id), ..c.clone() };
        Store::insert_doc_in(tx, &mut nc)?;
        dedupe.inserted(key, nc.id);
        if !c.folder.trim().is_empty() {
            contact_folders.push(c.folder.clone());
        }
        summary.added.contacts += 1;
    }
    let existing: Vec<ContactList> = Store::query_docs_in(tx, "WHERE account_id = ?1 ORDER BY id", [target])?;
    let mut dedupe = Dedupe::with(existing.iter().map(|l| (l.name.trim().to_lowercase(), l.id)));
    for l in &data.contact_lists {
        let key = l.name.trim().to_lowercase();
        if dedupe.find(&key).is_some() {
            summary.skipped.contact_lists += 1;
            continue;
        }
        let mut nl = ContactList { id: 0, account_id: target, ..l.clone() };
        Store::insert_doc_in(tx, &mut nl)?;
        dedupe.inserted(key, nl.id);
        summary.added.contact_lists += 1;
    }

    // ---- To Do
    let mut lists: Vec<TaskList> = Store::query_docs_in(tx, "WHERE account_id = ?1 ORDER BY id", [target])?;
    for ld in &data.task_lists {
        let l = &ld.list;
        let found = l
            .remote_id
            .as_ref()
            .and_then(|r| lists.iter().find(|e| e.remote_id.as_ref() == Some(r)))
            .or_else(|| if l.is_default { lists.iter().find(|e| e.is_default) } else { None })
            .or_else(|| lists.iter().find(|e| e.name.trim().eq_ignore_ascii_case(l.name.trim())))
            .map(|e| e.id);
        let list_id = match found {
            Some(id) => {
                if fresh && let Some(e) = lists.iter_mut().find(|e| e.id == id) {
                    *e = TaskList { name: l.name.clone(), emoji: l.emoji.clone(), group: l.group.clone(), sort_order: l.sort_order, ..e.clone() };
                    Store::update_doc_in(tx, e)?;
                }
                summary.skipped.task_lists += 1;
                id
            }
            None => {
                let has_default = lists.iter().any(|e| e.is_default);
                let mut nl = TaskList {
                    id: 0,
                    account_id: target,
                    remote_id: keep_remote(&l.remote_id),
                    is_default: l.is_default && !has_default,
                    ..l.clone()
                };
                Store::insert_doc_in(tx, &mut nl)?;
                lists.push(nl.clone());
                summary.added.task_lists += 1;
                nl.id
            }
        };
        ids.task_lists.insert(l.id, list_id);
        let existing: Vec<Task> = Store::query_docs_in(tx, "WHERE list_id = ?1 ORDER BY id", [list_id])?;
        let mut dedupe = Dedupe::with(existing.iter().map(|t| ((t.title.clone(), t.created_at.timestamp()), t.id)));
        for t in &ld.tasks {
            let key = (t.title.clone(), t.created_at.timestamp());
            if dedupe.find(&key).is_some() {
                summary.skipped.tasks += 1;
                continue;
            }
            let mut nt = Task {
                id: 0,
                list_id,
                remote_id: keep_remote(&t.remote_id),
                linked_message_id: t.linked_message_id.and_then(|m| msg_map.get(&m).copied()),
                ..t.clone()
            };
            Store::insert_doc_in(tx, &mut nt)?;
            dedupe.inserted(key, nt.id);
            summary.added.tasks += 1;
        }
    }

    // ---- account-scoped key/values
    for e in &data.kv {
        let Some((_, structural)) = ACCOUNT_KV.iter().find(|(p, _)| *p == e.prefix) else { continue };
        let key = format!("{}{}{}", e.prefix, target, e.rest);
        if looks_secret(&key) {
            continue;
        }
        if *structural {
            let names: Vec<String> = serde_json::from_str(&e.value).unwrap_or_default();
            union_merge(tx, &key, &names, false)?;
        } else if opts.settings {
            kv_put(tx, &key, &e.value)?;
        }
    }
    Ok(())
}

/// Settings, categories, address history, sender lists and app-wide
/// key/values.
fn restore_shared(tx: &Connection, payload: &Payload, opts: RestoreOptions, current: &Settings, ids: &IdMaps, summary: &mut RestoreSummary) -> Result<()> {
    for c in &payload.categories {
        let n = tx.execute(
            "INSERT OR IGNORE INTO categories (name, json, sort_order) VALUES (?1, ?2, (SELECT COUNT(*) FROM categories))",
            params![c.name, serde_json::to_string(c)?],
        )?;
        if n > 0 { summary.added.categories += 1 } else { summary.skipped.categories += 1 }
    }
    for a in &payload.recent_addresses {
        tx.execute(
            "INSERT OR IGNORE INTO recent_addresses (address, name, last_used, use_count) VALUES (lower(?1), ?2, ?3, ?4)",
            params![a.address, a.name, a.last_used, a.use_count],
        )?;
    }
    for b in &payload.blocked_senders {
        tx.execute(
            "INSERT OR IGNORE INTO mail_blocked_senders (address, name, added_at) VALUES (?1, ?2, ?3)",
            params![b.address, b.name, b.added_at],
        )?;
    }
    for s in &payload.safe_senders {
        tx.execute("INSERT OR IGNORE INTO mail_safe_senders (address, added_at) VALUES (?1, ?2)", params![s.address, s.added_at])?;
    }
    for o in &payload.inference_overrides {
        tx.execute(
            "INSERT OR IGNORE INTO mail_inference_overrides (address, inference) VALUES (?1, ?2)",
            params![o.address, o.inference],
        )?;
    }
    for e in &payload.kv {
        if !kv_allowed(&e.key) {
            continue;
        }
        if KV_STRUCTURAL.contains(&e.key.as_str()) {
            let names: Vec<String> = serde_json::from_str(&e.value).unwrap_or_default();
            union_merge(tx, &e.key, &names, e.key == "people.folders")?;
        } else if opts.settings {
            let now: Option<String> = tx.query_row("SELECT value FROM kv WHERE key = ?1", [&e.key], |r| r.get(0)).optional()?;
            match remap_ids(&e.key, &e.value, now.as_deref(), ids) {
                Some(v) => kv_put(tx, &e.key, &v)?,
                None => kv_put(tx, &e.key, &e.value)?,
            }
        }
    }
    if opts.settings {
        let mut s = payload.settings.clone();
        // Where and how this device backs up stays as it is.
        s.backup_auto = current.backup_auto;
        s.backup_dir = current.backup_dir.clone();
        s.backup_keep = current.backup_keep;
        tx.execute(
            "INSERT INTO settings (id, json) VALUES (1, ?1) ON CONFLICT(id) DO UPDATE SET json = excluded.json",
            [serde_json::to_string(&s)?],
        )?;
        summary.settings_restored = true;
    }
    Ok(())
}

fn parse_time(s: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(s).ok().map(|t| t.timestamp())
}

/// Preferences that refer to task lists or accounts by id, rewritten for
/// this store: `todo.prefs` (per-list `views."list:<id>"`, `last_view`) and
/// `mail.folder_pane` (`collapsed_accounts`). References the backup can't
/// map are dropped, and the current value's entries for lists and accounts
/// the backup doesn't cover are kept. `None` for other keys or values that
/// don't parse (written as they are).
fn remap_ids(key: &str, value: &str, current: Option<&str>, ids: &IdMaps) -> Option<String> {
    use serde_json::Value;
    let mut v: Value = serde_json::from_str(value).ok()?;
    let cur: Value = current.and_then(|c| serde_json::from_str(c).ok()).unwrap_or(Value::Null);
    let obj = v.as_object_mut()?;
    match key {
        "todo.prefs" => {
            let list_key = |k: &str| k.strip_prefix("list:").and_then(|id| id.parse::<TaskListId>().ok());
            if let Some(views) = obj.get_mut("views").and_then(Value::as_object_mut) {
                let old = std::mem::take(views);
                for (k, val) in old {
                    match list_key(&k) {
                        Some(id) => {
                            if let Some(new) = ids.task_lists.get(&id) {
                                views.insert(format!("list:{new}"), val);
                            }
                        }
                        None => {
                            views.insert(k, val);
                        }
                    }
                }
                if let Some(cur_views) = cur.get("views").and_then(Value::as_object) {
                    for (k, val) in cur_views {
                        if list_key(k).is_some() && !views.contains_key(k) {
                            views.insert(k.clone(), val.clone());
                        }
                    }
                }
            }
            if let Some(last) = obj.get_mut("last_view")
                && let Some(id) = last.get("List").and_then(Value::as_i64)
            {
                *last = match ids.task_lists.get(&id) {
                    Some(new) => serde_json::json!({ "List": new }),
                    None => cur.get("last_view").cloned().unwrap_or(Value::Null),
                };
            }
        }
        "mail.folder_pane" => {
            if let Some(list) = obj.get_mut("collapsed_accounts").and_then(Value::as_array_mut) {
                let mut out: Vec<Value> =
                    list.iter().filter_map(Value::as_i64).filter_map(|id| ids.accounts.get(&id)).map(|n| Value::from(*n)).collect();
                let covered: Vec<AccountId> = ids.accounts.values().copied().collect();
                for id in cur.get("collapsed_accounts").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_i64) {
                    if !covered.contains(&id) && !out.contains(&Value::from(id)) {
                        out.push(Value::from(id));
                    }
                }
                *list = out;
            }
        }
        _ => return None,
    }
    serde_json::to_string(&v).ok()
}
