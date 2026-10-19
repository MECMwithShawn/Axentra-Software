//! Task list pane (3.4–3.7, 3.11): header with sort and list options,
//! "Add a task", rows, Planned sections, "Completed N", empty states and
//! search results.

use std::collections::HashMap;

use axentra_core::*;
use dioxus::prelude::*;

use super::actions;
use super::add_task::AddTaskBar;
use super::ctx::{Confirm, Drag, SortKey, Target, TodoCtx, View, use_todo};
use super::flagged::FlaggedPane;
use super::menus::{self, MenuAnchor, list_label};
use super::sidebar::RenameInput;
use super::sort::{self, PLANNED_SECTIONS};
use super::task_row::{RowData, TaskRow};
use super::themes::theme_style;
use crate::kit::{EmptyState, Icon, IconButton, Menu};
use crate::state::{AppState, store, use_app};

#[component]
pub fn ListPane() -> Element {
    let app = use_app();
    let ctx = use_todo();
    let search = app.search.read().clone();
    let view = (ctx.view)();
    if search.active && !search.query.trim().is_empty() {
        return rsx! { SearchPane { query: search.query.trim().to_string() } };
    }
    if view == View::Smart(SmartList::Flagged) {
        return rsx! { FlaggedPane {} };
    }
    rsx! { TasksPane { view } }
}

/// Title and empty-state copy (3.16) of a view.
fn view_title(view: View, lists: &[TaskList]) -> String {
    match view {
        View::Smart(SmartList::MyDay) => "My Day".into(),
        View::Smart(SmartList::Important) => "Important".into(),
        View::Smart(SmartList::Planned) => "Planned".into(),
        View::Smart(SmartList::Assigned) => "Assigned to me".into(),
        View::Smart(SmartList::Flagged) => "Flagged email".into(),
        View::Smart(SmartList::Tasks) => "Tasks".into(),
        View::All => "All".into(),
        View::Completed => "Completed".into(),
        View::List(id) => lists.iter().find(|l| l.id == id).map(|l| l.name.clone()).unwrap_or_default(),
    }
}

fn empty_copy(view: View) -> (&'static str, &'static str, &'static str) {
    match view {
        View::Smart(SmartList::MyDay) => ("weather_sunny", "Nothing planned for today", "Add tasks here or open Suggestions. My Day starts fresh every night."),
        View::Smart(SmartList::Important) => ("star", "No important tasks", "Star a task to see it here."),
        View::Smart(SmartList::Planned) => ("calendar_ltr", "Nothing planned", "Tasks with a due date or reminder show up here."),
        View::Smart(SmartList::Assigned) => ("person", "Nothing assigned to you", "Tasks assigned to you show up here."),
        View::Completed => ("checkmark_circle", "No completed tasks", "Tasks you complete show up here."),
        _ => ("task_list_square_ltr", "No tasks yet", "Use Add a task above to get started."),
    }
}

/// Selection change for a row click: plain click opens the task,
/// Ctrl toggles it, Shift selects the range from the anchor.
pub fn click_row(mut ctx: TodoCtx, order: &[TaskId], id: TaskId, shift: bool, ctrl: bool) {
    if ctrl {
        let mut sel = ctx.selected.peek().clone();
        if let Some(i) = sel.iter().position(|x| *x == id) {
            sel.remove(i);
        } else {
            sel.push(id);
        }
        ctx.detail.set(if sel.len() == 1 { sel.first().copied() } else { None });
        ctx.selected.set(sel);
    } else if shift {
        let anchor = ctx.selected.peek().last().copied().unwrap_or(id);
        let pos = |x: TaskId| order.iter().position(|o| *o == x);
        let (Some(a), Some(b)) = (pos(anchor), pos(id)) else {
            ctx.open(id);
            return;
        };
        let mut sel: Vec<TaskId> = order[a.min(b)..=a.max(b)].iter().copied().filter(|x| *x != anchor).collect();
        sel.push(anchor);
        ctx.detail.set(if sel.len() == 1 { Some(id) } else { None });
        ctx.selected.set(sel);
    } else {
        ctx.open(id);
    }
}

/// Arrow / Home / End / Enter / Ctrl+Arrow handling for a task grid.
fn grid_key(app: AppState, mut ctx: TodoCtx, order: &[TaskId], reorderable: bool, e: &KeyboardEvent) {
    let m = e.modifiers();
    let current = ctx.selected.peek().last().copied();
    let pos = current.and_then(|c| order.iter().position(|o| *o == c));
    let target = match e.key() {
        Key::ArrowDown | Key::ArrowUp if m.ctrl() => {
            e.prevent_default();
            if let (Some(id), true) = (current, reorderable)
                && app.check(store().shift_task(id, e.key() == Key::ArrowUp)).unwrap_or(false) {
                    app.bump_tasks();
                    actions::focus_later(&format!("[data-task-row=\"{id}\"]"), 40);
                }
            return;
        }
        Key::ArrowDown => pos.map(|p| (p + 1).min(order.len().saturating_sub(1))).or(if order.is_empty() { None } else { Some(0) }),
        Key::ArrowUp => pos.map(|p| p.saturating_sub(1)).or(if order.is_empty() { None } else { Some(0) }),
        Key::Home if !order.is_empty() => Some(0),
        Key::End if !order.is_empty() => Some(order.len() - 1),
        Key::Enter => {
            if let Some(id) = current {
                e.prevent_default();
                ctx.open(id);
            }
            return;
        }
        _ => return,
    };
    e.prevent_default();
    let Some(i) = target else { return };
    let id = order[i];
    if m.shift() {
        click_row(ctx, order, id, true, false);
    } else {
        ctx.selected.set(vec![id]);
        if ctx.detail.peek().is_some() {
            ctx.detail.set(Some(id));
        }
    }
    actions::focus_row(id, false);
}

/// Run a list header / options command for the open view.
fn run_view_command(app: AppState, mut ctx: TodoCtx, view: View, id: &str) {
    let (group, arg) = id.split_once(':').unwrap_or((id, ""));
    match (group, arg) {
        ("sort", "reverse") => ctx.update_view(view, |v| v.reverse = !v.reverse),
        ("sort", "none") => ctx.update_view(view, |v| {
            v.sort = None;
            v.reverse = false;
        }),
        ("sort", k) => {
            if let Some(k) = SortKey::parse(k) {
                ctx.update_view(view, |v| {
                    v.sort = Some(k);
                    v.reverse = false;
                });
            }
        }
        ("theme", "none") => ctx.update_view(view, |v| v.theme = None),
        ("theme", t) => {
            let t = t.to_string();
            ctx.update_view(view, |v| v.theme = Some(t));
        }
        ("list", "rename") => {
            if let View::List(l) = view {
                ctx.renaming_list.set(Some(l));
            }
        }
        ("list", "hide_completed") => ctx.update_view(view, |v| v.show_completed = Some(false)),
        ("list", "show_completed") => ctx.update_view(view, |v| v.show_completed = Some(true)),
        ("list", "duplicate") => {
            if let View::List(l) = view
                && let Some(copy) = app.check(store().duplicate_task_list(l)) {
                    app.bump_tasks();
                    ctx.go(View::List(copy));
                }
        }
        ("list", "print") => super::print(ctx),
        ("list", "delete") => {
            if let View::List(l) = view {
                ctx.confirm.set(Some(Confirm::DeleteList(l)));
            }
        }
        _ => {}
    }
}

/// Header: title (rename on click for custom lists), My Day date, toolbar
/// and the sort chip.
#[component]
fn ListHeader(view: View, title: String, emoji: Option<String>, custom: bool, sort: Option<SortKey>, reverse: bool, theme: Option<String>, show_completed: bool) -> Element {
    let app = use_app();
    let mut ctx = use_todo();
    let mut renaming = use_signal(|| false);
    let today = (ctx.today)();
    let subtitle = view.is_my_day().then(|| today.format("%A, %B %-d").to_string());
    let suggestions = (ctx.suggestions)();
    let full_name = match &emoji {
        Some(e) => format!("{e} {title}"),
        None => title.clone(),
    };
    let run = move |id: String| run_view_command(app, ctx, view, &id);
    rsx! {
        header { class: "ax-todo-head", class: if subtitle.is_some() { "ax-todo-head-sub" },
            div { class: "ax-todo-head-row",
                if renaming() {
                    RenameInput {
                        initial: full_name.clone(),
                        label: "List name",
                        on_commit: move |name: String| {
                            if let View::List(id) = view
                                && name.trim() != full_name && !name.trim().is_empty() && app.check(store().rename_task_list(id, &name)).is_some() {
                                    app.bump_tasks();
                                }
                            renaming.set(false);
                        },
                        on_cancel: move |_| renaming.set(false),
                    }
                } else {
                    h2 {
                        class: "ax-todo-title",
                        class: if custom { "ax-todo-title-editable" },
                        title: if custom { "Rename list" } else { "" },
                        onclick: move |_| if custom { renaming.set(true) },
                        if let Some(e) = &emoji { span { class: "ax-todo-title-emoji", "{e}" } }
                        "{title}"
                    }
                }
                span { class: "ax-grow" }
                div { class: "ax-todo-head-tools", role: "toolbar", "aria-label": "List actions",
                    if view.is_my_day() {
                        IconButton {
                            icon: if suggestions { "lightbulb_filled" } else { "lightbulb" },
                            title: "Suggestions",
                            toggled: suggestions,
                            onclick: move |_| {
                                let open = !suggestions;
                                ctx.suggestions.set(open);
                                if open {
                                    ctx.detail.set(None);
                                }
                            },
                        }
                    }
                    MenuAnchor { class: "ax-icon-btn", title: "Sort", items: menus::sort_menu(sort), on_select: run,
                        Icon { name: "arrow_sort" }
                    }
                    MenuAnchor {
                        class: "ax-icon-btn",
                        title: "List options",
                        items: menus::list_options_menu(view, sort, theme.as_deref(), show_completed, custom),
                        on_select: run,
                        Icon { name: "more_horizontal" }
                    }
                }
            }
            if let Some(s) = &subtitle {
                div { class: "ax-todo-subtitle", "{s}" }
            }
            if let Some(k) = sort {
                div { class: "ax-todo-sortchip",
                    span { "{k.chip()}" }
                    IconButton {
                        icon: if reverse { "chevron_up" } else { "chevron_down" },
                        title: "Reverse sort order",
                        small: true,
                        onclick: move |_| run_view_command(app, ctx, view, "sort:reverse"),
                    }
                    IconButton { icon: "dismiss", title: "Remove sort order", small: true, onclick: move |_| run_view_command(app, ctx, view, "sort:none") }
                }
            }
        }
    }
}

/// Row event handlers, stable across renders (`use_callback`).
#[derive(Clone, Copy)]
struct RowHandlers {
    on_click: EventHandler<(TaskId, bool, bool)>,
    on_context: EventHandler<(TaskId, f64, f64)>,
    on_drop: EventHandler<TaskId>,
    on_open: EventHandler<TaskId>,
}

/// Hooks for the handlers of a rows block; `order` is the visible order.
fn use_row_handlers(app: AppState, mut ctx: TodoCtx, order: Vec<TaskId>, mut menu: Signal<Option<(f64, f64)>>) -> RowHandlers {
    let on_click = use_callback(move |(id, shift, ctrl): (TaskId, bool, bool)| click_row(ctx, &order, id, shift, ctrl));
    let on_context = use_callback(move |(id, x, y): (TaskId, f64, f64)| {
        if !ctx.selected.peek().contains(&id) {
            ctx.selected.set(vec![id]);
            // An open detail pane follows the row the menu is for.
            if ctx.detail.peek().is_some() {
                ctx.detail.set(Some(id));
            }
        }
        menu.set(Some((x, y)));
    });
    let on_drop = use_callback(move |target: TaskId| {
        if let Some(Drag::Tasks(ids)) = ctx.drag.peek().clone() {
            let s = store();
            let list = s.task(target).ok().flatten().map(|t| t.list_id);
            for id in ids {
                if s.task(id).ok().flatten().map(|t| t.list_id) == list {
                    app.check(s.reorder_task(id, Some(target)));
                }
            }
            app.bump_tasks();
        }
    });
    let on_open = use_callback(move |id: TaskId| ctx.open(id));
    RowHandlers { on_click, on_context, on_drop, on_open }
}

/// Everything a rows block needs to render one task.
struct RowEnv<'a> {
    view: View,
    selected: &'a [TaskId],
    list_names: &'a HashMap<TaskListId, String>,
    colors: &'a HashMap<String, String>,
    today: chrono::NaiveDate,
    include_due: bool,
    h24: bool,
    reorderable: bool,
    focus: Option<TaskId>,
    handlers: RowHandlers,
}

impl RowEnv<'_> {
    fn row(&self, t: &Task) -> RowData {
        let selected = self.selected.contains(&t.id);
        let show_list = match self.view {
            View::List(id) => id != t.list_id,
            _ => true,
        };
        RowData {
            task: t.clone(),
            list_name: show_list.then(|| self.list_names.get(&t.list_id).cloned().unwrap_or_default()),
            in_my_day: actions::in_my_day(t, self.today, self.include_due),
            show_my_day: !self.view.is_my_day(),
            selected,
            tab_stop: self.focus == Some(t.id),
            drop_before: self.reorderable && !t.is_completed,
            reorderable: self.reorderable && !t.is_completed,
            drag_ids: if selected { self.selected.to_vec() } else { vec![t.id] },
            categories: t.categories.iter().map(|c| (c.clone(), self.colors.get(c).cloned().unwrap_or_else(|| "gray".into()))).collect(),
            today: self.today,
            h24: self.h24,
            on_click: self.handlers.on_click,
            on_context: self.handlers.on_context,
            on_drop: self.handlers.on_drop,
            on_open: self.handlers.on_open,
        }
    }
}

#[component]
fn TasksPane(view: View) -> Element {
    let app = use_app();
    let mut ctx = use_todo();
    let _ = app.tasks_rev();
    let mut menu = use_signal(|| None::<(f64, f64)>);
    let prefs = ctx.prefs.read().clone();
    let vp = prefs.view(view);
    let today = (ctx.today)();
    let h24 = app.settings.read().use_24h_time;
    let week_start = app.settings.read().week_start;
    let s = store();
    let lists = s.task_lists().unwrap_or_default();
    let default_list = lists.iter().find(|l| l.is_default).map(|l| l.id);
    let view = if view == View::Smart(SmartList::Tasks) { default_list.map(View::List).unwrap_or(view) } else { view };
    let current = match view {
        View::List(id) => lists.iter().find(|l| l.id == id).cloned(),
        _ => None,
    };
    let custom = current.as_ref().is_some_and(|l| !l.is_default);
    let title = view_title(view, &lists);
    let show_completed = prefs.show_completed(view);
    let include_due = prefs.due_today_in_my_day;
    let tasks: Vec<Task> = match view {
        View::Smart(SmartList::MyDay) => s.my_day_tasks(today, include_due),
        View::Smart(l) => s.smart_list_tasks(l, today, include_due, show_completed),
        View::All => s.all_open_tasks(),
        View::Completed => s.all_completed_tasks(),
        View::List(id) => s.tasks_in_list(id),
    }
    .unwrap_or_default();
    let (mut open, mut done): (Vec<Task>, Vec<Task>) = tasks.into_iter().partition(|t| !t.is_completed);
    if view == View::Completed {
        std::mem::swap(&mut open, &mut done);
    }
    if let Some(k) = vp.sort {
        sort::sort_tasks(&mut open, k, vp.reverse);
    }
    if prefs.starred_on_top && view != View::Completed {
        sort::starred_first(&mut open);
    }
    done.sort_by_key(|t| std::cmp::Reverse(t.completed_at));
    let reorderable = vp.sort.is_none() && !prefs.starred_on_top && matches!(view, View::List(_));
    let completed_open = vp.completed_open;
    let planned = view == View::Smart(SmartList::Planned);

    // Visible order (for arrows and Shift+click).
    let sections: Vec<(&str, &str, Vec<Task>)> = if planned {
        PLANNED_SECTIONS
            .iter()
            .map(|(k, label)| (*k, *label, open.iter().filter(|t| sort::planned_section(t, today, week_start) == *k).cloned().collect::<Vec<_>>()))
            .filter(|(_, _, v)| !v.is_empty())
            .collect()
    } else {
        vec![("", "", open.clone())]
    };
    let mut order: Vec<TaskId> = Vec::new();
    for (k, _, v) in &sections {
        if !vp.collapsed.iter().any(|c| c == k) {
            order.extend(v.iter().map(|t| t.id));
        }
    }
    let completed_visible = show_completed && view != View::Completed && !done.is_empty();
    if completed_visible && completed_open {
        order.extend(done.iter().map(|t| t.id));
    }
    let selected = ctx.selected.read().clone();
    let focus = selected.last().copied().filter(|id| order.contains(id)).or(order.first().copied());

    let list_names: HashMap<TaskListId, String> = lists.iter().map(|l| (l.id, if l.is_default { "Tasks".to_string() } else { l.name.clone() })).collect();
    let colors: HashMap<String, String> = s.categories().unwrap_or_default().into_iter().map(|c| (c.name, c.color)).collect();
    ctx.order.set(order.clone());
    let handlers = use_row_handlers(app, ctx, order.clone(), menu);
    let env = RowEnv {
        view,
        selected: &selected,
        list_names: &list_names,
        colors: &colors,
        today,
        include_due,
        h24,
        reorderable,
        focus,
        handlers,
    };
    let rows_open: Vec<(String, String, bool, Vec<RowData>)> = sections
        .iter()
        .map(|(k, label, v)| {
            let collapsed = vp.collapsed.iter().any(|c| c == k);
            (k.to_string(), label.to_string(), collapsed, v.iter().map(|t| env.row(t)).collect())
        })
        .collect();
    let rows_done: Vec<RowData> = if completed_visible { done.iter().map(|t| env.row(t)).collect() } else { Vec::new() };
    let done_count = done.len();
    let is_empty = open.is_empty() && !completed_visible;
    let (empty_icon, empty_title, empty_body) = empty_copy(view);
    let order_keys = order.clone();
    let menu_tasks: Vec<Task> = if menu().is_some() { selected.iter().filter_map(|id| s.task(*id).ok().flatten()).collect() } else { Vec::new() };
    let theme = vp.theme.clone();
    let drag_end_over = ctx.drop_target.read().as_deref() == Some("task:end");
    let dragging_tasks = matches!(*ctx.drag.read(), Some(Drag::Tasks(_)));

    rsx! {
        section {
            class: "ax-pane ax-todo-list",
            class: if theme.is_some() { "ax-todo-themed" },
            style: "{theme_style(theme.as_deref())}",
            "aria-label": "{title}",
            ListHeader {
                view,
                title: title.clone(),
                emoji: current.as_ref().and_then(|l| l.emoji.clone()),
                custom,
                sort: vp.sort,
                reverse: vp.reverse,
                theme: theme.clone(),
                show_completed,
            }
            div { class: "ax-todo-scroll",
                div { class: "ax-todo-content",
                    if view.has_add_bar() {
                        AddTaskBar { key: "{view.key()}", view, list_id: current.as_ref().map(|l| l.id), list_name: title.clone() }
                    }
                    div {
                        class: "ax-todo-grid",
                        role: "grid",
                        "aria-label": "{title}",
                        "aria-multiselectable": "true",
                        onkeydown: move |e: KeyboardEvent| grid_key(app, ctx, &order_keys, reorderable, &e),
                        for (key, label, collapsed, rows) in rows_open {
                            if !label.is_empty() {
                                SectionHeader { key: "head-{key}", view, section: key.clone(), label, count: rows.len(), collapsed }
                            }
                            if !collapsed {
                                for p in rows {
                                    TaskRow { key: "{p.task.id}", row: p }
                                }
                            }
                        }
                        if reorderable && dragging_tasks {
                            div {
                                class: "ax-todo-end-drop",
                                class: if drag_end_over { "ax-todo-drop-line" },
                                ondragover: move |_| if !drag_end_over { ctx.drop_target.set(Some("task:end".into())) },
                                ondrop: move |_| {
                                    if let Some(Drag::Tasks(ids)) = ctx.drag.peek().clone() {
                                        for id in ids {
                                            app.check(store().reorder_task(id, None));
                                        }
                                        app.bump_tasks();
                                    }
                                    ctx.drag.set(None);
                                    ctx.drop_target.set(None);
                                },
                            }
                        }
                        if completed_visible {
                            button {
                                r#type: "button",
                                class: "ax-todo-completed-head",
                                "aria-expanded": if completed_open { "true" } else { "false" },
                                onclick: move |_| ctx.update_view(view, |v| v.completed_open = !v.completed_open),
                                Icon { name: "chevron_down", size: 12, class: if completed_open { "ax-todo-chevron" } else { "ax-todo-chevron ax-collapsed" } }
                                span { "Completed" }
                                span { class: "ax-todo-completed-count", "{done_count}" }
                            }
                            if completed_open {
                                for p in rows_done {
                                    TaskRow { key: "{p.task.id}", row: p }
                                }
                            }
                        }
                    }
                    if is_empty {
                        EmptyState { icon: empty_icon, title: empty_title, subtitle: empty_body }
                    }
                }
            }
            if let Some((x, y)) = menu() {
                Menu {
                    items: menus::task_context_menu(&menu_tasks, &lists, today, include_due),
                    x,
                    y,
                    on_select: move |cmd: String| {
                        let ids = ctx.selected.peek().clone();
                        actions::run(app, ctx, Target::Tasks(ids), &cmd);
                    },
                    on_close: move |_| {
                        menu.set(None);
                        actions::refocus_selection(ctx);
                    },
                }
            }
        }
    }
}

/// Collapsible Planned section heading with its count.
#[component]
fn SectionHeader(view: View, section: String, label: String, count: usize, collapsed: bool) -> Element {
    let ctx = use_todo();
    rsx! {
        button {
            r#type: "button",
            class: "ax-todo-section-head",
            "aria-expanded": if collapsed { "false" } else { "true" },
            onclick: move |_| {
                let key = section.clone();
                ctx.update_view(view, move |v| {
                    if let Some(i) = v.collapsed.iter().position(|c| *c == key) {
                        v.collapsed.remove(i);
                    } else {
                        v.collapsed.push(key);
                    }
                });
            },
            Icon { name: "chevron_down", size: 12, class: if collapsed { "ax-todo-chevron ax-collapsed" } else { "ax-todo-chevron" } }
            span { "{label}" }
            span { class: "ax-todo-completed-count", "{count}" }
        }
    }
}

/// Search results across all lists (Ctrl+E / header search in To Do).
#[component]
fn SearchPane(query: String) -> Element {
    let app = use_app();
    let mut ctx = use_todo();
    let _ = app.tasks_rev();
    let mut menu = use_signal(|| None::<(f64, f64)>);
    let s = store();
    let today = (ctx.today)();
    let h24 = app.settings.read().use_24h_time;
    let include_due = ctx.prefs.read().due_today_in_my_day;
    let results = s.search_tasks(&query).unwrap_or_default();
    let lists = s.task_lists().unwrap_or_default();
    let list_names: HashMap<TaskListId, String> = lists.iter().map(|l| (l.id, if l.is_default { "Tasks".to_string() } else { list_label(l) })).collect();
    let colors: HashMap<String, String> = s.categories().unwrap_or_default().into_iter().map(|c| (c.name, c.color)).collect();
    let order: Vec<TaskId> = results.iter().map(|t| t.id).collect();
    let selected = ctx.selected.read().clone();
    let focus = selected.last().copied().filter(|id| order.contains(id)).or(order.first().copied());
    ctx.order.set(order.clone());
    let handlers = use_row_handlers(app, ctx, order.clone(), menu);
    let env = RowEnv {
        view: View::Completed,
        selected: &selected,
        list_names: &list_names,
        colors: &colors,
        today,
        include_due,
        h24,
        reorderable: false,
        focus,
        handlers,
    };
    let rows: Vec<RowData> = results.iter().map(|t| RowData { show_my_day: true, ..env.row(t) }).collect();
    let menu_tasks: Vec<Task> = if menu().is_some() { selected.iter().filter_map(|id| s.task(*id).ok().flatten()).collect() } else { Vec::new() };
    let order_keys = order.clone();
    rsx! {
        section { class: "ax-pane ax-todo-list", "aria-label": "Search results",
            header { class: "ax-todo-head",
                div { class: "ax-todo-head-row",
                    h2 { class: "ax-todo-title ax-ellipsis", "Searching for \u{201c}{query}\u{201d}" }
                }
            }
            div { class: "ax-todo-scroll",
                div { class: "ax-todo-content",
                    if rows.is_empty() {
                        EmptyState { icon: "search", title: "No results for \u{201c}{query}\u{201d}" }
                    } else {
                        div {
                            class: "ax-todo-grid",
                            role: "grid",
                            "aria-label": "Search results",
                            onkeydown: move |e: KeyboardEvent| grid_key(app, ctx, &order_keys, false, &e),
                            for p in rows {
                                TaskRow { key: "{p.task.id}", row: p }
                            }
                        }
                    }
                }
            }
            if let Some((x, y)) = menu() {
                Menu {
                    items: menus::task_context_menu(&menu_tasks, &lists, today, include_due),
                    x,
                    y,
                    on_select: move |cmd: String| {
                        let ids = ctx.selected.peek().clone();
                        actions::run(app, ctx, Target::Tasks(ids), &cmd);
                    },
                    on_close: move |_| {
                        menu.set(None);
                        actions::refocus_selection(ctx);
                    },
                }
            }
        }
    }
}
