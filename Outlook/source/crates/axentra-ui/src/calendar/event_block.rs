//! Shared pieces of an event's rendering: Show-as / response classes, the
//! title text, status icons and the accessible name.

use axentra_core::*;
use axentra_core::Event;
use dioxus::prelude::*;

use super::text::{self, show_as_legend};
use crate::fmt;
use crate::kit::Icon;

/// Title as shown on blocks: "(No title)" for empty, "Canceled: " prefix.
pub fn display_title(e: &Event) -> String {
    let t = if e.title.trim().is_empty() { "(No title)" } else { e.title.trim() };
    if e.is_cancelled { format!("Canceled: {t}") } else { t.to_string() }
}

/// Classes describing how an event is drawn: Show-as pattern, response
/// state, canceled. `invitation` marks meetings from someone else.
pub fn state_classes(e: &Event, invitation: bool) -> String {
    let mut c = String::from(match e.show_as {
        ShowAs::Free => "ax-sa-free",
        ShowAs::WorkingElsewhere => "ax-sa-elsewhere",
        ShowAs::Tentative => "ax-sa-tentative",
        ShowAs::Busy => "ax-sa-busy",
        ShowAs::OutOfOffice => "ax-sa-oof",
    });
    if invitation && e.my_response == ResponseStatus::NotResponded {
        // Unanswered invitations draw with the Tentative pattern and italic title.
        c = "ax-sa-tentative ax-cal-unanswered".into();
    }
    if e.my_response == ResponseStatus::Declined {
        c = "ax-sa-free ax-cal-declined".into();
    }
    if e.is_cancelled {
        c.push_str(" ax-cal-canceled");
    }
    c
}

/// Screen-reader name: "<Title>, <start> to <end>, <date>, <location>,
/// organizer <name>, <Show as>, Recurring, Private, <response>".
pub fn aria_label(occ: &Occurrence, h24: bool) -> String {
    let e = &occ.event;
    let mut parts = vec![display_title(e)];
    if e.all_day {
        parts.push("All day".into());
    } else {
        parts.push(format!("{} to {}", fmt::time(occ.start, h24), fmt::time(occ.end, h24)));
    }
    parts.push(local_date(occ.start).format("%A, %B %-d, %Y").to_string());
    if !e.location.trim().is_empty() {
        parts.push(e.location.trim().to_string());
    }
    if let Some(o) = &e.organizer {
        parts.push(format!("organizer {}", o.display()));
    }
    parts.push(show_as_legend(e.show_as).to_string());
    if e.recurrence.is_some() {
        parts.push("Recurring".into());
    } else if e.series_id.is_some() {
        parts.push("Changed occurrence of a series".into());
    }
    if e.is_private {
        parts.push("Private".into());
    }
    let r = text::response_label(e.my_response);
    if !r.is_empty() && e.my_response != ResponseStatus::Organizer {
        parts.push(r.into());
    }
    parts.join(", ")
}

/// Status icons at the end of the title row, in Outlook's order: recurring
/// (or "changed occurrence"), online meeting, private.
#[component]
pub fn EventIcons(event: Event) -> Element {
    rsx! {
        span { class: "ax-cal-ev-icons",
            if event.recurrence.is_some() {
                Icon { name: "arrow_repeat_all", size: 12 }
            } else if event.series_id.is_some() {
                Icon { name: "arrow_repeat_all_off", size: 12 }
            }
            if event.is_online_meeting { Icon { name: "video", size: 12 } }
            if event.is_private { Icon { name: "lock_closed", size: 12 } }
        }
    }
}
