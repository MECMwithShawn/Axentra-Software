//! Client-side decorations (docs/spec/shell.md §3.1 #12, §12.4): the header
//! is the window's title bar unless Settings › General › Appearance › "Use
//! the system title bar" is on or `AXENTRA_SYSTEM_TITLEBAR` is set.
//!
//! This module reads the desktop's title-bar preferences once at startup
//! (GNOME's `button-layout` and click actions, or GTK's
//! `gtk-decoration-layout` elsewhere) and provides the UI's
//! `WindowControls` context, backed by the tao window. The window's state
//! (maximised, tiled edges) comes from GDK directly, since tao reports
//! neither tiling nor state changes as such.

use std::path::PathBuf;
use std::rc::Rc;

use axentra_ui::shell::window::{ButtonLayout, ResizeEdge, TiledEdges, TitlebarAction, TitlebarPrefs, WindowControls};
use dioxus::desktop::tao::event::{Event, WindowEvent};
use dioxus::desktop::tao::window::ResizeDirection;
use dioxus::desktop::{DesktopService, use_window, use_wry_event_handler};
use dioxus::prelude::*;

/// GNOME's window-manager preferences schema.
const WM_SCHEMA: &str = "org.gnome.desktop.wm.preferences";

/// Native decorations instead of the header title bar: the setting, or
/// `AXENTRA_SYSTEM_TITLEBAR` set to anything but empty, `0`, `false` or `no`.
pub fn system_title_bar(setting: bool, env: Option<&str>) -> bool {
    setting || env.is_some_and(|v| !matches!(v.trim().to_ascii_lowercase().as_str(), "" | "0" | "false" | "no"))
}

/// One key's value in `gsettings list-recursively org.gnome.desktop.wm.preferences`
/// output (lines like `org.gnome.desktop.wm.preferences button-layout 'appmenu:close'`).
pub fn gsettings_value<'a>(listing: &'a str, key: &str) -> Option<&'a str> {
    listing.lines().find_map(|line| {
        let rest = line.trim().strip_prefix(WM_SCHEMA)?.trim_start().strip_prefix(key)?;
        rest.starts_with(char::is_whitespace).then(|| rest.trim())
    })
}

/// `gtk-decoration-layout` from a GTK `settings.ini` (KDE, Xfce and others write it).
pub fn gtk_ini_layout(ini: &str) -> Option<&str> {
    ini.lines().find_map(|line| {
        let (k, v) = line.split_once('=')?;
        (k.trim() == "gtk-decoration-layout").then(|| v.trim())
    })
}

/// Whether the desktop's window manager follows GNOME's wm preferences
/// (`XDG_CURRENT_DESKTOP`, colon-separated, e.g. `ubuntu:GNOME`). Unknown
/// (empty) counts as GNOME, the likeliest case.
pub fn follows_gnome_wm_prefs(current_desktop: &str) -> bool {
    current_desktop.trim().is_empty()
        || current_desktop.split(':').any(|d| {
            matches!(d.trim().to_ascii_lowercase().as_str(), "gnome" | "gnome-classic" | "gnome-flashback" | "unity" | "pantheon" | "budgie" | "budgie-desktop")
        })
}

/// Title-bar preferences from what the desktop reported: the GNOME wm
/// listing when there is one, else GTK's `settings.ini`, else the defaults
/// (Minimize, Maximize, Close at the right; double-click maximises).
/// `override_layout` (`AXENTRA_BUTTON_LAYOUT`) replaces the button layout.
pub fn prefs_from(override_layout: Option<&str>, gnome: Option<&str>, gtk_ini: Option<&str>) -> TitlebarPrefs {
    let mut p = TitlebarPrefs::default();
    if let Some(list) = gnome {
        if let Some(l) = gsettings_value(list, "button-layout") {
            p.layout = ButtonLayout::parse(l);
        }
        if let Some(a) = gsettings_value(list, "action-double-click-titlebar").and_then(TitlebarAction::parse) {
            p.double_click = a;
        }
        if let Some(a) = gsettings_value(list, "action-middle-click-titlebar").and_then(TitlebarAction::parse) {
            p.middle_click = a;
        }
    } else if let Some(l) = gtk_ini.and_then(gtk_ini_layout) {
        p.layout = ButtonLayout::parse(l);
    }
    if let Some(l) = override_layout.filter(|l| !l.trim().is_empty()) {
        p.layout = ButtonLayout::parse(l);
    }
    p
}

fn gtk_settings_ini() -> Option<String> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    std::fs::read_to_string(config.join("gtk-3.0").join("settings.ini")).ok()
}

/// Ask the desktop (one `gsettings` call on GNOME-like desktops, a file read elsewhere).
pub fn read_prefs() -> TitlebarPrefs {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    let gnome = if follows_gnome_wm_prefs(&desktop) { crate::appearance::run("gsettings", &["list-recursively", WM_SCHEMA]) } else { None };
    let ini = if gnome.is_none() { gtk_settings_ini() } else { None };
    let layout = std::env::var("AXENTRA_BUTTON_LAYOUT").ok();
    prefs_from(layout.as_deref(), gnome.as_deref(), ini.as_deref())
}

fn direction(edge: ResizeEdge) -> ResizeDirection {
    match edge {
        ResizeEdge::North => ResizeDirection::North,
        ResizeEdge::South => ResizeDirection::South,
        ResizeEdge::East => ResizeDirection::East,
        ResizeEdge::West => ResizeDirection::West,
        ResizeEdge::NorthEast => ResizeDirection::NorthEast,
        ResizeEdge::NorthWest => ResizeDirection::NorthWest,
        ResizeEdge::SouthEast => ResizeDirection::SouthEast,
        ResizeEdge::SouthWest => ResizeDirection::SouthWest,
    }
}

/// The tiled edges in a GDK window state. Compositors without per-edge
/// detail only set `TILED`; then every edge counts as tiled.
#[cfg(target_os = "linux")]
fn tiled_edges(state: gtk::gdk::WindowState) -> TiledEdges {
    use gtk::gdk::WindowState as S;
    let edges = TiledEdges {
        top: state.contains(S::TOP_TILED),
        right: state.contains(S::RIGHT_TILED),
        bottom: state.contains(S::BOTTOM_TILED),
        left: state.contains(S::LEFT_TILED),
    };
    if !edges.any() && state.contains(S::TILED) { TiledEdges::ALL } else { edges }
}

/// Follow the window's GDK state: maximised (as it changes, not only on the
/// next resize) and tiled edges, which tao does not report at all.
#[cfg(target_os = "linux")]
fn use_gdk_window_state(desktop: &Rc<DesktopService>, maximized: Signal<bool>, tiled: Signal<TiledEdges>) {
    use dioxus::core::{Runtime, current_scope_id, use_hook_with_cleanup};
    use dioxus::desktop::tao::platform::unix::WindowExtUnix;
    use gtk::prelude::*;
    use std::cell::Cell;

    let weak = Rc::downgrade(desktop);
    use_hook_with_cleanup(
        || {
            let runtime = Runtime::current();
            let scope = current_scope_id();
            let id = desktop.gtk_window().connect_window_state_event(move |_, event| {
                let state = event.new_window_state();
                let (max, edges) = (state.contains(gtk::gdk::WindowState::MAXIMIZED), tiled_edges(state));
                // Signals are handles: copy them to write from this `Fn`.
                let (mut maximized, mut tiled) = (maximized, tiled);
                runtime.in_scope(scope, || {
                    if maximized.try_peek().is_ok_and(|m| *m != max) {
                        maximized.set(max);
                    }
                    if tiled.try_peek().is_ok_and(|t| *t != edges) {
                        tiled.set(edges);
                    }
                });
                gtk::glib::Propagation::Proceed
            });
            Rc::new(Cell::new(Some(id)))
        },
        move |id| {
            if let (Some(d), Some(id)) = (weak.upgrade(), id.take()) {
                d.gtk_window().disconnect(id);
            }
        },
    );
}

/// Root component in title-bar mode: provides `WindowControls` for this
/// window (the `TitlebarPrefs` come from the launch context), keeps
/// `is_maximized` and `tiled` in step with the window (GDK state changes,
/// and tao's resize events as a fallback for maximise and restore), then
/// renders the app.
#[allow(non_snake_case)]
pub fn CsdRoot() -> Element {
    let desktop = use_window();
    let prefs = try_use_context::<TitlebarPrefs>().unwrap_or_default();
    let mut maximized = use_signal(|| desktop.is_maximized());
    let tiled = use_signal(TiledEdges::default);
    #[cfg(target_os = "linux")]
    use_gdk_window_state(&desktop, maximized, tiled);
    // Weak handles: the window must be free to close and drop.
    let weak = Rc::downgrade(&desktop);
    let w = weak.clone();
    use_wry_event_handler(move |event, _| {
        if let Event::WindowEvent { event: WindowEvent::Resized(_), .. } = event
            && let Some(d) = w.upgrade()
        {
            let now = d.is_maximized();
            if *maximized.peek() != now {
                maximized.set(now);
            }
        }
    });
    use_context_provider(move || {
        let on = |f: fn(&DesktopService)| {
            let w = weak.clone();
            Callback::new(move |()| {
                if let Some(d) = w.upgrade() {
                    f(&d);
                }
            })
        };
        let w = weak.clone();
        WindowControls {
            minimize: on(|d| d.set_minimized(true)),
            toggle_maximize: on(|d| d.toggle_maximized()),
            close: on(|d| d.close()),
            start_drag: on(|d| d.drag()),
            start_resize: Callback::new(move |edge: ResizeEdge| {
                if let Some(d) = w.upgrade()
                    && let Err(e) = d.drag_resize_window(direction(edge))
                {
                    tracing::warn!("window resize: {e}");
                }
            }),
            is_maximized: maximized,
            tiled,
            layout: prefs.layout,
            double_click: prefs.double_click,
            middle_click: prefs.middle_click,
        }
    });
    rsx! {
        axentra_ui::App {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axentra_ui::shell::window::WindowButton::*;

    const ZORIN: &str = "org.gnome.desktop.wm.preferences action-double-click-titlebar 'toggle-maximize'\n\
org.gnome.desktop.wm.preferences action-middle-click-titlebar 'minimize'\n\
org.gnome.desktop.wm.preferences action-right-click-titlebar 'menu'\n\
org.gnome.desktop.wm.preferences button-layout 'appmenu:minimize,maximize,close'\n";

    #[test]
    fn system_title_bar_from_setting_or_env() {
        assert!(!system_title_bar(false, None));
        assert!(system_title_bar(true, None));
        assert!(system_title_bar(false, Some("1")));
        assert!(system_title_bar(false, Some("yes")));
        assert!(!system_title_bar(false, Some("0")));
        assert!(!system_title_bar(false, Some("")));
        assert!(!system_title_bar(false, Some("False")));
    }

    #[test]
    fn gsettings_listing() {
        assert_eq!(gsettings_value(ZORIN, "button-layout"), Some("'appmenu:minimize,maximize,close'"));
        assert_eq!(gsettings_value(ZORIN, "action-middle-click-titlebar"), Some("'minimize'"));
        assert_eq!(gsettings_value(ZORIN, "button"), None, "whole key names only");
        assert_eq!(gsettings_value("No such schema “org.gnome.desktop.wm.preferences”", "button-layout"), None);
    }

    #[test]
    fn gnome_prefs() {
        let p = prefs_from(None, Some(ZORIN), Some("gtk-decoration-layout=close:"));
        assert_eq!(p.layout.end().collect::<Vec<_>>(), vec![Minimize, Maximize, Close]);
        assert_eq!((p.double_click, p.middle_click), (TitlebarAction::ToggleMaximize, TitlebarAction::Minimize));
        // A listing without the keys keeps the defaults.
        assert_eq!(prefs_from(None, Some(""), None), TitlebarPrefs::default());
    }

    #[test]
    fn gtk_ini_elsewhere_and_override() {
        let ini = "[Settings]\ngtk-theme-name=Breeze\ngtk-decoration-layout = close,minimize:\n";
        assert_eq!(gtk_ini_layout(ini), Some("close,minimize:"));
        let p = prefs_from(None, None, Some(ini));
        assert_eq!(p.layout.start().collect::<Vec<_>>(), vec![Close, Minimize]);
        assert_eq!(p.double_click, TitlebarAction::ToggleMaximize);
        assert_eq!(prefs_from(None, None, Some("[Settings]\n")), TitlebarPrefs::default());
        let p = prefs_from(Some("close:"), Some(ZORIN), None);
        assert_eq!(p.layout.start().collect::<Vec<_>>(), vec![Close]);
        assert_eq!(prefs_from(Some("  "), None, None), TitlebarPrefs::default());
    }

    #[test]
    fn which_desktops_use_gnome_prefs() {
        assert!(follows_gnome_wm_prefs("zorin:GNOME"));
        assert!(follows_gnome_wm_prefs("ubuntu:GNOME"));
        assert!(follows_gnome_wm_prefs(""));
        assert!(follows_gnome_wm_prefs("Unity"));
        assert!(!follows_gnome_wm_prefs("KDE"));
        assert!(!follows_gnome_wm_prefs("XFCE"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn gdk_tiled_states() {
        use gtk::gdk::WindowState as S;
        assert_eq!(tiled_edges(S::empty()), TiledEdges::NONE);
        assert_eq!(tiled_edges(S::FOCUSED), TiledEdges::NONE);
        // GNOME's left half: per-edge flags plus the legacy one.
        let left = S::TILED | S::TOP_TILED | S::BOTTOM_TILED | S::LEFT_TILED | S::RIGHT_RESIZABLE;
        assert_eq!(tiled_edges(left), TiledEdges { top: true, right: false, bottom: true, left: true });
        // Only the legacy flag: every edge.
        assert_eq!(tiled_edges(S::TILED), TiledEdges::ALL);
    }

    #[test]
    fn every_edge_has_a_direction() {
        for e in ResizeEdge::ALL {
            let _ = direction(e);
        }
        assert!(matches!(direction(ResizeEdge::SouthWest), ResizeDirection::SouthWest));
    }
}
