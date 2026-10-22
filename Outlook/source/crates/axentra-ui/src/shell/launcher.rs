//! Header flyout container and the app launcher (waffle) flyout (§3.1 #1).

use dioxus::prelude::*;

use super::use_shell;
use crate::kit::{Icon, dom, filled_icon};
use crate::state::{Module, use_app};

/// A non-modal flyout under a header or rail button: closes on Escape, an
/// outside click or when focus moves out of it (Tab past its last item),
/// takes focus when it opens and gives it back when it closes.
#[component]
pub fn FlyoutPanel(
    #[props(into)] label: String,
    #[props(into, default)] class: String,
    on_close: EventHandler<()>,
    children: Element,
) -> Element {
    let id = dom::use_unique_id("ax-flyout");
    rsx! {
        div {
            class: "ax-flyout-backdrop",
            onclick: move |e| { e.stop_propagation(); on_close.call(()) },
        }
        div {
            id: "{id}",
            class: "ax-flyout {class}",
            role: "dialog",
            "aria-label": "{label}",
            tabindex: "-1",
            onmounted: {
                let id = id.clone();
                move |_| dom::open_popup(&id)
            },
            onclick: move |e| e.stop_propagation(),
            onfocusout: {
                let id = id.clone();
                move |_| {
                    // Checked once focus has landed: on another control of the
                    // page (not lost to the page body, not still inside). A
                    // mouse press on the open flyout's own button leaves it
                    // to that button's click, which toggles it; keyboard focus
                    // moving there (Shift+Tab) closes it like any other exit.
                    let js = format!(
                        "await new Promise((r) => setTimeout(r, 0)); const el = document.getElementById({}); const a = document.activeElement; \
                         const kbd = document.documentElement.hasAttribute('data-ax-kbd'); \
                         return !!el && !!a && a !== document.body && !el.contains(a) && (kbd || !a.closest('[aria-expanded=\"true\"]'));",
                        dom::js_str(&id)
                    );
                    spawn(async move {
                        if let Ok(true) = document::eval(&js).join::<bool>().await {
                            on_close.call(());
                        }
                    });
                }
            },
            onkeydown: move |e: KeyboardEvent| {
                if e.key() == Key::Escape {
                    e.stop_propagation();
                    on_close.call(());
                }
            },
            {children}
        }
    }
}

/// Grid of Axentra apps. Arrow keys move in the grid; Enter opens.
#[component]
pub fn LauncherFlyout() -> Element {
    let app = use_app();
    let mut shell = use_shell();
    let current = (app.module)();
    rsx! {
        FlyoutPanel { label: "App launcher", class: "ax-launcher", on_close: move |_| shell.flyout.set(None),
            div { class: "ax-flyout-title", "Apps" }
            div { class: "ax-launcher-grid", role: "menu", "aria-label": "Apps", "data-ax-roving": "grid", "data-ax-cols": "3",
                for m in Module::ALL {
                    button {
                        key: "{m:?}",
                        r#type: "button",
                        role: "menuitem",
                        class: "ax-launcher-app",
                        class: if m == current { "ax-selected" },
                        "data-ax-item": "true",
                        "aria-current": if m == current { "page" },
                        tabindex: if m == current { "0" } else { "-1" },
                        onclick: move |_| {
                            app.go(m);
                            shell.flyout.set(None);
                        },
                        span { class: "ax-launcher-icon",
                            Icon { name: if m == current { filled_icon(m.icon()) } else { m.icon().to_string() }, size: 24 }
                        }
                        span { "{m.label()}" }
                    }
                }
            }
        }
    }
}
