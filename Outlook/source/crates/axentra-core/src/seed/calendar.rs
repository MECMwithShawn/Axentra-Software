//! Demo calendar: a realistic work week around `now`, with recurring
//! meetings (including edited and deleted occurrences), overlaps, all-day and
//! multi-day items, invitations waiting for an answer and a canceled meeting.

use chrono::{Datelike, Duration, NaiveDate, NaiveTime, Timelike, Weekday};

use super::{SeedCtx, person};
use crate::model::*;
use crate::recurrence::nth_weekday_of_month;
use crate::{Result, Store, local_to_utc};

fn at(date: NaiveDate, h: u32, m: u32) -> chrono::DateTime<chrono::Utc> {
    local_to_utc(date, NaiveTime::from_hms_opt(h, m, 0).expect("valid time"))
}

fn attendees(idx: &[usize]) -> Vec<Attendee> {
    idx.iter()
        .map(|&i| Attendee { email: person(i), kind: AttendeeKind::Required, response: ResponseStatus::Accepted })
        .collect()
}

/// Attendees with individual responses (for the organizer's tally).
fn attendees_with(list: &[(usize, AttendeeKind, ResponseStatus)]) -> Vec<Attendee> {
    list.iter().map(|&(i, kind, response)| Attendee { email: person(i), kind, response }).collect()
}

fn weekly(days: &[Weekday], interval: u32) -> Option<Recurrence> {
    Some(Recurrence { freq: RecurrenceFreq::Weekly, interval, by_weekday: days.to_vec(), ..Default::default() })
}

pub(super) fn seed(store: &Store, ctx: &SeedCtx) -> Result<()> {
    use ResponseStatus::*;
    use Weekday::*;

    let mut main = store.default_calendar(Some(ctx.account_id))?.expect("account has a calendar");
    // The default calendar starts as Blue (Fluent `blue` primary).
    main.color = "#0078d4".into();
    store.update_calendar(&main)?;
    let mut birthdays = Calendar {
        account_id: ctx.account_id,
        name: "Birthdays".into(),
        color: "#e43ba6".into(),
        is_visible: true,
        can_edit: true,
        group: "My calendars".into(),
        sort_order: 1,
        ..Default::default()
    };
    store.add_calendar(&mut birthdays)?;
    let mut personal = Calendar {
        account_id: ctx.account_id,
        name: "Personal".into(),
        color: "#f7630c".into(),
        is_visible: true,
        can_edit: true,
        group: "My calendars".into(),
        sort_order: 2,
        ..Default::default()
    };
    store.add_calendar(&mut personal)?;
    let mut holidays = Calendar {
        account_id: ctx.account_id,
        name: "United States holidays".into(),
        color: "#107c10".into(),
        is_visible: true,
        can_edit: false,
        group: "Other calendars".into(),
        sort_order: 3,
        ..Default::default()
    };
    store.add_calendar(&mut holidays)?;

    let today = ctx.now.date_naive();
    let monday = today - Duration::days(today.weekday().num_days_from_monday() as i64);
    let day = |offset: i64| monday + Duration::days(offset);
    let me = Some(ctx.me.clone());
    let online = |slug: &str| Some(format!("https://meet.axentra.example/{slug}"));

    // A daily standup with one occurrence moved and one deleted this week.
    let mut standup = Event {
        calendar_id: main.id,
        title: "Daily standup".into(),
        location: "Online meeting".into(),
        start: at(monday - Duration::weeks(3), 9, 0),
        end: at(monday - Duration::weeks(3), 9, 15),
        recurrence: weekly(&[Mon, Tue, Wed, Thu, Fri], 1),
        organizer: Some(person(1)),
        attendees: attendees(&[1, 3, 7]),
        my_response: Accepted,
        is_online_meeting: true,
        online_meeting_url: online("standup"),
        reminder_minutes: Some(5),
        time_zone: "Local".into(),
        ..Default::default()
    };
    store.insert_event(&mut standup)?;
    let moved = Event {
        title: "Daily standup (moved for the launch review)".into(),
        start: at(day(3), 9, 30),
        end: at(day(3), 9, 45),
        ..standup.clone()
    };
    store.update_occurrence(standup.id, at(day(3), 9, 0), &moved)?;
    store.delete_occurrence(standup.id, at(day(4), 9, 0))?;

    // A monthly guild on "the Nth Thursday", where N puts it in this week.
    let nth = (((day(3).day() - 1) / 7 + 1) as i32).min(4);
    let guild_month = NaiveDate::from_ymd_opt(day(3).year(), day(3).month(), 1).expect("valid") - chrono::Months::new(2);
    let guild_start = nth_weekday_of_month(guild_month.year(), guild_month.month(), nth, Thu).unwrap_or(day(3));

    let mut events: Vec<Event> = vec![
        Event {
            calendar_id: main.id,
            title: "1:1 Alex / Marcus".into(),
            start: at(day(1) - Duration::weeks(2), 10, 0),
            end: at(day(1) - Duration::weeks(2), 10, 30),
            recurrence: Some(Recurrence { freq: RecurrenceFreq::Weekly, interval: 1, ..Default::default() }),
            organizer: Some(person(1)),
            attendees: attendees(&[1]),
            my_response: Accepted,
            is_online_meeting: true,
            online_meeting_url: online("alex-marcus"),
            reminder_minutes: Some(15),
            ..Default::default()
        },
        Event {
            calendar_id: main.id,
            title: "Weekly planning".into(),
            location: "Conf Room Rainier".into(),
            start: at(monday - Duration::weeks(3), 8, 30),
            end: at(monday - Duration::weeks(3), 9, 0),
            recurrence: weekly(&[Mon], 1),
            organizer: me.clone(),
            my_response: Organizer,
            attendees: attendees_with(&[
                (3, AttendeeKind::Required, Accepted),
                (7, AttendeeKind::Required, Accepted),
                (4, AttendeeKind::Optional, Tentative),
            ]),
            reminder_minutes: Some(15),
            ..Default::default()
        },
        Event {
            calendar_id: main.id,
            title: "Focus time".into(),
            start: at(day(0), 13, 30),
            end: at(day(0), 15, 30),
            show_as: ShowAs::Busy,
            organizer: me.clone(),
            my_response: Organizer,
            is_private: true,
            ..Default::default()
        },
        Event {
            calendar_id: main.id,
            title: "Lunch with Sofia".into(),
            location: "Harbor Café".into(),
            start: at(today, 12, 30),
            end: at(today, 13, 30),
            organizer: me.clone(),
            my_response: Organizer,
            attendees: attendees(&[6]),
            categories: vec!["Green category".into()],
            reminder_minutes: Some(15),
            ..Default::default()
        },
        Event {
            calendar_id: main.id,
            title: "Customer visit — Northwind".into(),
            location: "Northwind Traders HQ, 400 Market St".into(),
            start: at(day(1), 9, 30),
            end: at(day(1), 11, 30),
            show_as: ShowAs::WorkingElsewhere,
            organizer: Some(person(0)),
            attendees: attendees(&[0, 10]),
            my_response: Accepted,
            ..Default::default()
        },
        Event {
            calendar_id: main.id,
            title: "Marketing sync".into(),
            start: at(day(1), 13, 0),
            end: at(day(1), 13, 30),
            organizer: Some(person(4)),
            attendees: attendees(&[4]),
            my_response: Accepted,
            is_cancelled: true,
            body_html: "<p>Canceled: we'll cover this in Thursday's launch sync.</p>".into(),
            ..Default::default()
        },
        Event {
            calendar_id: main.id,
            title: "Budget walkthrough".into(),
            location: "Conf Room Baker".into(),
            body_html: "<p>Walk through the FY27 budget draft before it goes to finance.</p>".into(),
            start: at(day(1), 15, 0),
            end: at(day(1), 15, 45),
            organizer: me.clone(),
            my_response: Organizer,
            attendees: attendees_with(&[
                (5, AttendeeKind::Required, Accepted),
                (0, AttendeeKind::Required, Tentative),
                (3, AttendeeKind::Required, NotResponded),
                (4, AttendeeKind::Optional, Declined),
                (10, AttendeeKind::Optional, NotResponded),
            ]),
            reminder_minutes: Some(15),
            ..Default::default()
        },
        // Three overlapping meetings on Wednesday.
        Event {
            calendar_id: main.id,
            title: "Design review: onboarding flow".into(),
            location: "Conf Room Rainier".into(),
            start: at(day(2), 11, 0),
            end: at(day(2), 12, 0),
            organizer: Some(person(2)),
            attendees: attendees(&[2, 0, 3]),
            my_response: Accepted,
            categories: vec!["Purple category".into()],
            ..Default::default()
        },
        Event {
            calendar_id: main.id,
            title: "Lunch & learn: async Rust".into(),
            location: "Kitchen".into(),
            start: at(day(2), 11, 15),
            end: at(day(2), 12, 15),
            show_as: ShowAs::Free,
            organizer: Some(person(3)),
            attendees: attendees(&[3, 7]),
            my_response: Accepted,
            ..Default::default()
        },
        Event {
            calendar_id: main.id,
            title: "Vendor call — Lumen".into(),
            start: at(day(2), 11, 30),
            end: at(day(2), 12, 30),
            organizer: Some(person(6)),
            attendees: attendees(&[6]),
            my_response: Tentative,
            show_as: ShowAs::Tentative,
            is_online_meeting: true,
            online_meeting_url: online("lumen"),
            ..Default::default()
        },
        Event {
            calendar_id: main.id,
            title: "Interview: Senior Engineer (system design)".into(),
            location: "Conf Room Baker".into(),
            start: at(day(2), 14, 0),
            end: at(day(2), 15, 0),
            organizer: Some(person(9)),
            attendees: attendees(&[9]),
            my_response: NotResponded,
            show_as: ShowAs::Tentative,
            reminder_minutes: Some(15),
            ..Default::default()
        },
        Event {
            calendar_id: main.id,
            title: "Q4 launch sync".into(),
            location: "Online meeting".into(),
            body_html: "<p>Agenda: go/no-go, comms plan, support readiness.</p>".into(),
            start: at(day(3), 14, 0),
            end: at(day(3), 15, 0),
            organizer: Some(person(0)),
            attendees: attendees(&[0, 1, 4]),
            my_response: Accepted,
            is_online_meeting: true,
            online_meeting_url: online("q4-sync"),
            categories: vec!["Red category".into()],
            reminder_minutes: Some(15),
            ..Default::default()
        },
        Event {
            calendar_id: main.id,
            title: "Architecture guild".into(),
            location: "Online meeting".into(),
            body_html: "<p>Monthly guild: RFC reviews and tech-debt triage.</p>".into(),
            start: at(guild_start, 16, 0),
            end: at(guild_start, 17, 0),
            recurrence: Some(Recurrence {
                freq: RecurrenceFreq::Monthly,
                interval: 1,
                nth_weekday: Some((nth, Thu)),
                ..Default::default()
            }),
            organizer: Some(person(1)),
            attendees: attendees(&[1, 3, 7]),
            my_response: NotResponded,
            show_as: ShowAs::Tentative,
            is_online_meeting: true,
            online_meeting_url: online("guild"),
            ..Default::default()
        },
        Event {
            calendar_id: main.id,
            title: "Release night".into(),
            location: "War room".into(),
            start: at(day(3), 21, 0),
            end: at(day(4), 2, 0),
            organizer: Some(person(1)),
            attendees: attendees(&[1, 3, 7]),
            my_response: Accepted,
            ..Default::default()
        },
        Event {
            calendar_id: main.id,
            title: "Dentist".into(),
            start: at(day(4), 8, 0),
            end: at(day(4), 9, 0),
            show_as: ShowAs::OutOfOffice,
            is_private: true,
            organizer: me.clone(),
            my_response: Organizer,
            reminder_minutes: Some(60),
            ..Default::default()
        },
        Event {
            calendar_id: main.id,
            title: "Company offsite".into(),
            location: "Pier 27".into(),
            start: at(day(4), 0, 0),
            end: at(day(5), 0, 0),
            all_day: true,
            show_as: ShowAs::WorkingElsewhere,
            organizer: Some(person(10)),
            my_response: Accepted,
            ..Default::default()
        },
        Event {
            calendar_id: main.id,
            title: "Team retro".into(),
            start: at(day(4), 16, 0),
            end: at(day(4), 17, 0),
            organizer: Some(person(1)),
            attendees: attendees(&[1, 3, 7]),
            my_response: Accepted,
            recurrence: Some(Recurrence { freq: RecurrenceFreq::Weekly, interval: 2, ..Default::default() }),
            ..Default::default()
        },
        Event {
            calendar_id: main.id,
            title: "Seattle trip".into(),
            location: "Seattle, WA".into(),
            start: at(day(7), 0, 0),
            end: at(day(10), 0, 0),
            all_day: true,
            show_as: ShowAs::OutOfOffice,
            organizer: me.clone(),
            my_response: Organizer,
            ..Default::default()
        },
        Event {
            calendar_id: main.id,
            title: "Northwind partner summit".into(),
            location: "Seattle Convention Center".into(),
            start: at(day(8), 0, 0),
            end: at(day(10), 0, 0),
            all_day: true,
            show_as: ShowAs::Busy,
            organizer: Some(person(0)),
            my_response: Accepted,
            categories: vec!["Blue category".into()],
            ..Default::default()
        },
        // Personal calendar.
        Event {
            calendar_id: personal.id,
            title: "Gym".into(),
            location: "Harbor Fitness".into(),
            start: at(monday - Duration::weeks(1), 7, 0),
            end: at(monday - Duration::weeks(1), 8, 0),
            recurrence: weekly(&[Mon, Wed, Fri], 1),
            show_as: ShowAs::Free,
            is_private: true,
            organizer: me.clone(),
            my_response: Organizer,
            ..Default::default()
        },
        Event {
            calendar_id: personal.id,
            title: "Dinner with Grace".into(),
            location: "Il Porto".into(),
            start: at(day(3), 19, 0),
            end: at(day(3), 21, 0),
            organizer: me.clone(),
            my_response: Organizer,
            reminder_minutes: Some(60),
            ..Default::default()
        },
    ];

    // Something coming up shortly after launch, so the reminder dialog has
    // a realistic first customer.
    let now = ctx.now.with_timezone(&chrono::Utc);
    let past_half = (ctx.now.minute() % 30) as i64 * 60 + ctx.now.second() as i64;
    let prep_start = now - Duration::seconds(past_half) + Duration::minutes(60);
    let prep_start = prep_start - Duration::nanoseconds(prep_start.timestamp_subsec_nanos() as i64);
    events.push(Event {
        calendar_id: main.id,
        title: "Prep: launch talking points".into(),
        start: prep_start,
        end: prep_start + Duration::minutes(30),
        organizer: me.clone(),
        my_response: Organizer,
        reminder_minutes: Some(15),
        ..Default::default()
    });

    // US holidays near the demo period.
    let year = today.year();
    let mut holiday_dates: Vec<(&str, NaiveDate)> = Vec::new();
    for y in [year, year + 1] {
        let fixed = [("New Year's Day", 1, 1), ("Juneteenth", 6, 19), ("Independence Day", 7, 4), ("Halloween", 10, 31), ("Veterans Day", 11, 11), ("Christmas Eve", 12, 24), ("Christmas Day", 12, 25)];
        for (title, m, d) in fixed {
            if let Some(date) = NaiveDate::from_ymd_opt(y, m, d) {
                holiday_dates.push((title, date));
            }
        }
        let floating = [
            ("Martin Luther King Jr. Day", 1, 3, Mon),
            ("Presidents' Day", 2, 3, Mon),
            ("Memorial Day", 5, -1, Mon),
            ("Labor Day", 9, 1, Mon),
            ("Columbus Day", 10, 2, Mon),
            ("Thanksgiving Day", 11, 4, Thu),
        ];
        for (title, m, n, wd) in floating {
            if let Some(date) = nth_weekday_of_month(y, m, n, wd) {
                holiday_dates.push((title, date));
            }
        }
    }
    for (title, date) in holiday_dates {
        if date < today - Duration::days(120) || date > today + Duration::days(240) {
            continue;
        }
        events.push(Event {
            calendar_id: holidays.id,
            title: title.into(),
            start: at(date, 0, 0),
            end: at(date + Duration::days(1), 0, 0),
            all_day: true,
            show_as: ShowAs::Free,
            ..Default::default()
        });
    }

    for (title, date) in [
        ("Priya Raman's birthday", NaiveDate::from_ymd_opt(year, today.month(), 1).unwrap_or(today) + Duration::days(22)),
        ("Marcus Lee's birthday", day(3).with_year(year - 1).unwrap_or(day(3))),
    ] {
        events.push(Event {
            calendar_id: birthdays.id,
            title: title.into(),
            start: at(date, 0, 0),
            end: at(date + Duration::days(1), 0, 0),
            all_day: true,
            show_as: ShowAs::Free,
            recurrence: Some(Recurrence { freq: RecurrenceFreq::Yearly, interval: 1, ..Default::default() }),
            reminder_minutes: Some(18 * 60),
            ..Default::default()
        });
    }

    for e in &mut events {
        if e.time_zone.is_empty() {
            e.time_zone = "Local".into();
        }
        store.insert_event(e)?;
    }
    Ok(())
}
