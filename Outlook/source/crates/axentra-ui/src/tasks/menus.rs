//! Menu contents (due date, reminder, repeat, categories, move, sort, theme,
//! task context menu) and [`MenuAnchor`], a trigger that opens a menu under
//! itself.

use std::rc::Rc;

use axentra_core::*;
use chrono::{Duration, Local, NaiveDate};
use dioxus::prelude::*;

use super::ctx::{SortKey, TodoCtx, View};
use super::dates;
use super::themes::THEMES;
use crate::kit::{Menu, MenuItem};

/// "Add due date" menu (3.10.1). Each quick date shows its weekday.
pub fn due_menu(today: NaiveDate, has_value: bool) -> Vec<MenuItem> {
    let mut items = vec![
        MenuItem::new("due:today", "Today").icon("calendar_today").shortcut(dates::weekday_short(today)),
        MenuItem::new("due:tomorrow", "Tomorrow").icon("calendar_arrow_right").shortcut(dates::weekday_short(today + Duration::days(1))),
        MenuItem::new("due:next_week", "Next week").icon("calendar_week_start").shortcut(dates::weekday_short(dates::next_week(today))),
        MenuItem::divider(),
        MenuItem::new("due:pick", "Pick a date").icon("calendar_edit"),
    ];
    if has_value {
        items.extend([MenuItem::divider(), MenuItem::new("due:none", "Remove due date").icon("dismiss").danger()]);
    }
    items
}

/// "Remind me" menu (3.10.2). "Later today" is hidden after 21:00.
pub fn reminder_menu(today: NaiveDate, has_value: bool, h24: bool) -> Vec<MenuItem> {
    let when = |t| format!("{}, {}", dates::weekday_short(local_date(t)), crate::fmt::time(t, h24));
    let mut items = Vec::new();
    if let Some(t) = dates::reminder_later_today(Local::now()) {
        items.push(MenuItem::new("rem:later", "Later today").icon("clock").shortcut(crate::fmt::time(t, h24)));
    }
    items.extend([
        MenuItem::new("rem:tomorrow", "Tomorrow").icon("calendar_arrow_right").shortcut(when(dates::reminder_tomorrow(today))),
        MenuItem::new("rem:next_week", "Next week").icon("calendar_week_start").shortcut(when(dates::reminder_next_week(today))),
        MenuItem::divider(),
        MenuItem::new("rem:pick", "Pick a date & time").icon("calendar_clock"),
    ]);
    if has_value {
        items.extend([MenuItem::divider(), MenuItem::new("rem:none", "Remove reminder").icon("dismiss").danger()]);
    }
    items
}

/// "Repeat" menu (3.10.3).
pub fn repeat_menu(has_value: bool) -> Vec<MenuItem> {
    let mut items = vec![
        MenuItem::new("rep:daily", "Daily").icon("calendar_day"),
        MenuItem::new("rep:weekdays", "Weekdays").icon("calendar_work_week"),
        MenuItem::new("rep:weekly", "Weekly").icon("calendar_week_numbers"),
        MenuItem::new("rep:monthly", "Monthly").icon("calendar_month"),
        MenuItem::new("rep:yearly", "Yearly").icon("calendar_ltr"),
        MenuItem::divider(),
        MenuItem::new("rep:custom", "Custom").icon("calendar_edit"),
    ];
    if has_value {
        items.extend([MenuItem::divider(), MenuItem::new("rep:none", "Remove recurrence").icon("dismiss").danger()]);
    }
    items
}

/// "Pick a category": the account's categories with swatches (checked when
/// every target has it), then "Manage categories".
pub fn category_menu(categories: &[Category], chosen: &[String]) -> Vec<MenuItem> {
    let mut items: Vec<MenuItem> = categories
        .iter()
        .map(|c| {
            MenuItem::new(format!("cat:{}", c.name), c.name.clone())
                .swatch(crate::kit::category_colors(&c.color).0)
                .checked(chosen.contains(&c.name))
        })
        .collect();
    if items.is_empty() {
        items.push(MenuItem::new("cat:none", "No categories yet").disabled(true));
    }
    items.extend([MenuItem::divider(), MenuItem::new("cat:manage", "Manage categories").icon("tag")]);
    items
}

/// Label of a list in menus: its emoji and name.
pub fn list_label(l: &TaskList) -> String {
    match &l.emoji {
        Some(e) => format!("{e} {}", l.name),
        None => l.name.clone(),
    }
}

/// "Move task to…" submenu: every list except `current`.
pub fn move_menu(lists: &[TaskList], current: Option<TaskListId>) -> Vec<MenuItem> {
    lists
        .iter()
        .filter(|l| Some(l.id) != current)
        .map(|l| {
            let item = MenuItem::new(format!("move:{}", l.id), list_label(l));
            match (l.is_default, &l.emoji) {
                (true, _) => item.icon("home"),
                (false, None) => item.icon("text_bullet_list_ltr"),
                (false, Some(_)) => item,
            }
        })
        .collect()
}

/// "Sort by" items (3.4).
pub fn sort_menu(current: Option<SortKey>) -> Vec<MenuItem> {
    let mut items: Vec<MenuItem> = vec![MenuItem::header("Sort by")];
    items.extend(SortKey::ALL.into_iter().map(|k| MenuItem::new(format!("sort:{}", k.id()), k.label()).icon(k.icon()).checked(current == Some(k))));
    items
}

/// "Change theme" swatches.
pub fn theme_menu(current: Option<&str>) -> Vec<MenuItem> {
    let mut items = vec![MenuItem::new("theme:none", "Default").checked(current.is_none()).swatch("var(--ax-bg-brand)")];
    items.extend(THEMES.iter().map(|(id, label, light, _)| MenuItem::new(format!("theme:{id}"), *label).swatch(*light).checked(current == Some(*id))));
    items
}

/// "List options" (…) menu (3.4). Smart lists get sort, theme and the
/// completed toggle only.
pub fn list_options_menu(view: View, sort: Option<SortKey>, theme: Option<&str>, show_completed: bool, custom: bool) -> Vec<MenuItem> {
    let mut items = Vec::new();
    if custom {
        items.push(MenuItem::new("list:rename", "Rename list").icon("rename"));
    }
    items.push(MenuItem::new("list:sort", "Sort by").icon("arrow_sort").submenu(sort_menu(sort)));
    items.push(MenuItem::new("list:theme", "Change theme").icon("color").submenu(theme_menu(theme)));
    if view != View::Completed {
        items.push(if show_completed {
            MenuItem::new("list:hide_completed", "Hide completed tasks").icon("eye_off")
        } else {
            MenuItem::new("list:show_completed", "Show completed tasks").icon("eye")
        });
    }
    if custom {
        items.extend([
            MenuItem::new("list:duplicate", "Duplicate list").icon("copy"),
            MenuItem::new("list:print", "Print list").icon("print"),
            MenuItem::divider(),
            MenuItem::new("list:delete", "Delete list").icon("delete").danger(),
        ]);
    } else {
        items.push(MenuItem::new("list:print", "Print list").icon("print"));
    }
    items
}

/// Right-click menu of task rows (3.6), acting on all of `tasks`.
pub fn task_context_menu(tasks: &[Task], lists: &[TaskList], today: NaiveDate, include_due_today: bool) -> Vec<MenuItem> {
    let all = |f: &dyn Fn(&Task) -> bool| !tasks.is_empty() && tasks.iter().all(f);
    let in_day = all(&|t| super::actions::in_my_day(t, today, include_due_today));
    let important = all(&|t| t.is_important);
    let done = all(&|t| t.is_completed);
    let any_due = tasks.iter().any(|t| t.due.is_some());
    let current = tasks.first().map(|t| t.list_id).filter(|l| tasks.iter().all(|t| t.list_id == *l));
    let mut items = vec![
        if in_day {
            MenuItem::new("myday:remove", "Remove from My Day").icon("weather_sunny")
        } else {
            MenuItem::new("myday:add", "Add to My Day").icon("weather_sunny").shortcut("Ctrl+T")
        },
        if important {
            MenuItem::new("imp:off", "Remove importance").icon("star_filled")
        } else {
            MenuItem::new("imp:on", "Mark as important").icon("star")
        },
        if done {
            MenuItem::new("done:off", "Mark as not completed").icon("circle")
        } else {
            MenuItem::new("done:on", "Mark as completed").icon("checkmark_circle").shortcut("Ctrl+D")
        },
        MenuItem::divider(),
        MenuItem::new("due:today", "Due today").icon("calendar_today"),
        MenuItem::new("due:tomorrow", "Due tomorrow").icon("calendar_arrow_right"),
        MenuItem::new("due:pick", "Pick a date").icon("calendar_edit"),
    ];
    if any_due {
        items.push(MenuItem::new("due:none", "Remove due date").icon("calendar_cancel"));
    }
    items.extend([
        MenuItem::divider(),
        MenuItem::new("move", "Move task to...").icon("folder_arrow_right").submenu(move_menu(lists, current)),
        MenuItem::divider(),
        MenuItem::new("delete", if tasks.len() > 1 { "Delete tasks" } else { "Delete task" }).icon("delete").danger().shortcut("Delete"),
    ]);
    items
}

/// A button that opens `items` under itself. Children are the button's
/// content.
#[component]
pub fn MenuAnchor(
    items: Vec<MenuItem>,
    on_select: EventHandler<String>,
    #[props(into, default)] class: String,
    #[props(into, default)] title: String,
    #[props(default)] disabled: bool,
    /// Told when the menu opens (true) and closes (false).
    on_open: Option<EventHandler<bool>>,
    children: Element,
) -> Element {
    let mut pos = use_signal(|| None::<(f64, f64)>);
    let mut anchor = use_signal(|| None::<Rc<MountedData>>);
    let todo = try_use_context::<TodoCtx>();
    rsx! {
        button {
            r#type: "button",
            class: "{class}",
            title: "{title}",
            "aria-label": if title.is_empty() { None } else { Some(title.clone()) },
            "aria-haspopup": "menu",
            "aria-expanded": if pos().is_some() { "true" } else { "false" },
            disabled,
            onmounted: move |e| anchor.set(Some(e.data())),
            onclick: move |e: MouseEvent| {
                e.stop_propagation();
                let fallback = e.client_coordinates();
                spawn(async move {
                    let rect = match anchor() {
                        Some(m) => m.get_client_rect().await.ok(),
                        None => None,
                    };
                    pos.set(Some(rect.map(|r| (r.min_x(), r.max_y() + 4.0)).unwrap_or((fallback.x, fallback.y + 8.0))));
                    if let Some(h) = on_open {
                        h.call(true);
                    }
                });
            },
            {children}
        }
        if let Some((x, y)) = pos() {
            Menu {
                items: items.clone(),
                x,
                y,
                on_select,
                on_close: move |_| {
                    pos.set(None);
                    if let Some(h) = on_open {
                        h.call(false);
                    }
                    // Focus returns to the trigger unless the command opened
                    // a picker or dialog (keyboard spec 12.4).
                    spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
                        if !todo.is_some_and(|t| t.overlay_open())
                            && let Some(m) = anchor.peek().clone()
                        {
                            let _ = m.set_focus(true).await;
                        }
                    });
                },
            }
        }
    }
}
