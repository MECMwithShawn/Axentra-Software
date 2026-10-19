//! Modal dialogs (Fluent DialogSurface): scrim, focus trap, initial focus,
//! Escape to close and focus return to the invoker.

use dioxus::prelude::*;

use super::button::{Button, IconButton, Variant};
use super::dom;

/// Modal dialog. Escape and clicking the overlay call `on_close`.
///
/// Initial focus goes to an element marked `data-ax-autofocus` (the
/// `autofocus` prop of kit buttons and fields), else the first focusable
/// element of the body. Tab is trapped inside; when the dialog closes,
/// focus returns to the element that was focused before it opened.
#[component]
pub fn Dialog(
    #[props(into)] title: String,
    on_close: EventHandler<()>,
    children: Element,
    actions: Option<Element>,
    #[props(default = 480)] width: u32,
    #[props(into, default)] class: String,
    #[props(default = true)] close_button: bool,
    /// No title row or padding: `children` fill the surface (Settings).
    #[props(default)]
    bare: bool,
    /// `role="alertdialog"` for destructive confirmations.
    #[props(default)]
    alert: bool,
    /// Fixed height in px (the body scrolls).
    height: Option<u32>,
    /// Clicking the scrim does not close the dialog.
    #[props(default)]
    modal_scrim: bool,
) -> Element {
    let id = dom::use_unique_id("ax-dialog");
    let title_id = format!("{id}-title");
    let body_id = format!("{id}-body");
    let h = height.map(|h| format!(";height:min({h}px, calc(100vh - 48px))")).unwrap_or_default();
    rsx! {
        div {
            class: "ax-dialog-overlay",
            onclick: move |_| if !modal_scrim { on_close.call(()) },
            div {
                id: "{id}",
                class: "ax-dialog {class}",
                class: if bare { "ax-dialog-bare" },
                role: if alert { "alertdialog" } else { "dialog" },
                "aria-modal": "true",
                "aria-labelledby": if bare { None } else { Some(title_id.clone()) },
                "aria-label": if bare { Some(title.clone()) } else { None },
                "aria-describedby": if alert { Some(body_id.clone()) } else { None },
                tabindex: "-1",
                style: "width:min({width}px, calc(100vw - 48px)){h}",
                onclick: move |e| e.stop_propagation(),
                onmounted: {
                    let id = id.clone();
                    move |_| dom::open_dialog(&id)
                },
                // Keys inside a modal never reach module or global shortcuts
                // (keyboard spec §2.1 scope 1).
                onkeydown: move |e: KeyboardEvent| {
                    e.stop_propagation();
                    if e.key() == Key::Escape {
                        on_close.call(());
                    }
                },
                if bare {
                    {children}
                } else {
                    div { class: "ax-dialog-title",
                        h2 { id: "{title_id}", class: "ax-grow", "{title}" }
                        if close_button { IconButton { icon: "dismiss", title: "Close", onclick: move |_| on_close.call(()) } }
                    }
                    div { id: "{body_id}", class: "ax-dialog-body", {children} }
                    if let Some(a) = actions { div { class: "ax-dialog-actions", {a} } }
                }
            }
        }
    }
}

/// Two-button confirmation (`alertdialog`). Escape cancels.
#[component]
pub fn ConfirmDialog(
    #[props(into)] title: String,
    #[props(into)] message: String,
    #[props(into)] confirm_label: String,
    #[props(into, default = "Cancel".to_string())] cancel_label: String,
    on_confirm: EventHandler<()>,
    on_cancel: EventHandler<()>,
    #[props(default)] danger: bool,
    /// Initial focus on the confirm button instead of Cancel.
    #[props(default)]
    focus_confirm: bool,
) -> Element {
    rsx! {
        Dialog {
            title,
            alert: true,
            width: 448,
            close_button: false,
            on_close: move |_| on_cancel.call(()),
            actions: rsx! {
                Button {
                    label: confirm_label,
                    variant: if danger { Variant::Danger } else { Variant::Primary },
                    autofocus: focus_confirm,
                    onclick: move |_| on_confirm.call(()),
                }
                Button { label: cancel_label, autofocus: !focus_confirm, onclick: move |_| on_cancel.call(()) }
            },
            p { class: "ax-dialog-text", "{message}" }
        }
    }
}
