//! Modal dialogs (Fluent DialogSurface): scrim, focus trap, initial focus,
//! Escape to close, Enter for the default button and focus return to the
//! invoker. Dialogs with a `resize_key` are resizable ([`super::surface`]).

use dioxus::prelude::*;

use super::button::{Button, IconButton, Variant};
use super::dom;
use super::surface::{SurfaceGrips, SurfaceSpec, use_surface};

/// Modal dialog. Escape and clicking the overlay call `on_close`.
///
/// Initial focus goes to `initial_focus`, else an element marked
/// `data-ax-autofocus` (the `autofocus` prop of kit buttons and fields), else
/// the first focusable element of the body, else the default button. Tab is
/// trapped inside; Enter outside buttons and lists presses the default
/// button (`on_default`, or a `Button { default_action: true }`); when the
/// dialog closes, focus returns to the element that opened it.
///
/// With `resize_key` the dialog is resizable from its edges and corners and
/// remembers its size under that key; `width` and `height` are then the
/// default size. Small confirmations stay fixed (no key).
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
    /// Enter (with focus outside buttons, links, text areas, menus and
    /// lists) runs this, as the dialog's default button would. A kit
    /// `Button { default_action: true }` in `actions` does the same.
    on_default: Option<EventHandler<()>>,
    /// Selector of the element to focus first (default: the element marked
    /// `autofocus`, else the first focusable element of the body).
    #[props(into)]
    initial_focus: Option<String>,
    /// Make the dialog resizable and remember its size under this key
    /// ("settings", "contact-form", ...; see `kit::surface`).
    #[props(into)]
    resize_key: Option<String>,
    /// Smallest size while resizing (default: `kit::surface::default_min`).
    min_width: Option<u32>,
    min_height: Option<u32>,
) -> Element {
    let id = dom::use_unique_id("ax-dialog");
    let surface = use_surface(&id, resize_key.clone().map(|k| SurfaceSpec::new(k, width, height).min(min_width, min_height)));
    let spec = surface.spec();
    // Focus goes back to whatever opened the dialog.
    use_drop({
        let id = id.clone();
        move || dom::close_dialog(&id)
    });
    let title_id = format!("{id}-title");
    let body_id = format!("{id}-body");
    let style = if spec.is_some() {
        surface.style()
    } else {
        let h = height.map(|h| format!(";height:min({h}px, calc(100vh - 48px))")).unwrap_or_default();
        format!("width:min({width}px, calc(100vw - 48px)){h}")
    };
    rsx! {
        div {
            class: "ax-dialog-overlay",
            onclick: move |_| if !modal_scrim { on_close.call(()) },
            div {
                id: "{id}",
                class: "ax-dialog {class}",
                class: if bare { "ax-dialog-bare" },
                class: if spec.is_some() { "ax-surface" },
                role: if alert { "alertdialog" } else { "dialog" },
                "aria-modal": "true",
                "aria-labelledby": if bare { None } else { Some(title_id.clone()) },
                "aria-label": if bare { Some(title.clone()) } else { None },
                "aria-describedby": if alert { Some(body_id.clone()) } else { None },
                tabindex: "-1",
                style: "{style}",
                onclick: move |e| e.stop_propagation(),
                onmounted: {
                    let id = id.clone();
                    let initial = initial_focus.clone();
                    move |_| dom::open_dialog_with(&id, initial.as_deref())
                },
                // Keys inside a modal never reach module or global shortcuts
                // (keyboard spec §2.1 scope 1).
                onkeydown: move |e: KeyboardEvent| {
                    e.stop_propagation();
                    if e.key() == Key::Escape {
                        on_close.call(());
                    }
                },
                ..surface.attributes(),
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
                if let Some(h) = on_default {
                    // Pressed by the Enter handling in dom.js.
                    button {
                        r#type: "button",
                        class: "ax-dialog-default",
                        hidden: true,
                        tabindex: "-1",
                        "aria-hidden": "true",
                        "data-ax-default": "true",
                        onclick: move |_| h.call(()),
                    }
                }
                if spec.is_some() {
                    SurfaceGrips {}
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
                    default_action: true,
                    onclick: move |_| on_confirm.call(()),
                }
                Button { label: cancel_label, autofocus: !focus_confirm, onclick: move |_| on_cancel.call(()) }
            },
            p { class: "ax-dialog-text", "{message}" }
        }
    }
}
