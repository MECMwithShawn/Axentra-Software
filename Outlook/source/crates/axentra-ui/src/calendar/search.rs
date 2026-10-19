//! Calendar search (header search box while the Calendar is open): matching
//! events, upcoming first, then past ones newest first. Picking one shows it
//! on the calendar.

use axentra_core::recurrence::occurrence_dates;
use axentra_core::*;
use axentra_core::Event;
use chrono::{Duration, Local, NaiveDate, Utc};
use dioxus::prelude::*;

use super::colors::ramp;
use super::event_block::display_title;
use super::state::*;
use crate::kit::{EmptyState, Icon, IconButton};
use crate::state::{AppState, SearchState, store, use_app};

/// One search hit: the event and the occurrence to show for it (the next
/// one for a series, if any).
#[derive(Clone, Debug, PartialEq)]
struct Hit {
    event: Event,
    start: chrono::DateTime<Utc>,
    end: chrono::DateTime<Utc>,
    color: String,
}

fn next_start(e: &Event, today: NaiveDate, week_start: chrono::Weekday) -> Option<chrono::DateTime<Utc>> {
    let rule = e.recurrence.as_ref()?;
    let l = e.start.with_timezone(&Local);
    occurrence_dates(rule, l.date_naive(), week_start, &e.exdates, today, today + Duration::days(366))
        .first()
        .map(|d| local_to_utc(*d, l.time()))
}

fn exit_search(mut app: AppState) {
    let serial = app.search.peek().serial + 1;
    app.search.set(SearchState { serial, ..Default::default() });
}

#[component]
pub fn SearchResults() -> Element {
    let app = use_app();
    let cal = use_cal();
    let _ = app.calendar_rev();
    let query = app.search.read().query.trim().to_string();
    let s = app.settings.read().clone();
    let h24 = s.use_24h_time;
    let today = (cal.today)();
    let colors: Vec<(CalendarId, String)> = store().calendars().unwrap_or_default().into_iter().map(|c| (c.id, c.color)).collect();
    let mut hits: Vec<Hit> = store()
        .search_events(&query)
        .unwrap_or_default()
        .into_iter()
        .map(|e| {
            let len = e.end - e.start;
            let start = next_start(&e, today, s.week_start).unwrap_or(e.start);
            let color = colors.iter().find(|(id, _)| *id == e.calendar_id).map(|(_, c)| c.clone()).unwrap_or_default();
            Hit { start, end: start + len, color, event: e }
        })
        .collect();
    let now = Utc::now();
    let (mut upcoming, mut past): (Vec<Hit>, Vec<Hit>) = hits.drain(..).partition(|h| h.end >= now);
    upcoming.sort_by_key(|h| h.start);
    past.sort_by_key(|h| std::cmp::Reverse(h.start));
    let total = upcoming.len() + past.len();
    rsx! {
        div { class: "ax-cal-search", role: "region", "aria-label": "Search results",
            div { class: "ax-cal-search-head",
                span { class: "ax-cal-search-title", {format!("Results for \u{201c}{query}\u{201d}")} }
                span { class: "ax-muted", role: "status", {if total == 1 { "1 result".to_string() } else { format!("{total} results") }} }
                span { class: "ax-grow" }
                IconButton { icon: "dismiss", title: "Exit search", onclick: move |_| exit_search(app) }
            }
            if total == 0 {
                EmptyState { icon: "search", title: "We didn't find anything", subtitle: "Check the spelling or try different words." }
            }
            div { class: "ax-cal-search-list", role: "list",
                if !upcoming.is_empty() { div { class: "ax-cal-search-group", "Upcoming" } }
                for h in upcoming {
                    SearchRow { key: "u{h.event.id}", hit: h, h24 }
                }
                if !past.is_empty() { div { class: "ax-cal-search-group", "Past" } }
                for h in past {
                    SearchRow { key: "p{h.event.id}", hit: h, h24 }
                }
            }
        }
    }
}

#[component]
fn SearchRow(hit: Hit, h24: bool) -> Element {
    let app = use_app();
    let mut cal = use_cal();
    let e = hit.event.clone();
    let date = local_date(hit.start);
    let when = if e.all_day { "All day".to_string() } else { super::text::time_range(hit.start, hit.end, h24) };
    let color = ramp(&hit.color).primary;
    let occ = Occurrence { start: hit.start, end: hit.end, original_start: hit.start, color: hit.color.clone(), event: e.clone() };
    let occ2 = occ.clone();
    rsx! {
        button {
            r#type: "button",
            class: "ax-cal-search-row",
            role: "listitem",
            onclick: move |_| {
                exit_search(app);
                let s = app.settings.peek().clone();
                cal.go_to(&s, date);
                cal.selected.set(Some(OccKey::of(&occ)));
                super::actions::open_peek(cal, occ.clone(), Anchor::point(-1.0, -1.0));
            },
            ondoubleclick: move |_| {
                exit_search(app);
                cal.open_occurrence(&occ2);
            },
            span { class: "ax-cal-search-bar", style: "background:{color}" }
            div { class: "ax-cal-search-date",
                span { class: "ax-cal-search-day", {date.format("%-d").to_string()} }
                span { class: "ax-muted ax-small-text", {date.format("%a, %b %Y").to_string()} }
            }
            div { class: "ax-col ax-grow", style: "min-width:0",
                span { class: "ax-semibold ax-ellipsis", {display_title(&e)} }
                span { class: "ax-muted ax-small-text", "{when}" }
                if !e.location.is_empty() { span { class: "ax-muted ax-small-text ax-ellipsis", "{e.location}" } }
            }
            if e.recurrence.is_some() { Icon { name: "arrow_repeat_all", size: 16, class: "ax-muted" } }
            if let Some(o) = &e.organizer { span { class: "ax-muted ax-small-text ax-ellipsis ax-cal-search-org", {o.display().to_string()} } }
        }
    }
}
