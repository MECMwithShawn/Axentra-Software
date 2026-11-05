//! Grouping the extractions of one account into orders, building each
//! order from its emails, and which section of Packages it belongs in.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Local, NaiveDate, Utc};

use super::{Extraction, ExtractionKind, Order, OrderEvent, OrderReason, OrderStatus, OrderUserState, Shipment, Validation};
use crate::model::{AccountId, MessageId};

/// How long delivered and cancelled orders stay listed, in days.
pub const KEEP_DAYS: [u32; 4] = [7, 30, 90, 365];

/// One message's extraction, for [`group`].
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub message_id: MessageId,
    pub account_id: AccountId,
    pub received_at: DateTime<Utc>,
    pub internet_message_id: String,
    pub subject: String,
    pub extraction: Extraction,
}

/// The messages of one order.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    /// Sorted.
    pub keys: Vec<String>,
    /// Sorted by `received_at`.
    pub entries: Vec<Entry>,
}

/// The sections of Packages, in display order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum OrderSection {
    NeedsAttention,
    ArrivingSoon,
    OnTheWay,
    Ordered,
    Delivered,
    CancelledReturned,
}

/// Messages kept per order at most (the newest).
const MAX_ENTRIES: usize = 200;
/// A non-final order with no news for this long counts as done.
const STALE_DAYS: i64 = 45;

/// True when `e` may join an order but not start one: tracking numbers
/// only, and none with a check digit or the message has warning signs.
pub(crate) fn join_only(e: &Extraction) -> bool {
    e.order_number.is_none() && (!e.tracking.iter().any(|t| t.validated == Validation::CheckDigit) || e.reasons.contains(&OrderReason::JunkCaution))
}

/// Order emails grouped by shared order and tracking keys.
pub fn group(entries: Vec<Entry>) -> Vec<Group> {
    // Order emails only, each copy of a message once (the earliest id).
    let mut entries: Vec<Entry> = entries.into_iter().filter(|e| e.extraction.kind == ExtractionKind::Order).collect();
    entries.sort_by_key(|e| e.message_id);
    let mut seen = HashSet::new();
    entries.retain(|e| e.internet_message_id.is_empty() || seen.insert(e.internet_message_id.clone()));
    entries.sort_by(|a, b| a.received_at.cmp(&b.received_at).then(a.message_id.cmp(&b.message_id)));

    let keys: Vec<Vec<String>> = entries.iter().map(|e| e.extraction.keys(e.account_id)).collect();
    let mut sets = Sets::new(entries.len());
    for (i, k) in keys.iter().enumerate() {
        for key in k {
            if let Some((merchant, number)) = order_key(key) {
                sets.orders[i].push((merchant.to_string(), number.to_string()));
            }
        }
    }
    let mut holders: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, k) in keys.iter().enumerate() {
        for key in k {
            let h = holders.entry(key.as_str()).or_default();
            for &j in h.iter() {
                sets.union(i, j);
            }
            h.push(i);
        }
    }

    let mut components: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..entries.len() {
        components.entry(sets.find(i)).or_default().push(i);
    }
    // A carrier's email about a shipment two orders share (no order number
    // of its own) is news for both, not only the one it was joined to.
    for (i, k) in keys.iter().enumerate() {
        if k.iter().any(|key| order_key(key).is_some()) {
            continue;
        }
        let own = sets.find(i);
        let mut roots: Vec<usize> = k.iter().flat_map(|key| holders[key.as_str()].iter().copied()).map(|j| sets.find(j)).filter(|r| *r != own).collect();
        roots.sort_unstable();
        roots.dedup();
        for r in roots {
            if let Some(members) = components.get_mut(&r) {
                members.push(i);
                members.sort_unstable();
            }
        }
    }
    // A group of join-only messages can't start an order on its own.
    let starts: Vec<bool> = entries.iter().map(|e| !join_only(&e.extraction)).collect();
    let mut out: Vec<Group> = Vec::new();
    for members in components.into_values() {
        if !members.iter().any(|&i| starts[i]) {
            continue;
        }
        let mut gk: Vec<String> = members.iter().flat_map(|&i| keys[i].iter().cloned()).collect();
        gk.sort();
        gk.dedup();
        // Members are in received order: keep the newest.
        let skip = members.len().saturating_sub(MAX_ENTRIES);
        let entries = members[skip..].iter().map(|&i| entries[i].clone()).collect();
        out.push(Group { keys: gk, entries });
    }
    out.sort_by(|a, b| a.keys.cmp(&b.keys));
    out
}

/// `(merchant key, order number)` of an `"{acct}:o:{merchant}:{number}"` key.
fn order_key(key: &str) -> Option<(&str, &str)> {
    let (_, rest) = key.split_once(":o:")?;
    rest.rsplit_once(':')
}

/// Union-find over entries that refuses to join two orders of one merchant.
struct Sets {
    parent: Vec<usize>,
    /// Per root: the `(merchant key, order number)` pairs of its entries.
    orders: Vec<Vec<(String, String)>>,
}

impl Sets {
    fn new(n: usize) -> Self {
        Self { parent: (0..n).collect(), orders: vec![Vec::new(); n] }
    }

    fn find(&mut self, mut i: usize) -> usize {
        while self.parent[i] != i {
            self.parent[i] = self.parent[self.parent[i]];
            i = self.parent[i];
        }
        i
    }

    fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a == b {
            return;
        }
        // Two orders sharing a consolidated shipment stay two orders.
        let clash = self.orders[a].iter().any(|(m, n)| self.orders[b].iter().any(|(m2, n2)| m == m2 && n != n2));
        if clash {
            return;
        }
        let moved = std::mem::take(&mut self.orders[b]);
        for o in moved {
            if !self.orders[a].contains(&o) {
                self.orders[a].push(o);
            }
        }
        self.parent[b] = a;
    }
}

/// Precedence of statuses at the same instant, lowest first (the status
/// vocabulary's priority order, reversed).
fn rank(s: OrderStatus) -> u8 {
    match s {
        OrderStatus::Ordered => 0,
        OrderStatus::Shipped => 1,
        OrderStatus::InTransit => 2,
        OrderStatus::ReadyForPickup => 3,
        OrderStatus::OutForDelivery => 4,
        OrderStatus::Delivered => 5,
        OrderStatus::Delayed => 6,
        OrderStatus::Exception => 7,
        OrderStatus::Returned => 8,
        OrderStatus::Cancelled => 9,
        OrderStatus::Refunded => 10,
    }
}

/// True when a later `next` replaces the current status `cur`. `dated`: the
/// message that says `next` also gives a delivery date.
fn replaces(cur: OrderStatus, next: OrderStatus, dated: bool) -> bool {
    use OrderStatus::*;
    match cur {
        // Late "Shipped" reminders never take a delivery back.
        Delivered => matches!(next, Exception | Returned | Refunded),
        // Part of a cancelled order can still ship.
        Cancelled => matches!(next, Shipped | InTransit | OutForDelivery | ReadyForPickup | Delivered | Refunded | Returned),
        // A reminder doesn't end a delay; a new date to go with it does.
        Delayed if dated && matches!(next, Shipped | InTransit) => true,
        Delayed | Exception => !matches!(next, Ordered | Shipped | InTransit),
        // A replacement can still ship, but a review request or a receipt
        // sent again ("Thank you for your order") never reopens the order.
        Refunded | Returned => !matches!(next, Ordered),
        // On the way: a problem, or a step at least as far along (a resent
        // receipt saying "Thank you for your order" never takes it back).
        // The driver couldn't deliver and left it at a pickup point.
        OutForDelivery if next == ReadyForPickup => true,
        _ => match (cur.step(), next.step()) {
            (Some(c), Some(n)) => n >= c,
            _ => true,
        },
    }
}

/// Folds the statuses of `events` (in order) into one, with when it was set.
fn fold<'a>(events: impl Iterator<Item = &'a &'a Entry>) -> Option<(OrderStatus, DateTime<Utc>)> {
    let mut cur: Option<(OrderStatus, DateTime<Utc>)> = None;
    for e in events {
        let Some(s) = e.extraction.status else { continue };
        if cur.is_none_or(|(c, _)| replaces(c, s, e.extraction.expected.is_some())) {
            cur = Some((s, e.received_at));
        }
    }
    cur
}

/// One order from its group, with the user's changes applied.
pub fn build_order(g: &Group, user: Option<&OrderUserState>) -> Order {
    let mut entries: Vec<&Entry> = g.entries.iter().collect();
    // In time order; at the same instant the higher-ranked status last, so it wins.
    entries.sort_by_key(|e| (e.received_at, e.extraction.status.map(rank), e.message_id));
    let first = entries.first().map(|e| e.received_at).unwrap_or_default();
    let mut o = Order {
        key: g.keys.iter().find(|k| k.contains(":o:")).or_else(|| g.keys.first()).cloned().unwrap_or_default(),
        keys: g.keys.clone(),
        account_id: entries.first().map(|e| e.account_id).unwrap_or_default(),
        status_at: first,
        updated_at: entries.last().map(|e| e.received_at).unwrap_or_default(),
        ..Order::default()
    };
    for e in &entries {
        let x = &e.extraction;
        o.events.push(OrderEvent { message_id: e.message_id, at: e.received_at, status: x.status, subject: e.subject.clone() });
        if o.merchant.name.is_empty() && !x.merchant.name.is_empty() {
            o.merchant = x.merchant.clone();
        }
        if o.order_number.is_none() {
            o.order_number = x.order_number.clone();
        }
        if o.items.is_empty() {
            o.items = x.items.clone();
        }
        if x.expected.is_some() {
            o.expected = x.expected;
        }
        if o.delivered_at.is_none() && x.status == Some(OrderStatus::Delivered) {
            o.delivered_at = Some(e.received_at);
        }
        for r in &x.reasons {
            if !o.reasons.contains(r) {
                o.reasons.push(r.clone());
            }
        }
        for t in &x.tracking {
            match o.shipments.iter_mut().find(|s| s.tracking.number == t.number) {
                Some(s) if t.validated == Validation::CheckDigit => s.tracking.validated = Validation::CheckDigit,
                Some(_) => {}
                None => o.shipments.push(Shipment { tracking: t.clone(), status: None, last_update: e.received_at }),
            }
        }
    }

    if let Some((status, at)) = fold(entries.iter()) {
        o.status = status;
        o.status_at = at;
    }
    // Each shipment from the emails that list it; an email that lists no
    // tracking number speaks for the whole order.
    let mut last_delivery: Option<DateTime<Utc>> = None;
    for s in &mut o.shipments {
        let number = s.tracking.number.clone();
        let lists = |e: &&&Entry| e.extraction.tracking.iter().any(|t| t.number == number);
        if let Some(last) = entries.iter().rev().find(lists) {
            s.last_update = last.received_at;
        }
        let folded = fold(entries.iter().filter(|e| e.extraction.tracking.is_empty() || lists(e)));
        if let Some((OrderStatus::Delivered, at)) = folded {
            last_delivery = last_delivery.max(Some(at));
        }
        s.status = folded.map(|(st, _)| st);
    }
    // Delivered when the last parcel arrived, not the first.
    if o.status == OrderStatus::Delivered
        && let Some(last) = last_delivery.filter(|last| *last > o.status_at)
    {
        o.status_at = last;
        o.delivered_at = Some(last);
    }
    let known: Vec<OrderStatus> = o.shipments.iter().filter_map(|s| s.status).collect();
    if o.status == OrderStatus::Delivered && known.len() >= 2 && known.iter().any(|s| *s != OrderStatus::Delivered) {
        o.status = OrderStatus::InTransit;
        o.partly_delivered = true;
    } else if known.len() >= 2 && known.contains(&OrderStatus::Delivered) && known.iter().any(|s| *s != OrderStatus::Delivered) {
        o.partly_delivered = true;
    }

    o.mail_status = Some(o.status);
    if let Some(u) = user {
        o.hidden = u.hidden;
        if let Some(s) = u.status_override {
            o.status = s;
            o.overridden = true;
        }
    }
    o
}

/// Where `o` shows in Packages; `None` when hidden or aged out after
/// `keep_days`.
pub fn section(o: &Order, now: DateTime<Utc>, today: NaiveDate, keep_days: u32) -> Option<OrderSection> {
    if o.hidden {
        return None;
    }
    let local = |t: DateTime<Utc>| t.with_timezone(&Local).date_naive();
    let keep = i64::from(keep_days);
    if o.status.is_terminal() {
        let at = if o.status == OrderStatus::Delivered { o.delivered_at.unwrap_or(o.status_at) } else { o.status_at };
        if (today - local(at)).num_days() > keep {
            return None;
        }
        return Some(match o.status {
            OrderStatus::Delivered if local(at) == today => OrderSection::ArrivingSoon,
            OrderStatus::Delivered | OrderStatus::Returned => OrderSection::Delivered,
            _ => OrderSection::CancelledReturned,
        });
    }
    // No news for weeks: shown as done ("No recent updates"), then aged out.
    let quiet = (now - o.updated_at).num_days();
    if quiet > STALE_DAYS {
        return (quiet <= STALE_DAYS + keep).then_some(OrderSection::Delivered);
    }
    let days = |d: NaiveDate| (d - today).num_days();
    Some(match o.status {
        OrderStatus::Delayed | OrderStatus::Exception => OrderSection::NeedsAttention,
        _ if o.expected.is_some_and(|w| days(w.to) < -2) => OrderSection::NeedsAttention,
        OrderStatus::OutForDelivery | OrderStatus::ReadyForPickup => OrderSection::ArrivingSoon,
        _ if o.expected.is_some_and(|w| days(w.from) <= 2) => OrderSection::ArrivingSoon,
        _ if o.partly_delivered => OrderSection::OnTheWay,
        OrderStatus::Shipped | OrderStatus::InTransit => OrderSection::OnTheWay,
        _ => OrderSection::Ordered,
    })
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::super::fixtures::{self, Known, Mail, monday, run};
    use super::super::{Carrier, DateWindow, Merchant, TrackingNumber};
    use super::*;

    fn entry(id: MessageId, at: DateTime<Utc>, extraction: Extraction) -> Entry {
        Entry { message_id: id, account_id: 1, received_at: at, internet_message_id: format!("{id}@shop.example"), subject: format!("Message {id}"), extraction }
    }

    fn mail(id: MessageId, at: DateTime<Utc>, m: &Mail) -> Entry {
        entry(id, at, run(m, &Known::default()))
    }

    /// An order email of the Brightleaf shop.
    fn ex(order: Option<&str>, tracking: &[&str], status: Option<OrderStatus>) -> Extraction {
        Extraction {
            kind: ExtractionKind::Order,
            merchant: Merchant { domain: "brightleaf.example".into(), name: "Brightleaf Home".into(), key: "brightleaf.example".into() },
            order_number: order.map(str::to_string),
            tracking: tracking.iter().map(|n| TrackingNumber { carrier: Carrier::Ups, number: n.to_string(), validated: Validation::CheckDigit }).collect(),
            status,
            confidence: 0.9,
            ..Extraction::none()
        }
    }

    fn days(n: i64) -> DateTime<Utc> {
        monday(12) - Duration::days(n)
    }

    #[test]
    fn an_orders_emails_make_one_order() {
        let carrier = Mail::new("UPS", "mcinfo@ups.com", "Your UPS package is out for delivery").text("Package from Brightleaf Home. Tracking number 1Z4X7Q203660123459");
        let delivered = Mail::new("Brightleaf Home", "orders@brightleaf.example", "Delivered: your order #BL-48213").text("Your package was delivered.");
        let entries = vec![
            mail(1, days(3), &fixtures::confirmation()),
            mail(2, days(2), &fixtures::shipped_ups()),
            mail(3, monday(9), &carrier),
            mail(4, monday(15), &delivered),
        ];
        assert_eq!(entries[2].extraction.keys(1), ["1:t:ups:1Z4X7Q203660123459"]);
        let groups = group(entries);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].keys, ["1:o:brightleaf.example:BL-48213", "1:t:ups:1Z4X7Q203660123459"]);
        let o = build_order(&groups[0], None);
        assert_eq!(o.status, OrderStatus::Delivered);
        assert_eq!(o.events.len(), 4);
        assert_eq!(o.key, "1:o:brightleaf.example:BL-48213");
        assert_eq!(o.merchant.name, "Brightleaf Home");
        assert_eq!(o.order_number.as_deref(), Some("BL-48213"));
        assert_eq!(o.items, ["Linen throw blanket", "Stoneware mug set"]);
        assert_eq!((o.delivered_at, o.status_at, o.updated_at), (Some(monday(15)), monday(15), monday(15)));
        assert_eq!(o.shipments.len(), 1);
        assert_eq!(o.shipments[0].status, Some(OrderStatus::Delivered));
        assert!(!o.partly_delivered);
        assert!(o.reasons.contains(&OrderReason::OrderNumber { label: "BL-48213".into() }));
        assert_eq!(section(&o, monday(16), monday(16).with_timezone(&Local).date_naive(), 30), Some(OrderSection::ArrivingSoon));
    }

    #[test]
    fn late_reminders_do_not_undo_a_delivery() {
        let entries = vec![
            entry(1, days(3), ex(Some("BL-1"), &[], Some(OrderStatus::Ordered))),
            entry(2, days(2), ex(Some("BL-1"), &["1Z999AA10123456784"], Some(OrderStatus::Delivered))),
            entry(3, days(1), ex(Some("BL-1"), &["1Z999AA10123456784"], Some(OrderStatus::Shipped))),
        ];
        let o = build_order(&group(entries)[0], None);
        assert_eq!((o.status, o.status_at), (OrderStatus::Delivered, days(2)));
        // A problem after delivery still shows.
        let mut entries = vec![entry(1, days(2), ex(Some("BL-1"), &[], Some(OrderStatus::Delivered)))];
        entries.push(entry(2, days(1), ex(Some("BL-1"), &[], Some(OrderStatus::Returned))));
        assert_eq!(build_order(&group(entries)[0], None).status, OrderStatus::Returned);
        // A delay is cleared by the delivery, not by a reminder.
        let entries = vec![entry(1, days(3), ex(Some("BL-1"), &[], Some(OrderStatus::Delayed))), entry(2, days(2), ex(Some("BL-1"), &[], Some(OrderStatus::InTransit)))];
        assert_eq!(build_order(&group(entries)[0], None).status, OrderStatus::Delayed);
        let entries = vec![entry(1, days(3), ex(Some("BL-1"), &[], Some(OrderStatus::Delayed))), entry(2, days(2), ex(Some("BL-1"), &[], Some(OrderStatus::OutForDelivery)))];
        assert_eq!(build_order(&group(entries)[0], None).status, OrderStatus::OutForDelivery);
        // A new date does: the parcel is moving again.
        let dated = Extraction { expected: Some(DateWindow { from: days(0).date_naive(), to: days(0).date_naive() }), ..ex(Some("BL-1"), &[], Some(OrderStatus::InTransit)) };
        let entries = vec![entry(1, days(3), ex(Some("BL-1"), &[], Some(OrderStatus::Delayed))), entry(2, days(2), dated)];
        assert_eq!(build_order(&group(entries)[0], None).status, OrderStatus::InTransit);
        // At the same instant the higher-priority status wins, whatever the id order.
        let entries = vec![entry(2, days(1), ex(Some("BL-1"), &[], Some(OrderStatus::Refunded))), entry(1, days(1), ex(Some("BL-1"), &[], Some(OrderStatus::Shipped)))];
        assert_eq!(build_order(&group(entries)[0], None).status, OrderStatus::Refunded);
    }

    #[test]
    fn two_parcels_one_order() {
        let entries = vec![
            entry(1, days(3), ex(Some("BL-1"), &["1Z999AA10123456784"], Some(OrderStatus::Shipped))),
            entry(2, days(2), ex(Some("BL-1"), &["1Z4X7Q203660123459"], Some(OrderStatus::Shipped))),
            entry(3, days(1), Extraction { order_number: None, ..ex(None, &["1Z999AA10123456784"], Some(OrderStatus::Delivered)) }),
        ];
        let groups = group(entries);
        assert_eq!(groups.len(), 1);
        let o = build_order(&groups[0], None);
        assert_eq!(o.shipments.len(), 2);
        let status = |n: &str| o.shipments.iter().find(|s| s.tracking.number == n).and_then(|s| s.status);
        assert_eq!(status("1Z999AA10123456784"), Some(OrderStatus::Delivered));
        assert_eq!(status("1Z4X7Q203660123459"), Some(OrderStatus::Shipped));
        assert!(o.partly_delivered);
        assert_eq!(o.status, OrderStatus::InTransit);
        assert_eq!(section(&o, monday(12), monday(12).with_timezone(&Local).date_naive(), 30), Some(OrderSection::OnTheWay));
    }

    #[test]
    fn a_multi_parcel_order_is_delivered_when_the_last_parcel_arrives() {
        let entries = vec![
            entry(1, days(12), ex(Some("A1"), &["1Z999AA10123456784", "1Z4X7Q203660123459"], Some(OrderStatus::Shipped))),
            entry(2, days(10), ex(None, &["1Z999AA10123456784"], Some(OrderStatus::Delivered))),
            entry(3, days(0), ex(None, &["1Z4X7Q203660123459"], Some(OrderStatus::Delivered))),
        ];
        let o = build_order(&group(entries)[0], None);
        assert_eq!((o.status, o.delivered_at, o.status_at), (OrderStatus::Delivered, Some(days(0)), days(0)));
        let today = monday(12).with_timezone(&Local).date_naive();
        assert_eq!(section(&o, monday(13), today, 7), Some(OrderSection::ArrivingSoon), "Delivered today");
    }

    #[test]
    fn a_late_confirmation_does_not_take_an_order_back() {
        let entries = vec![
            entry(1, days(3), ex(Some("A1"), &[], Some(OrderStatus::Ordered))),
            entry(2, days(2), ex(Some("A1"), &["1Z999AA10123456784"], Some(OrderStatus::OutForDelivery))),
            entry(3, days(1), ex(Some("A1"), &[], Some(OrderStatus::Ordered))),
        ];
        assert_eq!(build_order(&group(entries)[0], None).status, OrderStatus::OutForDelivery);
        // A new date does: the parcel is moving again.
        let dated = Extraction { expected: Some(DateWindow { from: days(0).date_naive(), to: days(0).date_naive() }), ..ex(Some("BL-1"), &[], Some(OrderStatus::InTransit)) };
        let entries = vec![entry(1, days(3), ex(Some("BL-1"), &[], Some(OrderStatus::Delayed))), entry(2, days(2), dated)];
        assert_eq!(build_order(&group(entries)[0], None).status, OrderStatus::InTransit);
        // Out of order: the shipping email first, the confirmation after.
        let entries = vec![entry(1, days(3), ex(Some("A1"), &[], Some(OrderStatus::Shipped))), entry(2, days(2), ex(Some("A1"), &[], Some(OrderStatus::Ordered)))];
        assert_eq!(build_order(&group(entries)[0], None).status, OrderStatus::Shipped);
        // Not delivered after all: left at a pickup point.
        let entries = vec![
            entry(1, days(3), ex(Some("A1"), &[], Some(OrderStatus::Shipped))),
            entry(2, days(2), ex(Some("A1"), &[], Some(OrderStatus::OutForDelivery))),
            entry(3, days(1), ex(Some("A1"), &[], Some(OrderStatus::ReadyForPickup))),
        ];
        assert_eq!(build_order(&group(entries)[0], None).status, OrderStatus::ReadyForPickup);
        // Problems still replace a step on the way.
        let entries = vec![entry(1, days(3), ex(Some("A1"), &[], Some(OrderStatus::OutForDelivery))), entry(2, days(2), ex(Some("A1"), &[], Some(OrderStatus::Delayed)))];
        assert_eq!(build_order(&group(entries)[0], None).status, OrderStatus::Delayed);
    }

    #[test]
    fn orders_sharing_a_shipment_stay_apart() {
        let entries = vec![
            entry(1, days(3), ex(Some("BL-1"), &["1Z999AA10123456784"], Some(OrderStatus::Shipped))),
            entry(2, days(2), ex(Some("BL-2"), &["1Z999AA10123456784"], Some(OrderStatus::Shipped))),
            entry(3, days(1), ex(None, &["1Z999AA10123456784"], Some(OrderStatus::Delivered))),
        ];
        let groups = group(entries.clone());
        assert_eq!(groups.len(), 2);
        for g in &groups {
            assert!(g.keys.contains(&"1:t:ups:1Z999AA10123456784".to_string()), "{:?}", g.keys);
            let o = build_order(g, None);
            assert_eq!(o.shipments.len(), 1);
            // The carrier's delivery is news for both orders.
            assert_eq!((o.status, o.events.len()), (OrderStatus::Delivered, 2), "{}", o.key);
        }
        // Whichever order the carrier's email came in.
        let reversed = vec![entries[1].clone(), entries[0].clone(), Entry { received_at: days(4), ..entries[2].clone() }];
        let groups = group(reversed);
        assert_eq!(groups.len(), 2);
        assert!(groups.iter().all(|g| g.entries.iter().any(|e| e.message_id == 3)));
        // Another shop's order with the same number is another order too.
        let other = Extraction { merchant: Merchant { key: "northpeak.example".into(), ..Default::default() }, ..ex(Some("BL-1"), &[], None) };
        assert_eq!(group(vec![entry(1, days(2), ex(Some("BL-1"), &[], None)), entry(2, days(1), other)]).len(), 2);
    }

    #[test]
    fn cancelled_then_shipped_is_shipped() {
        let entries = vec![entry(1, days(3), ex(Some("BL-1"), &[], Some(OrderStatus::Cancelled))), entry(2, days(2), ex(Some("BL-1"), &["1Z999AA10123456784"], Some(OrderStatus::Shipped)))];
        assert_eq!(build_order(&group(entries)[0], None).status, OrderStatus::Shipped);
        let entries = vec![entry(1, days(3), ex(Some("BL-1"), &[], Some(OrderStatus::Cancelled))), entry(2, days(2), ex(Some("BL-1"), &[], Some(OrderStatus::Ordered)))];
        assert_eq!(build_order(&group(entries)[0], None).status, OrderStatus::Cancelled);
    }

    #[test]
    fn a_review_request_does_not_reopen_a_refund_or_return() {
        for done in [OrderStatus::Refunded, OrderStatus::Returned] {
            let entries = vec![entry(1, days(3), ex(Some("BL-1"), &[], Some(done))), entry(2, days(1), ex(Some("BL-1"), &[], Some(OrderStatus::Ordered)))];
            let o = build_order(&group(entries)[0], None);
            assert_eq!((o.status, o.status_at), (done, days(3)), "{done:?}");
        }
        // A replacement that ships still shows; a refund after a return too.
        let entries = vec![entry(1, days(3), ex(Some("BL-1"), &[], Some(OrderStatus::Returned))), entry(2, days(2), ex(Some("BL-1"), &["1Z999AA10123456784"], Some(OrderStatus::Shipped)))];
        assert_eq!(build_order(&group(entries)[0], None).status, OrderStatus::Shipped);
        let entries = vec![entry(1, days(3), ex(Some("BL-1"), &[], Some(OrderStatus::Returned))), entry(2, days(2), ex(Some("BL-1"), &[], Some(OrderStatus::Refunded)))];
        assert_eq!(build_order(&group(entries)[0], None).status, OrderStatus::Refunded);
    }

    #[test]
    fn the_users_changes_win() {
        let g = &group(vec![entry(1, days(3), ex(Some("BL-1"), &[], Some(OrderStatus::Shipped)))])[0];
        let user = OrderUserState { key: "1:o:brightleaf.example:BL-1".into(), hidden: true, status_override: Some(OrderStatus::Delivered), updated_at: 1 };
        let o = build_order(g, Some(&user));
        assert!(o.hidden && o.overridden);
        assert_eq!((o.status, o.mail_status), (OrderStatus::Delivered, Some(OrderStatus::Shipped)), "what the mail says is kept");
        assert_eq!(build_order(g, None).mail_status, Some(OrderStatus::Shipped));
        assert_eq!(section(&o, monday(12), monday(12).with_timezone(&Local).date_naive(), 30), None, "hidden");
    }

    #[test]
    fn copies_are_counted_once_and_join_only_groups_dropped() {
        let mut copy = entry(9, days(2), ex(Some("BL-1"), &[], Some(OrderStatus::Shipped)));
        copy.internet_message_id = "1@shop.example".into();
        let groups = group(vec![entry(1, days(3), ex(Some("BL-1"), &[], Some(OrderStatus::Ordered))), copy]);
        assert_eq!(groups[0].entries.len(), 1);
        assert_eq!(groups[0].entries[0].message_id, 1);

        let labelled = Extraction {
            tracking: vec![TrackingNumber { carrier: Carrier::Dhl, number: "JJD000390007788663366".into(), validated: Validation::Labelled }],
            ..ex(None, &[], Some(OrderStatus::Shipped))
        };
        assert!(join_only(&labelled));
        assert!(group(vec![entry(1, days(1), labelled.clone())]).is_empty());
        let caution = Extraction { reasons: vec![OrderReason::JunkCaution], ..ex(None, &["1Z999AA10123456784"], None) };
        assert!(group(vec![entry(1, days(1), caution.clone())]).is_empty());
        // Either joins an order that started elsewhere.
        let starter = Extraction { tracking: labelled.tracking.clone(), ..ex(Some("BL-1"), &[], Some(OrderStatus::Shipped)) };
        assert_eq!(group(vec![entry(1, days(2), starter), entry(2, days(1), labelled)])[0].entries.len(), 2);
        // Not order emails at all.
        assert!(group(vec![entry(1, days(1), Extraction::none())]).is_empty());
    }

    fn delivered(days_ago: i64) -> Order {
        Order { status: OrderStatus::Delivered, delivered_at: Some(days(days_ago)), status_at: days(days_ago), updated_at: days(days_ago), ..Order::default() }
    }

    #[test]
    fn sections() {
        let now = monday(12);
        let today = now.with_timezone(&Local).date_naive();
        assert_eq!(section(&delivered(0), now, today, 30), Some(OrderSection::ArrivingSoon));
        assert_eq!(section(&delivered(3), now, today, 30), Some(OrderSection::Delivered));
        assert_eq!(section(&delivered(30), now, today, 30), Some(OrderSection::Delivered));
        assert_eq!(section(&delivered(31), now, today, 30), None);
        assert_eq!(section(&delivered(31), now, today, 90), Some(OrderSection::Delivered));
        let quiet = Order { status: OrderStatus::Shipped, updated_at: days(46), status_at: days(46), ..Order::default() };
        assert_eq!(section(&quiet, now, today, 30), Some(OrderSection::Delivered));
        assert_eq!(section(&Order { updated_at: days(80), ..quiet.clone() }, now, today, 30), None);
        let window = |from: i64, to: i64| Some(DateWindow { from: today + Duration::days(from), to: today + Duration::days(to) });
        let late = Order { expected: window(-3, -3), updated_at: days(5), ..quiet.clone() };
        assert_eq!(section(&late, now, today, 30), Some(OrderSection::NeedsAttention));
        let soon = Order { expected: window(1, 2), updated_at: days(1), ..quiet.clone() };
        assert_eq!(section(&soon, now, today, 30), Some(OrderSection::ArrivingSoon));
        let later = Order { expected: window(5, 6), updated_at: days(1), ..quiet.clone() };
        assert_eq!(section(&later, now, today, 30), Some(OrderSection::OnTheWay));
        assert_eq!(section(&Order { status: OrderStatus::Ordered, ..later.clone() }, now, today, 30), Some(OrderSection::Ordered));
        assert_eq!(section(&Order { status: OrderStatus::Delayed, ..later.clone() }, now, today, 30), Some(OrderSection::NeedsAttention));
        assert_eq!(section(&Order { status: OrderStatus::ReadyForPickup, ..later.clone() }, now, today, 30), Some(OrderSection::ArrivingSoon));
        let refunded = Order { status: OrderStatus::Refunded, status_at: days(2), updated_at: days(2), ..Order::default() };
        assert_eq!(section(&refunded, now, today, 30), Some(OrderSection::CancelledReturned));
        let returned = Order { status: OrderStatus::Returned, ..refunded.clone() };
        assert_eq!(section(&returned, now, today, 30), Some(OrderSection::Delivered));
    }
}
