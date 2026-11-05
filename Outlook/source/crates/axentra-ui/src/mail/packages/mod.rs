//! Packages (Mail > Packages): the orders and deliveries found in mail on
//! this device, as cards in sections (Needs attention, Arriving soon, …).
//! It takes the message list's place; the reading pane follows the layout
//! as usual. Orders come from the store's cache (`Store::orders`) and are
//! addressed by their key, which survives rebuilds.

mod card;

use std::collections::{BTreeMap, BTreeSet};

use axentra_core::orders::{self, Order, OrderSection};
use axentra_core::store::OrdersProgress;
use chrono::{DateTime, NaiveDate, Utc};
use dioxus::html::geometry::PixelsVector2D;
use dioxus::prelude::*;

use super::ctx::use_mail;
use super::list::header::ViewSwitcher;
use crate::kit::{EmptyState, IconButton, Pivot, ProgressBar, dom};
use crate::state::{store, use_app};
use card::OrderCard;
pub use card::window_text;

/// The Packages pivot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum PackagesTab {
    #[default]
    Active,
    Delivered,
    All,
}

impl PackagesTab {
    fn id(self) -> &'static str {
        match self {
            PackagesTab::Active => "active",
            PackagesTab::Delivered => "delivered",
            PackagesTab::All => "all",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "delivered" => PackagesTab::Delivered,
            "all" => PackagesTab::All,
            _ => PackagesTab::Active,
        }
    }

    /// The sections this tab lists.
    pub fn shows(self, section: OrderSection) -> bool {
        match self {
            PackagesTab::Active | PackagesTab::All => true,
            PackagesTab::Delivered => matches!(section, OrderSection::Delivered | OrderSection::CancelledReturned),
        }
    }

    /// Sections collapsed until opened: what's done, under Active.
    pub fn collapsed_by_default(self, section: OrderSection) -> bool {
        self == PackagesTab::Active && matches!(section, OrderSection::Delivered | OrderSection::CancelledReturned)
    }
}

/// Sections the user opened or closed, per tab (against that tab's
/// default): opening Delivered under Active leaves the Delivered tab as it was.
pub type Folds = BTreeMap<PackagesTab, BTreeSet<OrderSection>>;

/// `section` is folded on `tab`.
pub fn folded(folds: &Folds, tab: PackagesTab, section: OrderSection) -> bool {
    tab.collapsed_by_default(section) != folds.get(&tab).is_some_and(|s| s.contains(&section))
}

/// Fold or unfold `section` on `tab` only.
pub fn toggle_fold(folds: &mut Folds, tab: PackagesTab, section: OrderSection) {
    let s = folds.entry(tab).or_default();
    if !s.remove(&section) {
        s.insert(section);
    }
}

/// How Packages was left (kept in `MailCtx` for the session), so the
/// cards come back as they were after an email opened over them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PackagesMemory {
    pub tab: PackagesTab,
    /// Sections the user opened or closed, per tab.
    pub toggled: Folds,
    /// Orders (by key) whose "Emails (n)" list is open.
    pub emails_open: BTreeSet<String>,
    pub scroll: f64,
    /// The order whose email was opened last: its card gets focus back.
    pub last: Option<String>,
    /// An email was opened over the cards (full width): scroll and focus
    /// are put back when the cards return.
    pub restore: bool,
}

/// Section order on the page.
pub const SECTIONS: [OrderSection; 6] = [
    OrderSection::NeedsAttention,
    OrderSection::ArrivingSoon,
    OrderSection::OnTheWay,
    OrderSection::Ordered,
    OrderSection::Delivered,
    OrderSection::CancelledReturned,
];

pub fn section_label(s: OrderSection) -> &'static str {
    match s {
        OrderSection::NeedsAttention => "Needs attention",
        OrderSection::ArrivingSoon => "Arriving soon",
        OrderSection::OnTheWay => "On the way",
        OrderSection::Ordered => "Ordered",
        OrderSection::Delivered => "Delivered",
        OrderSection::CancelledReturned => "Cancelled or refunded",
    }
}

fn section_id(s: OrderSection) -> &'static str {
    match s {
        OrderSection::NeedsAttention => "attention",
        OrderSection::ArrivingSoon => "soon",
        OrderSection::OnTheWay => "on-the-way",
        OrderSection::Ordered => "ordered",
        OrderSection::Delivered => "delivered",
        OrderSection::CancelledReturned => "cancelled",
    }
}

/// Orders by section, in page order, newest first within a section;
/// orders without a section (hidden, or gone after `keep_days`) are left out.
pub fn bucket(list: &[Order], now: DateTime<Utc>, today: NaiveDate, keep_days: u32) -> Vec<(OrderSection, Vec<Order>)> {
    let mut out: Vec<(OrderSection, Vec<Order>)> = SECTIONS.iter().map(|s| (*s, Vec::new())).collect();
    for o in list {
        if let Some(sec) = orders::section(o, now, today, keep_days)
            && let Some((_, v)) = out.iter_mut().find(|(s, _)| *s == sec)
        {
            v.push(o.clone());
        }
    }
    for (_, v) in out.iter_mut() {
        v.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then_with(|| a.key.cmp(&b.key)));
    }
    out.retain(|(_, v)| !v.is_empty());
    out
}

/// Orders still on their way (the Active count).
pub fn active_count(sections: &[(OrderSection, Vec<Order>)]) -> usize {
    sections.iter().filter(|(s, _)| !matches!(s, OrderSection::Delivered | OrderSection::CancelledReturned)).map(|(_, v)| v.len()).sum()
}

/// DOM id of an order's card.
pub fn card_dom_id(key: &str) -> String {
    format!("ax-pkg-{:x}", super::ctx::fnv(key))
}

/// `n` with thousands separators.
fn grouped(n: u32) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn progress_text(p: OrdersProgress) -> String {
    if p.left > 0 {
        format!("Checking your mail for orders: {} of {}", grouped(p.done()), grouped(p.total))
    } else {
        "Updating your packages\u{2026}".to_string()
    }
}

/// How full the bar is; `None` (it just moves) once the mail is read.
fn progress_share(p: OrdersProgress) -> Option<f64> {
    (p.left > 0 && p.total > 0).then(|| f64::from(p.done()) / f64::from(p.total))
}

#[component]
pub fn PackagesPane(columns: bool, #[props(default)] style: String) -> Element {
    let app = use_app();
    let mut ctx = use_mail();
    let _ = app.mail_rev();
    let settings = app.settings.read().clone();
    let today = *ctx.today.read();
    let now = Utc::now();
    let s = store();
    let list = s.orders(false).unwrap_or_default();
    // Off the UI thread: counting is a walk over the mailbox.
    let progress = use_resource(move || async move {
        let _ = app.mail_rev();
        tokio::task::spawn_blocking(|| store().orders_progress(Utc::now()).ok()).await.ok().flatten()
    });
    let progress = progress.read().flatten();
    let sections = bucket(&list, now, today, settings.orders_keep_days());
    let active = active_count(&sections);
    let mut tab = use_signal(|| ctx.packages.peek().tab);
    // Sections the user opened or closed, per tab.
    let mut toggled = use_signal(|| ctx.packages.peek().toggled.clone());
    let mut why = use_signal(|| false);
    let current = tab();
    use_effect(move || {
        let (t, tg) = (tab(), toggled.read().clone());
        let mut memory = ctx.packages.write();
        memory.tab = t;
        memory.toggled = tg;
    });

    // "View in Packages" from the reading pane: show that card and pulse it.
    let focus_key = ctx.focus_order.read().clone();
    let focus_section = focus_key.as_ref().and_then(|k| sections.iter().find(|(_, v)| v.iter().any(|o| &o.key == k)).map(|(s, _)| *s));
    use_effect(use_reactive!(|focus_key, focus_section| {
        let Some(key) = focus_key else { return };
        if let Some(sec) = focus_section {
            let mut t = *tab.peek();
            if !t.shows(sec) {
                t = PackagesTab::All;
                tab.set(t);
            }
            // Open its section when it is folded on the tab shown.
            if folded(&toggled.peek(), t, sec) {
                toggle_fold(&mut toggled.write(), t, sec);
            }
        }
        ctx.focus_order.set(None);
        let id = card_dom_id(&key);
        spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(60)).await;
            dom::flash(&format!("#{id}"));
            dom::focus(&format!("#{id}"));
        });
    }));

    let shown: Vec<(OrderSection, Vec<Order>)> = sections.iter().filter(|(s, _)| current.shows(*s)).cloned().collect();

    // Focus back on the card whose email was open (Escape or Back from it),
    // else on the first card; never lost with no message list to take it.
    let on_page: Vec<String> = shown
        .iter()
        .filter(|(sec, _)| !folded(&toggled.read(), current, *sec))
        .flat_map(|(_, v)| v.iter().map(|o| o.key.clone()))
        .collect();
    let mut cards = use_hook(|| CopyValue::new(Vec::<String>::new()));
    cards.set(on_page);
    let focus_card = move || {
        let last = ctx.packages.peek().last.clone().filter(|k| cards.peek().contains(k));
        match last {
            Some(k) => dom::focus(&format!("#{}", card_dom_id(&k))),
            None => dom::focus(".ax-pkg-card[tabindex=\"0\"]"),
        }
    };
    {
        let mut seen = use_signal(|| *ctx.focus_list.peek());
        use_effect(move || {
            let req = *ctx.focus_list.read();
            if req != *seen.peek() {
                seen.set(req);
                focus_card();
            }
        });
    }
    let tab_items = vec![
        ("active".to_string(), format!("Active ({active})")),
        ("delivered".to_string(), "Delivered".to_string()),
        ("all".to_string(), "All".to_string()),
    ];
    let desktop = settings.desktop_view();
    rsx! {
        section {
            class: "ax-mail-list ax-pkg-pane",
            class: if columns { "ax-columns" },
            role: "region",
            "aria-label": "Packages",
            "data-ax-region": "true",
            style: "{style}",
            div { class: "ax-mail-list-header ax-pkg-header",
                div { class: "ax-mail-title-row",
                    if desktop {
                        ViewSwitcher { title: "Packages" }
                    } else {
                        h2 { class: "ax-mail-title ax-ellipsis", "Packages" }
                    }
                    span { class: "ax-grow" }
                    IconButton {
                        icon: "info",
                        title: "Why only some orders?",
                        toggled: why(),
                        onclick: move |_| why.set(!why()),
                    }
                }
                div { class: "ax-pkg-subtitle",
                    "Found in your mail on this device"
                    if progress.is_some_and(|p| !p.busy()) { " \u{b7} Up to date" }
                }
                if why() {
                    p { class: "ax-set-note ax-pkg-why", role: "note",
                        "Packages are found on this device from order and shipping emails. Suspicious delivery emails are left out."
                    }
                }
                Pivot {
                    class: "ax-mail-pivot",
                    label: "Packages",
                    items: tab_items,
                    selected: current.id(),
                    onselect: move |k: String| tab.set(PackagesTab::parse(&k)),
                }
                if let Some(p) = progress.filter(|p| p.busy()) {
                    div { class: "ax-pkg-pending",
                        span { role: "status", "{progress_text(p)}" }
                        ProgressBar { value: progress_share(p), label: "Checking your mail for orders" }
                    }
                }
            }
            div {
                class: "ax-pkg-scroll",
                tabindex: "-1",
                onmounted: move |e| {
                    let el = e.data();
                    let back = std::mem::take(&mut ctx.packages.write().restore);
                    if back {
                        let y = ctx.packages.peek().scroll;
                        spawn(async move {
                            let _ = el.scroll(PixelsVector2D::new(0.0, y), ScrollBehavior::Instant).await;
                            focus_card();
                        });
                    }
                },
                onscroll: move |e: ScrollEvent| ctx.packages.write().scroll = e.data().scroll_top(),
                if list.is_empty() || sections.is_empty() {
                    EmptyState {
                        icon: "box",
                        icon_size: 48,
                        title: "No packages right now",
                        subtitle: "Order confirmations and shipping emails you get show up here. Your mail is checked only on this device.",
                    }
                } else if shown.is_empty() {
                    EmptyState { icon: "box", icon_size: 48, title: "No delivered packages", subtitle: "Delivered and cancelled orders show up here for a while." }
                } else {
                    for (sec, orders) in shown.into_iter() {
                        {
                            let collapsed = folded(&toggled.read(), current, sec);
                            let heading = format!("ax-pkg-sec-{}", section_id(sec));
                            let count = orders.len();
                            rsx! {
                                section { key: "{heading}", class: "ax-pkg-section", "aria-labelledby": "{heading}",
                                    h3 { class: "ax-pkg-section-title",
                                        button {
                                            r#type: "button",
                                            id: "{heading}",
                                            class: "ax-pkg-section-toggle",
                                            "aria-expanded": if collapsed { "false" } else { "true" },
                                            onclick: move |_| toggle_fold(&mut toggled.write(), current, sec),
                                            crate::kit::Icon { name: if collapsed { "chevron_right" } else { "chevron_down" }, size: 16 }
                                            span { "{section_label(sec)}" }
                                            span { class: "ax-pkg-section-count", "{count}" }
                                        }
                                    }
                                    // Each section is its own composite: its first card
                                    // is its tab stop, Up and Down move within it.
                                    if !collapsed {
                                        div { class: "ax-pkg-grid", role: "list", "data-ax-roving": "vertical",
                                            for (i, o) in orders.into_iter().enumerate() {
                                                OrderCard {
                                                    key: "{o.key}",
                                                    first: i == 0,
                                                    section: sec,
                                                    order: o,
                                                    today,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_show_their_sections_and_fold_whats_done() {
        assert!(PackagesTab::Active.shows(OrderSection::NeedsAttention));
        assert!(PackagesTab::Active.collapsed_by_default(OrderSection::Delivered));
        assert!(PackagesTab::Active.collapsed_by_default(OrderSection::CancelledReturned));
        assert!(!PackagesTab::Active.collapsed_by_default(OrderSection::ArrivingSoon));
        assert!(!PackagesTab::Delivered.shows(OrderSection::OnTheWay));
        assert!(PackagesTab::Delivered.shows(OrderSection::Delivered));
        assert!(!PackagesTab::All.collapsed_by_default(OrderSection::Delivered));
        assert_eq!(PackagesTab::parse("all"), PackagesTab::All);
        assert_eq!(PackagesTab::parse("nonsense"), PackagesTab::Active);
    }

    #[test]
    fn each_tab_keeps_its_own_folds() {
        let mut folds = Folds::new();
        assert!(folded(&folds, PackagesTab::Active, OrderSection::Delivered));
        // Opened under Active: still open (its default) on the Delivered tab.
        toggle_fold(&mut folds, PackagesTab::Active, OrderSection::Delivered);
        assert!(!folded(&folds, PackagesTab::Active, OrderSection::Delivered));
        assert!(!folded(&folds, PackagesTab::Delivered, OrderSection::Delivered));
        // Folding Arriving soon under Active leaves All alone.
        toggle_fold(&mut folds, PackagesTab::Active, OrderSection::ArrivingSoon);
        assert!(folded(&folds, PackagesTab::Active, OrderSection::ArrivingSoon));
        assert!(!folded(&folds, PackagesTab::All, OrderSection::ArrivingSoon));
        toggle_fold(&mut folds, PackagesTab::Active, OrderSection::ArrivingSoon);
        assert!(!folded(&folds, PackagesTab::Active, OrderSection::ArrivingSoon));
    }

    #[test]
    fn progress_counts_the_mail_then_the_orders() {
        let p = OrdersProgress { total: 12_500, left: 2_500, regrouping: 3 };
        assert_eq!(progress_text(p), "Checking your mail for orders: 10,000 of 12,500");
        assert_eq!(progress_share(p), Some(0.8));
        let p = OrdersProgress { left: 0, ..p };
        assert_eq!(progress_text(p), "Updating your packages\u{2026}");
        assert_eq!(progress_share(p), None);
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1_234_567), "1,234,567");
    }

    #[test]
    fn card_ids_are_dom_safe() {
        let id = card_dom_id("1:o:brightleaf:BL-48213");
        assert!(id.starts_with("ax-pkg-") && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
        assert_ne!(id, card_dom_id("1:o:brightleaf:BL-48214"));
    }

    #[test]
    fn active_counts_what_is_still_coming() {
        let o = Order::default();
        let sections = vec![
            (OrderSection::ArrivingSoon, vec![o.clone(), o.clone()]),
            (OrderSection::Ordered, vec![o.clone()]),
            (OrderSection::Delivered, vec![o.clone()]),
        ];
        assert_eq!(active_count(&sections), 3);
    }
}
