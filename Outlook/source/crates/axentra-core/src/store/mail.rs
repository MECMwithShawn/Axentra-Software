//! Folders and messages.
//!
//! Besides the plain folder/message documents this file keeps the mail
//! features that Outlook runs server-side but Microsoft Graph does not expose,
//! so Axentra keeps them locally for every account type (see
//! `docs/spec/mail.md` §14.17): blocked and safe senders, ignored
//! conversations, sweep rules, the Snoozed folder, restore origins, favourite
//! order and the full-text index used by search.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, NaiveTime, Utc, Weekday};
use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, params, params_from_iter};

use super::{Doc, Store, enum_str};
use crate::model::*;
use crate::{Error, Result};

pub(crate) const SCHEMA_EXTRA: &str = r#"
CREATE TABLE IF NOT EXISTS mail_origins (
    message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    folder_id INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS mail_blocked_senders (
    address TEXT PRIMARY KEY, name TEXT NOT NULL DEFAULT '', added_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS mail_safe_senders (
    address TEXT PRIMARY KEY, added_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS mail_ignored_conversations (
    conversation_id TEXT PRIMARY KEY, account_id INTEGER NOT NULL, last_activity INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS mail_sweep_rules (
    id INTEGER PRIMARY KEY, account_id INTEGER NOT NULL, folder_id INTEGER NOT NULL,
    sender TEXT NOT NULL, mode TEXT NOT NULL, target_folder_id INTEGER NOT NULL,
    created_at INTEGER NOT NULL, last_run INTEGER);
CREATE TABLE IF NOT EXISTS mail_special_folders (
    account_id INTEGER NOT NULL, kind TEXT NOT NULL, folder_id INTEGER NOT NULL,
    PRIMARY KEY (account_id, kind));
CREATE TABLE IF NOT EXISTS mail_inference_overrides (
    address TEXT PRIMARY KEY, inference TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS mail_in_reply_to ON messages(json_extract(json, '$.in_reply_to'));

CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
    subject, sender, to_rcpt, cc_rcpt, preview, body, tokenize = 'unicode61 remove_diacritics 2');
CREATE VIEW IF NOT EXISTS mail_fts_source AS
SELECT m.id AS id,
    COALESCE(json_extract(m.json, '$.subject'), '') AS subject,
    COALESCE(json_extract(m.json, '$.from.name'), '') || ' ' || COALESCE(json_extract(m.json, '$.from.address'), '') AS sender,
    COALESCE((SELECT group_concat(COALESCE(json_extract(r.value, '$.name'), '') || ' ' || COALESCE(json_extract(r.value, '$.address'), ''), ' ')
              FROM json_each(m.json, '$.to') r), '') AS to_rcpt,
    COALESCE((SELECT group_concat(COALESCE(json_extract(r.value, '$.name'), '') || ' ' || COALESCE(json_extract(r.value, '$.address'), ''), ' ')
              FROM json_each(m.json, '$.cc') r), '') || ' ' ||
    COALESCE((SELECT group_concat(COALESCE(json_extract(r.value, '$.name'), '') || ' ' || COALESCE(json_extract(r.value, '$.address'), ''), ' ')
              FROM json_each(m.json, '$.bcc') r), '') AS cc_rcpt,
    COALESCE(json_extract(m.json, '$.preview'), '') AS preview,
    COALESCE(b.text, '') AS body
FROM messages m LEFT JOIN message_bodies b ON b.message_id = m.id;
CREATE TRIGGER IF NOT EXISTS mail_fts_ai AFTER INSERT ON messages BEGIN
    INSERT INTO messages_fts (rowid, subject, sender, to_rcpt, cc_rcpt, preview, body)
    SELECT id, subject, sender, to_rcpt, cc_rcpt, preview, body FROM mail_fts_source WHERE id = NEW.id;
END;
CREATE TRIGGER IF NOT EXISTS mail_fts_au AFTER UPDATE OF json ON messages
WHEN json_extract(OLD.json, '$.subject') IS NOT json_extract(NEW.json, '$.subject')
  OR json_extract(OLD.json, '$.preview') IS NOT json_extract(NEW.json, '$.preview')
  OR json_extract(OLD.json, '$.from') IS NOT json_extract(NEW.json, '$.from')
  OR json_extract(OLD.json, '$.to') IS NOT json_extract(NEW.json, '$.to')
  OR json_extract(OLD.json, '$.cc') IS NOT json_extract(NEW.json, '$.cc')
  OR json_extract(OLD.json, '$.bcc') IS NOT json_extract(NEW.json, '$.bcc')
BEGIN
    DELETE FROM messages_fts WHERE rowid = OLD.id;
    INSERT INTO messages_fts (rowid, subject, sender, to_rcpt, cc_rcpt, preview, body)
    SELECT id, subject, sender, to_rcpt, cc_rcpt, preview, body FROM mail_fts_source WHERE id = NEW.id;
END;
CREATE TRIGGER IF NOT EXISTS mail_fts_ad AFTER DELETE ON messages BEGIN
    DELETE FROM messages_fts WHERE rowid = OLD.id;
END;
CREATE TRIGGER IF NOT EXISTS mail_fts_bi AFTER INSERT ON message_bodies BEGIN
    DELETE FROM messages_fts WHERE rowid = NEW.message_id;
    INSERT INTO messages_fts (rowid, subject, sender, to_rcpt, cc_rcpt, preview, body)
    SELECT id, subject, sender, to_rcpt, cc_rcpt, preview, body FROM mail_fts_source WHERE id = NEW.message_id;
END;
CREATE TRIGGER IF NOT EXISTS mail_fts_bu AFTER UPDATE ON message_bodies BEGIN
    DELETE FROM messages_fts WHERE rowid = NEW.message_id;
    INSERT INTO messages_fts (rowid, subject, sender, to_rcpt, cc_rcpt, preview, body)
    SELECT id, subject, sender, to_rcpt, cc_rcpt, preview, body FROM mail_fts_source WHERE id = NEW.message_id;
END;
INSERT INTO messages_fts (rowid, subject, sender, to_rcpt, cc_rcpt, preview, body)
SELECT id, subject, sender, to_rcpt, cc_rcpt, preview, body FROM mail_fts_source
WHERE id NOT IN (SELECT rowid FROM messages_fts);
"#;

/// Snoozed messages stay hidden until due, except inside the Snoozed folder itself.
const VISIBLE_NOW: &str =
    "(snoozed_until IS NULL OR snoozed_until <= ? OR folder_id IN (SELECT folder_id FROM mail_special_folders WHERE kind = 'snoozed'))";

/// Outlook stops ignoring a conversation after 14 days without activity.
const IGNORE_EXPIRY_DAYS: i64 = 14;

const FAVORITE_ORDER_KEY: &str = "mail.favorite_order";
const RECENT_FOLDERS_KEY: &str = "mail.recent_folders";

/// Sweep modes accepted by [`Store::sweep`], in the dialog's order.
const SWEEP_MODES: [&str; 4] = ["move_all", "move_all_future", "keep_latest", "older_than_10_days"];

/// Where a message was before a move, so the move can be undone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MoveRecord {
    pub message_id: MessageId,
    pub from_folder: FolderId,
}

/// A saved sweep rule: (id, folder, sender, mode, target folder).
type SweepRuleRow = (i64, FolderId, String, String, FolderId);

/// The user's latest reply to or forward of a message (list icons and the
/// reading pane's "You replied to this message on…" banner).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LastResponse {
    /// The reply or forward.
    pub message_id: MessageId,
    pub at: DateTime<Utc>,
    pub forwarded: bool,
}

fn like_pattern(term: &str) -> String {
    let escaped = term.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
    format!("%{escaped}%")
}

fn id_list(ids: &[MessageId]) -> String {
    ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",")
}

/// WHERE clauses with numbered parameters.
#[derive(Default)]
struct Where {
    clauses: Vec<String>,
    values: Vec<Value>,
}

impl Where {
    /// Each `?` in `clause` becomes the next numbered parameter.
    fn push(&mut self, clause: &str, vals: Vec<Value>) {
        let mut vals = vals.into_iter();
        let mut c = String::with_capacity(clause.len() + 8);
        for ch in clause.chars() {
            if ch == '?' {
                self.values.push(vals.next().expect("fewer values than placeholders"));
                c.push_str(&format!("?{}", self.values.len()));
            } else {
                c.push(ch);
            }
        }
        self.clauses.push(c);
    }

    fn raw(&mut self, clause: impl Into<String>) {
        self.clauses.push(clause.into());
    }
}

/// An FTS5 string for one search term: prefix match for words (Outlook's
/// "ray" finds "Raymond" but not "tray"), exact for phrases and for numbers
/// shorter than five digits. `None` when the term has nothing searchable.
fn fts_term(term: &str) -> Option<String> {
    if !term.chars().any(|c| c.is_alphanumeric()) {
        return None;
    }
    let quoted = format!("\"{}\"", term.replace('"', "\"\""));
    let short_number = term.chars().all(|c| c.is_ascii_digit()) && term.len() < 5;
    if term.contains(char::is_whitespace) || short_number { Some(quoted) } else { Some(format!("{quoted}*")) }
}

/// FTS5 column set for a search keyword, `None` for keywords that are not text searches.
fn fts_columns(key: &str) -> Option<&'static str> {
    Some(match key {
        "from" => "{sender}",
        "to" => "{to_rcpt}",
        "cc" | "bcc" => "{cc_rcpt}",
        "participants" => "{sender to_rcpt cc_rcpt}",
        "subject" => "{subject}",
        "body" | "content" => "{body preview}",
        _ => return None,
    })
}

fn truthy(term: &str) -> Option<bool> {
    match term.to_ascii_lowercase().as_str() {
        "yes" | "true" | "1" | "on" => Some(true),
        "no" | "false" | "0" | "off" => Some(false),
        _ => None,
    }
}

fn start_of_week(d: NaiveDate, week_start: Weekday) -> NaiveDate {
    d - Duration::days(((d.weekday().num_days_from_monday() + 7 - week_start.num_days_from_monday()) % 7) as i64)
}

fn first_of_month(y: i32, m: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, 1).expect("valid month")
}

fn next_month(d: NaiveDate) -> NaiveDate {
    if d.month() == 12 { first_of_month(d.year() + 1, 1) } else { first_of_month(d.year(), d.month() + 1) }
}

const WEEKDAYS: [(&str, Weekday); 7] = [
    ("monday", Weekday::Mon),
    ("tuesday", Weekday::Tue),
    ("wednesday", Weekday::Wed),
    ("thursday", Weekday::Thu),
    ("friday", Weekday::Fri),
    ("saturday", Weekday::Sat),
    ("sunday", Weekday::Sun),
];

const MONTHS: [&str; 12] = [
    "january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december",
];

/// `[start, end)` local dates for one date word or date (`today`,
/// `"last week"`, `monday`, `march`, `1/15/2026`, `2026-01-15`, `1/15`).
fn date_span(s: &str, today: NaiveDate, week_start: Weekday) -> Option<(NaiveDate, NaiveDate)> {
    let s = s.trim().trim_matches('"').trim().to_lowercase();
    let week = start_of_week(today, week_start);
    let month = first_of_month(today.year(), today.month());
    let day = Duration::days(1);
    let span = match s.as_str() {
        "today" => (today, today + day),
        "yesterday" => (today - day, today),
        "tomorrow" => (today + day, today + day * 2),
        "this week" => (week, week + Duration::days(7)),
        "last week" => (week - Duration::days(7), week),
        "next week" => (week + Duration::days(7), week + Duration::days(14)),
        "this month" => (month, next_month(month)),
        "last month" => {
            let prev = month.checked_sub_months(chrono::Months::new(1))?;
            (prev, month)
        }
        "this year" => (first_of_month(today.year(), 1), first_of_month(today.year() + 1, 1)),
        "last year" => (first_of_month(today.year() - 1, 1), first_of_month(today.year(), 1)),
        other => {
            if let Some((_, wd)) = WEEKDAYS.iter().find(|(n, _)| *n == other || n[..3] == *other) {
                let back = (today.weekday().num_days_from_monday() + 7 - wd.num_days_from_monday()) % 7;
                let d = today - Duration::days(back as i64);
                (d, d + day)
            } else if let Some(i) = MONTHS.iter().position(|n| *n == other || (other.len() == 3 && n[..3] == *other)) {
                let m = i as u32 + 1;
                let y = if m > today.month() { today.year() - 1 } else { today.year() };
                let start = first_of_month(y, m);
                (start, next_month(start))
            } else {
                let d = ["%m/%d/%Y", "%Y-%m-%d"]
                    .iter()
                    .find_map(|f| NaiveDate::parse_from_str(other, f).ok())
                    .or_else(|| {
                        // M/D/YY and M/D (current year).
                        let parts: Vec<&str> = other.split('/').collect();
                        let m = parts.first()?.parse().ok()?;
                        let dd = parts.get(1)?.parse().ok()?;
                        let y = match parts.get(2) {
                            Some(y) if y.len() <= 2 => 2000 + y.parse::<i32>().ok()?,
                            Some(y) => y.parse::<i32>().ok()?,
                            None => today.year(),
                        };
                        NaiveDate::from_ymd_opt(y, m, dd)
                    })?;
                (d, d + day)
            }
        }
    };
    Some(span)
}

/// A received:/sent:/due: expression as an optional `[start, end)` range.
/// Accepts `>`, `>=`, `<`, `<=` prefixes and `a..b` ranges.
fn parse_date_expr(expr: &str, today: NaiveDate, week_start: Weekday) -> Option<(Option<NaiveDate>, Option<NaiveDate>)> {
    let e = expr.trim();
    if let Some((a, b)) = e.split_once("..") {
        let (s, _) = date_span(a, today, week_start)?;
        let (_, en) = date_span(b, today, week_start)?;
        return Some((Some(s), Some(en)));
    }
    let (op, rest) = [">=", "<=", ">", "<", "="]
        .iter()
        .find_map(|op| e.strip_prefix(op).map(|r| (*op, r)))
        .unwrap_or(("=", e));
    let (s, en) = date_span(rest, today, week_start)?;
    Some(match op {
        ">" => (Some(en), None),
        ">=" => (Some(s), None),
        "<" => (None, Some(s)),
        "<=" => (None, Some(en)),
        _ => (Some(s), Some(en)),
    })
}

fn local_midnight_ts(d: NaiveDate) -> i64 {
    crate::local_to_utc(d, NaiveTime::MIN).timestamp()
}

/// Folder roles that hold items "on their way out"; moving out of them does
/// not overwrite the remembered original folder.
fn is_holding_role(role: &str) -> bool {
    matches!(role, "deleted" | "junk")
}

fn role_folder_in(conn: &Connection, account_id: AccountId, role: FolderRole) -> Result<Option<FolderId>> {
    Ok(conn
        .query_row(
            "SELECT id FROM folders WHERE account_id = ?1 AND role = ?2 ORDER BY id LIMIT 1",
            params![account_id, role.as_str()],
            |r| r.get(0),
        )
        .optional()?)
}

fn folder_role_in(conn: &Connection, folder_id: FolderId) -> Result<Option<String>> {
    Ok(conn.query_row("SELECT role FROM folders WHERE id = ?1", [folder_id], |r| r.get(0)).optional()?)
}

/// Remember where messages came from, unless they came from Deleted Items or Junk
/// (then the earlier origin stays).
fn record_origins_in(conn: &Connection, records: &[MoveRecord]) -> Result<()> {
    for r in records {
        let role = folder_role_in(conn, r.from_folder)?.unwrap_or_default();
        if is_holding_role(&role) {
            continue;
        }
        conn.execute(
            "INSERT INTO mail_origins (message_id, folder_id) VALUES (?1, ?2)
             ON CONFLICT(message_id) DO UPDATE SET folder_id = excluded.folder_id",
            params![r.message_id, r.from_folder],
        )?;
    }
    Ok(())
}

/// Group messages (already sorted) by conversation, keeping the order in
/// which each conversation first appears.
fn group_by_conversation(msgs: Vec<Message>) -> Vec<Vec<Message>> {
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut groups: Vec<Vec<Message>> = Vec::new();
    for m in msgs {
        match index.get(&m.conversation_id) {
            Some(&i) => groups[i].push(m),
            None => {
                index.insert(m.conversation_id.clone(), groups.len());
                groups.push(vec![m]);
            }
        }
    }
    groups
}

impl Store {
    // ------------------------------------------------------------ folders

    /// Folders of one account (or all accounts), with unread/total counts,
    /// ordered by account then sort order.
    pub fn folders(&self, account_id: Option<AccountId>) -> Result<Vec<Folder>> {
        let conn = self.conn();
        let mut folders: Vec<Folder> = match account_id {
            Some(a) => Self::query_docs_in(&conn, "WHERE account_id = ?1 ORDER BY sort_order, id", [a])?,
            None => Self::query_docs_in(&conn, "ORDER BY account_id, sort_order, id", [])?,
        };
        let now = Utc::now().timestamp();
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT folder_id, COUNT(*), SUM(CASE WHEN is_read = 0 THEN 1 ELSE 0 END)
             FROM messages WHERE {} GROUP BY folder_id",
            VISIBLE_NOW.replace('?', "?1")
        ))?;
        let counts: HashMap<FolderId, (i64, i64)> = stmt
            .query_map([now], |r| Ok((r.get::<_, i64>(0)?, (r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))))?
            .collect::<std::result::Result<_, _>>()?;
        for f in &mut folders {
            let (total, unread) = counts.get(&f.id).copied().unwrap_or((0, 0));
            f.total_count = total;
            f.unread_count = unread;
        }
        Ok(folders)
    }

    pub fn folder(&self, id: FolderId) -> Result<Option<Folder>> {
        Ok(self.folders(None)?.into_iter().find(|f| f.id == id))
    }

    pub fn folder_by_role(&self, account_id: AccountId, role: FolderRole) -> Result<Option<Folder>> {
        Ok(self.folders(Some(account_id))?.into_iter().find(|f| f.role == role))
    }

    fn folder_id_by_role(&self, account_id: AccountId, role: FolderRole) -> Result<FolderId> {
        role_folder_in(&self.conn(), account_id, role)?
            .ok_or_else(|| Error::NotFound(format!("{} folder for account {account_id}", role.as_str())))
    }

    fn favorite_order(&self) -> Vec<FolderId> {
        self.kv_get(FAVORITE_ORDER_KEY).ok().flatten().and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default()
    }

    fn save_favorite_order(&self, order: &[FolderId]) -> Result<()> {
        self.kv_set(FAVORITE_ORDER_KEY, &serde_json::to_string(order)?)
    }

    /// Favourite folders across accounts, in the user's Favorites order
    /// (folder order for favourites that were never reordered).
    pub fn favorites(&self) -> Result<Vec<Folder>> {
        let mut favs: Vec<Folder> = self.folders(None)?.into_iter().filter(|f| f.is_favorite).collect();
        let order = self.favorite_order();
        favs.sort_by_key(|f| order.iter().position(|o| *o == f.id).unwrap_or(usize::MAX));
        Ok(favs)
    }

    /// Move a favourite up (`delta < 0`) or down in the Favorites section.
    pub fn move_favorite(&self, id: FolderId, delta: i32) -> Result<()> {
        let mut ids: Vec<FolderId> = self.favorites()?.into_iter().map(|f| f.id).collect();
        let Some(pos) = ids.iter().position(|f| *f == id) else {
            return Err(Error::NotFound(format!("favorite {id}")));
        };
        let target = (pos as i64 + delta as i64).clamp(0, ids.len() as i64 - 1) as usize;
        let item = ids.remove(pos);
        ids.insert(target, item);
        self.save_favorite_order(&ids)
    }

    pub fn create_folder(&self, account_id: AccountId, parent_id: Option<FolderId>, name: &str) -> Result<FolderId> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::Invalid("folder name is empty".into()));
        }
        let siblings = self.folders(Some(account_id))?;
        if siblings.iter().any(|f| f.parent_id == parent_id && f.name.eq_ignore_ascii_case(name)) {
            return Err(Error::Invalid(format!("a folder named \"{name}\" already exists here")));
        }
        let max = siblings.iter().map(|f| f.sort_order).max().unwrap_or(0);
        let mut folder = Folder {
            account_id,
            parent_id,
            name: name.into(),
            role: FolderRole::Custom,
            sort_order: max + 1,
            is_expanded: true,
            ..Default::default()
        };
        self.insert_doc(&mut folder)
    }

    /// Make a custom folder the account's well-known folder for `role`
    /// ("Choose an existing folder…" in Set up Archive). A folder that held
    /// the role before becomes a custom folder.
    pub fn set_folder_role(&self, id: FolderId, role: FolderRole) -> Result<()> {
        let folder: Folder = self.get_doc(id)?.ok_or_else(|| Error::NotFound(format!("folder {id}")))?;
        if folder.role != FolderRole::Custom && folder.role != role {
            return Err(Error::Invalid("well-known folders keep their role".into()));
        }
        let previous = if role == FolderRole::Custom { None } else { role_folder_in(&self.conn(), folder.account_id, role)? };
        if let Some(old) = previous.filter(|old| *old != id) {
            self.with_folder(old, |f| f.role = FolderRole::Custom)?;
        }
        self.with_folder(id, |f| f.role = role)
    }

    /// The account's folder for `role`, created at the top level (with the
    /// role's default name, or taking over a custom folder of that name)
    /// when the account has none ("Set up Archive").
    pub fn ensure_role_folder(&self, account_id: AccountId, role: FolderRole) -> Result<FolderId> {
        let current = role_folder_in(&self.conn(), account_id, role)?;
        if let Some(id) = current {
            return Ok(id);
        }
        let name = role.default_name();
        let existing = self
            .folders(Some(account_id))?
            .into_iter()
            .find(|f| f.parent_id.is_none() && f.role == FolderRole::Custom && f.name.eq_ignore_ascii_case(name));
        let id = match existing {
            Some(f) => f.id,
            None => self.create_folder(account_id, None, name)?,
        };
        self.set_folder_role(id, role)?;
        Ok(id)
    }

    fn with_folder(&self, id: FolderId, f: impl FnOnce(&mut Folder)) -> Result<()> {
        let mut folder: Folder = self.get_doc(id)?.ok_or_else(|| Error::NotFound(format!("folder {id}")))?;
        f(&mut folder);
        self.update_doc(&folder)
    }

    pub fn rename_folder(&self, id: FolderId, name: &str) -> Result<()> {
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err(Error::Invalid("folder name is empty".into()));
        }
        let folder: Folder = self.get_doc(id)?.ok_or_else(|| Error::NotFound(format!("folder {id}")))?;
        let siblings = self.folders(Some(folder.account_id))?;
        if siblings.iter().any(|f| f.id != id && f.parent_id == folder.parent_id && f.name.eq_ignore_ascii_case(&name)) {
            return Err(Error::Invalid(format!("a folder named \"{name}\" already exists here")));
        }
        self.with_folder(id, |f| f.name = name)
    }

    pub fn set_folder_favorite(&self, id: FolderId, favorite: bool) -> Result<()> {
        let mut order: Vec<FolderId> = self.favorites()?.into_iter().map(|f| f.id).filter(|f| *f != id).collect();
        self.with_folder(id, |f| f.is_favorite = favorite)?;
        if favorite {
            order.push(id);
        }
        self.save_favorite_order(&order)
    }

    pub fn set_folder_expanded(&self, id: FolderId, expanded: bool) -> Result<()> {
        self.with_folder(id, |f| f.is_expanded = expanded)
    }

    /// True when `folder` is `ancestor` or sits somewhere below it.
    pub fn folder_is_within(&self, folder: FolderId, ancestor: FolderId) -> Result<bool> {
        let all = self.folders(None)?;
        let parent: HashMap<FolderId, Option<FolderId>> = all.iter().map(|f| (f.id, f.parent_id)).collect();
        let mut cur = Some(folder);
        let mut guard = 0;
        while let Some(c) = cur {
            if c == ancestor {
                return Ok(true);
            }
            cur = parent.get(&c).copied().flatten();
            guard += 1;
            if guard > all.len() {
                break;
            }
        }
        Ok(false)
    }

    /// Move a folder under a new parent (`None` = top level of its account).
    /// A folder can't move into itself, into its own subfolders, or into
    /// another account.
    pub fn move_folder(&self, id: FolderId, new_parent: Option<FolderId>) -> Result<()> {
        if let Some(p) = new_parent {
            if self.folder_is_within(p, id)? {
                return Err(Error::Invalid("cannot move a folder into itself or one of its subfolders".into()));
            }
            let folder: Folder = self.get_doc(id)?.ok_or_else(|| Error::NotFound(format!("folder {id}")))?;
            let parent: Folder = self.get_doc(p)?.ok_or_else(|| Error::NotFound(format!("folder {p}")))?;
            if parent.account_id != folder.account_id {
                return Err(Error::Invalid("folders can't move between accounts".into()));
            }
        }
        self.with_folder(id, |f| f.parent_id = new_parent)
    }

    /// Delete a custom folder. Like Outlook, a folder outside Deleted Items is
    /// moved into Deleted Items; a folder already inside it is removed with its
    /// messages.
    pub fn delete_folder(&self, id: FolderId) -> Result<()> {
        let folder: Folder = self.get_doc(id)?.ok_or_else(|| Error::NotFound(format!("folder {id}")))?;
        if folder.role != FolderRole::Custom {
            return Err(Error::Invalid("well-known folders cannot be deleted".into()));
        }
        let deleted = self.folder_id_by_role(folder.account_id, FolderRole::Deleted)?;
        let all = self.folders(Some(folder.account_id))?;
        let mut cur = folder.parent_id;
        let mut inside_deleted = false;
        while let Some(p) = cur {
            if p == deleted {
                inside_deleted = true;
                break;
            }
            cur = all.iter().find(|f| f.id == p).and_then(|f| f.parent_id);
        }
        if inside_deleted {
            // Remove the subtree.
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
        } else {
            self.move_folder(id, Some(deleted))
        }
    }

    /// Folders most recently used as a Move target, newest first.
    pub fn recent_folders(&self, limit: usize) -> Result<Vec<Folder>> {
        let ids: Vec<FolderId> =
            self.kv_get(RECENT_FOLDERS_KEY)?.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default();
        let all: HashMap<FolderId, Folder> = self.folders(None)?.into_iter().map(|f| (f.id, f)).collect();
        Ok(ids.into_iter().filter_map(|id| all.get(&id).cloned()).take(limit).collect())
    }

    /// Remember a Move target for the "recent folders" part of Move menus.
    pub fn record_recent_folder(&self, id: FolderId) -> Result<()> {
        let mut ids: Vec<FolderId> =
            self.kv_get(RECENT_FOLDERS_KEY)?.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default();
        ids.retain(|f| *f != id);
        ids.insert(0, id);
        ids.truncate(10);
        self.kv_set(RECENT_FOLDERS_KEY, &serde_json::to_string(&ids)?)
    }

    // ------------------------------------------------------------ messages

    /// Insert a message (and optionally its body). Fills in `id`.
    ///
    /// New mail for the Inbox goes through the local filters first: senders
    /// with a block rule land in Junk Email, ignored conversations in Deleted Items, and
    /// "Always move to Focused/Other" overrides set the inference class.
    pub fn insert_message(&self, message: &mut Message, body: Option<&MessageBody>) -> Result<MessageId> {
        self.tx(|tx| {
            if message.conversation_id.is_empty() {
                message.conversation_id = format!("local-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0));
            }
            let rule = Self::route_incoming_in(tx, message)?;
            let id = Self::insert_doc_in(tx, message)?;
            if let Some(rule) = rule {
                super::junk::record_routing_in(tx, id, &rule)?;
            }
            if let Some(b) = body {
                tx.execute(
                    "INSERT OR REPLACE INTO message_bodies (message_id, html, text) VALUES (?1, ?2, ?3)",
                    params![id, b.html, b.text],
                )?;
                for a in &b.attachments {
                    let mut a = a.clone();
                    a.id = 0;
                    a.message_id = id;
                    Self::insert_doc_in(tx, &mut a)?;
                }
            }
            Ok(id)
        })
    }

    /// Local filters for new Inbox mail (also used for synced mail). Returns
    /// the block rule when one sent the message to Junk Email.
    pub(crate) fn route_incoming_in(conn: &Connection, m: &mut Message) -> Result<Option<crate::junk::SenderRule>> {
        if m.is_draft || folder_role_in(conn, m.folder_id)?.as_deref() != Some("inbox") {
            return Ok(None);
        }
        let me: Option<String> = conn
            .query_row("SELECT lower(json_extract(json, '$.email')) FROM accounts WHERE id = ?1", [m.account_id], |r| r.get(0))
            .optional()?
            .flatten();
        let from = m.from.address.trim().to_lowercase();
        if from.is_empty() || me.as_deref() == Some(from.as_str()) {
            return Ok(None);
        }
        let class: Option<String> = conn
            .query_row("SELECT inference FROM mail_inference_overrides WHERE address = ?1", [&from], |r| r.get(0))
            .optional()?;
        match class.as_deref() {
            Some("Other") => m.inference = InferenceClass::Other,
            Some("Focused") => m.inference = InferenceClass::Focused,
            _ => {}
        }
        // A block rule applies to mail that arrives after blocking took
        // effect (a day of slack for clocks), so a first sync doesn't sweep
        // old mail. An arrival time more than a day ahead isn't a server's
        // (it came from the sender's Date header): treated as unknown, so the
        // rule doesn't move it.
        let now = Utc::now().timestamp();
        let arrived = m.received_at.timestamp();
        if let Some(rule) = super::junk::decide_in(conn, m.account_id, &from)?
            && rule.action() == crate::junk::RuleAction::Block
            && arrived >= rule.effective_since() - 86_400
            && arrived <= now + 86_400
        {
            if let Some(junk) = role_folder_in(conn, m.account_id, FolderRole::Junk)? {
                m.folder_id = junk;
                return Ok(Some(rule));
            }
            return Ok(None);
        }
        let last: Option<i64> = conn
            .query_row(
                "SELECT last_activity FROM mail_ignored_conversations WHERE conversation_id = ?1",
                [&m.conversation_id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(last) = last
            && now - last <= IGNORE_EXPIRY_DAYS * 86_400
            && let Some(deleted) = role_folder_in(conn, m.account_id, FolderRole::Deleted)?
        {
            m.folder_id = deleted;
            conn.execute(
                "UPDATE mail_ignored_conversations SET last_activity = ?1 WHERE conversation_id = ?2",
                params![now, m.conversation_id],
            )?;
        }
        Ok(None)
    }

    pub fn update_message(&self, message: &Message) -> Result<()> {
        self.update_doc(message)
    }

    pub fn set_message_body(&self, message_id: MessageId, html: Option<&str>, text: &str) -> Result<()> {
        self.conn().execute(
            "INSERT OR REPLACE INTO message_bodies (message_id, html, text) VALUES (?1, ?2, ?3)",
            params![message_id, html, text],
        )?;
        Ok(())
    }

    pub fn message(&self, id: MessageId) -> Result<Option<Message>> {
        self.get_doc(id)
    }

    /// Several messages by id (any order; missing ids are skipped).
    pub fn messages_by_ids(&self, ids: &[MessageId]) -> Result<Vec<Message>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        self.query_docs(&format!("WHERE id IN ({}) ORDER BY received_at DESC, id DESC", id_list(ids)), [])
    }

    pub fn message_by_remote_id(&self, account_id: AccountId, remote_id: &str) -> Result<Option<Message>> {
        Ok(self
            .query_docs::<Message>("WHERE account_id = ?1 AND remote_id = ?2 LIMIT 1", params![account_id, remote_id])?
            .into_iter()
            .next())
    }

    pub fn message_body(&self, id: MessageId) -> Result<Option<MessageBody>> {
        let conn = self.conn();
        let row: Option<(Option<String>, String)> = conn
            .query_row("SELECT html, text FROM message_bodies WHERE message_id = ?1", [id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        let Some((html, text)) = row else { return Ok(None) };
        let attachments = Self::query_docs_in::<Attachment>(&conn, "WHERE message_id = ?1 ORDER BY id", [id])?;
        Ok(Some(MessageBody { message_id: id, html, text, attachments }))
    }

    /// Attachments of several messages at once (for list attachment previews).
    pub fn attachments_for(&self, ids: &[MessageId]) -> Result<HashMap<MessageId, Vec<Attachment>>> {
        let mut out: HashMap<MessageId, Vec<Attachment>> = HashMap::new();
        if ids.is_empty() {
            return Ok(out);
        }
        for a in self.query_docs::<Attachment>(&format!("WHERE message_id IN ({}) ORDER BY id", id_list(ids)), [])? {
            out.entry(a.message_id).or_default().push(a);
        }
        Ok(out)
    }

    pub fn insert_attachment(&self, attachment: &mut Attachment, data: Option<&[u8]>) -> Result<AttachmentId> {
        let conn = self.conn();
        let id = Self::insert_doc_in(&conn, attachment)?;
        if let Some(d) = data {
            conn.execute("UPDATE attachments SET data = ?1 WHERE id = ?2", params![d, id])?;
        }
        Ok(id)
    }

    pub fn attachment_data(&self, id: AttachmentId) -> Result<Option<Vec<u8>>> {
        Ok(self
            .conn()
            .query_row("SELECT data FROM attachments WHERE id = ?1", [id], |r| r.get::<_, Option<Vec<u8>>>(0))
            .optional()?
            .flatten())
    }

    /// Which of these attachments have their content stored locally (without
    /// loading the content).
    pub fn downloaded_attachments(&self, ids: &[AttachmentId]) -> Result<HashSet<AttachmentId>> {
        if ids.is_empty() {
            return Ok(HashSet::new());
        }
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!("SELECT id FROM attachments WHERE id IN ({}) AND data IS NOT NULL", id_list(ids)))?;
        let rows = stmt.query_map([], |r| r.get::<_, AttachmentId>(0))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Store downloaded content for an attachment.
    pub fn set_attachment_data(&self, id: AttachmentId, data: &[u8]) -> Result<()> {
        let n = self.conn().execute("UPDATE attachments SET data = ?1 WHERE id = ?2", params![data, id])?;
        if n == 0 {
            return Err(Error::NotFound(format!("attachment {id}")));
        }
        Ok(())
    }

    /// Messages matching `q`. Pinned messages sort first when sorting by date
    /// (newest). Snoozed messages are hidden until their snooze time, except
    /// in the Snoozed folder.
    ///
    /// Search uses the full-text index with Outlook's matching rules (word
    /// prefixes, phrases in quotes, `AND` / `OR` / `NOT`) and the keywords of
    /// `docs/spec/mail.md` §7.4: `from:`, `to:`, `cc:`, `bcc:`,
    /// `participants:`, `subject:`, `body:`, `hasattachment(s):`, `is:`,
    /// `read:`, `hasflag:` / `isflagged:` / `followupflag:`, `category:`,
    /// `importance:`, `received:` / `sent:` and `due:`.
    pub fn messages(&self, q: &MessageQuery) -> Result<Vec<Message>> {
        let mut w = Where::default();
        if let Some(folder) = q.folder_id {
            w.push("folder_id = ?", vec![Value::Integer(folder)]);
        }
        if let Some(class) = q.inference {
            w.push("inference = ?", vec![Value::Text(enum_str(&class))]);
        }
        w.push(VISIBLE_NOW, vec![Value::Integer(Utc::now().timestamp())]);
        match q.filter {
            MessageFilter::All => {}
            MessageFilter::Unread => w.raw("is_read = 0"),
            MessageFilter::Flagged => w.raw("flag = 'Flagged'"),
            MessageFilter::Pinned => w.raw("is_pinned = 1"),
            MessageFilter::HasFiles => w.raw(
                "EXISTS (SELECT 1 FROM attachments a WHERE a.message_id = messages.id AND json_extract(a.json, '$.is_inline') = 0)
                 OR (json_extract(json, '$.has_attachments') = 1 AND NOT EXISTS (SELECT 1 FROM attachments a WHERE a.message_id = messages.id))",
            ),
            MessageFilter::Mentions => w.raw("json_extract(json, '$.mentions_me') = 1"),
            MessageFilter::ToMe => w.raw(
                "EXISTS (SELECT 1 FROM accounts a, json_each(json_extract(messages.json, '$.to')) je
                  WHERE a.id = messages.account_id
                  AND lower(json_extract(je.value, '$.address')) = lower(json_extract(a.json, '$.email')))
                 OR EXISTS (SELECT 1 FROM accounts a, json_each(json_extract(messages.json, '$.cc')) je
                  WHERE a.id = messages.account_id
                  AND lower(json_extract(je.value, '$.address')) = lower(json_extract(a.json, '$.email')))",
            ),
        }
        if let Some(search) = q.search.as_deref()
            && !self.push_search(&mut w, search)?
        {
            return Ok(Vec::new());
        }
        let order = match q.sort {
            MessageSort::DateNewest => "is_pinned DESC, received_at DESC, id DESC",
            MessageSort::DateOldest => "received_at ASC, id ASC",
            MessageSort::From => "COALESCE(NULLIF(json_extract(json,'$.from.name'), ''), json_extract(json,'$.from.address')) COLLATE NOCASE, received_at DESC",
            MessageSort::Subject => "json_extract(json,'$.subject') COLLATE NOCASE, received_at DESC",
            MessageSort::Size => {
                "(COALESCE((SELECT SUM(json_extract(a.json,'$.size')) FROM attachments a WHERE a.message_id = messages.id), 0)
                  + COALESCE((SELECT LENGTH(COALESCE(b.html, b.text)) FROM message_bodies b WHERE b.message_id = messages.id), 0)) DESC,
                 received_at DESC"
            }
            MessageSort::Importance => {
                "CASE json_extract(json,'$.importance') WHEN 'High' THEN 0 WHEN 'Normal' THEN 1 ELSE 2 END, received_at DESC"
            }
            MessageSort::Flag => "CASE flag WHEN 'Flagged' THEN 0 WHEN 'Complete' THEN 1 ELSE 2 END, received_at DESC",
        };
        let mut tail = String::new();
        if !w.clauses.is_empty() {
            tail.push_str("WHERE ");
            tail.push_str(&w.clauses.iter().map(|c| format!("({c})")).collect::<Vec<_>>().join(" AND "));
        }
        tail.push_str(" ORDER BY ");
        tail.push_str(order);
        if let Some(limit) = q.limit {
            tail.push_str(&format!(" LIMIT {limit}"));
        }
        self.query_docs(&tail, params_from_iter(w.values))
    }

    /// Add the clauses for a search string. Returns false when the search
    /// has nothing that can match (e.g. only punctuation).
    fn push_search(&self, w: &mut Where, search: &str) -> Result<bool> {
        let today = Local::now().date_naive();
        let week_start = self.settings().week_start;
        let mut fts: Vec<(&'static str, String)> = Vec::new();
        let mut pending_op: &'static str = "AND";
        let mut criteria = 0usize;
        for (key, term) in parse_search(search) {
            if key.is_none() && matches!(term.as_str(), "AND" | "OR" | "NOT") {
                pending_op = match term.as_str() {
                    "OR" => "OR",
                    "NOT" => "NOT",
                    _ => "AND",
                };
                continue;
            }
            let op = std::mem::replace(&mut pending_op, "AND");
            let key = key.as_deref();
            if let Some(cols) = key.and_then(fts_columns) {
                if let Some(t) = fts_term(&term) {
                    fts.push((op, format!("{cols} : {t}")));
                }
                continue;
            }
            match key {
                Some("hasattachment" | "hasattachments" | "hasattach" | "attachments") => {
                    let yes = truthy(&term).unwrap_or(true);
                    w.raw(format!("json_extract(json, '$.has_attachments') = {}", yes as i32));
                    criteria += 1;
                }
                Some("is") => {
                    let clause = match term.to_ascii_lowercase().as_str() {
                        "unread" => "is_read = 0",
                        "read" => "is_read = 1",
                        "flagged" => "flag = 'Flagged'",
                        "pinned" => "is_pinned = 1",
                        "draft" => "is_draft = 1",
                        _ => continue,
                    };
                    w.raw(clause);
                    criteria += 1;
                }
                Some("read") => {
                    let Some(yes) = truthy(&term) else { continue };
                    w.raw(format!("is_read = {}", yes as i32));
                    criteria += 1;
                }
                Some("hasflag" | "isflagged" | "followupflag" | "flag") => {
                    let t = term.to_ascii_lowercase();
                    let clause = if t == "complete" || t == "completed" {
                        "flag = 'Complete'"
                    } else if truthy(&t) == Some(false) {
                        "flag <> 'Flagged'"
                    } else {
                        "flag = 'Flagged'"
                    };
                    w.raw(clause);
                    criteria += 1;
                }
                Some("category") => {
                    w.push("json_extract(json,'$.categories') LIKE ? ESCAPE '\\'", vec![Value::Text(like_pattern(&term))]);
                    criteria += 1;
                }
                Some("importance") => {
                    let v = match term.to_ascii_lowercase().as_str() {
                        "high" => "High",
                        "low" => "Low",
                        "normal" => "Normal",
                        _ => continue,
                    };
                    w.push("json_extract(json,'$.importance') = ?", vec![Value::Text(v.into())]);
                    criteria += 1;
                }
                Some("received" | "sent" | "date") => {
                    let Some((start, end)) = parse_date_expr(&term, today, week_start) else { continue };
                    if let Some(s) = start {
                        w.push("received_at >= ?", vec![Value::Integer(local_midnight_ts(s))]);
                    }
                    if let Some(e) = end {
                        w.push("received_at < ?", vec![Value::Integer(local_midnight_ts(e))]);
                    }
                    criteria += 1;
                }
                Some("due") => {
                    let Some((start, end)) = parse_date_expr(&term, today, week_start) else { continue };
                    w.raw("flag = 'Flagged' AND json_extract(json, '$.flag_due') IS NOT NULL");
                    if let Some(s) = start {
                        w.push("json_extract(json, '$.flag_due') >= ?", vec![Value::Text(s.to_string())]);
                    }
                    if let Some(e) = end {
                        w.push("json_extract(json, '$.flag_due') < ?", vec![Value::Text(e.to_string())]);
                    }
                    criteria += 1;
                }
                // Free text (and unknown keywords, searched as text).
                _ => {
                    if let Some(t) = fts_term(&term) {
                        fts.push((op, t));
                    }
                }
            }
        }
        if !fts.is_empty() {
            // A leading NOT has no left operand in FTS5: exclude it separately.
            let mut rest = fts.as_slice();
            while let Some(((op, expr), tail)) = rest.split_first() {
                if *op != "NOT" {
                    break;
                }
                w.push("id NOT IN (SELECT rowid FROM messages_fts WHERE messages_fts MATCH ?)", vec![Value::Text(expr.clone())]);
                criteria += 1;
                rest = tail;
            }
            if let Some(((_, first), tail)) = rest.split_first() {
                let mut expr = first.clone();
                for (op, e) in tail {
                    expr.push_str(&format!(" {op} {e}"));
                }
                w.push("id IN (SELECT rowid FROM messages_fts WHERE messages_fts MATCH ?)", vec![Value::Text(expr)]);
                criteria += 1;
            }
        }
        Ok(criteria > 0)
    }

    /// Messages matching `q` grouped by conversation: one inner list per
    /// conversation, in list order (the order of each conversation's first
    /// message in `q`'s sort), messages in sort order within it.
    pub fn conversation_groups(&self, q: &MessageQuery) -> Result<Vec<Vec<Message>>> {
        Ok(group_by_conversation(self.messages(q)?))
    }

    /// Number of messages in each conversation across folders, not counting
    /// Deleted Items and Junk Email.
    pub fn conversation_sizes(&self, conversation_ids: &[String]) -> Result<HashMap<String, i64>> {
        let mut out = HashMap::new();
        if conversation_ids.is_empty() {
            return Ok(out);
        }
        let marks = (1..=conversation_ids.len()).map(|i| format!("?{i}")).collect::<Vec<_>>().join(",");
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT conversation_id, COUNT(*) FROM messages
             WHERE conversation_id IN ({marks})
             AND folder_id NOT IN (SELECT id FROM folders WHERE role IN ('deleted', 'junk'))
             GROUP BY conversation_id"
        ))?;
        let rows = stmt.query_map(params_from_iter(conversation_ids.iter()), |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        for row in rows {
            let (k, v) = row?;
            out.insert(k, v);
        }
        Ok(out)
    }

    /// All messages of a conversation across folders, oldest first.
    pub fn conversation(&self, conversation_id: &str) -> Result<Vec<Message>> {
        self.query_docs("WHERE conversation_id = ?1 ORDER BY received_at ASC, id ASC", [conversation_id])
    }

    /// Several conversations in one query: conversation id to its messages
    /// across folders, oldest first. Unknown ids are left out.
    pub fn conversations(&self, conversation_ids: &[String]) -> Result<HashMap<String, Vec<Message>>> {
        let mut out: HashMap<String, Vec<Message>> = HashMap::new();
        for chunk in conversation_ids.chunks(500) {
            let marks = (1..=chunk.len()).map(|i| format!("?{i}")).collect::<Vec<_>>().join(",");
            let msgs: Vec<Message> = self.query_docs(
                &format!("WHERE conversation_id IN ({marks}) ORDER BY received_at ASC, id ASC"),
                params_from_iter(chunk.iter()),
            )?;
            for m in msgs {
                out.entry(m.conversation_id.clone()).or_default().push(m);
            }
        }
        Ok(out)
    }

    /// The user's latest reply or forward for each of `ids`: a sent (not
    /// draft) message from one of the user's accounts whose `In-Reply-To` is
    /// the message's Message-ID. Subjects starting "FW:" / "Fwd:" count as
    /// forwards. Messages nobody responded to are left out.
    pub fn last_responses(&self, ids: &[MessageId]) -> Result<HashMap<MessageId, LastResponse>> {
        let mut out = HashMap::new();
        if ids.is_empty() {
            return Ok(out);
        }
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT m.id, r.id, r.received_at, COALESCE(json_extract(r.json, '$.subject'), '')
             FROM messages m JOIN messages r ON json_extract(r.json, '$.in_reply_to') = m.internet_message_id
             WHERE m.id IN ({}) AND m.internet_message_id <> '' AND r.is_draft = 0
               AND lower(json_extract(r.json, '$.from.address')) IN (SELECT lower(json_extract(json, '$.email')) FROM accounts)
             ORDER BY r.received_at ASC, r.id ASC",
            id_list(ids)
        ))?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, MessageId>(0)?, r.get::<_, MessageId>(1)?, r.get::<_, i64>(2)?, r.get::<_, String>(3)?))
        })?;
        for row in rows {
            let (id, reply, at, subject) = row?;
            let lower = subject.trim_start().to_ascii_lowercase();
            out.insert(
                id,
                LastResponse {
                    message_id: reply,
                    at: DateTime::from_timestamp(at, 0).unwrap_or_default(),
                    forwarded: lower.starts_with("fw:") || lower.starts_with("fwd:"),
                },
            );
        }
        Ok(out)
    }

    /// Flagged messages across all folders, newest first (To Do "Flagged email").
    pub fn flagged_messages(&self) -> Result<Vec<Message>> {
        self.query_docs("WHERE flag = 'Flagged' ORDER BY received_at DESC", [])
    }

    fn update_many(&self, ids: &[MessageId], f: impl Fn(&mut Message)) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        self.tx(|tx| {
            let msgs: Vec<Message> = Self::query_docs_in(tx, &format!("WHERE id IN ({})", id_list(ids)), [])?;
            for mut m in msgs {
                f(&mut m);
                Self::update_doc_in(tx, &m)?;
            }
            Ok(())
        })
    }

    pub fn set_read(&self, ids: &[MessageId], read: bool) -> Result<()> {
        self.update_many(ids, |m| m.is_read = read)
    }

    /// Mark every message in a folder read.
    pub fn mark_folder_read(&self, folder_id: FolderId) -> Result<()> {
        let ids: Vec<MessageId> = self
            .query_docs::<Message>("WHERE folder_id = ?1 AND is_read = 0", [folder_id])?
            .into_iter()
            .map(|m| m.id)
            .collect();
        self.set_read(&ids, true)
    }

    /// Mark every message of an account read (account header "Mark all as read").
    pub fn mark_account_read(&self, account_id: AccountId) -> Result<()> {
        let ids: Vec<MessageId> = self
            .query_docs::<Message>("WHERE account_id = ?1 AND is_read = 0", [account_id])?
            .into_iter()
            .map(|m| m.id)
            .collect();
        self.set_read(&ids, true)
    }

    /// Set the follow-up flag. `due` is kept only for `Flagged`.
    pub fn set_flag(&self, ids: &[MessageId], status: FlagStatus, due: Option<NaiveDate>) -> Result<()> {
        self.update_many(ids, |m| {
            m.flag = status;
            m.flag_due = if status == FlagStatus::Flagged { due } else { None };
        })
    }

    pub fn set_pinned(&self, ids: &[MessageId], pinned: bool) -> Result<()> {
        self.update_many(ids, |m| m.is_pinned = pinned)
    }

    pub fn set_categories(&self, ids: &[MessageId], categories: &[String]) -> Result<()> {
        self.update_many(ids, |m| m.categories = categories.to_vec())
    }

    /// Add the category to every message lacking it, or remove it from all if
    /// every message already has it (Outlook's Categorize toggle).
    pub fn toggle_category(&self, ids: &[MessageId], category: &str) -> Result<()> {
        let msgs: Vec<Message> = self.query_docs(&format!("WHERE id IN ({})", id_list(ids)), [])?;
        let all_have = !msgs.is_empty() && msgs.iter().all(|m| m.categories.iter().any(|c| c == category));
        self.update_many(ids, |m| {
            if all_have {
                m.categories.retain(|c| c != category);
            } else if !m.categories.iter().any(|c| c == category) {
                m.categories.push(category.to_string());
            }
        })
    }

    fn messages_with_category(&self, name: &str) -> Result<Vec<Message>> {
        let msgs: Vec<Message> =
            self.query_docs("WHERE json_extract(json,'$.categories') LIKE ?1 ESCAPE '\\'", [like_pattern(name)])?;
        Ok(msgs.into_iter().filter(|m| m.categories.iter().any(|c| c == name)).collect())
    }

    /// Rename a category on every message that has it. Returns how many changed.
    pub fn rename_message_category(&self, old: &str, new: &str) -> Result<usize> {
        let ids: Vec<MessageId> = self.messages_with_category(old)?.iter().map(|m| m.id).collect();
        self.update_many(&ids, |m| {
            let mut cats: Vec<String> = Vec::new();
            for c in &m.categories {
                let c = if c == old { new.to_string() } else { c.clone() };
                if !cats.contains(&c) {
                    cats.push(c);
                }
            }
            m.categories = cats;
        })?;
        Ok(ids.len())
    }

    /// Remove a category from every message. Returns how many changed.
    pub fn remove_message_category(&self, name: &str) -> Result<usize> {
        let ids: Vec<MessageId> = self.messages_with_category(name)?.iter().map(|m| m.id).collect();
        self.update_many(&ids, |m| m.categories.retain(|c| c != name))?;
        Ok(ids.len())
    }

    pub fn set_inference(&self, ids: &[MessageId], class: InferenceClass) -> Result<()> {
        self.update_many(ids, |m| m.inference = class)
    }

    /// "Always move to Focused/Other inbox": future mail from `address` gets
    /// this class (applied in [`Store::insert_message`]).
    pub fn set_sender_inference(&self, address: &str, class: InferenceClass) -> Result<()> {
        self.conn().execute(
            "INSERT INTO mail_inference_overrides (address, inference) VALUES (lower(?1), ?2)
             ON CONFLICT(address) DO UPDATE SET inference = excluded.inference",
            params![address.trim(), enum_str(&class)],
        )?;
        Ok(())
    }

    pub fn sender_inference(&self, address: &str) -> Result<Option<InferenceClass>> {
        let v: Option<String> = self
            .conn()
            .query_row("SELECT inference FROM mail_inference_overrides WHERE address = lower(?1)", [address.trim()], |r| r.get(0))
            .optional()?;
        Ok(v.map(|s| if s == "Other" { InferenceClass::Other } else { InferenceClass::Focused }))
    }

    // ------------------------------------------------------------ snooze

    /// The account's Snoozed folder, if one was created.
    pub fn snoozed_folder(&self, account_id: AccountId) -> Result<Option<FolderId>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT s.folder_id FROM mail_special_folders s JOIN folders f ON f.id = s.folder_id
                 WHERE s.account_id = ?1 AND s.kind = 'snoozed'",
                [account_id],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Snoozed folders of all accounts.
    pub fn snoozed_folder_ids(&self) -> Result<Vec<FolderId>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT s.folder_id FROM mail_special_folders s JOIN folders f ON f.id = s.folder_id WHERE s.kind = 'snoozed'",
        )?;
        let ids = stmt.query_map([], |r| r.get(0))?.collect::<std::result::Result<Vec<FolderId>, _>>()?;
        Ok(ids)
    }

    /// The Snoozed folder, created on first use (Outlook shows it after the first snooze).
    fn ensure_snoozed_folder(&self, account_id: AccountId) -> Result<FolderId> {
        if let Some(id) = self.snoozed_folder(account_id)? {
            return Ok(id);
        }
        let existing = self
            .folders(Some(account_id))?
            .into_iter()
            .find(|f| f.role == FolderRole::Custom && f.parent_id.is_none() && f.name.eq_ignore_ascii_case("Snoozed"));
        let id = match existing {
            Some(f) => f.id,
            None => self.create_folder(account_id, None, "Snoozed")?,
        };
        self.conn().execute(
            "INSERT OR REPLACE INTO mail_special_folders (account_id, kind, folder_id) VALUES (?1, 'snoozed', ?2)",
            params![account_id, id],
        )?;
        Ok(id)
    }

    /// Snooze messages until `until`: they move to the account's Snoozed
    /// folder and come back to the Inbox, unread, when due (see
    /// [`Store::wake_snoozed`]). `None` un-snoozes them back to where they were.
    pub fn snooze(&self, ids: &[MessageId], until: Option<chrono::DateTime<Utc>>) -> Result<()> {
        let msgs = self.messages_by_ids(ids)?;
        match until {
            Some(t) => {
                let mut targets: HashMap<AccountId, FolderId> = HashMap::new();
                for m in &msgs {
                    if let std::collections::hash_map::Entry::Vacant(e) = targets.entry(m.account_id) {
                        e.insert(self.ensure_snoozed_folder(m.account_id)?);
                    }
                }
                self.tx(|tx| {
                    for mut m in msgs {
                        let target = targets[&m.account_id];
                        if m.folder_id != target {
                            record_origins_in(tx, &[MoveRecord { message_id: m.id, from_folder: m.folder_id }])?;
                            m.folder_id = target;
                        }
                        m.snoozed_until = Some(t);
                        m.is_read = true;
                        Self::update_doc_in(tx, &m)?;
                    }
                    Ok(())
                })
            }
            None => {
                let snoozed: HashSet<FolderId> = self.snoozed_folder_ids()?.into_iter().collect();
                let back: Vec<MessageId> = msgs.iter().filter(|m| snoozed.contains(&m.folder_id)).map(|m| m.id).collect();
                self.restore_messages(&back)?;
                self.update_many(ids, |m| m.snoozed_until = None)
            }
        }
    }

    /// Bring back messages whose snooze time has passed: they return to the
    /// top of the Inbox as unread. Returns how many woke up.
    pub fn wake_snoozed(&self, now: DateTime<Utc>) -> Result<usize> {
        let due: Vec<Message> =
            self.query_docs("WHERE snoozed_until IS NOT NULL AND snoozed_until <= ?1", [now.timestamp()])?;
        if due.is_empty() {
            return Ok(0);
        }
        let snoozed: HashSet<FolderId> = self.snoozed_folder_ids()?.into_iter().collect();
        let n = due.len();
        self.tx(|tx| {
            for mut m in due {
                if snoozed.contains(&m.folder_id)
                    && let Some(inbox) = role_folder_in(tx, m.account_id, FolderRole::Inbox)?
                {
                    m.folder_id = inbox;
                }
                m.is_read = false;
                m.snoozed_until = None;
                m.received_at = now;
                tx.execute("DELETE FROM mail_origins WHERE message_id = ?1", [m.id])?;
                Self::update_doc_in(tx, &m)?;
            }
            Ok(())
        })?;
        Ok(n)
    }

    // ------------------------------------------------------------ moves

    /// Move messages to `folder_id`. Returns where each came from (for undo).
    pub fn move_messages(&self, ids: &[MessageId], folder_id: FolderId) -> Result<Vec<MoveRecord>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        self.tx(|tx| Self::move_messages_in(tx, ids, folder_id))
    }

    pub(crate) fn move_messages_in(conn: &Connection, ids: &[MessageId], folder_id: FolderId) -> Result<Vec<MoveRecord>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let msgs: Vec<Message> = Self::query_docs_in(conn, &format!("WHERE id IN ({})", id_list(ids)), [])?;
        let target_role = folder_role_in(conn, folder_id)?.ok_or_else(|| Error::NotFound(format!("folder {folder_id}")))?;
        let mut records = Vec::new();
        for mut m in msgs {
            if m.folder_id == folder_id {
                continue;
            }
            records.push(MoveRecord { message_id: m.id, from_folder: m.folder_id });
            m.folder_id = folder_id;
            if m.snoozed_until.is_some() {
                // Moving a snoozed message out of Snoozed cancels the snooze.
                m.snoozed_until = None;
            }
            Self::update_doc_in(conn, &m)?;
        }
        if is_holding_role(&target_role) {
            record_origins_in(conn, &records)?;
        }
        Ok(records)
    }

    /// Copy messages (with bodies and attachments) into `folder_id`.
    /// Returns the new message ids.
    pub fn copy_messages(&self, ids: &[MessageId], folder_id: FolderId) -> Result<Vec<MessageId>> {
        let target: Folder = self.get_doc(folder_id)?.ok_or_else(|| Error::NotFound(format!("folder {folder_id}")))?;
        let mut out = Vec::new();
        for m in self.messages_by_ids(ids)? {
            let body = self.message_body(m.id)?.unwrap_or_default();
            let attachments: Vec<(Attachment, Option<Vec<u8>>)> = {
                let conn = self.conn();
                let mut stmt = conn.prepare_cached("SELECT json, data FROM attachments WHERE message_id = ?1 ORDER BY id")?;
                let rows = stmt.query_map([m.id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<Vec<u8>>>(1)?)))?;
                let mut v = Vec::new();
                for row in rows {
                    let (json, data) = row?;
                    v.push((serde_json::from_str::<Attachment>(&json)?, data));
                }
                v
            };
            let mut copy = Message {
                id: 0,
                account_id: target.account_id,
                folder_id,
                remote_id: None,
                snoozed_until: None,
                ..m.clone()
            };
            let new_body = MessageBody { message_id: 0, html: body.html, text: body.text, attachments: Vec::new() };
            let id = self.insert_message(&mut copy, Some(&new_body))?;
            for (mut a, data) in attachments {
                a.id = 0;
                a.message_id = id;
                self.insert_attachment(&mut a, data.as_deref())?;
            }
            out.push(id);
        }
        Ok(out)
    }

    /// Reverse a move / delete / archive.
    pub fn undo_moves(&self, records: &[MoveRecord]) -> Result<()> {
        self.tx(|tx| Self::undo_moves_in(tx, records))
    }

    pub(crate) fn undo_moves_in(conn: &Connection, records: &[MoveRecord]) -> Result<()> {
        for r in records {
            if let Some(mut m) = Self::get_doc_in::<Message>(conn, r.message_id)? {
                m.folder_id = r.from_folder;
                Self::update_doc_in(conn, &m)?;
            }
        }
        Ok(())
    }

    /// Delete messages: moved to the account's Deleted Items, or removed for
    /// good when already there. Returns move records (empty for permanent).
    pub fn delete_messages(&self, ids: &[MessageId]) -> Result<Vec<MoveRecord>> {
        let msgs: Vec<Message> = self.query_docs(&format!("WHERE id IN ({})", id_list(ids)), [])?;
        let mut records = Vec::new();
        for m in msgs {
            let deleted = self.folder_id_by_role(m.account_id, FolderRole::Deleted)?;
            if m.folder_id == deleted {
                self.delete_doc::<Message>(m.id)?;
            } else {
                records.extend(self.move_messages(&[m.id], deleted)?);
            }
        }
        Ok(records)
    }

    /// Permanently delete regardless of folder (Shift+Delete).
    pub fn delete_messages_permanently(&self, ids: &[MessageId]) -> Result<()> {
        self.conn().execute(&format!("DELETE FROM messages WHERE id IN ({})", id_list(ids)), [])?;
        Ok(())
    }

    /// "Delete all": move every message in the folder to Deleted Items
    /// (permanently when the folder is Deleted Items itself).
    pub fn delete_all_in_folder(&self, folder_id: FolderId) -> Result<Vec<MoveRecord>> {
        let ids: Vec<MessageId> =
            self.query_docs::<Message>("WHERE folder_id = ?1", [folder_id])?.into_iter().map(|m| m.id).collect();
        self.delete_messages(&ids)
    }

    /// Move to the owning account's Archive folder.
    pub fn archive_messages(&self, ids: &[MessageId]) -> Result<Vec<MoveRecord>> {
        let msgs: Vec<Message> = self.query_docs(&format!("WHERE id IN ({})", id_list(ids)), [])?;
        let mut records = Vec::new();
        for m in msgs {
            let archive = self.folder_id_by_role(m.account_id, FolderRole::Archive)?;
            records.extend(self.move_messages(&[m.id], archive)?);
        }
        Ok(records)
    }

    /// Move to Junk Email.
    pub fn mark_junk(&self, ids: &[MessageId]) -> Result<Vec<MoveRecord>> {
        self.tx(|tx| Self::mark_junk_in(tx, ids))
    }

    pub(crate) fn mark_junk_in(conn: &Connection, ids: &[MessageId]) -> Result<Vec<MoveRecord>> {
        let msgs: Vec<Message> = Self::query_docs_in(conn, &format!("WHERE id IN ({})", id_list(ids)), [])?;
        let mut records = Vec::new();
        for m in msgs {
            let junk = role_folder_in(conn, m.account_id, FolderRole::Junk)?
                .ok_or_else(|| Error::NotFound(format!("junk folder for account {}", m.account_id)))?;
            records.extend(Self::move_messages_in(conn, &[m.id], junk)?);
        }
        Ok(records)
    }

    /// The folder a message was in before it went to Deleted Items, Junk
    /// Email or Snoozed, if known.
    pub fn message_origin(&self, id: MessageId) -> Result<Option<FolderId>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT o.folder_id FROM mail_origins o JOIN folders f ON f.id = o.folder_id WHERE o.message_id = ?1",
                [id],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// "Restore" / "Not junk": move messages back to the folder they came
    /// from, or to the Inbox when that is unknown or gone.
    pub fn restore_messages(&self, ids: &[MessageId]) -> Result<Vec<MoveRecord>> {
        self.tx(|tx| Self::restore_messages_in(tx, ids))
    }

    pub(crate) fn restore_messages_in(conn: &Connection, ids: &[MessageId]) -> Result<Vec<MoveRecord>> {
        let mut records = Vec::new();
        let msgs: Vec<Message> = Self::query_docs_in(conn, &format!("WHERE id IN ({})", id_list(ids)), [])?;
        for m in msgs {
            let origin: Option<FolderId> = conn
                .query_row(
                    "SELECT o.folder_id FROM mail_origins o JOIN folders f ON f.id = o.folder_id
                     WHERE o.message_id = ?1 AND f.account_id = ?2 AND o.folder_id != ?3",
                    params![m.id, m.account_id, m.folder_id],
                    |r| r.get(0),
                )
                .optional()?;
            let target = match origin {
                Some(f) => f,
                None => role_folder_in(conn, m.account_id, FolderRole::Inbox)?
                    .ok_or_else(|| Error::NotFound(format!("inbox folder for account {}", m.account_id)))?,
            };
            records.extend(Self::move_messages_in(conn, &[m.id], target)?);
            conn.execute("DELETE FROM mail_origins WHERE message_id = ?1", [m.id])?;
        }
        Ok(records)
    }

    /// Delete every message in the folder permanently (Empty folder).
    pub fn empty_folder(&self, folder_id: FolderId) -> Result<()> {
        self.conn().execute("DELETE FROM messages WHERE folder_id = ?1", [folder_id])?;
        Ok(())
    }

    // ------------------------------------------------------------ ignore

    /// Ignore a conversation: its messages (except your own in Sent Items and
    /// Drafts) move to Deleted Items, and new ones follow for 14 days after
    /// the last activity.
    pub fn ignore_conversation(&self, conversation_id: &str) -> Result<Vec<MoveRecord>> {
        let msgs = self.conversation(conversation_id)?;
        let Some(first) = msgs.first() else {
            return Err(Error::NotFound(format!("conversation {conversation_id}")));
        };
        let account = first.account_id;
        let mut records = Vec::new();
        for m in &msgs {
            let role = folder_role_in(&self.conn(), m.folder_id)?.unwrap_or_default();
            if matches!(role.as_str(), "sent" | "drafts" | "deleted") || m.is_draft {
                continue;
            }
            let deleted = self.folder_id_by_role(m.account_id, FolderRole::Deleted)?;
            records.extend(self.move_messages(&[m.id], deleted)?);
        }
        self.conn().execute(
            "INSERT INTO mail_ignored_conversations (conversation_id, account_id, last_activity) VALUES (?1, ?2, ?3)
             ON CONFLICT(conversation_id) DO UPDATE SET last_activity = excluded.last_activity",
            params![conversation_id, account, Utc::now().timestamp()],
        )?;
        Ok(records)
    }

    /// "Stop ignoring": forget the conversation and move its messages out of
    /// Deleted Items back to where they were.
    pub fn stop_ignoring_conversation(&self, conversation_id: &str) -> Result<Vec<MoveRecord>> {
        self.conn().execute("DELETE FROM mail_ignored_conversations WHERE conversation_id = ?1", [conversation_id])?;
        let deleted: HashSet<FolderId> = self
            .folders(None)?
            .into_iter()
            .filter(|f| f.role == FolderRole::Deleted)
            .map(|f| f.id)
            .collect();
        let ids: Vec<MessageId> = self
            .conversation(conversation_id)?
            .into_iter()
            .filter(|m| deleted.contains(&m.folder_id))
            .map(|m| m.id)
            .collect();
        self.restore_messages(&ids)
    }

    /// True while the conversation is ignored (within 14 days of activity).
    pub fn is_conversation_ignored(&self, conversation_id: &str) -> Result<bool> {
        let last: Option<i64> = self
            .conn()
            .query_row(
                "SELECT last_activity FROM mail_ignored_conversations WHERE conversation_id = ?1",
                [conversation_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(last.is_some_and(|l| Utc::now().timestamp() - l <= IGNORE_EXPIRY_DAYS * 86_400))
    }

    // ------------------------------------------------------------ sweep

    fn apply_sweep(&self, folder_id: FolderId, sender: &str, mode: &str, target: FolderId, now: DateTime<Utc>) -> Result<Vec<MoveRecord>> {
        let sender = sender.trim().to_lowercase();
        let msgs: Vec<Message> = self.query_docs(
            "WHERE folder_id = ?1 AND lower(json_extract(json, '$.from.address')) = ?2 ORDER BY received_at DESC, id DESC",
            params![folder_id, sender],
        )?;
        let ids: Vec<MessageId> = match mode {
            "keep_latest" => msgs.iter().skip(1).map(|m| m.id).collect(),
            "older_than_10_days" => msgs.iter().filter(|m| m.received_at < now - Duration::days(10)).map(|m| m.id).collect(),
            _ => msgs.iter().map(|m| m.id).collect(),
        };
        self.move_messages(&ids, target)
    }

    /// Sweep the messages from `sender` in `folder_id` into `target`.
    ///
    /// `mode` is one of `"move_all"` (once), `"move_all_future"`,
    /// `"keep_latest"` or `"older_than_10_days"`; the last three also save a
    /// sweep rule that [`Store::run_sweep_rules`] applies daily.
    pub fn sweep(&self, folder_id: FolderId, sender: &str, mode: &str, target: FolderId) -> Result<Vec<MoveRecord>> {
        if !SWEEP_MODES.contains(&mode) {
            return Err(Error::Invalid(format!("unknown sweep mode {mode}")));
        }
        if folder_id == target {
            return Err(Error::Invalid("choose a different folder to move to".into()));
        }
        let folder: Folder = self.get_doc(folder_id)?.ok_or_else(|| Error::NotFound(format!("folder {folder_id}")))?;
        let now = Utc::now();
        let records = self.apply_sweep(folder_id, sender, mode, target, now)?;
        if mode != "move_all" {
            let conn = self.conn();
            conn.execute(
                "DELETE FROM mail_sweep_rules WHERE folder_id = ?1 AND sender = lower(?2)",
                params![folder_id, sender.trim()],
            )?;
            conn.execute(
                "INSERT INTO mail_sweep_rules (account_id, folder_id, sender, mode, target_folder_id, created_at, last_run)
                 VALUES (?1, ?2, lower(?3), ?4, ?5, ?6, ?6)",
                params![folder.account_id, folder_id, sender.trim(), mode, target, now.timestamp()],
            )?;
        }
        Ok(records)
    }

    /// Saved sweep rules: (id, folder, sender, mode, target folder).
    pub fn sweep_rules(&self) -> Result<Vec<SweepRuleRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached("SELECT id, folder_id, sender, mode, target_folder_id FROM mail_sweep_rules ORDER BY id")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn delete_sweep_rule(&self, id: i64) -> Result<()> {
        self.conn().execute("DELETE FROM mail_sweep_rules WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Apply every saved sweep rule (Outlook runs them once a day; Axentra
    /// on launch and daily while running). Rules whose folders are gone are
    /// dropped. Returns the number of messages moved.
    pub fn run_sweep_rules(&self) -> Result<usize> {
        let now = Utc::now();
        let folders: HashSet<FolderId> = self.folders(None)?.into_iter().map(|f| f.id).collect();
        let mut moved = 0;
        for (id, folder, sender, mode, target) in self.sweep_rules()? {
            if !folders.contains(&folder) || !folders.contains(&target) {
                self.delete_sweep_rule(id)?;
                continue;
            }
            moved += self.apply_sweep(folder, &sender, &mode, target, now)?.len();
            self.conn().execute("UPDATE mail_sweep_rules SET last_run = ?1 WHERE id = ?2", params![now.timestamp(), id])?;
        }
        Ok(moved)
    }

    // ------------------------------------------------------------ export

    /// The message as an RFC 5322 `.eml` document (Save as), with its HTML
    /// and text bodies and any downloaded attachments.
    pub fn message_as_eml(&self, id: MessageId) -> Result<String> {
        let m = self.message(id)?.ok_or_else(|| Error::NotFound(format!("message {id}")))?;
        let body = self.message_body(id)?.unwrap_or_default();
        let mut parts: Vec<(Attachment, Vec<u8>)> = Vec::new();
        for a in &body.attachments {
            if let Some(data) = self.attachment_data(a.id)? {
                parts.push((a.clone(), data));
            }
        }
        let date = m.sent_at.unwrap_or(m.received_at).with_timezone(&Local).to_rfc2822();
        let mut out = String::new();
        out.push_str(&format!("Date: {date}\r\n"));
        out.push_str(&format!("From: {}\r\n", addr_header(std::slice::from_ref(&m.from))));
        if !m.to.is_empty() {
            out.push_str(&format!("To: {}\r\n", addr_header(&m.to)));
        }
        if !m.cc.is_empty() {
            out.push_str(&format!("Cc: {}\r\n", addr_header(&m.cc)));
        }
        out.push_str(&format!("Subject: {}\r\n", encode_header(&m.subject)));
        if !m.internet_message_id.is_empty() {
            out.push_str(&format!("Message-ID: <{}>\r\n", m.internet_message_id));
        }
        if let Some(r) = &m.in_reply_to {
            out.push_str(&format!("In-Reply-To: <{r}>\r\n"));
        }
        match m.importance {
            Importance::High => out.push_str("Importance: high\r\nX-Priority: 1\r\n"),
            Importance::Low => out.push_str("Importance: low\r\nX-Priority: 5\r\n"),
            Importance::Normal => {}
        }
        out.push_str("MIME-Version: 1.0\r\n");
        let alt_boundary = format!("=_axentra_alt_{id}");
        let alternative = {
            let mut s = format!("Content-Type: multipart/alternative; boundary=\"{alt_boundary}\"\r\n\r\n");
            s.push_str(&format!("--{alt_boundary}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n"));
            s.push_str(&wrap76(&base64(body.text.as_bytes())));
            if let Some(html) = &body.html {
                s.push_str(&format!("--{alt_boundary}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n"));
                s.push_str(&wrap76(&base64(html.as_bytes())));
            }
            s.push_str(&format!("--{alt_boundary}--\r\n"));
            s
        };
        if parts.is_empty() {
            out.push_str(&alternative);
        } else {
            let mixed = format!("=_axentra_mixed_{id}");
            out.push_str(&format!("Content-Type: multipart/mixed; boundary=\"{mixed}\"\r\n\r\n--{mixed}\r\n"));
            out.push_str(&alternative);
            for (a, data) in parts {
                let ct = if a.content_type.is_empty() { "application/octet-stream" } else { &a.content_type };
                out.push_str(&format!(
                    "--{mixed}\r\nContent-Type: {ct}; name=\"{name}\"\r\nContent-Disposition: attachment; filename=\"{name}\"\r\nContent-Transfer-Encoding: base64\r\n\r\n",
                    name = encode_header(&a.name).replace('"', "'"),
                ));
                out.push_str(&wrap76(&base64(&data)));
            }
            out.push_str(&format!("--{mixed}--\r\n"));
        }
        Ok(out)
    }

    // ------------------------------------------------------------ addresses

    /// Address-book suggestions for recipient fields: recent recipients,
    /// contacts and past senders whose name or address contains `prefix`.
    pub fn suggest_addresses(&self, prefix: &str, limit: usize) -> Result<Vec<EmailAddress>> {
        let pat = like_pattern(prefix.trim());
        let conn = self.conn();
        let mut out: Vec<EmailAddress> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut add = |e: EmailAddress, out: &mut Vec<EmailAddress>| {
            if !e.address.is_empty() && seen.insert(e.address.to_lowercase()) {
                out.push(e);
            }
        };
        {
            let mut stmt = conn.prepare_cached(
                "SELECT name, address FROM recent_addresses WHERE name LIKE ?1 ESCAPE '\\' OR address LIKE ?1 ESCAPE '\\'
                 ORDER BY use_count DESC, last_used DESC LIMIT 20",
            )?;
            for row in stmt.query_map([&pat], |r| Ok(EmailAddress::new(r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
                add(row?, &mut out);
            }
        }
        {
            let mut stmt = conn.prepare_cached(
                "SELECT display_name, json_extract(je.value, '$.value') FROM contacts, json_each(json_extract(contacts.json, '$.emails')) je
                 WHERE display_name LIKE ?1 ESCAPE '\\' OR json_extract(je.value, '$.value') LIKE ?1 ESCAPE '\\'
                 ORDER BY is_favorite DESC, display_name LIMIT 20",
            )?;
            for row in stmt.query_map([&pat], |r| Ok(EmailAddress::new(r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
                add(row?, &mut out);
            }
        }
        {
            let mut stmt = conn.prepare_cached(
                "SELECT json_extract(json,'$.from.name'), json_extract(json,'$.from.address'), COUNT(*) c FROM messages
                 WHERE json_extract(json,'$.from.name') LIKE ?1 ESCAPE '\\' OR json_extract(json,'$.from.address') LIKE ?1 ESCAPE '\\'
                 GROUP BY lower(json_extract(json,'$.from.address')) ORDER BY c DESC LIMIT 20",
            )?;
            for row in stmt.query_map([&pat], |r| {
                Ok(EmailAddress::new(r.get::<_, Option<String>>(0)?.unwrap_or_default(), r.get::<_, Option<String>>(1)?.unwrap_or_default()))
            })? {
                add(row?, &mut out);
            }
        }
        out.truncate(limit);
        Ok(out)
    }

    /// Remember addresses the user sent to (feeds [`Store::suggest_addresses`]).
    pub fn record_recent_addresses(&self, addrs: &[EmailAddress]) -> Result<()> {
        let now = Utc::now().timestamp();
        let conn = self.conn();
        for a in addrs {
            conn.execute(
                "INSERT INTO recent_addresses (address, name, last_used, use_count) VALUES (lower(?1), ?2, ?3, 1)
                 ON CONFLICT(address) DO UPDATE SET name = CASE WHEN excluded.name <> '' THEN excluded.name ELSE name END,
                 last_used = excluded.last_used, use_count = use_count + 1",
                params![a.address, a.name, now],
            )?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- MIME helpers

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { T[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[(n & 63) as usize] as char } else { '=' });
    }
    out
}

/// Base64 text in 76-character CRLF lines.
fn wrap76(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + s.len() / 76 * 2 + 2);
    for chunk in s.as_bytes().chunks(76) {
        out.push_str(std::str::from_utf8(chunk).unwrap_or_default());
        out.push_str("\r\n");
    }
    out
}

/// RFC 2047 encoded-word for non-ASCII header text.
fn encode_header(s: &str) -> String {
    if s.is_ascii() && !s.contains(['\r', '\n']) { s.to_string() } else { format!("=?UTF-8?B?{}?=", base64(s.as_bytes())) }
}

fn addr_header(list: &[EmailAddress]) -> String {
    list.iter()
        .map(|a| {
            if a.name.trim().is_empty() {
                a.address.clone()
            } else if a.name.is_ascii() {
                format!("\"{}\" <{}>", a.name.replace('"', "'"), a.address)
            } else {
                format!("{} <{}>", encode_header(&a.name), a.address)
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Split a search string into (keyword, term) pairs. Supports `key:value`,
/// `key:"quoted value"` and `"quoted phrases"`.
pub fn parse_search(input: &str) -> Vec<(Option<String>, String)> {
    let mut out = Vec::new();
    let mut chars = input.chars().peekable();
    while chars.peek().is_some() {
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        let mut word = String::new();
        let mut key: Option<String> = None;
        let mut quoted = false;
        while let Some(&c) = chars.peek() {
            if c == '"' {
                chars.next();
                quoted = !quoted;
                continue;
            }
            if c.is_whitespace() && !quoted {
                break;
            }
            if c == ':' && key.is_none() && !quoted && !word.is_empty() {
                chars.next();
                key = Some(std::mem::take(&mut word).to_ascii_lowercase());
                continue;
            }
            word.push(c);
            chars.next();
        }
        if !word.is_empty() {
            out.push((key, word));
        }
    }
    out
}

impl Store {
    /// Internal helper for other store modules.
    pub(crate) fn folder_for_role(&self, account_id: AccountId, role: FolderRole) -> Result<FolderId> {
        self.folder_id_by_role(account_id, role)
    }
}

#[allow(dead_code)]
fn _assert_doc<T: Doc>() {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_util::store_with_account;

    fn msg(store: &Store, acct: AccountId, role: FolderRole, subject: &str, read: bool) -> MessageId {
        let folder = store.folder_for_role(acct, role).unwrap();
        let mut m = Message {
            account_id: acct,
            folder_id: folder,
            from: EmailAddress::new("Megan Bowen", "megan@contoso.example"),
            to: vec![EmailAddress::new("Test User", "test@axentra.example")],
            subject: subject.into(),
            preview: format!("preview of {subject}"),
            received_at: Utc::now(),
            is_read: read,
            ..Default::default()
        };
        let body = MessageBody { html: None, text: format!("body text {subject}"), ..Default::default() };
        store.insert_message(&mut m, Some(&body)).unwrap()
    }

    fn msg_from(store: &Store, acct: AccountId, folder: FolderId, from: EmailAddress, subject: &str, days_ago: i64, conv: &str) -> MessageId {
        let mut m = Message {
            account_id: acct,
            folder_id: folder,
            conversation_id: conv.into(),
            from,
            to: vec![EmailAddress::new("Test User", "test@axentra.example")],
            subject: subject.into(),
            received_at: Utc::now() - Duration::days(days_ago),
            ..Default::default()
        };
        store.insert_message(&mut m, Some(&MessageBody { text: format!("{subject} text"), ..Default::default() })).unwrap()
    }

    fn search(store: &Store, q: &str) -> Vec<String> {
        let mut v: Vec<String> = store
            .messages(&MessageQuery { search: Some(q.into()), ..Default::default() })
            .unwrap()
            .into_iter()
            .map(|m| m.subject)
            .collect();
        v.sort();
        v
    }

    #[test]
    fn counts_filters_and_search() {
        let (store, acct) = store_with_account();
        let a = msg(&store, acct, FolderRole::Inbox, "Quarterly report", false);
        msg(&store, acct, FolderRole::Inbox, "Lunch?", true);
        let inbox = store.folder_by_role(acct, FolderRole::Inbox).unwrap().unwrap();
        assert_eq!((inbox.total_count, inbox.unread_count), (2, 1));

        let unread = store
            .messages(&MessageQuery { folder_id: Some(inbox.id), filter: MessageFilter::Unread, ..Default::default() })
            .unwrap();
        assert_eq!(unread.iter().map(|m| m.id).collect::<Vec<_>>(), vec![a]);

        let to_me = store
            .messages(&MessageQuery { folder_id: Some(inbox.id), filter: MessageFilter::ToMe, ..Default::default() })
            .unwrap();
        assert_eq!(to_me.len(), 2);

        let found = store.messages(&MessageQuery { search: Some("quarterly".into()), ..Default::default() }).unwrap();
        assert_eq!(found.len(), 1);
        let by_body = store.messages(&MessageQuery { search: Some("\"body text Lunch\"".into()), ..Default::default() }).unwrap();
        assert_eq!(by_body.len(), 1);
        let by_from = store.messages(&MessageQuery { search: Some("from:megan is:unread".into()), ..Default::default() }).unwrap();
        assert_eq!(by_from.len(), 1);
    }

    #[test]
    fn full_text_search_follows_outlook_matching_rules() {
        let (store, acct) = store_with_account();
        let inbox = store.folder_for_role(acct, FolderRole::Inbox).unwrap();
        let ray = EmailAddress::new("Raymond Holt", "raymond@nine-nine.example");
        let jake = EmailAddress::new("Jake Peralta", "jake@nine-nine.example");
        msg_from(&store, acct, inbox, ray.clone(), "Budget review Q3", 1, "c1");
        msg_from(&store, acct, inbox, jake.clone(), "Tray of donuts", 1, "c2");
        msg_from(&store, acct, inbox, jake, "Order 98101 shipped", 2, "c3");

        // Prefix of a word, never the middle of one.
        assert_eq!(search(&store, "ray"), vec!["Budget review Q3"]);
        assert_eq!(search(&store, "from:raymond"), vec!["Budget review Q3"]);
        assert_eq!(search(&store, "from:jake subject:donuts"), vec!["Tray of donuts"]);
        // Short numbers don't match inside longer ones; full numbers do.
        assert!(search(&store, "9810").is_empty());
        assert_eq!(search(&store, "98101"), vec!["Order 98101 shipped"]);
        // Operators and phrases.
        assert_eq!(search(&store, "budget OR donuts"), vec!["Budget review Q3", "Tray of donuts"]);
        assert_eq!(search(&store, "NOT budget"), vec!["Order 98101 shipped", "Tray of donuts"]);
        assert_eq!(search(&store, "\"review Q3\""), vec!["Budget review Q3"]);
        assert!(search(&store, "\"Q3 review\"").is_empty());
        // Body text and recipients are indexed, and updates re-index.
        assert_eq!(search(&store, "body:donuts"), vec!["Tray of donuts"]);
        assert_eq!(search(&store, "to:test"), vec!["Budget review Q3", "Order 98101 shipped", "Tray of donuts"]);
        let id = store.messages(&MessageQuery { search: Some("donuts".into()), ..Default::default() }).unwrap()[0].id;
        let mut m = store.message(id).unwrap().unwrap();
        m.subject = "Bagels instead".into();
        store.update_message(&m).unwrap();
        assert_eq!(search(&store, "bagels"), vec!["Bagels instead"]);
        // Nothing searchable matches nothing.
        assert!(search(&store, "!!!").is_empty());
        store.delete_messages_permanently(&[id]).unwrap();
        assert!(search(&store, "bagels").is_empty());
    }

    #[test]
    fn search_keywords_for_dates_flags_and_attachments() {
        let (store, acct) = store_with_account();
        let inbox = store.folder_for_role(acct, FolderRole::Inbox).unwrap();
        let p = EmailAddress::new("Pat", "pat@x.example");
        let today = msg_from(&store, acct, inbox, p.clone(), "Fresh", 0, "a");
        let old = msg_from(&store, acct, inbox, p, "Ancient", 400, "b");
        store.set_flag(&[old], FlagStatus::Flagged, Some(Local::now().date_naive())).unwrap();
        let mut m = store.message(today).unwrap().unwrap();
        m.has_attachments = true;
        store.update_message(&m).unwrap();

        assert_eq!(search(&store, "received:today"), vec!["Fresh"]);
        assert_eq!(search(&store, "received:<\"this year\""), vec!["Ancient"]);
        assert_eq!(search(&store, "hasattachments:yes"), vec!["Fresh"]);
        assert_eq!(search(&store, "hasattachment:no"), vec!["Ancient"]);
        assert_eq!(search(&store, "isflagged:true"), vec!["Ancient"]);
        assert_eq!(search(&store, "due:today"), vec!["Ancient"]);
        assert_eq!(search(&store, "read:no"), vec!["Ancient", "Fresh"]);
    }

    #[test]
    fn date_expressions() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 22).unwrap(); // Thursday
        let d = |m, dd| NaiveDate::from_ymd_opt(2026, m, dd).unwrap();
        let ws = Weekday::Sun;
        assert_eq!(parse_date_expr("today", today, ws), Some((Some(d(10, 22)), Some(d(10, 23)))));
        assert_eq!(parse_date_expr("\"this week\"", today, ws), Some((Some(d(10, 18)), Some(d(10, 25)))));
        assert_eq!(parse_date_expr("last month", today, ws), Some((Some(d(9, 1)), Some(d(10, 1)))));
        assert_eq!(parse_date_expr(">1/15/2026", today, ws), Some((Some(d(1, 16)), None)));
        assert_eq!(parse_date_expr("<=2026-03-01", today, ws), Some((None, Some(d(3, 2)))));
        assert_eq!(parse_date_expr("1/1/2026..1/31/2026", today, ws), Some((Some(d(1, 1)), Some(d(2, 1)))));
        assert_eq!(parse_date_expr("monday", today, ws), Some((Some(d(10, 19)), Some(d(10, 20)))));
        assert_eq!(parse_date_expr("december", today, ws).unwrap().0, NaiveDate::from_ymd_opt(2025, 12, 1));
        assert_eq!(parse_date_expr("10/5", today, ws), Some((Some(d(10, 5)), Some(d(10, 6)))));
        assert_eq!(parse_date_expr("whenever", today, ws), None);
    }

    #[test]
    fn delete_moves_then_purges_and_undo_restores() {
        let (store, acct) = store_with_account();
        let id = msg(&store, acct, FolderRole::Inbox, "Old news", true);
        let inbox = store.folder_for_role(acct, FolderRole::Inbox).unwrap();
        let deleted = store.folder_for_role(acct, FolderRole::Deleted).unwrap();
        let recs = store.delete_messages(&[id]).unwrap();
        assert_eq!(store.message(id).unwrap().unwrap().folder_id, deleted);
        store.undo_moves(&recs).unwrap();
        assert_eq!(store.message(id).unwrap().unwrap().folder_id, inbox);
        store.delete_messages(&[id]).unwrap();
        assert!(store.delete_messages(&[id]).unwrap().is_empty());
        assert!(store.message(id).unwrap().is_none());
    }

    #[test]
    fn restore_returns_to_origin_through_junk_and_deleted() {
        let (store, acct) = store_with_account();
        let projects = store.create_folder(acct, None, "Projects").unwrap();
        let inbox = store.folder_for_role(acct, FolderRole::Inbox).unwrap();
        let id = msg_from(&store, acct, projects, EmailAddress::new("A", "a@x.example"), "Plan", 1, "p");
        store.mark_junk(&[id]).unwrap();
        store.delete_messages(&[id]).unwrap();
        assert_eq!(store.message_origin(id).unwrap(), Some(projects));
        let recs = store.restore_messages(&[id]).unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(store.message(id).unwrap().unwrap().folder_id, projects);
        // Unknown origin: back to the Inbox.
        let junk = store.folder_for_role(acct, FolderRole::Junk).unwrap();
        let other = msg_from(&store, acct, junk, EmailAddress::new("B", "b@x.example"), "Spam?", 1, "q");
        store.restore_messages(&[other]).unwrap();
        assert_eq!(store.message(other).unwrap().unwrap().folder_id, inbox);
    }

    #[test]
    fn conversation_groups_and_sizes() {
        let (store, acct) = store_with_account();
        let inbox = store.folder_for_role(acct, FolderRole::Inbox).unwrap();
        let sent = store.folder_for_role(acct, FolderRole::Sent).unwrap();
        let deleted = store.folder_for_role(acct, FolderRole::Deleted).unwrap();
        let a = EmailAddress::new("A", "a@x.example");
        msg_from(&store, acct, inbox, a.clone(), "Thread", 3, "t");
        msg_from(&store, acct, inbox, a.clone(), "Solo", 2, "s");
        msg_from(&store, acct, inbox, a.clone(), "RE: Thread", 1, "t");
        msg_from(&store, acct, sent, EmailAddress::new("Test User", "test@axentra.example"), "RE: Thread", 2, "t");
        msg_from(&store, acct, deleted, a, "RE: Thread", 0, "t");
        let groups = store
            .conversation_groups(&MessageQuery { folder_id: Some(inbox), ..Default::default() })
            .unwrap();
        let shape: Vec<(String, usize)> = groups.iter().map(|g| (g[0].conversation_id.clone(), g.len())).collect();
        assert_eq!(shape, vec![("t".to_string(), 2), ("s".to_string(), 1)]);
        let sizes = store.conversation_sizes(&["t".into(), "s".into()]).unwrap();
        assert_eq!(sizes["t"], 3);
        assert_eq!(sizes["s"], 1);
    }

    #[test]
    fn block_rules_send_new_mail_to_junk_and_an_exact_allow_wins() {
        use crate::junk::RuleAction;
        let (store, acct) = store_with_account();
        let inbox = store.folder_for_role(acct, FolderRole::Inbox).unwrap();
        let junk = store.folder_for_role(acct, FolderRole::Junk).unwrap();
        let spammer = EmailAddress::new("Deals", "deals@spam.example");
        store.set_sender_rule(Some(acct), &spammer.address, "Deals", RuleAction::Block).unwrap();
        let id = msg_from(&store, acct, inbox, spammer.clone(), "Win", 0, "w");
        assert_eq!(store.message(id).unwrap().unwrap().folder_id, junk);
        assert_eq!(store.routed_by_rule(id).unwrap().as_deref(), Some("deals@spam.example"), "audited");
        // Domain rules cover the whole domain and its subdomains.
        store.set_sender_rule(Some(acct), "spam.example", "", RuleAction::Block).unwrap();
        let id2 = msg_from(&store, acct, inbox, EmailAddress::new("X", "x@mx.spam.example"), "Win 2", 0, "w2");
        assert_eq!(store.message(id2).unwrap().unwrap().folder_id, junk);
        // An exact allow is an exception to the blocked domain.
        store.set_sender_rule(Some(acct), &spammer.address, "", RuleAction::Allow).unwrap();
        let id3 = msg_from(&store, acct, inbox, spammer, "Win 3", 0, "w3");
        assert_eq!(store.message(id3).unwrap().unwrap().folder_id, inbox);
        assert_eq!(store.routed_by_rule(id3).unwrap(), None);
    }

    #[test]
    fn ignore_conversation_moves_existing_and_future_mail() {
        let (store, acct) = store_with_account();
        let inbox = store.folder_for_role(acct, FolderRole::Inbox).unwrap();
        let sent = store.folder_for_role(acct, FolderRole::Sent).unwrap();
        let deleted = store.folder_for_role(acct, FolderRole::Deleted).unwrap();
        let a = EmailAddress::new("A", "a@x.example");
        let first = msg_from(&store, acct, inbox, a.clone(), "Noise", 2, "noise");
        let mine = msg_from(&store, acct, sent, EmailAddress::new("Test User", "test@axentra.example"), "RE: Noise", 1, "noise");
        let recs = store.ignore_conversation("noise").unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(store.message(first).unwrap().unwrap().folder_id, deleted);
        assert_eq!(store.message(mine).unwrap().unwrap().folder_id, sent);
        assert!(store.is_conversation_ignored("noise").unwrap());
        let later = msg_from(&store, acct, inbox, a, "RE: Noise", 0, "noise");
        assert_eq!(store.message(later).unwrap().unwrap().folder_id, deleted);
        store.stop_ignoring_conversation("noise").unwrap();
        assert!(!store.is_conversation_ignored("noise").unwrap());
        assert_eq!(store.message(first).unwrap().unwrap().folder_id, inbox);
        assert_eq!(store.message(later).unwrap().unwrap().folder_id, inbox);
    }

    #[test]
    fn sweep_modes_and_saved_rules() {
        let (store, acct) = store_with_account();
        let inbox = store.folder_for_role(acct, FolderRole::Inbox).unwrap();
        let deleted = store.folder_for_role(acct, FolderRole::Deleted).unwrap();
        let news = EmailAddress::new("News", "news@paper.example");
        let newest = msg_from(&store, acct, inbox, news.clone(), "Today's paper", 0, "n1");
        msg_from(&store, acct, inbox, news.clone(), "Yesterday's paper", 1, "n2");
        let old = msg_from(&store, acct, inbox, news.clone(), "Old paper", 20, "n3");
        let other = msg_from(&store, acct, inbox, EmailAddress::new("B", "b@x.example"), "Keep me", 30, "k");

        assert!(store.sweep(inbox, "news@paper.example", "bogus", deleted).is_err());
        let recs = store.sweep(inbox, "news@paper.example", "older_than_10_days", deleted).unwrap();
        assert_eq!(recs.iter().map(|r| r.message_id).collect::<Vec<_>>(), vec![old]);
        let recs = store.sweep(inbox, "NEWS@paper.example", "keep_latest", deleted).unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(store.message(newest).unwrap().unwrap().folder_id, inbox);
        assert_eq!(store.message(other).unwrap().unwrap().folder_id, inbox);
        // One rule per sender and folder; the newer choice replaces the older.
        let rules = store.sweep_rules().unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].3, "keep_latest");
        // The rule keeps applying to new mail.
        let newer = msg_from(&store, acct, inbox, news, "Tomorrow's paper", -1, "n4");
        assert_eq!(store.run_sweep_rules().unwrap(), 1);
        assert_eq!(store.message(newest).unwrap().unwrap().folder_id, deleted);
        assert_eq!(store.message(newer).unwrap().unwrap().folder_id, inbox);
        store.delete_sweep_rule(rules[0].0).unwrap();
        assert!(store.sweep_rules().unwrap().is_empty());
    }

    #[test]
    fn snooze_moves_to_snoozed_and_wakes_into_inbox() {
        let (store, acct) = store_with_account();
        let inbox = store.folder_for_role(acct, FolderRole::Inbox).unwrap();
        let id = msg(&store, acct, FolderRole::Inbox, "Later", false);
        assert!(store.snoozed_folder(acct).unwrap().is_none());
        let until = Utc::now() + Duration::hours(3);
        store.snooze(&[id], Some(until)).unwrap();
        let snoozed = store.snoozed_folder(acct).unwrap().expect("snoozed folder created");
        let m = store.message(id).unwrap().unwrap();
        assert_eq!((m.folder_id, m.is_read, m.snoozed_until.is_some()), (snoozed, true, true));
        // Visible in Snoozed, counted there, not in the Inbox.
        let in_snoozed = store.messages(&MessageQuery { folder_id: Some(snoozed), ..Default::default() }).unwrap();
        assert_eq!(in_snoozed.len(), 1);
        assert_eq!(store.folder(snoozed).unwrap().unwrap().total_count, 1);
        assert_eq!(store.wake_snoozed(Utc::now()).unwrap(), 0);
        assert_eq!(store.wake_snoozed(until + Duration::minutes(1)).unwrap(), 1);
        let m = store.message(id).unwrap().unwrap();
        assert_eq!((m.folder_id, m.is_read, m.snoozed_until), (inbox, false, None));
        // Un-snoozing returns to the original folder.
        store.snooze(&[id], Some(until)).unwrap();
        store.snooze(&[id], None).unwrap();
        assert_eq!(store.message(id).unwrap().unwrap().folder_id, inbox);
        assert_eq!(store.snoozed_folder_ids().unwrap(), vec![snoozed]);
    }

    #[test]
    fn favorites_keep_user_order() {
        let (store, acct) = store_with_account();
        let names = |s: &Store| s.favorites().unwrap().into_iter().map(|f| f.name).collect::<Vec<_>>();
        assert_eq!(names(&store), vec!["Inbox", "Drafts", "Sent Items"]);
        let projects = store.create_folder(acct, None, "Projects").unwrap();
        store.set_folder_favorite(projects, true).unwrap();
        assert_eq!(names(&store), vec!["Inbox", "Drafts", "Sent Items", "Projects"]);
        store.move_favorite(projects, -2).unwrap();
        assert_eq!(names(&store), vec!["Inbox", "Projects", "Drafts", "Sent Items"]);
        let inbox = store.folder_for_role(acct, FolderRole::Inbox).unwrap();
        store.move_favorite(inbox, 1).unwrap();
        assert_eq!(names(&store), vec!["Projects", "Inbox", "Drafts", "Sent Items"]);
        store.set_folder_favorite(inbox, false).unwrap();
        assert_eq!(names(&store), vec!["Projects", "Drafts", "Sent Items"]);
    }

    #[test]
    fn flags_categories_and_suggestions() {
        let (store, acct) = store_with_account();
        let id = msg(&store, acct, FolderRole::Inbox, "Budget", false);
        store.set_flag(&[id], FlagStatus::Flagged, None).unwrap();
        assert_eq!(store.flagged_messages().unwrap().len(), 1);
        store.toggle_category(&[id], "Red category").unwrap();
        assert_eq!(store.message(id).unwrap().unwrap().categories, vec!["Red category".to_string()]);
        store.toggle_category(&[id], "Red category").unwrap();
        assert!(store.message(id).unwrap().unwrap().categories.is_empty());
        let s = store.suggest_addresses("meg", 5).unwrap();
        assert_eq!(s[0].address, "megan@contoso.example");
    }

    #[test]
    fn category_rename_and_removal_on_messages() {
        let (store, acct) = store_with_account();
        let a = msg(&store, acct, FolderRole::Inbox, "One", true);
        let b = msg(&store, acct, FolderRole::Inbox, "Two", true);
        store.set_categories(&[a], &["Red category".into(), "Blue category".into()]).unwrap();
        store.set_categories(&[b], &["Red category x".into()]).unwrap();
        assert_eq!(store.rename_message_category("Red category", "Urgent").unwrap(), 1);
        assert_eq!(store.message(a).unwrap().unwrap().categories, vec!["Urgent".to_string(), "Blue category".to_string()]);
        assert_eq!(store.message(b).unwrap().unwrap().categories, vec!["Red category x".to_string()]);
        assert_eq!(store.remove_message_category("Blue category").unwrap(), 1);
        assert_eq!(store.message(a).unwrap().unwrap().categories, vec!["Urgent".to_string()]);
    }

    #[test]
    fn custom_folder_lifecycle() {
        let (store, acct) = store_with_account();
        let f = store.create_folder(acct, None, "Projects").unwrap();
        assert!(store.create_folder(acct, None, "projects").is_err());
        store.rename_folder(f, "Project X").unwrap();
        let g = store.create_folder(acct, None, "Other").unwrap();
        assert!(store.rename_folder(g, "project x").is_err());
        let child = store.create_folder(acct, Some(f), "Child").unwrap();
        assert!(store.move_folder(f, Some(child)).is_err());
        assert!(store.move_folder(f, Some(f)).is_err());
        assert!(store.folder_is_within(child, f).unwrap());
        store.move_folder(child, Some(g)).unwrap();
        assert_eq!(store.folder(child).unwrap().unwrap().parent_id, Some(g));
        store.delete_folder(f).unwrap();
        let deleted = store.folder_for_role(acct, FolderRole::Deleted).unwrap();
        assert_eq!(store.folder(f).unwrap().unwrap().parent_id, Some(deleted));
        store.delete_folder(f).unwrap();
        assert!(store.folder(f).unwrap().is_none());
    }

    #[test]
    fn copy_and_recent_folders() {
        let (store, acct) = store_with_account();
        let archive = store.folder_for_role(acct, FolderRole::Archive).unwrap();
        let id = msg(&store, acct, FolderRole::Inbox, "Contract", true);
        let mut att = Attachment { message_id: id, name: "c.txt".into(), content_type: "text/plain".into(), size: 5, ..Default::default() };
        store.insert_attachment(&mut att, Some(b"hello")).unwrap();
        let copies = store.copy_messages(&[id], archive).unwrap();
        assert_eq!(copies.len(), 1);
        let copy = store.message(copies[0]).unwrap().unwrap();
        assert_eq!((copy.folder_id, copy.subject.as_str()), (archive, "Contract"));
        let body = store.message_body(copy.id).unwrap().unwrap();
        assert_eq!(body.text, "body text Contract");
        assert_eq!(store.attachment_data(body.attachments[0].id).unwrap().as_deref(), Some(&b"hello"[..]));
        assert!(store.message(id).unwrap().is_some());

        store.record_recent_folder(archive).unwrap();
        let inbox = store.folder_for_role(acct, FolderRole::Inbox).unwrap();
        store.record_recent_folder(inbox).unwrap();
        store.record_recent_folder(archive).unwrap();
        let recent: Vec<FolderId> = store.recent_folders(5).unwrap().into_iter().map(|f| f.id).collect();
        assert_eq!(recent, vec![archive, inbox]);
    }

    #[test]
    fn inference_override_applies_to_new_mail() {
        let (store, acct) = store_with_account();
        let inbox = store.folder_for_role(acct, FolderRole::Inbox).unwrap();
        store.set_sender_inference("Promo@Shop.example", InferenceClass::Other).unwrap();
        assert_eq!(store.sender_inference("promo@shop.example").unwrap(), Some(InferenceClass::Other));
        let id = msg_from(&store, acct, inbox, EmailAddress::new("Shop", "promo@shop.example"), "Sale", 0, "sale");
        assert_eq!(store.message(id).unwrap().unwrap().inference, InferenceClass::Other);
    }

    #[test]
    fn eml_export_round_trips() {
        let (store, acct) = store_with_account();
        let inbox = store.folder_for_role(acct, FolderRole::Inbox).unwrap();
        let mut m = Message {
            account_id: acct,
            folder_id: inbox,
            from: EmailAddress::new("Zoë Müller", "zoe@x.example"),
            to: vec![EmailAddress::new("Test User", "test@axentra.example")],
            subject: "Café plans ☕".into(),
            internet_message_id: "abc@x.example".into(),
            received_at: Utc::now(),
            ..Default::default()
        };
        let body = MessageBody { html: Some("<p>See you at <b>noon</b></p>".into()), text: "See you at noon".into(), ..Default::default() };
        let id = store.insert_message(&mut m, Some(&body)).unwrap();
        let mut att = Attachment { message_id: id, name: "menu.txt".into(), content_type: "text/plain".into(), size: 4, ..Default::default() };
        store.insert_attachment(&mut att, Some(b"soup")).unwrap();
        let eml = store.message_as_eml(id).unwrap();
        let parsed = mail_parser::MessageParser::default().parse(eml.as_bytes()).expect("parses");
        assert_eq!(parsed.subject(), Some("Café plans ☕"));
        assert_eq!(parsed.from().and_then(|f| f.first()).and_then(|a| a.name()), Some("Zoë Müller"));
        assert_eq!(parsed.body_text(0).as_deref(), Some("See you at noon"));
        assert!(parsed.body_html(0).unwrap().contains("<b>noon</b>"));
        assert_eq!(parsed.attachment(0).map(|a| a.contents()), Some(&b"soup"[..]));
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"Man"), "TWFu");
    }

    #[test]
    fn conversations_load_in_one_batch() {
        let (store, acct) = store_with_account();
        let inbox = store.folder_for_role(acct, FolderRole::Inbox).unwrap();
        let sent = store.folder_for_role(acct, FolderRole::Sent).unwrap();
        let megan = EmailAddress::new("Megan Bowen", "megan@contoso.example");
        let me = EmailAddress::new("Test User", "test@axentra.example");
        let a = msg_from(&store, acct, inbox, megan.clone(), "Plan", 3, "c1");
        let b = msg_from(&store, acct, sent, me, "RE: Plan", 2, "c1");
        let c = msg_from(&store, acct, inbox, megan, "Other", 1, "c2");
        let convs = store.conversations(&["c1".into(), "c2".into(), "nope".into()]).unwrap();
        assert_eq!(convs["c1"].iter().map(|m| m.id).collect::<Vec<_>>(), vec![a, b]);
        assert_eq!(convs["c2"].iter().map(|m| m.id).collect::<Vec<_>>(), vec![c]);
        assert!(!convs.contains_key("nope"));
        assert!(store.conversations(&[]).unwrap().is_empty());
    }

    #[test]
    fn replies_and_forwards_from_me_are_found() {
        let (store, acct) = store_with_account();
        let inbox = store.folder_for_role(acct, FolderRole::Inbox).unwrap();
        let sent = store.folder_for_role(acct, FolderRole::Sent).unwrap();
        let drafts = store.folder_for_role(acct, FolderRole::Drafts).unwrap();
        let megan = EmailAddress::new("Megan Bowen", "megan@contoso.example");
        let me = EmailAddress::new("Test User", "TEST@axentra.example");
        let insert = |folder: FolderId, from: &EmailAddress, subject: &str, mid: &str, reply_to: Option<&str>, hours: i64, draft: bool| {
            let mut m = Message {
                account_id: acct,
                folder_id: folder,
                conversation_id: "c".into(),
                internet_message_id: mid.into(),
                in_reply_to: reply_to.map(Into::into),
                from: from.clone(),
                subject: subject.into(),
                received_at: Utc::now() - Duration::hours(hours),
                is_draft: draft,
                ..Default::default()
            };
            store.insert_message(&mut m, None).unwrap()
        };
        let original = insert(inbox, &megan, "Plan", "m1@x", None, 10, false);
        let quiet = insert(inbox, &megan, "Lunch", "m2@x", None, 9, false);
        let reply = insert(sent, &me, "RE: Plan", "m3@x", Some("m1@x"), 8, false);
        let forward = insert(sent, &me, "FW: Plan", "m4@x", Some("m1@x"), 7, false);
        // Someone else's reply and my unsent draft don't count.
        insert(inbox, &megan, "RE: Lunch", "m5@x", Some("m2@x"), 6, false);
        insert(drafts, &me, "RE: Lunch", "m6@x", Some("m2@x"), 5, true);
        let r = store.last_responses(&[original, quiet, reply]).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[&original].message_id, forward);
        assert!(r[&original].forwarded);
        // Only the reply: the latest response is a reply.
        store.delete_messages_permanently(&[forward]).unwrap();
        let r = store.last_responses(&[original]).unwrap();
        assert_eq!((r[&original].message_id, r[&original].forwarded), (reply, false));
    }

    #[test]
    fn attachment_download_state_without_loading_content() {
        let (store, acct) = store_with_account();
        let id = msg(&store, acct, FolderRole::Inbox, "Files", false);
        let mut a = Attachment { message_id: id, name: "a.txt".into(), ..Default::default() };
        let mut b = Attachment { message_id: id, name: "b.pdf".into(), ..Default::default() };
        let a = store.insert_attachment(&mut a, Some(b"hello")).unwrap();
        let b = store.insert_attachment(&mut b, None).unwrap();
        assert_eq!(store.downloaded_attachments(&[a, b]).unwrap(), HashSet::from([a]));
        store.set_attachment_data(b, b"%PDF").unwrap();
        assert_eq!(store.downloaded_attachments(&[a, b]).unwrap().len(), 2);
        assert!(store.downloaded_attachments(&[]).unwrap().is_empty());
    }

    #[test]
    fn archive_can_be_set_up_for_accounts_without_one() {
        let (store, acct) = store_with_account();
        let archive = store.folder_for_role(acct, FolderRole::Archive).unwrap();
        // Simulate an IMAP account without a \Archive folder.
        store.with_folder(archive, |f| f.role = FolderRole::Custom).unwrap();
        store.rename_folder(archive, "Old stuff").unwrap();
        assert!(store.folder_by_role(acct, FolderRole::Archive).unwrap().is_none());
        let id = msg(&store, acct, FolderRole::Inbox, "Keep", true);
        assert!(store.archive_messages(&[id]).is_err());

        let created = store.ensure_role_folder(acct, FolderRole::Archive).unwrap();
        assert_ne!(created, archive);
        let f = store.folder(created).unwrap().unwrap();
        assert_eq!((f.name.as_str(), f.role, f.parent_id), ("Archive", FolderRole::Archive, None));
        assert_eq!(store.ensure_role_folder(acct, FolderRole::Archive).unwrap(), created);
        store.archive_messages(&[id]).unwrap();
        assert_eq!(store.message(id).unwrap().unwrap().folder_id, created);

        // "Choose an existing folder": the role moves, the old holder turns custom.
        store.set_folder_role(archive, FolderRole::Archive).unwrap();
        assert_eq!(store.folder_by_role(acct, FolderRole::Archive).unwrap().unwrap().id, archive);
        assert_eq!(store.folder(created).unwrap().unwrap().role, FolderRole::Custom);
        let inbox = store.folder_for_role(acct, FolderRole::Inbox).unwrap();
        assert!(store.set_folder_role(inbox, FolderRole::Archive).is_err());
    }

    #[test]
    fn parse_search_handles_keys_and_quotes() {
        assert_eq!(
            parse_search(r#"from:megan "budget review" subject:"Q3 plan" x"#),
            vec![
                (Some("from".into()), "megan".into()),
                (None, "budget review".into()),
                (Some("subject".into()), "Q3 plan".into()),
                (None, "x".into()),
            ]
        );
    }
}
