//! My Day "Suggestions" panel (3.11): overdue, due today, upcoming, earlier
//! and recently added tasks, each with "Add to My Day".

use std::collections::HashMap;

use axentra_core::*;
use chrono::Utc;
use dioxus::prelude::*;

use super::actions;
use super::ctx::use_todo;
use super::dates::{self, DueState};
use super::menus::list_label;
use crate::kit::{EmptyState, Icon, IconButton};
use crate::state::{store, use_app};

fn heading(key: &str) -> &'static str {
    match key {
        "overdue" => "Overdue",
        "today" => "Due today",
        "upcoming" => "Upcoming",
        "earlier" => "Earlier",
        _ => "Recently added",
    }
}

#[component]
pub fn SuggestionsPane() -> Element {
    let app = use_app();
    let mut ctx = use_todo();
    let _ = app.tasks_rev();
    let today = (ctx.today)();
    let include_due = ctx.prefs.read().due_today_in_my_day;
    let sections = store().my_day_suggestions(today, include_due, Utc::now()).unwrap_or_default();
    let names: HashMap<TaskListId, String> = store().task_lists().unwrap_or_default().iter().map(|l| (l.id, list_label(l))).collect();
    let empty = sections.iter().all(|(_, v)| v.is_empty());
    rsx! {
        aside { class: "ax-pane ax-todo-detail ax-todo-suggest", "aria-label": "Suggestions",
            div { class: "ax-todo-suggest-head",
                span { class: "ax-grow", "Suggestions" }
                IconButton { icon: "dismiss", title: "Close", onclick: move |_| ctx.suggestions.set(false) }
            }
            div { class: "ax-todo-detail-scroll",
                if empty {
                    EmptyState { icon: "lightbulb", title: "No suggestions right now", subtitle: "Tasks that are overdue, due soon or recently added show up here." }
                }
                for (key, tasks) in sections.into_iter().filter(|(_, v)| !v.is_empty()) {
                    section { key: "{key}", class: "ax-todo-suggest-section", "aria-label": "{heading(key)}",
                        h3 { class: "ax-todo-suggest-title", "{heading(key)}" }
                        for t in tasks {
                            SuggestionRow { key: "{t.id}", list_name: names.get(&t.list_id).cloned().unwrap_or_default(), task: t }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn SuggestionRow(task: Task, list_name: String) -> Element {
    let app = use_app();
    let ctx = use_todo();
    let today = (ctx.today)();
    let id = task.id;
    let due = task.due.map(|d| (dates::due_row_label(d, today), dates::due_state(d, today)));
    rsx! {
        div { class: "ax-todo-card ax-todo-suggest-row",
            div { class: "ax-todo-card-main",
                div { class: "ax-todo-card-title", "{task.title}" }
                div { class: "ax-todo-card-sub",
                    span { class: "ax-todo-sub-item", "{list_name}" }
                    if let Some((label, state)) = due {
                        span { class: "ax-todo-sub-item", class: if state == DueState::Overdue { "ax-todo-overdue" },
                            Icon { name: "calendar_ltr", size: 12 }
                            "{label}"
                        }
                    }
                }
            }
            IconButton { icon: "add", title: "Add to My Day", onclick: move |_| actions::set_my_day(app, ctx, &[id], true) }
        }
    }
}
