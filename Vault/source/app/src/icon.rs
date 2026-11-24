//! The Axentra Vault icon set (app/assets/icons, drawn in the Cybercore style: 24x24 grid,
//! 1.5px #22d3ee stroke) and the window/taskbar icon using the Glass + Precision circuit-halo design.

use std::sync::Arc;

use eframe::egui::{self, IconData, Image, ImageSource, include_image};
#[cfg(test)]
use resvg::usvg;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Download,
    Unlock,
    Activity,
    Check,
    Cross,
    Launch,
    More,
    Plus,
    Settings,
    Star,
    StarFilled,
    Tag,
    Archive,
    Attention,
    Copy,
    Delete,
    Edit,
    Generate,
    Hide,
    History,
    Key,
    Lock,
    NewEntry,
    Passkey,
    Protected,
    RecycleBin,
    Restore,
    Save,
    Search,
    Security,
    Show,
    SortDown,
    SortUp,
    Vault,
}

impl Icon {
    pub fn source(self) -> ImageSource<'static> {
        match self {
            Icon::Download => include_image!("../assets/icons/download.svg"),
            Icon::Unlock => include_image!("../assets/icons/unlock.svg"),
            Icon::Activity => include_image!("../assets/icons/activity.svg"),
            Icon::Check => include_image!("../assets/icons/check.svg"),
            Icon::Cross => include_image!("../assets/icons/cross.svg"),
            Icon::Launch => include_image!("../assets/icons/launch.svg"),
            Icon::More => include_image!("../assets/icons/more.svg"),
            Icon::Plus => include_image!("../assets/icons/plus.svg"),
            Icon::Settings => include_image!("../assets/icons/settings.svg"),
            Icon::Star => include_image!("../assets/icons/star.svg"),
            Icon::StarFilled => include_image!("../assets/icons/star-filled.svg"),
            Icon::Tag => include_image!("../assets/icons/tag.svg"),
            Icon::Archive => include_image!("../assets/icons/archive.svg"),
            Icon::Attention => include_image!("../assets/icons/attention.svg"),
            Icon::Copy => include_image!("../assets/icons/copy.svg"),
            Icon::Delete => include_image!("../assets/icons/delete.svg"),
            Icon::Edit => include_image!("../assets/icons/edit.svg"),
            Icon::Generate => include_image!("../assets/icons/generate.svg"),
            Icon::Hide => include_image!("../assets/icons/hide.svg"),
            Icon::History => include_image!("../assets/icons/history.svg"),
            Icon::Key => include_image!("../assets/icons/key.svg"),
            Icon::Lock => include_image!("../assets/icons/lock.svg"),
            Icon::NewEntry => include_image!("../assets/icons/new-entry.svg"),
            Icon::Passkey => include_image!("../assets/icons/passkey.svg"),
            Icon::Protected => include_image!("../assets/icons/protected.svg"),
            Icon::RecycleBin => include_image!("../assets/icons/recycle-bin.svg"),
            Icon::Restore => include_image!("../assets/icons/restore.svg"),
            Icon::Save => include_image!("../assets/icons/save.svg"),
            Icon::Search => include_image!("../assets/icons/search.svg"),
            Icon::Security => include_image!("../assets/icons/security.svg"),
            Icon::Show => include_image!("../assets/icons/show.svg"),
            Icon::SortDown => include_image!("../assets/icons/sort-down.svg"),
            Icon::SortUp => include_image!("../assets/icons/sort-up.svg"),
            Icon::Vault => include_image!("../assets/icons/vault.svg"),
        }
    }

    pub fn image(self, size: f32) -> Image<'static> {
        Image::new(self.source()).fit_to_exact_size(egui::vec2(size, size))
    }
}

/// A button with an icon and a label (the label stays, so meaning never depends on the glyph).
pub fn button(ui: &mut egui::Ui, icon: Icon, text: &str) -> egui::Response {
    ui.add(egui::Button::image_and_text(icon.image(15.0), text))
}

pub fn small_button(ui: &mut egui::Ui, icon: Icon, text: &str) -> egui::Response {
    ui.add(egui::Button::image_and_text(icon.image(13.0), text).small())
}

/// The approved icy-blue lock and circuit halo, exported by the suite icon renderer.
const MARK_PNG: &[u8] = include_bytes!("../assets/app/vault-64.png");
const SIZE: u32 = 64;

pub fn app_icon() -> Arc<IconData> {
    Arc::new(IconData {
        rgba: png_rgba(MARK_PNG, SIZE),
        width: SIZE,
        height: SIZE,
    })
}

/// Choose the nearest larger optical size before resizing for a tray DPI.
pub fn tray_rgba(size: u32) -> Vec<u8> {
    let bytes: &[u8] = match size {
        0..=16 => include_bytes!("../assets/app/vault-16.png"),
        17..=24 => include_bytes!("../assets/app/vault-24.png"),
        25..=32 => include_bytes!("../assets/app/vault-32.png"),
        33..=48 => include_bytes!("../assets/app/vault-48.png"),
        _ => MARK_PNG,
    };
    png_rgba(bytes, size)
}

fn png_rgba(bytes: &[u8], size: u32) -> Vec<u8> {
    let image = image::load_from_memory(bytes)
        .expect("bundled icon PNG decodes")
        .into_rgba8();
    if image.width() == size {
        image.into_raw()
    } else {
        image::imageops::resize(&image, size, size, image::imageops::FilterType::Lanczos3)
            .into_raw()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_icon_has_glass_tile_blue_padlock_and_clear_corners() {
        let icon = app_icon();
        let px = |x: u32, y: u32| &icon.rgba[((y * SIZE + x) * 4) as usize..][..4];
        assert_eq!(icon.rgba.len(), (SIZE * SIZE * 4) as usize);
        assert_eq!(px(0, 0)[3], 0, "corner is transparent");
        assert_eq!(px(32, 4)[3], 255, "tile is opaque");
        let bright = (20..44)
            .flat_map(|y| (20..44).map(move |x| (x, y)))
            .filter(|&(x, y)| {
                let p = px(x, y);
                p[2] > 200 && p[1] > 110 && p[0] < 190
            });
        assert!(bright.count() > 12, "blue glass padlock in the centre");
        let cyan = icon
            .rgba
            .chunks(4)
            .filter(|p| p[1] > 150 && p[2] > 150 && p[0] < 140)
            .count();
        assert!(cyan > 25, "blue accent pixels drawn ({cyan} cyan pixels)");
    }

    #[test]
    fn every_icon_is_valid_svg() {
        for icon in [
            Icon::Download,
            Icon::Unlock,
            Icon::Activity,
            Icon::Check,
            Icon::Cross,
            Icon::Launch,
            Icon::More,
            Icon::Plus,
            Icon::Settings,
            Icon::Star,
            Icon::StarFilled,
            Icon::Tag,
            Icon::Archive,
            Icon::Attention,
            Icon::Copy,
            Icon::Delete,
            Icon::Edit,
            Icon::Generate,
            Icon::Hide,
            Icon::History,
            Icon::Key,
            Icon::Lock,
            Icon::NewEntry,
            Icon::Passkey,
            Icon::Protected,
            Icon::RecycleBin,
            Icon::Restore,
            Icon::Save,
            Icon::Search,
            Icon::Security,
            Icon::Show,
            Icon::SortDown,
            Icon::SortUp,
            Icon::Vault,
        ] {
            let ImageSource::Bytes { bytes, .. } = icon.source() else {
                panic!("{icon:?} not embedded")
            };
            usvg::Tree::from_data(&bytes, &usvg::Options::default())
                .unwrap_or_else(|e| panic!("{icon:?}: {e}"));
        }
    }
}
