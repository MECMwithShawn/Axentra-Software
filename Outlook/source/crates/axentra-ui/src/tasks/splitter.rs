//! Drag handles that resize the sidebar (220-400 px) and the detail pane
//! (300-520 px), spec 3.2. Widths persist in the To Do preferences.

use std::time::{Duration, Instant};

use dioxus::prelude::*;

use super::ctx::{PaneEdge, Resize, use_todo};

/// Keyboard step of a focused splitter.
const KEY_STEP: f64 = 16.0;
/// Two presses this close together restore the default width.
const DOUBLE_PRESS: Duration = Duration::from_millis(400);

/// The handle on the inner edge of a pane. Drag it, or focus it and use
/// Left / Right (Home / End for the limits); double-click restores the
/// default width.
#[component]
pub fn Splitter(edge: PaneEdge) -> Element {
    let mut ctx = use_todo();
    // Second press of a double-click (the drag layer swallows `dblclick`).
    let mut last_press = use_signal(|| None::<Instant>);
    let width = ctx.pane_width(edge);
    let (min, max) = edge.range();
    let (side, label) = match edge {
        PaneEdge::Sidebar => ("end", "Resize the lists pane"),
        PaneEdge::Detail => ("start", "Resize the details pane"),
    };
    // Moving the pointer right widens the sidebar but narrows the detail pane.
    let grow = match edge {
        PaneEdge::Sidebar => 1.0,
        PaneEdge::Detail => -1.0,
    };
    rsx! {
        div {
            class: "ax-todo-splitter ax-todo-splitter-{side}",
            role: "separator",
            "aria-orientation": "vertical",
            "aria-label": "{label}",
            "aria-valuenow": "{width}",
            "aria-valuemin": "{min}",
            "aria-valuemax": "{max}",
            tabindex: "0",
            title: "{label}",
            onmousedown: move |e: MouseEvent| {
                e.stop_propagation();
                let now = Instant::now();
                if last_press.peek().is_some_and(|t| now.duration_since(t) < DOUBLE_PRESS) {
                    last_press.set(None);
                    ctx.set_pane_width(edge, edge.default_width());
                    return;
                }
                last_press.set(Some(now));
                let x = e.client_coordinates().x;
                ctx.resize.set(Some(Resize { edge, start_x: x, start_width: width, width }));
            },
            onkeydown: move |e: KeyboardEvent| {
                let w = width as f64;
                let next = match e.key() {
                    Key::ArrowLeft => w - KEY_STEP * grow,
                    Key::ArrowRight => w + KEY_STEP * grow,
                    Key::Home => min as f64,
                    Key::End => max as f64,
                    _ => return,
                };
                e.prevent_default();
                e.stop_propagation();
                ctx.set_pane_width(edge, edge.clamp(next));
            },
        }
    }
}

/// While an edge is dragged: a window-wide layer that follows the pointer
/// (so the drag survives leaving the handle) and commits on release.
#[component]
pub fn ResizeLayer() -> Element {
    let mut ctx = use_todo();
    if ctx.resize.read().is_none() {
        return rsx! {};
    }
    let finish = move || {
        let r = *ctx.resize.peek();
        ctx.resize.set(None);
        if let Some(r) = r {
            ctx.set_pane_width(r.edge, r.width);
        }
    };
    let mut finish_up = finish;
    let mut finish_leave = finish;
    rsx! {
        div {
            class: "ax-todo-resize-layer",
            onmousemove: move |e: MouseEvent| {
                let Some(r) = *ctx.resize.peek() else { return };
                let dx = e.client_coordinates().x - r.start_x;
                let w = match r.edge {
                    PaneEdge::Sidebar => r.start_width as f64 + dx,
                    PaneEdge::Detail => r.start_width as f64 - dx,
                };
                let width = r.edge.clamp(w);
                if width != r.width {
                    ctx.resize.set(Some(Resize { width, ..r }));
                }
            },
            onmouseup: move |_| finish_up(),
            onmouseleave: move |_| finish_leave(),
        }
    }
}
