//! Message list header: folder title and favourite star, Select, Jump to,
//! Filter and sort controls, the Focused/Other pivot, the active-filter chip
//! and the column headers of the single-line layout (docs/spec/mail.md §3.1–3.3).

use axentra_core::*;
use dioxus::prelude::*;

use super::super::actions;
use super::super::ctx::{ListView, SortField, filter_label, use_mail, use_model};
use crate::kit::{Icon, IconButton, MenuButton, MenuItem, Pivot, Variant};
use crate::state::{store, use_app};

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

fn sort_items(view: &ListView) -> Vec<MenuItem> {
    let mut items = vec![MenuItem::header("Sort by")];
    for f in SortField::ALL {
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
    let title = if searching {
        "Results".to_string()
    } else if let Some(p) = &m.person {
        p.name.clone()
    } else {
        folder.as_ref().map(|f| f.name.clone()).unwrap_or_default()
    };
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
    filter_items.extend(sort_items(&view));

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
                h2 { class: "ax-mail-title ax-ellipsis", title: "{title}", "{title}" }
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
                        items: sort_items(&view),
                        on_select: move |id: String| on_view_pick(id),
                    }
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
