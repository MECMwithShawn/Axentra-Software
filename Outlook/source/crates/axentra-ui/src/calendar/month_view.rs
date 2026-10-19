//! Month view: whole weeks, multi-day bars on consistent lanes, timed
//! events as one-line chips, "+N more" overflow, drag between days.

use std::time::Instant;

use axentra_core::*;
use chrono::{Datelike, Duration, NaiveDate};
use dioxus::html::input_data::MouseButton;
use dioxus::prelude::*;

use super::actions::{self, Perms};
use super::colors::event_style;
use super::event_block::{EventIcons, aria_label, display_title, state_classes};
use super::range::{self, ViewKind};
use super::state::*;
use super::time_grid::{Bar, build_bars, event_key, is_long, load_occurrences};
use crate::fmt;
use crate::state::{AppState, store, use_app};

/// Weekday header height and the date label area at the top of a cell.
const HEADER_H: f64 = 32.0;
const LABEL_H: f64 = 26.0;
/// Chip / bar pitch: 20px items with a 2px gap.
const LANE_H: f64 = 22.0;

/// A timed occurrence on one day of a week row.
#[derive(Clone, Debug, PartialEq)]
struct Chip {
    occ: Occurrence,
    day: usize,
    lane: usize,
    style: String,
    movable: bool,
    invitation: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct WeekRow {
    days: Vec<NaiveDate>,
    bars: Vec<Bar>,
    chips: Vec<Chip>,
}

fn build_weeks(cells: &[NaiveDate], occs: &[Occurrence], cats: &[Category]) -> Vec<WeekRow> {
    let perms = Perms::load();
    let (long, timed): (Vec<Occurrence>, Vec<Occurrence>) = occs.iter().cloned().partition(is_long);
    cells
        .chunks(7)
        .map(|week| {
            let bars = build_bars(week, &long, cats, &perms);
            let mut chips = Vec::new();
            for (i, day) in week.iter().enumerate() {
                let mut taken: Vec<usize> = bars.iter().filter(|b| b.first <= i && b.last >= i).map(|b| b.lane).collect();
                for o in timed.iter().filter(|o| local_date(o.start) == *day || (o.start < at_min(*day, 0) && o.end > at_min(*day, 0))) {
                    let lane = (0..).find(|l| !taken.contains(l)).unwrap_or(0);
                    taken.push(lane);
                    chips.push(Chip {
                        day: i,
                        lane,
                        style: event_style(o, cats),
                        movable: perms.can_reschedule(&o.event),
                        invitation: perms.is_invitation(&o.event),
                        occ: o.clone(),
                    });
                }
            }
            WeekRow { days: week.to_vec(), bars, chips }
        })
        .collect()
}

#[component]
pub fn MonthView() -> Element {
    let app = use_app();
    let cal = use_cal();
    let s = app.settings.read().clone();
    let anchor = (cal.anchor)();
    let today = (cal.today)();
    let cells = range::month_grid(anchor, s.week_start);
    let mut size = use_signal(|| (0.0f64, 0.0f64));

    let weeks = use_memo(move || {
        let _ = app.calendar_rev();
        let s = app.settings.read();
        let cells = range::month_grid((cal.anchor)(), s.week_start);
        let filter = (cal.filter)();
        let cats = store().categories().unwrap_or_default();
        build_weeks(&cells, &load_occurrences(&cells, &filter), &cats)
    });
    let weeks = weeks.read().clone();

    let rows = weeks.len().max(1);
    let (w, h) = size();
    let weeknums = s.show_week_numbers;
    let col_w = if w > 0.0 { (w - if weeknums { 32.0 } else { 0.0 }) / 7.0 } else { 160.0 };
    let row_h = if h > 0.0 { ((h - HEADER_H) / rows as f64).max(96.0) } else { 120.0 };
    let lanes = (((row_h - LABEL_H - 4.0) / LANE_H).floor() as usize).max(1);
    let long_names = col_w >= 120.0;
    let header: Vec<NaiveDate> = cells.iter().take(7).copied().collect();
    let label = range::grid_label(ViewKind::Month, anchor, &cells);
    let dragging = (cal.drag)().is_some_and(|d| d.month);

    rsx! {
        div {
            class: "ax-cal-month",
            tabindex: "-1",
            onmounted: move |e| async move { let _ = e.set_focus(true).await; },
            class: if dragging { "ax-cal-dragging" },
            class: if weeknums { "ax-with-weeknums" },
            role: "grid",
            "aria-label": "{label}",
            onresize: move |e: ResizeEvent| {
                if let Ok(sz) = e.get_content_box_size() {
                    let next = (sz.width, sz.height);
                    if size.peek().0 != next.0 || size.peek().1 != next.1 {
                        size.set(next);
                    }
                }
            },
            div { class: "ax-cal-month-head", role: "row",
                if weeknums { div { class: "ax-cal-weeknum-head" } }
                for d in header {
                    div { key: "{d}", class: "ax-cal-month-wd", role: "columnheader",
                        {if long_names { d.format("%A").to_string() } else { d.format("%a").to_string() }}
                    }
                }
            }
            div { class: "ax-cal-month-body",
                for (r, week) in weeks.into_iter().enumerate() {
                    MonthWeek { key: "{week.days[0]}", row: r, week, lanes, today, anchor_month: anchor.month(), weeknums, col_w, h24: s.use_24h_time }
                }
            }
        }
    }
}

#[component]
fn MonthWeek(row: usize, week: WeekRow, lanes: usize, today: NaiveDate, anchor_month: u32, weeknums: bool, col_w: f64, h24: bool) -> Element {
    let app = use_app();
    let cal = use_cal();
    let drag = (cal.drag)();
    let selected = (cal.selected)();
    let slot = (cal.slot)();
    let s = app.settings.read().clone();
    // Per day: everything on it, and whether it overflows the lanes.
    let count = |i: usize| {
        week.bars.iter().filter(|b| b.first <= i && b.last >= i).count() + week.chips.iter().filter(|c| c.day == i).count()
    };
    let overflow: Vec<bool> = (0..7).map(|i| {
        let max_lane = week
            .bars
            .iter()
            .filter(|b| b.first <= i && b.last >= i)
            .map(|b| b.lane)
            .chain(week.chips.iter().filter(|c| c.day == i).map(|c| c.lane))
            .max();
        max_lane.is_some_and(|l| l >= lanes)
    })
    .collect();
    let limit = |covers_overflow: bool| if covers_overflow { lanes.saturating_sub(1) } else { lanes };
    let shown_bars: Vec<Bar> = week
        .bars
        .iter()
        .filter(|b| b.lane < limit((b.first..=b.last).any(|i| overflow[i])))
        .cloned()
        .collect();
    let shown_chips: Vec<Chip> = week.chips.iter().filter(|c| c.lane < limit(overflow[c.day])).cloned().collect();
    let more: Vec<usize> = (0..7)
        .map(|i| {
            let shown = shown_bars.iter().filter(|b| b.first <= i && b.last >= i).count() + shown_chips.iter().filter(|c| c.day == i).count();
            count(i) - shown
        })
        .collect();
    let drop_day = drag.as_ref().filter(|d| d.month && d.active).map(|d| d.cur_day);
    let drag_src = drag.as_ref().filter(|d| d.month && d.active).and_then(|d| d.occ.as_ref().map(|o| OccKey::of(o)));
    let base = row * 7;
    let left0 = if weeknums { 32.0 } else { 0.0 };
    let wk = range::week_number(week.days[0], s.week_start);

    rsx! {
        div { class: "ax-cal-month-row", role: "row",
            if weeknums {
                button {
                    r#type: "button",
                    class: "ax-cal-weeknum",
                    title: "Go to week {wk}",
                    "aria-label": "Week {wk}",
                    onclick: {
                        let d = week.days[0];
                        move |_| {
                            let s = app.settings.peek().clone();
                            cal.set_view(app, &s, ViewKind::Week);
                            cal.go_to(&s, d);
                        }
                    },
                    "{wk}"
                }
            }
            for (i, d) in week.days.iter().copied().enumerate() {
                MonthCell {
                    key: "{d}",
                    idx: base + i,
                    date: d,
                    today: d == today,
                    outside: d.month() != anchor_month,
                    selected: slot.is_some_and(|s| s.all_day && s.start_day == d),
                    drop_target: drop_day == Some(base + i),
                    more: more[i],
                    more_top: LABEL_H + lanes.saturating_sub(1) as f64 * LANE_H,
                    col_w,
                }
            }
            div { class: "ax-cal-month-layer", style: "left:{left0}px",
                for b in shown_bars {
                    MonthBar {
                        key: "b{b.occ.event.id}-{b.occ.original_start.timestamp()}",
                        selected: selected == Some(OccKey::of(&b.occ)),
                        drag_source: drag_src == Some(OccKey::of(&b.occ)),
                        base,
                        col_w,
                        h24,
                        bar: b,
                    }
                }
                for c in shown_chips {
                    MonthChip {
                        key: "c{c.occ.event.id}-{c.occ.original_start.timestamp()}",
                        selected: selected == Some(OccKey::of(&c.occ)),
                        drag_source: drag_src == Some(OccKey::of(&c.occ)),
                        base,
                        h24,
                        chip: c,
                    }
                }
            }
        }
    }
}

#[component]
fn MonthCell(idx: usize, date: NaiveDate, today: bool, outside: bool, selected: bool, drop_target: bool, more: usize, more_top: f64, col_w: f64) -> Element {
    let app = use_app();
    let mut cal = use_cal();
    let label = if date.day() == 1 { date.format("%b %-d").to_string() } else { date.day().to_string() };
    rsx! {
        div {
            class: "ax-cal-month-cell",
            class: if today { "ax-today" },
            class: if outside { "ax-outside" },
            class: if selected { "ax-selected" },
            class: if drop_target { "ax-drop-target" },
            role: "gridcell",
            "aria-label": format!("{}{}", date.format("%A, %B %-d"), if today { ", today" } else { "" }),
            "aria-selected": if selected { "true" } else { "false" },
            onmousemove: move |e: MouseEvent| {
                let Some(mut d) = cal.drag.peek().clone() else { return };
                if !d.month {
                    return;
                }
                if e.held_buttons().is_empty() {
                    cal.drag.set(None);
                    return;
                }
                let c = e.client_coordinates();
                if d.track(idx, 0, (c.x, c.y)) {
                    cal.drag.set(Some(d));
                }
            },
            onclick: move |e: MouseEvent| {
                let (c, off) = (e.client_coordinates(), e.element_coordinates());
                let sel = SlotSel { start_day: date, end_day: date, start_min: 0, end_min: 0, all_day: true };
                // Beside the day cell.
                let anchor = Anchor { x: c.x - off.x, y: c.y - off.y, w: col_w, h: LABEL_H };
                cal.slot.set(Some(sel));
                cal.selected.set(None);
                cal.popover.set(Some(Popover::QuickCreate { sel, anchor }));
            },
            ondoubleclick: move |_| {
                let last = cal.last_click.peek().clone();
                if let Some((occ, _)) = last.filter(|(_, at)| at.elapsed().as_millis() < 600) {
                    cal.close_popover();
                    cal.open_occurrence(&occ);
                    return;
                }
                cal.new_event(app, at_min(date, 0), at_min(date + Duration::days(1), 0), true);
            },
            button {
                r#type: "button",
                class: "ax-cal-month-date",
                title: format!("Go to {}", date.format("%A, %B %-d, %Y")),
                onclick: move |e| {
                    e.stop_propagation();
                    let s = app.settings.peek().clone();
                    cal.set_view(app, &s, ViewKind::Day(1));
                    cal.go_to(&s, date);
                },
                span { "{label}" }
            }
            if more > 0 {
                button {
                    r#type: "button",
                    class: "ax-cal-month-more",
                    style: "top:{more_top}px",
                    onclick: move |e: MouseEvent| {
                        e.stop_propagation();
                        let c = e.client_coordinates();
                        cal.popover.set(Some(Popover::DayEvents { date, anchor: Anchor::point(c.x, c.y) }));
                    },
                    {format!("+{more} more")}
                }
            }
        }
    }
}

fn start_drag(mut cal: Cal, occ: &Occurrence, day: usize, movable: bool, e: &MouseEvent) {
    let c = e.client_coordinates();
    cal.close_popover();
    cal.drag.set(Some(Drag {
        kind: DragKind::Move,
        occ: Some(Box::new(occ.clone())),
        client: (c.x, c.y),
        active: false,
        origin_day: day,
        origin_min: 0,
        cur_day: day,
        cur_min: 0,
        col_top: 0.0,
        scroll0: 0.0,
        col_left: 0.0,
        col_w: 0.0,
        movable,
        month: true,
        allday: false,
    }));
}

#[component]
fn MonthBar(bar: Bar, base: usize, col_w: f64, selected: bool, drag_source: bool, h24: bool) -> Element {
    let app = use_app();
    let mut cal = use_cal();
    let key = OccKey::of(&bar.occ);
    use_drop(move || {
        if let Ok(mut m) = cal.blocks.try_write() {
            m.remove(&key);
        }
    });
    let left = bar.first as f64 * 100.0 / 7.0;
    let width = (bar.last - bar.first + 1) as f64 * 100.0 / 7.0;
    let top = LABEL_H + bar.lane as f64 * LANE_H;
    let classes = state_classes(&bar.occ.event, bar.invitation);
    let movable = bar.movable;
    let (o1, o2, o3) = (bar.occ.clone(), bar.occ.clone(), bar.occ.clone());
    let first = bar.first;
    rsx! {
        div {
            class: "ax-cal-ev ax-cal-bar ax-cal-month-bar {classes}",
            class: if selected { "ax-selected" },
            class: if drag_source { "ax-cal-dragsrc" },
            class: if bar.cont_before { "ax-cont-before" },
            class: if bar.cont_after { "ax-cont-after" },
            role: "button",
            "data-occ": key.dom_id(),
            tabindex: "0",
            "aria-label": aria_label(&bar.occ, h24),
            style: "{bar.style}left:calc({left}% + 2px);width:calc({width}% - 4px);top:{top}px",
            onmounted: move |e| { cal.blocks.write().insert(key, e.data()); },
            onmousedown: move |e: MouseEvent| {
                if e.trigger_button() != Some(MouseButton::Primary) {
                    return;
                }
                e.stop_propagation();
                let grabbed = first + (e.element_coordinates().x / col_w.max(1.0)) as usize;
                start_drag(cal, &o1, base + grabbed.min(6), movable, &e);
            },
            onclick: move |e| e.stop_propagation(),
            ondoubleclick: move |e| { e.stop_propagation(); cal.open_occurrence(&o2) },
            oncontextmenu: move |e: MouseEvent| {
                e.prevent_default();
                e.stop_propagation();
                let c = e.client_coordinates();
                cal.selected.set(Some(key));
                cal.popover.set(Some(Popover::Context { occ: Box::new(o3.clone()), x: c.x, y: c.y }));
            },
            onfocus: move |_| if *cal.selected.peek() != Some(key) { cal.selected.set(Some(key)) },
            onkeydown: {
                let occ = bar.occ.clone();
                move |e: KeyboardEvent| event_key(app, cal, &occ, &e)
            },
            span { class: "ax-cal-ev-title", {display_title(&bar.occ.event)} }
            EventIcons { event: bar.occ.event.clone() }
        }
    }
}

#[component]
fn MonthChip(chip: Chip, base: usize, selected: bool, drag_source: bool, h24: bool) -> Element {
    let app = use_app();
    let mut cal = use_cal();
    let key = OccKey::of(&chip.occ);
    use_drop(move || {
        if let Ok(mut m) = cal.blocks.try_write() {
            m.remove(&key);
        }
    });
    let left = chip.day as f64 * 100.0 / 7.0;
    let width = 100.0 / 7.0;
    let top = LABEL_H + chip.lane as f64 * LANE_H;
    let e = &chip.occ.event;
    let classes = state_classes(e, chip.invitation);
    let (o1, o2, o3, o4) = (chip.occ.clone(), chip.occ.clone(), chip.occ.clone(), chip.occ.clone());
    let day = base + chip.day;
    let movable = chip.movable;
    rsx! {
        div {
            class: "ax-cal-chip {classes}",
            class: if selected { "ax-selected" },
            class: if drag_source { "ax-cal-dragsrc" },
            role: "button",
            "data-occ": key.dom_id(),
            tabindex: "0",
            "aria-label": aria_label(&chip.occ, h24),
            style: "{chip.style}left:calc({left}% + 2px);width:calc({width}% - 4px);top:{top}px",
            onmounted: move |ev| { cal.blocks.write().insert(key, ev.data()); },
            onmousedown: move |ev: MouseEvent| {
                if ev.trigger_button() != Some(MouseButton::Primary) {
                    return;
                }
                ev.stop_propagation();
                start_drag(cal, &o1, day, movable, &ev);
            },
            onclick: move |ev| ev.stop_propagation(),
            ondoubleclick: move |ev| { ev.stop_propagation(); cal.open_occurrence(&o2) },
            oncontextmenu: move |ev: MouseEvent| {
                ev.prevent_default();
                ev.stop_propagation();
                let c = ev.client_coordinates();
                cal.selected.set(Some(key));
                cal.popover.set(Some(Popover::Context { occ: Box::new(o3.clone()), x: c.x, y: c.y }));
            },
            onfocus: move |_| if *cal.selected.peek() != Some(key) { cal.selected.set(Some(key)) },
            onkeydown: move |ev: KeyboardEvent| event_key(app, cal, &o4, &ev),
            span { class: "ax-cal-chip-dot" }
            span { class: "ax-cal-chip-time", {fmt::time(chip.occ.start, h24)} }
            span { class: "ax-cal-chip-title", {display_title(e)} }
            EventIcons { event: e.clone() }
        }
    }
}

/// Mouse released after pressing on a month chip or bar: a click opens the
/// peek card, a drag moves the event to another day keeping its time.
pub fn finish_month_drag(app: AppState, mut cal: Cal, d: Drag, client: (f64, f64)) {
    let Some(occ) = d.occ.as_deref().cloned() else { return };
    if !d.active {
        cal.last_click.set(Some((occ.clone(), Instant::now())));
        actions::open_peek(cal, occ, Anchor::point(client.0, client.1));
        return;
    }
    let s = app.settings.peek().clone();
    let cells = range::month_grid(*cal.anchor.peek(), s.week_start);
    let (Some(from), Some(to)) = (cells.get(d.origin_day), cells.get(d.cur_day)) else { return };
    let delta = *to - *from;
    if delta.is_zero() {
        return;
    }
    let (start, end) = shift_days(&occ, delta.num_days());
    let all_day = occ.event.all_day;
    actions::request_move(app, cal, &occ, start, end, all_day);
}

/// Move an occurrence by whole days keeping its wall-clock times.
pub fn shift_days(occ: &Occurrence, days: i64) -> (chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>) {
    let s = occ.start.with_timezone(&chrono::Local);
    let e = occ.end.with_timezone(&chrono::Local);
    let start = local_to_utc(s.date_naive() + Duration::days(days), s.time());
    let end = local_to_utc(e.date_naive() + Duration::days(days), e.time());
    (start, end.max(start))
}
