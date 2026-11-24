//! Packages: the cached extraction of each message, the orders built from
//! them, the user's changes to orders and "Don't track orders from…".
//! The pure logic lives in [`crate::orders`].
//!
//! Extraction is per message and pending until a row with the current
//! version exists (`order_extractions`). Each order email has keys (order
//! number, tracking numbers) in `order_keys`; a changed key goes into
//! `orders_dirty`, and [`Store::rebuild_dirty_orders`] rebuilds just the
//! orders those keys reach. Order emails are grouped only once the junk
//! check has looked at them, and never from Junk Email or Deleted Items.

use std::collections::{HashMap, HashSet, VecDeque, hash_map};

use chrono::{DateTime, Duration, NaiveDate, Utc};
use rusqlite::types::FromSql;
use rusqlite::{Connection, OptionalExtension, params, params_from_iter};

use super::mail::id_list;
use super::{
    Store, changed_sql, kv_level, marks, no_insights_folders_sql, own_mail_folders_sql, sender_address_sql, sender_domain_sql, sender_match_sql, settings_in,
};
use crate::junk::{self, RuleAction, SenderRule};
use crate::model::{AccountId, Message, MessageId};
use crate::orders::{self, Entry, Extraction, ExtractionKind, Order, OrderSection, OrderStatus, OrderUserState};
use crate::{Error, Result};

pub(crate) const SCHEMA_EXTRA: &str = r#"
-- A received_at index for the newest-first pending queries (orders and topics).
CREATE INDEX IF NOT EXISTS messages_received_at ON messages(received_at DESC);
CREATE TABLE IF NOT EXISTS order_extractions (
    message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    account_id INTEGER NOT NULL,
    version TEXT NOT NULL,
    junk_version TEXT NOT NULL DEFAULT '',
    kind TEXT NOT NULL,
    join_only INTEGER NOT NULL DEFAULT 0,
    confidence REAL NOT NULL,
    status TEXT,
    json TEXT NOT NULL,
    created_at INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS order_extractions_kind ON order_extractions(kind);
CREATE TABLE IF NOT EXISTS order_keys (
    key TEXT NOT NULL,
    message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    PRIMARY KEY (key, message_id)) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS order_keys_message ON order_keys(message_id);
CREATE TABLE IF NOT EXISTS orders (
    id INTEGER PRIMARY KEY,
    account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    key TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL,
    hidden INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL,
    delivered_at INTEGER,
    json TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS orders_updated ON orders(updated_at);
CREATE TABLE IF NOT EXISTS order_messages (
    order_id INTEGER NOT NULL REFERENCES orders(id) ON DELETE CASCADE,
    message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    PRIMARY KEY (order_id, message_id)) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS order_messages_message ON order_messages(message_id);
CREATE TABLE IF NOT EXISTS order_group_keys (
    key TEXT PRIMARY KEY,
    order_id INTEGER NOT NULL REFERENCES orders(id) ON DELETE CASCADE) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS order_group_keys_order ON order_group_keys(order_id);
CREATE TABLE IF NOT EXISTS orders_dirty (key TEXT PRIMARY KEY) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS order_user_state (
    key TEXT PRIMARY KEY,
    hidden INTEGER NOT NULL DEFAULT 0,
    status_override TEXT,
    updated_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS order_sender_ignores (
    pattern TEXT PRIMARY KEY, is_domain INTEGER NOT NULL, created_at INTEGER NOT NULL);
-- Junk sender rules that came, went or changed since the last extraction
-- batch (see the orders_regate_rule_* triggers and drain_regate_in).
CREATE TABLE IF NOT EXISTS orders_regate_senders (
    pattern TEXT NOT NULL, domain INTEGER NOT NULL, PRIMARY KEY (pattern, domain)) WITHOUT ROWID;
-- A message reported as phishing is gated like one (see junk::GATE_VERSION_SQL).
CREATE INDEX IF NOT EXISTS junk_labels_message ON junk_labels(message_id);
"#;

/// Schema level of the order tables (see [`upgrade_schema_in`]).
const SCHEMA_LEVEL: &str = "orders.schema";
/// Mail older than this isn't looked at, and orders older than this aren't listed.
const HORIZON_DAYS: i64 = 400;
/// One rebuild follows an order's keys through at most this many messages
/// and keys.
const MAX_GROUP_MESSAGES: usize = 200;
const MAX_GROUP_KEYS: usize = 400;
/// One call starts no further group once it has taken this many messages
/// (each group it starts is followed to the end).
const MAX_REBUILD_MESSAGES: usize = 200;

/// Messages still to extract: no extraction, an older version, or an order
/// email whose junk gate changed since (so a later phishing verdict or
/// report, or the first check at all, is taken into account). A check
/// that is missing again (thrown away, to be redone) changes nothing until
/// it is redone: the extraction and its order stay as they are. A
/// carrier's mail read before its first check is read again once checked
/// too, whatever it was taken for: only the check's authentication results
/// make it the carrier's own. `?1` version, `?2` limit, `?3` the horizon
/// (unix seconds).
const PENDING: &str = "SELECT m.id FROM messages m
    JOIN folders f ON f.id = m.folder_id
    LEFT JOIN order_extractions e ON e.message_id = m.id
    LEFT JOIN junk_assessments j ON j.message_id = m.id
    WHERE m.folder_id NOT IN OWN_MAIL AND m.is_draft = 0 AND m.received_at >= ?3
      AND (e.message_id IS NULL OR e.version != ?1
           OR (e.kind != 'none' AND j.version IS NOT NULL AND e.junk_version IS NOT GATE)
           OR (e.junk_version = '' AND j.version IS NOT NULL AND json_extract(e.json, '$.merchant.domain') IN (CARRIERS)))";

/// [`PENDING`] with its parts filled in.
fn pending_sql() -> String {
    PENDING.replace("GATE", super::junk::GATE_VERSION_SQL).replace("OWN_MAIL", &own_mail_folders_sql()).replace("CARRIERS", &carrier_domains_sql())
}

/// The messages of a key that may be part of an order: order emails the
/// junk check has looked at, outside Junk Email, Deleted Items and your
/// own mail (and the folders under them).
fn eligible_by_key_sql() -> String {
    format!(
        "SELECT k.message_id FROM order_keys k
         JOIN order_extractions e ON e.message_id = k.message_id
         JOIN messages m ON m.id = k.message_id
         WHERE k.key = ?1 AND e.kind = 'order' AND e.junk_version != ''
           AND m.folder_id NOT IN {} AND m.is_draft = 0",
        no_insights_folders_sql()
    )
}

/// True for an order (row `orders`) that a carrier's own mail, from the
/// carrier's domain and vouched for by the account's provider, is part of:
/// the carrier confirms the shop's parcel. The shop's own tracking numbers
/// don't count, check digit or not: anyone can work one out for a number
/// they made up. Nor does a carrier's From address nobody vouched for:
/// anyone can write that too.
fn carrier_confirmed_sql() -> String {
    format!(
        "EXISTS (SELECT 1 FROM order_messages om JOIN order_extractions oe ON oe.message_id = om.message_id
                 WHERE om.order_id = orders.id AND json_extract(oe.json, '$.merchant.domain') IN ({})
                   AND json_extract(oe.json, '$.carrier_verified') = 1)",
        carrier_domains_sql()
    )
}

/// The carriers' sender domains, as an SQL list.
fn carrier_domains_sql() -> String {
    orders::carriers::carrier_sender_domains().iter().map(|d| format!("'{}'", d.replace('\'', "''"))).collect::<Vec<_>>().join(", ")
}

/// A change to one order from its More menu.
#[derive(Clone, Debug, PartialEq)]
pub enum OrderStateChange {
    /// Remove from (or put back in) Packages.
    Hide(bool),
    /// Mark as delivered / not delivered; `None` goes back to what the mail says.
    Override(Option<OrderStatus>),
}

/// "Don't track orders from…": an address, or a domain with `is_domain`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderSenderIgnore {
    pub pattern: String,
    pub is_domain: bool,
    pub created_at: i64,
}

/// The folder pane's Packages count.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OrdersSummary {
    /// Orders not yet delivered, cancelled or returned.
    pub active: u32,
    /// Delayed, a delivery problem, or late.
    pub attention: u32,
}

/// How far the look through your mail for orders has got.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OrdersProgress {
    /// Messages that can hold an order.
    pub total: u32,
    /// Of those, still to read.
    pub left: u32,
    /// Changed orders still to put back together.
    pub regrouping: u32,
}

impl OrdersProgress {
    pub fn busy(self) -> bool {
        self.left > 0 || self.regrouping > 0
    }

    /// Messages read so far.
    pub fn done(self) -> u32 {
        self.total.saturating_sub(self.left)
    }
}

fn version() -> String {
    format!("o{}", orders::EXTRACT_VERSION)
}

fn horizon(now: DateTime<Utc>) -> i64 {
    (now - Duration::days(HORIZON_DAYS)).timestamp()
}

fn kind_str(kind: ExtractionKind) -> &'static str {
    match kind {
        ExtractionKind::None => "none",
        ExtractionKind::Order => "order",
        ExtractionKind::Suppressed => "suppressed",
    }
}

/// Triggers that keep extractions honest: a message's sender, subject or
/// body drops its extraction and keys (so its old keys go dirty and its
/// topic follows); a move regroups its orders; a junk sender rule that
/// comes, goes or changes is queued, and the next extraction batch drops
/// the order emails of the senders it covers (an allow rule lets a Danger
/// message be an order email). Queued, not dropped at once: a backup
/// restore or a legacy import writes hundreds of rules, and each would
/// read every order email.
fn triggers_sql() -> String {
    let drop = |name: &str, when: &str, id: &str| {
        format!(
            "CREATE TRIGGER IF NOT EXISTS {name} {when} BEGIN
                DELETE FROM order_keys WHERE message_id = {id}; DELETE FROM order_extractions WHERE message_id = {id}; END;\n"
        )
    };
    let mut s = String::new();
    s += &drop("orders_reextract_message", &format!("AFTER UPDATE OF json ON messages WHEN {}", changed_sql(&["from", "subject", "account_id"])), "NEW.id");
    s += &drop("orders_reextract_body_insert", "AFTER INSERT ON message_bodies", "NEW.message_id");
    // An unchanged body downloaded again is no change.
    s += &drop("orders_reextract_body_update", "AFTER UPDATE ON message_bodies WHEN OLD.html IS NOT NEW.html OR OLD.text IS NOT NEW.text", "NEW.message_id");
    s += &drop("orders_reextract_body_delete", "AFTER DELETE ON message_bodies", "OLD.message_id");
    s += "CREATE TRIGGER IF NOT EXISTS orders_regroup_on_move AFTER UPDATE OF folder_id ON messages WHEN OLD.folder_id IS NOT NEW.folder_id BEGIN
            INSERT OR IGNORE INTO orders_dirty (key) SELECT key FROM order_keys WHERE message_id = NEW.id; END;\n";
    // A folder moved (deleted into Deleted Items, or out of it again): the
    // mail in it and in every folder under it.
    s += "CREATE TRIGGER IF NOT EXISTS orders_regroup_on_folder_move AFTER UPDATE OF parent_id ON folders WHEN OLD.parent_id IS NOT NEW.parent_id BEGIN
            INSERT OR IGNORE INTO orders_dirty (key) SELECT k.key FROM order_keys k JOIN messages m ON m.id = k.message_id
            WHERE m.folder_id IN (WITH RECURSIVE sub(id) AS (SELECT NEW.id UNION SELECT c.id FROM folders c JOIN sub ON c.parent_id = sub.id) SELECT id FROM sub); END;\n";
    // Also when a deleted message takes its keys with it (foreign key cascade).
    s += "CREATE TRIGGER IF NOT EXISTS orders_regroup_on_key_delete AFTER DELETE ON order_keys BEGIN
            INSERT OR IGNORE INTO orders_dirty (key) VALUES (OLD.key); END;\n";
    // The senders of rule `r` (NEW or OLD), for the next batch. Not `OR
    // IGNORE`: the statement that fires a trigger overrides its conflict
    // policy (an `INSERT OR REPLACE` of a rule would replace, an UPSERT
    // abort).
    let regate = |name: &str, when: &str, rules: &[&str]| {
        let mut body = String::new();
        for r in rules {
            body += &format!(
                "INSERT INTO orders_regate_senders (pattern, domain) SELECT {r}.pattern, {r}.domain
                 WHERE NOT EXISTS (SELECT 1 FROM orders_regate_senders WHERE pattern = {r}.pattern AND domain = {r}.domain);\n"
            );
        }
        format!("CREATE TRIGGER IF NOT EXISTS {name} {when} BEGIN\n{body}END;\n")
    };
    s += &regate("orders_regate_rule_insert", "AFTER INSERT ON junk_sender_rules", &["NEW"]);
    s += &regate(
        "orders_regate_rule_update",
        "AFTER UPDATE ON junk_sender_rules WHEN OLD.action IS NOT NEW.action OR OLD.pattern IS NOT NEW.pattern OR OLD.domain IS NOT NEW.domain
            OR OLD.account_key IS NOT NEW.account_key OR OLD.exact_domain IS NOT NEW.exact_domain",
        &["OLD", "NEW"],
    );
    s += &regate("orders_regate_rule_delete", "AFTER DELETE ON junk_sender_rules", &["OLD"]);
    s
}

/// Create the triggers. A later change to a trigger's body needs a
/// `DROP TRIGGER` gated on a higher [`SCHEMA_LEVEL`].
pub(crate) fn upgrade_schema_in(conn: &Connection) -> Result<()> {
    if kv_level(conn, SCHEMA_LEVEL)? < 2 {
        // Level 2: a changed junk sender rule is queued, not applied at once.
        for name in ["orders_regate_rule_insert", "orders_regate_rule_update", "orders_regate_rule_delete"] {
            conn.execute(&format!("DROP TRIGGER IF EXISTS {name}"), [])?;
        }
        conn.execute("INSERT OR REPLACE INTO kv (key, value) VALUES (?1, '2')", [SCHEMA_LEVEL])?;
    }
    conn.execute_batch(&triggers_sql())?;
    Ok(())
}

/// The account's changes to its orders (its orders and extractions go with
/// its messages).
pub(crate) fn remove_account_in(conn: &Connection, account: AccountId) -> Result<()> {
    let prefix = format!("{account}:");
    conn.execute("DELETE FROM order_user_state WHERE substr(key, 1, length(?1)) = ?1", [&prefix])?;
    conn.execute("DELETE FROM orders_dirty WHERE substr(key, 1, length(?1)) = ?1", [&prefix])?;
    Ok(())
}

/// The first column of every row.
fn column<T: FromSql>(conn: &Connection, sql: &str, p: impl rusqlite::Params) -> Result<Vec<T>> {
    let mut stmt = conn.prepare_cached(sql)?;
    let rows = stmt.query_map(p, |r| r.get::<_, T>(0))?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

/// Orders as stored; a row that doesn't parse is left out (it is rebuilt
/// from the mail).
fn orders_from(conn: &Connection, sql: &str, p: impl rusqlite::Params) -> Result<Vec<Order>> {
    Ok(column::<String>(conn, sql, p)?.into_iter().filter_map(|j| serde_json::from_str(&j).ok()).collect())
}

fn user_state_in(conn: &Connection, key: &str) -> Result<Option<OrderUserState>> {
    Ok(conn
        .query_row("SELECT key, hidden, status_override, updated_at FROM order_user_state WHERE key = ?1", [key], user_state_from_row)
        .optional()?)
}

fn user_state_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<OrderUserState> {
    Ok(OrderUserState {
        key: r.get(0)?,
        hidden: r.get(1)?,
        status_override: r.get::<_, Option<String>>(2)?.as_deref().and_then(OrderStatus::parse),
        updated_at: r.get(3)?,
    })
}

fn put_user_state_in(conn: &Connection, s: &OrderUserState) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO order_user_state (key, hidden, status_override, updated_at) VALUES (?1, ?2, ?3, ?4)",
        params![s.key, s.hidden, s.status_override.map(|o| o.as_str()), s.updated_at],
    )?;
    Ok(())
}

/// A user state from a backup; its order picks it up on the next rebuild.
pub(crate) fn restore_user_state_in(conn: &Connection, s: &OrderUserState) -> Result<()> {
    put_user_state_in(conn, s)?;
    conn.execute("INSERT OR IGNORE INTO orders_dirty (key) VALUES (?1)", [&s.key])?;
    Ok(())
}

/// An ignored sender from a backup (never a free mail domain); its mail is
/// looked at again.
pub(crate) fn restore_sender_ignore_in(conn: &Connection, i: &OrderSenderIgnore) -> Result<()> {
    let Ok((pattern, is_domain)) = new_ignore_pattern(&i.pattern) else { return Ok(()) };
    conn.execute(
        "INSERT OR REPLACE INTO order_sender_ignores (pattern, is_domain, created_at) VALUES (?1, ?2, ?3)",
        params![pattern, is_domain, i.created_at],
    )?;
    forget_sender_in(conn, &pattern, is_domain)
}

/// Every user state, for the backup.
pub(crate) fn user_states_in(conn: &Connection) -> Result<Vec<OrderUserState>> {
    let mut stmt = conn.prepare_cached("SELECT key, hidden, status_override, updated_at FROM order_user_state ORDER BY key")?;
    let rows = stmt.query_map([], user_state_from_row)?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

pub(crate) fn sender_ignores_in(conn: &Connection) -> Result<Vec<OrderSenderIgnore>> {
    let mut stmt = conn.prepare_cached("SELECT pattern, is_domain, created_at FROM order_sender_ignores ORDER BY pattern")?;
    let rows = stmt.query_map([], |r| Ok(OrderSenderIgnore { pattern: r.get(0)?, is_domain: r.get(1)?, created_at: r.get(2)? }))?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

fn is_ignored(ignores: &[OrderSenderIgnore], address: &str) -> bool {
    ignores.iter().any(|i| if i.is_domain { junk::domain_matches(junk::domain_of(address), &i.pattern) } else { i.pattern == address })
}

/// A sender's extractions are thrown away, to be made again (with or
/// without an ignore); their orders are regrouped.
fn forget_sender_in(conn: &Connection, pattern: &str, domain: bool) -> Result<()> {
    let matching = format!("SELECT m.id FROM messages m WHERE {}", sender_match_sql(domain, 1));
    conn.execute(&format!("DELETE FROM order_keys WHERE message_id IN ({matching})"), [pattern])?;
    conn.execute(&format!("DELETE FROM order_extractions WHERE message_id IN ({matching})"), [pattern])?;
    Ok(())
}

/// What every extraction of a batch needs, read once.
struct ExtractContext {
    junk: super::junk::CachedJunk,
    rules: Vec<SenderRule>,
    own: Vec<String>,
    contacts: HashSet<String>,
    ignores: Vec<OrderSenderIgnore>,
    /// (account, merchant key) of the orders already found.
    merchants: HashSet<(AccountId, String)>,
    /// ... of those with a tracking number a carrier confirms.
    trusted: HashSet<(AccountId, String)>,
}

/// One message's extraction, ready to write.
struct Extracted {
    extraction: Extraction,
    junk_version: String,
}

/// Write one extraction (an upsert, never a replace: the topic triggers
/// must see an order email turn into none) and its keys. The keys go dirty
/// whatever changed, so their orders pick up a new status or junk gate.
fn write_extraction_in(conn: &Connection, m: &Message, x: &Extracted, now: i64) -> Result<()> {
    let exists: bool = conn.query_row("SELECT EXISTS (SELECT 1 FROM messages WHERE id = ?1)", [m.id], |r| r.get(0))?;
    if !exists {
        return Ok(());
    }
    let ex = &x.extraction;
    conn.execute(
        "INSERT INTO order_extractions (message_id, account_id, version, junk_version, kind, join_only, confidence, status, json, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(message_id) DO UPDATE SET
            account_id = excluded.account_id, version = excluded.version, junk_version = excluded.junk_version,
            kind = excluded.kind, join_only = excluded.join_only, confidence = excluded.confidence,
            status = excluded.status, json = excluded.json, created_at = excluded.created_at",
        params![
            m.id,
            m.account_id,
            version(),
            x.junk_version,
            kind_str(ex.kind),
            // A tracking-only order email that can join an order but never start one.
            ex.kind == ExtractionKind::Order && orders::join_only(ex),
            f64::from(ex.confidence),
            ex.status.map(|s| s.as_str()),
            serde_json::to_string(ex)?,
            now
        ],
    )?;
    let mut keys = ex.keys(m.account_id);
    keys.sort();
    keys.dedup();
    let current: Vec<String> = column(conn, "SELECT key FROM order_keys WHERE message_id = ?1 ORDER BY key", [m.id])?;
    if current != keys {
        // The delete trigger marks the old keys dirty.
        conn.execute("DELETE FROM order_keys WHERE message_id = ?1", [m.id])?;
        for k in &keys {
            conn.execute("INSERT OR IGNORE INTO order_keys (key, message_id) VALUES (?1, ?2)", params![k, m.id])?;
        }
    }
    for k in &keys {
        conn.execute("INSERT OR IGNORE INTO orders_dirty (key) VALUES (?1)", [k])?;
    }
    Ok(())
}

/// The keys and messages one rebuild has taken, and the orders they were in.
#[derive(Default)]
struct Walk {
    /// Keys taken (no longer dirty once the rebuild is written).
    keys: Vec<String>,
    queued: HashSet<String>,
    messages: Vec<MessageId>,
    seen: HashSet<MessageId>,
    /// The orders to replace.
    old: HashSet<i64>,
}

impl Walk {
    fn queue(&mut self, queue: &mut VecDeque<String>, key: String) {
        if self.queued.insert(key.clone()) {
            queue.push_back(key);
        }
    }

    /// Follow one group from `seed` to its end: every eligible message of a
    /// key, every key of those messages, and every key of the orders they
    /// were in (so an order is only ever replaced whole). A group past its
    /// cap is cut there: the keys it didn't get to are taken too, so the
    /// same group isn't walked again and again.
    fn group(&mut self, conn: &Connection, seed: &str) -> Result<()> {
        let mut by_key = conn.prepare_cached(&eligible_by_key_sql())?;
        let mut keys_of = conn.prepare_cached("SELECT key FROM order_keys WHERE message_id = ?1 ORDER BY key")?;
        let mut order_of_key = conn.prepare_cached("SELECT order_id FROM order_group_keys WHERE key = ?1")?;
        let mut orders_of_message = conn.prepare_cached("SELECT order_id FROM order_messages WHERE message_id = ?1")?;
        let mut keys_of_order = conn.prepare_cached("SELECT key FROM order_group_keys WHERE order_id = ?1 ORDER BY key")?;
        let mut queue = VecDeque::new();
        self.queue(&mut queue, seed.to_string());
        let (mut keys, mut messages) = (0, 0);
        'walk: while let Some(key) = queue.pop_front() {
            if keys >= MAX_GROUP_KEYS {
                queue.push_front(key);
                break;
            }
            keys += 1;
            let ids: Vec<MessageId> = by_key.query_map([&key], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
            let mut orders: Vec<i64> = order_of_key.query_map([&key], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
            self.keys.push(key);
            for id in ids {
                if self.seen.contains(&id) {
                    continue;
                }
                if messages >= MAX_GROUP_MESSAGES {
                    break 'walk;
                }
                messages += 1;
                self.seen.insert(id);
                self.messages.push(id);
                for k in keys_of.query_map([id], |r| r.get::<_, String>(0))? {
                    self.queue(&mut queue, k?);
                }
                for o in orders_of_message.query_map([id], |r| r.get::<_, i64>(0))? {
                    orders.push(o?);
                }
            }
            // A key that went away still reaches the rest of its order.
            for o in orders {
                if self.old.insert(o) {
                    for k in keys_of_order.query_map([o], |r| r.get::<_, String>(0))? {
                        self.queue(&mut queue, k?);
                    }
                }
            }
        }
        self.keys.extend(queue);
        Ok(())
    }
}

/// Rebuild the orders `seeds` (keys) are part of, or were part of before:
/// follow each seed's group through every eligible message and its keys,
/// group what is found, and replace the old orders with the new ones. Seeds
/// past the batch's budget are left dirty for the next call; a group is
/// never cut off at the budget. Writes only the order tables (never
/// `order_keys`), so it can't queue itself again. Returns the keys it got
/// to (no longer dirty).
fn rebuild_in(conn: &Connection, seeds: &[String]) -> Result<Vec<String>> {
    let mut walk = Walk::default();
    for seed in seeds {
        if walk.queued.contains(seed) {
            continue;
        }
        if !walk.keys.is_empty() && walk.messages.len() >= MAX_REBUILD_MESSAGES {
            break;
        }
        walk.group(conn, seed)?;
    }
    let Walk { keys: visited_keys, messages, old, .. } = walk;

    let mut entries = Vec::with_capacity(messages.len());
    if !messages.is_empty() {
        let mut stmt = conn.prepare(&format!(
            "SELECT m.id, m.account_id, m.received_at, COALESCE(m.internet_message_id, ''), COALESCE(json_extract(m.json, '$.subject'), ''), e.json
             FROM messages m JOIN order_extractions e ON e.message_id = m.id WHERE m.id IN ({})",
            id_list(&messages)
        ))?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, MessageId>(0)?, r.get::<_, AccountId>(1)?, r.get::<_, i64>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?, r.get::<_, String>(5)?))
        })?;
        for row in rows {
            let (message_id, account_id, at, internet_message_id, subject, json) = row?;
            let Ok(extraction) = serde_json::from_str::<Extraction>(&json) else { continue };
            let received_at = DateTime::from_timestamp(at, 0).unwrap_or_default();
            entries.push(Entry { message_id, account_id, received_at, internet_message_id, subject, extraction });
        }
    }
    let groups = orders::group(entries);

    // The user's changes, matched by any key of a group.
    let mut all_keys: Vec<String> = visited_keys.clone();
    all_keys.extend(groups.iter().flat_map(|g| g.keys.iter().cloned()));
    all_keys.sort();
    all_keys.dedup();
    let mut states: HashMap<String, OrderUserState> = HashMap::new();
    for chunk in all_keys.chunks(500) {
        let mut stmt =
            conn.prepare(&format!("SELECT key, hidden, status_override, updated_at FROM order_user_state WHERE key IN ({})", marks(chunk.len())))?;
        for s in stmt.query_map(params_from_iter(chunk), user_state_from_row)? {
            let s = s?;
            states.insert(s.key.clone(), s);
        }
    }

    let built: Vec<(&orders::Group, Order)> = groups
        .iter()
        .map(|g| (g, orders::build_order(g, merged_state(&g.keys, &states).as_ref())))
        .filter(|(_, o)| !o.key.is_empty())
        .collect();
    // How well each merchant is known before the old orders go (see
    // `reread_merchants_in`).
    let mut merchants: HashMap<(AccountId, String), u8> = HashMap::new();
    for (_, o) in &built {
        let k = (o.account_id, o.merchant.key.clone());
        if let hash_map::Entry::Vacant(e) = merchants.entry(k) {
            let known = merchant_level_in(conn, e.key().0, &e.key().1)?;
            e.insert(known);
        }
    }

    // Every order these messages and keys were in goes; the groups replace them.
    if !old.is_empty() {
        let list = old.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",");
        conn.execute(&format!("DELETE FROM orders WHERE id IN ({list})"), [])?;
    }
    for (g, mut o) in built {
        conn.execute("DELETE FROM orders WHERE key = ?1", [&o.key])?;
        conn.execute(
            "INSERT INTO orders (account_id, key, status, hidden, updated_at, delivered_at, json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, '{}')",
            params![o.account_id, o.key, o.status.as_str(), o.hidden, o.updated_at.timestamp(), o.delivered_at.map(|d| d.timestamp())],
        )?;
        o.id = conn.last_insert_rowid();
        conn.execute("UPDATE orders SET json = ?1 WHERE id = ?2", params![serde_json::to_string(&o)?, o.id])?;
        let ids: HashSet<MessageId> = g.entries.iter().map(|e| e.message_id).chain(o.events.iter().map(|e| e.message_id)).collect();
        for id in ids {
            conn.execute("INSERT OR IGNORE INTO order_messages (order_id, message_id) VALUES (?1, ?2)", params![o.id, id])?;
        }
        for k in g.keys.iter().chain(&o.keys) {
            conn.execute("INSERT OR IGNORE INTO order_group_keys (key, order_id) VALUES (?1, ?2)", params![k, o.id])?;
        }
    }
    for chunk in visited_keys.chunks(500) {
        conn.execute(&format!("DELETE FROM orders_dirty WHERE key IN ({})", marks(chunk.len())), params_from_iter(chunk))?;
    }
    reread_merchants_in(conn, &merchants)?;
    Ok(visited_keys)
}

/// How well the user knows a merchant: 0 no order from it, 1 an order, 2
/// an order a carrier's own mail confirms (see
/// `orders::Input::known_merchant` and `trusted_merchant`).
fn merchant_level_in(conn: &Connection, account: AccountId, key: &str) -> Result<u8> {
    let level: Option<i64> = conn.query_row(
        &format!("SELECT MAX(1 + {}) FROM orders WHERE account_id = ?1 AND json_extract(json, '$.merchant.key') = ?2", carrier_confirmed_sql()),
        params![account, key],
        |r| r.get(0),
    )?;
    Ok(level.unwrap_or(0) as u8)
}

/// Mail is read newest first, so a shop's later mail can be read before
/// the order it belongs to exists, when the shop isn't known yet. When a
/// rebuild makes a merchant better known than it was (`before`), that
/// merchant's mail that wasn't taken for an order email (within the
/// horizon) is read again.
fn reread_merchants_in(conn: &Connection, before: &HashMap<(AccountId, String), u8>) -> Result<()> {
    let since = horizon(Utc::now());
    for ((account, key), was) in before {
        if merchant_level_in(conn, *account, key)? <= *was {
            continue;
        }
        conn.execute(
            "DELETE FROM order_extractions WHERE account_id = ?1 AND kind != 'order' AND json_extract(json, '$.merchant.key') = ?2
               AND message_id IN (SELECT id FROM messages WHERE received_at >= ?3)",
            params![account, key, since],
        )?;
    }
    Ok(())
}

/// The order emails (and suppressed ones) from the senders of the junk
/// sender rules queued since the last batch, dropped so they are read again
/// under the rules as they are now: one pass over the order emails for all
/// of them. A domain rule's subdomains too, though it may match only the
/// domain itself: reading one again is harmless.
fn drain_regate_in(conn: &Connection) -> Result<()> {
    let queued: bool = conn.query_row("SELECT EXISTS (SELECT 1 FROM orders_regate_senders)", [], |r| r.get(0))?;
    if !queued {
        return Ok(());
    }
    let covered = format!(
        "SELECT e.message_id FROM order_extractions e JOIN messages m ON m.id = e.message_id
         WHERE e.kind IN ('order', 'suppressed') AND EXISTS (SELECT 1 FROM orders_regate_senders r
           WHERE CASE WHEN r.domain THEN {} ELSE {} END)",
        sender_domain_sql("r.pattern"),
        sender_address_sql("r.pattern")
    );
    conn.execute_batch(&format!(
        "DELETE FROM order_keys WHERE message_id IN ({covered});
         DELETE FROM order_extractions WHERE message_id IN ({covered});
         DELETE FROM orders_regate_senders;"
    ))?;
    Ok(())
}

/// Everything found, as when order tracking is switched off (see
/// [`Store::clear_orders`]): the topics worked out from what detection
/// found are sorted again too.
pub(crate) fn clear_in(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "DELETE FROM order_keys;
         DELETE FROM order_extractions;
         DELETE FROM order_messages;
         DELETE FROM order_group_keys;
         DELETE FROM orders;
         DELETE FROM orders_dirty;
         DELETE FROM orders_regate_senders;",
    )?;
    Ok(())
}

/// Everything found, to be found again with order tracking on ("Look
/// through my mail again"). Mail that wasn't an order email is marked for
/// another look instead of deleted: a deleted extraction takes the
/// message's topic with it (see `topics_follow_order_delete`), and almost
/// all mail is no order email. Its topic only changes if the new look
/// finds an order after all (`topics_follow_order_update`).
fn rescan_in(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "UPDATE order_extractions SET version = '' WHERE kind = 'none';
         DELETE FROM order_keys;
         DELETE FROM order_extractions WHERE kind != 'none';
         DELETE FROM order_messages;
         DELETE FROM order_group_keys;
         DELETE FROM orders;
         DELETE FROM orders_dirty;",
    )?;
    Ok(())
}

/// The user's changes to the order `key` is in, merged from all its keys'.
fn group_state_in(conn: &Connection, key: &str) -> Result<Option<OrderUserState>> {
    let mut keys: Vec<String> = column(
        conn,
        "SELECT g2.key FROM order_group_keys g1 JOIN order_group_keys g2 ON g2.order_id = g1.order_id WHERE g1.key = ?1",
        [key],
    )?;
    keys.push(key.to_string());
    let mut states = HashMap::new();
    for k in &keys {
        if let Some(s) = user_state_in(conn, k)? {
            states.insert(k.clone(), s);
        }
    }
    Ok(merged_state(&keys, &states))
}

/// The user's changes to a group, from the states of any of its keys:
/// hidden if any says so, the status of the latest change.
fn merged_state(keys: &[String], states: &HashMap<String, OrderUserState>) -> Option<OrderUserState> {
    let matched: Vec<&OrderUserState> = keys.iter().filter_map(|k| states.get(k)).collect();
    let newest = matched.iter().max_by(|a, b| a.updated_at.cmp(&b.updated_at).then_with(|| b.key.cmp(&a.key)))?;
    Some(OrderUserState { hidden: matched.iter().any(|s| s.hidden), ..(*newest).clone() })
}

/// An ignore pattern as typed: an address or a domain.
fn ignore_pattern(input: &str) -> Result<(String, bool)> {
    junk::normalize_pattern(input).ok_or_else(|| Error::Invalid(format!("\u{201c}{}\u{201d} isn't an email address or domain", input.trim())))
}

/// A new ignore: as [`ignore_pattern`], but never a free mail domain (it
/// would stop tracking every seller there).
fn new_ignore_pattern(input: &str) -> Result<(String, bool)> {
    let (pattern, is_domain) = ignore_pattern(input)?;
    if is_domain && crate::topics::is_freemail(&pattern) {
        return Err(Error::Invalid(format!("{pattern} is used by lots of people. Choose an address there instead.")));
    }
    Ok((pattern, is_domain))
}

impl Store {
    /// Extract up to `limit` messages that have no current extraction,
    /// newest first. Returns how many were done; call again until it
    /// returns 0. Nothing is done while order tracking is off.
    pub fn extract_orders_pending(&self, limit: u32, now: DateTime<Utc>) -> Result<usize> {
        if !self.settings().order_tracking {
            return Ok(0);
        }
        self.tx(|tx| drain_regate_in(tx))?;
        let ids: Vec<MessageId> =
            column(&self.conn(), &format!("{} ORDER BY m.received_at DESC LIMIT ?2", pending_sql()), params![version(), limit, horizon(now)])?;
        if ids.is_empty() {
            return Ok(0);
        }
        let msgs = self.messages_by_ids(&ids)?;
        let ctx = self.extract_context(&ids)?;
        let mut done = Vec::with_capacity(msgs.len());
        for m in &msgs {
            done.push(self.extract_one(m, &ctx)?);
        }
        let at = Utc::now().timestamp();
        #[cfg(test)]
        super::run_meanwhile(self);
        self.tx(|tx| {
            // Switched off, a sender ignored (or tracked again), or a junk
            // allow or block rule changed while the batch was worked out: it
            // is stale, and the next batch redoes it.
            if !settings_in(tx).order_tracking || sender_ignores_in(tx)? != ctx.ignores || super::junk::rules_in(tx)? != ctx.rules {
                return Ok(());
            }
            for (m, x) in msgs.iter().zip(&done) {
                write_extraction_in(tx, m, x, at)?;
            }
            Ok(())
        })?;
        Ok(msgs.len())
    }

    fn extract_context(&self, ids: &[MessageId]) -> Result<ExtractContext> {
        let conn = self.conn();
        let (mut merchants, mut trusted) = (HashSet::new(), HashSet::new());
        {
            let mut stmt = conn.prepare_cached(&format!(
                "SELECT account_id, json_extract(json, '$.merchant.key'), MAX({})
                 FROM orders WHERE json_extract(json, '$.merchant.key') IS NOT NULL GROUP BY 1, 2",
                carrier_confirmed_sql()
            ))?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, AccountId>(0)?, r.get::<_, String>(1)?, r.get::<_, bool>(2)?)))?;
            for row in rows {
                let (account, key, confirmed) = row?;
                if confirmed {
                    trusted.insert((account, key.clone()));
                }
                merchants.insert((account, key));
            }
        }
        Ok(ExtractContext {
            junk: super::junk::cached_junk_in(&conn, ids)?,
            rules: super::junk::rules_in(&conn)?,
            own: column(&conn, "SELECT lower(json_extract(json, '$.email')) FROM accounts WHERE json_extract(json, '$.email') IS NOT NULL", [])?,
            contacts: column::<String>(
                &conn,
                "SELECT lower(json_extract(e.value, '$.value')) FROM contacts c, json_each(c.json, '$.emails') e
                 WHERE json_extract(e.value, '$.value') IS NOT NULL",
                [],
            )?
            .into_iter()
            .collect(),
            ignores: sender_ignores_in(&conn)?,
            merchants,
            trusted,
        })
    }

    /// One message's extraction, from stored data only (the cached junk
    /// check; the check itself is never run here).
    fn extract_one(&self, m: &Message, ctx: &ExtractContext) -> Result<Extracted> {
        let body = self.message_body(m.id)?.unwrap_or_default();
        let address = m.from.address.trim().to_lowercase();
        let (junk_version, junk, reported_phishing, impersonation, auth) = ctx.junk.get(&m.id).cloned().unwrap_or_default();
        let sender_allowed = junk::decide(&ctx.rules, m.account_id, &address).is_some_and(|r| r.action() == RuleAction::Allow);
        let merchant = (m.account_id, orders::merchant_key(&m.from));
        let extraction = orders::extract_guarded(&orders::Input {
            message: m,
            html: body.html.as_deref(),
            text: &body.text,
            junk,
            impersonation,
            reported_phishing,
            sender_allowed,
            sender_authenticated: auth.vouches_for(junk::domain_of(&address)),
            own_addresses: &ctx.own,
            contacts: &ctx.contacts,
            known_merchant: ctx.merchants.contains(&merchant),
            trusted_merchant: ctx.trusted.contains(&merchant),
            ignored: is_ignored(&ctx.ignores, &address),
        });
        Ok(Extracted { extraction, junk_version })
    }

    /// Rebuild the orders of up to `limit` changed keys. Returns how many
    /// keys were taken; call again until it returns 0.
    pub fn rebuild_dirty_orders(&self, limit: u32) -> Result<usize> {
        let seeds: Vec<String> = column(&self.conn(), "SELECT key FROM orders_dirty ORDER BY key LIMIT ?1", [limit])?;
        if seeds.is_empty() {
            return Ok(0);
        }
        let taken = self.tx(|tx| rebuild_in(tx, &seeds))?;
        Ok(seeds.iter().filter(|s| taken.contains(s)).count())
    }

    /// How many messages still need extracting, counted up to `cap`.
    pub fn orders_pending_count(&self, cap: u32, now: DateTime<Utc>) -> Result<u32> {
        if !self.settings().order_tracking {
            return Ok(0);
        }
        let n: i64 = self
            .conn()
            .query_row(&format!("SELECT COUNT(*) FROM ({} LIMIT ?2)", pending_sql()), params![version(), cap, horizon(now)], |r| r.get(0))?;
        Ok(n as u32)
    }

    /// How far the workers have got; all zero while Packages is off.
    pub fn orders_progress(&self, now: DateTime<Utc>) -> Result<OrdersProgress> {
        if !self.settings().order_tracking {
            return Ok(OrdersProgress::default());
        }
        let left = self.orders_pending_count(u32::MAX, now)?;
        let conn = self.conn();
        let total = format!("SELECT COUNT(*) FROM messages m WHERE m.folder_id NOT IN {} AND m.is_draft = 0 AND m.received_at >= ?1", own_mail_folders_sql());
        let total: i64 = conn.query_row(&total, [horizon(now)], |r| r.get(0))?;
        let regrouping: i64 = conn.query_row("SELECT COUNT(*) FROM orders_dirty", [], |r| r.get(0))?;
        Ok(OrdersProgress { total: total as u32, left, regrouping: regrouping as u32 })
    }

    /// Every order updated within 400 days, newest first.
    pub fn orders(&self, include_hidden: bool) -> Result<Vec<Order>> {
        let sql = if include_hidden {
            "SELECT json FROM orders WHERE updated_at >= ?1 ORDER BY updated_at DESC, id DESC"
        } else {
            "SELECT json FROM orders WHERE updated_at >= ?1 AND hidden = 0 ORDER BY updated_at DESC, id DESC"
        };
        orders_from(&self.conn(), sql, [horizon(Utc::now())])
    }

    pub fn orders_summary(&self, now: DateTime<Utc>, today: NaiveDate, keep_days: u32) -> Result<OrdersSummary> {
        let mut s = OrdersSummary::default();
        for o in self.orders(false)? {
            match orders::section(&o, now, today, keep_days) {
                Some(OrderSection::NeedsAttention) => {
                    s.active += 1;
                    s.attention += 1;
                }
                Some(OrderSection::ArrivingSoon | OrderSection::OnTheWay | OrderSection::Ordered) => s.active += 1,
                _ => {}
            }
        }
        Ok(s)
    }

    /// The order with this key: its canonical key, or any key of its group
    /// (a rebuild can change which key is canonical).
    pub fn order_by_key(&self, key: &str) -> Result<Option<Order>> {
        Ok(orders_from(
            &self.conn(),
            "SELECT json FROM orders WHERE key = ?1
             UNION ALL SELECT o.json FROM order_group_keys g JOIN orders o ON o.id = g.order_id WHERE g.key = ?1
             LIMIT 1",
            [key],
        )?
        .into_iter()
        .next())
    }

    /// The visible order a message belongs to (cached only).
    pub fn order_for_message(&self, id: MessageId) -> Result<Option<Order>> {
        Ok(orders_from(
            &self.conn(),
            "SELECT o.json FROM order_messages om JOIN orders o ON o.id = om.order_id WHERE om.message_id = ?1 AND o.hidden = 0 LIMIT 1",
            [id],
        )?
        .into_iter()
        .next())
    }

    /// A message's extraction (cached only).
    pub fn extraction_for(&self, id: MessageId) -> Result<Option<Extraction>> {
        let json: Option<String> =
            self.conn().query_row("SELECT json FROM order_extractions WHERE message_id = ?1", [id], |r| r.get(0)).optional()?;
        Ok(json.and_then(|j| serde_json::from_str(&j).ok()))
    }

    /// Change an order (by its canonical key). Returns the previous state,
    /// for Undo. The order is rebuilt with the change at once.
    pub fn set_order_state(&self, key: &str, change: OrderStateChange) -> Result<Option<OrderUserState>> {
        let now = Utc::now().timestamp();
        self.tx(|tx| {
            let previous = user_state_in(tx, key)?;
            // A key's first change starts from what the order's other keys
            // say, so it doesn't undo a change saved under one of them.
            let mut s = match &previous {
                Some(s) => s.clone(),
                None => OrderUserState { key: key.into(), ..group_state_in(tx, key)?.unwrap_or_default() },
            };
            match change {
                OrderStateChange::Hide(hidden) => {
                    s.hidden = hidden;
                    if !hidden {
                        // Shown again: no other key of the order keeps it hidden.
                        tx.execute(
                            "UPDATE order_user_state SET hidden = 0, updated_at = ?2 WHERE hidden = 1 AND key IN
                                (SELECT g2.key FROM order_group_keys g1 JOIN order_group_keys g2 ON g2.order_id = g1.order_id WHERE g1.key = ?1)",
                            params![key, now],
                        )?;
                    }
                }
                OrderStateChange::Override(status) => s.status_override = status,
            }
            s.updated_at = now;
            put_user_state_in(tx, &s)?;
            rebuild_in(tx, &[key.to_string()])?;
            Ok(previous)
        })
    }

    /// Undo [`Store::set_order_state`].
    pub fn restore_order_state(&self, key: &str, previous: Option<OrderUserState>) -> Result<()> {
        self.tx(|tx| {
            match &previous {
                Some(s) => put_user_state_in(tx, s)?,
                None => {
                    tx.execute("DELETE FROM order_user_state WHERE key = ?1", [key])?;
                }
            }
            rebuild_in(tx, &[key.to_string()])?;
            Ok(())
        })
    }

    /// Put every hidden order back. Returns how many Packages lists again
    /// (orders past the 400 days it lists aren't counted).
    pub fn unhide_all_orders(&self) -> Result<usize> {
        let now = Utc::now().timestamp();
        self.tx(|tx| {
            let listed: i64 = tx.query_row("SELECT COUNT(*) FROM orders WHERE hidden = 1 AND updated_at >= ?1", [horizon(Utc::now())], |r| r.get(0))?;
            let hidden: Vec<String> = column(tx, "SELECT key FROM orders WHERE hidden = 1", [])?;
            tx.execute("UPDATE order_user_state SET hidden = 0, updated_at = ?1 WHERE hidden = 1", [now])?;
            if hidden.is_empty() {
                return Ok(0);
            }
            // Shown at once; the orders a rebuild doesn't get to in this
            // batch are rebuilt by the background pass.
            tx.execute("UPDATE orders SET hidden = 0, json = json_set(json, '$.hidden', json('false')) WHERE hidden = 1", [])?;
            let taken = rebuild_in(tx, &hidden)?;
            for k in hidden.iter().filter(|k| !taken.contains(k)) {
                tx.execute("INSERT OR IGNORE INTO orders_dirty (key) VALUES (?1)", [k])?;
            }
            Ok(listed as usize)
        })
    }

    /// Hidden orders that Packages would list (updated within 400 days).
    pub fn hidden_order_count(&self) -> Result<u32> {
        let n: i64 = self.conn().query_row("SELECT COUNT(*) FROM orders WHERE hidden = 1 AND updated_at >= ?1", [horizon(Utc::now())], |r| r.get(0))?;
        Ok(n as u32)
    }

    /// Stop tracking orders from `pattern` (`"a@b.example"` or `"b.example"`).
    /// The sender's mail is looked at again, and found to be ignored.
    pub fn ignore_order_sender(&self, pattern: &str) -> Result<()> {
        let (pattern, is_domain) = new_ignore_pattern(pattern)?;
        let now = Utc::now().timestamp();
        self.tx(|tx| {
            tx.execute(
                "INSERT OR IGNORE INTO order_sender_ignores (pattern, is_domain, created_at) VALUES (?1, ?2, ?3)",
                params![pattern, is_domain, now],
            )?;
            forget_sender_in(tx, &pattern, is_domain)
        })
    }

    /// Track orders from `pattern` again; its mail is looked at again.
    pub fn remove_order_sender_ignore(&self, pattern: &str) -> Result<()> {
        let (pattern, is_domain) = ignore_pattern(pattern)?;
        self.tx(|tx| {
            tx.execute("DELETE FROM order_sender_ignores WHERE pattern = ?1", [&pattern])?;
            forget_sender_in(tx, &pattern, is_domain)
        })
    }

    pub fn order_sender_ignores(&self) -> Result<Vec<OrderSenderIgnore>> {
        sender_ignores_in(&self.conn())
    }

    /// Forget everything found (it is found again from the mail while
    /// order tracking is on); keeps the user's changes and ignored senders.
    pub fn clear_orders(&self) -> Result<()> {
        self.tx(|tx| if settings_in(tx).order_tracking { rescan_in(tx) } else { clear_in(tx) })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;
    use crate::junk::analyze::{Analysis, Finding, Severity};
    use crate::model::*;
    use crate::store::test_util::{during_next_batch, messages_json, settle, store_with_account};
    use crate::topics::{Source, Topic};

    pub(crate) fn folder(store: &Store, acct: AccountId, role: FolderRole) -> FolderId {
        store.folder_for_role(acct, role).unwrap()
    }

    /// A message received `minutes` ago with an HTML body, in the Inbox.
    pub(crate) fn mail(store: &Store, acct: AccountId, from: (&str, &str), subject: &str, html: &str, minutes: i64) -> MessageId {
        mail_with_auth(store, acct, from, subject, html, minutes, None)
    }

    /// [`mail`] with an `Authentication-Results` header.
    fn mail_with_auth(store: &Store, acct: AccountId, from: (&str, &str), subject: &str, html: &str, minutes: i64, auth: Option<&str>) -> MessageId {
        static N: AtomicU32 = AtomicU32::new(0);
        let mut m = Message {
            auth_results: auth.map(str::to_string),
            account_id: acct,
            folder_id: folder(store, acct, FolderRole::Inbox),
            from: EmailAddress::new(from.0, from.1),
            subject: subject.into(),
            internet_message_id: format!("m{}@test.example", N.fetch_add(1, Ordering::Relaxed)),
            received_at: Utc::now() - Duration::minutes(minutes),
            ..Default::default()
        };
        let body = MessageBody { html: Some(html.into()), text: crate::html::html_to_text(html), ..Default::default() };
        store.insert_message(&mut m, Some(&body)).unwrap()
    }

    /// Ordinary mail that isn't about an order.
    pub(crate) fn plain(store: &Store, acct: AccountId, i: usize) -> MessageId {
        mail(store, acct, ("", &format!("p{i}@normal.example")), &format!("note {i}"), "<p>hello</p>", i as i64)
    }

    const UPS: &str = "1Z999AA10123456784";
    const UPS2: &str = "1Z4X7Q203660123459";
    const BRIGHTLEAF: (&str, &str) = ("Brightleaf Home", "orders@brightleaf.example");

    fn confirmation(store: &Store, acct: AccountId, minutes: i64) -> MessageId {
        mail(
            store,
            acct,
            BRIGHTLEAF,
            "Order confirmed: #BL-48213",
            "<p>Thanks for your order!</p><p>Order number: BL-48213</p>
             <table><tr><th>Item</th><th>Qty</th><th>Price</th></tr>
             <tr><td>Linen throw blanket</td><td>1</td><td>&#36;48.00</td></tr></table>",
            minutes,
        )
    }

    fn shipped(store: &Store, acct: AccountId, number: &str, minutes: i64) -> MessageId {
        mail(
            store,
            acct,
            BRIGHTLEAF,
            "Your Brightleaf order #BL-48213 has shipped",
            &format!("<p>Order number: BL-48213</p><p>Shipped with UPS. Tracking number: {number}</p>"),
            minutes,
        )
    }

    fn carrier_out_for_delivery(store: &Store, acct: AccountId, number: &str, minutes: i64) -> MessageId {
        mail(
            store,
            acct,
            ("UPS", "mcinfo@ups.com"),
            "Your UPS package is out for delivery",
            &format!("<p>Your package is out for delivery today.</p><p>Tracking number: {number}</p>"),
            minutes,
        )
    }

    /// The account's mail comes from Gmail, whose `Authentication-Results`
    /// are believed.
    fn on_gmail(store: &Store, acct: AccountId) {
        store.conn().execute("UPDATE accounts SET json = json_set(json, '$.config', json('{\"imap\":{\"host\":\"imap.gmail.com\"}}')) WHERE id = ?1", [acct]).unwrap();
    }

    /// [`carrier_out_for_delivery`], which Gmail vouches came from ups.com
    /// (see [`on_gmail`]).
    fn vouched_out_for_delivery(store: &Store, acct: AccountId, number: &str, minutes: i64) -> MessageId {
        mail_with_auth(
            store,
            acct,
            ("UPS", "mcinfo@ups.com"),
            "Your UPS package is out for delivery",
            &format!("<p>Your package is out for delivery today.</p><p>Tracking number: {number}</p>"),
            minutes,
            Some("mx.google.com; dkim=pass header.d=ups.com; spf=pass smtp.mailfrom=ups.com; dmarc=pass header.from=ups.com"),
        )
    }

    fn delivered(store: &Store, acct: AccountId, number: &str, minutes: i64) -> MessageId {
        mail(
            store,
            acct,
            BRIGHTLEAF,
            "Delivered: your Brightleaf order #BL-48213",
            &format!("<p>Your package was delivered. Order number: BL-48213. UPS tracking number {number}</p>"),
            minutes,
        )
    }

    fn order_key(acct: AccountId) -> String {
        format!("{acct}:o:brightleaf.example:BL-48213")
    }

    fn only_order(store: &Store) -> Order {
        let orders = store.orders(true).unwrap();
        assert_eq!(orders.len(), 1, "{orders:#?}");
        orders.into_iter().next().unwrap()
    }

    fn mark_cached(store: &Store) {
        store.conn().execute_batch("UPDATE order_extractions SET created_at = 1; UPDATE topic_assignments SET created_at = 1;").unwrap();
    }

    /// (extraction still cached, topic still cached) since [`mark_cached`].
    fn still_cached(store: &Store, id: MessageId) -> (bool, bool) {
        let conn = store.conn();
        let one = |sql: &str| conn.query_row(sql, [id], |r| r.get::<_, i64>(0)).optional().unwrap() == Some(1);
        (
            one("SELECT created_at FROM order_extractions WHERE message_id = ?1"),
            one("SELECT created_at FROM topic_assignments WHERE message_id = ?1"),
        )
    }

    fn count(store: &Store, sql: &str) -> i64 {
        store.conn().query_row(sql, [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn schema_upgrade_is_idempotent() {
        let dir = std::env::temp_dir().join(format!("axentra-orders-schema-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("axentra.db");
        let objects = |store: &Store| -> Vec<String> {
            let conn = store.conn();
            let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY name").unwrap();
            stmt.query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect()
        };
        let first = objects(&Store::open(&path).unwrap());
        assert_eq!(objects(&Store::open(&path).unwrap()), first, "opening again changes nothing");
        for name in [
            "messages_received_at",
            "order_extractions",
            "order_keys",
            "orders",
            "order_messages",
            "order_group_keys",
            "orders_dirty",
            "order_user_state",
            "order_sender_ignores",
            "topic_assignments",
            "topic_sender_rules",
            "topic_message_overrides",
            "topic_dirty_senders",
            "orders_reextract_message",
            "orders_reextract_body_update",
            "orders_regroup_on_move",
            "orders_regroup_on_key_delete",
            "orders_regate_rule_update",
            "topics_reclassify_message",
            "topics_follow_order_update",
            "topics_follow_order_delete",
            "topics_people_contact_update",
            // Junk's own, alongside.
            "junk_recheck_message",
            "junk_recheck_body_update",
            "junk_labels_forget_message",
        ] {
            assert!(first.contains(&name.to_string()), "{name} missing");
        }
        // A database from before packages and topics: none of it there.
        {
            let conn = Connection::open(&path).unwrap();
            for name in &first {
                if name.starts_with("orders_") || name.starts_with("topics_") {
                    conn.execute_batch(&format!("DROP TRIGGER IF EXISTS {name}")).unwrap();
                }
            }
            for table in [
                "order_messages",
                "order_group_keys",
                "orders",
                "order_keys",
                "order_extractions",
                "orders_dirty",
                "order_user_state",
                "order_sender_ignores",
                "topic_assignments",
                "topic_sender_rules",
                "topic_message_overrides",
                "topic_dirty_senders",
                "orders_regate_senders",
            ] {
                conn.execute_batch(&format!("DROP TABLE {table}")).unwrap();
            }
            conn.execute_batch("DROP INDEX messages_received_at; DELETE FROM kv WHERE key IN ('orders.schema', 'topics.schema');").unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(objects(&store), first, "upgraded to the same schema");
        assert_eq!((store.kv_get("orders.schema").unwrap(), store.kv_get("topics.schema").unwrap()), (Some("2".into()), Some("3".into())));
        drop(store);
        // A database from the levels before (orders 1, topics 2): a topic
        // table without `stale`, and triggers with the old bodies.
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "DROP TABLE topic_assignments;
                 CREATE TABLE topic_assignments (message_id INTEGER PRIMARY KEY, version TEXT NOT NULL, junk_version TEXT NOT NULL DEFAULT '',
                   topic TEXT NOT NULL, confidence REAL NOT NULL, source TEXT NOT NULL, json TEXT NOT NULL, created_at INTEGER NOT NULL);
                 DROP TRIGGER orders_regate_rule_insert;
                 CREATE TRIGGER orders_regate_rule_insert AFTER INSERT ON junk_sender_rules BEGIN DELETE FROM orders_dirty; END;
                 DROP TRIGGER topics_follow_order_delete;
                 CREATE TRIGGER topics_follow_order_delete AFTER DELETE ON order_extractions BEGIN DELETE FROM topic_assignments; END;
                 UPDATE kv SET value = '1' WHERE key = 'orders.schema'; UPDATE kv SET value = '2' WHERE key = 'topics.schema';",
            )
            .unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert!(super::super::has_column(&store.conn(), "topic_assignments", "stale").unwrap());
        let body = |name: &str| -> String { store.conn().query_row("SELECT sql FROM sqlite_master WHERE name = ?1", [name], |r| r.get(0)).unwrap() };
        assert!(body("orders_regate_rule_insert").contains("orders_regate_senders"));
        assert!(body("topics_follow_order_delete").contains("stale = 1"));
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pending_drains_and_is_idempotent() {
        let (store, acct) = store_with_account();
        let n = 4 * 250 + 1;
        let ids: Vec<MessageId> = (0..n).map(|i| plain(&store, acct, i)).collect();
        // Mail past the horizon is never looked at for orders.
        let old = mail(&store, acct, ("", "old@normal.example"), "old", "<p>old</p>", 401 * 1440);
        let now = Utc::now();
        assert_eq!(store.orders_pending_count(10_000, now).unwrap(), n as u32);
        assert_eq!(store.orders_pending_count(10, now).unwrap(), 10, "counted up to the cap");
        let p = store.orders_progress(now).unwrap();
        assert_eq!((p.total, p.left, p.done()), (n as u32, n as u32, 0), "the old message is past the horizon: {p:?}");
        assert!(p.busy());
        assert_eq!(store.extract_orders_pending(250, now).unwrap(), 250);
        assert!(store.extraction_for(ids[0]).unwrap().is_some() && store.extraction_for(ids[n - 1]).unwrap().is_none(), "newest first");
        let mut batches = 1;
        while store.extract_orders_pending(250, now).unwrap() > 0 {
            batches += 1;
        }
        assert_eq!(batches, 5);
        assert_eq!(store.extract_orders_pending(250, now).unwrap(), 0, "nothing left: no busy loop");
        assert_eq!(store.orders_pending_count(10, now).unwrap(), 0);
        assert!(store.extraction_for(old).unwrap().is_none());

        // Topics wait for the junk check.
        assert_eq!((store.topics_pending_count(10).unwrap(), store.classify_topics_pending(250).unwrap()), (0, 0));
        while store.assess_pending(500).unwrap() > 0 {}
        assert_eq!(store.topics_pending_count(10_000).unwrap(), n as u32 + 1);
        assert_eq!(store.classify_topics_pending(250).unwrap(), 250);
        assert!(store.topics_for(&[ids[0]]).unwrap().contains_key(&ids[0]) && store.topics_for(&[old]).unwrap().is_empty(), "newest first");
        let mut batches = 1;
        while store.classify_topics_pending(250).unwrap() > 0 {
            batches += 1;
        }
        assert_eq!(batches, 5);
        assert_eq!(store.classify_topics_pending(250).unwrap(), 0);
        assert_eq!(store.topics_pending_count(10).unwrap(), 0);
        // Junk checks redone (a new contact changes what every check knows)
        // don't requeue mail that isn't about orders.
        let mut c = Contact { account_id: acct, given_name: "Nobody".into(), emails: vec![LabeledValue { label: "Home".into(), value: "nobody@elsewhere.example".into() }], ..Default::default() };
        store.insert_contact(&mut c).unwrap();
        assert!(store.assess_pending(500).unwrap() > 0);
        while store.assess_pending(500).unwrap() > 0 {}
        assert_eq!((store.extract_orders_pending(250, now).unwrap(), store.classify_topics_pending(250).unwrap()), (0, 0));
    }

    #[test]
    fn read_flag_move_do_not_reextract() {
        let (store, acct) = store_with_account();
        let id = plain(&store, acct, 1);
        settle(&store);
        mark_cached(&store);
        store.set_read(&[id], true).unwrap();
        store.set_flag(&[id], FlagStatus::Flagged, None).unwrap();
        store.set_pinned(&[id], true).unwrap();
        store.set_categories(&[id], &["Red category".to_string()]).unwrap();
        store.move_messages(&[id], folder(&store, acct, FolderRole::Archive)).unwrap();
        assert_eq!(still_cached(&store, id), (true, true));
        assert_eq!((store.orders_pending_count(10, Utc::now()).unwrap(), store.topics_pending_count(10).unwrap()), (0, 0));
    }

    #[test]
    fn subject_change_reextracts() {
        let (store, acct) = store_with_account();
        let id = plain(&store, acct, 1);
        let other = plain(&store, acct, 2);
        settle(&store);
        mark_cached(&store);
        let mut m = store.message(id).unwrap().unwrap();
        m.subject = "Your order has shipped".into();
        store.update_message(&m).unwrap();
        assert_eq!(still_cached(&store, id), (false, false));
        assert_eq!(still_cached(&store, other), (true, true));
        // The topic waits for the junk check, which the new subject redoes too.
        assert_eq!((store.orders_pending_count(10, Utc::now()).unwrap(), store.topics_pending_count(10).unwrap()), (1, 0));
        while store.assess_pending(250).unwrap() > 0 {}
        assert_eq!(store.topics_pending_count(10).unwrap(), 1);
        // Recipients and meetings matter to topics only.
        settle(&store);
        mark_cached(&store);
        let mut m = store.message(id).unwrap().unwrap();
        m.cc.push(EmailAddress::new("", "x@normal.example"));
        store.update_message(&m).unwrap();
        assert_eq!(still_cached(&store, id), (true, false));
    }

    #[test]
    fn late_body_reextracts() {
        let (store, acct) = store_with_account();
        let mut m = Message {
            account_id: acct,
            folder_id: folder(&store, acct, FolderRole::Inbox),
            from: EmailAddress::new("", "shop@brightleaf.example"),
            subject: "Hello".into(),
            received_at: Utc::now(),
            ..Default::default()
        };
        let id = store.insert_message(&mut m, None).unwrap();
        settle(&store);
        mark_cached(&store);
        store.set_message_body(id, Some("<p>Order number: BL-48213</p>"), "Order number: BL-48213").unwrap();
        assert_eq!(still_cached(&store, id), (false, false), "the body arrived after the headers");
    }

    #[test]
    fn unchanged_body_resync_does_not_reextract() {
        let (store, acct) = store_with_account();
        let id = plain(&store, acct, 1);
        settle(&store);
        mark_cached(&store);
        let body = store.message_body(id).unwrap().unwrap();
        store.set_message_body(id, body.html.as_deref(), &body.text).unwrap();
        assert_eq!(still_cached(&store, id), (true, true), "the same body downloaded again");
        store.set_message_body(id, Some("<p>changed</p>"), "changed").unwrap();
        assert_eq!(still_cached(&store, id), (false, false));
    }

    #[test]
    fn extraction_upsert_fires_topic_follow() {
        let (store, acct) = store_with_account();
        let id = plain(&store, acct, 1);
        settle(&store);
        // As if an older version had found an order here, and sorted it into Orders.
        store.conn().execute("UPDATE order_extractions SET kind = 'order', version = 'o0' WHERE message_id = ?1", [id]).unwrap();
        assert!(store.topics_for(&[id]).unwrap().is_empty(), "a changed kind drops the topic");
        assert_eq!(store.classify_topics_pending(250).unwrap(), 1);
        store.conn().execute("UPDATE topic_assignments SET topic = 'orders', source = 'order' WHERE message_id = ?1", [id]).unwrap();
        assert_eq!(store.topics_for(&[id]).unwrap()[&id].0, Topic::Orders);
        // Extracted again: not an order. The upsert lets the topic follow.
        assert_eq!(store.extract_orders_pending(250, Utc::now()).unwrap(), 1);
        assert_eq!(store.extraction_for(id).unwrap().unwrap().kind, ExtractionKind::None);
        assert!(store.topics_for(&[id]).unwrap().is_empty(), "the Orders topic was dropped");
        settle(&store);
        assert_ne!(store.topics_for(&[id]).unwrap()[&id].0, Topic::Orders);
    }

    #[test]
    fn key_delete_and_move_mark_orders_dirty() {
        let (store, acct) = store_with_account();
        let id = plain(&store, acct, 1);
        settle(&store);
        let key = format!("{acct}:t:ups:{UPS}");
        store.conn().execute("INSERT INTO order_keys (key, message_id) VALUES (?1, ?2)", params![key, id]).unwrap();
        store.conn().execute("DELETE FROM orders_dirty", []).unwrap();
        store.move_messages(&[id], folder(&store, acct, FolderRole::Junk)).unwrap();
        assert_eq!(count(&store, "SELECT COUNT(*) FROM orders_dirty"), 1, "a move regroups");
        store.conn().execute("DELETE FROM orders_dirty", []).unwrap();
        store.delete_messages_permanently(&[id]).unwrap();
        assert_eq!(count(&store, "SELECT COUNT(*) FROM order_keys"), 0, "cascaded");
        assert_eq!(count(&store, "SELECT COUNT(*) FROM orders_dirty"), 1, "the cascade's delete trigger fired");
        assert_eq!(store.rebuild_dirty_orders(200).unwrap(), 1);
        assert_eq!(store.rebuild_dirty_orders(200).unwrap(), 0, "a rebuild never queues itself again");
    }

    #[test]
    fn ignore_sender_forgets_only_that_senders_mail() {
        let (store, acct) = store_with_account();
        let a = mail(&store, acct, ("", "shop@brightleaf.example"), "hi", "<p>a</p>", 1);
        let b = mail(&store, acct, ("", "news@mail.brightleaf.example"), "hi", "<p>b</p>", 2);
        let c = mail(&store, acct, ("", "shop@evilbrightleaf.example"), "hi", "<p>c</p>", 3);
        settle(&store);
        assert!(store.ignore_order_sender("not an address").is_err());
        store.ignore_order_sender(" Brightleaf.example ").unwrap();
        assert_eq!(store.order_sender_ignores().unwrap().iter().map(|i| (i.pattern.as_str(), i.is_domain)).collect::<Vec<_>>(), [("brightleaf.example", true)]);
        let cached = |id| store.extraction_for(id).unwrap().is_some();
        assert_eq!((cached(a), cached(b), cached(c)), (false, false, true), "a label boundary, not a suffix");
        settle(&store);
        assert_eq!((cached(a), cached(b), cached(c)), (true, true, true));
        store.remove_order_sender_ignore("brightleaf.example").unwrap();
        assert!(store.order_sender_ignores().unwrap().is_empty());
        assert_eq!((cached(a), cached(b)), (false, false), "looked at again");
        store.ignore_order_sender("shop@brightleaf.example").unwrap();
        assert!(!store.order_sender_ignores().unwrap()[0].is_domain, "an address");
    }

    #[test]
    fn clear_orders_keeps_user_state_and_ignores() {
        let (store, acct) = store_with_account();
        plain(&store, acct, 1);
        let key = order_key(acct);
        store.set_order_state(&key, OrderStateChange::Hide(true)).unwrap();
        store.ignore_order_sender("brightleaf.example").unwrap();
        settle(&store);
        assert!(count(&store, "SELECT COUNT(*) FROM order_extractions") > 0);
        store.clear_orders().unwrap();
        for table in ["order_keys", "orders", "order_messages", "order_group_keys", "orders_dirty"] {
            assert_eq!(count(&store, &format!("SELECT COUNT(*) FROM {table}")), 0, "{table}");
        }
        assert_eq!(count(&store, "SELECT COUNT(*) FROM order_extractions WHERE version = ''"), 1, "marked for another look");
        assert_eq!(store.orders_pending_count(10, Utc::now()).unwrap(), 1, "found again from the mail");
        assert_eq!(count(&store, "SELECT COUNT(*) FROM order_user_state WHERE hidden = 1"), 1);
        assert_eq!(store.order_sender_ignores().unwrap().len(), 1);
        // Switched off: nothing found is kept.
        let mut st = store.settings();
        st.order_tracking = false;
        store.save_settings(&st).unwrap();
        store.clear_orders().unwrap();
        assert_eq!(count(&store, "SELECT COUNT(*) FROM order_extractions"), 0);
    }

    #[test]
    fn looking_through_the_mail_again_keeps_the_topics() {
        let (store, acct) = store_with_account();
        for i in 0..20 {
            plain(&store, acct, i);
        }
        let order = confirmation(&store, acct, 30);
        settle(&store);
        let ids: Vec<MessageId> = (1..=21).collect();
        assert_eq!(store.topics_for(&ids).unwrap().len(), 21);
        let before = store.topic_counts().unwrap().iter().map(|c| c.total).sum::<u32>();
        store.clear_orders().unwrap();
        // Only the order email's topic waits for its order to be found again.
        let kept = store.topics_for(&ids).unwrap();
        assert_eq!(kept.len(), 20);
        assert!(!kept.contains_key(&order));
        assert_eq!(store.topic_counts().unwrap().iter().map(|c| c.total).sum::<u32>(), before - 1);
        settle(&store);
        assert_eq!(store.topics_for(&ids).unwrap().len(), 21);
        assert_eq!(store.topic_for(order).unwrap().unwrap().topic, Topic::Orders);
        assert_eq!(store.orders(false).unwrap().len(), 1);
    }

    #[test]
    fn remove_account_cleans_rules_and_orders() {
        let (store, acct) = store_with_account();
        let mut a = Account { kind: AccountKind::Local, email: "second@axentra.example".into(), ..Default::default() };
        let other = store.add_account(&mut a).unwrap();
        plain(&store, acct, 1);
        plain(&store, other, 2);
        settle(&store);
        for a in [acct, other] {
            store.set_order_state(&format!("{a}:o:shop.example:A-1001"), OrderStateChange::Hide(true)).unwrap();
            store.set_topic(Some(a), crate::store::TopicTarget::Sender("p@normal.example".into()), Some(Topic::Travel)).unwrap();
        }
        store.set_topic(None, crate::store::TopicTarget::Domain("normal.example".into()), Some(Topic::Finance)).unwrap();
        // An order of the account, as a rebuild would have left it.
        store
            .conn()
            .execute("INSERT INTO orders (account_id, key, status, updated_at, json) VALUES (?1, 'k', 'ordered', 0, '{}')", [acct])
            .unwrap();
        store.remove_account(acct).unwrap();
        let rules: Vec<i64> = store.topic_rules().unwrap().iter().map(|r| r.rule.account_key).collect();
        assert_eq!(rules, [0, other]);
        let states: Vec<String> = column(&store.conn(), "SELECT key FROM order_user_state", []).unwrap();
        assert_eq!(states, [format!("{other}:o:shop.example:A-1001")]);
        assert_eq!(count(&store, "SELECT COUNT(*) FROM orders"), 0);
        assert_eq!(count(&store, "SELECT COUNT(*) FROM order_extractions WHERE account_id != 0") as usize, store.account_messages(other).unwrap().len());
    }

    #[test]
    fn messages_json_untouched() {
        let (store, acct) = store_with_account();
        let ids: Vec<MessageId> = (0..5).map(|i| plain(&store, acct, i)).collect();
        confirmation(&store, acct, 10);
        shipped(&store, acct, UPS, 5);
        let before = messages_json(&store);
        settle(&store);
        store.set_topic(Some(acct), crate::store::TopicTarget::Sender("p1@normal.example".into()), Some(Topic::Travel)).unwrap();
        store.set_topic(None, crate::store::TopicTarget::Domain("normal.example".into()), Some(Topic::Social)).unwrap();
        store.set_topic(None, crate::store::TopicTarget::Message(ids[2]), Some(Topic::Finance)).unwrap();
        store.set_order_state(&order_key(acct), OrderStateChange::Override(Some(OrderStatus::Delivered))).unwrap();
        store.set_order_state(&order_key(acct), OrderStateChange::Hide(true)).unwrap();
        store.unhide_all_orders().unwrap();
        store.ignore_order_sender("brightleaf.example").unwrap();
        settle(&store);
        store.remove_order_sender_ignore("brightleaf.example").unwrap();
        settle(&store);
        store.clear_orders().unwrap();
        store.clear_topics().unwrap();
        store.resort_topics().unwrap();
        store.clear_topic_rules().unwrap();
        settle(&store);
        assert_eq!(messages_json(&store), before, "nothing writes to the mail itself");
    }

    // ------------------------------------------------------------ with extraction (part A)

    #[test]
    fn grouping_confirmation_shipped_carrier_delivered() {
        let (store, acct) = store_with_account();
        let ids = [
            confirmation(&store, acct, 4 * 1440),
            shipped(&store, acct, UPS, 3 * 1440),
            carrier_out_for_delivery(&store, acct, UPS, 1440),
            delivered(&store, acct, UPS, 60),
        ];
        settle(&store);
        let o = only_order(&store);
        assert_eq!(o.status, OrderStatus::Delivered);
        assert_eq!(o.events.len(), 4);
        assert_eq!(o.key, order_key(acct));
        assert_eq!(o.shipments.len(), 1);
        for id in ids {
            assert_eq!(store.order_for_message(id).unwrap().map(|x| x.key), Some(o.key.clone()), "{id}");
        }
        assert_eq!(store.order_by_key(&format!("{acct}:t:ups:{UPS}")).unwrap().map(|x| x.id), Some(o.id), "found by any key");
        // Delivered today still counts under Arriving soon.
        let (now, today) = (Utc::now(), chrono::Local::now().date_naive());
        let today_section = orders::section(&o, now, today, 30) == Some(OrderSection::ArrivingSoon);
        assert_eq!(store.orders_summary(now, today, 30).unwrap(), OrdersSummary { active: u32::from(today_section), attention: 0 });
        assert_eq!(store.topics_for(&ids).unwrap().values().filter(|t| t.0 == Topic::Orders).count(), 4);
    }

    #[test]
    fn two_parcels_one_order() {
        let (store, acct) = store_with_account();
        confirmation(&store, acct, 4 * 1440);
        shipped(&store, acct, UPS, 3 * 1440);
        shipped(&store, acct, UPS2, 2 * 1440);
        delivered(&store, acct, UPS, 60);
        settle(&store);
        let o = only_order(&store);
        assert_eq!(o.shipments.len(), 2, "{:?}", o.shipments);
        assert!(o.partly_delivered);
        assert_ne!(o.status, OrderStatus::Delivered);
    }

    #[test]
    fn same_message_in_two_folders_deduped() {
        let (store, acct) = store_with_account();
        confirmation(&store, acct, 2 * 1440);
        let s = shipped(&store, acct, UPS, 1440);
        store.copy_messages(&[s], folder(&store, acct, FolderRole::Archive)).unwrap();
        settle(&store);
        assert_eq!(only_order(&store).events.len(), 2, "a copy counts once");
    }

    #[test]
    fn move_to_junk_drops_from_order_and_move_back_restores() {
        let (store, acct) = store_with_account();
        confirmation(&store, acct, 3 * 1440);
        shipped(&store, acct, UPS, 2 * 1440);
        let d = delivered(&store, acct, UPS, 60);
        settle(&store);
        assert_eq!(only_order(&store).status, OrderStatus::Delivered);
        let moves = store.mark_junk(&[d]).unwrap();
        settle(&store);
        let o = only_order(&store);
        assert_eq!((o.events.len(), o.status), (2, OrderStatus::Shipped));
        assert!(store.order_for_message(d).unwrap().is_none());
        store.undo_moves(&moves).unwrap();
        settle(&store);
        assert_eq!(only_order(&store).events.len(), 3);
        // Deleted Items too.
        let moves = store.delete_messages(&[d]).unwrap();
        settle(&store);
        assert_eq!(only_order(&store).events.len(), 2);
        store.undo_moves(&moves).unwrap();
        settle(&store);
        assert_eq!(only_order(&store).status, OrderStatus::Delivered);
    }

    #[test]
    fn delete_message_cascades_and_prunes() {
        let (store, acct) = store_with_account();
        let c = confirmation(&store, acct, 2 * 1440);
        let s = shipped(&store, acct, UPS, 1440);
        settle(&store);
        assert!(only_order(&store).keys.contains(&format!("{acct}:t:ups:{UPS}")));
        // The shipped mail held the only tracking key of the order.
        store.delete_messages_permanently(&[s]).unwrap();
        settle(&store);
        let o = only_order(&store);
        assert_eq!((o.events.len(), o.status), (1, OrderStatus::Ordered));
        assert!(o.shipments.is_empty() && !o.keys.iter().any(|k| k.contains(":t:")), "{:?}", o.keys);
        store.delete_messages_permanently(&[c]).unwrap();
        settle(&store);
        assert!(store.orders(true).unwrap().is_empty());
        assert_eq!(count(&store, "SELECT COUNT(*) FROM order_group_keys") + count(&store, "SELECT COUNT(*) FROM orders_dirty"), 0);
    }

    #[test]
    fn user_state_survives_rebuild_and_split() {
        let (store, acct) = store_with_account();
        confirmation(&store, acct, 3 * 1440);
        let s = shipped(&store, acct, UPS, 2 * 1440);
        carrier_out_for_delivery(&store, acct, UPS, 60);
        settle(&store);
        let key = only_order(&store).key;
        let previous = store.set_order_state(&key, OrderStateChange::Hide(true)).unwrap();
        assert!(store.orders(false).unwrap().is_empty() && store.hidden_order_count().unwrap() == 1);
        // Extracted again and rebuilt: still hidden.
        let mut m = store.message(s).unwrap().unwrap();
        m.subject.push_str(" (updated)");
        store.update_message(&m).unwrap();
        settle(&store);
        assert!(only_order(&store).hidden);
        // Split (the message joining order number and tracking number goes):
        // the order keeps its state.
        store.delete_messages_permanently(&[s]).unwrap();
        settle(&store);
        let all = store.orders(true).unwrap();
        assert_eq!(all.len(), 2, "{all:#?}");
        assert!(all.iter().find(|o| o.key == key).unwrap().hidden);
        // Undo.
        store.restore_order_state(&key, previous).unwrap();
        assert_eq!(store.hidden_order_count().unwrap(), 0);
        // Marked delivered, then back to what the mail says.
        store.set_order_state(&key, OrderStateChange::Override(Some(OrderStatus::Delivered))).unwrap();
        let o = store.order_by_key(&key).unwrap().unwrap();
        assert!(o.overridden && o.status == OrderStatus::Delivered);
        store.set_order_state(&key, OrderStateChange::Override(None)).unwrap();
        let o = store.order_by_key(&key).unwrap().unwrap();
        assert!(!o.overridden && o.status == OrderStatus::Ordered);
    }

    #[test]
    fn ignore_sender_suppresses_and_unignore_restores() {
        let (store, acct) = store_with_account();
        let c = confirmation(&store, acct, 2 * 1440);
        shipped(&store, acct, UPS, 1440);
        settle(&store);
        only_order(&store);
        store.ignore_order_sender("brightleaf.example").unwrap();
        settle(&store);
        assert!(store.orders(true).unwrap().is_empty());
        let x = store.extraction_for(c).unwrap().unwrap();
        assert_eq!((x.kind, x.suppressed), (ExtractionKind::Suppressed, Some(orders::SuppressReason::IgnoredSender)));
        // Not suspicious for that: sorted as if order tracking were off for it.
        let a = store.topic_for(c).unwrap().unwrap();
        assert_eq!((a.topic, a.source), (Topic::Orders, Source::Signals), "{:?}", a.reasons);
        store.remove_order_sender_ignore("brightleaf.example").unwrap();
        settle(&store);
        only_order(&store);
    }

    #[test]
    fn a_change_made_while_a_batch_is_worked_out_is_kept() {
        let (store, acct) = store_with_account();
        let c = confirmation(&store, acct, 60);
        while store.assess_pending(250).unwrap() > 0 {}
        // "Don't track orders from…" while the batch reads the mail.
        during_next_batch(|s| s.ignore_order_sender("brightleaf.example").unwrap());
        assert_eq!(store.extract_orders_pending(250, Utc::now()).unwrap(), 1);
        assert!(store.extraction_for(c).unwrap().is_none(), "worked out before the ignore: not written");
        settle(&store);
        assert_eq!(store.extraction_for(c).unwrap().unwrap().suppressed, Some(orders::SuppressReason::IgnoredSender));
        assert!(store.orders(true).unwrap().is_empty());
        // Switched off meanwhile: nothing is left behind.
        store.remove_order_sender_ignore("brightleaf.example").unwrap();
        during_next_batch(|s| {
            let mut st = s.settings();
            st.order_tracking = false;
            s.save_settings(&st).unwrap();
            s.clear_orders().unwrap();
        });
        assert_eq!(store.extract_orders_pending(250, Utc::now()).unwrap(), 1);
        assert_eq!(count(&store, "SELECT COUNT(*) FROM order_extractions") + count(&store, "SELECT COUNT(*) FROM order_keys"), 0);
        assert_eq!(store.extract_orders_pending(250, Utc::now()).unwrap(), 0);
    }

    #[test]
    fn uncached_junk_holds_order_back() {
        let (store, acct) = store_with_account();
        let c = confirmation(&store, acct, 60);
        let now = Utc::now();
        assert_eq!(store.extract_orders_pending(250, now).unwrap(), 1);
        assert_eq!(store.extraction_for(c).unwrap().unwrap().kind, ExtractionKind::Order);
        let jv: String = store.conn().query_row("SELECT junk_version FROM order_extractions WHERE message_id = ?1", [c], |r| r.get(0)).unwrap();
        assert_eq!(jv, "", "not checked for junk yet");
        while store.rebuild_dirty_orders(200).unwrap() > 0 {}
        assert!(store.orders(true).unwrap().is_empty(), "never shown before the junk check");
        assert_eq!(store.extract_orders_pending(250, now).unwrap(), 0, "and no busy loop meanwhile");
        while store.assess_pending(250).unwrap() > 0 {}
        assert_eq!(store.orders_pending_count(10, now).unwrap(), 1, "the junk check makes it pending again");
        settle(&store);
        only_order(&store);
    }

    #[test]
    fn a_big_batch_never_cuts_an_order_short() {
        let (store, acct) = store_with_account();
        let mut moved: Vec<MessageId> = (0..199)
            .map(|i| {
                let n = format!("AA-{i:04}");
                mail(&store, acct, ("Aaa Shop", "orders@aaa.example"), &format!("Order confirmed: #{n}"), &format!("<p>Order number: {n}</p>"), 3 * 1440 + i as i64)
            })
            .collect();
        moved.push(confirmation(&store, acct, 3 * 1440));
        shipped(&store, acct, UPS, 2 * 1440);
        delivered(&store, acct, UPS, 60);
        settle(&store);
        assert_eq!(store.order_by_key(&order_key(acct)).unwrap().unwrap().events.len(), 3);
        // 200 keys go dirty at once; the Brightleaf one sorts last.
        store.move_messages(&moved, folder(&store, acct, FolderRole::Archive)).unwrap();
        assert_eq!(count(&store, "SELECT COUNT(*) FROM orders_dirty"), 200);
        settle(&store);
        let o = store.order_by_key(&order_key(acct)).unwrap().unwrap();
        assert_eq!((o.events.len(), o.status), (3, OrderStatus::Delivered));
        assert_eq!(store.orders(true).unwrap().len(), 200);
        assert_eq!(count(&store, "SELECT COUNT(*) FROM orders_dirty"), 0);
    }

    #[test]
    fn a_batch_leaves_the_seeds_past_its_budget_dirty() {
        let (store, acct) = store_with_account();
        for i in 0..MAX_REBUILD_MESSAGES + 1 {
            let n = format!("AA-{i:04}");
            mail(&store, acct, ("Aaa Shop", "orders@aaa.example"), &format!("Order confirmed: #{n}"), &format!("<p>Order number: {n}</p>"), i as i64);
        }
        settle(&store);
        store.conn().execute("INSERT OR IGNORE INTO orders_dirty (key) SELECT key FROM order_keys", []).unwrap();
        assert_eq!(store.rebuild_dirty_orders(500).unwrap(), MAX_REBUILD_MESSAGES);
        assert_eq!(count(&store, "SELECT COUNT(*) FROM orders_dirty"), 1, "the rest waits for the next call");
        assert_eq!(store.rebuild_dirty_orders(500).unwrap(), 1);
        assert_eq!(store.orders(true).unwrap().len(), MAX_REBUILD_MESSAGES + 1);
    }

    #[test]
    fn showing_hidden_orders_brings_back_every_one() {
        let (store, acct) = store_with_account();
        let n = 60;
        for i in 0..n {
            let no = format!("HD-{i:04}");
            for k in 0..4 {
                mail(&store, acct, ("Hd Shop", "orders@hd.example"), &format!("Order confirmed: #{no}"), &format!("<p>Order number: {no}</p>"), (i * 4 + k) as i64);
            }
        }
        settle(&store);
        let keys: Vec<String> = store.orders(true).unwrap().into_iter().map(|o| o.key).collect();
        assert_eq!(keys.len(), n);
        for k in &keys {
            store.set_order_state(k, OrderStateChange::Hide(true)).unwrap();
        }
        assert_eq!(store.hidden_order_count().unwrap(), n as u32);
        // More emails than one rebuild takes.
        assert!(n * 4 > MAX_REBUILD_MESSAGES);
        assert_eq!(store.unhide_all_orders().unwrap(), n);
        assert_eq!(store.hidden_order_count().unwrap(), 0, "shown at once");
        assert!(store.orders(false).unwrap().iter().all(|o| !o.hidden));
        settle(&store);
        assert_eq!((store.hidden_order_count().unwrap(), store.orders(false).unwrap().len()), (0, n));
        assert_eq!(count(&store, "SELECT COUNT(*) FROM orders_dirty"), 0);
        assert!(store.orders(false).unwrap().iter().all(|o| o.events.len() == 4));
    }

    #[test]
    fn a_status_set_under_another_key_survives_hiding() {
        let (store, acct) = store_with_account();
        // The carrier's email first: the order's key is its tracking number.
        carrier_out_for_delivery(&store, acct, UPS, 3 * 1440);
        settle(&store);
        let t_key = only_order(&store).key;
        assert_eq!(t_key, format!("{acct}:t:ups:{UPS}"));
        store.set_order_state(&t_key, OrderStateChange::Override(Some(OrderStatus::Delivered))).unwrap();
        // The shop's email joins it: now the order number is the key.
        shipped(&store, acct, UPS, 1440);
        settle(&store);
        let o = only_order(&store);
        assert_eq!((o.key.as_str(), o.status), (order_key(acct).as_str(), OrderStatus::Delivered));
        store.set_order_state(&o.key, OrderStateChange::Hide(true)).unwrap();
        store.unhide_all_orders().unwrap();
        settle(&store);
        let o = only_order(&store);
        assert!(!o.hidden && o.overridden, "{o:#?}");
        assert_eq!((o.status, o.mail_status), (OrderStatus::Delivered, Some(OrderStatus::OutForDelivery)));
    }

    #[test]
    fn hidden_orders_past_the_horizon_are_not_counted() {
        let (store, acct) = store_with_account();
        confirmation(&store, acct, 60);
        settle(&store);
        let key = only_order(&store).key;
        store.set_order_state(&key, OrderStateChange::Hide(true)).unwrap();
        assert_eq!(store.hidden_order_count().unwrap(), 1);
        // Its last email arrived 450 days ago: Packages doesn't list it.
        let old = (Utc::now() - Duration::days(450)).timestamp();
        store.conn().execute("UPDATE orders SET updated_at = ?1", [old]).unwrap();
        assert!(store.orders(true).unwrap().is_empty());
        assert_eq!(store.hidden_order_count().unwrap(), 0);
        assert_eq!(store.unhide_all_orders().unwrap(), 0, "nothing comes back to Packages");
    }

    #[test]
    fn a_deleted_folder_takes_its_orders_and_topics_with_it() {
        let (store, acct) = store_with_account();
        let shopping = store.create_folder(acct, None, "Shopping").unwrap();
        let c = confirmation(&store, acct, 60);
        let note = plain(&store, acct, 1);
        store.move_messages(&[c, note], shopping).unwrap();
        settle(&store);
        let topics = |store: &Store| store.topic_counts().unwrap().iter().map(|t| t.total).sum::<u32>();
        let listed = |store: &Store| store.messages(&MessageQuery { topic: Some(TopicScope::Any), ..Default::default() }).unwrap().len();
        assert_eq!((store.orders(false).unwrap().len(), topics(&store), listed(&store)), (1, 2, 2));
        // Deleted: the folder moves into Deleted Items, keeping its role.
        store.delete_folder(shopping).unwrap();
        assert_eq!(store.folder(shopping).unwrap().unwrap().role, FolderRole::Custom);
        settle(&store);
        assert_eq!((store.orders(false).unwrap().len(), topics(&store), listed(&store)), (0, 0, 0));
        assert!(store.order_for_message(c).unwrap().is_none());
        // Moved back out: everything is back.
        store.move_folder(shopping, None).unwrap();
        settle(&store);
        assert_eq!((store.orders(false).unwrap().len(), topics(&store), listed(&store)), (1, 2, 2));
        // A folder made under Junk Email is junk too, however deep.
        let junk = folder(&store, acct, FolderRole::Junk);
        let sub = store.create_folder(acct, Some(junk), "Maybe").unwrap();
        let deeper = store.create_folder(acct, Some(sub), "Deeper").unwrap();
        store.move_messages(&[c, note], deeper).unwrap();
        settle(&store);
        assert_eq!((store.orders(false).unwrap().len(), topics(&store), listed(&store)), (0, 0, 0));
    }

    #[test]
    fn a_shops_own_check_digit_vouches_for_nothing() {
        let (store, acct) = store_with_account();
        let customs = |store: &Store, minutes| {
            mail(
                store,
                acct,
                BRIGHTLEAF,
                "Your order #BL-48213 is held at customs",
                "<p>Your order #BL-48213 is held at customs. Pay a small fee of $2.99 to release it: \
                 <a href=\"https://pay-release.example/p\">Pay now</a></p>",
                minutes,
            )
        };
        // The shop's own mail with a tracking number whose check digit
        // holds: anyone can work one out.
        shipped(&store, acct, UPS, 2 * 1440);
        let fee = customs(&store, 60);
        settle(&store);
        let merchant = (acct, "brightleaf.example".to_string());
        let ctx = store.extract_context(&[]).unwrap();
        assert!(ctx.merchants.contains(&merchant) && !ctx.trusted.contains(&merchant), "{:?}", ctx.trusted);
        assert_eq!(store.extraction_for(fee).unwrap().unwrap().suppressed, Some(orders::SuppressReason::DeliveryFeeLure));
        // Mail that only says it is from the carrier (nobody vouched for
        // its From address) confirms nothing.
        on_gmail(&store, acct);
        carrier_out_for_delivery(&store, acct, UPS, 1500);
        settle(&store);
        assert!(!store.extract_context(&[]).unwrap().trusted.contains(&merchant));
        assert_eq!(store.extraction_for(fee).unwrap().unwrap().suppressed, Some(orders::SuppressReason::DeliveryFeeLure));
        // The carrier's own mail about the same parcel: now it is confirmed,
        // and the shop's mail is read again.
        vouched_out_for_delivery(&store, acct, UPS, 1440);
        settle(&store);
        assert!(store.extract_context(&[]).unwrap().trusted.contains(&merchant));
        assert_eq!(store.extraction_for(fee).unwrap().unwrap().kind, ExtractionKind::Order);
        assert!(store.order_for_message(fee).unwrap().is_some());
    }

    #[test]
    fn a_carriers_address_nobody_vouched_for_is_no_carrier() {
        let (store, acct) = store_with_account();
        let lure = |store: &Store, auth: Option<&str>, minutes| {
            mail_with_auth(
                store,
                acct,
                ("UPS", "alerts@ups.com"),
                "UPS: your package is on hold",
                &format!(
                    "<p>Your package {UPS} is on hold at customs. Pay the $2.99 customs fee to release it: \
                     <a href=\"https://ups-customs-pay.example/pay\">Pay now</a></p>"
                ),
                minutes,
                auth,
            )
        };
        let forged = lure(&store, None, 60);
        settle(&store);
        let x = store.extraction_for(forged).unwrap().unwrap();
        assert_eq!((x.kind, x.suppressed), (ExtractionKind::Suppressed, Some(orders::SuppressReason::DeliveryFeeLure)));
        assert!(store.orders(true).unwrap().is_empty());
        assert_eq!(store.topic_for(forged).unwrap().unwrap().topic, Topic::Other);
        // The same from the carrier, as the account's provider vouches.
        on_gmail(&store, acct);
        let real = lure(&store, Some("mx.google.com; dkim=pass header.d=ups.com; dmarc=pass header.from=ups.com"), 30);
        settle(&store);
        let x = store.extraction_for(real).unwrap().unwrap();
        assert_eq!((x.kind, x.suppressed, x.carrier_verified), (ExtractionKind::Order, None, true));
        // A header the provider didn't write is believed no more than none.
        let claimed = lure(&store, Some("mx.evil.example; dmarc=pass header.from=ups.com"), 20);
        settle(&store);
        assert_eq!(store.extraction_for(claimed).unwrap().unwrap().suppressed, Some(orders::SuppressReason::DeliveryFeeLure));
    }

    #[test]
    fn a_carriers_mail_read_before_its_junk_check_is_read_again() {
        let (store, acct) = store_with_account();
        on_gmail(&store, acct);
        let id = vouched_out_for_delivery(&store, acct, UPS, 60);
        // Extracted before the junk check: nothing vouches for it yet.
        store.extract_orders_pending(250, Utc::now()).unwrap();
        assert!(!store.extraction_for(id).unwrap().unwrap().carrier_verified);
        settle(&store);
        assert!(store.extraction_for(id).unwrap().unwrap().carrier_verified);
        assert_eq!(store.orders_pending_count(100, Utc::now()).unwrap(), 0, "no busy loop");
    }

    #[test]
    fn a_shops_name_on_another_domain_never_makes_an_order() {
        let (store, acct) = store_with_account();
        let fake = mail(
            &store,
            acct,
            ("Amazon.com", "orders@amzn-account-services.example"),
            "Your order #112-4839201-5512345 has shipped",
            &format!("<p>Order total: $1,249.00</p><p>Tracking number: {UPS}</p><p>Didn't place this order? Cancel it here</p>"),
            60,
        );
        store.assess_pending(250).unwrap();
        // Only a caution: the sender calls itself a brand it isn't.
        let posing = Analysis {
            findings: vec![Finding::DisplayNameBrand { brand: "Amazon".into(), actual: "orders@amzn-account-services.example".into() }],
            ..Default::default()
        };
        assert_eq!(posing.severity(), Some(Severity::Caution));
        store.conn().execute("UPDATE junk_assessments SET json = ?1, gate = 'impersonation'", [serde_json::to_string(&posing).unwrap()]).unwrap();
        settle(&store);
        assert_eq!(store.extraction_for(fake).unwrap().unwrap().suppressed, Some(orders::SuppressReason::Impersonation));
        assert!(store.orders(true).unwrap().is_empty() && store.order_for_message(fake).unwrap().is_none());
        let a = store.topic_for(fake).unwrap().unwrap();
        assert_eq!((a.topic, a.source), (Topic::Other, Source::Suspicious));
    }

    #[test]
    fn thrown_away_junk_checks_keep_the_orders() {
        let (store, acct) = store_with_account();
        confirmation(&store, acct, 2 * 1440);
        shipped(&store, acct, UPS, 1440);
        settle(&store);
        only_order(&store);
        store.forget_learning().unwrap();
        store.conn().execute("DELETE FROM junk_assessments", []).unwrap();
        let now = Utc::now();
        assert_eq!(store.orders_pending_count(10, now).unwrap(), 0, "a missing check is no new verdict");
        while store.extract_orders_pending(250, now).unwrap() + store.rebuild_dirty_orders(200).unwrap() > 0 {}
        assert_eq!(only_order(&store).events.len(), 2);
        settle(&store);
        assert_eq!(only_order(&store).events.len(), 2);
    }

    #[test]
    fn mail_reported_as_phishing_stays_out_after_a_move_back() {
        let (store, acct) = store_with_account();
        confirmation(&store, acct, 2 * 1440);
        let s = shipped(&store, acct, UPS, 1440);
        settle(&store);
        assert_eq!(only_order(&store).events.len(), 2);
        let op = store.report_junk(&[s], crate::junk::Label::Phishing, false).unwrap();
        settle(&store);
        assert_eq!(only_order(&store).events.len(), 1);
        // A plain move back to the Inbox, not "Not junk".
        store.move_messages(&[s], folder(&store, acct, FolderRole::Inbox)).unwrap();
        settle(&store);
        assert_eq!(only_order(&store).events.len(), 1);
        assert!(store.order_for_message(s).unwrap().is_none());
        assert_eq!(store.extraction_for(s).unwrap().unwrap().suppressed, Some(orders::SuppressReason::Danger));
        let a = store.topic_for(s).unwrap().unwrap();
        assert_eq!((a.topic, a.source), (Topic::Other, Source::Suspicious));
        // Undone: an order email again.
        store.undo_junk_operation(&op).unwrap();
        store.move_messages(&[s], folder(&store, acct, FolderRole::Inbox)).unwrap();
        settle(&store);
        assert_eq!(only_order(&store).events.len(), 2);
        assert_eq!(store.topic_for(s).unwrap().unwrap().topic, Topic::Orders);
    }

    /// Every cached check found signs of phishing (and stays current).
    fn checked_as_danger(store: &Store) {
        let danger = Analysis {
            findings: vec![Finding::LinkTextMismatch { shown: "brightleaf.example".into(), target: "evil.example".into() }],
            ..Default::default()
        };
        store.conn().execute("UPDATE junk_assessments SET json = ?1, gate = 'danger'", [serde_json::to_string(&danger).unwrap()]).unwrap();
    }

    #[test]
    fn a_shops_later_mail_read_before_its_order_is_read_again() {
        let (store, acct) = store_with_account();
        confirmation(&store, acct, 60);
        // Not an order email on its own (no order address, no tracking
        // number): only once the shop is known.
        let update = mail(
            &store,
            acct,
            ("Brightleaf Home", "hello@brightleaf.example"),
            "An update about your Brightleaf purchase",
            "<p>Order number: BL-48213. Your order has shipped.</p>",
            10,
        );
        // Both in one batch, newest first: the update before the order exists.
        settle(&store);
        assert_eq!(store.extraction_for(update).unwrap().unwrap().kind, ExtractionKind::Order);
        assert_eq!(only_order(&store).events.len(), 2);
        // The same after starting over.
        store.clear_orders().unwrap();
        settle(&store);
        assert_eq!(only_order(&store).events.len(), 2);
        assert_eq!(only_order(&store).status, OrderStatus::Shipped);
    }

    #[test]
    fn an_allow_rule_that_goes_takes_its_phishing_order_with_it() {
        let (store, acct) = store_with_account();
        let change = store.set_sender_rule(None, "orders@brightleaf.example", "", RuleAction::Allow).unwrap();
        let s = shipped(&store, acct, UPS, 60);
        store.assess_pending(250).unwrap();
        checked_as_danger(&store);
        settle(&store);
        assert_eq!(store.extraction_for(s).unwrap().unwrap().kind, ExtractionKind::Order, "allowed");
        assert!(store.order_for_message(s).unwrap().is_some());
        // The rule goes: the message is gated again.
        store.undo_rule_change(&change).unwrap();
        assert!(store.sender_rules().unwrap().is_empty());
        settle(&store);
        assert_eq!(store.extraction_for(s).unwrap().unwrap().suppressed, Some(orders::SuppressReason::Danger));
        assert!(store.order_for_message(s).unwrap().is_none());
        assert!(store.orders(true).unwrap().is_empty());
        // And an allow rule added later lets it in again.
        let change = store.set_sender_rule(None, "brightleaf.example", "", RuleAction::Allow).unwrap();
        settle(&store);
        assert!(store.order_for_message(s).unwrap().is_some());
        // Changed to Block: gated again.
        store.set_sender_rule(None, "brightleaf.example", "", RuleAction::Block).unwrap();
        settle(&store);
        assert!(store.order_for_message(s).unwrap().is_none());
        store.undo_rule_change(&change).unwrap();
        settle(&store);
    }

    #[test]
    fn many_junk_rules_at_once_are_one_pass_over_the_order_emails() {
        let (store, acct) = store_with_account();
        let s = shipped(&store, acct, UPS, 60);
        settle(&store);
        assert!(store.order_for_message(s).unwrap().is_some());
        // Lots of order emails from other senders (as a big mailbox has).
        let inbox = folder(&store, acct, FolderRole::Inbox);
        for i in 0..2000 {
            let mut m = Message {
                account_id: acct,
                folder_id: inbox,
                from: EmailAddress::new("", format!("o{i}@shop{i}.example")),
                subject: format!("order {i}"),
                received_at: Utc::now() - Duration::minutes(i),
                ..Default::default()
            };
            let id = store.insert_message(&mut m, None).unwrap();
            store
                .conn()
                .execute(
                    "INSERT INTO order_extractions (message_id, account_id, version, kind, confidence, json, created_at) VALUES (?1, ?2, ?3, 'order', 1, '{}', 0)",
                    params![id, acct, version()],
                )
                .unwrap();
        }
        store.conn().execute("UPDATE order_extractions SET created_at = 0 WHERE message_id = ?1", [s]).unwrap();
        // A backup's blocked senders, restored in one go.
        let started = std::time::Instant::now();
        store
            .tx(|tx| {
                for i in 0..1000 {
                    tx.execute(
                        "INSERT INTO junk_sender_rules (account_key, pattern, domain, action, source, created_at, updated_at) VALUES (0, ?1, 0, 'block', 'user', 0, 0)",
                        [format!("x{i}@blocked.example")],
                    )?;
                }
                tx.execute(
                    "INSERT INTO junk_sender_rules (account_key, pattern, domain, action, source, created_at, updated_at) VALUES (0, 'brightleaf.example', 1, 'block', 'user', 0, 0)",
                    [],
                )?;
                Ok(())
            })
            .unwrap();
        assert!(started.elapsed() < std::time::Duration::from_millis(1000), "{:?}", started.elapsed());
        // Applied by the next batch: the blocked shop's order email is read again.
        assert_eq!(count(&store, "SELECT COUNT(*) FROM orders_regate_senders"), 1001);
        store.extract_orders_pending(250, Utc::now()).unwrap();
        assert_eq!(count(&store, "SELECT COUNT(*) FROM orders_regate_senders"), 0);
        assert!(count(&store, &format!("SELECT created_at FROM order_extractions WHERE message_id = {s}")) > 0, "read again");
        assert_eq!(count(&store, "SELECT COUNT(*) FROM order_extractions WHERE kind = 'order' AND json = '{}'"), 2000, "no other sender's");
    }

    #[test]
    fn a_phishing_report_beats_a_domain_allow_rule() {
        let (store, acct) = store_with_account();
        store.set_sender_rule(None, "brightleaf.example", "", RuleAction::Allow).unwrap();
        let s = shipped(&store, acct, UPS, 60);
        settle(&store);
        assert!(store.order_for_message(s).unwrap().is_some());
        store.report_junk(&[s], crate::junk::Label::Phishing, false).unwrap();
        store.move_messages(&[s], folder(&store, acct, FolderRole::Inbox)).unwrap();
        settle(&store);
        assert_eq!(store.extraction_for(s).unwrap().unwrap().suppressed, Some(orders::SuppressReason::Danger));
        assert!(store.order_for_message(s).unwrap().is_none());
        assert!(store.orders(true).unwrap().is_empty());
    }

    #[test]
    fn one_free_mail_seller_makes_no_other_sender_there_a_shop() {
        let (store, acct) = store_with_account();
        mail(
            &store,
            acct,
            ("Jane's Pottery", "janespottery@gmail.com"),
            "Your order #JP-1042 has shipped",
            &format!("<p>Tracking number: {UPS}</p>"),
            120,
        );
        settle(&store);
        assert_eq!(only_order(&store).key, format!("{acct}:o:janespottery@gmail.com:JP-1042"));
        let lure = mail(
            &store,
            acct,
            ("Parcel Notice", "parcel.notice.4471@gmail.com"),
            "Delivery delayed: order #PX-55120",
            &format!(
                "<p>Your order #PX-55120 is on hold at customs. Tracking number: {UPS2}.</p>\
                 <p>Pay a small fee of $2.99 to release it: <a href=\"https://pay-release.example/p\">Pay now</a></p>"
            ),
            10,
        );
        settle(&store);
        let e = store.extraction_for(lure).unwrap().unwrap();
        assert_eq!((e.kind, e.suppressed), (ExtractionKind::Suppressed, Some(orders::SuppressReason::DeliveryFeeLure)));
        assert!(store.order_for_message(lure).unwrap().is_none());
        assert_eq!(store.orders(true).unwrap().len(), 1);
    }

    #[test]
    fn a_topic_from_order_words_follows_order_detection() {
        let (store, acct) = store_with_account();
        let mut st = store.settings();
        st.order_tracking = false;
        store.save_settings(&st).unwrap();
        let id = mail(&store, acct, BRIGHTLEAF, "Thanks for your order", "<p>Thanks for your order! We'll let you know when your order has shipped.</p>", 60);
        settle(&store);
        let a = store.topic_for(id).unwrap().unwrap();
        assert_eq!((a.topic, a.source), (Topic::Orders, Source::Signals));
        st.order_tracking = true;
        store.save_settings(&st).unwrap();
        settle(&store);
        assert_eq!(store.extraction_for(id).unwrap().unwrap().kind, ExtractionKind::None);
        assert_ne!(store.topic_for(id).unwrap().unwrap().topic, Topic::Orders);
    }

    #[test]
    fn danger_reassessment_suppresses() {
        let (store, acct) = store_with_account();
        let ids = [confirmation(&store, acct, 2 * 1440), shipped(&store, acct, UPS, 1440)];
        settle(&store);
        only_order(&store);
        // A later check finds signs of phishing.
        let danger = Analysis {
            findings: vec![Finding::LinkTextMismatch { shown: "brightleaf.example".into(), target: "evil.example".into() }],
            ..Default::default()
        };
        assert_eq!(danger.severity(), Some(Severity::Danger));
        store
            .conn()
            .execute("UPDATE junk_assessments SET version = 'later', gate = 'danger', json = ?1", [serde_json::to_string(&danger).unwrap()])
            .unwrap();
        let now = Utc::now();
        while store.extract_orders_pending(250, now).unwrap() + store.rebuild_dirty_orders(200).unwrap() > 0 {}
        assert!(store.orders(true).unwrap().is_empty());
        assert_eq!(store.extraction_for(ids[0]).unwrap().unwrap().suppressed, Some(orders::SuppressReason::Danger));
        while store.classify_topics_pending(250).unwrap() > 0 {}
        for id in ids {
            let a = store.topic_for(id).unwrap().unwrap();
            assert_eq!((a.topic, a.source), (Topic::Other, Source::Suspicious));
        }
    }

    #[test]
    fn a_free_mail_domain_is_never_ignored_whole() {
        let (store, _) = store_with_account();
        assert!(matches!(store.ignore_order_sender("gmail.com"), Err(Error::Invalid(_))));
        assert!(matches!(store.ignore_order_sender("@GMail.com"), Err(Error::Invalid(_))));
        store.ignore_order_sender("janespottery@gmail.com").unwrap();
        assert_eq!(store.order_sender_ignores().unwrap().len(), 1);
    }

    #[test]
    fn a_rule_changed_while_a_batch_is_worked_out_is_kept() {
        let (store, acct) = store_with_account();
        let s = shipped(&store, acct, UPS, 60);
        store.assess_pending(250).unwrap();
        checked_as_danger(&store);
        // "Always allow" while the batch reads the mail.
        during_next_batch(|st| {
            st.set_sender_rule(None, "orders@brightleaf.example", "", RuleAction::Allow).unwrap();
        });
        assert_eq!(store.extract_orders_pending(250, Utc::now()).unwrap(), 1);
        assert!(store.extraction_for(s).unwrap().is_none(), "worked out before the rule: not written");
        settle(&store);
        assert_eq!(store.extraction_for(s).unwrap().unwrap().kind, ExtractionKind::Order);
        assert!(store.order_for_message(s).unwrap().is_some());
        // And the rule going meanwhile.
        let rule = store.sender_rules().unwrap().remove(0);
        store.conn().execute("DELETE FROM order_extractions", []).unwrap();
        during_next_batch(move |st| {
            st.remove_sender_rule(rule.id).unwrap();
        });
        store.extract_orders_pending(250, Utc::now()).unwrap();
        assert!(store.extraction_for(s).unwrap().is_none());
        settle(&store);
        assert_eq!(store.extraction_for(s).unwrap().unwrap().suppressed, Some(orders::SuppressReason::Danger));
        assert!(store.orders(true).unwrap().is_empty());
    }

    #[test]
    fn a_new_message_never_inherits_a_deleted_ones_phishing_report() {
        let (store, acct) = store_with_account();
        confirmation(&store, acct, 2 * 1440);
        let lure = mail(&store, acct, ("Brightleaf", "orders@brightleaf.example"), "Your order has shipped", "<p>Order number: BL-99999</p>", 60);
        settle(&store);
        store.report_junk(&[lure], crate::junk::Label::Phishing, false).unwrap();
        store.delete_messages_permanently(&[lure]).unwrap();
        // The next message gets the deleted one's id.
        let next = shipped(&store, acct, UPS, 10);
        assert_eq!(next, lure, "SQLite reuses the largest id");
        settle(&store);
        assert_eq!(store.extraction_for(next).unwrap().unwrap().kind, ExtractionKind::Order);
        assert_eq!(only_order(&store).events.len(), 2);
        assert_eq!(store.topic_for(next).unwrap().unwrap().topic, Topic::Orders);
    }

    #[test]
    fn a_check_redone_to_the_same_verdict_redoes_nothing() {
        let (store, acct) = store_with_account();
        confirmation(&store, acct, 2 * 1440);
        shipped(&store, acct, UPS, 1440);
        for i in 0..5 {
            plain(&store, acct, i);
        }
        settle(&store);
        // Writing to someone new changes every check's version.
        store.record_recent_addresses(&[EmailAddress::new("", "new.person@elsewhere.example")]).unwrap();
        assert_eq!(store.assess_pending(250).unwrap(), 7, "every message is checked again");
        let now = Utc::now();
        assert_eq!(store.orders_pending_count(10, now).unwrap(), 0);
        assert_eq!(store.extract_orders_pending(250, now).unwrap(), 0);
        assert_eq!(store.rebuild_dirty_orders(200).unwrap(), 0);
        assert_eq!(store.classify_topics_pending(250).unwrap(), 0);
        // A new verdict still counts.
        checked_as_danger(&store);
        assert_eq!(store.orders_pending_count(10, now).unwrap(), 2);
        settle(&store);
        assert!(store.orders(true).unwrap().is_empty());
    }

    #[test]
    fn switching_tracking_off_resorts_mail_order_detection_turned_down() {
        let (store, acct) = store_with_account();
        let words = "<p>Thanks for your order! We'll let you know when your order has shipped.</p>";
        let id = mail(&store, acct, BRIGHTLEAF, "Thanks for your order", words, 60);
        settle(&store);
        assert_eq!(store.extraction_for(id).unwrap().unwrap().kind, ExtractionKind::None);
        assert_ne!(store.topic_for(id).unwrap().unwrap().topic, Topic::Orders);
        let mut st = store.settings();
        st.order_tracking = false;
        store.save_settings(&st).unwrap();
        store.clear_orders().unwrap();
        settle(&store);
        // Sorted as the same mail arriving now would be.
        let fresh = mail(&store, acct, BRIGHTLEAF, "Thanks for your order", words, 30);
        settle(&store);
        let (a, b) = (store.topic_for(id).unwrap().unwrap(), store.topic_for(fresh).unwrap().unwrap());
        assert_eq!((a.topic, a.source), (b.topic, b.source));
        assert_eq!(a.topic, Topic::Orders);
    }

    #[test]
    fn switching_tracking_off_keeps_the_topics_shown_until_sorted_again() {
        let (store, acct) = store_with_account();
        for i in 0..20 {
            plain(&store, acct, i);
        }
        let order = confirmation(&store, acct, 30);
        settle(&store);
        let ids: Vec<MessageId> = (1..=21).collect();
        let before = store.topics_for(&ids).unwrap();
        assert_eq!(before.len(), 21);
        let total = store.topic_counts().unwrap().iter().map(|c| c.total).sum::<u32>();
        let mut st = store.settings();
        st.order_tracking = false;
        store.save_settings(&st).unwrap();
        store.clear_orders().unwrap();
        // Still shown (only the order email's topic goes), but sorted again.
        let kept = store.topics_for(&ids).unwrap();
        assert_eq!(kept.len(), 20);
        assert!(!kept.contains_key(&order));
        assert_eq!(store.topic_counts().unwrap().iter().map(|c| c.total).sum::<u32>(), total - 1);
        assert_eq!(store.topics_pending_count(1000).unwrap(), 21);
        settle(&store);
        assert_eq!(store.topics_pending_count(1000).unwrap(), 0);
        assert_eq!(store.topics_for(&ids).unwrap().len(), 21);
        assert_eq!(count(&store, "SELECT COUNT(*) FROM topic_assignments WHERE stale != 0"), 0);
    }
}
