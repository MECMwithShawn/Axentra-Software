//! Calendar module state, provided as a module-local context at the module
//! root (`use_cal()`), plus the small value types the views pass around.

use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;
use std::time::Instant;

use axentra_core::*;
use axentra_core::Event;
use chrono::{DateTime, Duration, Local, NaiveDate, NaiveTime, Timelike, Utc};
use dioxus::prelude::*;

use super::range::{self, ViewKind};
use crate::state::{AppState, store};

/// kv keys for per-profile calendar view state.
const KV_DAY_COUNT: &str = "calendar.day_count";
const KV_TIME_SCALE: &str = "calendar.time_scale";

/// Identifies one occurrence: the event row plus the occurrence's original start.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct OccKey {
    pub event_id: EventId,
    pub original_start: DateTime<Utc>,
}

impl OccKey {
    pub fn of(o: &Occurrence) -> Self {
        Self { event_id: o.event.id, original_start: o.original_start }
    }

    /// Value of the `data-occ` attribute on the occurrence's element.
    pub fn dom_id(&self) -> String {
        format!("{}-{}", self.event_id, self.original_start.timestamp())
    }
}

/// A rectangle in viewport (client) coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Anchor {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Anchor {
    pub fn point(x: f64, y: f64) -> Self {
        Self { x, y, w: 0.0, h: 0.0 }
    }
}

/// A selected slot range in the time grid: the same time span on each day
/// from `start_day` to `end_day` (an event made from it runs from
/// `start_day` + `start_min` to `end_day` + `end_min`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlotSel {
    pub start_day: NaiveDate,
    pub end_day: NaiveDate,
    pub start_min: i64,
    pub end_min: i64,
    pub all_day: bool,
}

impl SlotSel {
    pub fn start(&self) -> DateTime<Utc> {
        if self.all_day { at_min(self.start_day, 0) } else { at_min(self.start_day, self.start_min) }
    }

    pub fn end(&self) -> DateTime<Utc> {
        if self.all_day { at_min(self.end_day + Duration::days(1), 0) } else { at_min(self.end_day, self.end_min) }
    }

    pub fn covers(&self, day: NaiveDate) -> bool {
        day >= self.start_day && day <= self.end_day
    }
}

/// UTC instant of `minutes` after local midnight of `day` (wall clock).
pub fn at_min(day: NaiveDate, minutes: i64) -> DateTime<Utc> {
    let minutes = minutes.clamp(0, 24 * 60);
    if minutes == 24 * 60 {
        return local_to_utc(day + Duration::days(1), NaiveTime::MIN);
    }
    local_to_utc(day, NaiveTime::from_hms_opt((minutes / 60) as u32, (minutes % 60) as u32, 0).expect("valid time"))
}

/// Transient surfaces anchored on the grid.
#[derive(Clone, Debug, PartialEq)]
pub enum Popover {
    Peek { occ: Box<Occurrence>, anchor: Anchor },
    QuickCreate { sel: SlotSel, anchor: Anchor },
    DayEvents { date: NaiveDate, anchor: Anchor },
    Context { occ: Box<Occurrence>, x: f64, y: f64 },
}

/// How the editor saves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditMode {
    New,
    /// A single event, or an exception already split from a series.
    Single,
    /// One occurrence of a series ("This event"): saved as an exception.
    Occurrence { series_id: EventId, original_start: DateTime<Utc> },
    /// "Edit this and all following events": the series ends before this
    /// occurrence and the edit starts a new series there.
    Following { series_id: EventId, original_start: DateTime<Utc> },
    /// The whole series (the master row).
    Series,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EditorRequest {
    pub event: Event,
    pub mode: EditMode,
    /// Bumped per open so the form re-initialises.
    pub serial: u64,
}

/// What a drag does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragKind {
    /// Select empty slots (then quick-create).
    Create,
    Move,
    Resize,
}

/// Pointer drag in the time grid or month grid. Positions are a visible-day
/// index plus minutes from midnight (month view uses minutes = 0).
#[derive(Clone, Debug, PartialEq)]
pub struct Drag {
    pub kind: DragKind,
    pub occ: Option<Box<Occurrence>>,
    /// Pointer position at mouse down (client coordinates).
    pub client: (f64, f64),
    /// True once the pointer moved past the drag threshold.
    pub active: bool,
    pub origin_day: usize,
    pub origin_min: i64,
    pub cur_day: usize,
    pub cur_min: i64,
    /// Client y of the day columns' top edge at mouse down and the grid's
    /// scroll offset then: minutes = (y - col_top + scrolled) / px per minute.
    pub col_top: f64,
    pub scroll0: f64,
    /// Client x of the origin day column's left edge and the column width
    /// (0 when unknown), to anchor quick create beside the selection.
    pub col_left: f64,
    pub col_w: f64,
    /// False for events the user can't reschedule: the gesture is a click.
    pub movable: bool,
    /// Month view drags move by whole days.
    pub month: bool,
    /// The pointer is over the all-day row (a drop there makes the event
    /// all-day; an all-day event dropped on the grid becomes timed).
    pub allday: bool,
}

/// Pointer travel before a press becomes a drag.
pub const DRAG_THRESHOLD: f64 = 4.0;

impl Drag {
    /// Update the pointer position; returns true when anything visible changed.
    pub fn track(&mut self, day: usize, min: i64, client: (f64, f64)) -> bool {
        let was_active = self.active;
        if !self.active && self.movable {
            let (dx, dy) = (client.0 - self.client.0, client.1 - self.client.1);
            self.active = dx.hypot(dy) >= DRAG_THRESHOLD;
        }
        let moved = self.cur_day != day || self.cur_min != min;
        self.cur_day = day;
        self.cur_min = min;
        self.active != was_active || (self.active && moved)
    }
}

/// A modal flow waiting for the user.
#[derive(Clone, Debug, PartialEq)]
pub enum CalDialog {
    /// Recurring: "This event" / "This and all following events" / "All events in the series".
    DeleteScope(Box<Occurrence>),
    /// Organizer of a meeting: cancellation with a message.
    CancelMeeting(Box<Occurrence>),
    /// Invitation from someone else: "Delete" or "Delete and send decline".
    AttendeeDelete(Box<Occurrence>),
    /// A recurring occurrence was dropped somewhere else (`all_day` is the
    /// kind it becomes: dropping on the all-day row or the grid converts it).
    MoveScope { occ: Box<Occurrence>, start: DateTime<Utc>, end: DateTime<Utc>, all_day: bool },
    /// A meeting I organise was moved: "Send update?".
    SendUpdate { event: Box<Event> },
    AddCalendar,
    DeleteCalendar(CalendarId),
    Shortcuts,
}

/// Client-side filter (ribbon Filter menu). `None` sets mean "any".
#[derive(Clone, Debug, PartialEq)]
pub struct EventFilter {
    pub appointments: bool,
    pub meetings: bool,
    /// Category names; "" stands for "Not categorized".
    pub categories: Option<BTreeSet<String>>,
    pub show_as: Option<BTreeSet<&'static str>>,
    /// Some(true) = recurring only, Some(false) = not recurring only.
    pub recurring: Option<bool>,
    /// Some(true) = private only, Some(false) = not private only.
    pub private: Option<bool>,
    pub hide_declined: bool,
}

impl Default for EventFilter {
    fn default() -> Self {
        Self { appointments: true, meetings: true, categories: None, show_as: None, recurring: None, private: None, hide_declined: true }
    }
}

impl EventFilter {
    /// True when anything other than the defaults narrows the view.
    pub fn is_active(&self) -> bool {
        let base = Self { hide_declined: self.hide_declined, ..Self::default() };
        *self != base
    }

    pub fn matches(&self, e: &Event) -> bool {
        let meeting = !e.attendees.is_empty();
        if (meeting && !self.meetings) || (!meeting && !self.appointments) {
            return false;
        }
        if self.hide_declined && e.my_response == ResponseStatus::Declined {
            return false;
        }
        if let Some(cats) = &self.categories {
            let ok = if e.categories.is_empty() { cats.contains("") } else { e.categories.iter().any(|c| cats.contains(c)) };
            if !ok {
                return false;
            }
        }
        if let Some(sa) = &self.show_as
            && !sa.contains(super::text::show_as_id(e.show_as))
        {
            return false;
        }
        let recurring = e.recurrence.is_some() || e.series_id.is_some();
        if self.recurring.is_some_and(|r| r != recurring) {
            return false;
        }
        if self.private.is_some_and(|p| p != e.is_private) {
            return false;
        }
        true
    }
}

/// Requests for the time grid's scroll container.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScrollTo {
    /// Current time a third of the way down if today is visible, else 30
    /// minutes before the start of work.
    Initial,
    Minutes(i64),
    PageUp,
    PageDown,
}

/// Module-local state. Copy, like `AppState`.
#[derive(Clone, Copy)]
pub struct Cal {
    pub view: Signal<ViewKind>,
    pub anchor: Signal<NaiveDate>,
    pub today: Signal<NaiveDate>,
    pub now: Signal<DateTime<Utc>>,
    /// First day of the month shown in the folder-pane mini month.
    pub mini_month: Signal<NaiveDate>,
    pub selected: Signal<Option<OccKey>>,
    pub slot: Signal<Option<SlotSel>>,
    pub popover: Signal<Option<Popover>>,
    pub editor: Signal<Option<EditorRequest>>,
    pub dialog: Signal<Option<CalDialog>>,
    pub filter: Signal<EventFilter>,
    /// Time scale in minutes (5, 6, 10, 15, 30, 60).
    pub scale: Signal<u32>,
    pub drag: Signal<Option<Drag>>,
    /// Calendar picked in the folder pane (new events go there).
    pub active_calendar: Signal<Option<CalendarId>>,
    pub allday_expanded: Signal<bool>,
    pub date_flyout: Signal<bool>,
    pub scroll: Signal<Option<(u64, ScrollTo)>>,
    /// Serial of the last scroll request a grid carried out.
    pub scroll_applied: Signal<u64>,
    pub scroll_top: Signal<f64>,
    /// Polite live-region text after navigation.
    pub announce: Signal<String>,
    /// Folder-pane "Show all" state per group.
    pub expanded_groups: Signal<BTreeSet<String>>,
    pub collapsed_groups: Signal<BTreeSet<String>>,
    /// Calendar row being renamed inline.
    pub renaming: Signal<Option<CalendarId>>,
    /// Bumped to make the folder pane "shake" a refused checkbox.
    pub shake: Signal<Option<(CalendarId, u64)>>,
    /// Mounted event blocks, to anchor the peek card (only ever `peek()`ed).
    pub blocks: Signal<HashMap<OccKey, Rc<MountedData>>>,
    /// Grid scroll offset when the current popover opened.
    pub popover_scroll: Signal<f64>,
    /// Window size (for placing popovers).
    pub viewport: Signal<(f64, f64)>,
    /// Last click on an event (double-click detection survives drag handling).
    pub last_click: Signal<Option<(Occurrence, Instant)>>,
    /// Bumped whenever a peek card is requested or popovers close, so a peek
    /// still measuring its anchor never opens over a newer surface.
    pub peek_serial: Signal<u64>,
    serial: Signal<u64>,
}

pub fn use_cal() -> Cal {
    use_context::<Cal>()
}

/// Create the module state from settings and persisted view state.
pub fn provide_cal(app: AppState) -> Cal {
    use_context_provider(move || {
        let s = app.settings.peek().clone();
        let kv = |k: &str| store().kv_get(k).ok().flatten();
        let day_count = kv(KV_DAY_COUNT).and_then(|v| v.parse::<u8>().ok()).unwrap_or(1);
        let scale = kv(KV_TIME_SCALE).and_then(|v| v.parse::<u32>().ok()).filter(|v| SCALES.contains(v)).unwrap_or(30);
        let today = Local::now().date_naive();
        Cal {
            view: Signal::new(ViewKind::from_settings(s.calendar_view, day_count)),
            anchor: Signal::new(today),
            today: Signal::new(today),
            now: Signal::new(Utc::now()),
            mini_month: Signal::new(range::first_of_month(today)),
            selected: Signal::new(None),
            slot: Signal::new(None),
            popover: Signal::new(None),
            editor: Signal::new(None),
            dialog: Signal::new(None),
            filter: Signal::new(EventFilter::default()),
            scale: Signal::new(scale),
            drag: Signal::new(None),
            active_calendar: Signal::new(None),
            allday_expanded: Signal::new(false),
            date_flyout: Signal::new(false),
            scroll: Signal::new(Some((1, ScrollTo::Initial))),
            scroll_applied: Signal::new(0),
            scroll_top: Signal::new(0.0),
            announce: Signal::new(String::new()),
            expanded_groups: Signal::new(BTreeSet::new()),
            collapsed_groups: Signal::new(BTreeSet::new()),
            renaming: Signal::new(None),
            shake: Signal::new(None),
            blocks: Signal::new(HashMap::new()),
            popover_scroll: Signal::new(0.0),
            viewport: Signal::new((1440.0, 900.0)),
            last_click: Signal::new(None),
            peek_serial: Signal::new(0),
            serial: Signal::new(1),
        }
    })
}

/// Time scale choices (minutes), as in View › Time scale.
pub const SCALES: [u32; 6] = [60, 30, 15, 10, 6, 5];

/// Height of one slot for a time scale. 30-minute slots are 40px (80px per
/// hour, measured); other scales keep slots readable.
pub fn slot_height(scale: u32) -> f64 {
    match scale {
        60 | 30 => 40.0,
        15 => 32.0,
        10 => 28.0,
        _ => 24.0,
    }
}

/// Pixels per minute of the time grid.
pub fn px_per_min(scale: u32) -> f64 {
    slot_height(scale) / scale.max(1) as f64
}

impl Cal {
    fn next_serial(mut self) -> u64 {
        let n = *self.serial.peek() + 1;
        self.serial.set(n);
        n
    }

    /// Visible days for the current view.
    pub fn days(&self, s: &Settings) -> Vec<NaiveDate> {
        range::visible_days((self.view)(), (self.anchor)(), s.week_start, &s.work_days)
    }

    pub fn close_popover(mut self) {
        // Drop any peek card still being placed.
        let n = *self.peek_serial.peek() + 1;
        self.peek_serial.set(n);
        if self.popover.peek().is_some() {
            self.popover.set(None);
        }
    }

    pub fn clear_selection(mut self) {
        if self.selected.peek().is_some() {
            self.selected.set(None);
        }
        if self.slot.peek().is_some() {
            self.slot.set(None);
        }
    }

    pub fn request_scroll(mut self, to: ScrollTo) {
        let n = self.next_serial();
        self.scroll.set(Some((n, to)));
    }

    fn after_navigate(mut self, s: &Settings) {
        self.close_popover();
        self.clear_selection();
        self.date_flyout.set(false);
        let anchor = *self.anchor.peek();
        let first = range::first_of_month(anchor);
        if *self.mini_month.peek() != first {
            self.mini_month.set(first);
        }
        let view = *self.view.peek();
        let days = range::visible_days(view, anchor, s.week_start, &s.work_days);
        self.announce.set(range::announce_text(view, anchor, &days));
    }

    /// Show the range containing `date` in the current view.
    pub fn go_to(mut self, s: &Settings, date: NaiveDate) {
        self.anchor.set(date);
        self.after_navigate(s);
    }

    pub fn step(mut self, s: &Settings, dir: i32) {
        let a = range::step(*self.view.peek(), *self.anchor.peek(), dir);
        self.anchor.set(a);
        self.after_navigate(s);
    }

    /// "Today": go to today's range; if it is already showing, scroll to now.
    pub fn go_today(mut self, s: &Settings) {
        let today = *self.today.peek();
        let visible = range::visible_days(*self.view.peek(), *self.anchor.peek(), s.week_start, &s.work_days).contains(&today);
        self.anchor.set(today);
        self.after_navigate(s);
        if visible || (*self.view.peek()) != ViewKind::Month {
            self.request_scroll(ScrollTo::Initial);
        }
    }

    /// Switch view (persists it) keeping the anchor date.
    pub fn set_view(mut self, app: AppState, s: &Settings, v: ViewKind) {
        if let ViewKind::Day(n) = v {
            let _ = store().kv_set(KV_DAY_COUNT, &n.to_string());
        }
        self.view.set(v);
        if s.calendar_view != v.to_settings() {
            app.update_settings(|s| s.calendar_view = v.to_settings());
        }
        self.after_navigate(s);
        let range = self.announce.peek().clone();
        self.announce.set(format!("{} view, {range}", v.label()));
        if v != ViewKind::Month {
            self.request_scroll(ScrollTo::Initial);
        }
    }

    pub fn set_scale(mut self, scale: u32) {
        let _ = store().kv_set(KV_TIME_SCALE, &scale.to_string());
        // Keep the time at the top of the viewport while the grid zooms.
        let top_min = (*self.scroll_top.peek() / px_per_min(*self.scale.peek())) as i64;
        self.scale.set(scale);
        self.request_scroll(ScrollTo::Minutes(top_min));
    }

    /// Open the full editor for a new event.
    pub fn new_event(mut self, app: AppState, start: DateTime<Utc>, end: DateTime<Utc>, all_day: bool) {
        let event = new_event_draft(app, (self.active_calendar)(), start, end, all_day);
        self.close_popover();
        let serial = self.next_serial();
        self.editor.set(Some(EditorRequest { event, mode: EditMode::New, serial }));
    }

    /// Open the full editor with a prepared event.
    pub fn open_editor(mut self, event: Event, mode: EditMode) {
        self.close_popover();
        let serial = self.next_serial();
        self.editor.set(Some(EditorRequest { event, mode, serial }));
    }

    /// Open an occurrence in the full editor (an occurrence of a series opens
    /// as "this event", with "Edit series" offered in the form).
    pub fn open_occurrence(self, occ: &Occurrence) {
        let mut e = occ.event.clone();
        let mode = if e.recurrence.is_some() {
            e.start = occ.start;
            e.end = occ.end;
            EditMode::Occurrence { series_id: e.id, original_start: occ.original_start }
        } else {
            EditMode::Single
        };
        self.open_editor(e, mode);
    }

    /// Open the series an occurrence belongs to.
    pub fn open_series(self, series_id: EventId) {
        if let Ok(Some(master)) = store().event(series_id) {
            self.open_editor(master, EditMode::Series);
        }
    }

    /// Open "this and all following events" from the occurrence that
    /// started at `original_start` (now `start`–`end`).
    pub fn open_following(self, series_id: EventId, original_start: DateTime<Utc>, start: DateTime<Utc>, end: DateTime<Utc>) {
        if let Ok(Some(mut master)) = store().event(series_id) {
            master.start = start;
            master.end = end;
            master.exdates.clear();
            self.open_editor(master, EditMode::Following { series_id, original_start });
        }
    }
}

/// The next half-hour boundary after `now`.
pub fn next_half_hour(now: DateTime<Utc>) -> DateTime<Utc> {
    let l = now.with_timezone(&Local);
    let past = (l.minute() % 30) as i64 * 60 + l.second() as i64;
    let t = now - Duration::seconds(past) + Duration::minutes(30);
    t - Duration::nanoseconds(t.timestamp_subsec_nanos() as i64)
}

/// The user's own address (primary account).
pub fn my_address() -> Option<EmailAddress> {
    store().primary_account().ok().flatten().map(|a| EmailAddress::new(a.display_name, a.email))
}

/// True when I organise the event (or it's my own appointment).
pub fn i_organize(e: &Event, me: Option<&EmailAddress>) -> bool {
    match (&e.organizer, e.my_response) {
        (_, ResponseStatus::Organizer) | (None, _) => true,
        (Some(o), _) => me.is_some_and(|m| m.address.eq_ignore_ascii_case(&o.address)),
    }
}

/// A meeting someone else sent me.
pub fn is_invitation(e: &Event, me: Option<&EmailAddress>) -> bool {
    !i_organize(e, me)
}

/// A blank event with Outlook's defaults: Busy (Free for all-day), the
/// default reminder (18 hours before for all-day), in `calendar` if it is
/// writable, else the default calendar.
pub fn new_event_draft(app: AppState, calendar: Option<CalendarId>, start: DateTime<Utc>, end: DateTime<Utc>, all_day: bool) -> Event {
    let s = app.settings.peek().clone();
    let cals = store().calendars().unwrap_or_default();
    let calendar_id = calendar
        .and_then(|id| cals.iter().find(|c| c.id == id && c.can_edit))
        .or_else(|| cals.iter().find(|c| c.is_default && c.can_edit))
        .or_else(|| cals.iter().find(|c| c.can_edit))
        .map(|c| c.id)
        .unwrap_or_default();
    Event {
        calendar_id,
        start,
        end,
        all_day,
        time_zone: "Local".into(),
        show_as: if all_day { ShowAs::Free } else { ShowAs::Busy },
        reminder_minutes: Some(if all_day { 18 * 60 } else { s.default_reminder_minutes }),
        organizer: my_address(),
        my_response: ResponseStatus::Organizer,
        ..Default::default()
    }
}

/// Invite `people` to a new event (NavRequest::NewEventWith): required
/// attendees, each address once, never the organizer.
pub fn add_attendees(event: &mut Event, people: &[EmailAddress]) {
    for p in people {
        let addr = p.address.trim();
        let is_organizer = event.organizer.as_ref().is_some_and(|o| o.address.eq_ignore_ascii_case(addr));
        if addr.is_empty() || is_organizer || event.attendees.iter().any(|a| a.email.address.eq_ignore_ascii_case(addr)) {
            continue;
        }
        event.attendees.push(Attendee { email: EmailAddress::new(p.name.clone(), addr), ..Default::default() });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invited_people_become_required_attendees_once() {
        let mut e = Event { organizer: Some(EmailAddress::new("Me", "me@example.com")), ..Default::default() };
        add_attendees(
            &mut e,
            &[
                EmailAddress::new("Ada", "ada@example.com"),
                EmailAddress::new("", "ADA@example.com"),
                EmailAddress::new("Me", "Me@Example.com"),
                EmailAddress::new("", " "),
            ],
        );
        assert_eq!(e.attendees.len(), 1);
        assert_eq!((e.attendees[0].email.name.as_str(), e.attendees[0].kind), ("Ada", AttendeeKind::Required));
    }

    #[test]
    fn filter_matching() {
        let meeting = Event { attendees: vec![Attendee::default()], ..Default::default() };
        let appt = Event::default();
        let mut f = EventFilter::default();
        assert!(!f.is_active());
        assert!(f.matches(&meeting) && f.matches(&appt));
        f.meetings = false;
        assert!(f.is_active());
        assert!(!f.matches(&meeting) && f.matches(&appt));
        let mut f = EventFilter { categories: Some(["".to_string()].into()), ..Default::default() };
        assert!(f.matches(&appt));
        assert!(!f.matches(&Event { categories: vec!["Red category".into()], ..Default::default() }));
        f.categories = None;
        f.show_as = Some(["free"].into());
        assert!(!f.matches(&appt));
        assert!(f.matches(&Event { show_as: ShowAs::Free, ..Default::default() }));
        let declined = Event { my_response: ResponseStatus::Declined, ..Default::default() };
        assert!(!EventFilter::default().matches(&declined));
        assert!(EventFilter { hide_declined: false, ..Default::default() }.matches(&declined));
        assert!(!EventFilter { hide_declined: false, ..Default::default() }.is_active());
        let f = EventFilter { recurring: Some(true), private: Some(false), ..Default::default() };
        assert!(!f.matches(&appt));
        assert!(f.matches(&Event { series_id: Some(3), ..Default::default() }));
    }

    #[test]
    fn slot_selection_times_and_half_hours() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 19).unwrap();
        let sel = SlotSel { start_day: day, end_day: day, start_min: 9 * 60, end_min: 10 * 60 + 30, all_day: false };
        assert_eq!(sel.end() - sel.start(), Duration::minutes(90));
        let all = SlotSel { all_day: true, ..sel };
        assert_eq!(local_date(all.start()), day);
        assert_eq!(local_date(all.end()), day + Duration::days(1));
        let t = |h, m, s| local_to_utc(day, NaiveTime::from_hms_opt(h, m, s).unwrap());
        assert_eq!(next_half_hour(t(9, 10, 5)), t(9, 30, 0));
        assert_eq!(next_half_hour(t(9, 30, 0)), t(10, 0, 0));
        assert_eq!(next_half_hour(t(9, 59, 59)), t(10, 0, 0));
        assert_eq!(at_min(day, 24 * 60), local_to_utc(day + Duration::days(1), NaiveTime::MIN));
        assert_eq!(px_per_min(30), 80.0 / 60.0);
    }

    #[test]
    fn organizer_detection() {
        let me = EmailAddress::new("Me", "me@x.example");
        let mine = Event { organizer: Some(me.clone()), my_response: ResponseStatus::Accepted, ..Default::default() };
        assert!(i_organize(&mine, Some(&me)));
        let theirs = Event { organizer: Some(EmailAddress::new("P", "p@x.example")), my_response: ResponseStatus::NotResponded, ..Default::default() };
        assert!(is_invitation(&theirs, Some(&me)));
        assert!(i_organize(&Event::default(), Some(&me)));
    }
}
