//! One order as a card: merchant, status, a four-step tracker, shipments
//! with Track, items and the emails it was found in. The only page a card
//! ever opens is a carrier's fixed tracking page
//! (`Carrier::tracking_url`), and only from a click on Track.

use axentra_core::orders::{DateWindow, Order, OrderSection, OrderStatus};
use axentra_core::*;
use chrono::{DateTime, Local, NaiveDate, Utc};
use dioxus::prelude::*;

use super::super::actions::{self, OrderMenuCmd};
use super::super::ctx::{MailCtx, use_mail};
use super::super::keyboard;
use crate::kit::{Avatar, Icon, IconButton, MenuButton, MenuItem, dom, keys};
use crate::settings::PageId;
use crate::state::{AppState, NavRequest, store, use_app};

/// Labels of the tracker's steps.
const STEPS: [&str; 4] = ["Ordered", "Shipped", "Out for delivery", "Delivered"];

fn local_date(t: DateTime<Utc>) -> NaiveDate {
    t.with_timezone(&Local).date_naive()
}

/// "Thursday, Oct 22".
fn day(d: NaiveDate) -> String {
    d.format("%A, %b %-d").to_string()
}

/// "Nov 2 – 5", or "Oct 30 – Nov 2" across months.
pub fn window_text(w: DateWindow) -> String {
    use chrono::Datelike;
    if w.from.month() == w.to.month() && w.from.year() == w.to.year() {
        format!("{} \u{2013} {}", w.from.format("%b %-d"), w.to.format("%-d"))
    } else {
        format!("{} \u{2013} {}", w.from.format("%b %-d"), w.to.format("%b %-d"))
    }
}

/// The card's headline: when it arrives, or where it is.
pub fn headline(o: &Order, section: OrderSection, today: NaiveDate) -> String {
    match o.status {
        OrderStatus::Cancelled | OrderStatus::Refunded | OrderStatus::Returned => return o.status.label().to_string(),
        OrderStatus::Delivered => {
            let at = local_date(o.delivered_at.unwrap_or(o.status_at));
            return if at == today { "Delivered today".to_string() } else { format!("Delivered {}", at.format("%b %-d")) };
        }
        _ => {}
    }
    if section == OrderSection::Delivered {
        return "No recent updates".to_string();
    }
    match o.status {
        OrderStatus::Delayed | OrderStatus::Exception | OrderStatus::OutForDelivery | OrderStatus::ReadyForPickup => {
            return o.status.label().to_string();
        }
        _ => {}
    }
    if o.partly_delivered {
        return "Partly delivered".to_string();
    }
    match o.expected {
        Some(w) if w.from == w.to && w.from == today => "Arriving today".to_string(),
        Some(w) if w.from == w.to && Some(w.from) == today.succ_opt() => "Arriving tomorrow".to_string(),
        Some(w) if w.from == w.to && w.from > today => format!("Arriving {}", day(w.from)),
        Some(w) => format!("Expected {}", if w.from == w.to { w.from.format("%b %-d").to_string() } else { window_text(w) }),
        None if o.status == OrderStatus::Ordered => {
            let at = o.events.first().map(|e| e.at).unwrap_or(o.status_at);
            format!("Ordered {}", local_date(at).format("%b %-d"))
        }
        None => o.status.label().to_string(),
    }
}

/// The status badge's tone: success when delivered, warning when there's
/// a problem, neutral otherwise.
pub fn badge_tone(status: OrderStatus) -> &'static str {
    match status {
        OrderStatus::Delivered => "ax-pkg-badge-success",
        OrderStatus::Delayed | OrderStatus::Exception => "ax-pkg-badge-warning",
        _ => "ax-pkg-badge-neutral",
    }
}

/// One step of the tracker.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub label: String,
    /// The status icon, when the step shows a problem instead of its own label.
    pub icon: Option<&'static str>,
    pub done: bool,
    pub current: bool,
    pub warn: bool,
    /// When the step was reached (an email's date), for the tooltip.
    pub title: String,
}

/// The tracker (Ordered → Shipped → Out for delivery → Delivered), or
/// `None` for a cancelled or refunded order. Off the happy path the last
/// step shows the problem.
pub fn tracker(o: &Order) -> Option<Vec<Step>> {
    if matches!(o.status, OrderStatus::Cancelled | OrderStatus::Refunded) {
        return None;
    }
    let reached_at = |i: u8| o.events.iter().find(|e| e.status.and_then(|s| s.step()) == Some(i)).map(|e| local_date(e.at).format("%b %-d").to_string());
    let furthest = o.events.iter().filter_map(|e| e.status.and_then(|s| s.step())).max().unwrap_or(0);
    let step = o.status.step();
    let mut steps: Vec<Step> = STEPS
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let i = i as u8;
            let (done, current) = match step {
                Some(s) => (i <= s, i == s),
                None => (i <= furthest && i < 3, false),
            };
            Step { label: label.to_string(), icon: None, done, current, warn: false, title: reached_at(i).unwrap_or_default() }
        })
        .collect();
    if step.is_none()
        && let Some(last) = steps.last_mut()
    {
        last.label = o.status.label().to_string();
        last.icon = Some(o.status.icon());
        last.current = true;
        last.warn = matches!(o.status, OrderStatus::Delayed | OrderStatus::Exception);
    }
    Some(steps)
}

/// Up to three items, and how many more there are.
pub fn items_shown(items: &[String]) -> (Vec<String>, usize) {
    (items.iter().take(3).cloned().collect(), items.len().saturating_sub(3))
}

/// The merchant's name, else its domain.
fn merchant_name(o: &Order) -> String {
    if o.merchant.name.trim().is_empty() { o.merchant.domain.clone() } else { o.merchant.name.clone() }
}

/// Open one of the order's (`key`) emails in the reading pane (over the
/// cards when the reading pane is hidden; Back returns to them, as they
/// were, with focus on this card).
fn open_email(app: AppState, mut ctx: MailCtx, key: &str, id: MessageId) {
    let Some(m) = store().message(id).ok().flatten() else {
        app.error("That email is no longer available.");
        return;
    };
    let settings = app.settings.peek().clone();
    actions::open_message(ctx, &m, settings.conversation_view);
    let full = settings.reading_pane_shown() == ReadingPanePosition::Hidden;
    {
        let mut memory = ctx.packages.write();
        memory.last = Some(key.to_string());
        memory.restore = full;
    }
    if full {
        ctx.full_view.set(true);
    }
}

fn copy_text(app: AppState, text: &str, done: &str) {
    dom::run(&format!("navigator.clipboard && navigator.clipboard.writeText({})", dom::js_str(text)));
    app.toast(done.to_string());
}

/// The first shipment's tracking page, if its carrier has one.
fn first_track_url(o: &Order) -> Option<String> {
    o.shipments.iter().find_map(|s| s.tracking.carrier.tracking_url(&s.tracking.number))
}

#[component]
pub fn OrderCard(order: Order, section: OrderSection, today: NaiveDate, first: bool) -> Element {
    let app = use_app();
    let mut ctx = use_mail();
    let mut why = use_signal(|| false);
    let mut emails_open = use_signal(|| ctx.packages.peek().emails_open.contains(&order.key));
    let o = order;
    let name = merchant_name(&o);
    let domain = o.merchant.domain.clone();
    let head = headline(&o, section, today);
    let steps = tracker(&o);
    let (items, more_items) = items_shown(&o.items);
    let tone = badge_tone(o.status);
    let status_label = if o.partly_delivered { "Partly delivered" } else { o.status.label() };
    let key = o.key.clone();
    let dom_id = super::card_dom_id(&o.key);
    let newest = o.events.iter().max_by_key(|e| e.at).map(|e| e.message_id);
    let track_first = first_track_url(&o);
    let h24 = app.settings.read().use_24h_time;
    let mut events = o.events.clone();
    events.sort_by_key(|e| std::cmp::Reverse(e.at));
    let mut menu = vec![if o.status == OrderStatus::Delivered {
        MenuItem::new(OrderMenuCmd::MarkNotDelivered.id(), "Mark as not delivered").icon("arrow_undo")
    } else {
        MenuItem::new(OrderMenuCmd::MarkDelivered.id(), "Mark as delivered").icon("checkmark_circle")
    }];
    menu.push(MenuItem::new(OrderMenuCmd::Hide.id(), "Remove from Packages").icon("eye_off"));
    if let Some(sender) = actions::ignore_target(&o, actions::order_sender) {
        menu.push(MenuItem::new(OrderMenuCmd::IgnoreSender.id(), format!("Don't track orders from {sender}")).icon("dismiss_circle"));
    }
    menu.push(MenuItem::new(OrderMenuCmd::WhyHere.id(), "Why is this here?").icon("info"));
    menu.push(MenuItem::divider());
    menu.push(MenuItem::new("o_settings", "Package settings\u{2026}").icon("settings"));
    let key_menu = key.clone();
    let key_open = key.clone();
    let reasons: Vec<String> = o.reasons.iter().map(|r| r.describe()).collect();
    let order_no = o.order_number.clone();
    let label = format!("{name}, {head}");
    rsx! {
        article {
            id: "{dom_id}",
            class: "ax-pkg-card",
            role: "listitem",
            tabindex: if first { "0" } else { "-1" },
            "data-ax-item": "true",
            "aria-label": "{label}",
            onkeydown: move |e: KeyboardEvent| {
                // Delete, Backspace and the other message commands act on
                // the selected email, which may be another order's: not
                // from a card.
                if keyboard::acts_on_messages(&keys::chord(&e)) {
                    e.stop_propagation();
                    return;
                }
                let m = e.modifiers();
                if m.ctrl() || m.alt() || m.meta() {
                    return;
                }
                match e.key() {
                    Key::Enter => {
                        e.prevent_default();
                        if let Some(id) = newest {
                            open_email(app, ctx, &key_open, id);
                        }
                    }
                    Key::Character(c) if c.eq_ignore_ascii_case("t") => {
                        e.prevent_default();
                        if let Some(url) = &track_first {
                            crate::calendar::actions::open_external(url);
                        }
                    }
                    _ => {}
                }
            },
            // Enter and T on the card's own controls are theirs, not the
            // card's; every other key goes on up (F6, Ctrl+Z, Ctrl+1…), and
            // the card itself keeps message commands from them.
            div {
                class: "ax-pkg-card-body",
                onkeydown: move |e: KeyboardEvent| {
                    if card_key(&keys::chord(&e)) {
                        e.stop_propagation();
                    }
                },
                div { class: "ax-pkg-card-head",
                    Avatar { name: name.clone(), email: domain.clone(), size: 32 }
                    div { class: "ax-pkg-merchant",
                        div { class: "ax-pkg-merchant-name ax-ellipsis", title: "{name}", "{name}" }
                        if !domain.is_empty() {
                            div { class: "ax-pkg-domain ax-ellipsis", title: "Sent from {domain}", "{domain}" }
                        }
                    }
                    span { class: "ax-badge ax-pkg-badge {tone}", "{status_label}" }
                    MenuButton {
                        label: "More options for this order",
                        icon: "more_horizontal",
                        icon_only: true,
                        align_end: true,
                        items: menu,
                        on_select: move |id: String| match id.as_str() {
                            "o_settings" => actions::open_settings_page(app, PageId::Packages),
                            other => match OrderMenuCmd::parse(other) {
                                Some(OrderMenuCmd::WhyHere) => why.set(true),
                                Some(cmd) => actions::order_action(app, &key_menu, cmd),
                                None => {}
                            },
                        },
                    }
                }
                if let Some(n) = order_no.clone() {
                    div { class: "ax-pkg-orderno",
                        span { class: "ax-selectable", "Order {n}" }
                        IconButton {
                            icon: "copy",
                            title: "Copy order number",
                            small: true,
                            onclick: move |_| copy_text(app, &n, "Order number copied"),
                        }
                    }
                }
                div { class: "ax-pkg-headline", "{head}" }
                if let Some(steps) = steps {
                    ol { class: "ax-pkg-steps", "aria-label": "Delivery progress",
                        for (i, st) in steps.into_iter().enumerate() {
                            li {
                                key: "{i}",
                                class: "ax-pkg-step",
                                class: if st.done { "ax-done" },
                                class: if st.current { "ax-current" },
                                class: if st.warn { "ax-warn" },
                                "aria-current": if st.current { "step" },
                                title: "{st.title}",
                                span { class: "ax-pkg-step-dot",
                                    if let Some(icon) = st.icon { Icon { name: icon, size: 12 } }
                                }
                                span { class: "ax-pkg-step-label", "{st.label}" }
                            }
                        }
                    }
                }
                if !o.shipments.is_empty() {
                    ul { class: "ax-pkg-shipments",
                        for (i, sh) in o.shipments.iter().cloned().enumerate() {
                            {
                                let carrier = sh.tracking.carrier.name();
                                let number = sh.tracking.number.clone();
                                let number_copy = number.clone();
                                let url = sh.tracking.carrier.tracking_url(&number);
                                let host = sh.tracking.carrier.host().unwrap_or("the carrier's website");
                                let aria = format!("Track package from {name}");
                                rsx! {
                                    li { key: "{i}", class: "ax-pkg-shipment",
                                        span { class: "ax-pkg-carrier", "{carrier}" }
                                        span { class: "ax-pkg-number ax-selectable", "{number}" }
                                        IconButton {
                                            icon: "copy",
                                            title: "Copy tracking number",
                                            small: true,
                                            onclick: move |_| copy_text(app, &number_copy, "Tracking number copied"),
                                        }
                                        span { class: "ax-grow" }
                                        if let Some(url) = url {
                                            button {
                                                r#type: "button",
                                                class: "ax-btn ax-small ax-pkg-track",
                                                title: "Opens {host} in your browser",
                                                "aria-label": "{aria}",
                                                onclick: move |_| crate::calendar::actions::open_external(&url),
                                                Icon { name: "open" }
                                                span { class: "ax-btn-label", "Track package" }
                                            }
                                        } else if let Some(id) = newest {
                                            button {
                                                r#type: "button",
                                                class: "ax-mail-link",
                                                title: "The order email has the tracking link",
                                                onclick: {
                                                    let key = key.clone();
                                                    move |_| open_email(app, ctx, &key, id)
                                                },
                                                "Track from the order email"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                if !items.is_empty() {
                    ul { class: "ax-pkg-items",
                        for (i, it) in items.iter().enumerate() {
                            li { key: "{i}", class: "ax-ellipsis", title: "{it}", "{it}" }
                        }
                        if more_items > 0 {
                            li { class: "ax-pkg-more", "+{more_items} more" }
                        }
                    }
                }
                if why() {
                    div { class: "ax-pkg-why", role: "note",
                        div { class: "ax-pkg-why-head",
                            span { class: "ax-semibold", "Why is this here?" }
                            IconButton { icon: "dismiss", title: "Close", small: true, onclick: move |_| why.set(false) }
                        }
                        ul {
                            for (i, r) in reasons.iter().enumerate() {
                                li { key: "{i}", "{r}" }
                            }
                        }
                    }
                }
                div { class: "ax-pkg-footer",
                    button {
                        r#type: "button",
                        class: "ax-pkg-emails-toggle",
                        "aria-expanded": if emails_open() { "true" } else { "false" },
                        onclick: {
                            let key = key.clone();
                            move |_| {
                                let open = !emails_open();
                                emails_open.set(open);
                                let mut memory = ctx.packages.write();
                                if open {
                                    memory.emails_open.insert(key.clone());
                                } else {
                                    memory.emails_open.remove(&key);
                                }
                            }
                        },
                        Icon { name: if emails_open() { "chevron_down" } else { "chevron_right" }, size: 12 }
                        span { "Emails ({events.len()})" }
                    }
                    if emails_open() {
                        ul { class: "ax-pkg-emails",
                            for e in events.iter().cloned() {
                                {
                                    let when = crate::fmt::full_time(e.at, h24);
                                    let status = e.status.map(|s| s.label()).unwrap_or("Update");
                                    let subject = if e.subject.trim().is_empty() { "(No subject)".to_string() } else { e.subject.clone() };
                                    let id = e.message_id;
                                    let key = key.clone();
                                    rsx! {
                                        li { key: "{id}", class: "ax-pkg-email",
                                            button {
                                                r#type: "button",
                                                class: "ax-pkg-email-open ax-ellipsis",
                                                title: "{subject}",
                                                onclick: move |_| open_email(app, ctx, &key, id),
                                                "{when} \u{00b7} {status} \u{00b7} {subject}"
                                            }
                                            button {
                                                r#type: "button",
                                                class: "ax-mail-link",
                                                onclick: move |_| app.navigate(NavRequest::Message(id)),
                                                "Show in folder"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// A key the card acts on itself (open the newest email, track): pressed
/// on one of its controls, it stays with the control.
fn card_key(chord: &str) -> bool {
    matches!(chord, "Enter" | "Shift+Enter" | "T" | "Shift+T")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axentra_core::orders::OrderEvent;
    use chrono::TimeZone;

    fn at(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        Local.with_ymd_and_hms(y, m, d, 12, 0, 0).unwrap().with_timezone(&Utc)
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn event(status: OrderStatus, when: DateTime<Utc>) -> OrderEvent {
        OrderEvent { message_id: 1, at: when, status: Some(status), subject: String::new() }
    }

    #[test]
    fn only_the_cards_own_keys_stop_at_its_controls() {
        for k in ["Enter", "T", "Shift+T"] {
            assert!(card_key(k), "{k}");
        }
        // Region cycling, undo, module switching and new mail go on up.
        for k in ["F6", "Shift+F6", "Ctrl+Z", "Ctrl+1", "Ctrl+N", "Space", "Tab", "Ctrl+T"] {
            assert!(!card_key(k), "{k}");
        }
    }

    #[test]
    fn headlines_say_when_and_where() {
        let today = date(2026, 10, 19);
        let mut o = Order { status: OrderStatus::Shipped, ..Default::default() };
        o.expected = Some(DateWindow { from: date(2026, 10, 22), to: date(2026, 10, 22) });
        assert_eq!(headline(&o, OrderSection::OnTheWay, today), "Arriving Thursday, Oct 22");
        o.expected = Some(DateWindow { from: date(2026, 11, 2), to: date(2026, 11, 5) });
        assert_eq!(headline(&o, OrderSection::OnTheWay, today), "Expected Nov 2 \u{2013} 5");
        o.expected = Some(DateWindow { from: date(2026, 10, 20), to: date(2026, 10, 20) });
        assert_eq!(headline(&o, OrderSection::ArrivingSoon, today), "Arriving tomorrow");
        o.status = OrderStatus::OutForDelivery;
        assert_eq!(headline(&o, OrderSection::ArrivingSoon, today), "Out for delivery");
        o.status = OrderStatus::Delivered;
        o.delivered_at = Some(at(2026, 10, 19));
        assert_eq!(headline(&o, OrderSection::ArrivingSoon, today), "Delivered today");
        o.delivered_at = Some(at(2026, 10, 17));
        assert_eq!(headline(&o, OrderSection::Delivered, today), "Delivered Oct 17");
        let mut ordered = Order { status: OrderStatus::Ordered, ..Default::default() };
        ordered.events = vec![event(OrderStatus::Ordered, at(2026, 10, 16))];
        assert_eq!(headline(&ordered, OrderSection::Ordered, today), "Ordered Oct 16");
        let stale = Order { status: OrderStatus::InTransit, ..Default::default() };
        assert_eq!(headline(&stale, OrderSection::Delivered, today), "No recent updates");
        let cancelled = Order { status: OrderStatus::Cancelled, ..Default::default() };
        assert_eq!(headline(&cancelled, OrderSection::CancelledReturned, today), "Cancelled");
    }

    #[test]
    fn the_tracker_follows_the_status() {
        let mut o = Order { status: OrderStatus::OutForDelivery, ..Default::default() };
        o.events = vec![event(OrderStatus::Ordered, at(2026, 10, 15)), event(OrderStatus::Shipped, at(2026, 10, 16))];
        let steps = tracker(&o).unwrap();
        assert_eq!(steps.iter().map(|s| (s.done, s.current)).collect::<Vec<_>>(), [(true, false), (true, false), (true, true), (false, false)]);
        assert_eq!(steps[1].title, "Oct 16");
        // A problem replaces the last step, after the furthest step reached.
        o.status = OrderStatus::Exception;
        let steps = tracker(&o).unwrap();
        assert_eq!(steps[3].label, "Delivery problem");
        assert!(steps[3].warn && steps[3].current && steps[3].icon == Some("warning"));
        assert!(steps[1].done && !steps[2].done);
        o.status = OrderStatus::Cancelled;
        assert_eq!(tracker(&o), None);
        o.status = OrderStatus::Refunded;
        assert_eq!(tracker(&o), None);
    }

    #[test]
    fn badges_and_items() {
        assert_eq!(badge_tone(OrderStatus::Delivered), "ax-pkg-badge-success");
        assert_eq!(badge_tone(OrderStatus::Delayed), "ax-pkg-badge-warning");
        assert_eq!(badge_tone(OrderStatus::Shipped), "ax-pkg-badge-neutral");
        let items: Vec<String> = ["Lamp", "Shade", "Bulb", "Cord", "Hook"].iter().map(|s| s.to_string()).collect();
        assert_eq!(items_shown(&items), (items[..3].to_vec(), 2));
        assert_eq!(items_shown(&items[..2]), (items[..2].to_vec(), 0));
    }
}
