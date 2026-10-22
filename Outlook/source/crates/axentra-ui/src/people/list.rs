//! Contact list pane: header (title, count, sort), search box and the
//! virtualised list with index headings, selection and keyboard
//! (docs/spec/people-todo.md 2.5, keyboard spec 7 and 14).

use std::collections::HashSet;
use std::rc::Rc;
use std::time::{Duration, Instant};

use axentra_core::*;
use dioxus::prelude::*;

use super::actions;
use super::avatar::{PeopleAvatar, contact_color_key, list_color_key};
use super::ctx::{COLUMN_ROW_H, ContextMenu, HEADING_H, MULTI_COLUMN_W, MenuTarget, OVERSCAN_ROWS, PeopleCtx, row_height};
use super::text::fold;
use super::view::{Item, ListModel, Node, Row, SortBy, display_name, main_phone};
use crate::kit::{Button, EmptyState, Icon, MenuButton, MenuItem, Variant, keys};
use crate::state::{AppState, use_app};

const SCROLL_ID: &str = "ax-people-scroll";
pub const SEARCH_ID: &str = "ax-people-search";

/// Row tops (prefix sums) for the virtual list.
fn row_tops(model: &ListModel, row_h: f64) -> Vec<f64> {
    let mut tops = Vec::with_capacity(model.rows.len() + 1);
    let mut y = 0.0;
    tops.push(0.0);
    for r in &model.rows {
        y += if matches!(r, Row::Heading(_)) { HEADING_H } else { row_h };
        tops.push(y);
    }
    tops
}

#[component]
pub fn ListPane() -> Element {
    let app = use_app();
    let mut ctx = use_context::<PeopleCtx>();
    let model = ctx.model.read().clone();
    let node = ctx.node.read().clone();
    let density = app.settings.read().density;
    let width = *ctx.list_w.read();
    let columns = width >= MULTI_COLUMN_W;
    let row_h = if columns { COLUMN_ROW_H } else { row_height(density) };
    // Row offsets and the selection set change rarely; scrolling re-renders
    // often, so neither is rebuilt per frame.
    let tops_memo = use_memo(move || {
        let columns = *ctx.list_w.read() >= MULTI_COLUMN_W;
        let row_h = if columns { COLUMN_ROW_H } else { row_height(app.settings.read().density) };
        Rc::new(row_tops(&ctx.model.read(), row_h))
    });
    let selected = use_memo(move || Rc::new(ctx.selection.read().iter().cloned().collect::<HashSet<Item>>()));
    let tops = tops_memo.read().clone();
    let total = *tops.last().unwrap_or(&0.0);

    let mut scroll_top = use_signal(|| 0.0f64);
    let mut viewport = use_signal(|| 800.0f64);
    let mut jump_open = use_signal(|| false);
    let mut typeahead = use_signal(|| (String::new(), Instant::now()));
    // Favorites can be reordered by dragging (2.10).
    let fav_drop = use_signal(|| None::<Item>);

    // Move DOM focus / scroll to the focus row when asked.
    use_effect(move || {
        let (serial, move_focus) = (ctx.focus_req)();
        if serial == 0 {
            return;
        }
        let Some(item) = ctx.focus.peek().clone() else { return };
        let id = serde_json::to_string(&item.dom_id()).unwrap_or_default();
        document::eval(&format!(
            r#"const go = (tries) => {{
              const el = document.getElementById({id}); const sc = document.getElementById('{SCROLL_ID}');
              if (!el || !sc) {{ if (tries > 0) requestAnimationFrame(() => go(tries - 1)); return; }}
              const top = el.offsetTop, h = el.offsetHeight, pad = {HEADING_H};
              if (top - pad < sc.scrollTop) sc.scrollTop = Math.max(0, top - pad);
              else if (top + h > sc.scrollTop + sc.clientHeight) sc.scrollTop = top + h - sc.clientHeight;
              if ({move_focus}) el.focus({{ preventScroll: true }});
            }};
            go(30);"#
        ));
    });

    // Visible range plus overscan; the focus row and selected rows stay
    // mounted so focus never drops to <body> (keyboard spec 14).
    let over = OVERSCAN_ROWS * row_h;
    let st = scroll_top();
    let start = tops.partition_point(|t| *t <= st - over).saturating_sub(1);
    let end = tops.partition_point(|t| *t < st + viewport() + over).min(model.rows.len());
    let focus = ctx.focus.read().clone();
    let selected = selected.read().clone();
    let multi = selected.len() > 1;
    let mut visible: Vec<usize> = (start..end).collect();
    if let Some(r) = focus.as_ref().and_then(|f| model.row_index(f)).filter(|r| *r < start || *r >= end) {
        visible.push(r);
    }
    // The roving tab stop: the focus row if it is listed, else the first item.
    let tab_item = focus.clone().filter(|f| model.item_index(f).is_some()).or_else(|| model.items().next());
    let flat = matches!(node, Node::Favorites | Node::Suggested);

    // Sticky heading overlay: the section at the top, pushed up by the next.
    let sticky = if flat {
        None
    } else {
        let current = model.headings.iter().rposition(|(_, idx)| tops[*idx] <= st);
        current.map(|h| {
            let (label, _) = model.headings[h].clone();
            let push = model.headings.get(h + 1).map(|(_, idx)| (tops[*idx] - st - HEADING_H).min(0.0)).unwrap_or(0.0);
            (label, push)
        })
    };

    let query = {
        let f = ctx.filter.read().clone();
        super::ctx::effective_query(&f, &app.search.read())
    };
    let sort = *ctx.sort.read();
    let last_first = *ctx.last_first.read();
    let item_count = model.count;

    let on_keydown = {
        let model = model.clone();
        move |e: KeyboardEvent| {
            let items = model.item_list();
            let chord = keys::chord(&e);
            let current = ctx.focus.peek().as_ref().and_then(|f| model.item_index(f));
            let page = ((viewport() / row_h).floor() as usize).max(1);
            let last = items.len().saturating_sub(1);
            let target = match chord.as_str() {
                "ArrowDown" | "Shift+ArrowDown" => Some(current.map(|c| (c + 1).min(last)).unwrap_or(0)),
                "ArrowUp" | "Shift+ArrowUp" => Some(current.map(|c| c.saturating_sub(1)).unwrap_or(0)),
                "Home" | "Shift+Home" => Some(0),
                "End" | "Shift+End" => Some(last),
                "PageDown" | "Shift+PageDown" => Some(current.map(|c| (c + page).min(last)).unwrap_or(0)),
                "PageUp" | "Shift+PageUp" => Some(current.map(|c| c.saturating_sub(page)).unwrap_or(0)),
                _ => None,
            };
            if let Some(t) = target {
                e.prevent_default();
                e.stop_propagation();
                if items.is_empty() {
                    return;
                }
                let item = items[t].clone();
                if chord.starts_with("Shift+") {
                    let anchor = ctx.anchor.peek().clone().unwrap_or_else(|| item.clone());
                    extend_to(ctx, items, &anchor, &item);
                    ctx.focus.set(Some(item));
                    ctx.request_focus(true);
                } else if ctx.select_guarded(Some(item), true) {
                    // selected and focused
                }
                return;
            }
            match chord.as_str() {
                "Space" | "Ctrl+Space" => {
                    e.prevent_default();
                    e.stop_propagation();
                    if let Some(f) = ctx.focus.peek().clone() {
                        toggle(ctx, &f);
                    }
                }
                "Enter" => {
                    e.stop_propagation();
                    document::eval("const d = document.querySelector('#ax-people-details button:not([disabled]), #ax-people-details a'); if (d) d.focus();");
                }
                "Ctrl+A" => {
                    e.prevent_default();
                    e.stop_propagation();
                    if ctx.editing.peek().is_none() {
                        ctx.selection.set(items.to_vec());
                    }
                }
                "Escape" => {
                    e.stop_propagation();
                    let sel_len = ctx.selection.peek().len();
                    if sel_len > 1 {
                        let keep = ctx.focus.peek().clone();
                        ctx.selection.set(keep.into_iter().collect());
                    } else if !ctx.filter.peek().is_empty() {
                        ctx.filter.set(String::new());
                    } else if app.search.peek().active {
                        let serial = app.search.peek().serial + 1;
                        let mut search = app.search;
                        search.set(crate::state::SearchState { serial, ..Default::default() });
                    } else {
                        // "Discard contact / unselect all" (2.4).
                        ctx.select_guarded(None, false);
                    }
                }
                "Shift+F10" | "ContextMenu" => {
                    e.prevent_default();
                    e.stop_propagation();
                    if let Some(f) = ctx.focus.peek().clone() {
                        open_menu_at_row(ctx, &f);
                    }
                }
                _ => {
                    // Type-ahead on printable characters (2.5.3).
                    let m = e.modifiers();
                    if let Key::Character(c) = e.key()
                        && !m.ctrl()
                        && !m.alt()
                        && !m.meta()
                        && !m.shift()
                        && c.chars().count() == 1
                        && c != " "
                    {
                        let (prev, at) = typeahead.peek().clone();
                        let prefix = if at.elapsed() < Duration::from_secs(1) { format!("{prev}{c}") } else { c.clone() };
                        typeahead.set((prefix.clone(), Instant::now()));
                        let p = fold(&prefix);
                        let hit = model.rows.iter().find_map(|r| {
                            let name = match r {
                                Row::Contact(c) => display_name(c, last_first),
                                Row::List(l) => l.name.clone(),
                                Row::Suggested(a, _) => a.display().to_string(),
                                Row::Heading(_) => return None,
                            };
                            fold(&name).starts_with(&p).then(|| r.item()).flatten()
                        });
                        if let Some(item) = hit {
                            e.stop_propagation();
                            ctx.select_guarded(Some(item), true);
                        }
                    }
                }
            }
        }
    };

    let title = node.title();
    let empty = empty_state(&node, &query, item_count);

    rsx! {
        section {
            id: "ax-people-listpane",
            class: "ax-people-list ax-pane",
            class: if columns { "ax-columns" },
            style: "width:{width}px",
            role: "navigation",
            "aria-label": "{title}",
            div { class: "ax-people-list-head",
                h2 { class: "ax-people-list-title", title: "{title}", "{title}" }
                span { class: "ax-people-list-count", "aria-label": "{item_count} items", "{item_count}" }
                span { class: "ax-grow" }
                if !flat {
                    MenuButton {
                        label: sort.button_label(),
                        variant: Variant::Subtle,
                        class: "ax-people-sort",
                        items: SortBy::MENU.iter().map(|s| MenuItem::new(format!("sort:{}", s.key()), s.label()).checked(*s == sort)).collect::<Vec<_>>(),
                        on_select: move |id: String| actions::run(ctx, app, &id, &MenuTarget::Selection),
                    }
                }
            }
            SearchBox {}
            if columns && item_count > 0 {
                div { class: "ax-people-colhead", role: "presentation",
                    span { class: "ax-people-col-lead" }
                    button {
                        r#type: "button",
                        class: "ax-people-col ax-people-col-name",
                        class: if matches!(sort, SortBy::FirstName | SortBy::LastName) { "ax-sorted" },
                        title: "Sort by name",
                        onclick: move |_| ctx.set_sort(if sort == SortBy::FirstName { SortBy::LastName } else { SortBy::FirstName }),
                        "Name"
                        if matches!(sort, SortBy::FirstName | SortBy::LastName) { Icon { name: "arrow_down", size: 12 } }
                    }
                    span { class: "ax-people-col ax-people-col-email", "Email" }
                    button {
                        r#type: "button",
                        class: "ax-people-col ax-people-col-company",
                        class: if sort == SortBy::Company { "ax-sorted" },
                        title: "Sort by company",
                        onclick: move |_| ctx.set_sort(SortBy::Company),
                        "Company"
                        if sort == SortBy::Company { Icon { name: "arrow_down", size: 12 } }
                    }
                    span { class: "ax-people-col ax-people-col-phone", "Phone" }
                    span { class: "ax-people-col-star" }
                }
            }
            if let Some((title, body, action)) = empty {
                div { class: "ax-people-empty",
                    EmptyState { icon: empty_icon(&node, &query), title, subtitle: body,
                        if let Some((label, id)) = action {
                            Button {
                                label,
                                variant: Variant::Primary,
                                onclick: move |_| actions::run(ctx, app, id, &MenuTarget::Selection),
                            }
                        }
                    }
                }
            } else {
                div {
                    id: SCROLL_ID,
                    class: "ax-people-scroll",
                    role: "listbox",
                    "aria-multiselectable": "true",
                    "aria-label": "Contacts",
                    "data-reorder": if node == Node::Favorites { "1" } else { "0" },
                    onscroll: move |e| {
                        scroll_top.set(e.scroll_top());
                        let h = e.client_height() as f64;
                        if h > 0.0 { viewport.set(h) }
                    },
                    onresize: move |e| {
                        if let Ok(size) = e.get_content_box_size() && size.height > 0.0 {
                            viewport.set(size.height);
                        }
                    },
                    onmounted: move |e| async move {
                        if let Ok(r) = e.data().get_client_rect().await && r.height() > 0.0 {
                            viewport.set(r.height());
                        }
                        // Arrow keys and Space must not scroll the pane in liveview,
                        // where preventDefault from Rust arrives too late.
                        document::eval(&format!(
                            r#"const sc = document.getElementById('{SCROLL_ID}');
                            if (sc && !sc.dataset.keysReady) {{ sc.dataset.keysReady = '1';
                              sc.addEventListener('keydown', e => {{ if (e.target.closest('[role=option]') &&
                                ['ArrowUp','ArrowDown','PageUp','PageDown','Home','End',' '].includes(e.key)) e.preventDefault(); }});
                              sc.addEventListener('dragover', e => {{ if (sc.dataset.reorder === '1' && e.target.closest('[role=option]')) e.preventDefault(); }}); }}"#
                        ));
                    },
                    onkeydown: on_keydown,
                    if let Some((label, push)) = sticky {
                        div {
                            class: "ax-people-sticky",
                            style: "transform:translateY({push}px)",
                            "aria-hidden": "true",
                            button { r#type: "button", class: "ax-people-heading-btn", tabindex: "-1", onclick: move |_| jump_open.set(true), "{label}" }
                        }
                    }
                    div { class: "ax-people-virtual", style: "height:{total}px",
                        for i in visible.iter().copied() {
                            match &model.rows[i] {
                                Row::Heading(h) => rsx! {
                                    div {
                                        key: "h-{h}",
                                        class: "ax-people-heading",
                                        role: "presentation",
                                        style: "top:{tops[i]}px;height:{HEADING_H}px",
                                        button { r#type: "button", class: "ax-people-heading-btn", tabindex: "-1", title: "Jump to a letter", onclick: move |_| jump_open.set(true), "{h}" }
                                    }
                                },
                                row => {
                                    let item = row.item().unwrap_or(Item::Contact(0));
                                    rsx! {
                                        ListRow {
                                            key: "{item.dom_id()}",
                                            row: row.clone(),
                                            top: tops[i],
                                            height: row_h,
                                            selected: selected.contains(&item),
                                            tab_stop: tab_item.as_ref() == Some(&item),
                                            multi,
                                            columns,
                                            compact: density == Density::Compact,
                                            last_first,
                                            pos: model.item_index(&item).map(|i| i + 1).unwrap_or(0),
                                            setsize: item_count,
                                            reorder: node == Node::Favorites,
                                            drop_before: fav_drop.read().as_ref() == Some(&item),
                                            fav_drop,
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if jump_open() && !flat {
                JumpGrid { headings: model.headings.iter().map(|(h, idx)| (h.clone(), tops[*idx])).collect::<Vec<_>>(), on_close: move |_| jump_open.set(false) }
            }
        }
    }
}

/// New Favorites order after dropping `dragged` before `target`, as
/// (is_list, id) top to bottom; empty when nothing moves.
fn reorder_favorites(model: &ListModel, dragged: &[Item], target: &Item) -> Vec<(bool, i64)> {
    if dragged.is_empty() || dragged.contains(target) {
        return Vec::new();
    }
    let mut items: Vec<Item> = model.items().filter(|i| !dragged.contains(i)).collect();
    let at = items.iter().position(|i| i == target).unwrap_or(items.len());
    let moved: Vec<Item> = model.items().filter(|i| dragged.contains(i)).collect();
    for (k, d) in moved.into_iter().enumerate() {
        items.insert(at + k, d);
    }
    items
        .into_iter()
        .filter_map(|i| match i {
            Item::Contact(id) => Some((false, id)),
            Item::List(id) => Some((true, id)),
            Item::Suggested(_) => None,
        })
        .collect()
}

/// Shift-range from the anchor to `to`.
fn extend_to(mut ctx: PeopleCtx, items: &[Item], anchor: &Item, to: &Item) {
    let a = items.iter().position(|i| i == anchor).unwrap_or(0);
    let b = items.iter().position(|i| i == to).unwrap_or(a);
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    ctx.editing.set(None);
    ctx.selection.set(items[lo..=hi].to_vec());
}

fn toggle(mut ctx: PeopleCtx, item: &Item) {
    let mut sel = ctx.selection.peek().clone();
    if let Some(p) = sel.iter().position(|i| i == item) {
        sel.remove(p);
    } else {
        sel.push(item.clone());
    }
    ctx.editing.set(None);
    ctx.selection.set(sel);
    ctx.anchor.set(Some(item.clone()));
    ctx.focus.set(Some(item.clone()));
}

/// Open the context menu for the selection at a row (keyboard).
fn open_menu_at_row(ctx: PeopleCtx, item: &Item) {
    let id = serde_json::to_string(&item.dom_id()).unwrap_or_default();
    spawn(async move {
        let mut ctx = ctx;
        let pos = document::eval(&format!(
            "const el = document.getElementById({id}); if (!el) return [80, 200]; const r = el.getBoundingClientRect(); return [r.left + 48, r.top + r.height / 2];"
        ))
        .join::<(f64, f64)>()
        .await
        .unwrap_or((80.0, 200.0));
        let items = actions::selection_menu(&ctx);
        ctx.menu.set(Some(ContextMenu { x: pos.0, y: pos.1, items, target: MenuTarget::Selection }));
    });
}

/// Empty-state copy: title, body and an optional (button label, command id).
type EmptyCopy = (String, String, Option<(&'static str, &'static str)>);

fn empty_state(node: &Node, query: &str, count: usize) -> Option<EmptyCopy> {
    if count > 0 {
        return None;
    }
    if !query.trim().is_empty() {
        return Some((format!("No results for \u{201c}{}\u{201d}", query.trim()), "Check the spelling or try a different name.".into(), None));
    }
    Some(match node {
        Node::AllContacts => ("No contacts yet".into(), "People you add will show up here.".into(), Some(("New contact", "new_contact"))),
        Node::Favorites => ("No favorites yet".into(), "Select a contact and choose Add to favorites to keep them close.".into(), None),
        Node::AllLists => ("No contact lists".into(), "Group people into a list to email them all at once.".into(), Some(("New contact list", "new_list"))),
        Node::Folder(_) => ("This folder is empty".into(), String::new(), Some(("New contact", "new_contact"))),
        Node::Category(_) => ("No contacts in this category".into(), String::new(), None),
        Node::Suggested => (
            "No suggestions right now".into(),
            "People you email who aren't saved as contacts will show up here.".into(),
            None,
        ),
    })
}

fn empty_icon(node: &Node, query: &str) -> &'static str {
    if !query.trim().is_empty() {
        return "search";
    }
    match node {
        Node::Favorites => "star",
        Node::AllLists => "people_team",
        Node::Folder(_) => "folder",
        Node::Category(_) => "tag",
        Node::Suggested => "person_lightbulb",
        Node::AllContacts => "people",
    }
}

/// The list pane's search box: live filter, debounced 150 ms (2.5.1).
#[component]
fn SearchBox() -> Element {
    let mut ctx = use_context::<PeopleCtx>();
    let mut text = use_signal(|| ctx.filter.peek().clone());
    // Follow outside resets (Escape in the list, node changes).
    use_effect(move || {
        let f = (ctx.filter)();
        if f.is_empty() {
            text.set(String::new());
        }
    });
    rsx! {
        div { class: "ax-people-search", role: "search",
            Icon { name: "search", size: 16 }
            input {
                id: SEARCH_ID,
                r#type: "text",
                placeholder: "Search contacts",
                "aria-label": "Search contacts",
                "aria-keyshortcuts": "Control+E",
                value: "{text}",
                oninput: move |e| {
                    let v = e.value();
                    text.set(v.clone());
                    spawn(async move {
                        tokio::time::sleep(Duration::from_millis(150)).await;
                        if *text.peek() == v {
                            ctx.filter.set(v);
                        }
                    });
                },
                onkeydown: move |e: KeyboardEvent| {
                    keys::stop_typing_keys(&e);
                    match keys::chord(&e).as_str() {
                        "Ctrl+A" => e.stop_propagation(),
                        "ArrowDown" | "Enter" => {
                            ctx.filter.set(text.peek().clone());
                            let first = ctx.model.peek().items().next();
                            if let Some(item) = first {
                                ctx.select_guarded(Some(item), true);
                            }
                        }
                        "Escape" => {
                            e.stop_propagation();
                            if text.peek().is_empty() {
                                ctx.request_focus(true);
                            } else {
                                text.set(String::new());
                                ctx.filter.set(String::new());
                            }
                        }
                        _ => {}
                    }
                },
            }
            if !text().is_empty() {
                button {
                    r#type: "button",
                    class: "ax-people-search-clear",
                    title: "Clear search",
                    "aria-label": "Clear search",
                    onclick: move |_| {
                        text.set(String::new());
                        ctx.filter.set(String::new());
                        super::io::focus_id(SEARCH_ID);
                    },
                    Icon { name: "dismiss", size: 16 }
                }
            }
        }
    }
}

/// Flyout of the headings present; picking one scrolls there (2.5.3).
#[component]
fn JumpGrid(headings: Vec<(String, f64)>, on_close: EventHandler<()>) -> Element {
    rsx! {
        div { class: "ax-menu-backdrop", onclick: move |_| on_close.call(()) }
        div {
            class: "ax-people-jump",
            role: "dialog",
            "aria-label": "Jump to",
            onmounted: move |e| async move { let _ = e.set_focus(true).await; },
            tabindex: "-1",
            onkeydown: move |e: KeyboardEvent| if e.key() == Key::Escape { e.stop_propagation(); on_close.call(()) },
            for (h, top) in headings {
                button {
                    key: "{h}",
                    r#type: "button",
                    class: "ax-people-jump-btn",
                    onclick: move |_| {
                        document::eval(&format!("const sc = document.getElementById('{SCROLL_ID}'); if (sc) sc.scrollTop = {top};"));
                        on_close.call(());
                    },
                    "{h}"
                }
            }
        }
    }
}

#[component]
fn ListRow(
    row: Row,
    top: f64,
    height: f64,
    selected: bool,
    tab_stop: bool,
    multi: bool,
    columns: bool,
    compact: bool,
    last_first: bool,
    pos: usize,
    setsize: usize,
    /// Favorites view: rows accept drops to reorder.
    reorder: bool,
    drop_before: bool,
    fav_drop: Signal<Option<Item>>,
) -> Element {
    let app = use_app();
    let mut ctx = use_context::<PeopleCtx>();
    let mut fav_drop = fav_drop;
    let Some(item) = row.item() else { return rsx! {} };
    let is_focus = ctx.focus.read().as_ref() == Some(&item);
    let avatar_size = if columns { 24 } else { 32 };

    let (name, line2, line3, phone, avatar, favorite) = match &row {
        Row::Contact(c) => (
            display_name(c, last_first),
            c.primary_email().unwrap_or_default().to_string(),
            c.company.clone(),
            main_phone(c).unwrap_or_default().to_string(),
            rsx! { PeopleAvatar { name: c.display_name.clone(), color_key: contact_color_key(c), size: avatar_size, photo: c.photo.clone() } },
            Some(c.is_favorite),
        ),
        Row::List(l) => (
            l.name.clone(),
            format!("{} member{}", l.members.len(), if l.members.len() == 1 { "" } else { "s" }),
            if columns { String::new() } else { l.description.clone() },
            String::new(),
            rsx! { PeopleAvatar { name: l.name.clone(), color_key: list_color_key(l), size: avatar_size, square: true, fallback_icon: "people" } },
            Some(l.is_favorite),
        ),
        Row::Suggested(a, n) => (
            a.display().to_string(),
            a.address.clone(),
            format!("{n} email{}", if *n == 1 { "" } else { "s" }),
            String::new(),
            rsx! { PeopleAvatar { name: a.display().to_string(), color_key: a.address.to_lowercase(), size: avatar_size } },
            None,
        ),
        Row::Heading(_) => return rsx! {},
    };
    // Accessible name: never rely on the visual-only favourite star.
    let aria_label = [name.as_str(), line2.as_str(), line3.as_str(), if favorite == Some(true) { "Favorite" } else { "" }]
        .iter()
        .filter(|s| !s.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(", ");
    let suggested_addr = match &row {
        Row::Suggested(a, _) => Some(a.address.clone()),
        _ => None,
    };
    let item_click = item.clone();
    let item_menu = item.clone();
    let item_check = item.clone();
    let item_drag = item.clone();
    let item_dbl = item.clone();
    let item_fav = item.clone();
    let item_over = item.clone();
    let item_drop = item.clone();

    rsx! {
        div {
            id: item.dom_id(),
            class: "ax-people-row",
            class: if selected { "ax-selected" },
            class: if multi { "ax-multi" },
            class: if is_focus { "ax-focus-row" },
            class: if compact { "ax-compact" },
            class: if drop_before { "ax-drop-before" },
            role: "option",
            "aria-selected": if selected { "true" } else { "false" },
            "aria-label": "{aria_label}",
            "aria-setsize": "{setsize}",
            "aria-posinset": "{pos}",
            tabindex: if tab_stop { "0" } else { "-1" },
            // F6 into the list lands on its tab stop (dom.js `enter`).
            "data-ax-default-focus": if tab_stop { "true" },
            style: "top:{top}px;height:{height}px",
            draggable: if suggested_addr.is_none() { "true" } else { "false" },
            onclick: move |e: MouseEvent| {
                let m = e.modifiers();
                if m.ctrl() || m.meta() {
                    toggle(ctx, &item_click);
                } else if m.shift() {
                    let items: Vec<Item> = ctx.model.peek().items().collect();
                    let anchor = ctx.anchor.peek().clone().unwrap_or_else(|| item_click.clone());
                    extend_to(ctx, &items, &anchor, &item_click);
                    ctx.focus.set(Some(item_click.clone()));
                } else {
                    ctx.select_guarded(Some(item_click.clone()), false);
                }
            },
            ondoubleclick: move |_| {
                if matches!(item_dbl, Item::Contact(_) | Item::List(_)) {
                    actions::edit(ctx);
                }
            },
            oncontextmenu: move |e: MouseEvent| {
                e.prevent_default();
                if !ctx.selection.peek().contains(&item_menu) && !ctx.select_guarded(Some(item_menu.clone()), false) {
                    return;
                }
                let c = e.client_coordinates();
                let items = actions::selection_menu(&ctx);
                ctx.menu.set(Some(ContextMenu { x: c.x, y: c.y, items, target: MenuTarget::Selection }));
            },
            ondragstart: move |_| {
                let sel = ctx.selection.peek().clone();
                let dragged = if sel.contains(&item_drag) { sel } else { vec![item_drag.clone()] };
                ctx.drag.set(dragged);
            },
            ondragover: move |_| {
                if reorder && fav_drop.peek().as_ref() != Some(&item_over) {
                    fav_drop.set(Some(item_over.clone()));
                }
            },
            ondragend: move |_| {
                fav_drop.set(None);
                ctx.drag.set(Vec::new());
            },
            ondrop: move |e: DragEvent| {
                fav_drop.set(None);
                if !reorder {
                    return;
                }
                e.prevent_default();
                let dragged = ctx.drag.peek().clone();
                ctx.drag.set(Vec::new());
                let order = reorder_favorites(&ctx.model.peek(), &dragged, &item_drop);
                if !order.is_empty() && app.check(crate::state::store().reorder_favorites(&order)).is_some() {
                    app.bump_people();
                }
            },
            div { class: "ax-people-row-lead",
                span { class: "ax-people-row-avatar", {avatar} }
                span {
                    class: "ax-people-check",
                    class: if selected { "ax-checked" },
                    role: "checkbox",
                    "aria-checked": if selected { "true" } else { "false" },
                    "aria-label": "Select {name}",
                    onclick: move |e| {
                        e.stop_propagation();
                        toggle(ctx, &item_check);
                    },
                    if selected { Icon { name: "checkmark", size: 12 } }
                }
            }
            if columns {
                span { class: "ax-people-col ax-people-col-name ax-ellipsis", "{name}" }
                span { class: "ax-people-col ax-people-col-email ax-ellipsis", "{line2}" }
                span { class: "ax-people-col ax-people-col-company ax-ellipsis", "{line3}" }
                span { class: "ax-people-col ax-people-col-phone ax-ellipsis", "{phone}" }
            } else {
                div { class: "ax-people-row-text",
                    span { class: "ax-people-row-name ax-ellipsis", "{name}" }
                    if !line2.is_empty() { span { class: "ax-people-row-sub ax-ellipsis", "{line2}" } }
                    if !compact && !line3.is_empty() { span { class: "ax-people-row-sub ax-ellipsis", "{line3}" } }
                }
            }
            if let Some(addr) = suggested_addr.clone() {
                SuggestedActions { address: addr }
            } else if let Some(fav) = favorite {
                button {
                    r#type: "button",
                    class: "ax-people-star",
                    class: if fav { "ax-on" },
                    tabindex: "-1",
                    "aria-hidden": "true",
                    title: if fav { "Remove from favorites" } else { "Add to favorites" },
                    onclick: move |e| {
                        e.stop_propagation();
                        match &item_fav {
                            Item::Contact(id) => actions::set_favorite(app, &[*id], &[], !fav),
                            Item::List(id) => actions::set_favorite(app, &[], &[*id], !fav),
                            Item::Suggested(_) => {}
                        }
                    },
                    Icon { name: if fav { "star_filled" } else { "star" }, size: 16 }
                }
            }
        }
    }
}

/// Inline "Add to contacts" and Dismiss on a Suggested row (2.12).
#[component]
fn SuggestedActions(address: String) -> Element {
    let app: AppState = use_app();
    let ctx = use_context::<PeopleCtx>();
    let add = address.clone();
    rsx! {
        div { class: "ax-people-suggested-actions",
            Button {
                label: "Add to contacts",
                small: true,
                onclick: move |e: MouseEvent| {
                    e.stop_propagation();
                    actions::add_suggested(ctx, &add);
                },
            }
            button {
                r#type: "button",
                class: "ax-icon-btn ax-small",
                title: "Dismiss",
                "aria-label": "Dismiss",
                onclick: move |e| {
                    e.stop_propagation();
                    if app.check(crate::state::store().dismiss_suggested_person(&address)).is_some() {
                        app.bump_people();
                    }
                },
                Icon { name: "dismiss", size: 16 }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    #[test]
    fn tops_account_for_headings() {
        let c = |id| Row::Contact(Rc::new(Contact { id, ..Default::default() }));
        let model = ListModel::new(
            vec![Row::Heading("A".into()), c(1), c(2), Row::Heading("B".into()), c(3)],
            vec![("A".into(), 0), ("B".into(), 3)],
        );
        assert_eq!(model.count, 3);
        assert_eq!(row_tops(&model, 64.0), vec![0.0, 28.0, 92.0, 156.0, 184.0, 248.0]);
    }

    #[test]
    fn favorites_drop_reorders() {
        let c = |id| Row::Contact(Rc::new(Contact { id, ..Default::default() }));
        let model = ListModel::new(vec![c(1), Row::List(Rc::new(ContactList { id: 7, ..Default::default() })), c(2), c(3)], Vec::new());
        // Drag contact 3 above the list.
        assert_eq!(reorder_favorites(&model, &[Item::Contact(3)], &Item::List(7)), vec![(false, 1), (false, 3), (true, 7), (false, 2)]);
        // Dropping on itself does nothing.
        assert!(reorder_favorites(&model, &[Item::Contact(2)], &Item::Contact(2)).is_empty());
        // Several items keep their relative order.
        assert_eq!(
            reorder_favorites(&model, &[Item::Contact(3), Item::Contact(1)], &Item::Contact(2)),
            vec![(true, 7), (false, 1), (false, 3), (false, 2)]
        );
    }
}
