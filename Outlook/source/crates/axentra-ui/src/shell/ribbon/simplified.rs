//! Simplified ribbon row: one line of commands that drop their labels and
//! then move into "More options (…)" as the window narrows (§6.11).

use std::rc::Rc;

use dioxus::prelude::*;

use super::command::{Look, RibbonButton};
use super::custom::LabelMode;
use super::layout::{self, Fit};
use super::{Command, RibbonTab};
use crate::kit::{Icon, Menu, MenuItem};

/// Menu id of the Customize entry in More options.
pub const CUSTOMIZE_ID: &str = "shell.customize";

/// Ignore width changes smaller than this, so the row doesn't flicker.
const HYSTERESIS: f64 = 4.0;

/// A visible command: (index in its group, icon only, key tip).
type Slot = (usize, bool, String);

#[component]
pub fn SimplifiedRow(
    tab: RibbonTab,
    on_command: EventHandler<String>,
    #[props(default)] labels: LabelMode,
    /// Buttons the user removed in Customize (listed under More options).
    #[props(default)]
    removed: Vec<Command>,
    /// Offer Customize under More options.
    #[props(default)]
    customizable: bool,
) -> Element {
    let mut width = use_signal(|| 0.0f64);
    let more_always = customizable || !removed.is_empty();
    let fit = if width() > 0.0 {
        layout::fit_simplified_with(&tab.groups, width(), labels, more_always)
    } else {
        Fit {
            visible: tab.groups.iter().enumerate().flat_map(|(g, cmds)| (0..cmds.len()).map(move |c| (g, c, false))).collect(),
            overflow: Vec::new(),
        }
    };
    let tips = layout::assign_keytips(
        &fit.visible
            .iter()
            .map(|&(g, c, _)| (tab.groups[g][c].label.clone(), tab.groups[g][c].keytip.clone()))
            .chain([("More options".to_string(), Some("ZM".to_string())), ("Switch layout".to_string(), Some("ZL".to_string()))])
            .collect::<Vec<_>>(),
    );
    let mut overflow_items: Vec<MenuItem> = fit.overflow.iter().map(|&(g, c)| tab.groups[g][c].as_menu_item()).collect();
    if !removed.is_empty() {
        if !overflow_items.is_empty() {
            overflow_items.push(MenuItem::divider());
        }
        overflow_items.extend(removed.iter().map(Command::as_menu_item));
    }
    if customizable {
        if !overflow_items.is_empty() {
            overflow_items.push(MenuItem::divider());
        }
        overflow_items.push(MenuItem::new(CUSTOMIZE_ID, "Customize").icon("options").tooltip(format!("Choose the buttons on the {} tab", tab.label)));
    }
    // Visible commands per group, keeping ribbon order.
    let mut rows: Vec<Vec<Slot>> = vec![Vec::new(); tab.groups.len()];
    for (i, &(g, c, icon_only)) in fit.visible.iter().enumerate() {
        rows[g].push((c, icon_only, tips.get(i).cloned().unwrap_or_default()));
    }
    let groups: Vec<(usize, Vec<Slot>)> = rows.into_iter().enumerate().filter(|(_, r)| !r.is_empty()).collect();
    let more_tip = tips.get(fit.visible.len()).cloned().unwrap_or_default();
    let switch_tip = tips.get(fit.visible.len() + 1).cloned().unwrap_or_default();
    rsx! {
        div {
            class: "ax-ribbon-commands",
            role: "toolbar",
            "aria-label": "{tab.label}",
            "aria-orientation": "horizontal",
            "data-ax-roving": "horizontal",
            "data-ax-tab": "{tab.id}",
            onresize: move |e: ResizeEvent| {
                if let Ok(size) = e.get_content_box_size()
                    && (size.width - width()).abs() >= HYSTERESIS
                {
                    width.set(size.width);
                }
            },
            div { class: "ax-ribbon-row",
                for (gi, (g, cmds)) in groups.iter().cloned().enumerate() {
                    div { key: "g{g}", class: "ax-ribbon-group",
                        if gi > 0 { div { class: "ax-ribbon-sep", role: "separator", "aria-orientation": "vertical" } }
                        for (ci, (c, icon_only, tip)) in cmds.into_iter().enumerate() {
                            RibbonButton {
                                key: "{tab.groups[g][c].id}",
                                cmd: tab.groups[g][c].clone(),
                                look: Look::Simplified { icon_only },
                                keytip: tip,
                                tab_stop: gi == 0 && ci == 0,
                                on_command,
                            }
                        }
                    }
                }
            }
            div { class: "ax-ribbon-end",
                if !overflow_items.is_empty() {
                    OverflowButton { items: overflow_items, keytip: more_tip, on_command }
                }
                LayoutSwitch { simplified: true, keytip: switch_tip, on_command }
            }
        }
    }
}

/// "More options (…)": the commands that didn't fit, in ribbon order.
#[component]
pub fn OverflowButton(items: Vec<crate::kit::MenuItem>, keytip: String, on_command: EventHandler<String>) -> Element {
    let mut menu_at = use_signal(|| None::<(f64, f64)>);
    let mut anchor = use_signal(|| None::<Rc<MountedData>>);
    let open = move || {
        spawn(async move {
            let rect = match anchor() {
                Some(m) => m.get_client_rect().await.ok(),
                None => None,
            };
            menu_at.set(Some(rect.map(|r| (r.max_x(), r.max_y() + 4.0)).unwrap_or((600.0, 120.0))));
        });
    };
    rsx! {
        button {
            r#type: "button",
            class: "ax-cmd ax-ribbon-more",
            title: "More options",
            "aria-label": "More options",
            "aria-haspopup": "menu",
            "aria-expanded": if menu_at().is_some() { "true" } else { "false" },
            "data-ax-item": "true",
            "data-ax-keytip": "{keytip}",
            tabindex: "-1",
            onmounted: move |e| anchor.set(Some(e.data())),
            onclick: move |_| open(),
            onkeydown: move |e: KeyboardEvent| {
                if e.key() == Key::ArrowDown {
                    e.prevent_default();
                    e.stop_propagation();
                    open();
                }
            },
            Icon { name: "more_horizontal" }
        }
        if let Some((x, y)) = menu_at() {
            Menu { items, x, y, anchor_end: true, label: "More options", on_select: move |id| on_command.call(id), on_close: move |_| menu_at.set(None) }
        }
    }
}

/// The caret at the far right that switches Simplified ↔ Classic (§6.1).
#[component]
pub fn LayoutSwitch(simplified: bool, keytip: String, on_command: EventHandler<String>) -> Element {
    let title = if simplified { "Switch to classic ribbon" } else { "Switch to simplified ribbon" };
    rsx! {
        button {
            r#type: "button",
            class: "ax-cmd ax-ribbon-switch",
            title,
            "aria-label": title,
            "data-ax-item": "true",
            "data-ax-keytip": "{keytip}",
            tabindex: "-1",
            onclick: move |_| on_command.call("shell.toggle_layout".into()),
            Icon { name: if simplified { "chevron_down" } else { "chevron_up" }, size: 16 }
        }
    }
}
