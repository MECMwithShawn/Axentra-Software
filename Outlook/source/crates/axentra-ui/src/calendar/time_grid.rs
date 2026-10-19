//! Day / N-day / Work week / Week time grid: day headers, all-day row, hour
//! gutter, day columns with working-hours shading, side-by-side events, the
//! current-time line, slot selection and drag to create / move / resize.

use std::time::Instant;

use axentra_core::*;
use chrono::{Datelike, Duration, Local, NaiveDate};
use dioxus::html::geometry::PixelsVector2D;
use dioxus::html::input_data::MouseButton;
use dioxus::prelude::*;

use super::actions::{self, Perms};
use super::colors::event_style;
use super::event_block::{EventIcons, aria_label, display_title, state_classes};
use super::layout::{ColBox, pack_columns, pack_lanes, visible_columns};
use super::range::{self, ViewKind};
use super::state::*;
use super::text::{clock, minutes_of_day};
use crate::fmt;
use crate::kit::Icon;
use crate::state::{AppState, store, use_app};

/// One timed segment placed in a day column.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub occ: Occurrence,
    pub start_min: i64,
    pub end_min: i64,
    pub boxed: ColBox,
    pub cont_before: bool,
    pub cont_after: bool,
    pub style: String,
    pub movable: bool,
    pub invitation: bool,
}

/// One bar in the all-day row (or a month week row).
#[derive(Clone, Debug, PartialEq)]
pub struct Bar {
    pub occ: Occurrence,
    pub first: usize,
    pub last: usize,
    pub lane: usize,
    pub cont_before: bool,
    pub cont_after: bool,
    pub style: String,
    pub invitation: bool,
    pub movable: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct GridData {
    columns: Vec<Vec<Placed>>,
    bars: Vec<Bar>,
    lanes: usize,
}

/// True for items that belong in the all-day row: all-day events and timed
/// events lasting 24 hours or more.
pub fn is_long(o: &Occurrence) -> bool {
    o.event.all_day || o.end - o.start >= Duration::hours(24)
}

/// Last calendar day an occurrence covers (exclusive end).
pub fn last_day(o: &Occurrence) -> NaiveDate {
    let start = local_date(o.start);
    if o.end <= o.start { start } else { local_date(o.end - Duration::seconds(1)).max(start) }
}

/// Place long items on the visible days and pack them into lanes.
pub fn build_bars(days: &[NaiveDate], occs: &[Occurrence], cats: &[Category], perms: &Perms) -> Vec<Bar> {
    let mut bars = Vec::new();
    for o in occs {
        let (sd, ld) = (local_date(o.start), last_day(o));
        let Some(first) = days.iter().position(|d| *d >= sd) else { continue };
        let Some(last) = days.iter().rposition(|d| *d <= ld) else { continue };
        if first > last {
            continue;
        }
        bars.push(Bar {
            first,
            last,
            lane: 0,
            cont_before: sd < days[first],
            cont_after: ld > days[last],
            style: event_style(o, cats),
            invitation: perms.is_invitation(&o.event),
            movable: perms.can_reschedule(&o.event),
            occ: o.clone(),
        });
    }
    let lanes = pack_lanes(&bars.iter().map(|b| (b.first, b.last)).collect::<Vec<_>>());
    for (b, l) in bars.iter_mut().zip(lanes) {
        b.lane = l;
    }
    bars
}

fn build(days: &[NaiveDate], occs: &[Occurrence], cats: &[Category], scale: u32) -> GridData {
    let perms = Perms::load();
    let (long, timed): (Vec<Occurrence>, Vec<Occurrence>) = occs.iter().cloned().partition(is_long);
    let bars = build_bars(days, &long, cats, &perms);
    let lanes = bars.iter().map(|b| b.lane + 1).max().unwrap_or(0);
    let min_len = ((20.0 / px_per_min(scale)).ceil() as i64).max(1);
    let mut columns = Vec::with_capacity(days.len());
    for day in days {
        let (ds, de) = (at_min(*day, 0), at_min(*day, 24 * 60));
        let mut items: Vec<Placed> = Vec::new();
        for o in &timed {
            let overlaps = o.start < de && (o.end > ds || (o.start == o.end && o.start >= ds));
            if !overlaps {
                continue;
            }
            let start_min = if o.start <= ds { 0 } else { minutes_of_day(o.start) };
            let end_min = if o.end >= de { 24 * 60 } else { minutes_of_day(o.end).max(start_min) };
            items.push(Placed {
                start_min,
                end_min,
                boxed: ColBox::default(),
                cont_before: o.start < ds,
                cont_after: o.end > de,
                style: event_style(o, cats),
                movable: perms.can_reschedule(&o.event),
                invitation: perms.is_invitation(&o.event),
                occ: o.clone(),
            });
        }
        let boxes = pack_columns(&items.iter().map(|p| (p.start_min, p.end_min)).collect::<Vec<_>>(), min_len);
        for (p, b) in items.iter_mut().zip(boxes) {
            p.boxed = b;
        }
        columns.push(items);
    }
    GridData { columns, bars, lanes }
}

/// While a press in the grid is held, scroll the grid when the pointer is
/// within 48px of its top (below the sticky headers) or bottom edge, at up
/// to 600px/s, and replay the pointer position so the drag follows. Removes
/// itself on mouse up (or when `window.__axCalAuto()` is called).
const AUTO_SCROLL_JS: &str = r#"(function(x0,y0){
  if (window.__axCalAuto) window.__axCalAuto();
  const sc = document.querySelector('.ax-cal-scroll');
  if (!sc) return;
  const st = {x:x0, y:y0, raf:0, last:0};
  const EDGE = 48, MAX = 600;
  function tick(t){
    st.raf = 0;
    const r = sc.getBoundingClientRect();
    const sticky = sc.querySelector('.ax-cal-sticky');
    const top = r.top + (sticky ? sticky.offsetHeight : 0);
    let v = 0;
    if (st.y < top + EDGE && st.y >= top) v = -MAX * Math.min(1, (top + EDGE - st.y) / EDGE);
    else if (st.y > r.bottom - EDGE) v = MAX * Math.min(1, (st.y - (r.bottom - EDGE)) / EDGE);
    const dt = st.last ? Math.min(50, t - st.last) : 16;
    st.last = t;
    if (!v) { st.last = 0; return; }
    const before = sc.scrollTop;
    sc.scrollTop = before + v * dt / 1000;
    if (sc.scrollTop !== before) {
      const el = document.elementFromPoint(st.x, st.y);
      if (el) el.dispatchEvent(new MouseEvent('mousemove', {clientX: st.x, clientY: st.y, buttons: 1, bubbles: true}));
    }
    st.raf = requestAnimationFrame(tick);
  }
  function move(e){
    st.x = e.clientX; st.y = e.clientY;
    if (!(e.buttons & 1)) return stop();
    if (Math.hypot(st.x - x0, st.y - y0) < 4) return;
    if (!st.raf) st.raf = requestAnimationFrame(tick);
  }
  function stop(){
    document.removeEventListener('mousemove', move, true);
    document.removeEventListener('mouseup', stop, true);
    if (st.raf) cancelAnimationFrame(st.raf);
    window.__axCalAuto = null;
  }
  window.__axCalAuto = stop;
  document.addEventListener('mousemove', move, true);
  document.addEventListener('mouseup', stop, true);
})"#;

fn start_auto_scroll(client: (f64, f64)) {
    document::eval(&format!("{AUTO_SCROLL_JS}({},{})", client.0, client.1));
}

/// Stop edge auto-scrolling (a drag was cancelled with Escape).
pub fn stop_auto_scroll() {
    document::eval("if(window.__axCalAuto)window.__axCalAuto()");
}

/// Occurrences for the visible range after the ribbon filter.
pub fn load_occurrences(days: &[NaiveDate], filter: &EventFilter) -> Vec<Occurrence> {
    let (Some(first), Some(last)) = (days.first(), days.last()) else { return Vec::new() };
    store()
        .events_in_range(at_min(*first, 0), at_min(*last + Duration::days(1), 0))
        .unwrap_or_default()
        .into_iter()
        .filter(|o| filter.matches(&o.event))
        .collect()
}

#[component]
pub fn TimeGrid() -> Element {
    let app = use_app();
    let mut cal = use_cal();
    let s = app.settings.read().clone();
    let days = cal.days(&s);
    let scale = (cal.scale)();
    let ppm = px_per_min(scale);
    let today = (cal.today)();
    let view = (cal.view)();
    let anchor = (cal.anchor)();

    let data = use_memo(move || {
        let _ = app.calendar_rev();
        let s = app.settings.read();
        let days = range::visible_days((cal.view)(), (cal.anchor)(), s.week_start, &s.work_days);
        let filter = (cal.filter)();
        let cats = store().categories().unwrap_or_default();
        build(&days, &load_occurrences(&days, &filter), &cats, (cal.scale)())
    });
    let data = data.read().clone();

    // Scroll requests (initial position, Today, time-scale changes, PageUp/Down).
    // A grid mounted again (after the editor or Month view) restores the
    // previous scroll offset unless a new request is pending.
    let mut restored = use_signal(|| false);
    // This grid's own scroll container (a new one mounts per view switch).
    let mut grid_el = use_signal(|| None::<std::rc::Rc<MountedData>>);
    use_effect(move || {
        let req = (cal.scroll)();
        let el = grid_el();
        let (Some((serial, to)), Some(el)) = (req, el) else { return };
        if *cal.scroll_applied.peek() == serial {
            if !*restored.peek() {
                restored.set(true);
                let top = *cal.scroll_top.peek();
                spawn(async move {
                    let _ = el.scroll(PixelsVector2D::new(0.0, top), ScrollBehavior::Instant).await;
                });
            }
            return;
        }
        cal.scroll_applied.set(serial);
        restored.set(true);
        let s = app.settings.peek().clone();
        let ppm = px_per_min(*cal.scale.peek());
        let days = range::visible_days(*cal.view.peek(), *cal.anchor.peek(), s.week_start, &s.work_days);
        let today_visible = days.contains(&Local::now().date_naive());
        let top = *cal.scroll_top.peek();
        spawn(async move {
            let h = el.get_client_rect().await.map(|r| r.height()).unwrap_or(600.0) - 80.0;
            let y = match to {
                ScrollTo::Initial if today_visible => minutes_of_day(chrono::Utc::now()) as f64 * ppm - h / 3.0,
                ScrollTo::Initial => (s.work_start_hour as f64 * 60.0 - 30.0) * ppm,
                ScrollTo::Minutes(m) => m as f64 * ppm,
                ScrollTo::PageUp => top - h,
                ScrollTo::PageDown => top + h,
            };
            let _ = el.scroll(PixelsVector2D::new(0.0, y.max(0.0)), ScrollBehavior::Instant).await;
        });
    });

    let n = days.len().max(1);
    // Width of the day columns, to anchor quick create beside a selection.
    let mut days_w = use_signal(|| 0.0f64);
    let col_w = days_w() / n as f64;
    let hour_h = 60.0 * ppm;
    let body_h = 24.0 * hour_h;
    let dragging = (cal.drag)().is_some();
    let label = range::grid_label(view, anchor, &days);
    let work = (s.work_start_hour as i64 * 60, s.work_end_hour as i64 * 60);
    let slot_lines: Vec<(f64, &'static str)> = (1..(24 * 60 / scale as i64))
        .map(|k| {
            let m = k * scale as i64;
            let class = if m % 60 == 0 { "ax-cal-hline" } else if m % 30 == 0 { "ax-cal-halfline" } else { "ax-cal-subline" };
            (m as f64 * ppm, class)
        })
        .collect();

    rsx! {
        div {
            class: "ax-cal-grid",
            class: if dragging { "ax-cal-dragging" },
            role: "grid",
            "aria-label": "{label}",
            style: "--cal-cols:{n};--cal-hour-h:{hour_h}px",
            div {
                class: "ax-cal-scroll",
                tabindex: "-1",
                onmounted: move |e| {
                    let el = e.data();
                    grid_el.set(Some(el.clone()));
                    spawn(async move { let _ = el.set_focus(true).await; });
                },
                onscroll: move |e| {
                    let top = e.scroll_top();
                    cal.scroll_top.set(top);
                    let moved = (top - *cal.popover_scroll.peek()).abs() > 40.0;
                    if moved && matches!(*cal.popover.peek(), Some(Popover::Peek { .. }) | Some(Popover::Context { .. })) {
                        cal.close_popover();
                    }
                },
                div { class: "ax-cal-sticky",
                    div { class: "ax-cal-dayheads", role: "row",
                        div { class: "ax-cal-gutter-head" }
                        for d in days.iter().copied() {
                            DayHeader { key: "{d}", date: d, today: d == today, single: matches!(view, ViewKind::Day(1)) }
                        }
                    }
                    AllDayRow { days: days.clone(), bars: data.bars.clone(), lanes: data.lanes }
                }
                div { class: "ax-cal-body", style: "height:{body_h}px",
                    div { class: "ax-cal-gutter", "aria-hidden": "true",
                        for h in 1..24u32 {
                            span { key: "{h}", class: "ax-cal-hour-label", style: "top:{h as f64 * hour_h}px", {fmt::hour_label(h, s.use_24h_time)} }
                        }
                    }
                    div {
                        class: "ax-cal-days",
                        onresize: move |e: ResizeEvent| {
                            if let Ok(sz) = e.get_border_box_size()
                                && *days_w.peek() != sz.width
                            {
                                days_w.set(sz.width);
                            }
                        },
                        div { class: "ax-cal-lines", "aria-hidden": "true",
                            for (y, class) in slot_lines {
                                div { key: "{y}", class: "{class}", style: "top:{y}px" }
                            }
                        }
                        for (i, d) in days.iter().copied().enumerate() {
                            DayColumn {
                                key: "{d}",
                                idx: i,
                                date: d,
                                items: data.columns.get(i).cloned().unwrap_or_default(),
                                ppm,
                                work: if s.work_days.contains(&d.weekday()) { Some(work) } else { None },
                                h24: s.use_24h_time,
                                col_w,
                            }
                        }
                        NowLine { days: days.clone(), ppm }
                    }
                }
            }
        }
    }
}

#[component]
fn DayHeader(date: NaiveDate, today: bool, single: bool) -> Element {
    let app = use_app();
    let cal = use_cal();
    let full = date.format("%A, %B %-d, %Y").to_string();
    rsx! {
        button {
            r#type: "button",
            class: "ax-cal-dayhead",
            class: if today { "ax-today" },
            role: "columnheader",
            "aria-current": if today { "date" } else { "false" },
            "aria-label": "{full}",
            title: if single { full.clone() } else { format!("Go to {full}") },
            onclick: move |_| {
                let s = app.settings.peek().clone();
                cal.set_view(app, &s, ViewKind::Day(1));
                cal.go_to(&s, date);
            },
            span { class: "ax-cal-dayhead-num", "{date.day()}" }
            span { class: "ax-cal-dayhead-wd", {date.format("%a").to_string()} }
        }
    }
}

/// Collapsed all-day rows (more go behind "+N").
const ALLDAY_ROWS: usize = 3;

#[component]
fn AllDayRow(days: Vec<NaiveDate>, bars: Vec<Bar>, lanes: usize) -> Element {
    let app = use_app();
    let mut cal = use_cal();
    let expanded = (cal.allday_expanded)();
    let n = days.len().max(1);
    let overflow = lanes > ALLDAY_ROWS;
    let shown = if expanded || !overflow { lanes } else { ALLDAY_ROWS };
    // Per-day count of hidden bars.
    let hidden: Vec<usize> = (0..n).map(|i| bars.iter().filter(|b| b.lane >= shown && b.first <= i && b.last >= i).count()).collect();
    let rows = shown + usize::from(hidden.iter().any(|h| *h > 0));
    let height = (rows as f64 * 24.0 + 4.0).max(28.0);
    let selected = (cal.selected)();
    let sel = (cal.slot)();
    let h24 = app.settings.read().use_24h_time;
    // Width of the day area, to tell which day of a bar was grabbed.
    let mut area_w = use_signal(|| 0.0f64);
    let col_w = if area_w() > 0.0 { area_w() / n as f64 } else { 0.0 };
    // An event dragged over the row: where it would land as an all-day bar.
    let drag = (cal.drag)();
    let drag_src = drag.as_ref().filter(|d| d.active && !d.month).and_then(|d| d.occ.as_ref().map(|o| OccKey::of(o)));
    let ghost = drag.as_ref().filter(|d| d.active && d.allday && d.kind == DragKind::Move && !d.month).and_then(|d| {
        let occ = d.occ.as_ref()?;
        let (start, end, all_day) = drag_times(d, &days, (cal.scale)() as i64)?;
        let moved = Occurrence { start, end, ..(**occ).clone() };
        let (first_day, last) = (local_date(start), if all_day || is_long(&moved) { last_day(&moved) } else { local_date(start) });
        let first = days.iter().position(|x| *x >= first_day)?;
        let last = days.iter().rposition(|x| *x <= last)?;
        (first <= last).then(|| {
            let left = first as f64 * 100.0 / n as f64;
            let width = (last - first + 1) as f64 * 100.0 / n as f64;
            let style = format!("{}left:calc({left}% + 2px);width:calc({width}% - 4px);top:2px", event_style(occ, &store().categories().unwrap_or_default()));
            (style, format!("ax-cal-ev ax-cal-bar ax-cal-ghost {}", state_classes(&occ.event, false)), display_title(&occ.event))
        })
    });
    rsx! {
        div { class: "ax-cal-allday", role: "row", style: "height:{height}px",
            div { class: "ax-cal-gutter-head ax-cal-allday-gutter",
                if overflow {
                    button {
                        r#type: "button",
                        class: "ax-cal-allday-toggle",
                        title: if expanded { "Show fewer all-day events" } else { "Show all all-day events" },
                        "aria-expanded": if expanded { "true" } else { "false" },
                        onclick: move |_| cal.allday_expanded.set(!expanded),
                        Icon { name: if expanded { "chevron_up" } else { "chevron_down" }, size: 16 }
                    }
                }
            }
            div {
                class: "ax-cal-allday-area",
                onresize: move |e: ResizeEvent| {
                    if let Ok(sz) = e.get_border_box_size()
                        && *area_w.peek() != sz.width
                    {
                        area_w.set(sz.width);
                    }
                },
                for (i, d) in days.iter().copied().enumerate() {
                    div {
                        key: "{d}",
                        class: "ax-cal-allday-cell",
                        class: if sel.is_some_and(|s| s.all_day && s.covers(d)) { "ax-selected" },
                        "aria-label": format!("{}, all day", d.format("%A, %B %-d")),
                        onmousemove: move |e: MouseEvent| {
                            // Moving an event over the row: it would land here as all-day.
                            let Some(mut dr) = cal.drag.peek().clone() else { return };
                            if dr.month || dr.kind != DragKind::Move {
                                return;
                            }
                            if e.held_buttons().is_empty() {
                                cal.drag.set(None);
                                return;
                            }
                            let c = e.client_coordinates();
                            let was = dr.allday;
                            dr.allday = true;
                            let min = dr.cur_min;
                            if dr.track(i, min, (c.x, c.y)) || !was {
                                cal.drag.set(Some(dr));
                            }
                        },
                        onclick: move |e: MouseEvent| {
                            let (c, off) = (e.client_coordinates(), e.element_coordinates());
                            let sel = SlotSel { start_day: d, end_day: d, start_min: 0, end_min: 0, all_day: true };
                            let anchor = if col_w > 0.0 { Anchor { x: c.x - off.x, y: c.y - off.y, w: col_w, h: 24.0 } } else { Anchor::point(c.x, c.y) };
                            cal.slot.set(Some(sel));
                            cal.selected.set(None);
                            cal.popover.set(Some(Popover::QuickCreate { sel, anchor }));
                        },
                        ondoubleclick: move |_| cal.new_event(app, at_min(d, 0), at_min(d + Duration::days(1), 0), true),
                        if hidden[i] > 0 {
                            button {
                                r#type: "button",
                                class: "ax-cal-allday-more",
                                style: "top:{shown as f64 * 24.0 + 2.0}px",
                                title: "Show all all-day events",
                                onclick: move |e| { e.stop_propagation(); cal.allday_expanded.set(true) },
                                {format!("+{}", hidden[i])}
                            }
                        }
                    }
                }
                for b in bars.iter().filter(|b| b.lane < shown).cloned() {
                    AllDayBar {
                        key: "{b.occ.event.id}-{b.occ.original_start.timestamp()}",
                        selected: selected == Some(OccKey::of(&b.occ)),
                        drag_source: drag_src == Some(OccKey::of(&b.occ)),
                        n,
                        col_w,
                        h24,
                        bar: b,
                    }
                }
                if let Some((style, class, title)) = ghost {
                    div { class: "{class}", style: "{style}", "aria-hidden": "true",
                        span { class: "ax-cal-ev-title", "{title}" }
                    }
                }
            }
        }
    }
}

#[component]
fn AllDayBar(bar: Bar, n: usize, col_w: f64, selected: bool, drag_source: bool, h24: bool) -> Element {
    let app = use_app();
    let mut cal = use_cal();
    let key = OccKey::of(&bar.occ);
    use_drop(move || {
        if let Ok(mut m) = cal.blocks.try_write() {
            m.remove(&key);
        }
    });
    let left = bar.first as f64 * 100.0 / n as f64;
    let width = (bar.last - bar.first + 1) as f64 * 100.0 / n as f64;
    let top = bar.lane as f64 * 24.0 + 2.0;
    let classes = state_classes(&bar.occ.event, bar.invitation);
    let movable = bar.movable;
    let (first, last) = (bar.first, bar.last);
    let occ = bar.occ.clone();
    let occ2 = bar.occ.clone();
    let occ3 = bar.occ.clone();
    let occ4 = bar.occ.clone();
    rsx! {
        div {
            class: "ax-cal-ev ax-cal-bar {classes}",
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
                e.stop_propagation();
                if e.trigger_button() != Some(MouseButton::Primary) {
                    return;
                }
                // A press is a click (peek on release) or the start of a move:
                // along the row by whole days, or onto the grid as a timed event.
                cal.close_popover();
                let c = e.client_coordinates();
                let grabbed = if col_w > 0.0 { first + ((e.element_coordinates().x + 2.0) / col_w) as usize } else { first };
                let day = grabbed.min(last);
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
                    scroll0: *cal.scroll_top.peek(),
                    col_left: 0.0,
                    col_w,
                    movable,
                    month: false,
                    allday: true,
                }));
                if movable {
                    start_auto_scroll((c.x, c.y));
                }
            },
            ondoubleclick: move |e| { e.stop_propagation(); cal.open_occurrence(&occ2) },
            oncontextmenu: move |e: MouseEvent| {
                e.prevent_default();
                e.stop_propagation();
                let c = e.client_coordinates();
                cal.selected.set(Some(key));
                cal.popover.set(Some(Popover::Context { occ: Box::new(occ3.clone()), x: c.x, y: c.y }));
            },
            onfocus: move |_| if *cal.selected.peek() != Some(key) { cal.selected.set(Some(key)) },
            onkeydown: move |e: KeyboardEvent| event_key(app, cal, &occ4, &e),
            if bar.cont_before { Icon { name: "chevron_left", size: 12, class: "ax-cal-cont" } }
            span { class: "ax-cal-ev-title", {display_title(&bar.occ.event)} }
            if !bar.occ.event.location.is_empty() {
                span { class: "ax-cal-ev-loc", {bar.occ.event.location.clone()} }
            }
            EventIcons { event: bar.occ.event.clone() }
            if bar.cont_after { Icon { name: "chevron_right", size: 12, class: "ax-cal-cont" } }
        }
    }
}

/// Keys on a focused event: Enter opens it, Space peeks, Delete deletes,
/// Shift+F10 / Menu opens the context menu, Up/Down move between events.
pub fn event_key(app: AppState, mut cal: Cal, occ: &Occurrence, e: &KeyboardEvent) {
    let chord = crate::kit::keys::chord(e);
    match chord.as_str() {
        "Enter" => {
            e.prevent_default();
            e.stop_propagation();
            cal.open_occurrence(occ);
        }
        "Space" => {
            e.prevent_default();
            e.stop_propagation();
            actions::open_peek(cal, occ.clone(), Anchor::point(-1.0, -1.0));
        }
        "Delete" | "Backspace" => {
            e.prevent_default();
            e.stop_propagation();
            actions::request_delete(app, cal, occ);
        }
        "Shift+F10" | "ContextMenu" => {
            e.prevent_default();
            e.stop_propagation();
            let key = OccKey::of(occ);
            let el = cal.blocks.peek().get(&key).cloned();
            let occ = occ.clone();
            spawn(async move {
                let (x, y) = match el {
                    Some(m) => m.get_client_rect().await.map(|r| (r.min_x() + 8.0, r.max_y())).unwrap_or((200.0, 200.0)),
                    None => (200.0, 200.0),
                };
                cal.popover.set(Some(Popover::Context { occ: Box::new(occ), x, y }));
            });
        }
        "ArrowUp" | "ArrowDown" => {
            e.prevent_default();
            e.stop_propagation();
            let dir = if chord == "ArrowUp" { -1 } else { 1 };
            // Nearest event above / below in the same day (DOM order is by start).
            document::eval(&format!(
                "const el=document.activeElement;const host=el&&el.closest('.ax-cal-col,.ax-cal-month-cell');\
                 if(host){{const all=[...host.querySelectorAll('.ax-cal-ev')];const i=all.indexOf(el);\
                 const next=all[i+({dir})];if(next)next.focus();}}"
            ));
        }
        _ => {}
    }
}

#[component]
fn DayColumn(idx: usize, date: NaiveDate, items: Vec<Placed>, ppm: f64, work: Option<(i64, i64)>, h24: bool, col_w: f64) -> Element {
    let app = use_app();
    let mut cal = use_cal();
    let drag = (cal.drag)();
    let slot = (cal.slot)();
    let selected = (cal.selected)();
    let scale = (cal.scale)() as i64;
    let shading = match work {
        Some((a, b)) => format!(
            "background:linear-gradient(to bottom, var(--ax-bg-2) 0 {}px, var(--ax-bg-1) {}px {}px, var(--ax-bg-2) {}px)",
            a as f64 * ppm,
            a as f64 * ppm,
            b as f64 * ppm,
            b as f64 * ppm
        ),
        None => "background:var(--ax-bg-2)".into(),
    };
    // Selection (slot click / create-drag in progress).
    let creating = drag.as_ref().filter(|d| d.kind == DragKind::Create && d.active && !d.month);
    let sel_here: Option<(i64, i64, bool)> = match creating {
        Some(d) => {
            let s = app.settings.peek().clone();
            let days = cal.days(&s);
            let (a, b) = (d.origin_day.min(d.cur_day), d.origin_day.max(d.cur_day));
            (idx >= a && idx <= b && days.get(idx).is_some()).then(|| {
                let (m0, m1) = (d.origin_min.min(d.cur_min), d.origin_min.max(d.cur_min) + scale);
                (m0, m1, true)
            })
        }
        None => slot.filter(|s| !s.all_day && s.covers(date)).map(|s| (s.start_min, s.end_min, false)),
    };
    // Ghost of an event being moved or resized.
    let ghost = drag.as_ref().filter(|d| d.active && d.kind != DragKind::Create && !d.month && !d.allday).and_then(|d| {
        let occ = d.occ.as_ref()?;
        let s = app.settings.peek().clone();
        let days = cal.days(&s);
        let (start, end, _) = drag_times(d, &days, scale)?;
        let day = if d.kind == DragKind::Resize { d.origin_day } else { d.cur_day };
        (day == idx).then(|| {
            let (a, b) = (minutes_of_day(start), if local_date(end) > date { 24 * 60 } else { minutes_of_day(end) });
            let label = format!("{} {} – {}", date.format("%a"), fmt::time(start, h24), fmt::time(end, h24));
            let style = format!(
                "{}top:{}px;height:{}px;left:1px;width:calc(100% - 10px)",
                event_style(occ, &store().categories().unwrap_or_default()),
                a as f64 * ppm,
                ((b.max(a) - a) as f64 * ppm).max(20.0)
            );
            (style, format!("ax-cal-ev ax-cal-ghost {}", state_classes(&occ.event, false)), display_title(&occ.event), label)
        })
    });
    let drag_src = drag.as_ref().filter(|d| d.active).and_then(|d| d.occ.as_ref().map(|o| OccKey::of(o)));
    let (items, folded) = fold_narrow_clusters(&items, col_w - 8.0);

    rsx! {
        div {
            class: "ax-cal-col",
            role: "gridcell",
            "aria-label": date.format("%A, %B %-d, %Y").to_string(),
            style: "{shading}",
            onmousedown: move |e: MouseEvent| {
                if e.trigger_button() != Some(MouseButton::Primary) {
                    return;
                }
                cal.close_popover();
                let off = e.element_coordinates();
                let c = e.client_coordinates();
                let min = ((off.y / ppm) as i64 / scale) * scale;
                cal.selected.set(None);
                cal.drag.set(Some(Drag {
                    kind: DragKind::Create,
                    occ: None,
                    client: (c.x, c.y),
                    active: false,
                    origin_day: idx,
                    origin_min: min.clamp(0, 24 * 60 - scale),
                    cur_day: idx,
                    cur_min: min.clamp(0, 24 * 60 - scale),
                    col_top: c.y - off.y,
                    scroll0: *cal.scroll_top.peek(),
                    col_left: c.x - off.x,
                    col_w,
                    movable: true,
                    month: false,
                    allday: false,
                }));
                start_auto_scroll((c.x, c.y));
            },
            onmousemove: move |e: MouseEvent| track_pointer(cal, idx, ppm, scale, &e),
            ondoubleclick: move |e: MouseEvent| {
                // A double-click on an event lands here when drag handling made
                // the block transparent to the pointer.
                let last = cal.last_click.peek().clone();
                if let Some((occ, at)) = last.filter(|(_, at)| at.elapsed().as_millis() < 600) {
                    let _ = at;
                    cal.close_popover();
                    cal.open_occurrence(&occ);
                    return;
                }
                let off = e.element_coordinates();
                let min = ((off.y / ppm) as i64 / scale) * scale;
                let start = at_min(date, min);
                let len = app.settings.peek().default_event_minutes.max(5) as i64;
                cal.new_event(app, start, start + Duration::minutes(len), false);
            },
            if let Some((a, b, live)) = sel_here {
                div {
                    class: "ax-cal-slot-sel",
                    style: "top:{a as f64 * ppm}px;height:{(b - a) as f64 * ppm}px",
                    if live || b - a > scale {
                        span { {format!("{} – {}", clock_of(a, h24), clock_of(b, h24))} }
                    }
                }
            }
            for (top, count, k) in folded {
                button {
                    key: "more-{top}",
                    r#type: "button",
                    class: "ax-cal-overlap-more",
                    style: "top:{top as f64 * ppm}px;left:calc((100% - 8px) * {k} / {k + 1} + 1px);width:calc((100% - 8px) / {k + 1} - 2px)",
                    title: if count == 1 { "1 more event".to_string() } else { format!("{count} more events") },
                    onmousedown: move |e| e.stop_propagation(),
                    onclick: move |e: MouseEvent| {
                        let c = e.client_coordinates();
                        cal.popover.set(Some(Popover::DayEvents { date, anchor: Anchor::point(c.x, c.y) }));
                    },
                    {format!("+{count}")}
                }
            }
            for p in items.iter().cloned() {
                TimedEvent {
                    key: "{p.occ.event.id}-{p.occ.original_start.timestamp()}",
                    selected: selected == Some(OccKey::of(&p.occ)),
                    drag_source: drag_src == Some(OccKey::of(&p.occ)),
                    idx,
                    ppm,
                    h24,
                    p,
                }
            }
            if let Some((style, class, title, label)) = ghost {
                div { class: "{class}", style: "{style}", "aria-hidden": "true",
                    span { class: "ax-cal-ev-title", "{title}" }
                    span { class: "ax-cal-ev-time", "{label}" }
                }
            }
        }
    }
}

/// Spec §6.11 step 7: in a cluster too narrow for its columns (blocks
/// under 28px), the events that don't fit fold into one "+N" chip at the
/// top of the cluster. Returns the events still shown (re-laid out over the
/// columns that fit) and per folded cluster (top minute, count, chip column).
fn fold_narrow_clusters(items: &[Placed], avail: f64) -> (Vec<Placed>, Vec<(i64, usize, usize)>) {
    // Top of each cluster that needs folding: cluster -> (top, hidden, chip column).
    let mut folded: Vec<(usize, i64, usize, usize)> = Vec::new();
    for p in items {
        if let Some(k) = visible_columns(p.boxed.cols, avail) {
            let hidden = usize::from(p.boxed.col >= k);
            match folded.iter_mut().find(|f| f.0 == p.boxed.cluster) {
                Some(f) => {
                    f.1 = f.1.min(p.start_min);
                    f.2 += hidden;
                }
                None => folded.push((p.boxed.cluster, p.start_min, hidden, k)),
            }
        }
    }
    if folded.is_empty() {
        return (items.to_vec(), Vec::new());
    }
    let shown = items
        .iter()
        .filter_map(|p| {
            let Some(&(_, _, _, k)) = folded.iter().find(|f| f.0 == p.boxed.cluster) else { return Some(p.clone()) };
            if p.boxed.col >= k {
                return None;
            }
            let mut q = p.clone();
            q.boxed.cols = k + 1;
            q.boxed.span = q.boxed.span.min(k - q.boxed.col);
            Some(q)
        })
        .collect();
    (shown, folded.into_iter().map(|(_, top, n, k)| (top, n, k)).collect())
}

fn clock_of(min: i64, h24: bool) -> String {
    let m = min.clamp(0, 24 * 60);
    let t = if m == 24 * 60 {
        chrono::NaiveTime::MIN
    } else {
        chrono::NaiveTime::from_hms_opt((m / 60) as u32, (m % 60) as u32, 0).unwrap_or(chrono::NaiveTime::MIN)
    };
    clock(t, h24)
}

/// Snap granularity for moves and resizes: the time scale, at least 15 minutes.
fn move_step(scale: i64) -> i64 {
    scale.max(15)
}

/// Pointer moved over day column `idx`: update the drag (writes only when
/// something visible changes).
fn track_pointer(mut cal: Cal, idx: usize, ppm: f64, scale: i64, e: &MouseEvent) {
    let Some(mut d) = cal.drag.peek().clone() else { return };
    if d.month {
        return;
    }
    let c = e.client_coordinates();
    if e.held_buttons().is_empty() {
        // The button was released outside the window.
        cal.drag.set(None);
        return;
    }
    let scrolled = *cal.scroll_top.peek() - d.scroll0;
    let step = move_step(scale);
    let min = match d.kind {
        DragKind::Create => {
            let raw = (c.y - d.col_top + scrolled) / ppm;
            ((raw as i64).clamp(0, 24 * 60 - 1) / scale) * scale
        }
        // An all-day bar dragged onto the grid: the slot under the pointer
        // (`col_top` is unknown for bars, so measure from this column).
        DragKind::Move if d.occ.as_ref().is_some_and(|o| is_long(o)) => {
            let y = e.element_coordinates().y;
            (((y / ppm) as i64 / step) * step).clamp(0, 24 * 60 - DROPPED_ALLDAY_MIN)
        }
        DragKind::Move => {
            let raw = (c.y - d.col_top + scrolled) / ppm;
            d.origin_min + ((raw - d.origin_min as f64) / step as f64).round() as i64 * step
        }
        DragKind::Resize => {
            let raw = (c.y - d.col_top + scrolled) / ppm;
            ((raw / step as f64).round() as i64 * step).clamp(0, 24 * 60)
        }
    };
    let was_allday = d.allday;
    d.allday = false;
    if d.track(idx, min, (c.x, c.y)) || was_allday {
        cal.drag.set(Some(d));
    }
}

/// Length of the timed event an all-day event becomes when dropped on the grid.
const DROPPED_ALLDAY_MIN: i64 = 30;

/// New start / end / all-day for a move or resize drag over the visible
/// `days`. Moves keep the duration; on the all-day row an event becomes
/// all-day for the day under the pointer, and an all-day event dropped on
/// the grid becomes a 30-minute event at the slot under the pointer.
pub fn drag_times(d: &Drag, days: &[NaiveDate], scale: i64) -> Option<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>, bool)> {
    let occ = d.occ.as_ref()?;
    let len = occ.end - occ.start;
    match d.kind {
        DragKind::Move => {
            let day_delta = (*days.get(d.cur_day)? - *days.get(d.origin_day)?).num_days();
            let long = is_long(occ);
            if d.allday {
                if long {
                    let (start, end) = super::month_view::shift_days(occ, day_delta);
                    return Some((start, end, occ.event.all_day));
                }
                let day = local_date(occ.start) + Duration::days(day_delta);
                return Some((at_min(day, 0), at_min(day + Duration::days(1), 0), true));
            }
            if long {
                let start = at_min(*days.get(d.cur_day)?, d.cur_min);
                return Some((start, start + Duration::minutes(DROPPED_ALLDAY_MIN), false));
            }
            let new_date = local_date(occ.start) + Duration::days(day_delta);
            let max_start = (24 * 60 - len.num_minutes().min(24 * 60)).max(0);
            let start_min = (minutes_of_day(occ.start) + d.cur_min - d.origin_min).clamp(0, max_start);
            let start = at_min(new_date, start_min);
            Some((start, start + len, false))
        }
        DragKind::Resize => {
            let day = *days.get(d.origin_day)?;
            let step = move_step(scale);
            let end = at_min(day, d.cur_min).max(occ.start + Duration::minutes(step));
            Some((occ.start, end, false))
        }
        DragKind::Create => None,
    }
}

/// Mouse released anywhere in the module: finish the pending gesture.
pub fn finish_drag(app: AppState, mut cal: Cal, client: (f64, f64)) {
    let Some(d) = cal.drag.peek().clone() else { return };
    cal.drag.set(None);
    if d.month {
        super::month_view::finish_month_drag(app, cal, d, client);
        return;
    }
    let s = app.settings.peek().clone();
    let days = range::visible_days(*cal.view.peek(), *cal.anchor.peek(), s.week_start, &s.work_days);
    let scale = *cal.scale.peek() as i64;
    match d.kind {
        DragKind::Create => {
            let (Some(&a), Some(&b)) = (days.get(d.origin_day.min(d.cur_day)), days.get(d.origin_day.max(d.cur_day))) else { return };
            let (m0, m1) = if d.active {
                (d.origin_min.min(d.cur_min), d.origin_min.max(d.cur_min) + scale)
            } else {
                (d.origin_min, d.origin_min + scale)
            };
            let sel = SlotSel { start_day: a, end_day: b, start_min: m0, end_min: m1.min(24 * 60), all_day: false };
            // Beside the selected slots (Outlook opens the callout next to them).
            let anchor = if d.col_w > 0.0 {
                let first = d.origin_day.min(d.cur_day);
                let span = d.origin_day.max(d.cur_day) - first + 1;
                let ppm = px_per_min(scale as u32);
                let top = d.col_top - (*cal.scroll_top.peek() - d.scroll0) + sel.start_min as f64 * ppm;
                Anchor {
                    x: d.col_left + (first as f64 - d.origin_day as f64) * d.col_w,
                    y: top,
                    w: d.col_w * span as f64,
                    h: (sel.end_min - sel.start_min) as f64 * ppm,
                }
            } else {
                Anchor::point(client.0, client.1)
            };
            cal.slot.set(Some(sel));
            cal.popover_scroll.set(*cal.scroll_top.peek());
            cal.popover.set(Some(Popover::QuickCreate { sel, anchor }));
        }
        DragKind::Move | DragKind::Resize => {
            let Some(occ) = d.occ.as_deref().cloned() else { return };
            if !d.active {
                cal.last_click.set(Some((occ.clone(), Instant::now())));
                actions::open_peek(cal, occ, Anchor::point(client.0, client.1));
                return;
            }
            if let Some((start, end, all_day)) = drag_times(&d, &days, scale) {
                actions::request_move(app, cal, &occ, start, end, all_day);
            }
        }
    }
}

#[component]
fn TimedEvent(p: Placed, idx: usize, ppm: f64, selected: bool, drag_source: bool, h24: bool) -> Element {
    let app = use_app();
    let mut cal = use_cal();
    let key = OccKey::of(&p.occ);
    use_drop(move || {
        if let Ok(mut m) = cal.blocks.try_write() {
            m.remove(&key);
        }
    });
    let top = p.start_min as f64 * ppm;
    let height = (((p.end_min - p.start_min) as f64 * ppm).max(20.0) - 1.0).max(19.0);
    let ColBox { col, span, cols, .. } = p.boxed;
    let cols = cols.max(1);
    let e = &p.occ.event;
    let classes = state_classes(e, p.invitation);
    let title = display_title(e);
    let time = super::text::time_range(p.occ.start, p.occ.end, h24);
    let short = height < 34.0;
    let show_loc = height >= 50.0 && !e.location.trim().is_empty();
    let movable = p.movable;
    let resizable = movable && !p.cont_after;
    let (occ_down, occ_dbl, occ_ctx, occ_key) = (p.occ.clone(), p.occ.clone(), p.occ.clone(), p.occ.clone());
    let start_min = p.start_min;
    rsx! {
        div {
            class: "ax-cal-ev ax-cal-timed {classes}",
            class: if selected { "ax-selected" },
            class: if drag_source { "ax-cal-dragsrc" },
            class: if p.cont_before { "ax-cont-before" },
            class: if p.cont_after { "ax-cont-after" },
            class: if resizable { "ax-resizable" },
            class: if short { "ax-short" },
            role: "button",
            "data-occ": key.dom_id(),
            tabindex: "0",
            "aria-label": aria_label(&p.occ, h24),
            "aria-selected": if selected { "true" } else { "false" },
            style: "{p.style}top:{top}px;height:{height}px;left:calc((100% - 8px) * {col} / {cols} + 1px);width:calc((100% - 8px) * {span} / {cols} - 2px)",
            onmounted: move |ev| { cal.blocks.write().insert(key, ev.data()); },
            onmousedown: move |ev: MouseEvent| {
                if ev.trigger_button() != Some(MouseButton::Primary) {
                    return;
                }
                ev.stop_propagation();
                cal.close_popover();
                let off = ev.element_coordinates();
                let c = ev.client_coordinates();
                let kind = if resizable && off.y >= height - 6.0 { DragKind::Resize } else { DragKind::Move };
                let origin = start_min + (off.y / ppm) as i64;
                cal.drag.set(Some(Drag {
                    kind,
                    occ: Some(Box::new(occ_down.clone())),
                    client: (c.x, c.y),
                    active: false,
                    origin_day: idx,
                    origin_min: if kind == DragKind::Resize { start_min } else { origin },
                    cur_day: idx,
                    cur_min: origin,
                    col_top: c.y - off.y - top,
                    scroll0: *cal.scroll_top.peek(),
                    col_left: 0.0,
                    col_w: 0.0,
                    movable,
                    month: false,
                    allday: false,
                }));
                if movable {
                    start_auto_scroll((c.x, c.y));
                }
            },
            ondoubleclick: move |ev| {
                ev.stop_propagation();
                cal.close_popover();
                cal.open_occurrence(&occ_dbl);
            },
            oncontextmenu: move |ev: MouseEvent| {
                ev.prevent_default();
                ev.stop_propagation();
                let c = ev.client_coordinates();
                cal.selected.set(Some(key));
                cal.popover.set(Some(Popover::Context { occ: Box::new(occ_ctx.clone()), x: c.x, y: c.y }));
            },
            onfocus: move |_| if *cal.selected.peek() != Some(key) { cal.selected.set(Some(key)) },
            onkeydown: move |ev: KeyboardEvent| event_key(app, cal, &occ_key, &ev),
            div { class: "ax-cal-ev-head",
                span { class: "ax-cal-ev-title",
                    "{title}"
                    if short && !e.all_day {
                        span { class: "ax-cal-ev-inline-time", {format!(", {}", fmt::time(p.occ.start, h24))} }
                    }
                }
                EventIcons { event: e.clone() }
            }
            if !short { span { class: "ax-cal-ev-time", "{time}" } }
            if show_loc { span { class: "ax-cal-ev-loc", {e.location.clone()} } }
        }
    }
}

/// Current-time line: red, solid with a dot in today's column, faint across
/// the other days. Re-renders once a minute on its own.
#[component]
fn NowLine(days: Vec<NaiveDate>, ppm: f64) -> Element {
    let cal = use_cal();
    let now = (cal.now)();
    let today = local_date(now);
    let Some(ti) = days.iter().position(|d| *d == today) else { return rsx! {} };
    let n = days.len().max(1);
    let top = minutes_of_day(now) as f64 * ppm;
    rsx! {
        div { class: "ax-cal-now", style: "top:{top}px", "aria-hidden": "true",
            div { class: "ax-cal-now-today", style: "left:calc({ti} * 100% / {n});width:calc(100% / {n})",
                span { class: "ax-cal-now-dot" }
            }
        }
    }
}

/// Visible-day helpers for other modules of the calendar.
pub fn days_peek(app: AppState, cal: Cal) -> Vec<NaiveDate> {
    let s = app.settings.peek().clone();
    range::visible_days(*cal.view.peek(), *cal.anchor.peek(), s.week_start, &s.work_days)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveTime;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
    }

    fn occ(start: chrono::DateTime<chrono::Utc>, end: chrono::DateTime<chrono::Utc>, all_day: bool) -> Occurrence {
        let event = axentra_core::Event { start, end, all_day, ..Default::default() };
        Occurrence { start, end, original_start: start, color: String::new(), event }
    }

    fn drag(o: &Occurrence, origin: (usize, i64), cur: (usize, i64), allday: bool) -> Drag {
        Drag {
            kind: DragKind::Move,
            occ: Some(Box::new(o.clone())),
            client: (0.0, 0.0),
            active: true,
            origin_day: origin.0,
            origin_min: origin.1,
            cur_day: cur.0,
            cur_min: cur.1,
            col_top: 0.0,
            scroll0: 0.0,
            col_left: 0.0,
            col_w: 0.0,
            movable: true,
            month: false,
            allday,
        }
    }

    #[test]
    fn drops_move_convert_and_resize() {
        // Mon 19 – Fri 23.
        let days: Vec<NaiveDate> = (19..=23).map(day).collect();
        let t = |d: u32, h: u32, m: u32| local_to_utc(day(d), NaiveTime::from_hms_opt(h, m, 0).unwrap());

        // A timed event moved one day later and an hour down keeps its length.
        let meeting = occ(t(19, 9, 0), t(19, 10, 30), false);
        let (s, e, all_day) = drag_times(&drag(&meeting, (0, 9 * 60), (1, 10 * 60), false), &days, 30).unwrap();
        assert_eq!((s, e, all_day), (t(20, 10, 0), t(20, 11, 30), false));

        // Dropped on the all-day row it becomes an all-day event on that day.
        let (s, e, all_day) = drag_times(&drag(&meeting, (0, 9 * 60), (2, 9 * 60), true), &days, 30).unwrap();
        assert_eq!((s, e, all_day), (at_min(day(21), 0), at_min(day(22), 0), true));

        // An all-day event moved along the row keeps its span; on the grid it
        // becomes a 30-minute event at the slot under the pointer.
        let offsite = occ(at_min(day(19), 0), at_min(day(21), 0), true);
        let (s, e, all_day) = drag_times(&drag(&offsite, (1, 0), (3, 0), true), &days, 30).unwrap();
        assert_eq!((s, e, all_day), (at_min(day(21), 0), at_min(day(23), 0), true));
        let (s, e, all_day) = drag_times(&drag(&offsite, (0, 0), (4, 14 * 60), false), &days, 30).unwrap();
        assert_eq!((s, e, all_day), (t(23, 14, 0), t(23, 14, 30), false));

        // Resizing never makes an event shorter than one step.
        let mut d = drag(&meeting, (0, 9 * 60), (0, 8 * 60), false);
        d.kind = DragKind::Resize;
        let (s, e, _) = drag_times(&d, &days, 30).unwrap();
        assert_eq!((s, e), (t(19, 9, 0), t(19, 9, 30)));
    }

    #[test]
    fn narrow_clusters_fold_their_overflow() {
        let t = |h: u32, m: u32| local_to_utc(day(19), NaiveTime::from_hms_opt(h, m, 0).unwrap());
        // Five events at once, then a lone one later.
        let mut spans: Vec<(i64, i64)> = (0..5).map(|i| (9 * 60 + i, 10 * 60)).collect();
        spans.push((12 * 60, 13 * 60));
        let boxes = pack_columns(&spans, 30);
        let items: Vec<Placed> = spans
            .iter()
            .zip(boxes)
            .map(|(&(a, b), boxed)| Placed {
                occ: occ(t(9, 0), t(10, 0), false),
                start_min: a,
                end_min: b,
                boxed,
                cont_before: false,
                cont_after: false,
                style: String::new(),
                movable: true,
                invitation: false,
            })
            .collect();
        // Wide enough: nothing folds.
        let (shown, more) = fold_narrow_clusters(&items, 200.0);
        assert_eq!((shown.len(), more.len()), (6, 0));
        // 100px fits three 28px columns: two events stay, "+3" takes the third.
        let (shown, more) = fold_narrow_clusters(&items, 100.0);
        assert_eq!(more, vec![(9 * 60, 3, 2)]);
        assert_eq!(shown.len(), 3);
        assert!(shown.iter().filter(|p| p.start_min < 12 * 60).all(|p| p.boxed.cols == 3 && p.boxed.col < 2));
        // The later event has its own cluster and keeps the full width.
        assert!(shown.iter().any(|p| p.start_min == 12 * 60 && p.boxed.cols == 1));
    }

    #[test]
    fn long_items_go_to_the_all_day_row() {
        let t = |d: u32, h: u32| local_to_utc(day(d), NaiveTime::from_hms_opt(h, 0, 0).unwrap());
        assert!(is_long(&occ(at_min(day(19), 0), at_min(day(20), 0), true)));
        assert!(is_long(&occ(t(19, 9), t(20, 9), false)));
        assert!(!is_long(&occ(t(19, 22), t(20, 2), false)));
        // Exclusive end: a one-day all-day event covers only its own day.
        assert_eq!(last_day(&occ(at_min(day(19), 0), at_min(day(20), 0), true)), day(19));
        assert_eq!(last_day(&occ(t(19, 22), t(20, 2), false)), day(20));
    }
}
