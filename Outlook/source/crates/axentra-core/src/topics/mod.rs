//! Topic sorting. Pure: never talks to the network, never touches the store.
//!
//! [`classify`] gives one message a [`Topic`]: the user's corrections first,
//! then the suspicious guard (phishing never lands in Orders, Finance or
//! Receipts), meetings, orders, people, and finally scored signals
//! (`lexicon.rs`). Nothing is moved: topics are a view of the mail.

use serde::{Deserialize, Serialize};

use crate::junk::analyze::Severity;
use crate::model::{AccountId, Attachment, Message};
use crate::orders::ExtractionKind;

mod classify;
mod lexicon;

pub use classify::{classify, classify_guarded};

/// Bump when how mail is sorted changes; cached topics with another version
/// are recomputed.
pub const CLASSIFY_VERSION: u32 = 4;

/// No `Default`: there is no topic a message has before it is sorted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Topic {
    People,
    Orders,
    Receipts,
    Finance,
    Travel,
    Events,
    Social,
    Notifications,
    Newsletters,
    Other,
}

impl Topic {
    /// In display order.
    pub const ALL: [Topic; 10] = [
        Topic::People,
        Topic::Orders,
        Topic::Receipts,
        Topic::Finance,
        Topic::Travel,
        Topic::Events,
        Topic::Social,
        Topic::Notifications,
        Topic::Newsletters,
        Topic::Other,
    ];

    /// Position in [`Topic::ALL`].
    pub fn index(self) -> usize {
        self as usize
    }

    /// The stored value.
    pub fn id(self) -> &'static str {
        match self {
            Topic::People => "people",
            Topic::Orders => "orders",
            Topic::Receipts => "receipts",
            Topic::Finance => "finance",
            Topic::Travel => "travel",
            Topic::Events => "events",
            Topic::Social => "social",
            Topic::Notifications => "notifications",
            Topic::Newsletters => "newsletters",
            Topic::Other => "other",
        }
    }

    pub fn parse(s: &str) -> Option<Topic> {
        Topic::ALL.into_iter().find(|t| t.id() == s)
    }

    pub fn label(self) -> &'static str {
        match self {
            Topic::People => "People",
            Topic::Orders => "Orders & shipping",
            Topic::Receipts => "Receipts & bills",
            Topic::Finance => "Finance",
            Topic::Travel => "Travel",
            Topic::Events => "Events & invitations",
            Topic::Social => "Social",
            Topic::Notifications => "Notifications & alerts",
            Topic::Newsletters => "Newsletters & promotions",
            Topic::Other => "Other",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Topic::People => "person",
            Topic::Orders => "box",
            Topic::Receipts => "receipt",
            Topic::Finance => "wallet",
            Topic::Travel => "airplane",
            Topic::Events => "calendar_ltr",
            Topic::Social => "people_community",
            Topic::Notifications => "alert_badge",
            Topic::Newsletters => "megaphone",
            Topic::Other => "tag",
        }
    }
}

/// Which rule decided a message's topic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source {
    MessageOverride,
    SenderRule,
    DomainRule,
    Suspicious,
    Meeting,
    Order,
    Person,
    Signals,
    Fallback,
}

impl Source {
    const ALL: [Source; 9] = [
        Source::MessageOverride,
        Source::SenderRule,
        Source::DomainRule,
        Source::Suspicious,
        Source::Meeting,
        Source::Order,
        Source::Person,
        Source::Signals,
        Source::Fallback,
    ];

    /// The stored value.
    pub fn as_str(self) -> &'static str {
        match self {
            Source::MessageOverride => "message_override",
            Source::SenderRule => "sender_rule",
            Source::DomainRule => "domain_rule",
            Source::Suspicious => "suspicious",
            Source::Meeting => "meeting",
            Source::Order => "order",
            Source::Person => "person",
            Source::Signals => "signals",
            Source::Fallback => "fallback",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Source::ALL.into_iter().find(|v| v.as_str() == s)
    }
}

/// Why a message got its topic, for "Why?".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Reason {
    MessageOverride,
    SenderRule { pattern: String },
    DomainRule { pattern: String },
    Suspicious,
    Meeting,
    OrderEmail { merchant: String },
    KnownPerson { name: String },
    SenderLocalPart { part: String },
    SubjectWord { word: String },
    BodyPhrase { phrase: String },
    UnsubscribeLink,
    VerificationCode,
    CarrierSender,
    LowConfidence { best: Topic },
}

impl Reason {
    /// Never empty.
    pub fn describe(&self) -> String {
        match self {
            Reason::MessageOverride => "You chose this topic for this message".into(),
            Reason::SenderRule { pattern } => format!("You chose this topic for mail from {pattern}"),
            Reason::DomainRule { pattern } => format!("You chose this topic for everything from {pattern}"),
            Reason::Suspicious => "This message has signs of phishing".into(),
            Reason::Meeting => "It's a meeting invitation or response".into(),
            Reason::OrderEmail { merchant } if merchant.is_empty() => "It's an order or shipping email".into(),
            Reason::OrderEmail { merchant } => format!("It's an order or shipping email from {merchant}"),
            Reason::KnownPerson { name } if name.is_empty() => "It's from someone you know".into(),
            Reason::KnownPerson { name } => format!("It's from {name}, someone you know"),
            Reason::SenderLocalPart { part } => format!("Sent from a \u{201c}{part}\u{201d} address"),
            Reason::SubjectWord { word } => format!("The subject mentions \u{201c}{word}\u{201d}"),
            Reason::BodyPhrase { phrase } => format!("The message mentions \u{201c}{phrase}\u{201d}"),
            Reason::UnsubscribeLink => "It has an unsubscribe link".into(),
            Reason::VerificationCode => "It has a verification code".into(),
            Reason::CarrierSender => "It's from a delivery company".into(),
            Reason::LowConfidence { best: Topic::Other } => "Not sure enough to sort it".into(),
            Reason::LowConfidence { best } => format!("Not sure enough to sort it (closest: {})", best.label()),
        }
    }
}

/// A message's topic and why.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Assignment {
    pub topic: Topic,
    /// 0..=1.
    pub confidence: f32,
    pub source: Source,
    pub reasons: Vec<Reason>,
}

/// A correction: mail from `pattern` (an address, or a domain with
/// `is_domain`) goes to `topic`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopicRule {
    /// The account the rule is for; 0 = all accounts.
    pub account_key: i64,
    /// Lowercase.
    pub pattern: String,
    pub is_domain: bool,
    pub topic: Topic,
}

/// What order detection found in a message (cached).
pub struct OrderHint<'a> {
    pub kind: ExtractionKind,
    pub confidence: f32,
    pub merchant: &'a str,
}

/// One message and what is known about it, for [`classify`].
pub struct Input<'a> {
    pub message: &'a Message,
    pub html: Option<&'a str>,
    pub text: &'a str,
    pub attachments: &'a [Attachment],
    /// The cached extraction; `None` when order tracking is off or the
    /// message isn't extracted yet.
    pub order: Option<OrderHint<'a>>,
    pub junk: Option<Severity>,
    /// The junk check found the sender posing as someone else (see
    /// `junk::analyze::Finding::impersonates`).
    pub impersonation: bool,
    /// The junk check's trusted `Authentication-Results` vouch for the
    /// From domain (see `orders::Input::sender_authenticated`).
    pub sender_authenticated: bool,
    /// The sender's name when they are a contact.
    pub contact_name: Option<&'a str>,
    /// The user has written to the sender.
    pub replied_before: bool,
    /// Loaded once per batch.
    pub rules: &'a [TopicRule],
    pub message_override: Option<Topic>,
}

/// A free mail provider: a domain correction would cover strangers, so
/// none is offered.
pub fn is_freemail(domain: &str) -> bool {
    let domain = domain.trim().trim_end_matches('.').to_ascii_lowercase();
    if lexicon::FREEMAIL.contains(&domain.as_str()) {
        return true;
    }
    // A provider's country domain: "hotmail.co.uk", "yahoo.com.br",
    // "gmx.at", "outlook.de". Not a subdomain of someone else's
    // ("mail.example.com").
    let Some((label, suffix)) = domain.split_once('.') else { return false };
    let country = |s: &str| (2..=3).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_lowercase());
    let suffix_ok = match suffix.split_once('.') {
        None => country(suffix),
        Some((second, cc)) => matches!(second, "co" | "com" | "ne" | "or") && cc.len() == 2 && country(cc),
    };
    suffix_ok && lexicon::FREEMAIL_FAMILIES.contains(&label)
}

/// True when `rule` covers `address_lower` in `account`: the exact address,
/// or the domain or a subdomain of it (label boundary).
pub fn rule_matches(rule: &TopicRule, account: AccountId, address_lower: &str) -> bool {
    if rule.account_key != 0 && rule.account_key != account {
        return false;
    }
    if rule.is_domain {
        crate::junk::domain_matches(crate::junk::domain_of(address_lower), &rule.pattern)
    } else {
        address_lower == rule.pattern
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topic_names_round_trip() {
        for (i, t) in Topic::ALL.into_iter().enumerate() {
            assert_eq!(t.index(), i);
            assert_eq!(Topic::parse(t.id()), Some(t));
            assert!(!t.label().is_empty() && !t.icon().is_empty());
        }
        assert_eq!(Topic::parse("work"), None);
        for s in Source::ALL {
            assert_eq!(Source::parse(s.as_str()), Some(s));
        }
    }

    #[test]
    fn rules_match_on_a_label_boundary() {
        let rule = TopicRule { account_key: 0, pattern: "example.com".into(), is_domain: true, topic: Topic::Travel };
        assert!(rule_matches(&rule, 1, "a@example.com"));
        assert!(rule_matches(&rule, 1, "a@mail.example.com"));
        assert!(!rule_matches(&rule, 1, "a@evilexample.com"));
        let mine = TopicRule { account_key: 2, pattern: "a@b.example".into(), is_domain: false, topic: Topic::Finance };
        assert!(rule_matches(&mine, 2, "a@b.example") && !rule_matches(&mine, 3, "a@b.example"));
        assert!(is_freemail("gmail.com") && !is_freemail("example.com"));
    }

    #[test]
    fn free_mail_country_domains_are_free_mail() {
        for d in ["hotmail.co.uk", "yahoo.co.uk", "yahoo.fr", "yahoo.com.br", "yahoo.co.jp", "outlook.de", "live.co.uk", "hotmail.fr", "gmx.at", "gmx.ch", "mail.com", "pm.me", "protonmail.ch", "Hotmail.CO.UK"] {
            assert!(is_freemail(d), "{d}");
        }
        for d in ["mail.example.com", "hotmail.example.com", "yahoo.example", "brightleaf.co.uk", "outlook.office365.com", "livenation.com"] {
            assert!(!is_freemail(d), "{d}");
        }
    }
}
