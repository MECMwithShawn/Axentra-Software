//! Status vocabulary: what an order email says happened, read from the
//! subject first, then the head of the body (never the footer).

use super::OrderStatus;
use crate::scan::{Prepared, contains_word, find_word, floor_char};

impl OrderStatus {
    /// The stored value.
    pub fn as_str(self) -> &'static str {
        match self {
            OrderStatus::Ordered => "ordered",
            OrderStatus::Shipped => "shipped",
            OrderStatus::InTransit => "in_transit",
            OrderStatus::OutForDelivery => "out_for_delivery",
            OrderStatus::ReadyForPickup => "ready_for_pickup",
            OrderStatus::Delivered => "delivered",
            OrderStatus::Delayed => "delayed",
            OrderStatus::Exception => "exception",
            OrderStatus::Cancelled => "cancelled",
            OrderStatus::Refunded => "refunded",
            OrderStatus::Returned => "returned",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "ordered" => OrderStatus::Ordered,
            "shipped" => OrderStatus::Shipped,
            "in_transit" => OrderStatus::InTransit,
            "out_for_delivery" => OrderStatus::OutForDelivery,
            "ready_for_pickup" => OrderStatus::ReadyForPickup,
            "delivered" => OrderStatus::Delivered,
            "delayed" => OrderStatus::Delayed,
            "exception" => OrderStatus::Exception,
            "cancelled" => OrderStatus::Cancelled,
            "refunded" => OrderStatus::Refunded,
            "returned" => OrderStatus::Returned,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            OrderStatus::Ordered => "Ordered",
            OrderStatus::Shipped => "Shipped",
            OrderStatus::InTransit => "In transit",
            OrderStatus::OutForDelivery => "Out for delivery",
            OrderStatus::ReadyForPickup => "Ready for pickup",
            OrderStatus::Delivered => "Delivered",
            OrderStatus::Delayed => "Delayed",
            OrderStatus::Exception => "Delivery problem",
            OrderStatus::Cancelled => "Cancelled",
            OrderStatus::Refunded => "Refunded",
            OrderStatus::Returned => "Returned",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            OrderStatus::Ordered => "box",
            OrderStatus::Shipped | OrderStatus::InTransit | OrderStatus::OutForDelivery => "vehicle_truck",
            OrderStatus::ReadyForPickup => "location",
            OrderStatus::Delivered => "checkmark_circle",
            OrderStatus::Delayed | OrderStatus::Exception => "warning",
            OrderStatus::Cancelled => "dismiss_circle",
            OrderStatus::Refunded | OrderStatus::Returned => "arrow_undo",
        }
    }

    /// The tracker step (Ordered → Shipped → Out for delivery → Delivered);
    /// `None` off the happy path.
    pub fn step(self) -> Option<u8> {
        match self {
            OrderStatus::Ordered => Some(0),
            OrderStatus::Shipped | OrderStatus::InTransit | OrderStatus::ReadyForPickup => Some(1),
            OrderStatus::OutForDelivery => Some(2),
            OrderStatus::Delivered => Some(3),
            OrderStatus::Delayed | OrderStatus::Exception | OrderStatus::Cancelled | OrderStatus::Refunded | OrderStatus::Returned => None,
        }
    }

    /// Nothing more is going to happen to the order.
    pub fn is_terminal(self) -> bool {
        matches!(self, OrderStatus::Delivered | OrderStatus::Cancelled | OrderStatus::Refunded | OrderStatus::Returned)
    }
}

/// Status phrases, in priority order: the first status with a phrase in
/// the text wins.
const PHRASES: &[(OrderStatus, &[&str])] = &[
    (
        OrderStatus::Refunded,
        &[
            "refund issued",
            "refund processed",
            "refund has been issued",
            "issued a refund",
            "issued your refund",
            "has been refunded",
            "refunded",
            "your refund",
            "rückerstattung",
            "erstattung",
            "remboursement",
            "reembolso",
            "返金",
        ],
    ),
    (
        OrderStatus::Cancelled,
        &[
            "order cancelled",
            "order canceled",
            "has been cancelled",
            "has been canceled",
            "was cancelled",
            "was canceled",
            "cancelled your order",
            "canceled your order",
            "cancelled:",
            "canceled:",
            "cancellation confirmed",
            "storniert",
            "annulée",
            "annulee",
            "cancelado",
            "キャンセル",
            "已取消",
        ],
    ),
    (
        OrderStatus::Returned,
        &["return received", "received your return", "return is complete", "return complete", "returned to sender", "return processed", "retour"],
    ),
    (
        OrderStatus::Exception,
        &["delivery exception", "unable to deliver", "delivery attempted", "attempted delivery", "failed delivery", "missed delivery", "address issue", "held at", "zustellversuch"],
    ),
    (OrderStatus::Delayed, &["delayed", "running late", "verspätet", "verspaetet", "retard", "retrasado", "遅延"]),
    (
        OrderStatus::Delivered,
        &[
            "was delivered",
            "has been delivered",
            "delivered:",
            "delivered today",
            "your package was delivered",
            "delivered",
            "zugestellt",
            "livrée",
            "livree",
            "livré",
            "entregado",
            "配達完了",
            "お届け済み",
            "已签收",
        ],
    ),
    (OrderStatus::OutForDelivery, &["out for delivery", "arriving today", "scheduled for delivery today", "will be delivered today", "in zustellung", "en cours de livraison", "en reparto", "配達中", "派送中"]),
    (OrderStatus::ReadyForPickup, &["ready for pickup", "ready for pick up", "ready to collect", "held at", "abholbereit", "disponible en point"]),
    (
        OrderStatus::InTransit,
        &[
            "in transit",
            "on its way",
            "on the way",
            "scheduled for delivery",
            "scheduled delivery",
            "new delivery date",
            "delivery date has changed",
            "unterwegs",
            "en route",
            "en camino",
            "輸送中",
            "运输中",
        ],
    ),
    (
        OrderStatus::Shipped,
        &["has shipped", "have shipped", "shipped", "dispatched", "versandt", "verschickt", "expédiée", "expediee", "expédié", "enviado", "発送", "已发货"],
    ),
    (
        OrderStatus::Ordered,
        &[
            "order confirmation",
            "order confirmed",
            "thanks for your order",
            "thank you for your order",
            "order received",
            "order has been received",
            "order placed",
            "has been placed",
            "ordered:",
            "we've received your order",
            "we received your order",
            "bestellbestätigung",
            "bestellbestaetigung",
            "confirmation de commande",
            "confirmación de pedido",
            "confirmacion de pedido",
            "ご注文ありがとうございます",
            "注文確認",
            "订单确认",
        ],
    ),
];

/// Before a Delivered phrase: it wasn't delivered (a delivery problem).
/// Before an earlier step ("hasn't shipped yet"): the step hasn't happened,
/// so the phrase is not read.
const NEGATIONS: &[&str] = &["not ", "n't ", "has not", "unable", "noch nicht", "pas été", "pas ete"];
/// The only words that may stand between a negation and the step it denies
/// ("has not yet been delivered", "could not be delivered"): anything else
/// means the negation is about something else ("Don't worry it was
/// delivered").
const NEGATED_GAP: &[&str] = &[
    "be", "been", "being", "yet", "it", "they", "was", "were", "is", "are", "has", "have", "had", "get", "got", "gotten", "still", "successfully",
    "fully", "all", "able", "to", "your", "the", "package", "parcel", "order", "item", "items", "shipment",
];
/// Where a clause ends: a negation before one of these is about something
/// else ("Didn't see the driver? It was delivered").
const CLAUSE_BREAKS: &[char] = &['.', '?', '!', ',', ';', ':', '\n'];
/// Before a step after Ordered: it hasn't happened yet ("we'll email you
/// when it has shipped", "once it's on its way").
const FUTURE: &[&str] = &["when ", "once ", "as soon as ", "sobald ", "will be ", "should be ", "to be "];

/// Before a phrase: policy or a maybe, not news ("items can be returned
/// and refunded", "some deliveries may be delayed").
const CONDITIONAL: &[&str] = &["can be ", "may be ", "might be ", "could be ", "if ", "eligible", "kann ", "können ", "peut ", "peuvent ", "puede "];
/// After a phrase: a policy's time limit ("refunded within 30 days",
/// "retour binnen 30 dagen").
const POLICY_AFTER: &[&str] = &["within ", "innerhalb ", "binnen ", "sous ", "dans les ", "dentro de ", "policy"];
/// After a step: how long shipping takes, not news ("Standard shipping,
/// delivered in 3-5 business days").
const DURATION_AFTER: &[&str] = &["business day", "working day", "werktag", "jours ouvr", "días hábiles", "dias habiles"];

/// After a bare "delivered" or "shipped": a promise ("delivered by
/// Friday", "get it shipped by Oct 23"), not news.
const PROMISE_BY: &[&str] = &[
    "today", "tonight", "tomorrow", "end", "monday", "mon", "tuesday", "tue", "tues", "wednesday", "wed", "thursday", "thu", "thur",
    "thurs", "friday", "fri", "saturday", "sat", "sunday", "sun", "january", "jan", "february", "feb", "march", "mar", "april", "apr", "may", "june",
    "jun", "july", "jul", "august", "aug", "september", "sept", "sep", "october", "oct", "november", "nov", "december", "dec",
];

/// Before a refund or cancellation: a promise, not news ("we'll issue
/// your refund", "you'll be refunded when…").
const PROMISED: &[&str] = &["we'll ", "we will ", "you'll ", "you will ", "will issue", "going to "];
/// After a refund: it waits for the item to come back ("your refund once
/// we receive your item").
const AWAITS_RETURN: &[&str] = &[
    "once we receive",
    "when we receive",
    "after we receive",
    "once we've received",
    "when we've received",
    "once we have received",
    "when we have received",
    "once your return",
    "when your return",
    "once the item",
    "when the item",
];
/// After "your refund": it was asked for or is still open, not issued
/// ("We received your refund request", "your refund is being processed").
const REFUND_OPEN: &[&str] = &[
    "request", "is being processed", "is processing", "is pending", "pending", "status", "eligibility", "application", "claim", "will ", "should ",
    "can ", "may ",
];
/// After "held at": where to collect it ("held at a UPS Access Point for
/// pickup"), not a delivery problem.
const COLLECT: &[&str] = &["pickup", "pick up", "pick-up", "collect", "access point", "locker"];

/// How far before a phrase the guards look.
const GUARD: usize = 24;
/// How close before a refund or cancellation a negation must be ("has not
/// been refunded"): further back it is usually about something else
/// ("couldn't ship, so your order has been cancelled").
const NEAR: usize = 12;
/// How far after "held at" the place to collect from is looked for.
const PLACE: usize = 48;
/// Nouns "delayed" can describe ("your delayed package has been
/// delivered").
const PARCEL_NOUNS: &[&str] = &["package", "parcel", "order", "shipment", "item"];

/// Statuses that, in the body, only count when no other status phrase comes
/// first: they are as often policy text or small print as news ("If you're
/// not home, your parcel will be held at the pickup point").
fn strong(s: OrderStatus) -> bool {
    matches!(s, OrderStatus::Refunded | OrderStatus::Cancelled | OrderStatus::Returned | OrderStatus::Delayed | OrderStatus::Exception)
}

/// The status a message states: (status, the phrase, whether it was in the
/// subject). The subject first, then the head of the body (never the footer).
pub(crate) fn status_of(p: &Prepared) -> Option<(OrderStatus, String, bool)> {
    let subject = p.subject.to_ascii_lowercase();
    let body = p.head_lower.get(subject.len()..).unwrap_or("");
    let found = status_in(&subject)
        .map(|(s, phrase)| (s, phrase, true))
        .or_else(|| subject_words(&subject).map(|(s, word)| (s, word.to_string(), true)))
        .or_else(|| status_in_body(body).map(|(s, phrase)| (s, phrase, false)));
    // A return that says its refund was issued is a refund: "We've received
    // your return and issued a refund of $20.00".
    if let Some((OrderStatus::Returned, _, in_subject)) = found
        && let Some((_, s, phrase)) = PHRASES.iter().find(|(s, _)| *s == OrderStatus::Refunded).and_then(|(s, phrases)| first_in(body, *s, phrases))
    {
        return Some((s, phrase, in_subject));
    }
    found
}

/// A status from the subject's words with the order number between them:
/// "Order #PP-77310 confirmed", "Order #BL-48213 cancelled".
fn subject_words(subject: &str) -> Option<(OrderStatus, &'static str)> {
    if !contains_word(subject, "order") {
        return None;
    }
    // Not a promise or a maybe: "Your order will be cancelled if…".
    let news = |i: usize| {
        let before = &subject[floor_char(subject, i.saturating_sub(GUARD))..i];
        !FUTURE.iter().chain(PROMISED).chain(CONDITIONAL).chain(NEGATIONS).any(|w| before.contains(w))
    };
    [(OrderStatus::Cancelled, "cancelled"), (OrderStatus::Cancelled, "canceled"), (OrderStatus::Ordered, "confirmed")]
        .into_iter()
        .find(|(_, w)| find_word(subject, w).any(news))
}

/// The status in the body (the head, after the subject). A refund,
/// cancellation, return, delay or delivery problem only counts when it is
/// the first status phrase: "Order number: BL-1 … items can be returned
/// and refunded" is a receipt, not a refund.
fn status_in_body(body: &str) -> Option<(OrderStatus, String)> {
    let first = PHRASES.iter().filter_map(|(status, phrases)| first_in(body, *status, phrases)).min_by_key(|(at, ..)| *at);
    match first {
        Some((_, s, phrase)) if strong(s) => Some((s, phrase)),
        _ => PHRASES.iter().filter(|(s, _)| !strong(*s)).find_map(|(status, phrases)| first_in(body, *status, phrases)).map(|(_, s, phrase)| (s, phrase)),
    }
}

/// The status in `lower` (a subject), by priority.
fn status_in(lower: &str) -> Option<(OrderStatus, String)> {
    PHRASES.iter().find_map(|(status, phrases)| first_in(lower, *status, phrases)).map(|(_, s, phrase)| (s, phrase))
}

/// The first of `phrases` in `lower` that the guards let through: where it
/// starts, the status it states and the phrase.
fn first_in(lower: &str, status: OrderStatus, phrases: &[&str]) -> Option<(usize, OrderStatus, String)> {
    let mut best: Option<(usize, OrderStatus, String)> = None;
    for phrase in phrases {
        for i in find_word(lower, phrase) {
            if best.as_ref().is_some_and(|(at, ..)| *at <= i) {
                break;
            }
            let before = &lower[floor_char(lower, i.saturating_sub(GUARD))..i];
            let end = i + phrase.len();
            let after = &lower[end..floor_char(lower, end + GUARD)];
            if status != OrderStatus::Ordered && (CONDITIONAL.iter().any(|c| before.contains(c)) || POLICY_AFTER.iter().any(|c| after.contains(c))) {
                continue;
            }
            // A delivery problem that may happen ("your parcel will be held
            // at…") or a question ("Missed delivery? Reschedule online").
            if status == OrderStatus::Exception && (FUTURE.iter().any(|f| before.contains(f)) || after.starts_with('?')) {
                continue;
            }
            // "Held at" a place to collect it is ready for pickup; held
            // anywhere else (the depot, customs) is a delivery problem.
            if *phrase == "held at" {
                let place = &lower[end..floor_char(lower, end + PLACE)];
                if COLLECT.iter().any(|c| place.contains(c)) != (status == OrderStatus::ReadyForPickup) {
                    continue;
                }
            }
            // A refund or cancellation promised ("we'll issue your refund
            // once we receive your item", "you'll be refunded") or denied
            // ("has not been refunded") hasn't happened.
            if *phrase == "your refund" && REFUND_OPEN.iter().any(|w| after.trim_start().starts_with(w)) {
                continue;
            }
            if matches!(status, OrderStatus::Refunded | OrderStatus::Cancelled) {
                let near = &lower[floor_char(lower, i.saturating_sub(NEAR))..i];
                let wider = &lower[end..floor_char(lower, end + PLACE)];
                if FUTURE.iter().chain(PROMISED).any(|f| before.contains(f))
                    || NEGATIONS.iter().any(|n| near.contains(n))
                    || AWAITS_RETURN.iter().any(|a| wider.contains(a))
                {
                    continue;
                }
            }
            // "Delayed" describing the parcel ("your delayed package has
            // been delivered") isn't the news; "Delayed order #123" is.
            let described = before.trim_end().rsplit(' ').next().is_some_and(|w| matches!(w, "your" | "the" | "our"));
            if *phrase == "delayed" && described && PARCEL_NOUNS.iter().any(|n| after.trim_start().starts_with(n)) {
                continue;
            }
            // Every step after Ordered: Shipped, In transit, Ready for
            // pickup, Out for delivery and Delivered.
            let step = matches!(status, OrderStatus::Shipped | OrderStatus::InTransit | OrderStatus::ReadyForPickup | OrderStatus::OutForDelivery | OrderStatus::Delivered);
            if step && negated(before) {
                if status == OrderStatus::Delivered {
                    best = Some((i, OrderStatus::Exception, phrase.to_string()));
                    break;
                }
                continue;
            }
            if step && (FUTURE.iter().any(|f| before.contains(f)) || in_duration(after) || DURATION_AFTER.iter().any(|d| after.contains(d))) {
                continue;
            }
            if matches!(*phrase, "delivered" | "shipped" | "dispatched") && promised_by(after) {
                continue;
            }
            best = Some((i, status, phrase.to_string()));
            break;
        }
    }
    best
}

/// `before` (the text before a step phrase) denies it: a negation in the
/// same clause with only auxiliaries after it ("could not be ", "hasn't
/// yet been ", "noch nicht ").
fn negated(before: &str) -> bool {
    let clause = before.rfind(CLAUSE_BREAKS).map_or(before, |i| &before[i + 1..]);
    NEGATIONS.iter().any(|n| {
        clause.rfind(n).is_some_and(|i| {
            let gap = &clause[i + n.len()..];
            // "n't " and "not " end in a space; the others are whole words.
            (n.ends_with(' ') || gap.is_empty() || gap.starts_with(' '))
                && gap.split_ascii_whitespace().all(|w| NEGATED_GAP.contains(&w))
        })
    })
}

/// `after` (the text after a bare step phrase) starts with a promised day:
/// "by Friday", "by Oct 23", "by 10/23", "by the end of the week".
fn promised_by(after: &str) -> bool {
    let Some(rest) = after.trim_start().strip_prefix("by ") else { return false };
    let rest = rest.trim_start();
    let word: String = rest.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    rest.starts_with(|c: char| c.is_ascii_digit()) || PROMISE_BY.contains(&word.as_str())
}

/// `after` (the text after a step phrase) starts with a duration: "in 3-5
/// days", "in 2 days".
fn in_duration(after: &str) -> bool {
    after.trim_start().strip_prefix("in ").is_some_and(|rest| rest.trim_start().starts_with(|c: char| c.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Message;
    use crate::scan::prepare;

    fn status(subject: &str, body: &str) -> Option<(OrderStatus, String, bool)> {
        let m = Message { subject: subject.into(), ..Default::default() };
        status_of(&prepare(&m, None, body))
    }

    fn of(subject: &str) -> Option<OrderStatus> {
        status(subject, "").map(|(s, _, _)| s)
    }

    #[test]
    fn every_status_from_an_english_subject() {
        use OrderStatus::*;
        for (subject, want) in [
            ("Refund issued for order #HL-3304", Refunded),
            ("Your order #HL-3304 has been cancelled", Cancelled),
            ("Return received: order 5521", Returned),
            ("Delivery exception for your parcel", Exception),
            ("Delayed: order #PP-77310 has a new delivery date", Delayed),
            ("Delivered: your Northpeak order #NP-20931", Delivered),
            ("Out for delivery: your order #BL-48213", OutForDelivery),
            ("Your package is ready for pickup", ReadyForPickup),
            ("Your order NP-20931 is on its way", InTransit),
            ("Your Brightleaf order #BL-48213 has shipped", Shipped),
            ("Order confirmed: #BL-48213", Ordered),
            ("Thanks for your order #NP-20931", Ordered),
            ("Order #PP-77310 confirmed", Ordered),
            ("Ordered: \"Linen throw\" and 1 more item", Ordered),
            ("Your order #KC-1001 has been placed", Ordered),
            ("Order placed: #KC-1001", Ordered),
            ("Your order has been received", Ordered),
        ] {
            assert_eq!(of(subject), Some(want), "{subject}");
        }
        assert_eq!(of("Lunch on Friday?"), None);
        assert_eq!(of("Your parcel is undelivered"), None, "word boundary");
    }

    #[test]
    fn other_languages() {
        use OrderStatus::*;
        assert_eq!(of("Ihre Bestellung wurde versandt"), Some(Shipped));
        assert_eq!(of("Ihr Paket wurde zugestellt"), Some(Delivered));
        assert_eq!(of("Votre colis a été livré"), Some(Delivered));
        assert_eq!(of("Votre commande est en cours de livraison"), Some(OutForDelivery));
        assert_eq!(of("Tu pedido ha sido enviado"), Some(Shipped));
        assert_eq!(of("Reembolso de tu pedido"), Some(Refunded));
        assert_eq!(of("【さくら雑貨】ご注文の商品を発送しました"), Some(Shipped));
        assert_eq!(of("ご注文ありがとうございます"), Some(Ordered));
        assert_eq!(of("お荷物は配達完了しました"), Some(Delivered));
    }

    #[test]
    fn negation_and_future_guards() {
        use OrderStatus::*;
        assert_eq!(of("Your package could not be delivered"), Some(Exception));
        assert_eq!(of("Your parcel has not been delivered"), Some(Exception));
        assert_eq!(of("We couldn't deliver it: package wasn't delivered"), Some(Exception));
        assert_eq!(status("Your purchase", "Thank you for your order. We'll email you when it has shipped.").map(|s| s.0), Some(Ordered));
        assert_eq!(of("Your order will be delivered today"), Some(OutForDelivery));
        assert_eq!(of("Your refund for order 123 that was delivered"), Some(Refunded));
        // Not shipped yet: a delay notice, not news that it shipped.
        assert_eq!(of("Your order #SH-12345 hasn't shipped yet"), None);
        assert_eq!(of("Your order has not shipped"), None);
        assert_eq!(status("Update on your order #SH-12345", "Unfortunately your order has not shipped yet.").map(|s| s.0), None);
        assert_eq!(of("Your parcel is not out for delivery today"), None);
        assert_eq!(of("Your parcel has not yet been delivered"), Some(Exception));
        assert_eq!(of("Ihr Paket wurde noch nicht zugestellt"), Some(Exception));
        // A negation about something else is no delivery problem.
        let ups = "Didn't see the driver? Don't worry, it was delivered to your front door at 2:14 PM.";
        assert_eq!(status("UPS Update: Package Delivered", ups).map(|s| s.0), Some(Delivered));
        assert_eq!(status("Your UPS package", ups).map(|s| s.0), Some(Delivered));
        assert_eq!(status("Your UPS package", "Don't worry it was delivered to your neighbour.").map(|s| s.0), Some(Delivered));
        // Every step after Ordered waits for "when" and "once".
        let body = "Hi Ann, thanks for your order! We'll email you when it's on its way.\nOrder number: SH-12345";
        assert_eq!(status("Thanks for shopping with Shop", body).map(|s| s.0), Some(Ordered));
        assert_eq!(status("Your purchase", "We'll let you know once it is ready for pickup.").map(|s| s.0), None);
    }

    #[test]
    fn shipping_method_text_is_not_a_step() {
        use OrderStatus::*;
        let body = |b: &str| status("Your Shop order #SH-12345", b).map(|s| s.0);
        assert_eq!(body("Thank you for your order!\nShipping: Standard, delivered in 3-5 business days.\nOrder number: SH-12345"), Some(Ordered));
        assert_eq!(body("Thank you for your order!\nExpress: delivered in 2 days."), Some(Ordered));
        assert_eq!(body("Thank you for your order! Free shipping: dispatched within 2 working days."), Some(Ordered));
        assert_eq!(body("Thank you for your order! Standard delivery: shipped in 1 business day."), Some(Ordered));
        // Still news.
        assert_eq!(body("Good news: your package was delivered in the afternoon."), Some(Delivered));
        assert_eq!(body("Your order has shipped and is in transit."), Some(InTransit));
    }

    #[test]
    fn policy_text_in_the_body_is_not_news() {
        use OrderStatus::*;
        let body = |b: &str| status("Your Brightleaf purchase #BL-55555", b).map(|s| s.0);
        assert_eq!(body("Thanks for shopping. Items can be returned and refunded within 30 days. Order number: BL-55555"), None);
        assert_eq!(body("Thank you for your order. Items can be returned and refunded within 30 days."), Some(Ordered));
        assert_eq!(body("Thank you for your order! Some deliveries may be delayed by the weather."), Some(Ordered));
        assert_eq!(body("Bedankt voor je bestelling. Gratis retour binnen 30 dagen."), None);
        // A happy-path phrase first: the later refund wording is boilerplate.
        assert_eq!(body("Your order has shipped. Not happy? Your refund is quick and easy."), Some(Shipped));
        // Real news still counts.
        assert_eq!(body("Good news: your refund has been issued. Your order has been refunded."), Some(Refunded));
        assert_eq!(body("We're sorry: your order has been cancelled."), Some(Cancelled));
        assert_eq!(body("Thank you for your order. We'll email you when it has shipped."), Some(Ordered));
    }

    #[test]
    fn small_print_about_delivery_problems_is_not_one() {
        use OrderStatus::*;
        let body = |subject: &str, b: &str| status(subject, b).map(|s| s.0);
        let shipped = "Good news, your order has shipped! Tracking number: 1Z4X7Q203660123459. \
                       If you're not home, your parcel will be held at the nearest pickup point for 7 days.";
        assert_eq!(body("Update on order #BL-48213", shipped), Some(Shipped));
        assert_eq!(body("Your Brightleaf order #BL-48213", "Your order is on its way. Missed delivery? You can reschedule online."), Some(InTransit));
        assert_eq!(body("Your Brightleaf order #BL-48213", "Missed delivery? Reschedule online."), None);
        assert_eq!(of("Missed delivery? Reschedule online"), None);
        // Still a problem when it is the news.
        assert_eq!(body("Update on order #BL-48213", "We attempted delivery today. Your parcel is held at the depot."), Some(Exception));
        assert_eq!(body("Update on order #BL-48213", "Your parcel is held at the depot. It was shipped on Monday."), Some(Exception));
        assert_eq!(of("Your parcel is held at the depot"), Some(Exception));
    }

    #[test]
    fn common_cancellation_refund_and_return_wordings() {
        use OrderStatus::*;
        assert_eq!(of("Your order was cancelled"), Some(Cancelled));
        assert_eq!(of("Your order was canceled"), Some(Cancelled));
        assert_eq!(of("We've cancelled your order #BL-48213"), Some(Cancelled));
        assert_eq!(of("Cancelled: order #BL-48213"), Some(Cancelled));
        assert_eq!(of("Order #BL-48213 cancelled"), Some(Cancelled));
        assert_eq!(of("Order #BL-48213 canceled"), Some(Cancelled));
        assert_eq!(of("Your return is complete"), Some(Returned));
        let (s, _, in_subject) = status("Your return is complete", "We've received your return and issued a refund of $20.00.").unwrap();
        assert_eq!((s, in_subject), (Refunded, true), "a return refunded is a refund");
        assert_eq!(status("Update on order #BL-48213", "We've received your return and issued a refund of $20.00.").map(|s| s.0), Some(Refunded));
        assert_eq!(status("Update on order #BL-48213", "Your refund has been issued.").map(|s| s.0), Some(Refunded));
        // Not news.
        assert_eq!(of("Orders can be cancelled within 24 hours"), None);
    }

    #[test]
    fn a_promised_or_denied_refund_is_not_one() {
        use OrderStatus::*;
        let body = |subject: &str, b: &str| status(subject, b).map(|s| s.0);
        assert_eq!(body("Your return has started", "Order number: BL-48213\nWe'll issue your refund once we receive your item."), None);
        assert_eq!(body("Your return has started", "Once we receive your item, we will issue your refund."), None);
        assert_eq!(of("Return started: you'll be refunded when we receive your item"), None);
        assert_eq!(body("Your order #BL-1", "Your order has not been refunded yet."), None);
        assert_eq!(of("Your order hasn't been refunded"), None);
        assert_eq!(of("Your order will be cancelled if payment fails"), None);
        // Asked for, not issued.
        assert_eq!(of("We received your refund request for order #BL-48213"), None);
        assert_eq!(status("Order #BL-48213: your refund request", "Our team will review it.").map(|s| s.0), None);
        assert_eq!(body("Update on order #BL-48213", "Your refund is being processed."), None);
        assert_eq!(body("Update on order #BL-48213", "Check your refund status online."), None);
        // Still news.
        assert_eq!(of("You've been refunded for order #BL-48213"), Some(Refunded));
        assert_eq!(body("Update on order #BL-48213", "We couldn't ship it, so your order has been cancelled."), Some(Cancelled));
        assert_eq!(body("Update on order #BL-48213", "We've received your item and your refund has been issued."), Some(Refunded));
    }

    #[test]
    fn held_for_pickup_is_ready_for_pickup() {
        use OrderStatus::*;
        assert_eq!(of("Your UPS package is being held at a UPS Access Point for pickup"), Some(ReadyForPickup));
        assert_eq!(of("Your parcel is held at the post office locker"), Some(ReadyForPickup));
        let b = status("Update on order #BL-48213", "Your package is held at the Brightleaf store, ready to pick up.").map(|s| s.0);
        assert_eq!(b, Some(ReadyForPickup));
        // Held anywhere else is still a problem.
        assert_eq!(of("Your parcel is held at customs"), Some(Exception));
    }

    #[test]
    fn a_delayed_parcel_that_arrived_is_delivered() {
        use OrderStatus::*;
        assert_eq!(of("Your delayed package has been delivered"), Some(Delivered));
        assert_eq!(of("Your delayed order is out for delivery"), Some(OutForDelivery));
        assert_eq!(of("Delayed order #PP-77310"), Some(Delayed));
        assert_eq!(of("Your package is delayed"), Some(Delayed));
    }

    #[test]
    fn a_promised_day_is_not_news() {
        use OrderStatus::*;
        assert_eq!(of("Order confirmed #BL-48213 - delivered by Friday"), Some(Ordered));
        assert_eq!(status("Your Brightleaf order #BL-48213", "Get it delivered by Friday, Oct 23.").map(|s| s.0), None);
        assert_eq!(status("Your Brightleaf order #BL-48213", "Thank you for your order! Get it delivered by Oct 23.").map(|s| s.0), Some(Ordered));
        assert_eq!(status("Your Brightleaf order #BL-48213", "Shipped by 10/23 if you order today.").map(|s| s.0), None);
        // Still news.
        assert_eq!(of("Your package was delivered by the courier"), Some(Delivered));
        assert_eq!(of("Delivered by UPS: your order #BL-48213"), Some(Delivered));
        assert_eq!(of("Your order shipped by UPS"), Some(Shipped));
    }

    #[test]
    fn the_subject_beats_the_body_and_the_footer_is_never_read() {
        let (s, phrase, in_subject) = status("Your order has shipped", "Your package was delivered yesterday.").unwrap();
        assert_eq!((s, phrase.as_str(), in_subject), (OrderStatus::Shipped, "has shipped", true));
        let (s, _, in_subject) = status("Update on order 123", "Good news: your package was delivered.").unwrap();
        assert_eq!((s, in_subject), (OrderStatus::Delivered, false));
        let footer = format!("Hello,\n{}\nTrack your past orders. Delivered with care. Shipped from our warehouse.", "Lorem ipsum dolor sit amet. ".repeat(200));
        assert_eq!(status("Our autumn catalogue", &footer), None);
    }
}
