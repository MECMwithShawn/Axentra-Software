//! Navigation pane: Favorites, "Your contacts" (All contacts, All contact
//! lists, Contacts, folders, categories) and Suggested
//! (docs/spec/people-todo.md 2.3).

use axentra_core::*;
use dioxus::prelude::*;

use super::ctx::{ContextMenu, MenuTarget, PeopleCtx};
use super::io;
use super::view::{Item, Node};
use crate::kit::{Icon, IconButton, MenuItem, keys};
use crate::state::{AppState, store, use_app};

#[derive(Clone, Debug, PartialEq)]
enum EntryKind {
    Node(Node),
    /// The "Your contacts" section header.
    YourContacts,
}

#[derive(Clone, Debug, PartialEq)]
struct Entry {
    kind: EntryKind,
    label: String,
    icon: &'static str,
    icon_selected: &'static str,
    level: u32,
    /// Category colour dot instead of an icon.
    swatch: Option<String>,
    /// Accepts dropped contacts.
    drop: bool,
}

impl Entry {
    fn node(n: Node, label: &str, icon: &'static str, icon_selected: &'static str, level: u32, drop: bool) -> Self {
        Self { kind: EntryKind::Node(n), label: label.to_string(), icon, icon_selected, level, swatch: None, drop }
    }

    fn target_node(&self) -> Node {
        match &self.kind {
            EntryKind::Node(n) => n.clone(),
            EntryKind::YourContacts => Node::AllContacts,
        }
    }
}

fn entries(ctx: &PeopleCtx) -> Vec<Entry> {
    let mut out = vec![Entry::node(Node::Favorites, "Favorites", "star", "star_filled", 1, true)];
    out.push(Entry {
        kind: EntryKind::YourContacts,
        label: "Your contacts".into(),
        icon: "people_list",
        icon_selected: "people_list_filled",
        level: 1,
        swatch: None,
        drop: false,
    });
    if *ctx.contacts_expanded.read() {
        out.push(Entry::node(Node::AllContacts, "All contacts", "person", "person_filled", 2, false));
        out.push(Entry::node(Node::AllLists, "All contact lists", "people_team", "people_team_filled", 2, false));
        out.push(Entry::node(Node::Folder(String::new()), "Contacts", "folder", "folder_filled", 2, true));
        for f in store().contact_folders().unwrap_or_default() {
            out.push(Entry::node(Node::Folder(f.clone()), &f, "folder", "folder_filled", 2, true));
        }
        let categories = store().categories().unwrap_or_default();
        for cat in store().contact_categories_in_use().unwrap_or_default() {
            let color = categories.iter().find(|c| c.name == cat).map(|c| crate::kit::category_colors(&c.color).0).unwrap_or("#7a7574");
            out.push(Entry { swatch: Some(color.to_string()), ..Entry::node(Node::Category(cat.clone()), &cat, "tag", "tag_filled", 2, false) });
        }
    }
    if *ctx.suggested_on.read() {
        out.push(Entry::node(Node::Suggested, "Suggested", "person_lightbulb", "person_lightbulb", 1, false));
    }
    out
}

fn dom_id(i: usize) -> String {
    format!("ax-people-nav-{i}")
}

#[component]
pub fn NavPane() -> Element {
    let app = use_app();
    let _ = app.people_rev();
    let ctx = use_context::<PeopleCtx>();
    let list = entries(&ctx);
    let focus = use_signal(|| 0usize);
    let drop_hover = use_signal(|| None::<usize>);
    let focus_idx = focus().min(list.len().saturating_sub(1));
    let width = *ctx.nav_w.read();
    let len = list.len();
    // The inline "New folder" box goes after the last folder.
    let split = list.iter().rposition(|e| matches!(e.kind, EntryKind::Node(Node::Folder(_)))).map(|i| i + 1).unwrap_or(len);
    let parent_of = |i: usize| list[..i].iter().rposition(|x| x.kind == EntryKind::YourContacts).filter(|_| list[i].level > 1);

    // Let contacts be dropped on folders: dragover must be cancelled
    // synchronously, which liveview cannot do from Rust.
    let install_drop = move |_| {
        document::eval(
            r#"const nav = document.getElementById('ax-people-nav');
            if (nav && !nav.dataset.dropReady) { nav.dataset.dropReady = '1';
              nav.addEventListener('dragover', e => { if (e.target.closest('[data-drop="1"]')) e.preventDefault(); }); }"#,
        );
    };

    rsx! {
        nav {
            id: "ax-people-nav",
            class: "ax-people-nav ax-pane",
            style: "width:{width}px",
            "aria-label": "Hub navigation menu",
            onmounted: install_drop,
            div { class: "ax-people-nav-scroll", role: "tree", "aria-label": "People",
                for (i, e) in list[..split].iter().cloned().enumerate() {
                    NavItem { key: "{i}-{e.label}", entry: e, index: i, len, focused: i == focus_idx, parent: parent_of(i), focus, drop_hover }
                }
                if *ctx.new_folder.read() {
                    div { class: "ax-people-nav-item ax-people-nav-new", style: "padding-left:28px",
                        span { class: "ax-people-nav-chevron-space" }
                        Icon { name: "folder" }
                        FolderNameInput { initial: "New folder", old: None }
                    }
                }
                for (j, e) in list[split..].iter().cloned().enumerate() {
                    NavItem {
                        key: "{split + j}-{e.label}",
                        entry: e,
                        index: split + j,
                        len,
                        focused: split + j == focus_idx,
                        parent: parent_of(split + j),
                        focus,
                        drop_hover,
                    }
                }
            }
            if !*ctx.suggested_on.read() && !*ctx.tip_dismissed.read() {
                div { class: "ax-people-tip", role: "note",
                    Icon { name: "person_lightbulb", class: "ax-people-tip-icon" }
                    div { class: "ax-people-tip-body",
                        span { "Turn on Suggested contacts to see people you email who aren't saved yet." }
                        button {
                            r#type: "button",
                            class: "ax-people-link",
                            onclick: move |_| {
                                ctx.set_suggested(true);
                                ctx.set_node(Node::Suggested);
                            },
                            "Turn on"
                        }
                    }
                    IconButton { icon: "dismiss", title: "Dismiss", small: true, onclick: move |_| ctx.dismiss_tip() }
                }
            }
        }
    }
}

#[component]
fn NavItem(entry: Entry, index: usize, len: usize, focused: bool, parent: Option<usize>, focus: Signal<usize>, drop_hover: Signal<Option<usize>>) -> Element {
    let app = use_app();
    let mut ctx = use_context::<PeopleCtx>();
    let mut focus = focus;
    let mut drop_hover = drop_hover;
    let i = index;
    let selected = matches!(&entry.kind, EntryKind::Node(n) if *n == *ctx.node.read());
    let is_header = entry.kind == EntryKind::YourContacts;
    let expanded = *ctx.contacts_expanded.read();
    let folder_name = match &entry.kind {
        EntryKind::Node(Node::Folder(f)) if !f.is_empty() => Some(f.clone()),
        _ => None,
    };
    let renaming = folder_name.is_some() && ctx.renaming.read().as_ref() == folder_name.as_ref();
    let mut move_focus = move |to: usize| {
        focus.set(to);
        io::focus_id(&dom_id(to));
    };
    let target = entry.target_node();
    let target_key = target.clone();
    let kind_menu = entry.kind.clone();
    let kind_keys = entry.kind.clone();
    let kind_drop = entry.kind.clone();
    let accepts_drop = entry.drop;
    let indent = 12 + (entry.level - 1) * 16;
    rsx! {
        div {
            id: dom_id(i),
            class: "ax-people-nav-item",
            class: if selected { "ax-selected" },
            class: if drop_hover() == Some(i) { "ax-drop-target" },
            style: "padding-left:{indent}px",
            role: "treeitem",
            "aria-level": "{entry.level}",
            "aria-selected": if selected { "true" } else { "false" },
            "aria-expanded": if is_header { if expanded { "true" } else { "false" } },
            "data-drop": if accepts_drop { "1" } else { "0" },
            tabindex: if focused { "0" } else { "-1" },
            title: "{entry.label}",
            onclick: move |_| {
                focus.set(i);
                if ctx.select_guarded(None, false) {
                    ctx.set_node(target.clone());
                }
            },
            oncontextmenu: move |ev: MouseEvent| {
                ev.prevent_default();
                let c = ev.client_coordinates();
                let menu = match &kind_menu {
                    EntryKind::YourContacts => Some((vec![MenuItem::new("new_folder", "New folder").icon("folder_add")], MenuTarget::YourContacts)),
                    EntryKind::Node(Node::Folder(f)) if !f.is_empty() => Some((
                        vec![
                            MenuItem::new("new_contact_here", "New contact").icon("person_add"),
                            MenuItem::divider(),
                            MenuItem::new("rename_folder", "Rename folder").icon("edit"),
                            MenuItem::new("delete_folder", "Delete folder").icon("delete").danger(),
                        ],
                        MenuTarget::Folder(f.clone()),
                    )),
                    EntryKind::Node(Node::Category(cat)) => Some((
                        vec![MenuItem::new("remove_category", "Remove category from all contacts").icon("dismiss")],
                        MenuTarget::Category(cat.clone()),
                    )),
                    _ => None,
                };
                if let Some((items, target)) = menu {
                    ctx.menu.set(Some(ContextMenu { x: c.x, y: c.y, items, target }));
                }
            },
            onkeydown: move |ev: KeyboardEvent| {
                match keys::chord(&ev).as_str() {
                    "ArrowDown" => {
                        ev.prevent_default();
                        ev.stop_propagation();
                        move_focus((i + 1).min(len - 1));
                    }
                    "ArrowUp" => {
                        ev.prevent_default();
                        ev.stop_propagation();
                        move_focus(i.saturating_sub(1));
                    }
                    "Home" => {
                        ev.prevent_default();
                        ev.stop_propagation();
                        move_focus(0);
                    }
                    "End" => {
                        ev.prevent_default();
                        ev.stop_propagation();
                        move_focus(len - 1);
                    }
                    "ArrowRight" if is_header => {
                        ev.stop_propagation();
                        if expanded { move_focus(i + 1) } else { ctx.contacts_expanded.set(true) }
                    }
                    "ArrowLeft" => {
                        ev.stop_propagation();
                        if is_header && expanded {
                            ctx.contacts_expanded.set(false);
                        } else if let Some(p) = parent {
                            move_focus(p);
                        }
                    }
                    "Enter" | "Space" => {
                        ev.prevent_default();
                        ev.stop_propagation();
                        if ctx.select_guarded(None, false) {
                            ctx.set_node(target_key.clone());
                        }
                    }
                    "F2" => {
                        if let EntryKind::Node(Node::Folder(f)) = &kind_keys && !f.is_empty() {
                            ctx.renaming.set(Some(f.clone()));
                        }
                    }
                    "Escape" => {
                        // Back to the contact list (keyboard spec 15).
                        ev.stop_propagation();
                        ctx.request_focus(true);
                    }
                    _ => {}
                }
            },
            ondragenter: move |_| if accepts_drop { drop_hover.set(Some(i)) },
            ondragleave: move |_| if drop_hover() == Some(i) { drop_hover.set(None) },
            ondrop: move |ev: DragEvent| {
                ev.prevent_default();
                drop_hover.set(None);
                drop_items(ctx, app, &kind_drop);
            },
            if is_header {
                button {
                    r#type: "button",
                    class: "ax-people-nav-chevron",
                    class: if expanded { "ax-expanded" },
                    tabindex: "-1",
                    "aria-hidden": "true",
                    onclick: move |ev| {
                        ev.stop_propagation();
                        ctx.contacts_expanded.set(!expanded);
                    },
                    Icon { name: "chevron_right", size: 12 }
                }
            } else if entry.level > 1 {
                span { class: "ax-people-nav-chevron-space" }
            }
            if let Some(color) = &entry.swatch {
                span { class: "ax-people-nav-dot", style: "background:{color}" }
            } else {
                Icon { name: if selected { entry.icon_selected } else { entry.icon } }
            }
            if renaming {
                FolderNameInput { initial: folder_name.clone().unwrap_or_default(), old: folder_name.clone() }
            } else {
                span { class: "ax-people-nav-label", "{entry.label}" }
            }
        }
    }
}

/// Contacts dropped on a navigation node: move to a folder, or add to Favorites.
fn drop_items(ctx: PeopleCtx, app: AppState, kind: &EntryKind) {
    let mut ctx = ctx;
    let dragged: Vec<Item> = ctx.drag.peek().clone();
    ctx.drag.set(Vec::new());
    let contacts: Vec<ContactId> = dragged.iter().filter_map(|i| if let Item::Contact(id) = i { Some(*id) } else { None }).collect();
    let lists: Vec<ContactListId> = dragged.iter().filter_map(|i| if let Item::List(id) = i { Some(*id) } else { None }).collect();
    match kind {
        EntryKind::Node(Node::Favorites) if !contacts.is_empty() || !lists.is_empty() => {
            super::actions::set_favorite(app, &contacts, &lists, true);
        }
        EntryKind::Node(Node::Folder(f)) => super::actions::move_to_folder(app, &contacts, f),
        _ => {}
    }
}

/// Inline text box for "New folder" and "Rename folder": Enter commits, Esc
/// cancels, an empty name cancels (2.3.3).
#[component]
fn FolderNameInput(#[props(into)] initial: String, old: Option<String>) -> Element {
    let app = use_app();
    let mut ctx = use_context::<PeopleCtx>();
    let mut value = use_signal(|| initial.clone());
    let mut done = use_signal(|| false);
    let mut commit = move |save: bool| {
        if done() {
            return;
        }
        done.set(true);
        let name = value().trim().to_string();
        match (&old, save && !name.is_empty()) {
            (None, true) => {
                if let Some(created) = app.check(store().create_contact_folder(&name)) {
                    app.bump_people();
                    ctx.set_node(Node::Folder(created));
                }
            }
            (Some(old), true) => {
                if let Some(renamed) = app.check(store().rename_contact_folder(old, &name)) {
                    app.bump_people();
                    if *ctx.node.peek() == Node::Folder(old.clone()) {
                        ctx.set_node(Node::Folder(renamed));
                    }
                }
            }
            _ => {}
        }
        ctx.new_folder.set(false);
        ctx.renaming.set(None);
    };
    rsx! {
        input {
            id: "ax-people-folder-input",
            class: "ax-input ax-people-nav-input",
            value: "{value}",
            "aria-label": "Folder name",
            onmounted: move |_| {
                document::eval("const el = document.getElementById('ax-people-folder-input'); if (el) { el.focus(); el.select(); }");
            },
            onclick: move |e| e.stop_propagation(),
            oninput: move |e| value.set(e.value()),
            onkeydown: {
                let mut commit = commit.clone();
                move |e: KeyboardEvent| {
                keys::stop_typing_keys(&e);
                match e.key() {
                    Key::Enter => {
                        e.prevent_default();
                        commit(true)
                    }
                    Key::Escape => {
                        e.stop_propagation();
                        commit(false)
                    }
                    _ => {}
                }
            }},
            onblur: move |_| commit(true),
        }
    }
}
