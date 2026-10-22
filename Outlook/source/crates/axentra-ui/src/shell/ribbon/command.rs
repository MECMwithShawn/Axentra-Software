//! One ribbon command (button, dropdown or split button) in any of the
//! three looks: Simplified (20px icon + label), Classic large (32px icon over
//! the label) and Classic small (16px icon beside the label).

use std::rc::Rc;

use dioxus::prelude::*;

use super::Command;
use crate::kit::{Icon, Menu, filled_icon, keys_to_aria};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    Simplified { icon_only: bool },
    Large,
    Small,
}

impl Look {
    fn class(self) -> &'static str {
        match self {
            Look::Simplified { .. } => "",
            Look::Large => "ax-cmd-large",
            Look::Small => "ax-cmd-small",
        }
    }

    fn icon_size(self) -> u32 {
        match self {
            Look::Simplified { .. } => 20,
            Look::Large => 32,
            Look::Small => 16,
        }
    }

    fn shows_label(self, cmd: &Command) -> bool {
        match self {
            Look::Simplified { icon_only } => !icon_only && !cmd.icon_only && !cmd.label.is_empty(),
            _ => !cmd.label.is_empty(),
        }
    }
}

#[component]
pub fn RibbonButton(
    cmd: Command,
    look: Look,
    #[props(into, default)] keytip: String,
    on_command: EventHandler<String>,
    /// The toolbar's tab stop (its first command); arrows move from there.
    #[props(default)]
    tab_stop: bool,
) -> Element {
    let mut menu_at = use_signal(|| None::<(f64, f64)>);
    let mut anchor = use_signal(|| None::<Rc<MountedData>>);
    let title = cmd.title();
    let aria_keys = cmd.shortcut.as_deref().map(keys_to_aria).filter(|k| !k.is_empty());
    let has_menu = !cmd.menu.is_empty();
    let disabled = cmd.disabled;
    let open_menu = move || {
        if disabled {
            return;
        }
        spawn(async move {
            let rect = match anchor() {
                Some(m) => m.get_client_rect().await.ok(),
                None => None,
            };
            menu_at.set(Some(rect.map(|r| (r.min_x(), r.max_y() + 4.0)).unwrap_or((60.0, 120.0))));
        });
    };
    let primary = if cmd.primary { "ax-primary" } else { "" };
    let toggled = if cmd.toggled { "ax-toggled" } else { "" };
    let icon = if cmd.toggled { filled_icon(&cmd.icon) } else { cmd.icon.clone() };
    let size = look.icon_size();
    let label = look.shows_label(&cmd).then(|| cmd.label.clone());
    let tabindex = if tab_stop { "0" } else { "-1" };
    let keep_focus = cmd.keep_focus;
    rsx! {
        if cmd.split {
            div {
                class: "ax-cmd-split {primary} {look.class()}",
                class: if disabled { "ax-disabled" },
                role: "group",
                "aria-label": "{cmd.label}",
                onmounted: move |e| anchor.set(Some(e.data())),
                button {
                    r#type: "button",
                    class: "ax-cmd {primary} {toggled} {look.class()}",
                    title: "{title}",
                    "aria-label": "{cmd.label}",
                    "aria-disabled": if disabled { "true" },
                    "aria-keyshortcuts": aria_keys.clone(),
                    "data-ax-item": "true",
                    "data-ax-keytip": if keytip.is_empty() { None } else { Some(keytip.clone()) },
                    tabindex,
                    onmousedown: move |e: MouseEvent| if keep_focus { e.prevent_default() },
                    onclick: {
                        let id = cmd.id.clone();
                        move |_| if !disabled { on_command.call(id.clone()) }
                    },
                    onkeydown: move |e: KeyboardEvent| {
                        if e.key() == Key::ArrowDown && e.modifiers().alt() {
                            e.prevent_default();
                            e.stop_propagation();
                            open_menu();
                        }
                    },
                    Icon { name: icon.clone(), size }
                    if let Some(l) = &label { span { class: "ax-cmd-label", "{l}" } }
                }
                button {
                    r#type: "button",
                    class: "ax-cmd ax-cmd-chevron {primary} {look.class()}",
                    title: "More {cmd.label} options",
                    "aria-label": "More {cmd.label} options",
                    "aria-haspopup": "menu",
                    "aria-expanded": if menu_at().is_some() { "true" } else { "false" },
                    "aria-disabled": if disabled { "true" },
                    "data-ax-item": "true",
                    tabindex: "-1",
                    onclick: move |_| open_menu(),
                    onkeydown: move |e: KeyboardEvent| {
                        if e.key() == Key::ArrowDown {
                            e.prevent_default();
                            e.stop_propagation();
                            open_menu();
                        }
                    },
                    Icon { name: "chevron_down", size: 12 }
                }
            }
        } else {
            button {
                r#type: "button",
                class: "ax-cmd {primary} {toggled} {look.class()}",
                class: if disabled { "ax-disabled" },
                class: if label.is_none() && !has_menu && matches!(look, Look::Simplified { .. }) { "ax-cmd-icononly" },
                title: "{title}",
                "aria-label": "{cmd.label}",
                "aria-haspopup": if has_menu { "menu" },
                "aria-expanded": if has_menu { if menu_at().is_some() { "true" } else { "false" } },
                "aria-pressed": if cmd.toggled { "true" },
                "aria-disabled": if disabled { "true" },
                "aria-keyshortcuts": aria_keys,
                "data-ax-item": "true",
                "data-ax-keytip": if keytip.is_empty() { None } else { Some(keytip.clone()) },
                tabindex,
                onmounted: move |e| anchor.set(Some(e.data())),
                onmousedown: move |e: MouseEvent| if keep_focus { e.prevent_default() },
                onclick: {
                    let id = cmd.id.clone();
                    move |_| {
                        if disabled {
                            return;
                        }
                        if has_menu { open_menu() } else { on_command.call(id.clone()) }
                    }
                },
                onkeydown: move |e: KeyboardEvent| {
                    if has_menu && e.key() == Key::ArrowDown {
                        e.prevent_default();
                        e.stop_propagation();
                        open_menu();
                    }
                },
                Icon { name: icon, size }
                if let Some(l) = &label { span { class: "ax-cmd-label", "{l}" } }
                if has_menu { Icon { name: "chevron_down", size: 12, class: "ax-cmd-caret" } }
            }
        }
        if let Some((x, y)) = menu_at() {
            Menu {
                items: cmd.menu.clone(),
                x,
                y,
                label: "{cmd.label}",
                on_select: move |id| on_command.call(id),
                on_close: move |_| menu_at.set(None),
            }
        }
    }
}
