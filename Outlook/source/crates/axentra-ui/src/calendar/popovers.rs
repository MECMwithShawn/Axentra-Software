//! Surfaces anchored on the grid: the peek card (single click), the
//! quick-create callout (click / drag on empty slots), the month "+N more"
//! day list and the event context menu.

use axentra_core::*;
use axentra_core::Event;
use chrono::{Duration, NaiveDate, NaiveTime};
use dioxus::prelude::*;

use super::actions::{self, can_edit_calendar, join_url, open_external, request_delete, respond};
use super::colors::ramp;
use super::event_block::display_title;
use super::fields::{AttendeePicker, DateField, TimeField, valid_address};
use super::state::*;
use super::text::{self, SHOW_AS_CHOICES, minutes_of_day, reminder_label, show_as_id, show_as_label, when_text};
use super::time_grid::load_occurrences;
use crate::kit::{Avatar, Button, Icon, IconButton, Menu, MenuButton, MenuItem, Switch, Variant, keys};
use crate::state::{AppState, store, use_app};

/// Top-left corner for a card of width `w` next to `a`: right of it, else
/// left, else clamped below. A negative anchor means "centre".
pub fn place(a: Anchor, w: f64, vp: (f64, f64)) -> (f64, f64) {
    if a.x < 0.0 {
        return (((vp.0 - w) / 2.0).max(8.0), 96.0);
    }
    let right = a.x + a.w + 8.0;
    let x = if right + w <= vp.0 - 8.0 {
        right
    } else if a.x - 8.0 - w >= 8.0 {
        a.x - 8.0 - w
    } else {
        a.x.clamp(8.0, (vp.0 - w - 8.0).max(8.0))
    };
    let y = a.y.clamp(8.0, (vp.1 - 160.0).max(8.0));
    (x, y)
}

fn card_style(x: f64, y: f64, w: f64) -> String {
    format!("left:{x}px;top:{y}px;width:{w}px;transform:translateY(min(0px, calc(100vh - {y}px - 100% - 8px)))")
}

#[component]
pub fn Popovers() -> Element {
    let app = use_app();
    let mut cal = use_cal();
    let pop = (cal.popover)();
    let vp = (cal.viewport)();
    match pop {
        None => rsx! {},
        Some(Popover::Peek { occ, anchor }) => {
            let (x, y) = place(anchor, 360.0, vp);
            rsx! { PeekCard { key: "{occ.event.id}-{occ.original_start.timestamp()}", occ: *occ, x, y } }
        }
        Some(Popover::QuickCreate { sel, anchor }) => {
            let (x, y) = place(anchor, 400.0, vp);
            rsx! { QuickCreate { key: "{sel.start_day}-{sel.start_min}-{sel.end_day}-{sel.end_min}-{sel.all_day}", sel, x, y } }
        }
        Some(Popover::DayEvents { date, anchor }) => {
            let (x, y) = place(Anchor { x: anchor.x - 160.0, y: anchor.y - 40.0, w: 0.0, h: 0.0 }, 320.0, vp);
            rsx! { DayEvents { date, x, y } }
        }
        Some(Popover::Context { occ, x, y }) => {
            let items = context_items(&occ);
            rsx! {
                Menu {
                    items,
                    x,
                    y,
                    on_select: move |id: String| context_command(app, cal, &occ, &id),
                    on_close: move |_| {
                        if matches!(*cal.popover.peek(), Some(Popover::Context { .. })) {
                            cal.popover.set(None);
                        }
                        actions::restore_focus(cal);
                    },
                }
            }
        }
    }
}

fn category_items(occ: &Occurrence) -> Vec<MenuItem> {
    let cats = store().categories().unwrap_or_default();
    let mut items: Vec<MenuItem> = cats
        .iter()
        .map(|c| {
            MenuItem::new(format!("cat:{}", c.name), c.name.clone())
                .swatch(crate::kit::category_colors(&c.color).0)
                .checked(occ.event.categories.contains(&c.name))
        })
        .collect();
    if items.is_empty() {
        items.push(MenuItem::new("nocats", "No categories yet").disabled(true));
    }
    items.push(MenuItem::divider());
    items.push(MenuItem::new("cat-clear", "Clear categories").icon("dismiss").disabled(occ.event.categories.is_empty()));
    items
}

fn show_as_items(occ: &Occurrence) -> Vec<MenuItem> {
    SHOW_AS_CHOICES
        .iter()
        .map(|s| MenuItem::new(format!("showas:{}", show_as_id(*s)), show_as_label(*s)).checked(occ.event.show_as == *s))
        .collect()
}

/// Right-click / Shift+F10 menu for an event.
pub fn context_items(occ: &Occurrence) -> Vec<MenuItem> {
    let e = &occ.event;
    let me = my_address();
    let editable = can_edit_calendar(e.calendar_id);
    let mut items = vec![MenuItem::new("open", "Open").icon("open").shortcut("Enter")];
    if is_invitation(e, me.as_ref()) && !e.is_cancelled && editable {
        items.push(MenuItem::new("accept", "Accept").icon("checkmark").checked(e.my_response == ResponseStatus::Accepted));
        items.push(MenuItem::new("tentative", "Tentative").icon("question_circle").checked(e.my_response == ResponseStatus::Tentative));
        items.push(MenuItem::new("decline", "Decline").icon("dismiss").checked(e.my_response == ResponseStatus::Declined));
    }
    if join_url(e).is_some() {
        items.push(MenuItem::new("join", "Join").icon("video"));
    }
    if editable {
        items.push(MenuItem::divider());
        items.push(MenuItem::new("categorize", "Categorize").icon("tag").submenu(category_items(occ)));
        items.push(MenuItem::new("showas", "Show as").icon("calendar_clock").submenu(show_as_items(occ)));
        items.push(MenuItem::new("private", "Private").icon("lock_closed").checked(e.is_private));
    }
    items.push(MenuItem::new("duplicate", "Duplicate event").icon("copy"));
    if editable {
        items.push(MenuItem::divider());
        let organizer_meeting = i_organize(e, me.as_ref()) && !e.attendees.is_empty() && !e.is_cancelled;
        items.push(if e.is_cancelled {
            MenuItem::new("delete", "Remove from calendar").icon("delete").danger()
        } else if organizer_meeting {
            MenuItem::new("delete", "Cancel").icon("calendar_cancel").shortcut("Delete").danger()
        } else {
            MenuItem::new("delete", "Delete").icon("delete").shortcut("Delete").danger()
        });
    }
    items
}

pub fn context_command(app: AppState, cal: Cal, occ: &Occurrence, id: &str) {
    if let Some(name) = id.strip_prefix("cat:") {
        actions::toggle_category(app, occ, name);
        return;
    }
    if let Some(s) = id.strip_prefix("showas:").and_then(text::show_as_from_id) {
        actions::set_show_as(app, occ, s);
        return;
    }
    match id {
        "open" => cal.open_occurrence(occ),
        "accept" => respond(app, cal, occ, ResponseStatus::Accepted),
        "tentative" => respond(app, cal, occ, ResponseStatus::Tentative),
        "decline" => respond(app, cal, occ, ResponseStatus::Declined),
        "join" => {
            if let Some(u) = join_url(&occ.event) {
                open_external(&u);
            }
        }
        "cat-clear" => actions::clear_categories(app, occ),
        "private" => actions::toggle_private(app, occ),
        "duplicate" => actions::duplicate(cal, occ),
        "delete" => request_delete(app, cal, occ),
        _ => {}
    }
}

/// Close the card and give focus back to the event it was opened from.
fn close_and_refocus(mut cal: Cal) {
    cal.popover.set(None);
    actions::restore_focus(cal);
}

#[component]
fn PeekCard(occ: Occurrence, x: f64, y: f64) -> Element {
    let app = use_app();
    let cal = use_cal();
    let _ = app.calendar_rev();
    // Show the latest saved state (an RSVP or Show-as change from this card).
    let fresh = store().event(occ.event.id).ok().flatten();
    let mut occ = occ;
    if let Some(e) = fresh {
        occ.event = e;
    }
    let e = occ.event.clone();
    let h24 = app.settings.read().use_24h_time;
    let me = my_address();
    let calendar = store().calendar(e.calendar_id).ok().flatten();
    let editable = calendar.as_ref().is_some_and(|c| c.can_edit);
    let organizer = i_organize(&e, me.as_ref());
    let invitation = !organizer;
    let meeting = !e.attendees.is_empty();
    let mut expanded = use_signal(|| false);
    // "Respond with note": the response waiting for its message.
    let mut note_for = use_signal(|| None::<ResponseStatus>);
    let mut note = use_signal(String::new);
    let color = ramp(&occ.color).primary;
    let title = display_title(&e);
    let when = when_text(&occ, h24);
    // Length of a timed event ("45 minutes", "1.5 hours"); all-day spans read from the dates.
    let length = (!e.all_day).then(|| crate::fmt::duration_label(occ.end - occ.start));
    let recur = e.recurrence.as_ref().map(|r| axentra_core::recurrence::describe(r, local_date(e.start)));
    let body = axentra_core::html::html_to_text(&e.body_html);
    let body = body.trim().to_string();
    let counts = |r: ResponseStatus| e.attendees.iter().filter(|a| a.response == r).count();
    let summary = format!(
        "Accepted {}, Tentative {}, Declined {}, Didn't respond {}",
        counts(ResponseStatus::Accepted),
        counts(ResponseStatus::Tentative),
        counts(ResponseStatus::Declined),
        e.attendees.iter().filter(|a| matches!(a.response, ResponseStatus::None | ResponseStatus::NotResponded)).count()
    );
    let join = join_url(&e);
    let now = (cal.now)();
    let join_soon = occ.start - Duration::minutes(5) <= now && now < occ.end;
    let loc_is_url = e.location.starts_with("http://") || e.location.starts_with("https://");
    let more_items = {
        let mut v = Vec::new();
        if editable {
            v.push(MenuItem::new("categorize", "Categorize").icon("tag").submenu(category_items(&occ)));
            v.push(MenuItem::new("showas", "Show as").icon("calendar_clock").submenu(show_as_items(&occ)));
            v.push(MenuItem::new("private", "Private").icon("lock_closed").checked(e.is_private));
        }
        v.push(MenuItem::new("duplicate", "Duplicate event").icon("copy"));
        if editable && invitation {
            v.push(MenuItem::divider());
            v.push(MenuItem::new("delete", "Delete").icon("delete").danger());
        }
        v
    };
    let (o_edit, o_del, o_open, o_more, o_rm, o_note) = (occ.clone(), occ.clone(), occ.clone(), occ.clone(), occ.clone(), occ.clone());
    let (o_acc, o_ten, o_dec) = (occ.clone(), occ.clone(), occ.clone());
    let organizer_name = e.organizer.as_ref().map(|o| o.display().to_string()).unwrap_or_else(|| "the organizer".into());
    let title_id = format!("ax-peek-{}", e.id);
    rsx! {
        div {
            class: "ax-cal-peek",
            role: "dialog",
            "aria-modal": "false",
            "aria-labelledby": "{title_id}",
            tabindex: "-1",
            style: card_style(x, y, 360.0),
            onmounted: move |ev| async move { let _ = ev.set_focus(true).await; },
            onmousedown: move |ev| ev.stop_propagation(),
            onkeydown: move |ev: KeyboardEvent| {
                if ev.key() == Key::Escape {
                    ev.stop_propagation();
                    close_and_refocus(cal);
                }
            },
            div { class: "ax-cal-peek-strip", style: "background:{color}" }
            div { class: "ax-cal-peek-actions",
                if editable && organizer && !e.is_cancelled {
                    IconButton { icon: "edit", title: "Edit", onclick: move |_| cal.open_occurrence(&o_edit) }
                    if meeting {
                        IconButton { icon: "calendar_cancel", title: "Cancel meeting", onclick: move |_| request_delete(app, cal, &o_del) }
                    } else {
                        IconButton { icon: "delete", title: "Delete", onclick: move |_| request_delete(app, cal, &o_del) }
                    }
                }
                IconButton { icon: "open", title: "Open event", onclick: move |_| cal.open_occurrence(&o_open) }
                MenuButton {
                    label: "More options",
                    icon: "more_horizontal",
                    icon_only: true,
                    items: more_items,
                    on_select: move |id: String| context_command(app, cal, &o_more, &id),
                }
                IconButton { icon: "dismiss", title: "Close", onclick: move |_| close_and_refocus(cal) }
            }
            div { class: "ax-cal-peek-body",
                h2 { id: "{title_id}", class: "ax-cal-peek-title", class: if e.is_cancelled { "ax-canceled" }, "{title}" }
                div { class: "ax-cal-peek-line",
                    Icon { name: "clock", size: 16 }
                    div { class: "ax-col",
                        span { "{when}" }
                        if let Some(l) = length { span { class: "ax-muted ax-small-text", "{l}" } }
                        if let Some(r) = recur { span { class: "ax-muted", "{r}" } }
                    }
                }
                if !e.location.trim().is_empty() {
                    div { class: "ax-cal-peek-line",
                        Icon { name: "location", size: 16 }
                        if loc_is_url {
                            a { href: "{e.location}", target: "_blank", rel: "noopener", "{e.location}" }
                        } else {
                            span { "{e.location}" }
                        }
                    }
                }
                if let Some(url) = join {
                    a {
                        class: "ax-btn ax-primary ax-cal-join",
                        class: if join_soon { "ax-soon" },
                        href: "{url}",
                        target: "_blank",
                        rel: "noopener",
                        title: "Join the online meeting",
                        Icon { name: "video" }
                        span { "Join" }
                    }
                }
                if e.is_cancelled {
                    div { class: "ax-message-bar ax-warning",
                        Icon { name: "calendar_cancel", size: 16 }
                        span { class: "ax-grow", "This meeting has been canceled." }
                        if editable {
                            Button { label: "Remove from calendar", small: true, onclick: move |_| actions::delete_now(app, cal, &o_rm) }
                        }
                    }
                }
                if let Some(o) = &e.organizer {
                    if meeting || invitation {
                        div { class: "ax-cal-peek-person",
                            Avatar { name: o.name.clone(), email: o.address.clone(), size: 24 }
                            div { class: "ax-col",
                                span { {o.display().to_string()} }
                                span { class: "ax-muted ax-small-text", "Organizer" }
                            }
                        }
                    }
                }
                if invitation && editable && !e.is_cancelled {
                    div { class: "ax-cal-rsvp", role: "group", "aria-label": "Respond",
                        RsvpButton { label: "Accept", icon: "checkmark", pressed: e.my_response == ResponseStatus::Accepted, onpick: move |o: String| rsvp_pick(app, cal, &o_acc, ResponseStatus::Accepted, &o, note, note_for) }
                        RsvpButton { label: "Tentative", icon: "question_circle", pressed: e.my_response == ResponseStatus::Tentative, onpick: move |o: String| rsvp_pick(app, cal, &o_ten, ResponseStatus::Tentative, &o, note, note_for) }
                        RsvpButton { label: "Decline", icon: "dismiss", pressed: e.my_response == ResponseStatus::Declined, onpick: move |o: String| rsvp_pick(app, cal, &o_dec, ResponseStatus::Declined, &o, note, note_for) }
                    }
                    if let Some(r) = note_for() {
                        div { class: "ax-cal-rsvp-note",
                            textarea {
                                class: "ax-textarea",
                                placeholder: "Add a message to {organizer_name}",
                                "aria-label": "Add a message to {organizer_name}",
                                value: "{note}",
                                onmounted: move |ev| async move { let _ = ev.set_focus(true).await; },
                                oninput: move |ev| note.set(ev.value()),
                                onkeydown: move |ev| keys::stop_typing_keys(&ev),
                            }
                            div { class: "ax-row", style: "gap:8px;justify-content:flex-end",
                                Button {
                                    label: "Send",
                                    variant: Variant::Primary,
                                    onclick: move |_| {
                                        note_for.set(None);
                                        actions::respond_how(app, cal, &o_note, r, actions::RsvpSend::WithNote);
                                    },
                                }
                                Button { label: "Cancel", onclick: move |_| note_for.set(None) }
                            }
                        }
                    }
                    if e.my_response == ResponseStatus::NotResponded {
                        div { class: "ax-muted ax-small-text", "You haven't responded yet." }
                    }
                }
                if organizer && meeting {
                    button {
                        r#type: "button",
                        class: "ax-cal-peek-summary",
                        "aria-expanded": if expanded() { "true" } else { "false" },
                        onclick: move |_| expanded.set(!expanded()),
                        Icon { name: "people", size: 16 }
                        span { class: "ax-grow", "{summary}" }
                        Icon { name: if expanded() { "chevron_up" } else { "chevron_down" }, size: 12 }
                    }
                    if expanded() {
                        div { class: "ax-cal-peek-attendees",
                            for a in e.attendees.iter().cloned() {
                                div { key: "{a.email.address}", class: "ax-cal-peek-person",
                                    Avatar { name: a.email.name.clone(), email: a.email.address.clone(), size: 24 }
                                    div { class: "ax-col ax-grow",
                                        span { {a.email.display().to_string()} }
                                        span { class: "ax-muted ax-small-text",
                                            {if a.kind == AttendeeKind::Optional { "Optional" } else { "Required" }}
                                        }
                                    }
                                    Icon {
                                        name: match a.response {
                                            ResponseStatus::Accepted => "checkmark_circle",
                                            ResponseStatus::Tentative => "question_circle",
                                            ResponseStatus::Declined => "dismiss_circle",
                                            _ => "circle",
                                        },
                                        size: 16,
                                        class: "ax-cal-resp-icon",
                                    }
                                }
                            }
                        }
                    }
                }
                if !body.is_empty() {
                    p { class: "ax-cal-peek-preview", "{body}" }
                }
                if !e.categories.is_empty() {
                    div { class: "ax-cal-peek-cats",
                        for c in e.categories.iter().cloned() {
                            {
                                let color = store().categories().unwrap_or_default().into_iter().find(|x| x.name == c).map(|x| x.color).unwrap_or_default();
                                rsx! { crate::kit::CategoryPill { key: "{c}", name: c.clone(), color } }
                            }
                        }
                    }
                }
                div { class: "ax-cal-peek-foot",
                    if e.reminder_minutes.is_some() {
                        span { class: "ax-row", style: "gap:6px",
                            Icon { name: "alert", size: 16 }
                            {format!("Reminder: {}", reminder_label(e.reminder_minutes))}
                        }
                    }
                    if let Some(c) = &calendar {
                        span { class: "ax-row", style: "gap:6px",
                            span { class: "ax-cal-dot", style: "background:{color}" }
                            "{c.name}"
                        }
                    }
                }
            }
        }
    }
}

/// An RSVP button or one of its send options ("now", "note", "silent").
fn rsvp_pick(app: AppState, cal: Cal, occ: &Occurrence, r: ResponseStatus, option: &str, mut note: Signal<String>, mut note_for: Signal<Option<ResponseStatus>>) {
    match option {
        "note" => {
            note.set(String::new());
            note_for.set(Some(r));
        }
        "silent" => actions::respond_how(app, cal, occ, r, actions::RsvpSend::Silent),
        _ => actions::respond_how(app, cal, occ, r, actions::RsvpSend::Now),
    }
}

/// Accept / Tentative / Decline as a split button: the body responds now,
/// the chevron offers "Respond with note" and "Respond without email".
/// `onpick` gets "now", "note" or "silent".
#[component]
fn RsvpButton(label: &'static str, icon: &'static str, pressed: bool, onpick: EventHandler<String>) -> Element {
    let options = vec![MenuItem::new("note", "Respond with note").icon("edit"), MenuItem::new("silent", "Respond without email").icon("mail_prohibited")];
    rsx! {
        div { class: "ax-cal-rsvp-split",
            button {
                r#type: "button",
                class: "ax-btn ax-cal-rsvp-btn",
                class: if pressed { "ax-pressed" },
                "aria-pressed": if pressed { "true" } else { "false" },
                onclick: move |_| onpick.call("now".into()),
                Icon { name: icon }
                span { "{label}" }
            }
            MenuButton {
                label: "{label} options",
                icon: "chevron_down",
                icon_only: true,
                class: "ax-cal-rsvp-more",
                items: options,
                on_select: move |id: String| onpick.call(id),
            }
        }
    }
}

#[component]
fn QuickCreate(sel: SlotSel, x: f64, y: f64) -> Element {
    let app = use_app();
    let mut cal = use_cal();
    let s = app.settings.read().clone();
    let h24 = s.use_24h_time;
    let cals: Vec<Calendar> = store().calendars().unwrap_or_default().into_iter().filter(|c| c.can_edit).collect();
    let draft0 = new_event_draft(app, (cal.active_calendar)(), sel.start(), sel.end(), sel.all_day);
    let mut draft = use_signal(|| draft0);
    let mut confirm = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let d = draft();
    let dirty = !d.title.trim().is_empty() || !d.attendees.is_empty() || !d.location.trim().is_empty();
    let start_date = local_date(d.start);
    let end_date = if d.all_day { local_date(d.end) - Duration::days(1) } else { local_date(d.end) };
    let st = d.start.with_timezone(&chrono::Local).time();
    let et = d.end.with_timezone(&chrono::Local).time();
    let send = !d.attendees.is_empty();
    let cal_color = ramp(&cals.iter().find(|c| c.id == d.calendar_id).map(|c| c.color.clone()).unwrap_or_default()).primary;
    let mut save = move |send_it: bool| {
        let mut e = draft.peek().clone();
        if send_it && e.attendees.iter().any(|a| !valid_address(&a.email.address)) {
            error.set(Some("Check the highlighted addresses".into()));
            return;
        }
        if send_it && e.title.trim().is_empty() {
            error.set(Some("Add a title before sending.".into()));
            return;
        }
        e.title = e.title.trim().to_string();
        let addrs: Vec<EmailAddress> = e.attendees.iter().map(|a| a.email.clone()).collect();
        if app.check(store().insert_event(&mut e)).is_some() {
            let _ = store().record_recent_addresses(&addrs);
            cal.popover.set(None);
            cal.slot.set(None);
            app.bump_calendar();
            actions::restore_focus(cal);
            if send_it {
                app.toast("Meeting saved. Invitations aren't sent from local calendars.");
            }
        }
    };
    let mut dismiss = move |dirty: bool| {
        let mut cal = cal;
        if dirty {
            confirm.set(true);
        } else {
            cal.popover.set(None);
            actions::restore_focus(cal);
        }
    };
    rsx! {
        div {
            class: "ax-cal-quick",
            role: "dialog",
            "aria-label": "New event",
            style: card_style(x, y, 400.0),
            onmousedown: move |e| e.stop_propagation(),
            onkeydown: move |e: KeyboardEvent| {
                if e.key() == Key::Escape {
                    e.stop_propagation();
                    if confirm() { confirm.set(false) } else { dismiss(dirty) }
                }
            },
            if cals.len() > 1 {
                div { class: "ax-cal-quick-cal",
                    span { class: "ax-cal-dot", style: "background:{cal_color}" }
                    select {
                        class: "ax-cal-plain-select",
                        "aria-label": "Calendar",
                        onchange: move |e| if let Ok(id) = e.value().parse::<CalendarId>() { draft.write().calendar_id = id },
                        for c in cals.iter() {
                            option { key: "{c.id}", value: "{c.id}", selected: c.id == d.calendar_id, "{c.name}" }
                        }
                    }
                }
            }
            input {
                class: "ax-cal-quick-title",
                placeholder: "Add a title",
                "aria-label": "Add a title",
                value: "{d.title}",
                onmounted: move |e| async move { let _ = e.set_focus(true).await; },
                oninput: move |e| draft.write().title = e.value(),
                onkeydown: move |e: KeyboardEvent| {
                    keys::stop_typing_keys(&e);
                    if e.key() == Key::Enter {
                        e.prevent_default();
                        save(!draft.peek().attendees.is_empty());
                    }
                },
            }
            div { class: "ax-cal-quick-row ax-cal-when",
                Icon { name: "clock", class: "ax-cal-row-icon" }
                div { class: "ax-cal-when-fields",
                    DateField {
                        value: start_date,
                        label: "Start date",
                        week_start: s.week_start,
                        onchange: move |nd: NaiveDate| {
                            let mut w = draft.write();
                            let delta = nd - local_date(w.start);
                            let (a, b) = super::month_view::shift_days(&Occurrence { event: w.clone(), start: w.start, end: w.end, original_start: w.start, color: String::new() }, delta.num_days());
                            w.start = a;
                            w.end = b;
                        },
                    }
                    if !d.all_day {
                        TimeField { value: st, label: "Start time", h24, from: None, onchange: move |t: NaiveTime| set_start_time(draft, t) }
                        span { class: "ax-muted", "to" }
                        TimeField { value: et, label: "End time", h24, from: (end_date == start_date).then_some(st), onchange: move |t: NaiveTime| { set_end_time(draft, t); } }
                    }
                    if d.all_day && end_date != start_date {
                        span { class: "ax-muted", {format!("to {}", text::short_date(end_date))} }
                    }
                    Switch {
                        checked: d.all_day,
                        label: "All day",
                        onchange: move |on: bool| toggle_all_day(app, draft, on),
                    }
                }
            }
            div { class: "ax-cal-quick-row",
                AttendeePicker {
                    attendees: d.attendees.clone(),
                    kind: AttendeeKind::Required,
                    placeholder: "Invite required attendees",
                    onchange: move |list: Vec<Attendee>| draft.write().attendees = list,
                }
            }
            div { class: "ax-cal-quick-row",
                Icon { name: "location", class: "ax-cal-row-icon" }
                input {
                    class: "ax-cal-plain-input",
                    placeholder: "Search for a room or location",
                    "aria-label": "Search for a room or location",
                    value: "{d.location}",
                    oninput: move |e| draft.write().location = e.value(),
                    onkeydown: move |e| keys::stop_typing_keys(&e),
                }
            }
            div { class: "ax-cal-quick-row",
                Icon { name: "video", class: "ax-cal-row-icon" }
                Switch {
                    checked: d.is_online_meeting,
                    label: "Online meeting",
                    onchange: move |on: bool| draft.write().is_online_meeting = on,
                }
            }
            if let Some(err) = error() {
                div { class: "ax-message-bar ax-error", role: "alert", Icon { name: "error_circle", size: 16 } span { "{err}" } }
            }
            div { class: "ax-cal-quick-foot",
                Button {
                    label: "More options",
                    onclick: move |_| {
                        let e = draft.peek().clone();
                        cal.slot.set(None);
                        cal.open_editor(e, EditMode::New);
                    },
                }
                Button { label: if send { "Send" } else { "Save" }, variant: Variant::Primary, onclick: move |_| save(send) }
            }
            if confirm() {
                div { class: "ax-cal-confirm", role: "alertdialog", "aria-label": "Discard this event?",
                    div { class: "ax-semibold", "Discard this event?" }
                    div { class: "ax-row", style: "gap:8px;justify-content:flex-end",
                        Button { label: "Discard", variant: Variant::Primary, onclick: move |_| { cal.popover.set(None); cal.slot.set(None); actions::restore_focus(cal); } }
                        Button { label: "Keep editing", onclick: move |_| confirm.set(false) }
                    }
                }
            }
        }
    }
}

/// Move the start keeping the duration.
pub fn set_start_time(mut draft: Signal<Event>, t: NaiveTime) {
    let mut w = draft.write();
    let len = w.end - w.start;
    w.start = local_to_utc(local_date(w.start), t);
    w.end = w.start + len;
}

/// Set the end; an end at or before the start keeps the old duration.
/// Returns true when the typed end had to be adjusted that way.
pub fn set_end_time(mut draft: Signal<Event>, t: NaiveTime) -> bool {
    let mut w = draft.write();
    let len = w.end - w.start;
    let end = local_to_utc(local_date(w.end - Duration::seconds(1)).max(local_date(w.start)), t);
    let adjusted = end <= w.start;
    w.end = if adjusted { w.start + len } else { end };
    adjusted
}

/// All day on: whole days (end exclusive). Off: back to the next half hour
/// on the start day with the default length.
pub fn toggle_all_day(app: AppState, mut draft: Signal<Event>, on: bool) {
    let mut w = draft.write();
    if on == w.all_day {
        return;
    }
    let first = local_date(w.start);
    if on {
        let last = local_date(w.end - Duration::seconds(1)).max(first);
        w.start = at_min(first, 0);
        w.end = at_min(last + Duration::days(1), 0);
        w.all_day = true;
        w.show_as = ShowAs::Free;
    } else {
        let now_min = minutes_of_day(next_half_hour(chrono::Utc::now()));
        let start_min = if first == chrono::Local::now().date_naive() { now_min } else { 9 * 60 };
        let len = app.settings.peek().default_event_minutes.max(5) as i64;
        w.start = at_min(first, start_min);
        w.end = w.start + Duration::minutes(len);
        w.all_day = false;
        w.show_as = ShowAs::Busy;
    }
}

#[component]
fn DayEvents(date: NaiveDate, x: f64, y: f64) -> Element {
    let app = use_app();
    let mut cal = use_cal();
    let _ = app.calendar_rev();
    let h24 = app.settings.read().use_24h_time;
    let filter = (cal.filter)();
    let occs = load_occurrences(&[date], &filter);
    rsx! {
        div {
            class: "ax-cal-dayevents",
            role: "dialog",
            "aria-label": date.format("%A, %B %-d").to_string(),
            tabindex: "-1",
            style: card_style(x, y, 320.0),
            onmounted: move |ev| async move { let _ = ev.set_focus(true).await; },
            onmousedown: move |e| e.stop_propagation(),
            onkeydown: move |e: KeyboardEvent| if e.key() == Key::Escape { e.stop_propagation(); cal.popover.set(None); actions::restore_focus(cal) },
            div { class: "ax-cal-dayevents-head",
                span { class: "ax-grow ax-semibold", {date.format("%A, %B %-d").to_string()} }
                IconButton { icon: "dismiss", title: "Close", onclick: move |_| { cal.popover.set(None); actions::restore_focus(cal) } }
            }
            if occs.is_empty() {
                div { class: "ax-muted", style: "padding:8px 16px", "No events" }
            }
            div { class: "ax-cal-dayevents-list",
                for o in occs {
                    {
                        let (o1, o2) = (o.clone(), o.clone());
                        let when = if o.event.all_day { "All day".to_string() } else { crate::fmt::time(o.start, h24) };
                        let dot = ramp(&o.color).primary;
                        rsx! {
                            button {
                                key: "{o.event.id}-{o.original_start.timestamp()}",
                                r#type: "button",
                                class: "ax-cal-dayevents-item",
                                "aria-label": super::event_block::aria_label(&o, h24),
                                onclick: move |e: MouseEvent| {
                                    let c = e.client_coordinates();
                                    actions::open_peek(cal, o1.clone(), Anchor::point(c.x, c.y));
                                },
                                ondoubleclick: move |_| cal.open_occurrence(&o2),
                                span { class: "ax-cal-dot", style: "background:{dot}" }
                                span { class: "ax-cal-dayevents-time", "{when}" }
                                span { class: "ax-ellipsis", {display_title(&o.event)} }
                            }
                        }
                    }
                }
            }
            button {
                r#type: "button",
                class: "ax-cal-link",
                onclick: move |_| cal.new_event(app, at_min(date, 0), at_min(date + Duration::days(1), 0), true),
                Icon { name: "add", size: 16 }
                span { "New event" }
            }
        }
    }
}
