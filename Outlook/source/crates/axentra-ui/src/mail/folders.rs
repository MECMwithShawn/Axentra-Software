//! Folder pane: Favorites and one tree per account (docs/spec/mail.md §2).
//!
//! The tree is rendered flat (each node carries its `aria-level`) so the
//! keyboard model (§4.1 of the keyboard spec) can walk the visible nodes in
//! order; the tree container keeps DOM focus and points at the focused node
//! with `aria-activedescendant`.

use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use axentra_core::*;
use dioxus::prelude::*;

use super::actions;
use super::ctx::{ContextMenu, Drag, FolderEdit, MailCtx, MailDialog, MenuTarget, PersonView, use_mail, use_model};
use super::menus;
use crate::kit::{Avatar, Icon, keys};
use crate::state::{AppState, SettingsPage, store, use_app};

/// Display order of the well-known folders (one place, per §2.5).
const ROLE_ORDER: [FolderRole; 9] = [
    FolderRole::Inbox,
    FolderRole::Drafts,
    FolderRole::Sent,
    FolderRole::Custom, // the Snoozed folder slots in here
    FolderRole::Deleted,
    FolderRole::Junk,
    FolderRole::Archive,
    FolderRole::Notes,
    FolderRole::ConversationHistory,
];

/// A favourite contact shown under Favorites; selecting it lists their mail.
#[derive(Clone, Debug, PartialEq)]
pub struct FavoritePerson {
    pub contact: ContactId,
    pub name: String,
    pub address: String,
    /// Unread mail from them in the Inbox.
    pub unread: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum NodeKind {
    FavoritesHeader,
    Favorite(Folder),
    Person(FavoritePerson),
    Account(Account),
    Folder(Folder),
    /// Inline field for a new folder.
    NewFolder { account: AccountId, parent: Option<FolderId> },
    /// "Create new folder" entry under an account's tree.
    CreateEntry(AccountId),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub dom_id: String,
    pub kind: NodeKind,
    pub level: u32,
    pub expandable: bool,
    pub expanded: bool,
    pub parent: Option<String>,
    /// Caption after the name: the account of a favourite whose name
    /// another favourite shares (two Inboxes, §2.3).
    pub suffix: Option<String>,
}

/// Count text per §2.6: unread for most folders, total for Drafts / Outbox /
/// Snoozed, nothing for Sent Items, Notes and Conversation History.
pub fn folder_count(f: &Folder, snoozed: bool) -> Option<(String, bool)> {
    let fmt = |n: i64| if n > 9999 { "9999+".to_string() } else { n.to_string() };
    if snoozed {
        return (f.total_count > 0).then(|| (fmt(f.total_count), false));
    }
    match f.role {
        FolderRole::Drafts | FolderRole::Outbox => (f.total_count > 0).then(|| (fmt(f.total_count), false)),
        FolderRole::Sent | FolderRole::Notes | FolderRole::ConversationHistory => None,
        _ => (f.unread_count > 0).then(|| (fmt(f.unread_count), true)),
    }
}

/// Folder icon per §2.5 (Outbox and Conversation History differ from the
/// model's generic icons).
fn folder_icon(f: &Folder, snoozed: bool) -> &'static str {
    match f.role {
        _ if snoozed => "snooze",
        FolderRole::Outbox => "mail_arrow_up",
        FolderRole::ConversationHistory => "history",
        role => role.icon(),
    }
}

/// Sort siblings: well-known folders in [`ROLE_ORDER`], then custom A to Z.
fn sort_siblings(folders: &mut [Folder], snoozed: &[FolderId]) {
    let rank = |f: &Folder| -> usize {
        if snoozed.contains(&f.id) {
            return 3;
        }
        if f.role == FolderRole::Custom {
            return ROLE_ORDER.len() + 1;
        }
        ROLE_ORDER.iter().position(|r| *r == f.role).unwrap_or(ROLE_ORDER.len())
    };
    folders.sort_by(|a, b| rank(a).cmp(&rank(b)).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
}

/// The visible nodes of the pane, in order.
pub fn build_nodes(
    accounts: &[Account],
    folders: &[Folder],
    favorites: &[Folder],
    people: &[FavoritePerson],
    snoozed: &[FolderId],
    prefs: &super::ctx::FolderPrefs,
    edit: Option<&FolderEdit>,
) -> Vec<Node> {
    let mut nodes = Vec::new();
    if !prefs.favorites_hidden {
        nodes.push(Node {
            dom_id: "ax-mail-node-fav".into(),
            kind: NodeKind::FavoritesHeader,
            level: 1,
            expandable: true,
            expanded: !prefs.favorites_collapsed,
            parent: None,
            suffix: None,
        });
        if !prefs.favorites_collapsed {
            for f in favorites {
                let shared_name = favorites.iter().any(|o| o.id != f.id && o.account_id != f.account_id && o.name.eq_ignore_ascii_case(&f.name));
                nodes.push(Node {
                    dom_id: format!("ax-mail-node-fav-{}", f.id),
                    kind: NodeKind::Favorite(f.clone()),
                    level: 2,
                    expandable: false,
                    expanded: false,
                    parent: Some("ax-mail-node-fav".into()),
                    suffix: shared_name.then(|| accounts.iter().find(|a| a.id == f.account_id).map(|a| a.email.clone())).flatten(),
                });
            }
            for p in people {
                nodes.push(Node {
                    dom_id: format!("ax-mail-node-person-{}", p.contact),
                    kind: NodeKind::Person(p.clone()),
                    level: 2,
                    expandable: false,
                    expanded: false,
                    parent: Some("ax-mail-node-fav".into()),
                    suffix: None,
                });
            }
        }
    }
    let mut children: HashMap<(AccountId, Option<FolderId>), Vec<Folder>> = HashMap::new();
    for f in folders {
        if f.role == FolderRole::Outbox && f.total_count == 0 {
            continue;
        }
        children.entry((f.account_id, f.parent_id)).or_default().push(f.clone());
    }
    for v in children.values_mut() {
        sort_siblings(v, snoozed);
    }
    for a in accounts {
        let acct_id = format!("ax-mail-node-acct-{}", a.id);
        let expanded = !prefs.collapsed_accounts.contains(&a.id);
        nodes.push(Node { dom_id: acct_id.clone(), kind: NodeKind::Account(a.clone()), level: 1, expandable: true, expanded, parent: None, suffix: None });
        if !expanded {
            continue;
        }
        fn walk(
            nodes: &mut Vec<Node>,
            children: &HashMap<(AccountId, Option<FolderId>), Vec<Folder>>,
            account: AccountId,
            parent: Option<FolderId>,
            parent_dom: &str,
            level: u32,
            edit: Option<&FolderEdit>,
        ) {
            if let Some(FolderEdit::Create { account: a, parent: p }) = edit
                && *a == account
                && *p == parent
                && parent.is_some()
            {
                nodes.push(Node {
                    dom_id: "ax-mail-node-new".into(),
                    kind: NodeKind::NewFolder { account, parent },
                    level,
                    expandable: false,
                    expanded: false,
                    parent: Some(parent_dom.to_string()),
                    suffix: None,
                });
            }
            for f in children.get(&(account, parent)).into_iter().flatten() {
                let dom = format!("ax-mail-node-{}", f.id);
                let has_children = children.get(&(account, Some(f.id))).is_some_and(|c| !c.is_empty())
                    || matches!(edit, Some(FolderEdit::Create { parent: Some(p), .. }) if *p == f.id);
                let expanded = has_children && (f.is_expanded || matches!(edit, Some(FolderEdit::Create { parent: Some(p), .. }) if *p == f.id));
                nodes.push(Node {
                    dom_id: dom.clone(),
                    kind: NodeKind::Folder(f.clone()),
                    level,
                    expandable: has_children,
                    expanded,
                    parent: Some(parent_dom.to_string()),
                    suffix: None,
                });
                if expanded {
                    walk(nodes, children, account, Some(f.id), &dom, level + 1, edit);
                }
            }
        }
        walk(&mut nodes, &children, a.id, None, &acct_id, 2, edit);
        if let Some(FolderEdit::Create { account, parent: None }) = edit
            && *account == a.id
        {
            nodes.push(Node {
                dom_id: "ax-mail-node-new".into(),
                kind: NodeKind::NewFolder { account: a.id, parent: None },
                level: 2,
                expandable: false,
                expanded: false,
                parent: Some(acct_id.clone()),
                suffix: None,
            });
        }
        nodes.push(Node {
            dom_id: format!("ax-mail-node-create-{}", a.id),
            kind: NodeKind::CreateEntry(a.id),
            level: 2,
            expandable: false,
            expanded: false,
            parent: Some(acct_id),
            suffix: None,
        });
    }
    nodes
}

/// Favourite contacts with an email address, with their unread Inbox mail.
fn favorite_people(folders: &[Folder]) -> Vec<FavoritePerson> {
    let s = store();
    let inboxes: Vec<FolderId> = folders.iter().filter(|f| f.role == FolderRole::Inbox).map(|f| f.id).collect();
    s.contacts()
        .unwrap_or_default()
        .into_iter()
        .filter(|c| c.is_favorite)
        .filter_map(|c| {
            let address = c.primary_email()?.to_string();
            let unread_query = MessageQuery {
                filter: MessageFilter::Unread,
                search: Some(format!("from:\"{}\"", address.replace('"', ""))),
                ..Default::default()
            };
            let unread = s.messages(&unread_query).unwrap_or_default().iter().filter(|m| inboxes.contains(&m.folder_id)).count() as i64;
            let name = if c.display_name.trim().is_empty() { address.clone() } else { c.display_name.clone() };
            Some(FavoritePerson { contact: c.id, name, address, unread })
        })
        .collect()
}

fn folder_of(node: &Node) -> Option<&Folder> {
    match &node.kind {
        NodeKind::Favorite(f) | NodeKind::Folder(f) => Some(f),
        _ => None,
    }
}

/// Run a folder pane menu command.
pub fn run_folder_command(mut app: AppState, mut ctx: MailCtx, cmd: &str, target: &MenuTarget) {
    let s = store();
    let folder_id = match target {
        MenuTarget::Folder(id) | MenuTarget::Favorite(id) => Some(*id),
        _ => None,
    };
    let folder = folder_id.and_then(|id| s.folder(id).ok().flatten());
    match (cmd, &folder) {
        ("f_new_sub", Some(f)) => {
            ctx.folder_edit.set(Some(FolderEdit::Create { account: f.account_id, parent: Some(f.id) }));
        }
        ("f_rename", Some(f)) => ctx.folder_edit.set(Some(FolderEdit::Rename(f.id))),
        ("f_delete", Some(f)) => ctx.dialog.set(Some(MailDialog::DeleteFolder(f.id))),
        ("f_mark_read", Some(f)) => {
            if app.check(s.mark_folder_read(f.id)).is_some() {
                app.toast(format!("Marked all items in {} as read", f.name));
                app.bump_mail();
            }
        }
        ("f_delete_all", Some(f)) => ctx.dialog.set(Some(MailDialog::DeleteAll(f.id))),
        ("f_empty", Some(f)) => ctx.dialog.set(Some(MailDialog::EmptyFolder(f.id))),
        ("f_fav_add", Some(f)) => {
            if app.check(s.set_folder_favorite(f.id, true)).is_some() {
                app.bump_mail();
            }
        }
        ("f_fav_remove", Some(f)) => {
            if app.check(s.set_folder_favorite(f.id, false)).is_some() {
                app.bump_mail();
            }
        }
        ("f_up", Some(f)) | ("f_down", Some(f)) => {
            if app.check(s.move_favorite(f.id, if cmd == "f_up" { -1 } else { 1 })).is_some() {
                app.bump_mail();
            }
        }
        ("f_move", Some(f)) => ctx.dialog.set(Some(MailDialog::MoveFolder(f.id))),
        ("fav_pin", _) => ctx.update_prefs(|p| p.favorites_pinned = true),
        ("fav_unpin", _) => ctx.update_prefs(|p| p.favorites_pinned = false),
        ("fav_hide", _) => {
            ctx.update_prefs(|p| p.favorites_hidden = true);
            app.toast("Favorites hidden. Show them again from View > Folder pane.");
        }
        ("a_new", _) => {
            if let MenuTarget::Account(a) = target {
                ctx.update_prefs(|p| p.collapsed_accounts.retain(|x| x != a));
                ctx.folder_edit.set(Some(FolderEdit::Create { account: *a, parent: None }));
            }
        }
        ("a_mark_read", _) => {
            if let MenuTarget::Account(a) = target
                && app.check(s.mark_account_read(*a)).is_some()
            {
                app.toast("Marked all as read");
                app.bump_mail();
            }
        }
        ("a_settings", _) => app.settings_dialog.set(Some(SettingsPage::Accounts)),
        ("p_email", _) | ("p_fav_remove", _) | ("p_contact", _) => {
            let MenuTarget::Person(id) = target else { return };
            let Some(c) = s.contact(*id).ok().flatten() else { return };
            match cmd {
                "p_email" => app.compose(crate::state::ComposeRequest::to(vec![EmailAddress::new(c.display_name.clone(), c.primary_email().unwrap_or_default())])),
                "p_contact" => app.navigate(crate::state::NavRequest::Contact(c.id)),
                _ => {
                    if app.check(s.set_contact_favorite(c.id, false)).is_some() {
                        if ctx.person.peek().as_ref().is_some_and(|p| p.contact == c.id)
                            && let Some(f) = *ctx.folder.peek()
                        {
                            actions::go_to_folder(app, ctx, f);
                        }
                        app.bump_people();
                    }
                }
            }
        }
        _ => {}
    }
}

/// An account node's name: its nickname when set, else the address.
pub fn account_label(a: &Account) -> String {
    if a.nickname.trim().is_empty() { a.email.clone() } else { a.nickname.trim().to_string() }
}

/// Trailing status glyph of an account node (protocols.md §2.6).
#[derive(Clone, Debug, PartialEq)]
pub struct AccountGlyph {
    /// `None` = spinner (first sync running).
    pub icon: Option<&'static str>,
    pub tip: String,
    pub class: &'static str,
}

/// The glyph for an account: a spinner during its first sync, else the
/// health icon (offline, needs sign-in, error...); `None` when healthy.
pub fn account_glyph(a: &Account) -> Option<AccountGlyph> {
    if a.kind == AccountKind::Local {
        return None;
    }
    let first_sync = crate::state::sync().and_then(|s| s.account_status(a.id)).filter(|l| l.syncing && l.first_sync);
    if let Some(l) = first_sync {
        return Some(AccountGlyph { icon: None, tip: l.progress.unwrap_or_else(|| "Syncing your mailbox…".into()), class: "ax-syncing" });
    }
    glyph_for(&axentra_sync::AccountHealth::of(a))
}

fn glyph_for(h: &axentra_sync::AccountHealth) -> Option<AccountGlyph> {
    let icon = h.icon?;
    let class = match icon {
        "cloud_off" => "ax-muted",
        "warning" | "lock_closed" => "ax-warning",
        _ => "ax-danger",
    };
    Some(AccountGlyph { icon: Some(icon), tip: h.message.clone().unwrap_or_default(), class })
}

/// Validate a folder name per §2.9; returns the error text.
pub fn validate_name(name: &str, account: AccountId, parent: Option<FolderId>, except: Option<FolderId>) -> Option<String> {
    let s = store();
    let imap = s.account(account).ok().flatten().is_some_and(|a| a.kind == AccountKind::Imap);
    if imap && name.contains('/') {
        return Some("Folder names can't contain \"/\".".into());
    }
    let dup = s
        .folders(Some(account))
        .unwrap_or_default()
        .iter()
        .any(|f| f.parent_id == parent && Some(f.id) != except && f.name.eq_ignore_ascii_case(name.trim()));
    dup.then(|| "A folder with this name already exists.".into())
}

/// True when the dragged item may be dropped on `folder`.
fn drop_allowed(drag: &Drag, folder: &Folder, current: Option<FolderId>) -> bool {
    match drag {
        Drag::Messages(_) => {
            Some(folder.id) != current && !matches!(folder.role, FolderRole::Notes | FolderRole::ConversationHistory | FolderRole::Outbox)
        }
        Drag::Folder(id) => *id != folder.id && !store().folder_is_within(folder.id, *id).unwrap_or(true),
    }
}

fn drop_on(app: AppState, mut ctx: MailCtx, model: &super::model::ListModel, folder: &Folder, copy: bool) {
    let drag = ctx.drag.peek().clone();
    ctx.drag.set(None);
    ctx.drop_target.set(None);
    let Some(drag) = drag else { return };
    if !drop_allowed(&drag, folder, *ctx.folder.peek()) {
        return;
    }
    match drag {
        Drag::Messages(ids) => {
            if copy {
                actions::copy_to(app, ids, folder.id);
            } else {
                actions::move_to(app, ctx, model, ids, folder.id);
            }
        }
        Drag::Folder(id) => {
            let s = store();
            let Some(f) = s.folder(id).ok().flatten() else { return };
            if f.account_id != folder.account_id {
                app.error("Folders can't move between accounts.");
                return;
            }
            if app.check(s.move_folder(id, Some(folder.id))).is_some() {
                let _ = s.set_folder_expanded(folder.id, true);
                app.toast(format!("Moved {} to {}", f.name, folder.name));
                app.bump_mail();
            }
        }
    }
}

#[component]
pub fn FolderPane(width: u32) -> Element {
    let app = use_app();
    let mut ctx = use_mail();
    let model = use_model();
    let _ = app.mail_rev();
    let _ = app.accounts_rev();
    // Favourite people come from People.
    let _ = app.people_rev();
    let s = store();
    let accounts = s.accounts().unwrap_or_default();
    let folders = s.folders(None).unwrap_or_default();
    let favorites = s.favorites().unwrap_or_default();
    let snoozed = s.snoozed_folder_ids().unwrap_or_default();
    let prefs = ctx.prefs.read().clone();
    let edit = ctx.folder_edit.read().clone();
    let people = favorite_people(&folders);
    let nodes = build_nodes(&accounts, &folders, &favorites, &people, &snoozed, &prefs, edit.as_ref());
    let searching = model.read().searching;
    let person = ctx.person.read().as_ref().map(|p| p.contact);
    let selected = if searching || person.is_some() { None } else { *ctx.folder.read() };
    let focus = ctx.tree_focus.read().clone();
    let mut tree_el = use_signal(|| None::<Rc<MountedData>>);
    let mut typeahead = use_signal(|| (String::new(), std::time::Instant::now()));

    // Ctrl+Y and other focus requests.
    let mut seen_focus_req = use_signal(|| *ctx.focus_tree.peek());
    use_effect(move || {
        let req = *ctx.focus_tree.read();
        if req != *seen_focus_req.peek() {
            seen_focus_req.set(req);
            if let Some(el) = tree_el.peek().clone() {
                spawn(async move {
                    let _ = el.set_focus(true).await;
                });
            }
        }
    });

    let nodes_rc = Rc::new(nodes.clone());
    let fav_count = nodes.iter().take_while(|n| matches!(n.kind, NodeKind::FavoritesHeader | NodeKind::Favorite(_))).count();
    let (fav_nodes, rest_nodes) = nodes.split_at(fav_count);
    let pinned = prefs.favorites_pinned && fav_count > 0;
    let active = focus.clone().filter(|f| nodes.iter().any(|n| &n.dom_id == f));

    let onkeydown = {
        let nodes = nodes_rc.clone();
        move |e: KeyboardEvent| {
            if tree_key(app, ctx, &nodes, &e, &mut typeahead) {
                e.prevent_default();
                e.stop_propagation();
            }
        }
    };
    let render = |list: &[Node]| -> Element {
        rsx! {
            for node in list.iter().cloned() {
                FolderNode {
                    key: "{node.dom_id}",
                    node: node.clone(),
                    selected,
                    selected_person: person,
                    focused: active.as_deref() == Some(node.dom_id.as_str()),
                    snoozed: snoozed.clone(),
                }
            }
        }
    };
    rsx! {
        nav { class: "ax-mail-folders", style: "width:{width}px", "aria-label": "Navigation pane", "data-ax-region": "true",
            div {
                class: "ax-mail-tree",
                role: "tree",
                "aria-label": "Folders",
                tabindex: "0",
                "data-ax-default-focus": "true",
                "aria-activedescendant": active.clone().unwrap_or_default(),
                onmounted: move |e| tree_el.set(Some(e.data())),
                onfocus: move |_| {
                    if ctx.tree_focus.peek().is_none() {
                        let first = selected.map(|f| format!("ax-mail-node-{f}"));
                        ctx.tree_focus.set(first);
                    }
                },
                onkeydown,
                if pinned {
                    div { class: "ax-mail-tree-pinned", {render(fav_nodes)} }
                    div { class: "ax-mail-tree-scroll", {render(rest_nodes)} }
                } else {
                    div { class: "ax-mail-tree-scroll", {render(&nodes)} }
                }
            }
            // "Add account" at the bottom of the folder list (protocols.md §2).
            div { class: "ax-mail-folders-foot",
                button {
                    r#type: "button",
                    class: "ax-mail-add-account",
                    title: "Add an email account",
                    onclick: move |_| match crate::shell::try_shell() {
                        Some(shell) => shell.open_add_account(),
                        None => {
                            let mut dialog = app.settings_dialog;
                            dialog.set(Some(SettingsPage::Accounts));
                        }
                    },
                    Icon { name: "person_add", size: 20 }
                    span { "Add account" }
                }
            }
        }
    }
}

/// Keyboard handling for the tree. Returns true when the key was used.
fn tree_key(app: AppState, mut ctx: MailCtx, nodes: &[Node], e: &KeyboardEvent, typeahead: &mut Signal<(String, std::time::Instant)>) -> bool {
    if ctx.folder_edit.peek().is_some() {
        return false;
    }
    let chord = keys::chord(e);
    let cur = ctx.tree_focus.peek().clone();
    let idx = cur.as_ref().and_then(|c| nodes.iter().position(|n| &n.dom_id == c));
    let mut go = |i: usize| {
        if let Some(n) = nodes.get(i) {
            ctx.tree_focus.set(Some(n.dom_id.clone()));
            document::eval(&format!("document.getElementById('{}')?.scrollIntoView({{block:'nearest'}})", n.dom_id));
        }
    };
    let node = idx.and_then(|i| nodes.get(i));
    match chord.as_str() {
        "ArrowDown" => go(idx.map(|i| (i + 1).min(nodes.len().saturating_sub(1))).unwrap_or(0)),
        "ArrowUp" => go(idx.map(|i| i.saturating_sub(1)).unwrap_or(0)),
        "Home" => go(0),
        "End" => go(nodes.len().saturating_sub(1)),
        "ArrowRight" => {
            if let (Some(i), Some(n)) = (idx, node) {
                if n.expandable && !n.expanded {
                    toggle_node(app, ctx, n);
                } else if n.expandable {
                    go(i + 1);
                }
            }
        }
        "ArrowLeft" => {
            if let Some(n) = node {
                if n.expandable && n.expanded {
                    toggle_node(app, ctx, n);
                } else if let Some(p) = n.parent.as_ref().and_then(|p| nodes.iter().position(|x| &x.dom_id == p)) {
                    go(p);
                }
            }
        }
        "Shift+8" | "*" | "Shift+*" => {
            // Expand all siblings.
            if let Some(n) = node {
                for sib in nodes.iter().filter(|x| x.parent == n.parent && x.expandable && !x.expanded) {
                    if let Some(f) = folder_of(sib) {
                        let _ = store().set_folder_expanded(f.id, true);
                    }
                }
                app.bump_mail();
            }
        }
        "Enter" | "Space" => {
            if let Some(n) = node {
                activate_node(app, ctx, n);
            }
        }
        "F2" => {
            if let Some(f) = node.and_then(folder_of)
                && f.role == FolderRole::Custom
            {
                ctx.folder_edit.set(Some(FolderEdit::Rename(f.id)));
            }
        }
        "Ctrl+Shift+E" => match node.map(|n| &n.kind) {
            Some(NodeKind::Folder(f)) | Some(NodeKind::Favorite(f)) => {
                ctx.folder_edit.set(Some(FolderEdit::Create { account: f.account_id, parent: Some(f.id) }))
            }
            Some(NodeKind::Account(a)) => ctx.folder_edit.set(Some(FolderEdit::Create { account: a.id, parent: None })),
            _ => {}
        },
        "Delete" => {
            if let Some(NodeKind::Folder(f)) = node.map(|n| &n.kind) {
                let snoozed = store().snoozed_folder_ids().unwrap_or_default();
                if !menus::is_fixed(f, &snoozed) {
                    ctx.dialog.set(Some(MailDialog::DeleteFolder(f.id)));
                }
            }
        }
        "Shift+F10" | "ContextMenu" => {
            if let Some(n) = node.cloned() {
                open_node_menu_at_element(ctx, n);
            }
        }
        "Escape" => ctx.request_list_focus(),
        _ => {
            // Type-ahead: jump to the next folder starting with the typed letters.
            let Key::Character(c) = e.key() else { return false };
            let m = e.modifiers();
            if m.ctrl() || m.alt() || m.meta() || c.chars().count() != 1 || !c.chars().all(|ch| ch.is_alphanumeric()) {
                return false;
            }
            let (mut buf, at) = typeahead.peek().clone();
            if at.elapsed() > Duration::from_millis(500) {
                buf.clear();
            }
            buf.push_str(&c.to_lowercase());
            typeahead.set((buf.clone(), std::time::Instant::now()));
            let start = idx.map(|i| if buf.chars().count() == 1 { i + 1 } else { i }).unwrap_or(0);
            let found = (0..nodes.len()).map(|k| (start + k) % nodes.len()).find(|&i| {
                folder_of(&nodes[i]).is_some_and(|f| f.name.to_lowercase().starts_with(&buf))
            });
            match found {
                Some(i) => go(i),
                None => return false,
            }
        }
    }
    true
}

fn toggle_node(app: AppState, ctx: MailCtx, n: &Node) {
    match &n.kind {
        NodeKind::FavoritesHeader => ctx.update_prefs(|p| p.favorites_collapsed = !p.favorites_collapsed),
        NodeKind::Account(a) => {
            let id = a.id;
            ctx.update_prefs(|p| {
                if p.collapsed_accounts.contains(&id) {
                    p.collapsed_accounts.retain(|x| *x != id);
                } else {
                    p.collapsed_accounts.push(id);
                }
            })
        }
        NodeKind::Folder(f) if app.check(store().set_folder_expanded(f.id, !n.expanded)).is_some() => app.bump_mail(),
        _ => {}
    }
}

fn activate_node(app: AppState, mut ctx: MailCtx, n: &Node) {
    match &n.kind {
        NodeKind::Favorite(f) | NodeKind::Folder(f) => actions::go_to_folder(app, ctx, f.id),
        NodeKind::Person(p) => {
            actions::show_person(app, ctx, PersonView { contact: p.contact, name: p.name.clone(), address: p.address.clone() });
        }
        NodeKind::CreateEntry(a) => ctx.folder_edit.set(Some(FolderEdit::Create { account: *a, parent: None })),
        NodeKind::FavoritesHeader | NodeKind::Account(_) => toggle_node(app, ctx, n),
        NodeKind::NewFolder { .. } => {}
    }
}

fn node_menu(n: &Node) -> Option<(Vec<crate::kit::MenuItem>, MenuTarget)> {
    let snoozed = store().snoozed_folder_ids().unwrap_or_default();
    match &n.kind {
        NodeKind::Folder(f) => Some((menus::folder_menu(f, false, &snoozed), MenuTarget::Folder(f.id))),
        NodeKind::Favorite(f) => Some((menus::folder_menu(f, true, &snoozed), MenuTarget::Favorite(f.id))),
        NodeKind::FavoritesHeader => Some((menus::favorites_header_menu(false), MenuTarget::FavoritesHeader)),
        NodeKind::Account(a) => Some((menus::account_menu(), MenuTarget::Account(a.id))),
        NodeKind::Person(p) => Some((menus::person_menu(), MenuTarget::Person(p.contact))),
        _ => None,
    }
}

fn open_node_menu(mut ctx: MailCtx, n: &Node, x: f64, y: f64) {
    if let Some((mut items, target)) = node_menu(n) {
        if matches!(n.kind, NodeKind::FavoritesHeader) {
            items = menus::favorites_header_menu(ctx.prefs.peek().favorites_pinned);
        }
        ctx.menu.set(Some(ContextMenu { x, y, items, target }));
    }
}

/// Open a node's menu below the node itself (keyboard: Shift+F10).
fn open_node_menu_at_element(ctx: MailCtx, n: Node) {
    spawn(async move {
        let js = format!(
            "const r = document.getElementById('{}')?.getBoundingClientRect(); return r ? [r.left + 24, r.bottom] : [80, 160];",
            n.dom_id
        );
        let pos = document::eval(&js).join::<Vec<f64>>().await.unwrap_or_else(|_| vec![80.0, 160.0]);
        open_node_menu(ctx, &n, pos.first().copied().unwrap_or(80.0), pos.get(1).copied().unwrap_or(160.0));
    });
}

#[component]
fn FolderNode(node: Node, selected: Option<FolderId>, selected_person: Option<ContactId>, focused: bool, snoozed: Vec<FolderId>) -> Element {
    let app = use_app();
    let mut ctx = use_mail();
    let model = use_model();
    let level = node.level;
    let indent = 8 + (level.saturating_sub(1)) * 16;
    match node.kind.clone() {
        NodeKind::FavoritesHeader | NodeKind::Account(_) => {
            let (label, class) = match &node.kind {
                NodeKind::Account(a) => (account_label(a), "ax-mail-node-account"),
                _ => ("Favorites".to_string(), "ax-mail-node-favorites"),
            };
            let glyph = if let NodeKind::Account(a) = &node.kind { account_glyph(a) } else { None };
            let n2 = node.clone();
            let n3 = node.clone();
            let n4 = node.clone();
            let account_id = if let NodeKind::Account(a) = &node.kind { Some(a.id) } else { None };
            // Headers use negative drop-target ids: -1 for Favorites, -1000 - id for accounts.
            let header_target = account_id.map(|a| -1000 - a).unwrap_or(-1);
            let is_drop = *ctx.drop_target.read() == Some(header_target);
            rsx! {
                div {
                    id: "{node.dom_id}",
                    class: "ax-mail-node ax-mail-node-header {class}",
                    class: if focused { "ax-focused" },
                    class: if is_drop { "ax-drop" },
                    role: "treeitem",
                    "aria-level": "{level}",
                    "aria-expanded": if node.expanded { "true" } else { "false" },
                    "aria-label": "{label}",
                    style: "padding-left:{indent}px",
                    onclick: move |_| {
                        ctx.tree_focus.set(Some(n2.dom_id.clone()));
                        toggle_node(app, ctx, &n2);
                    },
                    oncontextmenu: move |e: MouseEvent| {
                        e.prevent_default();
                        let c = e.client_coordinates();
                        open_node_menu(ctx, &n3, c.x, c.y);
                    },
                    // Folders dropped on Favorites become favourites; on an account, top-level folders.
                    ondragover: move |e: DragEvent| {
                        e.prevent_default();
                        let ok = matches!(*ctx.drag.peek(), Some(Drag::Folder(_)));
                        let want = ok.then_some(header_target);
                        if *ctx.drop_target.peek() != want {
                            ctx.drop_target.set(want);
                        }
                    },
                    ondragleave: move |_| {
                        if *ctx.drop_target.peek() == Some(header_target) {
                            ctx.drop_target.set(None);
                        }
                    },
                    ondrop: move |e: DragEvent| {
                        e.prevent_default();
                        let drag = ctx.drag.peek().clone();
                        ctx.drag.set(None);
                        ctx.drop_target.set(None);
                        if let Some(Drag::Folder(id)) = drag {
                            let s = store();
                            let result = match account_id {
                                None => s.set_folder_favorite(id, true),
                                Some(a) => match s.folder(id).ok().flatten() {
                                    Some(f) if f.account_id == a => s.move_folder(id, None),
                                    _ => Err(Error::Invalid("Folders can't move between accounts.".into())),
                                },
                            };
                            if app.check(result).is_some() {
                                app.bump_mail();
                            }
                        }
                    },
                    span { class: "ax-mail-node-chevron",
                        Icon { name: if node.expanded { "chevron_down" } else { "chevron_right" }, size: 16 }
                    }
                    span { class: "ax-mail-node-name ax-ellipsis", title: "{label}", "{label}" }
                    if let Some(g) = glyph {
                        span { class: "ax-mail-node-status {g.class}", role: "img", title: "{g.tip}", "aria-label": "{g.tip}",
                            match g.icon {
                                Some(icon) => rsx! { Icon { name: icon, size: 16 } },
                                None => rsx! { span { class: "ax-spinner", "aria-hidden": "true", style: "width:14px;height:14px;border-width:2px" } },
                            }
                        }
                    }
                    button {
                        r#type: "button",
                        class: "ax-mail-node-more",
                        tabindex: "-1",
                        title: "More options",
                        "aria-label": "More options",
                        onclick: move |e: MouseEvent| {
                            e.stop_propagation();
                            let c = e.client_coordinates();
                            open_node_menu(ctx, &n4, c.x, c.y + 8.0);
                        },
                        Icon { name: "more_horizontal", size: 16 }
                    }
                }
            }
        }
        NodeKind::Person(p) => {
            let is_selected = selected_person == Some(p.contact);
            let n_menu = node.clone();
            let n_menu2 = node.clone();
            let dom = node.dom_id.clone();
            let accessible = if p.unread > 0 { format!("{}, {} unread", p.name, p.unread) } else { p.name.clone() };
            let view = PersonView { contact: p.contact, name: p.name.clone(), address: p.address.clone() };
            let tip = format!("{} <{}>", p.name, p.address);
            let more_label = format!("More options for {}", p.name);
            let (name, address, unread) = (p.name.clone(), p.address.clone(), p.unread);
            rsx! {
                div {
                    id: "{node.dom_id}",
                    class: "ax-mail-node ax-mail-node-person",
                    class: if is_selected { "ax-selected" },
                    class: if focused { "ax-focused" },
                    class: if p.unread > 0 || is_selected { "ax-bold" },
                    role: "treeitem",
                    "aria-level": "{level}",
                    "aria-selected": if is_selected { "true" } else { "false" },
                    "aria-label": "{accessible}",
                    title: "{tip}",
                    style: "padding-left:{indent}px",
                    onclick: move |_| {
                        ctx.tree_focus.set(Some(dom.clone()));
                        actions::show_person(app, ctx, view.clone());
                    },
                    oncontextmenu: move |e: MouseEvent| {
                        e.prevent_default();
                        let c = e.client_coordinates();
                        open_node_menu(ctx, &n_menu, c.x, c.y);
                    },
                    span { class: "ax-mail-node-chevron" }
                    span { class: "ax-mail-node-icon", Avatar { name: name.clone(), email: address, size: 20 } }
                    span { class: "ax-mail-node-name ax-ellipsis", "{name}" }
                    if unread > 0 {
                        span { class: "ax-mail-node-count", "aria-hidden": "true", "{unread}" }
                    }
                    button {
                        r#type: "button",
                        class: "ax-mail-node-more",
                        tabindex: "-1",
                        title: "More options",
                        "aria-label": "{more_label}",
                        onclick: move |e: MouseEvent| {
                            e.stop_propagation();
                            let c = e.client_coordinates();
                            open_node_menu(ctx, &n_menu2, c.x, c.y + 8.0);
                        },
                        Icon { name: "more_horizontal", size: 16 }
                    }
                }
            }
        }
        NodeKind::CreateEntry(account) => rsx! {
            div {
                id: "{node.dom_id}",
                class: "ax-mail-node ax-mail-node-create",
                class: if focused { "ax-focused" },
                role: "treeitem",
                "aria-level": "{level}",
                style: "padding-left:{indent + 16}px",
                onclick: move |_| ctx.folder_edit.set(Some(FolderEdit::Create { account, parent: None })),
                Icon { name: "add", size: 20 }
                span { class: "ax-mail-node-name ax-ellipsis", title: "Create new folder", "Create new folder" }
            }
        },
        NodeKind::NewFolder { account, parent } => rsx! {
            FolderNameField { key: "{node.dom_id}", account, parent, folder: None, initial: String::new(), indent: indent + 16, icon: "folder" }
        },
        NodeKind::Favorite(f) | NodeKind::Folder(f) => {
            let is_fav = matches!(node.kind, NodeKind::Favorite(_));
            let is_snoozed = snoozed.contains(&f.id);
            let editing = !is_fav && *ctx.folder_edit.read() == Some(FolderEdit::Rename(f.id));
            if editing {
                return rsx! {
                    FolderNameField {
                        key: "rename-{f.id}",
                        account: f.account_id,
                        parent: f.parent_id,
                        folder: Some(f.id),
                        initial: f.name.clone(),
                        indent: indent + 16,
                        icon: folder_icon(&f, is_snoozed),
                    }
                };
            }
            let is_selected = selected == Some(f.id);
            let count = folder_count(&f, is_snoozed);
            let drag = ctx.drag.read().clone();
            let is_drop = *ctx.drop_target.read() == Some(f.id);
            let can_drop = drag.as_ref().is_some_and(|d| drop_allowed(d, &f, selected));
            let bold = is_selected || (f.unread_count > 0 && count.as_ref().is_some_and(|(_, u)| *u));
            let icon = folder_icon(&f, is_snoozed);
            let icon_name = if is_selected { format!("{icon}_filled") } else { icon.to_string() };
            let icon_name = if crate::icons::icon_svg(&icon_name).is_some() { icon_name } else { icon.to_string() };
            let name = match &node.suffix {
                Some(sfx) => format!("{} ({sfx})", f.name),
                None => f.name.clone(),
            };
            let accessible = match &count {
                Some((n, true)) => format!("{name}, {n} unread"),
                Some((n, false)) => format!("{name}, {n} items"),
                None => name.clone(),
            };
            let draggable = !is_fav && !menus::is_fixed(&f, &snoozed);
            let (fid, fid2, fid3) = (f.id, f.id, f.id);
            let f_drop = f.clone();
            let n_menu = node.clone();
            let n_menu2 = node.clone();
            let n_toggle = node.clone();
            let dom = node.dom_id.clone();
            let expandable = node.expandable;
            let expanded = node.expanded;
            rsx! {
                div {
                    id: "{node.dom_id}",
                    class: "ax-mail-node",
                    class: if is_selected { "ax-selected" },
                    class: if focused { "ax-focused" },
                    class: if bold { "ax-bold" },
                    class: if is_drop && can_drop { "ax-drop" },
                    class: if drag.is_some() && !can_drop { "ax-no-drop" },
                    role: "treeitem",
                    "aria-level": "{level}",
                    "aria-selected": if is_selected { "true" } else { "false" },
                    "aria-expanded": if expandable { if expanded { "true" } else { "false" } },
                    "aria-label": "{accessible}",
                    title: "{f.name}",
                    style: "padding-left:{indent}px",
                    draggable: if draggable { "true" } else { "false" },
                    onclick: move |_| {
                        ctx.tree_focus.set(Some(dom.clone()));
                        actions::go_to_folder(app, ctx, fid);
                    },
                    oncontextmenu: move |e: MouseEvent| {
                        e.prevent_default();
                        let c = e.client_coordinates();
                        open_node_menu(ctx, &n_menu, c.x, c.y);
                    },
                    ondragstart: move |_| ctx.drag.set(Some(Drag::Folder(fid2))),
                    ondragend: move |_| {
                        ctx.drag.set(None);
                        ctx.drop_target.set(None);
                    },
                    ondragover: move |e: DragEvent| {
                        e.prevent_default();
                        if *ctx.drop_target.peek() != Some(fid3) {
                            ctx.drop_target.set(Some(fid3));
                            // Hovering a collapsed parent for 700 ms expands it.
                            if expandable && !expanded {
                                spawn(async move {
                                    tokio::time::sleep(Duration::from_millis(700)).await;
                                    if *ctx.drop_target.peek() == Some(fid3) && store().set_folder_expanded(fid3, true).is_ok() {
                                        app.bump_mail();
                                    }
                                });
                            }
                        }
                    },
                    ondragleave: move |_| {
                        if *ctx.drop_target.peek() == Some(fid3) {
                            ctx.drop_target.set(None);
                        }
                    },
                    ondrop: move |e: DragEvent| {
                        e.prevent_default();
                        let copy = e.modifiers().ctrl();
                        drop_on(app, ctx, &model.read(), &f_drop, copy);
                    },
                    span {
                        class: "ax-mail-node-chevron",
                        onclick: move |e: MouseEvent| {
                            if expandable {
                                e.stop_propagation();
                                toggle_node(app, ctx, &n_toggle);
                            }
                        },
                        if expandable {
                            Icon { name: if expanded { "chevron_down" } else { "chevron_right" }, size: 16 }
                        }
                    }
                    span { class: "ax-mail-node-icon", Icon { name: icon_name, size: 20 } }
                    span { class: "ax-mail-node-name ax-ellipsis",
                        "{f.name}"
                        if let Some(sfx) = &node.suffix { span { class: "ax-mail-node-suffix", " {sfx}" } }
                    }
                    if let Some((n, unread)) = count {
                        span { class: if unread { "ax-mail-node-count" } else { "ax-mail-node-count ax-total" }, "aria-hidden": "true", "{n}" }
                    }
                    button {
                        r#type: "button",
                        class: "ax-mail-node-more",
                        tabindex: "-1",
                        title: "More options",
                        "aria-label": "More options for {f.name}",
                        onclick: move |e: MouseEvent| {
                            e.stop_propagation();
                            let c = e.client_coordinates();
                            open_node_menu(ctx, &n_menu2, c.x, c.y + 8.0);
                        },
                        Icon { name: "more_horizontal", size: 16 }
                    }
                }
            }
        }
    }
}

/// Inline name field for creating or renaming a folder.
#[component]
fn FolderNameField(account: AccountId, parent: Option<FolderId>, folder: Option<FolderId>, initial: String, indent: u32, icon: String) -> Element {
    let app = use_app();
    let mut ctx = use_mail();
    let mut value = use_signal(|| initial.clone());
    let original = use_signal(|| initial.clone());
    let mut error = use_signal(|| None::<String>);
    let mut done = use_signal(|| false);
    let mut commit = move |cancel: bool| {
        if *done.peek() {
            return;
        }
        let name = value.peek().trim().to_string();
        if cancel || name.is_empty() || (folder.is_some() && name == *original.peek()) {
            done.set(true);
            ctx.folder_edit.set(None);
            ctx.request_tree_focus();
            return;
        }
        if let Some(err) = validate_name(&name, account, parent, folder) {
            error.set(Some(err));
            return;
        }
        let s = store();
        let result = match folder {
            Some(id) => s.rename_folder(id, &name).map(|_| id),
            None => s.create_folder(account, parent, &name),
        };
        done.set(true);
        ctx.folder_edit.set(None);
        match result {
            Ok(id) => {
                if folder.is_none() {
                    app.toast(format!("Created folder {name}"));
                }
                ctx.tree_focus.set(Some(format!("ax-mail-node-{id}")));
                ctx.request_tree_focus();
                app.bump_mail();
            }
            Err(_) if folder.is_none() => {
                app.error("Couldn't create the folder. Try again.");
            }
            Err(e) => {
                app.error(format!("Couldn't rename the folder: {e}"));
            }
        }
    };
    rsx! {
        div { class: "ax-mail-node ax-mail-node-editing", style: "padding-left:{indent}px", role: "treeitem",
            span { class: "ax-mail-node-icon", Icon { name: icon.clone(), size: 20 } }
            div { class: "ax-mail-node-field",
                input {
                    class: "ax-input",
                    class: if error().is_some() { "ax-invalid" },
                    r#type: "text",
                    value: "{value}",
                    placeholder: "New folder",
                    "aria-label": if folder.is_some() { "Rename folder" } else { "New folder name" },
                    "aria-invalid": if error().is_some() { "true" } else { "false" },
                    onmounted: move |e| async move {
                        let _ = e.set_focus(true).await;
                        document::eval("document.activeElement?.select?.()");
                    },
                    oninput: move |e| {
                        value.set(e.value());
                        error.set(None);
                    },
                    onkeydown: move |e: KeyboardEvent| {
                        ctx.mark_typing_for(&e);
                        keys::stop_typing_keys(&e);
                        match e.key() {
                            Key::Enter => {
                                e.prevent_default();
                                commit(false);
                            }
                            Key::Escape => {
                                e.stop_propagation();
                                commit(true);
                            }
                            _ => {}
                        }
                    },
                    onblur: move |_| commit(false),
                }
                if let Some(err) = error() {
                    div { class: "ax-mail-node-error", role: "alert", "{err}" }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(id: FolderId, role: FolderRole, name: &str, unread: i64, total: i64) -> Folder {
        Folder { id, account_id: 1, role, name: name.into(), unread_count: unread, total_count: total, is_expanded: true, ..Default::default() }
    }

    #[test]
    fn account_nodes_use_nicknames_and_health_glyphs() {
        let mut a = Account { email: "ana@example.org".into(), ..Default::default() };
        assert_eq!(account_label(&a), "ana@example.org");
        a.nickname = " Work ".into();
        assert_eq!(account_label(&a), "Work");
        let healthy = axentra_sync::AccountHealth { state: axentra_sync::AccountState::Ok, icon: None, message: None, intent: "info", action: None };
        assert_eq!(glyph_for(&healthy), None);
        let offline = axentra_sync::AccountHealth { icon: Some("cloud_off"), message: Some("You're offline.".into()), ..healthy.clone() };
        assert_eq!(glyph_for(&offline), Some(AccountGlyph { icon: Some("cloud_off"), tip: "You're offline.".into(), class: "ax-muted" }));
        let locked = axentra_sync::AccountHealth { icon: Some("lock_closed"), ..healthy.clone() };
        assert_eq!(glyph_for(&locked).map(|g| g.class), Some("ax-warning"));
        let failed = axentra_sync::AccountHealth { icon: Some("error_circle"), ..healthy };
        assert_eq!(glyph_for(&failed).map(|g| g.class), Some("ax-danger"));
    }

    #[test]
    fn counts_follow_the_folder_role() {
        assert_eq!(folder_count(&folder(1, FolderRole::Inbox, "Inbox", 12, 40), false), Some(("12".into(), true)));
        assert_eq!(folder_count(&folder(2, FolderRole::Drafts, "Drafts", 0, 3), false), Some(("3".into(), false)));
        assert_eq!(folder_count(&folder(3, FolderRole::Sent, "Sent Items", 4, 9), false), None);
        assert_eq!(folder_count(&folder(4, FolderRole::Inbox, "Inbox", 0, 9), false), None);
        assert_eq!(folder_count(&folder(5, FolderRole::Custom, "Snoozed", 0, 2), true), Some(("2".into(), false)));
        assert_eq!(folder_count(&folder(6, FolderRole::Custom, "Big", 12_000, 12_000), false), Some(("9999+".into(), true)));
    }

    #[test]
    fn tree_order_and_nesting() {
        let account = Account { id: 1, email: "me@x.example".into(), ..Default::default() };
        let mut folders = vec![
            folder(10, FolderRole::Custom, "zeta", 0, 0),
            folder(2, FolderRole::Junk, "Junk Email", 0, 0),
            folder(1, FolderRole::Inbox, "Inbox", 0, 0),
            folder(11, FolderRole::Custom, "Alpha", 0, 0),
            folder(12, FolderRole::Custom, "Child", 0, 0),
            folder(13, FolderRole::Custom, "Snoozed", 0, 0),
            folder(3, FolderRole::Sent, "Sent Items", 0, 0),
            folder(4, FolderRole::Outbox, "Outbox", 0, 0),
        ];
        folders[4].parent_id = Some(11);
        use super::super::ctx::FolderPrefs;
        let prefs = FolderPrefs { favorites_hidden: true, ..Default::default() };
        let nodes = build_nodes(std::slice::from_ref(&account), &folders, &[], &[], &[13], &prefs, None);
        let names: Vec<(String, u32)> = nodes
            .iter()
            .filter_map(|n| folder_of(n).map(|f| (f.name.clone(), n.level)))
            .collect();
        assert_eq!(
            names,
            vec![
                ("Inbox".into(), 2),
                ("Sent Items".into(), 2),
                ("Snoozed".into(), 2),
                ("Junk Email".into(), 2),
                ("Alpha".into(), 2),
                ("Child".into(), 3),
                ("zeta".into(), 2),
            ]
        );
        assert!(matches!(nodes.last().unwrap().kind, NodeKind::CreateEntry(1)));
        // Two Inboxes in Favorites are told apart by their account.
        let people = vec![FavoritePerson { contact: 7, name: "Priya Raman".into(), address: "priya@x.example".into(), unread: 2 }];
        let other = Account { id: 2, email: "me@y.example".into(), ..Default::default() };
        let mut inbox2 = folder(20, FolderRole::Inbox, "Inbox", 0, 0);
        inbox2.account_id = 2;
        let favs = vec![folders[2].clone(), inbox2, folders[6].clone()];
        let shown = FolderPrefs { favorites_hidden: false, collapsed_accounts: vec![1, 2], ..Default::default() };
        let nodes = build_nodes(&[account.clone(), other], &folders, &favs, &people, &[13], &shown, None);
        let suffixes: Vec<Option<String>> = nodes.iter().filter(|n| matches!(n.kind, NodeKind::Favorite(_))).map(|n| n.suffix.clone()).collect();
        assert_eq!(suffixes, vec![Some("me@x.example".into()), Some("me@y.example".into()), None]);
        // Favourite people follow the favourite folders.
        assert!(matches!(&nodes[4].kind, NodeKind::Person(p) if p.name == "Priya Raman"));
        assert_eq!(nodes[4].dom_id, "ax-mail-node-person-7");
        // Creating a subfolder shows the field under its parent.
        let edit = FolderEdit::Create { account: 1, parent: Some(10) };
        let nodes = build_nodes(std::slice::from_ref(&account), &folders, &[], &[], &[13], &prefs, Some(&edit));
        let zeta = nodes.iter().position(|n| n.dom_id == "ax-mail-node-10").unwrap();
        assert!(matches!(nodes[zeta + 1].kind, NodeKind::NewFolder { parent: Some(10), .. }));
    }
}
