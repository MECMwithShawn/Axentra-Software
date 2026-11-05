//! One message to an [`Extraction`]: scam gates first, then the order
//! number, tracking numbers, status, expected date and items, and a
//! confidence built from the reasons.

use super::carriers::{carrier_sender, claimed_carrier, sends_for, tracking};
use super::dates::expected;
use super::status::status_of;
use super::{Carrier, Extraction, ExtractionKind, Input, MAX_ITEMS, Merchant, OrderReason, SuppressReason, TrackingNumber, Validation};
use crate::junk::analyze::{Severity, registrable};
use crate::junk::domain_of;
use crate::model::{EmailAddress, Message};
use crate::scan::{Prepared, contains_word, find_word, floor_char, has_unsubscribe, local_part_is, phone_word_before, prepare};
use crate::topics::is_freemail;

/// An order email needs at least this confidence.
const THRESHOLD: f32 = 0.70;

/// Order number labels, in the order they are tried.
const ORDER_LABELS: &[&str] = &[
    "order #",
    "order number",
    "order no",
    "order no.",
    "order id",
    "order:",
    "your order",
    "confirmation number",
    "bestellnummer",
    "bestellung nr",
    "numéro de commande",
    "numero de commande",
    "commande n",
    "número de pedido",
    "numero de pedido",
    "pedido n",
    "pedido #",
    "numero ordine",
    "bestelnummer",
    "注文番号",
    "ご注文番号",
    "订单号",
    "訂單編號",
    "주문번호",
];

/// The order label that bookings use too (see [`confirmation_counts`]).
const CONFIRMATION_LABEL: &str = "confirmation number";
/// Words of mail about an order.
const ORDER_WORDS: &[&str] = &["order", "orders", "purchase", "bestellung", "commande", "pedido"];
/// Words of a booking: a stay, a flight, a rental.
const BOOKING: &[&str] = &[
    "reservation",
    "booking",
    "booked",
    "flight",
    "check-in",
    "check in",
    "itinerary",
    "boarding",
    "hotel",
    "airline",
    "car rental",
    "rental car",
];

/// Label occurrences tried per text at most.
const MAX_LABEL_HITS: usize = 200;

/// Sender local parts of order and shipping mail.
const TRANSACTIONAL: &[&str] =
    &["order", "orders", "shipping", "shipment", "delivery", "noreply", "no-reply", "auto-confirm", "store", "shop", "sales", "customerservice", "tracking"];

/// Words of an order with nothing to ship: tickets, a meal.
const NOT_A_PARCEL: &[&str] = &[
    "tickets",
    "your ticket",
    "e-ticket",
    "eticket",
    "admission",
    "box office",
    "restaurant",
    "your food",
    "your meal",
    "picked up your food",
];
/// Words that a parcel is coming after all (the ticket shop's merchandise).
const SHIPPING: &[&str] = &["package", "parcel", "shipment", "shipping", "shipped", "dispatched", "tracking number"];
/// Ticket sellers and food delivery services (registrable domains).
const NOT_A_PARCEL_SENDERS: &[&str] = &[
    "eventbrite.com",
    "ticketmaster.com",
    "livenation.com",
    "seetickets.com",
    "stubhub.com",
    "axs.com",
    "dice.fm",
    "doordash.com",
    "ubereats.com",
    "grubhub.com",
    "seamless.com",
    "postmates.com",
    "deliveroo.com",
    "deliveroo.co.uk",
    "just-eat.com",
    "just-eat.co.uk",
    "lieferando.de",
];

/// Subject words of promotions.
const PROMO: &[&str] = &["% off", "sale", "deal", "new arrivals", "save", "coupon", "gutschein", "soldes", "oferta"];

/// Marketplaces that relay mail for many shops: orders are keyed by the
/// shop's name instead of the domain.
const MARKETPLACE_RELAYS: &[&str] = &["shops.example", "mailer.example"];

/// Words of a parcel notice: a carrier's name is only a claim to be that
/// carrier next to one of these (or a tracking number).
const PARCEL: &[&str] = &[
    "package",
    "parcel",
    "shipment",
    "shipping",
    "shipped",
    "delivery",
    "deliver",
    "delivered",
    "redelivery",
    "tracking",
    "track",
    "courier",
    "customs",
    "on hold",
    "fee",
    "paket",
    "sendung",
    "zustellung",
    "colis",
    "livraison",
    "paquete",
];

/// For the fee lure: words of a parcel held back, and of a request for
/// money, at most [`LURE_NEAR`] sentences apart.
const LURE_HOLD: &[&str] = &[
    "on hold",
    "held",
    "redelivery",
    "re-delivery",
    "reschedule",
    "customs",
    "could not be delivered",
    "couldn't deliver",
    "could not deliver",
    "unable to deliver",
    "delivery attempt",
    "delivery attempted",
    "attempted delivery",
    "missed delivery",
    "address incomplete",
    "incomplete address",
    "unpaid",
    "zoll",
    "douane",
    "aduana",
];
const LURE_PAY: &[&str] = &["pay", "pay now", "fee", "fees", "charge", "duty", "duties", "card details", "gebühr", "zahlen", "frais", "payer", "pagar"];
const LURE_NEAR: usize = 2;
/// A shop's note about duties abroad, not a demand.
const LURE_MAYBE: &[&str] = &["may apply", "may be subject", "may be charged"];

#[cfg(test)]
thread_local! {
    /// Makes [`extract`] panic, to test [`extract_guarded`].
    pub(super) static PANIC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// What `input` says about an order. Never panics on any input (see
/// [`extract_guarded`] for the belt to these braces).
pub fn extract(input: &Input) -> Extraction {
    #[cfg(test)]
    if PANIC.with(|p| p.get()) {
        panic!("extract: test panic");
    }
    let m = input.message;
    let p = prepare(m, input.html, input.text);
    let from = m.from.address.trim().to_ascii_lowercase();
    let host = domain_of(&from).to_string();
    let domain = registrable(&host);
    let out = Extraction { merchant: merchant_of(&m.from, &p), truncated: p.truncated, ..Extraction::none() };
    let suppress = |out: Extraction, why: SuppressReason| Extraction { kind: ExtractionKind::Suppressed, suppressed: Some(why), ..out };

    // Gates: never an order, whatever else the message says.
    if input.ignored {
        return suppress(out, SuppressReason::IgnoredSender);
    }
    // An allow rule only overrides the check's own findings, never your
    // phishing report.
    if input.reported_phishing || (input.junk == Some(Severity::Danger) && !input.sender_allowed) {
        return suppress(out, SuppressReason::Danger);
    }
    let (all_tracking, capped) = tracking(&p, &from, None);
    let (order_number, tracking) = numbers(&p, all_tracking);
    // A carrier's From address only counts when the provider vouched for
    // it: anyone can write "UPS <alerts@ups.com>".
    let carrier = carrier_sender(&domain).is_some();
    let carrier_mail = carrier && input.sender_authenticated;
    // A shop whose parcels a carrier's own mail has confirmed, under its
    // own domain: never a free-mail domain (anyone can write from one), a
    // relay, or a carrier's (its own mail vouches for itself, each time).
    let known_shop = input.trusted_merchant && out.merchant.key == out.merchant.domain && !is_freemail(&out.merchant.domain) && !carrier;
    if let Some(why) = scam(&m.from.name, &p, &domain, order_number.is_some(), &tracking, carrier_mail || known_shop) {
        return suppress(out, why);
    }

    let status = status_of(&p);
    let mut out = Extraction {
        expected: expected(&p, m.received_at, &host),
        items: items(&p),
        truncated: out.truncated || capped,
        ..out
    };

    let mut confidence = 0.0;
    let mut reasons = Vec::new();
    if let Some(n) = &order_number {
        confidence += 0.45;
        reasons.push(OrderReason::OrderNumber { label: n.clone() });
    }
    let has = |v: Validation| tracking.iter().any(|t| t.validated == v);
    confidence += if has(Validation::CheckDigit) {
        0.50
    } else if has(Validation::Link) {
        0.40
    } else if has(Validation::Labelled) {
        0.30
    } else {
        0.0
    };
    for t in &tracking {
        let check_digit = tracking.iter().any(|u| u.carrier == t.carrier && u.validated == Validation::CheckDigit);
        let r = OrderReason::Tracking { carrier: t.carrier.clone(), check_digit };
        if !reasons.contains(&r) {
            reasons.push(r);
        }
    }
    if let Some((_, phrase, in_subject)) = &status {
        confidence += if *in_subject { 0.25 } else { 0.10 };
        reasons.push(OrderReason::StatusPhrase { phrase: phrase.clone(), in_subject: *in_subject });
    }
    if input.known_merchant {
        confidence += 0.15;
        reasons.push(OrderReason::KnownMerchant);
    }
    let local = from.split('@').next().unwrap_or("");
    let transactional = TRANSACTIONAL.iter().any(|w| local_part_is(local, w));
    if transactional {
        confidence += 0.10;
    }
    if carrier_mail {
        confidence += 0.20;
    }
    if transactional || carrier_mail {
        reasons.push(OrderReason::TransactionalSender);
    }
    let subject = p.subject.to_ascii_lowercase();
    let re = ["re:", "aw:", "sv:", "antw:"].iter().any(|r| subject.trim_start().starts_with(r));
    if re || (m.in_reply_to.is_some() && input.contacts.contains(&from)) {
        confidence -= 0.35;
        reasons.push(OrderReason::Reply);
    }
    // A big shop's receipt has a footer and a menu of departments too: an
    // order number with the news in the subject, or from an order address,
    // is no newsletter.
    let receipt = order_number.is_some() && (transactional || status.as_ref().is_some_and(|s| s.2));
    if tracking.is_empty() && !receipt && has_unsubscribe(&p) && (p.links.len() > 15 || PROMO.iter().any(|w| contains_word(&subject, w))) {
        confidence -= 0.30;
        reasons.push(OrderReason::NewsletterShape);
    }
    if tracking.is_empty() && not_a_parcel(&p, &domain) {
        confidence -= 0.25;
        reasons.push(OrderReason::NotAParcel);
    }
    match input.junk {
        Some(Severity::Caution) => {
            confidence -= 0.25;
            reasons.push(OrderReason::JunkCaution);
        }
        // Allowed by the user: no penalty, but it may only join an order
        // (see `merge::join_only`).
        Some(Severity::Danger) => reasons.push(OrderReason::JunkCaution),
        _ => {}
    }
    if input.own_addresses.contains(&from) {
        confidence -= 0.20;
        reasons.push(OrderReason::OwnMessage);
    }

    let confidence = f32::clamp(confidence, 0.0, 1.0);
    let found = order_number.is_some() || !tracking.is_empty();
    // (Sums of tenths, compared with room for rounding.)
    let order = confidence >= THRESHOLD - 1e-4 && found;
    // The fake-order scam: a shop's name on someone else's domain, with an
    // order number and a tracking number it made up (a check digit is
    // arithmetic anyone can do). Never an order, whatever it adds up to,
    // unless you allowed the sender.
    if order && input.junk == Some(Severity::Caution) && input.impersonation && !input.sender_allowed {
        return suppress(Extraction { confidence, reasons, ..out }, SuppressReason::Impersonation);
    }
    out.kind = if order { ExtractionKind::Order } else { ExtractionKind::None };
    out.carrier_verified = carrier_mail;
    out.confidence = confidence;
    out.reasons = reasons;
    out.order_number = order_number;
    out.tracking = tracking;
    out.status = status.map(|(s, _, _)| s);
    out
}

/// Tickets or a meal, not a parcel: from a ticket seller or food delivery
/// service, or saying so with no word of shipping.
fn not_a_parcel(p: &Prepared, domain: &str) -> bool {
    let says = |words: &[&str]| words.iter().any(|w| contains_word(&p.head_lower, w));
    NOT_A_PARCEL_SENDERS.contains(&domain) || (says(NOT_A_PARCEL) && !says(SHIPPING))
}

/// [`extract`], with a failure recorded as incomplete (and not an order)
/// instead of stopping the caller.
pub fn extract_guarded(input: &Input) -> Extraction {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| extract(input))).unwrap_or_else(|_| Extraction { incomplete: true, ..Extraction::none() })
}

/// The order number, and the tracking numbers that aren't it. A number
/// right after an order label is the order number unless its shape alone
/// names a carrier (UPS, USPS, S10): a 10-digit DHL or 12-digit FedEx check
/// digit holds for one in 7 or 10 of any digits, so a shop's numeric order
/// number near "track your" or "DHL" passes for one.
fn numbers(p: &Prepared, all: Vec<TrackingNumber>) -> (Option<String>, Vec<TrackingNumber>) {
    let shaped: Vec<&str> = all
        .iter()
        .filter(|t| t.validated == Validation::CheckDigit && !matches!(t.carrier, Carrier::Dhl | Carrier::FedEx))
        .map(|t| t.number.as_str())
        .collect();
    let order_number = order_number(p, &shaped);
    let tracking = all.into_iter().filter(|t| Some(&t.number) != order_number.as_ref()).collect();
    (order_number, tracking)
}

/// The scam gates: a carrier's name on someone else's parcel notice, or a
/// fee demand. Whatever order number it quotes: one written-in "Your order
/// 12345" doesn't make a fee demand safe. Only `vouched` mail may talk
/// about fees: a carrier's own (authenticated as from its domain), or a
/// shop's on its own domain (not a name on a relay, not a free-mail
/// domain) with an order the carrier's own mail confirmed.
fn scam(from_name: &str, p: &Prepared, domain: &str, order_number: bool, tracking: &[TrackingNumber], vouched: bool) -> Option<SuppressReason> {
    if let Some(claimed) = impersonated(from_name, p, domain, order_number, tracking) {
        return Some(SuppressReason::CarrierImpersonation { claimed, domain: domain.to_string() });
    }
    (!vouched && fee_lure(p)).then_some(SuppressReason::DeliveryFeeLure)
}

/// The scam gates of [`extract`] on their own, for mail order detection
/// hasn't looked at (switched off, too old, or a sender not tracked), so
/// that a fee lure never sorts as an order email. `authenticated` as
/// `Input::sender_authenticated`.
pub(crate) fn scam_signs(m: &Message, p: &Prepared, authenticated: bool) -> Option<SuppressReason> {
    let from = m.from.address.trim().to_ascii_lowercase();
    let domain = registrable(domain_of(&from));
    let (all, _) = tracking(p, &from, None);
    let (order_number, tracking) = numbers(p, all);
    let carrier = carrier_sender(&domain).is_some() && authenticated;
    scam(&m.from.name, p, &domain, order_number.is_some(), &tracking, carrier)
}

/// The carrier a message pretends to be: named in the sender's name or at
/// the start of the subject of what reads as a parcel notice, from a domain
/// that isn't the carrier's. A shop's own shipping email that names its
/// carrier ("Shipped via UPS: order #BL-48213") is no claim.
fn impersonated(from_name: &str, p: &Prepared, domain: &str, order_number: bool, tracking: &[TrackingNumber]) -> Option<Carrier> {
    let (claimed, in_name) = claimed_carrier(from_name, &p.subject)?;
    if sends_for(&claimed, domain) {
        return None;
    }
    let parcel = |lower: &str| PARCEL.iter().any(|w| contains_word(lower, w));
    let notice = if in_name {
        parcel(&p.head_lower) || parcel(&from_name.to_ascii_lowercase()) || !tracking.is_empty()
    } else {
        let shop = order_number && !is_freemail(domain);
        let real_number = tracking.iter().any(|t| t.carrier == claimed && t.validated == Validation::CheckDigit);
        !shop && !real_number && (parcel(&p.subject.to_ascii_lowercase()) || !tracking.is_empty())
    };
    notice.then_some(claimed)
}

/// A parcel held back ("on hold", "customs", "redelivery") and a demand for
/// money a sentence or two from it, with a link to pay. A price
/// alone ("Free delivery on orders over £40", "Delivery Fee $2.99" on a
/// receipt) is not a demand. The whole body is read, not just its head:
/// padding (hidden or not) must not push the demand out of sight.
fn fee_lure(p: &Prepared) -> bool {
    let link = !p.links.is_empty() || p.body_lower.contains("http://") || p.body_lower.contains("https://") || p.body_lower.contains("www.");
    if !link {
        return false;
    }
    let subject = &p.head_lower[..p.subject.len().min(p.head_lower.len())];
    let sentences: Vec<&str> = sentences(subject).into_iter().chain(sentences(&p.body_lower)).collect();
    let pay = |s: &str| LURE_PAY.iter().any(|w| contains_word(s, w));
    sentences.iter().enumerate().any(|(i, s)| {
        LURE_HOLD.iter().any(|w| contains_word(s, w))
            && !LURE_MAYBE.iter().any(|w| s.contains(w))
            && sentences[i.saturating_sub(LURE_NEAR)..(i + LURE_NEAR + 1).min(sentences.len())].iter().any(|s| pay(s))
    })
}

/// The non-blank sentences of `s`: split at line ends, `!`, `?` and a `.`
/// before a space (not the one in "$1.99").
fn sentences(s: &str) -> Vec<&str> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut start = 0;
    for (i, &c) in b.iter().enumerate() {
        let end = matches!(c, b'\n' | b'!' | b'?') || (c == b'.' && b.get(i + 1).is_none_or(u8::is_ascii_whitespace));
        if end {
            out.push(&s[start..i]);
            start = i + 1;
        }
    }
    out.push(&s[start..]);
    out.retain(|s| !s.trim().is_empty());
    out
}

/// The labelled order number (uppercase, without a leading `#`) that isn't
/// one of `tracking`. The subject's wins over the body's.
pub(crate) fn order_number(p: &Prepared, tracking: &[&str]) -> Option<String> {
    let subject = p.subject.to_ascii_lowercase();
    let confirmation = confirmation_counts(p, !tracking.is_empty());
    [(p.subject.as_str(), subject.as_str()), (p.body.as_str(), p.body_lower.as_str())].into_iter().find_map(|(text, lower)| {
        ORDER_LABELS
            .iter()
            .filter(|l| confirmation || **l != CONFIRMATION_LABEL)
            .flat_map(|l| find_word(lower, l).map(move |i| i + l.len()))
            .take(MAX_LABEL_HITS)
            .find_map(|end| number_after(text, lower, end).filter(|n| !tracking.contains(&n.as_str())))
    })
}

/// "Confirmation number" is a hotel's and an airline's label too: it names
/// an order only in mail about one (an order word, or a parcel with a
/// tracking number), and never in a booking.
fn confirmation_counts(p: &Prepared, parcel: bool) -> bool {
    let says = |w: &&str| contains_word(&p.head_lower, w);
    !BOOKING.iter().any(says) && (parcel || ORDER_WORDS.iter().any(says))
}

/// The order number right after a label ending at `end`.
fn number_after(text: &str, lower: &str, end: usize) -> Option<String> {
    let skip = |c: char| c.is_whitespace() || matches!(c, ':' | '#' | '.' | '：' | '|' | '°' | 'º');
    let mut i = end;
    for _ in 0..3 {
        for (n, c) in lower[i..].chars().enumerate() {
            if n == 6 || !skip(c) {
                break;
            }
            i += c.len_utf8();
        }
        match ["no.", "nº", "n°", "is "].iter().find(|p| lower[i..].starts_with(*p)) {
            Some(p) => i += p.len(),
            None => break,
        }
    }
    let b = text.as_bytes();
    if !b.get(i).is_some_and(u8::is_ascii_alphanumeric) {
        return None;
    }
    let mut e = i;
    while e < b.len() && (b[e].is_ascii_alphanumeric() || b[e] == b'-') {
        e += 1;
    }
    let token = text[i..e].trim_end_matches('-');
    let e = i + token.len();
    let digits = token.bytes().filter(u8::is_ascii_digit).count();
    if !(4..=40).contains(&token.len()) || digits < 3 || token.eq_ignore_ascii_case("number") {
        return None;
    }
    // A date, a price or a phone number.
    let t = token.as_bytes();
    let iso = t.len() == 10 && t[4] == b'-' && t[7] == b'-' && t.iter().enumerate().all(|(k, c)| k == 4 || k == 7 || c.is_ascii_digit());
    let currency = text[..i].trim_end_matches(' ').ends_with(['$', '€', '£', '¥']);
    let cents = text[e..].strip_prefix('.').is_some_and(|r| {
        let r = r.as_bytes();
        r.len() >= 2 && r[0].is_ascii_digit() && r[1].is_ascii_digit() && !r.get(2).is_some_and(u8::is_ascii_digit)
    });
    let phone = digits == token.len() && (7..=15).contains(&digits) && phone_word_before(lower, i);
    if iso || currency || cents || phone {
        return None;
    }
    Some(token.to_ascii_uppercase())
}

/// Item names under an items heading, best effort.
pub(crate) fn items(p: &Prepared) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    // Lines left to read under the last heading.
    let mut under = 0;
    let mut prev = "";
    for line in p.body.lines().take(5000) {
        let l = line.trim();
        let lower = l.to_ascii_lowercase();
        if heading(&lower) {
            under = 60;
            prev = "";
            continue;
        }
        if under == 0 || l.is_empty() {
            continue;
        }
        under -= 1;
        if let Some(name) = item_row(l, &lower, prev) {
            let name = name[..floor_char(name, 120)].trim().to_string();
            if !out.contains(&name) {
                out.push(name);
            }
            if out.len() == MAX_ITEMS {
                break;
            }
        }
        prev = l;
    }
    out
}

/// An items heading, or the header row of an items table.
fn heading(lower: &str) -> bool {
    if lower.contains(" | ") {
        let cells: Vec<&str> = lower.split(" | ").map(str::trim).collect();
        return ["item", "items", "product", "description", "artikel", "article", "articles", "produit"].contains(&cells[0])
            && cells[1..].iter().any(|c| ["qty", "quantity", "menge", "anzahl", "qté", "quantité", "cantidad"].contains(c));
    }
    lower.len() <= 40 && ["items", "order summary", "your order", "you ordered", "artikel", "articles"].iter().any(|w| contains_word(lower, w))
}

/// The item a row names: the first cell of a row with a quantity cell, or
/// the text before (or the line above) "Qty:".
fn item_row<'a>(line: &'a str, lower: &str, prev: &'a str) -> Option<&'a str> {
    let name = if line.contains(" | ") {
        let cells: Vec<&str> = line.split(" | ").map(str::trim).collect();
        if !cells[1..].iter().any(|c| quantity(&c.to_ascii_lowercase())) {
            return None;
        }
        cells[0]
    } else {
        let at = lower.find("qty:").or_else(|| lower.find("quantity:"))?;
        let before = line[..at].trim().trim_end_matches(['-', '·', ',', '(', '|', '–']).trim();
        if before.is_empty() { prev } else { before }
    };
    let n = name.to_ascii_lowercase();
    let price_only = n.chars().all(|c| c.is_ascii_digit() || c.is_whitespace() || matches!(c, '.' | ',' | '$' | '€' | '£' | '¥' | '-'));
    let not_item = ["total", "subtotal", "tax", "shipping", "discount", "zwischensumme", "gesamt", "item", "items", "product", "qty", "ship to", "address"];
    if price_only || n.contains("http") || n.contains('@') || not_item.iter().any(|w| n.starts_with(w)) {
        return None;
    }
    Some(name)
}

/// A quantity cell: "2", "Qty 2", "Qty: 2", "× 2", "x2".
fn quantity(cell: &str) -> bool {
    let digits = |s: &str| (1..=3).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit());
    let rest = cell.strip_prefix("qty").or_else(|| cell.strip_prefix('×')).or_else(|| cell.strip_prefix('x'));
    digits(cell) || rest.is_some_and(|r| digits(r.trim_start_matches(':').trim()))
}

/// The key an order from `from` is filed under ([`Merchant::key`]): the
/// message's text only changes the name shown for a carrier's mail.
pub(crate) fn merchant_key(from: &EmailAddress) -> String {
    merchant_of(from, &Prepared::default()).key
}

/// Who the order is from.
pub(crate) fn merchant_of(from: &EmailAddress, p: &Prepared) -> Merchant {
    let address = from.address.trim().to_ascii_lowercase();
    let host = domain_of(&address).to_string();
    let domain = registrable(&host);
    if let Some(carrier) = carrier_sender(&domain) {
        // "Package from Brightleaf Home", else "Package from UPS".
        let name = shop_named_in(&p.head).unwrap_or_else(|| carrier.name());
        return Merchant { key: domain.clone(), domain, name };
    }
    let name = shop_name(&from.name).unwrap_or_else(|| {
        let label = domain.split('.').next().unwrap_or("");
        let mut c = label.chars();
        c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
    });
    // A relay's own domain: the shop's name alone. Any other `shops.` or
    // `mailer.` host: the name under that domain, since anyone can set up
    // `mailer.<anything>` and write a well-known shop's name.
    // A free-mail sender is a shop on its own: a seller at gmail.com says
    // nothing about anyone else there.
    let key = if is_freemail(&domain) {
        address
    } else if name.is_empty() {
        domain.clone()
    } else if MARKETPLACE_RELAYS.contains(&domain.as_str()) {
        format!("name:{}", name.to_lowercase())
    } else if host.starts_with("shops.") || host.starts_with("mailer.") {
        format!("name:{domain}:{}", name.to_lowercase())
    } else {
        domain.clone()
    };
    Merchant { domain, name, key }
}

/// The sender's name without "Orders", "Team", " via …" and the like.
fn shop_name(name: &str) -> Option<String> {
    let mut name = name[..floor_char(name, 120)].trim().trim_matches(['"', '\'']).trim();
    if let Some(i) = name.to_ascii_lowercase().find(" via ") {
        name = name[..i].trim();
    }
    loop {
        let lower = name.to_ascii_lowercase();
        let Some(s) = [" customer service", " orders", " order", " shipping", " team", " no-reply"].iter().find(|s| lower.ends_with(*s)) else { break };
        name = name[..name.len() - s.len()].trim();
    }
    (!name.is_empty()).then(|| name.to_string())
}

/// The shop a carrier's message names: "from {X}", "shipped by {X}",
/// "Shipper: {X}", "at the request of {X}" or "your {X} order"; never the
/// carrier itself ("A message from UPS").
fn shop_named_in(head: &str) -> Option<String> {
    let lower = head.to_ascii_lowercase();
    for label in ["from:", "shipper:", "from ", "shipped by ", "request of "] {
        for i in find_word(&lower, label) {
            let rest = head[i + label.len()..].trim_start();
            if let Some(name) = capitalised(rest.lines().next().unwrap_or(""))
                && claimed_carrier(&name, "").is_none()
            {
                return Some(name);
            }
        }
    }
    for i in find_word(&lower, "your ") {
        let rest = &head[i + 5..];
        if let Some(name) = capitalised(rest)
            && rest[name.len()..].to_ascii_lowercase().starts_with(" order")
        {
            return Some(name);
        }
    }
    None
}

/// Up to four capitalised words at the start of `s` ("Brightleaf Home").
fn capitalised(s: &str) -> Option<String> {
    let mut end = 0;
    for (n, word) in s.split(' ').enumerate() {
        let w = word.trim_end_matches(['.', ',', '!', ':', ';']);
        if n == 4 || w.is_empty() || !(w.starts_with(char::is_uppercase) || w == "&") || end + w.len() > 40 {
            break;
        }
        end += if n == 0 { w.len() } else { 1 + w.len() };
        if w.len() < word.len() {
            break;
        }
    }
    (end > 0).then(|| s[..end].trim_end_matches(" &").to_string())
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use chrono::NaiveDate;

    use super::super::fixtures::{self, Known, Mail, run};
    use super::super::merge::join_only;
    use super::super::{Carrier, OrderStatus};
    use super::*;

    fn order(mail: &Mail) -> Extraction {
        let e = run(mail, &Known::default());
        assert_eq!(e.kind, ExtractionKind::Order, "{}: {:?}", mail.message.subject, e.reasons);
        e
    }

    fn date(m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, d).unwrap()
    }

    #[test]
    fn order_emails_through_their_life() {
        let c = order(&fixtures::confirmation());
        assert_eq!(c.order_number.as_deref(), Some("BL-48213"));
        assert_eq!(c.status, Some(OrderStatus::Ordered));
        assert_eq!(c.items, ["Linen throw blanket", "Stoneware mug set"]);
        assert_eq!(c.merchant, Merchant { domain: "brightleaf.example".into(), name: "Brightleaf Home".into(), key: "brightleaf.example".into() });
        assert!(c.tracking.is_empty() && c.confidence >= 0.7 && c.version == super::super::EXTRACT_VERSION);

        let s = order(&fixtures::shipped_ups());
        assert_eq!((s.order_number.as_deref(), s.status), (Some("BL-48213"), Some(OrderStatus::Shipped)));
        assert_eq!(s.tracking.len(), 1);
        assert_eq!((&s.tracking[0].carrier, s.tracking[0].number.as_str(), s.tracking[0].validated), (&Carrier::Ups, "1Z4X7Q203660123459", Validation::CheckDigit));
        assert_eq!(s.expected.map(|w| (w.from, w.to)), Some((date(10, 20), date(10, 20))));
        assert!(s.reasons.contains(&OrderReason::Tracking { carrier: Carrier::Ups, check_digit: true }));

        let o = order(&fixtures::out_for_delivery());
        assert_eq!(o.status, Some(OrderStatus::OutForDelivery));
        assert_eq!(o.tracking[0].number, "1Z4X7Q203660123459");

        let d = order(&fixtures::delivered());
        assert_eq!((d.order_number.as_deref(), d.status), (Some("NP-20931"), Some(OrderStatus::Delivered)));
        assert_eq!(d.tracking[0].carrier, Carrier::FedEx);

        assert_eq!(order(&fixtures::not_delivered()).status, Some(OrderStatus::Exception));
        let late = order(&fixtures::delayed());
        assert_eq!((late.status, late.tracking[0].carrier.clone()), (Some(OrderStatus::Delayed), Carrier::Usps));
        assert_eq!(late.expected.map(|w| w.from), Some(date(10, 23)));
        assert_eq!(order(&fixtures::cancelled()).status, Some(OrderStatus::Cancelled));
        let r = order(&fixtures::refund());
        assert_eq!((r.order_number.as_deref(), r.status), (Some("HL-3304"), Some(OrderStatus::Refunded)));
    }

    #[test]
    fn promotions_and_chats_are_not_orders() {
        let p = run(&fixtures::promo(), &Known::default());
        assert_eq!(p.kind, ExtractionKind::None, "{:?}", p.reasons);
        let mut known = Known::default();
        known.contacts.insert("dana@home.example".into());
        let c = run(&fixtures::contact_chat(), &known);
        assert_eq!((c.kind, c.order_number.as_deref()), (ExtractionKind::None, None));
    }

    #[test]
    fn tickets_and_meals_are_not_parcels() {
        let tickets = Mail::new("Eventbrite", "noreply@order.eventbrite.com", "Order Confirmation for Jazz Night").text("Order #8123456789 Your tickets are attached.");
        let meal = Mail::new("DoorDash", "no-reply@doordash.com", "Order Confirmation for Bob from Chipotle")
            .text("Order #A1B2C3D4E5 Your order has been delivered. Total $23.10");
        let words = Mail::new("Lakeside Theatre", "noreply@lakeside-theatre.example", "Order confirmation #LT-55120").text("Order #LT-55120. Your tickets are attached. Admission from 7pm.");
        for m in [&tickets, &meal, &words] {
            let e = run(m, &Known::default());
            assert_eq!(e.kind, ExtractionKind::None, "{}: {} {:?}", m.message.subject, e.confidence, e.reasons);
            assert!(e.reasons.contains(&OrderReason::NotAParcel));
        }
        // A ticket shop's merchandise is shipped.
        let shirt = Mail::new("Lakeside Theatre", "noreply@lakeside-theatre.example", "Order confirmation #LT-55121")
            .text("Order #LT-55121. Your tour shirt ships soon. We'll email you the tracking number when your package has shipped.");
        assert_eq!(run(&shirt, &Known::default()).kind, ExtractionKind::Order);
    }

    #[test]
    fn other_languages_are_orders() {
        let j = order(&fixtures::japanese());
        assert_eq!(j.order_number.as_deref(), Some("SZ-100245"));
        assert_eq!((j.tracking[0].carrier.clone(), j.tracking[0].number.as_str()), (Carrier::JapanPost, "EJ517382041JP"));
        assert_eq!(j.status, Some(OrderStatus::Shipped));
        assert_eq!(j.expected.map(|w| w.from), Some(date(10, 22)));
        assert_eq!(j.merchant.name, "さくら雑貨");

        let g = order(&fixtures::german());
        assert_eq!(g.order_number.as_deref(), Some("BW-20931-77"));
        assert_eq!((g.tracking[0].carrier.clone(), g.tracking[0].validated), (Carrier::Dhl, Validation::Labelled));
        assert_eq!(g.status, Some(OrderStatus::Shipped));

        let t = run(&fixtures::turkish(), &Known::default());
        assert_eq!(t.order_number.as_deref(), Some("TR-55821"));
    }

    #[test]
    fn scams_are_suppressed() {
        let none = Known::default();
        let imp = run(&fixtures::carrier_impersonation(), &none);
        assert_eq!(imp.kind, ExtractionKind::Suppressed);
        assert_eq!(imp.suppressed, Some(SuppressReason::CarrierImpersonation { claimed: Carrier::Ups, domain: "ups-redelivery.example".into() }));
        assert_eq!(run(&fixtures::fee_lure(), &none).suppressed, Some(SuppressReason::DeliveryFeeLure));
        assert!(run(&fixtures::fee_lure(), &none).keys(1).is_empty());

        let danger = Known { junk: Some(Severity::Danger), ..Default::default() };
        let d = run(&fixtures::shipped_ups(), &danger);
        assert_eq!((d.kind, d.suppressed), (ExtractionKind::Suppressed, Some(SuppressReason::Danger)));
        let allowed = Known { junk: Some(Severity::Danger), allowed: true, ..Default::default() };
        assert_eq!(run(&fixtures::shipped_ups(), &allowed).kind, ExtractionKind::Order);
        // Your phishing report beats an allow rule.
        let reported = Known { junk: Some(Severity::Danger), reported: true, allowed: true, ..Default::default() };
        assert_eq!(run(&fixtures::shipped_ups(), &reported).suppressed, Some(SuppressReason::Danger));
        let ignored = Known { ignored: true, ..Default::default() };
        assert_eq!(run(&fixtures::confirmation(), &ignored).suppressed, Some(SuppressReason::IgnoredSender));

        // A real carrier's own mail about a fee is not a lure.
        let ups = Mail::new("UPS", "mcinfo@ups.com", "UPS Update: duty fee due").text("Pay import duties of $12.40 for tracking number 1Z999AA10123456784 at https://www.ups.com/");
        assert_eq!(run(&ups, &none).suppressed, None);
    }

    #[test]
    fn a_carriers_address_counts_only_when_vouched_for() {
        let lure = Mail::new("UPS", "alerts@ups.com", "UPS: your package is on hold").html(
            "<p>Your package 1Z999AA10123456784 is on hold at customs. Pay the $2.99 customs fee to release it: \
             <a href=https://ups-customs-pay.example/pay>Pay now</a></p>",
        );
        // Anyone can write that From address.
        let forged = run(&lure, &Known::default());
        assert_eq!((forged.kind, forged.suppressed), (ExtractionKind::Suppressed, Some(SuppressReason::DeliveryFeeLure)));
        let verified = Known { authenticated: true, ..Default::default() };
        let real = run(&lure, &verified);
        assert_eq!((real.kind, real.suppressed, real.carrier_verified), (ExtractionKind::Order, None, true));
        // The carrier bonus too: a body-only status from an unverified
        // carrier address is no order on its own.
        let note = Mail::new("UPS", "mcinfo@ups.com", "UPS Update").text("Your package is on its way. Tracking number 1Z999AA10123456784");
        let unverified = run(&note, &Known::default());
        assert!(!unverified.carrier_verified && !unverified.reasons.contains(&OrderReason::TransactionalSender), "{unverified:?}");
        assert!(run(&note, &verified).confidence > unverified.confidence);
        // A carrier is never a trusted shop: its mail vouches for itself, each time.
        let trusted = Known { trusted: true, merchant: true, ..Default::default() };
        assert_eq!(run(&lure, &trusted).suppressed, Some(SuppressReason::DeliveryFeeLure));
    }

    #[test]
    fn a_carriers_scheduled_delivery_is_news() {
        let body = "A message from UPS.\nScheduled Delivery Date: Thursday, 10/22/2026\nTracking Number: 1Z999AA10123456784\nFrom: FERN VALLEY\nShip To: Someone";
        let mail = Mail::new("UPS", "mcinfo@ups.com", "UPS Update: Package Scheduled for Delivery Tomorrow").text(body);
        // Microsoft accounts have no authentication results, so this
        // counts without the carrier's vouching too.
        let x = run(&mail, &Known::default());
        assert_eq!((x.kind, x.status), (ExtractionKind::Order, Some(OrderStatus::InTransit)), "{x:?}");
        assert_eq!(x.merchant.name, "FERN VALLEY");
        assert!(x.expected.is_some());
        let shipper = Mail::new("UPS", "mcinfo@ups.com", "UPS Ship Notification").text("At the request of Fern Valley, this notice alerts you.\nShipper:\nFern Valley\nTracking Number: 1Z999AA10123456784");
        assert_eq!(run(&shipper, &Known { authenticated: true, ..Default::default() }).merchant.name, "Fern Valley");
    }

    #[test]
    fn a_big_shops_receipt_is_not_a_newsletter() {
        let departments: String = (0..16).map(|i| format!("<a href=\"https://bigmart.example/d{i}\">Department {i}</a> ")).collect();
        let footer = "<p><a href=\"https://bigmart.example/unsubscribe\">Unsubscribe</a></p>";
        let receipt = |subject: &str| {
            Mail::new("BigMart", "orders@bigmart.example", subject).html(&format!("<p>{departments}</p><p>Order number: 1029384756</p><p>Thanks for shopping.</p>{footer}"))
        };
        for subject in ["Thanks for your order", "Order confirmed: #1029384756"] {
            let x = run(&receipt(subject), &Known::default());
            assert_eq!(x.kind, ExtractionKind::Order, "{subject}: {x:?}");
            assert!(!x.reasons.contains(&OrderReason::NewsletterShape), "{subject}");
        }
        // From an order address: no newsletter, though nothing says what happened.
        assert!(!run(&receipt("Your BigMart receipt"), &Known::default()).reasons.contains(&OrderReason::NewsletterShape));
        // A promotion that only talks about orders is still one.
        let promo = Mail::new("BigMart", "deals@bigmart.example", "Thanks for your order? Save 20% on the next one")
            .html(&format!("<p>{departments}</p><p>Thank you for your order last month!</p>{footer}"));
        let x = run(&promo, &Known::default());
        assert_eq!(x.kind, ExtractionKind::None);
    }

    #[test]
    fn naming_a_carrier_is_not_a_claim_to_be_one() {
        let none = Known::default();
        let not_suppressed = |mail: Mail| {
            let e = run(&mail, &none);
            assert_eq!(e.suppressed, None, "{}", mail.message.subject);
            e
        };
        not_suppressed(Mail::new("Dana Lee", "dana@corp.example", "Follow-ups from today's meeting").text("Notes attached, see https://corp.example/notes"));
        not_suppressed(Mail::new("Jonas", "jonas@firma.example", "Ups, falscher Anhang").text("Hier ist die richtige Datei."));
        not_suppressed(Mail::new("Acme", "team@acme.example", "New sign-ups this week").text("42 new sign-ups. https://acme.example/stats"));
        not_suppressed(Mail::new("Daily", "news@daily.example", "Royal Mail strike: what it means").text("Deliveries will be slower this week. https://daily.example/a"));
        // A shop's shipping email names its carrier.
        let ups = not_suppressed(
            Mail::new("Brightleaf Home", "orders@brightleaf.example", "Shipped via UPS: order #BL-48213").text("Tracking number: 1Z999AA10123456784"),
        );
        assert_eq!((ups.kind, ups.order_number.as_deref()), (ExtractionKind::Order, Some("BL-48213")));
        let etsy = not_suppressed(
            Mail::new("Etsy Transactions", "transaction@etsy.example", "Your USPS shipment for order #1234567 is on its way").text("Your order #1234567 has shipped."),
        );
        assert_eq!(etsy.order_number.as_deref(), Some("1234567"));
        // Still a claim: the carrier's name on a parcel notice from elsewhere.
        let spoof = Mail::new("USPS", "info@usps-parcel.example", "Action required").text("Your package could not be delivered. Update your address: https://usps-parcel.example/a");
        assert_eq!(run(&spoof, &none).suppressed, Some(SuppressReason::CarrierImpersonation { claimed: Carrier::Usps, domain: "usps-parcel.example".into() }));
        let subject = Mail::new("Parcel Desk", "desk@parcel.example", "DHL: your parcel is waiting").text("Confirm at https://parcel.example/c");
        assert!(matches!(run(&subject, &none).suppressed, Some(SuppressReason::CarrierImpersonation { claimed: Carrier::Dhl, .. })));
    }

    #[test]
    fn prices_near_delivery_words_are_not_fee_lures() {
        let none = Known::default();
        let no_lure = |mail: Mail| assert_ne!(run(&mail, &none).suppressed, Some(SuppressReason::DeliveryFeeLure), "{}", mail.message.subject);
        no_lure(Mail::new("Northpeak", "news@northpeak.example", "Autumn sale").html(
            "<p>Free delivery on orders over £40.</p><p>New jackets are in.</p><p><a href=\"https://northpeak.example/unsubscribe\">Unsubscribe</a></p>",
        ));
        no_lure(Mail::new("Uber Eats", "noreply@ubereats.example", "Your receipt").html(
            "<p>Thanks for ordering.</p><p>Total $23.50</p><p>Delivery Fee $2.99</p><p><a href=\"https://ubereats.example/r\">View receipt</a></p>",
        ));
        no_lure(Mail::new("Sunny Trips", "deals@sunnytrips.example", "Autumn escapes").html(
            "<p>7-night package to Crete from $499.</p><p><a href=\"https://sunnytrips.example/crete\">Book now</a></p>",
        ));
        no_lure(Mail::new("Brightleaf Home", "shipping@brightleaf.example", "Your package is on its way").html(
            "<p>Tracking number: 1Z999AA10123456784</p><p>Order total: $45.00</p><p><a href=\"https://brightleaf.example/o\">View order</a></p>",
        ));
        no_lure(Mail::new("Kiln & Co", "orders@kiln.example", "Your order #KC-20931 has shipped").html(
            "<p>Shipping abroad? Import duties and customs fees may apply.</p><p><a href=\"https://kiln.example/o\">View order</a></p>",
        ));
    }

    #[test]
    fn padding_does_not_hide_a_fee_lure() {
        let lure = |padding: &str| {
            Mail::new("Parcel Desk", "noreply@parcel-desk.example", "Delivery attempted: order #PX-55120").html(&format!(
                "{padding}<p>Your parcel is on hold at customs. Pay a small fee of $2.99 to release it: \
                 <a href=\"https://pay-release.example/p\">Pay now</a></p>"
            ))
        };
        let filler = "Lorem ipsum dolor sit amet consectetur. ".repeat(120);
        for padding in [String::new(), format!("<div style=\"display:none\">{filler}</div>"), format!("<p>{filler}</p>")] {
            assert!(padding.len() < 4096 || padding.len() > 4500);
            let e = run(&lure(&padding), &Known::default());
            assert_eq!(e.suppressed, Some(SuppressReason::DeliveryFeeLure), "padding {}: {:?}", padding.len(), e.reasons);
            assert!(e.keys(1).is_empty());
        }
    }

    #[test]
    fn an_order_number_does_not_let_a_fee_lure_through() {
        let lure = Mail::new("Parcel Service", "noreply@parcel-help.example", "Your order 48213 could not be delivered").html(
            "<p>The parcel is on hold. Pay a small fee of $1.99 to schedule redelivery: <a href=\"https://parcel-help.example/pay\">Pay now</a></p>",
        );
        let e = run(&lure, &Known::default());
        assert_eq!(e.suppressed, Some(SuppressReason::DeliveryFeeLure), "{:?}", e.reasons);
        assert!(e.keys(1).is_empty());
        // A known shop's name on a `mailer.` host of someone else's domain
        // isn't that shop.
        let relay = Mail::new("Brightleaf Home", "x@mailer.attacker.example", "Delivery attempted: package on hold").html(
            "<p>Tracking number: 1Z999AA10123456784</p><p>Pay a small fee of $1.99 to release it.</p>\
             <p><a href=\"https://attacker.example/pay\">Pay now</a></p>",
        );
        let known = Known { merchant: true, trusted: true, ..Default::default() };
        assert_eq!(run(&relay, &known).suppressed, Some(SuppressReason::DeliveryFeeLure));
        // The shop itself, on its own domain, may ask for duties.
        let shop = Mail::new("Brightleaf Home", "orders@brightleaf.example", "Your order #BL-48213 is held at customs").html(
            "<p>Your parcel is held at customs. Please pay the import duty of $12.40.</p><p><a href=\"https://brightleaf.example/duty\">Pay duty</a></p>",
        );
        assert_eq!(run(&shop, &known).kind, ExtractionKind::Order);
        assert_eq!(run(&shop, &Known::default()).suppressed, Some(SuppressReason::DeliveryFeeLure), "not a shop the user knows yet");
        // Orders from it, but none a carrier confirmed: its own say-so
        // ("Order confirmed") doesn't make its fee demands safe.
        let unconfirmed = Known { merchant: true, ..Default::default() };
        assert_eq!(run(&shop, &unconfirmed).suppressed, Some(SuppressReason::DeliveryFeeLure));
    }

    #[test]
    fn a_free_mail_seller_vouches_for_nobody_else_there() {
        let seller = Mail::new("Jane's Pottery", "janespottery@gmail.com", "Your order #JP-1042 has shipped").text("Tracking number: 1Z999AA10123456784");
        let jane = run(&seller, &Known::default());
        assert_eq!((jane.kind, jane.merchant.key.as_str(), jane.merchant.domain.as_str()), (ExtractionKind::Order, "janespottery@gmail.com", "gmail.com"));
        assert_eq!(merchant_key(&seller.message.from), "janespottery@gmail.com");
        // A stranger at the same free-mail domain, even if it were taken
        // for a known (and trusted) shop, never gets past the fee-lure gate.
        // The same for a provider's country domain ("hotmail.co.uk").
        for domain in ["gmail.com", "hotmail.co.uk", "yahoo.fr", "mail.com"] {
            let lure = Mail::new("Parcel Notice", &format!("parcel.notice.4471@{domain}"), "Delivery delayed: order #PX-55120").html(
                "<p>Your order #PX-55120 is on hold at customs. Tracking number: 1Z4X7Q203660123459.</p>\
                 <p>Pay a small fee of $2.99 to release it: <a href=\"https://pay-release.example/p\">Pay now</a></p>",
            );
            let known = Known { merchant: true, trusted: true, ..Default::default() };
            let e = run(&lure, &known);
            assert_eq!(e.merchant.key, format!("parcel.notice.4471@{domain}"));
            assert_eq!(e.suppressed, Some(SuppressReason::DeliveryFeeLure), "{domain}: {:?}", e.reasons);
            assert!(e.keys(1).is_empty());
        }
    }

    #[test]
    fn a_fake_order_from_a_brand_name_is_never_an_order() {
        let fake = Mail::new("Amazon.com", "orders@amzn-account-services.example", "Your order #112-4839201-5512345 has shipped").html(
            "<p>Order total: $1,249.00</p><p>Tracking number: 1Z999AA10123456784</p>\
             <p>Didn't place this order? <a href=\"https://amzn-account-services.example/cancel\">Cancel it here</a></p>",
        );
        let posing = Known { junk: Some(Severity::Caution), impersonation: true, ..Default::default() };
        let e = run(&fake, &posing);
        assert_eq!((e.kind, e.suppressed.clone()), (ExtractionKind::Suppressed, Some(SuppressReason::Impersonation)), "{:?}", e.reasons);
        assert!(e.keys(1).is_empty());
        // A caution for something else keeps it an order (less sure).
        assert_eq!(run(&fake, &Known { junk: Some(Severity::Caution), ..Default::default() }).kind, ExtractionKind::Order);
        // Allowed by the user: theirs to decide.
        assert_eq!(run(&fake, &Known { allowed: true, ..posing }).kind, ExtractionKind::Order);
        // Mail that isn't about an order stays plain mail.
        let note = Mail::new("Amazon.com", "news@amzn-account-services.example", "Hello").text("Hello there");
        assert_eq!(run(&note, &Known { junk: Some(Severity::Caution), impersonation: true, ..Default::default() }).kind, ExtractionKind::None);
    }

    #[test]
    fn warning_signs_and_own_mail_lower_confidence() {
        let caution = Known { junk: Some(Severity::Caution), merchant: true, ..Default::default() };
        let t = run(&fixtures::tracking_only(), &caution);
        assert_eq!(t.kind, ExtractionKind::Order, "{:?}", t.reasons);
        assert!(join_only(&t), "tracking only under a caution can't start an order");
        assert!(!join_only(&run(&fixtures::tracking_only(), &Known::default())));

        let plain = run(&fixtures::confirmation(), &Known { merchant: true, ..Default::default() });
        let wary = run(&fixtures::confirmation(), &caution);
        assert_eq!(wary.kind, ExtractionKind::Order);
        assert!(wary.confidence < plain.confidence && wary.reasons.contains(&OrderReason::JunkCaution));

        let own = Known { own: vec!["orders@brightleaf.example".into()], ..Default::default() };
        let mine = run(&fixtures::confirmation(), &own);
        assert!(mine.reasons.contains(&OrderReason::OwnMessage));
        assert!(mine.confidence < run(&fixtures::confirmation(), &Known::default()).confidence);
    }

    #[test]
    fn links_give_numbers_but_are_never_kept() {
        let mail = Mail::new("Brightleaf Home", "orders@brightleaf.example", "Your order #BL-48213 has shipped")
            .html(r#"<p>It's on its way.</p><p><a href="https://www.ups.com/track?tracknum=1Z999AA10123456784">Track your package</a></p>"#);
        let e = order(&mail);
        assert_eq!(e.tracking[0].number, "1Z999AA10123456784");
        assert_eq!(e.tracking[0].validated, Validation::Link);
        let json = serde_json::to_string(&e).unwrap();
        assert!(!json.contains("http") && !json.contains("ups.com"), "{json}");
    }

    #[test]
    fn order_number_rules() {
        let n = |subject: &str, body: &str| {
            let m = crate::model::Message { subject: subject.into(), ..Default::default() };
            order_number(&prepare(&m, None, body), &[])
        };
        assert_eq!(n("Order #bl-48213", ""), Some("BL-48213".into()));
        assert_eq!(n("Your order number is 553-1234-9", ""), Some("553-1234-9".into()));
        assert_eq!(n("Order No. 778812", ""), Some("778812".into()));
        assert_eq!(n("Hi", "Numéro de commande : FR-99812"), Some("FR-99812".into()));
        assert_eq!(n("Hi", "Commande n° 4471-22"), Some("4471-22".into()));
        assert_eq!(n("Hi", "注文番号：A123456"), Some("A123456".into()));
        // The subject's number wins.
        assert_eq!(n("Order #AA-1111", "Order number: BB-2222"), Some("AA-1111".into()));
        // Not order numbers.
        assert_eq!(n("Order now and save", ""), None);
        assert_eq!(n("Your order has shipped", ""), None);
        assert_eq!(n("Order date: 2026-10-19", ""), None);
        assert_eq!(n("Order total: $125.00", ""), None);
        assert_eq!(n("Your order: 125.00 due", ""), None);
        assert_eq!(n("Order #12", ""), None);
        assert_eq!(n("Your order: call 5550199123", ""), None);
        assert_eq!(n("Order #1Z999AA10123456784", ""), Some("1Z999AA10123456784".into()));
        let m = crate::model::Message { subject: "Order #1Z999AA10123456784".into(), ..Default::default() };
        assert_eq!(order_number(&prepare(&m, None, ""), &["1Z999AA10123456784"]), None);
    }

    #[test]
    fn a_numeric_order_number_stays_the_order_number() {
        // 482915037 % 7 == 3: the DHL check digit holds, and "track your"
        // is a tracking label within 80 bytes.
        assert!(super::super::carriers::validate(&Carrier::Dhl, "4829150373"));
        let shipped = Mail::new("Acme", "orders@acme.example", "Your Acme order has shipped").text("Track your package below.\nOrder number: 4829150373");
        let e = order(&shipped);
        assert_eq!(e.order_number.as_deref(), Some("4829150373"));
        assert!(e.tracking.is_empty(), "{:?}", e.tracking);
        let confirmed = order(&Mail::new("Acme", "orders@acme.example", "Order confirmed").text("Thanks for your order.\nOrder number: 4829150373"));
        assert_eq!(e.keys(1), confirmed.keys(1), "one order");
        assert_eq!(e.keys(1), ["1:o:acme.example:4829150373"]);
        // The same for a twelve-digit number next to "FedEx".
        let n = (0..).map(|i| (482_915_037_000u64 + i).to_string()).find(|n| super::super::carriers::validate(&Carrier::FedEx, n)).unwrap();
        let fedex = order(&Mail::new("Acme", "orders@acme.example", "Your Acme order has shipped with FedEx").text(&format!("Order number: {n}")));
        assert_eq!((fedex.order_number.as_deref(), fedex.tracking.len()), (Some(n.as_str()), 0));
        // A number whose shape names its carrier is still a tracking number.
        let ups = order(&Mail::new("Acme", "orders@acme.example", "Your Acme order #A-1001 has shipped").text("Order: 1Z999AA10123456784"));
        assert_eq!((ups.order_number.as_deref(), ups.tracking.len()), (Some("A-1001"), 1));
    }

    #[test]
    fn bookings_are_not_orders() {
        let none = Known::default();
        let hotel = Mail::new("Harbor Hotel", "reservations@harborhotel.example", "Your reservation has been cancelled")
            .text("Confirmation number: 83920174\nCheck-in: Nov 3\nWe hope to see you another time.");
        let h = run(&hotel, &none);
        assert_eq!((h.kind, h.order_number.as_deref()), (ExtractionKind::None, None), "{:?}", h.reasons);
        let flight = Mail::new("Skyways", "noreply@skyways.example", "Your flight to Denver is delayed").text("Confirmation number: K7X9P2\nNew departure 6:40 PM.");
        assert_eq!(run(&flight, &none).kind, ExtractionKind::None);
        let refund = Mail::new("Skyways", "noreply@skyways.example", "Your refund has been processed").text("Booking confirmation number: K7X9P2");
        assert_eq!(run(&refund, &none).kind, ExtractionKind::None);
        // A shop's confirmation number still is one.
        let shop = order(&Mail::new("Acme", "orders@acme.example", "Your order has shipped").text("Confirmation number: AC-55120"));
        assert_eq!(shop.order_number.as_deref(), Some("AC-55120"));
    }

    #[test]
    fn merchants() {
        let p = prepare(&crate::model::Message::default(), None, "");
        let m = |name: &str, address: &str| merchant_of(&EmailAddress::new(name, address), &p);
        assert_eq!(m("Northpeak Outfitters Orders", "shop@mail.northpeak.example").name, "Northpeak Outfitters");
        assert_eq!(m("Brightleaf Team", "a@brightleaf.example").name, "Brightleaf");
        assert_eq!(m("", "a@brightleaf.example").name, "Brightleaf");
        assert_eq!(m("Kiln & Co via Shops", "a@shops.example").key, "name:kiln & co");
        assert_eq!(m("Kiln & Co", "a@mailer.market.example").key, "name:market.example:kiln & co");
        // A shop's name on someone else's `mailer.` host is another merchant.
        assert_eq!(m("Brightleaf Home", "x@mailer.attacker.example").key, "name:attacker.example:brightleaf home");
        let carrier = Message { subject: "Your package from Brightleaf Home is on its way".into(), ..Default::default() };
        let pc = prepare(&carrier, None, "");
        assert_eq!(merchant_of(&EmailAddress::new("UPS", "mcinfo@ups.com"), &pc).name, "Brightleaf Home");
        assert_eq!(merchant_of(&EmailAddress::new("UPS", "mcinfo@ups.com"), &p).name, "UPS");
        // The key alone (is this a shop with orders already?) is the same
        // whatever the message says.
        for (name, address) in [("Kiln & Co via Shops", "a@shops.example"), ("Brightleaf Home", "orders@brightleaf.example"), ("UPS", "mcinfo@ups.com")] {
            let from = EmailAddress::new(name, address);
            assert_eq!(merchant_key(&from), merchant_of(&from, &pc).key, "{address}");
        }
    }

    use crate::model::Message;

    /// Finishes without panicking, well inside a second.
    fn quick(mail: &Mail) -> Extraction {
        let started = Instant::now();
        let e = run(mail, &Known::default());
        assert!(started.elapsed() < Duration::from_secs(1), "{} took {:?}", mail.message.subject, started.elapsed());
        e
    }

    #[test]
    fn odd_and_huge_inputs_never_panic() {
        let huge = "<p>Order number: AB-12345 &amp; more <b>text</b></p>".repeat(5 * 1024 * 1024 / 48);
        assert!(quick(&Mail::new("A", "a@a.example", "Big").html(&huge)).truncated);
        let e = "é".repeat(200_000);
        assert!(quick(&Mail::new("A", "a@a.example", &e).text(&e)).truncated);
        quick(&Mail::new("A", "a@a.example", "é").html(&e));
        let cells = format!("<table><tr>{}</tr></table>", "<td>Item 1</td>".repeat(10_000));
        quick(&Mail::new("A", "a@a.example", "Cells").html(&cells));
        let cjk = "注文番号：ＡＢ－１２３４５６　発送しました。お届け予定日：１０月２２日".repeat(2000);
        assert_eq!(quick(&Mail::new("ショップ", "shop@a.example", "ご注文").text(&cjk)).order_number.as_deref(), Some("AB-123456"));
        quick(&Mail::new("", "", ""));
        quick(&Mail::new("A", "a@a.example", "<").html(&"<".repeat(100_000)));
        quick(&Mail::new("A", "a@a.example", "Unclosed").html("<div><p><a href=\"https://x.example/<td><table><tr><th"));
        quick(&Mail::new("A", "a@a.example", "Unclosed").html(&"<a href='https://x.example/'>".repeat(20_000)));
        quick(&Mail::new("A", "a@a.example", "Entities").html(&"&#x110000;&#99999999999;&#;&amp".repeat(10_000)));
        let many: String = (0..1000).map(|i| format!("1Z999AA1{i:07}{} ", i % 10)).collect();
        let t = quick(&Mail::new("A", "a@a.example", "Tracking").text(&format!("Tracking number: {many}")));
        assert!(t.truncated && t.tracking.len() <= super::super::MAX_TRACKING);
        let big = quick(&Mail::new("Brightleaf Home", "orders@brightleaf.example", "Your order #BL-48213 has shipped").text("This message is too large to download. Open it in your provider's web mail."));
        assert_eq!(big.order_number.as_deref(), Some("BL-48213"));
    }

    #[test]
    fn a_failure_is_recorded_as_incomplete() {
        let mail = fixtures::confirmation();
        let input = Input {
            message: &mail.message,
            html: mail.html.as_deref(),
            text: &mail.text,
            junk: None,
            impersonation: false,
            reported_phishing: false,
            sender_allowed: false,
            sender_authenticated: false,
            own_addresses: &[],
            contacts: &Default::default(),
            known_merchant: false,
            trusted_merchant: false,
            ignored: false,
        };
        PANIC.with(|p| p.set(true));
        let e = extract_guarded(&input);
        PANIC.with(|p| p.set(false));
        assert!(e.incomplete && e.kind == ExtractionKind::None && e.version == super::super::EXTRACT_VERSION);
        assert_eq!(extract_guarded(&input).kind, ExtractionKind::Order);
    }
}
