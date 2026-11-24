//! Floating Modular chrome: one frameless, transparent OS window in which opaque "modules"
//! (title bar, navigation, list, details, security) are painted as detached glass panels with
//! chamfered corner accents and a cyan edge glow. Includes the floating title bar with drag,
//! double-click maximise and hand-made window controls, and edge resize grips.
//!
//! Transparency is only cosmetic: the OS window stays rectangular, so clicks in the gaps land on
//! the app, not the desktop. Where transparency is unavailable (Remote Desktop) or unwanted
//! (Windows high contrast), `opaque_backdrop()` fills the window instead of leaving black gaps.

use eframe::egui::{
    self, Color32, CursorIcon, Id, Pos2, Rect, ResizeDirection, Sense, Shape, Stroke,
    ViewportCommand, pos2, vec2,
};

use crate::theme::CYAN;

// Design tokens for modules.
pub const GAP: f32 = 14.0;
pub const PAD: f32 = 16.0;
pub const TOP_BAR_H: f32 = 64.0;
const CUT: f32 = 14.0;
const BRACKET: f32 = 22.0;
pub const MODULE_FILL: Color32 = Color32::from_rgb(0x0a, 0x13, 0x1e);
const BACKDROP: Color32 = Color32::from_rgb(0x05, 0x09, 0x11);

pub fn cyan(a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(
        CYAN.r(),
        CYAN.g(),
        CYAN.b(),
        (a.clamp(0.0, 1.0) * 255.0) as u8,
    )
}

/// Whether the desktop may show through the gaps between modules.
#[cfg(windows)]
pub fn transparency_available() -> bool {
    use windows_sys::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_REMOTESESSION, SPI_GETHIGHCONTRAST, SystemParametersInfoW,
    };
    unsafe {
        if GetSystemMetrics(SM_REMOTESESSION) != 0 {
            return false;
        }
        let mut hc = HIGHCONTRASTW {
            cbSize: std::mem::size_of::<HIGHCONTRASTW>() as u32,
            dwFlags: 0,
            lpszDefaultScheme: std::ptr::null_mut(),
        };
        let ok = SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            hc.cbSize,
            (&mut hc as *mut HIGHCONTRASTW).cast(),
            0,
        );
        !(ok != 0 && hc.dwFlags & HCF_HIGHCONTRASTON != 0)
    }
}

/// Solid window background for environments without transparency.
pub fn opaque_backdrop(ui: &egui::Ui, rect: Rect) {
    ui.painter().rect_filled(rect, 0.0, BACKDROP);
}

/// Panel outline with chamfered top-right and bottom-left corners.
fn outline(r: Rect, cut: f32) -> Vec<Pos2> {
    vec![
        pos2(r.left(), r.top()),
        pos2(r.right() - cut, r.top()),
        pos2(r.right(), r.top() + cut),
        pos2(r.right(), r.bottom()),
        pos2(r.left() + cut, r.bottom()),
        pos2(r.left(), r.bottom() - cut),
    ]
}

/// Paints a module: soft outer glow, opaque glass body, faint edge, and bright corner brackets.
pub fn module(painter: &egui::Painter, r: Rect) {
    module_body(painter, r, CUT);
    module_edges(painter, r, CUT);
}

/// Glow, opaque body and top sheen. Content goes on top, then `module_edges`.
pub fn module_body(painter: &egui::Painter, r: Rect, cut: f32) {
    let pts = outline(r, cut);
    for (w, a) in [(12.0, 0.025), (6.0, 0.05)] {
        painter.add(Shape::closed_line(pts.clone(), Stroke::new(w, cyan(a))));
    }
    painter.add(Shape::convex_polygon(pts, MODULE_FILL, Stroke::NONE));
    let sheen = Rect::from_min_max(r.min, pos2(r.right() - cut, r.top() + 46.0));
    let mut mesh = egui::Mesh::default();
    let top = Color32::from_rgba_unmultiplied(0x5f, 0xe3, 0xff, 10);
    let clear = Color32::from_rgba_unmultiplied(0x5f, 0xe3, 0xff, 0);
    mesh.colored_vertex(sheen.left_top(), top);
    mesh.colored_vertex(sheen.right_top(), top);
    mesh.colored_vertex(sheen.right_bottom(), clear);
    mesh.colored_vertex(sheen.left_bottom(), clear);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(Shape::mesh(mesh));
}

/// Faint outline, bright corner brackets and lit chamfers, drawn over the module's content.
pub fn module_edges(painter: &egui::Painter, r: Rect, cut: f32) {
    painter.add(Shape::closed_line(
        outline(r, cut),
        Stroke::new(1.0, cyan(0.26)),
    ));
    let b = Stroke::new(2.0, cyan(0.85));
    painter.line_segment([r.left_top() + vec2(0.0, BRACKET), r.left_top()], b);
    painter.line_segment([r.left_top(), r.left_top() + vec2(BRACKET, 0.0)], b);
    painter.line_segment([r.right_bottom() - vec2(0.0, BRACKET), r.right_bottom()], b);
    painter.line_segment([r.right_bottom(), r.right_bottom() - vec2(BRACKET, 0.0)], b);
    if cut > 0.0 {
        painter.line_segment(
            [
                pos2(r.right() - cut, r.top()),
                pos2(r.right(), r.top() + cut),
            ],
            Stroke::new(1.6, cyan(0.7)),
        );
        painter.line_segment(
            [
                pos2(r.left() + cut, r.bottom()),
                pos2(r.left(), r.bottom() - cut),
            ],
            Stroke::new(1.6, cyan(0.7)),
        );
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Ctl {
    Minimize,
    Maximize,
    Close,
}

/// The floating title bar: mark, name, tagline, a centre slot (search), window controls.
/// Dragging empty space moves the window; double-click maximises.
pub fn top_bar(ui: &mut egui::Ui, r: Rect, center: impl FnOnce(&mut egui::Ui)) {
    let ctx = ui.ctx().clone();
    let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
    let painter = ui.painter().clone();
    module(&painter, r);

    let bar = ui.interact(r, Id::new("title_bar"), Sense::click_and_drag());
    if bar.drag_started_by(egui::PointerButton::Primary) {
        ctx.send_viewport_cmd(ViewportCommand::StartDrag);
    }
    if bar.double_clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
    }

    // Brand.
    let mark = Rect::from_center_size(pos2(r.left() + 36.0, r.center().y), vec2(42.0, 42.0));
    egui::Image::new(egui::include_image!("../assets/app/vault-128.png")).paint_at(ui, mark);
    painter.text(
        pos2(mark.right() + 12.0, r.center().y - 8.0),
        egui::Align2::LEFT_CENTER,
        "Axentra Vault",
        egui::FontId::proportional(20.0),
        Color32::from_rgb(0xe7, 0xf6, 0xff),
    );
    let mut job = egui::text::LayoutJob::default();
    job.append(
        "SECURE  ·  PRIVATE  ·  ALWAYS YOURS",
        0.0,
        egui::TextFormat {
            font_id: egui::FontId::proportional(8.5),
            extra_letter_spacing: 1.6,
            color: Color32::from_rgb(0x7f, 0xbb, 0xcc),
            ..Default::default()
        },
    );
    let galley = painter.layout_job(job);
    let brand_right = (mark.right() + 13.0 + galley.size().x).max(mark.right() + 150.0);
    painter.galley(
        pos2(mark.right() + 13.0, r.center().y + 7.0),
        galley,
        Color32::WHITE,
    );

    // Window controls.
    let x = window_controls(
        ui,
        r.right() - 14.0,
        r.center().y,
        vec2(44.0, 32.0),
        &[Ctl::Close, Ctl::Maximize, Ctl::Minimize],
        false,
    );

    // Centre slot between the brand and the controls.
    let slot = Rect::from_min_max(
        pos2(brand_right + 28.0, r.top() + 14.0),
        pos2(x - 24.0, r.bottom() - 14.0),
    );
    if slot.width() > 120.0 {
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(slot)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            center,
        );
    }
}

/// Draws window buttons right-to-left ending at `right`, vertically centred on `cy`, and acts on
/// clicks. `subtle` dims them until hovered (used on the compact unlock card). Returns the left
/// edge of the leftmost button.
pub fn window_controls(
    ui: &mut egui::Ui,
    right: f32,
    cy: f32,
    btn: egui::Vec2,
    kinds: &[Ctl],
    subtle: bool,
) -> f32 {
    let ctx = ui.ctx().clone();
    let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
    let painter = ui.painter().clone();
    let mut x = right;
    for &kind in kinds {
        x -= btn.x;
        let rect = Rect::from_min_size(pos2(x, cy - btn.y / 2.0), btn);
        let resp = ui.interact(rect, Id::new(("window_ctl", kind as u8)), Sense::click());
        let name = match kind {
            Ctl::Minimize => "Minimise",
            Ctl::Maximize if maximized => "Restore",
            Ctl::Maximize => "Maximise",
            Ctl::Close => "Close",
        };
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name));
        let hot = resp.hovered();
        if hot {
            let bg = if kind == Ctl::Close {
                Color32::from_rgb(0xc4, 0x2b, 0x2b)
            } else {
                cyan(0.14)
            };
            painter.rect_filled(rect.shrink(2.0), 6.0, bg);
        }
        let ink = match (kind, hot, subtle) {
            (_, true, _) => Color32::WHITE,
            (_, false, true) => Color32::from_rgba_unmultiplied(0xa9, 0xc8, 0xd8, 120),
            (Ctl::Close, false, false) => Color32::from_rgb(0xff, 0x5a, 0x5a),
            _ => Color32::from_rgb(0xa9, 0xc8, 0xd8),
        };
        let c = rect.center();
        let s = Stroke::new(if subtle { 1.3 } else { 1.5 }, ink);
        let k = if subtle { 0.8 } else { 1.0 };
        match kind {
            Ctl::Minimize => {
                painter.line_segment([c + vec2(-7.0 * k, 0.0), c + vec2(7.0 * k, 0.0)], s);
            }
            Ctl::Maximize if maximized => {
                painter.rect_stroke(
                    Rect::from_center_size(c + vec2(2.0, -2.0), vec2(10.0, 10.0)),
                    1.0,
                    s,
                    egui::StrokeKind::Middle,
                );
                painter.rect_filled(
                    Rect::from_center_size(c + vec2(-2.0, 2.0), vec2(10.0, 10.0)),
                    1.0,
                    MODULE_FILL,
                );
                painter.rect_stroke(
                    Rect::from_center_size(c + vec2(-2.0, 2.0), vec2(10.0, 10.0)),
                    1.0,
                    s,
                    egui::StrokeKind::Middle,
                );
            }
            Ctl::Maximize => {
                painter.rect_stroke(
                    Rect::from_center_size(c, vec2(12.0, 12.0)),
                    1.0,
                    s,
                    egui::StrokeKind::Middle,
                );
            }
            Ctl::Close => {
                painter.line_segment([c + vec2(-6.0, -6.0) * k, c + vec2(6.0, 6.0) * k], s);
                painter.line_segment([c + vec2(6.0, -6.0) * k, c + vec2(-6.0, 6.0) * k], s);
            }
        }
        if resp.on_hover_text(name).clicked() {
            ctx.send_viewport_cmd(match kind {
                Ctl::Minimize => ViewportCommand::Minimized(true),
                Ctl::Maximize => ViewportCommand::Maximized(!maximized),
                Ctl::Close => ViewportCommand::Close,
            });
        }
    }
    x
}

/// Invisible strips along the window edges and corners that start an OS resize when dragged.
pub fn resize_grips(ui: &mut egui::Ui, r: Rect) {
    use ResizeDirection::*;
    if ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false)) {
        return;
    }
    let g = 8.0;
    let zones = [
        (
            Rect::from_min_max(
                pos2(r.left() + g, r.top()),
                pos2(r.right() - g, r.top() + 5.0),
            ),
            North,
            CursorIcon::ResizeVertical,
        ),
        (
            Rect::from_min_max(
                pos2(r.left() + g, r.bottom() - 5.0),
                pos2(r.right() - g, r.bottom()),
            ),
            South,
            CursorIcon::ResizeVertical,
        ),
        (
            Rect::from_min_max(
                pos2(r.left(), r.top() + g),
                pos2(r.left() + 5.0, r.bottom() - g),
            ),
            West,
            CursorIcon::ResizeHorizontal,
        ),
        (
            Rect::from_min_max(
                pos2(r.right() - 5.0, r.top() + g),
                pos2(r.right(), r.bottom() - g),
            ),
            East,
            CursorIcon::ResizeHorizontal,
        ),
        (
            Rect::from_min_size(r.left_top(), vec2(g, g)),
            NorthWest,
            CursorIcon::ResizeNwSe,
        ),
        (
            Rect::from_min_size(pos2(r.right() - g, r.top()), vec2(g, g)),
            NorthEast,
            CursorIcon::ResizeNeSw,
        ),
        (
            Rect::from_min_size(pos2(r.left(), r.bottom() - g), vec2(g, g)),
            SouthWest,
            CursorIcon::ResizeNeSw,
        ),
        (
            Rect::from_min_size(pos2(r.right() - g, r.bottom() - g), vec2(g, g)),
            SouthEast,
            CursorIcon::ResizeNwSe,
        ),
    ];
    // Panels are separate child UIs: their hit regions cover parent widgets.
    // Read the outer border directly and begin the native resize on button-down.
    let Some(point) = ui.ctx().input(|i| i.pointer.hover_pos()) else { return; };
    // The window buttons sit flush with the top edge; their tops belong to them, not to resizing.
    let controls = Rect::from_min_max(pos2(r.right() - 132.0, r.top()), pos2(r.right() - g, r.top() + 32.0));
    if controls.contains(point) {
        return;
    }
    for (rect, dir, cursor) in zones {
        if rect.contains(point) {
            ui.ctx().set_cursor_icon(cursor);
            if ui.ctx().input(|i| i.pointer.button_pressed(egui::PointerButton::Primary)) {
                ui.ctx().send_viewport_cmd(ViewportCommand::BeginResize(dir));
            }
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_outline_is_convex() {
        let pts = outline(Rect::from_min_size(Pos2::ZERO, vec2(300.0, 200.0)), CUT);
        for i in 0..pts.len() {
            let (a, b, c) = (pts[i], pts[(i + 1) % pts.len()], pts[(i + 2) % pts.len()]);
            let cross = (b - a).x * (c - b).y - (b - a).y * (c - b).x;
            assert!(cross >= 0.0, "turn {i} is not convex");
        }
    }
}

#[cfg(target_os="linux")]
pub fn transparency_available()->bool {
    static AVAILABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *AVAILABLE.get_or_init(|| !crate::linux::desktop_setting("org.gnome.desktop.interface", "gtk-theme").contains("HighContrast"))
}
