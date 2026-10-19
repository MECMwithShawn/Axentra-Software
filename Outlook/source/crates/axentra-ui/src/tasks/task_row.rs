//! Task row card (3.6): completion circle, title with #tags, subtitle line,
//! star; selection, context menu and drag.

use axentra_core::*;
use chrono::{NaiveDate, Utc};
use dioxus::prelude::*;

use super::actions;
use super::ctx::{Drag, use_todo};
use super::dates::{self, DueState};
use crate::kit::Icon;
use crate::state::{SearchState, use_app};

/// Completion circle: ring plus a check that draws in when completed.
pub const CHECK_SVG: &str = r#"<svg viewBox="0 0 20 20" aria-hidden="true"><circle class="ax-todo-ring" cx="10" cy="10" r="8.25"/><path class="ax-todo-tick" pathLength="1" d="M6.3 10.3l2.5 2.5 4.9-5.3"/></svg>"#;

/// Split a title into text runs and `#tag` runs.
pub fn title_parts(title: &str) -> Vec<(String, bool)> {
    let mut out: Vec<(String, bool)> = Vec::new();
    for word in title.split_inclusive(' ') {
        let bare = word.trim_end();
        let is_tag = bare.len() > 1 && bare.starts_with('#') && !bare[1..].starts_with('#');
        if is_tag {
            out.push((bare.to_string(), true));
            if word.len() > bare.len() {
                out.push((word[bare.len()..].to_string(), false));
            }
        } else if let Some(last) = out.last_mut().filter(|l| !l.1) {
            last.0.push_str(word);
        } else {
            out.push((word.to_string(), false));
        }
    }
    out
}

/// Title text with clickable hashtags that search for the tag.
#[component]
pub fn TaskTitle(#[props(into)] title: String) -> Element {
    rsx! {
        for (i, (text, tag)) in title_parts(&title).into_iter().enumerate() {
            if tag {
                TagLink { key: "{i}", tag: text }
            } else {
                span { key: "{i}", "{text}" }
            }
        }
    }
}

#[component]
fn TagLink(tag: String) -> Element {
    let mut app = use_app();
    let query = tag.clone();
    rsx! {
        span {
            class: "ax-todo-tag",
            role: "link",
            title: "Search for {tag}",
            onclick: move |e| {
                e.stop_propagation();
                let serial = app.search.peek().serial + 1;
                app.search.set(SearchState { query: query.clone(), active: true, serial });
            },
            "{tag}"
        }
    }
}

#[derive(Clone, PartialEq)]
pub struct RowData {
    pub task: Task,
    /// List name shown first in the subtitle (smart lists and search).
    pub list_name: Option<String>,
    /// The task is in today's My Day.
    pub in_my_day: bool,
    /// Show the "My Day" subtitle item (not in the My Day view itself).
    pub show_my_day: bool,
    pub selected: bool,
    /// Roving tab stop of the list.
    pub tab_stop: bool,
    pub drop_before: bool,
    pub reorderable: bool,
    /// Ids dragged with this row (the selection when it contains the row).
    pub drag_ids: Vec<TaskId>,
    /// (name, colour) of the task's categories.
    pub categories: Vec<(String, String)>,
    pub today: NaiveDate,
    pub h24: bool,
    /// (id, shift, ctrl)
    pub on_click: EventHandler<(TaskId, bool, bool)>,
    /// (id, x, y)
    pub on_context: EventHandler<(TaskId, f64, f64)>,
    /// A dragged task was dropped on this row.
    pub on_drop: EventHandler<TaskId>,
    pub on_open: EventHandler<TaskId>,
}

#[component]
pub fn TaskRow(row: RowData) -> Element {
    let props = row;
    let app = use_app();
    let mut ctx = use_todo();
    let t = &props.task;
    let id = t.id;
    let completing = ctx.completing.read().contains(&id);
    let checked = t.is_completed || completing;
    let important = t.is_important;
    let dragging = matches!(&*ctx.drag.read(), Some(Drag::Tasks(ids)) if ids.contains(&id));
    let drag_active = matches!(&*ctx.drag.read(), Some(Drag::Tasks(_)));
    let drop_key = format!("task:{id}");
    let over = props.drop_before && ctx.drop_target.read().as_deref() == Some(drop_key.as_str());
    let reorderable = props.reorderable;
    let drag_ids = props.drag_ids.clone();
    let drag_attr = drag_ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",");
    let on_click = props.on_click;
    let on_context = props.on_context;
    let on_drop = props.on_drop;
    let on_open = props.on_open;

    // Subtitle pieces, in spec order.
    let steps_done = t.steps.iter().filter(|s| s.is_completed).count();
    let due = t.due.map(|d| (dates::due_row_label(d, props.today), dates::due_state(d, props.today)));
    let reminder = t.reminder.filter(|r| *r > Utc::now()).map(|r| dates::reminder_tooltip(r, props.today, props.h24));
    let repeat = t.recurrence.as_ref().map(|r| dates::repeat_description(r, t.due.unwrap_or(props.today)));
    let has_note = !t.notes.trim().is_empty();
    let cats = props.categories.clone();
    let extra_cats = cats.len().saturating_sub(2);
    let check_label = if checked { "Mark as not completed" } else { "Mark as completed" };
    let star_label = if important { "Remove importance" } else { "Mark as important" };
    rsx! {
        div {
            class: "ax-todo-card",
            class: if props.selected { "ax-selected" },
            class: if checked { "ax-todo-done" },
            class: if completing { "ax-todo-completing" },
            class: if dragging { "ax-todo-dragging" },
            class: if over { "ax-todo-drop-before" },
            role: "row",
            "aria-selected": if props.selected { "true" } else { "false" },
            "data-task-row": "{id}",
            "data-task-ids": "{drag_attr}",
            "data-ctx": "1",
            tabindex: if props.tab_stop { "0" } else { "-1" },
            draggable: "true",
            onclick: move |e: MouseEvent| {
                let m = e.modifiers();
                on_click.call((id, m.shift(), m.ctrl() || m.meta()));
            },
            oncontextmenu: move |e: MouseEvent| {
                e.prevent_default();
                let c = e.client_coordinates();
                on_context.call((id, c.x, c.y));
            },
            ondragstart: move |_| ctx.drag.set(Some(Drag::Tasks(drag_ids.clone()))),
            ondragend: move |_| {
                ctx.drag.set(None);
                ctx.drop_target.set(None);
            },
            ondragover: move |_| {
                if reorderable && drag_active && !dragging && !over {
                    ctx.drop_target.set(Some(format!("task:{id}")));
                }
            },
            ondrop: move |e| {
                e.stop_propagation();
                if reorderable && !dragging {
                    on_drop.call(id);
                }
                ctx.drag.set(None);
                ctx.drop_target.set(None);
            },
            if reorderable {
                span { class: "ax-todo-handle", "aria-hidden": "true", Icon { name: "re_order_dots_vertical", size: 16 } }
            }
            button {
                r#type: "button",
                class: "ax-todo-check",
                class: if checked { "ax-checked" },
                role: "checkbox",
                "aria-checked": if checked { "true" } else { "false" },
                "aria-label": "{check_label}",
                title: "{check_label}",
                tabindex: if props.tab_stop { "0" } else { "-1" },
                onclick: move |e: MouseEvent| {
                    e.stop_propagation();
                    actions::complete(app, ctx, id, !checked);
                    if !checked {
                        actions::advance_from(ctx, id);
                    }
                },
                onkeydown: move |e: KeyboardEvent| if e.key() == Key::Enter || e.key() == Key::Character(" ".into()) { e.stop_propagation() },
                span { class: "ax-todo-check-svg", dangerous_inner_html: CHECK_SVG }
            }
            div { class: "ax-todo-card-main", role: "gridcell",
                div {
                    class: "ax-todo-card-title",
                    ondoubleclick: move |e| {
                        e.stop_propagation();
                        on_open.call(id);
                        actions::focus_later("#ax-todo-detail-title", 60);
                    },
                    if t.linked_message_id.is_some() {
                        span { class: "ax-todo-title-icon", title: "Linked email", Icon { name: "mail", size: 12 } }
                    }
                    TaskTitle { title: t.title.clone() }
                }
                div { class: "ax-todo-card-sub",
                    if let Some(name) = &props.list_name {
                        span { class: "ax-todo-sub-item", "{name}" }
                    }
                    if props.show_my_day && props.in_my_day {
                        span { class: "ax-todo-sub-item", Icon { name: "weather_sunny", size: 12 } "My Day" }
                    }
                    if !t.steps.is_empty() {
                        span { class: "ax-todo-sub-item", "{steps_done} of {t.steps.len()}" }
                    }
                    if let Some((label, state)) = &due {
                        span {
                            class: "ax-todo-sub-item",
                            class: if *state == DueState::Overdue && !checked { "ax-todo-overdue" },
                            class: if *state == DueState::Today && !checked { "ax-todo-due-today" },
                            Icon { name: "calendar_ltr", size: 12 }
                            "{label}"
                        }
                    }
                    if let Some(tip) = &reminder {
                        span { class: "ax-todo-sub-item", title: "{tip}", "aria-label": "{tip}", Icon { name: "alert", size: 12 } }
                    }
                    if let Some(tip) = &repeat {
                        span { class: "ax-todo-sub-item", title: "{tip}", "aria-label": "Repeats: {tip}", Icon { name: "arrow_repeat_all", size: 12 } }
                    }
                    if has_note {
                        span { class: "ax-todo-sub-item", title: "Note", "aria-label": "Has a note", Icon { name: "note", size: 12 } }
                    }
                    if !cats.is_empty() {
                        span { class: "ax-todo-sub-item ax-todo-cats",
                            for (name, color) in cats.iter().take(2) {
                                CategoryChip { key: "{name}", name: name.clone(), color: color.clone() }
                            }
                            if extra_cats > 0 { span { class: "ax-todo-cat-more", "+{extra_cats}" } }
                        }
                    }
                }
            }
            button {
                r#type: "button",
                class: "ax-todo-star",
                class: if important { "ax-toggled" },
                role: "gridcell",
                "aria-pressed": if important { "true" } else { "false" },
                "aria-label": "{star_label}",
                title: "{star_label}",
                tabindex: if props.tab_stop { "0" } else { "-1" },
                onclick: move |e: MouseEvent| {
                    e.stop_propagation();
                    actions::set_important(app, &[id], !important);
                    actions::follow_focus(ctx, &[id]);
                },
                onkeydown: move |e: KeyboardEvent| if e.key() == Key::Enter || e.key() == Key::Character(" ".into()) { e.stop_propagation() },
                Icon { name: if important { "star_filled" } else { "star" } }
            }
        }
    }
}

/// Category pill tinted with the category colour (light and dark shades).
#[component]
pub fn CategoryChip(#[props(into)] name: String, #[props(into)] color: String, on_remove: Option<EventHandler<()>>) -> Element {
    let (light, dark) = crate::kit::category_colors(&color);
    rsx! {
        span { class: "ax-todo-cat", style: "--ax-todo-cat-l:{light};--ax-todo-cat-d:{dark}",
            span { class: "ax-todo-cat-dot" }
            span { class: "ax-ellipsis", "{name}" }
            if let Some(h) = on_remove {
                button {
                    r#type: "button",
                    class: "ax-todo-cat-x",
                    title: "Remove {name}",
                    "aria-label": "Remove {name}",
                    onclick: move |e| { e.stop_propagation(); h.call(()) },
                    Icon { name: "dismiss", size: 12 }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::title_parts;

    #[test]
    fn hashtags_split_out() {
        let parts = title_parts("Plan offsite #team and #q4 now");
        let tags: Vec<&str> = parts.iter().filter(|p| p.1).map(|p| p.0.as_str()).collect();
        assert_eq!(tags, ["#team", "#q4"]);
        assert_eq!(parts.iter().map(|p| p.0.as_str()).collect::<String>(), "Plan offsite #team and #q4 now");
        assert!(title_parts("Issue # 4 and ##x").iter().all(|p| !p.1));
    }
}
