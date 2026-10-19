//! Task operations shared by rows, the detail pane, menus, the ribbon and
//! the keyboard. Each one writes to the store and bumps `tasks_rev`.

use axentra_core::*;
use chrono::{DateTime, Local, NaiveDate, Utc};
use dioxus::prelude::*;

use super::ctx::{Confirm, Draft, Picker, PickerKind, Target, TodoCtx};
use super::dates;
use crate::state::{AppState, NavRequest, UndoAction, store};

/// Original two-tone completion chime (~250 ms, about -12 dBFS), synthesised
/// with Web Audio so no sound asset ships.
const CHIME_JS: &str = r#"
try {
  const C = window.AudioContext || window.webkitAudioContext;
  const a = window.__axTodoAudio || (window.__axTodoAudio = new C());
  if (a.state === "suspended") a.resume();
  const t = a.currentTime, g = a.createGain();
  g.connect(a.destination);
  g.gain.setValueAtTime(0.0001, t);
  g.gain.exponentialRampToValueAtTime(0.18, t + 0.012);
  g.gain.exponentialRampToValueAtTime(0.0001, t + 0.25);
  [[880, 0], [1318.5, 0.07]].forEach(([f, d]) => {
    const o = a.createOscillator();
    o.type = "sine";
    o.frequency.setValueAtTime(f, t + d);
    o.connect(g);
    o.start(t + d);
    o.stop(t + 0.26);
  });
} catch (e) {}
"#;

/// Completion animation length before the task moves to "Completed".
const COMPLETE_MS: u64 = 600;

pub fn chime(ctx: TodoCtx) {
    if ctx.prefs.peek().completion_sound {
        document::eval(CHIME_JS);
    }
}

/// Focus an element by CSS selector (rows, inputs), scrolling it into view.
pub fn focus(selector: &str) {
    let sel = serde_json::to_string(selector).unwrap_or_default();
    document::eval(&format!(
        "const el = document.querySelector({sel}); if (el) {{ el.focus({{ preventScroll: true }}); el.scrollIntoView({{ block: 'nearest' }}); }}"
    ));
}

/// Focus an element after the next render (e.g. a pane that is opening).
pub fn focus_later(selector: &str, ms: u32) {
    let sel = serde_json::to_string(selector).unwrap_or_default();
    document::eval(&format!(
        "setTimeout(() => {{ const el = document.querySelector({sel}); if (el) {{ el.focus(); el.select?.(); }} }}, {ms});"
    ));
}

/// Rename a task (newlines become spaces; empty titles are ignored).
pub fn rename_task(app: AppState, id: TaskId, title: &str) {
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() {
        return;
    }
    edit(app, &[id], |t| t.title = title.clone());
}

/// Focus a task row (or its completion circle).
pub fn focus_row(id: TaskId, circle: bool) {
    if circle {
        focus(&format!("[data-task-row=\"{id}\"] .ax-todo-check"));
    } else {
        focus(&format!("[data-task-row=\"{id}\"]"));
    }
}

/// After a row's context menu closes, give the focus back to the selected
/// row unless the command opened a picker or dialog (keyboard spec 12.4).
pub fn refocus_selection(ctx: TodoCtx) {
    ctx.spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
        if !ctx.overlay_open()
            && let Some(&id) = ctx.selected.peek().last()
        {
            focus_row(id, false);
        }
    });
}

/// After completing `id` from its circle or the keyboard, the next visible
/// task (or the one above, at the end) takes the selection, unless the
/// detail pane shows `id`, and its circle gets the focus (spec 3.8).
pub fn advance_from(mut ctx: TodoCtx, id: TaskId) {
    let order = ctx.order.peek().clone();
    let Some(i) = order.iter().position(|x| *x == id) else { return };
    let Some(next) = order.get(i + 1).or_else(|| i.checked_sub(1).and_then(|j| order.get(j))).copied() else { return };
    if *ctx.detail.peek() != Some(id) && ctx.selected.peek().contains(&id) {
        ctx.selected.set(vec![next]);
    }
    focus_row(next, true);
}

/// An action may take `ids` out of the open view (Remove from My Day in My
/// Day, unstar in Important, move to another list). Once the list has
/// re-rendered, if they are gone, the row that took their place gets the
/// selection and the focus, so the keyboard user stays in the list. Skipped
/// while the detail pane shows one of them (the focus is there).
pub fn follow_focus(mut ctx: TodoCtx, ids: &[TaskId]) {
    let before = ctx.order.peek().clone();
    let Some(i) = before.iter().position(|id| ids.contains(id)) else { return };
    if ctx.detail.peek().is_some_and(|d| ids.contains(&d)) {
        return;
    }
    let ids = ids.to_vec();
    ctx.spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        let now = ctx.order.peek().clone();
        if ids.iter().any(|id| now.contains(id)) {
            return;
        }
        let next = before[i..].iter().find(|id| now.contains(id)).or_else(|| before[..i].iter().rev().find(|id| now.contains(id))).copied();
        if let Some(n) = next {
            if ctx.selected.peek().iter().all(|id| ids.contains(id)) {
                ctx.selected.set(vec![n]);
            }
            focus_row(n, false);
        }
    });
}

/// Complete / reopen one task from its circle. Completing plays the chime
/// and animates for 600 ms before the store changes; unchecking inside that
/// window cancels it.
pub fn complete(app: AppState, mut ctx: TodoCtx, id: TaskId, done: bool) {
    if !done {
        if ctx.completing.peek().contains(&id) {
            ctx.completing.write().retain(|x| *x != id);
            return;
        }
        if app.check(store().set_task_completed(id, false)).is_some() {
            app.bump_tasks();
        }
        return;
    }
    chime(ctx);
    ctx.say("Task completed");
    if *ctx.reduced_motion.peek() {
        finish_complete(app, ctx, id);
        return;
    }
    ctx.completing.write().push(id);
    ctx.spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(COMPLETE_MS)).await;
        if ctx.completing.peek().contains(&id) {
            ctx.completing.write().retain(|x| *x != id);
            finish_complete(app, ctx, id);
        }
    });
}

fn finish_complete(app: AppState, _ctx: TodoCtx, id: TaskId) {
    if app.check(store().set_task_completed(id, true)).is_some() {
        app.bump_tasks();
    }
}

/// Complete / reopen several tasks at once (Ctrl+D, menus, ribbon). A
/// single task animates like a circle click.
pub fn set_completed(app: AppState, ctx: TodoCtx, ids: &[TaskId], done: bool) {
    if let [id] = ids {
        complete(app, ctx, *id, done);
        return;
    }
    let s = store();
    for &id in ids {
        app.check(s.set_task_completed(id, done));
    }
    if done && !ids.is_empty() {
        chime(ctx);
        ctx.say(format!("{} tasks completed", ids.len()));
    }
    app.bump_tasks();
}

/// Load, change and save each task.
fn edit(app: AppState, ids: &[TaskId], f: impl Fn(&mut Task)) {
    let s = store();
    for &id in ids {
        if let Some(mut t) = app.check(s.task(id)).flatten() {
            f(&mut t);
            app.check(s.update_task(&t));
        }
    }
    app.bump_tasks();
}

pub fn set_important(app: AppState, ids: &[TaskId], on: bool) {
    let s = store();
    for &id in ids {
        app.check(s.set_task_important(id, on));
    }
    app.bump_tasks();
}

/// Add to / remove from today's My Day.
pub fn set_my_day(app: AppState, ctx: TodoCtx, ids: &[TaskId], add: bool) {
    let s = store();
    let today = *ctx.today.peek();
    for &id in ids {
        if add {
            app.check(s.set_task_my_day(id, Some(today)));
        } else {
            app.check(s.remove_task_from_my_day(id, today));
        }
    }
    ctx.say(if add { "Added to My Day" } else { "Removed from My Day" });
    app.bump_tasks();
    if !add {
        follow_focus(ctx, ids);
    }
}

/// Ctrl+T: add the tasks to My Day, or remove them when all are in it.
pub fn toggle_my_day(app: AppState, ctx: TodoCtx, ids: &[TaskId]) {
    if ids.is_empty() {
        return;
    }
    let today = *ctx.today.peek();
    let include_due = ctx.prefs.peek().due_today_in_my_day;
    let all_in = ids.iter().all(|&id| store().task(id).ok().flatten().is_some_and(|t| in_my_day(&t, today, include_due)));
    set_my_day(app, ctx, ids, !all_in);
}

/// Whether a task shows in today's My Day.
pub fn in_my_day(t: &Task, today: NaiveDate, include_due_today: bool) -> bool {
    t.my_day == Some(today) || (include_due_today && t.due == Some(today) && t.my_day_hidden != Some(today))
}

pub fn set_due(app: AppState, ids: &[TaskId], due: Option<NaiveDate>) {
    edit(app, ids, |t| t.due = due);
}

pub fn set_reminder(app: AppState, ids: &[TaskId], at: Option<DateTime<Utc>>) {
    edit(app, ids, |t| t.reminder = at);
}

/// Setting a repeat on a task without a due date also makes it due today.
pub fn set_repeat(app: AppState, ids: &[TaskId], rule: Option<Recurrence>, today: NaiveDate) {
    edit(app, ids, |t| {
        if rule.is_some() && t.due.is_none() {
            t.due = Some(today);
        }
        t.recurrence = rule.clone();
    });
}

/// Add the category to every task lacking it, or remove it when all have it.
pub fn toggle_category(app: AppState, ids: &[TaskId], name: &str) {
    let s = store();
    let all_have = ids.iter().all(|&id| s.task(id).ok().flatten().is_some_and(|t| t.categories.iter().any(|c| c == name)));
    edit(app, ids, |t| {
        if all_have {
            t.categories.retain(|c| c != name);
        } else if !t.categories.iter().any(|c| c == name) {
            t.categories.push(name.to_string());
        }
    });
}

pub fn move_to(app: AppState, ctx: TodoCtx, ids: &[TaskId], list_id: TaskListId) {
    if app.check(store().move_tasks_to_list(ids, list_id)).is_some() {
        let name = store().task_list(list_id).ok().flatten().map(|l| l.name).unwrap_or_default();
        ctx.say(format!("Moved to {name}"));
        app.bump_tasks();
        follow_focus(ctx, ids);
    }
}

/// Delete with the "Confirm before deleting" setting: ask first, or delete
/// at once with an Undo toast. Several tasks are always confirmed because
/// the Undo toast restores one task.
pub fn request_delete(app: AppState, mut ctx: TodoCtx, ids: Vec<TaskId>) {
    if ids.is_empty() {
        return;
    }
    if ctx.prefs.peek().confirm_delete || ids.len() > 1 {
        ctx.confirm.set(Some(Confirm::DeleteTasks(ids)));
    } else {
        delete_now(app, ctx, &ids, true);
    }
}

pub fn delete_now(app: AppState, mut ctx: TodoCtx, ids: &[TaskId], undo: bool) {
    let s = store();
    // The row that takes the deleted rows' place: the next one below, or
    // the last one above.
    let order = ctx.order.peek().clone();
    let first = order.iter().position(|id| ids.contains(id));
    let successor = first.and_then(|i| {
        order[i..].iter().find(|id| !ids.contains(id)).or_else(|| order[..i].iter().rev().find(|id| !ids.contains(id))).copied()
    });
    let mut last = None;
    for &id in ids {
        let Some(t) = app.check(s.task(id)).flatten() else { continue };
        if app.check(s.delete_task(id)).is_some() {
            last = Some(t);
        }
    }
    ctx.selected.write().retain(|id| !ids.contains(id));
    if ctx.detail.peek().is_some_and(|d| ids.contains(&d)) {
        ctx.detail.set(None);
    }
    if ctx.selected.peek().is_empty()
        && let Some(next) = successor {
            ctx.selected.set(vec![next]);
            focus_later(&format!("[data-task-row=\"{next}\"]"), 60);
        }
    if let Some(t) = last {
        if undo && ids.len() == 1 {
            app.toast_with_undo("Task deleted", UndoAction::RestoreTask(t));
        } else {
            ctx.say(if ids.len() == 1 { "Task deleted".to_string() } else { format!("{} tasks deleted", ids.len()) });
        }
    }
    app.bump_tasks();
}

/// Open the date/time picker or custom-repeat dialog at the last pointer
/// position.
pub fn open_picker(mut ctx: TodoCtx, kind: PickerKind, target: Target) {
    let (x, y) = *ctx.pointer.peek();
    ctx.picker.set(Some(Picker { kind, target, x, y }));
}

/// Run a menu / ribbon command id on a target. Ids: `due:*`, `rem:*`,
/// `rep:*`, `cat:<name>`, `move:<list>`, `myday:add|remove`,
/// `imp:on|off`, `done:on|off`, `delete`, `open`, `open_mail:<id>`.
pub fn run(app: AppState, mut ctx: TodoCtx, target: Target, id: &str) {
    let today = *ctx.today.peek();
    let (group, arg) = id.split_once(':').unwrap_or((id, ""));
    // Values the due / reminder / repeat commands resolve to.
    let due = |arg: &str| -> Option<Option<NaiveDate>> {
        Some(match arg {
            "today" => Some(today),
            "tomorrow" => Some(today + chrono::Duration::days(1)),
            "next_week" => Some(dates::next_week(today)),
            "none" => None,
            _ => return None,
        })
    };
    let reminder = |arg: &str| -> Option<Option<DateTime<Utc>>> {
        Some(match arg {
            "later" => dates::reminder_later_today(Local::now()),
            "tomorrow" => Some(dates::reminder_tomorrow(today)),
            "next_week" => Some(dates::reminder_next_week(today)),
            "none" => None,
            _ => return None,
        })
    };
    match (group, arg, &target) {
        ("due", "pick", _) => open_picker(ctx, PickerKind::Due, target.clone()),
        ("rem", "pick", _) => open_picker(ctx, PickerKind::Reminder, target.clone()),
        ("rep", "custom", _) => open_picker(ctx, PickerKind::CustomRepeat, target.clone()),
        ("due", a, Target::Tasks(ids)) => {
            if let Some(d) = due(a) {
                set_due(app, ids, d);
                follow_focus(ctx, ids);
            }
        }
        ("due", a, Target::NewTask) => {
            if let Some(d) = due(a) {
                ctx.draft.write().due = d;
            }
        }
        ("rem", a, Target::Tasks(ids)) => {
            if let Some(r) = reminder(a) {
                set_reminder(app, ids, r);
            }
        }
        ("rem", a, Target::NewTask) => {
            if let Some(r) = reminder(a) {
                ctx.draft.write().reminder = r;
            }
        }
        ("rep", a, Target::Tasks(ids)) => set_repeat(app, ids, dates::repeat_preset(a), today),
        ("rep", a, Target::NewTask) => {
            let rule = dates::repeat_preset(a);
            let mut d = ctx.draft.write();
            if rule.is_some() && d.due.is_none() {
                d.due = Some(today);
            }
            d.repeat = rule;
        }
        ("cat", "manage", _) => {
            let mut dialog = app.settings_dialog;
            dialog.set(Some(crate::state::SettingsPage::Accounts));
        }
        ("cat", name, Target::Tasks(ids)) => toggle_category(app, ids, name),
        ("move", list, Target::Tasks(ids)) => {
            if let Ok(list) = list.parse() {
                move_to(app, ctx, ids, list);
            }
        }
        ("myday", a, Target::Tasks(ids)) => set_my_day(app, ctx, ids, a == "add"),
        ("imp", a, Target::Tasks(ids)) => {
            set_important(app, ids, a == "on");
            follow_focus(ctx, ids);
        }
        ("done", a, Target::Tasks(ids)) => {
            set_completed(app, ctx, ids, a == "on");
            match ids.as_slice() {
                [id] if a == "on" => advance_from(ctx, *id),
                _ => follow_focus(ctx, ids),
            }
        }
        ("delete", _, Target::Tasks(ids)) => request_delete(app, ctx, ids.clone()),
        ("open", _, Target::Tasks(ids)) => {
            if let Some(&id) = ids.first() {
                ctx.open(id);
            }
        }
        ("open_mail", m, _) => {
            if let Ok(m) = m.parse() {
                app.navigate(NavRequest::Message(m));
            }
        }
        _ => {}
    }
}

/// A new task titled `title` with the attributes of the view it is added in
/// (smart lists store into "Tasks", spec 3.3.1) and the "Add a task" draft.
fn new_task(title: &str, list_id: Option<TaskListId>, view: super::ctx::View, draft: &Draft, today: NaiveDate) -> Task {
    use super::ctx::View;
    let mut t = Task { title: title.trim().to_string(), list_id: list_id.unwrap_or(0), ..Default::default() };
    match view {
        View::Smart(SmartList::MyDay) => {
            t.my_day = Some(today);
            t.my_day_at = Some(Utc::now());
        }
        View::Smart(SmartList::Important) => t.is_important = true,
        View::Smart(SmartList::Planned) => t.due = Some(today),
        _ => {}
    }
    if draft.due.is_some() {
        t.due = draft.due;
    }
    t.reminder = draft.reminder;
    if let Some(rule) = &draft.repeat {
        t.due = t.due.or(Some(today));
        t.recurrence = Some(rule.clone());
    }
    t
}

/// Create a task from the "Add a task" bar in the current view. Returns
/// the new id.
pub fn add_task(app: AppState, ctx: TodoCtx, title: &str, list_id: Option<TaskListId>, view: super::ctx::View) -> Option<TaskId> {
    add_tasks(app, ctx, &[title.to_string()], list_id, view).into_iter().next()
}

/// Create several tasks at once (a multi-line paste), in the given order,
/// each with the draft's due date, reminder and repeat. Returns their ids.
pub fn add_tasks(app: AppState, mut ctx: TodoCtx, titles: &[String], list_id: Option<TaskListId>, view: super::ctx::View) -> Vec<TaskId> {
    let today = *ctx.today.peek();
    let draft: Draft = ctx.draft.peek().clone();
    let on_top = ctx.prefs.peek().add_on_top;
    let s = store();
    // On top, insert the last line first so the lines keep their order.
    let ordered: Vec<&String> = if on_top { titles.iter().rev().collect() } else { titles.iter().collect() };
    let mut ids = Vec::new();
    for title in ordered.into_iter().filter(|t| !t.trim().is_empty()) {
        let mut t = new_task(title, list_id, view, &draft, today);
        match app.check(s.insert_task_at(&mut t, on_top)) {
            Some(id) => ids.push(id),
            None => break,
        }
    }
    if !ids.is_empty() {
        ctx.draft.set(Draft::default());
        ctx.say(if ids.len() == 1 { "Task added".to_string() } else { format!("{} tasks added", ids.len()) });
        app.bump_tasks();
    }
    ids
}
