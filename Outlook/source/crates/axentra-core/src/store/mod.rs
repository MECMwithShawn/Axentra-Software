//! SQLite-backed local store.
//!
//! Every entity is stored as a JSON document (`json` column) plus a handful of
//! indexed columns that queries filter and sort on. The JSON is the source of
//! truth; index columns are rewritten from the struct on every save via
//! [`Doc::index_values`]. Adding a field to a model therefore needs no
//! migration as long as it has a serde default.
//!
//! Each feature area owns one file (`mail.rs`, `drafts.rs`, `calendar.rs`,
//! `people.rs`, `tasks.rs`, `sync.rs`) with an `impl Store` block and an
//! optional `SCHEMA_EXTRA` of idempotent SQL run at open time.

mod calendar;
pub mod calendar_time {
    pub use super::calendar::{local_date, local_to_utc};
}
pub mod mail_search {
    pub use super::mail::parse_search;
}
mod drafts;
mod mail;
mod people;
mod sync;
mod tasks;

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use chrono::{DateTime, Utc};
use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, params, params_from_iter};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::model::*;
use crate::{Error, Result};

pub use mail::MoveRecord;

const BASE_SCHEMA: &str = r#"
PRAGMA foreign_keys = ON;
CREATE TABLE IF NOT EXISTS kv (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS settings (id INTEGER PRIMARY KEY CHECK (id = 1), json TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS accounts (
    id INTEGER PRIMARY KEY, json TEXT NOT NULL, sort_order INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS folders (
    id INTEGER PRIMARY KEY, json TEXT NOT NULL,
    account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    parent_id INTEGER, role TEXT NOT NULL, sort_order INTEGER NOT NULL DEFAULT 0, remote_id TEXT);
CREATE INDEX IF NOT EXISTS folders_account ON folders(account_id);
CREATE TABLE IF NOT EXISTS messages (
    id INTEGER PRIMARY KEY, json TEXT NOT NULL,
    account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    folder_id INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
    conversation_id TEXT NOT NULL, received_at INTEGER NOT NULL,
    is_read INTEGER NOT NULL, is_draft INTEGER NOT NULL, is_pinned INTEGER NOT NULL,
    flag TEXT NOT NULL, inference TEXT NOT NULL, remote_id TEXT,
    internet_message_id TEXT, snoozed_until INTEGER);
CREATE INDEX IF NOT EXISTS messages_folder ON messages(folder_id, received_at DESC);
CREATE INDEX IF NOT EXISTS messages_conversation ON messages(conversation_id, received_at);
CREATE INDEX IF NOT EXISTS messages_remote ON messages(account_id, remote_id);
CREATE TABLE IF NOT EXISTS message_bodies (
    message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    html TEXT, text TEXT NOT NULL DEFAULT '');
CREATE TABLE IF NOT EXISTS attachments (
    id INTEGER PRIMARY KEY, json TEXT NOT NULL,
    message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    data BLOB);
CREATE INDEX IF NOT EXISTS attachments_message ON attachments(message_id);
CREATE TABLE IF NOT EXISTS categories (name TEXT PRIMARY KEY, json TEXT NOT NULL, sort_order INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS recent_addresses (
    address TEXT PRIMARY KEY, name TEXT NOT NULL, last_used INTEGER NOT NULL, use_count INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS calendars (
    id INTEGER PRIMARY KEY, json TEXT NOT NULL,
    account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    sort_order INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS events (
    id INTEGER PRIMARY KEY, json TEXT NOT NULL,
    calendar_id INTEGER NOT NULL REFERENCES calendars(id) ON DELETE CASCADE,
    start_ts INTEGER NOT NULL, end_ts INTEGER NOT NULL, is_recurring INTEGER NOT NULL, remote_id TEXT);
CREATE INDEX IF NOT EXISTS events_range ON events(start_ts, end_ts);
CREATE TABLE IF NOT EXISTS contacts (
    id INTEGER PRIMARY KEY, json TEXT NOT NULL,
    account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    display_name TEXT NOT NULL, is_favorite INTEGER NOT NULL, remote_id TEXT);
CREATE TABLE IF NOT EXISTS contact_lists (
    id INTEGER PRIMARY KEY, json TEXT NOT NULL,
    account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE);
CREATE TABLE IF NOT EXISTS task_lists (
    id INTEGER PRIMARY KEY, json TEXT NOT NULL,
    account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    sort_order INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS tasks (
    id INTEGER PRIMARY KEY, json TEXT NOT NULL,
    list_id INTEGER NOT NULL REFERENCES task_lists(id) ON DELETE CASCADE,
    is_completed INTEGER NOT NULL, is_important INTEGER NOT NULL,
    due TEXT, my_day TEXT, sort_order INTEGER NOT NULL DEFAULT 0, remote_id TEXT);
CREATE INDEX IF NOT EXISTS tasks_list ON tasks(list_id);
"#;

/// A persisted entity: one row with a JSON document plus index columns.
pub(crate) trait Doc: Serialize + DeserializeOwned {
    const TABLE: &'static str;
    /// Index columns written from [`Doc::index_values`], in the same order.
    const COLUMNS: &'static [&'static str];
    fn id(&self) -> i64;
    fn set_id(&mut self, id: i64);
    fn index_values(&self) -> Vec<Value>;
}

pub(crate) fn ts(t: DateTime<Utc>) -> Value {
    Value::Integer(t.timestamp())
}

pub(crate) fn opt_ts(t: Option<DateTime<Utc>>) -> Value {
    t.map(ts).unwrap_or(Value::Null)
}

pub(crate) fn text(s: impl Into<String>) -> Value {
    Value::Text(s.into())
}

pub(crate) fn opt_text(s: Option<impl Into<String>>) -> Value {
    s.map(|s| Value::Text(s.into())).unwrap_or(Value::Null)
}

pub(crate) fn int(i: impl Into<i64>) -> Value {
    Value::Integer(i.into())
}

pub(crate) fn boolean(b: bool) -> Value {
    Value::Integer(b as i64)
}

/// serde_json string of a unit-like enum, e.g. `FlagStatus::Flagged` -> `Flagged`.
pub(crate) fn enum_str<T: Serialize>(v: &T) -> String {
    match serde_json::to_value(v) {
        Ok(serde_json::Value::String(s)) => s,
        Ok(other) => other.to_string(),
        Err(_) => String::new(),
    }
}

pub struct Store {
    conn: Mutex<Connection>,
    path: Option<PathBuf>,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store").field("path", &self.path).finish()
    }
}

impl Store {
    /// Open (creating if needed) the database at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Error::Other(format!("create {dir:?}: {e}")))?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        Self::init(conn, Some(path.to_path_buf()))
    }

    /// In-memory database, used by tests and the preview server.
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?, None)
    }

    /// `$XDG_DATA_HOME/axentra-outlook/axentra.db` (or `~/.local/share/...`).
    pub fn default_path() -> PathBuf {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("axentra-outlook").join("axentra.db")
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    fn init(conn: Connection, path: Option<PathBuf>) -> Result<Self> {
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(BASE_SCHEMA)?;
        for extra in [
            mail::SCHEMA_EXTRA,
            drafts::SCHEMA_EXTRA,
            calendar::SCHEMA_EXTRA,
            people::SCHEMA_EXTRA,
            tasks::SCHEMA_EXTRA,
            sync::SCHEMA_EXTRA,
        ] {
            if !extra.trim().is_empty() {
                conn.execute_batch(extra)?;
            }
        }
        Ok(Self { conn: Mutex::new(conn), path })
    }

    /// Lock the connection. Never hold the guard across an `.await`.
    pub(crate) fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Run `f` inside a transaction.
    pub(crate) fn tx<T>(&self, f: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<T>) -> Result<T> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let out = f(&tx)?;
        tx.commit()?;
        Ok(out)
    }

    // ------------------------------------------------------------ doc helpers

    pub(crate) fn insert_doc_in<T: Doc>(conn: &Connection, doc: &mut T) -> Result<i64> {
        let cols = T::COLUMNS;
        let mut names = String::from("json");
        let mut marks = String::from("?1");
        for (i, c) in cols.iter().enumerate() {
            names.push_str(", ");
            names.push_str(c);
            marks.push_str(&format!(", ?{}", i + 2));
        }
        if doc.id() > 0 {
            names.push_str(", id");
            marks.push_str(&format!(", ?{}", cols.len() + 2));
        }
        let sql = format!("INSERT INTO {} ({names}) VALUES ({marks})", T::TABLE);
        let mut values = vec![Value::Text(serde_json::to_string(doc)?)];
        values.extend(doc.index_values());
        if doc.id() > 0 {
            values.push(Value::Integer(doc.id()));
        }
        conn.execute(&sql, params_from_iter(values))?;
        let id = conn.last_insert_rowid();
        if doc.id() != id {
            doc.set_id(id);
            conn.execute(
                &format!("UPDATE {} SET json = ?1 WHERE id = ?2", T::TABLE),
                params![serde_json::to_string(doc)?, id],
            )?;
        }
        Ok(id)
    }

    pub(crate) fn update_doc_in<T: Doc>(conn: &Connection, doc: &T) -> Result<()> {
        let cols = T::COLUMNS;
        let mut sets = String::from("json = ?1");
        for (i, c) in cols.iter().enumerate() {
            sets.push_str(&format!(", {c} = ?{}", i + 2));
        }
        let sql = format!("UPDATE {} SET {sets} WHERE id = ?{}", T::TABLE, cols.len() + 2);
        let mut values = vec![Value::Text(serde_json::to_string(doc)?)];
        values.extend(doc.index_values());
        values.push(Value::Integer(doc.id()));
        let n = conn.execute(&sql, params_from_iter(values))?;
        if n == 0 {
            return Err(Error::NotFound(format!("{} {}", T::TABLE, doc.id())));
        }
        Ok(())
    }

    pub(crate) fn get_doc_in<T: Doc>(conn: &Connection, id: i64) -> Result<Option<T>> {
        let json: Option<String> = conn
            .query_row(&format!("SELECT json FROM {} WHERE id = ?1", T::TABLE), [id], |r| r.get(0))
            .optional()?;
        Ok(match json {
            Some(j) => Some(serde_json::from_str(&j)?),
            None => None,
        })
    }

    /// `SELECT json FROM <table> <tail>` where `tail` holds WHERE / ORDER BY / LIMIT.
    pub(crate) fn query_docs_in<T: Doc>(
        conn: &Connection,
        tail: &str,
        params: impl rusqlite::Params,
    ) -> Result<Vec<T>> {
        let mut stmt = conn.prepare_cached(&format!("SELECT json FROM {} {tail}", T::TABLE))?;
        let rows = stmt.query_map(params, |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(serde_json::from_str(&row?)?);
        }
        Ok(out)
    }

    pub(crate) fn delete_doc_in<T: Doc>(conn: &Connection, id: i64) -> Result<()> {
        conn.execute(&format!("DELETE FROM {} WHERE id = ?1", T::TABLE), [id])?;
        Ok(())
    }

    pub(crate) fn insert_doc<T: Doc>(&self, doc: &mut T) -> Result<i64> {
        Self::insert_doc_in(&self.conn(), doc)
    }

    pub(crate) fn update_doc<T: Doc>(&self, doc: &T) -> Result<()> {
        Self::update_doc_in(&self.conn(), doc)
    }

    pub(crate) fn get_doc<T: Doc>(&self, id: i64) -> Result<Option<T>> {
        Self::get_doc_in(&self.conn(), id)
    }

    pub(crate) fn query_docs<T: Doc>(&self, tail: &str, params: impl rusqlite::Params) -> Result<Vec<T>> {
        Self::query_docs_in(&self.conn(), tail, params)
    }

    pub(crate) fn delete_doc<T: Doc>(&self, id: i64) -> Result<()> {
        Self::delete_doc_in::<T>(&self.conn(), id)
    }

    // ------------------------------------------------------------ key/value

    pub fn kv_get(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row("SELECT value FROM kv WHERE key = ?1", [key], |r| r.get(0))
            .optional()?)
    }

    pub fn kv_set(&self, key: &str, value: &str) -> Result<()> {
        self.conn().execute(
            "INSERT INTO kv (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [key, value],
        )?;
        Ok(())
    }

    // ------------------------------------------------------------ settings

    pub fn settings(&self) -> Settings {
        let json: Option<String> = self
            .conn()
            .query_row("SELECT json FROM settings WHERE id = 1", [], |r| r.get(0))
            .optional()
            .ok()
            .flatten();
        json.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default()
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        self.conn().execute(
            "INSERT INTO settings (id, json) VALUES (1, ?1) ON CONFLICT(id) DO UPDATE SET json = excluded.json",
            [serde_json::to_string(settings)?],
        )?;
        Ok(())
    }

    // ------------------------------------------------------------ accounts

    pub fn accounts(&self) -> Result<Vec<Account>> {
        self.query_docs("ORDER BY sort_order, id", [])
    }

    pub fn account(&self, id: AccountId) -> Result<Option<Account>> {
        self.get_doc(id)
    }

    /// The first account (the default "From" and owner of new items).
    pub fn primary_account(&self) -> Result<Option<Account>> {
        Ok(self.accounts()?.into_iter().next())
    }

    /// Insert an account and create its well-known folders, default calendar
    /// and default task list.
    pub fn add_account(&self, account: &mut Account) -> Result<AccountId> {
        self.tx(|tx| {
            if account.created_at.timestamp() == 0 {
                account.created_at = Utc::now();
            }
            let id = Self::insert_doc_in(tx, account)?;
            for (i, role) in [
                FolderRole::Inbox,
                FolderRole::Drafts,
                FolderRole::Sent,
                FolderRole::Deleted,
                FolderRole::Junk,
                FolderRole::Archive,
                FolderRole::Outbox,
                FolderRole::Notes,
                FolderRole::ConversationHistory,
            ]
            .into_iter()
            .enumerate()
            {
                let mut f = Folder {
                    account_id: id,
                    name: role.default_name().into(),
                    role,
                    sort_order: i as i64,
                    is_favorite: matches!(role, FolderRole::Inbox | FolderRole::Sent | FolderRole::Drafts),
                    is_expanded: true,
                    ..Default::default()
                };
                Self::insert_doc_in(tx, &mut f)?;
            }
            let mut cal = Calendar {
                account_id: id,
                name: "Calendar".into(),
                color: "#0f6cbd".into(),
                is_visible: true,
                is_default: true,
                can_edit: true,
                group: "My calendars".into(),
                ..Default::default()
            };
            Self::insert_doc_in(tx, &mut cal)?;
            let mut list = TaskList {
                account_id: id,
                name: "Tasks".into(),
                is_default: true,
                ..Default::default()
            };
            Self::insert_doc_in(tx, &mut list)?;
            Ok(id)
        })
    }

    pub fn update_account(&self, account: &Account) -> Result<()> {
        self.update_doc(account)
    }

    /// Remove an account and everything that belongs to it.
    pub fn remove_account(&self, id: AccountId) -> Result<()> {
        self.delete_doc::<Account>(id)
    }

    // ------------------------------------------------------------ categories

    pub fn categories(&self) -> Result<Vec<Category>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached("SELECT json FROM categories ORDER BY sort_order, name")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(serde_json::from_str(&row?)?);
        }
        Ok(out)
    }

    pub fn upsert_category(&self, category: &Category) -> Result<()> {
        self.conn().execute(
            "INSERT INTO categories (name, json, sort_order) VALUES (?1, ?2, (SELECT COUNT(*) FROM categories))
             ON CONFLICT(name) DO UPDATE SET json = excluded.json",
            params![category.name, serde_json::to_string(category)?],
        )?;
        Ok(())
    }

    pub fn delete_category(&self, name: &str) -> Result<()> {
        self.conn().execute("DELETE FROM categories WHERE name = ?1", [name])?;
        Ok(())
    }

    /// True when no account exists yet (first run).
    pub fn is_empty(&self) -> Result<bool> {
        let n: i64 = self.conn().query_row("SELECT COUNT(*) FROM accounts", [], |r| r.get(0))?;
        Ok(n == 0)
    }
}

// ---------------------------------------------------------------- Doc impls

impl Doc for Account {
    const TABLE: &'static str = "accounts";
    const COLUMNS: &'static [&'static str] = &["sort_order"];
    fn id(&self) -> i64 { self.id }
    fn set_id(&mut self, id: i64) { self.id = id }
    fn index_values(&self) -> Vec<Value> { vec![int(self.sort_order)] }
}

impl Doc for Folder {
    const TABLE: &'static str = "folders";
    const COLUMNS: &'static [&'static str] = &["account_id", "parent_id", "role", "sort_order", "remote_id"];
    fn id(&self) -> i64 { self.id }
    fn set_id(&mut self, id: i64) { self.id = id }
    fn index_values(&self) -> Vec<Value> {
        vec![
            int(self.account_id),
            self.parent_id.map(Value::Integer).unwrap_or(Value::Null),
            text(self.role.as_str()),
            int(self.sort_order),
            opt_text(self.remote_id.clone()),
        ]
    }
}

impl Doc for Message {
    const TABLE: &'static str = "messages";
    const COLUMNS: &'static [&'static str] = &[
        "account_id", "folder_id", "conversation_id", "received_at", "is_read", "is_draft",
        "is_pinned", "flag", "inference", "remote_id", "internet_message_id", "snoozed_until",
    ];
    fn id(&self) -> i64 { self.id }
    fn set_id(&mut self, id: i64) { self.id = id }
    fn index_values(&self) -> Vec<Value> {
        vec![
            int(self.account_id),
            int(self.folder_id),
            text(self.conversation_id.clone()),
            ts(self.received_at),
            boolean(self.is_read),
            boolean(self.is_draft),
            boolean(self.is_pinned),
            text(enum_str(&self.flag)),
            text(enum_str(&self.inference)),
            opt_text(self.remote_id.clone()),
            text(self.internet_message_id.clone()),
            opt_ts(self.snoozed_until),
        ]
    }
}

impl Doc for Attachment {
    const TABLE: &'static str = "attachments";
    const COLUMNS: &'static [&'static str] = &["message_id"];
    fn id(&self) -> i64 { self.id }
    fn set_id(&mut self, id: i64) { self.id = id }
    fn index_values(&self) -> Vec<Value> { vec![int(self.message_id)] }
}

impl Doc for Calendar {
    const TABLE: &'static str = "calendars";
    const COLUMNS: &'static [&'static str] = &["account_id", "sort_order"];
    fn id(&self) -> i64 { self.id }
    fn set_id(&mut self, id: i64) { self.id = id }
    fn index_values(&self) -> Vec<Value> { vec![int(self.account_id), int(self.sort_order)] }
}

impl Doc for Event {
    const TABLE: &'static str = "events";
    const COLUMNS: &'static [&'static str] = &["calendar_id", "start_ts", "end_ts", "is_recurring", "remote_id"];
    fn id(&self) -> i64 { self.id }
    fn set_id(&mut self, id: i64) { self.id = id }
    fn index_values(&self) -> Vec<Value> {
        vec![
            int(self.calendar_id),
            ts(self.start),
            ts(self.end),
            boolean(self.recurrence.is_some()),
            opt_text(self.remote_id.clone()),
        ]
    }
}

impl Doc for Contact {
    const TABLE: &'static str = "contacts";
    const COLUMNS: &'static [&'static str] = &["account_id", "display_name", "is_favorite", "remote_id"];
    fn id(&self) -> i64 { self.id }
    fn set_id(&mut self, id: i64) { self.id = id }
    fn index_values(&self) -> Vec<Value> {
        vec![
            int(self.account_id),
            text(self.display_name.clone()),
            boolean(self.is_favorite),
            opt_text(self.remote_id.clone()),
        ]
    }
}

impl Doc for ContactList {
    const TABLE: &'static str = "contact_lists";
    const COLUMNS: &'static [&'static str] = &["account_id"];
    fn id(&self) -> i64 { self.id }
    fn set_id(&mut self, id: i64) { self.id = id }
    fn index_values(&self) -> Vec<Value> { vec![int(self.account_id)] }
}

impl Doc for TaskList {
    const TABLE: &'static str = "task_lists";
    const COLUMNS: &'static [&'static str] = &["account_id", "sort_order"];
    fn id(&self) -> i64 { self.id }
    fn set_id(&mut self, id: i64) { self.id = id }
    fn index_values(&self) -> Vec<Value> { vec![int(self.account_id), int(self.sort_order)] }
}

impl Doc for Task {
    const TABLE: &'static str = "tasks";
    const COLUMNS: &'static [&'static str] =
        &["list_id", "is_completed", "is_important", "due", "my_day", "sort_order", "remote_id"];
    fn id(&self) -> i64 { self.id }
    fn set_id(&mut self, id: i64) { self.id = id }
    fn index_values(&self) -> Vec<Value> {
        vec![
            int(self.list_id),
            boolean(self.is_completed),
            boolean(self.is_important),
            opt_text(self.due.map(|d| d.to_string())),
            opt_text(self.my_day.map(|d| d.to_string())),
            int(self.sort_order),
            opt_text(self.remote_id.clone()),
        ]
    }
}

#[cfg(test)]
pub(crate) mod test_util {
    use super::*;

    /// Fresh in-memory store with one local account.
    pub fn store_with_account() -> (Store, AccountId) {
        let store = Store::open_in_memory().unwrap();
        let mut acct = Account {
            kind: AccountKind::Local,
            display_name: "Test User".into(),
            email: "test@axentra.example".into(),
            color: "#0f6cbd".into(),
            ..Default::default()
        };
        let id = store.add_account(&mut acct).unwrap();
        (store, id)
    }
}

#[cfg(test)]
mod tests {
    use super::test_util::store_with_account;
    use super::*;

    #[test]
    fn add_account_creates_default_folders_calendar_and_list() {
        let (store, id) = store_with_account();
        let folders = store.folders(Some(id)).unwrap();
        assert!(folders.iter().any(|f| f.role == FolderRole::Inbox && f.name == "Inbox"));
        assert!(folders.iter().any(|f| f.role == FolderRole::Deleted && f.name == "Deleted Items"));
        assert_eq!(store.calendars().unwrap().len(), 1);
        assert_eq!(store.task_lists().unwrap().len(), 1);
    }

    #[test]
    fn settings_round_trip_and_default() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.settings(), Settings::default());
        let mut s = Settings::default();
        s.theme = ThemeMode::Dark;
        s.density = Density::Compact;
        store.save_settings(&s).unwrap();
        assert_eq!(store.settings(), s);
    }

    #[test]
    fn removing_account_cascades() {
        let (store, id) = store_with_account();
        store.remove_account(id).unwrap();
        assert!(store.folders(None).unwrap().is_empty());
        assert!(store.calendars().unwrap().is_empty());
    }
}
