//! My Day > Calendar > Day: a one-day timeline, 48 px per hour, with a
//! now-line and the current time scrolled into view.

use axentra_core::*;
use chrono::{DateTime, Local, NaiveDate, Timelike, Utc};
use dioxus::prelude::*;

use crate::kit::dom;

const HOUR_PX: f64 = 48.0;

/// Side-by-side layout of overlapping events: (lane, lanes in its cluster)
/// for each input event, in input order. Events must be sorted by start.
pub fn lanes(spans: &[(i64, i64)]) -> Vec<(usize, usize)> {
    let mut out = vec![(0, 1); spans.len()];
    let mut cluster: Vec<usize> = Vec::new();
    let mut lane_ends: Vec<i64> = Vec::new();
    let mut cluster_end = i64::MIN;
    let flush = |cluster: &mut Vec<usize>, lane_ends: &mut Vec<i64>, out: &mut Vec<(usize, usize)>| {
        let n = lane_ends.len().max(1);
        for &i in cluster.iter() {
            out[i].1 = n;
        }
        cluster.clear();
        lane_ends.clear();
    };
    for (i, &(start, end)) in spans.iter().enumerate() {
        if start >= cluster_end && !cluster.is_empty() {
            flush(&mut cluster, &mut lane_ends, &mut out);
        }
        let end = end.max(start + 1);
        let lane = match lane_ends.iter().position(|&e| e <= start) {
            Some(l) => {
                lane_ends[l] = end;
                l
            }
            None => {
                lane_ends.push(end);
                lane_ends.len() - 1
            }
        };
        out[i].0 = lane;
        cluster.push(i);
        cluster_end = cluster_end.max(end);
    }
    flush(&mut cluster, &mut lane_ends, &mut out);
    out
}

fn minutes_into(day: NaiveDate, t: DateTime<Utc>) -> i64 {
    let l = t.with_timezone(&Local);
    if l.date_naive() < day {
        0
    } else if l.date_naive() > day {
        24 * 60
    } else {
        (l.hour() * 60 + l.minute()) as i64
    }
}

#[component]
pub fn DayView(day: NaiveDate, occurrences: Vec<Occurrence>, now: DateTime<Local>, h24: bool, on_open: EventHandler<Occurrence>) -> Element {
    let id = dom::use_unique_id("ax-myday-timeline");
    let all_day: Vec<Occurrence> = occurrences.iter().filter(|o| o.event.all_day).cloned().collect();
    let mut timed: Vec<Occurrence> = occurrences.iter().filter(|o| !o.event.all_day && o.event.my_response != ResponseStatus::Declined).cloned().collect();
    timed.sort_by_key(|o| o.start);
    let spans: Vec<(i64, i64)> = timed.iter().map(|o| (minutes_into(day, o.start), minutes_into(day, o.end))).collect();
    let layout = lanes(&spans);
    // (event, position style, too short for a second line)
    let placed: Vec<(Occurrence, String, bool)> = timed
        .iter()
        .cloned()
        .enumerate()
        .map(|(i, o)| {
            let ((start, end), (lane, n)) = (spans[i], layout[i]);
            let top = start as f64 * HOUR_PX / 60.0;
            let height = ((end - start) as f64 * HOUR_PX / 60.0).max(20.0);
            let width = 100.0 / n as f64;
            let style = format!(
                "top:{top}px;height:{height}px;left:calc({}% + 2px);width:calc({width}% - 4px);--ax-event-color:{}",
                lane as f64 * width,
                o.color
            );
            (o, style, height < 36.0)
        })
        .collect();
    let is_today = day == now.date_naive();
    let now_px = (now.hour() * 60 + now.minute()) as f64 * HOUR_PX / 60.0;
    let scroll_to = if is_today { (now_px - HOUR_PX).max(0.0) } else { 8.0 * HOUR_PX };
    rsx! {
        if !all_day.is_empty() {
            div { class: "ax-dayview-allday",
                for o in all_day {
                    button {
                        key: "{o.event.id}",
                        r#type: "button",
                        class: "ax-dayview-event ax-allday",
                        style: "--ax-event-color:{o.color}",
                        onclick: {
                            let o = o.clone();
                            move |_| on_open.call(o.clone())
                        },
                        span { class: "ax-ellipsis", "{o.event.title}" }
                    }
                }
            }
        }
        div {
            id: "{id}",
            class: "ax-dayview",
            onmounted: {
                let id = id.clone();
                move |_| dom::run(&format!("(function(){{var e=document.getElementById({});if(e)e.scrollTop={scroll_to};}})()", dom::js_str(&id)))
            },
            div { class: "ax-dayview-grid", style: "height:{24.0 * HOUR_PX}px",
                for h in 0..24u32 {
                    div { key: "h{h}", class: "ax-dayview-hour", style: "top:{h as f64 * HOUR_PX}px",
                        span { class: "ax-dayview-label", if h > 0 { "{crate::fmt::hour_label(h, h24)}" } }
                    }
                }
                for (o, style, short) in placed.iter().cloned() {
                    button {
                        key: "{o.event.id}-{o.start.timestamp()}",
                        r#type: "button",
                        class: "ax-dayview-event",
                        class: if short { "ax-short" },
                        "aria-label": "{o.event.title}, {super::agenda::time_range(&o, h24)}",
                        style: "{style}",
                        onclick: {
                            let o = o.clone();
                            move |_| on_open.call(o.clone())
                        },
                        span { class: "ax-ellipsis ax-semibold", "{o.event.title}" }
                        if !short {
                            span { class: "ax-ellipsis ax-caption", "{super::agenda::time_range(&o, h24)}" }
                        }
                    }
                }
                if is_today {
                    div { class: "ax-dayview-now", style: "top:{now_px}px", "aria-hidden": "true" }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlapping_events_share_the_width() {
        // 9:00-10:00, 9:30-10:30, 11:00-11:30
        let l = lanes(&[(540, 600), (570, 630), (660, 690)]);
        assert_eq!(l, vec![(0, 2), (1, 2), (0, 1)]);
        // Back-to-back events reuse the lane.
        assert_eq!(lanes(&[(540, 600), (600, 660)]), vec![(0, 1), (0, 1)]);
        assert!(lanes(&[]).is_empty());
    }
}
