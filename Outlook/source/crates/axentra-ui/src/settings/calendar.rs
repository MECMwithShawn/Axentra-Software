//! Settings > Calendar (docs/spec/shell.md §7.2; calendar.md owns the
//! calendar module itself).

use axentra_core::*;
use chrono::Weekday;
use dioxus::prelude::*;

use super::controls::{Row, Section, Unavailable, use_settings};
use super::pages::PageId;
use crate::kit::{Checkbox, Select};

const WEEKDAYS: [(Weekday, &str); 7] = [
    (Weekday::Sun, "Sunday"),
    (Weekday::Mon, "Monday"),
    (Weekday::Tue, "Tuesday"),
    (Weekday::Wed, "Wednesday"),
    (Weekday::Thu, "Thursday"),
    (Weekday::Fri, "Friday"),
    (Weekday::Sat, "Saturday"),
];

fn hour_options(h24: bool) -> Vec<(String, String)> {
    (0..24u32).map(|h| (h.to_string(), crate::fmt::hour_label(h, h24))).collect()
}

/// Work days in week order starting at `week_start`.
pub fn ordered_days(week_start: Weekday) -> Vec<(Weekday, &'static str)> {
    let start = WEEKDAYS.iter().position(|(d, _)| *d == week_start).unwrap_or(0);
    (0..7).map(|i| WEEKDAYS[(start + i) % 7]).collect()
}

#[component]
pub fn CalendarViewPage() -> Element {
    let ctx = use_settings();
    let s = ctx.get();
    let days = ordered_days(s.week_start);
    let invalid_hours = s.work_end_hour <= s.work_start_hour;
    rsx! {
        Section { title: "Calendar appearance",
            Row { label: "First day of the week",
                Select {
                    value: format!("{:?}", s.week_start),
                    options: WEEKDAYS.iter().map(|(d, l)| (format!("{d:?}"), l.to_string())).collect::<Vec<_>>(),
                    aria_label: "First day of the week",
                    onchange: move |v: String| {
                        if let Some((d, _)) = WEEKDAYS.iter().find(|(d, _)| format!("{d:?}") == v) {
                            ctx.edit(|s| s.week_start = *d);
                        }
                    },
                }
            }
            Row { label: "Default view",
                Select {
                    value: format!("{:?}", s.calendar_view),
                    options: vec![
                        ("Day".into(), "Day".into()),
                        ("WorkWeek".into(), "Work week".into()),
                        ("Week".into(), "Week".into()),
                        ("Month".into(), "Month".into()),
                    ],
                    aria_label: "Default view",
                    onchange: move |v: String| ctx.edit(|s| s.calendar_view = match v.as_str() {
                        "Day" => CalendarView::Day,
                        "Week" => CalendarView::Week,
                        "Month" => CalendarView::Month,
                        _ => CalendarView::WorkWeek,
                    }),
                }
            }
            Checkbox { label: "Show week numbers", checked: s.show_week_numbers, onchange: move |v| ctx.edit(|s| s.show_week_numbers = v) }
        }
        Section { title: "Work hours and location", description: "Your work days and hours shade the calendar and guide meeting times.",
            div { class: "ax-workdays", role: "group", "aria-label": "Work days",
                for (d, label) in days {
                    Checkbox {
                        key: "{label}",
                        label: label[..3].to_string(),
                        aria_label: label.to_string(),
                        checked: s.work_days.contains(&d),
                        onchange: move |on: bool| ctx.edit(|s| {
                            s.work_days.retain(|x| *x != d);
                            if on {
                                s.work_days.push(d);
                            }
                        }),
                    }
                }
            }
            div { class: "ax-row", style: "gap:12px;flex-wrap:wrap",
                Select {
                    label: "Start time",
                    value: s.work_start_hour.to_string(),
                    options: hour_options(s.use_24h_time),
                    onchange: move |v: String| ctx.edit(|s| s.work_start_hour = v.parse().unwrap_or(8)),
                }
                Select {
                    label: "End time",
                    value: s.work_end_hour.to_string(),
                    options: hour_options(s.use_24h_time),
                    onchange: move |v: String| ctx.edit(|s| s.work_end_hour = v.parse().unwrap_or(17)),
                }
            }
            if invalid_hours {
                p { class: "ax-set-error", role: "alert", "The end time must be later than the start time." }
            }
        }
        Section { title: "Time zones",
            Row { label: "Time zone", description: "Events show in your system's time zone.",
                span { class: "ax-set-value", "{super::general::local_time_zone().unwrap_or_else(|| \"System time zone\".into())}" }
            }
        }
    }
}

#[component]
pub fn EventsInvitationsPage() -> Element {
    let ctx = use_settings();
    let s = ctx.get();
    let reminders: Vec<(String, String)> = [-1i32, 0, 5, 10, 15, 30, 60, 120, 1440, 10080]
        .iter()
        .map(|m| {
            let label = match m {
                -1 => "None".to_string(),
                0 => "At time of event".to_string(),
                60 => "1 hour before".to_string(),
                120 => "2 hours before".to_string(),
                1440 => "1 day before".to_string(),
                10080 => "1 week before".to_string(),
                m => format!("{m} minutes before"),
            };
            (m.to_string(), label)
        })
        .collect();
    rsx! {
        Section { title: "Events you create",
            Row { label: "Default reminder",
                Select {
                    value: s.default_reminder_minutes.to_string(),
                    options: reminders,
                    aria_label: "Default reminder",
                    onchange: move |v: String| ctx.edit(|s| s.default_reminder_minutes = v.parse().unwrap_or(15)),
                }
            }
            Row { label: "Default event length",
                Select {
                    value: s.default_event_minutes.to_string(),
                    options: [15i32, 30, 45, 60, 90, 120].iter().map(|m| (m.to_string(), crate::fmt::duration_label(chrono::Duration::minutes(*m as i64)))).collect::<Vec<_>>(),
                    aria_label: "Default event length",
                    onchange: move |v: String| ctx.edit(|s| s.default_event_minutes = v.parse().unwrap_or(30)),
                }
            }
        }
    }
}

/// Calendar pages whose features live in the calendar module.
#[component]
pub fn CalendarModulePage(page: PageId) -> Element {
    let (icon, title, text) = match page {
        PageId::Weather => ("weather_sunny", "Weather isn't available", "Forecasts above the calendar need a weather service, which isn't set up."),
        PageId::SharedCalendars => ("calendar_person", "No shared calendars", "Calendars you share or that others share with you appear here."),
        PageId::CalendarActions => ("options", "Event actions", "Actions shown on events follow the Calendar ribbon commands."),
        PageId::EventsFromEmail => ("mail", "Events from email", "Reservations and deliveries found in your mail aren't added to the calendar."),
        _ => ("calendar_ltr", "Nothing to set", ""),
    };
    rsx! { Unavailable { icon, title, text } }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_days_follow_the_week_start() {
        let d: Vec<Weekday> = ordered_days(Weekday::Mon).into_iter().map(|(d, _)| d).collect();
        assert_eq!(d[0], Weekday::Mon);
        assert_eq!(d[6], Weekday::Sun);
        assert_eq!(ordered_days(Weekday::Sun)[0].1, "Sunday");
    }
}
