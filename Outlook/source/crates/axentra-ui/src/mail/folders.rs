//! Folder pane: Favorites, Views (Packages and Topics) and one tree per
//! account (docs/spec/mail.md §2).
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
use super::ctx::{ContextMenu, Drag, DropPlace, FolderEdit, FolderPrefs, MailCtx, MailDialog, MenuTarget, PersonView, use_mail, use_model};
use super::menus;
use crate::kit::{Avatar, Icon, keys};
use crate::settings::PageId;
use crate::state::{AppState, MailView, SettingsPage, store, use_app};
use axentra_core::store::{OrdersSummary, TopicCount};
use axentra_core::topics::Topic;

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
    /// "Views": Packages and Topics, found on this device.
    ViewsHeader,
    /// Active orders, and how many need attention.
    Packages { active: u32, attention: u32 },
    TopicsParent { expanded: bool },
    /// A topic with its unread and total counts.
    Topic(Topic, u32, u32),
}

/// What the Views section shows; `None` = that feature is off.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ViewsInfo {
    pub packages: Option<OrdersSummary>,
    pub topics: Option<Vec<TopicCount>>,
    /// Topic ids hidden from the folder pane (`settings.topics_hidden`).
    pub hidden: Vec<String>,
}

/// The node's view (Packages, All topics or one topic).
fn view_of(node: &Node) -> Option<MailView> {
    match node.kind {
        NodeKind::Packages { .. } => Some(MailView::Packages),
        NodeKind::TopicsParent { .. } => Some(MailView::Topics(None)),
        NodeKind::Topic(t, ..) => Some(MailView::Topics(Some(t))),
        _ => None,
    }
}

/// A view node drawn dimmed: a topic with no mail at all. Never Packages,
/// which always opens a working page (delivered orders, or the first look
/// through the mail), whatever is on the way.
fn dimmed(kind: &NodeKind) -> bool {
    matches!(kind, NodeKind::Topic(_, _, 0))
}

/// The name type-ahead matches: a folder's name or a view's label.
fn node_name(node: &Node) -> Option<String> {
    match &node.kind {
        NodeKind::Favorite(f) | NodeKind::Folder(f) => Some(f.name.clone()),
        NodeKind::Packages { .. } => Some("Packages".into()),
        NodeKind::TopicsParent { .. } => Some("Topics".into()),
        NodeKind::Topic(t, ..) => Some(t.label().into()),
        _ => None,
    }
}

/// Type-ahead: the next node from `start` (wrapping) whose name starts with `typed`.
fn typeahead_target(nodes: &[Node], start: usize, typed: &str) -> Option<usize> {
    if nodes.is_empty() {
        return None;
    }
    (0..nodes.len()).map(|k| (start + k) % nodes.len()).find(|&i| node_name(&nodes[i]).is_some_and(|n| n.to_lowercase().starts_with(typed)))
}

/// The Views section: after Favorites, before the accounts. Hidden when
/// both features are off.
fn views_nodes(views: &ViewsInfo, prefs: &super::ctx::FolderPrefs) -> Vec<Node> {
    let mut nodes = Vec::new();
    if views.packages.is_none() && views.topics.is_none() {
        return nodes;
    }
    let header = "ax-mail-node-views".to_string();
    nodes.push(Node {
        dom_id: header.clone(),
        kind: NodeKind::ViewsHeader,
        level: 1,
        expandable: true,
        expanded: !prefs.views_collapsed,
        parent: None,
        suffix: None,
    });
    if prefs.views_collapsed {
        return nodes;
    }
    if let Some(p) = views.packages {
        nodes.push(Node {
            dom_id: "ax-mail-node-packages".into(),
            kind: NodeKind::Packages { active: p.active, attention: p.attention },
            level: 2,
            expandable: false,
            expanded: false,
            parent: Some(header.clone()),
            suffix: None,
        });
    }
    if let Some(counts) = &views.topics {
        let parent = "ax-mail-node-topics".to_string();
        let expanded = !prefs.topics_collapsed;
        nodes.push(Node {
            dom_id: parent.clone(),
            kind: NodeKind::TopicsParent { expanded },
            level: 2,
            expandable: true,
            expanded,
            parent: Some(header),
            suffix: None,
        });
        if expanded {
            // Topic::ALL order (Other last), without the hidden ones.
            for t in Topic::ALL.into_iter().filter(|t| !views.hidden.iter().any(|h| h == t.id())) {
                let c = counts.iter().find(|c| c.topic == t);
                nodes.push(Node {
                    dom_id: format!("ax-mail-node-topic-{}", t.id()),
                    kind: NodeKind::Topic(t, c.map(|c| c.unread).unwrap_or(0), c.map(|c| c.total).unwrap_or(0)),
                    level: 3,
                    expandable: false,
                    expanded: false,
                    parent: Some(parent.clone()),
                    suffix: None,
                });
            }
        }
    }
    nodes
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

/// Sort siblings: the order the user dragged them into (`custom`), and
/// otherwise well-known folders in [`ROLE_ORDER`], then custom A to Z.
/// Folders missing from `custom` (made since) go after the ones in it.
fn sort_siblings(folders: &mut [Folder], snoozed: &[FolderId], custom: Option<&Vec<FolderId>>) {
    sort_default(folders, snoozed);
    if let Some(order) = custom {
        folders.sort_by_key(|f| order.iter().position(|o| *o == f.id).unwrap_or(usize::MAX));
    }
}

/// `current` with `moving` placed just before (or after) `target`.
fn reordered(current: &[FolderId], moving: FolderId, target: FolderId, after: bool) -> Vec<FolderId> {
    if moving == target {
        return current.to_vec();
    }
    let mut ids: Vec<FolderId> = current.iter().copied().filter(|i| *i != moving).collect();
    let pos = ids.iter().position(|i| *i == target).map(|p| if after { p + 1 } else { p }).unwrap_or(ids.len());
    ids.insert(pos, moving);
    ids
}

/// The folders beside `folder` (same account and parent), as the pane
/// shows them now.
fn siblings_in_order(folder: &Folder, prefs: &FolderPrefs, snoozed: &[FolderId]) -> Vec<FolderId> {
    let mut sibs: Vec<Folder> =
        store()
            .folders(Some(folder.account_id))
            .unwrap_or_default()
            .into_iter()
            // As shown: an empty Outbox isn't in the pane (see build_nodes).
            .filter(|f| f.parent_id == folder.parent_id && !(f.role == FolderRole::Outbox && f.total_count == 0))
            .collect();
    sort_siblings(&mut sibs, snoozed, prefs.folder_order.get(&FolderPrefs::order_key(folder.account_id, folder.parent_id)));
    sibs.into_iter().map(|f| f.id).collect()
}

/// Put `moving` before or after its sibling `target` in the folder pane.
fn place_folder(ctx: MailCtx, moving: FolderId, target: &Folder, after: bool) {
    let current = siblings_in_order(target, &ctx.prefs.peek(), &snoozed_ids());
    let order = reordered(&current, moving, target.id, after);
    let key = FolderPrefs::order_key(target.account_id, target.parent_id);
    ctx.update_prefs(|p| {
        p.folder_order.insert(key, order);
    });
}

/// Move a folder one place up or down among its siblings (Move up / Move
/// down, for the keyboard).
fn nudge_folder(ctx: MailCtx, folder: &Folder, up: bool) {
    let current = siblings_in_order(folder, &ctx.prefs.peek(), &snoozed_ids());
    let Some(pos) = current.iter().position(|i| *i == folder.id) else { return };
    // Past the next folder shown, not a hidden one.
    let prefs = ctx.prefs.peek().clone();
    let all = store().folders(Some(folder.account_id)).unwrap_or_default();
    let shown = |i: &usize| all.iter().find(|f| f.id == current[*i]).is_none_or(|f| !left_out(f, &all, &prefs));
    let to = if up { (0..pos).rev().find(shown) } else { (pos + 1..current.len()).find(shown) };
    if let Some(to) = to {
        place_folder(ctx, folder.id, &Folder { id: current[to], ..folder.clone() }, !up);
    }
}

/// True when `folder` is left out of the list: hidden, with no unread
/// mail in it or its subfolders (new mail brings it back until read).
fn left_out(folder: &Folder, folders: &[Folder], prefs: &FolderPrefs) -> bool {
    fn unread_within(f: &Folder, folders: &[Folder], depth: u32) -> bool {
        f.unread_count > 0
            || (depth < 64 && folders.iter().any(|c| c.account_id == f.account_id && c.parent_id == Some(f.id) && unread_within(c, folders, depth + 1)))
    }
    prefs.hidden_folders.contains(&folder.id) && !unread_within(folder, folders, 0)
}

/// The hidden folders of `account` (those still there).
fn hidden_in(account: AccountId, prefs: &FolderPrefs) -> Vec<FolderId> {
    let ids: Vec<FolderId> = store().folders(Some(account)).unwrap_or_default().iter().map(|f| f.id).collect();
    prefs.hidden_folders.iter().copied().filter(|id| ids.contains(id)).collect()
}

/// Ids of the Snoozed folders (sorted with the well-known ones).
fn snoozed_ids() -> Vec<FolderId> {
    store().snoozed_folder_ids().unwrap_or_default()
}

/// Which part of a row the pointer is over: top quarter, middle, bottom
/// quarter. `edges` false: only the top or bottom half.
fn drop_zone(y: f64, height: f64, edges: bool) -> DropPlace {
    if !edges {
        return if y < height / 2.0 { DropPlace::Before } else { DropPlace::After };
    }
    if y < height / 4.0 {
        DropPlace::Before
    } else if y > height * 3.0 / 4.0 {
        DropPlace::After
    } else {
        DropPlace::Into
    }
}

fn sort_default(folders: &mut [Folder], snoozed: &[FolderId]) {
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
#[allow(clippy::too_many_arguments)]
pub fn build_nodes(
    accounts: &[Account],
    folders: &[Folder],
    favorites: &[Folder],
    people: &[FavoritePerson],
    snoozed: &[FolderId],
    prefs: &super::ctx::FolderPrefs,
    edit: Option<&FolderEdit>,
    views: &ViewsInfo,
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
    nodes.extend(views_nodes(views, prefs));
    let mut children: HashMap<(AccountId, Option<FolderId>), Vec<Folder>> = HashMap::new();
    for f in folders {
        if (f.role == FolderRole::Outbox && f.total_count == 0) || left_out(f, folders, prefs) {
            continue;
        }
        children.entry((f.account_id, f.parent_id)).or_default().push(f.clone());
    }
    for ((account, parent), v) in children.iter_mut() {
        sort_siblings(v, snoozed, prefs.folder_order.get(&FolderPrefs::order_key(*account, *parent)));
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
        ("f_order_up", Some(f)) | ("f_order_down", Some(f)) => nudge_folder(ctx, f, cmd == "f_order_up"),
        ("f_hide", Some(f)) => {
            ctx.update_prefs(|p| {
                if !p.hidden_folders.contains(&f.id) {
                    p.hidden_folders.push(f.id);
                }
            });
            let all = s.folders(Some(f.account_id)).unwrap_or_default();
            app.toast(if left_out(f, &all, &ctx.prefs.peek()) {
                format!("{} is hidden. It comes back while it has unread mail; to keep it in the list, right-click the account and choose Show hidden folders.", f.name)
            } else {
                format!("{} will hide once its mail is read, and come back when new mail arrives.", f.name)
            });
        }
        ("f_unhide", Some(f)) => ctx.update_prefs(|p| p.hidden_folders.retain(|id| *id != f.id)),
        ("a_show_hidden", _) => {
            if let MenuTarget::Account(a) = target {
                let shown = hidden_in(*a, &ctx.prefs.peek());
                ctx.update_prefs(|p| p.hidden_folders.retain(|id| !shown.contains(id)));
            }
        }
        ("a_reset_order", _) => {
            if let MenuTarget::Account(a) = target {
                let prefix = format!("{a}:");
                ctx.update_prefs(|p| p.folder_order.retain(|k, _| !k.starts_with(&prefix)));
            }
        }
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
        ("v_open", _) => match target {
            MenuTarget::Packages => actions::show_view(app, ctx, MailView::Packages),
            MenuTarget::Topic(t) => actions::show_view(app, ctx, MailView::Topics(*t)),
            _ => {}
        },
        ("v_hide_topic", _) => {
            if let MenuTarget::Topic(Some(t)) = target {
                let id = t.id().to_string();
                app.update_settings(|s| {
                    if !s.topics_hidden.contains(&id) {
                        s.topics_hidden.push(id.clone());
                    }
                });
                app.toast(format!("{} is hidden. Show it again in Settings > Mail > Topics.", t.label()));
            }
        }
        ("v_package_settings", _) => actions::open_settings_page(app, PageId::Packages),
        ("v_topic_settings", _) => actions::open_settings_page(app, PageId::Topics),
        ("v_collapse", _) | ("v_expand", _) => ctx.update_prefs(|p| p.views_collapsed = cmd == "v_collapse"),
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
fn drop_allowed(drag: &Drag, folder: &Folder, current: Option<FolderId>, all: &AllFolders) -> bool {
    match drag {
        Drag::Messages(_) => {
            Some(folder.id) != current && !matches!(folder.role, FolderRole::Notes | FolderRole::ConversationHistory | FolderRole::Outbox)
        }
        // Well-known folders (Inbox, Drafts…) stay where the server keeps them.
        Drag::Folder(id) => *id != folder.id && all.movable(*id) && !all.is_within(folder.id, *id),
        Drag::Favorite(_) => false,
    }
}

/// Every folder and the Snoozed folders, as the folder pane last read them,
/// shared by its rows. While something is dragged each row asks whether it
/// can take the drop, on every redraw: reading the store for each counted
/// all the mail once per row, seconds in a large mailbox, so the drag
/// stalled and never got going.
#[derive(Clone)]
struct AllFolders {
    folders: Rc<Vec<Folder>>,
    snoozed: Rc<Vec<FolderId>>,
}

impl PartialEq for AllFolders {
    fn eq(&self, other: &Self) -> bool {
        (Rc::ptr_eq(&self.folders, &other.folders) || self.folders == other.folders) && self.snoozed == other.snoozed
    }
}

impl AllFolders {
    fn new(folders: Vec<Folder>, snoozed: Vec<FolderId>) -> Self {
        Self { folders: Rc::new(folders), snoozed: Rc::new(snoozed) }
    }

    /// Read now (an event that acts on the store).
    fn current() -> Self {
        Self::new(store().folders(None).unwrap_or_default(), snoozed_ids())
    }

    fn get(&self, id: FolderId) -> Option<&Folder> {
        self.folders.iter().find(|f| f.id == id)
    }

    /// Your own folder (not Inbox, Drafts and the like): it can move.
    fn movable(&self, id: FolderId) -> bool {
        self.get(id).is_some_and(|f| !menus::is_fixed(f, &self.snoozed))
    }

    /// `folder` is `ancestor` or inside it (as `Store::folder_is_within`;
    /// an unknown folder counts as inside).
    fn is_within(&self, folder: FolderId, ancestor: FolderId) -> bool {
        let mut cur = Some(folder);
        for _ in 0..=self.folders.len() {
            match cur {
                Some(c) if c == ancestor => return true,
                Some(c) => match self.get(c) {
                    Some(f) => cur = f.parent_id,
                    None => return true,
                },
                None => return false,
            }
        }
        false
    }
}

/// Where a drop on a row lands: `place` relative to `anchor`, with the
/// line or highlight drawn on the row of `shown`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Landing {
    place: DropPlace,
    anchor: FolderId,
    shown: FolderId,
}

impl Landing {
    fn on(row: FolderId, place: DropPlace) -> Self {
        Landing { place, anchor: row, shown: row }
    }
}

/// The sibling of `moving` that `row` sits in (or is), walking up the
/// parents; `None` when `row` isn't under one, or is inside `moving`.
fn sibling_ancestor(folders: &[Folder], row: &Folder, moving: &Folder) -> Option<FolderId> {
    let mut cur = row;
    for _ in 0..64 {
        if cur.id == moving.id || cur.account_id != moving.account_id {
            return None;
        }
        if cur.parent_id == moving.parent_id {
            return Some(cur.id);
        }
        cur = folders.iter().find(|f| Some(f.id) == cur.parent_id)?;
    }
    None
}

/// The last row shown under `folder` (itself when closed or childless):
/// where a line meaning "after `folder`" goes.
fn last_visible_descendant(folders: &[Folder], folder: &Folder, prefs: &FolderPrefs, snoozed: &[FolderId]) -> FolderId {
    let mut cur = folder.clone();
    for _ in 0..64 {
        if !cur.is_expanded {
            break;
        }
        let mut kids: Vec<Folder> = folders
            .iter()
            .filter(|f| {
                f.account_id == cur.account_id
                    && f.parent_id == Some(cur.id)
                    && !(f.role == FolderRole::Outbox && f.total_count == 0)
                    && !left_out(f, folders, prefs)
            })
            .cloned()
            .collect();
        sort_siblings(&mut kids, snoozed, prefs.folder_order.get(&FolderPrefs::order_key(cur.account_id, Some(cur.id))));
        match kids.pop() {
            Some(last) => cur = last,
            None => break,
        }
    }
    cur.id
}

/// Where a folder dragged over a row of the account tree lands (pure:
/// `into` says whether the row may take it as a subfolder). Beside a
/// sibling the edges reorder; under an open parent come its subfolders, so
/// its bottom edge means "after it and all of them", drawn below the last.
/// Over a sibling's subfolders, a folder that can't go into them lands
/// after that sibling.
#[allow(clippy::too_many_arguments)]
fn folder_landing(
    folders: &[Folder],
    prefs: &FolderPrefs,
    snoozed: &[FolderId],
    moving: &Folder,
    row: &Folder,
    into: bool,
    y: f64,
    height: f64,
    open: bool,
) -> Option<Landing> {
    if moving.id == row.id {
        return None;
    }
    let after = |s: &Folder| Landing { place: DropPlace::After, anchor: s.id, shown: last_visible_descendant(folders, s, prefs, snoozed) };
    let Some(sib) = sibling_ancestor(folders, row, moving) else {
        return into.then(|| Landing::on(row.id, DropPlace::Into));
    };
    if sib == row.id {
        return Some(match drop_zone(y, height, into) {
            DropPlace::After if open && into => Landing::on(row.id, DropPlace::Into),
            DropPlace::After => after(row),
            p => Landing::on(row.id, p),
        });
    }
    let sib = folders.iter().find(|f| f.id == sib)?;
    let landing = after(sib);
    // The bottom edge of the last row under a sibling means "after it".
    if !into || (landing.shown == row.id && drop_zone(y, height, true) == DropPlace::After) {
        Some(landing)
    } else {
        Some(Landing::on(row.id, DropPlace::Into))
    }
}

/// Where `drag` lands on the row of `row` with the pointer `y` pixels into
/// a row `height` tall, or `None` when it can't be dropped there.
/// Favorites rows only reorder; see [`folder_landing`] for the tree.
#[allow(clippy::too_many_arguments)]
fn place_on(drag: &Drag, row: &Folder, is_fav: bool, current: Option<FolderId>, y: f64, height: f64, open: bool, prefs: &FolderPrefs, all: &AllFolders) -> Option<Landing> {
    match drag {
        Drag::Messages(_) => drop_allowed(drag, row, current, all).then(|| Landing::on(row.id, DropPlace::Into)),
        Drag::Folder(id) | Drag::Favorite(id) if is_fav => (*id != row.id).then(|| Landing::on(row.id, drop_zone(y, height, false))),
        Drag::Favorite(_) => None,
        Drag::Folder(id) => {
            let moving = all.get(*id)?;
            folder_landing(&all.folders, prefs, &all.snoozed, moving, row, drop_allowed(drag, row, current, all), y, height, open)
        }
    }
}

fn drop_on(app: AppState, mut ctx: MailCtx, model: &super::model::ListModel, folder: &Folder, copy: bool) {
    let drag = ctx.drag.peek().clone();
    ctx.drag.set(None);
    ctx.drop_target.set(None);
    let Some(drag) = drag else { return };
    if !drop_allowed(&drag, folder, *ctx.folder.peek(), &AllFolders::current()) {
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
        Drag::Favorite(_) => {}
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
    let settings = app.settings.read().clone();
    let views = views_info(&settings);
    let nodes = build_nodes(&accounts, &folders, &favorites, &people, &snoozed, &prefs, edit.as_ref(), &views);
    let all = AllFolders::new(folders, snoozed.clone());
    let searching = model.read().searching;
    let person = ctx.person.read().as_ref().map(|p| p.contact);
    let view = *ctx.view.read();
    let selected = selected_folder(searching, person.is_some(), view, *ctx.folder.read());
    let selected_view = view.filter(|_| !searching);
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
                    selected_view,
                    focused: active.as_deref() == Some(node.dom_id.as_str()),
                    snoozed: snoozed.clone(),
                    all: all.clone(),
                }
            }
        }
    };
    rsx! {
        nav { class: "ax-mail-folders", style: "width:{width}px", "aria-label": "Navigation pane", "data-ax-region": "true",
            div {
                class: "ax-mail-tree",
                // While dragging, rows (not their parts) get the drag events,
                // so the pointer's height in the row is measured from its top.
                class: if ctx.drag.read().is_some() { "ax-dragging" },
                role: "tree",
                "aria-label": "Folders",
                tabindex: "0",
                "data-ax-default-focus": "true",
                "aria-activedescendant": active.clone().unwrap_or_default(),
                onmounted: move |e| tree_el.set(Some(e.data())),
                onfocus: {
                    let nodes = nodes_rc.clone();
                    move |_| {
                        if ctx.tree_focus.peek().is_none() {
                            ctx.tree_focus.set(start_node(&nodes, selected, selected_view));
                        }
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

/// The node the tree starts on when it takes focus: the view shown (or,
/// with its node collapsed or hidden, the closest one listed: Topics, then
/// Views), else the folder shown.
fn start_node(nodes: &[Node], folder: Option<FolderId>, view: Option<MailView>) -> Option<String> {
    let candidates: Vec<String> = match view {
        Some(MailView::Packages) => vec!["ax-mail-node-packages".into()],
        Some(MailView::Topics(Some(t))) => vec![format!("ax-mail-node-topic-{}", t.id()), "ax-mail-node-topics".into()],
        Some(MailView::Topics(None)) => vec!["ax-mail-node-topics".into()],
        None => return folder.map(|f| format!("ax-mail-node-{f}")),
    };
    candidates.into_iter().chain(["ax-mail-node-views".to_string()]).find(|id| nodes.iter().any(|n| &n.dom_id == id))
}

/// The highlighted folder: none while searching or showing a person's
/// mail, Packages or a topic list (those highlight their own node).
fn selected_folder(searching: bool, person: bool, view: Option<MailView>, folder: Option<FolderId>) -> Option<FolderId> {
    if searching || person || view.is_some() { None } else { folder }
}

/// The Views section's data: two queries, only for the features that are on.
fn views_info(settings: &Settings) -> ViewsInfo {
    let s = store();
    let packages = settings.order_tracking.then(|| {
        let now = chrono::Utc::now();
        let today = chrono::Local::now().date_naive();
        s.orders_summary(now, today, settings.orders_keep_days()).unwrap_or_default()
    });
    let topics = settings.topics_view.then(|| s.topic_counts().unwrap_or_default());
    ViewsInfo { packages, topics, hidden: settings.topics_hidden.clone() }
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
            match typeahead_target(nodes, start, &buf) {
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
        NodeKind::ViewsHeader => ctx.update_prefs(|p| p.views_collapsed = !p.views_collapsed),
        NodeKind::TopicsParent { .. } => ctx.update_prefs(|p| p.topics_collapsed = !p.topics_collapsed),
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
        NodeKind::FavoritesHeader | NodeKind::Account(_) | NodeKind::ViewsHeader => toggle_node(app, ctx, n),
        NodeKind::Packages { .. } | NodeKind::TopicsParent { .. } | NodeKind::Topic(..) => {
            if let Some(v) = view_of(n) {
                actions::show_view(app, ctx, v);
            }
        }
        NodeKind::NewFolder { .. } => {}
    }
}

fn node_menu(n: &Node, prefs: &FolderPrefs) -> Option<(Vec<crate::kit::MenuItem>, MenuTarget)> {
    let snoozed = store().snoozed_folder_ids().unwrap_or_default();
    match &n.kind {
        NodeKind::Folder(f) => Some((menus::folder_menu(f, false, prefs.hidden_folders.contains(&f.id), &snoozed), MenuTarget::Folder(f.id))),
        NodeKind::Favorite(f) => Some((menus::folder_menu(f, true, false, &snoozed), MenuTarget::Favorite(f.id))),
        NodeKind::FavoritesHeader => Some((menus::favorites_header_menu(false), MenuTarget::FavoritesHeader)),
        NodeKind::Account(a) => {
            let hidden = hidden_in(a.id, prefs).len();
            Some((menus::account_menu(hidden), MenuTarget::Account(a.id)))
        }
        NodeKind::Person(p) => Some((menus::person_menu(), MenuTarget::Person(p.contact))),
        NodeKind::Packages { .. } => Some((menus::packages_menu(), MenuTarget::Packages)),
        NodeKind::Topic(t, ..) => Some((menus::topic_menu(false), MenuTarget::Topic(Some(*t)))),
        NodeKind::TopicsParent { .. } => Some((menus::topic_menu(true), MenuTarget::Topic(None))),
        NodeKind::ViewsHeader => Some((menus::views_header_menu(!n.expanded), MenuTarget::ViewsHeader)),
        _ => None,
    }
}

fn open_node_menu(mut ctx: MailCtx, n: &Node, x: f64, y: f64) {
    let menu = node_menu(n, &ctx.prefs.peek());
    if let Some((mut items, target)) = menu {
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
fn FolderNode(
    node: Node,
    selected: Option<FolderId>,
    selected_person: Option<ContactId>,
    selected_view: Option<MailView>,
    focused: bool,
    snoozed: Vec<FolderId>,
    all: AllFolders,
) -> Element {
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
            let all_over = all.clone();
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
                        // Any folder can become a favourite; only your own folders move.
                        let ok = match *ctx.drag.peek() {
                            Some(Drag::Folder(id)) => account_id.is_none() || all_over.movable(id),
                            _ => false,
                        };
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
                                    Some(f) if menus::is_fixed(&f, &snoozed_ids()) => Ok(()),
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
        NodeKind::ViewsHeader => {
            let n_toggle = node.clone();
            let n_menu = node.clone();
            rsx! {
                div {
                    id: "{node.dom_id}",
                    class: "ax-mail-node ax-mail-node-header ax-mail-node-views",
                    class: if focused { "ax-focused" },
                    role: "treeitem",
                    "aria-level": "{level}",
                    "aria-expanded": if node.expanded { "true" } else { "false" },
                    "aria-label": "Views",
                    title: "Found in your mail on this device",
                    style: "padding-left:{indent}px",
                    onclick: move |_| {
                        ctx.tree_focus.set(Some(n_toggle.dom_id.clone()));
                        toggle_node(app, ctx, &n_toggle);
                    },
                    oncontextmenu: move |e: MouseEvent| {
                        e.prevent_default();
                        let c = e.client_coordinates();
                        open_node_menu(ctx, &n_menu, c.x, c.y);
                    },
                    span { class: "ax-mail-node-chevron",
                        Icon { name: if node.expanded { "chevron_down" } else { "chevron_right" }, size: 16 }
                    }
                    span { class: "ax-mail-node-name ax-ellipsis", "Views" }
                }
            }
        }
        NodeKind::Packages { .. } | NodeKind::TopicsParent { .. } | NodeKind::Topic(..) => {
            let Some(view) = view_of(&node) else { return rsx! {} };
            let is_selected = selected_view == Some(view);
            let (icon, name, count, attention) = match node.kind {
                NodeKind::Packages { active, attention } => ("box", "Packages".to_string(), active, attention),
                NodeKind::Topic(t, unread, _) => (t.icon(), t.label().to_string(), unread, 0),
                _ => ("tag", "Topics".to_string(), 0, 0),
            };
            let is_packages = matches!(node.kind, NodeKind::Packages { .. });
            let accessible = match (is_packages, count, attention) {
                (true, 0, _) => "Packages".to_string(),
                (true, n, 0) => format!("Packages, {n} on the way"),
                (true, n, 1) => format!("Packages, {n} on the way, 1 package needs attention"),
                (true, n, a) => format!("Packages, {n} on the way, {a} packages need attention"),
                (false, 0, _) => name.clone(),
                (false, n, _) => format!("{name}, {n} unread"),
            };
            let empty = dimmed(&node.kind);
            let icon_name = if is_selected { crate::kit::filled_icon(icon) } else { icon.to_string() };
            let n_menu = node.clone();
            let n_menu2 = node.clone();
            let n_toggle = node.clone();
            let dom = node.dom_id.clone();
            let expandable = node.expandable;
            let expanded = node.expanded;
            let more_label = format!("More options for {name}");
            rsx! {
                div {
                    id: "{node.dom_id}",
                    class: "ax-mail-node ax-mail-node-view",
                    class: if is_selected { "ax-selected" },
                    class: if focused { "ax-focused" },
                    class: if is_selected || (!is_packages && count > 0) { "ax-bold" },
                    class: if empty { "ax-dim" },
                    role: "treeitem",
                    "aria-level": "{level}",
                    "aria-selected": if is_selected { "true" } else { "false" },
                    "aria-expanded": if expandable { if expanded { "true" } else { "false" } },
                    "aria-label": "{accessible}",
                    title: "{name}",
                    style: "padding-left:{indent}px",
                    onclick: move |_| {
                        ctx.tree_focus.set(Some(dom.clone()));
                        actions::show_view(app, ctx, view);
                    },
                    oncontextmenu: move |e: MouseEvent| {
                        e.prevent_default();
                        let c = e.client_coordinates();
                        open_node_menu(ctx, &n_menu, c.x, c.y);
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
                    span { class: "ax-mail-node-name ax-ellipsis", "{name}" }
                    if count > 0 {
                        span { class: if is_packages { "ax-mail-node-count ax-total" } else { "ax-mail-node-count" }, "aria-hidden": "true", "{count}" }
                    }
                    if attention > 0 {
                        span { class: "ax-mail-node-attention", "aria-hidden": "true", title: "Needs attention" }
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
            // The drop line can belong to another row than the one hovered.
            let is_drop = *ctx.drop_target.read() == Some(f.id) && *ctx.drop_fav.read() == is_fav;
            let place = *ctx.drop_place.read();
            let row_h = if app.settings.read().density == Density::Compact { 32.0 } else { 36.0 };
            let open = node.expandable && node.expanded;
            let can_drop = drag.as_ref().is_some_and(|d| place_on(d, &f, is_fav, selected, row_h / 2.0, row_h, open, &ctx.prefs.peek(), &all).is_some());
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
            let (fid, fid2, fid3) = (f.id, f.id, f.id);
            let f_drop = f.clone();
            let f_over = f.clone();
            let (all_over, all_drop) = (all.clone(), all.clone());
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
                    class: if is_drop && place == DropPlace::Into { "ax-drop" },
                    class: if is_drop && place == DropPlace::Before { "ax-drop-before" },
                    class: if is_drop && place == DropPlace::After { "ax-drop-after" },
                    class: if drag.is_some() && !can_drop { "ax-no-drop" },
                    role: "treeitem",
                    "aria-level": "{level}",
                    "aria-selected": if is_selected { "true" } else { "false" },
                    "aria-expanded": if expandable { if expanded { "true" } else { "false" } },
                    "aria-label": "{accessible}",
                    title: "{f.name}",
                    style: "padding-left:{indent}px",
                    // Every folder can be dragged: to reorder it, into Favorites,
                    // and (your own folders) into another folder.
                    draggable: "true",
                    onclick: move |_| {
                        ctx.tree_focus.set(Some(dom.clone()));
                        actions::go_to_folder(app, ctx, fid);
                    },
                    oncontextmenu: move |e: MouseEvent| {
                        e.prevent_default();
                        let c = e.client_coordinates();
                        open_node_menu(ctx, &n_menu, c.x, c.y);
                    },
                    ondragstart: move |_| {
                        ctx.drop_target.set(None);
                        ctx.drop_place.set(DropPlace::Into);
                        ctx.drag.set(Some(if is_fav { Drag::Favorite(fid2) } else { Drag::Folder(fid2) }));
                    },
                    ondragend: move |_| {
                        ctx.drag.set(None);
                        ctx.drop_target.set(None);
                        ctx.drop_place.set(DropPlace::Into);
                        ctx.drag_hover.set(None);
                    },
                    ondragover: move |e: DragEvent| {
                        e.prevent_default();
                        let Some(drag) = ctx.drag.peek().clone() else { return };
                        let landing = place_on(&drag, &f_over, is_fav, selected, e.element_coordinates().y, row_h, open, &ctx.prefs.peek(), &all_over);
                        match landing {
                            Some(l) => {
                                if *ctx.drop_place.peek() != l.place {
                                    ctx.drop_place.set(l.place);
                                }
                                if *ctx.drop_target.peek() != Some(l.shown) {
                                    ctx.drop_target.set(Some(l.shown));
                                }
                                if *ctx.drop_fav.peek() != is_fav {
                                    ctx.drop_fav.set(is_fav);
                                }
                            }
                            None => {
                                if ctx.drop_target.peek().is_some() {
                                    ctx.drop_target.set(None);
                                }
                            }
                        }
                        // Holding a drag on a collapsed parent for 700 ms opens it,
                        // even one that can't take the drop (not on its edges,
                        // which reorder).
                        if *ctx.drag_hover.peek() != Some(fid3) {
                            ctx.drag_hover.set(Some(fid3));
                            if expandable && !expanded {
                                spawn(async move {
                                    tokio::time::sleep(Duration::from_millis(700)).await;
                                    let reordering = ctx.drop_target.peek().is_some() && *ctx.drop_place.peek() != DropPlace::Into;
                                    if *ctx.drag_hover.peek() == Some(fid3) && !reordering && store().set_folder_expanded(fid3, true).is_ok() {
                                        app.bump_mail();
                                    }
                                });
                            }
                        }
                    },
                    ondragleave: move |_| {
                        // The line may be on another row, so go by the hovered one.
                        if *ctx.drag_hover.peek() == Some(fid3) {
                            ctx.drop_target.set(None);
                            ctx.drag_hover.set(None);
                        }
                    },
                    ondrop: move |e: DragEvent| {
                        e.prevent_default();
                        ctx.drag_hover.set(None);
                        let copy = e.modifiers().ctrl();
                        let drag = ctx.drag.peek().clone();
                        // Worked out again here: a row that refused the drag
                        // must not act on where an earlier row would have put it.
                        let landing = drag.as_ref().and_then(|d| place_on(d, &f_drop, is_fav, selected, e.element_coordinates().y, row_h, open, &ctx.prefs.peek(), &all_drop));
                        let Some(landing) = landing else {
                            ctx.drag.set(None);
                            ctx.drop_target.set(None);
                            ctx.drop_place.set(DropPlace::Into);
                            return;
                        };
                        let after = landing.place == DropPlace::After;
                        match drag {
                            Some(Drag::Folder(id) | Drag::Favorite(id)) if is_fav => {
                                ctx.drag.set(None);
                                ctx.drop_target.set(None);
                                if landing.place != DropPlace::Into && app.check(store().place_favorite(id, f_drop.id, after)).is_some() {
                                    app.bump_mail();
                                }
                            }
                            Some(Drag::Folder(id)) if landing.place != DropPlace::Into => {
                                ctx.drag.set(None);
                                ctx.drop_target.set(None);
                                if let Some(anchor) = store().folder(landing.anchor).ok().flatten() {
                                    place_folder(ctx, id, &anchor, after);
                                }
                            }
                            _ => drop_on(app, ctx, &model.read(), &f_drop, copy),
                        }
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
        let nodes = build_nodes(std::slice::from_ref(&account), &folders, &[], &[], &[13], &prefs, None, &ViewsInfo::default());
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
        // Hidden folders leave the list, with their subfolders.
        let hiding = FolderPrefs { favorites_hidden: true, hidden_folders: vec![2, 11], ..Default::default() };
        let nodes = build_nodes(std::slice::from_ref(&account), &folders, &[], &[], &[13], &hiding, None, &ViewsInfo::default());
        let left: Vec<String> = nodes.iter().filter_map(|n| folder_of(n).map(|f| f.name.clone())).collect();
        assert_eq!(left, vec!["Inbox", "Sent Items", "Snoozed", "zeta"]);
        // Until they have unread mail, theirs or a subfolder's.
        let mut unread = folders.clone();
        unread[4].unread_count = 1;
        let nodes = build_nodes(std::slice::from_ref(&account), &unread, &[], &[], &[13], &hiding, None, &ViewsInfo::default());
        let left: Vec<String> = nodes.iter().filter_map(|n| folder_of(n).map(|f| f.name.clone())).collect();
        assert_eq!(left, vec!["Inbox", "Sent Items", "Snoozed", "Alpha", "Child", "zeta"]);
        // Two Inboxes in Favorites are told apart by their account.
        let people = vec![FavoritePerson { contact: 7, name: "Priya Raman".into(), address: "priya@x.example".into(), unread: 2 }];
        let other = Account { id: 2, email: "me@y.example".into(), ..Default::default() };
        let mut inbox2 = folder(20, FolderRole::Inbox, "Inbox", 0, 0);
        inbox2.account_id = 2;
        let favs = vec![folders[2].clone(), inbox2, folders[6].clone()];
        let shown = FolderPrefs { favorites_hidden: false, collapsed_accounts: vec![1, 2], ..Default::default() };
        let nodes = build_nodes(&[account.clone(), other], &folders, &favs, &people, &[13], &shown, None, &ViewsInfo::default());
        let suffixes: Vec<Option<String>> = nodes.iter().filter(|n| matches!(n.kind, NodeKind::Favorite(_))).map(|n| n.suffix.clone()).collect();
        assert_eq!(suffixes, vec![Some("me@x.example".into()), Some("me@y.example".into()), None]);
        // Favourite people follow the favourite folders.
        assert!(matches!(&nodes[4].kind, NodeKind::Person(p) if p.name == "Priya Raman"));
        assert_eq!(nodes[4].dom_id, "ax-mail-node-person-7");
        // Creating a subfolder shows the field under its parent.
        let edit = FolderEdit::Create { account: 1, parent: Some(10) };
        let nodes = build_nodes(std::slice::from_ref(&account), &folders, &[], &[], &[13], &prefs, Some(&edit), &ViewsInfo::default());
        let zeta = nodes.iter().position(|n| n.dom_id == "ax-mail-node-10").unwrap();
        assert!(matches!(nodes[zeta + 1].kind, NodeKind::NewFolder { parent: Some(10), .. }));
    }

    fn views(packages: bool, topics: bool, hidden: &[&str]) -> ViewsInfo {
        let counts = vec![
            TopicCount { topic: Topic::Travel, total: 4, unread: 2 },
            TopicCount { topic: Topic::Orders, total: 3, unread: 0 },
        ];
        ViewsInfo {
            packages: packages.then_some(OrdersSummary { active: 2, attention: 1 }),
            topics: topics.then_some(counts),
            hidden: hidden.iter().map(|h| h.to_string()).collect(),
        }
    }

    #[test]
    fn views_follow_favorites_and_lead_the_accounts() {
        use super::super::ctx::FolderPrefs;
        let account = Account { id: 1, email: "me@x.example".into(), ..Default::default() };
        let folders = vec![folder(1, FolderRole::Inbox, "Inbox", 0, 0)];
        let prefs = FolderPrefs::default();
        let nodes = build_nodes(std::slice::from_ref(&account), &folders, &folders, &[], &[], &prefs, None, &views(true, true, &[]));
        let kinds: Vec<&str> = nodes
            .iter()
            .map(|n| match n.kind {
                NodeKind::FavoritesHeader => "favorites",
                NodeKind::Favorite(_) => "favorite",
                NodeKind::ViewsHeader => "views",
                NodeKind::Packages { .. } => "packages",
                NodeKind::TopicsParent { .. } => "topics",
                NodeKind::Topic(..) => "topic",
                NodeKind::Account(_) => "account",
                _ => "other",
            })
            .filter(|k| *k != "topic")
            .collect();
        assert_eq!(&kinds[..6], &["favorites", "favorite", "views", "packages", "topics", "account"]);
        let topics: Vec<(Topic, u32, u32)> = nodes.iter().filter_map(|n| if let NodeKind::Topic(t, u, c) = n.kind { Some((t, u, c)) } else { None }).collect();
        assert_eq!(topics.len(), Topic::ALL.len());
        assert_eq!(topics.last().map(|t| t.0), Some(Topic::Other), "Other last");
        assert!(topics.contains(&(Topic::Travel, 2, 4)) && topics.contains(&(Topic::People, 0, 0)));
        assert!(matches!(nodes.iter().find(|n| matches!(n.kind, NodeKind::Packages { .. })).map(|n| &n.kind), Some(NodeKind::Packages { active: 2, attention: 1 })));
        // Both off: no Views section at all.
        let off = build_nodes(std::slice::from_ref(&account), &folders, &[], &[], &[], &prefs, None, &views(false, false, &[]));
        assert!(off.iter().all(|n| view_of(n).is_none() && !matches!(n.kind, NodeKind::ViewsHeader)));
        // One feature off: only the other one.
        let only_topics = build_nodes(std::slice::from_ref(&account), &folders, &[], &[], &[], &prefs, None, &views(false, true, &[]));
        assert!(only_topics.iter().all(|n| !matches!(n.kind, NodeKind::Packages { .. })));
        assert!(only_topics.iter().any(|n| matches!(n.kind, NodeKind::ViewsHeader)));
        // Collapsed: the header only; Topics collapsed: no topics.
        let collapsed = FolderPrefs { views_collapsed: true, ..Default::default() };
        let nodes = build_nodes(std::slice::from_ref(&account), &folders, &[], &[], &[], &collapsed, None, &views(true, true, &[]));
        assert!(nodes.iter().all(|n| view_of(n).is_none()));
        let topics_closed = FolderPrefs { topics_collapsed: true, ..Default::default() };
        let nodes = build_nodes(std::slice::from_ref(&account), &folders, &[], &[], &[], &topics_closed, None, &views(true, true, &[]));
        assert!(nodes.iter().any(|n| matches!(n.kind, NodeKind::TopicsParent { expanded: false })));
        assert!(nodes.iter().all(|n| !matches!(n.kind, NodeKind::Topic(..))));
    }

    #[test]
    fn the_tree_takes_focus_on_the_view_shown() {
        use super::super::ctx::FolderPrefs;
        let account = Account { id: 1, email: "me@x.example".into(), ..Default::default() };
        let folders = vec![folder(1, FolderRole::Inbox, "Inbox", 0, 0)];
        let nodes = build_nodes(std::slice::from_ref(&account), &folders, &[], &[], &[], &FolderPrefs::default(), None, &views(true, true, &["social"]));
        let start = |view| start_node(&nodes, None, view);
        assert_eq!(start(Some(MailView::Packages)).as_deref(), Some("ax-mail-node-packages"));
        assert_eq!(start(Some(MailView::Topics(None))).as_deref(), Some("ax-mail-node-topics"));
        assert_eq!(start(Some(MailView::Topics(Some(Topic::Travel)))).as_deref(), Some("ax-mail-node-topic-travel"));
        assert_eq!(start(Some(MailView::Topics(Some(Topic::Social)))).as_deref(), Some("ax-mail-node-topics"), "hidden: its parent");
        assert_eq!(start_node(&nodes, Some(1), None).as_deref(), Some("ax-mail-node-1"));
        let collapsed = build_nodes(
            std::slice::from_ref(&account),
            &folders,
            &[],
            &[],
            &[],
            &FolderPrefs { views_collapsed: true, ..Default::default() },
            None,
            &views(true, true, &[]),
        );
        assert_eq!(start_node(&collapsed, None, Some(MailView::Packages)).as_deref(), Some("ax-mail-node-views"));
    }

    #[test]
    fn hidden_topics_leave_the_pane_and_typeahead_finds_topics() {
        use super::super::ctx::FolderPrefs;
        let account = Account { id: 1, email: "me@x.example".into(), ..Default::default() };
        let folders = vec![folder(1, FolderRole::Inbox, "Inbox", 0, 0), folder(9, FolderRole::Custom, "Trash talk", 0, 0)];
        let nodes = build_nodes(std::slice::from_ref(&account), &folders, &[], &[], &[], &FolderPrefs::default(), None, &views(true, true, &["social", "newsletters"]));
        let shown: Vec<Topic> = nodes.iter().filter_map(|n| if let NodeKind::Topic(t, ..) = n.kind { Some(t) } else { None }).collect();
        assert!(!shown.contains(&Topic::Social) && !shown.contains(&Topic::Newsletters));
        assert_eq!(shown.len(), Topic::ALL.len() - 2);
        // "Tra" reaches Travel (before the "Trash talk" folder further down).
        let travel = typeahead_target(&nodes, 0, "tra").unwrap();
        assert!(matches!(nodes[travel].kind, NodeKind::Topic(Topic::Travel, ..)));
        let trash = typeahead_target(&nodes, travel + 1, "tra").unwrap();
        assert!(matches!(&nodes[trash].kind, NodeKind::Folder(f) if f.name == "Trash talk"));
        assert!(matches!(nodes[typeahead_target(&nodes, 0, "pack").unwrap()].kind, NodeKind::Packages { .. }));
        assert_eq!(typeahead_target(&nodes, 0, "zzz"), None);
    }

    #[test]
    fn no_folder_is_selected_in_a_view() {
        assert_eq!(selected_folder(false, false, None, Some(3)), Some(3));
        assert_eq!(selected_folder(false, false, Some(MailView::Packages), Some(3)), None);
        assert_eq!(selected_folder(false, false, Some(MailView::Topics(Some(Topic::Travel))), Some(3)), None);
        assert_eq!(selected_folder(false, true, None, Some(3)), None);
        assert_eq!(selected_folder(true, false, None, Some(3)), None);
    }

    #[test]
    fn dragged_order_wins_and_new_folders_follow() {
        let mut v = vec![
            folder(1, FolderRole::Inbox, "Inbox", 0, 0),
            folder(2, FolderRole::Drafts, "Drafts", 0, 0),
            folder(3, FolderRole::Custom, "Zeta", 0, 0),
            folder(4, FolderRole::Custom, "Alpha", 0, 0),
        ];
        sort_siblings(&mut v, &[], None);
        assert_eq!(v.iter().map(|f| f.id).collect::<Vec<_>>(), vec![1, 2, 4, 3]);
        // Zeta dragged above Inbox; Alpha was never placed, so it goes last.
        let custom = reordered(&[1, 2, 3], 3, 1, false);
        assert_eq!(custom, vec![3, 1, 2]);
        sort_siblings(&mut v, &[], Some(&custom));
        assert_eq!(v.iter().map(|f| f.id).collect::<Vec<_>>(), vec![3, 1, 2, 4]);
        assert_eq!(reordered(&[1, 2, 3], 1, 3, true), vec![2, 3, 1]);
        assert_eq!(reordered(&[1, 2, 3], 2, 2, false), vec![1, 2, 3]);
        assert_eq!(FolderPrefs::order_key(5, None), "5:0");
    }

    #[test]
    fn drop_zones_split_the_row() {
        assert_eq!(drop_zone(2.0, 36.0, true), DropPlace::Before);
        assert_eq!(drop_zone(18.0, 36.0, true), DropPlace::Into);
        assert_eq!(drop_zone(34.0, 36.0, true), DropPlace::After);
        assert_eq!(drop_zone(17.0, 36.0, false), DropPlace::Before);
        assert_eq!(drop_zone(19.0, 36.0, false), DropPlace::After);
    }

    #[test]
    fn a_folder_dragged_over_subfolders_lands_after_their_parent() {
        // Inbox (1) holds Jobs (3) and Rent (4), and Rent holds Paid (5); Drafts (2) is beside Inbox.
        let mut jobs = folder(3, FolderRole::Custom, "Jobs", 0, 0);
        jobs.parent_id = Some(1);
        let mut rent = folder(4, FolderRole::Custom, "Rent", 0, 0);
        rent.parent_id = Some(1);
        let mut paid = folder(5, FolderRole::Custom, "Paid", 0, 0);
        paid.parent_id = Some(4);
        let inbox = folder(1, FolderRole::Inbox, "Inbox", 0, 0);
        let drafts = folder(2, FolderRole::Drafts, "Drafts", 0, 0);
        let mut all = vec![inbox.clone(), drafts.clone(), jobs.clone(), rent.clone(), paid.clone()];
        let prefs = FolderPrefs::default();
        let land = |all: &[Folder], row: &Folder, into: bool, y: f64, open: bool| folder_landing(all, &prefs, &[], &drafts, row, into, y, 36.0, open);
        let after_inbox = |shown| Some(Landing { place: DropPlace::After, anchor: 1, shown });

        // Anywhere over Inbox's subfolders: after Inbox, the line under the last row shown.
        assert_eq!(land(&all, &jobs, false, 2.0, false), after_inbox(5));
        assert_eq!(land(&all, &paid, false, 30.0, false), after_inbox(5));
        // Inbox's own bottom half means the same; its top half, before it.
        assert_eq!(land(&all, &inbox, false, 30.0, true), after_inbox(5));
        assert_eq!(land(&all, &inbox, false, 2.0, true), Some(Landing::on(1, DropPlace::Before)));
        // Rent closed: the line goes under Rent.
        all[3].is_expanded = false;
        assert_eq!(land(&all, &jobs, false, 2.0, false), after_inbox(4));
        // A folder that can go into a subfolder does, except on the last row's bottom edge.
        assert_eq!(land(&all, &jobs, true, 18.0, false), Some(Landing::on(3, DropPlace::Into)));
        assert_eq!(land(&all, &all[3].clone(), true, 34.0, false), after_inbox(4));
        // Not onto itself, nor anywhere inside itself.
        assert_eq!(folder_landing(&all, &prefs, &[], &inbox, &inbox, false, 30.0, 36.0, true), None);
        assert_eq!(folder_landing(&all, &prefs, &[], &inbox, &jobs, false, 30.0, 36.0, false), None);
    }

    #[test]
    fn only_a_topic_without_mail_is_dimmed() {
        assert!(dimmed(&NodeKind::Topic(Topic::Travel, 0, 0)));
        assert!(!dimmed(&NodeKind::Topic(Topic::Travel, 0, 3)));
        assert!(!dimmed(&NodeKind::Packages { active: 0, attention: 0 }), "nothing on the way still opens Packages");
        assert!(!dimmed(&NodeKind::TopicsParent { expanded: true }));
    }
}
