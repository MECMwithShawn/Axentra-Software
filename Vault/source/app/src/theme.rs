//! The Vault theme: deep navy surfaces with electric-cyan accents, matching the vault mark and
//! the unlock screen backdrop.

use eframe::egui::{self, Color32, CornerRadius, Stroke};

pub const ACCENT: Color32 = Color32::from_rgb(0x16, 0x7e, 0xa5);
pub const CYAN: Color32 = Color32::from_rgb(0x5f, 0xe3, 0xff);

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::ThemePreference::Dark);
    let mut v = egui::Visuals::dark();
    v.panel_fill = rgb(0x0a121c);
    v.window_fill = rgb(0x0d1724);
    v.window_stroke = Stroke::new(1.0, Color32::from_rgba_unmultiplied(0x5f, 0xe3, 0xff, 40));
    v.extreme_bg_color = rgb(0x060b12);
    v.faint_bg_color = rgb(0x0e1a28);
    v.code_bg_color = rgb(0x0e1a28);
    v.hyperlink_color = CYAN;
    v.selection.bg_fill = ACCENT;
    v.selection.stroke = Stroke::new(1.0, rgb(0xe7f6ff));

    let radius = CornerRadius::same(6);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = rgb(0x0a121c);
    w.noninteractive.weak_bg_fill = rgb(0x0a121c);
    w.noninteractive.bg_stroke = Stroke::new(1.0, rgb(0x1b2a3a));
    w.noninteractive.fg_stroke = Stroke::new(1.0, rgb(0xa9bccb));
    for (state, fill, stroke) in [
        (
            &mut w.inactive,
            rgb(0x13202f),
            Stroke::new(1.0, rgb(0x1f3346)),
        ),
        (
            &mut w.hovered,
            rgb(0x1a2c40),
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(0x5f, 0xe3, 0xff, 110)),
        ),
        (&mut w.active, ACCENT, Stroke::new(1.0, CYAN)),
        (
            &mut w.open,
            rgb(0x1a2c40),
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(0x5f, 0xe3, 0xff, 80)),
        ),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = stroke;
        state.corner_radius = radius;
    }
    w.inactive.fg_stroke = Stroke::new(1.0, rgb(0xd5e6f2));
    w.hovered.fg_stroke = Stroke::new(1.5, rgb(0xf2fbff));
    w.active.fg_stroke = Stroke::new(1.5, Color32::WHITE);
    w.noninteractive.corner_radius = radius;
    v.window_corner_radius = CornerRadius::same(12);
    v.menu_corner_radius = CornerRadius::same(8);
    ctx.set_visuals_of(egui::Theme::Dark, v);
}
