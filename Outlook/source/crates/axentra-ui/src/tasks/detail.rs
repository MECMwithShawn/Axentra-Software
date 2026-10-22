//! Task detail pane (3.9): title and steps, My Day, dates, categories,
//! files, linked email, note, footer.

use axentra_core::*;
use chrono::Utc;
use dioxus::prelude::*;

use super::actions;
use super::ctx::{PaneEdge, Target, TodoCtx, use_todo};
use super::dates::{self, DueState};
use super::menus::{self, MenuAnchor};
use super::splitter::Splitter;
use super::task_row::{CHECK_SVG, CategoryChip};
use crate::kit::{Avatar, Icon, IconButton, MenuItem, keys};
use crate::state::{AppState, NavRequest, store, use_app};

const TITLE_ID: &str = "ax-todo-detail-title";

/// Prefix of this pane's `data-ax-late-keys` tags (kit::dom::use_late_keys).
pub const LATE_PREFIX: &str = "todo";

/// A field of the pane, by its late-keys tag after `todo:`.
#[derive(Clone, Copy, Debug, PartialEq)]
enum LateField {
    Title(TaskId),
    Note(TaskId),
    Step(TaskId, i64),
}

impl LateField {
    fn tag(self) -> String {
        match self {
            LateField::Title(id) => format!("{LATE_PREFIX}:title:{id}"),
            LateField::Note(id) => format!("{LATE_PREFIX}:note:{id}"),
            LateField::Step(id, sid) => format!("{LATE_PREFIX}:step:{id}:{sid}"),
        }
    }

    fn parse(rest: &str) -> Option<LateField> {
        let mut it = rest.split(':');
        let f = match (it.next()?, it.next()?.parse().ok()?) {
            ("title", id) => LateField::Title(id),
            ("note", id) => LateField::Note(id),
            ("step", id) => LateField::Step(id, it.next()?.parse().ok()?),
            _ => return None,
        };
        it.next().is_none().then_some(f)
    }
}

/// Keys pressed in a title, note or step before a click that opened another
/// task, delivered by a busy page after the card went (its `use_drop` saved
/// what had landed): `value` is the field with them, saved as the card
/// would have. Quietly, as the task may be gone.
pub fn late_typed(app: AppState, rest: &str, value: &str) {
    let s = store();
    let changed = match LateField::parse(rest) {
        Some(LateField::Title(id)) => {
            let title = value.replace('\n', " ");
            let current = s.task(id).ok().flatten().map(|t| t.title);
            if !title.trim().is_empty() && current.is_some_and(|c| c != actions::task_title(&title)) {
                actions::rename_task(app, id, &title);
            }
            false
        }
        Some(LateField::Note(id)) => s.task(id).ok().flatten().is_some_and(|t| t.notes != value) && s.set_task_notes(id, value).is_ok(),
        Some(LateField::Step(id, sid)) => !value.trim().is_empty() && s.update_task_step(id, sid, Some(value), None).is_ok(),
        None => false,
    };
    if changed {
        app.bump_tasks();
    }
}

/// Grow a textarea to fit its content.
fn autosize(id: &str) {
    document::eval(&format!(
        "const el = document.getElementById('{id}'); if (el) {{ el.style.height = 'auto'; el.style.height = el.scrollHeight + 'px'; }}"
    ));
}

/// Close the pane and return focus to the task's row.
pub fn close_detail(mut ctx: TodoCtx) {
    let current = *ctx.detail.peek();
    if let Some(id) = current {
        ctx.detail.set(None);
        actions::focus_row(id, false);
    }
}

#[component]
pub fn DetailPane(id: TaskId) -> Element {
    let app = use_app();
    let ctx = use_todo();
    let _ = app.tasks_rev();
    let Some(task) = store().task(id).ok().flatten() else { return rsx! {} };
    let today = (ctx.today)();
    let lists = store().task_lists().unwrap_or_default();
    let list_name = lists.iter().find(|l| l.id == task.list_id).map(menus::list_label).unwrap_or_default();
    rsx! {
        aside { class: "ax-pane ax-todo-detail", "aria-label": "Task details",
            Splitter { edge: PaneEdge::Detail }
            div { class: "ax-todo-detail-scroll",
                TitleCard { task: task.clone() }
                MyDayCard { task: task.clone() }
                DatesCard { task: task.clone() }
                CategoryCard { task: task.clone() }
                div { class: "ax-todo-detail-card",
                    button {
                        r#type: "button",
                        class: "ax-todo-detail-row",
                        disabled: true,
                        title: "Attaching files to tasks isn't available yet",
                        Icon { name: "attach" }
                        span { class: "ax-grow", "Add file" }
                    }
                }
                if task.assigned_to_me {
                    div { class: "ax-todo-detail-card",
                        div { class: "ax-todo-detail-row ax-todo-static", Icon { name: "person" } span { class: "ax-grow", "Assigned to you" } }
                    }
                }
                if let Some(m) = task.linked_message_id {
                    LinkedEmail { message_id: m }
                }
                NoteCard { task: task.clone() }
                div { class: "ax-todo-detail-list", "In {list_name}" }
            }
            div { class: "ax-todo-detail-footer",
                IconButton { icon: "panel_right_contract", title: "Dismiss detail view", onclick: move |_| close_detail(ctx) }
                span { class: "ax-grow ax-todo-detail-created", "{dates::footer_label(&task, today)}" }
                IconButton { icon: "delete", title: "Delete task", onclick: move |_| actions::request_delete(app, ctx, vec![id]) }
            }
        }
    }
}

/// Typed text and a saved title say the same, as the task keeps titles.
fn same(typed: &str, kept: &str) -> bool {
    actions::task_title(typed) == actions::task_title(kept)
}

/// Whether the title field follows a change made elsewhere (a row rename,
/// undo): the task's `stored` title is not the one last `saved` here, and
/// the field still says what was saved (`typed`, not being edited). The
/// card's own autosave is not such a change, so the field keeps the
/// user's spaces ("milk " stays while "and eggs" is still to come).
fn follows_elsewhere(typed: &str, saved: &str, stored: &str) -> bool {
    !same(saved, stored) && same(typed, saved)
}

#[component]
fn TitleCard(task: Task) -> Element {
    let app = use_app();
    let ctx = use_todo();
    let id = task.id;
    // The field as typed, and the title last saved from it as the task
    // keeps it (`actions::task_title`: single spaces, trimmed).
    let mut title = use_signal(|| task.title.clone());
    let mut saved = use_signal(|| task.title.clone());
    if follows_elsewhere(&title.peek(), &saved.peek(), &task.title) {
        title.set(task.title.clone());
        saved.set(task.title.clone());
    }
    let completing = ctx.completing.read().contains(&id);
    let checked = task.is_completed || completing;
    let important = task.is_important;
    let mut save = move || {
        let t = title();
        if t.trim().is_empty() {
            title.set(saved());
        } else if !same(&t, &saved()) {
            saved.set(actions::task_title(&t));
            actions::rename_task(app, id, &t);
        }
    };
    use_effect(move || autosize(TITLE_ID));
    crate::kit::dom::use_late_tag(LateField::Title(id).tag());
    // Leaving To Do (Ctrl+1) removes the field without a blur, and the
    // autosave goes with the module: save what was typed.
    use_drop(move || {
        if let (Ok(t), Ok(s)) = (title.try_peek().map(|t| t.clone()), saved.try_peek().map(|s| s.clone()))
            && !same(&t, &s)
            && !t.trim().is_empty()
        {
            actions::rename_task(app, id, &t);
        }
    });
    let step_drag = use_signal(StepDrag::default);
    let steps = task.steps.clone();
    let check_label = if checked { "Mark as not completed" } else { "Mark as completed" };
    rsx! {
        div { class: "ax-todo-detail-card",
            div { class: "ax-todo-detail-title-row",
                button {
                    r#type: "button",
                    class: "ax-todo-check",
                    class: if checked { "ax-checked" },
                    role: "checkbox",
                    "aria-checked": if checked { "true" } else { "false" },
                    "aria-label": "{check_label}",
                    title: "{check_label}",
                    onclick: move |_| actions::complete(app, ctx, id, !checked),
                    span { class: "ax-todo-check-svg", dangerous_inner_html: CHECK_SVG }
                }
                textarea {
                    id: TITLE_ID,
                    class: "ax-todo-detail-title",
                    class: if checked { "ax-todo-struck" },
                    rows: "1",
                    maxlength: "255",
                    "aria-label": "Rename task",
                    // Keys typed just before a click on another task are
                    // still saved here (`late_typed`).
                    "data-ax-late-keys": LateField::Title(id).tag(),
                    value: "{title}",
                    oninput: move |e| {
                        let v = e.value().replace('\n', " ");
                        title.set(v.clone());
                        autosize(TITLE_ID);
                        // Autosave after a pause in typing, as this card's
                        // task: opening another task removes the card, and
                        // `use_drop` above saves what was typed instead.
                        crate::kit::after(1000, move || {
                            if *title.peek() == v && !same(&v, &saved.peek()) && !v.trim().is_empty() {
                                saved.set(actions::task_title(&v));
                                actions::rename_task(app, id, &v);
                            }
                        });
                    },
                    onkeydown: move |e: KeyboardEvent| {
                        keys::stop_typing_keys(&e);
                        match e.key() {
                            Key::Enter => {
                                e.prevent_default();
                                save();
                                actions::focus_row(id, false);
                            }
                            Key::Escape => {
                                e.stop_propagation();
                                title.set(saved());
                                actions::focus_row(id, false);
                            }
                            _ => {}
                        }
                    },
                    onblur: move |_| save(),
                }
                button {
                    r#type: "button",
                    class: "ax-todo-star",
                    class: if important { "ax-toggled" },
                    "aria-pressed": if important { "true" } else { "false" },
                    title: if important { "Remove importance" } else { "Mark as important" },
                    "aria-label": if important { "Remove importance" } else { "Mark as important" },
                    onclick: move |_| actions::set_important(app, &[id], !important),
                    Icon { name: if important { "star_filled" } else { "star" } }
                }
            }
            div {
                class: "ax-todo-steps",
                class: if step_drag().dragging.is_some() { "ax-todo-steps-dragging" },
                role: "list",
                "aria-label": "Steps",
                for step in steps.iter().cloned() {
                    StepRow { key: "{step.id}", task_id: id, step, drag: step_drag }
                }
            }
            AddStep { task_id: id, has_steps: !steps.is_empty(), drag: step_drag }
        }
    }
}

/// A step being dragged to a new place (3.9: steps reorder by drag).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct StepDrag {
    /// Step whose handle is pressed: its row is draggable while this is set.
    armed: Option<i64>,
    dragging: Option<i64>,
    /// Drop target: before this step, or `Some(None)` = after the last one.
    over: Option<Option<i64>>,
}

/// Drop the dragged step before `before` (or at the end).
fn drop_step(app: AppState, mut drag: Signal<StepDrag>, task_id: TaskId, before: Option<i64>) {
    let d = *drag.peek();
    drag.set(StepDrag::default());
    if let Some(sid) = d.dragging
        && Some(sid) != before
        && app.check(store().reorder_task_step(task_id, sid, before)).is_some()
    {
        app.bump_tasks();
    }
}

#[component]
fn StepRow(task_id: TaskId, step: TaskStep, drag: Signal<StepDrag>) -> Element {
    let app = use_app();
    let mut drag = drag;
    let mut text = use_signal(|| step.title.clone());
    let sid = step.id;
    let done = step.is_completed;
    let d = drag();
    let armed = d.armed == Some(sid);
    let dragging = d.dragging == Some(sid);
    let over = d.dragging.is_some() && !dragging && d.over == Some(Some(sid));
    let original = step.title.clone();
    // Typed in and not saved yet (or put back) since.
    let mut edited = use_hook(|| CopyValue::new(false));
    crate::kit::dom::use_late_tag(LateField::Step(task_id, sid).tag());
    let save = move || {
        edited.set(false);
        let t = text();
        if t.trim().is_empty() {
            text.set(original.clone());
        } else if t != original && app.check(store().update_task_step(task_id, sid, Some(&t), None)).is_some() {
            app.bump_tasks();
        }
    };
    let mut save_key = save.clone();
    let mut save_blur = save;
    // Removed without a blur (leaving To Do with a key): keep the edit. The
    // step may be gone with its task, so quietly.
    use_drop(move || {
        if edited.try_peek().is_ok_and(|e| *e)
            && let Ok(t) = text.try_peek().map(|t| t.clone())
            && !t.trim().is_empty()
            && store().update_task_step(task_id, sid, Some(&t), None).is_ok()
        {
            app.bump_tasks();
        }
    });
    let check_label = if done { "Mark as not completed" } else { "Mark as completed" };
    let menu = vec![
        MenuItem::new("toggle", check_label).icon(if done { "circle" } else { "checkmark_circle" }),
        MenuItem::new("promote", "Promote to task").icon("arrow_up_left"),
        MenuItem::divider(),
        MenuItem::new("delete", "Delete step").icon("delete").danger(),
    ];
    let set_done = move |d: bool| {
        if app.check(store().update_task_step(task_id, sid, None, Some(d))).is_some() {
            app.bump_tasks();
        }
    };
    rsx! {
        div {
            class: "ax-todo-step",
            class: if done { "ax-todo-done" },
            class: if dragging { "ax-todo-dragging" },
            class: if over { "ax-todo-drop-before" },
            role: "listitem",
            draggable: if armed { "true" } else { "false" },
            ondragstart: move |_| drag.set(StepDrag { armed: None, dragging: Some(sid), over: None }),
            ondragend: move |_| drag.set(StepDrag::default()),
            ondragover: move |_| {
                if drag.peek().dragging.is_some() && drag.peek().over != Some(Some(sid)) {
                    drag.write().over = Some(Some(sid));
                }
            },
            ondrop: move |_| {
                drop_step(app, drag, task_id, Some(sid));
            },
            span {
                class: "ax-todo-step-handle",
                title: "Drag to reorder",
                "aria-hidden": "true",
                onmousedown: move |_| drag.write().armed = Some(sid),
                onmouseup: move |_| drag.write().armed = None,
                Icon { name: "re_order_dots_vertical", size: 16 }
            }
            button {
                r#type: "button",
                class: "ax-todo-check ax-todo-check-small",
                class: if done { "ax-checked" },
                role: "checkbox",
                "aria-checked": if done { "true" } else { "false" },
                "aria-label": "{check_label}",
                title: "{check_label}",
                onclick: move |_| set_done(!done),
                span { class: "ax-todo-check-svg", dangerous_inner_html: CHECK_SVG }
            }
            input {
                class: "ax-todo-step-input",
                class: if done { "ax-todo-struck" },
                value: "{text}",
                "aria-label": "Step",
                maxlength: "255",
                "data-ax-late-keys": LateField::Step(task_id, sid).tag(),
                oninput: move |e| {
                    text.set(e.value());
                    edited.set(true);
                },
                onkeydown: move |e: KeyboardEvent| {
                    keys::stop_typing_keys(&e);
                    if e.key() == Key::Enter {
                        e.prevent_default();
                        save_key();
                    } else if e.key() == Key::Escape {
                        e.stop_propagation();
                        edited.set(false);
                        text.set(step.title.clone());
                    }
                },
                onblur: move |_| save_blur(),
            }
            MenuAnchor {
                class: "ax-icon-btn ax-small ax-todo-step-more",
                title: "Step options",
                items: menu,
                on_select: move |cmd: String| {
                    let s = store();
                    let r = match cmd.as_str() {
                        "toggle" => s.update_task_step(task_id, sid, None, Some(!done)).map(|_| ()),
                        "promote" => s.promote_task_step(task_id, sid).map(|_| ()),
                        "delete" => s.delete_task_step(task_id, sid),
                        _ => Ok(()),
                    };
                    if app.check(r).is_some() {
                        app.bump_tasks();
                    }
                },
                Icon { name: "more_horizontal", size: 16 }
            }
        }
    }
}

/// After a step is added from "Add step": the new step lands above the box
/// and pushes it down, out of the details pane at its bottom edge, so the
/// next key would jump the pane to centre it. Scroll it just into view now
/// and once more when the step has rendered (late on a busy machine), while
/// it still has focus.
const REVEAL_ADD_STEP_JS: &str = r#"(() => {
  const sel = ".ax-todo-detail .ax-todo-add-step input";
  const first = document.querySelector(sel);
  const box = first && first.closest(".ax-todo-detail-scroll");
  if (!box) return;
  const h0 = box.scrollHeight;
  const until = Date.now() + 3000;
  // Scrolling the pane by hand meanwhile ends it.
  let user = false;
  const mine = () => (user = true);
  box.addEventListener("wheel", mine, { once: true, passive: true });
  box.addEventListener("pointerdown", mine, { once: true, passive: true });
  const show = () => {
    const el = document.querySelector(sel);
    if (user || !el || document.activeElement !== el) return false;
    const row = el.closest(".ax-todo-add-step") || el;
    const r = row.getBoundingClientRect(), b = box.getBoundingClientRect();
    if (r.bottom > b.bottom + 1 || r.top < b.top - 1) row.scrollIntoView({ block: "nearest" });
    return true;
  };
  (function f() {
    if (show() && box.scrollHeight === h0 && Date.now() < until) return requestAnimationFrame(f);
    box.removeEventListener("wheel", mine);
    box.removeEventListener("pointerdown", mine);
  })();
})();"#;

#[component]
fn AddStep(task_id: TaskId, has_steps: bool, drag: Signal<StepDrag>) -> Element {
    let app = use_app();
    let mut drag = drag;
    let mut text = use_signal(String::new);
    let over = drag().dragging.is_some() && drag().over == Some(None);
    // True when the step was added (the page then empties the input).
    let mut add = move || -> bool {
        let t = text();
        if !t.trim().is_empty() && app.check(store().add_task_step(task_id, &t)).is_some() {
            text.set(String::new());
            app.bump_tasks();
            // The new step pushes the box down: keep it in view.
            document::eval(REVEAL_ADD_STEP_JS);
            return true;
        }
        false
    };
    // Removed without a blur (leaving To Do with Ctrl+1 or Ctrl+2): add
    // what was typed, as leaving it by a click does. The task may be gone
    // with the card, so quietly.
    use_drop(move || {
        if let Ok(t) = text.try_peek().map(|t| t.clone())
            && !t.trim().is_empty()
            && store().add_task_step(task_id, &t).is_ok()
        {
            app.bump_tasks();
        }
    });
    rsx! {
        label {
            class: "ax-todo-add-step",
            class: if over { "ax-todo-drop-before" },
            ondragover: move |_| {
                if drag.peek().dragging.is_some() && drag.peek().over != Some(None) {
                    drag.write().over = Some(None);
                }
            },
            ondrop: move |_| {
                drop_step(app, drag, task_id, None);
            },
            Icon { name: "add", size: 16 }
            input {
                class: "ax-todo-step-input",
                placeholder: if has_steps { "Next step" } else { "Add step" },
                "aria-label": "Add step",
                "data-ax-enter-clears": "true",
                maxlength: "255",
                value: "{text}",
                oninput: move |e| text.set(e.value()),
                onkeydown: move |e: KeyboardEvent| {
                    keys::stop_typing_keys(&e);
                    // Taken only when the step was added: a failed add
                    // keeps the text for a retry.
                    if e.key() == Key::Enter {
                        if add() {
                            e.prevent_default();
                        }
                    } else if e.key() == Key::Escape && !text().is_empty() {
                        e.stop_propagation();
                        text.set(String::new());
                    }
                },
                onblur: move |_| {
                    add();
                },
            }
        }
    }
}

#[component]
fn MyDayCard(task: Task) -> Element {
    let app = use_app();
    let ctx = use_todo();
    let today = (ctx.today)();
    let include_due = ctx.prefs.read().due_today_in_my_day;
    let in_day = actions::in_my_day(&task, today, include_due);
    let id = task.id;
    rsx! {
        div { class: "ax-todo-detail-card",
            if in_day {
                div { class: "ax-todo-detail-row ax-todo-set",
                    Icon { name: "weather_sunny" }
                    span { class: "ax-grow", "Added to My Day" }
                    IconButton { icon: "dismiss", title: "Remove from My Day", small: true, onclick: move |_| actions::set_my_day(app, ctx, &[id], false) }
                }
            } else {
                button {
                    r#type: "button",
                    class: "ax-todo-detail-row",
                    onclick: move |_| actions::set_my_day(app, ctx, &[id], true),
                    Icon { name: "weather_sunny" }
                    span { class: "ax-grow", "Add to My Day" }
                }
            }
        }
    }
}

#[component]
fn DatesCard(task: Task) -> Element {
    let app = use_app();
    let ctx = use_todo();
    let today = (ctx.today)();
    let h24 = app.settings.read().use_24h_time;
    let id = task.id;
    let run = move |cmd: String| actions::run(app, ctx, Target::Tasks(vec![id]), &cmd);
    let reminder = task.reminder.map(|r| (dates::reminder_lines(r, today, h24), r < Utc::now()));
    let due = task.due.map(|d| (dates::due_pane_label(d, today), dates::due_state(d, today)));
    let repeat = task.recurrence.as_ref().map(|r| (dates::repeat_label(r), dates::repeat_description(r, task.due.unwrap_or(today))));
    rsx! {
        div { class: "ax-todo-detail-card ax-todo-dates",
            div { class: "ax-todo-detail-line",
                MenuAnchor {
                    class: if reminder.is_some() { "ax-todo-detail-row ax-todo-set" } else { "ax-todo-detail-row" },
                    title: "Remind me",
                    items: menus::reminder_menu(today, reminder.is_some(), h24),
                    on_select: run,
                    Icon { name: "alert" }
                    if let Some(((line1, line2), past)) = &reminder {
                        span { class: "ax-grow ax-todo-two-line", class: if *past { "ax-todo-past" },
                            span { "{line1}" }
                            span { class: "ax-todo-second", "{line2}" }
                        }
                    } else {
                        span { class: "ax-grow", "Remind me" }
                    }
                }
                if reminder.is_some() {
                    IconButton { icon: "dismiss", title: "Remove reminder", small: true, onclick: move |_| actions::set_reminder(app, &[id], None) }
                }
            }
            div { class: "ax-todo-detail-line",
                MenuAnchor {
                    class: match &due {
                        Some((_, DueState::Overdue)) => "ax-todo-detail-row ax-todo-set ax-todo-overdue",
                        Some(_) => "ax-todo-detail-row ax-todo-set",
                        None => "ax-todo-detail-row",
                    },
                    title: "Add due date",
                    items: menus::due_menu(today, due.is_some()),
                    on_select: run,
                    Icon { name: "calendar_ltr" }
                    span { class: "ax-grow", if let Some((label, _)) = &due { "{label}" } else { "Add due date" } }
                }
                if due.is_some() {
                    IconButton { icon: "dismiss", title: "Remove due date", small: true, onclick: move |_| actions::set_due(app, &[id], None) }
                }
            }
            div { class: "ax-todo-detail-line",
                MenuAnchor {
                    class: if repeat.is_some() { "ax-todo-detail-row ax-todo-set" } else { "ax-todo-detail-row" },
                    title: repeat.as_ref().map(|r| r.1.clone()).unwrap_or_else(|| "Repeat".to_string()),
                    items: menus::repeat_menu(repeat.is_some()),
                    on_select: run,
                    Icon { name: "arrow_repeat_all" }
                    span { class: "ax-grow", if let Some((label, _)) = &repeat { "{label}" } else { "Repeat" } }
                }
                if repeat.is_some() {
                    IconButton { icon: "dismiss", title: "Remove recurrence", small: true, onclick: move |_| actions::set_repeat(app, &[id], None, today) }
                }
            }
        }
    }
}

#[component]
fn CategoryCard(task: Task) -> Element {
    let app = use_app();
    let ctx = use_todo();
    let id = task.id;
    let cats = store().categories().unwrap_or_default();
    let color_of = |name: &str| cats.iter().find(|c| c.name == name).map(|c| c.color.clone()).unwrap_or_else(|| "gray".into());
    let chosen: Vec<(String, String)> = task.categories.iter().map(|c| (c.clone(), color_of(c))).collect();
    rsx! {
        div { class: "ax-todo-detail-card",
            MenuAnchor {
                class: "ax-todo-detail-row",
                title: "Pick a category",
                items: menus::category_menu(&cats, &task.categories),
                on_select: move |cmd: String| actions::run(app, ctx, Target::Tasks(vec![id]), &cmd),
                Icon { name: "tag" }
                span { class: "ax-grow", "Pick a category" }
            }
            if !chosen.is_empty() {
                div { class: "ax-todo-detail-cats",
                    for (name, color) in chosen {
                        CategoryChip {
                            key: "{name}",
                            name: name.clone(),
                            color,
                            on_remove: move |_| actions::toggle_category(app, &[id], &name),
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn LinkedEmail(message_id: MessageId) -> Element {
    let app = use_app();
    let _ = app.mail_rev();
    let msg = store().message(message_id).ok().flatten();
    rsx! {
        div { class: "ax-todo-detail-card ax-todo-linked",
            if let Some(m) = msg {
                div { class: "ax-row", style: "gap:8px",
                    Avatar { name: m.from.name.clone(), email: m.from.address.clone(), size: 24 }
                    span { class: "ax-grow ax-ellipsis ax-semibold", "{m.from.display()}" }
                }
                div { class: "ax-todo-linked-subject", "{m.subject}" }
                div { class: "ax-todo-linked-preview", "{m.preview}" }
                button {
                    r#type: "button",
                    class: "ax-todo-link",
                    onclick: move |_| open_email(app, message_id),
                    Icon { name: "open", size: 16 }
                    "Open email"
                }
            } else {
                div { class: "ax-muted", "This email is no longer available" }
            }
        }
    }
}

fn open_email(app: AppState, id: MessageId) {
    app.navigate(NavRequest::Message(id));
}

#[component]
fn NoteCard(task: Task) -> Element {
    let app = use_app();
    let id = task.id;
    let mut note = use_signal(|| task.notes.clone());
    let mut saved = use_signal(|| task.notes.clone());
    if *saved.peek() != task.notes && *note.peek() == *saved.peek() {
        note.set(task.notes.clone());
        saved.set(task.notes.clone());
    }
    let mut save = move |text: String| {
        if text != *saved.peek() {
            saved.set(text.clone());
            if app.check(store().set_task_notes(id, &text)).is_some() {
                app.bump_tasks();
            }
        }
    };
    // Removed without a blur (leaving To Do with a key): keep the note. The
    // task may have been deleted, so quietly.
    use_drop(move || {
        if let (Ok(n), Ok(s)) = (note.try_peek().map(|n| n.clone()), saved.try_peek().map(|s| s.clone()))
            && n != s
            && store().set_task_notes(id, &n).is_ok()
        {
            app.bump_tasks();
        }
    });
    crate::kit::dom::use_late_tag(LateField::Note(id).tag());
    let updated = task.notes_updated_at.filter(|_| !task.notes.is_empty()).map(|t| dates::updated_label(t, Utc::now()));
    rsx! {
        div { class: "ax-todo-detail-card ax-todo-note",
            textarea {
                class: "ax-todo-note-input",
                placeholder: "Add a note",
                "aria-label": "Add a note",
                "data-ax-late-keys": LateField::Note(id).tag(),
                value: "{note}",
                oninput: move |e| {
                    let v = e.value();
                    note.set(v.clone());
                    // Autosave after a second without typing, as this
                    // card's task (`use_drop` saves when the card goes).
                    crate::kit::after(1000, move || {
                        if *note.peek() == v {
                            save(v);
                        }
                    });
                },
                onkeydown: move |e: KeyboardEvent| {
                    keys::stop_typing_keys(&e);
                    if e.key() == Key::Escape {
                        e.stop_propagation();
                        actions::focus_row(id, false);
                    }
                },
                onblur: move |_| save(note()),
            }
            if let Some(u) = updated {
                div { class: "ax-todo-note-updated", "{u}" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::sync::Arc;

    use super::*;
    use crate::state::provide_app_state;

    /// Keys the page still had for a title, note or step after its card
    /// went (another task was clicked) are saved; a gone task is left be.
    #[test]
    fn late_keys_are_saved_where_the_card_would_have() {
        thread_local! {
            static APP: Cell<Option<AppState>> = const { Cell::new(None) };
        }
        fn root() -> Element {
            let app = provide_app_state();
            APP.with(|a| a.set(Some(app)));
            rsx! {}
        }
        crate::state::init(Arc::new(axentra_core::Store::open_in_memory().unwrap()), None);
        let mut dom = VirtualDom::new(root);
        dom.rebuild_in_place();
        let app = APP.with(|a| a.get()).expect("the app state");
        dom.in_runtime(|| {
            let s = store();
            // Its own account and list (the store is shared by the tests).
            let mut acct = Account { kind: AccountKind::Local, email: "late-keys@example.test".into(), ..Default::default() };
            let account_id = s.add_account(&mut acct).unwrap();
            let list = s.insert_task_list(&mut TaskList { name: "Late keys".into(), account_id, ..Default::default() }).unwrap();
            let mut t = Task { title: "Review Q4 launch plan rena".into(), list_id: list, ..Default::default() };
            let id = s.insert_task_at(&mut t, false).unwrap();
            let sid = s.add_task_step(id, "Book the ro").unwrap();
            let rest = |f: LateField| f.tag().strip_prefix("todo:").unwrap().to_string();
            late_typed(app, &rest(LateField::Title(id)), "Review Q4 launch plan rena ra170");
            late_typed(app, &rest(LateField::Note(id)), "call the venue first");
            late_typed(app, &rest(LateField::Step(id, sid)), "Book the room");
            let t = s.task(id).unwrap().unwrap();
            assert_eq!(t.title, "Review Q4 launch plan rena ra170");
            assert_eq!(t.notes, "call the venue first");
            assert_eq!(t.steps.iter().map(|x| x.title.as_str()).collect::<Vec<_>>(), ["Book the room"]);
            // An emptied title is not saved; a task deleted meanwhile is not brought back.
            late_typed(app, &rest(LateField::Title(id)), "   ");
            assert_eq!(s.task(id).unwrap().unwrap().title, "Review Q4 launch plan rena ra170");
            s.delete_task(id).ok();
            late_typed(app, &rest(LateField::Title(id)), "Brought back");
            late_typed(app, &rest(LateField::Note(id)), "Brought back");
            assert!(s.task(id).unwrap().is_none_or(|t| t.title != "Brought back" && t.notes != "Brought back"));
        });
    }

    /// The title autosave stores "… milk" for a field showing "… milk ":
    /// that must not be written back into the field being typed in (the
    /// next word ran into the last: "milkand eggs"); a rename elsewhere is.
    #[test]
    fn the_title_field_keeps_its_spaces_after_an_autosave() {
        let typed = "Send interview feedback milk ";
        let stored = actions::task_title(typed);
        assert_eq!(stored, "Send interview feedback milk");
        assert!(!follows_elsewhere(typed, &stored, &stored), "its own autosave");
        assert!(!follows_elsewhere(typed, typed, &stored), "saved as typed, stored trimmed");
        assert!(!follows_elsewhere("Send  interview feedback", "Send interview feedback", "Send interview feedback"));
        assert!(follows_elsewhere("Oat milk", "Oat milk", "Oat milk renamed"), "renamed from its row");
        assert!(!follows_elsewhere("Oat milk and", "Oat milk", "Oat milk renamed"), "being edited: the typing wins");
    }

    #[test]
    fn late_field_tags_round_trip() {
        for f in [LateField::Title(42), LateField::Note(7), LateField::Step(42, 9)] {
            let tag = f.tag();
            let rest = tag.strip_prefix("todo:").unwrap();
            assert_eq!(LateField::parse(rest), Some(f), "{tag}");
        }
        for bad in ["", "title", "title:x", "step:1", "note:1:2", "title:1:2", "other:1"] {
            assert_eq!(LateField::parse(bad), None, "{bad}");
        }
    }
}
