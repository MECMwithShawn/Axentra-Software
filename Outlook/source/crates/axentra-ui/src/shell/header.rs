//! App header bar (docs/spec/shell.md §3): launcher, app name, search,
//! My Day, Notifications, Settings, More, account, and (when the header is
//! the window's title bar, `window::WindowControls`) the caption buttons on
//! the side the desktop puts them, with the empty space as a drag region.

use std::rc::Rc;

use dioxus::prelude::*;

use super::keymap::GlobalCmd;
use super::{Flyout, SidePane, account, launcher, notifications, run_command, search, use_shell, window};
use crate::kit::{Avatar, CounterBadge, Icon, Menu, MenuItem, filled_icon};
use crate::state::{Module, store, sync, use_app};

#[component]
pub fn Header() -> Element {
    let app = use_app();
    let shell = use_shell();
    let account = {
        let _ = app.accounts_rev();
        store().primary_account().ok().flatten()
    };
    let module = (app.module)();
    let pane = (shell.pane)();
    let flyout = (shell.flyout)();
    let my_day_open = (app.my_day_open)() && module != Module::Tasks;
    let unseen = notifications::use_unseen_count();
    let settings_open = (app.settings_dialog)().is_some();
    // Title bar mode: caption buttons on each side and drag regions on the
    // empty space (the shell root publishes the button counts to the CSS).
    // Both button groups come last in the DOM, so they follow the avatar in
    // the tab order (§3.7) wherever they sit; CSS moves the start group to
    // the left edge.
    let controls = window::use_window_controls();
    let (wc_start, wc_end) = controls.map(|c| (c.layout.start().count(), c.layout.end().count())).unwrap_or((0, 0));
    let drag = controls.is_some().then_some("true");
    rsx! {
        header { class: "ax-header", role: "banner", "data-ax-drag-region": drag,
            div { class: "ax-header-start", "data-ax-drag-region": drag,
                HeaderButton {
                    icon: "grid_dots",
                    title: "App launcher",
                    class: "ax-header-launcher",
                    selected: flyout == Some(Flyout::Launcher),
                    popup: true,
                    onclick: move |_| shell.toggle_flyout(Flyout::Launcher),
                }
                if flyout == Some(Flyout::Launcher) { launcher::LauncherFlyout {} }
                button {
                    r#type: "button",
                    class: "ax-header-appname",
                    title: "Go to Inbox",
                    onclick: move |_| super::go_to_inbox(app),
                    "{crate::APP_NAME}"
                }
                if !(shell.online)() {
                    span {
                        class: "ax-offline-pill",
                        role: "status",
                        title: "You're offline. Changes will sync when you reconnect.",
                        "Offline"
                    }
                }
            }
            search::SearchBox {}
            div { class: "ax-header-end", role: "toolbar", "aria-label": "Header actions", "data-ax-drag-region": drag,
                HeaderButton {
                    icon: "calendar_checkmark",
                    title: if module == Module::Tasks { "My Day is already open in To Do" } else { "My Day" },
                    selected: my_day_open,
                    toggle: true,
                    disabled: module == Module::Tasks,
                    onclick: {
                        let mut app = app;
                        let mut shell = shell;
                        move |_| {
                            let open = (app.my_day_open)();
                            app.my_day_open.set(!open);
                            if !open {
                                shell.pane.set(None);
                            }
                        }
                    },
                }
                HeaderButton {
                    icon: "alert",
                    title: "Notifications",
                    selected: pane == Some(SidePane::Notifications),
                    toggle: true,
                    badge: unseen,
                    onclick: move |_| shell.toggle_pane(app, SidePane::Notifications),
                }
                HeaderButton {
                    icon: "settings",
                    title: "Settings",
                    selected: settings_open,
                    keyshortcuts: super::keymap::aria_for(GlobalCmd::Settings, app.settings.read().keyboard_mode),
                    onclick: move |_| run_command(app, shell, GlobalCmd::Settings),
                }
                MoreMenu {}
                button {
                    r#type: "button",
                    class: "ax-header-me",
                    class: if flyout == Some(Flyout::Account) { "ax-selected" },
                    title: account.as_ref().map(|a| format!("{}\n{}", a.display_name, a.email)).unwrap_or_else(|| "Account manager".into()),
                    "aria-label": match &account { Some(a) => format!("Account manager for {}", a.display_name), None => "Account manager".into() },
                    "aria-haspopup": "dialog",
                    "aria-expanded": if flyout == Some(Flyout::Account) { "true" } else { "false" },
                    onclick: move |_| shell.toggle_flyout(Flyout::Account),
                    if let Some(a) = &account {
                        Avatar { name: a.display_name.clone(), email: a.email.clone(), size: 32 }
                    } else {
                        Icon { name: "person_circle", size: 24 }
                    }
                }
                if flyout == Some(Flyout::Account) { account::AccountFlyout {} }
            }
            if wc_start > 0 { window::CaptionButtons { end: false } }
            if wc_end > 0 { window::CaptionButtons { end: true } }
        }
    }
}

/// A 40×48 header button (§3.1 visual states). `selected` = its pane is open
/// (filled icon); `badge` shows a notification count.
#[component]
fn HeaderButton(
    #[props(into)] icon: String,
    #[props(into)] title: String,
    #[props(default)] selected: bool,
    #[props(default)] toggle: bool,
    #[props(default)] popup: bool,
    #[props(default)] disabled: bool,
    #[props(default)] badge: usize,
    #[props(into, default)] class: String,
    #[props(into, default)] keyshortcuts: String,
    onclick: EventHandler<MouseEvent>,
) -> Element {
    let shown = if selected { filled_icon(&icon) } else { icon.clone() };
    rsx! {
        button {
            r#type: "button",
            class: "ax-header-btn {class}",
            class: if selected { "ax-selected" },
            title: "{title}",
            "aria-label": if badge > 0 { format!("{title}, {badge} unread") } else { title.clone() },
            "aria-pressed": if toggle { if selected { "true" } else { "false" } },
            "aria-haspopup": if popup { "true" },
            "aria-expanded": if popup { if selected { "true" } else { "false" } },
            "aria-disabled": if disabled { "true" },
            "aria-keyshortcuts": if keyshortcuts.is_empty() { None } else { Some(keyshortcuts.clone()) },
            onclick: move |e| if !disabled { onclick.call(e) },
            Icon { name: shown }
            CounterBadge { count: badge, class: "ax-header-badge", label: "{{n}} unread notifications" }
        }
    }
}

/// "More options" (…): secondary header actions (§3.1 #10).
#[component]
fn MoreMenu() -> Element {
    let app = use_app();
    let mut shell = use_shell();
    let mut pos = use_signal(|| None::<(f64, f64)>);
    let mut anchor = use_signal(|| None::<Rc<MountedData>>);
    let mode = app.settings.read().keyboard_mode;
    let syncing = sync().map(|s| s.status().running).unwrap_or(false);
    let online = (shell.online)();
    let sync_item = MenuItem::new("sync", if syncing { "Syncing…" } else { "Sync" }).icon("arrow_sync").shortcut("F9");
    let items = vec![
        if !online {
            sync_item.disabled(true).tooltip("Sync (not available offline)")
        } else {
            sync_item.disabled(syncing)
        },
        MenuItem::new("help", "Help").icon("question_circle").shortcut("F1"),
        MenuItem::new("feedback", "Feedback").icon("person_feedback"),
        MenuItem::new("whats_new", "What's new").icon("megaphone"),
        if mode == axentra_core::KeyboardMode::Off {
            MenuItem::new("shortcuts", "Keyboard shortcuts").icon("keyboard")
        } else {
            MenuItem::new("shortcuts", "Keyboard shortcuts").icon("keyboard").shortcut("?")
        },
        MenuItem::divider(),
        MenuItem::new("about", format!("About {}", crate::APP_NAME)).icon("info"),
    ];
    let open = pos().is_some();
    rsx! {
        button {
            r#type: "button",
            class: "ax-header-btn",
            class: if open { "ax-selected" },
            title: "More options",
            "aria-label": "More options",
            "aria-haspopup": "menu",
            "aria-expanded": if open { "true" } else { "false" },
            onmounted: move |e| anchor.set(Some(e.data())),
            onclick: move |_| {
                spawn(async move {
                    let rect = match anchor() {
                        Some(m) => m.get_client_rect().await.ok(),
                        None => None,
                    };
                    pos.set(Some(rect.map(|r| (r.max_x(), r.max_y())).unwrap_or((0.0, 48.0))));
                });
            },
            Icon { name: "more_horizontal" }
        }
        if let Some((x, y)) = pos() {
            Menu {
                items,
                x,
                y,
                anchor_end: true,
                label: "More options",
                on_close: move |_| pos.set(None),
                on_select: move |id: String| match id.as_str() {
                    "sync" => run_command(app, shell, GlobalCmd::Sync),
                    "help" => run_command(app, shell, GlobalCmd::Help),
                    "feedback" => super::help::send_feedback(app),
                    "whats_new" => shell.whats_new_open.set(true),
                    "shortcuts" => shell.shortcuts_open.set(true),
                    "about" => shell.open_settings(app, crate::settings::PageId::About),
                    _ => {}
                },
            }
        }
    }
}
