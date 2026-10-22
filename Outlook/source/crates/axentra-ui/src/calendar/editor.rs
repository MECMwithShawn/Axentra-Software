//! Full event editor: an in-app panel over the calendar with its own
//! toolbar (Save / Send, Discard / Delete / Cancel, Show as, Reminder,
//! Recurring, Categorize, Time zones, …). Attendees of someone else's
//! meeting get the read-only view with Accept / Tentative / Decline.

use axentra_core::*;
use axentra_core::Event;
use chrono::{Duration, Local, NaiveDate, NaiveTime};
use dioxus::prelude::*;

use super::actions::{can_edit_calendar, join_url, open_external, request_delete};
use super::colors::ramp;
use super::fields::{AttendeePicker, DateField, TimeField, commit_typed, valid_address};
use super::popovers::{set_end_time, set_start_time, toggle_all_day};
use super::repeat::{RepeatDialog, RepeatMenu};
use super::state::*;
use super::text::{REMINDER_CHOICES, SHOW_AS_CHOICES, reminder_label, show_as_from_id, show_as_id, show_as_label, time_zone_label};
use crate::fmt;
use crate::kit::{Button, CategoryPill, Dialog, Icon, IconButton, MenuItem, Switch, Variant, keys};
use crate::shell::ribbon::{Command, Ribbon, RibbonTab};
use crate::state::{store, use_app};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Confirm {
    Discard,
    ResetExceptions,
}

/// What a save does: save, or send (validates the title and addresses).
/// The toolbar's first button does what it says when pressed: Send once
/// the event has attendees, else Save (an address still typed in a box is
/// added either way).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SaveAs {
    Save,
    Send,
}

/// Plain text from the description box as simple HTML paragraphs.
pub fn text_to_html(text: &str) -> String {
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    text.trim()
        .split("\n\n")
        .filter(|p| !p.trim().is_empty())
        .map(|p| format!("<p>{}</p>", esc(p.trim()).replace('\n', "<br>")))
        .collect()
}

#[component]
pub fn EventEditor(req: EditorRequest) -> Element {
    let app = use_app();
    let mut cal = use_cal();
    let s = app.settings.read().clone();
    let h24 = s.use_24h_time;
    let mode = req.mode;
    // The event as opened and its description text, to detect changes.
    let original = use_signal(|| req.event.clone());
    let body0 = use_signal(|| axentra_core::html::html_to_text(&req.event.body_html).trim().to_string());
    let mut draft = use_signal(|| req.event.clone());
    let mut body = use_signal(|| body0.peek().clone());
    let mut confirm = use_signal(|| None::<Confirm>);
    let mut error = use_signal(|| None::<String>);
    let mut show_optional = use_signal(|| req.event.attendees.iter().any(|a| a.kind == AttendeeKind::Optional));
    let mut show_tz = use_signal(|| false);
    let mut repeat_open = use_signal(|| false);
    let mut end_date_open = use_signal(|| false);
    // "End time was adjusted" after an end at or before the start.
    let mut end_adjusted = use_signal(|| false);
    // What is typed in the attendee boxes and not added yet.
    let typed_required = use_signal(String::new);
    let typed_optional = use_signal(String::new);
    // Saves from the toolbar run as this form's tasks (see `on_command`).
    let scope = dioxus::core::current_scope_id();
    let alive = use_hook(|| CopyValue::new(()));

    let d = draft();
    let me = my_address();
    let organizer = i_organize(&d, me.as_ref());
    let is_new = mode == EditMode::New;
    let editable_cal = is_new || can_edit_calendar(d.calendar_id);
    let readonly = !organizer || !editable_cal;
    let invitation = !organizer && editable_cal;
    let dirty = d != *original.read() || body() != *body0.read();
    let has_attendees = !d.attendees.is_empty();
    let send = has_attendees && organizer && !d.is_cancelled;
    let cancelled = d.is_cancelled;
    let writable: Vec<Calendar> = store().calendars().unwrap_or_default().into_iter().filter(|c| c.can_edit).collect();
    let cats = store().categories().unwrap_or_default();
    let start_date = local_date(d.start);
    let end_date = if d.all_day { local_date(d.end) - Duration::days(1) } else { local_date(d.end - Duration::seconds(1)).max(start_date) };
    let st = d.start.with_timezone(&Local).time();
    let et = d.end.with_timezone(&Local).time();
    let multi_day = end_date != start_date;
    let length = fmt::duration_label(d.end - d.start);
    let calendar_color = writable.iter().find(|c| c.id == d.calendar_id).map(|c| ramp(&c.color).primary).unwrap_or_default();

    // ---------------------------------------------------------------- save
    let mut do_save = move |how: SaveAs, reset_ok: bool| {
        // An address typed in an attendee box without Enter, Tab or ";" is
        // added first (a click on Save adds it too, as the box loses focus).
        let added = commit_typed(draft, &[(AttendeeKind::Required, typed_required), (AttendeeKind::Optional, typed_optional)]);
        if added.iter().any(|a| !valid_address(&a.address)) {
            error.set(Some("Check the highlighted addresses".into()));
            return;
        }
        let send_it = how == SaveAs::Send;
        let mut e = draft.peek().clone();
        let original = original.peek().clone();
        let text = body.peek().clone();
        if text != *body0.peek() {
            e.body_html = text_to_html(&text);
        }
        if e.end < e.start || (!e.all_day && e.end == e.start && e.start != original.start) {
            error.set(Some("The end time must be after the start time.".into()));
            return;
        }
        if send_it {
            if e.title.trim().is_empty() {
                error.set(Some("Add a title before sending.".into()));
                return;
            }
            if e.attendees.iter().any(|a| !valid_address(&a.email.address)) {
                error.set(Some("Check the highlighted addresses".into()));
                return;
            }
        }
        e.title = e.title.trim().to_string();
        let addrs: Vec<EmailAddress> = e.attendees.iter().map(|a| a.email.clone()).collect();
        let r = match mode {
            EditMode::New => store().insert_event(&mut e).map(|_| ()),
            EditMode::Single => store().update_event(&e),
            EditMode::Occurrence { series_id, original_start } => store().update_occurrence(series_id, original_start, &e).map(|_| ()),
            EditMode::Following { series_id, original_start } => store().split_series(series_id, original_start, &e).map(|_| ()),
            EditMode::Series => {
                let pattern_changed = e.recurrence != original.recurrence;
                let has_exceptions =
                    !original.exdates.is_empty() || store().exceptions_of(e.id).map(|x| !x.is_empty()).unwrap_or(false);
                if pattern_changed && has_exceptions && !reset_ok {
                    confirm.set(Some(Confirm::ResetExceptions));
                    return;
                }
                store().update_series(&e, pattern_changed && has_exceptions)
            }
        };
        if app.check(r).is_some() {
            let _ = store().record_recent_addresses(&addrs);
            app.bump_calendar();
            cal.editor.set(None);
            let s = app.settings.peek().clone();
            if !super::time_grid::days_peek(app, cal).contains(&local_date(e.start)) {
                cal.go_to(&s, local_date(e.start));
            }
            if send_it {
                app.toast(if is_new {
                    "Meeting saved. Invitations aren't sent from local calendars."
                } else {
                    "Meeting updated. Updates aren't sent from local calendars."
                });
            }
        }
    };
    let close = move || {
        let mut cal = cal;
        cal.editor.set(None);
    };
    let mut discard = move |dirty: bool| {
        let typed = !typed_required.peek().trim().is_empty() || !typed_optional.peek().trim().is_empty();
        if dirty || typed {
            confirm.set(Some(Confirm::Discard));
        } else {
            close();
        }
    };
    // The occurrence this form is about, for Delete / Cancel.
    let as_occurrence = move || -> Occurrence {
        let e = original.peek().clone();
        match mode {
            EditMode::Occurrence { series_id, original_start } | EditMode::Following { series_id, original_start } => {
                let master = store().event(series_id).ok().flatten().unwrap_or_else(|| e.clone());
                Occurrence { start: e.start, end: e.end, original_start, color: String::new(), event: master }
            }
            _ => Occurrence { start: e.start, end: e.end, original_start: e.original_start.unwrap_or(e.start), color: String::new(), event: e },
        }
    };
    let respond_and_close = move |r: ResponseStatus| {
        // Keep personal changes (reminder, Show as, categories) made in the form.
        let e = draft.peek().clone();
        let _ = store().modify_event(e.id, |x| {
            x.reminder_minutes = e.reminder_minutes;
            x.categories = e.categories.clone();
            x.is_private = e.is_private;
        });
        let occ = as_occurrence();
        super::actions::respond(app, cal, &occ, r);
        close();
    };

    // ---------------------------------------------------------------- toolbar
    let show_as_menu: Vec<MenuItem> = SHOW_AS_CHOICES
        .iter()
        .map(|sa| MenuItem::new(format!("showas:{}", show_as_id(*sa)), show_as_label(*sa)).checked(d.show_as == *sa))
        .chain([MenuItem::divider(), MenuItem::new("private", "Private").icon("lock_closed").checked(d.is_private)])
        .collect();
    let mut reminder_menu: Vec<MenuItem> = REMINDER_CHOICES
        .iter()
        .map(|m| MenuItem::new(format!("rem:{}", m.map(|v| v.to_string()).unwrap_or_else(|| "none".into())), reminder_label(*m)).checked(d.reminder_minutes == *m))
        .collect();
    if !REMINDER_CHOICES.contains(&d.reminder_minutes) {
        reminder_menu.push(MenuItem::new("rem:keep", reminder_label(d.reminder_minutes)).checked(true));
    }
    let mut cat_menu: Vec<MenuItem> = cats
        .iter()
        .map(|c| MenuItem::new(format!("cat:{}", c.name), c.name.clone()).swatch(crate::kit::category_colors(&c.color).0).checked(d.categories.contains(&c.name)))
        .collect();
    cat_menu.push(MenuItem::divider());
    cat_menu.push(MenuItem::new("cat-clear", "Clear categories").icon("dismiss").disabled(d.categories.is_empty()));
    let more_menu = vec![MenuItem::new("duplicate", "Duplicate event").icon("copy"), MenuItem::new("print", "Print").icon("print").shortcut("Ctrl+P")];
    let personal = vec![
        Command::new("showas", show_as_label(d.show_as), "calendar_clock").menu(show_as_menu).disabled(!editable_cal),
        Command::new("reminder", reminder_label(d.reminder_minutes), "alert").menu(reminder_menu).disabled(!editable_cal),
        Command::new("categorize", "Categorize", "tag").menu(cat_menu).disabled(!editable_cal),
    ];
    let mut groups: Vec<Vec<Command>> = Vec::new();
    if invitation {
        groups.push(vec![
            Command::new("accept", "Accept", "checkmark").toggled(d.my_response == ResponseStatus::Accepted).disabled(d.is_cancelled),
            Command::new("tentative", "Tentative", "question_circle").toggled(d.my_response == ResponseStatus::Tentative).disabled(d.is_cancelled),
            Command::new("decline", "Decline", "dismiss").toggled(d.my_response == ResponseStatus::Declined).disabled(d.is_cancelled),
        ]);
        groups.push(vec![Command::new("save", "Save", "save").tooltip("Save your reminder, Show as and category changes (Ctrl+S)"), Command::new("delete", "Delete", "delete")]);
        groups.push(personal);
    } else if readonly {
        groups.push(vec![Command::new("close", "Close", "dismiss")]);
    } else {
        // One command id for Save and Send: the button stays the same
        // element when attendees turn it into Send. (A new element in its
        // place took the click that blurred an attendee box and added
        // the chips, and the attendee menu opened instead of saving.) A
        // press keeps focus in the attendee box, so the address typed
        // there isn't added, turning Save into Send, under the pointer.
        let label = if send { "Send" } else { "Save" };
        let mut first = vec![
            Command::new("primary", label, if send { "send" } else { "save" })
                .primary()
                .keep_focus()
                .tooltip(if send { "Send (Ctrl+Enter)" } else { "Save (Ctrl+S)" }),
        ];
        if is_new {
            first.push(Command::new("discard", "Discard", "delete"));
        } else if has_attendees && !original.read().is_cancelled {
            first.push(Command::new("cancelmeeting", "Cancel", "calendar_cancel").tooltip("Cancel meeting"));
        } else {
            first.push(Command::new("delete", "Delete", "delete"));
        }
        groups.push(first);
        groups.push(personal);
        let can_recur = !matches!(mode, EditMode::Occurrence { .. }) && d.series_id.is_none();
        groups.push(vec![
            Command::new("recurring", "Recurring", "arrow_repeat_all")
                .disabled(!can_recur)
                .tooltip(if can_recur { "Make this a recurring event" } else { "Open the series to change how it repeats" }),
            Command::new("timezones", "Time zones", "globe").toggled(show_tz()),
        ]);
        groups.push(vec![Command::new("more", "More options", "more_horizontal").icon_only().menu(more_menu)]);
    }
    let tabs = vec![RibbonTab::new("event", "Event", groups)];
    let on_command = move |id: String| {
        if let Some(sa) = id.strip_prefix("showas:").and_then(show_as_from_id) {
            draft.write().show_as = sa;
            return;
        }
        if let Some(v) = id.strip_prefix("rem:") {
            match v {
                "none" => draft.write().reminder_minutes = None,
                "keep" => {}
                m => draft.write().reminder_minutes = m.parse().ok(),
            }
            return;
        }
        if let Some(name) = id.strip_prefix("cat:") {
            let mut w = draft.write();
            match w.categories.iter().position(|c| c == name) {
                Some(i) => {
                    w.categories.remove(i);
                }
                None => w.categories.push(name.to_string()),
            }
            return;
        }
        match id.as_str() {
            // A click: keys typed just before it may still be on their way
            // to an attendee box (a busy page delivers the click first).
            // Save once they have landed, so the address is whole.
            "primary" | "save" => {
                // What the button said when it was clicked.
                let how = if id == "primary" && send { SaveAs::Send } else { SaveAs::Save };
                let mut save = do_save;
                crate::kit::spawn_in(scope, alive, async move {
                    crate::kit::dom::keys_landed("body").await;
                    save(how, false);
                });
            }
            "discard" => discard(dirty),
            "close" => close(),
            "delete" | "cancelmeeting" => {
                let occ = as_occurrence();
                close();
                request_delete(app, cal, &occ);
            }
            "private" => {
                let p = draft.peek().is_private;
                draft.write().is_private = !p;
            }
            "cat-clear" => draft.write().categories.clear(),
            "recurring" => repeat_open.set(true),
            "timezones" => show_tz.set(!show_tz()),
            "accept" => respond_and_close(ResponseStatus::Accepted),
            "tentative" => respond_and_close(ResponseStatus::Tentative),
            "decline" => respond_and_close(ResponseStatus::Declined),
            "duplicate" => {
                let occ = Occurrence { start: draft.peek().start, end: draft.peek().end, original_start: draft.peek().start, color: String::new(), event: draft.peek().clone() };
                super::actions::duplicate(cal, &occ);
            }
            "print" => {
                document::eval("window.print()");
            }
            _ => {}
        }
    };

    let series_id = match mode {
        EditMode::Occurrence { series_id, .. } => Some(series_id),
        _ => d.series_id,
    };
    let join = join_url(&d);
    let title_label = if is_new { "New event" } else if readonly { "Event details" } else { "Edit event" };

    rsx! {
        Ribbon { tabs, on_command }
        div {
            class: "ax-pane ax-grow ax-cal-editor",
            role: "dialog",
            "aria-label": "{title_label}",
            onkeydown: move |e: KeyboardEvent| {
                let chord = keys::chord(&e);
                match chord.as_str() {
                    "Ctrl+S" => {
                        e.prevent_default();
                        e.stop_propagation();
                        if !readonly || invitation { do_save(SaveAs::Save, false) }
                    }
                    // Also with an attendee only typed so far (added on send).
                    "Ctrl+Enter" | "Alt+S" if organizer && !readonly && !cancelled
                        && (send || !typed_required.peek().trim().is_empty() || !typed_optional.peek().trim().is_empty()) =>
                    {
                        e.prevent_default();
                        e.stop_propagation();
                        do_save(SaveAs::Send, false);
                    }
                    "Escape" => {
                        e.stop_propagation();
                        if confirm().is_some() { confirm.set(None) } else if readonly && !invitation { close() } else { discard(dirty) }
                    }
                    _ => e.stop_propagation(),
                }
            },
            div { class: "ax-cal-editor-scroll",
                div { class: "ax-cal-editor-form",
                    div { class: "ax-cal-editor-top",
                        span { class: "ax-cal-editor-heading", "{title_label}" }
                        IconButton { icon: "dismiss", title: "Close", onclick: move |_| if readonly && !invitation { close() } else { discard(dirty) } }
                    }
                    if let Some(sid) = series_id {
                        div { class: "ax-message-bar",
                            Icon { name: if d.series_id.is_some() { "arrow_repeat_all_off" } else { "arrow_repeat_all" }, size: 16 }
                            span { class: "ax-grow",
                                if d.series_id.is_some() { "This event was changed from its series." } else { "This is one event in a series. Changes apply to this event only." }
                            }
                            if let (EditMode::Occurrence { original_start, .. }, true) = (mode, organizer && editable_cal) {
                                Button {
                                    label: "Edit this and all following events",
                                    small: true,
                                    onclick: move |_| {
                                        let o = original.peek().clone();
                                        cal.open_following(sid, original_start, o.start, o.end);
                                    },
                                }
                            }
                            Button {
                                label: if organizer && editable_cal { "Edit series" } else { "View series" },
                                small: true,
                                onclick: move |_| cal.open_series(sid),
                            }
                        }
                    }
                    if mode == EditMode::Series {
                        div { class: "ax-message-bar", Icon { name: "arrow_repeat_all", size: 16 } span { "You're editing every event in the series." } }
                    }
                    if matches!(mode, EditMode::Following { .. }) {
                        div { class: "ax-message-bar",
                            Icon { name: "arrow_repeat_all", size: 16 }
                            span { "You're editing this event and all following events in the series. Earlier events stay as they are." }
                        }
                    }
                    if d.is_cancelled {
                        div { class: "ax-message-bar ax-warning", Icon { name: "calendar_cancel", size: 16 } span { "This meeting has been canceled." } }
                    }
                    if !editable_cal {
                        div { class: "ax-message-bar", Icon { name: "lock_closed", size: 16 } span { "You have view-only access to this calendar." } }
                    }
                    if invitation {
                        div { class: "ax-message-bar",
                            Icon { name: "info", size: 16 }
                            span {
                                {format!(
                                    "{} invited you. {}",
                                    d.organizer.as_ref().map(|o| o.display().to_string()).unwrap_or_default(),
                                    match d.my_response {
                                        ResponseStatus::Accepted => "You accepted.",
                                        ResponseStatus::Tentative => "You tentatively accepted.",
                                        ResponseStatus::Declined => "You declined.",
                                        _ => "You haven't responded yet.",
                                    }
                                )}
                            }
                        }
                    }
                    if let Some(err) = error() {
                        div { class: "ax-message-bar ax-error", role: "alert", Icon { name: "error_circle", size: 16 } span { "{err}" } }
                    }
                    if writable.len() > 1 && !readonly {
                        div { class: "ax-cal-editor-row",
                            span { class: "ax-cal-dot", style: "background:{calendar_color}" }
                            select {
                                class: "ax-cal-plain-select",
                                "aria-label": "Calendar",
                                disabled: matches!(mode, EditMode::Occurrence { .. }),
                                onchange: move |e| if let Ok(id) = e.value().parse::<CalendarId>() { draft.write().calendar_id = id },
                                for c in writable.iter() {
                                    option { key: "{c.id}", value: "{c.id}", selected: c.id == d.calendar_id, "{c.name}" }
                                }
                            }
                        }
                    }
                    div { class: "ax-cal-editor-title-row",
                        input {
                            class: "ax-cal-editor-title",
                            placeholder: "Add a title",
                            "aria-label": "Add a title",
                            value: "{d.title}",
                            readonly,
                            onmounted: move |e| async move { let _ = e.set_focus(true).await; },
                            oninput: move |e| draft.write().title = e.value(),
                            onkeydown: move |e| keys::stop_typing_keys(&e),
                        }
                        if d.is_private { span { class: "ax-cal-private-chip", Icon { name: "lock_closed", size: 12 } "Private" } }
                    }
                    if !d.categories.is_empty() {
                        div { class: "ax-cal-editor-cats",
                            for c in d.categories.iter().cloned() {
                                {
                                    let color = cats.iter().find(|x| x.name == c).map(|x| x.color.clone()).unwrap_or_default();
                                    let name = c.clone();
                                    rsx! {
                                        span { key: "{c}", class: "ax-cal-cat-chip",
                                            CategoryPill { name: c.clone(), color }
                                            if editable_cal {
                                                button {
                                                    r#type: "button",
                                                    class: "ax-cal-chip-person-x",
                                                    title: "Remove {name}",
                                                    "aria-label": "Remove category {name}",
                                                    onclick: move |_| draft.write().categories.retain(|x| *x != name),
                                                    Icon { name: "dismiss", size: 12 }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    div { class: "ax-cal-editor-row ax-cal-editor-people",
                        AttendeePicker {
                            attendees: d.attendees.clone(),
                            kind: AttendeeKind::Required,
                            placeholder: "Invite required attendees",
                            disabled: readonly,
                            typed: typed_required,
                            onchange: move |list: Vec<Attendee>| draft.write().attendees = list,
                        }
                        if !show_optional() && !readonly {
                            button { r#type: "button", class: "ax-cal-link", onclick: move |_| show_optional.set(true), "Optional" }
                        }
                    }
                    if show_optional() {
                        div { class: "ax-cal-editor-row ax-cal-editor-people",
                            AttendeePicker {
                                attendees: d.attendees.clone(),
                                kind: AttendeeKind::Optional,
                                placeholder: "Invite optional attendees",
                                disabled: readonly,
                                typed: typed_optional,
                                onchange: move |list: Vec<Attendee>| draft.write().attendees = list,
                            }
                        }
                    }
                    div { class: "ax-cal-editor-row ax-cal-when",
                        Icon { name: "clock", class: "ax-cal-row-icon" }
                        div { class: "ax-cal-when-fields",
                            DateField {
                                value: start_date,
                                label: "Start date",
                                week_start: s.week_start,
                                disabled: readonly,
                                onchange: move |nd: NaiveDate| move_to_date(draft, nd),
                            }
                            if !d.all_day {
                                TimeField {
                                    value: st,
                                    label: "Start time",
                                    h24,
                                    from: None,
                                    disabled: readonly,
                                    onchange: move |t: NaiveTime| {
                                        end_adjusted.set(false);
                                        set_start_time(draft, t);
                                    },
                                }
                                Icon { name: "arrow_right", size: 16, class: "ax-muted" }
                                TimeField {
                                    value: et,
                                    label: "End time",
                                    h24,
                                    from: (!multi_day).then_some(st),
                                    disabled: readonly,
                                    onchange: move |t: NaiveTime| end_adjusted.set(set_end_time(draft, t)),
                                }
                            }
                            if multi_day || end_date_open() || d.all_day {
                                DateField {
                                    value: end_date,
                                    label: "End date",
                                    week_start: s.week_start,
                                    disabled: readonly,
                                    onchange: move |nd: NaiveDate| set_end_date(draft, nd),
                                }
                            } else if !readonly {
                                button { r#type: "button", class: "ax-cal-link", title: "Set an end date", onclick: move |_| end_date_open.set(true), "End date" }
                            }
                            Switch { checked: d.all_day, label: "All day", disabled: readonly, onchange: move |on: bool| toggle_all_day(app, draft, on) }
                        }
                    }
                    div { class: "ax-cal-editor-sub",
                        if end_adjusted() {
                            span { class: "ax-cal-field-hint", role: "status", "End time was adjusted" }
                            span { "·" }
                        }
                        span { "{length}" }
                        span { "·" }
                        span { title: "Event times are shown in this time zone", {time_zone_label()} }
                    }
                    if show_tz() {
                        div { class: "ax-cal-editor-row ax-cal-tz-row",
                            Icon { name: "globe", class: "ax-cal-row-icon" }
                            div { class: "ax-col",
                                span { {format!("Start time zone: {}", time_zone_label())} }
                                span { {format!("End time zone: {}", time_zone_label())} }
                                span { class: "ax-muted ax-small-text", "Local calendars use your computer's time zone." }
                            }
                        }
                    }
                    if !matches!(mode, EditMode::Occurrence { .. }) && d.series_id.is_none() {
                        div { class: "ax-cal-editor-row",
                            RepeatMenu {
                                rule: d.recurrence.clone(),
                                start: start_date,
                                work_days: s.work_days.clone(),
                                disabled: readonly,
                                onchange: move |r: Option<Recurrence>| draft.write().recurrence = r,
                                on_custom: move |_| repeat_open.set(true),
                            }
                        }
                    }
                    div { class: "ax-cal-editor-row",
                        Icon { name: "location", class: "ax-cal-row-icon" }
                        input {
                            class: "ax-cal-plain-input",
                            placeholder: "Search for a room or location",
                            "aria-label": "Search for a room or location",
                            value: "{d.location}",
                            readonly,
                            oninput: move |e| draft.write().location = e.value(),
                            onkeydown: move |e| keys::stop_typing_keys(&e),
                        }
                    }
                    div { class: "ax-cal-editor-row",
                        Icon { name: "video", class: "ax-cal-row-icon" }
                        Switch {
                            checked: d.is_online_meeting,
                            label: "Online meeting",
                            disabled: readonly,
                            onchange: move |on: bool| draft.write().is_online_meeting = on,
                        }
                        if let Some(url) = join {
                            button {
                                r#type: "button",
                                class: "ax-btn ax-primary ax-small",
                                title: "Join the online meeting",
                                onclick: move |_| open_external(&url),
                                Icon { name: "video" }
                                span { "Join" }
                            }
                        }
                    }
                    div { class: "ax-cal-editor-row ax-cal-editor-desc",
                        Icon { name: "text_align_left", class: "ax-cal-row-icon" }
                        textarea {
                            class: "ax-cal-desc",
                            placeholder: "Add a description or attach documents",
                            "aria-label": "Description",
                            value: "{body}",
                            readonly,
                            oninput: move |e| body.set(e.value()),
                            onkeydown: move |e| keys::stop_typing_keys(&e),
                        }
                    }
                }
            }
        }
        if repeat_open() {
            RepeatDialog {
                rule: d.recurrence.clone(),
                start: start_date,
                week_start: s.week_start,
                on_start: move |nd: NaiveDate| move_to_date(draft, nd),
                on_save: move |r: Option<Recurrence>| {
                    draft.write().recurrence = r;
                    repeat_open.set(false);
                },
                on_close: move |_| repeat_open.set(false),
            }
        }
        if let Some(c) = confirm() {
            match c {
                Confirm::Discard => rsx! {
                    Dialog {
                        title: "Discard changes?",
                        width: 400,
                        on_close: move |_| confirm.set(None),
                        actions: rsx! {
                            // Opens on Cancel (as kit::ConfirmDialog does): an Enter
                            // right after Esc keeps the event.
                            Button { label: "Discard", variant: Variant::Primary, default_action: true, onclick: move |_| { confirm.set(None); close() } }
                            Button { label: "Cancel", autofocus: true, onclick: move |_| confirm.set(None) }
                        },
                        p { "Your changes to this event haven't been saved." }
                    }
                },
                Confirm::ResetExceptions => rsx! {
                    Dialog {
                        title: "Change the series?",
                        width: 420,
                        on_close: move |_| confirm.set(None),
                        actions: rsx! {
                            Button { label: if send { "Send" } else { "Save" }, variant: Variant::Primary, default_action: true, onclick: move |_| { confirm.set(None); do_save(if send { SaveAs::Send } else { SaveAs::Save }, true) } }
                            Button { label: "Cancel", onclick: move |_| confirm.set(None) }
                        },
                        p { "Changing the series will remove changes made to individual occurrences." }
                    }
                },
            }
        }
    }
}

/// Move the event to another start date keeping times and duration.
fn move_to_date(mut draft: Signal<Event>, nd: NaiveDate) {
    let mut w = draft.write();
    let days = (nd - local_date(w.start)).num_days();
    let occ = Occurrence { event: w.clone(), start: w.start, end: w.end, original_start: w.start, color: String::new() };
    let (a, b) = super::month_view::shift_days(&occ, days);
    w.start = a;
    w.end = b;
}

/// Change the (inclusive) end date; an end before the start keeps one day.
fn set_end_date(mut draft: Signal<Event>, nd: NaiveDate) {
    let mut w = draft.write();
    let first = local_date(w.start);
    let nd = nd.max(first);
    if w.all_day {
        w.end = at_min(nd + Duration::days(1), 0);
    } else {
        let t = w.end.with_timezone(&Local).time();
        let end = local_to_utc(nd, t);
        w.end = if end > w.start { end } else { w.start + Duration::minutes(30) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The toolbar's first button saves or sends as its label said when it
    /// was clicked, and a press on it keeps focus in the attendee box (no
    /// blur adds the typed address and turns Save into Send under the
    /// pointer); the address is added by the save.
    #[test]
    fn the_first_button_does_what_it_says() {
        let src = include_str!("editor.rs");
        assert!(src.contains(".keep_focus()"));
        assert!(src.contains(r#"let how = if id == "primary" && send { SaveAs::Send } else { SaveAs::Save };"#));
        let cmd = include_str!("../shell/ribbon/command.rs");
        assert_eq!(cmd.matches("if keep_focus { e.prevent_default() }").count(), 2, "plain and split buttons");
    }

    #[test]
    fn description_to_html() {
        assert_eq!(text_to_html("Agenda:\n- a\n\n<b>bold</b> & co"), "<p>Agenda:<br>- a</p><p>&lt;b&gt;bold&lt;/b&gt; &amp; co</p>");
        assert_eq!(text_to_html("  "), "");
    }
}
