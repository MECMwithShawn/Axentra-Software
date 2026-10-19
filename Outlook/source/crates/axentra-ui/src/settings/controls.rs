//! Settings page building blocks, shared by every page (module-owned pages
//! included): the draft context, sections, option rows and notes.

use axentra_core::Settings;
use dioxus::prelude::*;

use super::pages::PageId;
use crate::kit::{EmptyState, Icon};

/// The settings draft a page edits. Pages change `draft`; the dialog's
/// footer saves it (or discards it back to `base`).
#[derive(Clone, Copy)]
pub struct SettingsCtx {
    pub draft: Signal<Settings>,
    pub base: Signal<Settings>,
    /// Navigate to another page (asks first when there are unsaved edits).
    pub go: Signal<Option<PageId>>,
}

impl SettingsCtx {
    pub fn get(&self) -> Settings {
        self.draft.read().clone()
    }

    pub fn edit(mut self, f: impl FnOnce(&mut Settings)) {
        f(&mut self.draft.write());
    }

    pub fn dirty(&self) -> bool {
        *self.draft.read() != *self.base.read()
    }

    /// Ask the dialog to show another page.
    pub fn open(mut self, page: PageId) {
        self.go.set(Some(page));
    }
}

pub fn use_settings() -> SettingsCtx {
    use_context::<SettingsCtx>()
}

/// A titled group of options. Its id lets search jump to it.
#[component]
pub fn Section(#[props(into)] title: String, #[props(into, default)] description: String, children: Element) -> Element {
    rsx! {
        section { class: "ax-set-section", id: "{PageId::anchor(&title)}",
            h3 { class: "ax-set-section-title", "{title}" }
            if !description.is_empty() { p { class: "ax-set-desc", "{description}" } }
            div { class: "ax-set-section-body", {children} }
        }
    }
}

/// A labelled option row: label (and description) on the left, the control
/// on the right.
#[component]
pub fn Row(#[props(into)] label: String, #[props(into, default)] description: String, children: Element) -> Element {
    rsx! {
        div { class: "ax-set-row", id: "{PageId::anchor(&label)}",
            div { class: "ax-set-row-text",
                span { "{label}" }
                if !description.is_empty() { span { class: "ax-set-desc", "{description}" } }
            }
            div { class: "ax-set-row-control", {children} }
        }
    }
}

/// An inline explanatory line (info icon + text).
#[component]
pub fn Note(#[props(into)] text: String, #[props(into, default = "info".to_string())] icon: String) -> Element {
    rsx! {
        p { class: "ax-set-note", Icon { name: icon, size: 16 } span { "{text}" } }
    }
}

/// Text button styled as a link.
#[component]
pub fn LinkButton(#[props(into)] label: String, onclick: EventHandler<MouseEvent>, #[props(into)] title: Option<String>) -> Element {
    rsx! {
        button { r#type: "button", class: "ax-link-btn", title, onclick: move |e| onclick.call(e), "{label}" }
    }
}

/// Body of a page whose feature belongs to a module that isn't available
/// for the account yet: an empty state that says so.
#[component]
pub fn Unavailable(#[props(into)] icon: String, #[props(into)] title: String, #[props(into)] text: String, children: Option<Element>) -> Element {
    rsx! {
        div { class: "ax-set-empty",
            EmptyState { icon, title, subtitle: text,
                if let Some(c) = children { {c} }
            }
        }
    }
}

/// The disabled state for options an administrator manages (§7.1).
#[component]
pub fn Managed(children: Element) -> Element {
    rsx! {
        div { class: "ax-set-managed", "aria-disabled": "true", title: "Managed by your organization",
            Icon { name: "lock_closed", size: 16 }
            span { "Managed by your organization" }
            {children}
        }
    }
}
