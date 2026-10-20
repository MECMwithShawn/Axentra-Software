//! Message list rows: three-line items (reading pane on the right), the
//! single-line columns layout (bottom / hidden), group headers and the
//! children of an expanded conversation (docs/spec/mail.md §3.5–3.11).

use std::rc::Rc;

use axentra_core::*;
use chrono::{DateTime, Datelike, Local, Utc};
use dioxus::prelude::*;

use super::super::actions;
use super::super::ctx::{ContextMenu, Drag, MenuTarget, RowKey, SearchScope, use_mail, use_model};
use super::super::menus::{self, Target};
use super::super::model::{ChildRow, Row, split_highlight};
use super::select;
use crate::kit::{Avatar, Icon, category_colors};
use crate::state::{SearchState, use_app};

/// How rows look: layout, density and the list toggles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RowLook {
    pub columns: bool,
    pub density: Density,
    pub preview: bool,
    pub avatar: bool,
    pub select_mode: bool,
    pub h24: bool,
    pub conversations: bool,
}

/// Row heights in px. Three-line values are estimates from the spec (§3.5);
/// the columns layout row is MEASURED at 37px.
pub fn row_height(look: &RowLook, row: &Row) -> f64 {
    if look.columns {
        return columns_height(look.density);
    }
    let chips = if !row.attachments.is_empty() && look.density == Density::Roomy { 30.0 } else { 0.0 };
    match (look.density, look.preview) {
        (Density::Roomy, true) => 84.0 + chips,
        (Density::Roomy, false) => 64.0 + chips,
        (Density::Cozy, true) => 76.0,
        (Density::Cozy, false) => 56.0,
        (Density::Compact, _) => 52.0,
    }
}

fn columns_height(d: Density) -> f64 {
    match d {
        Density::Roomy => 41.0,
        Density::Cozy => 37.0, // MEASURED
        Density::Compact => 32.0,
    }
}

pub fn header_height(look: &RowLook) -> f64 {
    if look.columns { 32.0 } else { 28.0 }
}

pub fn child_height(look: &RowLook) -> f64 {
    if look.columns {
        columns_height(look.density)
    } else if look.density == Density::Compact {
        36.0
    } else {
        44.0
    }
}

/// List time label: time today, weekday + time this week, then the date
/// ("Thu 9/26" in the three-line layouts, "9/26/2026" in columns).
pub fn row_time(t: DateTime<Utc>, now: DateTime<Local>, h24: bool, columns: bool) -> String {
    let l = t.with_timezone(&Local);
    let (d, today) = (l.date_naive(), now.date_naive());
    if d >= today || d > today - chrono::Duration::days(7) || columns {
        return crate::fmt::list_time(t, now, h24);
    }
    if d.year() == today.year() { l.format("%a %-m/%-d").to_string() } else { l.format("%-m/%-d/%Y").to_string() }
}

fn long_time(t: DateTime<Utc>, h24: bool) -> String {
    let l = t.with_timezone(&Local);
    format!("{} {}", l.format("%A, %B %-d, %Y"), crate::fmt::time(t, h24))
}

fn accessible_name(row: &Row, time: &str) -> String {
    let mut parts = Vec::new();
    if row.unread > 0 {
        parts.push("Unread".to_string());
    }
    if row.importance == Importance::High {
        parts.push("Important".into());
    }
    match row.flag {
        FlagStatus::Flagged => parts.push("Flagged".into()),
        FlagStatus::Complete => parts.push("Completed".into()),
        FlagStatus::NotFlagged => {}
    }
    if row.pinned {
        parts.push("Pinned".into());
    }
    if row.has_attachments {
        parts.push("Has attachments".into());
    }
    match row.responded {
        Some(false) => parts.push("Replied".into()),
        Some(true) => parts.push("Forwarded".into()),
        None => {}
    }
    if row.meeting == Some(MeetingMessageKind::Request) {
        parts.push("Meeting request".into());
    }
    if row.draft {
        parts.push("Draft".into());
    }
    parts.push(row.people.clone());
    parts.push(if row.latest.subject.is_empty() { "(No subject)".into() } else { row.latest.subject.clone() });
    parts.push(time.to_string());
    if row.count > 1 {
        parts.push(format!("{} messages", row.count));
    }
    parts.push(row.latest.preview.clone());
    parts.join(", ")
}

/// Text with search terms marked (`--ax-highlight` background).
fn highlighted(text: &str, terms: &[String]) -> Element {
    if terms.is_empty() {
        return rsx! { "{text}" };
    }
    let runs = split_highlight(text, terms);
    rsx! {
        for (i, (t, hl)) in runs.into_iter().enumerate() {
            if hl {
                mark { key: "{i}", class: "ax-mail-hl", "{t}" }
            } else {
                span { key: "{i}", "{t}" }
            }
        }
    }
}

/// Status icons in Outlook's order (§3.6).
#[component]
fn StatusIcons(row: Rc<Row>) -> Element {
    rsx! {
        span { class: "ax-mail-icons",
            if row.importance == Importance::High {
                span { class: "ax-mail-ico ax-important", title: "High importance", Icon { name: "important", size: 16 } }
            }
            if row.importance == Importance::Low {
                span { class: "ax-mail-ico ax-low", title: "Low importance", Icon { name: "arrow_down", size: 16 } }
            }
            if row.mentions_me {
                span { class: "ax-mail-ico ax-mention", title: "You were mentioned", Icon { name: "mention", size: 16 } }
            }
            match row.meeting {
                Some(MeetingMessageKind::Request) => rsx! { span { class: "ax-mail-ico", title: "Meeting request", Icon { name: "calendar_ltr", size: 16 } } },
                Some(MeetingMessageKind::Accepted | MeetingMessageKind::Tentative) => rsx! { span { class: "ax-mail-ico", title: "Meeting response", Icon { name: "calendar_checkmark", size: 16 } } },
                Some(MeetingMessageKind::Declined | MeetingMessageKind::Cancellation) => rsx! { span { class: "ax-mail-ico", title: "Meeting cancelled or declined", Icon { name: "calendar_cancel", size: 16 } } },
                None => rsx! {},
            }
            if row.has_attachments {
                span { class: "ax-mail-ico", title: "Has attachments", Icon { name: "attach", size: 16 } }
            }
            match row.responded {
                Some(false) => rsx! { span { class: "ax-mail-ico", title: "You replied to this message", Icon { name: "arrow_reply", size: 16 } } },
                Some(true) => rsx! { span { class: "ax-mail-ico", title: "You forwarded this message", Icon { name: "arrow_forward", size: 16 } } },
                None => rsx! {},
            }
            if row.pinned {
                span { class: "ax-mail-ico ax-pin", title: "Pinned", Icon { name: "pin_filled", size: 16 } }
            }
            match row.flag {
                FlagStatus::Flagged => rsx! { span { class: "ax-mail-ico ax-flag", title: "Flagged", Icon { name: "flag_filled", size: 16 } } },
                FlagStatus::Complete => rsx! { span { class: "ax-mail-ico ax-complete", title: "Completed", Icon { name: "checkmark", size: 16 } } },
                FlagStatus::NotFlagged => rsx! {},
            }
        }
    }
}

/// Category pills: two at most, then "+N"; clicking one searches for it.
#[component]
pub fn CategoryPills(categories: Vec<String>, #[props(default = 2)] max: usize) -> Element {
    let mut app = use_app();
    let mut ctx = use_mail();
    let model = use_model();
    let colors = &model.read().category_colors;
    // From the Inbox a category search spans all folders; elsewhere it stays
    // in the folder (§3.7).
    let in_folder = model.read().folder.as_ref().is_some_and(|f| f.role != FolderRole::Inbox);
    let shown: Vec<(String, &'static str, &'static str)> = categories
        .iter()
        .take(max)
        .map(|c| {
            let (l, d) = category_colors(colors.get(c).map(String::as_str).unwrap_or_default());
            (c.clone(), l, d)
        })
        .collect();
    let more = categories.len().saturating_sub(max);
    let tip = categories.join(", ");
    rsx! {
        span { class: "ax-mail-pills", title: "{tip}",
            for (name, light, dark) in shown {
                button {
                    key: "{name}",
                    r#type: "button",
                    class: "ax-mail-pill",
                    tabindex: "-1",
                    style: "--ax-mail-cat-l:{light};--ax-mail-cat-d:{dark}",
                    onclick: {
                        let name = name.clone();
                        move |e: MouseEvent| {
                            e.stop_propagation();
                            ctx.search_scope.set(if in_folder { SearchScope::CurrentFolder } else { SearchScope::AllFolders });
                            let serial = app.search.peek().serial + 1;
                            app.search.set(SearchState { query: format!("category:\"{name}\""), active: true, serial });
                        }
                    },
                    "{name}"
                }
            }
            if more > 0 { span { class: "ax-mail-pill ax-more", "+{more}" } }
        }
    }
}

/// Hover quick actions: Delete, Read/Unread, Flag, Pin (§3.10). They act on
/// this row only and never change the selection.
#[component]
fn HoverActions(row: Rc<Row>) -> Element {
    let app = use_app();
    let ctx = use_mail();
    let model = use_model();
    let ids = row.ids.clone();
    let act = move |cmd: &'static str| {
        let ids = ids.clone();
        move |e: MouseEvent| {
            e.stop_propagation();
            let m = model.read();
            actions::run(app, ctx, &m, cmd, ids.clone());
        }
    };
    let unread = row.unread > 0;
    let flagged = row.flag == FlagStatus::Flagged;
    rsx! {
        span { class: "ax-mail-hover", "aria-hidden": "true",
            button { r#type: "button", tabindex: "-1", class: "ax-mail-hover-btn", title: "Delete (Delete)", onclick: act("delete"),
                Icon { name: "delete" }
            }
            button {
                r#type: "button",
                tabindex: "-1",
                class: "ax-mail-hover-btn",
                title: if unread { "Mark as read (Ctrl+Q)" } else { "Mark as unread (Ctrl+U)" },
                onclick: act(if unread { "read" } else { "unread" }),
                Icon { name: if unread { "mail_read" } else { "mail_unread" } }
            }
            button {
                r#type: "button",
                tabindex: "-1",
                class: "ax-mail-hover-btn",
                class: if flagged { "ax-on ax-flag" },
                title: if flagged { "Clear flag" } else { "Flag (Insert)" },
                onclick: act("flag_toggle"),
                Icon { name: if flagged { "flag_filled" } else { "flag" } }
            }
            button {
                r#type: "button",
                tabindex: "-1",
                class: "ax-mail-hover-btn",
                class: if row.pinned { "ax-on ax-pin" },
                title: if row.pinned { "Unpin" } else { "Pin" },
                onclick: act("toggle_pin"),
                Icon { name: if row.pinned { "pin_filled" } else { "pin" } }
            }
        }
    }
}

#[component]
fn RowCheck(checked: bool, key_: RowKey) -> Element {
    let app = use_app();
    let ctx = use_mail();
    let model = use_model();
    rsx! {
        span {
            class: "ax-mail-check",
            class: if checked { "ax-checked" },
            "aria-hidden": "true",
            onclick: move |e: MouseEvent| {
                e.stop_propagation();
                let m = model.read();
                select::toggle(app, ctx, &m, key_.clone());
            },
            Icon { name: if checked { "checkmark_circle_filled" } else { "circle" }, size: 20 }
        }
    }
}

fn open_context_menu(ctx: super::super::ctx::MailCtx, model: &super::super::model::ListModel, app: crate::state::AppState, key: &RowKey, x: f64, y: f64) {
    let mut ctx = ctx;
    if !ctx.selection.peek().contains(key) {
        select::click(app, ctx, model, key.clone(), false, false);
    }
    let ids = model.ids_for(&ctx.selection.peek());
    let settings = app.settings.peek().clone();
    let role = if model.searching { None } else { model.folder.as_ref().map(|f| f.role) };
    let folder = model.folder.as_ref().map(|f| f.id);
    let t = Target::for_ids(&ids, role, folder, settings.focused_inbox, settings.use_24h_time);
    let items = menus::message_menu(&t, folder);
    ctx.menu.set(Some(ContextMenu { x, y, items, target: MenuTarget::Messages(ids) }));
}

/// Open a row's context menu below the row (keyboard: Shift+F10).
pub fn open_context_menu_at_row(app: crate::state::AppState, ctx: super::super::ctx::MailCtx, model: Rc<super::super::model::ListModel>, key: RowKey) {
    spawn(async move {
        let js = format!(
            "const r = document.getElementById('{}')?.getBoundingClientRect(); return r ? [r.left + 48, r.top + Math.min(r.height, 28)] : [320, 200];",
            key.dom_id()
        );
        let pos = document::eval(&js).join::<Vec<f64>>().await.unwrap_or_else(|_| vec![320.0, 200.0]);
        open_context_menu(ctx, &model, app, &key, pos.first().copied().unwrap_or(320.0), pos.get(1).copied().unwrap_or(200.0));
    });
}

#[derive(Props, Clone, PartialEq)]
pub struct ItemProps {
    pub row: Rc<Row>,
    pub look: RowLook,
    pub selected: bool,
    pub focused: bool,
    pub expanded: bool,
    pub top: f64,
    pub height: f64,
    pub posinset: usize,
    pub setsize: usize,
    /// Size of the selection when this row is selected (0 otherwise), so
    /// rows don't re-render for unrelated selection changes.
    pub sel_count: usize,
    /// Minute counter so time labels refresh.
    pub now_min: i64,
    /// Search terms to mark in the subject and preview.
    pub highlight: Rc<Vec<String>>,
}

#[component]
pub fn MessageItem(props: ItemProps) -> Element {
    let app = use_app();
    let mut ctx = use_mail();
    let model = use_model();
    let ItemProps { row, look, selected, focused, expanded, top, height, posinset, setsize, sel_count, highlight, .. } = props;
    let now = Local::now();
    let time = row_time(row.latest.received_at, now, look.h24, look.columns);
    let tip = long_time(row.latest.received_at, look.h24);
    let label = accessible_name(&row, &time);
    let unread = row.unread > 0;
    let key = row.key.clone();
    let dom_id = key.dom_id();
    let is_conv = matches!(key, RowKey::Conv(_)) && row.count > 1;
    let subject = if row.latest.subject.trim().is_empty() { "(No subject)".to_string() } else { row.latest.subject.clone() };
    let drag_count = sel_count.max(1);
    let drag_ids = super::ids_csv(&row.ids);
    let density = match look.density {
        Density::Roomy => "ax-roomy",
        Density::Cozy => "ax-cozy",
        Density::Compact => "ax-compact",
    };
    let checkbox_visible = look.select_mode || sel_count > 1;
    let dragging = matches!(&*ctx.drag.read(), Some(Drag::Messages(ids)) if row.ids.iter().any(|i| ids.contains(i)));

    let (k_click, k_dbl, k_menu, k_drag, k_chev) = (key.clone(), key.clone(), key.clone(), key.clone(), key.clone());
    let row_ids = row.ids.clone();
    let conv_id = row.latest.conversation_id.clone();
    let onclick = move |e: MouseEvent| {
        let m = e.modifiers();
        let model = model.read();
        select::click(app, ctx, &model, k_click.clone(), m.ctrl() || m.meta(), m.shift());
        ctx.request_list_focus();
    };
    let ondoubleclick = move |_| {
        let model = model.read();
        select::activate(app, ctx, &model, k_dbl.clone());
    };
    let oncontextmenu = move |e: MouseEvent| {
        e.prevent_default();
        let c = e.client_coordinates();
        let model = model.read();
        open_context_menu(ctx, &model, app, &k_menu, c.x, c.y);
    };
    let ondragstart = move |_| {
        let model = model.read();
        let sel = ctx.selection.peek().clone();
        let ids = if sel.contains(&k_drag) { model.ids_for(&sel) } else { row_ids.clone() };
        ctx.drag.set(Some(Drag::Messages(ids)));
    };
    let ondragend = move |_| {
        ctx.drag.set(None);
        ctx.drop_target.set(None);
    };
    let toggle_conv = move |e: MouseEvent| {
        e.stop_propagation();
        let open = ctx.expanded.peek().as_deref() == Some(conv_id.as_str());
        ctx.expanded.set(if open { None } else { Some(conv_id.clone()) });
        ctx.focus.set(Some(k_chev.clone()));
    };

    let common_class = format!(
        "{}{}{}{}{}",
        if unread { " ax-unread" } else { "" },
        if selected { " ax-selected" } else { "" },
        if focused { " ax-focused" } else { "" },
        if checkbox_visible { " ax-checking" } else { "" },
        if dragging { " ax-dragging" } else { "" },
    );
    let role_attr = if look.conversations { "treeitem" } else { "option" };

    if look.columns {
        return rsx! {
            div {
                id: "{dom_id}",
                class: "ax-mail-crow {density}{common_class}",
                role: role_attr,
                "aria-selected": if selected { "true" } else { "false" },
                "aria-expanded": if is_conv { if expanded { "true" } else { "false" } },
                "aria-label": "{label}",
                "aria-posinset": "{posinset}",
                "aria-setsize": "{setsize}",
                "data-drag-count": "{drag_count}",
                "data-message-ids": "{drag_ids}",
                draggable: "true",
                style: "top:{top}px;height:{height}px",
                onclick,
                ondoubleclick,
                oncontextmenu,
                ondragstart,
                ondragend,
                if unread { span { class: "ax-mail-bar" } }
                span { class: "ax-mail-ccheck", RowCheck { checked: selected, key_: key.clone() } }
                span { class: "ax-mail-cfrom",
                    if is_conv {
                        button { r#type: "button", class: "ax-mail-chevron", tabindex: "-1", title: if expanded { "Collapse conversation" } else { "Expand conversation" }, onclick: toggle_conv,
                            Icon { name: if expanded { "chevron_down" } else { "chevron_right" }, size: 16 }
                        }
                    }
                    span { class: "ax-mail-sender ax-ellipsis", "{row.people}" }
                    if is_conv { span { class: "ax-mail-conv-count", "{row.count}" } }
                    HoverActions { row: row.clone() }
                }
                span { class: "ax-mail-cicons", StatusIcons { row: row.clone() } }
                span { class: "ax-mail-csubject ax-ellipsis",
                    if row.draft { span { class: "ax-mail-draft", "[Draft] " } }
                    span { class: "ax-mail-subject", {highlighted(&subject, &highlight)} }
                    if look.preview && !row.latest.preview.is_empty() {
                        span { class: "ax-mail-preview", {highlighted(&row.latest.preview, &highlight)} }
                    }
                }
                if let Some(tag) = &row.folder_tag { span { class: "ax-mail-tag", "{tag}" } }
                if !row.categories.is_empty() {
                    span { class: "ax-mail-ccats", CategoryPills { categories: row.categories.clone() } }
                }
                span { class: "ax-mail-ctime", title: "{tip}", "{time}" }
            }
        };
    }

    let show_avatar = look.avatar;
    let compact = look.density == Density::Compact;
    rsx! {
        div {
            id: "{dom_id}",
            class: "ax-mail-item {density}{common_class}",
            class: if show_avatar { "ax-has-avatar" },
            role: role_attr,
            "aria-selected": if selected { "true" } else { "false" },
            "aria-expanded": if is_conv { if expanded { "true" } else { "false" } },
            "aria-label": "{label}",
            "aria-posinset": "{posinset}",
            "aria-setsize": "{setsize}",
            "data-drag-count": "{drag_count}",
            "data-message-ids": "{drag_ids}",
            draggable: "true",
            style: "top:{top}px;height:{height}px",
            onclick,
            ondoubleclick,
            oncontextmenu,
            ondragstart,
            ondragend,
            if unread { span { class: "ax-mail-bar" } }
            span { class: "ax-mail-lead",
                if show_avatar {
                    span { class: "ax-mail-avatar",
                        Avatar { name: row.latest.from.name.clone(), email: row.latest.from.address.clone(), size: 32 }
                    }
                }
                RowCheck { checked: selected, key_: key.clone() }
            }
            div { class: "ax-mail-main",
                div { class: "ax-mail-line1",
                    if is_conv {
                        button { r#type: "button", class: "ax-mail-chevron", tabindex: "-1", title: if expanded { "Collapse conversation" } else { "Expand conversation" }, onclick: toggle_conv,
                            Icon { name: if expanded { "chevron_down" } else { "chevron_right" }, size: 16 }
                        }
                    }
                    span { class: "ax-mail-sender ax-ellipsis", "{row.people}" }
                    if is_conv { span { class: "ax-mail-conv-count", title: "{row.count} messages", "{row.count}" } }
                    if compact { span { class: "ax-mail-time", title: "{tip}", "{time}" } }
                    HoverActions { row: row.clone() }
                }
                div { class: "ax-mail-line2",
                    if row.draft { span { class: "ax-mail-draft", "[Draft]" } }
                    span { class: "ax-mail-subject ax-ellipsis",
                        {highlighted(&subject, &highlight)}
                        if compact && look.preview && !row.latest.preview.is_empty() {
                            span { class: "ax-mail-preview ax-inline", "\u{2003}" {highlighted(&row.latest.preview, &highlight)} }
                        }
                    }
                    StatusIcons { row: row.clone() }
                    if !compact { span { class: "ax-mail-time", title: "{tip}", "{time}" } }
                }
                if !compact && look.preview {
                    div { class: "ax-mail-line3",
                        span { class: "ax-mail-preview ax-ellipsis",
                            if row.latest.preview.is_empty() { "(No preview)" } else { {highlighted(&row.latest.preview, &highlight)} }
                        }
                        if let Some(tag) = &row.folder_tag { span { class: "ax-mail-tag", "{tag}" } }
                        if !row.categories.is_empty() { CategoryPills { categories: row.categories.clone() } }
                    }
                }
                if !compact && !look.preview && (!row.categories.is_empty() || row.folder_tag.is_some()) {
                    // Without preview text the pills sit at the end of line 2's row below.
                    div { class: "ax-mail-line3 ax-pills-only",
                        if let Some(tag) = &row.folder_tag { span { class: "ax-mail-tag", "{tag}" } }
                        CategoryPills { categories: row.categories.clone() }
                    }
                }
                if !row.attachments.is_empty() && look.density == Density::Roomy {
                    AttachmentChips { attachments: row.attachments.clone() }
                }
            }
        }
    }
}

/// Up to three attachment chips and "+N" (Roomy only).
#[component]
fn AttachmentChips(attachments: Vec<Attachment>) -> Element {
    let more = attachments.len().saturating_sub(3);
    rsx! {
        div { class: "ax-mail-chips",
            for a in attachments.iter().take(3).cloned() {
                span { key: "{a.id}", class: "ax-mail-chip", title: "{a.name}",
                    Icon { name: super::super::reading::file_icon(&a.name, &a.content_type), size: 16 }
                    span { class: "ax-ellipsis", "{a.name}" }
                }
            }
            if more > 0 { span { class: "ax-mail-chip ax-more", "+{more}" } }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct ChildProps {
    pub child: Rc<ChildRow>,
    pub look: RowLook,
    pub selected: bool,
    pub focused: bool,
    pub top: f64,
    pub height: f64,
    pub now_min: i64,
}

/// One message of an expanded conversation.
#[component]
pub fn ChildItem(props: ChildProps) -> Element {
    let app = use_app();
    let mut ctx = use_mail();
    let model = use_model();
    let ChildProps { child, look, selected, focused, top, height, .. } = props;
    let m = &child.msg;
    let time = row_time(m.received_at, Local::now(), look.h24, look.columns);
    let key = child.key.clone();
    let (k1, k2, k3) = (key.clone(), key.clone(), key.clone());
    let id = m.id;
    let who = if m.is_draft { format!("[Draft] {}", m.to.iter().map(|a| a.display().to_string()).collect::<Vec<_>>().join("; ")) } else { m.from.display().to_string() };
    let preview = if m.preview.is_empty() { "(No preview)".to_string() } else { m.preview.clone() };
    rsx! {
        div {
            id: "{key.dom_id()}",
            class: if look.columns { "ax-mail-child ax-columns" } else { "ax-mail-child" },
            class: if !m.is_read { "ax-unread" },
            class: if selected { "ax-selected" },
            class: if focused { "ax-focused" },
            class: if child.last { "ax-last" },
            role: "treeitem",
            "aria-level": "2",
            "aria-selected": if selected { "true" } else { "false" },
            "aria-label": "{who}, {time}, {preview}",
            "data-drag-count": "1",
            "data-message-ids": "{id}",
            draggable: "true",
            style: "top:{top}px;height:{height}px",
            onclick: move |e: MouseEvent| {
                let mo = e.modifiers();
                let model = model.read();
                select::click(app, ctx, &model, k1.clone(), mo.ctrl() || mo.meta(), mo.shift());
                ctx.request_list_focus();
            },
            ondoubleclick: move |_| {
                let model = model.read();
                select::activate(app, ctx, &model, k2.clone());
            },
            oncontextmenu: move |e: MouseEvent| {
                e.prevent_default();
                let c = e.client_coordinates();
                let model = model.read();
                open_context_menu(ctx, &model, app, &k3, c.x, c.y);
            },
            ondragstart: move |_| ctx.drag.set(Some(Drag::Messages(vec![id]))),
            ondragend: move |_| {
                ctx.drag.set(None);
                ctx.drop_target.set(None);
            },
            if !m.is_read { span { class: "ax-mail-bar" } }
            span { class: "ax-mail-child-line" }
            span { class: "ax-mail-sender ax-ellipsis", "{who}" }
            span { class: "ax-mail-preview ax-ellipsis", "{preview}" }
            if let Some(tag) = &child.folder_tag { span { class: "ax-mail-tag", "{tag}" } }
            if m.has_attachments { span { class: "ax-mail-ico", Icon { name: "attach", size: 16 } } }
            if m.flag == FlagStatus::Flagged { span { class: "ax-mail-ico ax-flag", Icon { name: "flag_filled", size: 16 } } }
            span { class: "ax-mail-time", "{time}" }
        }
    }
}

#[component]
pub fn GroupHeader(label: String, count: usize, collapsed: bool, look: RowLook, top: f64, height: f64) -> Element {
    let mut ctx = use_mail();
    let l2 = label.clone();
    rsx! {
        div {
            class: if look.columns { "ax-mail-group ax-columns" } else { "ax-mail-group" },
            role: "presentation",
            style: "top:{top}px;height:{height}px",
            onclick: move |_| {
                let mut groups = ctx.collapsed_groups.write();
                if !groups.remove(&l2) {
                    groups.insert(l2.clone());
                }
            },
            button {
                r#type: "button",
                class: "ax-mail-group-toggle",
                tabindex: "-1",
                "aria-expanded": if collapsed { "false" } else { "true" },
                title: if collapsed { "Expand group" } else { "Collapse group" },
                Icon { name: if collapsed { "chevron_right" } else { "chevron_down" }, size: 16 }
            }
            span { class: "ax-mail-group-label", "{label}" }
            if collapsed { span { class: "ax-mail-group-count", "{count}" } }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn row_time_formats() {
        let now = Local.with_ymd_and_hms(2026, 10, 19, 11, 30, 0).unwrap();
        let t = |y, m, d, h| Local.with_ymd_and_hms(y, m, d, h, 16, 0).unwrap().with_timezone(&Utc);
        assert_eq!(row_time(t(2026, 10, 19, 10), now, false, false), "10:16 AM");
        assert_eq!(row_time(t(2026, 10, 18, 20), now, false, false), "Sun 8:16 PM");
        assert_eq!(row_time(t(2026, 9, 26, 9), now, false, false), "Sat 9/26");
        assert_eq!(row_time(t(2026, 9, 26, 9), now, false, true), "9/26/2026");
        assert_eq!(row_time(t(2025, 9, 26, 9), now, false, false), "9/26/2025");
    }

    #[test]
    fn heights_by_density() {
        let look = RowLook { columns: false, density: Density::Roomy, preview: true, avatar: true, select_mode: false, h24: false, conversations: true };
        let mut row = Row {
            key: RowKey::Msg(1),
            latest: Message::default(),
            ids: vec![1],
            count: 1,
            unread: 0,
            people: String::new(),
            flag: FlagStatus::NotFlagged,
            pinned: false,
            has_attachments: false,
            importance: Importance::Normal,
            mentions_me: false,
            meeting: None,
            categories: Vec::new(),
            draft: false,
            folder_tag: None,
            attachments: Vec::new(),
            responded: None,
            open: super::super::super::ctx::Open { message: 1, conversation: None, mark: vec![] },
        };
        assert_eq!(row_height(&look, &row), 84.0);
        row.attachments.push(Attachment::default());
        assert_eq!(row_height(&look, &row), 114.0);
        assert_eq!(row_height(&RowLook { density: Density::Compact, ..look }, &row), 52.0);
        assert_eq!(row_height(&RowLook { columns: true, density: Density::Cozy, ..look }, &row), 37.0);
    }
}
