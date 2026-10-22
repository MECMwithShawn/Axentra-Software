//! Form controls: Checkbox, Switch, RadioGroup, TextField, SearchField,
//! Select, Slider and Pivot (tabs).

use dioxus::prelude::*;

use super::button::Icon;
use super::keys;

fn is_space(e: &KeyboardEvent) -> bool {
    e.key() == Key::Character(" ".into())
}

#[component]
pub fn Checkbox(
    checked: bool,
    #[props(into, default)] label: String,
    onchange: EventHandler<bool>,
    #[props(default)] round: bool,
    #[props(default)] disabled: bool,
    #[props(into)] color: Option<String>,
    /// Secondary text under the label.
    #[props(into, default)]
    description: String,
    /// Tooltip (e.g. why the box is disabled).
    #[props(into)]
    title: Option<String>,
    /// Accessible name when there is no visible label.
    #[props(into)]
    aria_label: Option<String>,
) -> Element {
    let style = match (&color, checked) {
        (Some(c), true) => format!("background:{c};border-color:{c}"),
        (Some(c), false) => format!("border-color:{c}"),
        _ => String::new(),
    };
    rsx! {
        label {
            class: "ax-checkbox",
            class: if checked { "ax-checked" },
            class: if round { "ax-round" },
            class: if disabled { "ax-disabled" },
            role: "checkbox",
            "aria-checked": if checked { "true" } else { "false" },
            "aria-disabled": if disabled { "true" } else { "false" },
            "aria-label": aria_label,
            title,
            tabindex: "0",
            onclick: move |e| { e.stop_propagation(); e.prevent_default(); if !disabled { onchange.call(!checked) } },
            onkeydown: move |e: KeyboardEvent| if !disabled && is_space(&e) { e.prevent_default(); e.stop_propagation(); onchange.call(!checked) },
            span { class: "ax-checkbox-box", style: "{style}",
                if checked { Icon { name: "checkmark", size: 12 } }
            }
            if !label.is_empty() || !description.is_empty() {
                span { class: "ax-choice-text",
                    if !label.is_empty() { span { "{label}" } }
                    if !description.is_empty() { span { class: "ax-choice-desc", "{description}" } }
                }
            }
        }
    }
}

#[component]
pub fn Switch(
    checked: bool,
    #[props(into, default)] label: String,
    onchange: EventHandler<bool>,
    #[props(default)] disabled: bool,
    #[props(into)] title: Option<String>,
    /// Put the label before the track (settings rows).
    #[props(default)]
    label_first: bool,
) -> Element {
    rsx! {
        label {
            class: "ax-switch",
            class: if checked { "ax-checked" },
            class: if disabled { "ax-disabled" },
            class: if label_first { "ax-label-first" },
            role: "switch",
            tabindex: "0",
            title,
            "aria-checked": if checked { "true" } else { "false" },
            "aria-disabled": if disabled { "true" } else { "false" },
            onclick: move |e| { e.prevent_default(); if !disabled { onchange.call(!checked) } },
            onkeydown: move |e: KeyboardEvent| if !disabled && (e.key() == Key::Enter || is_space(&e)) { e.prevent_default(); e.stop_propagation(); onchange.call(!checked) },
            span { class: "ax-switch-track", span { class: "ax-switch-thumb" } }
            if !label.is_empty() { span { class: "ax-switch-label", "{label}" } }
        }
    }
}

/// One option of a [`RadioGroup`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RadioOption {
    pub value: String,
    pub label: String,
    pub description: String,
    pub disabled: bool,
    /// Tooltip, e.g. why the option is disabled.
    pub tooltip: Option<String>,
}

impl RadioOption {
    pub fn new(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self { value: value.into(), label: label.into(), ..Default::default() }
    }
    pub fn description(mut self, d: impl Into<String>) -> Self {
        self.description = d.into();
        self
    }
    pub fn disabled(mut self, reason: impl Into<String>) -> Self {
        self.disabled = true;
        self.tooltip = Some(reason.into());
        self
    }
}

/// Fluent RadioGroup: one tab stop, arrow keys move and select.
#[component]
pub fn RadioGroup(
    #[props(into)] value: String,
    options: Vec<RadioOption>,
    onchange: EventHandler<String>,
    /// Accessible name (the visible heading usually labels it).
    #[props(into)]
    label: String,
    #[props(default)] horizontal: bool,
    #[props(default)] disabled: bool,
) -> Element {
    rsx! {
        div {
            class: "ax-radio-group",
            class: if horizontal { "ax-horizontal" },
            role: "radiogroup",
            "aria-label": "{label}",
            "data-ax-roving": if horizontal { "horizontal" } else { "vertical" },
            "data-ax-wrap": "true",
            for opt in options.iter().cloned() {
                RadioItem {
                    key: "{opt.value}",
                    checked: opt.value == value,
                    disabled: disabled || opt.disabled,
                    option: opt.clone(),
                    first: options.first().map(|o| o.value == opt.value).unwrap_or(false) && !options.iter().any(|o| o.value == value),
                    onchange,
                }
            }
        }
    }
}

#[component]
fn RadioItem(option: RadioOption, checked: bool, disabled: bool, first: bool, onchange: EventHandler<String>) -> Element {
    let v = option.value.clone();
    let v2 = option.value.clone();
    rsx! {
        label {
            class: "ax-radio",
            class: if checked { "ax-checked" },
            class: if disabled { "ax-disabled" },
            role: "radio",
            "aria-checked": if checked { "true" } else { "false" },
            "aria-disabled": if disabled { "true" },
            "data-ax-item": "true",
            tabindex: if checked || first { "0" } else { "-1" },
            title: option.tooltip.clone(),
            onclick: move |e| { e.prevent_default(); if !disabled { onchange.call(v.clone()) } },
            onkeydown: move |e: KeyboardEvent| if !disabled && is_space(&e) { e.prevent_default(); e.stop_propagation(); onchange.call(v2.clone()) },
            // Selection follows focus for arrow keys (APG radio group).
            onfocus: {
                let v = option.value.clone();
                move |_| if !disabled && !checked { onchange.call(v.clone()) }
            },
            span { class: "ax-radio-circle", span { class: "ax-radio-dot" } }
            span { class: "ax-choice-text",
                span { "{option.label}" }
                if !option.description.is_empty() { span { class: "ax-choice-desc", "{option.description}" } }
            }
        }
    }
}

#[component]
pub fn TextField(
    #[props(into)] value: String,
    oninput: EventHandler<String>,
    #[props(into, default)] label: String,
    #[props(into, default)] placeholder: String,
    #[props(default)] multiline: bool,
    #[props(default)] autofocus: bool,
    #[props(into, default = "text".to_string())] input_type: String,
    #[props(into, default)] class: String,
    onkeydown: Option<EventHandler<KeyboardEvent>>,
    onblur: Option<EventHandler<FocusEvent>>,
    #[props(default)] disabled: bool,
    /// Helper or validation text under the field.
    #[props(into, default)]
    hint: String,
    #[props(into)] id: Option<String>,
    /// Number inputs: bounds.
    #[props(into)]
    min: Option<String>,
    #[props(into)] max: Option<String>,
    /// The page empties the input as Enter goes down when the handler took
    /// the key (prevent_default): quick entry, so the next text typed at
    /// once doesn't land behind the last (kit/typed_text.js).
    #[props(default)]
    enter_clears: bool,
    /// Marked invalid (red underline, `aria-invalid`).
    #[props(default)]
    invalid: bool,
    /// Id of text elsewhere that describes the field (`aria-describedby`).
    #[props(into)]
    describedby: Option<String>,
) -> Element {
    let aria = if label.is_empty() { placeholder.clone() } else { label.clone() };
    let input_class = if invalid { "ax-input ax-invalid" } else { "ax-input" };
    let area_class = if invalid { "ax-textarea ax-invalid" } else { "ax-textarea" };
    rsx! {
        div { class: "ax-field {class}",
            if !label.is_empty() { label { class: "ax-field-label", r#for: id.clone(), "{label}" } }
            if multiline {
                textarea {
                    id: id.clone(),
                    class: area_class,
                    "aria-invalid": if invalid { "true" },
                    "aria-describedby": describedby.clone(),
                    placeholder: "{placeholder}",
                    value: "{value}",
                    autofocus,
                    disabled,
                    "data-ax-autofocus": if autofocus { "true" },
                    "aria-label": "{aria}",
                    oninput: move |e| oninput.call(e.value()),
                    onkeydown: move |e| { keys::stop_typing_keys(&e); if let Some(h) = &onkeydown { h.call(e) } },
                    onblur: move |e| if let Some(h) = &onblur { h.call(e) },
                }
            } else {
                input {
                    id: id.clone(),
                    class: input_class,
                    "aria-invalid": if invalid { "true" },
                    "aria-describedby": describedby.clone(),
                    r#type: "{input_type}",
                    placeholder: "{placeholder}",
                    value: "{value}",
                    autofocus,
                    disabled,
                    min,
                    max,
                    "data-ax-autofocus": if autofocus { "true" },
                    "data-ax-enter-clears": if enter_clears { "true" },
                    "aria-label": "{aria}",
                    oninput: move |e| oninput.call(e.value()),
                    onkeydown: move |e| { keys::stop_typing_keys(&e); if let Some(h) = &onkeydown { h.call(e) } },
                    onblur: move |e| if let Some(h) = &onblur { h.call(e) },
                }
            }
            if !hint.is_empty() { span { class: "ax-field-hint", "{hint}" } }
        }
    }
}

/// Search input with a leading magnifier and a clear button. Escape clears
/// the text first, then bubbles (so a dialog can close on the next press).
#[component]
pub fn SearchField(
    #[props(into)] value: String,
    oninput: EventHandler<String>,
    #[props(into, default = "Search".to_string())] placeholder: String,
    #[props(into, default)] class: String,
    onkeydown: Option<EventHandler<KeyboardEvent>>,
    #[props(default)] autofocus: bool,
    #[props(into)] id: Option<String>,
) -> Element {
    let empty = value.is_empty();
    rsx! {
        div { class: "ax-searchfield {class}", role: "search",
            Icon { name: "search", size: 16 }
            input {
                id,
                r#type: "text",
                placeholder: "{placeholder}",
                "aria-label": "{placeholder}",
                value: "{value}",
                autofocus,
                "data-ax-autofocus": if autofocus { "true" },
                oninput: move |e| oninput.call(e.value()),
                onkeydown: move |e: KeyboardEvent| {
                    keys::stop_typing_keys(&e);
                    if e.key() == Key::Escape && !empty {
                        e.stop_propagation();
                        oninput.call(String::new());
                        return;
                    }
                    if let Some(h) = &onkeydown { h.call(e) }
                },
            }
            if !empty {
                button {
                    r#type: "button",
                    class: "ax-searchfield-clear",
                    title: "Clear",
                    "aria-label": "Clear",
                    onclick: move |_| oninput.call(String::new()),
                    Icon { name: "dismiss", size: 12 }
                }
            }
        }
    }
}

/// Native select styled as a Fluent dropdown. `options` are (value, label).
#[component]
pub fn Select(
    #[props(into)] value: String,
    options: Vec<(String, String)>,
    onchange: EventHandler<String>,
    #[props(into, default)] label: String,
    #[props(into, default)] class: String,
    #[props(default)] disabled: bool,
    #[props(into)] title: Option<String>,
    /// Accessible name when there is no visible label.
    #[props(into)]
    aria_label: Option<String>,
) -> Element {
    let aria = aria_label.unwrap_or_else(|| label.clone());
    rsx! {
        div { class: "ax-field {class}", title,
            if !label.is_empty() { label { class: "ax-field-label", "{label}" } }
            div { class: "ax-select-wrap",
                select {
                    class: "ax-select",
                    "aria-label": "{aria}",
                    disabled,
                    onchange: move |e| onchange.call(e.value()),
                    onkeydown: move |e| keys::stop_typing_keys(&e),
                    for (v, l) in options.iter() {
                        option { key: "{v}", value: "{v}", selected: *v == value, "{l}" }
                    }
                }
                Icon { name: "chevron_down", size: 12, class: "ax-select-chevron" }
            }
        }
    }
}

/// Range slider with labelled stops (e.g. undo-send delay).
#[component]
pub fn Slider(
    value: u32,
    min: u32,
    max: u32,
    #[props(default = 1)] step: u32,
    onchange: EventHandler<u32>,
    #[props(into)] label: String,
    /// Text shown next to the slider for the current value.
    #[props(into, default)]
    value_text: String,
) -> Element {
    let pct = if max > min { (value.saturating_sub(min)) as f64 * 100.0 / (max - min) as f64 } else { 0.0 };
    rsx! {
        div { class: "ax-slider",
            input {
                r#type: "range",
                min: "{min}",
                max: "{max}",
                step: "{step}",
                value: "{value}",
                "aria-label": "{label}",
                "aria-valuetext": "{value_text}",
                style: "--ax-slider-pct:{pct}%",
                oninput: move |e| if let Ok(v) = e.value().parse::<u32>() { onchange.call(v) },
                onkeydown: move |e| keys::stop_typing_keys(&e),
            }
            if !value_text.is_empty() { span { class: "ax-slider-value", "{value_text}" } }
        }
    }
}

/// Text tabs with the Fluent underline indicator. `items` are (key, label).
/// Arrow keys move between tabs and select them (automatic activation).
#[component]
pub fn Pivot(
    items: Vec<(String, String)>,
    #[props(into)] selected: String,
    onselect: EventHandler<String>,
    #[props(into, default)] class: String,
    /// Accessible name of the tab list.
    #[props(into)]
    label: Option<String>,
) -> Element {
    rsx! {
        div {
            class: "ax-pivot {class}",
            role: "tablist",
            "aria-label": label,
            "data-ax-roving": "horizontal",
            "data-ax-wrap": "true",
            for (k, l) in items.iter().cloned() {
                button {
                    key: "{k}",
                    r#type: "button",
                    role: "tab",
                    class: "ax-pivot-tab",
                    class: if k == selected { "ax-selected" },
                    "aria-selected": if k == selected { "true" } else { "false" },
                    "data-ax-item": "true",
                    tabindex: if k == selected { "0" } else { "-1" },
                    onclick: {
                        let k = k.clone();
                        move |_| onselect.call(k.clone())
                    },
                    onfocus: {
                        let k = k.clone();
                        let sel = selected.clone();
                        move |_| if k != sel { onselect.call(k.clone()) }
                    },
                    // Reserve the semibold width so tabs don't shift when selected.
                    span { class: "ax-pivot-label", "data-text": "{l}", "{l}" }
                }
            }
        }
    }
}
