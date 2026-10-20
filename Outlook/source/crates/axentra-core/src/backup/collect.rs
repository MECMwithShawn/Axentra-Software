//! Build a [`Payload`] from the store.
//!
//! Included: settings, categories, recent addresses, junk/sweep/Focused
//! preferences, the allowlisted key/value entries, every account's setup,
//! all data of local accounts, and for synced accounts only what exists on
//! no server: calendars, events, contacts, task lists and tasks that have
//! neither a `remote_id` nor a sync record (the sync engine records items
//! it pushed only in `sync_items`), contact lists (never synced) and
//! unsent drafts. Drafts are included even when the message came from the
//! server: their edits are never uploaded, so the drafts-table copy exists
//! only here, and restoring one never creates anything on a server.
//! Never included: passwords and tokens (they live in the OS keyring), sync
//! state (cursors, `sync_items`, `sync_folders`) and any key whose name looks
//! secret.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};

use super::payload::*;
use crate::model::*;
use crate::{Result, Store};

/// App-wide key/value entries that are user configuration. A key is
/// included when it equals an entry or starts with `entry` + `.`.
/// Anything else (sync cursors, notification state, search history, UI
/// state that holds ids) stays out.
pub const KV_ALLOWLIST: &[&str] = &[
    "todo.groups",
    "todo.prefs",
    "people.folders",
    "people.suggested_dismissed",
    "people.ui.sort",
    "people.ui.last_first",
    "people.ui.nav_open",
    "people.ui.suggested",
    "people.ui.suggested_tip_dismissed",
    "people.ui.nav_w",
    "people.ui.list_w",
    "mail.ignore_skip_confirm",
    "mail.folder_pane",
    "calendar.day_count",
    "calendar.time_scale",
    "shell.my_day_tab",
    "shell.my_day_view",
    // Remembered dialog and pop-out window sizes (axentra-ui kit::surface).
    "shell.surfaces",
];

/// Allowlisted keys that hold structure other data refers to by name
/// (contact folders, task-list groups). Restore merges them in always;
/// the other entries are preferences, restored only with the settings.
pub const KV_STRUCTURAL: &[&str] = &["todo.groups", "people.folders"];

/// Account-scoped keys `<prefix><account id><rest>`: (prefix, structural).
pub const ACCOUNT_KV: &[(&str, bool)] = &[("calendar.groups.", true), ("shell.ribbon.", false)];

/// Words that mark a key as secret. Matching keys are dropped from account
/// configuration and from the key/value entries, whatever the allowlist says.
const SECRET_WORDS: &[&str] = &["token", "secret", "password", "refresh", "access", "cookie"];

pub fn looks_secret(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    SECRET_WORDS.iter().any(|w| n.contains(w))
}

/// Remove every object key (at any depth) that [`looks_secret`].
pub fn strip_secrets(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            map.iter().filter(|(k, _)| !looks_secret(k)).map(|(k, v)| (k.clone(), strip_secrets(v))).collect::<serde_json::Map<_, _>>().into()
        }
        serde_json::Value::Array(items) => items.iter().map(strip_secrets).collect::<Vec<_>>().into(),
        other => other.clone(),
    }
}

pub fn kv_allowed(key: &str) -> bool {
    !looks_secret(key)
        && KV_ALLOWLIST.iter().any(|a| key == *a || key.strip_prefix(a).is_some_and(|rest| rest.starts_with('.')))
}

/// Split an account-scoped key into (prefix, account id, rest).
pub fn split_account_key(key: &str) -> Option<(&'static str, AccountId, &str)> {
    if looks_secret(key) {
        return None;
    }
    ACCOUNT_KV.iter().find_map(|(prefix, _)| {
        let tail = key.strip_prefix(prefix)?;
        let end = tail.find('.').unwrap_or(tail.len());
        let id: AccountId = tail[..end].parse().ok()?;
        Some((*prefix, id, &tail[end..]))
    })
}

/// The path of names from the root to `id` and the folder's role.
pub(super) fn folder_ref(folders: &HashMap<FolderId, Folder>, id: FolderId) -> Option<FolderRef> {
    let f = folders.get(&id)?;
    let mut path = vec![f.name.clone()];
    let mut parent = f.parent_id;
    let mut guard = 0;
    while let Some(p) = parent {
        let pf = folders.get(&p)?;
        path.push(pf.name.clone());
        parent = pf.parent_id;
        guard += 1;
        if guard > 64 {
            return None;
        }
    }
    path.reverse();
    Some(FolderRef { role: f.role, path })
}

/// Parents before children (stable otherwise).
pub(super) fn parents_first(mut folders: Vec<Folder>) -> Vec<Folder> {
    let ids: HashSet<FolderId> = folders.iter().map(|f| f.id).collect();
    let mut out = Vec::with_capacity(folders.len());
    let mut placed: HashSet<FolderId> = HashSet::new();
    while !folders.is_empty() {
        let before = folders.len();
        folders.retain(|f| {
            let ready = f.parent_id.is_none_or(|p| placed.contains(&p) || !ids.contains(&p));
            if ready {
                placed.insert(f.id);
                out.push(f.clone());
            }
            !ready
        });
        if folders.len() == before {
            // A parent cycle: append the rest as they are.
            out.append(&mut folders);
        }
    }
    out
}

/// `sync_items.kind` values of the sync engine (`axentra_sync::diff`).
const SYNC_EVENT: &str = "event";
const SYNC_CONTACT: &str = "contact";
const SYNC_TASK: &str = "task";
const SYNC_TASK_LIST: &str = "task_list";
const SYNC_CALENDAR: &str = "calendar";

/// Whether the sync engine has a record of item `local_id` of `kind`: it
/// came from or was pushed to the server, so it exists there.
fn synced(conn: &Connection, account: AccountId, kind: &str, local_id: i64) -> Result<bool> {
    let mut stmt = conn.prepare_cached("SELECT 1 FROM sync_items WHERE account_id = ?1 AND kind = ?2 AND local_id = ?3")?;
    Ok(stmt.exists(params![account, kind, local_id])?)
}

/// SQL condition (for `events`, `contacts` or `tasks`): no sync record of
/// `kind` for this row in account `?2`.
fn unsynced(table: &str, kind: &str) -> String {
    format!("NOT EXISTS (SELECT 1 FROM sync_items s WHERE s.account_id = ?2 AND s.kind = '{kind}' AND s.local_id = {table}.id)")
}

fn rows<T>(conn: &Connection, sql: &str, p: impl rusqlite::Params, f: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>) -> Result<Vec<T>> {
    let mut stmt = conn.prepare(sql)?;
    let out = stmt.query_map(p, f)?.collect::<rusqlite::Result<Vec<T>>>()?;
    Ok(out)
}

/// Everything of one message: body, attachments with bytes, draft, origin.
fn message_data(conn: &Connection, m: Message) -> Result<MessageData> {
    let body = conn
        .query_row("SELECT html, text FROM message_bodies WHERE message_id = ?1", [m.id], |r| {
            Ok(BodyData { html: r.get(0)?, text: r.get(1)? })
        })
        .optional()?;
    let attachments = rows(conn, "SELECT json, data FROM attachments WHERE message_id = ?1 ORDER BY id", [m.id], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, Option<Vec<u8>>>(1)?))
    })?
    .into_iter()
    .map(|(json, data)| Ok(AttachmentData { attachment: serde_json::from_str(&json)?, data }))
    .collect::<Result<Vec<_>>>()?;
    let draft = conn
        .query_row("SELECT json FROM drafts WHERE message_id = ?1", [m.id], |r| r.get::<_, String>(0))
        .optional()?
        .map(|j| serde_json::from_str::<Draft>(&j))
        .transpose()?;
    let origin_folder = conn.query_row("SELECT folder_id FROM mail_origins WHERE message_id = ?1", [m.id], |r| r.get(0)).optional()?;
    let draft_source_message_id = match draft.as_ref().and_then(|d| d.source_message_id) {
        Some(src) => conn
            .query_row("SELECT internet_message_id FROM messages WHERE id = ?1", [src], |r| r.get::<_, Option<String>>(0))
            .optional()?
            .flatten()
            .filter(|s| !s.is_empty()),
        None => None,
    };
    Ok(MessageData { message: m, body, attachments, draft, origin_folder, draft_source_message_id })
}

fn account_data(store: &Store, account: &Account, all_kv: &[(String, String)]) -> Result<AccountData> {
    let complete = account.kind == AccountKind::Local;
    let mut acct = account.clone();
    acct.config = strip_secrets(&account.config);
    let id = account.id;
    let mut data = AccountData { account: acct, complete, ..Default::default() };

    let all_folders: Vec<Folder> = Store::query_docs_in(&store.conn(), "WHERE account_id = ?1 ORDER BY sort_order, id", [id])?;
    let by_id: HashMap<FolderId, Folder> = all_folders.iter().map(|f| (f.id, f.clone())).collect();

    // Mail: everything for local accounts, unsent drafts for synced ones.
    let message_ids: Vec<MessageId> = {
        let conn = store.conn();
        if complete {
            rows(&conn, "SELECT id FROM messages WHERE account_id = ?1 ORDER BY id", [id], |r| r.get(0))?
        } else {
            rows(
                &conn,
                "SELECT m.id FROM messages m JOIN drafts d ON d.message_id = m.id WHERE m.account_id = ?1 AND m.is_draft = 1 ORDER BY m.id",
                [id],
                |r| r.get(0),
            )?
        }
    };
    for mid in message_ids {
        // One lock per message keeps the UI responsive during long backups.
        let conn = store.conn();
        let Some(m) = Store::get_doc_in::<Message>(&conn, mid)? else { continue };
        let mut md = message_data(&conn, m)?;
        if !complete {
            md.origin_folder = None;
        }
        data.messages.push(md);
    }
    if complete {
        data.folders = parents_first(all_folders.iter().cloned().map(|mut f| {
            f.unread_count = 0;
            f.total_count = 0;
            f
        }).collect());
        data.special_folders = rows(&store.conn(), "SELECT kind, folder_id FROM mail_special_folders WHERE account_id = ?1", [id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?;
    }

    // Calendars and events.
    let conn = store.conn();
    let calendars: Vec<Calendar> = Store::query_docs_in(&conn, "WHERE account_id = ?1 ORDER BY sort_order, id", [id])?;
    for cal in calendars {
        let local_calendar = complete || (cal.remote_id.is_none() && !synced(&conn, id, SYNC_CALENDAR, cal.id)?);
        let events: Vec<Event> = if complete {
            Store::query_docs_in(&conn, "WHERE calendar_id = ?1 ORDER BY id", [cal.id])?
        } else if local_calendar {
            let tail = format!("WHERE calendar_id = ?1 AND {} ORDER BY id", unsynced("events", SYNC_EVENT));
            Store::query_docs_in(&conn, &tail, params![cal.id, id])?
        } else {
            let tail = format!("WHERE calendar_id = ?1 AND remote_id IS NULL AND {} ORDER BY id", unsynced("events", SYNC_EVENT));
            Store::query_docs_in(&conn, &tail, params![cal.id, id])?
        };
        if !local_calendar && events.is_empty() {
            continue;
        }
        let ids: HashSet<EventId> = events.iter().map(|e| e.id).collect();
        let reminder_states = rows(
            &conn,
            "SELECT r.event_id, r.occurrence_start, r.dismissed, r.snoozed_until FROM reminder_state r
             JOIN events e ON e.id = r.event_id WHERE e.calendar_id = ?1",
            [cal.id],
            |r| Ok(ReminderState { event_id: r.get(0)?, occurrence_start: r.get(1)?, dismissed: r.get(2)?, snoozed_until: r.get(3)? }),
        )?
        .into_iter()
        .filter(|s| ids.contains(&s.event_id))
        .collect();
        data.calendars.push(CalendarData { calendar: cal, container_only: !local_calendar, events, reminder_states });
    }

    // People: contact lists are never synced, so they are always included.
    data.contacts = if complete {
        Store::query_docs_in(&conn, "WHERE account_id = ?1 ORDER BY id", [id])?
    } else {
        let tail = format!("WHERE account_id = ?1 AND remote_id IS NULL AND {} ORDER BY id", unsynced("contacts", SYNC_CONTACT));
        Store::query_docs_in(&conn, &tail, params![id, id])?
    };
    data.contact_lists = Store::query_docs_in(&conn, "WHERE account_id = ?1 ORDER BY id", [id])?;

    // To Do.
    let lists: Vec<TaskList> = Store::query_docs_in(&conn, "WHERE account_id = ?1 ORDER BY sort_order, id", [id])?;
    for list in lists {
        let local_list = complete || (list.remote_id.is_none() && !synced(&conn, id, SYNC_TASK_LIST, list.id)?);
        let tasks: Vec<Task> = if complete {
            Store::query_docs_in(&conn, "WHERE list_id = ?1 ORDER BY sort_order, id", [list.id])?
        } else if local_list {
            let tail = format!("WHERE list_id = ?1 AND {} ORDER BY sort_order, id", unsynced("tasks", SYNC_TASK));
            Store::query_docs_in(&conn, &tail, params![list.id, id])?
        } else {
            let tail = format!("WHERE list_id = ?1 AND remote_id IS NULL AND {} ORDER BY sort_order, id", unsynced("tasks", SYNC_TASK));
            Store::query_docs_in(&conn, &tail, params![list.id, id])?
        };
        if !local_list && tasks.is_empty() {
            continue;
        }
        data.task_lists.push(TaskListData { list, container_only: !local_list, tasks });
    }

    // Sweep rules, by folder role / path so they survive new ids.
    let sweep = rows(
        &conn,
        "SELECT folder_id, sender, mode, target_folder_id, created_at, last_run FROM mail_sweep_rules WHERE account_id = ?1 ORDER BY id",
        [id],
        |r| Ok((r.get::<_, FolderId>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, FolderId>(3)?, r.get::<_, i64>(4)?, r.get::<_, Option<i64>>(5)?)),
    )?;
    drop(conn);
    for (folder, sender, mode, target, created_at, last_run) in sweep {
        if let (Some(folder), Some(target)) = (folder_ref(&by_id, folder), folder_ref(&by_id, target)) {
            data.sweep_rules.push(SweepRule { folder, sender, mode, target, created_at, last_run });
        }
    }

    data.kv = all_kv
        .iter()
        .filter_map(|(k, v)| {
            let (prefix, owner, rest) = split_account_key(k)?;
            (owner == id).then(|| AccountKv { prefix: prefix.into(), rest: rest.into(), value: v.clone() })
        })
        .collect();
    Ok(data)
}

/// Gather the backup document. `now` becomes `manifest.created_at`.
pub fn collect(store: &Store, now: DateTime<Utc>) -> Result<Payload> {
    let (recent_addresses, blocked_senders, safe_senders, inference_overrides, all_kv) = {
        let conn = store.conn();
        let recent = rows(&conn, "SELECT address, name, last_used, use_count FROM recent_addresses ORDER BY address", [], |r| {
            Ok(RecentAddress { address: r.get(0)?, name: r.get(1)?, last_used: r.get(2)?, use_count: r.get(3)? })
        })?;
        let blocked = rows(&conn, "SELECT address, name, added_at FROM mail_blocked_senders ORDER BY address", [], |r| {
            Ok(BlockedSender { address: r.get(0)?, name: r.get(1)?, added_at: r.get(2)? })
        })?;
        let safe = rows(&conn, "SELECT address, added_at FROM mail_safe_senders ORDER BY address", [], |r| {
            Ok(SafeSender { address: r.get(0)?, added_at: r.get(1)? })
        })?;
        let overrides = rows(&conn, "SELECT address, inference FROM mail_inference_overrides ORDER BY address", [], |r| {
            Ok(InferenceOverride { address: r.get(0)?, inference: r.get(1)? })
        })?;
        let kv = rows(&conn, "SELECT key, value FROM kv ORDER BY key", params![], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        (recent, blocked, safe, overrides, kv)
    };
    let mut payload = Payload {
        settings: store.settings(),
        categories: store.categories()?,
        recent_addresses,
        blocked_senders,
        safe_senders,
        inference_overrides,
        kv: all_kv.iter().filter(|(k, _)| kv_allowed(k)).map(|(k, v)| KvEntry { key: k.clone(), value: v.clone() }).collect(),
        ..Default::default()
    };
    for account in store.accounts()? {
        payload.accounts.push(account_data(store, &account, &all_kv)?);
    }
    let mut counts = Counts { categories: payload.categories.len(), ..Default::default() };
    for a in &payload.accounts {
        counts.add(&a.counts());
    }
    payload.manifest = Manifest { created_at: now, counts, ..Default::default() };
    Ok(payload)
}
