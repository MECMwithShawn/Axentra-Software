//! Mail module state shared by the folder pane, message list, reading pane
//! and ribbon. Provided once at the module root (`MailModule`); nothing here
//! lives in `AppState`.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use axentra_core::*;
use chrono::{Local, NaiveDate};
use dioxus::prelude::*;

use crate::kit::MenuItem;

use super::model::ListModel;

/// One selectable row of the message list: a conversation (grouped view) or
/// a single message (flat view, search results, expanded conversation children).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RowKey {
    Conv(String),
    Msg(MessageId),
}

impl RowKey {
    /// DOM id used for `aria-activedescendant`.
    pub fn dom_id(&self) -> String {
        match self {
            RowKey::Msg(id) => format!("ax-mail-row-m{id}"),
            RowKey::Conv(c) => format!("ax-mail-row-c{:x}", fnv(c)),
        }
    }
}

fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Message list sort field ("By Date", "By From", ...).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SortField {
    #[default]
    Date,
    From,
    Category,
    Size,
    Importance,
    Subject,
    Flag,
}

impl SortField {
    pub const ALL: [SortField; 7] = [
        SortField::Date,
        SortField::From,
        SortField::Category,
        SortField::Size,
        SortField::Importance,
        SortField::Subject,
        SortField::Flag,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SortField::Date => "Date",
            SortField::From => "From",
            SortField::Category => "Category",
            SortField::Size => "Size",
            SortField::Importance => "Importance",
            SortField::Subject => "Subject",
            SortField::Flag => "Flag status",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            SortField::Date => "date",
            SortField::From => "from",
            SortField::Category => "category",
            SortField::Size => "size",
            SortField::Importance => "importance",
            SortField::Subject => "subject",
            SortField::Flag => "flag",
        }
    }

    pub fn parse(s: &str) -> Option<SortField> {
        Self::ALL.into_iter().find(|f| f.id() == s)
    }

    /// The natural direction: newest, largest, high, flagged first; names A to Z.
    pub fn default_descending(self) -> bool {
        matches!(self, SortField::Date | SortField::Size | SortField::Importance | SortField::Flag)
    }

    /// Labels for the two directions: (descending, ascending).
    pub fn order_labels(self) -> (&'static str, &'static str) {
        match self {
            SortField::Date => ("Newest on top", "Oldest on top"),
            SortField::Size => ("Largest on top", "Smallest on top"),
            SortField::Importance => ("High on top", "Low on top"),
            SortField::Flag => ("Flagged on top", "Not flagged on top"),
            SortField::From | SortField::Category | SortField::Subject => ("Z to A", "A to Z"),
        }
    }
}

/// Filter and sort for one folder (kept per folder for the session).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListView {
    pub filter: MessageFilter,
    pub sort: SortField,
    pub descending: bool,
}

impl Default for ListView {
    fn default() -> Self {
        ListView { filter: MessageFilter::All, sort: SortField::Date, descending: true }
    }
}

pub fn filter_label(f: MessageFilter) -> &'static str {
    match f {
        MessageFilter::All => "All",
        MessageFilter::Unread => "Unread",
        MessageFilter::Flagged => "Flagged",
        MessageFilter::ToMe => "To me",
        MessageFilter::HasFiles => "Has files",
        MessageFilter::Mentions => "@ Mentions me",
        MessageFilter::Pinned => "Pinned",
    }
}

/// A favourite person whose mail the list shows (Favorites, §2.3).
#[derive(Clone, Debug, PartialEq)]
pub struct PersonView {
    pub contact: ContactId,
    pub name: String,
    pub address: String,
}

/// What the reading pane shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Open {
    /// The message to show (and expand, in a conversation).
    pub message: MessageId,
    /// Show the whole conversation around it.
    pub conversation: Option<String>,
    /// Messages that count as read once this is displayed (a conversation
    /// row: its messages in the current folder).
    pub mark: Vec<MessageId>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SearchScope {
    #[default]
    AllFolders,
    CurrentFolder,
}

/// What is being dragged.
#[derive(Clone, Debug, PartialEq)]
pub enum Drag {
    Messages(Vec<MessageId>),
    Folder(FolderId),
}

/// Inline folder editing in the folder pane.
#[derive(Clone, Debug, PartialEq)]
pub enum FolderEdit {
    Rename(FolderId),
    Create { account: AccountId, parent: Option<FolderId> },
}

/// Who a context menu acts on.
#[derive(Clone, Debug, PartialEq)]
pub enum MenuTarget {
    Messages(Vec<MessageId>),
    Folder(FolderId),
    Favorite(FolderId),
    FavoritesHeader,
    Account(AccountId),
    /// A favourite person under Favorites.
    Person(ContactId),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ContextMenu {
    pub x: f64,
    pub y: f64,
    pub items: Vec<MenuItem>,
    pub target: MenuTarget,
}

/// Modal dialogs the Mail module opens.
#[derive(Clone, Debug, PartialEq)]
pub enum MailDialog {
    DeleteFolder(FolderId),
    /// Permanently delete every item (Deleted Items / Junk Email "Empty folder").
    EmptyFolder(FolderId),
    /// Move every item to Deleted Items ("Delete all").
    DeleteAll(FolderId),
    PermanentDelete(Vec<MessageId>),
    MoveTo { ids: Vec<MessageId>, copy: bool },
    MoveFolder(FolderId),
    Sweep(MessageId),
    Block(MessageId),
    Report { ids: Vec<MessageId>, phishing: bool },
    Ignore(String),
    Details(MessageId),
    NewCategory(Vec<MessageId>),
    ManageCategories,
    SnoozeUntil(Vec<MessageId>),
    FlagCustom(Vec<MessageId>),
    /// Archive when the account has no Archive folder yet.
    SetUpArchive(Vec<MessageId>),
    Shortcuts,
    Help,
}

/// What the list looked like in a folder, restored when the user comes back
/// to it within the session (docs/spec/mail.md §3.14).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FolderMemory {
    pub selection: Vec<RowKey>,
    pub anchor: Option<RowKey>,
    pub focus: Option<RowKey>,
    pub open: Option<Open>,
    pub tab: InferenceClass,
    pub scroll: f64,
}

/// Folder pane layout choices, persisted in the store's key/value table.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct FolderPrefs {
    pub favorites_pinned: bool,
    pub favorites_hidden: bool,
    pub favorites_collapsed: bool,
    pub collapsed_accounts: Vec<AccountId>,
    /// Height of the message list (percent) with the reading pane at the bottom.
    pub list_height_pct: Option<u32>,
}

impl FolderPrefs {
    const KEY: &'static str = "mail.folder_pane";

    pub fn load() -> Self {
        crate::state::store().kv_get(Self::KEY).ok().flatten().and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        if let Ok(j) = serde_json::to_string(self) {
            let _ = crate::state::store().kv_set(Self::KEY, &j);
        }
    }
}

#[derive(Clone, Copy)]
pub struct MailCtx {
    pub prefs: Signal<FolderPrefs>,
    /// Folder shown in the list (`None` only before the first folder loads).
    pub folder: Signal<Option<FolderId>>,
    /// Focused Inbox pivot.
    pub tab: Signal<InferenceClass>,
    pub views: Signal<HashMap<FolderId, ListView>>,
    /// Selected rows, in selection order.
    pub selection: Signal<Vec<RowKey>>,
    /// Shift+click / Shift+arrow anchor.
    pub anchor: Signal<Option<RowKey>>,
    /// Keyboard focus in the list (roving, shown with a focus ring).
    pub focus: Signal<Option<RowKey>>,
    pub open: Signal<Option<Open>>,
    /// "Select" mode: checkboxes on every row.
    pub select_mode: Signal<bool>,
    /// Conversation expanded in the list (only one at a time).
    pub expanded: Signal<Option<String>>,
    pub collapsed_groups: Signal<HashSet<String>>,
    pub drag: Signal<Option<Drag>>,
    pub drop_target: Signal<Option<FolderId>>,
    pub dialog: Signal<Option<MailDialog>>,
    pub menu: Signal<Option<ContextMenu>>,
    /// Reading pane hidden: the open message covers the list.
    pub full_view: Signal<bool>,
    pub search_scope: Signal<SearchScope>,
    /// Messages whose remote content the user allowed ("Show content").
    pub allowed_remote: Signal<HashSet<MessageId>>,
    /// "Switch background": light message bodies while the app is dark.
    pub light_body: Signal<bool>,
    /// Reading pane zoom in percent.
    pub zoom: Signal<u32>,
    /// The OS prefers dark (for the System theme), reported by the webview.
    pub system_dark: Signal<bool>,
    pub folder_edit: Signal<Option<FolderEdit>>,
    /// Keyboard focus in the folder tree (DOM id of the node).
    pub tree_focus: Signal<Option<String>>,
    /// Today's date, refreshed at midnight (date groups and time labels).
    pub today: Signal<NaiveDate>,
    /// Width of the reading pane, for body height estimates.
    pub reading_width: Signal<f64>,
    /// Width inside the read window ("Open in new window"; it is resizable),
    /// for its body height estimates.
    pub read_window_width: Signal<f64>,
    /// Set by text inputs inside the module so the module's own shortcut
    /// handler ignores the same key event (non-reactive; stale after 100 ms).
    pub typing: CopyValue<Option<std::time::Instant>>,
    /// Scroll requests for the list: (serial, row key).
    pub scroll_to: Signal<Option<(u64, RowKey)>>,
    /// Focus requests for the list / tree / reading pane.
    pub focus_list: Signal<u64>,
    pub focus_tree: Signal<u64>,
    pub focus_reading: Signal<u64>,
    /// "Open in new window": the conversation shown in the read window.
    pub popout: Signal<Option<Open>>,
    /// A favourite person's mail shown instead of the folder.
    pub person: Signal<Option<PersonView>>,
    /// When the user last looked at Other ("New messages in Other").
    pub other_seen: Signal<Option<chrono::DateTime<chrono::Utc>>>,
    /// Per-folder selection and scroll position for this session.
    pub memory: CopyValue<HashMap<FolderId, FolderMemory>>,
    /// The list's current scroll offset (written on scroll, non-reactive).
    pub list_scroll: CopyValue<f64>,
}

impl MailCtx {
    pub fn new() -> Self {
        MailCtx {
            prefs: Signal::new(FolderPrefs::load()),
            folder: Signal::new(None),
            tab: Signal::new(InferenceClass::Focused),
            views: Signal::new(HashMap::new()),
            selection: Signal::new(Vec::new()),
            anchor: Signal::new(None),
            focus: Signal::new(None),
            open: Signal::new(None),
            select_mode: Signal::new(false),
            expanded: Signal::new(None),
            collapsed_groups: Signal::new(HashSet::new()),
            drag: Signal::new(None),
            drop_target: Signal::new(None),
            dialog: Signal::new(None),
            menu: Signal::new(None),
            full_view: Signal::new(false),
            search_scope: Signal::new(SearchScope::AllFolders),
            allowed_remote: Signal::new(HashSet::new()),
            light_body: Signal::new(false),
            zoom: Signal::new(100),
            system_dark: Signal::new(false),
            folder_edit: Signal::new(None),
            tree_focus: Signal::new(None),
            today: Signal::new(Local::now().date_naive()),
            reading_width: Signal::new(640.0),
            read_window_width: Signal::new(898.0),
            typing: CopyValue::new(None),
            scroll_to: Signal::new(None),
            focus_list: Signal::new(0),
            focus_tree: Signal::new(0),
            focus_reading: Signal::new(0),
            popout: Signal::new(None),
            person: Signal::new(None),
            other_seen: Signal::new(None),
            memory: CopyValue::new(HashMap::new()),
            list_scroll: CopyValue::new(0.0),
        }
    }

    /// Change and persist the folder pane preferences.
    pub fn update_prefs(mut self, f: impl FnOnce(&mut FolderPrefs)) {
        let mut p = self.prefs.peek().clone();
        f(&mut p);
        p.save();
        self.prefs.set(p);
    }

    pub fn view(&self, folder: Option<FolderId>) -> ListView {
        folder.and_then(|f| self.views.read().get(&f).copied()).unwrap_or_default()
    }

    pub fn set_view(mut self, folder: Option<FolderId>, f: impl FnOnce(&mut ListView)) {
        let Some(folder) = folder else { return };
        let mut views = self.views.write();
        let v = views.entry(folder).or_default();
        f(v);
    }

    /// Mark that a text field (or the compose form) handled this key event.
    pub fn mark_typing(mut self) {
        self.typing.set(Some(std::time::Instant::now()));
    }

    /// Mark a key event from a field that calls `kit::keys::stop_typing_keys`:
    /// only the keys that still bubble (chords, Escape, F6) need marking.
    pub fn mark_typing_for(self, e: &KeyboardEvent) {
        let m = e.modifiers();
        if m.ctrl() || m.meta() || m.alt() || e.key() == Key::Escape || e.key() == Key::F6 {
            self.mark_typing();
        }
    }

    /// True (once) when a text field already handled this key event.
    pub fn take_typing(mut self) -> bool {
        let t = *self.typing.peek();
        self.typing.set(None);
        t.is_some_and(|at| at.elapsed() < std::time::Duration::from_millis(100))
    }

    pub fn clear_selection(mut self) {
        self.selection.set(Vec::new());
        self.anchor.set(None);
        self.open.set(None);
    }

    /// Scroll the list so `key` is visible.
    pub fn reveal(mut self, key: RowKey) {
        let serial = self.scroll_to.peek().as_ref().map(|(s, _)| s + 1).unwrap_or(1);
        self.scroll_to.set(Some((serial, key)));
    }

    pub fn request_list_focus(mut self) {
        self.focus_list += 1;
    }

    pub fn request_tree_focus(mut self) {
        self.focus_tree += 1;
    }
}

/// The list model, computed once per change at the module root.
#[derive(Clone, Copy)]
pub struct ListMemo(pub Memo<Rc<ListModel>>);

pub fn use_mail() -> MailCtx {
    use_context::<MailCtx>()
}

pub fn use_model() -> Memo<Rc<ListModel>> {
    use_context::<ListMemo>().0
}
