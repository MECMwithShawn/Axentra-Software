//! Event actions shared by the peek card, context menu, keyboard, drag and
//! dialogs. Each one writes to the store, bumps the calendar revision and
//! reports problems as toasts.

use std::rc::Rc;

use axentra_core::*;
use axentra_core::Event;
use chrono::{DateTime, Utc};
use dioxus::prelude::*;

use super::state::{Anchor, Cal, CalDialog, OccKey, Popover, SlotSel, i_organize, is_invitation, my_address};
use crate::state::{AppState, UndoAction, store};

/// Whether events in this calendar can be changed (read-only holiday or
/// subscribed calendars can't).
pub fn can_edit_calendar(calendar_id: CalendarId) -> bool {
    store().calendar(calendar_id).ok().flatten().is_some_and(|c| c.can_edit)
}

/// Permission facts read once per layout pass (instead of per event).
pub struct Perms {
    editable: std::collections::HashSet<CalendarId>,
    pub me: Option<EmailAddress>,
}

impl Perms {
    pub fn load() -> Self {
        let editable = store().calendars().unwrap_or_default().into_iter().filter(|c| c.can_edit).map(|c| c.id).collect();
        Self { editable, me: my_address() }
    }

    /// Whether the user may move / resize / edit times: their own calendar
    /// and not a meeting organised by someone else.
    pub fn can_reschedule(&self, e: &Event) -> bool {
        self.editable.contains(&e.calendar_id) && i_organize(e, self.me.as_ref())
    }

    pub fn is_invitation(&self, e: &Event) -> bool {
        is_invitation(e, self.me.as_ref())
    }
}

/// The online-meeting link, if any.
pub fn join_url(e: &Event) -> Option<String> {
    e.online_meeting_url.clone().filter(|u| u.starts_with("https://") || u.starts_with("http://"))
}

/// Put keyboard focus back on the grid (after the focused event went away).
pub fn focus_surface() {
    document::eval("setTimeout(()=>{const el=document.querySelector('.ax-cal-scroll, .ax-cal-month');if(el)el.focus({preventScroll:true});},0)");
}

/// After a popover, menu or dialog closes: if focus fell back to the page
/// body, return it to the selected event (or the grid) so the calendar's
/// shortcuts keep working.
pub fn restore_focus(cal: Cal) {
    let sel = (*cal.selected.peek()).map(|k| k.dom_id()).unwrap_or_default();
    // Runs after the closed surface has left the DOM (the patch may land
    // after this script in liveview). Focus the shell parked on the app root
    // when the surface's field left the page counts as lost too.
    document::eval(&format!(
        "setTimeout(()=>{{const a=document.activeElement;if(a&&a!==document.body&&a.isConnected&&!a.classList.contains('ax-app'))return;\
         const el=('{sel}'&&document.querySelector('[data-occ=\"{sel}\"]'))||document.querySelector('.ax-cal-scroll, .ax-cal-month');\
         if(el)el.focus({{preventScroll:true,focusVisible:document.documentElement.hasAttribute('data-ax-kbd')}});}},60)"
    ));
}

/// Open a meeting link outside the app: the system browser on the desktop
/// (the webview hands external navigations to it), a new tab in the preview.
pub fn open_external(url: &str) {
    let Ok(quoted) = serde_json::to_string(url) else { return };
    document::eval(&format!(
        "const u={quoted};if(window.interpreter&&window.interpreter.liveview){{window.open(u,'_blank','noopener')}}else{{window.location.href=u}}"
    ));
}

/// Open the peek card for an occurrence, anchored on its block when it is
/// on screen (falls back to the given point).
pub fn open_peek(mut cal: Cal, occ: Occurrence, fallback: Anchor) {
    let key = OccKey::of(&occ);
    cal.selected.set(Some(key));
    let serial = *cal.peek_serial.peek() + 1;
    cal.peek_serial.set(serial);
    let el: Option<Rc<MountedData>> = cal.blocks.peek().get(&key).cloned();
    let scroll = *cal.scroll_top.peek();
    spawn(async move {
        let anchor = match el {
            Some(m) => match m.get_client_rect().await {
                // A block that went away (view change) measures as empty.
                Ok(r) if r.width() > 0.0 => Anchor { x: r.min_x(), y: r.min_y(), w: r.width(), h: r.height() },
                _ => fallback,
            },
            None => fallback,
        };
        // Superseded meanwhile (another peek, a closed popover, the editor).
        if *cal.peek_serial.peek() != serial || cal.editor.peek().is_some() || cal.dialog.peek().is_some() {
            return;
        }
        cal.popover_scroll.set(scroll);
        cal.popover.set(Some(Popover::Peek { occ: Box::new(occ), anchor }));
    });
}

/// Quick create for a keyboard slot selection, placed beside the selected
/// slots (or day cells) as they are drawn.
pub fn quick_create_at_selection(mut cal: Cal, sel: SlotSel) {
    const JS: &str = "const els=[...document.querySelectorAll('.ax-cal-slot-sel, .ax-cal-allday-cell.ax-selected, .ax-cal-month-cell.ax-selected')];\
        if(!els.length) return null;\
        const rs=els.map(e=>e.getBoundingClientRect());\
        const x=Math.min(...rs.map(r=>r.left)), y=Math.min(...rs.map(r=>r.top));\
        return [x, y, Math.max(...rs.map(r=>r.right))-x, Math.max(...rs.map(r=>r.bottom))-y];";
    spawn(async move {
        let anchor = match document::eval(JS).join::<Option<(f64, f64, f64, f64)>>().await {
            Ok(Some((x, y, w, h))) => Anchor { x, y, w, h },
            _ => Anchor::point(-1.0, -1.0),
        };
        cal.popover_scroll.set(*cal.scroll_top.peek());
        cal.popover.set(Some(Popover::QuickCreate { sel, anchor }));
    });
}

/// Delete key and the Delete / Cancel buttons: picks the right flow.
pub fn request_delete(app: AppState, mut cal: Cal, occ: &Occurrence) {
    let e = &occ.event;
    if !can_edit_calendar(e.calendar_id) {
        app.error("You can't delete events from this calendar.");
        return;
    }
    cal.close_popover();
    let me = my_address();
    if e.is_cancelled {
        delete_now(app, cal, occ);
    } else if e.recurrence.is_some() {
        cal.dialog.set(Some(CalDialog::DeleteScope(Box::new(occ.clone()))));
    } else if is_invitation(e, me.as_ref()) {
        cal.dialog.set(Some(CalDialog::AttendeeDelete(Box::new(occ.clone()))));
    } else if !e.attendees.is_empty() {
        cal.dialog.set(Some(CalDialog::CancelMeeting(Box::new(occ.clone()))));
    } else {
        delete_now(app, cal, occ);
    }
}

/// Delete a single event now, with Undo.
pub fn delete_now(app: AppState, cal: Cal, occ: &Occurrence) {
    let e = occ.event.clone();
    if app.check(store().delete_event(e.id)).is_some() {
        cal.clear_selection();
        focus_surface();
        app.bump_calendar();
        let text = if e.is_cancelled { "Removed from calendar" } else { "Event deleted" };
        app.toast_with_undo(text, UndoAction::RestoreEvent(e));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    This,
    Following,
    All,
}

/// Delete an occurrence, the rest of its series, or the whole series.
pub fn delete_scoped(app: AppState, cal: Cal, occ: &Occurrence, scope: Scope) {
    let id = occ.event.id;
    let r = match scope {
        Scope::This if occ.event.recurrence.is_some() => store().delete_occurrence(id, occ.original_start),
        Scope::This => store().delete_event(id),
        Scope::Following => store().delete_following(id, occ.original_start),
        Scope::All => store().delete_series(id),
    };
    if app.check(r).is_some() {
        cal.clear_selection();
        focus_surface();
        app.bump_calendar();
        app.toast(match scope {
            Scope::This => "Event deleted",
            Scope::Following => "Events deleted",
            Scope::All => "Series deleted",
        });
    }
}

/// How an RSVP goes out (spec §11.2): straight away, with a note to the
/// organizer, or without telling the organizer at all.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RsvpSend {
    Now,
    WithNote(String),
    Silent,
}

/// Accept / Tentative / Decline.
pub fn respond(app: AppState, cal: Cal, occ: &Occurrence, response: ResponseStatus) {
    respond_how(app, cal, occ, response, RsvpSend::Now);
}

pub fn respond_how(app: AppState, cal: Cal, occ: &Occurrence, response: ResponseStatus, how: RsvpSend) {
    let (send, note) = match &how {
        RsvpSend::Now => (true, ""),
        RsvpSend::WithNote(n) => (true, n.as_str()),
        RsvpSend::Silent => (false, ""),
    };
    if app.check(store().respond_to_event_with(occ.event.id, response, send, note)).is_some() {
        app.bump_calendar();
        if response == ResponseStatus::Declined {
            cal.close_popover();
            cal.clear_selection();
            focus_surface();
        }
        let verb = match response {
            ResponseStatus::Accepted => "Accepted",
            ResponseStatus::Tentative => "Tentatively accepted",
            ResponseStatus::Declined => "Declined",
            _ => "Response saved",
        };
        app.toast(match (how, response) {
            (RsvpSend::Silent, _) => format!("{verb}. The organizer won't be notified."),
            (RsvpSend::WithNote(_), _) => format!("{verb} with your note."),
            (RsvpSend::Now, ResponseStatus::Declined) => "Declined. The event was removed from your calendar.".to_string(),
            (RsvpSend::Now, _) => verb.to_string(),
        });
    }
}

pub fn set_show_as(app: AppState, occ: &Occurrence, show_as: ShowAs) {
    if app.check(store().modify_event(occ.event.id, |e| e.show_as = show_as)).is_some() {
        app.bump_calendar();
    }
}

pub fn toggle_private(app: AppState, occ: &Occurrence) {
    if app.check(store().modify_event(occ.event.id, |e| e.is_private = !e.is_private)).is_some() {
        app.bump_calendar();
    }
}

/// Tick or untick one category (several are allowed).
pub fn toggle_category(app: AppState, occ: &Occurrence, name: &str) {
    let name = name.to_string();
    let r = store().modify_event(occ.event.id, |e| {
        if let Some(i) = e.categories.iter().position(|c| *c == name) {
            e.categories.remove(i);
        } else {
            e.categories.push(name.clone());
        }
    });
    if app.check(r).is_some() {
        app.bump_calendar();
    }
}

pub fn clear_categories(app: AppState, occ: &Occurrence) {
    if app.check(store().modify_event(occ.event.id, |e| e.categories.clear())).is_some() {
        app.bump_calendar();
    }
}

/// "Duplicate event": a new event with the same details, opened in the editor.
pub fn duplicate(cal: Cal, occ: &Occurrence) {
    let me = my_address();
    let mut e = occ.event.clone();
    e.id = 0;
    e.remote_id = None;
    e.start = occ.start;
    e.end = occ.end;
    e.recurrence = None;
    e.exdates.clear();
    e.series_id = None;
    e.original_start = None;
    e.is_cancelled = false;
    e.organizer = me;
    e.my_response = ResponseStatus::Organizer;
    for a in &mut e.attendees {
        a.response = ResponseStatus::None;
    }
    if !can_edit_calendar(e.calendar_id)
        && let Ok(Some(c)) = store().default_calendar(None)
    {
        e.calendar_id = c.id;
    }
    cal.open_editor(e, super::state::EditMode::New);
}

/// A drop or keyboard move to new times (`all_day`: what the event becomes;
/// drops on the all-day row or the grid convert it): asks first for series
/// and for meetings with attendees, otherwise saves straight away.
pub fn request_move(app: AppState, mut cal: Cal, occ: &Occurrence, start: DateTime<Utc>, end: DateTime<Utc>, all_day: bool) {
    let e = &occ.event;
    if start == occ.start && end == occ.end && all_day == e.all_day {
        return;
    }
    if e.recurrence.is_some() {
        cal.dialog.set(Some(CalDialog::MoveScope { occ: Box::new(occ.clone()), start, end, all_day }));
        return;
    }
    let mut moved = e.clone();
    moved.start = start;
    moved.end = end;
    convert_kind(&mut moved, all_day);
    if !e.attendees.is_empty() && !e.is_cancelled {
        cal.dialog.set(Some(CalDialog::SendUpdate { event: Box::new(moved) }));
        return;
    }
    save_moved(app, cal, &moved);
}

/// Switch an event between all-day and timed after a drop, with Outlook's
/// defaults for the new kind (all-day: Free and an 18-hour reminder).
pub fn convert_kind(e: &mut Event, all_day: bool) {
    if e.all_day == all_day {
        return;
    }
    e.all_day = all_day;
    if all_day {
        if e.show_as == ShowAs::Busy {
            e.show_as = ShowAs::Free;
        }
    } else if e.show_as == ShowAs::Free {
        e.show_as = ShowAs::Busy;
    }
}

pub fn save_moved(app: AppState, cal: Cal, moved: &Event) {
    if app.check(store().update_event(moved)).is_some() {
        cal.close_popover();
        app.bump_calendar();
    }
}

/// Move one occurrence ("Move this event") or shift the whole series' time
/// of day ("Move all events in the series").
pub fn move_occurrence(app: AppState, cal: Cal, occ: &Occurrence, start: DateTime<Utc>, end: DateTime<Utc>, all_day: bool, whole_series: bool) {
    let r = if whole_series {
        let delta = start - occ.start;
        let length = end - start;
        store().event(occ.event.id).and_then(|m| {
            let mut m = m.ok_or_else(|| Error::NotFound("event".into()))?;
            m.start += delta;
            m.end = m.start + length;
            store().update_series(&m, false)
        })
    } else {
        let mut edited = occ.event.clone();
        edited.start = start;
        edited.end = end;
        convert_kind(&mut edited, all_day);
        store().update_occurrence(occ.event.id, occ.original_start, &edited).map(|_| ())
    };
    if app.check(r).is_some() {
        cal.close_popover();
        app.bump_calendar();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converting_between_all_day_and_timed() {
        let mut e = Event { show_as: ShowAs::Busy, ..Default::default() };
        convert_kind(&mut e, true);
        assert_eq!((e.all_day, e.show_as), (true, ShowAs::Free));
        convert_kind(&mut e, false);
        assert_eq!((e.all_day, e.show_as), (false, ShowAs::Busy));
        // Other Show-as choices are kept.
        let mut oof = Event { show_as: ShowAs::OutOfOffice, ..Default::default() };
        convert_kind(&mut oof, true);
        assert_eq!(oof.show_as, ShowAs::OutOfOffice);
    }
}
