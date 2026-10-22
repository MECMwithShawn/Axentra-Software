//! Module-local state of To Do, shared through a context provided at the
//! module root: the open view, selection, detail pane, pickers, drag state
//! and the persisted To Do preferences.

use std::collections::HashMap;

use axentra_core::*;
use chrono::{DateTime, NaiveDate, Utc};
use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

use crate::state::store;

/// kv key of the persisted [`Prefs`].
const PREFS_KEY: &str = "todo.prefs";

/// What the task list pane shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum View {
    Smart(SmartList),
    /// Optional smart list: every incomplete task.
    All,
    /// Optional smart list: every completed task.
    Completed,
    List(TaskListId),
}

impl View {
    /// Key for per-view preferences.
    pub fn key(self) -> String {
        match self {
            View::Smart(s) => format!("{s:?}"),
            View::All => "All".into(),
            View::Completed => "Completed".into(),
            View::List(id) => format!("list:{id}"),
        }
    }

    pub fn is_my_day(self) -> bool {
        self == View::Smart(SmartList::MyDay)
    }

    /// Smart lists without an "Add a task" bar.
    pub fn has_add_bar(self) -> bool {
        !matches!(self, View::Smart(SmartList::Assigned | SmartList::Flagged) | View::Completed)
    }

    /// Whether completed tasks show by default (Important / Planned /
    /// Assigned hide them until "Show completed tasks").
    pub fn completed_by_default(self) -> bool {
        !matches!(self, View::Smart(SmartList::Important | SmartList::Planned | SmartList::Assigned))
    }
}

/// "Sort by" options, in menu order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SortKey {
    Importance,
    DueDate,
    MyDay,
    Alphabetical,
    Created,
}

impl SortKey {
    pub const ALL: [SortKey; 5] = [SortKey::Importance, SortKey::DueDate, SortKey::MyDay, SortKey::Alphabetical, SortKey::Created];

    pub fn id(self) -> &'static str {
        match self {
            SortKey::Importance => "importance",
            SortKey::DueDate => "due",
            SortKey::MyDay => "myday",
            SortKey::Alphabetical => "alpha",
            SortKey::Created => "created",
        }
    }

    pub fn parse(s: &str) -> Option<SortKey> {
        Self::ALL.into_iter().find(|k| k.id() == s)
    }

    pub fn label(self) -> &'static str {
        match self {
            SortKey::Importance => "Importance",
            SortKey::DueDate => "Due date",
            SortKey::MyDay => "Added to My Day",
            SortKey::Alphabetical => "Alphabetically",
            SortKey::Created => "Creation date",
        }
    }

    /// Text of the sort chip under the list title.
    pub fn chip(self) -> &'static str {
        match self {
            SortKey::Importance => "Sorted by importance",
            SortKey::DueDate => "Sorted by due date",
            SortKey::MyDay => "Sorted by added to My Day",
            SortKey::Alphabetical => "Sorted alphabetically",
            SortKey::Created => "Sorted by creation date",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            SortKey::Importance => "star",
            SortKey::DueDate => "calendar_ltr",
            SortKey::MyDay => "weather_sunny",
            SortKey::Alphabetical => "text_sort_ascending",
            SortKey::Created => "calendar_add",
        }
    }
}

/// Per-view preferences (sort, completed section, theme, collapsed groups).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewPrefs {
    pub sort: Option<SortKey>,
    pub reverse: bool,
    /// `None` = the view's default (see [`View::completed_by_default`]).
    pub show_completed: Option<bool>,
    /// "Completed N" expanded (collapsed by default).
    pub completed_open: bool,
    /// List theme id (see `themes.rs`).
    pub theme: Option<String>,
    /// Collapsed Planned sections.
    pub collapsed: Vec<String>,
}

/// To Do settings (gear) plus remembered UI state, persisted in the kv store.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub confirm_delete: bool,
    pub add_on_top: bool,
    pub starred_on_top: bool,
    pub completion_sound: bool,
    pub reminders: bool,
    pub show_important: bool,
    pub show_planned: bool,
    pub show_all: bool,
    pub show_completed: bool,
    pub show_assigned: bool,
    pub auto_hide_empty: bool,
    pub due_today_in_my_day: bool,
    pub flagged_email: bool,
    pub sidebar_open: bool,
    /// Widths of the resizable panes (3.2), in px.
    pub sidebar_width: u32,
    pub detail_width: u32,
    pub last_view: Option<View>,
    pub collapsed_groups: Vec<String>,
    pub views: HashMap<String, ViewPrefs>,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            confirm_delete: true,
            add_on_top: false,
            starred_on_top: false,
            completion_sound: true,
            reminders: true,
            show_important: true,
            show_planned: true,
            show_all: false,
            show_completed: false,
            show_assigned: true,
            auto_hide_empty: false,
            due_today_in_my_day: true,
            flagged_email: true,
            sidebar_open: true,
            sidebar_width: PaneEdge::Sidebar.default_width(),
            detail_width: PaneEdge::Detail.default_width(),
            last_view: None,
            collapsed_groups: Vec::new(),
            views: HashMap::new(),
        }
    }
}

impl Prefs {
    pub fn load() -> Prefs {
        store().kv_get(PREFS_KEY).ok().flatten().and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default()
    }

    /// Width of a resizable pane, clamped to its range.
    pub fn width(&self, edge: PaneEdge) -> u32 {
        let w = match edge {
            PaneEdge::Sidebar => self.sidebar_width,
            PaneEdge::Detail => self.detail_width,
        };
        edge.clamp(w as f64)
    }

    pub fn view(&self, view: View) -> ViewPrefs {
        self.views.get(&view.key()).cloned().unwrap_or_default()
    }

    pub fn show_completed(&self, view: View) -> bool {
        self.view(view).show_completed.unwrap_or_else(|| view.completed_by_default())
    }

    /// Whether a hideable smart list is turned on in settings.
    pub fn smart_visible(&self, view: View) -> bool {
        match view {
            View::Smart(SmartList::Important) => self.show_important,
            View::Smart(SmartList::Planned) => self.show_planned,
            View::Smart(SmartList::Assigned) => self.show_assigned,
            View::Smart(SmartList::Flagged) => self.flagged_email,
            View::All => self.show_all,
            View::Completed => self.show_completed,
            _ => true,
        }
    }
}

/// Values picked in the "Add a task" toolbar before the task exists.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Draft {
    pub due: Option<NaiveDate>,
    pub reminder: Option<DateTime<Utc>>,
    pub repeat: Option<Recurrence>,
}

/// What a due / reminder / repeat / category command applies to.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Tasks(Vec<TaskId>),
    /// The "Add a task" draft.
    NewTask,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PickerKind {
    Due,
    Reminder,
    CustomRepeat,
}

/// An open date picker / custom-repeat dialog.
#[derive(Clone, Debug, PartialEq)]
pub struct Picker {
    pub kind: PickerKind,
    pub target: Target,
    pub x: f64,
    pub y: f64,
}

/// A pending confirmation dialog.
#[derive(Clone, Debug, PartialEq)]
pub enum Confirm {
    DeleteTasks(Vec<TaskId>),
    DeleteList(TaskListId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TodoDialog {
    Settings,
    Shortcuts,
}

/// A resizable pane edge (3.2: sidebar 220-400 px, detail 300-520 px).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneEdge {
    Sidebar,
    Detail,
}

impl PaneEdge {
    pub fn default_width(self) -> u32 {
        match self {
            PaneEdge::Sidebar => 280,
            PaneEdge::Detail => 360,
        }
    }

    pub fn range(self) -> (u32, u32) {
        match self {
            PaneEdge::Sidebar => (220, 400),
            PaneEdge::Detail => (300, 520),
        }
    }

    pub fn clamp(self, w: f64) -> u32 {
        let (min, max) = self.range();
        (w.round().max(0.0) as u32).clamp(min, max)
    }
}

/// A pane edge being dragged: the pointer and width where it started and
/// the live width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Resize {
    pub edge: PaneEdge,
    pub start_x: f64,
    pub start_width: u32,
    pub width: u32,
}

/// Something being dragged inside To Do.
#[derive(Clone, Debug, PartialEq)]
pub enum Drag {
    Tasks(Vec<TaskId>),
    List(TaskListId),
}

#[derive(Clone, Copy)]
pub struct TodoCtx {
    pub view: Signal<View>,
    pub prefs: Signal<Prefs>,
    /// Selected tasks; the last one is the focus / range anchor.
    pub selected: Signal<Vec<TaskId>>,
    /// Task shown in the detail pane.
    pub detail: Signal<Option<TaskId>>,
    pub suggestions: Signal<bool>,
    pub renaming_list: Signal<Option<TaskListId>>,
    pub renaming_group: Signal<Option<String>>,
    pub draft: Signal<Draft>,
    pub picker: Signal<Option<Picker>>,
    pub confirm: Signal<Option<Confirm>>,
    pub dialog: Signal<Option<TodoDialog>>,
    pub drag: Signal<Option<Drag>>,
    /// Drop target under the pointer while dragging (sidebar key or `task:<id>`).
    pub drop_target: Signal<Option<String>>,
    /// A pane edge being resized.
    pub resize: Signal<Option<Resize>>,
    /// Tasks in their completion animation (completed in the store after 600 ms).
    pub completing: Signal<Vec<TaskId>>,
    /// Polite live-region text ("Task added", "Task completed").
    pub announce: Signal<String>,
    /// Last pointer position, used to place pickers opened from menus.
    pub pointer: Signal<(f64, f64)>,
    pub reduced_motion: Signal<bool>,
    pub printing: Signal<bool>,
    /// Visible row order of the task list as last rendered (not reactive),
    /// used to pick the next row after a delete.
    pub order: CopyValue<Vec<TaskId>>,
    /// Local date the module last rendered for (midnight refresh).
    pub today: Signal<NaiveDate>,
    /// The module root, which owns timers that must outlive a row.
    pub root: ScopeId,
}

pub fn use_todo() -> TodoCtx {
    use_context::<TodoCtx>()
}

/// Create the context; call once at the module root.
pub fn provide_todo() -> TodoCtx {
    use_context_provider(|| {
        let prefs = Prefs::load();
        // Reopen the last list used (My Day on first run).
        let view = prefs
            .last_view
            .filter(|v| match v {
                View::List(id) => store().task_list(*id).ok().flatten().is_some(),
                _ => true,
            })
            .unwrap_or(View::Smart(SmartList::MyDay));
        new_ctx(view, prefs)
    })
}

fn new_ctx(view: View, prefs: Prefs) -> TodoCtx {
    TodoCtx {
        view: Signal::new(view),
        prefs: Signal::new(prefs),
        selected: Signal::new(Vec::new()),
        detail: Signal::new(None),
        suggestions: Signal::new(false),
        renaming_list: Signal::new(None),
        renaming_group: Signal::new(None),
        draft: Signal::new(Draft::default()),
        picker: Signal::new(None),
        confirm: Signal::new(None),
        dialog: Signal::new(None),
        drag: Signal::new(None),
        drop_target: Signal::new(None),
        resize: Signal::new(None),
        completing: Signal::new(Vec::new()),
        announce: Signal::new(String::new()),
        pointer: Signal::new((0.0, 0.0)),
        reduced_motion: Signal::new(false),
        printing: Signal::new(false),
        order: CopyValue::new(Vec::new()),
        today: Signal::new(super::dates::today()),
        root: dioxus::core::current_scope_id(),
    }
}

impl TodoCtx {
    /// Change preferences and persist them.
    pub fn update_prefs(mut self, f: impl FnOnce(&mut Prefs)) {
        let mut p = self.prefs.read().clone();
        f(&mut p);
        if let Ok(json) = serde_json::to_string(&p)
            && let Err(e) = store().kv_set(PREFS_KEY, &json) {
                tracing::warn!("couldn't save To Do preferences: {e}");
            }
        self.prefs.set(p);
    }

    pub fn update_view(self, view: View, f: impl FnOnce(&mut ViewPrefs)) {
        self.update_prefs(|p| f(p.views.entry(view.key()).or_default()));
    }

    /// Open a view: clears the selection and the detail pane.
    pub fn go(mut self, view: View) {
        if *self.view.peek() != view {
            self.selected.set(Vec::new());
            self.detail.set(None);
            self.draft.set(Draft::default());
        }
        if !view.is_my_day() {
            self.suggestions.set(false);
        }
        self.view.set(view);
        if self.prefs.peek().last_view != Some(view) {
            self.update_prefs(|p| p.last_view = Some(view));
        }
    }

    /// Select one task and show it in the detail pane.
    pub fn open(mut self, id: TaskId) {
        self.selected.set(vec![id]);
        self.detail.set(Some(id));
        self.suggestions.set(false);
    }

    /// Current width of a pane (live while its edge is dragged).
    pub fn pane_width(&self, edge: PaneEdge) -> u32 {
        match *self.resize.read() {
            Some(r) if r.edge == edge => r.width,
            _ => self.prefs.read().width(edge),
        }
    }

    /// Set and persist a pane width.
    pub fn set_pane_width(self, edge: PaneEdge, width: u32) {
        let w = edge.clamp(width as f64);
        self.update_prefs(|p| match edge {
            PaneEdge::Sidebar => p.sidebar_width = w,
            PaneEdge::Detail => p.detail_width = w,
        });
    }

    /// Tasks the selection-based commands act on.
    pub fn targets(&self) -> Vec<TaskId> {
        self.selected.read().clone()
    }

    /// A picker, dialog or inline rename owns the focus.
    pub fn overlay_open(&self) -> bool {
        self.picker.peek().is_some()
            || self.confirm.peek().is_some()
            || self.dialog.peek().is_some()
            || self.renaming_list.peek().is_some()
            || self.renaming_group.peek().is_some()
    }

    /// Run a future owned by the module root, so it survives the component
    /// that started it (a row that disappears, a closed menu). Leaving To Do
    /// drops it (see [`crate::kit::spawn_in`]).
    pub fn spawn(self, fut: impl std::future::Future<Output = ()> + 'static) {
        crate::kit::spawn_in(self.root, self.order, fut);
    }

    /// Announce through the polite live region (re-announces identical text).
    pub fn say(mut self, text: impl Into<String>) {
        let text = text.into();
        self.announce.set(String::new());
        self.spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            self.announce.set(text);
        });
    }
}
