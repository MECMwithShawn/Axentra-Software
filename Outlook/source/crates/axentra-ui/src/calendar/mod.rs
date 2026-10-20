//! Calendar module (docs/spec/calendar.md): folder pane (New event, mini
//! month, calendar groups), the surface (Day / N-day / Work week / Week time
//! grid and Month grid), peek card, quick create, the full event editor,
//! reminders and calendar search.
//!
//! Module state lives in a module-local context (`state::Cal`); store data
//! is re-read on `app.calendar_rev()` like every other module.

mod actions;
mod add_calendar;
mod colors;
mod dialogs;
mod editor;
mod event_block;
mod fields;
mod keyboard;
mod layout;
mod mini_month;
mod month_view;
mod nav_pane;
mod popovers;
mod range;
mod reminders;
mod repeat;
mod ribbon;
mod search;
mod state;
mod surface_header;
mod text;
mod time_grid;

pub use reminders::ReminderHost;

use axentra_core::*;
use chrono::{Duration, Local, Utc};
use dioxus::prelude::*;

use crate::state::{AppState, NavRequest, store, use_app};
use range::ViewKind;
use state::*;

#[component]
pub fn CalendarModule() -> Element {
    let app = use_app();
    let mut cal = provide_cal(app);
    let s = app.settings.read().clone();

    // One tick per minute, aligned to :00 — moves the current-time line and
    // rolls "today" over at midnight.
    use_future(move || async move {
        let mut cal = cal;
        loop {
            let into_minute = Utc::now().timestamp_millis().rem_euclid(60_000) as u64;
            tokio::time::sleep(std::time::Duration::from_millis(60_000 - into_minute + 20)).await;
            cal.now.set(Utc::now());
            let today = Local::now().date_naive();
            let prev = *cal.today.peek();
            if today != prev {
                cal.today.set(today);
                if *cal.anchor.peek() == prev {
                    cal.anchor.set(today);
                }
            }
        }
    });

    // Requests from other modules (open an event, a date, a new event).
    use_effect(move || {
        let req = app.nav.read().clone();
        if let Some(req) = req {
            handle_nav(app, cal, req);
        }
    });

    // Window size, for placing popovers.
    let measure = move || {
        spawn(async move {
            let mut cal = cal;
            if let Ok(v) = document::eval("return [window.innerWidth, window.innerHeight];").join::<(f64, f64)>().await
                && *cal.viewport.peek() != v
            {
                cal.viewport.set(v);
            }
        });
    };
    use_hook(measure);

    let editor = (cal.editor)();
    let searching = {
        let q = app.search.read();
        q.active && !q.query.trim().is_empty()
    };
    let view = (cal.view)();
    let announce = (cal.announce)();
    rsx! {
        div {
            class: "ax-cal-root",
            onkeydown: move |e: KeyboardEvent| keyboard::handle(app, cal, &e),
            onmousedown: move |_| {
                cal.close_popover();
                if *cal.date_flyout.peek() {
                    cal.date_flyout.set(false);
                }
            },
            onmouseup: move |e: MouseEvent| {
                let c = e.client_coordinates();
                time_grid::finish_drag(app, cal, (c.x, c.y));
            },
            onresize: move |_| measure(),
            if let Some(req) = editor {
                editor::EventEditor { key: "{req.serial}", req }
            } else {
                ribbon::CalendarRibbon {}
                div { class: "ax-panes ax-cal-panes",
                    if !s.folder_pane_collapsed {
                        nav_pane::NavPane {}
                    }
                    main { class: "ax-pane ax-grow ax-cal-surface", "aria-label": "Calendar",
                        if searching {
                            search::SearchResults {}
                        } else {
                            surface_header::SurfaceHeader {}
                            if view == ViewKind::Month {
                                month_view::MonthView {}
                            } else {
                                time_grid::TimeGrid {}
                            }
                        }
                    }
                }
                popovers::Popovers {}
            }
            dialogs::Dialogs {}
            ReminderHost {}
            div { class: "ax-visually-hidden", role: "status", "aria-live": "polite", "{announce}" }
        }
    }
}

/// Consume a calendar navigation request (and clear it).
fn handle_nav(mut app: AppState, mut cal: Cal, req: NavRequest) {
    let s = app.settings.peek().clone();
    match req {
        NavRequest::CalendarDate(d) => cal.go_to(&s, d),
        NavRequest::NewEvent { start, end, all_day } => {
            cal.go_to(&s, local_date(start));
            cal.new_event(app, start, end, all_day);
        }
        NavRequest::NewEventWith { start, end, attendees } => {
            cal.go_to(&s, local_date(start));
            let mut event = new_event_draft(app, (cal.active_calendar)(), start, end, false);
            add_attendees(&mut event, &attendees);
            cal.open_editor(event, EditMode::New);
        }
        NavRequest::Event(id) => match store().event(id).ok().flatten() {
            None => {
                app.error("This event no longer exists.");
            }
            Some(e) => {
                // A series opens on its next occurrence (or its first).
                let today = Local::now().date_naive();
                let occ = if e.recurrence.is_some() {
                    store()
                        .occurrences_in_range(at_min(today, 0), at_min(today + Duration::days(366), 0), true)
                        .unwrap_or_default()
                        .into_iter()
                        .find(|o| o.event.id == id)
                } else {
                    None
                };
                let color = store().calendar(e.calendar_id).ok().flatten().map(|c| c.color).unwrap_or_default();
                let occ = occ.unwrap_or(Occurrence { start: e.start, end: e.end, original_start: e.original_start.unwrap_or(e.start), color, event: e.clone() });
                if let Ok(Some(c)) = store().calendar(e.calendar_id)
                    && !c.is_visible
                    && app.check(store().set_calendar_visible(c.id, true)).is_some()
                {
                    app.bump_calendar();
                }
                cal.editor.set(None);
                cal.go_to(&s, local_date(occ.start));
                actions::open_peek(cal, occ, Anchor::point(-1.0, -1.0));
            }
        },
        _ => return,
    }
    app.nav.set(None);
}
