//! Desktop entry point: opens the local database (seeding demo data on first
//! run unless AXENTRA_NO_DEMO is set), starts sync and opens the window.
//!
//! The window has no native decorations: the app header is the title bar
//! (`titlebar.rs`). "Use the system title bar" in Settings, or
//! AXENTRA_SYSTEM_TITLEBAR=1, brings the desktop's own title bar back.
//! AXENTRA_BUTTON_LAYOUT (GNOME `button-layout` syntax) overrides where the
//! window buttons go.

mod appearance;
mod icon;
mod titlebar;

use std::sync::Arc;

use axentra_core::Store;
use dioxus::desktop::tao::window::{Icon, Theme};
use dioxus::desktop::{Config, LogicalSize, WindowBuilder};

/// Minimum window size (docs/spec/shell.md §2).
const MIN_SIZE: (f64, f64) = (640.0, 480.0);
/// First-run window size: the reference layout's 1440 × 900.
const DEFAULT_SIZE: (f64, f64) = (1440.0, 900.0);

const TEXT_ONLY_CONTEXT_MENU: &str = "<script>document.addEventListener('contextmenu', e => { \
if (!(e.target instanceof Element) || !e.target.closest('input, textarea, [contenteditable]:not([contenteditable=\"false\"])')) e.preventDefault(); \
}, true);</script>";

fn main() {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).init();
    let force_system_title_bar = std::env::var("AXENTRA_SYSTEM_TITLEBAR").ok();
    // The desktop's title-bar preferences, read while the store opens.
    let titlebar_prefs = std::thread::spawn(titlebar::read_prefs);
    let path = std::env::var_os("AXENTRA_DB").map(Into::into).unwrap_or_else(Store::default_path);
    let store = Arc::new(Store::open(&path).unwrap_or_else(|e| panic!("open {}: {e}", path.display())));
    if std::env::var_os("AXENTRA_NO_DEMO").is_none() {
        axentra_core::seed::seed_demo(&store, chrono::Local::now()).expect("seed demo data");
    }
    let settings = store.settings();
    let dark = appearance::is_dark(settings.theme, appearance::system_scheme);
    let csd = !titlebar::system_title_bar(settings.system_title_bar, force_system_title_bar.as_deref());
    axentra_ui::shell::theme::set_system_prefs(appearance::system_prefs());
    let sync = axentra_sync::SyncHandle::start(store.clone());
    axentra_ui::init(store, Some(sync));

    let mut window = WindowBuilder::new()
        .with_title(axentra_ui::APP_NAME)
        .with_inner_size(LogicalSize::new(DEFAULT_SIZE.0, DEFAULT_SIZE.1))
        .with_min_inner_size(LogicalSize::new(MIN_SIZE.0, MIN_SIZE.1))
        // GTK's dark preference also drives the webview's prefers-color-scheme.
        .with_theme(Some(if dark { Theme::Dark } else { Theme::Light }))
        .with_decorations(!csd)
        // The page draws the frame: rounded corners need a see-through window.
        .with_transparent(csd);
    match dioxus::desktop::icon_from_memory::<Icon>(icon::APP_ICON_PNG) {
        Ok(i) => window = window.with_window_icon(Some(i)),
        Err(e) => tracing::warn!("app icon: {e}"),
    }
    let mut cfg = Config::new()
        .with_menu(None)
        .with_window(window)
        // The webview's own menu (cut / copy / paste) only inside editable
        // fields; everywhere else the app shows its own menus or nothing.
        .with_disable_context_menu(false)
        .with_custom_head(TEXT_ONLY_CONTEXT_MENU.to_string());
    let mut launch = dioxus::LaunchBuilder::desktop();
    let root: fn() -> dioxus::prelude::Element = if csd {
        launch = launch.with_context(titlebar_prefs.join().unwrap_or_default());
        titlebar::CsdRoot
    } else {
        // A transparent webview ignores the background colour, so only here.
        cfg = cfg.with_background_color(if dark { appearance::CANVAS_DARK } else { appearance::CANVAS_LIGHT });
        axentra_ui::App
    };
    launch.with_cfg(cfg).launch(root);
}
