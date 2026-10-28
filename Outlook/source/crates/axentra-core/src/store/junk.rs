//! Junk email: sender rules (per account, or all accounts for lists carried
//! over from 0.1.4), picture permissions (separate from the rules), confirmed
//! labels with their history, the cached checks of each message, the learned
//! models, and the audit trail of rule-routed mail.
//! The pure logic lives in [`crate::junk`].

use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};

use super::Store;
use super::mail::MoveRecord;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::junk::analyze::{self, Severity};
use crate::junk::learn::{Features, Model};
use crate::junk::{self, Advice, Assessment, Label, LabelEvent, RuleAction, RuleChange, RuleSource, SenderRule};
use crate::model::*;
use crate::{Error, Result};

pub(crate) const SCHEMA_EXTRA: &str = r#"
CREATE TABLE IF NOT EXISTS junk_sender_rules (
    id INTEGER PRIMARY KEY,
    account_key INTEGER NOT NULL,
    pattern TEXT NOT NULL,
    domain INTEGER NOT NULL,
    action TEXT NOT NULL,
    name TEXT NOT NULL DEFAULT '',
    source TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    exact_domain INTEGER NOT NULL DEFAULT 0,
    effective_at INTEGER NOT NULL DEFAULT 0,
    UNIQUE (account_key, pattern));
CREATE TABLE IF NOT EXISTS junk_image_senders (
    address TEXT PRIMARY KEY, added_at INTEGER NOT NULL, subdomains INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS junk_label_events (
    id INTEGER PRIMARY KEY,
    account_id INTEGER NOT NULL,
    message_id INTEGER NOT NULL,
    fingerprint TEXT NOT NULL,
    label TEXT NOT NULL,
    previous INTEGER,
    created_at INTEGER NOT NULL,
    undone INTEGER NOT NULL DEFAULT 0,
    subject TEXT NOT NULL DEFAULT '',
    sender TEXT NOT NULL DEFAULT '');
CREATE TABLE IF NOT EXISTS junk_labels (
    account_id INTEGER NOT NULL,
    fingerprint TEXT NOT NULL,
    message_id INTEGER NOT NULL,
    label TEXT NOT NULL,
    event_id INTEGER NOT NULL,
    features TEXT,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (account_id, fingerprint));
CREATE TABLE IF NOT EXISTS junk_models (
    digest TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    parent TEXT,
    status TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    gate TEXT NOT NULL DEFAULT '',
    bytes BLOB NOT NULL);
CREATE TABLE IF NOT EXISTS junk_routing (
    message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    rule_id INTEGER, pattern TEXT NOT NULL, at INTEGER NOT NULL);
"#;

/// The cached checks of each message: the warning signs (`json`, an
/// [`analyze::Analysis`]) and features, valid for `version` (what was
/// checked and against which contacts / accounts), plus the score of the
/// model `model` ('' for none), which is redone on its own when only the
/// model changes. Triggers drop a row when its message's content changes.
const ASSESSMENTS_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS junk_assessments (
    message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    version TEXT NOT NULL,
    json TEXT NOT NULL,
    features TEXT NOT NULL,
    model TEXT NOT NULL DEFAULT '',
    score REAL,
    created_at INTEGER NOT NULL);
"#;

const LEGACY_MIGRATED: &str = "junk.legacy_migrated";
/// Schema / repair level of the junk tables (see [`upgrade_schema_in`]).
const SCHEMA_LEVEL: &str = "junk.schema";
/// Bumped on every confirmed-label change; the learner rebuilds from it, and
/// training started before a change is thrown away.
pub(crate) const LABEL_REVISION: &str = "junk.label_revision";
/// Bumped (by triggers) when contacts, the addresses you write to or your
/// accounts change: the warning signs depend on them.
const CONTEXT_REVISION: &str = "junk.context_rev";

fn account_key(account: Option<AccountId>) -> i64 {
    account.unwrap_or(0)
}

/// SQL that bumps a kv counter (usable inside a trigger).
fn bump_sql(key: &str) -> String {
    format!(
        "INSERT OR IGNORE INTO kv (key, value) VALUES ('{key}', '0');
         UPDATE kv SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT) WHERE key = '{key}';"
    )
}

/// Triggers that keep cached checks honest: a message's own content
/// (sender, subject, reply-to, authentication header, body, attachments)
/// drops its cached check; contacts, addresses written to and accounts bump
/// the context revision, which every cached check is keyed on.
fn triggers_sql() -> String {
    let drop = |name: &str, when: &str, id: &str| {
        format!("CREATE TRIGGER IF NOT EXISTS {name} {when} BEGIN DELETE FROM junk_assessments WHERE message_id = {id}; END;\n")
    };
    let bump = |name: &str, when: &str| format!("CREATE TRIGGER IF NOT EXISTS {name} {when} BEGIN {} END;\n", bump_sql(CONTEXT_REVISION));
    let changed = |fields: &[&str]| {
        fields.iter().map(|f| format!("json_extract(OLD.json, '$.{f}') IS NOT json_extract(NEW.json, '$.{f}')")).collect::<Vec<_>>().join(" OR ")
    };
    let mut s = String::new();
    s += &drop(
        "junk_recheck_message",
        &format!("AFTER UPDATE OF json ON messages WHEN {}", changed(&["from", "subject", "reply_to", "auth_results", "account_id"])),
        "NEW.id",
    );
    s += &drop("junk_recheck_body_insert", "AFTER INSERT ON message_bodies", "NEW.message_id");
    s += &drop("junk_recheck_body_update", "AFTER UPDATE ON message_bodies", "NEW.message_id");
    s += &drop("junk_recheck_body_delete", "AFTER DELETE ON message_bodies", "OLD.message_id");
    s += &drop("junk_recheck_attachment_insert", "AFTER INSERT ON attachments", "NEW.message_id");
    s += &drop("junk_recheck_attachment_update", "AFTER UPDATE OF json ON attachments", "NEW.message_id");
    s += &drop("junk_recheck_attachment_delete", "AFTER DELETE ON attachments", "OLD.message_id");
    s += &bump("junk_context_contact_insert", "AFTER INSERT ON contacts");
    s += &bump("junk_context_contact_update", "AFTER UPDATE ON contacts WHEN OLD.json IS NOT NEW.json OR OLD.display_name IS NOT NEW.display_name");
    s += &bump("junk_context_contact_delete", "AFTER DELETE ON contacts");
    s += &bump("junk_context_recent_insert", "AFTER INSERT ON recent_addresses");
    s += &bump("junk_context_recent_update", "AFTER UPDATE OF address ON recent_addresses");
    s += &bump("junk_context_recent_delete", "AFTER DELETE ON recent_addresses");
    s += &bump("junk_context_account_insert", "AFTER INSERT ON accounts");
    s += &bump("junk_context_account_update", &format!("AFTER UPDATE OF json ON accounts WHEN {}", changed(&["email", "config"])));
    s += &bump("junk_context_account_delete", "AFTER DELETE ON accounts");
    s
}

fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let names = stmt.query_map([], |r| r.get::<_, String>(1))?;
    for n in names {
        if n? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Bring junk tables written by earlier 0.1.5 builds up to date, and repair
/// what they got wrong, once:
/// - rules carried over from 0.1.4 matched a domain exactly, not its
///   subdomains, and never moved synced mail: they keep that meaning, and
///   only mail arriving after this repair is filed by them;
/// - picture permissions that came from the old safe-sender list (that list
///   is still there, so their origin is known) cover the exact domain only.
///   Ones you added in 0.1.5 keep covering subdomains, as they said;
/// - cached checks are rebuilt in the new layout.
pub(crate) fn upgrade_schema_in(conn: &Connection) -> Result<()> {
    if !has_column(conn, "junk_sender_rules", "exact_domain")? {
        conn.execute("ALTER TABLE junk_sender_rules ADD COLUMN exact_domain INTEGER NOT NULL DEFAULT 0", [])?;
    }
    if !has_column(conn, "junk_sender_rules", "effective_at")? {
        conn.execute("ALTER TABLE junk_sender_rules ADD COLUMN effective_at INTEGER NOT NULL DEFAULT 0", [])?;
    }
    if !has_column(conn, "junk_image_senders", "subdomains")? {
        conn.execute("ALTER TABLE junk_image_senders ADD COLUMN subdomains INTEGER NOT NULL DEFAULT 1", [])?;
        conn.execute(
            "UPDATE junk_image_senders SET subdomains = 0
             WHERE address LIKE '%@%' OR address IN (SELECT lower(trim(address)) FROM mail_safe_senders)",
            [],
        )?;
    }
    let assessments: Option<String> =
        conn.query_row("SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'junk_assessments'", [], |r| r.get(0)).optional()?;
    if assessments.is_some() && !has_column(conn, "junk_assessments", "features")? {
        conn.execute("DROP TABLE junk_assessments", [])?;
    }
    conn.execute_batch(ASSESSMENTS_DDL)?;
    let level: i64 =
        conn.query_row("SELECT value FROM kv WHERE key = ?1", [SCHEMA_LEVEL], |r| r.get::<_, String>(0)).optional()?.and_then(|v| v.parse().ok()).unwrap_or(0);
    if level < 2 {
        let now = Utc::now().timestamp();
        conn.execute("UPDATE junk_sender_rules SET exact_domain = 1 WHERE source = 'legacy' AND domain = 1", [])?;
        conn.execute("UPDATE junk_sender_rules SET effective_at = ?1 WHERE source = 'legacy' AND effective_at = 0", [now])?;
        conn.execute("INSERT OR REPLACE INTO kv (key, value) VALUES (?1, '2')", [SCHEMA_LEVEL])?;
    }
    conn.execute_batch(&triggers_sql())?;
    Ok(())
}

fn rule_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<SenderRule> {
    let key: i64 = r.get(1)?;
    Ok(SenderRule {
        id: r.get(0)?,
        account_id: (key != 0).then_some(key),
        pattern: r.get(2)?,
        domain: r.get(3)?,
        action: RuleAction::parse(&r.get::<_, String>(4)?),
        name: r.get(5)?,
        source: RuleSource::parse(&r.get::<_, String>(6)?),
        created_at: r.get(7)?,
        updated_at: r.get(8)?,
        exact_domain: r.get(9)?,
        effective_at: r.get(10)?,
    })
}

const RULE_COLUMNS: &str = "id, account_key, pattern, domain, action, name, source, created_at, updated_at, exact_domain, effective_at";

pub(crate) fn rules_in(conn: &Connection) -> Result<Vec<SenderRule>> {
    let mut stmt = conn.prepare_cached(&format!("SELECT {RULE_COLUMNS} FROM junk_sender_rules ORDER BY account_key, pattern"))?;
    let rows = stmt.query_map([], rule_from_row)?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

fn rule_by_key_in(conn: &Connection, account: Option<AccountId>, pattern: &str) -> Result<Option<SenderRule>> {
    Ok(conn
        .query_row(
            &format!("SELECT {RULE_COLUMNS} FROM junk_sender_rules WHERE account_key = ?1 AND pattern = ?2"),
            params![account_key(account), pattern],
            rule_from_row,
        )
        .optional()?)
}

fn rule_by_id_in(conn: &Connection, id: i64) -> Result<Option<SenderRule>> {
    Ok(conn.query_row(&format!("SELECT {RULE_COLUMNS} FROM junk_sender_rules WHERE id = ?1"), [id], rule_from_row).optional()?)
}

/// Write a rule (insert, or replace the one with the same account and
/// pattern). The record keeps its first `created_at`; everything else,
/// including when the current action took effect, is the given rule's.
fn put_rule_in(conn: &Connection, r: &SenderRule) -> Result<i64> {
    conn.execute(
        "INSERT INTO junk_sender_rules (id, account_key, pattern, domain, action, name, source, created_at, updated_at, exact_domain, effective_at)
         VALUES (NULLIF(?1, 0), ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT(account_key, pattern) DO UPDATE SET
            domain = excluded.domain, action = excluded.action, name = excluded.name,
            source = excluded.source, updated_at = excluded.updated_at,
            exact_domain = excluded.exact_domain, effective_at = excluded.effective_at",
        params![
            r.id,
            account_key(r.account_id),
            r.pattern,
            r.domain,
            r.action().as_str(),
            r.name,
            r.source.as_str(),
            r.created_at,
            r.updated_at,
            r.exact_domain,
            r.effective_at
        ],
    )?;
    Ok(conn.query_row(
        "SELECT id FROM junk_sender_rules WHERE account_key = ?1 AND pattern = ?2",
        params![account_key(r.account_id), r.pattern],
        |row| row.get(0),
    )?)
}

/// Add or change the rule for `pattern` in `account`; the returned change
/// undoes it. A changed action takes effect now (an Allow from years ago
/// turned into a Block doesn't reach back to mail from then); repeating the
/// same action keeps when it took effect. A domain rule carried over as
/// "this domain only" stays that way: nothing is widened without asking.
fn set_rule_in(conn: &Connection, account: Option<AccountId>, pattern: &str, name: &str, action: RuleAction, now: i64) -> Result<RuleChange> {
    let Some((pattern, domain)) = junk::normalize_pattern(pattern) else {
        return Err(Error::Invalid(format!("\u{201c}{}\u{201d} isn't an email address or a domain", pattern.trim())));
    };
    let previous = rule_by_key_in(conn, account, &pattern)?;
    let same = previous.as_ref().filter(|p| p.action() == action);
    let rule = SenderRule {
        id: 0,
        account_id: account,
        pattern,
        domain,
        action: Some(action),
        name: name.trim().to_string(),
        source: RuleSource::User,
        created_at: previous.as_ref().map(|p| p.created_at).unwrap_or(now),
        updated_at: now,
        exact_domain: previous.as_ref().is_some_and(|p| p.exact_domain),
        effective_at: same.map(|p| p.effective_since()).unwrap_or(now),
    };
    let rule_id = put_rule_in(conn, &rule)?;
    Ok(RuleChange { rule_id, previous, applied_at: now })
}

/// Undo [`set_rule_in`], unless the rule was changed again since (then the
/// later choice is the user's and stays).
fn undo_rule_change_in(conn: &Connection, change: &RuleChange) -> Result<()> {
    let current = rule_by_id_in(conn, change.rule_id)?;
    if change.applied_at != 0 && current.as_ref().is_none_or(|c| c.updated_at != change.applied_at) {
        return Ok(());
    }
    conn.execute("DELETE FROM junk_sender_rules WHERE id = ?1", [change.rule_id])?;
    if let Some(p) = &change.previous {
        put_rule_in(conn, p)?;
    }
    Ok(())
}

/// Bring the blocked / safe sender lists of 0.1.4 and earlier over as
/// all-accounts rules (marked as carried over), without widening trust:
/// a safe sender becomes an allow rule plus a picture permission, exactly
/// what it meant before. Runs once; the old tables are left as they were.
pub(crate) fn migrate_legacy_in(conn: &Connection) -> Result<()> {
    let done: Option<String> = conn.query_row("SELECT value FROM kv WHERE key = ?1", [LEGACY_MIGRATED], |r| r.get(0)).optional()?;
    if done.is_some() {
        return Ok(());
    }
    let blocked: Vec<(String, String, i64)> = {
        let mut stmt = conn.prepare("SELECT address, name, added_at FROM mail_blocked_senders")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect::<std::result::Result<_, _>>()?
    };
    let safe: Vec<(String, i64)> = {
        let mut stmt = conn.prepare("SELECT address, added_at FROM mail_safe_senders")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<std::result::Result<_, _>>()?
    };
    import_legacy_in(conn, &blocked, &safe)?;
    conn.execute("INSERT OR REPLACE INTO kv (key, value) VALUES (?1, '1')", [LEGACY_MIGRATED])?;
    Ok(())
}

/// Legacy lists (from the old tables or an old backup) as all-accounts
/// rules. An existing rule for the same pattern is kept. The old lists
/// matched a domain exactly (not its subdomains) and never moved mail on a
/// server, so domains stay exact and blocking takes effect now: mail that
/// arrived before isn't swept into Junk Email by a first sync.
pub(crate) fn import_legacy_in(conn: &Connection, blocked: &[(String, String, i64)], safe: &[(String, i64)]) -> Result<()> {
    let now = Utc::now().timestamp();
    let add = |pattern: &str, name: &str, at: i64, action: RuleAction| -> Result<()> {
        let Some((pattern, domain)) = junk::normalize_pattern(pattern) else { return Ok(()) };
        conn.execute(
            "INSERT OR IGNORE INTO junk_sender_rules (account_key, pattern, domain, action, name, source, created_at, updated_at, exact_domain, effective_at)
             VALUES (0, ?1, ?2, ?3, ?4, 'legacy', ?5, ?5, ?2, ?6)",
            params![pattern, domain, action.as_str(), name, at, now],
        )?;
        Ok(())
    };
    for (address, name, at) in blocked {
        add(address, name, *at, RuleAction::Block)?;
    }
    for (address, at) in safe {
        add(address, "", *at, RuleAction::Allow)?;
        if let Some((p, _)) = junk::normalize_pattern(address) {
            conn.execute("INSERT OR IGNORE INTO junk_image_senders (address, added_at, subdomains) VALUES (?1, ?2, 0)", params![p, at])?;
        }
    }
    Ok(())
}

/// A backed-up rule, unless the same account and pattern already has one.
/// Carried-over rules keep their exact-domain meaning and file only mail
/// that arrives from now on, as at migration.
pub(crate) fn restore_rule_in(conn: &Connection, r: &SenderRule) -> Result<()> {
    if rule_by_key_in(conn, r.account_id, &r.pattern)?.is_none() && junk::normalize_pattern(&r.pattern).is_some() {
        let mut r = r.clone();
        if r.source == RuleSource::Legacy {
            r.exact_domain |= r.domain;
            if r.effective_at == 0 {
                r.effective_at = Utc::now().timestamp();
            }
        }
        put_rule_in(conn, &r)?;
    }
    Ok(())
}

/// A backed-up picture permission. `subdomains` is only ever what the
/// backup says (old backups don't say: exact).
pub(crate) fn restore_image_sender_in(conn: &Connection, address: &str, added_at: i64, subdomains: bool) -> Result<()> {
    let Some((p, domain)) = junk::normalize_pattern(address) else { return Ok(()) };
    conn.execute(
        "INSERT OR IGNORE INTO junk_image_senders (address, added_at, subdomains) VALUES (?1, ?2, ?3)",
        params![p, added_at, domain && subdomains],
    )?;
    Ok(())
}

/// The rule that decides for this message's sender, if any.
pub(crate) fn decide_in(conn: &Connection, account: AccountId, address: &str) -> Result<Option<SenderRule>> {
    let rules = rules_in(conn)?;
    Ok(junk::decide(&rules, account, address).cloned())
}

pub(crate) fn record_routing_in(conn: &Connection, message_id: MessageId, rule: &SenderRule) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO junk_routing (message_id, rule_id, pattern, at) VALUES (?1, ?2, ?3, ?4)",
        params![message_id, rule.id, rule.pattern, Utc::now().timestamp()],
    )?;
    Ok(())
}

/// Everything junk-related that came from one account: its rules, reports
/// and their history, and every learned model (active, kept or rejected)
/// that was trained on any of them. Its messages (and their cached checks)
/// go with the account itself.
pub(crate) fn remove_account_in(conn: &Connection, account: AccountId) -> Result<()> {
    conn.execute("DELETE FROM junk_sender_rules WHERE account_key = ?1", [account])?;
    conn.execute("DELETE FROM junk_label_events WHERE account_id = ?1", [account])?;
    conn.execute("DELETE FROM junk_labels WHERE account_id = ?1", [account])?;
    invalidate_models_in(conn, Withdrawn::Delete)?;
    bump_revision_in(conn)
}

fn bump_revision_in(conn: &Connection) -> Result<()> {
    conn.execute_batch(&bump_sql(LABEL_REVISION))?;
    Ok(())
}

fn kv_in(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn.query_row("SELECT value FROM kv WHERE key = ?1", [key], |r| r.get(0)).optional()?)
}

fn kv_set_in(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute("INSERT INTO kv (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value", [key, value])?;
    Ok(())
}

fn revision_in(conn: &Connection) -> Result<i64> {
    Ok(kv_in(conn, LABEL_REVISION)?.and_then(|v| v.parse().ok()).unwrap_or(0))
}

fn event_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<LabelEvent> {
    Ok(LabelEvent {
        id: r.get(0)?,
        account_id: r.get(1)?,
        message_id: r.get(2)?,
        fingerprint: r.get(3)?,
        label: Label::parse(&r.get::<_, String>(4)?).unwrap_or(Label::Spam),
        previous: r.get(5)?,
        created_at: r.get(6)?,
        undone: r.get(7)?,
        subject: r.get(8)?,
        sender: r.get(9)?,
    })
}

const EVENT_COLUMNS: &str = "id, account_id, message_id, fingerprint, label, previous, created_at, undone, subject, sender";

/// Domains the user deals with: their accounts, contacts and the people they
/// wrote to. Look-alikes of these are flagged.
fn familiar_domains_in(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare_cached(
        "SELECT lower(substr(v, instr(v, '@') + 1)) FROM (
            SELECT json_extract(e.value, '$.value') AS v FROM contacts c, json_each(c.json, '$.emails') e
            UNION SELECT address FROM recent_addresses
            UNION SELECT json_extract(json, '$.email') FROM accounts)
         WHERE v LIKE '%@%.%'",
    )?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    let mut out: Vec<String> = rows.collect::<std::result::Result<_, _>>()?;
    out.sort();
    out.dedup();
    Ok(out)
}

/// Contacts by lower-cased display name, with their addresses.
fn known_people_in(conn: &Connection) -> Result<Vec<(String, Vec<String>)>> {
    let mut stmt = conn.prepare_cached(
        "SELECT lower(trim(c.display_name)), lower(json_extract(e.value, '$.value'))
         FROM contacts c, json_each(c.json, '$.emails') e WHERE trim(c.display_name) LIKE '% %'",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    let mut by_name: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
    for row in rows {
        let (n, a) = row?;
        by_name.entry(n).or_default().push(a);
    }
    Ok(by_name.into_iter().collect())
}

/// authserv-ids believed for an account: its IMAP provider's, for the few
/// providers known to clean up forged headers. Microsoft (Graph) and local
/// accounts get no headers, so none.
fn trusted_reporters_in(conn: &Connection, account: AccountId) -> Result<Vec<String>> {
    let host: Option<String> = conn
        .query_row("SELECT json_extract(json, '$.config.imap.host') FROM accounts WHERE id = ?1", [account], |r| r.get(0))
        .optional()?
        .flatten();
    Ok(host.map(|h| analyze::trusted_reporters(&h)).unwrap_or_default())
}

/// Advice from a cached assessment and what the user has said since.
/// Security signs always show; a junk suggestion needs a model score, the
/// Inbox, no label from the user and no allow rule.
pub(crate) fn advise(a: &Assessment, threshold: f32, label: Option<Label>, allowed: bool, in_inbox: bool) -> Advice {
    match a.analysis.severity() {
        Some(Severity::Danger) => Advice::Phishing,
        _ if label.is_none() && !allowed && in_inbox && a.score.is_some_and(|s| s >= threshold) => Advice::LooksLikeJunk,
        Some(Severity::Caution) => Advice::Caution,
        Some(Severity::Info) | None => Advice::None,
    }
}

/// What every check in one pass shares: the cache key and the context.
struct CheckContext {
    version: String,
    familiar: Vec<String>,
    people: Vec<(String, Vec<String>)>,
    model: Option<Arc<Model>>,
}

/// One cached row.
struct CachedCheck {
    version: String,
    analysis: analyze::Analysis,
    features: Option<Features>,
    model: String,
    score: Option<f32>,
}

/// Warning signs of one message. A check that fails (it shouldn't: inputs
/// are capped and scanned defensively) is recorded as incomplete, never as
/// a clean result, and the next message is still checked.
fn analyze_guarded(input: &analyze::Input<'_>) -> analyze::Analysis {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| analyze::analyze(input)))
        .unwrap_or_else(|_| analyze::Analysis { version: analyze::ANALYSIS_VERSION, incomplete: true, ..Default::default() })
}

/// A message's confirmed label as prepared outside the transaction that
/// records it (the features need the message parsed).
struct PreparedLabel {
    message: Message,
    fingerprint: String,
    features: String,
}

/// What a report or "Not junk" changed, so one Undo can put all of it back.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JunkOperation {
    pub events: Vec<i64>,
    pub rules: Vec<RuleChange>,
    pub moves: Vec<MoveRecord>,
}

fn apply_labels_in(conn: &Connection, prepared: &[PreparedLabel], label: Label, now: i64) -> Result<Vec<i64>> {
    let mut events = Vec::new();
    for p in prepared {
        let m = &p.message;
        // The message may have gone since it was prepared.
        let exists: Option<i64> = conn.query_row("SELECT id FROM messages WHERE id = ?1", [m.id], |r| r.get(0)).optional()?;
        if exists.is_none() {
            continue;
        }
        let current: Option<(String, i64)> = conn
            .query_row(
                "SELECT label, event_id FROM junk_labels WHERE account_id = ?1 AND fingerprint = ?2",
                params![m.account_id, p.fingerprint],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((l, _)) = &current
            && *l == label.as_str()
        {
            conn.execute(
                "UPDATE junk_labels SET message_id = ?1 WHERE account_id = ?2 AND fingerprint = ?3",
                params![m.id, m.account_id, p.fingerprint],
            )?;
            continue;
        }
        conn.execute(
            "INSERT INTO junk_label_events (account_id, message_id, fingerprint, label, previous, created_at, subject, sender)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![m.account_id, m.id, p.fingerprint, label.as_str(), current.as_ref().map(|c| c.1), now, m.subject, m.from.address],
        )?;
        let event = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO junk_labels (account_id, fingerprint, message_id, label, event_id, features, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(account_id, fingerprint) DO UPDATE SET
                message_id = excluded.message_id, label = excluded.label, event_id = excluded.event_id,
                features = excluded.features, updated_at = excluded.updated_at",
            params![m.account_id, p.fingerprint, m.id, label.as_str(), event, p.features, now],
        )?;
        events.push(event);
    }
    if !events.is_empty() {
        bump_revision_in(conn)?;
        // A relabel withdraws what a model learnt: it stops being used now.
        invalidate_models_in(conn, Withdrawn::MarkAffected)?;
    }
    Ok(events)
}

fn undo_label_events_in(conn: &Connection, events: &[i64]) -> Result<()> {
    let mut changed = false;
    for id in events.iter().rev() {
        let Some(e) = conn.query_row(&format!("SELECT {EVENT_COLUMNS} FROM junk_label_events WHERE id = ?1"), [id], event_from_row).optional()?
        else {
            continue;
        };
        if e.undone {
            continue;
        }
        conn.execute("UPDATE junk_label_events SET undone = 1 WHERE id = ?1", [id])?;
        changed = true;
        let effective: Option<i64> = conn
            .query_row(
                "SELECT event_id FROM junk_labels WHERE account_id = ?1 AND fingerprint = ?2",
                params![e.account_id, e.fingerprint],
                |r| r.get(0),
            )
            .optional()?;
        if effective != Some(e.id) {
            continue;
        }
        let prev = match e.previous {
            Some(p) => conn
                .query_row(&format!("SELECT {EVENT_COLUMNS} FROM junk_label_events WHERE id = ?1 AND undone = 0"), [p], event_from_row)
                .optional()?,
            None => None,
        };
        match prev {
            Some(p) => conn.execute(
                "UPDATE junk_labels SET label = ?1, event_id = ?2 WHERE account_id = ?3 AND fingerprint = ?4",
                params![p.label.as_str(), p.id, e.account_id, e.fingerprint],
            )?,
            None => conn.execute("DELETE FROM junk_labels WHERE account_id = ?1 AND fingerprint = ?2", params![e.account_id, e.fingerprint])?,
        };
    }
    if changed {
        bump_revision_in(conn)?;
        invalidate_models_in(conn, Withdrawn::MarkAffected)?;
    }
    Ok(())
}

/// What happens to learned models trained on a label that is gone.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Withdrawn {
    /// Undo / relabel: kept for the record, never used again.
    MarkAffected,
    /// Account removal: deleted.
    Delete,
}

/// Every learned model trained on a label that is no longer in effect stops
/// being usable, in the same transaction as the change that withdrew it: the
/// active one (then rules only, or the shipped model, until a replacement
/// passes), kept ones Roll back could return to, and rejected candidates.
fn invalidate_models_in(conn: &Connection, how: Withdrawn) -> Result<()> {
    let current: HashSet<String> = {
        let mut stmt = conn.prepare_cached("SELECT fingerprint || ':' || label FROM junk_labels")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.collect::<std::result::Result<_, _>>()?
    };
    let models: Vec<(String, String, Vec<u8>)> = {
        let mut stmt = conn.prepare_cached("SELECT digest, status, bytes FROM junk_models")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect::<std::result::Result<_, _>>()?
    };
    let mut withdrawn = HashSet::new();
    for (digest, status, bytes) in models {
        let stale = match Model::from_bytes(&bytes) {
            Some(m) => m.manifest.trained_on.iter().any(|t| !current.contains(t)),
            // Unreadable: can't show what it learnt from, so it can't stay.
            None => true,
        };
        if !stale {
            continue;
        }
        withdrawn.insert(digest.clone());
        match how {
            Withdrawn::Delete => {
                conn.execute("DELETE FROM junk_models WHERE digest = ?1", [&digest])?;
            }
            Withdrawn::MarkAffected if status != "affected" => {
                conn.execute("UPDATE junk_models SET status = 'affected' WHERE digest = ?1", [&digest])?;
            }
            Withdrawn::MarkAffected => {}
        }
    }
    if kv_in(conn, ACTIVE_MODEL)?.is_some_and(|d| withdrawn.contains(&d)) {
        kv_set_in(conn, ACTIVE_MODEL, &fallback_model())?;
    }
    Ok(())
}

/// What scores mail when no personal model may: the shipped model, else nothing.
fn fallback_model() -> String {
    junk::base_model().map(|b| b.digest.clone()).unwrap_or_else(|| "off".into())
}

impl Store {
    // ------------------------------------------------------------ assessments

    fn check_context(&self) -> Result<CheckContext> {
        let (version, familiar, people) = {
            let conn = self.conn();
            let rev = kv_in(&conn, CONTEXT_REVISION)?.unwrap_or_else(|| "0".into());
            (format!("a{}-c{rev}", analyze::ANALYSIS_VERSION), familiar_domains_in(&conn)?, known_people_in(&conn)?)
        };
        Ok(CheckContext { version, familiar, people, model: self.active_model()? })
    }

    fn cached_check(&self, id: MessageId) -> Result<Option<CachedCheck>> {
        let row: Option<(String, String, String, String, Option<f64>)> = self
            .conn()
            .query_row("SELECT version, json, features, model, score FROM junk_assessments WHERE message_id = ?1", [id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .optional()?;
        Ok(row.and_then(|(version, json, features, model, score)| {
            Some(CachedCheck {
                version,
                analysis: serde_json::from_str(&json).ok()?,
                features: junk::learn::features_from_json(&features),
                model,
                score: score.map(|s| s as f32),
            })
        }))
    }

    /// Check one message (warning signs, and the model's score when one is
    /// active) and cache the result. Reads only stored data.
    fn compute_check(&self, m: &Message, ctx: &CheckContext) -> Result<CachedCheck> {
        let body = self.message_body(m.id)?;
        let empty = MessageBody::default();
        let b = body.as_ref().unwrap_or(&empty);
        let trusted = trusted_reporters_in(&self.conn(), m.account_id)?;
        let analysis = analyze_guarded(&analyze::Input {
            message: m,
            html: b.html.as_deref(),
            text: &b.text,
            attachments: &b.attachments,
            familiar_domains: &ctx.familiar,
            trusted_reporters: &trusted,
            known_people: &ctx.people,
        });
        let features = junk::learn::features(m, body.as_ref(), &analysis);
        let (model, score) = match &ctx.model {
            Some(model) => (model.digest.clone(), Some(model.score(&features))),
            None => (String::new(), None),
        };
        self.conn().execute(
            "INSERT OR REPLACE INTO junk_assessments (message_id, version, json, features, model, score, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                m.id,
                ctx.version,
                serde_json::to_string(&analysis)?,
                junk::learn::features_to_json(&features),
                model,
                score.map(f64::from),
                Utc::now().timestamp()
            ],
        )?;
        Ok(CachedCheck { version: ctx.version.clone(), analysis, features: Some(features), model, score })
    }

    /// Score a cached check with the active model (the warning signs are
    /// still current, only the model changed).
    fn rescore(&self, id: MessageId, check: &mut CachedCheck, model: Option<&Arc<Model>>) -> Result<()> {
        let (digest, score) = match (model, &check.features) {
            (Some(m), Some(f)) => (m.digest.clone(), Some(m.score(f))),
            (Some(m), None) => (m.digest.clone(), None),
            (None, _) => (String::new(), None),
        };
        self.conn().execute(
            "UPDATE junk_assessments SET model = ?1, score = ?2 WHERE message_id = ?3",
            params![digest, score.map(f64::from), id],
        )?;
        check.model = digest;
        check.score = score;
        Ok(())
    }

    /// The assessment of a message with up-to-date advice (computed now if
    /// it isn't cached yet, or its message, contacts or model changed).
    /// `None` for your own mail and drafts.
    pub fn assessment(&self, id: MessageId) -> Result<Option<Assessment>> {
        let Some(m) = self.message(id)? else { return Ok(None) };
        let role = self.folder(m.folder_id)?.map(|f| f.role);
        if m.is_draft || matches!(role, Some(FolderRole::Sent | FolderRole::Drafts | FolderRole::Outbox)) {
            return Ok(None);
        }
        let ctx = self.check_context()?;
        let digest = ctx.model.as_ref().map(|m| m.digest.clone()).unwrap_or_default();
        let check = match self.cached_check(id)? {
            Some(mut c) if c.version == ctx.version => {
                if c.model != digest {
                    self.rescore(id, &mut c, ctx.model.as_ref())?;
                }
                c
            }
            _ => self.compute_check(&m, &ctx)?,
        };
        let mut a = Assessment { analysis: check.analysis, score: check.score, model: (!check.model.is_empty()).then_some(check.model), advice: Advice::None };
        let allowed = self.sender_decision(m.account_id, &m.from.address)?.is_some_and(|r| r.action() == RuleAction::Allow);
        a.advice = advise(&a, self.junk_threshold()?, self.message_label(id)?, allowed, role == Some(FolderRole::Inbox));
        Ok(Some(a))
    }

    /// Check up to `limit` messages whose check is missing or out of date,
    /// newest first (the background pass), then rescore up to the rest of
    /// `limit` whose only change is the model. Returns how many were done;
    /// call again until it returns 0.
    pub fn assess_pending(&self, limit: u32) -> Result<usize> {
        let ctx = self.check_context()?;
        let digest = ctx.model.as_ref().map(|m| m.digest.clone()).unwrap_or_default();
        let ids: Vec<MessageId> = {
            let conn = self.conn();
            let mut stmt = conn.prepare_cached(
                "SELECT m.id FROM messages m JOIN folders f ON f.id = m.folder_id
                 LEFT JOIN junk_assessments a ON a.message_id = m.id
                 WHERE (a.message_id IS NULL OR a.version != ?1) AND f.role NOT IN ('sent', 'drafts', 'outbox')
                   AND COALESCE(json_extract(m.json, '$.is_draft'), 0) = 0
                 ORDER BY json_extract(m.json, '$.received_at') DESC LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![ctx.version, limit], |r| r.get(0))?;
            rows.collect::<std::result::Result<_, _>>()?
        };
        let mut done = 0;
        for m in self.messages_by_ids(&ids)? {
            self.compute_check(&m, &ctx)?;
            done += 1;
        }
        let rest = limit.saturating_sub(done as u32);
        if rest > 0 {
            let stale: Vec<MessageId> = {
                let conn = self.conn();
                let mut stmt = conn.prepare_cached("SELECT message_id FROM junk_assessments WHERE version = ?1 AND model != ?2 LIMIT ?3")?;
                let rows = stmt.query_map(params![ctx.version, digest, rest], |r| r.get(0))?;
                rows.collect::<std::result::Result<_, _>>()?
            };
            for id in stale {
                if let Some(mut c) = self.cached_check(id)? {
                    self.rescore(id, &mut c, ctx.model.as_ref())?;
                    done += 1;
                }
            }
        }
        Ok(done)
    }

    /// Advice for list rows, from current cached checks only (rows that
    /// haven't been checked yet have none).
    pub fn advice_for(&self, ids: &[MessageId]) -> Result<HashMap<MessageId, Advice>> {
        let mut out = HashMap::new();
        if ids.is_empty() {
            return Ok(out);
        }
        let version = {
            let conn = self.conn();
            format!("a{}-c{}", analyze::ANALYSIS_VERSION, kv_in(&conn, CONTEXT_REVISION)?.unwrap_or_else(|| "0".into()))
        };
        let digest = self.active_model()?.map(|m| m.digest.clone()).unwrap_or_default();
        let threshold = self.junk_threshold()?;
        let rules = self.sender_rules()?;
        let msgs = self.messages_by_ids(ids)?;
        let inbox_ids: HashSet<FolderId> = self.folders(None)?.into_iter().filter(|f| f.role == FolderRole::Inbox).map(|f| f.id).collect();
        for m in msgs {
            let Some(c) = self.cached_check(m.id)?.filter(|c| c.version == version) else { continue };
            let a = Assessment { analysis: c.analysis, score: c.score.filter(|_| c.model == digest), ..Default::default() };
            let allowed = junk::decide(&rules, m.account_id, &m.from.address).is_some_and(|r| r.action() == RuleAction::Allow);
            // The label only matters for a junk suggestion: look it up then.
            let label = if a.score.is_some_and(|s| s >= threshold) { self.message_label(m.id)? } else { None };
            let advice = advise(&a, threshold, label, allowed, inbox_ids.contains(&m.folder_id));
            if advice != Advice::None {
                out.insert(m.id, advice);
            }
        }
        Ok(out)
    }

    // ------------------------------------------------------------ rules

    /// Every sender rule, all-accounts ones first.
    pub fn sender_rules(&self) -> Result<Vec<SenderRule>> {
        rules_in(&self.conn())
    }

    /// Add or change the rule for `pattern` (an address or a domain) in
    /// `account` (`None`: every account). There is one rule per account and
    /// pattern, so this replaces an earlier choice; the returned change
    /// undoes it.
    pub fn set_sender_rule(&self, account: Option<AccountId>, pattern: &str, name: &str, action: RuleAction) -> Result<RuleChange> {
        let now = Utc::now().timestamp();
        self.tx(|tx| set_rule_in(tx, account, pattern, name, action, now))
    }

    /// Remove a rule; returns it so it can be put back.
    pub fn remove_sender_rule(&self, id: i64) -> Result<Option<SenderRule>> {
        self.tx(|tx| {
            let rule = rule_by_id_in(tx, id)?;
            tx.execute("DELETE FROM junk_sender_rules WHERE id = ?1", [id])?;
            Ok(rule)
        })
    }

    /// Put a removed rule back (Undo).
    pub fn restore_sender_rule(&self, rule: &SenderRule) -> Result<()> {
        put_rule_in(&self.conn(), rule).map(|_| ())
    }

    /// Undo [`Store::set_sender_rule`]: the rule goes, the earlier one (if
    /// any) comes back, unless the rule was changed again since.
    pub fn undo_rule_change(&self, change: &RuleChange) -> Result<()> {
        self.tx(|tx| undo_rule_change_in(tx, change))
    }

    /// The rule that decides for `address` in `account`.
    pub fn sender_decision(&self, account: AccountId, address: &str) -> Result<Option<SenderRule>> {
        decide_in(&self.conn(), account, address)
    }

    /// True when a block rule applies to `address` in `account`.
    pub fn is_sender_blocked(&self, account: AccountId, address: &str) -> Result<bool> {
        Ok(self.sender_decision(account, address)?.is_some_and(|r| r.action() == RuleAction::Block))
    }

    /// The block rule that sent this message to Junk Email on arrival, if any.
    pub fn routed_by_rule(&self, message_id: MessageId) -> Result<Option<String>> {
        Ok(self.conn().query_row("SELECT pattern FROM junk_routing WHERE message_id = ?1", [message_id], |r| r.get(0)).optional()?)
    }

    // ------------------------------------------------------------ pictures

    /// Load pictures from the internet in mail from this sender or domain
    /// (a domain covers its subdomains). Only a picture permission: it does
    /// not change junk handling.
    pub fn allow_images_from(&self, address: &str) -> Result<()> {
        let Some((pattern, domain)) = junk::normalize_pattern(address) else {
            return Err(Error::Invalid("sender address is empty".into()));
        };
        self.conn().execute(
            "INSERT OR IGNORE INTO junk_image_senders (address, added_at, subdomains) VALUES (?1, ?2, ?3)",
            params![pattern, Utc::now().timestamp(), domain],
        )?;
        Ok(())
    }

    pub fn remove_image_sender(&self, address: &str) -> Result<()> {
        self.conn().execute("DELETE FROM junk_image_senders WHERE address = lower(?1)", [address.trim()])?;
        Ok(())
    }

    pub fn image_senders(&self) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached("SELECT address FROM junk_image_senders ORDER BY address")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// True when pictures load automatically for this sender: its address,
    /// its domain, or (for a permission that covers them) a parent domain.
    pub fn images_allowed_for(&self, address: &str) -> Result<bool> {
        let a = address.trim().to_lowercase();
        if a.is_empty() {
            return Ok(false);
        }
        let d = junk::domain_of(&a).to_string();
        let conn = self.conn();
        let mut stmt = conn.prepare_cached("SELECT address, subdomains FROM junk_image_senders")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?)))?;
        for row in rows {
            let (p, subdomains) = row?;
            if p == a || (!p.contains('@') && (p == d || (subdomains && junk::domain_matches(&d, &p)))) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    // ------------------------------------------------------------ labels

    fn prepare_labels(&self, ids: &[MessageId]) -> Result<Vec<PreparedLabel>> {
        let msgs = self.messages_by_ids(ids)?;
        let ctx = self.check_context()?;
        let mut prepared = Vec::with_capacity(msgs.len());
        for m in msgs {
            let body = self.message_body(m.id)?;
            let fingerprint = junk::fingerprint_of(&m, body.as_ref());
            let features = match self.cached_check(m.id)?.filter(|c| c.version == ctx.version).and_then(|c| c.features) {
                Some(f) => f,
                None => self.compute_check(&m, &ctx)?.features.unwrap_or_default(),
            };
            prepared.push(PreparedLabel { message: m, fingerprint, features: junk::learn::features_to_json(&features) });
        }
        Ok(prepared)
    }

    /// Record the user's confirmed label for these messages. Repeating the
    /// same label changes nothing; a different label replaces the effective
    /// one (the old one stays in the history). Returns the new events, for
    /// Undo.
    pub fn label_messages(&self, ids: &[MessageId], label: Label) -> Result<Vec<i64>> {
        let prepared = self.prepare_labels(ids)?;
        let now = Utc::now().timestamp();
        let events = self.tx(|tx| apply_labels_in(tx, &prepared, label, now))?;
        self.forget_cached_model();
        Ok(events)
    }

    /// Undo label events: each one's message goes back to the label it had
    /// before (or none). Undoing twice does nothing more.
    pub fn undo_label_events(&self, events: &[i64]) -> Result<()> {
        self.tx(|tx| undo_label_events_in(tx, events))?;
        self.forget_cached_model();
        Ok(())
    }

    /// Report junk / Report phishing, as one change: the label, a block rule
    /// for each sender in its own account (when `block`), and the move to
    /// Junk Email. Either all of it is recorded or none; the result undoes
    /// all of it.
    pub fn report_junk(&self, ids: &[MessageId], label: Label, block: bool) -> Result<JunkOperation> {
        let prepared = self.prepare_labels(ids)?;
        let now = Utc::now().timestamp();
        let op = self.tx(|tx| {
            let events = apply_labels_in(tx, &prepared, label, now)?;
            let mut rules = Vec::new();
            if block {
                let mut seen = HashSet::new();
                for p in &prepared {
                    let m = &p.message;
                    if seen.insert((m.account_id, m.from.address.to_lowercase())) {
                        rules.push(set_rule_in(tx, Some(m.account_id), &m.from.address, &m.from.name, RuleAction::Block, now)?);
                    }
                }
            }
            let ids: Vec<MessageId> = prepared.iter().map(|p| p.message.id).collect();
            let moves = Self::mark_junk_in(tx, &ids)?;
            Ok(JunkOperation { events, rules, moves })
        })?;
        self.forget_cached_model();
        Ok(op)
    }

    /// "Not junk", as one change: the label, then the messages that are in
    /// Junk Email go back where they came from.
    pub fn mark_not_junk(&self, ids: &[MessageId]) -> Result<JunkOperation> {
        let prepared = self.prepare_labels(ids)?;
        let now = Utc::now().timestamp();
        let op = self.tx(|tx| {
            let events = apply_labels_in(tx, &prepared, Label::NotSpam, now)?;
            let mut in_junk = Vec::new();
            for p in &prepared {
                let role: Option<String> = tx
                    .query_row("SELECT f.role FROM messages m JOIN folders f ON f.id = m.folder_id WHERE m.id = ?1", [p.message.id], |r| r.get(0))
                    .optional()?;
                if role.as_deref() == Some("junk") {
                    in_junk.push(p.message.id);
                }
            }
            let moves = Self::restore_messages_in(tx, &in_junk)?;
            Ok(JunkOperation { events, rules: Vec::new(), moves })
        })?;
        self.forget_cached_model();
        Ok(op)
    }

    /// Undo a report or "Not junk", all of it or (on any error) none of it:
    /// the moves, the rules (unless changed again since) and the labels.
    pub fn undo_junk_operation(&self, op: &JunkOperation) -> Result<()> {
        self.tx(|tx| {
            Self::undo_moves_in(tx, &op.moves)?;
            for r in op.rules.iter().rev() {
                undo_rule_change_in(tx, r)?;
            }
            undo_label_events_in(tx, &op.events)
        })?;
        self.forget_cached_model();
        Ok(())
    }

    /// The confirmed label of a message, if the user gave it one: found by
    /// the message's account and content, never by its row id alone (ids are
    /// reused after a message is deleted for good).
    pub fn message_label(&self, id: MessageId) -> Result<Option<Label>> {
        let Some(m) = self.message(id)? else { return Ok(None) };
        let body = self.message_body(id)?;
        let fp = junk::fingerprint_of(&m, body.as_ref());
        let l: Option<String> = self
            .conn()
            .query_row("SELECT label FROM junk_labels WHERE account_id = ?1 AND fingerprint = ?2", params![m.account_id, fp], |r| r.get(0))
            .optional()?;
        Ok(l.as_deref().and_then(Label::parse))
    }

    /// Recent label events, newest first (Settings history).
    pub fn label_history(&self, limit: u32) -> Result<Vec<LabelEvent>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(&format!("SELECT {EVENT_COLUMNS} FROM junk_label_events ORDER BY id DESC LIMIT ?1"))?;
        let rows = stmt.query_map([limit], event_from_row)?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Effective labels with their stored features (the learner's data).
    pub fn effective_labels(&self) -> Result<Vec<junk::EffectiveLabel>> {
        effective_labels_in(&self.conn())
    }

    /// Changes whenever a confirmed label is added, changed or undone.
    pub fn label_revision(&self) -> Result<i64> {
        revision_in(&self.conn())
    }

    /// Forget learned data: every confirmed label and its history, and so
    /// every model learnt from them. Rules and picture permissions stay.
    pub fn forget_labels(&self) -> Result<()> {
        self.tx(|tx| {
            tx.execute("DELETE FROM junk_labels", [])?;
            tx.execute("DELETE FROM junk_label_events", [])?;
            bump_revision_in(tx)?;
            invalidate_models_in(tx, Withdrawn::MarkAffected)
        })?;
        self.forget_cached_model();
        Ok(())
    }
}

fn effective_labels_in(conn: &Connection) -> Result<Vec<junk::EffectiveLabel>> {
    let mut stmt = conn.prepare_cached("SELECT account_id, fingerprint, label, features FROM junk_labels ORDER BY account_id, fingerprint")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, String>(2)?, r.get(3)?)))?;
    let mut out = Vec::new();
    for row in rows {
        let (a, fp, l, f): (AccountId, String, String, Option<String>) = row?;
        if let Some(label) = Label::parse(&l) {
            out.push(junk::EffectiveLabel { account_id: a, fingerprint: fp, label, features: f });
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- models

const ACTIVE_MODEL: &str = "junk.active_model";
const TRAINED_REVISION: &str = "junk.trained_revision";
const LEARN_STATUS: &str = "junk.learn_status";
/// Fewest confirmed examples of each kind before a personal model is tried,
/// counted in independent groups (see [`junk::campaign_key`]): thirty copies
/// of one campaign are one example.
pub const MIN_PER_CLASS: usize = 5;

/// Where the learner stands, for Settings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LearnStatus {
    pub active: Option<Model>,
    /// The approved model before the active one (Roll back goes there).
    pub previous: Option<String>,
    pub wanted: usize,
    pub unwanted: usize,
    /// What happened at the last training attempt.
    pub last: String,
}

/// A usable stored model: one that passed its checks and learnt nothing
/// since withdrawn (those are marked affected or deleted).
fn model_by_digest_in(conn: &Connection, digest: &str) -> Result<Option<Model>> {
    if let Some(b) = junk::base_model().filter(|b| b.digest == digest) {
        return Ok(Some(b.clone()));
    }
    let bytes: Option<Vec<u8>> = conn
        .query_row("SELECT bytes FROM junk_models WHERE digest = ?1 AND status IN ('active', 'retired')", [digest], |r| r.get(0))
        .optional()?;
    Ok(bytes.and_then(|b| Model::from_bytes(&b)))
}

fn save_model_in(conn: &Connection, m: &Model, status: &str) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO junk_models (digest, kind, parent, status, created_at, gate, bytes) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![m.digest, m.manifest.kind, m.manifest.parent, status, Utc::now().timestamp(), serde_json::to_string(&m.manifest.gate)?, m.to_bytes()],
    )?;
    Ok(())
}

impl Store {
    pub(crate) fn forget_cached_model(&self) {
        *self.junk_model.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }

    /// Test hook: called at each step of a training run, outside any lock.
    fn learn_step(&self, _step: &str) {
        #[cfg(test)]
        {
            let hook = self.learn_hook.lock().unwrap_or_else(|p| p.into_inner()).clone();
            if let Some(h) = hook {
                h(_step);
            }
        }
    }

    /// The model that scores mail now: the approved personal model, else
    /// the shipped base model. `None` when suggestions are off or there is
    /// no usable model. The choice and the model are read together, so a
    /// model being withdrawn at the same time is never handed out.
    pub fn active_model(&self) -> Result<Option<Arc<Model>>> {
        let conn = self.conn();
        let digest = match kv_in(&conn, ACTIVE_MODEL)? {
            Some(d) if d == "off" => return Ok(None),
            Some(d) => d,
            None => match junk::base_model() {
                Some(b) => b.digest.clone(),
                None => return Ok(None),
            },
        };
        {
            let cache = self.junk_model.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(m) = cache.as_ref().filter(|m| m.digest == digest) {
                return Ok(Some(m.clone()));
            }
        }
        let Some(model) = model_by_digest_in(&conn, &digest)? else {
            return Ok(junk::base_model().map(|b| Arc::new(b.clone())));
        };
        let model = Arc::new(model);
        // Still holding the connection: nothing withdrew it meanwhile.
        *self.junk_model.lock().unwrap_or_else(|p| p.into_inner()) = Some(model.clone());
        Ok(Some(model))
    }

    /// The score at which "looks like junk" is suggested; never reached
    /// when suggestions are off.
    pub fn junk_threshold(&self) -> Result<f32> {
        if !self.settings().junk_suggestions {
            return Ok(f32::INFINITY);
        }
        Ok(self.active_model()?.map(|m| m.threshold).unwrap_or(f32::INFINITY))
    }

    /// Train a candidate when the confirmed labels changed since the last
    /// attempt, check it, and make it active only if it passes.
    ///
    /// Training runs on a snapshot, without locks. Its result is recorded
    /// only if nothing changed meanwhile (no label added, changed, undone or
    /// forgotten, no account removed, no other model made active): otherwise
    /// it is thrown away whole and the next pass starts again. The candidate,
    /// the switch and "this revision is done" are written in one
    /// transaction. Returns true when the active model changed.
    pub fn retrain_if_needed(&self) -> Result<bool> {
        let (rev, active_key, labels) = {
            let conn = self.conn();
            let rev = revision_in(&conn)?;
            if kv_in(&conn, TRAINED_REVISION)?.and_then(|v| v.parse::<i64>().ok()) == Some(rev) {
                return Ok(false);
            }
            (rev, kv_in(&conn, ACTIVE_MODEL)?, effective_labels_in(&conn)?)
        };
        self.learn_step("snapshot");
        let base = junk::base_model();
        // Split by campaign: every copy of one goes to training, or every
        // copy is held back to check the result, never some of each.
        let mut train_set = Vec::new();
        let mut holdout = Vec::new();
        let mut groups: [HashSet<String>; 2] = Default::default();
        for l in &labels {
            let Some(f) = l.features.as_deref().and_then(junk::learn::features_from_json) else { continue };
            let group = junk::campaign_key(&f);
            let e = junk::learn::Example { features: f, unwanted: l.label.is_unwanted(), phishing: l.label == Label::Phishing, id: l.fingerprint.clone() };
            if junk::held_back(&group) {
                holdout.push(e);
            } else {
                groups[usize::from(e.unwanted)].insert(group);
                train_set.push((e, format!("{}:{}", l.fingerprint, l.label.as_str())));
            }
        }
        let (wanted, unwanted) = (groups[0].len(), groups[1].len());
        if unwanted < MIN_PER_CLASS || wanted < MIN_PER_CLASS {
            let status = format!(
                "Not enough different confirmed examples yet ({unwanted} junk, {wanted} not junk; {MIN_PER_CLASS} of each needed, copies of one message count once)."
            );
            self.commit_training(rev, &active_key, |tx| kv_set_in(tx, LEARN_STATUS, &status).map(|_| false))?;
            return Ok(false);
        }
        let params = junk::learn::TrainParams::default();
        let examples: Vec<junk::learn::Example> = train_set.iter().map(|(e, _)| e.clone()).collect();
        let mut cand = junk::learn::train(base, &examples, params);
        self.learn_step("trained");
        // Threshold on development data only: the shipped development set
        // plus the reports trained on, never the held-back checks.
        let fixtures = junk::gate_fixtures();
        let mut dev: Vec<_> = fixtures.iter().filter(|f| f.set == "dev").map(|f| f.example()).collect();
        dev.extend(examples.iter().filter(|e| !e.unwanted).cloned());
        cand.threshold = junk::learn::choose_threshold(&cand, &dev, 0.0);
        let active = active_key.as_deref().filter(|d| *d != "off").and_then(|d| model_by_digest_in(&self.conn(), d).ok().flatten());
        let active = active.or_else(|| if active_key.is_none() { base.cloned() } else { None });
        let reference = active.clone().unwrap_or_else(Model::rules_only);
        cand.manifest = junk::learn::Manifest {
            kind: "personal".into(),
            parent: active.as_ref().map(|a| a.digest.clone()),
            label_revision: rev,
            examples_unwanted: train_set.iter().filter(|(e, _)| e.unwanted).count(),
            examples_wanted: train_set.iter().filter(|(e, _)| !e.unwanted).count(),
            epochs: params.epochs,
            learning_rate: params.learning_rate,
            l2: params.l2,
            seed_data: base.map(|b| format!("starts from shipped model {}", b.digest)).unwrap_or_else(|| "your reports only".into()),
            trained_on: train_set.iter().map(|(_, t)| t.clone()).collect(),
            gate: None,
        };
        let verdict = if !cand.threshold.is_finite() {
            junk::learn::Verdict::InsufficientEvidence("no score separates your junk from the mail you want".into())
        } else {
            let mut sets = Vec::new();
            for (name, set) in [("regression fixtures", "regression"), ("fresh validation", "fresh")] {
                let xs: Vec<_> = fixtures.iter().filter(|f| f.set == set).map(|f| f.example()).collect();
                sets.push(junk::learn::GateSet {
                    name: name.into(),
                    active: junk::learn::evaluate(&reference, &xs, reference.threshold),
                    candidate: junk::learn::evaluate(&cand, &xs, cand.threshold),
                    needs_phishing: true,
                });
            }
            sets.push(junk::learn::GateSet {
                name: "your held-back reports".into(),
                active: junk::learn::evaluate(&reference, &holdout, reference.threshold),
                candidate: junk::learn::evaluate(&cand, &holdout, cand.threshold),
                needs_phishing: false,
            });
            junk::learn::gate(&sets)
        };
        cand.manifest.gate = Some(verdict.clone());
        let cand = cand.seal();
        self.learn_step("evaluated");
        let changed = self.commit_training(rev, &active_key, |tx| match &verdict {
            junk::learn::Verdict::Promote => {
                if let Some(a) = active.as_ref().filter(|a| a.manifest.kind == "personal") {
                    tx.execute("UPDATE junk_models SET status = 'retired' WHERE digest = ?1 AND status = 'active'", [&a.digest])?;
                }
                save_model_in(tx, &cand, "active")?;
                kv_set_in(tx, ACTIVE_MODEL, &cand.digest)?;
                kv_set_in(tx, LEARN_STATUS, &format!("Updated from {unwanted} junk and {wanted} not-junk reports; passed every check."))?;
                Ok(true)
            }
            junk::learn::Verdict::Reject(why) => {
                save_model_in(tx, &cand, "rejected")?;
                kv_set_in(tx, LEARN_STATUS, &format!("A new version was trained but not used: {}.", why.join("; ")))?;
                Ok(false)
            }
            junk::learn::Verdict::InsufficientEvidence(why) => {
                if cand.threshold.is_finite() {
                    save_model_in(tx, &cand, "rejected")?;
                }
                kv_set_in(tx, LEARN_STATUS, &format!("Not enough evidence to check a new version ({why}); the current one stays."))?;
                Ok(false)
            }
        })?;
        Ok(changed.unwrap_or(false))
    }

    /// Record the outcome of a training run in one transaction, together
    /// with "revision `rev` is done", but only if the labels and the active
    /// model are still what the run started from. `None` when the run was
    /// stale and nothing was written.
    fn commit_training(&self, rev: i64, active_key: &Option<String>, f: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<bool>) -> Result<Option<bool>> {
        self.learn_step("commit");
        let out = self.tx(|tx| {
            if revision_in(tx)? != rev || kv_in(tx, ACTIVE_MODEL)? != *active_key {
                return Ok(None);
            }
            let changed = f(tx)?;
            kv_set_in(tx, TRAINED_REVISION, &rev.to_string())?;
            Ok(Some(changed))
        })?;
        self.forget_cached_model();
        Ok(out)
    }

    pub fn learn_status(&self) -> Result<LearnStatus> {
        let active = self.active_model()?.map(|m| (*m).clone());
        let previous = active.as_ref().and_then(|a| a.manifest.parent.clone()).filter(|p| model_by_digest_in(&self.conn(), p).ok().flatten().is_some());
        let labels = self.effective_labels()?;
        let unwanted = labels.iter().filter(|l| l.label.is_unwanted()).count();
        Ok(LearnStatus { previous, wanted: labels.len() - unwanted, unwanted, last: self.kv_get(LEARN_STATUS)?.unwrap_or_default(), active })
    }

    /// Go back to the model before the active one (or the shipped one).
    /// Reports and rules are untouched. Never returns to a model that learnt
    /// from something since withdrawn (those aren't usable any more).
    pub fn roll_back_model(&self) -> Result<()> {
        self.tx(|tx| {
            let Some(digest) = kv_in(tx, ACTIVE_MODEL)?.filter(|d| d != "off") else { return Ok(()) };
            let Some(active) = model_by_digest_in(tx, &digest)? else { return Ok(()) };
            let target = active
                .manifest
                .parent
                .clone()
                .filter(|p| model_by_digest_in(tx, p).ok().flatten().is_some())
                .or_else(|| junk::base_model().map(|b| b.digest.clone()).filter(|b| *b != active.digest))
                .unwrap_or_else(|| "off".into());
            if active.manifest.kind == "personal" {
                tx.execute("UPDATE junk_models SET status = 'rolled_back' WHERE digest = ?1", [&active.digest])?;
            }
            if target != "off" {
                tx.execute("UPDATE junk_models SET status = 'active' WHERE digest = ?1", [&target])?;
            }
            kv_set_in(tx, ACTIVE_MODEL, &target)?;
            kv_set_in(tx, LEARN_STATUS, "Rolled back by you.")?;
            // Don't retrain the same labels straight into the version just left.
            kv_set_in(tx, TRAINED_REVISION, &revision_in(tx)?.to_string())
        })?;
        self.forget_cached_model();
        Ok(())
    }

    /// Forget learned data: confirmed labels, their history, every personal
    /// model and every cached score, in one transaction. A training run
    /// already under way sees the change and throws its result away. The
    /// shipped model and all rules stay.
    pub fn forget_learning(&self) -> Result<()> {
        self.tx(|tx| {
            tx.execute("DELETE FROM junk_labels", [])?;
            tx.execute("DELETE FROM junk_label_events", [])?;
            tx.execute("DELETE FROM junk_models", [])?;
            tx.execute("DELETE FROM junk_assessments", [])?;
            tx.execute("DELETE FROM kv WHERE key IN (?1, ?2, ?3)", params![ACTIVE_MODEL, TRAINED_REVISION, LEARN_STATUS])?;
            bump_revision_in(tx)
        })?;
        self.forget_cached_model();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_util::store_with_account;

    fn inbox(store: &Store, acct: AccountId) -> FolderId {
        store.folder_for_role(acct, FolderRole::Inbox).unwrap()
    }

    fn msg(store: &Store, acct: AccountId, from: &str, subject: &str) -> MessageId {
        let mut m = Message {
            account_id: acct,
            folder_id: inbox(store, acct),
            from: EmailAddress::new("", from),
            subject: subject.into(),
            received_at: Utc::now(),
            ..Default::default()
        };
        let body = MessageBody { text: format!("body of {subject}"), ..Default::default() };
        store.insert_message(&mut m, Some(&body)).unwrap()
    }

    fn second_account(store: &Store) -> AccountId {
        let mut a = Account { kind: AccountKind::Local, email: "second@axentra.example".into(), ..Default::default() };
        store.add_account(&mut a).unwrap()
    }

    #[test]
    fn labels_are_idempotent_replace_and_undo() {
        let (store, acct) = store_with_account();
        let id = msg(&store, acct, "a@x.example", "Win");
        let r0 = store.label_revision().unwrap();
        let e1 = store.label_messages(&[id], Label::Spam).unwrap();
        assert_eq!(e1.len(), 1);
        assert!(store.label_messages(&[id], Label::Spam).unwrap().is_empty(), "repeat is a no-op");
        assert_eq!(store.effective_labels().unwrap().len(), 1);
        let e2 = store.label_messages(&[id], Label::NotSpam).unwrap();
        assert_eq!(store.message_label(id).unwrap(), Some(Label::NotSpam));
        assert_eq!(store.effective_labels().unwrap().len(), 1, "one effective example per message");
        store.undo_label_events(&e2).unwrap();
        assert_eq!(store.message_label(id).unwrap(), Some(Label::Spam));
        store.undo_label_events(&e2).unwrap();
        assert_eq!(store.message_label(id).unwrap(), Some(Label::Spam), "undo twice changes nothing");
        store.undo_label_events(&e1).unwrap();
        assert_eq!(store.message_label(id).unwrap(), None);
        assert!(store.effective_labels().unwrap().is_empty());
        assert!(store.label_revision().unwrap() > r0);
        assert_eq!(store.label_history(10).unwrap().len(), 2);
    }

    #[test]
    fn undoing_an_older_event_keeps_the_newer_label() {
        let (store, acct) = store_with_account();
        let id = msg(&store, acct, "a@x.example", "Hello");
        let e1 = store.label_messages(&[id], Label::Spam).unwrap();
        let e2 = store.label_messages(&[id], Label::Phishing).unwrap();
        store.undo_label_events(&e1).unwrap();
        assert_eq!(store.message_label(id).unwrap(), Some(Label::Phishing));
        store.undo_label_events(&e2).unwrap();
        assert_eq!(store.message_label(id).unwrap(), None, "the replaced label was undone too");
    }

    // ------------------------------------------------------------ label identity (F4)

    #[test]
    fn a_resynced_copy_has_the_label_without_another_report() {
        for label in [Label::Spam, Label::NotSpam, Label::Phishing] {
            let (store, acct) = store_with_account();
            let id = msg(&store, acct, "x@normal.example", "same");
            store.label_messages(&[id], label).unwrap();
            // The same mail again under a new id, while the old one is still there.
            let mut copy = store.message(id).unwrap().unwrap();
            let body = store.message_body(id).unwrap().unwrap();
            let new = store.insert_synced_message(&mut copy, &body, &[]).unwrap();
            assert_ne!(id, new);
            assert_eq!(store.message_label(new).unwrap(), Some(label), "{label:?}");
            assert_eq!(store.message_label(id).unwrap(), Some(label), "both live copies");
            assert_eq!(store.effective_labels().unwrap().len(), 1, "one training example, not two");
            // Moved and restored, still the same message.
            store.mark_junk(&[new]).unwrap();
            store.restore_messages(&[new]).unwrap();
            assert_eq!(store.message_label(new).unwrap(), Some(label));
        }
    }

    #[test]
    fn a_reused_row_id_does_not_inherit_an_unrelated_label() {
        let (store, acct) = store_with_account();
        let other = second_account(&store);
        let id = msg(&store, acct, "friend@normal.example", "safe");
        store.label_messages(&[id], Label::NotSpam).unwrap();
        store.delete_messages_permanently(&[id]).unwrap();
        // SQLite hands out the deleted highest id again.
        let replacement = msg(&store, acct, "stranger@other.example", "entirely different");
        assert_eq!(id, replacement);
        assert_eq!(store.message_label(replacement).unwrap(), None);
        // Across accounts too.
        let id2 = msg(&store, acct, "friend@normal.example", "kept");
        store.label_messages(&[id2], Label::Phishing).unwrap();
        store.delete_messages_permanently(&[id2]).unwrap();
        let in_other = msg(&store, other, "friend@normal.example", "kept");
        assert_eq!(in_other, id2);
        assert_eq!(store.message_label(in_other).unwrap(), None, "a label is per account");
        // The history stays after the message is gone.
        assert_eq!(store.label_history(10).unwrap().len(), 2);
    }

    #[test]
    fn html_only_and_changed_content_have_their_own_identity() {
        let (store, acct) = store_with_account();
        let add = |html: &str| {
            let mut m = Message { account_id: acct, folder_id: inbox(&store, acct), from: EmailAddress::new("", "a@x.example"), subject: "Same".into(), ..Default::default() };
            store.insert_message(&mut m, Some(&MessageBody { html: Some(html.into()), ..Default::default() })).unwrap()
        };
        let a = add("<p>first offer</p>");
        let b = add("<p>another message</p>");
        store.label_messages(&[a], Label::Spam).unwrap();
        assert_eq!(store.message_label(b).unwrap(), None, "same headers, different HTML");
        assert_eq!(store.message_label(add("<p>first offer</p>")).unwrap(), Some(Label::Spam));
        // A message whose content is changed is no longer the one reported.
        store.set_message_body(a, Some("<p>rewritten</p>"), "").unwrap();
        assert_eq!(store.message_label(a).unwrap(), None);
    }

    // ------------------------------------------------------------ rules

    #[test]
    fn rules_replace_per_scope_and_undo() {
        let (store, acct) = store_with_account();
        let c1 = store.set_sender_rule(Some(acct), "Deals@Spam.example", "Deals", RuleAction::Block).unwrap();
        assert!(c1.previous.is_none());
        assert!(store.is_sender_blocked(acct, "deals@spam.example").unwrap());
        assert!(!store.is_sender_blocked(acct + 1, "deals@spam.example").unwrap(), "account-scoped");
        let c2 = store.set_sender_rule(Some(acct), "deals@spam.example", "", RuleAction::Allow).unwrap();
        assert_eq!(c2.previous.as_ref().unwrap().action(), RuleAction::Block);
        assert_eq!(store.sender_rules().unwrap().len(), 1, "one choice per account and pattern");
        store.undo_rule_change(&c2).unwrap();
        assert!(store.is_sender_blocked(acct, "deals@spam.example").unwrap());
        store.undo_rule_change(&c1).unwrap();
        assert!(store.sender_rules().unwrap().is_empty());
        assert!(store.set_sender_rule(None, "not an address", "", RuleAction::Block).is_err());
        let gone = store.set_sender_rule(None, "spam.example", "", RuleAction::Block).unwrap();
        let removed = store.remove_sender_rule(gone.rule_id).unwrap().unwrap();
        assert!(store.sender_rules().unwrap().is_empty());
        store.restore_sender_rule(&removed).unwrap();
        assert!(store.is_sender_blocked(acct, "x@mail.spam.example").unwrap());
    }

    #[test]
    fn undoing_a_rule_change_keeps_a_later_edit() {
        let (store, acct) = store_with_account();
        let c1 = store.set_sender_rule(Some(acct), "a@x.example", "", RuleAction::Block).unwrap();
        // Changed again later (one second on, so the record differs).
        store.conn().execute("UPDATE junk_sender_rules SET action = 'allow', updated_at = updated_at + 1 WHERE id = ?1", [c1.rule_id]).unwrap();
        store.undo_rule_change(&c1).unwrap();
        assert_eq!(store.sender_decision(acct, "a@x.example").unwrap().unwrap().action(), RuleAction::Allow, "the later choice stays");
    }

    fn rule_of(store: &Store) -> SenderRule {
        store.sender_rules().unwrap().remove(0)
    }

    #[test]
    fn a_new_block_takes_effect_now_not_when_an_old_allow_was_made() {
        let (store, acct) = store_with_account();
        let old = 1_600_000_000;
        let r = SenderRule { account_id: Some(acct), pattern: "x@normal.example".into(), action: Some(RuleAction::Allow), created_at: old, updated_at: old, ..Default::default() };
        store.restore_sender_rule(&r).unwrap();
        let before = Utc::now().timestamp();
        let change = store.set_sender_rule(Some(acct), "x@normal.example", "", RuleAction::Block).unwrap();
        let current = rule_of(&store);
        assert!(current.effective_since() >= before, "the blocking choice didn't inherit the old allow date");
        assert_eq!(current.created_at, old, "the record keeps when it was first made");
        // Repeating the block keeps when it took effect.
        store.conn().execute("UPDATE junk_sender_rules SET effective_at = effective_at - 100", []).unwrap();
        let since = rule_of(&store).effective_since();
        store.set_sender_rule(Some(acct), "x@normal.example", "", RuleAction::Block).unwrap();
        assert_eq!(rule_of(&store).effective_since(), since);
        // Block → Allow → Block starts again.
        store.set_sender_rule(Some(acct), "x@normal.example", "", RuleAction::Allow).unwrap();
        store.set_sender_rule(Some(acct), "x@normal.example", "", RuleAction::Block).unwrap();
        assert!(rule_of(&store).effective_since() > since);
        // Undo of the first change brings back the old allow, with its dates.
        store.conn().execute("UPDATE junk_sender_rules SET updated_at = ?1", [change.applied_at]).unwrap();
        store.undo_rule_change(&change).unwrap();
        let back = rule_of(&store);
        assert_eq!((back.action(), back.created_at, back.updated_at), (RuleAction::Allow, old, old));
    }

    #[test]
    fn undo_restores_when_blocking_took_effect() {
        let (store, acct) = store_with_account();
        let old = 1_600_000_000;
        let r = SenderRule { account_id: Some(acct), pattern: "x@normal.example".into(), action: Some(RuleAction::Block), created_at: old, updated_at: old, effective_at: old, ..Default::default() };
        store.restore_sender_rule(&r).unwrap();
        let c = store.set_sender_rule(Some(acct), "x@normal.example", "", RuleAction::Allow).unwrap();
        store.undo_rule_change(&c).unwrap();
        assert_eq!(rule_of(&store).effective_since(), old);
    }

    fn sync_in(store: &Store, acct: AccountId, from: &str, received: &str) -> Message {
        let mut m = Message { account_id: acct, folder_id: inbox(store, acct), from: EmailAddress::new("", from), received_at: received.parse().unwrap(), ..Default::default() };
        store.insert_synced_message(&mut m, &MessageBody::default(), &[]).unwrap();
        m
    }

    #[test]
    fn switching_an_old_allow_to_block_does_not_route_old_mail() {
        let (store, acct) = store_with_account();
        store.set_sender_rule(Some(acct), "x@normal.example", "", RuleAction::Allow).unwrap();
        let mut r = rule_of(&store);
        r.created_at = 1_600_000_000;
        r.updated_at = r.created_at;
        r.effective_at = r.created_at;
        store.remove_sender_rule(r.id).unwrap();
        store.restore_sender_rule(&r).unwrap();
        store.set_sender_rule(Some(acct), "x@normal.example", "", RuleAction::Block).unwrap();
        let old = sync_in(&store, acct, "x@normal.example", "2025-01-01T00:00:00Z");
        assert_eq!(old.folder_id, inbox(&store, acct), "mail from before the block stays");
        let new = sync_in(&store, acct, "x@normal.example", &Utc::now().to_rfc3339());
        assert_eq!(new.folder_id, store.folder_for_role(acct, FolderRole::Junk).unwrap());
    }

    #[test]
    fn the_day_of_slack_and_unknown_arrival_times() {
        let (store, acct) = store_with_account();
        store.set_sender_rule(Some(acct), "x@normal.example", "", RuleAction::Block).unwrap();
        let since = rule_of(&store).effective_since();
        let junk = store.folder_for_role(acct, FolderRole::Junk).unwrap();
        let at = |t: i64| chrono::DateTime::from_timestamp(t, 0).unwrap().to_rfc3339();
        assert_eq!(sync_in(&store, acct, "x@normal.example", &at(since - 86_400)).folder_id, junk, "exactly a day before: filed");
        assert_eq!(sync_in(&store, acct, "x@normal.example", &at(since - 86_401)).folder_id, inbox(&store, acct), "older: left alone");
        // A date far ahead isn't a server's arrival time: not moved.
        let future = Utc::now().timestamp() + 30 * 86_400;
        assert_eq!(sync_in(&store, acct, "x@normal.example", &at(future)).folder_id, inbox(&store, acct));
        // No arrival time at all (the epoch): not moved.
        assert_eq!(sync_in(&store, acct, "x@normal.example", "1970-01-01T00:00:00Z").folder_id, inbox(&store, acct));
    }

    // ------------------------------------------------------------ pictures and legacy lists (F5)

    #[test]
    fn allow_rules_never_grant_pictures_and_pictures_never_allow() {
        let (store, acct) = store_with_account();
        store.set_sender_rule(Some(acct), "friend@x.example", "", RuleAction::Allow).unwrap();
        assert!(!store.images_allowed_for("friend@x.example").unwrap());
        store.allow_images_from("news@y.example").unwrap();
        assert!(store.images_allowed_for("NEWS@y.example").unwrap());
        assert!(store.sender_decision(acct, "news@y.example").unwrap().is_none());
        assert!(store.label_history(5).unwrap().is_empty(), "no label from a picture permission");
        // A domain permission given now covers its subdomains, as it says.
        store.allow_images_from("pics.example").unwrap();
        assert!(store.images_allowed_for("a@cdn.pics.example").unwrap());
        assert!(!store.images_allowed_for("a@notpics.example").unwrap());
    }

    fn legacy_store(blocked: &[&str], safe: &[&str]) -> Store {
        let store = Store::open_in_memory().unwrap();
        {
            let conn = store.conn();
            conn.execute("DELETE FROM kv WHERE key = ?1", [LEGACY_MIGRATED]).unwrap();
            for b in blocked {
                conn.execute("INSERT INTO mail_blocked_senders (address, name, added_at) VALUES (?1, '', 5)", [b]).unwrap();
            }
            for s in safe {
                conn.execute("INSERT INTO mail_safe_senders (address, added_at) VALUES (?1, 6)", [s]).unwrap();
            }
            migrate_legacy_in(&conn).unwrap();
            migrate_legacy_in(&conn).unwrap();
        }
        store
    }

    #[test]
    fn legacy_lists_migrate_once_as_exact_all_account_rules() {
        let store = legacy_store(&["spam.example", "bad@other.example"], &["friend@ok.example", "safe.example", "not valid"]);
        let acct = second_account(&store);
        let rules = store.sender_rules().unwrap();
        assert_eq!(rules.len(), 4);
        assert!(rules.iter().all(|r| r.account_id.is_none() && r.source == RuleSource::Legacy));
        assert!(rules.iter().any(|r| r.pattern == "spam.example" && r.domain && r.exact_domain && r.action() == RuleAction::Block));
        // As in 0.1.4: the domain itself, not its subdomains or siblings.
        assert!(store.is_sender_blocked(acct, "x@spam.example").unwrap());
        assert!(!store.is_sender_blocked(acct, "x@mail.spam.example").unwrap());
        assert!(!store.is_sender_blocked(acct, "x@notspam.example").unwrap());
        assert!(store.is_sender_blocked(acct, "bad@other.example").unwrap());
        assert!(!store.is_sender_blocked(acct, "good@other.example").unwrap());
        assert!(store.images_allowed_for("friend@ok.example").unwrap(), "old safe senders keep their pictures");
        assert!(store.images_allowed_for("anyone@safe.example").unwrap());
        assert!(!store.images_allowed_for("attacker@tenant.safe.example").unwrap(), "no subdomain gains pictures");
        assert!(!store.images_allowed_for("x@ok.example").unwrap());
        // Blocking by a carried-over rule starts now: old mail isn't swept.
        assert!(rules.iter().all(|r| r.effective_since() >= Utc::now().timestamp() - 60));
        // A new explicit domain rule keeps its advertised reach.
        store.set_sender_rule(Some(acct), "new.example", "", RuleAction::Block).unwrap();
        assert!(store.is_sender_blocked(acct, "x@sub.new.example").unwrap());
        // Re-choosing a carried-over domain doesn't widen it unasked.
        store.set_sender_rule(None, "spam.example", "", RuleAction::Block).unwrap();
        assert!(!store.is_sender_blocked(acct, "x@mail.spam.example").unwrap());
    }

    #[test]
    fn an_earlier_build_is_repaired_without_guessing() {
        // A profile from the first 0.1.5 build: widened legacy rows, no new columns.
        let store = Store::open_in_memory().unwrap();
        {
            let conn = store.conn();
            conn.execute_batch(
                "DROP TABLE junk_sender_rules; DROP TABLE junk_image_senders; DROP TABLE junk_assessments;
                 CREATE TABLE junk_sender_rules (id INTEGER PRIMARY KEY, account_key INTEGER NOT NULL, pattern TEXT NOT NULL,
                    domain INTEGER NOT NULL, action TEXT NOT NULL, name TEXT NOT NULL DEFAULT '', source TEXT NOT NULL,
                    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, UNIQUE (account_key, pattern));
                 CREATE TABLE junk_image_senders (address TEXT PRIMARY KEY, added_at INTEGER NOT NULL);
                 CREATE TABLE junk_assessments (message_id INTEGER PRIMARY KEY, version TEXT NOT NULL, json TEXT NOT NULL, created_at INTEGER NOT NULL);
                 INSERT INTO junk_sender_rules (account_key, pattern, domain, action, source, created_at, updated_at)
                    VALUES (0, 'old.example', 1, 'block', 'legacy', 5, 5), (0, 'mine.example', 1, 'block', 'user', 7, 7);
                 INSERT INTO mail_safe_senders (address, added_at) VALUES ('legacy.example', 1);
                 INSERT INTO junk_image_senders (address, added_at) VALUES ('legacy.example', 1), ('chosen.example', 2);
                 DELETE FROM kv WHERE key = 'junk.schema';",
            )
            .unwrap();
            upgrade_schema_in(&conn).unwrap();
            upgrade_schema_in(&conn).unwrap();
        }
        let acct = second_account(&store);
        assert!(!store.is_sender_blocked(acct, "x@sub.old.example").unwrap(), "carried over: exact again");
        assert!(store.is_sender_blocked(acct, "x@sub.mine.example").unwrap(), "chosen in 0.1.5: as advertised");
        assert!(!store.images_allowed_for("x@sub.legacy.example").unwrap());
        assert!(store.images_allowed_for("x@sub.chosen.example").unwrap());
        let old = store.sender_rules().unwrap().into_iter().find(|r| r.pattern == "old.example").unwrap();
        assert!(old.effective_since() > 5, "doesn't reach back to old mail");
    }

    // ------------------------------------------------------------ assessments (F8, F1)

    fn demo() -> (Store, AccountId) {
        let store = Store::open_in_memory().unwrap();
        crate::seed::seed_demo(&store, chrono::Local::now()).unwrap();
        let acct = store.accounts().unwrap()[0].id;
        (store, acct)
    }

    fn by_subject(store: &Store, acct: AccountId, subject: &str) -> Message {
        store.account_messages(acct).unwrap().into_iter().find(|m| m.subject == subject).unwrap()
    }

    fn drain(store: &Store) {
        while store.assess_pending(200).unwrap() > 0 {}
    }

    #[test]
    fn demo_warning_signs_are_found_and_only_advised() {
        let (store, acct) = demo();
        drain(&store);
        let inbox = inbox(&store, acct);
        for subject in ["Your account is on hold", "Updated brand assets (new link)", "Invoice 4471 overdue"] {
            let m = by_subject(&store, acct, subject);
            let a = store.assessment(m.id).unwrap().unwrap();
            assert_eq!(a.advice, Advice::Phishing, "{subject}: {:?}", a.analysis.findings);
            assert_eq!(store.message(m.id).unwrap().unwrap().folder_id, inbox, "advice never moves mail");
        }
        let lurnen = store.assessment(by_subject(&store, acct, "Updated brand assets (new link)").id).unwrap().unwrap();
        assert!(lurnen.analysis.findings.iter().any(|f| matches!(f, analyze::Finding::LookalikeDomain { resembles, .. } if resembles == "lumen.example")));
        // Ordinary demo mail from contacts stays quiet.
        let ok = store.assessment(by_subject(&store, acct, "Updated brand assets").id).unwrap().unwrap();
        assert_eq!(ok.advice, Advice::None, "{:?}", ok.analysis.findings);
        let marks = store.advice_for(&store.folder_messages(inbox).unwrap().iter().map(|m| m.id).collect::<Vec<_>>()).unwrap();
        assert_eq!(marks.values().filter(|a| **a == Advice::Phishing).count(), 3, "{marks:?}");
        // Your own mail isn't checked.
        let sent = store.folder_for_role(acct, FolderRole::Sent).unwrap();
        let mine = store.folder_messages(sent).unwrap()[0].id;
        assert!(store.assessment(mine).unwrap().is_none());
    }

    #[test]
    fn allow_rules_and_not_junk_never_clear_security_warnings() {
        let (store, acct) = demo();
        let m = by_subject(&store, acct, "Your account is on hold");
        store.set_sender_rule(Some(acct), &m.from.address, "", RuleAction::Allow).unwrap();
        store.label_messages(&[m.id], Label::NotSpam).unwrap();
        store.allow_images_from(&m.from.address).unwrap();
        assert_eq!(store.assessment(m.id).unwrap().unwrap().advice, Advice::Phishing);
    }

    #[test]
    fn junk_suggestions_need_a_score_the_inbox_no_label_and_no_allow_rule() {
        let a = Assessment { score: Some(0.9), ..Default::default() };
        assert_eq!(advise(&a, 0.8, None, false, true), Advice::LooksLikeJunk);
        assert_eq!(advise(&a, 0.95, None, false, true), Advice::None);
        assert_eq!(advise(&a, 0.8, Some(Label::NotSpam), false, true), Advice::None);
        assert_eq!(advise(&a, 0.8, None, true, true), Advice::None);
        assert_eq!(advise(&a, 0.8, None, false, false), Advice::None, "only Inbox mail gets the suggestion");
        assert_eq!(advise(&a, f32::INFINITY, None, false, true), Advice::None, "suggestions off");
        let none = Assessment::default();
        assert_eq!(advise(&none, 0.5, None, false, true), Advice::None, "no model, no suggestion");
    }

    #[test]
    fn a_changed_message_is_checked_again() {
        let (store, acct) = store_with_account();
        let id = msg(&store, acct, "x@normal.example", "hello");
        assert_eq!(store.assessment(id).unwrap().unwrap().advice, Advice::None);
        assert_eq!(store.assess_pending(100).unwrap(), 0, "nothing to do: it is cached");
        // Body.
        store.set_message_body(id, Some("<a href='https://evil.example'>https://paypal.com</a>"), "").unwrap();
        assert_eq!(store.assess_pending(100).unwrap(), 1, "changed content is queued");
        assert_eq!(store.assessment(id).unwrap().unwrap().advice, Advice::Phishing);
        store.set_message_body(id, None, "plain again").unwrap();
        assert_eq!(store.assessment(id).unwrap().unwrap().advice, Advice::None);
        // Sender and authentication header.
        let mut m = store.message(id).unwrap().unwrap();
        m.from.name = "billing@paypal.com".into();
        store.update_message(&m).unwrap();
        assert_eq!(store.assessment(id).unwrap().unwrap().advice, Advice::Phishing, "display name shows another address");
        m.from.name = String::new();
        store.update_message(&m).unwrap();
        assert_eq!(store.assessment(id).unwrap().unwrap().advice, Advice::None);
        // Attachments.
        let mut a = Attachment { message_id: id, name: "invoice.pdf.exe".into(), ..Default::default() };
        Store::insert_doc_in(&store.conn(), &mut a).unwrap();
        assert_eq!(store.assessment(id).unwrap().unwrap().advice, Advice::Phishing);
        // Read state alone doesn't throw the check away.
        let mut m = store.message(id).unwrap().unwrap();
        m.is_read = !m.is_read;
        store.update_message(&m).unwrap();
        assert_eq!(store.assess_pending(100).unwrap(), 0);
    }

    #[test]
    fn contact_changes_refresh_impersonation_warnings() {
        let (store, acct) = store_with_account();
        let id = msg(&store, acct, "stranger@normal.example", "meeting");
        let mut m = store.message(id).unwrap().unwrap();
        m.from.name = "Jane Smith".into();
        store.update_message(&m).unwrap();
        assert_eq!(store.assessment(id).unwrap().unwrap().advice, Advice::None);
        let mut c = Contact {
            account_id: acct,
            given_name: "Jane".into(),
            surname: "Smith".into(),
            emails: vec![LabeledValue { label: "Work".into(), value: "jane@workplace.example".into() }],
            ..Default::default()
        };
        store.insert_contact(&mut c).unwrap();
        assert_eq!(store.assessment(id).unwrap().unwrap().advice, Advice::Phishing, "the new contact's name is now in use elsewhere");
        assert!(store.advice_for(&[id]).unwrap().is_empty() || store.advice_for(&[id]).unwrap()[&id] == Advice::Phishing);
        // The contact's address corrected to this one: no longer elsewhere.
        c.emails[0].value = "stranger@normal.example".into();
        store.update_contact(&c).unwrap();
        assert_eq!(store.assessment(id).unwrap().unwrap().advice, Advice::None);
        // Removed again.
        store.delete_contact(c.id).unwrap();
        assert_eq!(store.assessment(id).unwrap().unwrap().advice, Advice::None);
    }

    #[test]
    fn a_malformed_message_does_not_stop_the_others() {
        let (store, acct) = store_with_account();
        let a = msg(&store, acct, "a@normal.example", "first");
        let bad = msg(&store, acct, "b@normal.example", "second");
        store.set_message_body(bad, Some(&format!("<a href='https://normal.example/'>{}", "é".repeat(2000))), "").unwrap();
        let c = msg(&store, acct, "c@normal.example", "third");
        drain(&store);
        for id in [a, bad, c] {
            let x = store.assessment(id).unwrap().unwrap();
            assert!(!x.analysis.incomplete, "{id}");
        }
        assert_eq!(store.assessment(bad).unwrap().unwrap().analysis.links, 1, "opening it doesn't panic");
    }

    // ------------------------------------------------------------ learning

    /// Varied mail with words of its own (so a model learnt from it says
    /// nothing about the shipped checks): each message is a different
    /// campaign.
    fn learn_mail(store: &Store, acct: AccountId, n: usize) -> (Vec<MessageId>, Vec<MessageId>) {
        learn_mail_from(store, acct, n, 0)
    }

    fn learn_mail_from(store: &Store, acct: AccountId, n: usize, start: usize) -> (Vec<MessageId>, Vec<MessageId>) {
        let inbox = inbox(store, acct);
        let add = |from: &str, subject: String, text: String| {
            let at = chrono::DateTime::from_timestamp(1_790_000_000, 0).unwrap();
            let mut m = Message { account_id: acct, folder_id: inbox, from: EmailAddress::new("", from), subject, received_at: at, ..Default::default() };
            store.insert_message(&mut m, Some(&MessageBody { text, ..Default::default() })).unwrap()
        };
        let tag = |i: usize| format!("q{}", (b'a' + (i % 26) as u8) as char).repeat(1 + i / 26);
        let spam = (start..start + n).map(|i| add("x@zqxoffers.example", format!("zqxwin zqxbonus zqx{}", tag(i)), format!("zqxclaim zqxprize zqxnow zqx{}", tag(i)))).collect();
        let ham = (start..start + n).map(|i| add("y@kvworkplace.example", format!("kvagenda kvsync kv{}", tag(i)), format!("kvnotes kvreview kvteam kv{}", tag(i)))).collect();
        (spam, ham)
    }

    fn trained(store: &Store, acct: AccountId) -> (Vec<i64>, Arc<Model>) {
        let (spam, ham) = learn_mail(store, acct, 30);
        let events = store.label_messages(&spam, Label::Spam).unwrap();
        store.label_messages(&ham, Label::NotSpam).unwrap();
        assert!(store.retrain_if_needed().unwrap(), "{}", store.learn_status().unwrap().last);
        (events, store.active_model().unwrap().expect("promoted"))
    }

    fn trained_event(store: &Store, events: &[i64], model: &Model) -> i64 {
        events
            .iter()
            .copied()
            .find(|e| {
                let fp: String = store.conn().query_row("SELECT fingerprint FROM junk_label_events WHERE id = ?1", [e], |r| r.get(0)).unwrap();
                model.manifest.trained_on.iter().any(|t| t.starts_with(&fp))
            })
            .unwrap()
    }

    fn model_status(store: &Store, digest: &str) -> Option<String> {
        store.conn().query_row("SELECT status FROM junk_models WHERE digest = ?1", [digest], |r| r.get(0)).optional().unwrap()
    }

    #[test]
    fn too_few_reports_keep_rules_only() {
        let (store, acct) = store_with_account();
        let (spam, ham) = learn_mail(&store, acct, 3);
        store.label_messages(&spam, Label::Spam).unwrap();
        store.label_messages(&ham, Label::NotSpam).unwrap();
        assert!(!store.retrain_if_needed().unwrap());
        assert!(store.active_model().unwrap().is_none());
        assert!(store.learn_status().unwrap().last.starts_with("Not enough different confirmed examples"));
        assert!(!store.retrain_if_needed().unwrap(), "nothing new, nothing done");
    }

    #[test]
    fn copies_of_one_campaign_count_once_and_never_straddle_the_split() {
        let (store, acct) = store_with_account();
        // Thirty copies of one junk mail and one wanted mail, differing only by a counter.
        let inbox = inbox(&store, acct);
        let mut ids = (Vec::new(), Vec::new());
        for i in 0..30 {
            let at = chrono::DateTime::from_timestamp(1_790_000_000 + i, 0).unwrap();
            let mut s = Message { account_id: acct, folder_id: inbox, from: EmailAddress::new("", "x@zqxoffers.example"), subject: format!("zqxwin zqxbonus {i}"), received_at: at, ..Default::default() };
            ids.0.push(store.insert_message(&mut s, Some(&MessageBody { text: format!("zqxclaim zqxprize zqxnow {i}"), ..Default::default() })).unwrap());
            let mut h = Message { account_id: acct, folder_id: inbox, from: EmailAddress::new("", "y@kvworkplace.example"), subject: format!("kvagenda kvsync {i}"), received_at: at, ..Default::default() };
            ids.1.push(store.insert_message(&mut h, Some(&MessageBody { text: format!("kvnotes kvreview kvteam {i}"), ..Default::default() })).unwrap());
        }
        store.label_messages(&ids.0, Label::Spam).unwrap();
        store.label_messages(&ids.1, Label::NotSpam).unwrap();
        let mut train = HashSet::new();
        let mut held = HashSet::new();
        for l in store.effective_labels().unwrap() {
            let f = junk::learn::features_from_json(&l.features.unwrap()).unwrap();
            let key = (l.label, f.clone());
            if junk::held_back(&junk::campaign_key(&f)) { held.insert(key) } else { train.insert(key) };
        }
        assert_eq!(held.intersection(&train).count(), 0, "a copy trained on while another checks the result");
        // Sixty reports, two independent examples: not enough to learn from.
        assert!(!store.retrain_if_needed().unwrap());
        assert!(store.active_model().unwrap().is_none());
        assert!(store.learn_status().unwrap().last.contains("copies of one message count once"), "{}", store.learn_status().unwrap().last);
    }

    #[test]
    fn a_checked_model_is_used_until_a_report_it_learnt_from_is_undone() {
        let (store, acct) = store_with_account();
        let (spam_events, model) = trained(&store, acct);
        assert_eq!(model.manifest.kind, "personal");
        assert_eq!(model.manifest.gate, Some(junk::learn::Verdict::Promote));
        assert!(model.manifest.examples_unwanted >= MIN_PER_CLASS);
        // New mail like the reported junk is suggested as junk, and only that.
        let (more_spam, more_ham) = learn_mail_from(&store, acct, 1, 200);
        drain(&store);
        assert_eq!(store.assessment(more_spam[0]).unwrap().unwrap().advice, Advice::LooksLikeJunk);
        assert_eq!(store.assessment(more_ham[0]).unwrap().unwrap().advice, Advice::None);
        // Turning suggestions off hides them; warning signs are unaffected.
        let mut settings = store.settings();
        settings.junk_suggestions = false;
        store.save_settings(&settings).unwrap();
        assert_eq!(store.assessment(more_spam[0]).unwrap().unwrap().advice, Advice::None);
        settings.junk_suggestions = true;
        store.save_settings(&settings).unwrap();

        // Undoing a report the model learnt from takes it out of use at
        // once: before any retraining, and after a restart.
        store.undo_label_events(&[trained_event(&store, &spam_events, &model)]).unwrap();
        assert!(store.active_model().unwrap().is_none_or(|m| m.digest != model.digest), "affected model still active after Undo");
        assert_eq!(store.assessment(more_spam[0]).unwrap().unwrap().advice, Advice::None, "no suggestion from it any more");
        assert_eq!(model_status(&store, &model.digest).as_deref(), Some("affected"));
        store.retrain_if_needed().unwrap();
        assert!(store.active_model().unwrap().is_none_or(|m| m.digest != model.digest));
    }

    #[test]
    fn a_relabel_stops_the_model_at_once() {
        let (store, acct) = store_with_account();
        let (spam_events, model) = trained(&store, acct);
        let event = trained_event(&store, &spam_events, &model);
        let id: MessageId = store.conn().query_row("SELECT message_id FROM junk_label_events WHERE id = ?1", [event], |r| r.get(0)).unwrap();
        store.label_messages(&[id], Label::NotSpam).unwrap();
        assert!(store.active_model().unwrap().is_none_or(|m| m.digest != model.digest));
    }

    #[test]
    fn roll_back_never_returns_to_a_model_with_withdrawn_reports() {
        let (store, acct) = store_with_account();
        let (spam_events, first) = trained(&store, acct);
        // A second model on top of the first.
        let (s2, h2) = learn_mail_from(&store, acct, 10, 100);
        store.label_messages(&s2, Label::Spam).unwrap();
        store.label_messages(&h2, Label::NotSpam).unwrap();
        store.retrain_if_needed().unwrap();
        let second = store.active_model().unwrap().unwrap();
        assert_ne!(second.digest, first.digest, "{}", store.learn_status().unwrap().last);
        assert_eq!(model_status(&store, &first.digest).as_deref(), Some("retired"));
        // Withdraw a report only the first model learnt from... both did:
        // after that neither may be used, and Roll back lands on rules only.
        store.undo_label_events(&[trained_event(&store, &spam_events, &first)]).unwrap();
        store.roll_back_model().unwrap();
        assert!(store.active_model().unwrap().is_none());
        assert_eq!(model_status(&store, &first.digest).as_deref(), Some("affected"));
    }

    #[test]
    fn roll_back_and_forget() {
        let (store, acct) = store_with_account();
        trained(&store, acct);
        store.set_sender_rule(Some(acct), "x@zqxoffers.example", "", RuleAction::Block).unwrap();
        store.roll_back_model().unwrap();
        assert!(store.active_model().unwrap().is_none(), "back to rules only");
        assert_eq!(store.learn_status().unwrap().unwanted, 30, "reports stay");
        assert!(!store.retrain_if_needed().unwrap(), "not retrained straight back");
        store.forget_learning().unwrap();
        assert!(store.effective_labels().unwrap().is_empty());
        assert!(store.label_history(5).unwrap().is_empty());
        assert!(store.active_model().unwrap().is_none());
        let n: i64 = store.conn().query_row("SELECT COUNT(*) FROM junk_models", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0);
        let scores: i64 = store.conn().query_row("SELECT COUNT(*) FROM junk_assessments WHERE score IS NOT NULL", [], |r| r.get(0)).unwrap();
        assert_eq!(scores, 0);
        assert_eq!(store.sender_rules().unwrap().len(), 1, "rules stay");
    }

    #[test]
    fn removing_an_account_removes_its_rules_labels_and_models() {
        let (store, acct) = store_with_account();
        let other = second_account(&store);
        let keep = msg(&store, other, "k@kept.example", "Kept");
        store.label_messages(&[keep], Label::Spam).unwrap();
        let (_, model) = trained(&store, acct);
        let id = msg(&store, acct, "a@x.example", "Bye");
        store.label_messages(&[id], Label::Spam).unwrap();
        store.set_sender_rule(Some(acct), "a@x.example", "", RuleAction::Block).unwrap();
        store.set_sender_rule(None, "y.example", "", RuleAction::Block).unwrap();
        store.remove_account(acct).unwrap();
        assert_eq!(store.sender_rules().unwrap().len(), 1, "all-accounts rules stay");
        assert_eq!(store.effective_labels().unwrap().len(), 1, "the other account's report stays");
        assert_eq!(store.label_history(100).unwrap().len(), 1);
        // Its model is gone at once, not at the next training run.
        assert!(store.active_model().unwrap().is_none(), "removed account's trained model still usable");
        assert_eq!(model_status(&store, &model.digest), None, "nothing derived from it is kept");
        let left: i64 = store.conn().query_row("SELECT COUNT(*) FROM junk_models", [], |r| r.get(0)).unwrap();
        assert_eq!(left, 0);
    }

    /// Run a training job on another thread and, when it reaches `step`,
    /// do `meddle` before letting it go on.
    fn interrupt_training(store: &Arc<Store>, step: &'static str, meddle: impl FnOnce(&Store)) {
        let (reached_tx, reached_rx) = std::sync::mpsc::channel::<()>();
        let (go_tx, go_rx) = std::sync::mpsc::channel::<()>();
        let go_rx = std::sync::Mutex::new(go_rx);
        let reached_tx = std::sync::Mutex::new(reached_tx);
        *store.learn_hook.lock().unwrap() = Some(Arc::new(move |s: &str| {
            if s == step {
                reached_tx.lock().unwrap().send(()).unwrap();
                go_rx.lock().unwrap().recv().unwrap();
            }
        }));
        let worker = store.clone();
        let job = std::thread::spawn(move || worker.retrain_if_needed().unwrap());
        reached_rx.recv_timeout(std::time::Duration::from_secs(30)).expect("training reached the step");
        meddle(store);
        go_tx.send(()).unwrap();
        job.join().unwrap();
        *store.learn_hook.lock().unwrap() = None;
    }

    fn labelled_store() -> (Arc<Store>, AccountId, Vec<i64>) {
        let (store, acct) = store_with_account();
        let (spam, ham) = learn_mail(&store, acct, 40);
        let events = store.label_messages(&spam, Label::Spam).unwrap();
        store.label_messages(&ham, Label::NotSpam).unwrap();
        (Arc::new(store), acct, events)
    }

    #[test]
    fn forget_during_training_never_brings_learning_back() {
        for step in ["snapshot", "trained", "evaluated", "commit"] {
            let (store, _, _) = labelled_store();
            interrupt_training(&store, step, |s| s.forget_learning().unwrap());
            assert!(store.effective_labels().unwrap().is_empty(), "{step}");
            assert!(store.label_history(10).unwrap().is_empty(), "{step}");
            assert!(store.active_model().unwrap().is_none(), "{step}: background training recreated a model after Forget");
            let n: i64 = store.conn().query_row("SELECT COUNT(*) FROM junk_models", [], |r| r.get(0)).unwrap();
            assert_eq!(n, 0, "{step}: a candidate was saved after Forget");
            // And nothing comes back on the next pass either.
            assert!(!store.retrain_if_needed().unwrap());
            assert!(store.active_model().unwrap().is_none());
        }
    }

    #[test]
    fn other_changes_during_training_discard_the_stale_result() {
        // Undo of a report, a relabel, removing the account, a rollback:
        // the run that started before is thrown away, and the next run
        // starts from what is true now.
        type Meddle = Box<dyn Fn(&Store, AccountId, &[i64])>;
        let cases: Vec<(&str, Meddle)> = vec![
            ("undo", Box::new(|s: &Store, _, ev: &[i64]| s.undo_label_events(&ev[..1]).unwrap())),
            ("relabel", Box::new(|s: &Store, _, ev: &[i64]| {
                let id: MessageId = s.conn().query_row("SELECT message_id FROM junk_label_events WHERE id = ?1", [ev[0]], |r| r.get(0)).unwrap();
                s.label_messages(&[id], Label::NotSpam).unwrap();
            })),
            ("remove account", Box::new(|s: &Store, a, _: &[i64]| s.remove_account(a).unwrap())),
            ("roll back", Box::new(|s: &Store, _, _: &[i64]| {
                s.conn().execute("INSERT OR REPLACE INTO kv (key, value) VALUES (?1, 'off')", [ACTIVE_MODEL]).unwrap();
            })),
        ];
        for (name, meddle) in cases {
            let (store, acct, events) = labelled_store();
            let rev = store.label_revision().unwrap();
            interrupt_training(&store, "evaluated", |s| meddle(s, acct, &events));
            assert!(store.active_model().unwrap().is_none(), "{name}: stale result made active");
            let trained: Option<String> = store.kv_get(TRAINED_REVISION).unwrap();
            assert_ne!(trained, Some(rev.to_string()), "{name}: the stale run marked its revision done");
            if name != "remove account" && name != "roll back" {
                // The next pass trains on the current labels.
                store.retrain_if_needed().unwrap();
                if let Some(m) = store.active_model().unwrap() {
                    let current: HashSet<String> = store.effective_labels().unwrap().iter().map(|l| format!("{}:{}", l.fingerprint, l.label.as_str())).collect();
                    assert!(m.manifest.trained_on.iter().all(|t| current.contains(t)), "{name}");
                }
            }
        }
    }

    #[test]
    fn a_promotion_survives_a_restart_whole() {
        let dir = std::env::temp_dir().join(format!("axentra-junk-{}-{}", std::process::id(), Utc::now().timestamp_nanos_opt().unwrap_or(0)));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.db");
        let digest = {
            let store = Store::open(&path).unwrap();
            let mut a = Account { kind: AccountKind::Local, email: "t@axentra.example".into(), ..Default::default() };
            let acct = store.add_account(&mut a).unwrap();
            trained(&store, acct).1.digest.clone()
        };
        let store = Store::open(&path).unwrap();
        assert_eq!(store.active_model().unwrap().map(|m| m.digest.clone()), Some(digest.clone()));
        assert_eq!(model_status(&store, &digest).as_deref(), Some("active"));
        assert!(!store.retrain_if_needed().unwrap(), "the finished revision was recorded with it");
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ------------------------------------------------------------ compound operations (F10)

    fn fail_on(store: &Store, table: &str, op: &str) {
        store
            .conn()
            .execute_batch(&format!("CREATE TEMP TRIGGER fault BEFORE {op} ON main.{table} BEGIN SELECT RAISE(ABORT, 'injected fault'); END;"))
            .unwrap();
    }

    fn clear_fault(store: &Store) {
        store.conn().execute_batch("DROP TRIGGER IF EXISTS temp.fault;").unwrap();
    }

    fn snapshot(store: &Store, ids: &[MessageId]) -> (Vec<FolderId>, usize, usize, usize, i64) {
        let folders = ids.iter().map(|id| store.message(*id).unwrap().unwrap().folder_id).collect();
        let rev = store.label_revision().unwrap();
        (folders, store.effective_labels().unwrap().len(), store.label_history(100).unwrap().len(), store.sender_rules().unwrap().len(), rev)
    }

    #[test]
    fn a_failed_report_leaves_nothing_behind() {
        let (store, acct) = store_with_account();
        let other = second_account(&store);
        let ids = vec![msg(&store, acct, "a@x.example", "one"), msg(&store, other, "b@y.example", "two")];
        let before = snapshot(&store, &ids);
        for (table, op) in [("junk_label_events", "INSERT"), ("junk_labels", "INSERT"), ("junk_sender_rules", "INSERT"), ("messages", "UPDATE")] {
            fail_on(&store, table, op);
            assert!(store.report_junk(&ids, Label::Phishing, true).is_err(), "{table}");
            clear_fault(&store);
            assert_eq!(snapshot(&store, &ids), before, "{table} {op}: partly applied");
        }
        // Without faults: all of it, across two accounts, undone in one go.
        let op = store.report_junk(&ids, Label::Phishing, true).unwrap();
        assert_eq!((op.events.len(), op.rules.len(), op.moves.len()), (2, 2, 2));
        assert_eq!(store.message_label(ids[1]).unwrap(), Some(Label::Phishing));
        assert!(store.is_sender_blocked(other, "b@y.example").unwrap() && !store.is_sender_blocked(acct, "b@y.example").unwrap());
        fail_on(&store, "junk_label_events", "UPDATE");
        assert!(store.undo_junk_operation(&op).is_err());
        clear_fault(&store);
        assert!(store.message(ids[0]).unwrap().unwrap().folder_id == store.folder_for_role(acct, FolderRole::Junk).unwrap(), "a failed Undo changed nothing");
        store.undo_junk_operation(&op).unwrap();
        store.undo_junk_operation(&op).unwrap();
        let after = snapshot(&store, &ids);
        assert_eq!((after.0, after.1, after.3), (before.0.clone(), before.1, before.3), "repeated Undo is harmless");
    }

    #[test]
    fn undoing_a_report_keeps_later_rule_edits_and_goes_back_to_the_folder() {
        let (store, acct) = store_with_account();
        let archive = store.folders(Some(acct)).unwrap().into_iter().find(|f| f.role == FolderRole::Archive).map(|f| f.id);
        let id = msg(&store, acct, "a@x.example", "one");
        if let Some(a) = archive {
            store.move_messages(&[id], a).unwrap();
        }
        let from = store.message(id).unwrap().unwrap().folder_id;
        let op = store.report_junk(&[id], Label::Spam, true).unwrap();
        // Later, the user changes their mind about the sender in Settings.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        store.set_sender_rule(Some(acct), "a@x.example", "", RuleAction::Allow).unwrap();
        store.undo_junk_operation(&op).unwrap();
        assert_eq!(store.message(id).unwrap().unwrap().folder_id, from);
        assert_eq!(store.message_label(id).unwrap(), None);
        assert_eq!(store.sender_decision(acct, "a@x.example").unwrap().unwrap().action(), RuleAction::Allow, "later edit kept");
    }

    #[test]
    fn not_junk_is_one_change_too() {
        let (store, acct) = store_with_account();
        let id = msg(&store, acct, "a@x.example", "one");
        store.report_junk(&[id], Label::Spam, false).unwrap();
        fail_on(&store, "messages", "UPDATE");
        assert!(store.mark_not_junk(&[id]).is_err());
        clear_fault(&store);
        assert_eq!(store.message_label(id).unwrap(), Some(Label::Spam), "label not changed by the failed attempt");
        let op = store.mark_not_junk(&[id]).unwrap();
        assert_eq!(store.message(id).unwrap().unwrap().folder_id, inbox(&store, acct));
        assert_eq!(store.message_label(id).unwrap(), Some(Label::NotSpam));
        store.undo_junk_operation(&op).unwrap();
        assert_eq!(store.message_label(id).unwrap(), Some(Label::Spam));
        assert_eq!(store.message(id).unwrap().unwrap().folder_id, store.folder_for_role(acct, FolderRole::Junk).unwrap());
    }

    #[test]
    fn a_report_survives_a_restart_whole() {
        let dir = std::env::temp_dir().join(format!("axentra-junk-op-{}-{}", std::process::id(), Utc::now().timestamp_nanos_opt().unwrap_or(0)));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.db");
        let (acct, id) = {
            let store = Store::open(&path).unwrap();
            let mut a = Account { kind: AccountKind::Local, email: "t@axentra.example".into(), ..Default::default() };
            let acct = store.add_account(&mut a).unwrap();
            let id = msg(&store, acct, "a@x.example", "one");
            store.report_junk(&[id], Label::Phishing, true).unwrap();
            (acct, id)
        };
        let store = Store::open(&path).unwrap();
        assert_eq!(store.message_label(id).unwrap(), Some(Label::Phishing));
        assert!(store.is_sender_blocked(acct, "a@x.example").unwrap());
        assert_eq!(store.message(id).unwrap().unwrap().folder_id, store.folder_for_role(acct, FolderRole::Junk).unwrap());
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
