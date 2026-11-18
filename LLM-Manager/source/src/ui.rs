//! The window: its own title strip, a toolbar, the Models list and Resources.
//!
//! The look follows Lab Manager (`labmanager.py`): the same palette, card and pill
//! metrics, glass state tiles and compact rows, so the two apps read as one family.

use std::hash::Hash;
use std::sync::Arc;

use crate::telemetry::{Engine, Gpu, Model, RankEntry, Resident, Snapshot, ollama_address};
use eframe::egui::{
    self, Align, Align2, Color32, ColorImage, Context, CursorIcon, FontData, FontDefinitions,
    FontFamily, FontId, Frame, Id, Label, LayerId, Layout, Margin, Mesh, Order, PointerButton,
    Pos2, Rect, ResizeDirection, Response, RichText, Sense, Shape, Stroke, StrokeKind,
    TextureFilter, TextureHandle, TextureOptions, Ui, UiBuilder, Vec2, ViewportCommand, WidgetInfo,
    WidgetType, pos2, vec2,
};

/// Samples kept for the activity chart (two seconds apart).
pub const HISTORY: usize = 45;
const TILE_SIZE: usize = 96;

// Lab Manager's palette.
const CANVAS: Color32 = Color32::from_rgb(12, 18, 27);
const SURFACE: Color32 = Color32::from_rgb(19, 28, 40);
const SURFACE_HOVER: Color32 = Color32::from_rgb(25, 36, 50);
const LINE: Color32 = Color32::from_rgb(40, 54, 72);
const INK: Color32 = Color32::from_rgb(233, 239, 247);
const SOFT: Color32 = Color32::from_rgb(184, 198, 216);
const MUTED: Color32 = Color32::from_rgb(161, 177, 196);
const DIM: Color32 = Color32::from_rgb(134, 154, 177);
const ACCENT: Color32 = Color32::from_rgb(89, 201, 239);
const ACCENT_FILL: Color32 = Color32::from_rgb(24, 50, 68);
const ACCENT_FILL_HOVER: Color32 = Color32::from_rgb(35, 67, 87);
const GREEN: Color32 = Color32::from_rgb(138, 204, 156);
const GREEN_BASE: Color32 = Color32::from_rgb(108, 203, 95);
const AMBER: Color32 = Color32::from_rgb(226, 184, 106);
const AMBER_BASE: Color32 = Color32::from_rgb(252, 225, 0);
const RED: Color32 = Color32::from_rgb(255, 153, 164);
const OTHER: Color32 = Color32::from_rgb(114, 134, 158);
const CHART: Color32 = Color32::from_rgb(16, 17, 25);
const CHART_LINE: Color32 = Color32::from_rgb(40, 39, 52);
const CLOSE_HOVER: Color32 = Color32::from_rgb(196, 43, 28);

const TITLE_HEIGHT: f32 = 46.0;
const CONTROL_WIDTH: f32 = 46.0;
const APP_ICON_SIZE: f32 = 42.0;
const ROW_HEIGHT: f32 = 54.0;
const METRICS_WIDTH: f32 = 310.0;
const ACTION_WIDTH: f32 = 98.0;
const RESIZE_MARGIN: f32 = 5.0;
const SEMIBOLD: &str = "semibold";

fn white(alpha: f32) -> Color32 {
    Color32::from_white_alpha((alpha * 255.0).round() as u8)
}

fn regular(size: f32) -> FontId {
    FontId::proportional(size)
}

fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD.into()))
}

fn line_height(ui: &Ui, font: &FontId) -> f32 {
    ui.fonts_mut(|fonts| fonts.row_height(font))
}

/// Inter is the desktop's UI font (and what Lab Manager shows through GTK). It is read
/// from the system at start; egui's bundled font is the fallback.
fn install_fonts(ctx: &Context) {
    const FOLDERS: [&str; 4] = [
        "/usr/share/fonts/truetype/inter-zorin-os",
        "/usr/share/fonts/truetype/inter",
        "/usr/share/fonts/opentype/inter",
        "/usr/share/fonts/inter",
    ];
    let read = |stem: &str| {
        FOLDERS.iter().find_map(|folder| {
            ["ttf", "otf"]
                .iter()
                .find_map(|kind| std::fs::read(format!("{folder}/{stem}.{kind}")).ok())
        })
    };
    let mut fonts = FontDefinitions::default();
    let mut strong = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    if let Some(bytes) = read("Inter-Regular") {
        fonts
            .font_data
            .insert("inter".to_owned(), Arc::new(FontData::from_owned(bytes)));
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .insert(0, "inter".to_owned());
        strong.insert(0, "inter".to_owned());
    }
    if let Some(bytes) = read("Inter-SemiBold") {
        fonts.font_data.insert(
            "inter-semibold".to_owned(),
            Arc::new(FontData::from_owned(bytes)),
        );
        strong.insert(0, "inter-semibold".to_owned());
    }
    fonts
        .families
        .insert(FontFamily::Name(SEMIBOLD.into()), strong);
    ctx.set_fonts(fonts);
}

pub fn configure(ctx: &Context) {
    install_fonts(ctx);
    ctx.set_theme(egui::Theme::Dark);
    ctx.global_style_mut(|style| {
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = CANVAS;
        visuals.window_fill = SURFACE_HOVER;
        visuals.window_stroke = Stroke::new(1.0, white(0.08));
        visuals.window_corner_radius = 8.into();
        visuals.extreme_bg_color = CANVAS;
        visuals.override_text_color = Some(INK);
        visuals.weak_text_color = Some(MUTED);
        visuals.selection.bg_fill = ACCENT_FILL;
        visuals.selection.stroke = Stroke::new(1.0, ACCENT);
        style.visuals = visuals;
        style.interaction.selectable_labels = false;
        style.spacing.item_spacing = Vec2::ZERO;
        style
            .text_styles
            .insert(egui::TextStyle::Body, regular(13.0));
    });
}

/// The state tiles (rendered by `assets/render-tiles.py`) and the app icon.
struct Art {
    loaded: TextureHandle,
    busy: TextureHandle,
    idle: TextureHandle,
    gpu: TextureHandle,
    mark: TextureHandle,
}

impl Art {
    fn load(ctx: &Context, icon: &egui::IconData) -> Self {
        let options = TextureOptions {
            mipmap_mode: Some(TextureFilter::Linear),
            ..TextureOptions::LINEAR
        };
        let texture = |name: &str, side: usize, rgba: &[u8]| {
            let image = ColorImage::from_rgba_unmultiplied([side, side], rgba);
            ctx.load_texture(name, image, options)
        };
        Self {
            loaded: texture(
                "tile-loaded",
                TILE_SIZE,
                include_bytes!("../assets/tile-loaded.rgba"),
            ),
            busy: texture(
                "tile-busy",
                TILE_SIZE,
                include_bytes!("../assets/tile-busy.rgba"),
            ),
            idle: texture(
                "tile-idle",
                TILE_SIZE,
                include_bytes!("../assets/tile-idle.rgba"),
            ),
            gpu: texture(
                "tile-gpu",
                TILE_SIZE,
                include_bytes!("../assets/tile-gpu.rgba"),
            ),
            // The same decoded PNG the window and launcher use.
            mark: ctx.load_texture(
                "app-icon",
                ColorImage::from_rgba_unmultiplied(
                    [icon.width as usize, icon.height as usize],
                    &icon.rgba,
                ),
                options,
            ),
        }
    }
}

fn image(ui: &Ui, texture: &TextureHandle, rect: Rect) {
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    ui.painter().image(texture.id(), rect, uv, Color32::WHITE);
}

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Refresh,
    SetResident { model: String, load: bool },
    OpenPanel,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Tab {
    #[default]
    Models,
    Resources,
}

pub struct View {
    tab: Tab,
    art: Art,
}

impl View {
    pub fn new(ctx: &Context, icon: &egui::IconData) -> Self {
        Self {
            tab: Tab::default(),
            art: Art::load(ctx, icon),
        }
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        snapshot: Option<&Snapshot>,
        pending: Option<(&str, bool)>,
        message: Option<(&str, bool)>,
        gpu_util_history: &[f32],
    ) -> Option<Action> {
        let ctx = ui.ctx().clone();
        let window = ui.max_rect();
        let maximized = ctx.input(|input| input.viewport().maximized.unwrap_or(false));
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        ui.painter().rect_filled(window, 0, CANVAS);
        let art = &self.art;

        title_bar(ui, art, snapshot, maximized);
        let mut action = toolbar(ui, &mut self.tab);

        let scroll = egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .id_salt(match self.tab {
                Tab::Models => "models",
                Tab::Resources => "resources",
            });
        let tab = self.tab;
        scroll.show(ui, |ui| {
            let side = if tab == Tab::Models { 20 } else { 24 };
            Frame::new()
                .inner_margin(Margin {
                    left: side,
                    right: side,
                    top: 4,
                    bottom: 20,
                })
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    match tab {
                        Tab::Models => {
                            if let Some(row_action) = models(ui, art, snapshot, pending) {
                                action = Some(row_action);
                            }
                        }
                        Tab::Resources => resources(ui, art, snapshot, gpu_util_history),
                    }
                });
        });

        toast(&ctx, pending, message);
        if !maximized {
            window_frame(&ctx, window);
        }
        action
    }
}

// ---------------------------------------------------------------------------
// Window chrome

#[derive(Clone, Copy)]
enum Control {
    Minimize,
    Maximize,
    Close,
}

/// The app draws its own title strip (the window has no system header): identity at
/// the left, window controls at the right, and the rest moves the window.
fn title_bar(ui: &mut Ui, art: &Art, snapshot: Option<&Snapshot>, maximized: bool) {
    let ctx = ui.ctx().clone();
    let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), TITLE_HEIGHT), Sense::hover());
    let painter = ui.painter().clone();
    painter.rect_filled(bar, 0, SURFACE);
    painter.hline(
        bar.x_range(),
        bar.bottom() - 0.5,
        Stroke::new(1.0, white(0.06)),
    );

    let mut right = bar.right();
    for control in [Control::Close, Control::Maximize, Control::Minimize] {
        let rect = Rect::from_min_max(
            pos2(right - CONTROL_WIDTH, bar.top()),
            pos2(right, bar.bottom() - 1.0),
        );
        right -= CONTROL_WIDTH;
        let (name, id) = match control {
            Control::Minimize => ("Minimize", "window-minimize"),
            Control::Maximize if maximized => ("Restore", "window-maximize"),
            Control::Maximize => ("Maximize", "window-maximize"),
            Control::Close => ("Close", "window-close"),
        };
        // Clickable, but left out of the Tab order: the desktop's own shortcuts cover these.
        let response = ui.interact(rect, Id::new(id), Sense::CLICK);
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, name));
        let hot = response.hovered();
        if hot {
            let fill = match control {
                Control::Close => CLOSE_HOVER,
                _ => white(0.08),
            };
            painter.rect_filled(rect, 0, fill);
        }
        let ink = Stroke::new(1.0, if hot { Color32::WHITE } else { SOFT });
        let c = rect.center();
        match control {
            Control::Minimize => {
                painter.hline(c.x - 5.0..=c.x + 5.0, c.y + 0.5, ink);
            }
            Control::Maximize if maximized => {
                let back = Rect::from_min_size(pos2(c.x - 2.5, c.y - 5.5), Vec2::splat(8.0));
                let front = Rect::from_min_size(pos2(c.x - 5.5, c.y - 2.5), Vec2::splat(8.0));
                painter.rect_stroke(back, 1, ink, StrokeKind::Inside);
                let cover = if hot { white(0.0) } else { SURFACE };
                painter.rect(front, 1, cover, ink, StrokeKind::Inside);
            }
            Control::Maximize => {
                let square = Rect::from_center_size(c, Vec2::splat(10.0));
                painter.rect_stroke(square, 1, ink, StrokeKind::Inside);
            }
            Control::Close => {
                painter.line_segment([c + vec2(-5.0, -5.0), c + vec2(5.0, 5.0)], ink);
                painter.line_segment([c + vec2(5.0, -5.0), c + vec2(-5.0, 5.0)], ink);
            }
        }
        if response.clicked() {
            ctx.send_viewport_cmd(match control {
                Control::Minimize => ViewportCommand::Minimized(true),
                Control::Maximize => ViewportCommand::Maximized(!maximized),
                Control::Close => ViewportCommand::Close,
            });
        }
    }

    let handle = Rect::from_min_max(bar.min, pos2(right, bar.bottom()));
    let drag = ui.interact(handle, Id::new("window-move"), Sense::CLICK | Sense::DRAG);
    if drag.double_clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
    } else if drag.drag_started_by(PointerButton::Primary) {
        ctx.send_viewport_cmd(ViewportCommand::StartDrag);
    }

    let mark = Rect::from_center_size(
        pos2(bar.left() + 10.0 + APP_ICON_SIZE / 2.0, bar.center().y),
        Vec2::splat(APP_ICON_SIZE),
    );
    image(ui, &art.mark, mark);

    let (status, dot) = match snapshot.map(|sample| &sample.engine) {
        Some(Ok(engine)) => (engine_summary(engine), GREEN),
        Some(Err(_)) => (format!("Ollama unavailable  ·  {}", ollama_address()), RED),
        None => ("Connecting to local Ollama…".to_owned(), AMBER),
    };
    let title_font = semibold(14.0);
    let status_font = regular(11.0);
    let title_height = line_height(ui, &title_font);
    let status_height = line_height(ui, &status_font);
    let left = mark.right() + 10.0;
    let top = bar.center().y - (title_height + 1.0 + status_height) / 2.0;
    let text_width = (right - 12.0 - left).max(0.0);
    text_at(
        ui,
        "window-title",
        Rect::from_min_size(pos2(left, top), vec2(text_width, title_height)),
        "LLM Manager",
        title_font,
        INK,
        Align::Min,
    );
    let status_top = top + title_height + 1.0;
    painter.circle_filled(pos2(left + 3.0, status_top + status_height / 2.0), 3.0, dot);
    text_at(
        ui,
        "window-status",
        Rect::from_min_size(
            pos2(left + 11.0, status_top),
            vec2((text_width - 11.0).max(0.0), status_height),
        ),
        &status,
        status_font,
        MUTED,
        Align::Min,
    );
}

fn engine_summary(engine: &Engine) -> String {
    let mut text = match &engine.version {
        Some(version) => format!("Ollama {version}  ·  {}", ollama_address()),
        None => format!("Ollama  ·  {}", ollama_address()),
    };
    let installed = engine.models.len().max(engine.residents.len());
    text.push_str(&format!(
        "  ·  {} of {installed} loaded",
        engine.residents.len()
    ));
    if !engine.residents.is_empty() {
        text.push_str(&format!(
            "  ·  {} model VRAM",
            format_bytes(resident_vram(engine))
        ));
    }
    text
}

fn resident_vram(engine: &Engine) -> u64 {
    engine
        .residents
        .iter()
        .fold(0u64, |sum, model| sum.saturating_add(model.size_vram))
}

/// A thin outline, plus edges and corners that resize the undecorated window.
fn window_frame(ctx: &Context, window: Rect) {
    ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("window-outline")))
        .rect_stroke(window, 0, Stroke::new(1.0, LINE), StrokeKind::Inside);

    let Some(pointer) = ctx.pointer_hover_pos() else {
        return;
    };
    let near = |edge: f32, at: f32| (edge - at).abs() <= RESIZE_MARGIN;
    let (west, east) = (
        near(window.left(), pointer.x),
        near(window.right(), pointer.x),
    );
    let (north, south) = (
        near(window.top(), pointer.y),
        near(window.bottom(), pointer.y),
    );
    let (direction, cursor) = match (north, south, west, east) {
        (true, _, true, _) => (ResizeDirection::NorthWest, CursorIcon::ResizeNorthWest),
        (true, _, _, true) => (ResizeDirection::NorthEast, CursorIcon::ResizeNorthEast),
        (_, true, true, _) => (ResizeDirection::SouthWest, CursorIcon::ResizeSouthWest),
        (_, true, _, true) => (ResizeDirection::SouthEast, CursorIcon::ResizeSouthEast),
        (true, ..) => (ResizeDirection::North, CursorIcon::ResizeNorth),
        (_, true, ..) => (ResizeDirection::South, CursorIcon::ResizeSouth),
        (_, _, true, _) => (ResizeDirection::West, CursorIcon::ResizeWest),
        (_, _, _, true) => (ResizeDirection::East, CursorIcon::ResizeEast),
        _ => return,
    };
    ctx.set_cursor_icon(cursor);
    if ctx.input(|input| input.pointer.primary_pressed()) {
        ctx.send_viewport_cmd(ViewportCommand::BeginResize(direction));
    }
}

fn toolbar(ui: &mut Ui, tab: &mut Tab) -> Option<Action> {
    let mut action = None;
    let frame = Frame::new()
        .fill(SURFACE)
        .inner_margin(Margin::symmetric(20, 8))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                Frame::new()
                    .fill(CANVAS)
                    .corner_radius(7)
                    .inner_margin(Margin::same(3))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            for (target, label) in
                                [(Tab::Models, "Models"), (Tab::Resources, "Resources")]
                            {
                                if button(ui, label, Skin::Tab(*tab == target), true).clicked() {
                                    *tab = target;
                                }
                            }
                        });
                    });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if button(ui, "Open Offline Panel", Skin::Subtle, true)
                        .on_hover_text("Open the local chat at 127.0.0.1:8766.")
                        .clicked()
                    {
                        action = Some(Action::OpenPanel);
                    }
                    ui.add_space(8.0);
                    if button(ui, "Refresh", Skin::Subtle, true)
                        .on_hover_text("Request a fresh Ollama and GPU sample.")
                        .clicked()
                    {
                        action = Some(Action::Refresh);
                    }
                });
            });
        });
    let rect = frame.response.rect;
    ui.painter()
        .hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, LINE));
    action
}

// ---------------------------------------------------------------------------
// Building blocks

#[derive(Clone, Copy)]
enum Skin {
    /// Quiet toolbar button.
    Subtle,
    /// A row's main action, tinted with the accent.
    Tint,
    /// One segment of the view switcher.
    Tab(bool),
}

#[derive(Clone, Copy)]
enum Glyph {
    Play,
    Power,
}

fn button(ui: &mut Ui, label: &str, skin: Skin, enabled: bool) -> Response {
    button_with(ui, label, skin, enabled, None, 0.0)
}

fn button_with(
    ui: &mut Ui,
    label: &str,
    skin: Skin,
    enabled: bool,
    glyph: Option<Glyph>,
    min_width: f32,
) -> Response {
    let (font, pad, height, radius) = match skin {
        Skin::Subtle => (regular(13.0), 12.0, 30.0, 6),
        Skin::Tint => (semibold(13.0), 10.0, 30.0, 6),
        Skin::Tab(_) => (regular(13.0), 14.0, 26.0, 5),
    };
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font, Color32::PLACEHOLDER);
    let glyph_width = if glyph.is_some() { 17.0 } else { 0.0 };
    let content = galley.size().x + glyph_width;
    let size = vec2((content + pad * 2.0).max(min_width), height);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, label));
    if ui.is_rect_visible(rect) {
        let hot = enabled && (response.hovered() || response.has_focus());
        let (fill, ink, border) = match skin {
            Skin::Subtle if !enabled => (white(0.04), white(0.35), white(0.04)),
            Skin::Subtle if hot => (white(0.10), INK, white(0.06)),
            Skin::Subtle => (white(0.06), INK, white(0.06)),
            Skin::Tint if !enabled => (SURFACE_HOVER, DIM, SURFACE_HOVER),
            Skin::Tint if hot => (ACCENT_FILL_HOVER, ACCENT, ACCENT_FILL_HOVER),
            Skin::Tint => (ACCENT_FILL, ACCENT, ACCENT_FILL),
            Skin::Tab(true) => (ACCENT_FILL, ACCENT, ACCENT_FILL),
            Skin::Tab(false) if hot => (SURFACE_HOVER, INK, SURFACE_HOVER),
            Skin::Tab(false) => (Color32::TRANSPARENT, MUTED, Color32::TRANSPARENT),
        };
        let painter = ui.painter();
        painter.rect(
            rect,
            radius,
            fill,
            Stroke::new(1.0, border),
            StrokeKind::Inside,
        );
        let mut x = rect.center().x - content / 2.0;
        if let Some(glyph) = glyph {
            draw_glyph(painter, glyph, pos2(x + 5.5, rect.center().y), ink);
            x += glyph_width;
        }
        let text_top = rect.center().y - galley.size().y / 2.0;
        painter.galley(pos2(x, text_top), galley, ink);
        if response.has_focus() {
            painter.rect_stroke(
                rect.expand(2.0),
                radius + 2,
                Stroke::new(1.5, ACCENT),
                StrokeKind::Outside,
            );
        }
    }
    response
}

fn draw_glyph(painter: &egui::Painter, glyph: Glyph, center: Pos2, ink: Color32) {
    match glyph {
        Glyph::Play => {
            painter.add(Shape::convex_polygon(
                vec![
                    center + vec2(-3.5, -4.5),
                    center + vec2(4.5, 0.0),
                    center + vec2(-3.5, 4.5),
                ],
                ink,
                Stroke::NONE,
            ));
        }
        Glyph::Power => {
            let stroke = Stroke::new(1.4, ink);
            let arc: Vec<Pos2> = (0..=20)
                .map(|step| {
                    // An open ring: 300 degrees, with the gap at the top.
                    let angle = (-60.0 + 300.0 * step as f32 / 20.0).to_radians();
                    center + 4.6 * vec2(angle.cos(), angle.sin())
                })
                .collect();
            painter.add(Shape::line(arc, stroke));
            painter.line_segment([center + vec2(0.0, -5.6), center + vec2(0.0, -1.0)], stroke);
        }
    }
}

/// One line of text in an exact place, truncated to fit. Truncated egui labels show
/// their full text on hover by themselves.
fn text_at(
    ui: &mut Ui,
    salt: impl Hash + std::fmt::Debug,
    rect: Rect,
    text: &str,
    font: FontId,
    color: Color32,
    align: Align,
) -> Response {
    let layout = match align {
        Align::Max => Layout::right_to_left(Align::Center),
        _ => Layout::left_to_right(Align::Center),
    };
    let mut child = ui.new_child(
        UiBuilder::new()
            .id_salt(Id::new(salt))
            .max_rect(rect)
            .layout(layout),
    );
    child.add(Label::new(RichText::new(text).font(font).color(color)).truncate())
}

fn pill(
    ui: &mut Ui,
    salt: impl Hash + std::fmt::Debug,
    left_center: Pos2,
    text: &str,
    ink: Color32,
    fill: Color32,
) {
    let font = semibold(11.0);
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font.clone(), ink);
    let size = galley.size() + vec2(20.0, 3.0);
    let rect = Rect::from_min_size(pos2(left_center.x, left_center.y - size.y / 2.0), size);
    ui.painter().rect_filled(rect, 99, fill);
    text_at(
        ui,
        salt,
        rect.shrink2(vec2(10.0, 1.5)),
        text,
        font,
        ink,
        Align::Min,
    );
}

fn chip(
    ui: &mut Ui,
    salt: impl Hash + std::fmt::Debug,
    left_top: Pos2,
    text: &str,
    max_width: f32,
) {
    let font = regular(11.0);
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font.clone(), SOFT);
    let size = vec2(
        (galley.size().x + 12.0).min(max_width),
        galley.size().y + 2.0,
    );
    let rect = Rect::from_min_size(left_top, size);
    ui.painter().rect(
        rect,
        4,
        white(0.06),
        Stroke::new(1.0, white(0.06)),
        StrokeKind::Inside,
    );
    text_at(
        ui,
        salt,
        rect.shrink2(vec2(6.0, 1.0)),
        text,
        font,
        SOFT,
        Align::Min,
    );
}

fn meter(ui: &Ui, rect: Rect, parts: &[(f32, Color32)]) {
    let painter = ui.painter();
    let radius = (rect.height() / 2.0).round() as u8;
    painter.rect_filled(rect, radius, white(0.08));
    let mut left = rect.left();
    for (fraction, color) in parts {
        let width = rect.width() * fraction.clamp(0.0, 1.0);
        if width < 0.5 {
            continue;
        }
        let right = (left + width).min(rect.right());
        let part = Rect::from_min_max(pos2(left, rect.top()), pos2(right, rect.bottom()));
        painter.rect_filled(part, radius, *color);
        left = right;
    }
}

fn section(ui: &mut Ui, label: &str) {
    ui.add_space(18.0);
    ui.horizontal(|ui| {
        ui.add_space(2.0);
        ui.label(RichText::new(label).font(semibold(13.0)).color(SOFT));
    });
    ui.add_space(8.0);
}

fn placeholder(ui: &mut Ui, title: &str, body: &str) {
    ui.add_space(96.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(title).font(semibold(18.0)).color(INK));
        ui.add_space(6.0);
        ui.label(RichText::new(body).font(regular(14.0)).color(MUTED));
    });
}

fn notice(ui: &mut Ui, color: Color32, text: &str) {
    Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(8)
        .inner_margin(Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(RichText::new(text).font(regular(13.0)).color(color));
        });
}

/// Progress and results float at the bottom, as Lab Manager's toasts do.
fn toast(ctx: &Context, pending: Option<(&str, bool)>, message: Option<(&str, bool)>) {
    let (text, color) = match (pending, message) {
        (Some((model, true)), _) => (format!("Loading {model}…"), AMBER),
        (Some((model, false)), _) => (format!("Unloading {model}…"), AMBER),
        (None, Some((text, true))) => (text.to_owned(), RED),
        (None, Some((text, false))) => (text.to_owned(), GREEN),
        (None, None) => return,
    };
    let width = (ctx.content_rect().width() - 64.0).max(120.0);
    egui::Area::new(Id::new("toast"))
        .order(Order::Foreground)
        .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -18.0))
        .interactable(false)
        .show(ctx, |ui| {
            Frame::new()
                .fill(SURFACE_HOVER)
                .stroke(Stroke::new(1.0, white(0.08)))
                .corner_radius(8)
                .inner_margin(Margin::symmetric(16, 10))
                .show(ui, |ui| {
                    ui.set_max_width(width);
                    ui.horizontal(|ui| {
                        let (dot, _) = ui.allocate_exact_size(vec2(8.0, 16.0), Sense::hover());
                        ui.painter().circle_filled(dot.center(), 3.5, color);
                        ui.add_space(8.0);
                        ui.add(
                            Label::new(RichText::new(text).font(regular(13.0)).color(INK)).wrap(),
                        );
                    });
                });
        });
}

// ---------------------------------------------------------------------------
// Models

fn models(
    ui: &mut Ui,
    art: &Art,
    snapshot: Option<&Snapshot>,
    pending: Option<(&str, bool)>,
) -> Option<Action> {
    let Some(snapshot) = snapshot else {
        placeholder(ui, "Connecting to Ollama", "Waiting for the first sample.");
        return None;
    };
    let engine = match &snapshot.engine {
        Ok(engine) => engine,
        Err(error) => {
            placeholder(ui, "Ollama isn't reachable", error);
            let mut action = None;
            ui.add_space(16.0);
            ui.vertical_centered(|ui| {
                if button(ui, "Try again", Skin::Subtle, true).clicked() {
                    action = Some(Action::Refresh);
                }
            });
            return action;
        }
    };
    if engine.models.is_empty() && engine.residents.is_empty() {
        placeholder(
            ui,
            "No models yet",
            "Ollama reports no installed or loaded models.",
        );
        return None;
    }

    let rankings = &snapshot.rankings;
    let entry = |model: Option<&Model>| {
        model.and_then(|model| rankings.for_model(&model.name, &model.digest))
    };
    // Ranked models first, best first; the rest keep Ollama's order.
    let order = |model: Option<&Model>| entry(model).map_or(u32::MAX, |entry| entry.rank);

    let mut action = None;
    if !rankings.models.is_empty() {
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.add_space(2.0);
            let basis = if rankings.ranking == "benchmark" {
                "Ranked by Offline Panel's benchmark: most correct answers, then quickest replies. \
                 The top 3 that fit on the GPU are recommended."
            } else {
                "Ranked by size until Offline Panel has benchmarked every model that fits on the GPU."
            };
            ui.label(RichText::new(basis).font(regular(12.0)).color(MUTED));
        });
    }
    if !engine.residents.is_empty() {
        section(ui, "Loaded in memory");
        let mut residents: Vec<_> = engine
            .residents
            .iter()
            .map(|resident| {
                let model = engine
                    .models
                    .iter()
                    .find(|model| model.name == resident.name);
                (resident, model)
            })
            .collect();
        residents.sort_by_key(|(_, model)| order(*model));
        for (resident, model) in residents {
            if let Some(row_action) =
                model_row(ui, art, model, Some(resident), entry(model), pending)
            {
                action = Some(row_action);
            }
        }
    }

    let mut installed: Vec<&Model> = engine
        .models
        .iter()
        .filter(|model| {
            !engine
                .residents
                .iter()
                .any(|resident| resident.name == model.name)
        })
        .collect();
    installed.sort_by_key(|model| order(Some(*model)));
    if !installed.is_empty() {
        section(ui, "Installed on disk");
        for model in installed {
            if let Some(row_action) =
                model_row(ui, art, Some(model), None, entry(Some(model)), pending)
            {
                action = Some(row_action);
            }
        }
    }
    action
}

/// A compact row: tile and identity at the left, metrics in shared columns, and the
/// action pinned to the right, so every row lines up with the next.
fn model_row(
    ui: &mut Ui,
    art: &Art,
    model: Option<&Model>,
    resident: Option<&Resident>,
    rank: Option<&RankEntry>,
    pending: Option<(&str, bool)>,
) -> Option<Action> {
    let name = model
        .map(|model| model.name.as_str())
        .or_else(|| resident.map(|resident| resident.name.as_str()))?;
    let loaded = resident.is_some();
    let busy = pending.filter(|(candidate, _)| *candidate == name);

    let (card, _) = ui.allocate_exact_size(
        vec2(ui.available_width(), ROW_HEIGHT + 24.0),
        Sense::hover(),
    );
    ui.add_space(6.0);
    if !ui.is_rect_visible(card) {
        return None;
    }
    let hovered = ui.rect_contains_pointer(card);
    ui.painter().rect(
        card,
        8,
        if hovered { SURFACE_HOVER } else { SURFACE },
        Stroke::new(1.0, if hovered { white(0.10) } else { LINE }),
        StrokeKind::Inside,
    );
    let inner = card.shrink2(vec2(14.0, 12.0));

    let tile = Rect::from_center_size(
        pos2(inner.left() + 16.0, inner.center().y),
        Vec2::splat(32.0),
    );
    let texture = match (busy, loaded) {
        (Some(_), _) => &art.busy,
        (None, true) => &art.loaded,
        (None, false) => &art.idle,
    };
    image(ui, texture, tile);

    let actions = Rect::from_min_max(pos2(inner.right() - ACTION_WIDTH, inner.top()), inner.max);
    let metrics = Rect::from_min_max(
        pos2(actions.left() - 12.0 - METRICS_WIDTH, inner.top()),
        pos2(actions.left() - 12.0, inner.bottom()),
    );
    let identity = Rect::from_min_max(
        pos2(tile.right() + 12.0, inner.top()),
        pos2(metrics.left() - 12.0, inner.bottom()),
    );

    // Identity: name and state, what it is, and its build.
    let (state, ink, fill) = match busy {
        Some((_, true)) => ("Loading", AMBER, AMBER_BASE.gamma_multiply(0.14)),
        Some((_, false)) => ("Unloading", AMBER, AMBER_BASE.gamma_multiply(0.14)),
        None if loaded => ("Loaded", GREEN, GREEN_BASE.gamma_multiply(0.16)),
        None => ("Installed", MUTED, white(0.07)),
    };
    let details = model.map(|model| &model.details);
    let role = details
        .map(|details| {
            let size = details.parameter_size.trim();
            [
                details.family.trim().to_owned(),
                if size.is_empty() {
                    String::new()
                } else {
                    format!("{size} parameters")
                },
            ]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("  ·  ")
        })
        .filter(|role| !role.is_empty())
        .unwrap_or_else(|| "Missing from the local inventory".to_owned());
    let role = match rank.and_then(RankEntry::score) {
        Some(score) => format!("{role}  ·  {score}"),
        None => role,
    };
    let build = [
        details
            .map(|details| details.quantization_level.trim().to_owned())
            .unwrap_or_default(),
        model
            .map(|model| format!("{} on disk", format_bytes(model.size)))
            .unwrap_or_default(),
    ]
    .into_iter()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join("  ·  ");

    let name_font = semibold(14.0);
    let role_font = regular(12.0);
    let name_height = line_height(ui, &name_font);
    let role_height = line_height(ui, &role_font);
    let chip_height = line_height(ui, &regular(11.0)) + 2.0;
    let block = name_height + 2.0 + role_height + 3.0 + chip_height;
    let top = identity.center().y - block / 2.0;
    let pill_width = |ui: &Ui, text: &str| {
        ui.painter()
            .layout_no_wrap(text.to_owned(), semibold(11.0), ink)
            .size()
            .x
            + 20.0
    };
    let recommended = rank.filter(|rank| rank.recommended).map(|_| "Recommended");
    let state_width = pill_width(ui, state);
    let pills = state_width + recommended.map_or(0.0, |text| pill_width(ui, text) + 6.0);
    // The rank, when Offline Panel has one, leads the name: "#1".
    let rank_width = match rank {
        Some(rank) => {
            let label = format!("#{}", rank.rank);
            let width = ui
                .painter()
                .layout_no_wrap(label.clone(), name_font.clone(), ACCENT)
                .size()
                .x;
            text_at(
                ui,
                (name, "rank"),
                Rect::from_min_size(pos2(identity.left(), top), vec2(width + 2.0, name_height)),
                &label,
                name_font.clone(),
                ACCENT,
                Align::Min,
            );
            width + 8.0
        }
        None => 0.0,
    };
    let name_rect = Rect::from_min_size(
        pos2(identity.left() + rank_width, top),
        vec2(
            (identity.width() - rank_width - pills - 8.0).max(40.0),
            name_height,
        ),
    );
    let name_label = text_at(
        ui,
        (name, "name"),
        name_rect,
        name,
        name_font,
        INK,
        Align::Min,
    );
    pill(
        ui,
        (name, "state"),
        pos2(name_label.rect.right() + 8.0, name_rect.center().y),
        state,
        ink,
        fill,
    );
    if let Some(text) = recommended {
        pill(
            ui,
            (name, "recommended"),
            pos2(
                name_label.rect.right() + 8.0 + state_width + 6.0,
                name_rect.center().y,
            ),
            text,
            ACCENT,
            ACCENT_FILL,
        );
    }
    if let Some(rank) = rank.filter(|rank| !rank.reason.is_empty()) {
        ui.interact(identity, Id::new((name, "rank-tip")), Sense::hover())
            .on_hover_text(format!("#{}: {}", rank.rank, rank.reason));
    }
    text_at(
        ui,
        (name, "role"),
        Rect::from_min_size(
            pos2(identity.left(), top + name_height + 2.0),
            vec2(identity.width(), role_height),
        ),
        &role,
        role_font,
        MUTED,
        Align::Min,
    );
    if !build.is_empty() {
        chip(
            ui,
            (name, "build"),
            pos2(identity.left(), top + name_height + role_height + 5.0),
            &build,
            identity.width(),
        );
    }

    let max_context = model
        .and_then(|model| model.max_context)
        .filter(|context| *context > 0);
    match resident {
        Some(resident) => {
            let cells = stat_cells(ui, metrics);
            let on_gpu = if resident.size > 0 {
                (resident.size_vram as f32 / resident.size as f32).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let bar = stat(
                ui,
                (name, "vram"),
                cells[0],
                "VRAM",
                &format_bytes(resident.size_vram),
            );
            let bar = Rect::from_center_size(bar.center(), vec2(bar.width(), 4.0));
            meter(ui, bar, &[(on_gpu, ACCENT)]);
            ui.interact(cells[0], Id::new((name, "vram-tip")), Sense::hover())
                .on_hover_text("VRAM this model holds. The bar is its share that runs on the GPU.");

            let share = if resident.size == 0 {
                String::new()
            } else if resident.size_vram == 0 {
                "CPU only".to_owned()
            } else {
                format!("{:.0}% on GPU", on_gpu * 100.0)
            };
            let note = stat(
                ui,
                (name, "memory"),
                cells[1],
                "MEMORY",
                &format_bytes(resident.size),
            );
            text_at(
                ui,
                (name, "memory-note"),
                note,
                &share,
                regular(11.0),
                MUTED,
                Align::Min,
            );
            ui.interact(cells[1], Id::new((name, "memory-tip")), Sense::hover())
                .on_hover_text("Total memory Ollama allocated for this model, on GPU and CPU.");

            let context = resident
                .context_length
                .filter(|context| *context > 0)
                .map_or_else(|| "—".to_owned(), format_tokens);
            let note = stat(ui, (name, "context"), cells[2], "CONTEXT", &context);
            let limit = max_context
                .map(|tokens| format!("max {}", format_tokens(tokens)))
                .unwrap_or_default();
            text_at(
                ui,
                (name, "context-note"),
                note,
                &limit,
                regular(11.0),
                MUTED,
                Align::Min,
            );

            let mut tip = String::from("Context window Ollama loaded this model with.");
            if let Some(expiry) = &resident.expires_at {
                tip.push_str(&format!("\nOllama expiry: {expiry}"));
            }
            if let Some(runner) = &resident.runner {
                tip.push_str(&format!("\nRunner: {runner}"));
            }
            ui.interact(cells[2], Id::new((name, "context-tip")), Sense::hover())
                .on_hover_text(tip);
        }
        None => {
            let sum_font = regular(12.0);
            let sub_font = regular(11.0);
            let sum_height = line_height(ui, &sum_font);
            let sub_height = line_height(ui, &sub_font);
            let top = metrics.center().y - (sum_height + 1.0 + sub_height) / 2.0;
            text_at(
                ui,
                (name, "idle"),
                Rect::from_min_size(pos2(metrics.left(), top), vec2(metrics.width(), sum_height)),
                "Not in memory",
                sum_font,
                SOFT,
                Align::Max,
            );
            let limit = max_context.map_or_else(
                || "Context limit unavailable".to_owned(),
                |tokens| format!("Max context {} tokens", format_tokens(tokens)),
            );
            text_at(
                ui,
                (name, "idle-note"),
                Rect::from_min_size(
                    pos2(metrics.left(), top + sum_height + 1.0),
                    vec2(metrics.width(), sub_height),
                ),
                &limit,
                sub_font,
                DIM,
                Align::Max,
            );
        }
    }

    let slot = Rect::from_min_size(
        pos2(actions.left(), actions.center().y - 15.0),
        vec2(ACTION_WIDTH, 30.0),
    );
    let mut child = ui.new_child(
        UiBuilder::new()
            .id_salt(Id::new((name, "action")))
            .max_rect(slot)
            .layout(Layout::left_to_right(Align::Min)),
    );
    let (label, skin, glyph) = if loaded {
        ("Unload", Skin::Subtle, Glyph::Power)
    } else {
        ("Load", Skin::Tint, Glyph::Play)
    };
    let explanation = if model.is_none() {
        "This loaded tag is missing from the local inventory."
    } else if loaded {
        "Ask Ollama to release this model when its active requests finish."
    } else {
        "Load this model for five minutes. Ollama may evict another idle model to make room."
    };
    let enabled = pending.is_none() && model.is_some();
    let clicked = button_with(&mut child, label, skin, enabled, Some(glyph), ACTION_WIDTH)
        .on_hover_text(explanation)
        .clicked();
    clicked.then(|| Action::SetResident {
        model: name.to_owned(),
        load: !loaded,
    })
}

/// The three metric columns every loaded row shares.
fn stat_cells(ui: &Ui, metrics: Rect) -> [Rect; 3] {
    let height = line_height(ui, &semibold(11.0))
        + line_height(ui, &regular(13.0))
        + line_height(ui, &regular(11.0))
        + 2.0;
    let top = metrics.center().y - height / 2.0;
    let mut left = metrics.left();
    [72.0, 100.0, 110.0].map(|width| {
        let cell = Rect::from_min_size(pos2(left, top), vec2(width, height));
        left += width + 14.0;
        cell
    })
}

/// Caption over value. Returns the third line's place, for a note or a small visual.
fn stat(
    ui: &mut Ui,
    salt: impl Hash + std::fmt::Debug + Copy,
    cell: Rect,
    caption: &str,
    value: &str,
) -> Rect {
    let caption_font = semibold(11.0);
    let value_font = regular(13.0);
    let caption_height = line_height(ui, &caption_font);
    let value_height = line_height(ui, &value_font);
    text_at(
        ui,
        (salt, "caption"),
        Rect::from_min_size(cell.min, vec2(cell.width(), caption_height)),
        caption,
        caption_font,
        DIM,
        Align::Min,
    );
    text_at(
        ui,
        (salt, "value"),
        Rect::from_min_size(
            pos2(cell.left(), cell.top() + caption_height + 1.0),
            vec2(cell.width(), value_height),
        ),
        value,
        value_font,
        INK,
        Align::Min,
    );
    Rect::from_min_max(
        pos2(
            cell.left(),
            cell.top() + caption_height + value_height + 2.0,
        ),
        cell.max,
    )
}

// ---------------------------------------------------------------------------
// Resources

fn resources(ui: &mut Ui, art: &Art, snapshot: Option<&Snapshot>, gpu_util_history: &[f32]) {
    let Some(snapshot) = snapshot else {
        placeholder(ui, "Reading the GPU", "Waiting for the first sample.");
        return;
    };
    let engine = snapshot.engine.as_ref().ok();
    match &snapshot.gpus {
        Ok(gpus) if gpus.is_empty() => {
            ui.add_space(14.0);
            notice(ui, MUTED, "No NVIDIA GPUs reported by nvidia-smi.");
        }
        Ok(gpus) => {
            for (position, gpu) in gpus.iter().enumerate() {
                let history = (position == 0).then_some(gpu_util_history);
                gpu_overview(ui, art, gpu, engine.filter(|_| gpus.len() == 1), history);
            }
            activity(ui, gpu_util_history);
        }
        Err(error) => {
            ui.add_space(14.0);
            notice(ui, RED, error);
        }
    }
    memory_table(ui, &snapshot.engine);
    footer(ui, snapshot);
}

fn gpu_overview(
    ui: &mut Ui,
    art: &Art,
    gpu: &Gpu,
    engine: Option<&Engine>,
    history: Option<&[f32]>,
) {
    ui.add_space(14.0);
    let heading_font = semibold(20.0);
    let (heading, _) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::hover());
    let tile = Rect::from_center_size(
        pos2(heading.left() + 16.0, heading.center().y),
        Vec2::splat(32.0),
    );
    image(ui, &art.gpu, tile);
    let summary = format!("GPU {}  ·  whole device", gpu.index);
    let summary_width = ui
        .painter()
        .layout_no_wrap(summary.clone(), regular(12.0), MUTED)
        .size()
        .x;
    let height = line_height(ui, &heading_font);
    text_at(
        ui,
        (gpu.index, "gpu-name"),
        Rect::from_min_max(
            pos2(tile.right() + 12.0, heading.center().y - height / 2.0),
            pos2(
                heading.right() - summary_width - 16.0,
                heading.center().y + height / 2.0,
            ),
        ),
        &gpu.name,
        heading_font,
        INK,
        Align::Min,
    );
    text_at(
        ui,
        (gpu.index, "gpu-summary"),
        Rect::from_min_max(
            pos2(
                heading.right() - summary_width - 2.0,
                heading.center().y - 8.0,
            ),
            pos2(heading.right(), heading.center().y + 8.0),
        ),
        &summary,
        regular(12.0),
        MUTED,
        Align::Max,
    );
    ui.add_space(12.0);

    const GIB: f32 = 1024.0;
    let gap = 12.0;
    let width = ((ui.available_width() - gap * 2.0) / 3.0).floor();
    ui.horizontal(|ui| {
        let utilization = gpu.utilization;
        let peak = history
            .filter(|samples| !samples.is_empty())
            .map(|samples| {
                let peak = samples.iter().copied().fold(0.0f32, f32::max);
                format!("Peak {peak:.0}% in the last {} s", samples.len() * 2)
            })
            .unwrap_or_default();
        metric_card(
            ui,
            width,
            "GPU utilization",
            &utilization.map_or_else(|| "—".to_owned(), |value| format!("{value:.0}%")),
            "Whole device, all processes",
            &[(utilization.unwrap_or(0.0) / 100.0, ACCENT)],
            Legend::Text(&peak),
        );
        ui.add_space(gap);

        let used = gpu.memory_used_mib;
        let total = gpu.memory_total_mib.filter(|total| *total > 0.0);
        let models_mib = engine.map(|engine| resident_vram(engine) as f32 / (GIB * GIB));
        let value = used.map_or_else(|| "—".to_owned(), |used| format!("{:.1} GiB", used / GIB));
        let caption = total.map_or_else(
            || "Total unavailable".to_owned(),
            |total| format!("of {:.1} GiB on this GPU", total / GIB),
        );
        let (parts, legend) = match (used, total, models_mib) {
            (Some(used), Some(total), Some(models)) => {
                let models = models.min(used);
                (
                    vec![(models / total, ACCENT), ((used - models) / total, OTHER)],
                    Legend::Split(
                        format!("Models {:.1} GiB", models / GIB),
                        format!("Other {:.1} GiB", (used - models) / GIB),
                    ),
                )
            }
            (Some(used), Some(total), None) => (vec![(used / total, ACCENT)], Legend::Text("")),
            _ => (Vec::new(), Legend::Text("")),
        };
        metric_card(ui, width, "VRAM used", &value, &caption, &parts, legend);
        ui.add_space(gap);

        let temperature = gpu.temperature_c;
        let heat = match temperature {
            Some(value) if value >= 85.0 => RED,
            Some(value) if value >= 70.0 => AMBER,
            _ => ACCENT,
        };
        metric_card(
            ui,
            width,
            "Temperature",
            &temperature.map_or_else(|| "—".to_owned(), |value| format!("{value:.0} °C")),
            &gpu.power_w.map_or_else(
                || "Power draw unavailable".to_owned(),
                |value| format!("Drawing {value:.1} W"),
            ),
            &[(temperature.unwrap_or(0.0) / 100.0, heat)],
            Legend::Text("Bar spans 0 to 100 °C"),
        );
    });
}

enum Legend<'a> {
    Text(&'a str),
    Split(String, String),
}

fn metric_card(
    ui: &mut Ui,
    width: f32,
    label: &str,
    value: &str,
    caption: &str,
    parts: &[(f32, Color32)],
    legend: Legend<'_>,
) {
    Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(10)
        .inner_margin(Margin::symmetric(16, 14))
        .show(ui, |ui| {
            let inner = width - 32.0;
            ui.set_width(inner);
            ui.vertical(|ui| {
                let line = |ui: &mut Ui, text: &str, font: FontId, color: Color32| {
                    ui.add(Label::new(RichText::new(text).font(font).color(color)).truncate());
                };
                line(ui, label, regular(12.0), MUTED);
                ui.add_space(2.0);
                line(ui, value, semibold(30.0), INK);
                ui.add_space(2.0);
                line(ui, caption, regular(11.0), MUTED);
                ui.add_space(10.0);
                let (bar, _) = ui.allocate_exact_size(vec2(inner, 5.0), Sense::hover());
                meter(ui, bar, parts);
                ui.add_space(8.0);
                let height = line_height(ui, &regular(11.0));
                ui.allocate_ui_with_layout(
                    vec2(inner, height),
                    Layout::left_to_right(Align::Center),
                    |ui| match legend {
                        Legend::Text(text) => line(ui, text, regular(11.0), MUTED),
                        Legend::Split(first, second) => {
                            for (text, color) in [(first, ACCENT), (second, OTHER)] {
                                let (swatch, _) =
                                    ui.allocate_exact_size(vec2(7.0, 7.0), Sense::hover());
                                ui.painter().rect_filled(swatch, 2, color);
                                ui.add_space(5.0);
                                ui.label(RichText::new(text).font(regular(11.0)).color(MUTED));
                                ui.add_space(12.0);
                            }
                        }
                    },
                );
            });
        });
}

fn activity(ui: &mut Ui, history: &[f32]) {
    ui.add_space(12.0);
    Frame::new()
        .fill(CHART)
        .stroke(Stroke::new(1.0, CHART_LINE))
        .corner_radius(10)
        .inner_margin(Margin::symmetric(16, 14))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new("Activity").font(semibold(13.0)).color(INK));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new("GPU utilization  ·  sampled every 2 s")
                            .font(regular(11.0))
                            .color(Color32::from_rgb(170, 169, 192)),
                    );
                });
            });
            ui.add_space(10.0);
            chart(ui, history);
        });
}

/// Real samples only, newest at the right; the line grows in from that edge.
fn chart(ui: &mut Ui, history: &[f32]) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 120.0), Sense::hover());
    let painter = ui.painter();
    let plot = Rect::from_min_max(
        pos2(rect.left() + 38.0, rect.top() + 8.0),
        pos2(rect.right() - 6.0, rect.bottom() - 8.0),
    );
    for (fraction, label) in [(0.0, "100%"), (0.5, "50%"), (1.0, "0%")] {
        let y = egui::lerp(plot.y_range(), fraction);
        painter.hline(plot.x_range(), y, Stroke::new(1.0, white(0.06)));
        painter.text(
            pos2(plot.left() - 10.0, y),
            Align2::RIGHT_CENTER,
            label,
            regular(11.0),
            DIM,
        );
    }
    if history.is_empty() {
        painter.text(
            plot.center(),
            Align2::CENTER_CENTER,
            "Waiting for samples",
            regular(12.0),
            MUTED,
        );
        return;
    }
    let step = plot.width() / (HISTORY - 1) as f32;
    let point = |index: usize, sample: f32| {
        pos2(
            plot.right() - (history.len() - 1 - index) as f32 * step,
            egui::lerp(plot.y_range(), 1.0 - sample.clamp(0.0, 100.0) / 100.0),
        )
    };
    let samples: Vec<Pos2> = history
        .iter()
        .enumerate()
        .map(|(index, sample)| point(index, *sample))
        .collect();
    let curve = smooth(&samples, plot);
    if curve.len() > 1 {
        let mut area = Mesh::default();
        for (index, at) in curve.iter().enumerate() {
            area.colored_vertex(*at, ACCENT.gamma_multiply(0.30));
            area.colored_vertex(pos2(at.x, plot.bottom()), ACCENT.gamma_multiply(0.0));
            if index > 0 {
                let base = (index as u32 - 1) * 2;
                area.add_triangle(base, base + 1, base + 2);
                area.add_triangle(base + 1, base + 3, base + 2);
            }
        }
        painter.add(Shape::mesh(area));
        painter.add(Shape::line(
            curve.clone(),
            Stroke::new(5.0, ACCENT.gamma_multiply(0.14)),
        ));
        painter.add(Shape::line(curve, Stroke::new(1.6, ACCENT)));
    }
    if let Some(last) = samples.last() {
        painter.circle_filled(*last, 5.0, ACCENT.gamma_multiply(0.2));
        painter.circle_filled(*last, 2.2, ACCENT);
    }
}

/// Catmull-Rom through the samples, kept inside the plot.
fn smooth(points: &[Pos2], bounds: Rect) -> Vec<Pos2> {
    if points.len() < 3 {
        return points.to_vec();
    }
    const STEPS: usize = 6;
    let at = |index: isize| points[index.clamp(0, points.len() as isize - 1) as usize];
    let mut curve = Vec::with_capacity(points.len() * STEPS);
    for segment in 0..points.len() as isize - 1 {
        let (p0, p1, p2, p3) = (
            at(segment - 1),
            at(segment),
            at(segment + 1),
            at(segment + 2),
        );
        for step in 0..STEPS {
            let t = step as f32 / STEPS as f32;
            let (t2, t3) = (t * t, t * t * t);
            let blend = |a: f32, b: f32, c: f32, d: f32| {
                0.5 * (2.0 * b
                    + (c - a) * t
                    + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2
                    + (3.0 * b - a - 3.0 * c + d) * t3)
            };
            curve.push(pos2(
                blend(p0.x, p1.x, p2.x, p3.x),
                blend(p0.y, p1.y, p2.y, p3.y).clamp(bounds.top(), bounds.bottom()),
            ));
        }
    }
    curve.push(points[points.len() - 1]);
    curve
}

#[derive(Clone, Copy)]
enum RowKind {
    Head,
    Body,
    Total,
}

fn memory_table(ui: &mut Ui, engine: &Result<Engine, String>) {
    section(ui, "Models in memory");
    Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(10)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            let engine = match engine {
                Ok(engine) => engine,
                Err(error) => {
                    table_note(ui, error, RED);
                    return;
                }
            };
            table_row(
                ui,
                "head",
                RowKind::Head,
                ["Model", "VRAM", "Memory", "On GPU", "Context"].map(str::to_owned),
            );
            if engine.residents.is_empty() {
                table_rule(ui);
                table_note(ui, "No models are in memory.", MUTED);
                return;
            }
            let mut memory = 0u64;
            for resident in &engine.residents {
                table_rule(ui);
                memory = memory.saturating_add(resident.size);
                let share = if resident.size > 0 {
                    format!(
                        "{:.0}%",
                        (resident.size_vram as f64 / resident.size as f64).min(1.0) * 100.0
                    )
                } else {
                    "—".to_owned()
                };
                table_row(
                    ui,
                    &resident.name,
                    RowKind::Body,
                    [
                        resident.name.clone(),
                        format_bytes(resident.size_vram),
                        format_bytes(resident.size),
                        share,
                        resident
                            .context_length
                            .filter(|context| *context > 0)
                            .map_or_else(|| "—".to_owned(), format_tokens),
                    ],
                );
            }
            table_row(
                ui,
                "total",
                RowKind::Total,
                [
                    "All loaded models".to_owned(),
                    format_bytes(resident_vram(engine)),
                    format_bytes(memory),
                    String::new(),
                    String::new(),
                ],
            );
        });
}

fn table_rule(ui: &mut Ui) {
    let (rule, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rule, 0, white(0.05));
}

fn table_note(ui: &mut Ui, text: &str, color: Color32) {
    Frame::new()
        .inner_margin(Margin::symmetric(16, 12))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.add(Label::new(RichText::new(text).font(regular(12.0)).color(color)).wrap());
        });
}

fn table_row(ui: &mut Ui, salt: &str, kind: RowKind, cells: [String; 5]) {
    const NUMBER_WIDTH: f32 = 96.0;
    let height = match kind {
        RowKind::Head => 32.0,
        RowKind::Body => 36.0,
        RowKind::Total => 40.0,
    };
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let painter = ui.painter();
    match kind {
        RowKind::Head => {
            let corners = egui::CornerRadius {
                nw: 9,
                ne: 9,
                sw: 0,
                se: 0,
            };
            painter.rect_filled(row.shrink(1.0), corners, white(0.02));
        }
        RowKind::Total => {
            let corners = egui::CornerRadius {
                nw: 0,
                ne: 0,
                sw: 9,
                se: 9,
            };
            painter.rect_filled(row.shrink(1.0), corners, ACCENT.gamma_multiply(0.05));
            painter.hline(
                row.x_range(),
                row.top() + 0.5,
                Stroke::new(1.0, ACCENT.gamma_multiply(0.20)),
            );
        }
        RowKind::Body => {
            if ui.rect_contains_pointer(row) {
                painter.rect_filled(row.shrink2(vec2(1.0, 0.0)), 0, white(0.04));
            }
        }
    }
    let inner = row.shrink2(vec2(16.0, 0.0));
    let numbers = NUMBER_WIDTH * 4.0;
    let mut right = inner.right();
    for (index, text) in cells.iter().enumerate().rev() {
        let (font, color) = match (kind, index) {
            (RowKind::Head, _) => (semibold(11.0), DIM),
            (RowKind::Body, 0) => (semibold(13.0), INK),
            (RowKind::Body, _) => (regular(13.0), INK),
            (RowKind::Total, 0) => (regular(13.0), SOFT),
            (RowKind::Total, _) => (semibold(13.0), ACCENT),
        };
        let line = line_height(ui, &font);
        let (left, align) = if index == 0 {
            (inner.left(), Align::Min)
        } else {
            (right - NUMBER_WIDTH + 12.0, Align::Max)
        };
        let end = if index == 0 {
            (inner.right() - numbers).max(inner.left() + 60.0)
        } else {
            right
        };
        text_at(
            ui,
            (salt, "cell", index),
            Rect::from_min_max(
                pos2(left, row.center().y - line / 2.0),
                pos2(end, row.center().y + line / 2.0),
            ),
            text,
            font,
            color,
            align,
        );
        right -= NUMBER_WIDTH;
    }
}

fn footer(ui: &mut Ui, snapshot: &Snapshot) {
    ui.add_space(12.0);
    let engine = snapshot.engine.as_ref().ok();
    let facts = [
        (
            "Ollama",
            engine
                .and_then(|engine| engine.version.clone())
                .unwrap_or_else(|| "—".to_owned()),
        ),
        ("Endpoint", ollama_address().to_owned()),
        (
            "Installed",
            engine.map_or_else(
                || "—".to_owned(),
                |engine| format!("{} models", engine.models.len()),
            ),
        ),
        ("Sampling", "Every 2 s".to_owned()),
        ("GPU data", "nvidia-smi".to_owned()),
    ];
    Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, white(0.05)))
        .corner_radius(8)
        .inner_margin(Margin::symmetric(16, 10))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                for (key, value) in facts {
                    ui.label(RichText::new(key).font(regular(11.0)).color(DIM));
                    ui.add_space(6.0);
                    ui.label(RichText::new(value).font(regular(12.0)).color(SOFT));
                    ui.add_space(22.0);
                }
            });
        });
}

fn format_bytes(bytes: u64) -> String {
    let (unit, scale) = if bytes >= 1024u64.pow(3) {
        ("GiB", 1024u64.pow(3))
    } else if bytes >= 1024u64.pow(2) {
        ("MiB", 1024u64.pow(2))
    } else if bytes >= 1024 {
        ("KiB", 1024)
    } else {
        return format!("{bytes} B");
    };
    format!("{:.1} {unit}", bytes as f64 / scale as f64)
}

fn format_tokens(tokens: u64) -> String {
    let digits = tokens.to_string();
    let mut text = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            text.push(',');
        }
        text.push(digit);
    }
    text
}
