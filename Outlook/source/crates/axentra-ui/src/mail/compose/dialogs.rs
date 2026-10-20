//! Compose dialogs: discard, send checks, link, schedule send, address
//! book and table size.

use std::collections::HashSet;
use std::rc::Rc;

use axentra_core::EmailAddress;
use chrono::{Datelike, Duration, Local, NaiveDate};
use dioxus::prelude::*;
use serde_json::json;

use super::actions;
use super::attachments::PreviewDialog;
use super::commands;
use super::draw::DrawDialog;
use super::recipients;
use super::schedule::{self, Preset};
use super::state::{ComposeDialog, Field, Form, SendOpts};
use crate::kit::{Avatar, Button, Checkbox, Dialog, Icon, IconButton, Menu, MenuItem, TextField, Variant, keys};
use crate::state::{store, use_app};

/// The dialog the form is showing, if any.
#[component]
pub fn ComposeDialogs() -> Element {
    let form = use_context::<Form>();
    match (form.dialog)() {
        None => rsx! {},
        Some(ComposeDialog::Discard) => rsx! { DiscardDialog {} },
        Some(ComposeDialog::NoSubject) => rsx! { NoSubjectDialog {} },
        Some(ComposeDialog::AttachReminder) => rsx! { AttachReminderDialog {} },
        Some(ComposeDialog::Link { text, url, editing }) => rsx! { LinkDialog { text, url, editing } },
        Some(ComposeDialog::Schedule) => rsx! { ScheduleDialog {} },
        Some(ComposeDialog::AddressBook(field)) => rsx! { AddressBookDialog { field } },
        Some(ComposeDialog::TableSize) => rsx! { TableSizeDialog {} },
        Some(ComposeDialog::Draw { tool, color }) => rsx! { DrawDialog { tool, color } },
        Some(ComposeDialog::Preview(index)) => rsx! { PreviewDialog { index } },
    }
}

fn close(mut form: Form) {
    form.dialog.set(None);
}

/// Move focus into the open dialog once it is on screen: the kit dialog
/// focuses itself, and `autofocus` does nothing for elements added later.
fn focus_in_dialog(selector: &str) {
    let js = format!(
        "setTimeout(() => {{ const e = document.querySelector({}); if (e) {{ e.focus(); if (e.select) e.select(); }} }}, 30);",
        serde_json::Value::String(selector.to_string())
    );
    document::eval(&js);
}

/// Confirmation dialogs start on their main button, so Enter confirms
/// (keyboard spec 15).
fn focus_main_button() {
    focus_in_dialog(".ax-dialog .ax-dialog-actions .ax-primary, .ax-dialog .ax-dialog-actions .ax-danger");
}

/// Cancel a send attempt that stopped at a dialog.
fn cancel_send(mut form: Form) {
    form.send_opts.set(SendOpts::default());
    form.dialog.set(None);
}

#[component]
fn DiscardDialog() -> Element {
    let app = use_app();
    let form = use_context::<Form>();
    use_hook(focus_main_button);
    rsx! {
        Dialog {
            title: "Discard this draft?",
            width: 400,
            on_close: move |_| close(form),
            actions: rsx! {
                Button { label: "Discard", variant: Variant::Danger, onclick: move |_| { close(form); actions::confirm_discard(app, form); } }
                Button { label: "Cancel", onclick: move |_| close(form) }
            },
            p { class: "ax-compose-dialog-text", "The draft will be moved to Deleted Items." }
        }
    }
}

#[component]
fn NoSubjectDialog() -> Element {
    let app = use_app();
    let mut form = use_context::<Form>();
    use_hook(focus_main_button);
    rsx! {
        Dialog {
            title: "Send without a subject?",
            width: 400,
            on_close: move |_| cancel_send(form),
            actions: rsx! {
                Button {
                    label: "Send",
                    variant: Variant::Primary,
                    onclick: move |_| {
                        form.send_opts.with_mut(|o| o.skip_subject = true);
                        close(form);
                        form.spawn(actions::send(app, form));
                    },
                }
                Button { label: "Don't send", onclick: move |_| { cancel_send(form); form.focus("subject"); } }
            },
            p { class: "ax-compose-dialog-text", "This message doesn't have a subject." }
        }
    }
}

#[component]
fn AttachReminderDialog() -> Element {
    let app = use_app();
    let mut form = use_context::<Form>();
    use_hook(focus_main_button);
    rsx! {
        Dialog {
            title: "Did you mean to attach a file?",
            width: 440,
            on_close: move |_| cancel_send(form),
            actions: rsx! {
                Button {
                    label: "Send anyway",
                    variant: Variant::Primary,
                    onclick: move |_| {
                        form.send_opts.with_mut(|o| o.skip_attachment = true);
                        close(form);
                        form.spawn(actions::send(app, form));
                    },
                }
                Button { label: "Attach a file", onclick: move |_| { cancel_send(form); commands::click(form, "file"); } }
                Button { label: "Cancel", onclick: move |_| cancel_send(form) }
            },
            p { class: "ax-compose-dialog-text", "You mentioned an attachment, but nothing is attached." }
        }
    }
}

/// Add a scheme to a typed address: "example.com" -> "https://example.com",
/// "a@b.com" -> "mailto:a@b.com".
pub fn normalize_url(url: &str) -> String {
    let u = url.trim();
    let lower = u.to_ascii_lowercase();
    if u.is_empty() || lower.contains("://") || lower.starts_with("mailto:") || lower.starts_with("tel:") {
        u.to_string()
    } else if super::address::is_valid(u) {
        format!("mailto:{u}")
    } else {
        format!("https://{u}")
    }
}

#[component]
fn LinkDialog(text: String, url: String, editing: bool) -> Element {
    let form = use_context::<Form>();
    use_hook(|| focus_in_dialog(".ax-compose-link-url input"));
    let mut display = use_signal(|| text.clone());
    let mut address = use_signal(|| url.clone());
    let insert = move || {
        let url = normalize_url(&address.peek());
        if url.is_empty() {
            return;
        }
        let text = display.peek().trim().to_string();
        close(form);
        form.exec("link", json!({ "url": url, "text": text }));
    };
    let can_insert = !address().trim().is_empty();
    rsx! {
        Dialog {
            title: if editing { "Edit link" } else { "Insert link" },
            width: 440,
            on_close: move |_| { close(form); form.exec("focus", serde_json::Value::Null); },
            actions: rsx! {
                if editing {
                    Button {
                        label: "Remove link",
                        variant: Variant::Subtle,
                        icon: "link_dismiss",
                        class: "ax-compose-dialog-left",
                        onclick: move |_| { close(form); form.exec("unlink", serde_json::Value::Null); },
                    }
                }
                Button { label: if editing { "Save" } else { "Insert" }, variant: Variant::Primary, disabled: !can_insert, onclick: move |_| insert() }
                Button { label: "Cancel", onclick: move |_| { close(form); form.exec("focus", serde_json::Value::Null); } }
            },
            div { class: "ax-compose-dialog-fields",
                TextField {
                    label: "Display as",
                    value: display(),
                    oninput: move |v| display.set(v),
                    onkeydown: move |e: KeyboardEvent| if keys::chord(&e) == "Enter" { insert() },
                }
                TextField {
                    label: "Web address",
                    placeholder: "https://",
                    class: "ax-compose-link-url",
                    value: address(),
                    oninput: move |v| address.set(v),
                    onkeydown: move |e: KeyboardEvent| if keys::chord(&e) == "Enter" { insert() },
                }
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Choice {
    Preset(usize),
    Custom,
}

#[component]
fn ScheduleDialog() -> Element {
    let app = use_app();
    let mut form = use_context::<Form>();
    let h24 = app.settings.read().use_24h_time;
    let now = Local::now();
    let presets: Vec<Preset> = use_hook(move || schedule::presets(Local::now()));
    let (def_date, def_time) = use_hook(move || schedule::default_custom(Local::now()));
    let mut choice = use_signal(|| Choice::Preset(0));
    let mut date = use_signal(|| def_date);
    let mut time_text = use_signal(|| schedule::time_label(def_time, h24));
    let mut error = use_signal(|| None::<String>);

    let resolve = {
        let presets = presets.clone();
        move || -> Result<chrono::DateTime<Local>, String> {
            match choice() {
                Choice::Preset(i) => presets.get(i).map(|p| p.at).ok_or_else(|| "Choose a time.".to_string()),
                Choice::Custom => {
                    let t = schedule::parse_time(&time_text.peek()).ok_or_else(|| "Enter a valid time, for example 8:30 AM.".to_string())?;
                    let at = schedule::combine(date(), t);
                    if at <= Local::now() { Err("Choose a time in the future.".into()) } else { Ok(at) }
                }
            }
        }
    };
    let send = {
        let resolve = resolve.clone();
        move |_| match resolve() {
            Ok(at) => {
                form.send_opts.set(SendOpts { schedule: Some(at.with_timezone(&chrono::Utc)), ..SendOpts::default() });
                close(form);
                form.spawn(actions::send(app, form));
            }
            Err(msg) => error.set(Some(msg)),
        }
    };

    rsx! {
        Dialog {
            title: "Schedule send",
            width: 480,
            on_close: move |_| cancel_send(form),
            actions: rsx! {
                Button { label: "Send", variant: Variant::Primary, onclick: send }
                Button { label: "Cancel", onclick: move |_| cancel_send(form) }
            },
            div { class: "ax-compose-schedule", role: "radiogroup", "aria-label": "When to send",
                for (i, p) in presets.iter().enumerate() {
                    ScheduleOption {
                        key: "{p.id}",
                        label: p.label,
                        caption: schedule::preset_caption(p.at, h24),
                        selected: choice() == Choice::Preset(i),
                        onselect: move |_| { choice.set(Choice::Preset(i)); error.set(None); },
                    }
                }
                ScheduleOption {
                    label: "Custom time",
                    caption: String::new(),
                    selected: choice() == Choice::Custom,
                    onselect: move |_| { choice.set(Choice::Custom); error.set(None); },
                }
                if choice() == Choice::Custom {
                    div { class: "ax-compose-schedule-custom",
                        DatePicker { value: date(), min: now.date_naive(), onchange: move |d| { date.set(d); error.set(None); } }
                        TimeCombo { value: time_text(), h24, onchange: move |t| { time_text.set(t); error.set(None); } }
                    }
                }
                if let Some(e) = error() {
                    div { class: "ax-compose-error", role: "alert", Icon { name: "error_circle", size: 16 } span { "{e}" } }
                }
            }
        }
    }
}

#[component]
fn ScheduleOption(label: &'static str, caption: String, selected: bool, onselect: EventHandler<()>) -> Element {
    rsx! {
        div {
            class: "ax-compose-schedule-option",
            class: if selected { "ax-selected" },
            role: "radio",
            tabindex: "0",
            "aria-checked": if selected { "true" } else { "false" },
            onclick: move |_| onselect.call(()),
            onkeydown: move |e: KeyboardEvent| if matches!(e.key(), Key::Enter) || e.key() == Key::Character(" ".into()) { e.prevent_default(); onselect.call(()) },
            Icon { name: if selected { "radio_button_filled" } else { "radio_button" }, size: 20 }
            span { class: "ax-grow", "{label}" }
            if !caption.is_empty() { span { class: "ax-compose-schedule-caption", "{caption}" } }
        }
    }
}

/// Month grid for the custom date.
#[component]
fn DatePicker(value: NaiveDate, min: NaiveDate, onchange: EventHandler<NaiveDate>) -> Element {
    let mut month = use_signal(|| NaiveDate::from_ymd_opt(value.year(), value.month(), 1).unwrap_or(value));
    let first = month();
    let start = first - Duration::days(first.weekday().num_days_from_sunday() as i64);
    let next_month = first.checked_add_months(chrono::Months::new(1)).unwrap_or(first);
    let prev_month = first.checked_sub_months(chrono::Months::new(1)).unwrap_or(first);
    let min_month = NaiveDate::from_ymd_opt(min.year(), min.month(), 1).unwrap_or(min);
    let can_go_back = prev_month >= min_month;
    let today = Local::now().date_naive();
    rsx! {
        div { class: "ax-compose-datepicker", role: "group", "aria-label": "Date",
            div { class: "ax-compose-datepicker-head",
                span { class: "ax-semibold ax-grow", "{first.format(\"%B %Y\")}" }
                IconButton { icon: "chevron_up", title: "Previous month", small: true, disabled: !can_go_back, onclick: move |_| month.set(prev_month) }
                IconButton { icon: "chevron_down", title: "Next month", small: true, onclick: move |_| month.set(next_month) }
            }
            div { class: "ax-compose-datepicker-grid", role: "grid",
                for d in ["S", "M", "T", "W", "T", "F", "S"].iter().enumerate() {
                    span { key: "h{d.0}", class: "ax-compose-datepicker-dow", "aria-hidden": "true", "{d.1}" }
                }
                for i in 0..42i64 {
                    {
                        let day = start + Duration::days(i);
                        let outside = day.month() != first.month();
                        let disabled = day < min;
                        rsx! {
                            button {
                                key: "{day}",
                                r#type: "button",
                                class: "ax-compose-datepicker-day",
                                class: if outside { "ax-outside" },
                                class: if day == value { "ax-selected" },
                                class: if day == today { "ax-today" },
                                disabled,
                                "aria-label": "{day.format(\"%A, %B %-d, %Y\")}",
                                "aria-selected": if day == value { "true" } else { "false" },
                                onclick: move |_| onchange.call(day),
                                "{day.day()}"
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Time box with a 30-minute drop-down; free typing is allowed.
#[component]
fn TimeCombo(value: String, h24: bool, onchange: EventHandler<String>) -> Element {
    let mut menu = use_signal(|| None::<(f64, f64)>);
    let mut anchor = use_signal(|| None::<Rc<MountedData>>);
    let items: Vec<MenuItem> = schedule::time_options(h24).into_iter().map(|(_, l)| MenuItem::new(l.clone(), l)).collect();
    rsx! {
        div { class: "ax-compose-timecombo", onmounted: move |e| anchor.set(Some(e.data())),
            label { class: "ax-field-label", "Time" }
            div { class: "ax-row",
                input {
                    class: "ax-input",
                    r#type: "text",
                    "aria-label": "Time",
                    value: "{value}",
                    oninput: move |e| onchange.call(e.value()),
                    onkeydown: move |e: KeyboardEvent| keys::stop_typing_keys(&e),
                }
                IconButton {
                    icon: "chevron_down",
                    title: "Choose a time",
                    onclick: move |_| {
                        spawn(async move {
                            let rect = match anchor() {
                                Some(m) => m.get_client_rect().await.ok(),
                                None => None,
                            };
                            menu.set(Some(rect.map(|r| (r.min_x(), r.max_y() + 4.0)).unwrap_or((0.0, 0.0))));
                        });
                    },
                }
            }
        }
        if let Some((x, y)) = menu() {
            Menu { items, x, y, on_close: move |_| menu.set(None), on_select: move |t: String| onchange.call(t) }
        }
    }
}

#[component]
fn AddressBookDialog(field: Field) -> Element {
    let app = use_app();
    let form = use_context::<Form>();
    use_hook(|| focus_in_dialog(".ax-compose-book-search input"));
    let mut query = use_signal(String::new);
    let mut picked = use_signal(HashSet::<String>::new);
    let _ = app.people_rev();
    let q = query().trim().to_lowercase();
    let mut people: Vec<EmailAddress> = store()
        .contacts()
        .unwrap_or_default()
        .into_iter()
        .flat_map(|c| {
            let name = c.display_name.clone();
            c.emails.into_iter().map(move |e| EmailAddress::new(name.clone(), e.value))
        })
        .filter(|a| q.is_empty() || a.name.to_lowercase().contains(&q) || a.address.to_lowercase().contains(&q))
        .collect();
    people.sort_by_key(|a| a.name.to_lowercase());
    let chosen = picked();
    let save = move |_| {
        let chosen = picked.peek().clone();
        let list: Vec<EmailAddress> = store()
            .contacts()
            .unwrap_or_default()
            .into_iter()
            .flat_map(|c| {
                let name = c.display_name.clone();
                c.emails.into_iter().map(move |e| EmailAddress::new(name.clone(), e.value))
            })
            .filter(|a| chosen.contains(&a.address.to_lowercase()))
            .collect();
        let mut f = form;
        if field == Field::Cc {
            f.show_cc.set(true);
        }
        if field == Field::Bcc {
            f.show_bcc.set(true);
        }
        recipients::add(form, field, list);
        close(form);
        form.focus(field.key());
    };
    rsx! {
        Dialog {
            title: "Add recipients",
            width: 480,
            resize_key: "address-book", min_width: 380, min_height: 360,
            on_close: move |_| close(form),
            actions: rsx! {
                Button { label: "Save", variant: Variant::Primary, disabled: chosen.is_empty(), onclick: save }
                Button { label: "Cancel", onclick: move |_| close(form) }
            },
            div { class: "ax-compose-book ax-surface-fill",
                TextField { placeholder: "Search people", class: "ax-compose-book-search", value: query(), oninput: move |v| query.set(v) }
                div { class: "ax-compose-book-list ax-surface-fill", role: "listbox", "aria-multiselectable": "true", "aria-label": "Contacts",
                    if people.is_empty() {
                        div { class: "ax-compose-book-empty", "We didn't find anyone. Check the spelling or search for someone else." }
                    }
                    for a in people {
                        {
                            let key = a.address.to_lowercase();
                            let on = chosen.contains(&key);
                            let k2 = key.clone();
                            rsx! {
                                div {
                                    key: "{key}",
                                    class: "ax-compose-book-row",
                                    role: "option",
                                    "aria-selected": if on { "true" } else { "false" },
                                    onclick: move |_| { let k = k2.clone(); picked.with_mut(|p| if !p.remove(&k) { p.insert(k); }); },
                                    Checkbox { checked: on, onchange: move |_| { let k = key.clone(); picked.with_mut(|p| if !p.remove(&k) { p.insert(k); }); } }
                                    Avatar { name: a.name.clone(), email: a.address.clone(), size: 32 }
                                    div { class: "ax-compose-book-text",
                                        span { class: "ax-ellipsis", "{a.name}" }
                                        span { class: "ax-ellipsis ax-muted", "{a.address}" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn TableSizeDialog() -> Element {
    let form = use_context::<Form>();
    use_hook(|| focus_in_dialog(".ax-dialog .ax-compose-dialog-fields input"));
    let mut cols = use_signal(|| "3".to_string());
    let mut rows = use_signal(|| "3".to_string());
    let parsed = (cols().trim().parse::<u32>().ok().filter(|c| (1..=20).contains(c)), rows().trim().parse::<u32>().ok().filter(|r| (1..=50).contains(r)));
    let valid = matches!(parsed, (Some(_), Some(_)));
    rsx! {
        Dialog {
            title: "Insert table",
            width: 360,
            on_close: move |_| close(form),
            actions: rsx! {
                Button {
                    label: "Insert",
                    variant: Variant::Primary,
                    disabled: !valid,
                    onclick: move |_| {
                        if let (Some(c), Some(r)) = parsed {
                            close(form);
                            form.exec("table", json!({ "rows": r, "cols": c }));
                        }
                    },
                }
                Button { label: "Cancel", onclick: move |_| close(form) }
            },
            div { class: "ax-compose-dialog-fields",
                TextField { label: "Number of columns", input_type: "number", value: cols(), oninput: move |v| cols.set(v) }
                TextField { label: "Number of rows", input_type: "number", value: rows(), oninput: move |v| rows.set(v) }
                if !valid { div { class: "ax-compose-dialog-hint", "Use 1 to 20 columns and 1 to 50 rows." } }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_get_a_scheme() {
        assert_eq!(normalize_url("example.com"), "https://example.com");
        assert_eq!(normalize_url("http://x.example"), "http://x.example");
        assert_eq!(normalize_url("a@b.example"), "mailto:a@b.example");
        assert_eq!(normalize_url("mailto:a@b.example"), "mailto:a@b.example");
        assert_eq!(normalize_url("  "), "");
    }

    #[test]
    fn time_menu_labels_parse_back() {
        // The custom time box accepts everything its menu offers.
        for h24 in [false, true] {
            for (t, label) in schedule::time_options(h24) {
                assert_eq!(schedule::parse_time(&label), Some(t), "{label}");
            }
        }
    }
}
