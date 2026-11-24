//! Window actions go through egui so close guards and platform behaviour remain intact.
use eframe::egui::{self, Align, Color32, Layout, Rect, Sense, Stroke, ViewportCommand, vec2};

pub fn buttons(ui: &mut egui::Ui) {
    let maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
    let mut boxes = serde_json::Map::new();
    // Call inside a right-to-left layout.
    for (kind, label) in [(0, "Close"), (1, if maximized { "Restore" } else { "Maximize" }), (2, "Minimize")] {
        let (rect, response) = ui.allocate_exact_size(vec2(36.0, 30.0), Sense::click());
        boxes.insert(label.to_owned(), serde_json::json!({"x":rect.min.x,"y":rect.min.y,"width":rect.width(),"height":rect.height()}));
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
        let ink = if response.hovered() { Color32::WHITE } else { ui.visuals().text_color() };
        if response.hovered() {
            ui.painter().rect_filled(rect.shrink(1.0), 4.0, if kind == 0 { Color32::from_rgb(196,43,54) } else { ui.visuals().widgets.hovered.bg_fill });
        }
        let c = rect.center(); let stroke = Stroke::new(1.3, ink);
        match kind {
            0 => { ui.painter().line_segment([c+vec2(-5.,-5.),c+vec2(5.,5.)],stroke); ui.painter().line_segment([c+vec2(5.,-5.),c+vec2(-5.,5.)],stroke); }
            1 => {
                let r = Rect::from_center_size(c, vec2(10.,10.));
                if maximized { ui.painter().line_segment([r.left_top()+vec2(2.,-2.),r.right_top()+vec2(2.,-2.)],stroke); ui.painter().line_segment([r.right_top()+vec2(2.,-2.),r.right_bottom()+vec2(2.,-2.)],stroke); }
                ui.painter().rect_stroke(r,0.,stroke,egui::StrokeKind::Middle);
            }
            _ => { ui.painter().line_segment([c+vec2(-5.,0.),c+vec2(5.,0.)],stroke); }
        }
        if response.clicked() {
            ui.ctx().send_viewport_cmd(match kind { 0 => ViewportCommand::Close, 1 => ViewportCommand::Maximized(!maximized), _ => ViewportCommand::Minimized(true) });
        }
        response.on_hover_text(label);
    }
    if let Some(path) = std::env::var_os("AXENTRA_CHROME_REPORT") {
        let path = std::path::PathBuf::from(path); let temp = path.with_extension("tmp");
        let value = serde_json::json!({"maximized":maximized,"controls":boxes});
        if std::fs::write(&temp, value.to_string()).is_ok() { let _ = std::fs::rename(temp,path); }
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
    }
}
pub fn drag(ui: &mut egui::Ui, rect: Rect) {
    let response = ui.interact(rect, ui.id().with("window_drag"), Sense::click_and_drag());
    if response.drag_started_by(egui::PointerButton::Primary) { ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag); }
    if response.double_clicked() { let max = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false)); ui.ctx().send_viewport_cmd(ViewportCommand::Maximized(!max)); }
}
pub fn header(ui: &mut egui::Ui, label: &str) {
    ui.horizontal(|ui| {
        let rect = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width()-124.,32.));
        drag(ui, rect);
        ui.label(label);
        ui.with_layout(Layout::right_to_left(Align::Center), buttons);
    });
}

use eframe::egui::{ResizeDirection, CursorIcon, Id, pos2};
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
