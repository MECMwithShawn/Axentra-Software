//! Order and package detection. Pure: never talks to the network, never touches the store.
//!
//! [`extract`] reads one message (capped, scanned as text) into an
//! [`Extraction`]; [`group`] and [`build_order`] merge the extractions of an
//! order's emails into one [`Order`]. Links in mail are never stored, shown
//! or followed: the only URL ever built is [`Carrier::tracking_url`], from a
//! fixed template.

use std::collections::HashSet;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::junk::analyze::Severity;
use crate::model::{AccountId, Message, MessageId};

pub(crate) mod carriers;
mod dates;
mod extract;
#[cfg(test)]
mod fixtures;
mod merge;
mod status;

pub use carriers::validate;
pub use extract::{extract, extract_guarded};
pub(crate) use extract::scam_signs;
pub(crate) use extract::merchant_key;
pub(crate) use merge::join_only;
pub use merge::{Entry, Group, KEEP_DAYS, OrderSection, build_order, group, section};

/// Bump when what is extracted (or how) changes; cached extractions with
/// another version are redone.
pub const EXTRACT_VERSION: u32 = 7;
/// Tracking numbers kept per message.
pub const MAX_TRACKING: usize = 10;
pub const MAX_ITEMS: usize = 10;

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Carrier {
    Ups,
    Usps,
    FedEx,
    Dhl,
    DhlEcommerce,
    AmazonLogistics,
    RoyalMail,
    CanadaPost,
    PostNl,
    DeutschePost,
    LaPoste,
    JapanPost,
    ChinaPost,
    Evri,
    Dpd,
    OnTrac,
    /// A UPU S10 number of another postal service: its country code
    /// (uppercase, 2 letters).
    Postal(String),
}

/// How a tracking number was confirmed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Validation {
    /// The carrier's check digit matches.
    CheckDigit,
    /// No check digit; next to a tracking label or the carrier's name.
    Labelled,
    /// Read from a link to the carrier's tracking page.
    Link,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TrackingNumber {
    pub carrier: Carrier,
    /// `[A-Z0-9]{8,40}`.
    pub number: String,
    pub validated: Validation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum OrderStatus {
    Ordered,
    Shipped,
    InTransit,
    OutForDelivery,
    ReadyForPickup,
    Delivered,
    Delayed,
    Exception,
    Cancelled,
    Refunded,
    Returned,
}

/// Expected delivery: `from == to` for a single day.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DateWindow {
    pub from: NaiveDate,
    pub to: NaiveDate,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExtractionKind {
    /// Not an order email.
    #[default]
    None,
    Order,
    /// Looks like an order email but must not become one (a scam sign or
    /// an ignored sender).
    Suppressed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SuppressReason {
    /// The junk check found signs of phishing.
    Danger,
    /// Claims to be a carrier, sent from a domain that isn't the carrier's.
    CarrierImpersonation { claimed: Carrier, domain: String },
    /// A parcel "on hold" that asks for a fee.
    DeliveryFeeLure,
    /// "Don't track orders from…" in Settings.
    IgnoredSender,
    /// Looks like an order email, but the junk check found the sender
    /// posing as someone else ("Amazon.com" on a look-alike domain): the
    /// fake-order scam.
    Impersonation,
}

/// Why a message counts (or less so) as an order email, for "Why is this here?".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum OrderReason {
    OrderNumber { label: String },
    Tracking { carrier: Carrier, check_digit: bool },
    StatusPhrase { phrase: String, in_subject: bool },
    KnownMerchant,
    TransactionalSender,
    Reply,
    NewsletterShape,
    /// Tickets or a meal: an order with nothing to ship.
    NotAParcel,
    JunkCaution,
    OwnMessage,
}

impl OrderReason {
    pub fn describe(&self) -> String {
        match self {
            OrderReason::OrderNumber { label } => format!("Has order number {label}"),
            OrderReason::Tracking { carrier, check_digit: true } => format!("{} tracking number with a valid check digit", carrier.name()),
            OrderReason::Tracking { carrier, check_digit: false } => format!("{} tracking number", carrier.name()),
            OrderReason::StatusPhrase { phrase, in_subject: true } => format!("Says \u{201c}{phrase}\u{201d} in the subject"),
            OrderReason::StatusPhrase { phrase, in_subject: false } => format!("Says \u{201c}{phrase}\u{201d}"),
            OrderReason::KnownMerchant => "You've had orders from this sender before".into(),
            OrderReason::TransactionalSender => "Sent from an order or shipping address".into(),
            OrderReason::Reply => "Looks like a reply in a conversation".into(),
            OrderReason::NewsletterShape => "Looks like a newsletter or promotion".into(),
            OrderReason::NotAParcel => "Looks like tickets or a food delivery".into(),
            OrderReason::JunkCaution => "Has some warning signs".into(),
            OrderReason::OwnMessage => "Sent from your own address".into(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Merchant {
    /// Registrable domain of the sender.
    pub domain: String,
    pub name: String,
    /// Groups orders: the domain, the address for a free-mail sender, or
    /// `name:` + the lowercased name for marketplace relays.
    pub key: String,
}

/// What one message says about an order.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Extraction {
    pub version: u32,
    pub kind: ExtractionKind,
    pub merchant: Merchant,
    /// Uppercase, no leading `#`.
    pub order_number: Option<String>,
    /// At most [`MAX_TRACKING`], deduped.
    pub tracking: Vec<TrackingNumber>,
    pub status: Option<OrderStatus>,
    pub expected: Option<DateWindow>,
    /// At most [`MAX_ITEMS`], 120 characters each.
    pub items: Vec<String>,
    /// 0..=1.
    pub confidence: f32,
    pub reasons: Vec<OrderReason>,
    pub suppressed: Option<SuppressReason>,
    /// An input cap was hit.
    pub truncated: bool,
    /// The extraction failed part way; nothing in it is used.
    pub incomplete: bool,
    /// A carrier's own mail, which the account's provider vouched came
    /// from the carrier's domain (see `Input::sender_authenticated`). Only
    /// such mail confirms a shop's parcels (`trusted_merchant`).
    pub carrier_verified: bool,
}

impl Extraction {
    /// Not an order email.
    pub fn none() -> Self {
        Self { version: EXTRACT_VERSION, ..Default::default() }
    }

    /// The keys that group this message with the rest of its order:
    /// `"{acct}:o:{merchant}:{ORDERNO}"` and one `"{acct}:t:{carrier}:{NUMBER}"`
    /// per tracking number. Empty unless this is an order email.
    pub fn keys(&self, account: AccountId) -> Vec<String> {
        if self.kind != ExtractionKind::Order {
            return Vec::new();
        }
        let order = self.order_number.iter().map(|n| format!("{account}:o:{}:{n}", self.merchant.key));
        let tracking = self.tracking.iter().map(|t| format!("{account}:t:{}:{}", t.carrier.id(), t.number));
        order.chain(tracking).collect()
    }
}

/// One message and what is known about it, for [`extract`].
pub struct Input<'a> {
    pub message: &'a Message,
    pub html: Option<&'a str>,
    pub text: &'a str,
    /// Cached junk severity; `None` = no warning signs (or not checked yet).
    pub junk: Option<Severity>,
    /// The junk check found the sender posing as someone else (a brand
    /// name on another domain, a look-alike domain, replies elsewhere).
    pub impersonation: bool,
    /// You reported the message as phishing (also Danger in `junk`): no
    /// allow rule lets it through.
    pub reported_phishing: bool,
    /// A junk sender rule allows this sender.
    pub sender_allowed: bool,
    /// The junk check's trusted `Authentication-Results` vouch for the
    /// From domain (DMARC pass, or an aligned DKIM pass). Without it a
    /// carrier's From address is anyone's to write: no carrier exemption
    /// from the fee-lure gate and no carrier bonus.
    pub sender_authenticated: bool,
    /// The accounts' own addresses, lowercase.
    pub own_addresses: &'a [String],
    /// Contact addresses, lowercase.
    pub contacts: &'a HashSet<String>,
    /// The sender's merchant key already has an order in this account.
    pub known_merchant: bool,
    /// ... and a carrier's own mail (from the carrier's domain) is part of
    /// one of its orders: only then is its parcel-fee mail trusted (see
    /// `extract::scam`). A check digit is no confirmation: anyone can work
    /// one out.
    pub trusted_merchant: bool,
    /// The sender matches "Don't track orders from…".
    pub ignored: bool,
}

/// The user's own changes to an order, kept across rebuilds (and backed
/// up). Keyed by any key of the order's group.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct OrderUserState {
    pub key: String,
    pub hidden: bool,
    pub status_override: Option<OrderStatus>,
    pub updated_at: i64,
}

/// One tracking number of an order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Shipment {
    pub tracking: TrackingNumber,
    pub status: Option<OrderStatus>,
    pub last_update: DateTime<Utc>,
}

/// One email of an order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderEvent {
    pub message_id: MessageId,
    pub at: DateTime<Utc>,
    pub status: Option<OrderStatus>,
    pub subject: String,
}

/// An order built from all its emails.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Order {
    /// Row id, NOT stable across rebuilds; the UI addresses orders by `key`.
    pub id: i64,
    /// The canonical key (stable).
    pub key: String,
    /// All keys of the group.
    pub keys: Vec<String>,
    pub account_id: AccountId,
    pub merchant: Merchant,
    pub order_number: Option<String>,
    pub shipments: Vec<Shipment>,
    pub status: OrderStatus,
    pub partly_delivered: bool,
    pub expected: Option<DateWindow>,
    pub items: Vec<String>,
    /// One per source message, deduped by `internet_message_id`.
    pub events: Vec<OrderEvent>,
    /// `received_at` of the event that set `status`.
    pub status_at: DateTime<Utc>,
    pub delivered_at: Option<DateTime<Utc>>,
    /// The newest event.
    pub updated_at: DateTime<Utc>,
    pub hidden: bool,
    /// `status` is the user's.
    pub overridden: bool,
    /// What the mail says, under the user's status (`None` in an order
    /// built before this was kept).
    pub mail_status: Option<OrderStatus>,
    /// Union of the events' reasons, deduped, for "Why is this here?".
    pub reasons: Vec<OrderReason>,
}

impl Default for Order {
    fn default() -> Self {
        Self {
            id: 0,
            key: String::new(),
            keys: Vec::new(),
            account_id: 0,
            merchant: Merchant::default(),
            order_number: None,
            shipments: Vec::new(),
            status: OrderStatus::Ordered,
            partly_delivered: false,
            expected: None,
            items: Vec::new(),
            events: Vec::new(),
            status_at: DateTime::default(),
            delivered_at: None,
            updated_at: DateTime::default(),
            hidden: false,
            overridden: false,
            mail_status: None,
            reasons: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_only_for_order_emails() {
        let mut e = Extraction { order_number: Some("BL-48213".into()), merchant: Merchant { key: "brightleaf.example".into(), ..Default::default() }, ..Extraction::none() };
        e.tracking.push(TrackingNumber { carrier: Carrier::Ups, number: "1Z999AA10123456784".into(), validated: Validation::CheckDigit });
        assert!(e.keys(1).is_empty());
        e.kind = ExtractionKind::Order;
        assert_eq!(e.keys(1), ["1:o:brightleaf.example:BL-48213", "1:t:ups:1Z999AA10123456784"]);
        assert_eq!(Extraction::none().version, EXTRACT_VERSION);
    }

    #[test]
    fn status_names_round_trip() {
        use OrderStatus::*;
        for s in [Ordered, Shipped, InTransit, OutForDelivery, ReadyForPickup, Delivered, Delayed, Exception, Cancelled, Refunded, Returned] {
            assert_eq!(OrderStatus::parse(s.as_str()), Some(s));
            assert!(!s.label().is_empty() && !s.icon().is_empty());
            assert_eq!(s.step().is_none(), matches!(s, Delayed | Exception | Cancelled | Refunded | Returned));
        }
        assert_eq!(OrderStatus::parse("lost"), None);
    }

    #[test]
    fn order_detection_and_topics_do_no_io() {
        let sources = [
            ("scan.rs", include_str!("../scan.rs")),
            ("orders/mod.rs", include_str!("mod.rs")),
            ("orders/carriers.rs", include_str!("carriers.rs")),
            ("orders/dates.rs", include_str!("dates.rs")),
            ("orders/extract.rs", include_str!("extract.rs")),
            ("orders/fixtures.rs", include_str!("fixtures.rs")),
            ("orders/merge.rs", include_str!("merge.rs")),
            ("orders/status.rs", include_str!("status.rs")),
            ("topics/mod.rs", include_str!("../topics/mod.rs")),
            ("topics/classify.rs", include_str!("../topics/classify.rs")),
            ("topics/lexicon.rs", include_str!("../topics/lexicon.rs")),
        ];
        // Spelled in pieces so this test doesn't find itself.
        let banned: Vec<String> = [["std::", "net"], ["std::", "fs"], ["std::", "process"], ["req", "west"], ["ur", "eq"], ["hy", "per"], ["crate::", "store"], ["rus", "qlite"]]
            .iter()
            .map(|p| p.concat())
            .collect();
        for (name, src) in sources {
            for b in &banned {
                assert!(!src.contains(b.as_str()), "{name} mentions {b}");
            }
        }
    }

    #[test]
    fn reasons_describe_themselves() {
        let r = OrderReason::Tracking { carrier: Carrier::Ups, check_digit: true };
        assert_eq!(r.describe(), "UPS tracking number with a valid check digit");
        assert_eq!(OrderReason::OrderNumber { label: "BL-48213".into() }.describe(), "Has order number BL-48213");
    }
}
