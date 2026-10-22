//! New contact dialog and the inline Edit form (docs/spec/people-todo.md
//! 2.8): same fields, same validation, discard confirmation, Ctrl+S / Esc.

use axentra_core::*;
use axentra_core::Store;
use chrono::NaiveDate;
use dioxus::prelude::*;

use super::avatar::PeopleAvatar;
use super::ctx::PeopleCtx;
use super::details::open_item;
use super::dialogs::ConfirmDialog;
use super::io;
use super::photo::PhotoCropDialog;
use super::view::Item;
use crate::kit::{Dialog, Icon, IconButton, MenuButton, MenuItem, Variant, keys};
use crate::state::{AppState, store, use_app};

pub const MONTHS: [&str; 12] =
    ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const EMAIL_TYPES: [&str; 3] = ["Work", "Personal", "Other"];
const PHONE_TYPES: [&str; 3] = ["Mobile", "Work", "Home"];
const ADDRESS_TYPES: [&str; 3] = ["Home", "Work", "Other"];
/// The "Add photo" / "Change photo" button, which gets focus back after cropping.
const PHOTO_BUTTON: &str = ".ax-people-form-photo button";
/// Exchange contacts have three email slots (2.8).
const MAX_EMAILS: usize = 3;

/// Editable copy of a contact. `base` keeps fields the form doesn't show.
#[derive(Clone, Debug, PartialEq)]
pub struct FormState {
    pub base: Contact,
    pub given: String,
    pub surname: String,
    pub emails: Vec<(String, String)>,
    pub phones: Vec<(String, String)>,
    pub addresses: Vec<PostalAddress>,
    pub bday_month: u32,
    pub bday_day: u32,
    pub bday_year: String,
    pub company: String,
    pub job_title: String,
    pub department: String,
    pub notes: String,
    pub middle: String,
    pub nickname: String,
    pub title: String,
    pub suffix: String,
    pub phonetic_given: String,
    pub phonetic_surname: String,
    pub phonetic_company: String,
    pub office: String,
    pub manager: String,
    pub assistant: String,
    pub profession: String,
    pub website: String,
    pub im: String,
    pub spouse: String,
    pub children: String,
    pub file_as: String,
    pub photo: Option<String>,
}

/// Problems that block saving.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FormErrors {
    /// Index of each invalid email row.
    pub emails: Vec<usize>,
    pub birthday: bool,
}

impl FormErrors {
    pub fn is_empty(&self) -> bool {
        self.emails.is_empty() && !self.birthday
    }
}

/// Simplified RFC 5322 addr-spec: `local@domain.tld`, no spaces.
pub fn valid_email(s: &str) -> bool {
    let s = s.trim();
    let Some((local, domain)) = s.rsplit_once('@') else { return false };
    let local_ok = !local.is_empty() && !local.contains('@') && !local.starts_with('.') && !local.ends_with('.') && !local.contains("..");
    let domain_ok = domain.contains('.')
        && !domain.starts_with(['.', '-'])
        && !domain.ends_with(['.', '-'])
        && !domain.contains("..")
        && domain.chars().all(|c| c.is_alphanumeric() || c == '.' || c == '-');
    let chars_ok = !s.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '<' | '>' | '(' | ')' | ',' | ';' | ':' | '\\' | '"' | '[' | ']'));
    local_ok && domain_ok && chars_ok
}

impl FormState {
    pub fn from_contact(c: &Contact) -> Self {
        let mut emails: Vec<(String, String)> = c.emails.iter().map(|e| (normal_email_label(&e.label), e.value.clone())).collect();
        if emails.is_empty() {
            emails.push(("Work".into(), String::new()));
        }
        let mut phones: Vec<(String, String)> = c.phones.iter().map(|p| (p.label.clone(), p.value.clone())).collect();
        if phones.is_empty() {
            phones.push(("Mobile".into(), String::new()));
        }
        let (bday_month, bday_day, bday_year) = match c.birthday {
            Some(d) => {
                use chrono::Datelike;
                let year = if d.year() == Store::birthday_no_year() { String::new() } else { d.year().to_string() };
                (d.month(), d.day(), year)
            }
            None => (0, 0, String::new()),
        };
        Self {
            base: c.clone(),
            given: c.given_name.clone(),
            surname: c.surname.clone(),
            emails,
            phones,
            addresses: c.addresses.clone(),
            bday_month,
            bday_day,
            bday_year,
            company: c.company.clone(),
            job_title: c.job_title.clone(),
            department: c.department.clone(),
            notes: c.notes.clone(),
            middle: c.middle_name.clone(),
            nickname: c.nickname.clone(),
            title: c.more.title.clone(),
            suffix: c.more.suffix.clone(),
            phonetic_given: c.more.phonetic_given_name.clone(),
            phonetic_surname: c.more.phonetic_surname.clone(),
            phonetic_company: c.more.phonetic_company.clone(),
            office: c.office_location.clone(),
            manager: c.more.manager.clone(),
            assistant: c.more.assistant.clone(),
            profession: c.more.profession.clone(),
            website: c.websites.first().cloned().unwrap_or_default(),
            im: c.im_addresses.first().cloned().unwrap_or_default(),
            spouse: c.more.spouse.clone(),
            children: c.more.children.clone(),
            file_as: c.more.file_as.clone(),
            photo: c.photo.clone(),
        }
    }

    /// Save stays disabled until a first name, last name, company or email
    /// is filled in (2.8).
    pub fn can_save(&self) -> bool {
        [&self.given, &self.surname, &self.company].iter().any(|s| !s.trim().is_empty())
            || self.emails.iter().any(|(_, v)| !v.trim().is_empty())
    }

    fn birthday(&self) -> std::result::Result<Option<NaiveDate>, ()> {
        match (self.bday_month, self.bday_day) {
            (0, 0) if self.bday_year.trim().is_empty() => Ok(None),
            (0, _) | (_, 0) => Err(()),
            (m, d) => {
                let y = match self.bday_year.trim() {
                    "" => Store::birthday_no_year(),
                    y => y.parse::<i32>().ok().filter(|y| (1..=9999).contains(y)).ok_or(())?,
                };
                NaiveDate::from_ymd_opt(y, m, d).map(Some).ok_or(())
            }
        }
    }

    pub fn errors(&self) -> FormErrors {
        FormErrors {
            emails: self.emails.iter().enumerate().filter(|(_, (_, v))| !v.trim().is_empty() && !valid_email(v)).map(|(i, _)| i).collect(),
            birthday: self.birthday().is_err(),
        }
    }

    /// The contact as it would be saved (`base` plus the edited fields).
    pub fn to_contact(&self) -> Contact {
        let mut c = self.base.clone();
        let names_changed = c.given_name.trim() != self.given.trim() || c.surname.trim() != self.surname.trim();
        c.given_name = self.given.trim().to_string();
        c.surname = self.surname.trim().to_string();
        let joined = [c.given_name.as_str(), c.surname.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" ");
        if names_changed || c.display_name.trim().is_empty() || c.display_name == c.company || c.primary_email() == Some(c.display_name.as_str()) {
            // Recompute; an empty value lets the store fall back to email / company.
            c.display_name = joined;
        }
        c.emails = self
            .emails
            .iter()
            .filter(|(_, v)| !v.trim().is_empty())
            .map(|(l, v)| LabeledValue { label: l.clone(), value: v.trim().to_string() })
            .collect();
        c.phones = self
            .phones
            .iter()
            .filter(|(_, v)| !v.trim().is_empty())
            .map(|(l, v)| LabeledValue { label: l.clone(), value: v.trim().to_string() })
            .collect();
        c.addresses = self
            .addresses
            .iter()
            .filter(|a| [&a.street, &a.city, &a.state, &a.postal_code, &a.country].iter().any(|s| !s.trim().is_empty()))
            .cloned()
            .collect();
        c.birthday = self.birthday().ok().flatten();
        c.company = self.company.trim().to_string();
        c.job_title = self.job_title.trim().to_string();
        c.department = self.department.trim().to_string();
        c.notes = self.notes.clone();
        c.middle_name = self.middle.trim().to_string();
        c.nickname = self.nickname.trim().to_string();
        c.more.title = self.title.trim().to_string();
        c.more.suffix = self.suffix.trim().to_string();
        c.more.phonetic_given_name = self.phonetic_given.trim().to_string();
        c.more.phonetic_surname = self.phonetic_surname.trim().to_string();
        c.more.phonetic_company = self.phonetic_company.trim().to_string();
        c.office_location = self.office.trim().to_string();
        c.more.manager = self.manager.trim().to_string();
        c.more.assistant = self.assistant.trim().to_string();
        c.more.profession = self.profession.trim().to_string();
        set_first(&mut c.websites, &self.website);
        set_first(&mut c.im_addresses, &self.im);
        c.more.spouse = self.spouse.trim().to_string();
        c.more.children = self.children.trim().to_string();
        c.more.file_as = self.file_as.trim().to_string();
        c.photo = self.photo.clone();
        c
    }

    /// Another saved contact with the same primary email.
    pub fn duplicate(&self) -> Option<Contact> {
        let email = self.emails.iter().map(|(_, v)| v.trim()).find(|v| !v.is_empty())?;
        if !valid_email(email) {
            return None;
        }
        store().contact_by_email(email).ok().flatten().filter(|c| c.id != self.base.id)
    }
}

fn set_first(list: &mut Vec<String>, value: &str) {
    let v = value.trim();
    match (list.is_empty(), v.is_empty()) {
        (true, true) => {}
        (true, false) => list.push(v.to_string()),
        (false, true) => {
            list.remove(0);
        }
        (false, false) => list[0] = v.to_string(),
    }
}

fn normal_email_label(l: &str) -> String {
    match l {
        "Work" | "Personal" | "Other" => l.to_string(),
        "Home" => "Personal".into(),
        _ => "Other".into(),
    }
}

/// Insert or update; returns the saved contact's id.
fn save(app: AppState, form: &FormState) -> Option<ContactId> {
    let mut c = form.to_contact();
    let result = if c.id == 0 { store().insert_contact(&mut c) } else { store().update_contact(&c).map(|_| c.id) };
    let id = app.check(result)?;
    app.bump_people();
    Some(id)
}

// ---------------------------------------------------------------- frames

/// The form's fields remember which was focused last (see `keep_editing`).
const TRACK_FIELD_JS: &str = r#"
if (!window.__axPeopleFieldTrack) {
  window.__axPeopleFieldTrack = true;
  document.addEventListener("focusin", (e) => {
    const t = e.target;
    const form = t && t.closest && t.closest(".ax-people-form");
    if (form && t.matches("input, textarea, select")) form.__axLastField = t;
  }, true);
}
"#;

/// "Keep editing" in "Discard changes?": once the prompt has gone, focus
/// goes back to the field edited last (else the first), not to the form's
/// first button (its Close), where the next letter typed was lost.
const KEEP_EDITING_JS: &str = r#"
(function back(n) {
  if (document.querySelector(".ax-people-confirm") && n < 50) return setTimeout(() => back(n + 1), 20);
  const form = document.querySelector(".ax-people-form");
  if (!form) return;
  const last = form.__axLastField;
  const el = last && last.isConnected && form.contains(last) && !last.disabled ? last : form.querySelector("input:not([disabled]), textarea:not([disabled])");
  if (el) el.focus();
})(0);
"#;

fn keep_editing() {
    document::eval(KEEP_EDITING_JS);
}

/// The modal "New contact" form (600 px, 2.8).
#[component]
pub fn NewContactDialog(initial: Contact) -> Element {
    let app = use_app();
    let ctx = use_context::<PeopleCtx>();
    let original = use_hook(|| FormState::from_contact(&initial));
    let form = use_signal(|| original.clone());
    let mut confirm = use_signal(|| false);
    let mut tried = use_signal(|| false);
    let dirty = form() != original;
    let can_save = form.read().can_save();
    let mut submit = move || {
        tried.set(true);
        if !form.read().can_save() || !form.read().errors().is_empty() {
            return;
        }
        if let Some(id) = save(app, &form.read()) {
            ctx.close_dialog();
            open_item(ctx, Item::Contact(id));
            ctx.request_focus(true);
        }
    };
    let mut cancel = move || {
        if dirty {
            confirm.set(true);
        } else {
            ctx.close_dialog();
        }
    };
    rsx! {
        Dialog {
            title: "New contact",
            width: 600,
            resize_key: "contact-form", min_width: 440, min_height: 360,
            class: "ax-people-form-dialog",
            on_close: move |_| cancel(),
            actions: rsx! {
                button { r#type: "button", class: "ax-btn ax-primary", disabled: !can_save, title: if can_save { "Save (Ctrl+S)" } else { "Add a name, company or email address to save" }, onclick: move |_| submit(), "Save" }
                button { r#type: "button", class: "ax-btn", onclick: move |_| cancel(), "Cancel" }
            },
            div {
                class: "ax-people-form-keys",
                onkeydown: move |e: KeyboardEvent| match keys::chord(&e).as_str() {
                    "Ctrl+S" => {
                        e.prevent_default();
                        e.stop_propagation();
                        submit();
                    }
                    "Escape" => {
                        e.stop_propagation();
                        cancel();
                    }
                    _ => {}
                },
                ContactFormFields { form, show_errors: tried() }
            }
        }
        if confirm() {
            ConfirmDialog {
                title: "Discard changes?",
                body: "Your changes to this contact won't be saved.",
                confirm_label: "Discard",
                cancel_label: "Keep editing",
                on_confirm: move |_| {
                    confirm.set(false);
                    ctx.close_dialog();
                },
                on_cancel: move |_| {
                    confirm.set(false);
                    keep_editing();
                },
            }
        }
    }
}

/// The details pane in edit mode.
#[component]
pub fn InlineEditor(id: ContactId) -> Element {
    // Read once: the form keeps its own copy while editing.
    let contact = use_hook(|| store().contact(id).ok().flatten());
    match contact {
        Some(contact) => rsx! { InlineEditorInner { contact } },
        None => rsx! {},
    }
}

#[component]
fn InlineEditorInner(contact: Contact) -> Element {
    let app = use_app();
    let mut ctx = use_context::<PeopleCtx>();
    let original = use_hook(|| FormState::from_contact(&contact));
    let form = use_signal(|| original.clone());
    let mut confirm = use_signal(|| false);
    let mut tried = use_signal(|| false);
    let dirty = form() != original;
    let can_save = form.read().can_save();
    use_effect(move || {
        let d = form() != FormState::from_contact(&contact);
        if *ctx.edit_dirty.peek() != d {
            ctx.edit_dirty.set(d);
        }
    });
    let mut submit = move || {
        tried.set(true);
        if !form.read().can_save() || !form.read().errors().is_empty() {
            return;
        }
        if save(app, &form.read()).is_some() {
            ctx.edit_dirty.set(false);
            ctx.editing.set(None);
            ctx.request_focus(true);
        }
    };
    let mut cancel = move || {
        if dirty {
            confirm.set(true);
        } else {
            ctx.edit_dirty.set(false);
            ctx.editing.set(None);
            ctx.request_focus(true);
        }
    };
    rsx! {
        div {
            class: "ax-people-inline-edit",
            role: "form",
            "aria-label": "Edit contact",
            onkeydown: move |e: KeyboardEvent| match keys::chord(&e).as_str() {
                "Ctrl+S" => {
                    e.prevent_default();
                    e.stop_propagation();
                    submit();
                }
                "Escape" => {
                    e.stop_propagation();
                    cancel();
                }
                _ => {}
            },
            div { class: "ax-people-inline-title", "Edit contact" }
            div { class: "ax-people-inline-body", ContactFormFields { form, show_errors: tried() } }
            div { class: "ax-people-inline-footer",
                button { r#type: "button", class: "ax-btn ax-primary", disabled: !can_save, title: if can_save { "Save (Ctrl+S)" } else { "Add a name, company or email address to save" }, onclick: move |_| submit(), "Save" }
                button { r#type: "button", class: "ax-btn", onclick: move |_| cancel(), "Cancel" }
            }
        }
        if confirm() {
            ConfirmDialog {
                title: "Discard changes?",
                body: "Your changes to this contact won't be saved.",
                confirm_label: "Discard",
                cancel_label: "Keep editing",
                on_confirm: move |_| {
                    confirm.set(false);
                    ctx.edit_dirty.set(false);
                    ctx.editing.set(None);
                },
                on_cancel: move |_| {
                    confirm.set(false);
                    keep_editing();
                },
            }
        }
    }
}

// ---------------------------------------------------------------- fields

/// Keys for every editable element: no shortcut leaks, Ctrl+A stays in the box.
fn field_keys(e: &KeyboardEvent) {
    keys::stop_typing_keys(e);
    if keys::chord(e) == "Ctrl+A" {
        e.stop_propagation();
    }
}

#[component]
fn Input(
    #[props(into)] label: String,
    value: String,
    oninput: EventHandler<String>,
    #[props(default)] autofocus: bool,
    #[props(into, default = "text".to_string())] kind: String,
    #[props(into, default)] placeholder: String,
    #[props(default)] error: Option<String>,
    #[props(default)] hide_label: bool,
) -> Element {
    rsx! {
        label { class: "ax-people-input",
            if !hide_label { span { class: "ax-field-label", "{label}" } }
            input {
                class: "ax-input",
                class: if error.is_some() { "ax-invalid" },
                r#type: "{kind}",
                value: "{value}",
                placeholder: "{placeholder}",
                autofocus,
                "aria-label": "{label}",
                "aria-invalid": if error.is_some() { "true" } else { "false" },
                oninput: move |e| oninput.call(e.value()),
                onkeydown: move |e| field_keys(&e),
                onmounted: move |e| async move {
                    if autofocus {
                        let _ = e.set_focus(true).await;
                    }
                },
            }
            if let Some(err) = &error { span { class: "ax-people-error", role: "alert", "{err}" } }
        }
    }
}

#[component]
fn TypeSelect(#[props(into)] label: String, value: String, options: Vec<&'static str>, onchange: EventHandler<String>) -> Element {
    rsx! {
        div { class: "ax-select-wrap",
            select {
                class: "ax-select ax-people-type",
                "aria-label": "{label}",
                onchange: move |e| onchange.call(e.value()),
                onkeydown: move |e| field_keys(&e),
                for o in options {
                    option { key: "{o}", value: "{o}", selected: o == value, "{o}" }
                }
            }
            Icon { name: "chevron_down", size: 12, class: "ax-select-chevron" }
        }
    }
}

#[component]
fn AddLink(#[props(into)] label: String, #[props(default)] disabled: bool, #[props(into, default)] tooltip: String, onclick: EventHandler<()>) -> Element {
    rsx! {
        button {
            r#type: "button",
            class: "ax-people-link ax-people-add-link",
            disabled,
            title: "{tooltip}",
            onclick: move |_| onclick.call(()),
            Icon { name: "add", size: 16 }
            span { "{label}" }
        }
    }
}

/// All fields of 2.8, in order. Writes into `form`.
#[component]
pub fn ContactFormFields(form: Signal<FormState>, show_errors: bool) -> Element {
    let app = use_app();
    let ctx = use_context::<PeopleCtx>();
    let mut form = form;
    let mut more = use_signal(|| false);
    use_hook(|| {
        document::eval(TRACK_FIELD_JS);
    });
    // An uploaded image waiting for the crop step (2.6.4).
    let mut cropping = use_signal(|| None::<io::PickedImage>);
    let f = form.read().clone();
    let errors = f.errors();
    let duplicate = f.duplicate();
    let avatar_name = {
        let c = f.to_contact();
        if c.display_name.is_empty() { c.company.clone() } else { c.display_name.clone() }
    };
    let color_key = f.emails.iter().map(|(_, v)| v.trim().to_lowercase()).find(|v| !v.is_empty()).unwrap_or_else(|| avatar_name.clone());
    let has_photo = f.photo.is_some();
    let notes_rows = (f.notes.lines().count() + 1).clamp(4, 12);
    let used_address_types: Vec<String> = f.addresses.iter().map(|a| a.label.clone()).collect();
    let next_address_type = ADDRESS_TYPES.iter().find(|t| !used_address_types.iter().any(|u| u == *t)).copied();
    // Inline errors appear after a Save attempt, not while typing.
    let email_error = |i: usize| (show_errors && errors.emails.contains(&i)).then(|| "Enter a valid email address".to_string());

    macro_rules! text_field {
        ($label:expr, $field:ident) => {
            rsx! { Input { label: $label, value: f.$field.clone(), oninput: move |v| form.write().$field = v } }
        };
    }

    rsx! {
        div { class: "ax-people-form",
            div { class: "ax-people-form-photo",
                PeopleAvatar { name: avatar_name.clone(), color_key, size: 72, photo: f.photo.clone() }
                MenuButton {
                    label: if has_photo { "Change photo" } else { "Add photo" },
                    icon: "camera",
                    variant: Variant::Subtle,
                    items: vec![
                        MenuItem::new("upload", "Upload a photo").icon("image"),
                        MenuItem::new("remove", "Remove photo").icon("delete").disabled(!has_photo),
                    ],
                    on_select: move |id: String| {
                        if id == "remove" {
                            form.write().photo = None;
                        } else {
                            spawn(async move {
                                match io::pick_image().await {
                                    Ok(Some(image)) => cropping.set(Some(image)),
                                    Ok(None) => {}
                                    Err(()) => {
                                        app.error("That image couldn't be read. Try a JPEG, PNG or WebP file.");
                                    }
                                }
                            });
                        }
                    },
                }
            }
            if let Some(image) = cropping() {
                PhotoCropDialog {
                    image,
                    on_apply: move |photo: String| {
                        form.write().photo = Some(photo);
                        cropping.set(None);
                        io::focus_selector(PHOTO_BUTTON);
                    },
                    on_cancel: move |_| {
                        cropping.set(None);
                        io::focus_selector(PHOTO_BUTTON);
                    },
                }
            }
            if let Some(dup) = duplicate {
                div { class: "ax-message-bar ax-warning ax-people-dup", role: "status",
                    Icon { name: "warning" }
                    span { class: "ax-grow", "A contact with this email already exists." }
                    button {
                        r#type: "button",
                        class: "ax-people-link",
                        onclick: move |_| {
                            let mut ctx = ctx;
                            ctx.dialog.set(None);
                            ctx.editing.set(None);
                            ctx.edit_dirty.set(false);
                            open_item(ctx, Item::Contact(dup.id));
                            ctx.request_focus(true);
                        },
                        "Open contact"
                    }
                }
            }
            div { class: "ax-people-form-grid",
                Input { label: "First name", value: f.given.clone(), autofocus: true, oninput: move |v| form.write().given = v }
                Input { label: "Last name", value: f.surname.clone(), oninput: move |v| form.write().surname = v }
            }
            div { class: "ax-people-form-group",
                span { class: "ax-field-label", "Email" }
                for (i, (label, value)) in f.emails.iter().cloned().enumerate() {
                    div { key: "email-{i}", class: "ax-people-form-row",
                        TypeSelect { label: "Email type", value: label, options: EMAIL_TYPES.to_vec(), onchange: move |v| form.write().emails[i].0 = v }
                        Input {
                            label: "Email",
                            hide_label: true,
                            kind: "email",
                            value,
                            placeholder: "name@example.com",
                            error: email_error(i),
                            oninput: move |v| form.write().emails[i].1 = v,
                        }
                        IconButton { icon: "dismiss", title: "Remove email", small: true, onclick: move |_| { form.write().emails.remove(i); } }
                    }
                }
                AddLink {
                    label: "Add email",
                    disabled: f.emails.len() >= MAX_EMAILS,
                    tooltip: if f.emails.len() >= MAX_EMAILS { "A contact can have up to three email addresses" } else { "" },
                    onclick: move |_| form.write().emails.push(("Personal".into(), String::new())),
                }
            }
            div { class: "ax-people-form-group",
                span { class: "ax-field-label", "Phone" }
                for (i, (label, value)) in f.phones.iter().cloned().enumerate() {
                    div { key: "phone-{i}", class: "ax-people-form-row",
                        TypeSelect { label: "Phone type", value: label, options: PHONE_TYPES.to_vec(), onchange: move |v| form.write().phones[i].0 = v }
                        Input { label: "Phone", hide_label: true, kind: "tel", value, oninput: move |v| form.write().phones[i].1 = v }
                        IconButton { icon: "dismiss", title: "Remove phone number", small: true, onclick: move |_| { form.write().phones.remove(i); } }
                    }
                }
                AddLink { label: "Add phone number", onclick: move |_| form.write().phones.push(("Mobile".into(), String::new())) }
            }
            div { class: "ax-people-form-group",
                span { class: "ax-field-label", "Address" }
                for (i, a) in f.addresses.iter().cloned().enumerate() {
                    div { key: "addr-{i}", class: "ax-people-form-address",
                        div { class: "ax-people-form-row",
                            TypeSelect { label: "Address type", value: a.label.clone(), options: ADDRESS_TYPES.to_vec(), onchange: move |v| form.write().addresses[i].label = v }
                            span { class: "ax-grow" }
                            IconButton { icon: "dismiss", title: "Remove address", small: true, onclick: move |_| { form.write().addresses.remove(i); } }
                        }
                        Input { label: "Street address", value: a.street.clone(), oninput: move |v| form.write().addresses[i].street = v }
                        div { class: "ax-people-form-grid",
                            Input { label: "City", value: a.city.clone(), oninput: move |v| form.write().addresses[i].city = v }
                            Input { label: "State/Province", value: a.state.clone(), oninput: move |v| form.write().addresses[i].state = v }
                            Input { label: "ZIP/Postal code", value: a.postal_code.clone(), oninput: move |v| form.write().addresses[i].postal_code = v }
                            Input { label: "Country/Region", value: a.country.clone(), oninput: move |v| form.write().addresses[i].country = v }
                        }
                    }
                }
                AddLink {
                    label: "Add address",
                    disabled: next_address_type.is_none(),
                    tooltip: if next_address_type.is_none() { "A contact has one home, work and other address" } else { "" },
                    onclick: move |_| {
                        if let Some(t) = next_address_type {
                            form.write().addresses.push(PostalAddress { label: t.into(), ..Default::default() });
                        }
                    },
                }
            }
            div { class: "ax-people-form-group",
                span { class: "ax-field-label", "Birthday" }
                div { class: "ax-people-form-row ax-people-bday",
                    div { class: "ax-select-wrap",
                        select {
                            class: "ax-select",
                            "aria-label": "Month",
                            onchange: move |e| form.write().bday_month = e.value().parse().unwrap_or(0),
                            onkeydown: move |e| field_keys(&e),
                            option { value: "0", selected: f.bday_month == 0, "Month" }
                            for (m, name) in MONTHS.iter().enumerate() {
                                option { key: "{m}", value: "{m + 1}", selected: f.bday_month == m as u32 + 1, "{name}" }
                            }
                        }
                        Icon { name: "chevron_down", size: 12, class: "ax-select-chevron" }
                    }
                    div { class: "ax-select-wrap",
                        select {
                            class: "ax-select",
                            "aria-label": "Day",
                            onchange: move |e| form.write().bday_day = e.value().parse().unwrap_or(0),
                            onkeydown: move |e| field_keys(&e),
                            option { value: "0", selected: f.bday_day == 0, "Day" }
                            for d in 1..=31u32 {
                                option { key: "{d}", value: "{d}", selected: f.bday_day == d, "{d}" }
                            }
                        }
                        Icon { name: "chevron_down", size: 12, class: "ax-select-chevron" }
                    }
                    Input { label: "Year (optional)", hide_label: true, value: f.bday_year.clone(), placeholder: "Year (optional)", oninput: move |v| form.write().bday_year = v }
                }
                if errors.birthday && (show_errors || (f.bday_month != 0 && f.bday_day != 0)) {
                    span { class: "ax-people-error", role: "alert", "Enter a valid date" }
                }
            }
            div { class: "ax-people-form-grid",
                {text_field!("Company", company)}
                {text_field!("Job title", job_title)}
                {text_field!("Department", department)}
            }
            label { class: "ax-people-input",
                span { class: "ax-field-label", "Notes" }
                textarea {
                    class: "ax-textarea",
                    rows: "{notes_rows}",
                    value: "{f.notes}",
                    "aria-label": "Notes",
                    oninput: move |e| form.write().notes = e.value(),
                    onkeydown: move |e| field_keys(&e),
                }
            }
            button {
                r#type: "button",
                class: "ax-people-link ax-people-more",
                "aria-expanded": if more() { "true" } else { "false" },
                onclick: move |_| more.set(!more()),
                Icon { name: if more() { "chevron_up" } else { "chevron_down" }, size: 16 }
                span { if more() { "Fewer fields" } else { "More fields..." } }
            }
            if more() {
                div { class: "ax-people-form-grid",
                    {text_field!("Middle name", middle)}
                    {text_field!("Nickname", nickname)}
                    {text_field!("Title", title)}
                    {text_field!("Suffix", suffix)}
                    {text_field!("Phonetic first name", phonetic_given)}
                    {text_field!("Phonetic last name", phonetic_surname)}
                    {text_field!("Phonetic company", phonetic_company)}
                    {text_field!("Office location", office)}
                    {text_field!("Manager", manager)}
                    {text_field!("Assistant", assistant)}
                    {text_field!("Profession", profession)}
                    {text_field!("Website", website)}
                    {text_field!("IM address", im)}
                    {text_field!("Spouse/partner", spouse)}
                    {text_field!("Children", children)}
                    {text_field!("File as", file_as)}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_validation() {
        for ok in ["ada@example.com", "a.b+tag@sub.example.co.uk", "José@exämple.com"] {
            assert!(valid_email(ok), "{ok}");
        }
        for bad in ["", "ada", "ada@", "@example.com", "ada@example", "a b@example.com", "ada@@example.com", "ada@.example.com", "a..b@example.com", "<ada@example.com>"] {
            assert!(!valid_email(bad), "{bad}");
        }
    }

    #[test]
    fn form_round_trip_and_rules() {
        let c = Contact {
            id: 7,
            given_name: "Ada".into(),
            surname: "Lovelace".into(),
            display_name: "Ada Lovelace".into(),
            emails: vec![LabeledValue { label: "Home".into(), value: "ada@example.com".into() }],
            birthday: NaiveDate::from_ymd_opt(Store::birthday_no_year(), 12, 10),
            websites: vec!["https://a.example".into(), "https://b.example".into()],
            ..Default::default()
        };
        let mut f = FormState::from_contact(&c);
        assert_eq!(f.emails[0].0, "Personal");
        assert_eq!(f.phones.len(), 1, "an empty phone row is offered");
        assert_eq!((f.bday_month, f.bday_day, f.bday_year.as_str()), (12, 10, ""));
        let back = f.to_contact();
        assert_eq!(back.birthday, c.birthday);
        assert_eq!(back.websites, c.websites);
        assert!(back.phones.is_empty());

        f.given = "Augusta".into();
        f.website = String::new();
        let renamed = f.to_contact();
        assert_eq!(renamed.display_name, "Augusta Lovelace");
        assert_eq!(renamed.websites, vec!["https://b.example".to_string()]);

        f.bday_day = 31;
        f.bday_month = 2;
        assert!(f.errors().birthday);
        f.bday_month = 0;
        f.bday_day = 0;
        assert!(!f.errors().birthday);
        f.emails[0].1 = "not-an-email".into();
        assert_eq!(f.errors().emails, vec![0]);

        let empty = FormState::from_contact(&Contact::default());
        assert!(!empty.can_save());
        let mut company_only = empty.clone();
        company_only.company = "Bayview Dental".into();
        assert!(company_only.can_save());
    }
}
