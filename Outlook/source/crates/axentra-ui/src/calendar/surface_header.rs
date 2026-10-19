//! Surface header: Today, previous / next, and the range title with its
//! go-to-date flyout (also Ctrl+G).

use dioxus::prelude::*;

use super::mini_month::MiniMonth;
use super::range::{self, ViewKind, first_of_month};
use super::state::use_cal;
use crate::kit::{Icon, IconButton};
use crate::state::use_app;

#[component]
pub fn SurfaceHeader() -> Element {
    let app = use_app();
    let mut cal = use_cal();
    let s = app.settings.read().clone();
    let view = (cal.view)();
    let anchor = (cal.anchor)();
    let days = cal.days(&s);
    let title = range::range_title(view, anchor, &days);
    let unit = view.unit();
    let flyout = (cal.date_flyout)();
    let mut flyout_month = use_signal(|| first_of_month(anchor));
    // Opening the flyout (click or Ctrl+G) starts at the shown month.
    use_effect(move || {
        if (cal.date_flyout)() {
            flyout_month.set(first_of_month(*cal.anchor.peek()));
        }
    });
    let today = (cal.today)();
    rsx! {
        div { class: "ax-cal-surface-head", role: "toolbar", "aria-label": "Calendar navigation",
            button {
                r#type: "button",
                class: "ax-btn ax-cal-today-btn",
                title: "Go to today (Alt+Shift+Y)",
                onclick: move |_| {
                    let s = app.settings.peek().clone();
                    cal.go_today(&s);
                },
                Icon { name: "calendar_today" }
                span { "Today" }
            }
            IconButton {
                icon: "chevron_left",
                title: "Go to previous {unit}",
                onclick: move |_| {
                    let s = app.settings.peek().clone();
                    cal.step(&s, -1);
                },
            }
            IconButton {
                icon: "chevron_right",
                title: "Go to next {unit}",
                onclick: move |_| {
                    let s = app.settings.peek().clone();
                    cal.step(&s, 1);
                },
            }
            div { class: "ax-cal-title-wrap",
                button {
                    r#type: "button",
                    class: "ax-cal-title",
                    "aria-haspopup": "dialog",
                    "aria-expanded": if flyout { "true" } else { "false" },
                    title: "Go to a date (Ctrl+G)",
                    onclick: move |_| {
                        flyout_month.set(first_of_month(*cal.anchor.peek()));
                        cal.date_flyout.set(!flyout);
                    },
                    span { "aria-live": "polite", "{title}" }
                    Icon { name: "chevron_down", size: 16 }
                }
                if flyout {
                    div { class: "ax-menu-backdrop", onclick: move |_| cal.date_flyout.set(false) }
                    div {
                        class: "ax-cal-flyout",
                        role: "dialog",
                        "aria-label": "Go to date",
                        onmousedown: move |e| e.stop_propagation(),
                        onkeydown: move |e: KeyboardEvent| {
                            if e.key() == Key::Escape {
                                e.stop_propagation();
                                cal.date_flyout.set(false);
                                super::actions::restore_focus(cal);
                            }
                        },
                        MiniMonth {
                            month: flyout_month(),
                            on_month: move |m| flyout_month.set(m),
                            on_pick: move |d| {
                                let s = app.settings.peek().clone();
                                cal.go_to(&s, d);
                            },
                            today,
                            week_start: s.week_start,
                            selected: Some(anchor),
                            range: if view == ViewKind::Month { Vec::new() } else { days.clone() },
                        }
                        button {
                            r#type: "button",
                            class: "ax-cal-flyout-today",
                            onclick: move |_| {
                                let s = app.settings.peek().clone();
                                cal.go_today(&s);
                            },
                            "Today"
                        }
                    }
                }
            }
        }
    }
}
