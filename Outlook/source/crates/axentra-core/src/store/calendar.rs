//! Calendars and events.
//!
//! Series are stored once with a [`Recurrence`] and expanded on read.
//! Editing one occurrence ("This event") adds its date to the series'
//! `exdates` and saves the edited copy as a standalone event whose
//! `series_id` / `original_start` point back at the series (an exception).

use std::collections::HashMap;

use chrono::{DateTime, Duration, Local, NaiveDate, TimeZone, Utc};
use rusqlite::params;

use super::Store;
use crate::model::*;
use crate::recurrence::{count_before, occurrence_dates};
use crate::{Error, Result};

pub(crate) const SCHEMA_EXTRA: &str = r#"
CREATE TABLE IF NOT EXISTS reminder_state (
    event_id INTEGER NOT NULL REFERENCES events(id) ON DELETE CASCADE,
    occurrence_start INTEGER NOT NULL,
    dismissed INTEGER NOT NULL DEFAULT 0,
    snoozed_until INTEGER,
    PRIMARY KEY (event_id, occurrence_start));
"#;

/// Local calendar date of a UTC instant.
pub fn local_date(t: DateTime<Utc>) -> NaiveDate {
    t.with_timezone(&Local).date_naive()
}

/// UTC instant of a local wall-clock date + time (earliest if ambiguous,
/// shifted forward over a DST gap).
pub fn local_to_utc(date: NaiveDate, time: chrono::NaiveTime) -> DateTime<Utc> {
    let naive = date.and_time(time);
    match Local.from_local_datetime(&naive) {
        chrono::LocalResult::Single(t) => t.with_timezone(&Utc),
        chrono::LocalResult::Ambiguous(a, _) => a.with_timezone(&Utc),
        chrono::LocalResult::None => Local
            .from_local_datetime(&(naive + Duration::hours(1)))
            .earliest()
            .map(|t| t.with_timezone(&Utc))
            .unwrap_or_else(|| Utc.from_utc_datetime(&naive)),
    }
}

/// The group every account has, listed first in the folder pane.
const DEFAULT_GROUP: &str = "My calendars";

/// kv key prefix for an account's calendar group order (JSON list).
const KV_GROUPS: &str = "calendar.groups.";

/// The group a calendar is listed under (blank means "My calendars").
fn group_of(c: &Calendar) -> &str {
    if c.group.trim().is_empty() { DEFAULT_GROUP } else { c.group.trim() }
}

/// Persisted reminder state per (event id, occurrence start timestamp):
/// (dismissed, snoozed until).
type ReminderStates = HashMap<(EventId, i64), (bool, Option<DateTime<Utc>>)>;

fn not_found(id: EventId) -> Error {
    Error::NotFound(format!("event {id}"))
}

fn check_times(e: &Event) -> Result<()> {
    if e.end < e.start {
        return Err(Error::Invalid("event ends before it starts".into()));
    }
    Ok(())
}

impl Store {
    pub fn calendars(&self) -> Result<Vec<Calendar>> {
        self.query_docs("ORDER BY account_id, sort_order, id", [])
    }

    pub fn calendar(&self, id: CalendarId) -> Result<Option<Calendar>> {
        self.get_doc(id)
    }

    /// The default calendar of an account, or of the primary account.
    pub fn default_calendar(&self, account_id: Option<AccountId>) -> Result<Option<Calendar>> {
        let cals = self.calendars()?;
        let account_id = match account_id {
            Some(a) => a,
            None => match self.primary_account()? {
                Some(a) => a.id,
                None => return Ok(None),
            },
        };
        Ok(cals
            .iter()
            .find(|c| c.account_id == account_id && c.is_default)
            .or_else(|| cals.iter().find(|c| c.account_id == account_id))
            .cloned())
    }

    pub fn add_calendar(&self, calendar: &mut Calendar) -> Result<CalendarId> {
        self.insert_doc(calendar)
    }

    /// Create a blank local calendar at the end of `group` ("Add calendar" ›
    /// "Create blank calendar"). The name must not be empty.
    pub fn create_calendar(&self, account_id: AccountId, name: &str, color: &str, group: &str) -> Result<CalendarId> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::Invalid("A calendar needs a name.".into()));
        }
        let sort_order = self.calendars()?.iter().filter(|c| c.account_id == account_id).map(|c| c.sort_order + 1).max().unwrap_or(0);
        let mut cal = Calendar {
            account_id,
            name: name.to_string(),
            color: color.to_string(),
            is_visible: true,
            can_edit: true,
            group: if group.trim().is_empty() { "My calendars".into() } else { group.trim().to_string() },
            sort_order,
            ..Default::default()
        };
        self.insert_doc(&mut cal)
    }

    pub fn update_calendar(&self, calendar: &Calendar) -> Result<()> {
        self.update_doc(calendar)
    }

    pub fn delete_calendar(&self, id: CalendarId) -> Result<()> {
        self.delete_doc::<Calendar>(id)
    }

    pub fn set_calendar_visible(&self, id: CalendarId, visible: bool) -> Result<()> {
        let mut c: Calendar = self.get_doc(id)?.ok_or_else(|| Error::NotFound(format!("calendar {id}")))?;
        c.is_visible = visible;
        self.update_doc(&c)
    }

    /// Rename a calendar (trimmed, must not be empty).
    pub fn rename_calendar(&self, id: CalendarId, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::Invalid("A calendar needs a name.".into()));
        }
        let mut c: Calendar = self.get_doc(id)?.ok_or_else(|| Error::NotFound(format!("calendar {id}")))?;
        c.name = name.to_string();
        self.update_doc(&c)
    }

    pub fn set_calendar_color(&self, id: CalendarId, color: &str) -> Result<()> {
        let mut c: Calendar = self.get_doc(id)?.ok_or_else(|| Error::NotFound(format!("calendar {id}")))?;
        c.color = color.to_string();
        self.update_doc(&c)
    }

    /// Calendar groups of an account in folder-pane order: "My calendars"
    /// first, then the saved order (groups made with "New calendar group"
    /// exist before they hold a calendar), then any other group a calendar
    /// names, in calendar order.
    pub fn calendar_groups(&self, account_id: AccountId) -> Result<Vec<String>> {
        let mut out = vec![DEFAULT_GROUP.to_string()];
        let mut push = |g: &str| {
            if !out.iter().any(|x| x.eq_ignore_ascii_case(g)) {
                out.push(g.to_string());
            }
        };
        for g in self.saved_groups(account_id)? {
            push(&g);
        }
        for c in self.calendars()?.iter().filter(|c| c.account_id == account_id) {
            push(group_of(c));
        }
        Ok(out)
    }

    /// "New calendar group": an empty group at the end of the list.
    pub fn add_calendar_group(&self, account_id: AccountId, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::Invalid("A calendar group needs a name.".into()));
        }
        let mut groups = self.calendar_groups(account_id)?;
        if groups.iter().any(|g| g.eq_ignore_ascii_case(name)) {
            return Err(Error::Invalid(format!("There's already a group called \u{201c}{name}\u{201d}.")));
        }
        groups.push(name.to_string());
        self.save_groups(account_id, &groups)
    }

    /// Rename a group and move its calendars along. "My calendars" keeps its name.
    pub fn rename_calendar_group(&self, account_id: AccountId, old: &str, new: &str) -> Result<()> {
        let new = new.trim();
        if new.is_empty() {
            return Err(Error::Invalid("A calendar group needs a name.".into()));
        }
        if old == DEFAULT_GROUP {
            return Err(Error::Invalid("\u{201c}My calendars\u{201d} can't be renamed.".into()));
        }
        let mut groups = self.calendar_groups(account_id)?;
        if groups.iter().any(|g| g.eq_ignore_ascii_case(new) && g != old) {
            return Err(Error::Invalid(format!("There's already a group called \u{201c}{new}\u{201d}.")));
        }
        let Some(pos) = groups.iter().position(|g| g == old) else {
            return Err(Error::NotFound(format!("calendar group {old}")));
        };
        groups[pos] = new.to_string();
        self.tx(|tx| {
            let cals: Vec<Calendar> = Self::query_docs_in(tx, "WHERE account_id = ?1", [account_id])?;
            for mut c in cals.into_iter().filter(|c| group_of(c) == old) {
                c.group = new.to_string();
                Self::update_doc_in(tx, &c)?;
            }
            Ok(())
        })?;
        self.save_groups(account_id, &groups)
    }

    /// Delete an empty group ("My calendars" always stays).
    pub fn delete_calendar_group(&self, account_id: AccountId, name: &str) -> Result<()> {
        if name == DEFAULT_GROUP {
            return Err(Error::Invalid("\u{201c}My calendars\u{201d} can't be deleted.".into()));
        }
        if self.calendars()?.iter().any(|c| c.account_id == account_id && group_of(c) == name) {
            return Err(Error::Invalid("Move or delete the calendars in this group first.".into()));
        }
        let mut groups = self.calendar_groups(account_id)?;
        groups.retain(|g| g != name);
        self.save_groups(account_id, &groups)
    }

    /// "Move up" (`delta` -1) / "Move down" (+1) among the groups after
    /// "My calendars", which stays first.
    pub fn move_calendar_group(&self, account_id: AccountId, name: &str, delta: i32) -> Result<()> {
        let mut groups = self.calendar_groups(account_id)?;
        let Some(pos) = groups.iter().position(|g| g == name) else {
            return Err(Error::NotFound(format!("calendar group {name}")));
        };
        let to = pos as i64 + delta as i64;
        if pos == 0 || to < 1 || to >= groups.len() as i64 {
            return Ok(());
        }
        groups.swap(pos, to as usize);
        self.save_groups(account_id, &groups)
    }

    fn saved_groups(&self, account_id: AccountId) -> Result<Vec<String>> {
        Ok(match self.kv_get(&format!("{KV_GROUPS}{account_id}"))? {
            Some(v) => serde_json::from_str(&v)?,
            None => Vec::new(),
        })
    }

    fn save_groups(&self, account_id: AccountId, groups: &[String]) -> Result<()> {
        self.kv_set(&format!("{KV_GROUPS}{account_id}"), &serde_json::to_string(groups)?)
    }

    /// "Show only this calendar": hide every other calendar.
    pub fn show_only_calendar(&self, id: CalendarId) -> Result<()> {
        self.tx(|tx| {
            let cals: Vec<Calendar> = Self::query_docs_in(tx, "", [])?;
            if !cals.iter().any(|c| c.id == id) {
                return Err(Error::NotFound(format!("calendar {id}")));
            }
            for mut c in cals {
                let visible = c.id == id;
                if c.is_visible != visible {
                    c.is_visible = visible;
                    Self::update_doc_in(tx, &c)?;
                }
            }
            Ok(())
        })
    }

    pub fn event(&self, id: EventId) -> Result<Option<Event>> {
        self.get_doc(id)
    }

    pub fn event_by_remote_id(&self, calendar_id: CalendarId, remote_id: &str) -> Result<Option<Event>> {
        Ok(self
            .query_docs::<Event>("WHERE calendar_id = ?1 AND remote_id = ?2 LIMIT 1", params![calendar_id, remote_id])?
            .into_iter()
            .next())
    }

    pub fn insert_event(&self, event: &mut Event) -> Result<EventId> {
        check_times(event)?;
        let now = Utc::now();
        if event.created_at.timestamp() == 0 {
            event.created_at = now;
        }
        event.updated_at = now;
        self.insert_doc(event)
    }

    pub fn update_event(&self, event: &Event) -> Result<()> {
        check_times(event)?;
        let mut e = event.clone();
        e.updated_at = Utc::now();
        self.update_doc(&e)
    }

    /// Load an event, change it with `f` and save it.
    pub fn modify_event(&self, id: EventId, f: impl FnOnce(&mut Event)) -> Result<()> {
        let mut e: Event = self.get_doc(id)?.ok_or_else(|| not_found(id))?;
        f(&mut e);
        self.update_event(&e)
    }

    pub fn delete_event(&self, id: EventId) -> Result<()> {
        self.delete_doc::<Event>(id)
    }

    /// Delete one occurrence of a series (adds an exception date).
    pub fn delete_occurrence(&self, id: EventId, original_start: DateTime<Utc>) -> Result<()> {
        let mut e: Event = self.get_doc(id)?.ok_or_else(|| not_found(id))?;
        if e.recurrence.is_none() {
            return self.delete_event(id);
        }
        let d = local_date(original_start);
        if !e.exdates.contains(&d) {
            e.exdates.push(d);
        }
        self.update_doc(&e)
    }

    /// Edited occurrences ("exceptions") saved from a series, by original start.
    pub fn exceptions_of(&self, series_id: EventId) -> Result<Vec<Event>> {
        let mut out: Vec<Event> = self.query_docs("WHERE json_extract(json, '$.series_id') = ?1", [series_id])?;
        out.sort_by_key(|e| e.original_start.unwrap_or(e.start));
        Ok(out)
    }

    /// Save `edited` as the new version of one occurrence of a series
    /// ("This event"). The series skips that date from now on and the edit is
    /// stored as an exception linked to it. For an event that is not a series
    /// this is a plain update. Returns the id of the saved event.
    pub fn update_occurrence(&self, series_id: EventId, original_start: DateTime<Utc>, edited: &Event) -> Result<EventId> {
        check_times(edited)?;
        let now = Utc::now();
        self.tx(|tx| {
            let mut master: Event = Self::get_doc_in(tx, series_id)?.ok_or_else(|| not_found(series_id))?;
            if master.recurrence.is_none() {
                let mut e = edited.clone();
                e.id = series_id;
                e.updated_at = now;
                Self::update_doc_in(tx, &e)?;
                return Ok(series_id);
            }
            let d = local_date(original_start);
            if !master.exdates.contains(&d) {
                master.exdates.push(d);
            }
            master.updated_at = now;
            Self::update_doc_in(tx, &master)?;
            let mut ex = edited.clone();
            ex.id = 0;
            ex.remote_id = None;
            ex.recurrence = None;
            ex.exdates.clear();
            ex.series_id = Some(series_id);
            ex.original_start = Some(original_start);
            ex.created_at = now;
            ex.updated_at = now;
            Self::insert_doc_in(tx, &mut ex)
        })
    }

    /// Save a whole series. With `reset_exceptions` (the pattern changed), the
    /// edited and deleted occurrences are dropped, as Outlook does after its
    /// "Changing the series will remove changes made to individual
    /// occurrences" warning.
    pub fn update_series(&self, master: &Event, reset_exceptions: bool) -> Result<()> {
        check_times(master)?;
        self.tx(|tx| {
            let mut m = master.clone();
            m.updated_at = Utc::now();
            if reset_exceptions {
                m.exdates.clear();
                tx.execute("DELETE FROM events WHERE json_extract(json, '$.series_id') = ?1", [m.id])?;
            }
            Self::update_doc_in(tx, &m)
        })
    }

    /// Delete a series with its exceptions ("All events in the series").
    pub fn delete_series(&self, id: EventId) -> Result<()> {
        self.tx(|tx| {
            tx.execute("DELETE FROM events WHERE json_extract(json, '$.series_id') = ?1", [id])?;
            Self::delete_doc_in::<Event>(tx, id)
        })
    }

    /// "This and all following events": end the series the day before the
    /// occurrence that started at `original_start` and drop the exceptions
    /// from then on. From the first occurrence this deletes the whole series.
    pub fn delete_following(&self, id: EventId, original_start: DateTime<Utc>) -> Result<()> {
        let master: Event = self.get_doc(id)?.ok_or_else(|| not_found(id))?;
        if master.recurrence.is_none() {
            return self.delete_event(id);
        }
        if local_date(original_start) <= local_date(master.start) {
            return self.delete_series(id);
        }
        self.tx(|tx| Self::end_series_before_in(tx, master, original_start))
    }

    fn end_series_before_in(tx: &rusqlite::Connection, mut master: Event, original_start: DateTime<Utc>) -> Result<()> {
        let cut = local_date(original_start);
        if let Some(rule) = master.recurrence.as_mut() {
            let last = cut - Duration::days(1);
            rule.end = match rule.end {
                RecurrenceEnd::Until(u) if u < last => RecurrenceEnd::Until(u),
                _ => RecurrenceEnd::Until(last),
            };
        }
        master.exdates.retain(|d| *d < cut);
        master.updated_at = Utc::now();
        Self::update_doc_in(tx, &master)?;
        let exceptions: Vec<Event> =
            Self::query_docs_in(tx, "WHERE json_extract(json, '$.series_id') = ?1", [master.id])?;
        for ex in exceptions {
            if ex.original_start.is_some_and(|o| o >= original_start) {
                Self::delete_doc_in::<Event>(tx, ex.id)?;
            }
        }
        Ok(())
    }

    /// "This and all following events" for an edit: the old series ends
    /// before the occurrence and `new_series` (with its own recurrence, or the
    /// old one when `None`) starts there. A numbered end keeps the total count.
    /// Editing from the first occurrence updates the whole series instead.
    /// Returns the id of the series that now holds the occurrence.
    pub fn split_series(&self, id: EventId, original_start: DateTime<Utc>, new_series: &Event) -> Result<EventId> {
        check_times(new_series)?;
        let week_start = self.settings().week_start;
        let master: Event = self.get_doc(id)?.ok_or_else(|| not_found(id))?;
        let Some(old_rule) = master.recurrence.clone() else {
            let mut e = new_series.clone();
            e.id = id;
            self.update_event(&e)?;
            return Ok(id);
        };
        if local_date(original_start) <= local_date(master.start) {
            let mut e = new_series.clone();
            e.id = id;
            self.update_series(&e, e.recurrence != master.recurrence)?;
            return Ok(id);
        }
        let now = Utc::now();
        self.tx(|tx| {
            let mut rule = new_series.recurrence.clone().unwrap_or_else(|| old_rule.clone());
            if let RecurrenceEnd::Count(n) = rule.end {
                let before = count_before(&old_rule, local_date(master.start), week_start, local_date(original_start));
                rule.end = RecurrenceEnd::Count(n.saturating_sub(before).max(1));
            }
            Self::end_series_before_in(tx, master, original_start)?;
            let mut e = new_series.clone();
            e.id = 0;
            e.remote_id = None;
            e.recurrence = Some(rule);
            e.exdates.clear();
            e.series_id = None;
            e.original_start = None;
            e.created_at = now;
            e.updated_at = now;
            Self::insert_doc_in(tx, &mut e)
        })
    }

    /// Set my response to a meeting.
    pub fn respond_to_event(&self, id: EventId, response: ResponseStatus) -> Result<()> {
        self.respond_to_event_with(id, response, true, "")
    }

    /// [`Store::respond_to_event`] with the send options of spec §11.2:
    /// `send = false` is "Respond without email"; `note` goes to the
    /// organizer with the response. Sync reads both when it pushes the RSVP.
    pub fn respond_to_event_with(&self, id: EventId, response: ResponseStatus, send: bool, note: &str) -> Result<()> {
        let mut e: Event = self.get_doc(id)?.ok_or_else(|| not_found(id))?;
        e.my_response = response;
        e.response_silent = !send;
        e.response_note = if send { note.trim().to_string() } else { String::new() };
        e.show_as = match response {
            ResponseStatus::Tentative => ShowAs::Tentative,
            ResponseStatus::Declined => ShowAs::Free,
            // An unanswered invitation shows as Tentative; accepting makes it Busy.
            ResponseStatus::Accepted if e.show_as == ShowAs::Tentative => ShowAs::Busy,
            _ => e.show_as,
        };
        self.update_doc(&e)
    }

    /// Occurrences on visible calendars overlapping `[start, end)`, expanded
    /// from recurring series, sorted by start then longest first.
    pub fn events_in_range(&self, start: DateTime<Utc>, end: DateTime<Utc>) -> Result<Vec<Occurrence>> {
        self.occurrences_in_range(start, end, false)
    }

    /// Like [`Store::events_in_range`], optionally including hidden calendars
    /// (reminders fire for those too).
    pub fn occurrences_in_range(&self, start: DateTime<Utc>, end: DateTime<Utc>, include_hidden: bool) -> Result<Vec<Occurrence>> {
        let calendars = self.calendars()?;
        let shown: HashMap<CalendarId, &Calendar> =
            calendars.iter().filter(|c| include_hidden || c.is_visible).map(|c| (c.id, c)).collect();
        let events: Vec<Event> = self.query_docs(
            "WHERE is_recurring = 1 OR (start_ts < ?1 AND end_ts > ?2) OR (start_ts = end_ts AND start_ts >= ?2 AND start_ts < ?1)",
            params![end.timestamp(), start.timestamp()],
        )?;
        let week_start = self.settings().week_start;
        let mut out = Vec::new();
        for e in events {
            let Some(cal) = shown.get(&e.calendar_id) else { continue };
            if e.is_cancelled && e.my_response == ResponseStatus::Declined {
                continue;
            }
            match &e.recurrence {
                None => {
                    if e.start < end && (e.end > start || (e.start == e.end && e.start >= start)) {
                        let original_start = e.original_start.unwrap_or(e.start);
                        out.push(Occurrence { start: e.start, end: e.end, original_start, color: cal.color.clone(), event: e.clone() });
                    }
                }
                Some(rule) => {
                    let dur = e.end - e.start;
                    let local_start = e.start.with_timezone(&Local);
                    let first = local_start.date_naive();
                    let time = local_start.time();
                    // Widen the window by the duration so multi-day occurrences that began earlier are kept.
                    let from = local_date(start - dur) - Duration::days(1);
                    let to = local_date(end) + Duration::days(1);
                    for d in occurrence_dates(rule, first, week_start, &e.exdates, from, to) {
                        let s = local_to_utc(d, time);
                        let en = s + dur;
                        if s < end && (en > start || (dur.is_zero() && s >= start)) {
                            out.push(Occurrence { start: s, end: en, original_start: s, color: cal.color.clone(), event: e.clone() });
                        }
                    }
                }
            }
        }
        out.sort_by(|a, b| a.start.cmp(&b.start).then((b.end - b.start).cmp(&(a.end - a.start))).then(a.event.id.cmp(&b.event.id)));
        Ok(out)
    }

    /// Reminders due at `now`: occurrences (on any calendar) whose reminder
    /// time has passed, that haven't ended, weren't dismissed and aren't
    /// snoozed, with the time each became due. Ended events are dismissed
    /// automatically. Sorted by start.
    pub fn due_reminders(&self, now: DateTime<Utc>) -> Result<Vec<(Occurrence, DateTime<Utc>)>> {
        // Reminders reach at most a week ahead ("1 week before").
        let occs = self.occurrences_in_range(now - Duration::days(1), now + Duration::days(8), true)?;
        let states = self.reminder_states()?;
        let mut out = Vec::new();
        for o in occs {
            let Some(minutes) = o.event.reminder_minutes else { continue };
            if minutes < 0 || o.end <= now || o.event.is_cancelled || o.event.my_response == ResponseStatus::Declined {
                continue;
            }
            let mut due = o.start - Duration::minutes(minutes as i64);
            match states.get(&(o.event.id, o.start.timestamp())) {
                Some((true, _)) => continue,
                Some((false, Some(until))) => due = due.max(*until),
                _ => {}
            }
            if due <= now {
                out.push((o, due));
            }
        }
        out.sort_by_key(|(o, _)| (o.start, o.event.id));
        Ok(out)
    }

    /// When the next reminder after `now` becomes due (to arm a timer).
    pub fn next_reminder_at(&self, now: DateTime<Utc>) -> Result<Option<DateTime<Utc>>> {
        let occs = self.occurrences_in_range(now - Duration::days(1), now + Duration::days(8), true)?;
        let states = self.reminder_states()?;
        let mut next: Option<DateTime<Utc>> = None;
        for o in occs {
            let Some(minutes) = o.event.reminder_minutes else { continue };
            if minutes < 0 || o.end <= now || o.event.is_cancelled {
                continue;
            }
            let mut due = o.start - Duration::minutes(minutes as i64);
            match states.get(&(o.event.id, o.start.timestamp())) {
                Some((true, _)) => continue,
                Some((false, Some(until))) => due = due.max(*until),
                _ => {}
            }
            if due > now && next.is_none_or(|n| due < n) {
                next = Some(due);
            }
        }
        Ok(next)
    }

    fn reminder_states(&self) -> Result<ReminderStates> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached("SELECT event_id, occurrence_start, dismissed, snoozed_until FROM reminder_state")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                (r.get::<_, i64>(0)?, r.get::<_, i64>(1)?),
                (r.get::<_, bool>(2)?, r.get::<_, Option<i64>>(3)?.and_then(|t| DateTime::from_timestamp(t, 0))),
            ))
        })?;
        let mut out = HashMap::new();
        for row in rows {
            let (k, v) = row?;
            out.insert(k, v);
        }
        Ok(out)
    }

    /// Dismiss the reminder of one occurrence (identified by its start).
    pub fn dismiss_reminder(&self, event_id: EventId, occurrence_start: DateTime<Utc>) -> Result<()> {
        self.conn().execute(
            "INSERT INTO reminder_state (event_id, occurrence_start, dismissed, snoozed_until) VALUES (?1, ?2, 1, NULL)
             ON CONFLICT(event_id, occurrence_start) DO UPDATE SET dismissed = 1, snoozed_until = NULL",
            params![event_id, occurrence_start.timestamp()],
        )?;
        Ok(())
    }

    /// Snooze the reminder of one occurrence until `until`.
    pub fn snooze_reminder(&self, event_id: EventId, occurrence_start: DateTime<Utc>, until: DateTime<Utc>) -> Result<()> {
        self.conn().execute(
            "INSERT INTO reminder_state (event_id, occurrence_start, dismissed, snoozed_until) VALUES (?1, ?2, 0, ?3)
             ON CONFLICT(event_id, occurrence_start) DO UPDATE SET dismissed = 0, snoozed_until = excluded.snoozed_until",
            params![event_id, occurrence_start.timestamp(), until.timestamp()],
        )?;
        Ok(())
    }

    /// Events whose title, location, body or organizer contains `q`
    /// (case-insensitive), newest first.
    pub fn search_events(&self, q: &str) -> Result<Vec<Event>> {
        let pat = format!("%{}%", q.trim());
        self.query_docs(
            "WHERE json_extract(json,'$.title') LIKE ?1 OR json_extract(json,'$.location') LIKE ?1 OR json_extract(json,'$.body_html') LIKE ?1
                OR json_extract(json,'$.organizer.name') LIKE ?1 OR json_extract(json,'$.organizer.address') LIKE ?1
             ORDER BY start_ts DESC LIMIT 200",
            [pat],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_util::store_with_account;
    use chrono::NaiveTime;

    fn t(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    fn day() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 19).unwrap()
    }

    fn standup(store: &Store, cal: CalendarId) -> Event {
        let mut e = Event {
            calendar_id: cal,
            title: "Standup".into(),
            start: local_to_utc(day(), t(9, 0)),
            end: local_to_utc(day(), t(9, 15)),
            recurrence: Some(Recurrence { freq: RecurrenceFreq::Daily, interval: 1, ..Default::default() }),
            ..Default::default()
        };
        store.insert_event(&mut e).unwrap();
        e
    }

    fn week(store: &Store) -> Vec<Occurrence> {
        let from = local_to_utc(day(), t(0, 0));
        store.events_in_range(from, from + Duration::days(7)).unwrap()
    }

    #[test]
    fn range_query_expands_series_and_hides_hidden_calendars() {
        let (store, _acct) = store_with_account();
        let cal = store.default_calendar(None).unwrap().unwrap();
        let mut single = Event {
            calendar_id: cal.id,
            title: "1:1".into(),
            start: local_to_utc(day(), t(10, 0)),
            end: local_to_utc(day(), t(10, 30)),
            ..Default::default()
        };
        store.insert_event(&mut single).unwrap();
        let standup = standup(&store, cal.id);

        let week_start = local_to_utc(day(), t(0, 0));
        let week_end = week_start + Duration::days(5);
        let occ = store.events_in_range(week_start, week_end).unwrap();
        assert_eq!(occ.iter().filter(|o| o.event.title == "Standup").count(), 5);
        assert_eq!(occ.iter().filter(|o| o.event.title == "1:1").count(), 1);
        assert_eq!(occ[0].event.title, "Standup");

        store.delete_occurrence(standup.id, local_to_utc(day() + Duration::days(1), t(9, 0))).unwrap();
        assert_eq!(store.events_in_range(week_start, week_end).unwrap().iter().filter(|o| o.event.title == "Standup").count(), 4);

        store.set_calendar_visible(cal.id, false).unwrap();
        assert!(store.events_in_range(week_start, week_end).unwrap().is_empty());
        assert!(!store.occurrences_in_range(week_start, week_end, true).unwrap().is_empty());
    }

    #[test]
    fn rejects_inverted_times_and_records_response() {
        let (store, _) = store_with_account();
        let cal = store.default_calendar(None).unwrap().unwrap();
        let now = Utc::now();
        let mut bad = Event { calendar_id: cal.id, start: now, end: now - Duration::hours(1), ..Default::default() };
        assert!(store.insert_event(&mut bad).is_err());
        let mut ok = Event { calendar_id: cal.id, start: now, end: now + Duration::hours(1), ..Default::default() };
        let id = store.insert_event(&mut ok).unwrap();
        store.respond_to_event(id, ResponseStatus::Tentative).unwrap();
        let e = store.event(id).unwrap().unwrap();
        assert_eq!((e.my_response, e.show_as), (ResponseStatus::Tentative, ShowAs::Tentative));
        store.respond_to_event(id, ResponseStatus::Accepted).unwrap();
        assert_eq!(store.event(id).unwrap().unwrap().show_as, ShowAs::Busy);
    }

    #[test]
    fn response_send_options_are_kept_for_sync() {
        let (store, _) = store_with_account();
        let cal = store.default_calendar(None).unwrap().unwrap();
        let now = Utc::now();
        let mut e = Event { calendar_id: cal.id, start: now, end: now + Duration::hours(1), ..Default::default() };
        let id = store.insert_event(&mut e).unwrap();
        store.respond_to_event_with(id, ResponseStatus::Declined, true, "  On holiday that week. ").unwrap();
        let e = store.event(id).unwrap().unwrap();
        assert_eq!((e.my_response, e.response_silent, e.response_note.as_str()), (ResponseStatus::Declined, false, "On holiday that week."));
        // Without email: no note travels.
        store.respond_to_event_with(id, ResponseStatus::Accepted, false, "ignored").unwrap();
        let e = store.event(id).unwrap().unwrap();
        assert_eq!((e.response_silent, e.response_note.as_str()), (true, ""));
        // The plain call sends a response again.
        store.respond_to_event(id, ResponseStatus::Tentative).unwrap();
        assert!(!store.event(id).unwrap().unwrap().response_silent);
    }

    #[test]
    fn editing_one_occurrence_creates_a_linked_exception() {
        let (store, _) = store_with_account();
        let cal = store.default_calendar(None).unwrap().unwrap();
        let series = standup(&store, cal.id);
        let tuesday = local_to_utc(day() + Duration::days(1), t(9, 0));
        let mut edited = series.clone();
        edited.title = "Standup (moved)".into();
        edited.start = local_to_utc(day() + Duration::days(1), t(11, 0));
        edited.end = local_to_utc(day() + Duration::days(1), t(11, 30));
        let ex_id = store.update_occurrence(series.id, tuesday, &edited).unwrap();
        assert_ne!(ex_id, series.id);

        let occ = week(&store);
        assert_eq!(occ.iter().filter(|o| o.event.title == "Standup").count(), 6);
        let moved: Vec<_> = occ.iter().filter(|o| o.event.title == "Standup (moved)").collect();
        assert_eq!(moved.len(), 1);
        assert_eq!(moved[0].event.series_id, Some(series.id));
        assert_eq!(moved[0].original_start, tuesday);
        assert!(moved[0].event.recurrence.is_none());
        assert_eq!(store.exceptions_of(series.id).unwrap().len(), 1);

        // Changing the pattern of the whole series drops the exception.
        let mut master = store.event(series.id).unwrap().unwrap();
        master.recurrence.as_mut().unwrap().interval = 2;
        store.update_series(&master, true).unwrap();
        assert!(store.exceptions_of(series.id).unwrap().is_empty());
        assert!(store.event(series.id).unwrap().unwrap().exdates.is_empty());
        assert_eq!(week(&store).len(), 4);

        // update_occurrence on a single event is a plain update.
        let mut single = Event { calendar_id: cal.id, start: tuesday, end: tuesday + Duration::hours(1), ..Default::default() };
        let id = store.insert_event(&mut single).unwrap();
        single.title = "Renamed".into();
        assert_eq!(store.update_occurrence(id, tuesday, &single).unwrap(), id);
        assert_eq!(store.event(id).unwrap().unwrap().title, "Renamed");
    }

    #[test]
    fn deleting_this_and_following_or_the_whole_series() {
        let (store, _) = store_with_account();
        let cal = store.default_calendar(None).unwrap().unwrap();
        let series = standup(&store, cal.id);
        let thursday = local_to_utc(day() + Duration::days(3), t(9, 0));
        let friday = local_to_utc(day() + Duration::days(4), t(9, 0));
        let mut moved = series.clone();
        moved.start = friday + Duration::hours(2);
        moved.end = moved.start + Duration::minutes(15);
        store.update_occurrence(series.id, friday, &moved).unwrap();

        store.delete_following(series.id, thursday).unwrap();
        let occ = week(&store);
        assert_eq!(occ.len(), 3, "Mon, Tue, Wed remain");
        assert!(store.exceptions_of(series.id).unwrap().is_empty());
        let rule = store.event(series.id).unwrap().unwrap().recurrence.unwrap();
        assert_eq!(rule.end, RecurrenceEnd::Until(day() + Duration::days(2)));

        // From the first occurrence it removes everything.
        store.delete_following(series.id, series.start).unwrap();
        assert!(store.event(series.id).unwrap().is_none());

        let again = standup(&store, cal.id);
        store.update_occurrence(again.id, friday, &moved).unwrap();
        store.delete_series(again.id).unwrap();
        assert!(week(&store).is_empty());
    }

    #[test]
    fn splitting_a_series_keeps_earlier_occurrences_and_the_count() {
        let (store, _) = store_with_account();
        let cal = store.default_calendar(None).unwrap().unwrap();
        let mut series = standup(&store, cal.id);
        series.recurrence.as_mut().unwrap().end = RecurrenceEnd::Count(10);
        store.update_event(&series).unwrap();
        let wednesday = local_to_utc(day() + Duration::days(2), t(9, 0));
        let mut later = series.clone();
        later.title = "Standup v2".into();
        later.start = local_to_utc(day() + Duration::days(2), t(9, 30));
        later.end = later.start + Duration::minutes(15);
        let new_id = store.split_series(series.id, wednesday, &later).unwrap();
        assert_ne!(new_id, series.id);
        let occ = week(&store);
        assert_eq!(occ.iter().filter(|o| o.event.title == "Standup").count(), 2);
        assert_eq!(occ.iter().filter(|o| o.event.title == "Standup v2").count(), 5);
        let new_rule = store.event(new_id).unwrap().unwrap().recurrence.unwrap();
        assert_eq!(new_rule.end, RecurrenceEnd::Count(8));

        // Splitting at the first occurrence edits the series in place.
        let same = store.split_series(new_id, later.start, &Event { title: "Standup v3".into(), ..later.clone() }).unwrap();
        assert_eq!(same, new_id);
        assert_eq!(store.event(new_id).unwrap().unwrap().title, "Standup v3");
    }

    #[test]
    fn reminders_fire_snooze_and_dismiss() {
        let (store, _) = store_with_account();
        let cal = store.default_calendar(None).unwrap().unwrap();
        let now = local_to_utc(day(), t(8, 50));
        let mut series = standup(&store, cal.id);
        series.reminder_minutes = Some(15);
        store.update_event(&series).unwrap();
        let mut later = Event {
            calendar_id: cal.id,
            title: "Review".into(),
            start: local_to_utc(day(), t(10, 0)),
            end: local_to_utc(day(), t(11, 0)),
            reminder_minutes: Some(30),
            ..Default::default()
        };
        store.insert_event(&mut later).unwrap();

        let due = store.due_reminders(now).unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].0.event.title, "Standup");
        assert_eq!(due[0].1, local_to_utc(day(), t(8, 45)));
        assert_eq!(store.next_reminder_at(now).unwrap(), Some(local_to_utc(day(), t(9, 30))));

        let occ_start = due[0].0.start;
        store.snooze_reminder(series.id, occ_start, now + Duration::minutes(5)).unwrap();
        assert!(store.due_reminders(now).unwrap().is_empty());
        assert_eq!(store.due_reminders(now + Duration::minutes(5)).unwrap().len(), 1);
        store.dismiss_reminder(series.id, occ_start).unwrap();
        assert!(store.due_reminders(now + Duration::minutes(6)).unwrap().is_empty());

        // Ended events no longer remind; the next day's standup does.
        let tomorrow = local_to_utc(day() + Duration::days(1), t(8, 50));
        let due = store.due_reminders(tomorrow).unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(local_date(due[0].0.start), day() + Duration::days(1));
    }

    #[test]
    fn calendar_management() {
        let (store, acct) = store_with_account();
        assert!(store.create_calendar(acct, "  ", "#0078d4", "My calendars").is_err());
        let id = store.create_calendar(acct, "Personal", "#107c10", "").unwrap();
        let cal = store.calendar(id).unwrap().unwrap();
        assert_eq!((cal.group.as_str(), cal.is_visible, cal.can_edit), ("My calendars", true, true));
        assert_eq!(cal.sort_order, 1);
        store.rename_calendar(id, "Family").unwrap();
        store.set_calendar_color(id, "#f7630c").unwrap();
        let cal = store.calendar(id).unwrap().unwrap();
        assert_eq!((cal.name.as_str(), cal.color.as_str()), ("Family", "#f7630c"));
        store.show_only_calendar(id).unwrap();
        let visible: Vec<_> = store.calendars().unwrap().into_iter().filter(|c| c.is_visible).map(|c| c.id).collect();
        assert_eq!(visible, vec![id]);
        store.modify_event(12345, |_| {}).unwrap_err();
    }

    #[test]
    fn calendar_groups_add_rename_move_delete() {
        let (store, acct) = store_with_account();
        let base = store.calendar_groups(acct).unwrap();
        assert_eq!(base[0], "My calendars");
        let id = store.create_calendar(acct, "Club", "#107c10", "Other calendars").unwrap();
        assert_eq!(store.calendar_groups(acct).unwrap().last().unwrap(), "Other calendars");

        // A new group exists before it holds a calendar; names are unique.
        store.add_calendar_group(acct, "  Family ").unwrap();
        assert!(store.add_calendar_group(acct, "family").is_err());
        assert!(store.add_calendar_group(acct, " ").is_err());
        let groups = store.calendar_groups(acct).unwrap();
        assert_eq!(&groups[groups.len() - 2..], ["Other calendars", "Family"]);

        // Move up / down never passes "My calendars".
        store.move_calendar_group(acct, "Family", -1).unwrap();
        let groups = store.calendar_groups(acct).unwrap();
        assert_eq!(&groups[groups.len() - 2..], ["Family", "Other calendars"]);
        store.move_calendar_group(acct, "My calendars", 1).unwrap();
        assert_eq!(store.calendar_groups(acct).unwrap()[0], "My calendars");

        // Renaming moves the group's calendars along.
        store.rename_calendar_group(acct, "Other calendars", "Teams").unwrap();
        assert_eq!(store.calendar(id).unwrap().unwrap().group, "Teams");
        assert!(store.rename_calendar_group(acct, "My calendars", "Mine").is_err());
        assert!(store.rename_calendar_group(acct, "Teams", "family").is_err());

        // Only empty groups can be deleted.
        assert!(store.delete_calendar_group(acct, "Teams").is_err());
        store.delete_calendar_group(acct, "Family").unwrap();
        assert!(!store.calendar_groups(acct).unwrap().contains(&"Family".to_string()));
        assert!(store.delete_calendar_group(acct, "My calendars").is_err());
    }

    #[test]
    fn search_matches_organizer() {
        let (store, _) = store_with_account();
        let cal = store.default_calendar(None).unwrap().unwrap();
        let now = Utc::now();
        let mut e = Event {
            calendar_id: cal.id,
            title: "Sync".into(),
            start: now,
            end: now + Duration::hours(1),
            organizer: Some(EmailAddress::new("Priya Raman", "priya@x.example")),
            ..Default::default()
        };
        store.insert_event(&mut e).unwrap();
        assert_eq!(store.search_events("priya").unwrap().len(), 1);
        assert_eq!(store.search_events("sync").unwrap().len(), 1);
        assert!(store.search_events("nothing").unwrap().is_empty());
    }
}
