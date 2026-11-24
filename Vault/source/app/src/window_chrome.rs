//! Window actions go through egui so close guards and platform behaviour remain intact.
use eframe::egui::{self, Align, Color32, Layout, Rect, Sense, Stroke, ViewportCommand, vec2};

pub fn buttons(ui: &mut egui::Ui) {
    let maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
    let mut boxes = serde_json::Map::new();
    // Call inside a right-to-left layout.
    for (kind, label) in [
        (0, "Close"),
        (1, if maximized { "Restore" } else { "Maximize" }),
        (2, "Minimize"),
    ] {
        let (rect, response) = ui.allocate_exact_size(vec2(36.0, 30.0), Sense::click());
        boxes.insert(label.to_owned(), serde_json::json!({"x":rect.min.x,"y":rect.min.y,"width":rect.width(),"height":rect.height()}));
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
        let ink = if response.hovered() {
            Color32::WHITE
        } else {
            ui.visuals().text_color()
        };
        if response.hovered() {
            ui.painter().rect_filled(
                rect.shrink(1.0),
                4.0,
                if kind == 0 {
                    Color32::from_rgb(196, 43, 54)
                } else {
                    ui.visuals().widgets.hovered.bg_fill
                },
            );
        }
        let c = rect.center();
        let stroke = Stroke::new(1.3, ink);
        match kind {
            0 => {
                ui.painter()
                    .line_segment([c + vec2(-5., -5.), c + vec2(5., 5.)], stroke);
                ui.painter()
                    .line_segment([c + vec2(5., -5.), c + vec2(-5., 5.)], stroke);
            }
            1 => {
                let r = Rect::from_center_size(c, vec2(10., 10.));
                if maximized {
                    ui.painter().line_segment(
                        [r.left_top() + vec2(2., -2.), r.right_top() + vec2(2., -2.)],
                        stroke,
                    );
                    ui.painter().line_segment(
                        [
                            r.right_top() + vec2(2., -2.),
                            r.right_bottom() + vec2(2., -2.),
                        ],
                        stroke,
                    );
                }
                ui.painter()
                    .rect_stroke(r, 0., stroke, egui::StrokeKind::Middle);
            }
            _ => {
                ui.painter()
                    .line_segment([c + vec2(-5., 0.), c + vec2(5., 0.)], stroke);
            }
        }
        if response.clicked() {
            ui.ctx().send_viewport_cmd(match kind {
                0 => ViewportCommand::Close,
                1 => ViewportCommand::Maximized(!maximized),
                _ => ViewportCommand::Minimized(true),
            });
        }
        response.on_hover_text(label);
    }
    if let Some(path) = std::env::var_os("AXENTRA_CHROME_REPORT") {
        let path = std::path::PathBuf::from(path);
        let temp = path.with_extension("tmp");
        let value = serde_json::json!({"maximized":maximized,"controls":boxes});
        if std::fs::write(&temp, value.to_string()).is_ok() {
            let _ = std::fs::rename(temp, path);
        }
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));
    }
}
pub fn drag(ui: &mut egui::Ui, rect: Rect) {
    let response = ui.interact(rect, ui.id().with("window_drag"), Sense::click_and_drag());
    if response.drag_started_by(egui::PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }
    if response.double_clicked() {
        let max = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
        ui.ctx().send_viewport_cmd(ViewportCommand::Maximized(!max));
    }
}
pub fn header(ui: &mut egui::Ui, label: &str) {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            vec2((ui.available_width() - 132.).max(0.), 32.),
            Layout::left_to_right(Align::Center),
            |ui| {
                drag(ui, ui.max_rect());
                ui.add(egui::Label::new(label).truncate());
            },
        );
        ui.with_layout(Layout::right_to_left(Align::Center), buttons);
    });
}
