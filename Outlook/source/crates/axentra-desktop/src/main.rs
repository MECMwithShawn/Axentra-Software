//! Desktop entry point: opens the local database (seeding demo data on first
//! run unless AXENTRA_NO_DEMO is set), starts sync and opens the window.
//!
//! The window has no native decorations: the app header is the title bar
//! (`titlebar.rs`). "Use the system title bar" in Settings, or
//! AXENTRA_SYSTEM_TITLEBAR=1, brings the desktop's own title bar back.
//! AXENTRA_BUTTON_LAYOUT (GNOME `button-layout` syntax) overrides where the
//! window buttons go.
//!
//! One app runs per database (`instance.rs`): launching it again shows the
//! open window and exits.
//!
//! Command line: `--version` and `--help` print and exit; anything else is
//! ignored with a note (no files or links are opened from the command line).

mod appearance;
mod icon;
mod instance;
mod pin;
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

const USAGE: &str = "Usage: axentra-outlook [--version | --help]

Opens Axentra Outlook, or shows its window when it is already open on this
profile.

Environment:
  AXENTRA_DB=<file>          use another profile database (a test profile)
  AXENTRA_NO_DEMO=1          don't add demo data to a new profile
  AXENTRA_SYSTEM_TITLEBAR=1  use the desktop's own title bar
  RUST_LOG=debug             log to the terminal";

/// What the command line asks for (see [`USAGE`]).
#[derive(Debug, PartialEq)]
enum Args {
    Run { ignored: Vec<String> },
    Version,
    Help,
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Args {
    let mut ignored = Vec::new();
    for a in args {
        match a.as_str() {
            "--version" | "-V" => return Args::Version,
            "--help" | "-h" => return Args::Help,
            _ => ignored.push(a),
        }
    }
    Args::Run { ignored }
}

/// The database this copy opens (`AXENTRA_DB`, else the default profile).
fn database_path() -> std::path::PathBuf {
    std::env::var_os("AXENTRA_DB").map(Into::into).unwrap_or_else(Store::default_path)
}

fn main() {
    match parse_args(std::env::args().skip(1)) {
        Args::Version => {
            println!("{} {}", axentra_ui::APP_NAME, env!("CARGO_PKG_VERSION"));
            return;
        }
        Args::Help => {
            println!("{USAGE}");
            return;
        }
        Args::Run { ignored } if !ignored.is_empty() => {
            eprintln!("axentra-outlook: ignoring {}: files and links can't be opened from the command line.", ignored.join(" "));
        }
        Args::Run { .. } => {}
    }
    let path = database_path();
    // Pin to desktop on Wayland: GNOME ignores "keep below" from Wayland
    // windows, so a pinned window opens through XWayland, where it is
    // honoured. Decided from a marker next to the database, before any
    // thread or GTK starts (the setting itself is in the store).
    if pin::wants_xwayland(&path, |k| std::env::var_os(k)) {
        // SAFETY: still single-threaded; nothing has read the environment yet.
        unsafe { std::env::set_var("GDK_BACKEND", "x11") };
    }
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).init();
    // First, before anything slow: is this profile already open? A copy the
    // lock can't see (one started before this version, which takes no lock,
    // or one whose lock is elsewhere) is checked for too.
    let primary = match instance::start(&path, instance::launch_token().as_deref()) {
        Ok(instance::Start::Run(primary, db_lock)) => {
            // Held until exit: another launch on this database, whatever
            // its runtime directory, sees it.
            std::mem::forget(db_lock);
            primary
        }
        Ok(instance::Start::Shown) => {
            eprintln!("{} is already running; showing its window.", axentra_ui::APP_NAME);
            return;
        }
        // The lock has been let go: a launch made while this dialog is up
        // (after closing the other copy, as it says) opens the app.
        Ok(instance::Start::Refused(other)) => {
            let detail = if other.outdated {
                format!(
                    "A copy started before {} was updated is still running on this mailbox (process {}). Close its window, then open {} again.",
                    axentra_ui::APP_NAME,
                    other.pid,
                    axentra_ui::APP_NAME
                )
            } else {
                format!(
                    "Another copy is already running on this mailbox (process {}). Switch to its window, or close it and open {} again.",
                    other.pid,
                    axentra_ui::APP_NAME
                )
            };
            tell_user(&format!("{} is already open.", axentra_ui::APP_NAME), &detail);
            std::process::exit(1);
        }
        Err(e) => {
            tell_user(&format!("{e}."), "Wait for it to finish what it is doing, or close it, and try again.");
            std::process::exit(1);
        }
    };
    let _instance = primary.map(|primary| {
        #[cfg(target_os = "linux")]
        instance::watch_gtk();
        primary.serve(|req| {
            tracing::debug!("instance: show requested");
            #[cfg(target_os = "linux")]
            instance::show_on_gtk_thread(req);
            #[cfg(not(target_os = "linux"))]
            req.answer(instance::Answer::Shown);
        })
    });
    let force_system_title_bar = std::env::var("AXENTRA_SYSTEM_TITLEBAR").ok();
    // The desktop's title-bar preferences, read while the store opens.
    let titlebar_prefs = std::thread::spawn(titlebar::read_prefs);
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
        titlebar::SystemRoot
    };
    launch.with_cfg(cfg).launch(root);
}

/// Say why the app is not opening: on stderr, and in a small dialog, since
/// launches from Command or the app menu have no terminal.
fn tell_user(headline: &str, detail: &str) {
    eprintln!("{}: {headline} {detail}", axentra_ui::APP_NAME);
    #[cfg(target_os = "linux")]
    {
        use gtk::prelude::*;
        if gtk::init().is_err() {
            return;
        }
        let dialog = gtk::MessageDialog::new(None::<&gtk::Window>, gtk::DialogFlags::MODAL, gtk::MessageType::Info, gtk::ButtonsType::Ok, headline);
        dialog.set_title(axentra_ui::APP_NAME);
        dialog.set_secondary_text(Some(detail));
        dialog.set_icon_name(Some("axentra-outlook"));
        dialog.run();
        dialog.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Args {
        parse_args(a.iter().map(|s| s.to_string()))
    }

    #[test]
    fn command_line() {
        assert_eq!(args(&[]), Args::Run { ignored: vec![] });
        assert_eq!(args(&["--version"]), Args::Version);
        assert_eq!(args(&["-V"]), Args::Version);
        assert_eq!(args(&["--help"]), Args::Help);
        assert_eq!(args(&["mailto:someone@example.com", "--help"]), Args::Help);
        assert_eq!(args(&["mailto:someone@example.com", "/etc/hostname"]), Args::Run { ignored: vec!["mailto:someone@example.com".into(), "/etc/hostname".into()] });
        assert!(USAGE.contains("--version") && USAGE.contains("AXENTRA_DB"));
    }
}
