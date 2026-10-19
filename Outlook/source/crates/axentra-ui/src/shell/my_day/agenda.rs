//! My Day > Calendar: date navigation, agenda or day view, event details,
//! Join for online meetings (people-todo.md §4.3).

use std::collections::HashSet;

use axentra_core::*;
use chrono::{DateTime, Duration, Local, NaiveDate, NaiveTime, Timelike, Utc};
use dioxus::prelude::*;

use super::day_view::DayView;
use super::mini_month::MiniMonth;
use crate::kit::{Button, EmptyState, Icon, IconButton, Menu, MenuItem, Variant};
use crate::shell::use_shell;
use crate::state::{NavRequest, store, use_app};

const VIEW_KEY: &str = "shell.my_day_view";
const HIDDEN_KEY: &str = "shell.my_day_hidden_calendars";

/// "Today", "Tomorrow" or "Tue, Oct 20".
pub fn day_label(d: NaiveDate, today: NaiveDate) -> String {
    if d == today {
        "Today".into()
    } else if d == today + Duration::days(1) {
        "Tomorrow".into()
    } else if d == today - Duration::days(1) {
        "Yesterday".into()
    } else {
        crate::fmt::short_date(d)
    }
}

/// An online meeting can be joined from 10 minutes before it starts until
/// it ends (people-todo.md §4.3).
pub fn joinable(o: &Occurrence, now: DateTime<Utc>) -> bool {
    o.event.is_online_meeting
        && o.event.online_meeting_url.as_deref().is_some_and(|u| !u.is_empty())
        && now >= o.start - Duration::minutes(10)
        && now < o.end
}

/// "9:00 AM – 9:30 AM", "All day".
pub fn time_range(o: &Occurrence, h24: bool) -> String {
    if o.event.all_day {
        return "All day".into();
    }
    format!("{} – {}", crate::fmt::time(o.start, h24), crate::fmt::time(o.end, h24))
}

/// Events from `day` for `days` days, grouped by local start date, all-day
/// events first; on today, events that already ended are hidden.
pub fn agenda(occurrences: Vec<Occurrence>, day: NaiveDate, days: i64, now: DateTime<Local>) -> Vec<(NaiveDate, Vec<Occurrence>)> {
    let mut groups: Vec<(NaiveDate, Vec<Occurrence>)> = (0..days).map(|i| (day + Duration::days(i), Vec::new())).collect();
    let today = now.date_naive();
    for o in occurrences {
        if o.event.my_response == ResponseStatus::Declined || o.event.is_cancelled {
            continue;
        }
        let start_day = local_date(o.start).max(day);
        // Today's events that already ended are hidden (December 2024 behaviour).
        if start_day == today && o.end <= now.with_timezone(&Utc) {
            continue;
        }
        if let Some((_, list)) = groups.iter_mut().find(|(d, _)| *d == start_day) {
            list.push(o);
        }
    }
    for (_, list) in &mut groups {
        list.sort_by_key(|o| (!o.event.all_day, o.start));
    }
    groups.retain(|(_, l)| !l.is_empty());
    groups
}

fn load_hidden() -> HashSet<CalendarId> {
    store().kv_get(HIDDEN_KEY).ok().flatten().and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default()
}

/// Start of the next half hour on `day` (today) or 9:00 (other days).
fn next_slot(day: NaiveDate, now: DateTime<Local>) -> DateTime<Utc> {
    if day == now.date_naive() {
        let mins = now.hour() * 60 + now.minute();
        let next = (mins / 30 + 1) * 30;
        if next < 24 * 60 {
            return local_to_utc(day, NaiveTime::from_hms_opt(next / 60, next % 60, 0).unwrap_or(NaiveTime::MIN));
        }
    }
    local_to_utc(day, NaiveTime::from_hms_opt(9, 0, 0).unwrap_or(NaiveTime::MIN))
}

#[component]
pub fn CalendarTab() -> Element {
    let app = use_app();
    let shell = use_shell();
    let now = (shell.now)();
    let today = now.date_naive();
    let mut day = use_signal(|| today);
    let mut view = use_signal(|| store().kv_get(VIEW_KEY).ok().flatten().unwrap_or_else(|| "agenda".into()));
    let mut hidden = use_signal(load_hidden);
    let mut details = use_signal(|| None::<Occurrence>);
    let mut picker = use_signal(|| false);
    let mut menu_at = use_signal(|| None::<(f64, f64)>);
    let _ = app.calendar_rev();
    let (h24, week_start, event_minutes) = {
        let s = app.settings.read();
        (s.use_24h_time, s.week_start, s.default_event_minutes.max(15) as i64)
    };
    let calendars = store().calendars().unwrap_or_default();

    if let Some(o) = details() {
        return rsx! { EventDetails { occurrence: o, h24, on_back: move |_| details.set(None) } };
    }

    let d = day();
    let is_day = view() == "day";
    let days = if is_day { 1 } else { 7 };
    let from = local_to_utc(d, NaiveTime::MIN);
    let to = local_to_utc(d + Duration::days(days), NaiveTime::MIN);
    let hidden_set = hidden();
    let occ: Vec<Occurrence> =
        store().events_in_range(from, to).unwrap_or_default().into_iter().filter(|o| !hidden_set.contains(&o.event.calendar_id)).collect();
    let groups = agenda(occ.clone(), d, days, now);

    let mut menu_items = vec![
        MenuItem::header("View"),
        MenuItem::new("view:agenda", "Agenda").radio(!is_day),
        MenuItem::new("view:day", "Day").radio(is_day),
        MenuItem::divider(),
        MenuItem::header("Calendars"),
    ];
    for c in &calendars {
        menu_items.push(MenuItem::new(format!("cal:{}", c.id), c.name.clone()).checked(!hidden_set.contains(&c.id)).swatch(c.color.clone()));
    }

    rsx! {
        div { class: "ax-myday-toolbar",
            Button {
                label: "New event",
                icon: "calendar_add",
                variant: Variant::Subtle,
                onclick: move |_| {
                    let start = next_slot(day(), (shell.now)());
                    app.navigate(NavRequest::NewEvent { start, end: start + Duration::minutes(event_minutes), all_day: false });
                },
            }
            span { class: "ax-grow" }
            IconButton {
                icon: "more_horizontal",
                title: "Switch views",
                onclick: move |e: MouseEvent| {
                    let c = e.client_coordinates();
                    menu_at.set(Some((c.x, c.y + 12.0)));
                },
            }
        }
        if let Some((x, y)) = menu_at() {
            Menu {
                items: menu_items,
                x,
                y,
                anchor_end: true,
                label: "Switch views",
                on_close: move |_| menu_at.set(None),
                on_select: move |id: String| {
                    if let Some(v) = id.strip_prefix("view:") {
                        let _ = store().kv_set(VIEW_KEY, v);
                        view.set(v.to_string());
                    } else if let Some(cal) = id.strip_prefix("cal:").and_then(|c| c.parse::<CalendarId>().ok()) {
                        let mut set = hidden();
                        if !set.remove(&cal) {
                            set.insert(cal);
                        }
                        if let Ok(json) = serde_json::to_string(&set) {
                            let _ = store().kv_set(HIDDEN_KEY, &json);
                        }
                        hidden.set(set);
                    }
                },
            }
        }
        div { class: "ax-myday-datebar",
            IconButton { icon: "chevron_left", title: "Previous day", small: true, onclick: move |_| day.set(day() - Duration::days(1)) }
            button {
                r#type: "button",
                class: "ax-myday-date",
                "aria-haspopup": "dialog",
                "aria-expanded": if picker() { "true" } else { "false" },
                title: crate::fmt::long_date(d),
                onclick: move |_| picker.set(!picker()),
                span { "{day_label(d, today)}" }
                Icon { name: "chevron_down", size: 12 }
            }
            IconButton { icon: "chevron_right", title: "Next day", small: true, onclick: move |_| day.set(day() + Duration::days(1)) }
            if d != today {
                Button { label: "Today", small: true, variant: Variant::Subtle, onclick: move |_| day.set(today) }
            }
        }
        if picker() {
            MiniMonth {
                selected: d,
                today,
                week_start,
                on_pick: move |p: NaiveDate| {
                    day.set(p);
                    picker.set(false);
                },
                on_close: move |_| picker.set(false),
            }
        }
        div { class: "ax-sidepanel-body",
            if is_day {
                DayView { day: d, occurrences: occ, now, h24, on_open: move |o| details.set(Some(o)) }
            } else if groups.is_empty() {
                EmptyState {
                    icon: "calendar_empty",
                    title: if d == today { "Nothing scheduled for today" } else { "Nothing scheduled" },
                    subtitle: "Enjoy the free time, or add an event.",
                }
            } else {
                div { class: "ax-agenda", role: "list",
                    for (gd, list) in groups {
                        div { key: "{gd}", class: "ax-agenda-day", role: "presentation", "{day_label(gd, today)}" }
                        for o in list {
                            AgendaRow { key: "{o.event.id}-{o.start.timestamp()}", occurrence: o.clone(), h24, now: now.with_timezone(&Utc), on_open: move |o| details.set(Some(o)) }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn AgendaRow(occurrence: Occurrence, h24: bool, now: DateTime<Utc>, on_open: EventHandler<Occurrence>) -> Element {
    let o = occurrence.clone();
    let join = joinable(&o, now).then(|| o.event.online_meeting_url.clone().unwrap_or_default());
    let label = format!("{}, {}{}", o.event.title, time_range(&o, h24), if o.event.location.is_empty() { String::new() } else { format!(", {}", o.event.location) });
    rsx! {
        div { class: "ax-agenda-row", role: "listitem",
            button {
                r#type: "button",
                class: "ax-agenda-event",
                "aria-label": "{label}",
                onclick: {
                    let o = o.clone();
                    move |_| on_open.call(o.clone())
                },
                span { class: "ax-agenda-bar", style: "background:{o.color}" }
                span { class: "ax-agenda-text",
                    span { class: "ax-agenda-time", "{time_range(&o, h24)}" }
                    span { class: "ax-agenda-title ax-ellipsis", "{o.event.title}" }
                    if !o.event.location.is_empty() {
                        span { class: "ax-agenda-where ax-ellipsis", "{o.event.location}" }
                    }
                }
                if o.event.is_online_meeting && join.is_none() {
                    Icon { name: "video", size: 16, class: "ax-agenda-online" }
                }
            }
            if let Some(url) = join {
                a { class: "ax-btn ax-primary ax-small ax-join", href: "{url}", target: "_blank", rel: "noopener noreferrer", "Join" }
            }
        }
    }
}

#[component]
fn EventDetails(occurrence: Occurrence, h24: bool, on_back: EventHandler<()>) -> Element {
    let app = use_app();
    let shell = use_shell();
    let o = occurrence;
    let now = (shell.now)().with_timezone(&Utc);
    let date = crate::fmt::long_date(local_date(o.start));
    let body = axentra_core::html::html_to_text(&o.event.body_html);
    let response = match o.event.my_response {
        ResponseStatus::Accepted => "Accepted",
        ResponseStatus::Tentative => "Tentatively accepted",
        ResponseStatus::Declined => "Declined",
        ResponseStatus::Organizer => "You're the organizer",
        ResponseStatus::NotResponded => "Not responded",
        ResponseStatus::None => "",
    };
    let id = o.event.id;
    rsx! {
        div { class: "ax-myday-toolbar",
            Button { label: "Back", icon: "arrow_left", variant: Variant::Subtle, autofocus: true, onclick: move |_| on_back.call(()) }
            span { class: "ax-grow" }
            Button { label: "Open in Calendar", icon: "open", variant: Variant::Subtle, onclick: move |_| app.navigate(NavRequest::Event(id)) }
        }
        div { class: "ax-sidepanel-body ax-event-details",
            div { class: "ax-row", style: "gap:8px;align-items:flex-start",
                span { class: "ax-agenda-bar", style: "background:{o.color};height:28px" }
                h3 { class: "ax-subtitle-1", "{o.event.title}" }
            }
            div { class: "ax-detail-line", Icon { name: "clock", size: 16 } span { "{date}, {time_range(&o, h24)}" } }
            if !o.event.location.is_empty() {
                div { class: "ax-detail-line", Icon { name: "location", size: 16 } span { "{o.event.location}" } }
            }
            if let Some(org) = &o.event.organizer {
                div { class: "ax-detail-line", Icon { name: "person", size: 16 } span { "Organizer: {org.display()}" } }
            }
            if !o.event.attendees.is_empty() {
                div { class: "ax-detail-line", Icon { name: "people", size: 16 } span { "{o.event.attendees.len()} attendees" } }
            }
            if !response.is_empty() {
                div { class: "ax-detail-line", Icon { name: "checkmark_circle", size: 16 } span { "{response}" } }
            }
            if let Some(url) = o.event.online_meeting_url.clone().filter(|u| !u.is_empty()) {
                div { class: "ax-detail-line",
                    Icon { name: "video", size: 16 }
                    if joinable(&o, now) {
                        a { class: "ax-btn ax-primary ax-small", href: "{url}", target: "_blank", rel: "noopener noreferrer", "Join" }
                    } else {
                        a { href: "{url}", target: "_blank", rel: "noopener noreferrer", "Online meeting" }
                    }
                }
            }
            if !body.trim().is_empty() {
                p { class: "ax-event-body ax-selectable", "{body}" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn occ(title: &str, start: DateTime<Local>, mins: i64, all_day: bool) -> Occurrence {
        let s = start.with_timezone(&Utc);
        let e = s + Duration::minutes(mins);
        Occurrence {
            event: axentra_core::Event { title: title.into(), start: s, end: e, all_day, ..Default::default() },
            start: s,
            end: e,
            original_start: s,
            color: "#0f6cbd".into(),
        }
    }

    #[test]
    fn agenda_hides_past_events_today_and_groups_by_day() {
        let now = Local.with_ymd_and_hms(2026, 10, 19, 11, 0, 0).unwrap();
        let today = now.date_naive();
        let events = vec![
            occ("Standup", Local.with_ymd_and_hms(2026, 10, 19, 9, 0, 0).unwrap(), 15, false),
            occ("Review", Local.with_ymd_and_hms(2026, 10, 19, 14, 0, 0).unwrap(), 60, false),
            occ("Holiday", Local.with_ymd_and_hms(2026, 10, 19, 0, 0, 0).unwrap(), 24 * 60, true),
            occ("Lunch", Local.with_ymd_and_hms(2026, 10, 20, 12, 0, 0).unwrap(), 60, false),
        ];
        let groups = agenda(events, today, 7, now);
        assert_eq!(groups.len(), 2);
        let titles: Vec<&str> = groups[0].1.iter().map(|o| o.event.title.as_str()).collect();
        assert_eq!(titles, vec!["Holiday", "Review"], "past standup hidden, all-day first");
        assert_eq!(groups[1].0, today + Duration::days(1));
        assert_eq!(day_label(today, today), "Today");
        assert_eq!(day_label(today + Duration::days(1), today), "Tomorrow");
    }

    #[test]
    fn join_window_opens_ten_minutes_early() {
        let start = Local.with_ymd_and_hms(2026, 10, 19, 9, 0, 0).unwrap();
        let mut o = occ("Standup", start, 15, false);
        o.event.is_online_meeting = true;
        o.event.online_meeting_url = Some("https://meet.example/x".into());
        let at = |h: u32, m: u32| Local.with_ymd_and_hms(2026, 10, 19, h, m, 0).unwrap().with_timezone(&Utc);
        assert!(!joinable(&o, at(8, 49)));
        assert!(joinable(&o, at(8, 50)));
        assert!(joinable(&o, at(9, 14)));
        assert!(!joinable(&o, at(9, 15)));
    }

    #[test]
    fn new_event_starts_at_the_next_half_hour() {
        let now = Local.with_ymd_and_hms(2026, 10, 19, 10, 40, 0).unwrap();
        let s = next_slot(now.date_naive(), now).with_timezone(&Local);
        assert_eq!((s.hour(), s.minute()), (11, 0));
        let other = next_slot(now.date_naive() + Duration::days(1), now).with_timezone(&Local);
        assert_eq!((other.hour(), other.minute()), (9, 0));
    }
}
