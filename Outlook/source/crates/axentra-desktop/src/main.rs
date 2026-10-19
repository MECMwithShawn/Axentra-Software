//! Desktop entry point: opens the local database (seeding demo data on first
//! run unless AXENTRA_NO_DEMO is set), starts sync and opens the window.

mod appearance;
mod icon;

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
    let path = std::env::var_os("AXENTRA_DB").map(Into::into).unwrap_or_else(Store::default_path);
    let store = Arc::new(Store::open(&path).unwrap_or_else(|e| panic!("open {}: {e}", path.display())));
    if std::env::var_os("AXENTRA_NO_DEMO").is_none() {
        axentra_core::seed::seed_demo(&store, chrono::Local::now()).expect("seed demo data");
    }
    let dark = appearance::is_dark(store.settings().theme, appearance::system_scheme);
    axentra_ui::shell::theme::set_system_prefs(appearance::system_prefs());
    let sync = axentra_sync::SyncHandle::start(store.clone());
    axentra_ui::init(store, Some(sync));

    let mut window = WindowBuilder::new()
        .with_title(axentra_ui::APP_NAME)
        .with_inner_size(LogicalSize::new(DEFAULT_SIZE.0, DEFAULT_SIZE.1))
        .with_min_inner_size(LogicalSize::new(MIN_SIZE.0, MIN_SIZE.1))
        // GTK's dark preference also drives the webview's prefers-color-scheme.
        .with_theme(Some(if dark { Theme::Dark } else { Theme::Light }));
    match dioxus::desktop::icon_from_memory::<Icon>(icon::APP_ICON_PNG) {
        Ok(i) => window = window.with_window_icon(Some(i)),
        Err(e) => tracing::warn!("app icon: {e}"),
    }
    dioxus::LaunchBuilder::desktop()
        .with_cfg(
            Config::new()
                .with_menu(None)
                .with_window(window)
                // The webview's own menu (cut / copy / paste) only inside editable
                // fields; everywhere else the app shows its own menus or nothing.
                .with_disable_context_menu(false)
                .with_custom_head(TEXT_ONLY_CONTEXT_MENU.to_string())
                .with_background_color(if dark { appearance::CANVAS_DARK } else { appearance::CANVAS_LIGHT }),
        )
        .launch(axentra_ui::App);
}
