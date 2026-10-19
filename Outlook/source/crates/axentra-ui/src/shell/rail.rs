//! Left app rail (docs/spec/shell.md §4): pinned modules, More apps,
//! pin/unpin, Up/Down/Home/End navigation.

use dioxus::prelude::*;

use super::keymap::{self, GlobalCmd};
use super::launcher::FlyoutPanel;
use super::{Flyout, use_shell};
use crate::kit::{Icon, IconButton, Menu, MenuItem, filled_icon};
use crate::state::{AppState, Module, use_app};

/// Settings id of a module (for `Settings::unpinned_apps`).
pub fn app_id(m: Module) -> &'static str {
    match m {
        Module::Mail => "mail",
        Module::Calendar => "calendar",
        Module::People => "people",
        Module::Tasks => "tasks",
    }
}

/// Mail and Calendar cannot be unpinned (§4.1).
pub fn can_unpin(m: Module) -> bool {
    matches!(m, Module::People | Module::Tasks)
}

fn command(m: Module) -> GlobalCmd {
    match m {
        Module::Mail => GlobalCmd::GoMail,
        Module::Calendar => GlobalCmd::GoCalendar,
        Module::People => GlobalCmd::GoPeople,
        Module::Tasks => GlobalCmd::GoTasks,
    }
}

/// Modules shown on the rail, in rail order: Mail and Calendar first, then
/// the movable ones in the user's order (§4.1).
pub fn pinned(unpinned: &[String], order: &[String]) -> Vec<Module> {
    let rank = |m: Module| order.iter().position(|o| o == app_id(m)).unwrap_or(order.len() + m as usize);
    let mut movable: Vec<Module> = Module::ALL.into_iter().filter(|m| can_unpin(*m) && !unpinned.iter().any(|u| u == app_id(*m))).collect();
    movable.sort_by_key(|m| rank(*m));
    Module::ALL.into_iter().filter(|m| !can_unpin(*m)).chain(movable).collect()
}

/// The movable part of the rail after moving `m` before `before` (or to
/// the end), as `Settings::rail_order` ids.
pub fn reordered(shown: &[Module], m: Module, before: Option<Module>) -> Vec<String> {
    let mut movable: Vec<Module> = shown.iter().copied().filter(|x| can_unpin(*x) && *x != m).collect();
    let at = before.and_then(|b| movable.iter().position(|x| *x == b)).unwrap_or(movable.len());
    movable.insert(at, m);
    movable.into_iter().map(|x| app_id(x).to_string()).collect()
}

/// Move a pinned module one place up (`-1`) or down (`1`) among the movable ones.
fn shift(app: AppState, m: Module, dir: i32) {
    let shown = {
        let s = app.settings.read();
        pinned(&s.unpinned_apps, &s.rail_order)
    };
    let movable: Vec<Module> = shown.iter().copied().filter(|x| can_unpin(*x)).collect();
    let Some(i) = movable.iter().position(|x| *x == m) else { return };
    let j = i as i32 + dir;
    if j < 0 || j as usize >= movable.len() {
        return;
    }
    let before = if dir < 0 { Some(movable[j as usize]) } else { movable.get(j as usize + 1).copied() };
    let order = reordered(&shown, m, before);
    app.update_settings(|s| s.rail_order = order);
}

fn set_pinned(app: AppState, m: Module, pin: bool) {
    if !can_unpin(m) {
        return;
    }
    app.update_settings(|s| {
        s.unpinned_apps.retain(|u| u != app_id(m));
        if !pin {
            s.unpinned_apps.push(app_id(m).to_string());
        }
    });
}

#[component]
pub fn AppRail() -> Element {
    let app = use_app();
    let shell = use_shell();
    let current = (app.module)();
    let (unpinned, order, mode) = {
        let s = app.settings.read();
        (s.unpinned_apps.clone(), s.rail_order.clone(), s.keyboard_mode)
    };
    let apps = pinned(&unpinned, &order);
    let more_open = (shell.flyout)() == Some(Flyout::MoreApps);
    let current_hidden = !apps.contains(&current);
    let dragging = use_signal(|| None::<Module>);
    let drop_before = use_signal(|| None::<Module>);
    let movable = apps.iter().filter(|m| can_unpin(**m)).count();
    rsx! {
        nav { class: "ax-rail", "aria-label": "App bar",
            div { class: "ax-rail-apps", role: "toolbar", "aria-orientation": "vertical", "aria-label": "Apps", "data-ax-roving": "vertical",
                for m in apps.iter().copied() {
                    RailItem {
                        key: "{m:?}",
                        module: m,
                        selected: m == current,
                        keys: keymap::aria_for(command(m), mode),
                        shown: apps.clone(),
                        movable,
                        dragging,
                        drop_before,
                    }
                }
                button {
                    r#type: "button",
                    class: "ax-rail-btn",
                    class: if more_open || current_hidden { "ax-selected" },
                    title: "More apps",
                    "aria-label": "More apps",
                    "aria-haspopup": "dialog",
                    "aria-expanded": if more_open { "true" } else { "false" },
                    "data-ax-tip-pos": "right",
                    "data-ax-item": "true",
                    tabindex: "-1",
                    onclick: move |_| shell.toggle_flyout(Flyout::MoreApps),
                    Icon { name: "apps" }
                }
            }
            if more_open { MoreAppsFlyout {} }
        }
    }
}

#[component]
fn RailItem(
    module: Module,
    selected: bool,
    keys: String,
    /// The rail's modules, in order.
    shown: Vec<Module>,
    /// How many of them can move.
    movable: usize,
    dragging: Signal<Option<Module>>,
    drop_before: Signal<Option<Module>>,
) -> Element {
    let app = use_app();
    let mut menu_at = use_signal(|| None::<(f64, f64)>);
    let mut dragging = dragging;
    let mut drop_before = drop_before;
    let icon = if selected { filled_icon(module.icon()) } else { module.icon().to_string() };
    let label = module.label();
    let can_move = can_unpin(module);
    let index = shown.iter().filter(|m| can_unpin(**m)).position(|m| *m == module);
    let insertion = drop_before() == Some(module) && dragging().is_some_and(|d| d != module);
    rsx! {
        button {
            r#type: "button",
            class: "ax-rail-btn",
            class: if selected { "ax-selected" },
            class: if insertion { "ax-drop-before" },
            draggable: if can_move { "true" } else { "false" },
            "data-ax-drop": if can_move { "true" },
            ondragstart: move |_| if can_move { dragging.set(Some(module)) },
            ondragend: move |_| {
                dragging.set(None);
                drop_before.set(None);
            },
            ondragover: move |_| if can_move && drop_before() != Some(module) { drop_before.set(Some(module)) },
            ondrop: {
                let shown = shown.clone();
                move |e: DragEvent| {
                    e.prevent_default();
                    if let Some(m) = dragging()
                        && can_move
                        && m != module
                    {
                        let order = reordered(&shown, m, Some(module));
                        app.update_settings(|s| s.rail_order = order);
                    }
                    dragging.set(None);
                    drop_before.set(None);
                }
            },
            title: "{label}",
            "aria-label": "{label}",
            "aria-current": if selected { "page" },
            "aria-keyshortcuts": if keys.is_empty() { None } else { Some(keys.clone()) },
            "data-ax-tip-pos": "right",
            "data-ax-item": "true",
            tabindex: if selected { "0" } else { "-1" },
            onclick: move |_| app.go(module),
            oncontextmenu: move |e: MouseEvent| {
                e.prevent_default();
                let c = e.client_coordinates();
                menu_at.set(Some((c.x, c.y)));
            },
            onkeydown: move |e: KeyboardEvent| {
                if e.key() == Key::ContextMenu || (e.key() == Key::F10 && e.modifiers().shift()) {
                    e.prevent_default();
                    menu_at.set(Some((48.0, 120.0)));
                }
            },
            Icon { name: icon }
        }
        if let Some((x, y)) = menu_at() {
            Menu {
                x,
                y,
                label: "{label}",
                items: vec![
                    MenuItem::new("open", format!("Open {label}")).icon(module.icon()),
                    if can_move {
                        MenuItem::new("unpin", "Unpin").icon("pin_off")
                    } else {
                        MenuItem::new("unpin", "Unpin").icon("pin_off").disabled(true).tooltip(format!("{label} can't be unpinned"))
                    },
                    MenuItem::divider(),
                    MenuItem::new("up", "Move up").icon("arrow_up").disabled(!can_move || index == Some(0)).tooltip(if can_move { "" } else { "Mail and Calendar stay at the top" }),
                    MenuItem::new("down", "Move down").icon("arrow_down").disabled(!can_move || index.map(|i| i + 1 >= movable).unwrap_or(true)),
                ],
                on_select: move |id: String| match id.as_str() {
                    "open" => app.go(module),
                    "unpin" => set_pinned(app, module, false),
                    "up" => shift(app, module, -1),
                    "down" => shift(app, module, 1),
                    _ => {}
                },
                on_close: move |_| menu_at.set(None),
            }
        }
    }
}

/// All apps, with pin / unpin (§4.3 "More apps").
#[component]
fn MoreAppsFlyout() -> Element {
    let app = use_app();
    let mut shell = use_shell();
    let current = (app.module)();
    let shown = {
        let s = app.settings.read();
        pinned(&s.unpinned_apps, &s.rail_order)
    };
    rsx! {
        FlyoutPanel { label: "More apps", class: "ax-more-apps", on_close: move |_| shell.flyout.set(None),
            div { class: "ax-flyout-title", "Apps" }
            div { class: "ax-more-apps-list", "data-ax-roving": "vertical",
                for m in Module::ALL {
                    div { key: "{m:?}", class: "ax-more-app",
                        button {
                            r#type: "button",
                            class: "ax-more-app-open",
                            class: if m == current { "ax-selected" },
                            "data-ax-item": "true",
                            "aria-current": if m == current { "page" },
                            onclick: move |_| {
                                app.go(m);
                                shell.flyout.set(None);
                            },
                            Icon { name: m.icon(), size: 20 }
                            span { "{m.label()}" }
                        }
                        if can_unpin(m) {
                            IconButton {
                                icon: if shown.contains(&m) { "pin_filled" } else { "pin" },
                                title: if shown.contains(&m) { format!("Unpin {} from the app bar", m.label()) } else { format!("Pin {} to the app bar", m.label()) },
                                toggle: true,
                                toggled: shown.contains(&m),
                                small: true,
                                onclick: move |_| {
                                    // Copy out first: `set_pinned` writes the settings.
                                    let shown = {
                                        let s = app.settings.read();
                                        pinned(&s.unpinned_apps, &s.rail_order).contains(&m)
                                    };
                                    set_pinned(app, m, !shown);
                                },
                            }
                        } else {
                            IconButton {
                                icon: "pin_filled",
                                title: format!("{} is always pinned", m.label()),
                                disabled: true,
                                small: true,
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mail_and_calendar_stay_pinned() {
        let all = pinned(&["mail".into(), "calendar".into(), "people".into()], &[]);
        assert_eq!(all, vec![Module::Mail, Module::Calendar, Module::Tasks]);
        assert_eq!(pinned(&[], &[]), Module::ALL.to_vec());
    }

    #[test]
    fn movable_apps_follow_the_saved_order() {
        let order = vec!["tasks".to_string(), "people".to_string()];
        assert_eq!(pinned(&[], &order), vec![Module::Mail, Module::Calendar, Module::Tasks, Module::People]);
        let shown = pinned(&[], &[]);
        assert_eq!(reordered(&shown, Module::Tasks, Some(Module::People)), vec!["tasks", "people"]);
        assert_eq!(reordered(&shown, Module::People, None), vec!["tasks", "people"]);
        // Mail can't be dragged into the movable part.
        assert_eq!(reordered(&shown, Module::People, Some(Module::Mail)), vec!["tasks", "people"]);
    }
}
