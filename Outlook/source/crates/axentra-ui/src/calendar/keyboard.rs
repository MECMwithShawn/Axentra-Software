//! Calendar keyboard shortcuts (docs/spec/keyboard.md §6, "Outlook" mode)
//! and the grid's slot / day navigation. Keys typed in fields never get
//! here: editable elements stop them with `stop_typing_keys`.

use axentra_core::*;
use chrono::{Datelike, Duration, NaiveDate};
use dioxus::prelude::*;

use super::actions::request_delete;
use super::nav_pane::new_event_now;
use super::range::{ViewKind, add_months, week_start_of};
use super::state::*;
use crate::kit::keys;
use crate::state::AppState;

/// Module-level key handler (bubbles up from the grid, panes and ribbon).
pub fn handle(app: AppState, mut cal: Cal, e: &KeyboardEvent) {
    if cal.editor.peek().is_some() || cal.dialog.peek().is_some() {
        return;
    }
    let chord = keys::chord(e);
    let s = app.settings.peek().clone();
    let off = s.keyboard_mode == KeyboardMode::Off;
    if e.is_auto_repeating() && matches!(chord.as_str(), "Ctrl+N" | "Ctrl+P" | "Ctrl+G") {
        return;
    }
    let view = *cal.view.peek();
    let mut handled = true;
    match chord.as_str() {
        // Commands (disabled when shortcuts are turned off).
        "Ctrl+N" | "Ctrl+Shift+A" | "Ctrl+Shift+Q" if !off => new_event_now(app, cal, None),
        "Ctrl+Alt+1" | "Alt+Shift+1" if !off => cal.set_view(app, &s, ViewKind::Day(1)),
        "Ctrl+Alt+2" | "Alt+Shift+2" if !off => cal.set_view(app, &s, ViewKind::WorkWeek),
        "Ctrl+Alt+3" | "Alt+-" | "Alt+Shift+3" if !off => cal.set_view(app, &s, ViewKind::Week),
        "Ctrl+Alt+4" | "Alt+=" | "Alt+Shift+4" if !off => cal.set_view(app, &s, ViewKind::Month),
        "Alt+1" | "Alt+2" | "Alt+3" | "Alt+4" | "Alt+5" | "Alt+6" | "Alt+7" if !off => {
            // N days starting today.
            let n = chord[4..].parse::<u8>().unwrap_or(1);
            cal.set_view(app, &s, ViewKind::Day(n));
            let today = *cal.today.peek();
            cal.go_to(&s, today);
        }
        "Alt+Shift+Y" | "Home" => cal.go_today(&s),
        "Ctrl+Alt+ArrowLeft" if !off => cal.step(&s, -1),
        "Ctrl+Alt+ArrowRight" if !off => cal.step(&s, 1),
        "Alt+ArrowUp" if !off => {
            let a = *cal.anchor.peek() - Duration::weeks(1);
            cal.go_to(&s, a);
        }
        "Alt+ArrowDown" if !off => {
            let a = *cal.anchor.peek() + Duration::weeks(1);
            cal.go_to(&s, a);
        }
        "Alt+PageUp" if !off => {
            let a = add_months(*cal.anchor.peek(), -1);
            cal.go_to(&s, a);
        }
        "Alt+PageDown" if !off => {
            let a = add_months(*cal.anchor.peek(), 1);
            cal.go_to(&s, a);
        }
        "Ctrl+G" if !off => cal.date_flyout.set(true),
        "Ctrl+P" if !off => {
            document::eval("window.print()");
        }
        "Escape" => {
            if cal.drag.peek().is_some() {
                // Cancel the drag: the event stays where it was.
                cal.drag.set(None);
                super::time_grid::stop_auto_scroll();
            } else if cal.popover.peek().is_some() {
                cal.close_popover();
                super::actions::restore_focus(cal);
            } else if cal.date_flyout.peek().to_owned() {
                cal.date_flyout.set(false);
            } else {
                cal.close_popover();
                cal.clear_selection();
            }
        }
        "Delete" => {
            let sel = *cal.selected.peek();
            match sel.and_then(|k| find_occurrence(app, cal, k)) {
                Some(o) => request_delete(app, cal, &o),
                None => handled = false,
            }
        }
        "Enter" => {
            if let Some(o) = cal.selected.peek().and_then(|k| find_occurrence(app, cal, k)) {
                cal.open_occurrence(&o);
            } else if let Some(sel) = *cal.slot.peek() {
                super::actions::quick_create_at_selection(cal, sel);
            } else {
                handled = false;
            }
        }
        "PageUp" if view != ViewKind::Month => cal.request_scroll(ScrollTo::PageUp),
        "PageDown" if view != ViewKind::Month => cal.request_scroll(ScrollTo::PageDown),
        "Ctrl+Home" | "Ctrl+End" if view != ViewKind::Month => {
            let mut sel = current_slot(app, cal);
            let scale = *cal.scale.peek() as i64;
            let m = if chord == "Ctrl+Home" { s.work_start_hour as i64 * 60 } else { (s.work_end_hour as i64 * 60 - scale).max(0) };
            sel.start_min = m;
            sel.end_min = m + scale;
            sel.end_day = sel.start_day;
            set_slot(cal, sel);
        }
        "ArrowUp" | "ArrowDown" | "ArrowLeft" | "ArrowRight" | "Shift+ArrowUp" | "Shift+ArrowDown" | "Shift+ArrowLeft" | "Shift+ArrowRight" => {
            if view == ViewKind::Month {
                month_arrow(app, cal, &chord);
            } else {
                grid_arrow(app, cal, &chord);
            }
        }
        "PageUp" | "PageDown" | "End" if view == ViewKind::Month => month_arrow(app, cal, &chord),
        _ => handled = false,
    }
    if handled {
        e.prevent_default();
        e.stop_propagation();
    }
}

/// The selected occurrence, re-read from the visible range.
fn find_occurrence(app: AppState, cal: Cal, key: OccKey) -> Option<Occurrence> {
    let days = super::time_grid::days_peek(app, cal);
    let (first, last) = (*days.first()?, *days.last()?);
    store_occurrences(first, last).into_iter().find(|o| OccKey::of(o) == key)
}

fn store_occurrences(first: NaiveDate, last: NaiveDate) -> Vec<Occurrence> {
    crate::state::store().events_in_range(at_min(first, 0), at_min(last + Duration::days(1), 0)).unwrap_or_default()
}

/// The slot selection, or a default one: now (today) or the start of work.
fn current_slot(app: AppState, cal: Cal) -> SlotSel {
    if let Some(s) = *cal.slot.peek()
        && !s.all_day
    {
        return s;
    }
    let s = app.settings.peek();
    let days = super::time_grid::days_peek(app, cal);
    let today = *cal.today.peek();
    let scale = *cal.scale.peek() as i64;
    let (day, min) = if days.contains(&today) {
        (today, super::text::minutes_of_day(chrono::Utc::now()) / scale * scale)
    } else {
        (days.first().copied().unwrap_or(today), s.work_start_hour as i64 * 60)
    };
    SlotSel { start_day: day, end_day: day, start_min: min, end_min: min + scale, all_day: false }
}

fn set_slot(mut cal: Cal, sel: SlotSel) {
    cal.selected.set(None);
    cal.slot.set(Some(sel));
    // Keep the selection on screen.
    let ppm = px_per_min(*cal.scale.peek());
    let top = *cal.scroll_top.peek() / ppm;
    let visible = (cal.viewport.peek().1 - 320.0).max(120.0) / ppm;
    if (sel.start_min as f64) < top || (sel.end_min as f64) > top + visible {
        cal.request_scroll(ScrollTo::Minutes((sel.start_min - 60).max(0)));
    }
}

/// Up/Down move one slot, Left/Right one day (paging at the edges); with
/// Shift they extend the selection.
fn grid_arrow(app: AppState, cal: Cal, chord: &str) {
    let scale = *cal.scale.peek() as i64;
    let mut sel = current_slot(app, cal);
    let days = super::time_grid::days_peek(app, cal);
    let s = app.settings.peek().clone();
    let shift = chord.starts_with("Shift+");
    let key = chord.trim_start_matches("Shift+");
    let day_step = |d: NaiveDate, dir: i64| -> (NaiveDate, bool) {
        let i = days.iter().position(|x| *x == d);
        match i.map(|i| i as i64 + dir) {
            Some(j) if j >= 0 && (j as usize) < days.len() => (days[j as usize], false),
            _ => (d + Duration::days(dir), true),
        }
    };
    match (key, shift) {
        ("ArrowUp", false) => {
            let m = (sel.start_min - scale).max(0);
            sel = SlotSel { start_min: m, end_min: m + scale, end_day: sel.start_day, ..sel };
        }
        ("ArrowDown", false) => {
            let m = (sel.start_min + scale).min(24 * 60 - scale);
            sel = SlotSel { start_min: m, end_min: m + scale, end_day: sel.start_day, ..sel };
        }
        ("ArrowUp", true) => sel.end_min = (sel.end_min - scale).max(sel.start_min + scale),
        ("ArrowDown", true) => sel.end_min = (sel.end_min + scale).min(24 * 60),
        ("ArrowLeft", false) | ("ArrowRight", false) => {
            let dir = if key == "ArrowLeft" { -1 } else { 1 };
            let (d, paged) = day_step(sel.start_day, dir);
            if paged {
                cal.step(&s, dir as i32);
            }
            sel = SlotSel { start_day: d, end_day: d, end_min: sel.start_min + scale, ..sel };
        }
        ("ArrowLeft", true) => {
            if sel.end_day > sel.start_day {
                sel.end_day = day_step(sel.end_day, -1).0;
            }
        }
        ("ArrowRight", true) => {
            let (d, paged) = day_step(sel.end_day, 1);
            if !paged {
                sel.end_day = d;
            }
        }
        _ => return,
    }
    set_slot(cal, sel);
}

/// Month view: arrows move the selected day by a day / week; PageUp/Down a
/// month; End jumps to the end of the week (Home is "go to today").
fn month_arrow(app: AppState, mut cal: Cal, chord: &str) {
    let s = app.settings.peek().clone();
    let current = cal.slot.peek().filter(|x| x.all_day).map(|x| x.start_day).unwrap_or(*cal.anchor.peek());
    let next = match chord.trim_start_matches("Shift+") {
        "ArrowLeft" => current - Duration::days(1),
        "ArrowRight" => current + Duration::days(1),
        "ArrowUp" => current - Duration::weeks(1),
        "ArrowDown" => current + Duration::weeks(1),
        "PageUp" => add_months(current, -1),
        "PageDown" => add_months(current, 1),
        "End" => week_start_of(current, s.week_start) + Duration::days(6),
        _ => return,
    };
    if next.month() != cal.anchor.peek().month() || next.year() != cal.anchor.peek().year() {
        cal.go_to(&s, next);
    }
    cal.slot.set(Some(SlotSel { start_day: next, end_day: next, start_min: 0, end_min: 0, all_day: true }));
    cal.selected.set(None);
}
