//! Message list header: folder title and favourite star, Select, Jump to,
//! Filter and sort controls, the Focused/Other pivot (or the Topic pivot),
//! the active-filter chip and the column headers of the single-line layout
//! (docs/spec/mail.md §3.1–3.3). In the pinned desktop view the title is a
//! switcher between the Inbox, Packages and the topics.

use axentra_core::*;
use dioxus::prelude::*;

use super::super::actions;
use super::super::ctx::{ListView, MailCtx, SortField, filter_label, use_mail, use_model};
use crate::kit::{Icon, IconButton, Intent, MenuButton, MenuItem, MessageBar, Pivot, Variant, dom};
use crate::settings::PageId;
use crate::settings::controls::LinkButton;
use crate::state::{AppState, MailView, store, use_app};

const FILTERS: [MessageFilter; 6] = [
    MessageFilter::All,
    MessageFilter::Unread,
    MessageFilter::Flagged,
    MessageFilter::ToMe,
    MessageFilter::HasFiles,
    MessageFilter::Mentions,
];

fn filter_id(f: MessageFilter) -> &'static str {
    match f {
        MessageFilter::All => "all",
        MessageFilter::Unread => "unread",
        MessageFilter::Flagged => "flagged",
        MessageFilter::ToMe => "to_me",
        MessageFilter::HasFiles => "has_files",
        MessageFilter::Mentions => "mentions",
        MessageFilter::Pinned => "pinned",
    }
}

/// Sort menu items; `by_topic` offers Topic (All topics only).
fn sort_items(view: &ListView, by_topic: bool) -> Vec<MenuItem> {
    let mut items = vec![MenuItem::header("Sort by")];
    let fields = if by_topic { vec![SortField::Topic] } else { Vec::new() };
    for f in fields.into_iter().chain(SortField::ALL) {
        items.push(MenuItem::new(format!("sort:{}", f.id()), f.label()).checked(view.sort == f));
    }
    items.push(MenuItem::divider());
    let (desc, asc) = view.sort.order_labels();
    // Show the natural direction first.
    let natural_desc = view.sort.default_descending();
    let (first, second) = if natural_desc { ((desc, true), (asc, false)) } else { ((asc, false), (desc, true)) };
    for (label, d) in [first, second] {
        items.push(MenuItem::new(if d { "order:desc" } else { "order:asc" }, label).checked(view.descending == d));
    }
    items
}

/// Tri-state "Select all" box for select mode and the columns header.
#[component]
pub fn SelectAll() -> Element {
    let app = use_app();
    let ctx = use_mail();
    let model = use_model();
    let total = model.read().keys.len();
    let selected = ctx.selection.read().len();
    let state = if selected == 0 { "false" } else if selected >= total && total > 0 { "true" } else { "mixed" };
    let any = state != "false";
    let icon = match state {
        "true" => "checkmark_circle_filled",
        "mixed" => "subtract",
        _ => "circle",
    };
    rsx! {
        button {
            r#type: "button",
            class: "ax-mail-selectall",
            class: if any { "ax-checked" },
            role: "checkbox",
            "aria-checked": state,
            title: if state == "true" { "Deselect all" } else { "Select all" },
            "aria-label": "Select all",
            onclick: move |e: MouseEvent| {
                e.stop_propagation();
                if state == "true" {
                    ctx.clear_selection();
                } else {
                    let m = model.read();
                    actions::run(app, ctx, &m, "select_all", Vec::new());
                }
            },
            Icon { name: icon, size: 20 }
        }
    }
}

/// The topics shown (not hidden in Settings > Mail > Topics), in order.
pub fn visible_topics(settings: &Settings) -> Vec<topics::Topic> {
    topics::Topic::ALL.into_iter().filter(|t| !settings.topics_hidden.iter().any(|h| h == t.id())).collect()
}

/// The pinned desktop view's switcher items: the Inbox, Packages, and
/// Topics with each shown topic (for the features that are on). The Inbox
/// is checked only when it is what the list shows (`inbox_shown`), not for
/// any other folder ("Show in folder" can open Archive there).
pub fn switcher_items(settings: &Settings, current: Option<MailView>, inbox_shown: bool) -> Vec<MenuItem> {
    let mut items = vec![MenuItem::new("sw_inbox", "Inbox").icon("mail_inbox").checked(current.is_none() && inbox_shown)];
    if settings.order_tracking {
        items.push(MenuItem::new("sw_packages", "Packages").icon("box").checked(current == Some(MailView::Packages)));
    }
    if settings.topics_view {
        let mut sub = vec![MenuItem::new("sw_topic:all", "All topics").icon("tag").checked(current == Some(MailView::Topics(None)))];
        sub.extend(
            visible_topics(settings)
                .into_iter()
                .map(|t| MenuItem::new(format!("sw_topic:{}", t.id()), t.label()).icon(t.icon()).checked(current == Some(MailView::Topics(Some(t))))),
        );
        items.push(MenuItem::new("sw_topics", "Topics").icon("tag").submenu(sub));
    }
    items
}

/// Run a switcher item.
pub fn run_switcher(app: AppState, ctx: MailCtx, id: &str) {
    match id {
        "sw_inbox" => actions::go_to_inbox(app, ctx),
        "sw_packages" => actions::show_view(app, ctx, MailView::Packages),
        other => {
            if let Some(t) = other.strip_prefix("sw_topic:") {
                actions::show_view(app, ctx, MailView::Topics(topics::Topic::parse(t)));
            }
        }
    }
}

/// The title of the pinned desktop view: a menu that switches between the
/// Inbox, Packages and the topics (the folder pane is hidden there).
#[component]
pub fn ViewSwitcher(#[props(into)] title: String) -> Element {
    let app = use_app();
    let ctx = use_mail();
    let settings = app.settings.read().clone();
    let model = use_model();
    let current = *ctx.view.read();
    let inbox_shown = model.read().folder.as_ref().is_some_and(|f| f.role == FolderRole::Inbox);
    let items = switcher_items(&settings, current, inbox_shown);
    rsx! {
        MenuButton {
            label: title.clone(),
            icon: "grid_dots",
            variant: Variant::Subtle,
            class: "ax-mail-switcher",
            title: "Switch between the Inbox, Packages and topics",
            items,
            on_select: move |id: String| run_switcher(app, ctx, &id),
        }
    }
}

/// Title text of what the list shows.
fn view_title(v: MailView) -> String {
    match v {
        MailView::Packages => "Packages".into(),
        MailView::Topics(None) => "Topics".into(),
        MailView::Topics(Some(t)) => t.label().into(),
    }
}

#[component]
pub fn ListHeader(narrow: bool, on_jump: EventHandler<String>) -> Element {
    let app = use_app();
    let mut ctx = use_mail();
    let model = use_model();
    let m = model.read();
    let settings = app.settings.read().clone();
    let folder = m.folder.clone();
    let view = ctx.view(m.view_key());
    let select_mode = *ctx.select_mode.read();
    let sel_count = ctx.selection.read().len();
    let searching = m.searching;
    let mail_view = m.view.filter(|_| !searching);
    let title = if searching {
        "Results".to_string()
    } else if let Some(v) = mail_view {
        view_title(v)
    } else if let Some(p) = &m.person {
        p.name.clone()
    } else {
        folder.as_ref().map(|f| f.name.clone()).unwrap_or_default()
    };
    let title_icon = match mail_view {
        Some(MailView::Topics(Some(t))) => Some(t.icon()),
        Some(MailView::Topics(None)) => Some("tag"),
        _ => None,
    };
    let topic_view = match mail_view {
        Some(MailView::Topics(t)) => Some(t),
        _ => None,
    };
    let desktop = settings.desktop_view();
    // While topics are still being sorted (not counted when it's off).
    let _ = app.mail_rev();
    let sorting = if topic_view.is_some() { store().topics_pending_count(10_000).unwrap_or(0) } else { 0 };
    let topic_items: Vec<(String, String)> = std::iter::once(("all".to_string(), "All".to_string()))
        .chain(visible_topics(&settings).into_iter().map(|t| (t.id().to_string(), t.label().to_string())))
        .collect();
    let topic_selected = topic_view.flatten().map(|t| t.id().to_string()).unwrap_or_else(|| "all".into());
    // The Topic pivot scrolls sideways: keep the selected topic in view
    // (also when it was picked in the folder pane).
    let pivot_tab = topic_view.map(|_| topic_selected.clone());
    use_effect(use_reactive!(|pivot_tab| {
        if pivot_tab.is_some() {
            dom::run(
                "setTimeout(()=>{const el=document.querySelector('.ax-mail-topic-pivot .ax-pivot-tab.ax-selected');if(el)el.scrollIntoView({block:'nearest',inline:'nearest'});},0)",
            );
        }
    }));
    let is_fav = folder.as_ref().is_some_and(|f| f.is_favorite);
    let pivot = !searching && settings.focused_inbox_shown() && folder.as_ref().is_some_and(|f| f.role == FolderRole::Inbox);
    let tab = *ctx.tab.read();
    let groups = m.groups.clone();
    let total = m.total;
    let other_new = pivot && tab == InferenceClass::Focused && m.other_has_new;
    // Looking at Other (or leaving it) counts as having seen its new mail.
    let mut show_tab = move |class: InferenceClass| {
        let was = *ctx.tab.peek();
        if was == InferenceClass::Other || class == InferenceClass::Other {
            ctx.other_seen.set(Some(chrono::Utc::now()));
        }
        ctx.tab.set(class);
        ctx.clear_selection();
        ctx.focus.set(None);
    };
    let folder_id = m.view_key();

    let mut filter_items: Vec<MenuItem> = FILTERS
        .iter()
        .map(|f| MenuItem::new(format!("filter:{}", filter_id(*f)), filter_label(*f)).checked(view.filter == *f))
        .collect();
    filter_items.push(MenuItem::divider());
    let by_topic = topic_view == Some(None);
    filter_items.extend(sort_items(&view, by_topic));

    let on_view_pick = move |id: String| {
        if let Some(f) = id.strip_prefix("filter:") {
            if let Some(filter) = FILTERS.iter().copied().find(|x| filter_id(*x) == f) {
                ctx.set_view(folder_id, |v| v.filter = filter);
            }
        } else if let Some(f) = id.strip_prefix("sort:").and_then(SortField::parse) {
            ctx.set_view(folder_id, |v| {
                v.sort = f;
                v.descending = f.default_descending();
            });
        } else if id == "order:desc" || id == "order:asc" {
            ctx.set_view(folder_id, |v| v.descending = id == "order:desc");
        }
        ctx.clear_selection();
    };
    let jump_items: Vec<MenuItem> = groups.iter().map(|g| MenuItem::new(format!("jump:{g}"), g.clone())).collect();
    let sort_label = format!("By {}", if view.sort == SortField::Flag { "Flag status".to_string() } else { view.sort.label().to_string() });
    let filter_active = view.filter != MessageFilter::All;

    rsx! {
        div { class: "ax-mail-list-header",
            div { class: "ax-mail-title-row",
                if searching {
                    IconButton { icon: "arrow_left", title: "Exit search", onclick: move |_| actions::exit_search(app) }
                }
                if select_mode { SelectAll {} }
                if desktop && !searching {
                    ViewSwitcher { title: title.clone() }
                } else {
                    if let Some(icon) = title_icon {
                        span { class: "ax-mail-title-icon", Icon { name: icon, size: 20 } }
                    }
                    h2 { class: "ax-mail-title ax-ellipsis", title: "{title}", "{title}" }
                }
                if let Some(f) = folder.clone().filter(|_| !searching) {
                    IconButton {
                        icon: if is_fav { "star_filled" } else { "star" },
                        title: if is_fav { "Remove from Favorites" } else { "Add to Favorites" },
                        class: if is_fav { "ax-mail-star ax-on" } else { "ax-mail-star" },
                        onclick: move |_| {
                            if app.check(store().set_folder_favorite(f.id, !is_fav)).is_some() {
                                app.bump_mail();
                            }
                        },
                    }
                }
                if searching {
                    span { class: "ax-mail-count", role: "status", if total == 1 { "1 result" } else { "{total} results" } }
                } else if select_mode {
                    span { class: "ax-mail-count", role: "status", "{sel_count} selected" }
                }
                span { class: "ax-grow" }
                div { class: "ax-mail-header-tools",
                    button {
                        r#type: "button",
                        class: "ax-mail-tool",
                        class: if select_mode { "ax-on" },
                        title: if select_mode { "Exit selection mode" } else { "Select" },
                        "aria-label": "Select",
                        "aria-pressed": if select_mode { "true" } else { "false" },
                        onclick: move |_| {
                            let on = !*ctx.select_mode.peek();
                            ctx.select_mode.set(on);
                            if !on && ctx.selection.peek().len() > 1 {
                                ctx.clear_selection();
                            }
                        },
                        Icon { name: "multiselect_ltr" }
                        if !narrow { span { "Select" } }
                    }
                    MenuButton {
                        label: "Jump to",
                        icon: "arrow_down",
                        icon_only: narrow,
                        chevron: false,
                        variant: Variant::Subtle,
                        class: "ax-mail-tool",
                        disabled: groups.is_empty(),
                        items: jump_items,
                        on_select: move |id: String| {
                            if let Some(g) = id.strip_prefix("jump:") {
                                let mut groups = ctx.collapsed_groups.write();
                                groups.remove(g);
                                on_jump.call(g.to_string());
                            }
                        },
                    }
                    MenuButton {
                        label: "Filter",
                        icon: if filter_active { "filter_dismiss" } else { "filter" },
                        icon_only: narrow,
                        chevron: false,
                        variant: Variant::Subtle,
                        class: if filter_active { "ax-mail-tool ax-on" } else { "ax-mail-tool" },
                        items: filter_items,
                        on_select: move |id: String| on_view_pick(id),
                    }
                    MenuButton {
                        label: sort_label,
                        variant: Variant::Subtle,
                        class: "ax-mail-tool ax-mail-sort",
                        items: sort_items(&view, by_topic),
                        on_select: move |id: String| on_view_pick(id),
                    }
                }
            }
            if topic_view.is_some() {
                Pivot {
                    class: "ax-mail-pivot ax-mail-topic-pivot",
                    label: "Topics",
                    items: topic_items,
                    selected: topic_selected,
                    onselect: move |k: String| actions::show_view(app, ctx, MailView::Topics(topics::Topic::parse(&k))),
                }
                if sorting > 0 {
                    MessageBar { intent: Intent::Info, class: "ax-mail-sorting", text: "Sorting your mail on this device\u{2026} about {sorting} left" }
                }
                div { class: "ax-mail-topic-hint",
                    span { "Sorted on this device. Nothing is moved." }
                    LinkButton { label: "Change how this works", onclick: move |_| actions::open_settings_page(app, PageId::Topics) }
                }
            }
            if pivot {
                Pivot {
                    class: "ax-mail-pivot",
                    items: vec![("Focused".to_string(), "Focused".to_string()), ("Other".to_string(), "Other".to_string())],
                    selected: if tab == InferenceClass::Other { "Other" } else { "Focused" },
                    onselect: move |k: String| show_tab(if k == "Other" { InferenceClass::Other } else { InferenceClass::Focused }),
                }
            }
            if other_new {
                div { class: "ax-mail-other-bar", role: "status",
                    Icon { name: "mail_inbox", size: 16 }
                    span { class: "ax-grow", "New messages in Other" }
                    button { r#type: "button", class: "ax-mail-link", onclick: move |_| show_tab(InferenceClass::Other), "View" }
                }
            }
            if filter_active {
                div { class: "ax-mail-chiprow",
                    button {
                        r#type: "button",
                        class: "ax-mail-filter-chip",
                        title: "Clear filter",
                        onclick: move |_| ctx.set_view(folder_id, |v| v.filter = MessageFilter::All),
                        span { "Filter: {filter_label(view.filter)}" }
                        Icon { name: "dismiss", size: 12 }
                    }
                }
            }
        }
    }
}

/// "From | Subject | Received" column headers; clicking one sorts by it.
#[component]
pub fn ColumnsHeader() -> Element {
    let ctx = use_mail();
    let model = use_model();
    let folder_id = model.read().view_key();
    let view = ctx.view(folder_id);
    let sort_by = move |f: SortField| {
        ctx.set_view(folder_id, |v| {
            if v.sort == f {
                v.descending = !v.descending;
            } else {
                v.sort = f;
                v.descending = f.default_descending();
            }
        });
    };
    let arrow = |f: SortField| -> Option<&'static str> {
        (view.sort == f).then_some(if view.descending { "chevron_down" } else { "chevron_up" })
    };
    rsx! {
        div { class: "ax-mail-cols", role: "row",
            span { class: "ax-mail-ccheck", SelectAll {} }
            button { r#type: "button", class: "ax-mail-colhead ax-mail-cfrom", role: "columnheader", onclick: move |_| sort_by(SortField::From),
                "From"
                if let Some(a) = arrow(SortField::From) { Icon { name: a, size: 12 } }
            }
            span { class: "ax-mail-cicons" }
            button { r#type: "button", class: "ax-mail-colhead ax-mail-csubject", role: "columnheader", onclick: move |_| sort_by(SortField::Subject),
                "Subject"
                if let Some(a) = arrow(SortField::Subject) { Icon { name: a, size: 12 } }
            }
            button { r#type: "button", class: "ax-mail-colhead ax-mail-ctime", role: "columnheader", onclick: move |_| sort_by(SortField::Date),
                "Received"
                if let Some(a) = arrow(SortField::Date) { Icon { name: a, size: 12 } }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_switcher_checks_the_inbox_only_when_it_is_shown() {
        let settings = Settings::default();
        let checked = |items: &[MenuItem], id: &str| items.iter().find(|i| i.id == id).and_then(|i| i.checked);
        assert_eq!(checked(&switcher_items(&settings, None, true), "sw_inbox"), Some(true));
        // Archive on screen ("Show in folder" from Packages): nothing checked.
        let archive = switcher_items(&settings, None, false);
        assert_eq!(checked(&archive, "sw_inbox"), Some(false));
        assert!(archive.iter().all(|i| i.checked != Some(true)));
        let packages = Settings { order_tracking: true, ..Settings::default() };
        assert_eq!(checked(&switcher_items(&packages, Some(MailView::Packages), true), "sw_inbox"), Some(false));
        assert_eq!(checked(&switcher_items(&packages, Some(MailView::Packages), true), "sw_packages"), Some(true));
    }
}
