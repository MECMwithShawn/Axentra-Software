//! To Do ribbon: Home and Help tabs (shell spec section 5). Selection
//! commands are disabled, with the reason as tooltip, until a task is
//! selected.

use axentra_core::*;
use dioxus::prelude::*;

use super::actions;
use super::add_task::focus_add;
use super::ctx::{SortKey, Target, TodoCtx, TodoDialog, View};
use super::menus;
use super::sidebar::{new_group, new_list};
use crate::kit::MenuItem;
use crate::shell::ribbon::{Command, RibbonTab};
use crate::state::{AppState, store};

const NEED_SELECTION: &str = "Select a task first";

/// Build the tabs for the current selection and view.
pub fn tabs(ctx: TodoCtx, h24: bool) -> Vec<RibbonTab> {
    let selected: Vec<Task> = ctx.selected.read().iter().filter_map(|id| store().task(*id).ok().flatten()).collect();
    let view = (ctx.view)();
    let today = (ctx.today)();
    let include_due = ctx.prefs.read().due_today_in_my_day;
    let sort = ctx.prefs.read().view(view).sort;
    let none = selected.is_empty();
    let all = |f: &dyn Fn(&Task) -> bool| !none && selected.iter().all(f);
    let done = all(&|t| t.is_completed);
    let important = all(&|t| t.is_important);
    let in_day = all(&|t| actions::in_my_day(t, today, include_due));
    let lists = store().task_lists().unwrap_or_default();
    let current_list = selected.first().map(|t| t.list_id).filter(|l| selected.iter().all(|t| t.list_id == *l));
    let chosen: Vec<String> = selected.first().map(|t| t.categories.clone()).unwrap_or_default();
    let categories = store().categories().unwrap_or_default();
    let flagged = view == View::Smart(SmartList::Flagged);
    let sel = |c: Command| if none { c.disabled(true).tooltip(NEED_SELECTION) } else { c };

    let mut sort_items = menus::sort_menu(sort);
    if sort.is_some() {
        sort_items.extend([
            MenuItem::divider(),
            MenuItem::new("sort:reverse", "Reverse sort order").icon("arrow_sort"),
            MenuItem::new("sort:none", "Remove sort order").icon("dismiss"),
        ]);
    }
    let home = vec![
        vec![Command::new("new_task", "New task", "add")
            .primary()
            .tooltip("New task (Ctrl+N)")
            .split(vec![
                MenuItem::new("new:task", "Task").icon("checkmark_circle").shortcut("Ctrl+N"),
                MenuItem::new("new:list", "List").icon("text_bullet_list_ltr").shortcut("Ctrl+L"),
                MenuItem::new("new:group", "Group").icon("tab_add"),
            ])],
        vec![sel(Command::new("delete", "Delete", "delete").tooltip("Delete (Delete)"))],
        vec![
            sel(if done {
                Command::new("done:off", "Complete", "checkmark_circle").toggled(true).tooltip("Mark as not completed (Ctrl+D)")
            } else {
                Command::new("done:on", "Complete", "checkmark_circle").tooltip("Mark as complete (Ctrl+D)")
            }),
            sel(if important {
                Command::new("imp:off", "Important", "star_filled").toggled(true).tooltip("Remove importance")
            } else {
                Command::new("imp:on", "Important", "star").tooltip("Mark as important")
            }),
            sel(if in_day {
                Command::new("myday:remove", "My Day", "weather_sunny_filled").toggled(true).tooltip("Remove from My Day (Ctrl+T)")
            } else {
                Command::new("myday:add", "My Day", "weather_sunny").tooltip("Add to My Day (Ctrl+T)")
            }),
        ],
        vec![
            sel(Command::new("due", "Due date", "calendar_ltr").menu(menus::due_menu(today, selected.iter().any(|t| t.due.is_some())))),
            sel(Command::new("rem", "Remind me", "alert").menu(menus::reminder_menu(today, selected.iter().any(|t| t.reminder.is_some()), h24))),
            sel(Command::new("rep", "Repeat", "arrow_repeat_all").menu(menus::repeat_menu(selected.iter().any(|t| t.recurrence.is_some())))),
            sel(Command::new("cat", "Categorize", "tag").menu(menus::category_menu(&categories, &chosen))),
        ],
        vec![sel(Command::new("move", "Move to", "folder_arrow_right").menu(menus::move_menu(&lists, current_list)))],
        vec![
            if flagged {
                Command::new("sort", "Sort", "arrow_sort").disabled(true).tooltip("Flagged email is sorted by date")
            } else {
                Command::new("sort", "Sort", "arrow_sort").menu(sort_items)
            },
            if flagged {
                Command::new("print", "Print", "print").disabled(true).tooltip("Open a task list to print it")
            } else {
                Command::new("print", "Print", "print").tooltip("Print list")
            },
        ],
        vec![Command::new("settings", "To Do settings", "settings").icon_only()],
    ];
    let help = vec![vec![Command::new("shortcuts", "Keyboard shortcuts", "keyboard")]];
    vec![RibbonTab::new("home", "Home", home), RibbonTab::new("help", "Help", help)]
}

/// Handle a ribbon command or menu item id.
pub fn command(app: AppState, mut ctx: TodoCtx, id: &str) {
    let view = *ctx.view.peek();
    match id.split_once(':').unwrap_or((id, "")) {
        ("new_task", _) | ("new", "task") => {
            if (!view.has_add_bar() || view == View::Smart(SmartList::Flagged))
                && let Some(l) = store().default_task_list(None).ok().flatten() {
                    ctx.go(View::List(l.id));
                }
            focus_add();
        }
        ("new", "list") => new_list(app, ctx, None),
        ("new", "group") => new_group(app, ctx),
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
        ("print", _) => super::print(ctx),
        ("settings", _) => ctx.dialog.set(Some(TodoDialog::Settings)),
        ("shortcuts", _) => ctx.dialog.set(Some(TodoDialog::Shortcuts)),
        _ => actions::run(app, ctx, Target::Tasks(ctx.targets()), id),
    }
}
