//! Module-local state, shared through a context provided by `PeopleModule`.
//! View preferences persist in the store's key/value table (`people.ui.*`).

use axentra_core::*;
use dioxus::prelude::*;

use super::actions::Selection;
use super::view::{Item, ListModel, Node, SortBy};
use crate::kit::MenuItem;
use crate::state::store;

/// Default and limits of the resizable panes (docs/spec/people-todo.md 2.2).
pub const NAV_W: (u32, u32, u32) = (240, 200, 360);
pub const LIST_W: (u32, u32, u32) = (360, 280, 640);
/// List pane width from which the multi-column layout is used.
pub const MULTI_COLUMN_W: u32 = 640;
/// Index heading row height.
pub const HEADING_H: f64 = 28.0;
/// Rows rendered above and below the viewport.
pub const OVERSCAN_ROWS: f64 = 10.0;
/// Members from which a list shows the size warning (2.9).
pub const LIST_WARN_MEMBERS: usize = 500;
/// Days of mail scanned for "Suggested" (2.12).
pub const SUGGESTED_DAYS: i64 = 180;

/// Dialogs the module can show (one at a time).
#[derive(Clone, Debug, PartialEq)]
pub enum PeopleDialog {
    /// New contact form, pre-filled.
    NewContact(Box<Contact>),
    /// Contact list editor: `None` creates a list seeded with the members.
    EditList(Option<ContactListId>, Vec<EmailAddress>),
    ConfirmDelete { contacts: Vec<ContactId>, lists: Vec<ContactListId> },
    ConfirmDeleteFolder(String),
    NewCategory(Vec<ContactId>),
    Import,
    Export,
    Shortcuts,
    /// The inline editor has changes and the user picked another item.
    DiscardEdit(Option<Item>),
}

/// What a context menu acts on.
#[derive(Clone, Debug, PartialEq)]
pub enum MenuTarget {
    Selection,
    YourContacts,
    Folder(String),
    Category(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ContextMenu {
    pub x: f64,
    pub y: f64,
    pub items: Vec<MenuItem>,
    pub target: MenuTarget,
}

#[derive(Clone, Copy)]
pub struct PeopleCtx {
    pub node: Signal<Node>,
    pub sort: Signal<SortBy>,
    /// "Display contacts by: Last name".
    pub last_first: Signal<bool>,
    pub nav_open: Signal<bool>,
    pub contacts_expanded: Signal<bool>,
    pub suggested_on: Signal<bool>,
    pub tip_dismissed: Signal<bool>,
    /// Text in the list pane's search box.
    pub filter: Signal<String>,
    pub selection: Signal<Vec<Item>>,
    /// The selected contacts and lists as stored (follows `selection`).
    pub selected: Memo<std::rc::Rc<Selection>>,
    /// Start of a Shift range.
    pub anchor: Signal<Option<Item>>,
    /// The row holding `tabindex=0`.
    pub focus: Signal<Option<Item>>,
    /// Bumped to move DOM focus (true) or just scroll (false) to `focus`.
    pub focus_req: Signal<(u64, bool)>,
    /// Contact shown in the inline editor of the details pane.
    pub editing: Signal<Option<ContactId>>,
    /// Inline editor has unsaved changes.
    pub edit_dirty: Signal<bool>,
    pub dialog: Signal<Option<PeopleDialog>>,
    pub menu: Signal<Option<ContextMenu>>,
    pub nav_w: Signal<u32>,
    pub list_w: Signal<u32>,
    /// Inline "New folder" box in the navigation pane.
    pub new_folder: Signal<bool>,
    /// Folder being renamed inline.
    pub renaming: Signal<Option<String>>,
    /// Items being dragged onto the navigation pane.
    pub drag: Signal<Vec<Item>>,
    pub model: Memo<std::rc::Rc<ListModel>>,
}

/// Contact row height for a density (64 px Comfortable, 48 px Compact, 2.5.4).
pub fn row_height(density: Density) -> f64 {
    match density {
        Density::Roomy => 68.0,
        Density::Cozy => 64.0,
        Density::Compact => 48.0,
    }
}

/// Row height of the multi-column layout.
pub const COLUMN_ROW_H: f64 = 40.0;

const KEY_NODE: &str = "people.ui.node";
const KEY_SORT: &str = "people.ui.sort";
const KEY_LAST_FIRST: &str = "people.ui.last_first";
const KEY_NAV_OPEN: &str = "people.ui.nav_open";
const KEY_SUGGESTED: &str = "people.ui.suggested";
const KEY_TIP: &str = "people.ui.suggested_tip_dismissed";
const KEY_NAV_W: &str = "people.ui.nav_w";
const KEY_LIST_W: &str = "people.ui.list_w";

/// Text the list is filtered by: the list pane's box, else a submitted
/// search from the header box (`app.search`, see ARCHITECTURE.md).
pub fn effective_query(local: &str, search: &crate::state::SearchState) -> String {
    if !local.trim().is_empty() {
        local.to_string()
    } else if search.active {
        search.query.clone()
    } else {
        String::new()
    }
}

pub fn pref(key: &str) -> Option<String> {
    store().kv_get(key).ok().flatten()
}

fn pref_bool(key: &str, default: bool) -> bool {
    pref(key).map(|v| v == "1").unwrap_or(default)
}

fn pref_u32(key: &str, (default, min, max): (u32, u32, u32)) -> u32 {
    pref(key).and_then(|v| v.parse().ok()).unwrap_or(default).clamp(min, max)
}

fn save(key: &str, value: &str) {
    if let Err(e) = store().kv_set(key, value) {
        tracing::warn!("couldn't save {key}: {e}");
    }
}

/// Initial values read from the store.
pub struct Prefs {
    pub node: Node,
    pub sort: SortBy,
    pub last_first: bool,
    pub nav_open: bool,
    pub suggested_on: bool,
    pub tip_dismissed: bool,
    pub nav_w: u32,
    pub list_w: u32,
}

impl Prefs {
    pub fn load() -> Self {
        let suggested_on = pref_bool(KEY_SUGGESTED, false);
        let mut node = pref(KEY_NODE).map(|k| Node::from_key(&k)).unwrap_or(Node::AllContacts);
        if node == Node::Suggested && !suggested_on {
            node = Node::AllContacts;
        }
        Self {
            node,
            sort: pref(KEY_SORT).map(|k| SortBy::from_key(&k)).unwrap_or_default(),
            last_first: pref_bool(KEY_LAST_FIRST, false),
            nav_open: pref_bool(KEY_NAV_OPEN, true),
            suggested_on,
            tip_dismissed: pref_bool(KEY_TIP, false),
            nav_w: pref_u32(KEY_NAV_W, NAV_W),
            list_w: pref_u32(KEY_LIST_W, LIST_W),
        }
    }
}

impl PeopleCtx {
    pub fn set_node(mut self, node: Node) {
        save(KEY_NODE, &node.key());
        if *self.node.peek() != node {
            self.node.set(node);
            self.selection.set(Vec::new());
            self.anchor.set(None);
            self.focus.set(None);
        }
    }

    pub fn set_sort(mut self, sort: SortBy) {
        save(KEY_SORT, sort.key());
        self.sort.set(sort);
    }

    pub fn set_last_first(mut self, v: bool) {
        save(KEY_LAST_FIRST, if v { "1" } else { "0" });
        self.last_first.set(v);
    }

    pub fn toggle_nav(mut self) {
        let open = !*self.nav_open.peek();
        save(KEY_NAV_OPEN, if open { "1" } else { "0" });
        self.nav_open.set(open);
    }

    pub fn set_suggested(mut self, on: bool) {
        save(KEY_SUGGESTED, if on { "1" } else { "0" });
        self.suggested_on.set(on);
        if on {
            self.dismiss_tip();
        } else if *self.node.peek() == Node::Suggested {
            self.set_node(Node::AllContacts);
        }
    }

    pub fn dismiss_tip(mut self) {
        save(KEY_TIP, "1");
        self.tip_dismissed.set(true);
    }

    pub fn set_nav_w(mut self, w: u32) {
        let w = w.clamp(NAV_W.1, NAV_W.2);
        save(KEY_NAV_W, &w.to_string());
        self.nav_w.set(w);
    }

    pub fn set_list_w(mut self, w: u32) {
        let w = w.clamp(LIST_W.1, LIST_W.2);
        save(KEY_LIST_W, &w.to_string());
        self.list_w.set(w);
    }

    /// Select exactly one item, make it the focus row and scroll to it.
    pub fn select_only(mut self, item: Item, move_focus: bool) {
        self.selection.set(vec![item.clone()]);
        self.anchor.set(Some(item.clone()));
        self.focus.set(Some(item));
        self.request_focus(move_focus);
    }

    pub fn request_focus(mut self, move_focus: bool) {
        let n = self.focus_req.peek().0 + 1;
        self.focus_req.set((n, move_focus));
    }

    pub fn clear_selection(mut self) {
        self.selection.set(Vec::new());
    }

    /// Selected contact ids, in list order of selection.
    pub fn selected_contacts(&self) -> Vec<ContactId> {
        self.selection.read().iter().filter_map(|i| if let Item::Contact(id) = i { Some(*id) } else { None }).collect()
    }

    pub fn selected_lists(&self) -> Vec<ContactListId> {
        self.selection.read().iter().filter_map(|i| if let Item::List(id) = i { Some(*id) } else { None }).collect()
    }

    pub fn open_dialog(mut self, d: PeopleDialog) {
        self.menu.set(None);
        self.dialog.set(Some(d));
    }

    /// Close the dialog and return focus to where it was (spec 5.21).
    pub fn close_dialog(mut self) {
        self.dialog.set(None);
        super::io::restore_focus();
    }

    /// Change the selection, asking first when the inline editor has changes.
    pub fn select_guarded(mut self, item: Option<Item>, move_focus: bool) -> bool {
        if self.editing.peek().is_some() && *self.edit_dirty.peek() {
            self.dialog.set(Some(PeopleDialog::DiscardEdit(item)));
            return false;
        }
        self.editing.set(None);
        match item {
            Some(i) => self.select_only(i, move_focus),
            None => self.clear_selection(),
        }
        true
    }
}
