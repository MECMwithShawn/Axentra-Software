//! Topics: the cached topic of each message and the user's corrections
//! (per message, per sender, per domain). The pure logic lives in
//! [`crate::topics`].
//!
//! A message is pending until it has a row with the current version
//! (`t{CLASSIFY_VERSION}-c{topics.context_rev}`). Triggers drop a row when
//! the message, its body or attachments, or its order extraction change;
//! a changed contact queues only that contact's addresses. Corrections
//! update the cached rows they cover in place, so nothing flickers.

use std::collections::{HashMap, HashSet};

use chrono::Utc;
use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, params, params_from_iter};

use super::mail::id_list;
use super::{Store, bump_sql, changed_sql, has_column, kv_level, no_insights_folders_sql, own_mail_folders_sql, sender_match_sql, settings_in};
use crate::junk::analyze::Severity;
use crate::model::{AccountId, FolderId, Message, MessageId};
use crate::orders::ExtractionKind;
use crate::topics::{self, Assignment, OrderHint, Reason, Source, Topic, TopicRule};
use crate::{Error, Result};

pub(crate) const SCHEMA_EXTRA: &str = r#"
CREATE TABLE IF NOT EXISTS topic_assignments (
    message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    version TEXT NOT NULL,
    junk_version TEXT NOT NULL DEFAULT '',
    topic TEXT NOT NULL,
    confidence REAL NOT NULL,
    source TEXT NOT NULL,
    json TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    stale INTEGER NOT NULL DEFAULT 0);
CREATE INDEX IF NOT EXISTS topic_assignments_topic ON topic_assignments(topic, version);
CREATE TABLE IF NOT EXISTS topic_sender_rules (
    id INTEGER PRIMARY KEY,
    account_key INTEGER NOT NULL DEFAULT 0,
    pattern TEXT NOT NULL,
    is_domain INTEGER NOT NULL,
    topic TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (account_key, pattern));
CREATE TABLE IF NOT EXISTS topic_message_overrides (
    message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    topic TEXT NOT NULL,
    created_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS topic_dirty_senders (address TEXT PRIMARY KEY) WITHOUT ROWID;
"#;

/// Schema level of the topic tables (see [`upgrade_schema_in`]).
const SCHEMA_LEVEL: &str = "topics.schema";
/// Bumped when every message should be sorted again (corrections
/// forgotten, "Sort my mail again").
const CONTEXT_REVISION: &str = "topics.context_rev";

/// Messages still to sort: no topic, an older version, a topic marked
/// `stale` (shown until it is sorted again), or a topic a phishing verdict
/// can change whose junk check changed since. Like
/// Packages, a message waits for its first junk check (phishing never gets
/// the Orders, Finance or Receipts topic, not even for a moment). `?1`
/// version, `?2` limit.
const PENDING: &str = "SELECT m.id FROM messages m
    JOIN folders f ON f.id = m.folder_id
    JOIN junk_assessments j ON j.message_id = m.id
    LEFT JOIN topic_assignments t ON t.message_id = m.id
    WHERE m.folder_id NOT IN OWN_MAIL AND m.is_draft = 0
      AND (t.message_id IS NULL OR t.version != ?1 OR t.stale != 0 OR STALE_GATE)";

/// A topic (row `t`) whose junk check matters: phishing is sorted elsewhere.
const SENSITIVE: &str = "(t.source = 'suspicious' OR t.topic IN ('orders', 'finance', 'receipts'))";

/// A topic (row `t`) worked out with another junk gate than the check's
/// (`GATE`) that the change matters to: a topic phishing can't have, or
/// any topic when the message became phishing or stopped being it (it is
/// sorted as Suspicious, whatever its topic was: an impersonation in
/// Events or People included). A check redone to the same verdict, or
/// from no signs to a caution on a newsletter, changes nothing.
const STALE_GATE: &str = "(t.junk_version IS NOT GATE AND (SENSITIVE
    OR GATE LIKE 'danger%' OR GATE LIKE '%:phishing' OR t.junk_version LIKE 'danger%' OR t.junk_version LIKE '%:phishing'))";

/// [`STALE_GATE`] with its parts filled in, for a check row `j` (`GATE`
/// is `NULL` when there is none).
fn stale_gate_sql(gate: &str) -> String {
    STALE_GATE.replace("SENSITIVE", SENSITIVE).replace("GATE", gate)
}

/// [`PENDING`] with its parts filled in.
fn pending_sql() -> String {
    PENDING.replace("STALE_GATE", &stale_gate_sql(super::junk::GATE_VERSION_SQL)).replace("OWN_MAIL", &own_mail_folders_sql())
}

/// One topic's count in the folder pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TopicCount {
    pub topic: Topic,
    pub total: u32,
    pub unread: u32,
}

/// What a topic correction covers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TopicTarget {
    Message(MessageId),
    /// An address.
    Sender(String),
    /// A domain and its subdomains (never a free mail provider).
    Domain(String),
}

/// A correction as made, so Undo can put the previous one back.
#[derive(Clone, Debug, PartialEq)]
pub struct TopicChange {
    /// `None` = all accounts.
    pub account: Option<AccountId>,
    pub target: TopicTarget,
    pub previous: Option<Topic>,
    pub new: Option<Topic>,
}

/// A stored sender or domain correction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopicRuleRow {
    pub id: i64,
    pub rule: TopicRule,
    pub updated_at: i64,
}

/// The version every current topic row has.
pub(crate) fn current_version(conn: &Connection) -> Result<String> {
    Ok(format!("t{}-c{}", topics::CLASSIFY_VERSION, kv_level(conn, CONTEXT_REVISION)?))
}

/// Triggers that keep topics honest: what a message says (sender,
/// recipients, subject, meeting, body, attachments) or what order
/// detection made of it drops its topic; a changed contact queues its
/// addresses (only their mail is sorted again).
fn triggers_sql() -> String {
    let drop = |name: &str, when: &str, id: &str| {
        format!("CREATE TRIGGER IF NOT EXISTS {name} {when} BEGIN DELETE FROM topic_assignments WHERE message_id = {id}; END;\n")
    };
    let dirty = |emails: &str| {
        format!(
            "INSERT OR IGNORE INTO topic_dirty_senders (address)
             SELECT lower(json_extract(e.value, '$.value')) FROM json_each({emails}, '$.emails') e
             WHERE json_extract(e.value, '$.value') IS NOT NULL;"
        )
    };
    let mut s = String::new();
    s += &drop(
        "topics_reclassify_message",
        &format!("AFTER UPDATE OF json ON messages WHEN {}", changed_sql(&["from", "subject", "account_id", "meeting_kind", "to", "cc"])),
        "NEW.id",
    );
    s += &drop("topics_reclassify_body_insert", "AFTER INSERT ON message_bodies", "NEW.message_id");
    s += &drop("topics_reclassify_body_update", "AFTER UPDATE ON message_bodies WHEN OLD.html IS NOT NEW.html OR OLD.text IS NOT NEW.text", "NEW.message_id");
    s += &drop("topics_reclassify_body_delete", "AFTER DELETE ON message_bodies", "OLD.message_id");
    s += &drop("topics_reclassify_attachment_insert", "AFTER INSERT ON attachments", "NEW.message_id");
    s += &drop("topics_reclassify_attachment_delete", "AFTER DELETE ON attachments", "OLD.message_id");
    s += &drop("topics_follow_order_insert", "AFTER INSERT ON order_extractions WHEN NEW.kind != 'none'", "NEW.message_id");
    s += &drop("topics_follow_order_update", "AFTER UPDATE OF kind ON order_extractions WHEN OLD.kind IS NOT NEW.kind", "NEW.message_id");
    // An order email's topic goes with its extraction. A topic worked out
    // knowing detection said "not an order" (its order words were left
    // out) is sorted again by its words, with tracking off or before
    // detection looks again, but stays shown until then: almost all mail
    // is no order email, and switching tracking off must not empty the
    // topic lists for the length of a whole pass.
    s += "CREATE TRIGGER IF NOT EXISTS topics_follow_order_delete AFTER DELETE ON order_extractions BEGIN
            DELETE FROM topic_assignments WHERE message_id = OLD.message_id AND OLD.kind != 'none';
            UPDATE topic_assignments SET stale = 1 WHERE message_id = OLD.message_id AND OLD.kind = 'none'
              AND source IN ('signals', 'fallback', 'suspicious'); END;\n";
    // Sorted into Orders by its words before order detection looked (with
    // tracking off, or on the spot in the reading pane), and detection says
    // it isn't an order: sorted again.
    s += "CREATE TRIGGER IF NOT EXISTS topics_follow_order_none AFTER INSERT ON order_extractions WHEN NEW.kind = 'none' BEGIN
            DELETE FROM topic_assignments WHERE message_id = NEW.message_id AND topic = 'orders' AND source = 'signals'; END;\n";
    s += &format!("CREATE TRIGGER IF NOT EXISTS topics_people_contact_insert AFTER INSERT ON contacts BEGIN {} END;\n", dirty("NEW.json"));
    s += &format!(
        "CREATE TRIGGER IF NOT EXISTS topics_people_contact_update AFTER UPDATE ON contacts WHEN OLD.json IS NOT NEW.json BEGIN {} {} END;\n",
        dirty("OLD.json"),
        dirty("NEW.json")
    );
    s += &format!("CREATE TRIGGER IF NOT EXISTS topics_people_contact_delete AFTER DELETE ON contacts BEGIN {} END;\n", dirty("OLD.json"));
    s
}

/// Create the triggers (after the order tables: some follow
/// `order_extractions`). A later change to a trigger's body needs a
/// `DROP TRIGGER` gated on a higher [`SCHEMA_LEVEL`].
pub(crate) fn upgrade_schema_in(conn: &Connection) -> Result<()> {
    let level = kv_level(conn, SCHEMA_LEVEL)?;
    if !has_column(conn, "topic_assignments", "stale")? {
        conn.execute("ALTER TABLE topic_assignments ADD COLUMN stale INTEGER NOT NULL DEFAULT 0", [])?;
    }
    if level < 3 {
        // Level 2: now also follows a "not an order" extraction. Level 3:
        // marks that one stale instead of dropping it.
        conn.execute("DROP TRIGGER IF EXISTS topics_follow_order_delete", [])?;
        conn.execute("INSERT OR REPLACE INTO kv (key, value) VALUES (?1, '3')", [SCHEMA_LEVEL])?;
    }
    conn.execute_batch(&triggers_sql())?;
    Ok(())
}

/// The account's own corrections (its messages, their topics and
/// per-message corrections go with the account itself).
pub(crate) fn remove_account_in(conn: &Connection, account: AccountId) -> Result<()> {
    conn.execute("DELETE FROM topic_sender_rules WHERE account_key = ?1", [account])?;
    Ok(())
}

fn rule_rows_in(conn: &Connection) -> Result<Vec<TopicRuleRow>> {
    let mut stmt = conn.prepare_cached("SELECT id, account_key, pattern, is_domain, topic, updated_at FROM topic_sender_rules ORDER BY account_key, pattern")?;
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?, r.get::<_, bool>(3)?, r.get::<_, String>(4)?, r.get::<_, i64>(5)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, account_key, pattern, is_domain, topic, updated_at) = row?;
        if let Some(topic) = Topic::parse(&topic) {
            out.push(TopicRuleRow { id, rule: TopicRule { account_key, pattern, is_domain, topic }, updated_at });
        }
    }
    Ok(out)
}

/// The rule that decides for `address` (lowercase) in `account`: the exact
/// address (the account's own rule before an all-accounts one), else the
/// most specific domain (again the account's own first).
fn deciding_rule<'a>(rules: &'a [TopicRule], account: AccountId, address: &str) -> Option<&'a TopicRule> {
    let mine = |r: &TopicRule| u8::from(r.account_key != 0);
    let applies = || rules.iter().filter(move |r| topics::rule_matches(r, account, address));
    applies().filter(|r| !r.is_domain).max_by_key(|r| mine(r)).or_else(|| applies().filter(|r| r.is_domain).max_by_key(|r| (r.pattern.len(), mine(r))))
}

/// Bring the cached topics of the mail `pattern` covers (in `account_key`,
/// 0 = all accounts) up to date with the rules, in place: each takes the
/// rule that now decides for it; with none left it is sorted again.
/// Messages with a correction of their own keep it.
fn apply_rules_in(conn: &Connection, account_key: i64, pattern: &str, is_domain: bool) -> Result<()> {
    let mut sql = format!(
        "SELECT t.message_id, m.account_id, lower(json_extract(m.json, '$.from.address'))
         FROM topic_assignments t JOIN messages m ON m.id = t.message_id
         WHERE {} AND t.message_id NOT IN (SELECT message_id FROM topic_message_overrides)",
        sender_match_sql(is_domain, 1)
    );
    let mut values = vec![Value::Text(pattern.into())];
    if account_key != 0 {
        sql.push_str(" AND m.account_id = ?2");
        values.push(Value::Integer(account_key));
    }
    let covered: Vec<(MessageId, AccountId, String)> = {
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(values), |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, Option<String>>(2)?.unwrap_or_default())))?;
        rows.collect::<std::result::Result<_, _>>()?
    };
    update_covered_in(conn, covered)
}

/// A changed correction: (account key, pattern, is a domain).
type RuleKey = (i64, String, bool);

/// [`apply_rules_in`] for many corrections at once: one pass over the
/// sorted mail, whatever their number (one pass per correction made
/// "Change topic" on hundreds of senders take seconds).
fn apply_rules_batch_in(conn: &Connection, keys: &[RuleKey]) -> Result<()> {
    match keys {
        [] => return Ok(()),
        [(account_key, pattern, is_domain)] => return apply_rules_in(conn, *account_key, pattern, *is_domain),
        _ => {}
    }
    let keys: HashSet<(i64, &str, bool)> = keys.iter().map(|(a, p, d)| (*a, p.as_str(), *d)).collect();
    // An address is covered by its own correction or one for its domain or
    // a parent domain (a label boundary, as `junk::domain_matches`), for
    // its account or all accounts.
    let covers = |account: AccountId, address: &str| {
        let hit = |pattern: &str, domain: bool| keys.contains(&(0, pattern, domain)) || keys.contains(&(account, pattern, domain));
        let mut domain = crate::junk::domain_of(address);
        if hit(address, false) {
            return true;
        }
        while !domain.is_empty() {
            if hit(domain, true) {
                return true;
            }
            domain = domain.split_once('.').map(|(_, rest)| rest).unwrap_or("");
        }
        false
    };
    let covered: Vec<(MessageId, AccountId, String)> = {
        let mut stmt = conn.prepare(
            "SELECT t.message_id, m.account_id, lower(json_extract(m.json, '$.from.address'))
             FROM topic_assignments t JOIN messages m ON m.id = t.message_id
             WHERE t.message_id NOT IN (SELECT message_id FROM topic_message_overrides)",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, MessageId>(0)?, r.get::<_, AccountId>(1)?, r.get::<_, Option<String>>(2)?.unwrap_or_default())))?;
        let mut out = Vec::new();
        for row in rows {
            let (id, account, address) = row?;
            if covers(account, &address) {
                out.push((id, account, address));
            }
        }
        out
    };
    update_covered_in(conn, covered)
}

/// The cached topics of `covered` (id, account, lowercase sender), each
/// set to the rule that now decides for it, or dropped (sorted again) when
/// none does.
fn update_covered_in(conn: &Connection, covered: Vec<(MessageId, AccountId, String)>) -> Result<()> {
    if covered.is_empty() {
        return Ok(());
    }
    let rules: Vec<TopicRule> = rule_rows_in(conn)?.into_iter().map(|r| r.rule).collect();
    let ids: Vec<MessageId> = covered.iter().map(|c| c.0).collect();
    let junk = super::junk::cached_junk_in(conn, &ids)?;
    for (id, account, address) in covered {
        match deciding_rule(&rules, account, &address) {
            Some(rule) => {
                let (source, mut reasons) = if rule.is_domain {
                    (Source::DomainRule, vec![Reason::DomainRule { pattern: rule.pattern.clone() }])
                } else {
                    (Source::SenderRule, vec![Reason::SenderRule { pattern: rule.pattern.clone() }])
                };
                if junk.get(&id).is_some_and(|j| j.1 == Some(Severity::Danger)) {
                    reasons.push(Reason::Suspicious);
                }
                conn.prepare_cached("UPDATE topic_assignments SET topic = ?1, source = ?2, confidence = 1.0, json = ?3 WHERE message_id = ?4")?.execute(
                    params![rule.topic.id(), source.as_str(), serde_json::to_string(&reasons)?, id],
                )?;
            }
            None => {
                conn.prepare_cached("DELETE FROM topic_assignments WHERE message_id = ?1")?.execute([id])?;
            }
        }
    }
    Ok(())
}

/// Write a correction; `None` removes it.
fn put_rule_in(conn: &Connection, account_key: i64, pattern: &str, is_domain: bool, topic: Option<Topic>, now: i64) -> Result<()> {
    match topic {
        Some(t) => conn.execute(
            "INSERT INTO topic_sender_rules (account_key, pattern, is_domain, topic, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5)
             ON CONFLICT(account_key, pattern) DO UPDATE SET is_domain = excluded.is_domain, topic = excluded.topic, updated_at = excluded.updated_at",
            params![account_key, pattern, is_domain, t.id(), now],
        )?,
        None => conn.execute("DELETE FROM topic_sender_rules WHERE account_key = ?1 AND pattern = ?2", params![account_key, pattern])?,
    };
    Ok(())
}

/// Every sender and domain correction as (account key, pattern, is domain,
/// topic, created, updated), for the backup.
pub(crate) type RuleBackupRow = (i64, String, bool, String, i64, i64);

pub(crate) fn rules_for_backup_in(conn: &Connection) -> Result<Vec<RuleBackupRow>> {
    let mut stmt =
        conn.prepare_cached("SELECT account_key, pattern, is_domain, topic, created_at, updated_at FROM topic_sender_rules ORDER BY account_key, pattern")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

/// Every topic worked out (see [`Store::clear_topics`]); corrections stay.
pub(crate) fn clear_in(conn: &Connection) -> Result<()> {
    conn.execute_batch("DELETE FROM topic_assignments; DELETE FROM topic_dirty_senders;")?;
    Ok(())
}

/// Corrections from a backup (each replacing the one for the same account
/// and pattern); mail already sorted takes them at once, in one pass
/// however many there are.
pub(crate) fn restore_rules_in(conn: &Connection, rows: &[RuleBackupRow]) -> Result<()> {
    let mut keys: Vec<RuleKey> = Vec::new();
    for (account_key, pattern, _, topic, created_at, updated_at) in rows {
        let Some((pattern, is_domain)) = crate::junk::normalize_pattern(pattern) else { continue };
        if Topic::parse(topic).is_none() || (is_domain && topics::is_freemail(&pattern)) {
            continue;
        }
        conn.prepare_cached(
            "INSERT OR REPLACE INTO topic_sender_rules (account_key, pattern, is_domain, topic, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?
        .execute(params![account_key, pattern, is_domain, topic, created_at, updated_at])?;
        keys.push((*account_key, pattern, is_domain));
    }
    apply_rules_batch_in(conn, &keys)
}

/// What every message of a batch is sorted with, read once.
struct TopicContext {
    version: String,
    rules: Vec<TopicRule>,
    overrides: HashMap<MessageId, Topic>,
    /// Contact address → display name.
    contacts: HashMap<String, String>,
    /// Addresses you've written to.
    recent: HashSet<String>,
    junk: super::junk::CachedJunk,
    /// Cached extractions as (kind, confidence, merchant name); empty while
    /// order tracking is off, so no order logic runs over the mail.
    orders: HashMap<MessageId, (ExtractionKind, f32, String)>,
}

fn kind_from(s: &str) -> ExtractionKind {
    match s {
        "order" => ExtractionKind::Order,
        "suppressed" => ExtractionKind::Suppressed,
        _ => ExtractionKind::None,
    }
}

/// The per-message corrections of `ids`.
fn overrides_in(conn: &Connection, ids: &[MessageId]) -> Result<HashMap<MessageId, Topic>> {
    let mut stmt = conn.prepare(&format!("SELECT message_id, topic FROM topic_message_overrides WHERE message_id IN ({})", id_list(ids)))?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, MessageId>(0)?, r.get::<_, String>(1)?)))?;
    let mut out = HashMap::new();
    for row in rows {
        let (id, t) = row?;
        if let Some(t) = Topic::parse(&t) {
            out.insert(id, t);
        }
    }
    Ok(out)
}

fn topic_context_in(conn: &Connection, ids: &[MessageId], with_orders: bool) -> Result<TopicContext> {
    let list = id_list(ids);
    let overrides = overrides_in(conn, ids)?;
    let contacts = {
        let mut stmt = conn.prepare_cached(
            "SELECT lower(json_extract(e.value, '$.value')), c.display_name FROM contacts c, json_each(c.json, '$.emails') e
             WHERE json_extract(e.value, '$.value') IS NOT NULL",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        rows.collect::<std::result::Result<HashMap<_, _>, _>>()?
    };
    let recent = {
        let mut stmt = conn.prepare_cached("SELECT lower(address) FROM recent_addresses")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.collect::<std::result::Result<HashSet<_>, _>>()?
    };
    let mut orders = HashMap::new();
    if with_orders {
        // A sender you chose not to track is sorted as if order tracking
        // were off for it (its mail isn't suspicious for that).
        let mut stmt = conn.prepare(&format!(
            "SELECT message_id, kind, confidence, COALESCE(json_extract(json, '$.merchant.name'), '') FROM order_extractions
             WHERE message_id IN ({list}) AND json_extract(json, '$.suppressed') IS NOT 'IgnoredSender'"
        ))?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, MessageId>(0)?, r.get::<_, String>(1)?, r.get::<_, f64>(2)?, r.get::<_, String>(3)?)))?;
        for row in rows {
            let (id, kind, confidence, merchant) = row?;
            orders.insert(id, (kind_from(&kind), confidence as f32, merchant));
        }
    }
    Ok(TopicContext {
        version: current_version(conn)?,
        rules: rule_rows_in(conn)?.into_iter().map(|r| r.rule).collect(),
        overrides,
        contacts,
        recent,
        junk: super::junk::cached_junk_in(conn, ids)?,
        orders,
    })
}

/// Cache one message's topic, unless it was deleted meanwhile. No trigger
/// depends on this table, so a replace is fine.
fn write_assignment_in(conn: &Connection, id: MessageId, ctx: &TopicContext, a: &Assignment, now: i64) -> Result<()> {
    let exists: bool = conn.query_row("SELECT EXISTS (SELECT 1 FROM messages WHERE id = ?1)", [id], |r| r.get(0))?;
    if !exists {
        return Ok(());
    }
    let junk_version = ctx.junk.get(&id).map(|j| j.0.as_str()).unwrap_or("");
    conn.execute(
        "INSERT OR REPLACE INTO topic_assignments (message_id, version, junk_version, topic, confidence, source, json, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![id, ctx.version, junk_version, a.topic.id(), f64::from(a.confidence), a.source.as_str(), serde_json::to_string(&a.reasons)?, now],
    )?;
    Ok(())
}

/// Queued contact changes: only their addresses' mail is sorted again.
fn drain_dirty_senders_in(conn: &Connection) -> Result<()> {
    let any: bool = conn.query_row("SELECT EXISTS (SELECT 1 FROM topic_dirty_senders)", [], |r| r.get(0))?;
    if any {
        conn.execute_batch(
            "DELETE FROM topic_assignments WHERE message_id IN (SELECT id FROM messages
                WHERE lower(json_extract(json, '$.from.address')) IN (SELECT address FROM topic_dirty_senders));
             DELETE FROM topic_dirty_senders;",
        )?;
    }
    Ok(())
}

/// A correction's address or domain, lowercase; free mail domains are
/// refused (a correction would cover strangers).
fn rule_pattern(target: &TopicTarget) -> Result<Option<(String, bool)>> {
    let (input, domain) = match target {
        TopicTarget::Message(_) => return Ok(None),
        TopicTarget::Sender(a) => (a, false),
        TopicTarget::Domain(d) => (d, true),
    };
    let pattern = crate::junk::normalize_pattern(input)
        .filter(|(_, is_domain)| *is_domain == domain)
        .ok_or_else(|| Error::Invalid(format!("\u{201c}{}\u{201d} isn't an email {}", input.trim(), if domain { "domain" } else { "address" })))?;
    if domain && topics::is_freemail(&pattern.0) {
        return Err(Error::Invalid(format!("{} is used by lots of people, so it can't have a topic of its own", pattern.0)));
    }
    Ok(Some(pattern))
}

impl Store {
    /// Sort up to `limit` messages that have no current topic, newest
    /// first. Returns how many were done; call again until it returns 0.
    /// Nothing is done while topics are off.
    pub fn classify_topics_pending(&self, limit: u32) -> Result<usize> {
        let settings = self.settings();
        if !settings.topics_view {
            return Ok(0);
        }
        let version = self.tx(|tx| {
            drain_dirty_senders_in(tx)?;
            current_version(tx)
        })?;
        let ids: Vec<MessageId> = {
            let conn = self.conn();
            let mut stmt = conn.prepare_cached(&format!("{} ORDER BY m.received_at DESC LIMIT ?2", pending_sql()))?;
            let rows = stmt.query_map(params![version, limit], |r| r.get(0))?;
            rows.collect::<std::result::Result<_, _>>()?
        };
        if ids.is_empty() {
            return Ok(0);
        }
        let msgs = self.messages_by_ids(&ids)?;
        let ctx = topic_context_in(&self.conn(), &ids, settings.order_tracking)?;
        let mut done = Vec::with_capacity(msgs.len());
        for m in &msgs {
            done.push(self.classify_one(m, &ctx)?);
        }
        let now = Utc::now().timestamp();
        #[cfg(test)]
        super::run_meanwhile(self);
        self.tx(|tx| {
            // Switched off, or a correction or order tracking changed while
            // the batch was worked out: it is stale, and the next batch
            // sorts it again.
            let current = settings_in(tx);
            if !current.topics_view
                || current.order_tracking != settings.order_tracking
                || rule_rows_in(tx)?.into_iter().map(|r| r.rule).collect::<Vec<_>>() != ctx.rules
                || overrides_in(tx, &ids)? != ctx.overrides
            {
                return Ok(());
            }
            for (m, a) in msgs.iter().zip(&done) {
                write_assignment_in(tx, m.id, &ctx, a, now)?;
            }
            Ok(())
        })?;
        Ok(msgs.len())
    }

    /// One message's topic, from stored data only.
    fn classify_one(&self, m: &Message, ctx: &TopicContext) -> Result<Assignment> {
        let body = self.message_body(m.id)?.unwrap_or_default();
        let address = m.from.address.trim().to_lowercase();
        let order = ctx.orders.get(&m.id).map(|(kind, confidence, merchant)| OrderHint { kind: *kind, confidence: *confidence, merchant });
        Ok(topics::classify_guarded(&topics::Input {
            message: m,
            html: body.html.as_deref(),
            text: &body.text,
            attachments: &body.attachments,
            order,
            junk: ctx.junk.get(&m.id).and_then(|j| j.1),
            impersonation: ctx.junk.get(&m.id).is_some_and(|j| j.3),
            sender_authenticated: ctx.junk.get(&m.id).is_some_and(|j| j.4.vouches_for(crate::junk::domain_of(&address))),
            contact_name: ctx.contacts.get(&address).map(String::as_str),
            replied_before: ctx.recent.contains(&address),
            rules: &ctx.rules,
            message_override: ctx.overrides.get(&m.id).copied(),
        }))
    }

    /// How many messages still need sorting, counted up to `cap`.
    pub fn topics_pending_count(&self, cap: u32) -> Result<u32> {
        if !self.settings().topics_view {
            return Ok(0);
        }
        let conn = self.conn();
        let version = current_version(&conn)?;
        let n: i64 = conn.query_row(&format!("SELECT COUNT(*) FROM ({} LIMIT ?2)", pending_sql()), params![version, cap], |r| r.get(0))?;
        Ok(n as u32)
    }

    /// Total and unread mail per topic (one query), every topic in display
    /// order. Junk Email, Deleted Items, your own mail and snoozed mail not
    /// yet due aren't counted (as in the topic lists, `Store::messages`).
    pub fn topic_counts(&self) -> Result<Vec<TopicCount>> {
        let conn = self.conn();
        let version = current_version(&conn)?;
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT t.topic, COUNT(*), SUM(m.is_read = 0)
             FROM topic_assignments t JOIN messages m ON m.id = t.message_id
             WHERE t.version = ?1 AND m.folder_id NOT IN {} AND m.is_draft = 0
               AND (m.snoozed_until IS NULL OR m.snoozed_until <= ?2)
             GROUP BY t.topic",
            no_insights_folders_sql()
        ))?;
        let rows = stmt.query_map(params![version, Utc::now().timestamp()], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)))?;
        let mut counts: Vec<TopicCount> = Topic::ALL.into_iter().map(|topic| TopicCount { topic, total: 0, unread: 0 }).collect();
        for row in rows {
            let (topic, total, unread) = row?;
            if let Some(t) = Topic::parse(&topic) {
                counts[t.index()] = TopicCount { topic: t, total: total as u32, unread: unread as u32 };
            }
        }
        Ok(counts)
    }

    /// The cached topics of `ids` (current version only).
    pub fn topics_for(&self, ids: &[MessageId]) -> Result<HashMap<MessageId, (Topic, f32)>> {
        let mut out = HashMap::new();
        if ids.is_empty() {
            return Ok(out);
        }
        let conn = self.conn();
        let version = current_version(&conn)?;
        for chunk in ids.chunks(500) {
            let mut stmt = conn.prepare(&format!(
                "SELECT message_id, topic, confidence FROM topic_assignments WHERE version = ?1 AND message_id IN ({})",
                id_list(chunk)
            ))?;
            let rows = stmt.query_map([&version], |r| Ok((r.get::<_, MessageId>(0)?, r.get::<_, String>(1)?, r.get::<_, f64>(2)?)))?;
            for row in rows {
                let (id, topic, confidence) = row?;
                if let Some(t) = Topic::parse(&topic) {
                    out.insert(id, (t, confidence as f32));
                }
            }
        }
        Ok(out)
    }

    /// A message's topic: cached, else worked out now and cached (the
    /// reading pane, which checks the message for warning signs too: that
    /// check comes first, so the topic always knows of it). `None` while
    /// topics are off, and for your own mail.
    pub fn topic_for(&self, id: MessageId) -> Result<Option<Assignment>> {
        let settings = self.settings();
        if !settings.topics_view {
            return Ok(None);
        }
        let Some(m) = self.message(id)? else { return Ok(None) };
        // Your own mail, in a folder under Sent Items too: as the pass.
        let own: bool = self.conn().query_row(&format!("SELECT ?1 IN {}", own_mail_folders_sql()), [m.folder_id], |r| r.get(0))?;
        if m.is_draft || own {
            return Ok(None);
        }
        self.assessment(id)?;
        let cached: Option<(String, f64, String, String)> = {
            let conn = self.conn();
            let version = current_version(&conn)?;
            conn.query_row(
                &format!(
                    "SELECT t.topic, t.confidence, t.source, t.json FROM topic_assignments t LEFT JOIN junk_assessments j ON j.message_id = t.message_id
                     WHERE t.message_id = ?1 AND t.version = ?2 AND t.stale = 0 AND NOT {}",
                    stale_gate_sql(&format!("COALESCE({}, '')", super::junk::GATE_VERSION_SQL))
                ),
                params![id, version],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?
        };
        if let Some((topic, confidence, source, json)) = cached
            && let (Some(topic), Some(source)) = (Topic::parse(&topic), Source::parse(&source))
        {
            let reasons = serde_json::from_str(&json).unwrap_or_default();
            return Ok(Some(Assignment { topic, confidence: confidence as f32, source, reasons }));
        }
        let ctx = topic_context_in(&self.conn(), &[id], settings.order_tracking)?;
        let a = self.classify_one(&m, &ctx)?;
        write_assignment_in(&self.conn(), id, &ctx, &a, Utc::now().timestamp())?;
        Ok(Some(a))
    }

    /// The topic of each of `msgs`, to sort them into folders (auto-archive):
    /// cached as in [`Store::topic_for`], else worked out now (after the
    /// message's junk check) and cached. Works while Topics is off too.
    pub fn sort_topics(&self, msgs: &[Message]) -> Result<HashMap<MessageId, Topic>> {
        let mut out = HashMap::new();
        {
            let conn = self.conn();
            let version = current_version(&conn)?;
            for chunk in msgs.chunks(500) {
                let ids: Vec<MessageId> = chunk.iter().map(|m| m.id).collect();
                let mut stmt = conn.prepare(&format!(
                    "SELECT t.message_id, t.topic FROM topic_assignments t LEFT JOIN junk_assessments j ON j.message_id = t.message_id
                     WHERE t.message_id IN ({}) AND t.version = ?1 AND t.stale = 0 AND NOT {}",
                    id_list(&ids),
                    stale_gate_sql(&format!("COALESCE({}, '')", super::junk::GATE_VERSION_SQL))
                ))?;
                let rows = stmt.query_map([&version], |r| Ok((r.get::<_, MessageId>(0)?, r.get::<_, String>(1)?)))?;
                for row in rows {
                    let (id, topic) = row?;
                    if let Some(t) = Topic::parse(&topic) {
                        out.insert(id, t);
                    }
                }
            }
        }
        let missing: Vec<&Message> = msgs.iter().filter(|m| !out.contains_key(&m.id)).collect();
        let order_tracking = self.settings().order_tracking;
        for chunk in missing.chunks(500) {
            for m in chunk {
                self.assessment(m.id)?;
            }
            let ids: Vec<MessageId> = chunk.iter().map(|m| m.id).collect();
            let ctx = topic_context_in(&self.conn(), &ids, order_tracking)?;
            let now = Utc::now().timestamp();
            for m in chunk {
                let a = self.classify_one(m, &ctx)?;
                write_assignment_in(&self.conn(), m.id, &ctx, &a, now)?;
                out.insert(m.id, a.topic);
            }
        }
        Ok(out)
    }

    /// True when mail in `folder` has a topic to show and change: not your
    /// own mail, Junk Email or Deleted Items, nor a folder under them (as
    /// the topic lists and counts).
    pub fn folder_has_topics(&self, folder: FolderId) -> Result<bool> {
        let excluded: bool = self.conn().query_row(&format!("SELECT ?1 IN {}", no_insights_folders_sql()), [folder], |r| r.get(0))?;
        Ok(!excluded)
    }

    /// Correct a topic; `None` removes the correction. Mail already sorted
    /// takes it at once. Returns the change, for Undo.
    pub fn set_topic(&self, account: Option<AccountId>, target: TopicTarget, topic: Option<Topic>) -> Result<TopicChange> {
        let mut changes = self.set_topics(vec![(account, target, topic)])?;
        changes.pop().ok_or_else(|| Error::Invalid("nothing to correct".into()))
    }

    /// [`Store::set_topic`] for many corrections, made in order in one
    /// transaction, with the sorted mail brought up to date in one pass at
    /// the end. All or none: an invalid one changes nothing. Returns the
    /// changes in order, for Undo.
    pub fn set_topics(&self, changes: Vec<(Option<AccountId>, TopicTarget, Option<Topic>)>) -> Result<Vec<TopicChange>> {
        let patterns = changes.iter().map(|(_, target, _)| rule_pattern(target)).collect::<Result<Vec<_>>>()?;
        let now = Utc::now().timestamp();
        self.tx(|tx| {
            let mut out = Vec::with_capacity(changes.len());
            let mut keys: Vec<RuleKey> = Vec::new();
            for ((account, target, topic), pattern) in changes.iter().zip(&patterns) {
                let (change, key) = put_topic_in(tx, *account, target, *topic, pattern, now)?;
                if let Some(key) = key.filter(|k| !keys.contains(k)) {
                    keys.push(key);
                }
                out.push(change);
            }
            apply_rules_batch_in(tx, &keys)?;
            Ok(out)
        })
    }

    /// Undo [`Store::set_topics`]: the previous corrections (or none) are
    /// back, last change first.
    pub fn undo_topics(&self, changes: &[TopicChange]) -> Result<()> {
        self.set_topics(changes.iter().rev().map(|c| (c.account, c.target.clone(), c.previous)).collect()).map(|_| ())
    }
}

/// One correction of [`Store::set_topics`] (its `pattern` from
/// [`rule_pattern`]): a message's applied at once, a sender's or domain's
/// rule written and returned to be applied with the rest.
fn put_topic_in(
    tx: &Connection,
    account: Option<AccountId>,
    target: &TopicTarget,
    topic: Option<Topic>,
    pattern: &Option<(String, bool)>,
    now: i64,
) -> Result<(TopicChange, Option<RuleKey>)> {
    let target = match (target, pattern) {
        (TopicTarget::Message(id), _) => {
            let id = *id;
            let previous: Option<String> = tx.query_row("SELECT topic FROM topic_message_overrides WHERE message_id = ?1", [id], |r| r.get(0)).optional()?;
            match topic {
                Some(t) => {
                    tx.execute(
                        "INSERT INTO topic_message_overrides (message_id, topic, created_at) VALUES (?1, ?2, ?3)
                         ON CONFLICT(message_id) DO UPDATE SET topic = excluded.topic",
                        params![id, t.id(), now],
                    )?;
                    tx.execute(
                        "UPDATE topic_assignments SET topic = ?1, source = ?2, confidence = 1.0, json = ?3 WHERE message_id = ?4",
                        params![t.id(), Source::MessageOverride.as_str(), serde_json::to_string(&[Reason::MessageOverride])?, id],
                    )?;
                }
                None => {
                    tx.execute("DELETE FROM topic_message_overrides WHERE message_id = ?1", [id])?;
                    tx.execute("DELETE FROM topic_assignments WHERE message_id = ?1", [id])?;
                }
            }
            let change = TopicChange { account, target: TopicTarget::Message(id), previous: previous.as_deref().and_then(Topic::parse), new: topic };
            return Ok((change, None));
        }
        (TopicTarget::Sender(_), Some((p, _))) => TopicTarget::Sender(p.clone()),
        (TopicTarget::Domain(_), Some((p, _))) => TopicTarget::Domain(p.clone()),
        _ => return Err(Error::Invalid("nothing to correct".into())),
    };
    let account_key = account.unwrap_or(0);
    let (p, is_domain) = pattern.clone().unwrap_or_default();
    let previous: Option<String> =
        tx.query_row("SELECT topic FROM topic_sender_rules WHERE account_key = ?1 AND pattern = ?2", params![account_key, p], |r| r.get(0)).optional()?;
    put_rule_in(tx, account_key, &p, is_domain, topic, now)?;
    let change = TopicChange { account, target, previous: previous.as_deref().and_then(Topic::parse), new: topic };
    Ok((change, Some((account_key, p, is_domain))))
}

impl Store {
    /// Undo [`Store::set_topic`]: the previous correction (or none) is back.
    pub fn undo_topic(&self, change: &TopicChange) -> Result<()> {
        self.set_topic(change.account, change.target.clone(), change.previous).map(|_| ())
    }

    /// The topics set for single messages of `ids` ("Only this message").
    pub fn topic_overrides(&self, ids: &[MessageId]) -> Result<HashMap<MessageId, Topic>> {
        let conn = self.conn();
        let mut out = HashMap::new();
        for chunk in ids.chunks(500) {
            let mut stmt = conn.prepare(&format!("SELECT message_id, topic FROM topic_message_overrides WHERE message_id IN ({})", id_list(chunk)))?;
            for row in stmt.query_map([], |r| Ok((r.get::<_, MessageId>(0)?, r.get::<_, String>(1)?)))? {
                let (id, topic) = row?;
                if let Some(t) = Topic::parse(&topic) {
                    out.insert(id, t);
                }
            }
        }
        Ok(out)
    }

    pub fn topic_rules(&self) -> Result<Vec<TopicRuleRow>> {
        rule_rows_in(&self.conn())
    }

    /// Remove a sender or domain correction; its mail is sorted again.
    pub fn remove_topic_rule(&self, id: i64) -> Result<()> {
        self.tx(|tx| {
            let rule: Option<(i64, String, bool)> = tx
                .query_row("SELECT account_key, pattern, is_domain FROM topic_sender_rules WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .optional()?;
            let Some((account_key, pattern, is_domain)) = rule else { return Ok(()) };
            tx.execute("DELETE FROM topic_sender_rules WHERE id = ?1", [id])?;
            apply_rules_in(tx, account_key, &pattern, is_domain)
        })
    }

    /// Forget every correction; all mail is sorted again.
    pub fn clear_topic_rules(&self) -> Result<()> {
        self.tx(|tx| {
            tx.execute_batch("DELETE FROM topic_sender_rules; DELETE FROM topic_message_overrides;")?;
            tx.execute_batch(&bump_sql(CONTEXT_REVISION))?;
            Ok(())
        })
    }

    /// Forget every sorted topic (sorted again from the mail); keeps the
    /// corrections.
    pub fn clear_topics(&self) -> Result<()> {
        clear_in(&self.conn())
    }

    /// Sort all mail again.
    pub fn resort_topics(&self) -> Result<()> {
        self.conn().execute_batch(&bump_sql(CONTEXT_REVISION))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;
    use crate::model::*;
    use crate::store::orders::tests::{folder, mail, plain};
    use crate::store::test_util::{during_next_batch, settle, store_with_account};

    fn from(store: &Store, acct: AccountId, address: &str, minutes: i64) -> MessageId {
        mail(store, acct, ("", address), &format!("hello from {address}"), "<p>hello</p>", minutes)
    }

    fn topic_of(store: &Store, id: MessageId) -> Option<Topic> {
        store.topics_for(&[id]).unwrap().get(&id).map(|t| t.0)
    }

    fn mark_cached(store: &Store) {
        store.conn().execute("UPDATE topic_assignments SET created_at = 1", []).unwrap();
    }

    fn untouched(store: &Store) -> i64 {
        store.conn().query_row("SELECT COUNT(*) FROM topic_assignments WHERE created_at = 1", [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn set_topics_is_one_change_of_many() {
        let (store, acct) = store_with_account();
        let a = from(&store, acct, "a@x.example", 1);
        let b = from(&store, acct, "b@y.example", 2);
        let sub = from(&store, acct, "c@mail.z.example", 3);
        let evil = from(&store, acct, "d@evilz.example", 4);
        let other = from(&store, acct, "e@w.example", 5);
        settle(&store);
        mark_cached(&store);
        let changes = store
            .set_topics(vec![
                (Some(acct), TopicTarget::Sender("a@x.example".into()), Some(Topic::Travel)),
                (None, TopicTarget::Sender("b@y.example".into()), Some(Topic::Finance)),
                (Some(acct), TopicTarget::Domain("z.example".into()), Some(Topic::Social)),
                (None, TopicTarget::Message(other), Some(Topic::Events)),
            ])
            .unwrap();
        assert_eq!(changes.len(), 4);
        assert_eq!([a, b, sub, evil, other].map(|id| topic_of(&store, id)), [
            Some(Topic::Travel),
            Some(Topic::Finance),
            Some(Topic::Social),
            Some(Topic::Other),
            Some(Topic::Events)
        ]);
        assert_eq!(untouched(&store), 5, "updated in place");
        // All or none.
        let bad = store.set_topics(vec![
            (None, TopicTarget::Sender("e@w.example".into()), Some(Topic::Travel)),
            (None, TopicTarget::Domain("gmail.com".into()), Some(Topic::Travel)),
        ]);
        assert!(matches!(bad, Err(Error::Invalid(_))));
        assert!(store.topic_rules().unwrap().iter().all(|r| r.rule.pattern != "e@w.example"));
        // One Undo for all of them.
        store.undo_topics(&changes).unwrap();
        assert!(store.topic_rules().unwrap().is_empty() && store.topic_overrides(&[other]).unwrap().is_empty());
        settle(&store);
        assert!([a, b, sub, evil, other].iter().all(|id| topic_of(&store, *id) == Some(Topic::Other)));
    }

    #[test]
    fn changing_hundreds_of_senders_is_one_pass() {
        let (store, acct) = store_with_account();
        let inbox = folder(&store, acct, FolderRole::Inbox);
        let senders = 300;
        for i in 0..6000 {
            let mut m = Message {
                account_id: acct,
                folder_id: inbox,
                from: EmailAddress::new("", format!("s{}@shop{}.example", i % senders, i % senders)),
                subject: format!("note {i}"),
                received_at: Utc::now() - Duration::minutes(i),
                ..Default::default()
            };
            store.insert_message(&mut m, None).unwrap();
        }
        let version = current_version(&store.conn()).unwrap();
        store
            .conn()
            .execute(
                "INSERT INTO topic_assignments (message_id, version, topic, confidence, source, json, created_at) SELECT id, ?1, 'other', 0.5, 'fallback', '[]', 1 FROM messages",
                [&version],
            )
            .unwrap();
        let picks: Vec<_> = (0..senders).map(|i| (Some(acct), TopicTarget::Sender(format!("s{i}@shop{i}.example")), Some(Topic::Newsletters))).collect();
        let started = std::time::Instant::now();
        let changes = store.set_topics(picks).unwrap();
        let took = started.elapsed();
        assert_eq!(store.topic_counts().unwrap().iter().find(|c| c.topic == Topic::Newsletters).map(|c| c.total), Some(6000));
        // One scan per sender took seconds here; one for all, milliseconds.
        assert!(took < std::time::Duration::from_millis(1500), "{took:?}");
        let started = std::time::Instant::now();
        store.undo_topics(&changes).unwrap();
        assert!(started.elapsed() < std::time::Duration::from_millis(1500), "{:?}", started.elapsed());
        assert_eq!(store.topics_for(&[1]).unwrap().len(), 0, "sorted again");
        // A backup's corrections, restored, are one pass too.
        let rows: Vec<RuleBackupRow> = (0..senders)
            .map(|i| (if i % 2 == 0 { acct } else { 0 }, format!("s{i}@shop{i}.example"), false, "travel".to_string(), 1, 1))
            .chain([(0, "gmail.com".to_string(), true, "travel".to_string(), 1, 1)])
            .collect();
        store
            .conn()
            .execute(
                "INSERT OR REPLACE INTO topic_assignments (message_id, version, topic, confidence, source, json, created_at) SELECT id, ?1, 'other', 0.5, 'fallback', '[]', 1 FROM messages",
                [&version],
            )
            .unwrap();
        let started = std::time::Instant::now();
        store.tx(|tx| restore_rules_in(tx, &rows)).unwrap();
        assert!(started.elapsed() < std::time::Duration::from_millis(1500), "{:?}", started.elapsed());
        assert_eq!(store.topic_rules().unwrap().len(), senders as usize, "never a free mail domain");
        assert_eq!(store.topic_counts().unwrap().iter().find(|c| c.topic == Topic::Travel).map(|c| c.total), Some(6000));
    }

    #[test]
    fn your_own_mail_in_a_folder_under_sent_items_has_no_topic() {
        let (store, acct) = store_with_account();
        let id = from(&store, acct, "test@axentra.example", 1);
        let old_sent = store.create_folder(acct, Some(folder(&store, acct, FolderRole::Sent)), "Old sent").unwrap();
        let old_junk = store.create_folder(acct, Some(folder(&store, acct, FolderRole::Junk)), "Old junk").unwrap();
        store.move_messages(&[id], old_sent).unwrap();
        settle(&store);
        assert_eq!(store.topics_pending_count(10).unwrap(), 0);
        assert_eq!(store.topic_for(id).unwrap(), None);
        let rows: i64 = store.conn().query_row("SELECT COUNT(*) FROM topic_assignments", [], |r| r.get(0)).unwrap();
        assert_eq!(rows, 0, "nothing written");
        assert!(!store.folder_has_topics(old_sent).unwrap() && !store.folder_has_topics(old_junk).unwrap());
        assert!(store.folder_has_topics(folder(&store, acct, FolderRole::Inbox)).unwrap());
    }

    #[test]
    fn set_topic_sender_updates_in_place() {
        let (store, acct) = store_with_account();
        let a = [from(&store, acct, "a@x.example", 1), from(&store, acct, "A@x.example", 2)];
        let b = from(&store, acct, "b@x.example", 3);
        let sub = from(&store, acct, "c@mail.x.example", 4);
        let evil = from(&store, acct, "d@evilx.example", 5);
        settle(&store);
        mark_cached(&store);
        let change = store.set_topic(None, TopicTarget::Sender(" A@X.example ".into()), Some(Topic::Travel)).unwrap();
        assert_eq!(change, TopicChange { account: None, target: TopicTarget::Sender("a@x.example".into()), previous: None, new: Some(Topic::Travel) });
        for id in a {
            assert_eq!(store.topics_for(&[id]).unwrap()[&id], (Topic::Travel, 1.0));
        }
        assert_eq!(topic_of(&store, b), Some(Topic::Other));
        assert_eq!(untouched(&store), 5, "updated in place: no row recomputed");
        assert_eq!(store.topics_pending_count(10).unwrap(), 0, "nothing pending, nothing flickers");
        let why = store.topic_for(a[0]).unwrap().unwrap();
        assert_eq!((why.source, why.reasons), (Source::SenderRule, vec![Reason::SenderRule { pattern: "a@x.example".into() }]));

        // A domain rule: subdomains yes, look-alikes no, the exact address rule still wins.
        let domain = store.set_topic(Some(acct), TopicTarget::Domain("x.example".into()), Some(Topic::Finance)).unwrap();
        assert_eq!([topic_of(&store, a[0]), topic_of(&store, b), topic_of(&store, sub), topic_of(&store, evil)], [
            Some(Topic::Travel),
            Some(Topic::Finance),
            Some(Topic::Finance),
            Some(Topic::Other)
        ]);
        assert!(matches!(store.set_topic(None, TopicTarget::Domain("Gmail.com".into()), Some(Topic::People)), Err(Error::Invalid(_))));
        assert!(matches!(store.set_topic(None, TopicTarget::Sender("not an address".into()), Some(Topic::People)), Err(Error::Invalid(_))));

        // Changed again, then undone: the previous topic comes back.
        let again = store.set_topic(Some(acct), TopicTarget::Domain("x.example".into()), Some(Topic::Social)).unwrap();
        assert_eq!(again.previous, Some(Topic::Finance));
        store.undo_topic(&again).unwrap();
        assert_eq!(topic_of(&store, b), Some(Topic::Finance));
        // Removed (undoing the first change): its mail is sorted again.
        store.undo_topic(&domain).unwrap();
        assert_eq!((topic_of(&store, b), topic_of(&store, sub)), (None, None));
        assert_eq!(topic_of(&store, a[0]), Some(Topic::Travel), "still covered by its own rule");
        settle(&store);
        assert_eq!(topic_of(&store, b), Some(Topic::Other));
        let rules = store.topic_rules().unwrap();
        assert_eq!(rules.len(), 1);
        store.remove_topic_rule(rules[0].id).unwrap();
        assert!(store.topic_rules().unwrap().is_empty());
        assert_eq!(topic_of(&store, a[0]), None);
    }

    #[test]
    fn a_message_correction_beats_the_senders() {
        let (store, acct) = store_with_account();
        let [one, two] = [from(&store, acct, "a@x.example", 1), from(&store, acct, "a@x.example", 2)];
        settle(&store);
        let c = store.set_topic(None, TopicTarget::Message(one), Some(Topic::Events)).unwrap();
        store.set_topic(None, TopicTarget::Sender("a@x.example".into()), Some(Topic::Travel)).unwrap();
        assert_eq!((topic_of(&store, one), topic_of(&store, two)), (Some(Topic::Events), Some(Topic::Travel)));
        assert_eq!(store.topic_for(one).unwrap().unwrap().source, Source::MessageOverride);
        store.undo_topic(&c).unwrap();
        assert_eq!(topic_of(&store, one), None, "sorted again");
    }

    #[test]
    fn a_new_message_from_a_corrected_sender_gets_the_topic() {
        let (store, acct) = store_with_account();
        from(&store, acct, "a@x.example", 10);
        settle(&store);
        store.set_topic(None, TopicTarget::Sender("a@x.example".into()), Some(Topic::Travel)).unwrap();
        let new = from(&store, acct, "a@x.example", 1);
        settle(&store);
        assert_eq!(topic_of(&store, new), Some(Topic::Travel));
    }

    #[test]
    fn a_correction_made_while_a_batch_is_sorted_is_kept() {
        let (store, acct) = store_with_account();
        let ids = [from(&store, acct, "a@x.example", 1), from(&store, acct, "a@x.example", 2)];
        let checked = |store: &Store| while store.assess_pending(250).unwrap() > 0 {};
        checked(&store);
        during_next_batch(|s| {
            s.set_topic(None, TopicTarget::Sender("a@x.example".into()), Some(Topic::Travel)).unwrap();
        });
        assert_eq!(store.classify_topics_pending(250).unwrap(), 2);
        assert_eq!((topic_of(&store, ids[0]), topic_of(&store, ids[1])), (None, None), "sorted before the correction: not written");
        settle(&store);
        assert_eq!((topic_of(&store, ids[0]), topic_of(&store, ids[1])), (Some(Topic::Travel), Some(Topic::Travel)));
        // A message's own correction, and switching topics off, the same way.
        let new = from(&store, acct, "b@x.example", 0);
        checked(&store);
        during_next_batch(move |s| {
            s.set_topic(None, TopicTarget::Message(new), Some(Topic::Events)).unwrap();
        });
        store.classify_topics_pending(250).unwrap();
        settle(&store);
        assert_eq!(topic_of(&store, new), Some(Topic::Events));
        let late = from(&store, acct, "c@x.example", 0);
        checked(&store);
        during_next_batch(|s| {
            let mut st = s.settings();
            st.topics_view = false;
            s.save_settings(&st).unwrap();
            s.clear_topics().unwrap();
        });
        store.classify_topics_pending(250).unwrap();
        assert_eq!(store.conn().query_row("SELECT COUNT(*) FROM topic_assignments", [], |r| r.get::<_, i64>(0)).unwrap(), 0, "{late}");
    }

    #[test]
    fn phishing_never_shows_a_finance_topic_not_even_before_its_junk_check() {
        let (store, acct) = store_with_account();
        // A bank statement whose link shows the bank and goes somewhere else.
        let phish = |minutes| {
            mail(
                &store,
                acct,
                ("Harbor Bank", "alerts@harborbank.example"),
                "Your monthly statement is ready",
                r#"<p>Your October bank statement is ready.</p><p><a href="https://login.evil.example/">https://harborbank.example/statements</a></p>"#,
                minutes,
            )
        };
        let (read, background) = (phish(1), phish(2));
        assert_eq!((store.topics_pending_count(10).unwrap(), store.classify_topics_pending(250).unwrap()), (0, 0), "sorting waits for the junk check");
        // The reading pane checks first, then sorts.
        let a = store.topic_for(read).unwrap().unwrap();
        assert_eq!((a.topic, a.source), (Topic::Other, Source::Suspicious), "{:?}", a.reasons);
        settle(&store);
        assert_eq!(topic_of(&store, background), Some(Topic::Other));
        assert_eq!(store.topic_counts().unwrap()[Topic::Finance.index()].total, 0);
    }

    #[test]
    fn fee_lures_stay_out_of_orders_with_order_tracking_off() {
        let (store, acct) = store_with_account();
        let mut st = store.settings();
        st.order_tracking = false;
        store.save_settings(&st).unwrap();
        let lure = |minutes| {
            mail(
                &store,
                acct,
                ("Parcel Desk", "tracking@parcel-desk.example"),
                "Your order could not be delivered - pay $1.99 redelivery fee",
                "<p>Your order could not be delivered. Pay a small fee to reschedule: <a href=https://parcel-desk.example/pay>Pay now</a></p>",
                minutes,
            )
        };
        // One too old for order detection to ever look at, with it on.
        let (new, old) = (lure(60), lure(401 * 1440));
        settle(&store);
        for id in [new, old] {
            let a = store.topic_for(id).unwrap().unwrap();
            assert_eq!((a.topic, a.source), (Topic::Other, Source::Suspicious), "{:?}", a.reasons);
        }
        st.order_tracking = true;
        store.save_settings(&st).unwrap();
        settle(&store);
        assert!(store.extraction_for(old).unwrap().is_none(), "past the horizon");
        assert_eq!(store.extraction_for(new).unwrap().unwrap().suppressed, Some(crate::orders::SuppressReason::DeliveryFeeLure));
        for id in [new, old] {
            assert_eq!(store.topic_for(id).unwrap().unwrap().source, Source::Suspicious);
        }
    }

    #[test]
    fn mail_that_turns_out_to_be_phishing_leaves_any_topic() {
        let (store, acct) = store_with_account();
        let mut m = Message {
            account_id: acct,
            folder_id: folder(&store, acct, FolderRole::Inbox),
            from: EmailAddress::new("Daniel Kim", "daniel.kim.ceo@gmail.com"),
            subject: "Quick sync".into(),
            meeting_kind: Some(MeetingMessageKind::Request),
            received_at: Utc::now(),
            ..Default::default()
        };
        let id = store.insert_message(&mut m, Some(&MessageBody { text: "Can we meet at 3?".into(), ..Default::default() })).unwrap();
        settle(&store);
        let a = store.topic_for(id).unwrap().unwrap();
        assert_eq!((a.topic, a.source), (Topic::Events, Source::Meeting));
        // The real Daniel Kim is a contact: the meeting request uses his name
        // from another address, and the check says so.
        let mut c = Contact {
            account_id: acct,
            given_name: "Daniel".into(),
            surname: "Kim".into(),
            emails: vec![LabeledValue { label: "Work".into(), value: "daniel.kim@contoso.example".into() }],
            ..Default::default()
        };
        store.insert_contact(&mut c).unwrap();
        settle(&store);
        assert_eq!(store.assessment(id).unwrap().unwrap().analysis.severity(), Some(Severity::Danger));
        let a = store.topic_for(id).unwrap().unwrap();
        assert_eq!((a.topic, a.source), (Topic::Other, Source::Suspicious), "{:?}", a.reasons);
        assert_eq!(topic_of(&store, id), Some(Topic::Other));
        // Gone again: back to Events.
        store.delete_contact(c.id).unwrap();
        settle(&store);
        assert_eq!(store.topic_for(id).unwrap().unwrap().topic, Topic::Events);
    }

    #[test]
    fn contact_added_reclassifies_only_that_sender() {
        let (store, acct) = store_with_account();
        let friend = [from(&store, acct, "Friend@pals.example", 1), from(&store, acct, "friend@pals.example", 2)];
        for i in 0..3 {
            plain(&store, acct, 10 + i);
        }
        settle(&store);
        mark_cached(&store);
        let mut c = Contact {
            account_id: acct,
            given_name: "Fran".into(),
            emails: vec![LabeledValue { label: "Home".into(), value: "friend@pals.example".into() }],
            ..Default::default()
        };
        store.insert_contact(&mut c).unwrap();
        assert_eq!(store.classify_topics_pending(250).unwrap(), 2, "only the contact's mail");
        assert_eq!(untouched(&store), 3);
        assert!(friend.iter().all(|id| topic_of(&store, *id).is_some()));
        // Changed and removed: the old and new addresses' mail.
        mark_cached(&store);
        c.emails[0].value = "fran@elsewhere.example".into();
        store.update_contact(&c).unwrap();
        assert_eq!(store.classify_topics_pending(250).unwrap(), 2);
        store.delete_contact(c.id).unwrap();
        assert_eq!(store.classify_topics_pending(250).unwrap(), 0, "nobody's mail is from the removed address");
        assert_eq!(untouched(&store), 3);
    }

    #[test]
    fn topic_counts_matches_messages_query() {
        let (store, acct) = store_with_account();
        let inbox: Vec<MessageId> = (0..6).map(|i| plain(&store, acct, i)).collect();
        let archived = plain(&store, acct, 7);
        store.move_messages(&[archived], folder(&store, acct, FolderRole::Archive)).unwrap();
        let junk = plain(&store, acct, 8);
        store.move_messages(&[junk], folder(&store, acct, FolderRole::Junk)).unwrap();
        let deleted = plain(&store, acct, 9);
        store.move_messages(&[deleted], folder(&store, acct, FolderRole::Deleted)).unwrap();
        let sent = plain(&store, acct, 10);
        store.move_messages(&[sent], folder(&store, acct, FolderRole::Sent)).unwrap();
        settle(&store);
        for (id, t) in [(inbox[0], Topic::Travel), (inbox[1], Topic::Travel), (archived, Topic::Travel), (inbox[2], Topic::Finance), (junk, Topic::Travel)] {
            store.set_topic(None, TopicTarget::Message(id), Some(t)).unwrap();
        }
        store.set_read(&[inbox[0]], true).unwrap();
        let counts = store.topic_counts().unwrap();
        assert_eq!(counts.iter().map(|c| c.topic).collect::<Vec<_>>(), Topic::ALL, "every topic, in order");
        for c in &counts {
            let q = MessageQuery { topic: Some(TopicScope::One(c.topic)), ..Default::default() };
            let msgs = store.messages(&q).unwrap();
            assert_eq!(c.total as usize, msgs.len(), "{:?}", c.topic);
            assert_eq!(c.unread as usize, msgs.iter().filter(|m| !m.is_read).count(), "{:?}", c.topic);
        }
        assert_eq!((counts[Topic::Travel.index()].total, counts[Topic::Travel.index()].unread), (3, 2));
        let all: Vec<MessageId> = store.messages(&MessageQuery { topic: Some(TopicScope::Any), ..Default::default() }).unwrap().iter().map(|m| m.id).collect();
        assert_eq!(all.len(), 7, "across folders");
        assert!(!all.contains(&junk) && !all.contains(&deleted) && !all.contains(&sent));
        assert_eq!(counts.iter().map(|c| c.total).sum::<u32>(), 7);
    }

    #[test]
    fn topic_counts_leave_out_snoozed_mail() {
        let (store, acct) = store_with_account();
        let id = plain(&store, acct, 1);
        settle(&store);
        store.set_topic(None, TopicTarget::Message(id), Some(Topic::Travel)).unwrap();
        store.snooze(&[id], Some(Utc::now() + Duration::hours(1))).unwrap();
        let q = MessageQuery { topic: Some(TopicScope::One(Topic::Travel)), ..Default::default() };
        assert_eq!(store.messages(&q).unwrap().len(), 0);
        assert_eq!(store.topic_counts().unwrap()[Topic::Travel.index()], TopicCount { topic: Topic::Travel, total: 0, unread: 0 });
    }

    #[test]
    fn messages_query_topic_keeps_filters_and_snooze() {
        let (store, acct) = store_with_account();
        let ids: Vec<MessageId> = (0..4).map(|i| plain(&store, acct, i)).collect();
        settle(&store);
        for id in &ids[..3] {
            store.set_topic(None, TopicTarget::Message(*id), Some(Topic::Travel)).unwrap();
        }
        store.set_read(&[ids[0]], true).unwrap();
        let q = |filter, sort, limit| MessageQuery { topic: Some(TopicScope::One(Topic::Travel)), filter, sort, limit, ..Default::default() };
        let got = |q: MessageQuery| store.messages(&q).unwrap().iter().map(|m| m.id).collect::<Vec<_>>();
        assert_eq!(got(q(MessageFilter::All, MessageSort::DateNewest, None)), ids[..3]);
        assert_eq!(got(q(MessageFilter::Unread, MessageSort::DateNewest, None)), ids[1..3]);
        assert_eq!(got(q(MessageFilter::All, MessageSort::DateOldest, Some(1))), [ids[2]]);
        // Snoozed: hidden until due, even though it sits in the Snoozed folder.
        store.snooze(&[ids[1]], Some(Utc::now() + Duration::hours(3))).unwrap();
        assert_eq!(got(q(MessageFilter::All, MessageSort::DateNewest, None)), [ids[0], ids[2]]);
        // A search inside a topic.
        let mut s = q(MessageFilter::All, MessageSort::DateNewest, None);
        s.search = Some("note 2".into());
        assert_eq!(got(s), [ids[2]]);
    }

    #[test]
    fn clear_topics_keeps_rules() {
        let (store, acct) = store_with_account();
        let id = from(&store, acct, "a@x.example", 1);
        settle(&store);
        store.set_topic(None, TopicTarget::Sender("a@x.example".into()), Some(Topic::Travel)).unwrap();
        store.set_topic(None, TopicTarget::Message(id), Some(Topic::Finance)).unwrap();
        store.clear_topics().unwrap();
        assert_eq!(topic_of(&store, id), None);
        assert_eq!(store.topics_pending_count(10).unwrap(), 1);
        assert_eq!(store.topic_rules().unwrap().len(), 1);
        // Sort again: the version moves on, every row is redone.
        settle(&store);
        store.resort_topics().unwrap();
        assert_eq!(store.topics_pending_count(10).unwrap(), 1);
        assert!(store.topics_for(&[id]).unwrap().is_empty(), "an old version isn't current");
        settle(&store);
        // Forget all corrections.
        store.clear_topic_rules().unwrap();
        assert!(store.topic_rules().unwrap().is_empty());
        assert_eq!(store.topics_pending_count(10).unwrap(), 1);
    }

    #[test]
    fn topic_for_sorts_on_demand_but_not_your_own_mail_or_when_off() {
        let (store, acct) = store_with_account();
        let id = plain(&store, acct, 1);
        let a = store.topic_for(id).unwrap().unwrap();
        assert_eq!(store.topics_for(&[id]).unwrap()[&id].0, a.topic, "cached");
        assert_eq!(store.topics_pending_count(10).unwrap(), 0);
        let sent = plain(&store, acct, 2);
        store.move_messages(&[sent], folder(&store, acct, FolderRole::Sent)).unwrap();
        assert!(store.topic_for(sent).unwrap().is_none());
        let mut s = store.settings();
        s.topics_view = false;
        store.save_settings(&s).unwrap();
        assert!(store.topic_for(id).unwrap().is_none());
        assert_eq!((store.classify_topics_pending(250).unwrap(), store.topics_pending_count(10).unwrap()), (0, 0));
    }

    #[test]
    fn rules_are_decided_most_specific_first() {
        let rule = |account_key, pattern: &str, is_domain, topic| TopicRule { account_key, pattern: pattern.into(), is_domain, topic };
        let rules = [
            rule(0, "x.example", true, Topic::Finance),
            rule(0, "mail.x.example", true, Topic::Social),
            rule(2, "x.example", true, Topic::Travel),
            rule(0, "a@x.example", false, Topic::People),
            rule(2, "a@x.example", false, Topic::Events),
        ];
        let topic = |account, address| deciding_rule(&rules, account, address).map(|r| r.topic);
        assert_eq!(topic(1, "a@x.example"), Some(Topic::People));
        assert_eq!(topic(2, "a@x.example"), Some(Topic::Events), "the account's own rule first");
        assert_eq!(topic(1, "b@x.example"), Some(Topic::Finance));
        assert_eq!(topic(2, "b@x.example"), Some(Topic::Travel));
        assert_eq!(topic(2, "b@mail.x.example"), Some(Topic::Social), "the most specific domain");
        assert_eq!(topic(1, "b@evilx.example"), None);
    }
}
