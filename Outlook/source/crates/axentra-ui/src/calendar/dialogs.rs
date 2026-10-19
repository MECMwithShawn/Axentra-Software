//! Modal flows: delete / cancel with series scope, attendee delete, moving
//! a recurring event, "Send an update?", deleting a calendar and the
//! keyboard shortcut list.

use axentra_core::*;
use axentra_core::Event;
use dioxus::prelude::*;

use super::actions::{Scope, delete_now, delete_scoped, move_occurrence, save_moved};
use super::add_calendar::AddCalendarDialog;
use super::event_block::display_title;
use super::state::*;
use super::text::when_text;
use crate::kit::{Button, Dialog, Variant, keys};
use crate::state::{store, use_app};

#[component]
pub fn Dialogs() -> Element {
    let cal = use_cal();
    match (cal.dialog)() {
        None => rsx! {},
        Some(CalDialog::DeleteScope(occ)) => rsx! { DeleteScopeDialog { occ: *occ } },
        Some(CalDialog::CancelMeeting(occ)) => rsx! { CancelMeetingDialog { occ: *occ } },
        Some(CalDialog::AttendeeDelete(occ)) => rsx! { AttendeeDeleteDialog { occ: *occ } },
        Some(CalDialog::MoveScope { occ, start, end, all_day }) => rsx! { MoveScopeDialog { occ: *occ, start, end, all_day } },
        Some(CalDialog::SendUpdate { event }) => rsx! { SendUpdateDialog { event: *event } },
        Some(CalDialog::AddCalendar) => rsx! { AddCalendarDialog {} },
        Some(CalDialog::DeleteCalendar(id)) => rsx! { DeleteCalendarDialog { id } },
        Some(CalDialog::Shortcuts) => rsx! { ShortcutsDialog {} },
    }
}

fn close(mut cal: Cal) {
    cal.dialog.set(None);
    super::actions::restore_focus(cal);
}

/// A radio row (role="radio") used by the scope dialogs.
#[component]
fn Radio(checked: bool, label: String, onselect: EventHandler<()>, #[props(default)] disabled: bool) -> Element {
    rsx! {
        button {
            r#type: "button",
            class: "ax-cal-radio",
            class: if checked { "ax-checked" },
            role: "radio",
            "aria-checked": if checked { "true" } else { "false" },
            disabled,
            onclick: move |_| onselect.call(()),
            span { class: "ax-cal-radio-dot" }
            span { "{label}" }
        }
    }
}

#[component]
fn DeleteScopeDialog(occ: Occurrence) -> Element {
    let app = use_app();
    let cal = use_cal();
    let mut scope = use_signal(|| Scope::This);
    let mut message = use_signal(String::new);
    let me = my_address();
    let organizer_meeting = i_organize(&occ.event, me.as_ref()) && !occ.event.attendees.is_empty();
    let title = if organizer_meeting { "Cancel meeting" } else { "Delete recurring event" };
    let h24 = app.settings.read().use_24h_time;
    let when = when_text(&occ, h24);
    let o = occ.clone();
    rsx! {
        Dialog {
            title,
            width: 440,
            on_close: move |_| close(cal),
            actions: rsx! {
                Button {
                    label: if organizer_meeting { "Send" } else { "Delete" },
                    variant: Variant::Primary,
                    onclick: move |_| {
                        close(cal);
                        delete_scoped(app, cal, &o, scope());
                        if organizer_meeting {
                            app.toast("Meeting canceled. Cancellations aren't sent from local calendars.");
                        }
                    },
                }
                Button { label: "Cancel", onclick: move |_| close(cal) }
            },
            div { class: "ax-cal-dialog-body",
                p { class: "ax-semibold", {display_title(&occ.event)} }
                p { class: "ax-muted", "{when}" }
                div { role: "radiogroup", "aria-label": "Which events", class: "ax-col", style: "gap:4px",
                    Radio { checked: scope() == Scope::This, label: "This event", onselect: move |_| scope.set(Scope::This) }
                    Radio { checked: scope() == Scope::Following, label: "This and all following events", onselect: move |_| scope.set(Scope::Following) }
                    Radio { checked: scope() == Scope::All, label: "All events in the series", onselect: move |_| scope.set(Scope::All) }
                }
                if organizer_meeting {
                    textarea {
                        class: "ax-textarea ax-cal-cancel-msg",
                        placeholder: "Add a message for attendees",
                        "aria-label": "Add a message for attendees",
                        value: "{message}",
                        oninput: move |e| message.set(e.value()),
                        onkeydown: move |e| keys::stop_typing_keys(&e),
                    }
                }
            }
        }
    }
}

#[component]
fn CancelMeetingDialog(occ: Occurrence) -> Element {
    let app = use_app();
    let cal = use_cal();
    let mut message = use_signal(String::new);
    let h24 = app.settings.read().use_24h_time;
    let when = when_text(&occ, h24);
    let o = occ.clone();
    rsx! {
        Dialog {
            title: "Cancel meeting",
            width: 480,
            on_close: move |_| close(cal),
            actions: rsx! {
                Button {
                    label: "Send",
                    variant: Variant::Primary,
                    onclick: move |_| {
                        close(cal);
                        let e = o.event.clone();
                        if app.check(store().delete_event(e.id)).is_some() {
                            cal.clear_selection();
                            app.bump_calendar();
                            app.toast_with_undo("Meeting canceled. Cancellations aren't sent from local calendars.", crate::state::UndoAction::RestoreEvent(e));
                        }
                    },
                }
                Button { label: "Back", onclick: move |_| close(cal) }
            },
            div { class: "ax-cal-dialog-body",
                p { class: "ax-semibold", {display_title(&occ.event)} }
                p { class: "ax-muted", "{when}" }
                p { {format!("Attendees: {}", occ.event.attendees.iter().map(|a| a.email.display().to_string()).collect::<Vec<_>>().join(", "))} }
                textarea {
                    class: "ax-textarea ax-cal-cancel-msg",
                    placeholder: "Add a message for attendees",
                    "aria-label": "Add a message for attendees",
                    value: "{message}",
                    onmounted: move |e| async move { let _ = e.set_focus(true).await; },
                    oninput: move |e| message.set(e.value()),
                    onkeydown: move |e| keys::stop_typing_keys(&e),
                }
            }
        }
    }
}

#[component]
fn AttendeeDeleteDialog(occ: Occurrence) -> Element {
    let app = use_app();
    let cal = use_cal();
    let organizer = occ.event.organizer.as_ref().map(|o| o.display().to_string()).unwrap_or_default();
    let (o1, o2) = (occ.clone(), occ.clone());
    rsx! {
        Dialog {
            title: "Delete this event?",
            width: 440,
            on_close: move |_| close(cal),
            actions: rsx! {
                Button { label: "Delete", variant: Variant::Primary, onclick: move |_| { close(cal); delete_now(app, cal, &o1) } }
                Button {
                    label: "Delete and send decline",
                    onclick: move |_| {
                        close(cal);
                        let _ = store().respond_to_event(o2.event.id, ResponseStatus::Declined);
                        delete_now(app, cal, &o2);
                    },
                }
                Button { label: "Cancel", onclick: move |_| close(cal) }
            },
            div { class: "ax-cal-dialog-body",
                p { class: "ax-semibold", {display_title(&occ.event)} }
                if !organizer.is_empty() { p { class: "ax-muted", "Organized by {organizer}." } }
            }
        }
    }
}

#[component]
fn MoveScopeDialog(occ: Occurrence, start: chrono::DateTime<chrono::Utc>, end: chrono::DateTime<chrono::Utc>, all_day: bool) -> Element {
    let app = use_app();
    let cal = use_cal();
    let other_day = local_date(start) != local_date(occ.start);
    let converts = all_day != occ.event.all_day;
    let only_this = other_day || converts;
    let h24 = app.settings.read().use_24h_time;
    let mut moved = Occurrence { start, end, ..occ.clone() };
    moved.event.all_day = all_day;
    let when = when_text(&moved, h24);
    let (o1, o2) = (occ.clone(), occ.clone());
    let reason = if converts { "Only this event can change between all-day and timed." } else { "Only this event can move to another day." };
    rsx! {
        Dialog {
            title: "Move recurring event",
            width: 460,
            on_close: move |_| close(cal),
            actions: rsx! {
                Button { label: "Move this event", variant: Variant::Primary, onclick: move |_| { close(cal); move_occurrence(app, cal, &o1, start, end, all_day, false) } }
                Button {
                    label: "Move all events in the series",
                    disabled: only_this,
                    title: if only_this { reason } else { "" },
                    onclick: move |_| { close(cal); move_occurrence(app, cal, &o2, start, end, all_day, true) },
                }
                Button { label: "Cancel", onclick: move |_| close(cal) }
            },
            div { class: "ax-cal-dialog-body",
                p { class: "ax-semibold", {display_title(&occ.event)} }
                p { {format!("New time: {when}")} }
                if only_this { p { class: "ax-muted", "{reason}" } }
            }
        }
    }
}

#[component]
fn SendUpdateDialog(event: Event) -> Element {
    let app = use_app();
    let cal = use_cal();
    let h24 = app.settings.read().use_24h_time;
    let occ = Occurrence { start: event.start, end: event.end, original_start: event.start, color: String::new(), event: event.clone() };
    let when = when_text(&occ, h24);
    let (e1, e2) = (event.clone(), event.clone());
    rsx! {
        Dialog {
            title: "Send an update to attendees?",
            width: 460,
            on_close: move |_| close(cal),
            actions: rsx! {
                Button {
                    label: "Send update",
                    variant: Variant::Primary,
                    onclick: move |_| {
                        close(cal);
                        save_moved(app, cal, &e1);
                        app.toast("Update saved. Updates aren't sent from local calendars.");
                    },
                }
                Button { label: "Don't send", onclick: move |_| { close(cal); save_moved(app, cal, &e2) } }
                Button { label: "Cancel", onclick: move |_| close(cal) }
            },
            div { class: "ax-cal-dialog-body",
                p { class: "ax-semibold", {display_title(&event)} }
                p { {format!("New time: {when}")} }
            }
        }
    }
}

#[component]
fn DeleteCalendarDialog(id: CalendarId) -> Element {
    let app = use_app();
    let mut cal = use_cal();
    let Some(c) = store().calendar(id).ok().flatten() else { return rsx! {} };
    let (title, verb) = if c.can_edit { ("Delete calendar?", "Delete") } else { ("Remove calendar?", "Remove") };
    let body = if c.can_edit {
        format!("Delete \u{201c}{}\u{201d} and all of its events? You can't undo this.", c.name)
    } else {
        format!("Remove \u{201c}{}\u{201d} from your calendars?", c.name)
    };
    rsx! {
        Dialog {
            title,
            width: 440,
            on_close: move |_| close(cal),
            actions: rsx! {
                Button {
                    label: verb,
                    variant: Variant::Danger,
                    onclick: move |_| {
                        close(cal);
                        // Never leave every calendar hidden.
                        let others_visible = store().calendars().unwrap_or_default().iter().any(|x| x.id != id && x.is_visible);
                        if app.check(store().delete_calendar(id)).is_some() {
                            if !others_visible
                                && let Ok(Some(d)) = store().default_calendar(None)
                            {
                                let _ = store().set_calendar_visible(d.id, true);
                            }
                            if *cal.active_calendar.peek() == Some(id) {
                                cal.active_calendar.set(None);
                            }
                            app.bump_calendar();
                            app.toast(if verb == "Delete" { "Calendar deleted" } else { "Calendar removed" });
                        }
                    },
                }
                Button { label: "Cancel", onclick: move |_| close(cal) }
            },
            p { "{body}" }
        }
    }
}

/// The calendar's keyboard shortcuts (Outlook keyboard mode).
pub const SHORTCUTS: &[(&str, &str)] = &[
    ("New event", "Ctrl+N"),
    ("Day view", "Ctrl+Alt+1"),
    ("Work week view", "Ctrl+Alt+2"),
    ("Week view", "Ctrl+Alt+3 or Alt+-"),
    ("Month view", "Ctrl+Alt+4 or Alt+="),
    ("Show 1–7 days starting today", "Alt+1 … Alt+7"),
    ("Go to today", "Alt+Shift+Y or Home"),
    ("Previous / next time period", "Ctrl+Alt+Left / Ctrl+Alt+Right"),
    ("Previous / next week", "Alt+Up / Alt+Down"),
    ("Previous / next month", "Alt+Page Up / Alt+Page Down"),
    ("Go to a date", "Ctrl+G"),
    ("Open the selected event", "Enter"),
    ("Peek at the selected event", "Space"),
    ("Delete the selected event", "Delete"),
    ("Next / previous event", "Tab / Shift+Tab"),
    ("Move the slot selection", "Arrow keys"),
    ("Extend the slot selection", "Shift+Arrow keys"),
    ("Start / end of working hours", "Ctrl+Home / Ctrl+End"),
    ("Scroll a page", "Page Up / Page Down"),
    ("Event menu", "Shift+F10"),
    ("Print", "Ctrl+P"),
    ("Save the event (form)", "Ctrl+S"),
    ("Send the event (form)", "Ctrl+Enter or Alt+S"),
];

#[component]
fn ShortcutsDialog() -> Element {
    let cal = use_cal();
    rsx! {
        Dialog { title: "Keyboard shortcuts", width: 560, on_close: move |_| close(cal),
            div { class: "ax-cal-shortcuts",
                div { class: "ax-muted", style: "margin-bottom:8px", "Shortcut set: Outlook" }
                h3 { class: "ax-cal-shortcuts-head", "Calendar" }
                for (action, keys) in SHORTCUTS.iter() {
                    div { key: "{action}", class: "ax-cal-shortcut-row",
                        span { class: "ax-grow", "{action}" }
                        span { class: "ax-cal-keys",
                            for (i, k) in keys.split(" or ").enumerate() {
                                if i > 0 { span { class: "ax-muted", " or " } }
                                kbd { class: "ax-cal-kbd", "{k}" }
                            }
                        }
                    }
                }
            }
        }
    }
}
