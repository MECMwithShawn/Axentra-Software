//! Local junk-mail handling: sender rules, confirmed labels, and (in
//! [`analyze`] and [`learn`]) advisory checks that never move mail on their
//! own. See `docs/JUNK.md` for the data flow and rule precedence.
//!
//! Everything here runs on this computer. Nothing in this module talks to
//! the network.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::model::{AccountId, Message, MessageId};

pub mod analyze;
pub mod learn;

static BASE_MODEL_BYTES: &[u8] = include_bytes!("base_model.bin");
static GATE_FIXTURES_BYTES: &[u8] = include_bytes!("gate_fixtures.bin");

/// The fixed checks every model update must pass (features only).
pub fn gate_fixtures() -> &'static [learn::Fixture] {
    static F: std::sync::OnceLock<Vec<learn::Fixture>> = std::sync::OnceLock::new();
    F.get_or_init(|| learn::fixtures_from_bytes(GATE_FIXTURES_BYTES))
}

/// The model shipped with Axentra (trained on public data, see
/// `docs/JUNK.md`), if this build has one that loads.
pub fn base_model() -> Option<&'static learn::Model> {
    static BASE: std::sync::OnceLock<Option<learn::Model>> = std::sync::OnceLock::new();
    BASE.get_or_init(|| learn::Model::from_bytes(BASE_MODEL_BYTES)).as_ref()
}

/// A label the user confirmed with Report junk, Report phishing or Not junk.
/// Nothing else (a restore, a model's advice, opening a message, allowing
/// pictures) creates one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Label {
    Spam,
    NotSpam,
    Phishing,
}

impl Label {
    pub fn as_str(self) -> &'static str {
        match self {
            Label::Spam => "spam",
            Label::NotSpam => "not_spam",
            Label::Phishing => "phishing",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "spam" => Some(Label::Spam),
            "not_spam" => Some(Label::NotSpam),
            "phishing" => Some(Label::Phishing),
            _ => None,
        }
    }

    /// Unwanted mail for the learner (spam and phishing both count).
    pub fn is_unwanted(self) -> bool {
        !matches!(self, Label::NotSpam)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RuleAction {
    Allow,
    Block,
}

impl RuleAction {
    pub fn as_str(self) -> &'static str {
        match self {
            RuleAction::Allow => "allow",
            RuleAction::Block => "block",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "allow" => Some(RuleAction::Allow),
            "block" => Some(RuleAction::Block),
            _ => None,
        }
    }
}

/// Where a rule came from, for the settings list and the audit trail.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RuleSource {
    /// Added on purpose (Block, Never block sender, Settings).
    #[default]
    User,
    /// Moved over from the blocked / safe sender lists of 0.1.4 and earlier,
    /// which applied to every account.
    Legacy,
}

impl RuleSource {
    pub fn as_str(self) -> &'static str {
        match self {
            RuleSource::User => "user",
            RuleSource::Legacy => "legacy",
        }
    }

    pub fn parse(s: &str) -> Self {
        if s == "legacy" { RuleSource::Legacy } else { RuleSource::User }
    }
}

/// One allow / block rule. `account_id` `None` means every account (only
/// rules carried over from older versions, or ones the user widens on
/// purpose). `pattern` is a full address or, when `domain` is set, a domain
/// that also covers its subdomains, unless `exact_domain` is set (lists
/// carried over from 0.1.4, which matched only the domain itself).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SenderRule {
    pub id: i64,
    pub account_id: Option<AccountId>,
    pub pattern: String,
    pub domain: bool,
    pub action: Option<RuleAction>,
    pub name: String,
    pub source: RuleSource,
    pub created_at: i64,
    pub updated_at: i64,
    /// A domain rule that doesn't cover subdomains.
    pub exact_domain: bool,
    /// When the current action took effect (Unix seconds); a block rule only
    /// files mail that arrived after this. 0 for records written before this
    /// was kept: then the last change counts.
    pub effective_at: i64,
}

impl SenderRule {
    pub fn action(&self) -> RuleAction {
        self.action.unwrap_or(RuleAction::Block)
    }

    /// When the current action took effect, conservatively.
    pub fn effective_since(&self) -> i64 {
        if self.effective_at > 0 { self.effective_at } else { self.created_at.max(self.updated_at) }
    }

    /// True when the rule's pattern covers `domain`.
    pub fn covers_domain(&self, domain: &str) -> bool {
        self.domain && if self.exact_domain { domain == self.pattern } else { domain_matches(domain, &self.pattern) }
    }
}

/// A rule pattern as typed or taken from a message: an address or a domain,
/// lower-cased and trimmed (also of a leading `@` or `*.`). Plus-addresses
/// and dots are kept: `a+x@d` and `a@d` are different mailboxes.
pub fn normalize_pattern(input: &str) -> Option<(String, bool)> {
    let s = input.trim().trim_matches(|c| c == '<' || c == '>').to_lowercase();
    let s = s.strip_prefix("*.").or_else(|| s.strip_prefix('@')).unwrap_or(&s).to_string();
    if s.is_empty() || s.chars().any(|c| c.is_whitespace() || c == ',' || c == ';') {
        return None;
    }
    match s.split_once('@') {
        Some((local, domain)) => (!local.is_empty() && valid_domain(domain)).then_some((s.clone(), false)),
        None => valid_domain(&s).then_some((s.clone(), true)),
    }
}

fn valid_domain(d: &str) -> bool {
    d.contains('.') && !d.starts_with('.') && !d.ends_with('.') && !d.contains("..") && !d.contains('@')
}

/// True when `domain` is `rule` or a subdomain of it (label boundary, so
/// `evilexample.com` does not match `example.com`).
pub fn domain_matches(domain: &str, rule: &str) -> bool {
    domain == rule || domain.strip_suffix(rule).is_some_and(|head| head.ends_with('.'))
}

pub fn domain_of(address: &str) -> &str {
    address.rsplit_once('@').map(|(_, d)| d).unwrap_or("")
}

/// The rule that decides for `address` in `account`, by the documented
/// order: an exact address before any domain; for domains the most specific
/// one wins; at the same level an account's own rule beats an all-accounts
/// rule. `None` when no rule applies.
pub fn decide<'a>(rules: &'a [SenderRule], account: AccountId, address: &str) -> Option<&'a SenderRule> {
    let address = address.trim().to_lowercase();
    if address.is_empty() {
        return None;
    }
    let domain = domain_of(&address).to_string();
    let applies = |r: &&SenderRule| r.account_id.is_none_or(|a| a == account);
    let mine = |r: &SenderRule| u8::from(r.account_id.is_some());
    let exact = rules.iter().filter(applies).filter(|r| !r.domain && r.pattern == address).max_by_key(|r| mine(r));
    if exact.is_some() {
        return exact;
    }
    rules
        .iter()
        .filter(applies)
        .filter(|r| !domain.is_empty() && r.covers_domain(&domain))
        .max_by_key(|r| (r.pattern.len(), mine(r)))
}

/// Content fingerprint for a confirmed label: survives moves and a resync
/// (new local / remote ids), and does not rely only on the Message-ID header,
/// which the sender controls.
/// `body` is the plain text, or the HTML when there is no plain text (so
/// HTML-only mail isn't identified by its headers alone). Only a bounded
/// prefix of each part is read.
pub fn fingerprint(m: &Message, body: &str) -> String {
    let mut h = Sha256::new();
    h.update(m.account_id.to_le_bytes());
    h.update(prefix(m.from.address.trim(), 512).to_lowercase().as_bytes());
    h.update([0]);
    h.update(prefix(m.subject.trim(), 2048).as_bytes());
    h.update([0]);
    h.update(m.sent_at.unwrap_or(m.received_at).timestamp().to_le_bytes());
    let mut words = prefix(body, 64 * 1024).split_whitespace();
    let mut n = 0;
    if let Some(w) = words.next() {
        h.update(w.as_bytes());
        n += w.chars().count();
        for w in words {
            if n >= 4096 {
                break;
            }
            h.update(b" ");
            h.update(w.as_bytes());
            n += 1 + w.chars().count();
        }
    }
    h.finalize().iter().take(16).map(|b| format!("{b:02x}")).collect()
}

/// The fingerprint of a stored message.
pub fn fingerprint_of(m: &Message, body: Option<&crate::model::MessageBody>) -> String {
    let text = body.map(|b| if b.text.trim().is_empty() { b.html.as_deref().unwrap_or("") } else { b.text.as_str() }).unwrap_or("");
    fingerprint(m, text)
}

/// The first `max` bytes of `s`, cut on a character boundary.
fn prefix(s: &str, max: usize) -> &str {
    let mut cut = s.len().min(max);
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    &s[..cut]
}

/// The group a confirmed label belongs to for the train / held-back split:
/// reports whose features are the same (a campaign re-sent with a new
/// counter or date, the same mail in two accounts) are one group, so a copy
/// can't be trained on while another copy checks the result.
pub fn campaign_key(features: &learn::Features) -> String {
    let mut h = Sha256::new();
    h.update(learn::FEATURE_VERSION.to_le_bytes());
    for f in features {
        h.update(f.to_le_bytes());
    }
    h.finalize().iter().take(12).map(|b| format!("{b:02x}")).collect()
}

/// True when the reports of this group (see [`campaign_key`]) are held back
/// to check personal models: about one group in five, never trained on.
pub fn held_back(group: &str) -> bool {
    group.bytes().fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b))) % 5 == 0
}

/// One confirmed-label change, as kept in the history.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelEvent {
    pub id: i64,
    pub account_id: AccountId,
    pub message_id: MessageId,
    pub fingerprint: String,
    pub label: Label,
    /// The event this one replaced (Undo goes back to it).
    pub previous: Option<i64>,
    pub created_at: i64,
    pub undone: bool,
    /// Subject at the time, for the history list.
    pub subject: String,
    pub sender: String,
}

/// What the reading pane and list suggest about a message. Advice only:
/// nothing here moves, deletes or reports mail.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Advice {
    #[default]
    None,
    /// Warning signs worth a second look.
    Caution,
    /// Signs typical of phishing.
    Phishing,
    /// The learned model thinks it's junk (stays where it is).
    LooksLikeJunk,
}

/// The cached result for one message: warning signs plus (stage C) the
/// learned model's score, and the advice drawn from both.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Assessment {
    pub analysis: analyze::Analysis,
    /// Unwanted-mail score from the active model, 0..1. Not a calibrated
    /// probability. `None` when no model is active.
    pub score: Option<f32>,
    /// Digest of the model that scored it.
    pub model: Option<String>,
    pub advice: Advice,
}

/// A confirmed label as the learner sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectiveLabel {
    pub account_id: AccountId,
    pub fingerprint: String,
    pub label: Label,
    /// [`learn::features_json`] at the time of the report.
    pub features: Option<String>,
}

/// A rule change made as part of a report, so Undo can put it back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleChange {
    /// The rule that was added or changed.
    pub rule_id: i64,
    /// What was there before for the same account and pattern.
    pub previous: Option<SenderRule>,
    /// The rule's `updated_at` as this change wrote it: Undo leaves a rule
    /// that was changed again since alone.
    pub applied_at: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(account: Option<AccountId>, pattern: &str, action: RuleAction) -> SenderRule {
        let (pattern, domain) = normalize_pattern(pattern).unwrap();
        SenderRule { account_id: account, pattern, domain, action: Some(action), ..Default::default() }
    }

    #[test]
    fn patterns_normalise_but_keep_distinct_mailboxes() {
        assert_eq!(normalize_pattern(" Bob@Example.COM "), Some(("bob@example.com".into(), false)));
        assert_eq!(normalize_pattern("@example.com"), Some(("example.com".into(), true)));
        assert_eq!(normalize_pattern("*.example.com"), Some(("example.com".into(), true)));
        assert_eq!(normalize_pattern("a+news@example.com").unwrap().0, "a+news@example.com");
        assert_eq!(normalize_pattern("a.b@example.com").unwrap().0, "a.b@example.com");
        for bad in ["", "bob", "bob@", "@", "a b@x.com", "x..com", "a@b@c.com"] {
            assert_eq!(normalize_pattern(bad), None, "{bad}");
        }
    }

    #[test]
    fn domains_match_on_label_boundaries() {
        assert!(domain_matches("example.com", "example.com"));
        assert!(domain_matches("mail.example.com", "example.com"));
        assert!(!domain_matches("evilexample.com", "example.com"));
        assert!(!domain_matches("example.com.evil.net", "example.com"));
    }

    #[test]
    fn exact_address_beats_domain_and_account_beats_global() {
        let rules = vec![
            rule(None, "spam.example", RuleAction::Block),
            rule(Some(1), "friend@spam.example", RuleAction::Allow),
            rule(None, "x@other.example", RuleAction::Allow),
            rule(Some(1), "x@other.example", RuleAction::Block),
            rule(Some(2), "sub.spam.example", RuleAction::Allow),
        ];
        assert_eq!(decide(&rules, 1, "Friend@spam.example").unwrap().action(), RuleAction::Allow);
        assert_eq!(decide(&rules, 2, "friend@spam.example").unwrap().action(), RuleAction::Block);
        assert_eq!(decide(&rules, 1, "x@other.example").unwrap().action(), RuleAction::Block);
        assert_eq!(decide(&rules, 2, "x@other.example").unwrap().action(), RuleAction::Allow);
        // The more specific domain wins, but only in its own account.
        assert_eq!(decide(&rules, 2, "a@sub.spam.example").unwrap().action(), RuleAction::Allow);
        assert_eq!(decide(&rules, 1, "a@sub.spam.example").unwrap().action(), RuleAction::Block);
        assert!(decide(&rules, 1, "a@notspam.example").is_none());
        assert!(decide(&rules, 1, "").is_none());
    }

    #[test]
    fn fingerprint_ignores_ids_and_whitespace_but_not_content() {
        let m = Message { id: 1, account_id: 1, subject: "Hi".into(), from: crate::model::EmailAddress::new("A", "a@x.example"), ..Default::default() };
        let moved = Message { id: 9, folder_id: 4, remote_id: Some("r".into()), ..m.clone() };
        assert_eq!(fingerprint(&m, "hello  world"), fingerprint(&moved, "hello world"));
        assert_ne!(fingerprint(&m, "hello world"), fingerprint(&m, "goodbye"));
        assert_ne!(fingerprint(&m, "x"), fingerprint(&Message { account_id: 2, ..m.clone() }, "x"));
    }
}
