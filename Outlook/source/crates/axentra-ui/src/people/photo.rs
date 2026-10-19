//! The crop step of "Upload a photo" (docs/spec/people-todo.md 2.6.4):
//! drag to reposition inside a circle mask, zoom with the slider, "Apply".

use dioxus::prelude::*;

use super::io::{self, PickedImage};
use crate::kit::{Dialog, keys};
use crate::state::use_app;

/// Side of the crop viewport (CSS px).
pub const CROP_VIEW: f64 = 280.0;
const MAX_ZOOM: f64 = 3.0;
/// Arrow-key nudge (px) and zoom step for the keyboard and the wheel.
const NUDGE: f64 = 10.0;
const ZOOM_STEP: f64 = 0.1;

/// Zoom and offset of the image inside the viewport. At zoom 1 the image's
/// short side fills the viewport; the offset moves its centre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Crop {
    pub zoom: f64,
    pub dx: f64,
    pub dy: f64,
}

impl Default for Crop {
    fn default() -> Self {
        Self { zoom: 1.0, dx: 0.0, dy: 0.0 }
    }
}

impl Crop {
    /// Image pixels to viewport pixels.
    pub fn scale(&self, w: f64, h: f64) -> f64 {
        CROP_VIEW / w.min(h).max(1.0) * self.zoom
    }

    /// The same crop with zoom in range and the image still covering the
    /// whole viewport (no empty corners in the circle).
    pub fn clamped(self, w: f64, h: f64) -> Crop {
        let zoom = self.zoom.clamp(1.0, MAX_ZOOM);
        let s = CROP_VIEW / w.min(h).max(1.0) * zoom;
        let mx = ((w * s - CROP_VIEW) / 2.0).max(0.0);
        let my = ((h * s - CROP_VIEW) / 2.0).max(0.0);
        Crop { zoom, dx: self.dx.clamp(-mx, mx), dy: self.dy.clamp(-my, my) }
    }

    /// The visible square in image pixels: (x, y, side).
    pub fn source_rect(&self, w: f64, h: f64) -> (f64, f64, f64) {
        let s = self.scale(w, h);
        let side = CROP_VIEW / s;
        let cx = w / 2.0 - self.dx / s;
        let cy = h / 2.0 - self.dy / s;
        (cx - side / 2.0, cy - side / 2.0, side)
    }
}

#[component]
pub fn PhotoCropDialog(image: PickedImage, on_apply: EventHandler<String>, on_cancel: EventHandler<()>) -> Element {
    let app = use_app();
    let (w, h) = (image.width, image.height);
    let mut crop = use_signal(Crop::default);
    let mut drag = use_signal(|| None::<(f64, f64, Crop)>);
    let mut busy = use_signal(|| false);
    let c = crop();
    let s = c.scale(w, h);
    let (iw, ih) = (w * s, h * s);
    let left = (CROP_VIEW - iw) / 2.0 + c.dx;
    let top = (CROP_VIEW - ih) / 2.0 + c.dy;
    let zoom_pct = (c.zoom * 100.0).round();
    let mut set = move |next: Crop| crop.set(next.clamped(w, h));
    let url = image.url.clone();
    let apply = move |_| {
        if busy() {
            return;
        }
        busy.set(true);
        let url = url.clone();
        let rect = crop.peek().source_rect(w, h);
        spawn(async move {
            match io::crop_photo(&url, rect).await {
                Some(photo) => on_apply.call(photo),
                None => {
                    busy.set(false);
                    app.error("That image couldn't be read. Try a JPEG, PNG or WebP file.");
                }
            }
        });
    };
    rsx! {
        Dialog {
            title: "Crop photo",
            width: 360,
            class: "ax-people-crop-dialog",
            on_close: move |_| on_cancel.call(()),
            actions: rsx! {
                button { r#type: "button", class: "ax-btn ax-primary", disabled: busy(), onclick: apply, "Apply" }
                button { r#type: "button", class: "ax-btn", onclick: move |_| on_cancel.call(()), "Cancel" }
            },
            div {
                class: "ax-people-crop",
                // Keys stay in the crop step (the form below has Ctrl+S / Esc).
                onkeydown: move |e: KeyboardEvent| {
                    if e.key() != Key::Escape {
                        e.stop_propagation();
                    }
                },
                div {
                    class: "ax-people-crop-view",
                    class: if drag().is_some() { "ax-dragging" },
                    style: "width:{CROP_VIEW}px;height:{CROP_VIEW}px",
                    tabindex: "0",
                    role: "img",
                    "aria-label": "Photo position. Drag or use the arrow keys to move it, plus and minus to zoom.",
                    // No preventDefault: the click also focuses the view for the keys.
                    onmousedown: move |e: MouseEvent| {
                        let p = e.client_coordinates();
                        drag.set(Some((p.x, p.y, crop())));
                    },
                    onwheel: move |e: WheelEvent| {
                        let dy = e.delta().strip_units().y;
                        let c = crop();
                        set(Crop { zoom: c.zoom + if dy < 0.0 { ZOOM_STEP } else { -ZOOM_STEP }, ..c });
                    },
                    onkeydown: move |e: KeyboardEvent| {
                        let c = crop();
                        let next = match keys::chord(&e).as_str() {
                            "ArrowLeft" => Crop { dx: c.dx - NUDGE, ..c },
                            "ArrowRight" => Crop { dx: c.dx + NUDGE, ..c },
                            "ArrowUp" => Crop { dy: c.dy - NUDGE, ..c },
                            "ArrowDown" => Crop { dy: c.dy + NUDGE, ..c },
                            "+" | "Shift++" | "=" => Crop { zoom: c.zoom + ZOOM_STEP, ..c },
                            "-" => Crop { zoom: c.zoom - ZOOM_STEP, ..c },
                            _ => return,
                        };
                        e.prevent_default();
                        set(next);
                    },
                    img {
                        class: "ax-people-crop-img",
                        src: "{image.url}",
                        alt: "",
                        draggable: "false",
                        style: "width:{iw}px;height:{ih}px;left:{left}px;top:{top}px",
                    }
                    div { class: "ax-people-crop-mask", "aria-hidden": "true" }
                }
                label { class: "ax-people-crop-zoom",
                    span { class: "ax-field-label", "Zoom" }
                    input {
                        r#type: "range",
                        min: "100",
                        max: "{MAX_ZOOM * 100.0}",
                        step: "1",
                        value: "{zoom_pct}",
                        "aria-label": "Zoom",
                        "aria-valuetext": "{zoom_pct}%",
                        oninput: move |e| {
                            let z = e.value().parse::<f64>().unwrap_or(100.0) / 100.0;
                            set(Crop { zoom: z, ..crop() });
                        },
                        onkeydown: move |e| keys::stop_typing_keys(&e),
                    }
                }
            }
        }
        if let Some((x0, y0, start)) = drag() {
            div {
                class: "ax-people-drag-overlay ax-people-crop-drag",
                onmousemove: move |e: MouseEvent| {
                    let p = e.client_coordinates();
                    set(Crop { dx: start.dx + p.x - x0, dy: start.dy + p.y - y0, ..start });
                },
                onmouseup: move |_| drag.set(None),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_crop_is_the_centre_square() {
        let c = Crop::default();
        assert_eq!(c.source_rect(800.0, 600.0), (100.0, 0.0, 600.0));
        assert_eq!(c.source_rect(600.0, 900.0), (0.0, 150.0, 600.0));
    }

    #[test]
    fn offsets_and_zoom_map_to_image_pixels() {
        // 800 x 600 at zoom 2: scale 280 / 600 * 2; the visible side is 300 px.
        let c = Crop { zoom: 2.0, dx: 70.0, dy: 0.0 }.clamped(800.0, 600.0);
        let (x, y, side) = c.source_rect(800.0, 600.0);
        assert!((side - 300.0).abs() < 1e-9);
        // Moving the image right by 70 px shows 75 image px further left.
        assert!((x - (400.0 - 75.0 - 150.0)).abs() < 1e-9);
        assert!((y - 150.0).abs() < 1e-9);
    }

    #[test]
    fn clamping_keeps_the_circle_covered() {
        let c = Crop { zoom: 9.0, dx: 1e6, dy: -1e6 }.clamped(800.0, 600.0);
        assert_eq!(c.zoom, MAX_ZOOM);
        let (x, y, side) = c.source_rect(800.0, 600.0);
        assert!(x.abs() < 1e-9, "left edge reached, not passed");
        assert!((y + side - 600.0).abs() < 1e-9, "bottom edge reached, not passed");
        let small = Crop { zoom: 0.5, dx: 5.0, dy: 5.0 }.clamped(600.0, 600.0);
        assert_eq!(small, Crop { zoom: 1.0, dx: 0.0, dy: 0.0 });
    }
}
