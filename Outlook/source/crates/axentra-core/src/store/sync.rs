//! Storage helpers used by `axentra-sync` (owned by the sync engine).
//!
//! The engine keeps its bookkeeping in its own tables so the other areas
//! never need to know about servers:
//!
//! * `sync_items`: one row per synced item (message, event, contact, task,
//!   task list, calendar) with the remote id, the server container (folder,
//!   calendar or list id) and a JSON snapshot of the item as last synced.
//!   Rows are deliberately *not* cascaded from the item tables: a row whose
//!   local item is gone is the tombstone that tells the next sync to delete
//!   the item on the server.
//! * `sync_folders`: per mail folder cursors (IMAP UIDVALIDITY / UIDNEXT /
//!   MODSEQ, Graph delta links).
//! * `sync_cursors`: other per-account cursors and flags.
//! * `account_photos`: the signed-in user's own profile photo per account
//!   ([`AccountPhoto`] JSON), shown on the account avatars. Server data: it
//!   goes with the account and is never backed up.
//!
//! The snapshot types live in `axentra-sync`; these methods are generic over
//! serde so the core crate doesn't depend on them.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::Store;
use crate::model::*;
use crate::{Error, Result};

pub(crate) const SCHEMA_EXTRA: &str = "
CREATE TABLE IF NOT EXISTS sync_items (
    account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    local_id INTEGER NOT NULL,
    remote_id TEXT NOT NULL,
    container TEXT NOT NULL DEFAULT '',
    json TEXT NOT NULL,
    PRIMARY KEY (account_id, kind, local_id));
CREATE UNIQUE INDEX IF NOT EXISTS sync_items_remote ON sync_items(account_id, kind, remote_id);
CREATE INDEX IF NOT EXISTS sync_items_container ON sync_items(account_id, kind, container);
CREATE TABLE IF NOT EXISTS sync_folders (
    folder_id INTEGER PRIMARY KEY REFERENCES folders(id) ON DELETE CASCADE,
    account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    remote_id TEXT NOT NULL,
    json TEXT NOT NULL);
CREATE UNIQUE INDEX IF NOT EXISTS sync_folders_remote ON sync_folders(account_id, remote_id);
CREATE TABLE IF NOT EXISTS sync_cursors (
    account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (account_id, key));
CREATE TABLE IF NOT EXISTS account_photos (
    account_id INTEGER PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    json TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS messages_internet_id ON messages(account_id, internet_message_id);
";

/// Lower-cased subject without any number of `RE:` / `FW:` / `FWD:` / `AW:`
/// / `WG:` / `SV:` prefixes, used to thread messages that lack references.
pub(crate) fn normalize_subject(subject: &str) -> String {
    let mut s = subject.trim();
    loop {
        let lower = s.to_ascii_lowercase();
        let cut = ["re:", "fw:", "fwd:", "aw:", "wg:", "sv:", "vs:", "antw:", "réf:", "tr:"]
            .iter()
            .find(|p| lower.starts_with(*p))
            .map(|p| p.len())
            .or_else(|| {
                // "RE[2]:" style counters.
                let rest = lower.strip_prefix("re[")?;
                let close = rest.find("]:")?;
                rest[..close].chars().all(|c| c.is_ascii_digit()).then_some(3 + close + 2)
            });
        match cut {
            Some(n) => s = s[n..].trim_start(),
            None => break,
        }
    }
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

impl Store {
    // ------------------------------------------------------------ shadow rows

    /// Insert or replace the sync snapshot of one local item.
    pub fn sync_put<T: Serialize>(
        &self,
        account_id: AccountId,
        kind: &str,
        local_id: i64,
        remote_id: &str,
        container: &str,
        state: &T,
    ) -> Result<()> {
        let json = serde_json::to_string(state)?;
        let conn = self.conn();
        // A remote id moves to a new local id when an item was re-created
        // locally (e.g. after undo); keep the unique index happy.
        conn.execute(
            "DELETE FROM sync_items WHERE account_id = ?1 AND kind = ?2 AND remote_id = ?3 AND local_id <> ?4",
            params![account_id, kind, remote_id, local_id],
        )?;
        conn.execute(
            "INSERT INTO sync_items (account_id, kind, local_id, remote_id, container, json) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(account_id, kind, local_id) DO UPDATE SET remote_id = excluded.remote_id,
               container = excluded.container, json = excluded.json",
            params![account_id, kind, local_id, remote_id, container, json],
        )?;
        Ok(())
    }

    /// Snapshot of one local item.
    pub fn sync_get<T: DeserializeOwned>(&self, account_id: AccountId, kind: &str, local_id: i64) -> Result<Option<T>> {
        let json: Option<String> = self
            .conn()
            .query_row(
                "SELECT json FROM sync_items WHERE account_id = ?1 AND kind = ?2 AND local_id = ?3",
                params![account_id, kind, local_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(match json {
            Some(j) => Some(serde_json::from_str(&j)?),
            None => None,
        })
    }

    /// Local id and snapshot of the item with this remote id.
    pub fn sync_find<T: DeserializeOwned>(&self, account_id: AccountId, kind: &str, remote_id: &str) -> Result<Option<(i64, T)>> {
        let row: Option<(i64, String)> = self
            .conn()
            .query_row(
                "SELECT local_id, json FROM sync_items WHERE account_id = ?1 AND kind = ?2 AND remote_id = ?3",
                params![account_id, kind, remote_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        Ok(match row {
            Some((id, j)) => Some((id, serde_json::from_str(&j)?)),
            None => None,
        })
    }

    /// All snapshots of a kind (optionally only those in one container),
    /// as `(local_id, snapshot)` ordered by local id.
    pub fn sync_list<T: DeserializeOwned>(&self, account_id: AccountId, kind: &str, container: Option<&str>) -> Result<Vec<(i64, T)>> {
        let conn = self.conn();
        let mut out = Vec::new();
        let mut push = |rows: &mut rusqlite::Rows<'_>| -> Result<()> {
            while let Some(r) = rows.next()? {
                let id: i64 = r.get(0)?;
                let json: String = r.get(1)?;
                out.push((id, serde_json::from_str(&json)?));
            }
            Ok(())
        };
        match container {
            Some(c) => {
                let mut stmt = conn.prepare_cached(
                    "SELECT local_id, json FROM sync_items WHERE account_id = ?1 AND kind = ?2 AND container = ?3 ORDER BY local_id",
                )?;
                push(&mut stmt.query(params![account_id, kind, c])?)?;
            }
            None => {
                let mut stmt =
                    conn.prepare_cached("SELECT local_id, json FROM sync_items WHERE account_id = ?1 AND kind = ?2 ORDER BY local_id")?;
                push(&mut stmt.query(params![account_id, kind])?)?;
            }
        }
        Ok(out)
    }

    pub fn sync_remove(&self, account_id: AccountId, kind: &str, local_id: i64) -> Result<()> {
        self.conn().execute(
            "DELETE FROM sync_items WHERE account_id = ?1 AND kind = ?2 AND local_id = ?3",
            params![account_id, kind, local_id],
        )?;
        Ok(())
    }

    /// Forget every snapshot of a kind in one container (e.g. after an IMAP
    /// UIDVALIDITY change).
    pub fn sync_remove_container(&self, account_id: AccountId, kind: &str, container: &str) -> Result<()> {
        self.conn().execute(
            "DELETE FROM sync_items WHERE account_id = ?1 AND kind = ?2 AND container = ?3",
            params![account_id, kind, container],
        )?;
        Ok(())
    }

    /// Move every snapshot of a kind from container `old` to `new` (an IMAP
    /// mailbox was renamed; UIDs stay valid). Returns how many moved.
    pub fn sync_rename_container(&self, account_id: AccountId, kind: &str, old: &str, new: &str) -> Result<usize> {
        Ok(self.conn().execute(
            "UPDATE sync_items SET container = ?4 WHERE account_id = ?1 AND kind = ?2 AND container = ?3",
            params![account_id, kind, old, new],
        )?)
    }

    // ------------------------------------------------------------ folder cursors

    pub fn sync_folder_put<T: Serialize>(&self, folder_id: FolderId, account_id: AccountId, remote_id: &str, state: &T) -> Result<()> {
        let json = serde_json::to_string(state)?;
        let conn = self.conn();
        conn.execute(
            "DELETE FROM sync_folders WHERE account_id = ?1 AND remote_id = ?2 AND folder_id <> ?3",
            params![account_id, remote_id, folder_id],
        )?;
        conn.execute(
            "INSERT INTO sync_folders (folder_id, account_id, remote_id, json) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(folder_id) DO UPDATE SET remote_id = excluded.remote_id, json = excluded.json",
            params![folder_id, account_id, remote_id, json],
        )?;
        Ok(())
    }

    pub fn sync_folder_get<T: DeserializeOwned>(&self, folder_id: FolderId) -> Result<Option<T>> {
        let json: Option<String> = self
            .conn()
            .query_row("SELECT json FROM sync_folders WHERE folder_id = ?1", [folder_id], |r| r.get(0))
            .optional()?;
        Ok(match json {
            Some(j) => Some(serde_json::from_str(&j)?),
            None => None,
        })
    }

    /// `(folder_id, remote_id, state)` for every synced folder of an account.
    pub fn sync_folder_list<T: DeserializeOwned>(&self, account_id: AccountId) -> Result<Vec<(FolderId, String, T)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached("SELECT folder_id, remote_id, json FROM sync_folders WHERE account_id = ?1 ORDER BY folder_id")?;
        let mut rows = stmt.query([account_id])?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            let json: String = r.get(2)?;
            out.push((r.get(0)?, r.get(1)?, serde_json::from_str(&json)?));
        }
        Ok(out)
    }

    pub fn sync_folder_remove(&self, folder_id: FolderId) -> Result<()> {
        self.conn().execute("DELETE FROM sync_folders WHERE folder_id = ?1", [folder_id])?;
        Ok(())
    }

    // ------------------------------------------------------------ cursors

    pub fn sync_cursor(&self, account_id: AccountId, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row("SELECT value FROM sync_cursors WHERE account_id = ?1 AND key = ?2", params![account_id, key], |r| r.get(0))
            .optional()?)
    }

    /// Set (or with `None`, clear) a cursor.
    pub fn set_sync_cursor(&self, account_id: AccountId, key: &str, value: Option<&str>) -> Result<()> {
        let conn = self.conn();
        match value {
            Some(v) => conn.execute(
                "INSERT INTO sync_cursors (account_id, key, value) VALUES (?1, ?2, ?3)
                 ON CONFLICT(account_id, key) DO UPDATE SET value = excluded.value",
                params![account_id, key, v],
            )?,
            None => conn.execute("DELETE FROM sync_cursors WHERE account_id = ?1 AND key = ?2", params![account_id, key])?,
        };
        Ok(())
    }

    // ------------------------------------------------------------ account photos

    /// An account's stored profile photo record (photo and check times).
    pub fn account_photo(&self, account_id: AccountId) -> Result<Option<AccountPhoto>> {
        let json: Option<String> = self
            .conn()
            .query_row("SELECT json FROM account_photos WHERE account_id = ?1", [account_id], |r| r.get(0))
            .optional()?;
        Ok(json.and_then(|j| serde_json::from_str(&j).ok()))
    }

    /// Save the photo record of `account` (as read when its check began).
    /// Saves nothing, and returns `false`, when that account is gone: removed,
    /// or replaced by a newer account that reused its id (`accounts.id` is a
    /// plain rowid), which the creation time tells apart.
    pub fn set_account_photo(&self, account: &Account, photo: &AccountPhoto) -> Result<bool> {
        let json = serde_json::to_string(photo)?;
        self.tx(|tx| {
            let current = Self::get_doc_in::<Account>(tx, account.id)?;
            if current.is_none_or(|a| a.created_at != account.created_at) {
                return Ok(false);
            }
            tx.execute(
                "INSERT INTO account_photos (account_id, json) VALUES (?1, ?2) ON CONFLICT(account_id) DO UPDATE SET json = excluded.json",
                params![account.id, json],
            )?;
            Ok(true)
        })
    }

    /// Forget an account's photo and when it was checked.
    pub fn clear_account_photo(&self, account_id: AccountId) -> Result<()> {
        self.conn().execute("DELETE FROM account_photos WHERE account_id = ?1", [account_id])?;
        Ok(())
    }

    /// Every account that has a photo, as account id → `data:image/…` URL.
    pub fn account_photos(&self) -> Result<HashMap<AccountId, String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached("SELECT account_id, json FROM account_photos")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, AccountId>(0)?, r.get::<_, String>(1)?)))?;
        let mut out = HashMap::new();
        for row in rows {
            let (id, json) = row?;
            if let Some(url) = serde_json::from_str::<AccountPhoto>(&json).ok().and_then(|p| p.data_url).filter(|u| u.starts_with("data:image/")) {
                out.insert(id, url);
            }
        }
        Ok(out)
    }

    /// Rows changed through this store since it was opened. The engine
    /// compares it between ticks to notice local edits worth pushing.
    pub fn total_changes(&self) -> u64 {
        self.conn().total_changes()
    }

    // ------------------------------------------------------------ accounts

    /// Read-modify-write one account atomically, so the engine and the
    /// settings page can't overwrite each other's fields.
    pub fn update_account_with(&self, id: AccountId, f: impl FnOnce(&mut Account)) -> Result<Account> {
        self.tx(|tx| {
            let mut a: Account = Self::get_doc_in(tx, id)?.ok_or_else(|| Error::NotFound(format!("account {id}")))?;
            f(&mut a);
            a.id = id;
            Self::update_doc_in(tx, &a)?;
            Ok(a)
        })
    }

    /// Make `id` the primary account (first in the list); the others keep
    /// their relative order.
    pub fn set_primary_account(&self, id: AccountId) -> Result<()> {
        self.tx(|tx| {
            let mut all: Vec<Account> = Self::query_docs_in(tx, "ORDER BY sort_order, id", [])?;
            if !all.iter().any(|a| a.id == id) {
                return Err(Error::NotFound(format!("account {id}")));
            }
            all.sort_by_key(|a| (a.id != id, a.sort_order, a.id));
            for (i, a) in all.iter_mut().enumerate() {
                if a.sort_order != i as i64 {
                    a.sort_order = i as i64;
                    Self::update_doc_in(tx, a)?;
                }
            }
            Ok(())
        })
    }

    /// Sort order for a newly added account (after every existing one).
    pub fn next_account_sort_order(&self) -> Result<i64> {
        let max: Option<i64> = self.conn().query_row("SELECT MAX(sort_order) FROM accounts", [], |r| r.get(0))?;
        Ok(max.map(|m| m + 1).unwrap_or(0))
    }

    // ------------------------------------------------------------ folders

    /// Insert (when `id == 0`) or update a folder as written by the engine.
    pub fn save_synced_folder(&self, folder: &mut Folder) -> Result<FolderId> {
        let conn = self.conn();
        if folder.id == 0 {
            Self::insert_doc_in(&conn, folder)
        } else {
            Self::update_doc_in(&conn, folder)?;
            Ok(folder.id)
        }
    }

    /// Remove a folder, its subfolders and their messages (the folder was
    /// deleted on the server). Well-known folders are only unlinked.
    pub fn remove_synced_folder(&self, id: FolderId) -> Result<()> {
        let Some(folder) = Self::get_doc_in::<Folder>(&self.conn(), id)? else { return Ok(()) };
        if folder.role != FolderRole::Custom {
            let mut f = folder;
            f.remote_id = None;
            self.sync_folder_remove(id)?;
            return self.update_doc(&f);
        }
        let all: Vec<Folder> = self.query_docs("WHERE account_id = ?1", [folder.account_id])?;
        let mut stack = vec![id];
        let mut doomed = Vec::new();
        while let Some(f) = stack.pop() {
            doomed.push(f);
            stack.extend(all.iter().filter(|c| c.parent_id == Some(f)).map(|c| c.id));
        }
        self.tx(|tx| {
            for f in doomed {
                Self::delete_doc_in::<Folder>(tx, f)?;
            }
            Ok(())
        })
    }

    // ------------------------------------------------------------ messages

    /// Every message of an account, including snoozed ones, oldest id first.
    pub fn account_messages(&self, account_id: AccountId) -> Result<Vec<Message>> {
        self.query_docs("WHERE account_id = ?1 ORDER BY id", [account_id])
    }

    /// Every message in one folder, including snoozed ones.
    pub fn folder_messages(&self, folder_id: FolderId) -> Result<Vec<Message>> {
        self.query_docs("WHERE folder_id = ?1 ORDER BY id", [folder_id])
    }

    /// Insert a message received from a server together with its body and
    /// attachments (with their bytes, when downloaded), in one transaction.
    pub fn insert_synced_message(
        &self,
        message: &mut Message,
        body: &MessageBody,
        attachments: &[(Attachment, Option<Vec<u8>>)],
    ) -> Result<MessageId> {
        self.tx(|tx| {
            if message.conversation_id.is_empty() {
                message.conversation_id = format!("local-{}", uuid::Uuid::new_v4());
            }
            message.id = 0;
            let id = Self::insert_doc_in(tx, message)?;
            tx.execute(
                "INSERT OR REPLACE INTO message_bodies (message_id, html, text) VALUES (?1, ?2, ?3)",
                params![id, body.html, body.text],
            )?;
            for (a, data) in attachments {
                let mut a = a.clone();
                a.id = 0;
                a.message_id = id;
                let aid = Self::insert_doc_in(tx, &mut a)?;
                if let Some(d) = data {
                    tx.execute("UPDATE attachments SET data = ?1 WHERE id = ?2", params![d, aid])?;
                }
            }
            Ok(id)
        })
    }

    /// Replace the attachments of a message (bodies re-downloaded).
    pub fn replace_synced_attachments(&self, message_id: MessageId, attachments: &[(Attachment, Option<Vec<u8>>)]) -> Result<()> {
        self.tx(|tx| {
            tx.execute("DELETE FROM attachments WHERE message_id = ?1", [message_id])?;
            for (a, data) in attachments {
                let mut a = a.clone();
                a.id = 0;
                a.message_id = message_id;
                let aid = Self::insert_doc_in(tx, &mut a)?;
                if let Some(d) = data {
                    tx.execute("UPDATE attachments SET data = ?1 WHERE id = ?2", params![d, aid])?;
                }
            }
            Ok(())
        })
    }

    /// Conversation id of the first local message (of this account) whose
    /// Message-ID is one of `ids` (without angle brackets).
    pub fn conversation_for_message_ids(&self, account_id: AccountId, ids: &[String]) -> Result<Option<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT conversation_id FROM messages WHERE account_id = ?1 AND internet_message_id = ?2 ORDER BY id LIMIT 1",
        )?;
        for id in ids {
            if id.is_empty() {
                continue;
            }
            let found: Option<String> = stmt.query_row(params![account_id, id], |r| r.get(0)).optional()?;
            if found.is_some() {
                return Ok(found);
            }
        }
        Ok(None)
    }

    /// Conversation id of a recent message (received after `since`) with the
    /// same subject once `RE:` / `FW:` style prefixes are ignored.
    pub fn conversation_for_subject(&self, account_id: AccountId, subject: &str, since: DateTime<Utc>) -> Result<Option<String>> {
        let normalized = normalize_subject(subject);
        let normalized = normalized.as_str();
        if normalized.is_empty() {
            return Ok(None);
        }
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT conversation_id, json_extract(json, '$.subject') FROM messages
             WHERE account_id = ?1 AND received_at >= ?2 ORDER BY received_at DESC LIMIT 500",
        )?;
        let mut rows = stmt.query(params![account_id, since.timestamp()])?;
        while let Some(r) = rows.next()? {
            let subject: Option<String> = r.get(1)?;
            if subject.as_deref().map(normalize_subject).as_deref() == Some(normalized) {
                return Ok(Some(r.get(0)?));
            }
        }
        Ok(None)
    }

    /// Local message in `folder_id` with this Message-ID (used to match a
    /// server copy with a message the app itself sent, appended or moved).
    pub fn folder_message_by_internet_id(&self, folder_id: FolderId, internet_message_id: &str) -> Result<Option<Message>> {
        if internet_message_id.is_empty() {
            return Ok(None);
        }
        Ok(self
            .query_docs::<Message>("WHERE folder_id = ?1 AND internet_message_id = ?2 ORDER BY id LIMIT 1", params![folder_id, internet_message_id])?
            .into_iter()
            .next())
    }

    /// After a queued message was handed to the server: move it from Outbox
    /// to `sent_folder`, record its Message-ID and drop the editable draft.
    pub fn finish_sent_message(&self, id: MessageId, sent_folder: FolderId, internet_message_id: &str) -> Result<()> {
        self.tx(|tx| {
            let mut m: Message = Self::get_doc_in(tx, id)?.ok_or_else(|| Error::NotFound(format!("message {id}")))?;
            m.folder_id = sent_folder;
            m.is_draft = false;
            m.is_read = true;
            if !internet_message_id.is_empty() {
                m.internet_message_id = internet_message_id.to_string();
            }
            Self::update_doc_in(tx, &m)?;
            tx.execute("DELETE FROM drafts WHERE message_id = ?1", [id])?;
            Ok(())
        })
    }

    // ------------------------------------------------------------ calendar, people, tasks

    /// Every event of a calendar (series masters unexpanded).
    pub fn calendar_events(&self, calendar_id: CalendarId) -> Result<Vec<Event>> {
        self.query_docs("WHERE calendar_id = ?1 ORDER BY id", [calendar_id])
    }

    /// Every contact of one account.
    pub fn account_contacts(&self, account_id: AccountId) -> Result<Vec<Contact>> {
        self.query_docs("WHERE account_id = ?1 ORDER BY id", [account_id])
    }

    /// Item counts of an account: (messages, events, contacts, tasks), shown
    /// in the remove-account confirmation.
    pub fn account_item_counts(&self, account_id: AccountId) -> Result<(i64, i64, i64, i64)> {
        let conn = self.conn();
        let one = |sql: &str| -> Result<i64> { Ok(conn.query_row(sql, [account_id], |r| r.get(0))?) };
        Ok((
            one("SELECT COUNT(*) FROM messages WHERE account_id = ?1")?,
            one("SELECT COUNT(*) FROM events WHERE calendar_id IN (SELECT id FROM calendars WHERE account_id = ?1)")?,
            one("SELECT COUNT(*) FROM contacts WHERE account_id = ?1")?,
            one("SELECT COUNT(*) FROM tasks WHERE list_id IN (SELECT id FROM task_lists WHERE account_id = ?1)")?,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_util::store_with_account;
    use serde::Deserialize;

    #[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
    struct Snap {
        remote_id: String,
        is_read: bool,
    }

    fn message(store: &Store, acct: AccountId, subject: &str, mid: &str, conv: &str) -> MessageId {
        let inbox = store.folder_for_role(acct, FolderRole::Inbox).unwrap();
        let mut m = Message {
            account_id: acct,
            folder_id: inbox,
            subject: subject.into(),
            internet_message_id: mid.into(),
            conversation_id: conv.into(),
            received_at: Utc::now(),
            ..Default::default()
        };
        store.insert_synced_message(&mut m, &MessageBody::default(), &[]).unwrap()
    }

    #[test]
    fn shadow_rows_round_trip_and_survive_item_deletion() {
        let (store, acct) = store_with_account();
        let id = message(&store, acct, "Hello", "a@x", "c1");
        store.sync_put(acct, "message", id, "R1", "INBOX", &Snap { remote_id: "R1".into(), is_read: false }).unwrap();
        assert_eq!(store.sync_get::<Snap>(acct, "message", id).unwrap().unwrap().remote_id, "R1");
        assert_eq!(store.sync_find::<Snap>(acct, "message", "R1").unwrap().unwrap().0, id);
        assert_eq!(store.sync_list::<Snap>(acct, "message", Some("INBOX")).unwrap().len(), 1);
        assert!(store.sync_list::<Snap>(acct, "message", Some("Sent")).unwrap().is_empty());

        // Deleting the message keeps the tombstone.
        store.delete_messages_permanently(&[id]).unwrap();
        assert!(store.message(id).unwrap().is_none());
        assert_eq!(store.sync_list::<Snap>(acct, "message", None).unwrap().len(), 1);

        // Re-pointing a remote id at a new local id replaces the old row.
        store.sync_put(acct, "message", id + 100, "R1", "INBOX", &Snap::default()).unwrap();
        let all = store.sync_list::<Snap>(acct, "message", None).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].0, id + 100);

        assert_eq!(store.sync_rename_container(acct, "message", "INBOX", "Inbox2").unwrap(), 1);
        assert_eq!(store.sync_list::<Snap>(acct, "message", Some("Inbox2")).unwrap().len(), 1);
        assert_eq!(store.sync_rename_container(acct, "message", "INBOX", "Inbox2").unwrap(), 0);
        store.sync_remove_container(acct, "message", "Inbox2").unwrap();
        assert!(store.sync_list::<Snap>(acct, "message", None).unwrap().is_empty());

        // Removing the account cascades.
        store.sync_put(acct, "event", 1, "E1", "cal", &Snap::default()).unwrap();
        store.set_sync_cursor(acct, "calendar", Some("x")).unwrap();
        store.remove_account(acct).unwrap();
        assert!(store.sync_list::<Snap>(acct, "event", None).unwrap().is_empty());
        assert!(store.sync_cursor(acct, "calendar").unwrap().is_none());
    }

    #[test]
    fn folder_cursors_and_synced_folder_removal() {
        let (store, acct) = store_with_account();
        let mut parent = Folder { account_id: acct, name: "Projects".into(), remote_id: Some("Projects".into()), ..Default::default() };
        let pid = store.save_synced_folder(&mut parent).unwrap();
        let mut child = Folder { account_id: acct, parent_id: Some(pid), name: "X".into(), ..Default::default() };
        let cid = store.save_synced_folder(&mut child).unwrap();
        store.sync_folder_put(pid, acct, "Projects", &serde_json::json!({"uidnext": 5})).unwrap();
        let list = store.sync_folder_list::<serde_json::Value>(acct).unwrap();
        assert_eq!(list, vec![(pid, "Projects".to_string(), serde_json::json!({"uidnext": 5}))]);
        store.remove_synced_folder(pid).unwrap();
        assert!(store.folder(pid).unwrap().is_none());
        assert!(store.folder(cid).unwrap().is_none());
        assert!(store.sync_folder_list::<serde_json::Value>(acct).unwrap().is_empty());

        // Well-known folders are only unlinked.
        let mut inbox = store.folder_by_role(acct, FolderRole::Inbox).unwrap().unwrap();
        inbox.remote_id = Some("INBOX".into());
        store.save_synced_folder(&mut inbox).unwrap();
        store.remove_synced_folder(inbox.id).unwrap();
        let inbox = store.folder(inbox.id).unwrap().unwrap();
        assert_eq!(inbox.remote_id, None);
    }

    #[test]
    fn threading_lookups() {
        let (store, acct) = store_with_account();
        message(&store, acct, "Budget review", "root@x", "conv-1");
        assert_eq!(
            store.conversation_for_message_ids(acct, &["nope@x".into(), "root@x".into()]).unwrap().as_deref(),
            Some("conv-1")
        );
        assert_eq!(store.conversation_for_message_ids(acct, &["nope@x".into()]).unwrap(), None);
        let since = Utc::now() - chrono::Duration::days(1);
        assert_eq!(store.conversation_for_subject(acct, "RE: Budget Review", since).unwrap().as_deref(), Some("conv-1"));
        assert_eq!(store.conversation_for_subject(acct, "other", since).unwrap(), None);
        assert_eq!(normalize_subject("RE: Fwd: RE[2]:  Budget   Review"), "budget review");
        assert_eq!(normalize_subject("AW: WG: Hallo"), "hallo");
    }

    #[test]
    fn account_photos_round_trip_and_go_with_the_account() {
        let (store, a) = store_with_account();
        let mut other = Account { kind: AccountKind::Microsoft, email: "b@x".into(), display_name: "B".into(), ..Default::default() };
        let b = store.add_account(&mut other).unwrap();
        let (acct_a, acct_b) = (store.account(a).unwrap().unwrap(), store.account(b).unwrap().unwrap());
        assert_eq!(store.account_photo(a).unwrap(), None);
        assert!(store.account_photos().unwrap().is_empty());

        let photo = AccountPhoto { etag: "E1".into(), data_url: Some("data:image/png;base64,iVBORw0K".into()), checked_at: Some(Utc::now()), ..Default::default() };
        assert!(store.set_account_photo(&acct_a, &photo).unwrap());
        assert_eq!(store.account_photo(a).unwrap().as_ref(), Some(&photo));
        // Checked, no photo: a record for the throttle, nothing to show.
        store.set_account_photo(&acct_b, &AccountPhoto { checked_at: Some(Utc::now()), ..Default::default() }).unwrap();
        let all = store.account_photos().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[&a], "data:image/png;base64,iVBORw0K");

        // Replacing, then a value that isn't an image URL never reaches the avatars.
        store.set_account_photo(&acct_a, &AccountPhoto { etag: "E2".into(), data_url: Some("data:image/jpeg;base64,/9j/".into()), ..photo.clone() }).unwrap();
        assert_eq!(store.account_photos().unwrap()[&a], "data:image/jpeg;base64,/9j/");
        store.set_account_photo(&acct_b, &AccountPhoto { data_url: Some("https://example.com/me.png".into()), ..Default::default() }).unwrap();
        assert!(!store.account_photos().unwrap().contains_key(&b));

        store.clear_account_photo(b).unwrap();
        assert_eq!(store.account_photo(b).unwrap(), None);

        // Removing the account removes its photo; a late write for it is a no-op.
        store.remove_account(a).unwrap();
        assert_eq!(store.account_photo(a).unwrap(), None);
        assert!(!store.set_account_photo(&acct_a, &photo).unwrap());
        assert_eq!(store.account_photo(a).unwrap(), None);
        assert!(store.account_photos().unwrap().is_empty());

        // A newer account that reuses the id of a removed one never gets the
        // removed account's photo, only its own.
        store.remove_account(b).unwrap();
        let mut newer = Account { kind: AccountKind::Microsoft, email: "c@x".into(), display_name: "C".into(), ..Default::default() };
        let c = store.add_account(&mut newer).unwrap();
        assert_eq!(c, a, "SQLite reuses the ids of removed accounts");
        assert!(!store.set_account_photo(&acct_a, &photo).unwrap());
        assert!(!store.set_account_photo(&acct_b, &photo).unwrap());
        assert_eq!(store.account_photo(c).unwrap(), None);
        assert!(store.set_account_photo(&store.account(c).unwrap().unwrap(), &photo).unwrap());
        assert_eq!(store.account_photos().unwrap()[&c], "data:image/png;base64,iVBORw0K");
    }

    #[test]
    fn account_patches_primary_and_sent_finish() {
        let (store, a) = store_with_account();
        let mut other = Account { kind: AccountKind::Imap, email: "b@x".into(), display_name: "B".into(), ..Default::default() };
        other.sort_order = store.next_account_sort_order().unwrap();
        let b = store.add_account(&mut other).unwrap();
        assert_eq!(store.primary_account().unwrap().unwrap().id, a);
        store.set_primary_account(b).unwrap();
        assert_eq!(store.primary_account().unwrap().unwrap().id, b);
        let patched = store.update_account_with(a, |acct| acct.sync_error = Some("boom".into())).unwrap();
        assert_eq!(patched.sync_error.as_deref(), Some("boom"));
        assert_eq!(store.account(a).unwrap().unwrap().sync_error.as_deref(), Some("boom"));

        let id = message(&store, b, "Out", "", "c");
        let sent = store.folder_for_role(b, FolderRole::Sent).unwrap();
        store.finish_sent_message(id, sent, "uuid@x").unwrap();
        let m = store.message(id).unwrap().unwrap();
        assert_eq!((m.folder_id, m.internet_message_id.as_str()), (sent, "uuid@x"));
        assert_eq!(store.folder_message_by_internet_id(sent, "uuid@x").unwrap().unwrap().id, id);
        assert_eq!(store.account_item_counts(b).unwrap().0, 1);
    }
}
