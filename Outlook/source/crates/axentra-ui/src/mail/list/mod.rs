//! Message list: header, then a virtualised list of rows (docs/spec/mail.md
//! §3). Rows have fixed heights per layout and density, so only the rows in
//! view (plus overscan, plus the focused row) are rendered.

pub mod header;
pub mod row;
pub mod select;

use std::rc::Rc;

use axentra_core::*;
use chrono::Datelike;
use dioxus::html::geometry::PixelsVector2D;
use dioxus::prelude::*;

use super::ctx::{RowKey, SearchScope, use_mail, use_model};
use super::model::{Entry, ListModel};
use crate::kit::{Button, Icon, keys};
use crate::state::{use_app, store};
use header::{ColumnsHeader, ListHeader};
use row::{ChildItem, GroupHeader, MessageItem, RowLook};

/// Extra pixels rendered above and below the viewport (about ten rows).
const OVERSCAN: f64 = 600.0;

/// Positioned entries: (entry index, top, height).
fn layout(model: &ListModel, look: &RowLook) -> (Vec<(f64, f64)>, f64) {
    let mut y = 0.0;
    let mut out = Vec::with_capacity(model.entries.len());
    for e in &model.entries {
        let h = match e {
            Entry::Header { .. } => row::header_height(look),
            Entry::Row(r) => row::row_height(look, r),
            Entry::Child(_) => row::child_height(look),
        };
        out.push((y, h));
        y += h;
    }
    (out, y)
}

fn entry_key(e: &Entry) -> Option<&RowKey> {
    match e {
        Entry::Row(r) => Some(&r.key),
        Entry::Child(c) => Some(&c.key),
        Entry::Header { .. } => None,
    }
}

/// Empty-state copy per folder (§3.16).
fn empty_copy(model: &ListModel, tab: InferenceClass, filtered: bool, focused_inbox: bool) -> (&'static str, &'static str, &'static str) {
    if model.searching {
        return ("search", "No results found", "Try other keywords or change your filters.");
    }
    if filtered {
        return ("filter", "Nothing matches this filter", "");
    }
    let snoozed = model.folder.as_ref().is_some_and(|f| store().snoozed_folder_ids().unwrap_or_default().contains(&f.id));
    if snoozed {
        return ("snooze", "Nothing snoozed", "Snoozed messages wait here until it's time to bring them back.");
    }
    match model.folder.as_ref().map(|f| f.role) {
        Some(FolderRole::Inbox) if focused_inbox && tab == InferenceClass::Other => {
            ("mail_inbox", "Nothing in Other", "Less important messages will be kept here.")
        }
        Some(FolderRole::Inbox) => ("mail_inbox", "All clear", "New messages will show up here."),
        Some(FolderRole::Drafts) => ("drafts", "No drafts", "Messages you start but don't send are saved here."),
        Some(FolderRole::Sent) => ("send", "Nothing sent yet", "Messages you send will be listed here."),
        Some(FolderRole::Deleted) => ("delete", "Deleted Items is empty", "Items you delete stay here until this folder is emptied."),
        Some(FolderRole::Junk) => ("mail_prohibited", "No junk email", "Messages that look like junk are kept here for a limited time."),
        Some(FolderRole::Archive) => ("archive", "Nothing archived", "Archive messages to keep them without cluttering your Inbox."),
        _ => ("folder_open", "This folder is empty", ""),
    }
}

#[component]
fn ListEmpty() -> Element {
    let app = use_app();
    let mut ctx = use_mail();
    let model = use_model();
    let m = model.read();
    let settings = app.settings.read();
    let view = ctx.view(m.view_key());
    let tab = *ctx.tab.read();
    if let Some(err) = &m.error {
        return rsx! {
            div { class: "ax-mail-empty", role: "status",
                Icon { name: "cloud_off", size: 96 }
                div { class: "ax-mail-empty-title", "We couldn't load this folder" }
                div { class: "ax-mail-empty-body", "Check your connection and try again." }
                div { class: "ax-mail-empty-detail", "{err}" }
                Button { label: "Try again", onclick: move |_| app.bump_mail() }
            }
        };
    }
    let filtered = view.filter != MessageFilter::All;
    let (icon, title, body) = empty_copy(&m, tab, filtered, settings.focused_inbox_shown());
    // A favourite person with no mail yet.
    let person_title = m.person.as_ref().filter(|_| !m.searching && !filtered).map(|p| format!("No messages from {}", p.name));
    let icon = if person_title.is_some() { "person" } else { icon };
    let title = person_title.as_deref().unwrap_or(title);
    let narrow_scope = m.searching && *ctx.search_scope.read() == SearchScope::CurrentFolder;
    let folder_id = m.view_key();
    rsx! {
        div { class: "ax-mail-empty", role: "status",
            Icon { name: icon, size: 96 }
            div { class: "ax-mail-empty-title", "{title}" }
            if !body.is_empty() { div { class: "ax-mail-empty-body", "{body}" } }
            if filtered && !m.searching {
                Button { label: "Clear filter", onclick: move |_| ctx.set_view(folder_id, |v| v.filter = MessageFilter::All) }
            }
            if narrow_scope {
                Button { label: "Search all folders", onclick: move |_| ctx.search_scope.set(SearchScope::AllFolders) }
            }
        }
    }
}

#[component]
pub fn MessageList(columns: bool, #[props(default)] style: String) -> Element {
    let app = use_app();
    let mut ctx = use_mail();
    let model = use_model();
    let settings = app.settings.read().clone();
    let mut scroll_top = use_signal(|| 0.0f64);
    let mut viewport = use_signal(|| 800.0f64);
    let mut width = use_signal(|| 400.0f64);
    let mut scroller = use_signal(|| None::<Rc<MountedData>>);

    let m: Rc<ListModel> = model.read().clone();
    let look = RowLook {
        columns,
        density: settings.density,
        preview: settings.show_preview_text,
        avatar: settings.show_sender_images,
        select_mode: *ctx.select_mode.read(),
        h24: settings.use_24h_time,
        conversations: m.conversations,
    };
    let (pos, total_h) = layout(&m, &look);
    let selection = ctx.selection.read().clone();
    // Dragging a selected row drags the whole selection (other modules read
    // the ids from the drag data, see `keyboard::BOOT_JS`).
    let selected_ids = ids_csv(&m.ids_for(&selection));
    let focus = ctx.focus.read().clone();
    let expanded = ctx.expanded.read().clone();
    let today = ctx.today.read().num_days_from_ce() as i64;

    // Scroll requests: keep the requested row in view.
    {
        let m = m.clone();
        let pos = pos.clone();
        let look_h = look;
        let mut seen = use_signal(|| 0u64);
        let req = ctx.scroll_to.read().clone();
        use_effect(use_reactive!(|req| {
            let Some((serial, key)) = req else { return };
            if serial == *seen.peek() {
                return;
            }
            seen.set(serial);
            let Some(i) = m.entries.iter().position(|e| entry_key(e) == Some(&key)) else { return };
            let (top, h) = pos[i];
            let st = *scroll_top.peek();
            let vh = *viewport.peek();
            let header = if i > 0 && matches!(m.entries[i - 1], Entry::Header { .. }) { row::header_height(&look_h) } else { 0.0 };
            let target = if top - header < st {
                Some(top - header)
            } else if top + h > st + vh {
                Some(top + h - vh)
            } else {
                None
            };
            if let (Some(y), Some(el)) = (target, scroller.peek().clone()) {
                scroll_top.set(y.max(0.0));
                spawn(async move {
                    let _ = el.scroll(PixelsVector2D::new(0.0, y.max(0.0)), ScrollBehavior::Instant).await;
                });
            }
        }));
    }
    // Focus requests (clicks, Escape from the tree, ...).
    {
        let mut seen = use_signal(|| *ctx.focus_list.peek());
        use_effect(move || {
            let req = *ctx.focus_list.read();
            if req != *seen.peek() {
                seen.set(req);
                if let Some(el) = scroller.peek().clone() {
                    spawn(async move {
                        let _ = el.set_focus(true).await;
                    });
                }
            }
        });
    }
    // Back to the top when the tab or search changes, and to the remembered
    // position when coming back to a folder (§3.14).
    {
        let folder = *ctx.folder.read();
        let tab = *ctx.tab.read();
        let serial = app.search.read().serial;
        let person = ctx.person.read().as_ref().map(|p| p.contact);
        let mut prev = use_signal(|| (folder, tab, serial, person));
        use_effect(use_reactive!(|folder, tab, serial, person| {
            let (prev_folder, _, _, prev_person) = *prev.peek();
            if *prev.peek() == (folder, tab, serial, person) {
                return;
            }
            prev.set((folder, tab, serial, person));
            let searching = app.search.peek().active;
            let back_to_folder = prev_folder != folder || prev_person.is_some();
            let y = match folder {
                Some(f) if back_to_folder && person.is_none() && !searching => ctx.memory.peek().get(&f).map(|m| m.scroll).unwrap_or(0.0),
                _ => 0.0,
            };
            scroll_top.set(y);
            ctx.list_scroll.set(y);
            if let Some(el) = scroller.peek().clone() {
                spawn(async move {
                    let _ = el.scroll(PixelsVector2D::new(0.0, y), ScrollBehavior::Instant).await;
                });
            }
        }));
    }

    let st = scroll_top();
    let vh = viewport();
    let lo = st - OVERSCAN;
    let hi = st + vh + OVERSCAN;
    let first = pos.partition_point(|(top, h)| top + h < lo);
    let last = pos.partition_point(|(top, _)| *top < hi);
    let mut visible: Vec<usize> = (first..last.max(first)).collect();
    // The focused row stays mounted so keyboard focus never breaks.
    if let Some(fi) = focus.as_ref().and_then(|k| m.entries.iter().position(|e| entry_key(e) == Some(k)))
        && !visible.contains(&fi)
    {
        visible.push(fi);
    }
    let setsize = m.keys.len();
    let highlight = Rc::new(m.highlight.clone());
    // The group the top of the viewport is in keeps its header visible.
    let sticky = (st > 0.0)
        .then(|| m.entries.iter().zip(pos.iter()).take_while(|(_, (top, _))| *top <= st).filter(|(e, _)| matches!(e, Entry::Header { .. })).last())
        .flatten()
        .and_then(|(e, (top, _))| match e {
            Entry::Header { label, count, collapsed } if *top < st => Some((label.clone(), *count, *collapsed)),
            _ => None,
        });
    let active_id = focus.as_ref().filter(|k| m.index_of(k).is_some()).map(|k| k.dom_id());
    let narrow = width() < 520.0;
    let page = ((vh / row::row_height(&look, &dummy_row())).floor() as usize).max(1);

    let m_keys = m.clone();
    let onkeydown = move |e: KeyboardEvent| {
        let chord = keys::chord(&e);
        if select::list_key(app, ctx, m_keys.clone(), &chord, page) {
            e.prevent_default();
            e.stop_propagation();
        }
    };
    let m_jump = m.clone();
    let pos_jump = pos.clone();
    let on_jump = move |label: String| {
        let Some(i) = m_jump.entries.iter().position(|e| matches!(e, Entry::Header { label: l, .. } if *l == label)) else { return };
        let y = pos_jump[i].0;
        scroll_top.set(y);
        if let Some(el) = scroller.peek().clone() {
            spawn(async move {
                let _ = el.scroll(PixelsVector2D::new(0.0, y), ScrollBehavior::Instant).await;
            });
        }
    };
    let m_focus = m.clone();
    let list_role = if m.conversations { "tree" } else { "listbox" };
    let label = if m.searching {
        "Search results".to_string()
    } else if let Some(p) = &m.person {
        format!("Mail from {}", p.name)
    } else {
        m.folder.as_ref().map(|f| f.name.clone()).unwrap_or_else(|| "Message list".into())
    };
    // Sign-in, offline and first-sync status for the folder's account.
    let health_account = ctx.folder.read().and_then(|f| crate::state::store().folder(f).ok().flatten()).map(|f| f.account_id);
    let empty = m.keys.is_empty() && m.entries.iter().all(|e| !matches!(e, Entry::Header { collapsed: true, .. }));

    rsx! {
        section {
            class: "ax-mail-list",
            class: if columns { "ax-columns" },
            role: "region",
            "aria-label": "Message list",
            "data-ax-region": "true",
            style: "{style}",
            onresize: move |e: ResizeEvent| {
                if let Ok(size) = e.data().get_border_box_size() {
                    width.set(size.width);
                }
            },
            ListHeader { narrow, on_jump }
            if let Some(account_id) = health_account { crate::settings::accounts::AccountHealthBar { account_id } }
            if columns && !empty { ColumnsHeader {} }
            if empty {
                ListEmpty {}
            } else {
                div {
                    class: "ax-mail-scroll",
                    role: list_role,
                    tabindex: "0",
                    "data-ax-default-focus": "true",
                    "aria-label": "{label}",
                    "aria-multiselectable": "true",
                    "aria-activedescendant": active_id.unwrap_or_default(),
                    "data-selected-message-ids": "{selected_ids}",
                    onmounted: move |e| scroller.set(Some(e.data())),
                    onscroll: move |e: ScrollEvent| {
                        let y = e.data().scroll_top();
                        ctx.list_scroll.set(y);
                        if (y - *scroll_top.peek()).abs() >= 1.0 {
                            scroll_top.set(y);
                        }
                    },
                    onresize: move |e: ResizeEvent| {
                        if let Ok(size) = e.data().get_content_box_size()
                            && (size.height - *viewport.peek()).abs() >= 1.0
                        {
                            viewport.set(size.height);
                        }
                    },
                    onfocus: move |_| {
                        if ctx.focus.peek().is_none() {
                            let k = ctx.selection.peek().last().cloned().or_else(|| m_focus.keys.first().cloned());
                            ctx.focus.set(k);
                        }
                    },
                    onkeydown,
                    if let Some((label, count, collapsed)) = sticky {
                        div { class: "ax-mail-sticky", "aria-hidden": "true",
                            GroupHeader { key: "sticky", label, count, collapsed, look, top: 0.0, height: row::header_height(&look) }
                        }
                    }
                    div { class: "ax-mail-canvas", style: "height:{total_h}px",
                        for i in visible {
                            {
                                let (top, height) = pos[i];
                                match &m.entries[i] {
                                    Entry::Header { label, count, collapsed } => rsx! {
                                        GroupHeader { key: "h-{label}", label: label.clone(), count: *count, collapsed: *collapsed, look, top, height }
                                    },
                                    Entry::Row(r) => {
                                        let selected = selection.contains(&r.key);
                                        let is_expanded = expanded.as_deref() == Some(r.latest.conversation_id.as_str()) && r.count > 1;
                                        rsx! {
                                            MessageItem {
                                                key: "{r.key.dom_id()}",
                                                row: r.clone(),
                                                look,
                                                selected,
                                                focused: focus.as_ref() == Some(&r.key),
                                                expanded: is_expanded,
                                                top,
                                                height,
                                                posinset: m.index_of(&r.key).map(|p| p + 1).unwrap_or(0),
                                                setsize,
                                                sel_count: if selected { selection.len() } else { 0 },
                                                now_min: today,
                                                highlight: highlight.clone(),
                                            }
                                        }
                                    }
                                    Entry::Child(c) => rsx! {
                                        ChildItem {
                                            key: "{c.key.dom_id()}",
                                            child: c.clone(),
                                            look,
                                            selected: selection.contains(&c.key),
                                            focused: focus.as_ref() == Some(&c.key),
                                            top,
                                            height,
                                            now_min: today,
                                        }
                                    },
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// A plain row, for page-size estimates.
fn dummy_row() -> super::model::Row {
    super::model::Row {
        key: RowKey::Msg(0),
        latest: Message::default(),
        ids: Vec::new(),
        count: 1,
        unread: 0,
        people: String::new(),
        flag: FlagStatus::NotFlagged,
        pinned: false,
        has_attachments: false,
        importance: Importance::Normal,
        mentions_me: false,
        meeting: None,
        advice: Default::default(),
        categories: Vec::new(),
        draft: false,
        folder_tag: None,
        attachments: Vec::new(),
        responded: None,
        open: super::ctx::Open { message: 0, conversation: None, mark: Vec::new() },
    }
}

/// Message ids as drag data: "12,13,14".
pub(crate) fn ids_csv(ids: &[axentra_core::MessageId]) -> String {
    ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",")
}
