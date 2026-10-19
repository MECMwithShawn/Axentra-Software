//! Small building blocks for the account surfaces: the modal used by the
//! add-account flow (Fluent DialogSurface metrics, §2.1), form fields with
//! validation state, message bars (§8.5) and the advanced-settings expander.

use dioxus::prelude::*;

use crate::kit::{Icon, IconButton, keys};

/// Copy text to the clipboard (with a fallback for webviews without the
/// async Clipboard API).
pub fn copy_to_clipboard(text: &str) {
    let json = serde_json::to_string(text).unwrap_or_else(|_| "\"\"".into());
    let script = format!(
        "(function(t){{try{{if(navigator.clipboard&&navigator.clipboard.writeText){{navigator.clipboard.writeText(t).catch(function(){{f(t)}});return}}}}catch(e){{}}f(t);\
         function f(t){{var a=document.createElement('textarea');a.value=t;a.style.position='fixed';a.style.opacity='0';document.body.appendChild(a);a.select();\
         try{{document.execCommand('copy')}}catch(e){{}}document.body.removeChild(a)}}}})({json})"
    );
    let _ = document::eval(&script);
}

/// Move focus to the first / last focusable control of the open account
/// dialog (focus trap sentinels).
fn focus_edge(last: bool) {
    let pick = if last { "f[f.length-1]" } else { "f[0]" };
    let _ = document::eval(&format!(
        "(function(){{var d=document.querySelector('.ax-acct-dialog');if(!d)return;\
         var f=[].slice.call(d.querySelectorAll('button:not([disabled]),input:not([disabled]),select:not([disabled]),textarea,[tabindex=\"0\"]'));\
         if(f.length){{{pick}.focus()}}}})()"
    ));
}

/// Modal dialog for the account flows: 480 px, title in Subtitle 1, actions
/// right-aligned (primary last). Escape calls `on_cancel` and never reaches
/// the Settings dialog underneath; clicks on the backdrop are ignored so an
/// in-progress sign-in can't be dismissed by accident.
#[component]
pub fn AcctDialog(
    #[props(into)] title: String,
    on_cancel: EventHandler<()>,
    children: Element,
    actions: Option<Element>,
    /// Left-aligned extra action (e.g. "Test connection").
    footer_start: Option<Element>,
    #[props(default = 480)] width: u32,
    #[props(default = "dialog".to_string(), into)] role: String,
    #[props(into, default)] described_by: String,
) -> Element {
    // Remember what had focus, and give it back when the dialog closes (or,
    // when that element is gone, the account list / settings dialog).
    use_hook(|| {
        let _ = document::eval("(window.__axAcctOpeners=window.__axAcctOpeners||[]).push(document.activeElement)");
    });
    use_drop(|| {
        let _ = document::eval(
            "setTimeout(function(){var o=(window.__axAcctOpeners||[]).pop();\
             if(o&&document.contains(o)&&!o.disabled){o.focus();return}\
             var t=document.querySelector('.ax-acct-list .ax-acct-row-target[tabindex=\"0\"]')||document.querySelector('.ax-dialog');\
             if(t)t.focus()},30)",
        );
    });
    rsx! {
        div {
            class: "ax-acct-overlay",
            onclick: move |e| e.stop_propagation(),
            // Enter on a button activates it natively; in fields `Field::onenter`
            // runs the primary action (§2.3).
            onkeydown: move |e: KeyboardEvent| {
                e.stop_propagation();
                if e.key() == Key::Escape {
                    e.prevent_default();
                    on_cancel.call(());
                }
            },
            div { tabindex: "0", class: "ax-acct-sentinel", onfocus: move |_| focus_edge(true) }
            div {
                class: "ax-acct-dialog",
                role: "{role}",
                "aria-modal": "true",
                "aria-labelledby": "ax-acct-dialog-title",
                "aria-describedby": if described_by.is_empty() { None } else { Some(described_by.clone()) },
                style: "width:{width}px",
                div { class: "ax-acct-dialog-head",
                    h2 { id: "ax-acct-dialog-title", class: "ax-acct-dialog-title", "{title}" }
                    IconButton { icon: "dismiss", title: "Close", class: "ax-acct-dialog-close", onclick: move |_| on_cancel.call(()) }
                }
                div { class: "ax-acct-dialog-body", {children} }
                if actions.is_some() || footer_start.is_some() {
                    div { class: "ax-acct-dialog-actions",
                        if let Some(s) = footer_start { div { class: "ax-acct-dialog-actions-start", {s} } }
                        div { class: "ax-grow" }
                        if let Some(a) = actions { {a} }
                    }
                }
            }
            div { tabindex: "0", class: "ax-acct-sentinel", onfocus: move |_| focus_edge(false) }
        }
    }
}

/// A labelled input with Fluent `Field` validation styling (§2.3).
#[component]
pub fn Field(
    #[props(into)] label: String,
    #[props(into)] value: String,
    oninput: EventHandler<String>,
    #[props(into, default)] placeholder: String,
    #[props(into, default = "text".to_string())] input_type: String,
    #[props(into)] error: Option<String>,
    #[props(into)] hint: Option<String>,
    #[props(default)] autofocus: bool,
    #[props(default)] disabled: bool,
    #[props(into, default)] class: String,
    #[props(into, default)] autocomplete: String,
    #[props(into, default)] input_mode: String,
    /// Enter in the field (the dialog's primary action).
    onenter: Option<EventHandler<()>>,
) -> Element {
    let id = use_hook(|| format!("ax-acct-f{}", next_id()));
    let err_id = format!("{id}-err");
    let invalid = error.is_some();
    rsx! {
        div { class: "ax-acct-field {class}",
            label { class: "ax-acct-label", r#for: "{id}", "{label}" }
            input {
                id: "{id}",
                class: "ax-acct-input",
                class: if invalid { "ax-invalid" },
                r#type: "{input_type}",
                value: "{value}",
                placeholder: "{placeholder}",
                disabled,
                autocomplete: if autocomplete.is_empty() { "off".to_string() } else { autocomplete.clone() },
                inputmode: if input_mode.is_empty() { None } else { Some(input_mode.clone()) },
                spellcheck: "false",
                "aria-invalid": if invalid { "true" } else { "false" },
                "aria-describedby": if invalid { Some(err_id.clone()) } else { None },
                onmounted: move |e| async move {
                    if autofocus {
                        let _ = e.set_focus(true).await;
                    }
                },
                oninput: move |e| oninput.call(e.value()),
                onkeydown: move |e: KeyboardEvent| {
                    keys::stop_typing_keys(&e);
                    if e.key() == Key::Enter
                        && let Some(h) = &onenter
                    {
                        e.prevent_default();
                        h.call(());
                    }
                },
            }
            if let Some(err) = &error {
                div { id: "{err_id}", class: "ax-acct-field-error", role: "alert", "{err}" }
            } else if let Some(h) = &hint {
                div { class: "ax-acct-field-hint", "{h}" }
            }
        }
    }
}

fn next_id() -> u64 {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Fluent MessageBar (§8.5). `intent`: "info", "warning", "error", "success".
#[component]
pub fn MessageBar(
    #[props(into)] intent: String,
    #[props(into)] text: String,
    /// Details shown behind a "Details" expander (TLS errors).
    #[props(into)] detail: Option<String>,
    actions: Option<Element>,
    on_dismiss: Option<EventHandler<()>>,
    #[props(into, default)] class: String,
) -> Element {
    let mut open = use_signal(|| false);
    let icon = match intent.as_str() {
        "error" => "error_circle",
        "warning" => "warning",
        "success" => "checkmark_circle",
        _ => "info",
    };
    let role = if intent == "error" { "alert" } else { "status" };
    rsx! {
        div { class: "ax-acct-bar ax-acct-bar-{intent} {class}", role: "{role}",
            div { class: "ax-acct-bar-main",
                Icon { name: icon, size: 16, class: "ax-acct-bar-icon" }
                div { class: "ax-acct-bar-text",
                    span { "{text}" }
                    if detail.is_some() {
                        button {
                            r#type: "button",
                            class: "ax-acct-link ax-acct-bar-more",
                            "aria-expanded": if open() { "true" } else { "false" },
                            onclick: move |_| open.toggle(),
                            if open() { "Hide details" } else { "Details" }
                        }
                    }
                }
                if let Some(a) = actions { div { class: "ax-acct-bar-actions", {a} } }
                if let Some(d) = on_dismiss {
                    IconButton { icon: "dismiss", title: "Dismiss", small: true, onclick: move |_| d.call(()) }
                }
            }
            if let Some(d) = detail.as_ref().filter(|_| open()) {
                pre { class: "ax-acct-bar-detail ax-selectable", "{d}" }
            }
        }
    }
}

/// Fluent medium button for dialog actions (32 px tall, min width 96 px).
#[component]
pub fn DialogButton(
    #[props(into)] label: String,
    onclick: EventHandler<MouseEvent>,
    #[props(default)] primary: bool,
    #[props(default)] danger: bool,
    #[props(default)] disabled: bool,
    #[props(default)] autofocus: bool,
    #[props(into)] title: Option<String>,
) -> Element {
    let kind = if danger { "ax-danger" } else if primary { "ax-primary" } else { "" };
    rsx! {
        button {
            r#type: "button",
            class: "ax-btn ax-acct-btn {kind}",
            disabled,
            title: title.clone().unwrap_or_default(),
            "aria-disabled": if disabled { "true" } else { "false" },
            onmounted: move |e| async move {
                if autofocus {
                    let _ = e.set_focus(true).await;
                }
            },
            onclick: move |e| onclick.call(e),
            "{label}"
        }
    }
}

/// A link-styled button (Fluent `Link` as button).
#[component]
pub fn LinkButton(#[props(into)] label: String, onclick: EventHandler<MouseEvent>, #[props(default)] disabled: bool, #[props(into)] title: Option<String>) -> Element {
    rsx! {
        button {
            r#type: "button",
            class: "ax-acct-link",
            disabled,
            title: title.clone().unwrap_or_default(),
            onclick: move |e| onclick.call(e),
            "{label}"
        }
    }
}

/// "Advanced" disclosure.
#[component]
pub fn Expander(#[props(into)] label: String, open: bool, ontoggle: EventHandler<()>, children: Element) -> Element {
    rsx! {
        div { class: "ax-acct-expander",
            button {
                r#type: "button",
                class: "ax-acct-expander-head",
                "aria-expanded": if open { "true" } else { "false" },
                onclick: move |_| ontoggle.call(()),
                Icon { name: if open { "chevron_down" } else { "chevron_right" }, size: 16 }
                span { "{label}" }
            }
            if open { div { class: "ax-acct-expander-body", {children} } }
        }
    }
}

/// 16 px spinner with a label (Fluent extra-tiny spinner).
#[component]
pub fn Busy(#[props(into)] label: String) -> Element {
    rsx! {
        div { class: "ax-acct-busy", role: "status", "aria-live": "polite",
            span { class: "ax-acct-spinner", "aria-hidden": "true" }
            span { "{label}" }
        }
    }
}
