//! To Do module (docs/spec/people-todo.md section 3): sidebar of smart and
//! custom lists, the task list, and the detail / Suggestions pane.
//!
//! Module-local state lives in [`ctx::TodoCtx`], provided here. Store
//! reads subscribe through `app.tasks_rev()` (and `mail_rev()` for flagged
//! email); every mutation bumps it.

mod actions;
mod add_task;
mod ctx;
mod dates;
mod detail;
mod dialogs;
mod flagged;
mod list_view;
mod menus;
mod picker;
mod reminders;
mod ribbon;
mod sidebar;
mod sort;
mod splitter;
mod suggestions;
mod task_row;
mod themes;

use axentra_core::*;
use dioxus::prelude::*;

use crate::kit::keys;
use crate::shell::ribbon::Ribbon;
use crate::state::{NavRequest, SearchState, store, sync, use_app};
use ctx::{PaneEdge, TodoCtx, View};

pub use reminders::TaskReminders;

/// Installed once per page: puts the task-ids MIME type on task drags (for
/// drops outside To Do), keeps the browser's own context menu off rows that
/// have one, and reports `prefers-reduced-motion`.
const BOOT_JS: &str = r#"
if (!window.__axTodoBoot) {
  window.__axTodoBoot = true;
  document.addEventListener("dragstart", (e) => {
    if (!e.dataTransfer || !e.target.closest) return;
    const row = e.target.closest("[data-task-ids]");
    if (row) {
      e.dataTransfer.setData("application/x-axentra-task-ids", "[" + row.dataset.taskIds + "]");
      e.dataTransfer.effectAllowed = "move";
    } else if (e.target.closest(".ax-todo [draggable='true']")) {
      // Lists and steps: some engines only start a drag that carries data.
      e.dataTransfer.setData("application/x-axentra-todo", "");
      e.dataTransfer.effectAllowed = "move";
    }
  }, true);
  document.addEventListener("contextmenu", (e) => {
    if (e.target.closest && e.target.closest(".ax-todo [data-ctx]")) e.preventDefault();
  }, true);
}
return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
"#;

/// Print the open list: a print stylesheet shows only the list pane.
pub(crate) fn print(mut ctx: TodoCtx) {
    ctx.printing.set(true);
    ctx.spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let _ = document::eval("window.print(); return true;").join::<bool>().await;
        ctx.printing.set(false);
    });
}

/// Module-scope shortcuts (keyboard spec section 8).
fn module_key(app: crate::state::AppState, mut ctx: TodoCtx, e: &KeyboardEvent) {
    let chord = keys::chord(e);
    let repeat = e.is_auto_repeating();
    let targets = ctx.targets();
    match chord.as_str() {
        "Ctrl+N" if !repeat => ribbon::command(app, ctx, "new_task"),
        "Ctrl+L" if !repeat => sidebar::new_list(app, ctx, None),
        "Ctrl+T" => actions::toggle_my_day(app, ctx, &targets),
        "Ctrl+D" | "Insert" => {
            let all_done = !targets.is_empty() && targets.iter().all(|id| store().task(*id).ok().flatten().is_some_and(|t| t.is_completed));
            actions::set_completed(app, ctx, &targets, !all_done);
            if let [id] = targets.as_slice()
                && !all_done
            {
                actions::advance_from(ctx, *id);
            }
        }
        "Ctrl+F" | "Ctrl+E" if !repeat => {
            document::eval("const i = document.querySelector('.ax-search input'); if (i) { i.focus(); i.select(); }");
        }
        "F9" if !repeat => match sync() {
            Some(s) => {
                s.sync_now();
                app.toast("Syncing\u{2026}");
            }
            None => {
                app.toast("No account is set up to sync tasks");
            }
        },
        "Delete" => actions::request_delete(app, ctx, targets),
        "F2" => {
            if let Some(&id) = targets.last() {
                ctx.open(id);
                actions::focus_later("#ax-todo-detail-title", 60);
            } else if let View::List(id) = *ctx.view.peek()
                && store().task_list(id).ok().flatten().is_some_and(|l| !l.is_default) {
                    ctx.renaming_list.set(Some(id));
                }
        }
        "Escape" => {
            if ctx.picker.peek().is_some() {
                ctx.picker.set(None);
            } else if ctx.detail.peek().is_some() {
                detail::close_detail(ctx);
            } else if *ctx.suggestions.peek() {
                ctx.suggestions.set(false);
            } else if targets.len() > 1 {
                let keep = targets.last().copied();
                ctx.selected.set(keep.into_iter().collect());
            } else if app.search.peek().active {
                let mut search = app.search;
                let serial = search.peek().serial + 1;
                search.set(SearchState { serial, ..Default::default() });
            } else {
                return;
            }
        }
        _ => return,
    }
    e.prevent_default();
    e.stop_propagation();
}

#[component]
pub fn TasksModule() -> Element {
    let app = use_app();
    let mut ctx = ctx::provide_todo();
    let _ = app.tasks_rev();
    // A completion still animating finishes when To Do goes (its timer goes
    // with the module).
    use_drop(move || {
        let Ok(ids) = ctx.completing.try_peek().map(|c| c.clone()) else { return };
        if actions::finish_completions(&ids) {
            app.bump_tasks();
        }
    });

    // Cross-module navigation: open the requested list (or the task in its
    // list, with the detail pane) and clear the request.
    use_effect(move || {
        let req = app.nav.read().clone();
        let view = match req {
            Some(NavRequest::Task(id)) => {
                let mut nav = app.nav;
                nav.set(None);
                match store().task(id).ok().flatten() {
                    Some(t) => {
                        ctx.go(View::List(t.list_id));
                        ctx.open(id);
                        actions::focus_later("#ax-todo-detail-title", 60);
                    }
                    None => {
                        app.error("This task no longer exists.");
                    }
                }
                return;
            }
            Some(NavRequest::TaskList(id)) => View::List(id),
            Some(NavRequest::SmartList(SmartList::Tasks)) => match store().default_task_list(None).ok().flatten() {
                Some(l) => View::List(l.id),
                None => return,
            },
            Some(NavRequest::SmartList(s)) => View::Smart(s),
            _ => return,
        };
        ctx.go(view);
        let mut nav = app.nav;
        nav.set(None);
    });

    // Keys typed in a title, note or step just before a click that opened
    // another task, which the page still had for that field after it went.
    let on_late = use_callback(move |(rest, value): (String, String)| detail::late_typed(app, &rest, &value));
    crate::kit::dom::use_late_keys(detail::LATE_PREFIX, on_late);

    // Boot script, then a once-a-minute check for the local date changing
    // (My Day resets at midnight; also covers suspend and time-zone changes).
    use_future(move || async move {
        let reduced = document::eval(BOOT_JS).join::<bool>().await.unwrap_or(false);
        ctx.reduced_motion.set(reduced);
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            let today = dates::today();
            if today != *ctx.today.peek() {
                ctx.today.set(today);
                app.bump_tasks();
            }
        }
    });

    let prefs_sidebar = ctx.prefs.read().sidebar_open;
    let view = (ctx.view)();
    let suggestions = (ctx.suggestions)() && view.is_my_day();
    let detail = (ctx.detail)().filter(|id| store().task(*id).ok().flatten().is_some());
    let h24 = app.settings.read().use_24h_time;
    let printing = (ctx.printing)();
    let announce = (ctx.announce)();
    let resizing = ctx.resize.read().is_some();
    let widths = format!(
        "--ax-todo-sidebar-w:{}px;--ax-todo-detail-w:{}px",
        ctx.pane_width(PaneEdge::Sidebar),
        ctx.pane_width(PaneEdge::Detail)
    );
    rsx! {
        div {
            class: "ax-todo",
            class: if printing { "ax-todo-printing" },
            class: if resizing { "ax-todo-resizing" },
            style: "{widths}",
            tabindex: "-1",
            onmounted: move |e| async move { let _ = e.set_focus(true).await; },
            onkeydown: move |e: KeyboardEvent| module_key(app, ctx, &e),
            onmousedown: move |e: MouseEvent| {
                let c = e.client_coordinates();
                ctx.pointer.set((c.x, c.y));
            },
            Ribbon {
                tabs: ribbon::tabs(ctx, h24),
                on_command: move |id: String| ribbon::command(app, ctx, &id),
                on_toggle_nav: move |_| ctx.update_prefs(|p| p.sidebar_open = !p.sidebar_open),
                nav_open: prefs_sidebar,
            }
            div { class: "ax-panes ax-todo-panes",
                sidebar::Sidebar {}
                list_view::ListPane {}
                if suggestions {
                    suggestions::SuggestionsPane {}
                } else if let Some(id) = detail {
                    detail::DetailPane { key: "{id}", id }
                }
            }
            div { class: "ax-visually-hidden", role: "status", "aria-live": "polite", "{announce}" }
            splitter::ResizeLayer {}
            TaskReminders {}
            picker::PickerHost {}
            dialogs::ConfirmHost {}
            dialogs::DialogHost {}
        }
    }
}
