//! Draw tab: a drawing canvas with pen, highlighter and eraser; the
//! drawing goes into the body as a picture (mail spec 6.4, "Draw").

use dioxus::prelude::*;
use serde::Deserialize;
use serde_json::json;

use super::state::Form;
use crate::kit::{Button, Dialog, IconButton, Variant};

const SCRIPT: &str = include_str!("draw.js");

/// Ink colours (user content colours, not UI colours).
pub const PEN_COLORS: &[(&str, &str)] = &[
    ("#000000", "Black"),
    ("#C00000", "Dark red"),
    ("#FF0000", "Red"),
    ("#0070C0", "Blue"),
    ("#00B050", "Green"),
    ("#7030A0", "Purple"),
];
pub const HIGHLIGHTER_COLORS: &[(&str, &str)] = &[("#FFFF00", "Yellow"), ("#00FF00", "Bright green"), ("#00FFFF", "Turquoise"), ("#FF00FF", "Pink")];

/// Stroke widths in px: (label, width).
const WIDTHS: &[(&str, u32)] = &[("Thin", 2), ("Medium", 4), ("Thick", 8)];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Pen,
    Highlighter,
    Eraser,
}

impl Tool {
    pub fn parse(s: &str) -> Tool {
        match s {
            "highlighter" => Tool::Highlighter,
            "eraser" => Tool::Eraser,
            _ => Tool::Pen,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Tool::Pen => "pen",
            Tool::Highlighter => "highlighter",
            Tool::Eraser => "eraser",
        }
    }
}

#[derive(Deserialize)]
struct Picture {
    src: String,
    width: f64,
    height: f64,
}

#[derive(Deserialize)]
struct Count {
    /// Pen and highlighter strokes (something to insert).
    n: usize,
    /// Every stroke, eraser included (something to undo).
    all: usize,
}

/// The canvas dialog, opened with a tool and colour from the Draw tab.
#[component]
pub fn DrawDialog(tool: Tool, color: String) -> Element {
    let mut form = use_context::<Form>();
    let canvas_id = form.dom("draw");
    let mut tool = use_signal(|| tool);
    let mut color = use_signal(|| color);
    let mut width = use_signal(|| 4u32);
    let mut strokes = use_signal(|| (0usize, 0usize));

    // The canvas script lives as long as the dialog; it reports the stroke count.
    use_hook({
        let canvas_id = canvas_id.clone();
        move || {
            spawn(async move {
                let cfg = json!({ "id": canvas_id });
                let mut eval = document::eval(&SCRIPT.replacen("/*AX_CONFIG*/null", &cfg.to_string(), 1));
                while let Ok(c) = eval.recv::<Count>().await {
                    strokes.set((c.n, c.all));
                }
            });
        }
    });
    let call = {
        let canvas_id = canvas_id.clone();
        move |js: String| {
            document::eval(&format!(
                "const d = window.axDraw && window.axDraw[{}]; if (d) {{ {js} }}",
                serde_json::Value::String(canvas_id.clone())
            ));
        }
    };
    // Keep the script's pen in step with the toolbar.
    use_effect({
        let call = call.clone();
        move || {
            let pen = json!({ "tool": tool().key(), "color": color(), "width": width() });
            call(format!("d.set({pen});"));
        }
    });

    let insert = {
        let canvas_id = canvas_id.clone();
        move |_| {
            let id = canvas_id.clone();
            form.spawn(async move {
                let js = format!(
                    "const d = window.axDraw && window.axDraw[{}]; return d ? d.data() : null;",
                    serde_json::Value::String(id)
                );
                if let Ok(Some(p)) = document::eval(&js).join::<Option<Picture>>().await {
                    form.dialog.set(None);
                    form.exec("image", json!({ "src": p.src, "alt": "Drawing", "width": p.width.round(), "height": p.height.round() }));
                }
            });
        }
    };
    let palette: &[(&str, &str)] = if tool() == Tool::Highlighter { HIGHLIGHTER_COLORS } else { PEN_COLORS };
    let mut close = move || {
        form.dialog.set(None);
        form.exec("focus", serde_json::Value::Null);
    };
    let (ink, all) = strokes();
    let undo = {
        let call = call.clone();
        move |_| call("d.undo();".into())
    };
    let clear = move |_| call("d.clear();".into());
    rsx! {
        Dialog {
            title: "Drawing canvas",
            width: 640,
            on_close: move |_| close(),
            actions: rsx! {
                Button { label: "Insert", variant: Variant::Primary, disabled: ink == 0, title: if ink == 0 { "Draw something first" } else { "Insert the drawing into the message" }, onclick: insert }
                Button { label: "Cancel", onclick: move |_| close() }
            },
            div { class: "ax-compose-draw-tools", role: "toolbar", "aria-label": "Drawing tools",
                IconButton { icon: "inking_tool", title: "Pen", toggled: tool() == Tool::Pen, onclick: move |_| { tool.set(Tool::Pen); if !PEN_COLORS.iter().any(|(c, _)| *c == color()) { color.set(PEN_COLORS[0].0.into()); } } }
                IconButton { icon: "highlight", title: "Highlighter", toggled: tool() == Tool::Highlighter, onclick: move |_| { tool.set(Tool::Highlighter); if !HIGHLIGHTER_COLORS.iter().any(|(c, _)| *c == color()) { color.set(HIGHLIGHTER_COLORS[0].0.into()); } } }
                IconButton { icon: "eraser", title: "Eraser", toggled: tool() == Tool::Eraser, onclick: move |_| tool.set(Tool::Eraser) }
                div { class: "ax-ribbon-sep" }
                for (hex, name) in palette.iter() {
                    button {
                        key: "{hex}",
                        r#type: "button",
                        class: "ax-compose-swatch ax-compose-ink",
                        class: if color() == *hex && tool() != Tool::Eraser { "ax-selected" },
                        style: "background:{hex}",
                        title: "{name}",
                        "aria-label": "{name}",
                        "aria-pressed": if color() == *hex { "true" } else { "false" },
                        disabled: tool() == Tool::Eraser,
                        onclick: move |_| color.set(hex.to_string()),
                    }
                }
                div { class: "ax-ribbon-sep" }
                for (label, w) in WIDTHS.iter() {
                    button {
                        key: "{label}",
                        r#type: "button",
                        class: "ax-compose-ink-width",
                        class: if width() == *w { "ax-selected" },
                        title: "{label}",
                        "aria-label": "{label}",
                        "aria-pressed": if width() == *w { "true" } else { "false" },
                        onclick: move |_| width.set(*w),
                        span { style: "height:{w}px" }
                    }
                }
                div { class: "ax-grow" }
                IconButton { icon: "arrow_undo", title: "Undo", disabled: all == 0, onclick: undo }
                IconButton { icon: "delete", title: "Clear", disabled: all == 0, onclick: clear }
            }
            canvas { id: "{canvas_id}", class: "ax-compose-canvas", "aria-label": "Drawing area" }
        }
    }
}
