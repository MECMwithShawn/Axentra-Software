//! "Add calendar" dialog: a vertical nav on the left, the chosen page on the
//! right. "Create blank calendar" and "Holidays" work locally; the pages
//! that need a server account say so and keep their buttons disabled.

use axentra_core::recurrence::nth_weekday_of_month;
use axentra_core::*;
use axentra_core::Event;
use chrono::{Datelike, Duration, NaiveDate, NaiveTime, Weekday};
use dioxus::prelude::*;

use super::colors::PALETTE;
use super::state::*;
use crate::kit::{Button, Dialog, Icon, Variant, keys};
use crate::state::{AppState, store, use_app};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    Personal,
    Blank,
    Directory,
    Web,
    File,
    Holidays,
}

const PAGES: [(Page, &str, &str); 6] = [
    (Page::Personal, "Add personal calendars", "person"),
    (Page::Blank, "Create blank calendar", "calendar_add"),
    (Page::Directory, "Add from directory", "book_contacts"),
    (Page::Web, "Subscribe from web", "globe"),
    (Page::File, "Upload from file", "arrow_import"),
    (Page::Holidays, "Holidays", "calendar_ltr"),
];

#[component]
pub fn AddCalendarDialog() -> Element {
    let mut cal = use_cal();
    let mut page = use_signal(|| Page::Blank);
    rsx! {
        Dialog { title: "Add calendar", width: 800, resize_key: "add-calendar", min_width: 600, min_height: 400, class: "ax-cal-addcal", on_close: move |_| cal.dialog.set(None),
            div { class: "ax-cal-addcal-body",
                nav { class: "ax-cal-addcal-nav", role: "tablist", "aria-orientation": "vertical",
                    for (p, label, icon) in PAGES {
                        button {
                            key: "{label}",
                            r#type: "button",
                            role: "tab",
                            class: "ax-cal-addcal-tab",
                            class: if page() == p { "ax-selected" },
                            "aria-selected": if page() == p { "true" } else { "false" },
                            onclick: move |_| page.set(p),
                            Icon { name: icon }
                            span { "{label}" }
                        }
                    }
                }
                div { class: "ax-cal-addcal-page", role: "tabpanel",
                    match page() {
                        Page::Blank => rsx! { BlankCalendarPage {} },
                        Page::Holidays => rsx! { HolidaysPage {} },
                        Page::Personal => rsx! {
                            Unavailable {
                                title: "Add personal calendars",
                                text: "Connect a personal Outlook.com or Google calendar so its events count toward your availability. This is available for work or school accounts.",
                                action: "Add a personal calendar",
                                reason: "Personal calendars can be added to work or school accounts only.",
                            }
                        },
                        Page::Directory => rsx! {
                            Unavailable {
                                title: "Add from directory",
                                text: "Search your organization's directory for a person, group or room and add their calendar.",
                                action: "Add",
                                reason: "Directory search needs a work or school account.",
                                field: "Search by name or email",
                            }
                        },
                        Page::Web => rsx! {
                            Unavailable {
                                title: "Subscribe from web",
                                text: "Subscribe to an internet calendar (.ics). Subscribed calendars are read-only. Updates may take more than 24 hours.",
                                action: "Import",
                                reason: "Subscribing to web calendars isn't available for local accounts yet.",
                                field: "Link to the calendar",
                            }
                        },
                        Page::File => rsx! {
                            Unavailable {
                                title: "Upload from file",
                                text: "Import events from an .ics file into one of your calendars. This is a one-time copy that doesn't refresh.",
                                action: "Browse",
                                reason: "Importing .ics files isn't available yet.",
                            }
                        },
                    }
                }
            }
        }
    }
}

#[component]
fn Unavailable(title: &'static str, text: &'static str, action: &'static str, reason: &'static str, #[props(default)] field: &'static str) -> Element {
    rsx! {
        div { class: "ax-col", style: "gap:12px",
            h3 { class: "ax-cal-addcal-title", "{title}" }
            p { class: "ax-muted", "{text}" }
            if !field.is_empty() {
                input { class: "ax-input", placeholder: "{field}", "aria-label": "{field}", disabled: true, title: "{reason}" }
            }
            div { Button { label: action, variant: Variant::Primary, disabled: true, title: reason } }
            div { class: "ax-message-bar", Icon { name: "info", size: 16 } span { "{reason}" } }
        }
    }
}

#[component]
fn BlankCalendarPage() -> Element {
    let app = use_app();
    let mut cal = use_cal();
    let mut name = use_signal(String::new);
    let mut color = use_signal(|| PALETTE[2].primary.to_string());
    let mut group = use_signal(|| "My calendars".to_string());
    let mut error = use_signal(|| None::<String>);
    // The account's groups in folder-pane order (empty ones included).
    let account_id = store().primary_account().ok().flatten().map(|a| a.id).unwrap_or_default();
    let mut groups = store().calendar_groups(account_id).unwrap_or_else(|_| vec!["My calendars".into()]);
    if !groups.iter().any(|g| g == "Other calendars") {
        groups.push("Other calendars".into());
    }
    let mut save = move || {
        let n = name.peek().trim().to_string();
        if n.is_empty() {
            error.set(Some("Give the calendar a name.".into()));
            return;
        }
        let Some(account) = store().primary_account().ok().flatten() else {
            app.error("Add an account before creating calendars.");
            return;
        };
        if let Some(id) = app.check(store().create_calendar(account.id, &n, &color.peek(), &group.peek())) {
            cal.active_calendar.set(Some(id));
            cal.dialog.set(None);
            app.bump_calendar();
            app.toast(format!("Created \u{201c}{n}\u{201d}"));
        }
    };
    rsx! {
        div { class: "ax-col", style: "gap:16px",
            h3 { class: "ax-cal-addcal-title", "Create blank calendar" }
            div { class: "ax-field",
                label { class: "ax-field-label", "Calendar name" }
                input {
                    class: "ax-input",
                    placeholder: "Calendar name",
                    "aria-label": "Calendar name",
                    "aria-required": "true",
                    value: "{name}",
                    onmounted: move |e| async move { let _ = e.set_focus(true).await; },
                    oninput: move |e| { name.set(e.value()); error.set(None) },
                    onkeydown: move |e: KeyboardEvent| {
                        keys::stop_typing_keys(&e);
                        if e.key() == Key::Enter { save() }
                    },
                }
                if let Some(err) = error() { span { class: "ax-cal-field-error", role: "alert", "{err}" } }
            }
            div { class: "ax-field",
                span { class: "ax-field-label", "Color" }
                div { class: "ax-cal-swatches", role: "radiogroup", "aria-label": "Color",
                    for s in PALETTE.iter() {
                        button {
                            key: "{s.primary}",
                            r#type: "button",
                            class: "ax-cal-swatch",
                            class: if color() == s.primary { "ax-checked" },
                            role: "radio",
                            "aria-checked": if color() == s.primary { "true" } else { "false" },
                            title: "{s.name}",
                            "aria-label": "{s.name}",
                            style: "background:{s.primary}",
                            onclick: move |_| color.set(s.primary.to_string()),
                            if color() == s.primary { Icon { name: "checkmark", size: 16 } }
                        }
                    }
                }
            }
            crate::kit::Select {
                label: "Add to",
                value: group(),
                options: groups.iter().map(|g| (g.clone(), g.clone())).collect::<Vec<_>>(),
                onchange: move |v: String| group.set(v),
            }
            div { class: "ax-row", style: "gap:8px",
                Button { label: "Save", variant: Variant::Primary, onclick: move |_| save() }
            }
        }
    }
}

/// Countries offered on the Holidays page.
const COUNTRIES: [&str; 3] = ["United States", "Canada", "United Kingdom"];

/// Easter Sunday (anonymous Gregorian algorithm).
pub fn easter(year: i32) -> NaiveDate {
    let a = year % 19;
    let b = year / 100;
    let c = year % 100;
    let d = b / 4;
    let e = b % 4;
    let f = (b + 8) / 25;
    let g = (b - f + 1) / 3;
    let h = (19 * a + b - d - g + 15) % 30;
    let i = c / 4;
    let k = c % 4;
    let l = (32 + 2 * e + 2 * i - h - k) % 7;
    let m = (a + 11 * h + 22 * l) / 451;
    let month = (h + l - 7 * m + 114) / 31;
    let day = (h + l - 7 * m + 114) % 31 + 1;
    NaiveDate::from_ymd_opt(year, month as u32, day as u32).expect("valid Easter date")
}

/// Public holidays of a country in a year (fixed dates, Nth weekdays and
/// Easter-based days; observed-day shifts are not applied).
pub fn holidays(country: &str, year: i32) -> Vec<(&'static str, NaiveDate)> {
    use Weekday::*;
    let d = |m: u32, day: u32| NaiveDate::from_ymd_opt(year, m, day).expect("valid date");
    let nth = |m: u32, n: i32, w: Weekday| nth_weekday_of_month(year, m, n, w).expect("valid date");
    let e = easter(year);
    let mut out = match country {
        "United States" => vec![
            ("New Year's Day", d(1, 1)),
            ("Martin Luther King Jr. Day", nth(1, 3, Mon)),
            ("Presidents' Day", nth(2, 3, Mon)),
            ("Memorial Day", nth(5, -1, Mon)),
            ("Juneteenth", d(6, 19)),
            ("Independence Day", d(7, 4)),
            ("Labor Day", nth(9, 1, Mon)),
            ("Columbus Day", nth(10, 2, Mon)),
            ("Veterans Day", d(11, 11)),
            ("Thanksgiving Day", nth(11, 4, Thu)),
            ("Christmas Day", d(12, 25)),
        ],
        "Canada" => {
            // Victoria Day: the Monday before May 25.
            let may25 = d(5, 25);
            let victoria = may25 - Duration::days(((may25.weekday().num_days_from_monday() + 6) % 7 + 1) as i64);
            vec![
                ("New Year's Day", d(1, 1)),
                ("Good Friday", e - Duration::days(2)),
                ("Victoria Day", victoria),
                ("Canada Day", d(7, 1)),
                ("Civic Holiday", nth(8, 1, Mon)),
                ("Labour Day", nth(9, 1, Mon)),
                ("National Day for Truth and Reconciliation", d(9, 30)),
                ("Thanksgiving", nth(10, 2, Mon)),
                ("Remembrance Day", d(11, 11)),
                ("Christmas Day", d(12, 25)),
                ("Boxing Day", d(12, 26)),
            ]
        }
        "United Kingdom" => vec![
            ("New Year's Day", d(1, 1)),
            ("Good Friday", e - Duration::days(2)),
            ("Easter Monday", e + Duration::days(1)),
            ("Early May bank holiday", nth(5, 1, Mon)),
            ("Spring bank holiday", nth(5, -1, Mon)),
            ("Summer bank holiday", nth(8, -1, Mon)),
            ("Christmas Day", d(12, 25)),
            ("Boxing Day", d(12, 26)),
        ],
        _ => Vec::new(),
    };
    out.sort_by_key(|(_, d)| *d);
    out
}

fn holiday_calendar_name(country: &str) -> String {
    format!("{country} holidays")
}

/// Add (with this year's and next year's holidays) or remove a country's
/// holiday calendar.
fn set_holidays(app: AppState, country: &str, on: bool) {
    let name = holiday_calendar_name(country);
    let existing = store().calendars().unwrap_or_default().into_iter().find(|c| c.name == name);
    let r: axentra_core::Result<()> = match (on, existing) {
        (false, Some(c)) => store().delete_calendar(c.id),
        (true, None) => (|| {
            let account = store().primary_account()?.ok_or_else(|| Error::Invalid("No account".into()))?;
            let id = store().create_calendar(account.id, &name, PALETTE[2].primary, "Other calendars")?;
            store().calendar(id).and_then(|c| match c {
                Some(mut c) => {
                    c.can_edit = false;
                    store().update_calendar(&c)
                }
                None => Ok(()),
            })?;
            let year = chrono::Local::now().year();
            for y in [year, year + 1] {
                for (title, date) in holidays(country, y) {
                    let mut e = Event {
                        calendar_id: id,
                        title: title.into(),
                        start: local_to_utc(date, NaiveTime::MIN),
                        end: local_to_utc(date + Duration::days(1), NaiveTime::MIN),
                        all_day: true,
                        show_as: ShowAs::Free,
                        time_zone: "Local".into(),
                        ..Default::default()
                    };
                    store().insert_event(&mut e)?;
                }
            }
            Ok(())
        })(),
        _ => Ok(()),
    };
    if app.check(r).is_some() {
        app.bump_calendar();
        app.toast(if on { format!("Added {name}") } else { format!("Removed {name}") });
    }
}

#[component]
fn HolidaysPage() -> Element {
    let app = use_app();
    let _ = app.calendar_rev();
    let mut query = use_signal(String::new);
    let names: Vec<String> = store().calendars().unwrap_or_default().into_iter().map(|c| c.name).collect();
    let q = query().to_lowercase();
    rsx! {
        div { class: "ax-col", style: "gap:12px",
            h3 { class: "ax-cal-addcal-title", "Holidays" }
            p { class: "ax-muted", "Pick countries or regions. Their holidays are added to \u{201c}Other calendars\u{201d}." }
            input {
                class: "ax-input",
                placeholder: "Filter countries and regions",
                "aria-label": "Filter countries and regions",
                value: "{query}",
                oninput: move |e| query.set(e.value()),
                onkeydown: move |e| keys::stop_typing_keys(&e),
            }
            div { class: "ax-cal-holiday-list",
                for country in COUNTRIES.into_iter().filter(|c| c.to_lowercase().contains(&q)) {
                    {
                        let on = names.contains(&holiday_calendar_name(country));
                        rsx! {
                            crate::kit::Checkbox {
                                key: "{country}",
                                checked: on,
                                label: country,
                                onchange: move |v: bool| set_holidays(app, country, v),
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easter_and_holiday_rules() {
        assert_eq!(easter(2026), NaiveDate::from_ymd_opt(2026, 4, 5).unwrap());
        assert_eq!(easter(2027), NaiveDate::from_ymd_opt(2027, 3, 28).unwrap());
        assert_eq!(easter(2025), NaiveDate::from_ymd_opt(2025, 4, 20).unwrap());
        let us = holidays("United States", 2026);
        assert!(us.contains(&("Thanksgiving Day", NaiveDate::from_ymd_opt(2026, 11, 26).unwrap())));
        let ca = holidays("Canada", 2026);
        assert!(ca.contains(&("Victoria Day", NaiveDate::from_ymd_opt(2026, 5, 18).unwrap())));
        let uk = holidays("United Kingdom", 2026);
        assert!(uk.contains(&("Easter Monday", NaiveDate::from_ymd_opt(2026, 4, 6).unwrap())));
        assert!(holidays("Atlantis", 2026).is_empty());
    }
}
