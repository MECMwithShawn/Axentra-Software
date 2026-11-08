//! The Floating Modular layout (Design Handoff, primary concept): detached glass modules inside
//! one frameless window. Top bar with search, navigation rail, vault list, credential details,
//! and a collapsible security column with this session's activity.
//!
//! Every module reads the real, unlocked vault; nothing here is mock data. Shown only while the
//! vault is open; the locked screen never reaches this code.

use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, Color32, Layout, Rect, RichText, Sense, Stroke, UiBuilder, Vec2, pos2, vec2,
};
use vault_core::categorize::{self, Area, Category, Source};
use vault_core::envelope;
use vault_core::form::EntryForm;
use vault_core::generator;
use vault_core::keepass::db::EntryId;
use vault_core::organize::{self, Issue};
use zeroize::Zeroizing;

use super::{Confirm, Page, Session, SessionAction, Sort, View};
use crate::chrome::{self, GAP, PAD};
use crate::icon::{Icon, small_button};
use crate::settings::Layout as UiLayout;
use crate::theme::{ACCENT, CYAN};

const TEXT: Color32 = Color32::from_rgb(0xe7, 0xf6, 0xff);
const MUTED: Color32 = Color32::from_rgb(0x8a, 0xa7, 0xb8);
const FIELD_BG: Color32 = Color32::from_rgb(0x07, 0x10, 0x1a);
const BOX_BG: Color32 = Color32::from_rgb(0x0d, 0x19, 0x26);
const GOOD: Color32 = Color32::from_rgb(0x34, 0xd3, 0x99);
const WARN: Color32 = Color32::from_rgb(0xf0, 0xa0, 0x4b);

fn area_color(area: Area) -> Color32 {
    match area {
        Area::Financial => Color32::from_rgb(0x2e, 0xc4, 0x8f),
        Area::Personal => Color32::from_rgb(0x9b, 0x87, 0xff),
        Area::Digital => Color32::from_rgb(0x22, 0xd3, 0xee),
        Area::Work => Color32::from_rgb(0xf0, 0xb4, 0x4b),
        Area::Other => Color32::from_rgb(0x7d, 0x92, 0xa6),
    }
}

fn faded(c: Color32, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (a * 255.0) as u8)
}

/// Lays `add` out inside `rect`, top to bottom.
fn child<R>(ui: &mut egui::Ui, rect: Rect, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::top_down(Align::Min)),
        add,
    )
    .inner
}

/// Initial-letter tile coloured by the entry's category area (no third-party logos are bundled
/// or fetched: that would leak which sites are in the vault).
fn monogram(painter: &egui::Painter, rect: Rect, title: &str, color: Color32) {
    painter.rect_filled(rect, 8.0, faded(color, 0.16));
    painter.rect_stroke(
        rect,
        8.0,
        Stroke::new(1.0, faded(color, 0.55)),
        egui::StrokeKind::Inside,
    );
    let letter: String = title
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_else(|| "?".into());
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        letter,
        egui::FontId::proportional(rect.height() * 0.46),
        color,
    );
}

/// The entry's site icon on a dark tile, or its monogram when it has none.
fn entry_tile(
    ui: &egui::Ui,
    rect: Rect,
    icon: Option<egui::ImageSource<'static>>,
    title: &str,
    color: Color32,
) {
    match icon {
        Some(src) => {
            let p = ui.painter();
            p.rect_filled(rect, 8.0, Color32::from_rgb(0x0e, 0x1a, 0x27));
            p.rect_stroke(
                rect,
                8.0,
                Stroke::new(1.0, faded(color, 0.35)),
                egui::StrokeKind::Inside,
            );
            egui::Image::new(src).paint_at(ui, rect.shrink(rect.width() * 0.16));
        }
        None => monogram(ui.painter(), rect, title, color),
    }
}

/// Custom selectable row, announced to accessibility tools with its label.
fn row_response(
    ui: &mut egui::Ui,
    height: f32,
    selected: bool,
    label: &str,
) -> (Rect, egui::Response) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, label)
    });
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, 8.0, faded(ACCENT, 0.32));
        painter.rect_stroke(
            rect,
            8.0,
            Stroke::new(1.0, faded(CYAN, 0.75)),
            egui::StrokeKind::Inside,
        );
        painter.rect_filled(
            Rect::from_min_size(
                rect.left_top() + vec2(0.0, 8.0),
                vec2(3.0, rect.height() - 16.0),
            ),
            1.5,
            CYAN,
        );
    } else if resp.hovered() {
        painter.rect_filled(rect, 8.0, faded(CYAN, 0.06));
    }
    (rect, resp)
}

/// Icon-only button with an accessible name (screen readers and tests) and a matching tooltip.
fn icon_button(
    ui: &mut egui::Ui,
    icon: Icon,
    size: f32,
    name: &str,
    frame: bool,
) -> egui::Response {
    let resp = ui.add(egui::Button::image(icon.image(size)).frame(frame));
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name));
    resp.on_hover_text(name)
}

/// A title-bar page tab: icon, label and optional count; icon-only when `compact`.
fn tab_button(
    ui: &mut egui::Ui,
    selected: bool,
    icon: Icon,
    label: &str,
    badge: Option<usize>,
    compact: bool,
) -> egui::Response {
    let font = egui::FontId::proportional(14.5);
    let text_w = if compact {
        0.0
    } else {
        ui.painter()
            .layout_no_wrap(label.to_owned(), font.clone(), TEXT)
            .size()
            .x
    };
    let badge_text = badge.filter(|n| *n > 0 && !compact).map(|n| n.to_string());
    let badge_w = badge_text
        .as_ref()
        .map(|t| t.len() as f32 * 7.0 + 14.0)
        .unwrap_or(0.0);
    let width = if compact {
        38.0
    } else {
        36.0 + text_w + if badge_w > 0.0 { badge_w + 6.0 } else { 0.0 } + 10.0
    };
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 36.0), Sense::click());
    resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, label)
    });
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, 9.0, faded(ACCENT, 0.32));
        p.rect_stroke(
            rect,
            9.0,
            Stroke::new(1.0, faded(CYAN, 0.7)),
            egui::StrokeKind::Inside,
        );
    } else if resp.hovered() {
        p.rect_filled(rect, 9.0, faded(CYAN, 0.08));
    }
    let icon_rect =
        Rect::from_center_size(pos2(rect.left() + 19.0, rect.center().y), Vec2::splat(18.0));
    icon.image(18.0).paint_at(ui, icon_rect);
    if !compact {
        p.text(
            pos2(rect.left() + 34.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            font,
            TEXT,
        );
        if let Some(t) = badge_text {
            let b = Rect::from_min_size(
                pos2(rect.left() + 34.0 + text_w + 6.0, rect.center().y - 10.0),
                vec2(badge_w, 20.0),
            );
            p.rect_filled(b, 10.0, faded(CYAN, 0.12));
            p.text(
                b.center(),
                egui::Align2::CENTER_CENTER,
                t,
                egui::FontId::proportional(11.5),
                MUTED,
            );
        }
    }
    if compact {
        resp.on_hover_text(label)
    } else {
        resp
    }
}

/// What sits at the start of a rail row.
enum Lead {
    None,
    Icon(Icon),
    Dot(Color32),
    /// Expand/collapse chevron (open?) in the given colour.
    Chevron(bool, Color32),
}

fn rail_header(ui: &mut egui::Ui, text: &str) {
    ui.add_space(10.0);
    let mut job = egui::text::LayoutJob::default();
    job.append(
        text,
        6.0,
        egui::TextFormat {
            font_id: egui::FontId::proportional(10.5),
            extra_letter_spacing: 1.6,
            color: MUTED,
            ..Default::default()
        },
    );
    ui.label(job);
    ui.add_space(2.0);
}

/// One rail entry: selectable row with a lead mark, label and right-aligned count.
fn rail_row(
    ui: &mut egui::Ui,
    selected: bool,
    indent: f32,
    lead: Lead,
    label: &str,
    count: Option<usize>,
    strong: bool,
) -> egui::Response {
    let (rect, resp) = row_response(ui, 30.0, selected, label);
    let p = ui.painter();
    let x = rect.left() + indent + 12.0;
    let cy = rect.center().y;
    match lead {
        Lead::None => {}
        Lead::Icon(icon) => icon.image(16.0).paint_at(
            ui,
            Rect::from_center_size(pos2(x + 4.0, cy), Vec2::splat(16.0)),
        ),
        Lead::Dot(c) => {
            p.circle_filled(pos2(x + 4.0, cy), 4.0, c);
        }
        Lead::Chevron(open, c) => {
            let pts = if open {
                vec![
                    pos2(x - 1.0, cy - 2.5),
                    pos2(x + 4.0, cy + 2.5),
                    pos2(x + 9.0, cy - 2.5),
                ]
            } else {
                vec![
                    pos2(x + 1.5, cy - 5.0),
                    pos2(x + 6.5, cy),
                    pos2(x + 1.5, cy + 5.0),
                ]
            };
            p.add(egui::Shape::line(pts, Stroke::new(1.6, c)));
        }
    }
    let text_x = x + if matches!(lead, Lead::None) {
        0.0
    } else {
        18.0
    };
    let count_w = if count.is_some() { 40.0 } else { 0.0 };
    let clip = Rect::from_min_max(
        pos2(text_x, rect.top()),
        pos2(rect.right() - count_w - 6.0, rect.bottom()),
    );
    let font = egui::FontId::proportional(if strong { 15.0 } else { 14.0 });
    let galley = p.layout_no_wrap(
        label.to_owned(),
        font,
        if strong || selected {
            TEXT
        } else {
            Color32::from_rgb(0xc4, 0xd6, 0xe2)
        },
    );
    p.with_clip_rect(clip)
        .galley(pos2(text_x, cy - galley.size().y / 2.0), galley, TEXT);
    if let Some(n) = count {
        p.text(
            pos2(rect.right() - 10.0, cy),
            egui::Align2::RIGHT_CENTER,
            n.to_string(),
            egui::FontId::proportional(12.0),
            MUTED,
        );
    }
    resp
}

/// Depth-first (id, name, active count, depth) for the folder list.
fn flatten_tree(
    node: &super::GroupNode,
    depth: usize,
    out: &mut Vec<(vault_core::keepass::db::GroupId, String, usize, usize)>,
) {
    out.push((node.id, node.name.clone(), node.active, depth));
    for c in &node.children {
        flatten_tree(c, depth + 1, out);
    }
}

fn section_title(ui: &mut egui::Ui, icon: Icon, text: &str) {
    ui.horizontal(|ui| {
        ui.add(icon.image(18.0));
        ui.label(RichText::new(text).size(16.0).color(TEXT));
    });
}

fn strength_label(bits: f64) -> (&'static str, Color32, f32) {
    match bits {
        b if b >= 100.0 => ("Excellent", GOOD, 1.0),
        b if b >= 75.0 => ("Strong", GOOD, 0.8),
        b if b >= 50.0 => ("Fair", Color32::from_rgb(0xe6, 0xd0, 0x4a), 0.55),
        b if b >= 28.0 => ("Weak", WARN, 0.3),
        _ => ("Very weak", Color32::from_rgb(0xff, 0x5a, 0x5a), 0.12),
    }
}

fn meter(ui: &mut egui::Ui, fraction: f32, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 8.0), Sense::hover());
    ui.painter().rect_filled(rect, 4.0, faded(MUTED, 0.18));
    let fill = Rect::from_min_size(
        rect.min,
        vec2(rect.width() * fraction.clamp(0.02, 1.0), rect.height()),
    );
    ui.painter().rect_filled(fill, 4.0, color);
}

fn check_row(ui: &mut egui::Ui, ok: bool, text: &str) {
    ui.horizontal(|ui| {
        ui.add(if ok { Icon::Check } else { Icon::Cross }.image(16.0));
        ui.label(RichText::new(text).color(if ok { TEXT } else { MUTED }));
    });
}

fn ago(t: Instant) -> String {
    let s = t.elapsed().as_secs();
    match s {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", s / 60),
        _ => format!("{} h ago", s / 3600),
    }
}

/// Opens a website in the default browser. Only http(s): no file paths or other schemes.
fn launch_url(url: &str) -> Result<(), String> {
    let u = url.trim();
    let lower = u.to_lowercase();
    if !(lower.starts_with("https://") || lower.starts_with("http://")) {
        return Err("Only http:// and https:// addresses can be opened.".into());
    }
    #[cfg(target_os="linux")]
    return crate::linux::open_url(u);
    #[cfg(windows)]
    {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let wide = |s: &str| {
        s.encode_utf16()
            .chain(std::iter::once(0))
            .collect::<Vec<u16>>()
    };
    let (verb, target) = (wide("open"), wide(u));
    let r = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if r as isize > 32 {
        Ok(())
    } else {
        Err("Windows could not open that address.".into())
    }
    }
}

/// Everything the details module shows, copied out of the entry so the vault can be edited
/// while the module is drawn.
struct Detail {
    id: EntryId,
    title: String,
    username: String,
    password: Zeroizing<String>,
    url: String,
    form: EntryForm,
    created: Option<String>,
    modified: Option<String>,
    favorite: bool,
    archived: bool,
    tags: Vec<String>,
    category: Option<(Category, Source)>,
}

enum DetailAction {
    Copy(&'static str, Zeroizing<String>),
    Launch,
    ToggleFavorite,
    Edit,
    ToggleArchive,
    Delete,
    AddTag(String),
    RemoveTag(String),
    FilterTag(String),
    SetCategory(Option<Category>),
    ShowSecurity,
    FetchIcon,
}

impl Session {
    /// The Floating Modular layout inside `area` (the window minus its outer margin).
    pub fn show_floating(
        &mut self,
        ui: &mut egui::Ui,
        area: Rect,
        clip_left: Option<Duration>,
    ) -> SessionAction {
        let ctx = ui.ctx().clone();
        let (mut action, find) = self.frame_logic(&ctx);
        if matches!(action, SessionAction::Lock) {
            return action;
        }
        let health = self.health();

        let top = Rect::from_min_size(area.min, vec2(area.width(), chrome::TOP_BAR_H));
        chrome::top_bar(ui, top, |ui| self.top_bar_center(ui, find, &health));

        let mut body = Rect::from_min_max(pos2(area.left(), top.bottom() + GAP), area.max);
        // A backup opened as a vault says so above everything else.
        if let Some(banner) = self.read_only_banner() {
            let strip = Rect::from_min_size(body.min, vec2(body.width(), 34.0));
            let painter = ui.painter().clone();
            chrome::module_body(&painter, strip, 0.0);
            painter.rect_filled(strip, 0.0, faded(WARN, 0.10));
            painter.rect_stroke(
                strip,
                0.0,
                Stroke::new(1.0, faded(WARN, 0.6)),
                egui::StrokeKind::Inside,
            );
            child(ui, strip.shrink2(vec2(14.0, 8.0)), |ui| {
                ui.add(egui::Label::new(RichText::new(banner).color(WARN)).truncate());
            });
            body.min.y = strip.bottom() + GAP;
        }
        let w = body.width();
        let nav_w = if w < 1100.0 { 204.0 } else { 236.0 };
        let sec_w = if self.show_security && w >= 1240.0 {
            320.0
        } else {
            0.0
        };
        let nav = Rect::from_min_size(body.min, vec2(nav_w, body.height()));
        let left = nav.right() + GAP;
        let right = if sec_w > 0.0 {
            body.right() - sec_w - GAP
        } else {
            body.right()
        };

        if let Some(a) = self.rail_module(ui, nav, clip_left) {
            action = a;
        }
        let main = Rect::from_min_max(pos2(left, body.top()), pos2(right, body.bottom()));
        match self.page {
            Page::Generator => {
                if let Some(a) = self.generator_page(ui, main) {
                    action = a;
                }
            }
            Page::Settings => {
                if let Some(a) = self.settings_page(ui, main) {
                    action = a;
                }
            }
            _ => {
                let list_w = (main.width() * 0.38).clamp(250.0, 340.0);
                let list = Rect::from_min_max(main.min, pos2(main.left() + list_w, main.bottom()));
                let detail = Rect::from_min_max(pos2(list.right() + GAP, main.top()), main.max);
                self.list_module(ui, list, &health);
                if let Some(a) = self.detail_module(ui, detail) {
                    action = a;
                }
            }
        }
        if sec_w > 0.0 {
            let sec = Rect::from_min_max(pos2(body.right() - sec_w, body.top()), body.max);
            let act_h = (sec.height() * 0.34).clamp(170.0, 290.0);
            let overview =
                Rect::from_min_max(sec.min, pos2(sec.right(), sec.bottom() - act_h - GAP));
            let activity = Rect::from_min_max(pos2(sec.left(), overview.bottom() + GAP), sec.max);
            if let Some(a) = self.security_module(ui, overview, &health) {
                action = a;
            }
            self.activity_module(ui, activity);
        }
        self.toast(ui, main);
        self.editor_window(&ctx);
        self.backups_dialogs(&ctx);
        if let Some(a) = self.confirm_dialog(&ctx) {
            action = a;
        }
        self.key_change_modal(&ctx);
        action
    }

    /// Title bar centre: page tabs, then search with its options.
    fn top_bar_center(
        &mut self,
        ui: &mut egui::Ui,
        focus: bool,
        health: &std::collections::HashMap<EntryId, Vec<Issue>>,
    ) {
        let db = &self.vault.db;
        let (total, favorites) = db
            .iter_all_entries()
            .filter(|e| !organize::is_archived(e) && !organize::in_recycle_bin(e))
            .fold((0, 0), |(t, f), e| {
                (t + 1, f + usize::from(organize::is_favorite(&e)))
            });
        let tabs = [
            (Page::Vault, Icon::Vault, "Vault", Some(total)),
            (Page::Favorites, Icon::Star, "Favorites", Some(favorites)),
            (Page::Tags, Icon::Tag, "Tags", None),
            (Page::Generator, Icon::Generate, "Generator", None),
            (
                Page::Watchtower,
                Icon::Attention,
                "Watchtower",
                Some(health.len()),
            ),
            (Page::Settings, Icon::Settings, "Settings", None),
        ];
        // Icon-only tabs (named for screen readers and tooltips) when space is tight.
        let compact = ui.available_width() < 900.0;
        for (page, icon, label, badge) in tabs {
            let selected = self.page == page && self.search.trim().is_empty();
            if tab_button(ui, selected, icon, label, badge, compact).clicked() {
                self.page = page;
                self.search.clear();
                self.selected = None;
            }
        }
        ui.add_space(12.0);
        ui.add(Icon::Search.image(17.0));
        let width = (ui.available_width() - 44.0).clamp(150.0, 520.0);
        let resp = ui.add(
            egui::TextEdit::singleline(&mut self.search)
                .hint_text("Search…  (Ctrl+K)")
                .desired_width(width)
                .margin(vec2(10.0, 7.0)),
        );
        if focus {
            resp.request_focus();
        }
        // The edit gives up focus on Esc in the same frame, so check both.
        if (resp.has_focus() || resp.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Escape))
        {
            self.search.clear();
        }
        if resp.changed()
            && !self.search.is_empty()
            && matches!(self.page, Page::Generator | Page::Settings)
        {
            self.page = Page::Vault;
        }
        if self.search.is_empty() {
            // "Show" for email-domain matches applies to one search, not forever.
            self.search_opts.include_email_domains = false;
        }
        let options = ui.menu_image_button(Icon::More.image(16.0), |ui| {
            ui.label(RichText::new("Search also in").small().color(MUTED));
            ui.checkbox(&mut self.search_opts.include_notes, "Notes");
            ui.checkbox(&mut self.search_opts.include_archived, "Archived entries");
            ui.checkbox(
                &mut self.search_opts.include_email_domains,
                "Email domains (e.g. @outlook.com)",
            );
        });
        options.response.on_hover_text("Search options");
    }

    // --- Category rail ------------------------------------------------------------------------

    /// Left rail: library views, smart categories and folders (or tags on the Tags page), with
    /// the vault state card at the bottom.
    fn rail_module(
        &mut self,
        ui: &mut egui::Ui,
        r: Rect,
        clip_left: Option<Duration>,
    ) -> Option<SessionAction> {
        chrome::module(&ui.painter().clone(), r);
        let mut action = None;
        let inner = r.shrink2(vec2(10.0, PAD));
        let card_h = 128.0;
        let list_rect = Rect::from_min_max(
            inner.min,
            pos2(inner.right(), inner.bottom() - card_h - 8.0),
        );
        child(ui, list_rect, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("rail")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    if self.page == Page::Tags {
                        self.rail_tags(ui);
                    } else {
                        self.rail_library(ui);
                    }
                });
        });

        // Vault state card: real state only (file, saved or not, passkey or password).
        let card = Rect::from_min_max(pos2(inner.left(), inner.bottom() - card_h), inner.max);
        ui.painter().rect_filled(card, 10.0, BOX_BG);
        ui.painter().rect_stroke(
            card,
            10.0,
            Stroke::new(1.0, faded(CYAN, 0.18)),
            egui::StrokeKind::Inside,
        );
        let has_passkey = envelope::path_for(self.vault.path()).exists();
        let dirty = self.vault.is_dirty();
        let read_only = self.vault.is_read_only();
        child(ui, card.shrink(10.0), |ui| {
            ui.horizontal(|ui| {
                ui.add(Icon::Unlock.image(18.0));
                ui.label(
                    RichText::new(if read_only {
                        "BACKUP, READ-ONLY"
                    } else {
                        "VAULT UNLOCKED"
                    })
                    .size(11.0)
                    .color(if read_only { WARN } else { CYAN })
                    .strong(),
                );
            });
            let name = self
                .vault
                .path()
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            ui.add(egui::Label::new(RichText::new(name).color(TEXT)).truncate());
            let idle_left = super::IDLE_LOCK.saturating_sub(self.last_activity.elapsed());
            let status = match clip_left {
                Some(c) => format!("Clipboard clears in {}s", c.as_secs() + 1),
                None => format!(
                    "{} · {} · lock in {}:{:02}",
                    if dirty { "Unsaved" } else { "Saved" },
                    if has_passkey { "Passkey" } else { "Password" },
                    idle_left.as_secs() / 60,
                    idle_left.as_secs() % 60
                ),
            };
            ui.label(
                RichText::new(status)
                    .small()
                    .color(if dirty { WARN } else { MUTED }),
            );
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        dirty && !read_only,
                        egui::Button::image_and_text(Icon::Save.image(14.0), "Save"),
                    )
                    .on_hover_text("Ctrl+S")
                    .on_disabled_hover_text(if read_only {
                        super::READ_ONLY_SAVE
                    } else {
                        "No unsaved changes"
                    })
                    .clicked()
                {
                    self.save_clicked();
                }
                if ui
                    .add(egui::Button::image_and_text(Icon::Lock.image(14.0), "Lock"))
                    .on_hover_text("Ctrl+L")
                    .clicked()
                {
                    action = Some(self.lock_clicked());
                }
            });
        });
        action
    }

    fn rail_select(&mut self, view: View) {
        self.page = Page::Vault;
        self.view = view;
        self.search.clear();
        self.selected = None;
    }

    fn rail_library(&mut self, ui: &mut egui::Ui) {
        let db = &self.vault.db;
        let now = vault_core::keepass::db::Times::now();
        let mut total = 0;
        let mut recent = 0;
        let mut archived = 0;
        let mut counts: std::collections::HashMap<Category, usize> =
            std::collections::HashMap::new();
        for e in db.iter_all_entries() {
            if organize::in_recycle_bin(&e) {
                continue;
            }
            if organize::is_archived(&e) {
                archived += 1;
                continue;
            }
            total += 1;
            if e.times
                .last_modification
                .is_some_and(|t| (now - t).num_days() <= super::RECENT_DAYS)
            {
                recent += 1;
            }
            if let Some((c, _)) = self.categories.get(&e.id()) {
                *counts.entry(*c).or_default() += 1;
            }
        }
        let on_vault = self.page == Page::Vault && self.search.trim().is_empty();

        rail_header(ui, "LIBRARY");
        for (view, icon, label, n) in [
            (View::All, Icon::Vault, "All items", total),
            (View::Recent, Icon::History, "Recently changed", recent),
            (View::Archived, Icon::Archive, "Archived", archived),
        ] {
            if rail_row(
                ui,
                on_vault && self.view == view,
                0.0,
                Lead::Icon(icon),
                label,
                Some(n),
                false,
            )
            .clicked()
            {
                self.rail_select(view);
            }
        }

        rail_header(ui, "CATEGORIES");
        for area in Area::ALL {
            let cats: Vec<(Category, usize)> = Category::ALL
                .into_iter()
                .filter(|c| c.area() == area)
                .filter_map(|c| counts.get(&c).map(|n| (c, *n)))
                .collect();
            let area_total: usize = cats.iter().map(|(_, n)| n).sum();
            if area_total == 0 {
                continue;
            }
            let open_id = egui::Id::new(("rail_area_open", area.label()));
            let mut open = ui.data(|d| d.get_temp::<bool>(open_id)).unwrap_or(true);
            let color = area_color(area);
            let resp = rail_row(
                ui,
                on_vault && self.view == View::Area(area),
                0.0,
                Lead::Chevron(open, color),
                area.label(),
                Some(area_total),
                true,
            );
            // The chevron toggles; the rest of the row selects the whole area.
            let chevron = Rect::from_min_size(resp.rect.left_top(), vec2(26.0, resp.rect.height()));
            let toggle = ui.interact(chevron, open_id.with("toggle"), Sense::click());
            toggle.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    true,
                    format!(
                        "{} {}",
                        if open { "Collapse" } else { "Expand" },
                        area.label()
                    ),
                )
            });
            if toggle.clicked() {
                open = !open;
                ui.data_mut(|d| d.insert_temp(open_id, open));
            } else if resp.clicked() {
                self.rail_select(View::Area(area));
                ui.data_mut(|d| d.insert_temp(open_id, true));
            }
            if open {
                for (c, n) in cats {
                    if rail_row(
                        ui,
                        on_vault && self.view == View::Category(c),
                        18.0,
                        Lead::Dot(faded(color, 0.85)),
                        c.label(),
                        Some(n),
                        false,
                    )
                    .clicked()
                    {
                        self.rail_select(View::Category(c));
                    }
                }
            }
        }

        // Folders: the KeePass group tree, collapsed by default.
        let folders_id = egui::Id::new("rail_folders_open");
        let folders_open = ui.data(|d| d.get_temp::<bool>(folders_id)).unwrap_or(false);
        ui.add_space(8.0);
        let resp = rail_row(
            ui,
            false,
            0.0,
            Lead::Chevron(folders_open, MUTED),
            "FOLDERS",
            None,
            false,
        );
        if resp.clicked() {
            ui.data_mut(|d| d.insert_temp(folders_id, !folders_open));
        }
        if folders_open {
            let tree = super::build_tree(self.vault.db.root());
            let bin = self.vault.db.recycle_bin().map(|g| g.id());
            let mut flat = Vec::new();
            flatten_tree(&tree, 0, &mut flat);
            for (id, name, active, depth) in flat {
                let lead = if Some(id) == bin {
                    Lead::Icon(Icon::RecycleBin)
                } else {
                    Lead::None
                };
                let selected = on_vault && self.view == View::Group(id);
                if rail_row(
                    ui,
                    selected,
                    10.0 + depth as f32 * 12.0,
                    lead,
                    &name,
                    Some(active),
                    false,
                )
                .clicked()
                {
                    self.rail_select(View::Group(id));
                }
            }
        }
    }

    fn rail_tags(&mut self, ui: &mut egui::Ui) {
        rail_header(ui, "TAGS");
        let tags = organize::tag_counts(&self.vault.db);
        let tagged: usize = tags.iter().map(|(_, n)| n).sum();
        if rail_row(
            ui,
            self.tag_filter.is_none(),
            0.0,
            Lead::Icon(Icon::Tag),
            "All tagged",
            Some(tagged),
            false,
        )
        .clicked()
        {
            self.tag_filter = None;
            self.selected = None;
        }
        if tags.is_empty() {
            ui.add_space(8.0);
            ui.label(
                RichText::new("No tags yet. Add one from an entry's details with “Add tag”.")
                    .small()
                    .color(MUTED),
            );
        }
        for (tag, n) in tags {
            let on = self
                .tag_filter
                .as_deref()
                .is_some_and(|t| t.eq_ignore_ascii_case(&tag));
            if rail_row(ui, on, 0.0, Lead::Dot(CYAN), &tag, Some(n), false).clicked() {
                self.tag_filter = Some(tag);
                self.selected = None;
            }
        }
    }

    // --- Vault list ---------------------------------------------------------------------------

    fn filter_label(&self) -> String {
        match (self.page, self.view) {
            (Page::Favorites, _) => "Favorites".into(),
            (Page::Tags, _) => self.tag_filter.clone().unwrap_or_else(|| "All tags".into()),
            (Page::Watchtower, _) => "Needs attention".into(),
            (_, View::All) => "All items".into(),
            (_, View::Recent) => "Recently changed".into(),
            (_, View::Archived) => "Archived".into(),
            (_, View::Attention) => "Needs attention".into(),
            (_, View::Area(a)) => a.label().into(),
            (_, View::Category(c)) => c.label().into(),
            (_, View::Group(g)) => self
                .vault
                .db
                .group(g)
                .map(|g| g.name.clone())
                .unwrap_or_default(),
        }
    }

    fn page_rows(
        &self,
        health: &std::collections::HashMap<EntryId, Vec<Issue>>,
    ) -> (Vec<super::Row>, bool) {
        if !self.search.trim().is_empty() {
            let (rows, _, ranked) = self.rows(health);
            return (rows, ranked);
        }
        let db = &self.vault.db;
        let active = |e: &vault_core::keepass::db::EntryRef<'_>| {
            !organize::is_archived(e) && !organize::in_recycle_bin(e)
        };
        match self.page {
            Page::Favorites => (
                db.iter_all_entries()
                    .filter(active)
                    .filter(|e| organize::is_favorite(e))
                    .map(|e| self.row(&e, String::new(), false))
                    .collect(),
                false,
            ),
            Page::Watchtower => {
                let issues = |id: &EntryId| {
                    health
                        .get(id)
                        .map(|v| v.iter().map(|i| i.label()).collect::<Vec<_>>().join(", "))
                        .unwrap_or_default()
                };
                (
                    db.iter_all_entries()
                        .filter(|e| health.contains_key(&e.id()))
                        .map(|e| self.row(&e, issues(&e.id()), false))
                        .collect(),
                    false,
                )
            }
            Page::Tags => {
                let want = self.tag_filter.as_ref().map(|t| t.to_lowercase());
                (
                    db.iter_all_entries()
                        .filter(active)
                        .filter(|e| {
                            let mut tags = organize::user_tags(e);
                            match &want {
                                Some(w) => tags.any(|t| t.to_lowercase() == *w),
                                None => tags.next().is_some(),
                            }
                        })
                        .map(|e| self.row(&e, String::new(), false))
                        .collect(),
                    false,
                )
            }
            _ => {
                let (rows, _, ranked) = self.rows(health);
                (rows, ranked)
            }
        }
    }

    fn list_module(
        &mut self,
        ui: &mut egui::Ui,
        r: Rect,
        health: &std::collections::HashMap<EntryId, Vec<Issue>>,
    ) {
        chrome::module(&ui.painter().clone(), r);
        let (mut rows, ranked) = self.page_rows(health);
        if !ranked {
            match self.sort {
                Sort::Title => rows.sort_by_key(|r| r.title.to_lowercase()),
                Sort::Modified => rows.sort_by_key(|r| std::cmp::Reverse(r.modified)),
            }
        }
        let title = if !self.search.trim().is_empty() {
            "Search".to_owned()
        } else {
            match self.page {
                Page::Favorites => "Favorites".to_owned(),
                Page::Tags => self.tag_filter.clone().unwrap_or_else(|| "Tags".to_owned()),
                Page::Watchtower => "Watchtower".to_owned(),
                _ => self.filter_label(),
            }
        };
        let hidden = if self.search.trim().is_empty() {
            None
        } else {
            let report =
                vault_core::search::search_report(&self.vault.db, &self.search, self.search_opts);
            (report.hidden_email_matches > 0)
                .then_some((report.hidden_email_matches, report.hidden_domain))
        };
        child(ui, r.shrink(PAD), |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.add(
                        egui::Label::new(RichText::new(&title).size(20.0).color(TEXT)).truncate(),
                    );
                    let what = if ranked {
                        "results, best match first"
                    } else {
                        "items"
                    };
                    ui.label(
                        RichText::new(format!("{} {what}", rows.len()))
                            .small()
                            .color(MUTED),
                    );
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if icon_button(ui, Icon::Plus, 18.0, "New entry (Ctrl+N)", true).clicked() {
                        self.start_new_entry();
                    }
                });
            });
            ui.add_space(8.0);

            if self.page == Page::Watchtower && self.search.trim().is_empty() {
                let count = |want: Issue| health.values().filter(|v| v.contains(&want)).count();
                let breached = if self.breached.is_some() {
                    format!(" · {} breached", count(Issue::Breached))
                } else {
                    String::new()
                };
                ui.label(
                    RichText::new(format!(
                        "{} reused · {} weak · {} over a year old{breached}",
                        count(Issue::Reused),
                        count(Issue::Weak),
                        count(Issue::Old)
                    ))
                    .small()
                    .color(WARN),
                );
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            self.breach_job.is_none(),
                            egui::Button::image_and_text(
                                Icon::Security.image(14.0),
                                if self.breached.is_some() {
                                    "Check breaches again"
                                } else {
                                    "Check for breached passwords"
                                },
                            )
                            .small(),
                        )
                        .on_hover_text(
                            "Have I Been Pwned, k-anonymity: only the first 5 characters of each password's SHA-1 hash are sent",
                        )
                        .clicked()
                    {
                        self.start_breach_check(ui.ctx());
                    }
                    if self.breach_job.is_some() {
                        ui.spinner();
                        ui.ctx().request_repaint_after(Duration::from_millis(200));
                    }
                });
                ui.label(RichText::new(self.breach_status()).small().color(MUTED));
            } else if !ranked {
                ui.horizontal(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let (label, next) = match self.sort {
                            Sort::Title => ("A–Z", Sort::Modified),
                            Sort::Modified => ("Recent", Sort::Title),
                        };
                        if ui
                            .add(
                                egui::Button::image_and_text(Icon::SortDown.image(13.0), label)
                                    .frame(false),
                            )
                            .on_hover_text("Change sort order")
                            .clicked()
                        {
                            self.sort = next;
                        }
                    });
                });
            }
            ui.add_space(6.0);

            if rows.is_empty() {
                ui.add_space(24.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new(if ranked {
                            "No matches. Try “notes” or “archived”."
                        } else {
                            "Nothing here yet."
                        })
                        .color(MUTED),
                    );
                });
                return;
            }
            if let Some((n, domain)) = &hidden {
                ui.horizontal_wrapped(|ui| {
                    let domain = domain
                        .as_deref()
                        .map(|d| format!("an @{d}"))
                        .unwrap_or_else(|| "that".into());
                    ui.label(
                        RichText::new(format!(
                            "{n} more entr{} only match because {} use {domain} email.",
                            if *n == 1 { "y" } else { "ies" },
                            if *n == 1 { "it uses" } else { "they" },
                        ))
                        .small()
                        .color(MUTED),
                    );
                    if ui.small_button("Show").clicked() {
                        self.search_opts.include_email_domains = true;
                    }
                });
                ui.add_space(4.0);
            }
            let mut clicked = None;
            let mut opened = None;
            let icons: std::collections::HashMap<EntryId, Option<egui::ImageSource<'static>>> =
                rows.iter().map(|r| (r.id, self.entry_icon(r.id))).collect();
            egui::ScrollArea::vertical()
                .id_salt("float_list")
                .auto_shrink([false, false])
                .show_rows(ui, 54.0, rows.len(), |ui, range| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    for row in &rows[range] {
                        let selected = self.selected == Some(row.id);
                        let label = if row.title.is_empty() {
                            "(untitled)"
                        } else {
                            row.title.as_str()
                        };
                        let (rect, resp) = row_response(ui, 50.0, selected, label);
                        let (area, fav) = match self.vault.db.entry(row.id) {
                            Some(e) => (
                                self.categories
                                    .get(&row.id)
                                    .map(|(c, _)| c.area())
                                    .unwrap_or(Area::Other),
                                organize::is_favorite(&e),
                            ),
                            None => (Area::Other, false),
                        };
                        let painter = ui.painter();
                        let tile = Rect::from_min_size(
                            rect.left_top() + vec2(10.0, 7.0),
                            Vec2::splat(36.0),
                        );
                        entry_tile(
                            ui,
                            tile,
                            icons.get(&row.id).cloned().flatten(),
                            label,
                            area_color(area),
                        );
                        let text_x = tile.right() + 12.0;
                        let max_w = rect.right() - text_x - if fav { 34.0 } else { 10.0 };
                        let title_g = painter.layout(
                            label.to_owned(),
                            egui::FontId::proportional(14.5),
                            if row.dim { MUTED } else { TEXT },
                            f32::INFINITY,
                        );
                        let clip = Rect::from_min_max(
                            pos2(text_x, rect.top()),
                            pos2(text_x + max_w, rect.bottom()),
                        );
                        painter.with_clip_rect(clip).galley(
                            pos2(text_x, rect.top() + 8.0),
                            title_g,
                            TEXT,
                        );
                        let sub =
                            if !row.note.is_empty() && (self.page == Page::Watchtower || ranked) {
                                row.note.clone()
                            } else if row.username.is_empty() {
                                row.host.clone()
                            } else {
                                row.username.clone()
                            };
                        let sub_g = painter.layout(
                            sub,
                            egui::FontId::proportional(12.0),
                            MUTED,
                            f32::INFINITY,
                        );
                        painter.with_clip_rect(clip).galley(
                            pos2(text_x, rect.top() + 27.0),
                            sub_g,
                            MUTED,
                        );
                        if fav {
                            Icon::StarFilled.image(16.0).paint_at(
                                ui,
                                Rect::from_center_size(
                                    pos2(rect.right() - 20.0, rect.center().y),
                                    Vec2::splat(16.0),
                                ),
                            );
                        }
                        if resp.clicked() {
                            clicked = Some(row.id);
                        }
                        if resp.double_clicked() {
                            opened = Some(row.id);
                        }
                    }
                });
            if let Some(id) = clicked {
                self.selected = Some(id);
                self.reveal = false;
                self.reveal_notes = false;
                self.reveal_fields = false;
                self.reveal_previous = false;
            }
            if let Some(id) = opened {
                self.start_edit(id);
            }
        });
    }

    // --- Details ------------------------------------------------------------------------------

    fn detail_data(&self, id: EntryId) -> Option<Detail> {
        let e = self.vault.db.entry(id)?;
        Some(Detail {
            id,
            title: e.get_title().unwrap_or("").to_owned(),
            username: e.get_username().unwrap_or("").to_owned(),
            password: Zeroizing::new(e.get_password().unwrap_or("").to_owned()),
            url: e.get_url().unwrap_or("").to_owned(),
            form: EntryForm::from_entry(&e),
            created: e.times.creation.map(|t| t.format("%b %-d, %Y").to_string()),
            modified: e
                .times
                .last_modification
                .map(|t| t.format("%b %-d, %Y").to_string()),
            favorite: organize::is_favorite(&e),
            archived: organize::is_archived(&e),
            tags: organize::user_tags(&e).cloned().collect(),
            category: self.categories.get(&id).copied(),
        })
    }

    fn detail_module(&mut self, ui: &mut egui::Ui, r: Rect) -> Option<SessionAction> {
        chrome::module(&ui.painter().clone(), r);
        let Some(d) = self.selected.and_then(|id| self.detail_data(id)) else {
            child(ui, r.shrink(PAD), |ui| {
                ui.add_space(r.height() * 0.35);
                ui.vertical_centered(|ui| {
                    ui.add(Icon::Vault.image(40.0));
                    ui.label(RichText::new("Select an item to see its details").color(MUTED));
                });
            });
            self.selected = None;
            return None;
        };
        let mut act: Option<DetailAction> = None;
        let icon = self.entry_icon(d.id);
        let shown_title = if d.title.is_empty() {
            "(untitled)"
        } else {
            d.title.as_str()
        };
        let area = d.category.map(|(c, _)| c.area()).unwrap_or(Area::Other);

        child(ui, r.shrink(PAD + 2.0), |ui| {
            egui::ScrollArea::vertical()
                .id_salt("float_detail")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    // Header: tile, title, category, star, more menu.
                    ui.horizontal(|ui| {
                        let (tile, _) = ui.allocate_exact_size(Vec2::splat(58.0), Sense::hover());
                        entry_tile(ui, tile, icon.clone(), shown_title, area_color(area));
                        ui.vertical(|ui| {
                            ui.label(RichText::new(shown_title).size(22.0).color(TEXT));
                            let sub = match d.category {
                                Some((c, _)) => format!("{} · {}", c.area().label(), c.label()),
                                None => String::new(),
                            };
                            ui.label(RichText::new(sub).color(MUTED));
                        });
                        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                            ui.menu_image_button(Icon::More.image(18.0), |ui| {
                                if ui
                                    .add(egui::Button::image_and_text(
                                        Icon::Edit.image(14.0),
                                        "Edit…",
                                    ))
                                    .clicked()
                                {
                                    act = Some(DetailAction::Edit);
                                    ui.close();
                                }
                                let (icon, text) = if d.archived {
                                    (Icon::Restore, "Restore")
                                } else {
                                    (Icon::Archive, "Archive")
                                };
                                if ui
                                    .add(egui::Button::image_and_text(icon.image(14.0), text))
                                    .clicked()
                                {
                                    act = Some(DetailAction::ToggleArchive);
                                    ui.close();
                                }
                                if ui
                                    .add(egui::Button::image_and_text(
                                        Icon::Delete.image(14.0),
                                        "Delete…",
                                    ))
                                    .clicked()
                                {
                                    act = Some(DetailAction::Delete);
                                    ui.close();
                                }
                                if ui
                                    .add(egui::Button::image_and_text(
                                        Icon::Download.image(14.0),
                                        "Download site icon",
                                    ))
                                    .on_hover_text(
                                        "Fetches this site's logo directly from the site and saves it in the entry",
                                    )
                                    .clicked()
                                {
                                    act = Some(DetailAction::FetchIcon);
                                    ui.close();
                                }
                                ui.separator();
                                ui.menu_button("Category", |ui| {
                                    let manual = matches!(d.category, Some((_, Source::Manual)));
                                    if ui.selectable_label(!manual, "Automatic").clicked() {
                                        act = Some(DetailAction::SetCategory(None));
                                        ui.close();
                                    }
                                    for area in Area::ALL {
                                        ui.label(RichText::new(area.label()).small().color(MUTED));
                                        for c in
                                            Category::ALL.into_iter().filter(|c| c.area() == area)
                                        {
                                            let on =
                                                manual && d.category.is_some_and(|(x, _)| x == c);
                                            if ui.selectable_label(on, c.label()).clicked() {
                                                act = Some(DetailAction::SetCategory(Some(c)));
                                                ui.close();
                                            }
                                        }
                                    }
                                });
                                if !self.show_security && ui.button("Show security panel").clicked()
                                {
                                    act = Some(DetailAction::ShowSecurity);
                                    ui.close();
                                }
                            });
                            let star = if d.favorite {
                                Icon::StarFilled
                            } else {
                                Icon::Star
                            };
                            let tip = if d.favorite {
                                "Remove from favourites"
                            } else {
                                "Add to favourites"
                            };
                            if icon_button(ui, star, 20.0, tip, false).clicked() {
                                act = Some(DetailAction::ToggleFavorite);
                            }
                        });
                    });
                    if d.archived {
                        ui.label(
                            RichText::new(
                                "Archived: kept for reference, hidden from search and lists.",
                            )
                            .italics()
                            .color(MUTED),
                        );
                    }
                    ui.add_space(10.0);

                    // Credential fields.
                    egui::Frame::new()
                        .fill(BOX_BG)
                        .corner_radius(10.0)
                        .inner_margin(12.0)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            self.field_row(
                                ui,
                                "Username",
                                &d.username,
                                false,
                                |ui, a| {
                                    if small_button(ui, Icon::Copy, "Copy")
                                        .on_hover_text("Ctrl+B")
                                        .clicked()
                                    {
                                        *a = Some(DetailAction::Copy(
                                            "Username",
                                            Zeroizing::new(d.username.clone()),
                                        ));
                                    }
                                },
                                &mut act,
                            );
                            let reveal = self.reveal;
                            let mut toggle = false;
                            self.field_row(
                                ui,
                                "Password",
                                &d.password,
                                !reveal,
                                |ui, a| {
                                    if small_button(ui, Icon::Copy, "Copy")
                                        .on_hover_text("Ctrl+C")
                                        .clicked()
                                    {
                                        *a = Some(DetailAction::Copy(
                                            "Password",
                                            d.password.clone(),
                                        ));
                                    }
                                    let (icon, tip) = if reveal {
                                        (Icon::Hide, "Hide")
                                    } else {
                                        (Icon::Show, "Show")
                                    };
                                    if icon_button(ui, icon, 15.0, tip, true).clicked() {
                                        toggle = true;
                                    }
                                },
                                &mut act,
                            );
                            if toggle {
                                self.reveal = !self.reveal;
                            }
                            self.field_row(
                                ui,
                                "Website",
                                &d.url,
                                false,
                                |ui, a| {
                                    if !d.url.is_empty() {
                                        if small_button(ui, Icon::Copy, "Copy").clicked() {
                                            *a = Some(DetailAction::Copy(
                                                "Website",
                                                Zeroizing::new(d.url.clone()),
                                            ));
                                        }
                                        if icon_button(
                                            ui,
                                            Icon::Launch,
                                            15.0,
                                            "Open in browser",
                                            true,
                                        )
                                        .clicked()
                                        {
                                            *a = Some(DetailAction::Launch);
                                        }
                                    }
                                },
                                &mut act,
                            );
                            let reveal_fields = self.reveal_fields;
                            let mut toggle_fields = false;
                            for f in &d.form.custom {
                                let label = if f.protected {
                                    format!("{} (protected)", f.name)
                                } else {
                                    f.name.clone()
                                };
                                let masked = f.protected && !reveal_fields;
                                self.field_row(
                                    ui,
                                    &label,
                                    &f.value,
                                    masked,
                                    |ui, a| {
                                        if small_button(ui, Icon::Copy, "Copy").clicked() {
                                            *a = Some(DetailAction::Copy("Field", f.value.clone()));
                                        }
                                        if f.protected
                                            && icon_button(
                                                ui,
                                                if reveal_fields {
                                                    Icon::Hide
                                                } else {
                                                    Icon::Show
                                                },
                                                15.0,
                                                if reveal_fields { "Hide" } else { "Show" },
                                                true,
                                            )
                                            .clicked()
                                        {
                                            toggle_fields = true;
                                        }
                                    },
                                    &mut act,
                                );
                            }
                            if toggle_fields {
                                self.reveal_fields = !self.reveal_fields;
                            }
                        });
                    ui.add_space(12.0);

                    // Notes.
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(if d.form.notes_protected {
                                "Notes (protected)"
                            } else {
                                "Notes"
                            })
                            .color(MUTED),
                        );
                        if d.form.notes_protected && !d.form.notes.is_empty() {
                            let (icon, tip) = if self.reveal_notes {
                                (Icon::Hide, "Hide")
                            } else {
                                (Icon::Show, "Show")
                            };
                            if icon_button(ui, icon, 14.0, tip, false).clicked() {
                                self.reveal_notes = !self.reveal_notes;
                            }
                        }
                    });
                    egui::Frame::new()
                        .fill(FIELD_BG)
                        .corner_radius(8.0)
                        .inner_margin(12.0)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            let text = if d.form.notes.is_empty() {
                                RichText::new("No notes").color(MUTED)
                            } else if d.form.notes_protected && !self.reveal_notes {
                                RichText::new("••••••••••").color(TEXT)
                            } else {
                                RichText::new(d.form.notes.as_str()).color(TEXT)
                            };
                            ui.add(egui::Label::new(text).wrap());
                        });
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(format!(
                            "Created  {}     |     Last updated  {}",
                            d.created.as_deref().unwrap_or("—"),
                            d.modified.as_deref().unwrap_or("—")
                        ))
                        .small()
                        .color(MUTED),
                    );
                    ui.add_space(14.0);

                    // Quick actions.
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .add(
                                egui::Button::image_and_text(
                                    Icon::Copy.image(15.0),
                                    "Copy Username",
                                )
                                .min_size(vec2(0.0, 36.0)),
                            )
                            .clicked()
                        {
                            act = Some(DetailAction::Copy(
                                "Username",
                                Zeroizing::new(d.username.clone()),
                            ));
                        }
                        if ui
                            .add(
                                egui::Button::image_and_text(
                                    Icon::Lock.image(15.0),
                                    "Copy Password",
                                )
                                .min_size(vec2(0.0, 36.0)),
                            )
                            .clicked()
                        {
                            act = Some(DetailAction::Copy("Password", d.password.clone()));
                        }
                        let can_launch = {
                            let l = d.url.trim().to_lowercase();
                            l.starts_with("http://") || l.starts_with("https://")
                        };
                        let launch = egui::Button::image_and_text(
                            Icon::Launch.image(15.0),
                            RichText::new("Launch Site")
                                .color(Color32::from_rgb(0x04, 0x13, 0x1c))
                                .strong(),
                        )
                        .fill(CYAN)
                        .min_size(vec2(0.0, 36.0));
                        if ui.add_enabled(can_launch, launch).clicked() {
                            act = Some(DetailAction::Launch);
                        }
                    });
                    ui.add_space(12.0);

                    // Tags.
                    ui.horizontal_wrapped(|ui| {
                        for t in &d.tags {
                            let chip = ui.add(
                                egui::Button::new(RichText::new(t).small())
                                    .corner_radius(14.0)
                                    .min_size(vec2(0.0, 26.0)),
                            );
                            if chip.clicked() {
                                act = Some(DetailAction::FilterTag(t.clone()));
                            }
                            chip.context_menu(|ui| {
                                if ui.button("Remove tag").clicked() {
                                    act = Some(DetailAction::RemoveTag(t.clone()));
                                    ui.close();
                                }
                            });
                        }
                        match &mut self.tag_input {
                            Some(text) => {
                                let resp = ui.add(
                                    egui::TextEdit::singleline(text)
                                        .hint_text("New tag")
                                        .desired_width(110.0),
                                );
                                // Take focus once when the field opens; Enter then commits, Esc cancels.
                                if text.is_empty() && !resp.has_focus() && !resp.lost_focus() {
                                    resp.request_focus();
                                }
                                if resp.lost_focus() {
                                    let tag = text.trim().to_owned();
                                    if ui.input(|i| i.key_pressed(egui::Key::Enter))
                                        && !tag.is_empty()
                                    {
                                        act = Some(DetailAction::AddTag(tag));
                                    }
                                    self.tag_input = None;
                                }
                            }
                            None => {
                                if ui
                                    .add(
                                        egui::Button::image_and_text(
                                            Icon::Plus.image(12.0),
                                            RichText::new("Add tag").small(),
                                        )
                                        .corner_radius(14.0)
                                        .min_size(vec2(0.0, 26.0)),
                                    )
                                    .clicked()
                                {
                                    self.tag_input = Some(String::new());
                                }
                            }
                        }
                    });
                });
        });

        self.apply_detail_action(ui.ctx().clone(), d, act)
    }

    /// One credential row: label, value box (masked when asked), trailing buttons.
    fn field_row(
        &self,
        ui: &mut egui::Ui,
        label: &str,
        value: &str,
        masked: bool,
        buttons: impl FnOnce(&mut egui::Ui, &mut Option<DetailAction>),
        act: &mut Option<DetailAction>,
    ) {
        ui.horizontal(|ui| {
            ui.allocate_ui_with_layout(
                vec2(96.0, 32.0),
                Layout::left_to_right(Align::Center),
                |ui| {
                    ui.add(egui::Label::new(RichText::new(label).color(MUTED)).truncate());
                },
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                buttons(ui, act);
                let shown = if value.is_empty() {
                    RichText::new("—").color(MUTED)
                } else if masked {
                    RichText::new("••••••••••••••••").color(TEXT)
                } else {
                    RichText::new(value).color(TEXT)
                };
                egui::Frame::new()
                    .fill(FIELD_BG)
                    .corner_radius(7.0)
                    .inner_margin(vec2(10.0, 7.0))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            ui.add(egui::Label::new(shown).truncate());
                        });
                    });
            });
        });
        ui.add_space(4.0);
    }

    fn apply_detail_action(
        &mut self,
        ui_ctx: egui::Context,
        d: Detail,
        act: Option<DetailAction>,
    ) -> Option<SessionAction> {
        let id = d.id;
        match act? {
            DetailAction::Copy(what, value) => {
                return Some(self.copy_logged(what, &d.title, &value));
            }
            DetailAction::Launch => match launch_url(&d.url) {
                Ok(()) => self.log(Icon::Launch, format!("Site launched · {}", d.title)),
                Err(e) => self.error = Some(e),
            },
            DetailAction::ToggleFavorite => {
                if let Some(mut e) = self.vault.db.entry_mut(id) {
                    organize::set_favorite(&mut e, !d.favorite);
                    self.vault.mark_dirty();
                }
            }
            DetailAction::Edit => self.start_edit(id),
            DetailAction::ToggleArchive => {
                if let Some(mut e) = self.vault.db.entry_mut(id) {
                    organize::set_archived(&mut e, !d.archived);
                    self.vault.mark_dirty();
                    self.invalidate_categories();
                    self.log(
                        Icon::Archive,
                        format!(
                            "{} · {}",
                            if d.archived { "Restored" } else { "Archived" },
                            d.title
                        ),
                    );
                }
            }
            DetailAction::Delete => self.confirm = Some(Confirm::Delete(id)),
            DetailAction::AddTag(tag) => {
                if let Some(mut e) = self.vault.db.entry_mut(id)
                    && !e.tags.iter().any(|t| t.eq_ignore_ascii_case(&tag))
                {
                    e.tags.push(tag);
                    self.vault.mark_dirty();
                }
            }
            DetailAction::RemoveTag(tag) => {
                if let Some(mut e) = self.vault.db.entry_mut(id) {
                    e.tags.retain(|t| *t != tag);
                    self.vault.mark_dirty();
                }
            }
            DetailAction::FilterTag(tag) => {
                self.page = Page::Tags;
                self.tag_filter = Some(tag);
                self.search.clear();
            }
            DetailAction::SetCategory(c) => {
                if let Some(mut e) = self.vault.db.entry_mut(id) {
                    categorize::set_manual_category(&mut e, c);
                    self.vault.mark_dirty();
                    self.invalidate_categories();
                }
            }
            DetailAction::ShowSecurity => self.show_security = true,
            DetailAction::FetchIcon => self.start_icon_download(&ui_ctx, Some(id)),
        }
        None
    }

    // --- Security column ----------------------------------------------------------------------

    fn security_module(
        &mut self,
        ui: &mut egui::Ui,
        r: Rect,
        health: &std::collections::HashMap<EntryId, Vec<Issue>>,
    ) -> Option<SessionAction> {
        chrome::module(&ui.painter().clone(), r);
        let mut copy_code: Option<(String, Zeroizing<String>)> = None;
        child(ui, r.shrink(PAD), |ui| {
            ui.horizontal(|ui| {
                section_title(ui, Icon::Security, "Security Overview");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .add(egui::Button::new(RichText::new("Hide").small()).frame(false))
                        .on_hover_text("Hide this panel")
                        .clicked()
                    {
                        self.show_security = false;
                    }
                });
            });
            ui.add_space(8.0);
            let Some(e) = self.selected.and_then(|id| self.vault.db.entry(id)) else {
                let total = self.categories.len();
                ui.label(
                    RichText::new(format!(
                        "{} entries need attention out of {total}.",
                        health.len()
                    ))
                    .color(TEXT),
                );
                ui.label(
                    RichText::new("Select an entry to see its password health.")
                        .small()
                        .color(MUTED),
                );
                return;
            };
            egui::ScrollArea::vertical()
                .id_salt("float_security")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let pw = e.get_password().unwrap_or("");
                    let issues = health.get(&e.id()).cloned().unwrap_or_default();
                    if pw.is_empty() {
                        ui.label(RichText::new("No password stored for this entry.").color(MUTED));
                    } else {
                        let bits = organize::estimate_bits(pw);
                        let (label, color, frac) = strength_label(bits);
                        egui::Frame::new()
                            .fill(BOX_BG)
                            .corner_radius(10.0)
                            .inner_margin(12.0)
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    let good = issues.is_empty();
                                    ui.add(
                                        if good { Icon::Check } else { Icon::Attention }
                                            .image(30.0),
                                    );
                                    ui.vertical(|ui| {
                                        let head = if good {
                                            format!("{label} password")
                                        } else {
                                            "Needs attention".to_owned()
                                        };
                                        ui.label(RichText::new(head).size(16.0).color(if good {
                                            GOOD
                                        } else {
                                            WARN
                                        }));
                                        let reused = issues.contains(&Issue::Reused);
                                        let note = if let Some(n) = self.breach_count(pw) {
                                            format!(
                                                "Found in data breaches {n} time{}. Change it.",
                                                if n == 1 { "" } else { "s" }
                                            )
                                        } else if reused {
                                            let n = self
                                                .vault
                                                .db
                                                .iter_all_entries()
                                                .filter(|o| {
                                                    o.get_password() == Some(pw)
                                                        && !organize::is_archived(o)
                                                        && !organize::in_recycle_bin(o)
                                                })
                                                .count();
                                            format!(
                                                "Also used by {} other entr{}",
                                                n - 1,
                                                if n == 2 { "y" } else { "ies" }
                                            )
                                        } else {
                                            "Unique in this vault".to_owned()
                                        };
                                        ui.label(RichText::new(note).small().color(MUTED));
                                    });
                                });
                            });
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Password Strength").color(TEXT));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(RichText::new(label).color(color));
                            });
                        });
                        meter(ui, frac, color);
                        ui.add_space(8.0);
                        let n = pw.chars().count();
                        check_row(ui, n >= 12, &format!("{n} characters"));
                        check_row(
                            ui,
                            pw.chars().any(|c| c.is_lowercase())
                                && pw.chars().any(|c| c.is_uppercase()),
                            "Upper & lower case",
                        );
                        check_row(ui, pw.chars().any(|c| c.is_ascii_digit()), "Numbers");
                        check_row(
                            ui,
                            pw.chars().any(|c| !c.is_alphanumeric()),
                            "Special characters",
                        );
                        if let Some(set) = organize::password_set_at(&e) {
                            let days = (vault_core::keepass::db::Times::now() - set).num_days();
                            check_row(
                                ui,
                                days <= organize::OLD_DAYS,
                                &format!("Changed {} ({days} days ago)", set.format("%b %-d, %Y")),
                            );
                        }
                        let previous = organize::previous_passwords(&e).len();
                        if previous > 0 {
                            ui.label(
                                RichText::new(format!(
                                    "{previous} previous password(s) kept in history"
                                ))
                                .small()
                                .color(MUTED),
                            );
                        }
                    }

                    // Two-factor code from the entry's otp field, if it has one.
                    ui.add_space(10.0);
                    egui::Frame::new()
                        .fill(BOX_BG)
                        .corner_radius(10.0)
                        .inner_margin(12.0)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.add(Icon::Key.image(18.0));
                                ui.label(RichText::new("Two-Factor Authentication").color(TEXT));
                            });
                            match e.get_otp() {
                                Ok(totp) => match totp.value_now() {
                                    Ok(code) => {
                                        let shown = if code.code.len() == 6 {
                                            format!("{} {}", &code.code[..3], &code.code[3..])
                                        } else {
                                            code.code.clone()
                                        };
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                RichText::new(shown)
                                                    .size(28.0)
                                                    .color(CYAN)
                                                    .monospace(),
                                            );
                                            if small_button(ui, Icon::Copy, "Copy").clicked() {
                                                copy_code = Some((
                                                    e.get_title().unwrap_or("").to_owned(),
                                                    Zeroizing::new(code.code.clone()),
                                                ));
                                            }
                                            ui.with_layout(
                                                Layout::right_to_left(Align::Center),
                                                |ui| {
                                                    let (rect, _) = ui.allocate_exact_size(
                                                        Vec2::splat(44.0),
                                                        Sense::hover(),
                                                    );
                                                    let frac = code.valid_for.as_secs_f32()
                                                        / code.period.as_secs_f32().max(1.0);
                                                    let p = ui.painter();
                                                    p.circle_stroke(
                                                        rect.center(),
                                                        18.0,
                                                        Stroke::new(3.0, faded(MUTED, 0.25)),
                                                    );
                                                    let pts: Vec<_> = (0..=40)
                                                        .map(|k| {
                                                            let a = -std::f32::consts::FRAC_PI_2
                                                                + frac
                                                                    * std::f32::consts::TAU
                                                                    * k as f32
                                                                    / 40.0;
                                                            rect.center()
                                                                + vec2(a.cos(), a.sin()) * 18.0
                                                        })
                                                        .collect();
                                                    p.add(egui::Shape::line(
                                                        pts,
                                                        Stroke::new(3.0, CYAN),
                                                    ));
                                                    p.text(
                                                        rect.center(),
                                                        egui::Align2::CENTER_CENTER,
                                                        code.valid_for.as_secs().to_string(),
                                                        egui::FontId::proportional(13.0),
                                                        TEXT,
                                                    );
                                                },
                                            );
                                        });
                                        ui.ctx().request_repaint_after(Duration::from_millis(500));
                                    }
                                    Err(_) => {
                                        ui.label(
                                            RichText::new("The system clock is unavailable.")
                                                .small()
                                                .color(WARN),
                                        );
                                    }
                                },
                                Err(_) => {
                                    ui.label(
                                        RichText::new("No 2FA secret stored for this entry.")
                                            .small()
                                            .color(MUTED),
                                    );
                                }
                            }
                        });
                });
        });
        let (title, code) = copy_code?;
        Some(self.copy_logged("2FA code", &title, &code))
    }

    fn activity_module(&mut self, ui: &mut egui::Ui, r: Rect) {
        chrome::module(&ui.painter().clone(), r);
        child(ui, r.shrink(PAD), |ui| {
            ui.horizontal(|ui| {
                section_title(ui, Icon::Activity, "Recent Activity");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new("this session").small().color(MUTED));
                });
            });
            ui.add_space(6.0);
            if self.activity.is_empty() {
                ui.label(
                    RichText::new(
                        "Nothing yet. Copies, launches and changes made this session appear here.",
                    )
                    .small()
                    .color(MUTED),
                );
                return;
            }
            egui::ScrollArea::vertical()
                .id_salt("float_activity")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for a in self.activity.iter().rev().take(20) {
                        ui.horizontal(|ui| {
                            ui.add(a.icon.image(18.0));
                            ui.vertical(|ui| {
                                ui.add(
                                    egui::Label::new(RichText::new(&a.text).color(TEXT)).truncate(),
                                );
                                ui.label(RichText::new(ago(a.at)).small().color(MUTED));
                            });
                        });
                        ui.add_space(4.0);
                    }
                });
        });
    }

    fn toast(&mut self, ui: &mut egui::Ui, over: Rect) {
        let (text, color) = if let Some(err) = &self.error {
            (err.clone(), Color32::from_rgb(0xff, 0x6b, 0x6b))
        } else if let Some((msg, at)) = &self.notice
            && at.elapsed() < Duration::from_secs(6)
        {
            (msg.clone(), GOOD)
        } else {
            return;
        };
        let galley = ui.painter().layout(
            text,
            egui::FontId::proportional(13.5),
            color,
            (over.width() - 80.0).max(200.0),
        );
        let size = galley.size() + vec2(28.0, 18.0);
        let rect = Rect::from_center_size(
            pos2(over.center().x, over.bottom() - size.y / 2.0 - 14.0),
            size,
        );
        let p = ui.painter();
        p.rect_filled(rect, 10.0, chrome::MODULE_FILL);
        p.rect_stroke(
            rect,
            10.0,
            Stroke::new(1.0, faded(color, 0.6)),
            egui::StrokeKind::Inside,
        );
        p.galley(rect.min + vec2(14.0, 9.0), galley, color);
        if ui
            .interact(rect, egui::Id::new("toast"), Sense::click())
            .clicked()
        {
            self.error = None;
            self.notice = None;
        }
    }

    // --- Pages --------------------------------------------------------------------------------

    fn generator_page(&mut self, ui: &mut egui::Ui, r: Rect) -> Option<SessionAction> {
        chrome::module(&ui.painter().clone(), r);
        let mut action = None;
        child(ui, r.shrink(PAD + 8.0), |ui| {
            section_title(ui, Icon::Generate, "Password Generator");
            ui.label(RichText::new("Uses the operating system's secure random generator. Nothing is saved until you use it in an entry.").small().color(MUTED));
            ui.add_space(14.0);
            egui::Frame::new()
                .fill(FIELD_BG)
                .corner_radius(10.0)
                .inner_margin(16.0)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(
                        egui::Label::new(
                            RichText::new(self.generated.as_str())
                                .size(22.0)
                                .monospace()
                                .color(TEXT),
                        )
                        .wrap(),
                    );
                });
            let bits = self.generator.entropy_bits();
            let (label, color, frac) = strength_label(bits);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{label} · about {bits:.0} bits")).color(color));
            });
            meter(ui, frac, color);
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui
                    .add(
                        egui::Button::image_and_text(Icon::Generate.image(15.0), "Regenerate")
                            .min_size(vec2(0.0, 34.0)),
                    )
                    .clicked()
                {
                    self.generated = generator::generate(&self.generator);
                }
                if ui
                    .add(
                        egui::Button::image_and_text(Icon::Copy.image(15.0), "Copy")
                            .min_size(vec2(0.0, 34.0)),
                    )
                    .clicked()
                {
                    let value = self.generated.clone();
                    action = Some(self.copy_logged("Generated password", "Generator", &value));
                }
            });
            ui.add_space(16.0);
            let before = self.generator;
            let g = &mut self.generator;
            ui.add(egui::Slider::new(&mut g.length, 8..=64).text("characters"));
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut g.lowercase, "a–z");
                ui.checkbox(&mut g.uppercase, "A–Z");
                ui.checkbox(&mut g.digits, "0–9");
                ui.checkbox(&mut g.symbols, "!@#$");
                ui.checkbox(&mut g.avoid_ambiguous, "Avoid look-alikes (l, 1, O, 0)");
            });
            if self.generator != before {
                self.generated = generator::generate(&self.generator);
            }
        });
        action
    }

    fn settings_page(&mut self, ui: &mut egui::Ui, r: Rect) -> Option<SessionAction> {
        chrome::module(&ui.painter().clone(), r);
        let mut action = None;
        child(ui, r.shrink(PAD + 8.0), |ui| {
            egui::ScrollArea::vertical()
                .id_salt("settings")
                .auto_shrink([false, false])
                .show(ui, |ui| self.settings_contents(ui, &mut action));
        });
        action
    }

    fn settings_contents(&mut self, ui: &mut egui::Ui, action: &mut Option<SessionAction>) {
        section_title(ui, Icon::Settings, "Settings");
        ui.add_space(12.0);
        ui.label(RichText::new("Layout").color(TEXT));
        ui.horizontal(|ui| {
            let _ = ui.radio(true, "Floating Modular");
            if ui
                .radio(false, "Classic")
                .on_hover_text("The original single-panel layout")
                .clicked()
            {
                *action = Some(SessionAction::SetLayout(UiLayout::Classic));
            }
        });
        let transparent = chrome::transparency_available();
        ui.label(
                RichText::new(if transparent {
                    "Window transparency: on"
                } else {
                    "Window transparency: off (Remote Desktop or high contrast); using an opaque background"
                })
                .small()
                .color(MUTED),
            );
        ui.add_space(16.0);
        ui.label(RichText::new("Security").color(TEXT));
        let mut hide = crate::hardening::capture_hidden();
        if ui
            .add_enabled(crate::hardening::capture_supported(), egui::Checkbox::new(&mut hide, "Hide from screenshots and screen sharing"))
            .on_hover_text(
                crate::hardening::capture_hint(),
            )
            .changed()
        {
            crate::hardening::set_capture_hidden(hide);
            crate::settings::set_hide_from_capture(hide);
        }
        if let Some(status) = self.security {
            let color = if status.warning().is_some() {
                ui.visuals().warn_fg_color
            } else {
                MUTED
            };
            ui.label(RichText::new(status.summary()).small().color(color));
        }
        ui.add_space(4.0);
        for line in [
            "Other apps running as you can't read the vault's memory or inject code into it.",
            "Auto-lock after 5 minutes without input. If saving fails it still locks, keeping unsaved changes encrypted until you unlock.",
            "Copied secrets clear from the clipboard after 12 seconds and are kept out of Windows clipboard history and cloud clipboard.",
            "Passwords and protected fields stay masked until you choose Show.",
            "Recent Activity is kept in memory for this session only and never includes secret values.",
        ] {
            ui.label(RichText::new(format!("•  {line}")).small().color(MUTED));
        }
        ui.add_space(16.0);
        ui.label(RichText::new("Site icons").color(TEXT));
        ui.label(
                RichText::new(
                    "Downloads each website's own logo straight from that site (no third-party icon service; only the site's address is used) and saves it in the vault as a KeePass custom icon, so it works offline and in other KeePass apps.",
                )
                .small()
                .color(MUTED),
            );
        match &self.icon_job {
            Some(job) => {
                let done = job.progress.done.load(std::sync::atomic::Ordering::Relaxed);
                let total = job
                    .progress
                    .total
                    .load(std::sync::atomic::Ordering::Relaxed)
                    .max(1);
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(
                        RichText::new(format!("Fetching icons… {done} of {total} sites"))
                            .small()
                            .color(TEXT),
                    );
                });
                meter(ui, done as f32 / total as f32, CYAN);
                ui.ctx().request_repaint_after(Duration::from_millis(200));
            }
            None => {
                let missing: usize = self.icon_targets(None).values().map(Vec::len).sum();
                let label = format!(
                    "Download icons for {missing} entr{} without one",
                    if missing == 1 { "y" } else { "ies" }
                );
                if ui
                    .add_enabled(
                        missing > 0,
                        egui::Button::image_and_text(Icon::Download.image(15.0), label),
                    )
                    .clicked()
                {
                    self.start_icon_download(ui.ctx(), None);
                }
            }
        }
        ui.add_space(16.0);
        ui.label(RichText::new("This vault").color(TEXT));
        ui.add(
            egui::Label::new(
                RichText::new(self.vault.path().display().to_string())
                    .small()
                    .color(MUTED),
            )
            .truncate(),
        );
        ui.add_space(12.0);
        ui.label(RichText::new("Unlock keys").color(TEXT));
        let busy = self.key_job_pending();
        if self.unlock_keys.is_empty() {
            ui.label(
                RichText::new("Passkey unlock: not set up")
                    .small()
                    .color(MUTED),
            );
        }
        let removable = self.unlock_keys.len() > 1;
        let mut remove = None;
        for (i, label) in self.unlock_keys.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.add(Icon::Passkey.image(14.0).tint(CYAN));
                ui.label(RichText::new(label).small().color(TEXT));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(removable && !busy, egui::Button::new("Remove").small())
                        .on_hover_text("Stop this key unlocking the vault")
                        .on_disabled_hover_text(
                            "The only unlock key: add another before removing it.",
                        )
                        .clicked()
                    {
                        remove = Some(i);
                    }
                });
            });
        }
        if let Some(i) = remove {
            self.remove_unlock_key(i, ui.ctx());
        }
        ui.add_space(6.0);
        let (text, hint) = self.passkey_setup_text();
        if ui
            .add_enabled(
                !busy,
                egui::Button::image_and_text(Icon::Passkey.image(15.0), text),
            )
            .clicked()
        {
            self.start_passkey_setup(ui.ctx());
        }
        ui.label(RichText::new(hint).small().color(MUTED));
        if let Some(text) = self.busy_text() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new(text).small());
            });
        }
        ui.add_space(16.0);
        ui.label(RichText::new("Backups").color(TEXT));
        self.backups_section(ui);
        ui.add_space(8.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_addresses_launch() {
        assert!(launch_url("file:///C:/Windows/System32/calc.exe").is_err());
        assert!(launch_url("cmd://whoami").is_err());
        assert!(launch_url("javascript:alert(1)").is_err());
    }

    fn session_with(urls: &[(&str, &str)]) -> (Session, Vec<EntryId>) {
        use vault_core::keepass::{Database, DatabaseKey, config::KdfConfig, db::fields};
        let dir = std::env::temp_dir().join(format!("axv-icons-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("icons-{}.kdbx", urls.len()));
        let mut db = Database::new();
        db.config.kdf_config = KdfConfig::Aes { rounds: 100 };
        let ids = urls
            .iter()
            .map(|(t, u)| {
                db.root_mut()
                    .add_entry()
                    .edit(|e| {
                        e.set_unprotected(fields::TITLE, *t);
                        e.set_unprotected(fields::URL, *u);
                    })
                    .id()
            })
            .collect();
        let mut f = std::fs::File::create(&path).unwrap();
        db.save(&mut f, DatabaseKey::new().with_password("pw"))
            .unwrap();
        let creds = vault_core::vault::Credentials {
            password: Some(Zeroizing::new("pw".into())),
            key_file: None,
        };
        (
            Session::new(vault_core::vault::Vault::open(&path, creds).unwrap()),
            ids,
        )
    }

    #[test]
    fn downloaded_icons_are_shared_per_site_and_kept() {
        let (mut s, ids) = session_with(&[
            ("Chase", "https://www.chase.com/login"),
            ("Chase Freedom", "https://www.chase.com"),
            ("Router", "http://192.168.50.1"),
            ("No site", ""),
        ]);
        let targets = s.icon_targets(None);
        assert_eq!(
            targets.len(),
            1,
            "only the public site; router and blank skipped"
        );
        assert_eq!(targets["www.chase.com"].len(), 2);

        let png =
            crate::siteicons::to_png(include_bytes!("../../assets/app/vault-128.png")).unwrap();
        s.apply_icon_results(vec![("www.chase.com".into(), Ok(png))], None);
        assert!(s.vault.is_dirty());
        assert_eq!(
            s.vault.db.num_custom_icons(),
            1,
            "one stored icon shared by both entries"
        );
        assert!(s.entry_icon(ids[0]).is_some() && s.entry_icon(ids[1]).is_some());
        assert!(s.entry_icon(ids[2]).is_none());
        assert!(
            s.icon_targets(None).is_empty(),
            "entries with icons are not fetched again"
        );

        // Saved as KeePass custom icons: they survive a save and reopen.
        s.vault.save().unwrap();
        let reopened = vault_core::vault::Vault::open(
            s.vault.path(),
            vault_core::vault::Credentials {
                password: Some(Zeroizing::new("pw".into())),
                key_file: None,
            },
        )
        .unwrap();
        assert_eq!(reopened.db.num_custom_icons(), 1);
        assert!(reopened.db.entry(ids[0]).unwrap().custom_icon().is_some());
    }

    #[test]
    fn added_keys_join_the_ring_without_rekeying() {
        use vault_core::envelope::{Envelope, KeyRing, RingKey};
        use vault_core::vault::{Credentials, Vault};

        use crate::jobs;

        let (s, _) = session_with(&[("Bank", "https://bank.example")]);
        let path = s.vault.path().with_file_name("ring.kdbx");
        std::fs::copy(s.vault.path(), &path).unwrap();
        let env_path = envelope::path_for(&path);
        let _ = std::fs::remove_file(&env_path);

        // A vault already set up with a phone passkey: password + raw 32-byte key.
        let mut key = [0u8; 32];
        vault_core::random_bytes(&mut key);
        let backup = vault_core::keyfile::build_v2(&key);
        let pw = || Some(Zeroizing::new("pw".to_owned()));
        let mut v = Vault::open(
            &path,
            Credentials {
                password: pw(),
                key_file: None,
            },
        )
        .unwrap();
        // A key change keeps the previous version in the store.
        v.attach_store(vault_core::backups::BackupConfig::new(
            path.with_file_name("Backups"),
        ));
        v.set_credentials(Credentials {
            password: pw(),
            key_file: Some(Zeroizing::new(key.to_vec())),
        });
        v.save().unwrap();
        let seal = |id: &[u8], input: [u8; 32]| {
            let mut prf = [0u8; 32];
            vault_core::random_bytes(&mut prf);
            (Envelope::seal("localhost", id, input, &prf, &key), prf)
        };
        let mut input = [0u8; 32];
        vault_core::random_bytes(&mut input);
        let (phone, _) = seal(&[1], input);
        let ring = KeyRing {
            keys: vec![RingKey {
                label: "Phone passkey".into(),
                envelope: phone,
            }],
        };
        std::fs::write(&env_path, ring.serialize()).unwrap();
        let db_bytes = std::fs::read(&path).unwrap();

        // Opened with the backup .keyx: adding a key wraps the same database key.
        let mut vault = Vault::open(
            &path,
            Credentials {
                password: pw(),
                key_file: Some(Zeroizing::new(backup.clone().into_bytes())),
            },
        )
        .unwrap();
        vault.attach_store(vault_core::backups::BackupConfig::new(
            path.with_file_name("Backups"),
        ));
        let mut s = Session::new(vault);
        assert_eq!(s.unlock_keys, ["Phone passkey"]);
        assert_eq!(s.passkey_setup_text().0, "Add a phone or security key…");

        let ctx = egui::Context::default();
        for _ in 0..2 {
            let (yubikey, prf) = seal(&[2, s.unlock_keys.len() as u8], input);
            assert_eq!(*yubikey.open(&prf).unwrap(), key);
            s.poll_passkey_job(&ctx); // no job: nothing happens
            s.finish_add_key(jobs::NewPasskey {
                envelope: yubikey,
                key: Zeroizing::new(key),
                label: "Security key (USB)".into(),
                added: true,
            })
            .unwrap();
            s.load_unlock_keys();
        }
        assert_eq!(
            s.unlock_keys,
            [
                "Phone passkey",
                "Security key (USB)",
                "Security key (USB) 2"
            ]
        );
        assert_eq!(
            std::fs::read(&path).unwrap(),
            db_bytes,
            "the database is not re-keyed or rewritten"
        );
        assert!(
            Vault::open(
                &path,
                Credentials {
                    password: pw(),
                    key_file: Some(Zeroizing::new(backup.into_bytes())),
                },
            )
            .is_ok(),
            "the backup key file still opens it"
        );
        let text = std::fs::read_to_string(&env_path).unwrap();
        assert_eq!(KeyRing::parse(&text).unwrap().keys.len(), 3);
        assert!(
            !envelope::legacy_old_path_for(&path).exists(),
            "no copy of the previous ring beside the database"
        );
        // The current ring is in the store instead, labelled with the session's generation.
        let config = s.vault.backup_config().unwrap().clone();
        let (folders, _) = config.existing_folders(
            &path,
            &vault_core::backups::vault_id_of(&s.vault.db),
            &mut Default::default(),
        );
        let rings: Vec<_> = folders
            .iter()
            .flat_map(|f| f.list_rings().unwrap())
            .collect();
        assert_eq!(rings.len(), 2, "one per write");
        assert_eq!(std::fs::read_to_string(&rings[0].path).unwrap(), text);
        assert_eq!(rings[0].generation.as_deref(), Some(s.vault.generation()));
    }

    #[test]
    fn strength_bands() {
        assert_eq!(strength_label(120.0).0, "Excellent");
        assert_eq!(strength_label(60.0).0, "Fair");
        assert_eq!(strength_label(10.0).0, "Very weak");
    }
}
