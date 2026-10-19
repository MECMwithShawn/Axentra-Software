//! My Day > To Do: today's tasks, add / complete / mark important
//! (people-todo.md §4.4). Shares the store with the To Do module, so both
//! stay in step through `tasks_rev`.

use std::collections::HashMap;

use axentra_core::*;
use chrono::{Duration, NaiveDate};
use dioxus::prelude::*;

use crate::kit::{EmptyState, Icon, dom, keys};
use crate::shell::use_shell;
use crate::state::{AppState, NavRequest, store, use_app};

/// Subtitle of a task row: list name and due date.
pub fn subtitle(list: &str, due: Option<NaiveDate>, today: NaiveDate) -> String {
    let due = due.map(|d| {
        if d == today {
            "Due today".to_string()
        } else if d == today + Duration::days(1) {
            "Due tomorrow".to_string()
        } else if d < today {
            format!("Overdue, {}", d.format("%a, %b %-d"))
        } else {
            format!("Due {}", d.format("%a, %b %-d"))
        }
    });
    match due {
        Some(d) if !list.is_empty() => format!("{list} · {d}"),
        Some(d) => d,
        None => list.to_string(),
    }
}

fn add_task(app: AppState, title: &str, today: NaiveDate) -> bool {
    let title = title.trim();
    if title.is_empty() {
        return false;
    }
    let mut t = Task { title: title.to_string(), my_day: Some(today), ..Default::default() };
    if app.check(store().insert_task(&mut t)).is_some() {
        app.bump_tasks();
        dom::announce("Task added", false);
        return true;
    }
    false
}

#[component]
pub fn TodoTab() -> Element {
    let app = use_app();
    let shell = use_shell();
    let today = (shell.now)().date_naive();
    let _ = app.tasks_rev();
    let tasks = store().tasks_for(SmartList::MyDay, today).unwrap_or_default();
    let lists: HashMap<TaskListId, String> = store().task_lists().unwrap_or_default().into_iter().map(|l| (l.id, l.name)).collect();
    let (done, open): (Vec<Task>, Vec<Task>) = tasks.into_iter().partition(|t| t.is_completed);
    let mut draft = use_signal(String::new);
    let mut show_done = use_signal(|| false);
    rsx! {
        div { class: "ax-myday-add",
            Icon { name: "add", size: 20 }
            input {
                r#type: "text",
                class: "ax-myday-add-input",
                placeholder: "Add a task",
                "aria-label": "Add a task",
                value: "{draft}",
                oninput: move |e| draft.set(e.value()),
                onkeydown: move |e: KeyboardEvent| {
                    keys::stop_typing_keys(&e);
                    if e.key() == Key::Enter && add_task(app, &draft(), today) {
                        draft.set(String::new());
                    }
                },
            }
            if !draft().trim().is_empty() {
                button {
                    r#type: "button",
                    class: "ax-btn ax-small",
                    onclick: move |_| {
                        if add_task(app, &draft(), today) {
                            draft.set(String::new());
                        }
                    },
                    "Add"
                }
            }
        }
        div { class: "ax-sidepanel-body",
            if open.is_empty() && done.is_empty() {
                EmptyState { icon: "weather_sunny", title: "No tasks for today", subtitle: "Add a task or drag an email here." }
            } else {
                div { class: "ax-myday-tasks", role: "list", "aria-label": "My Day tasks",
                    for t in open.iter().cloned() {
                        TaskRow { key: "{t.id}", list: lists.get(&t.list_id).cloned().unwrap_or_default(), task: t, today }
                    }
                }
                if !done.is_empty() {
                    button {
                        r#type: "button",
                        class: "ax-myday-done-toggle",
                        "aria-expanded": if show_done() { "true" } else { "false" },
                        onclick: move |_| show_done.set(!show_done()),
                        Icon { name: if show_done() { "chevron_down" } else { "chevron_right" }, size: 16 }
                        span { "Completed" }
                        span { class: "ax-muted", "{done.len()}" }
                    }
                    if show_done() {
                        div { class: "ax-myday-tasks", role: "list", "aria-label": "Completed tasks",
                            for t in done.iter().cloned() {
                                TaskRow { key: "{t.id}", list: lists.get(&t.list_id).cloned().unwrap_or_default(), task: t, today }
                            }
                        }
                    }
                }
            }
        }
        div { class: "ax-sidepanel-foot ax-myday-foot",
            button {
                r#type: "button",
                class: "ax-link-btn ax-row",
                style: "gap:6px",
                title: "Open My Day in To Do",
                onclick: move |_| app.navigate(NavRequest::SmartList(SmartList::MyDay)),
                Icon { name: "open", size: 16 }
                span { "Open To Do" }
            }
            button {
                r#type: "button",
                class: "ax-link-btn",
                title: "See every task in To Do",
                onclick: move |_| app.navigate(NavRequest::SmartList(SmartList::Tasks)),
                "Manage all tasks"
            }
        }
    }
}

#[component]
fn TaskRow(task: Task, list: String, today: NaiveDate) -> Element {
    let app = use_app();
    let id = task.id;
    let done = task.is_completed;
    let important = task.is_important;
    let sub = subtitle(&list, task.due, today);
    let overdue = task.due.is_some_and(|d| d < today) && !done;
    rsx! {
        div { class: "ax-myday-task", class: if done { "ax-done" }, role: "listitem",
            button {
                r#type: "button",
                class: "ax-task-check",
                role: "checkbox",
                "aria-checked": if done { "true" } else { "false" },
                title: if done { "Mark as not completed" } else { "Mark as completed" },
                "aria-label": if done { "Mark as not completed" } else { "Mark as completed" },
                onclick: move |_| {
                    if app.check(store().set_task_completed(id, !done)).is_some() {
                        app.bump_tasks();
                        dom::announce(if done { "Marked as not completed" } else { "Task completed" }, false);
                    }
                },
                Icon { name: if done { "checkmark_circle_filled" } else { "circle" }, size: 20 }
            }
            button {
                r#type: "button",
                class: "ax-myday-task-text",
                title: "Open in To Do",
                onclick: move |_| app.navigate(NavRequest::SmartList(SmartList::MyDay)),
                span { class: "ax-ellipsis ax-myday-task-title", "{task.title}" }
                if !sub.is_empty() {
                    span { class: "ax-ellipsis ax-caption", class: if overdue { "ax-overdue" } else { "ax-muted" }, "{sub}" }
                }
            }
            button {
                r#type: "button",
                class: "ax-task-star",
                class: if important { "ax-on" },
                "aria-pressed": if important { "true" } else { "false" },
                title: if important { "Remove importance" } else { "Mark as important" },
                "aria-label": "Mark as important",
                onclick: move |_| {
                    if app.check(store().set_task_important(id, !important)).is_some() {
                        app.bump_tasks();
                    }
                },
                Icon { name: if important { "star_filled" } else { "star" }, size: 20 }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_subtitles() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 19).unwrap();
        assert_eq!(subtitle("Tasks", Some(today), today), "Tasks · Due today");
        assert_eq!(subtitle("Tasks", Some(today + Duration::days(1)), today), "Tasks · Due tomorrow");
        assert_eq!(subtitle("", Some(today + Duration::days(2)), today), "Due Wed, Oct 21");
        assert_eq!(subtitle("Groceries", None, today), "Groceries");
        assert!(subtitle("Tasks", Some(today - Duration::days(2)), today).starts_with("Tasks · Overdue"));
    }

    #[test]
    fn adding_a_task_puts_it_in_my_day() {
        // Store-level check of what the To Do tab writes.
        let store = axentra_core::Store::open_in_memory().unwrap();
        let mut acct = Account { display_name: "T".into(), email: "t@x.example".into(), ..Default::default() };
        store.add_account(&mut acct).unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 10, 19).unwrap();
        let mut t = Task { title: "Call Priya".into(), my_day: Some(today), ..Default::default() };
        store.insert_task(&mut t).unwrap();
        let mine = store.tasks_for(SmartList::MyDay, today).unwrap();
        assert_eq!(mine.len(), 1);
        assert!(store.tasks_for(SmartList::MyDay, today + Duration::days(1)).unwrap().is_empty(), "My Day starts fresh the next day");
    }
}
