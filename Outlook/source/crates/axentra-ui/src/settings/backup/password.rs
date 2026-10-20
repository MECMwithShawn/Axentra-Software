//! Backup password entry: the "new password" pair with hints and a
//! strength meter, and the dialog that asks for it when it isn't
//! remembered.

use axentra_core::backup;
use axentra_sync::Secret;
use dioxus::prelude::*;

use crate::kit::{Button, Checkbox, Dialog, TextField, Variant, dom};

/// Strength label for `backup::password_strength`.
pub fn strength_label(level: u8) -> &'static str {
    match level {
        0 => "Too short",
        1 => "Weak",
        2 => "Fair",
        _ => "Strong",
    }
}

/// The hint under the confirm field, once there is something to say.
pub fn hint(password: &str, confirm: &str) -> Option<&'static str> {
    if password.is_empty() {
        return None;
    }
    match backup::password_problem(password, confirm) {
        Some(p) if p.contains("match") && confirm.is_empty() => None,
        other => other,
    }
}

#[component]
fn StrengthMeter(password: String) -> Element {
    let level = backup::password_strength(&password);
    let label = strength_label(level);
    rsx! {
        div {
            class: "ax-backup-strength ax-backup-strength-{level}",
            role: "meter",
            "aria-label": "Password strength",
            "aria-valuemin": "0",
            "aria-valuemax": "3",
            "aria-valuenow": "{level}",
            "aria-valuetext": "{label}",
            span { class: "ax-backup-strength-bars", "aria-hidden": "true",
                span { class: "ax-backup-strength-bar" }
                span { class: "ax-backup-strength-bar" }
                span { class: "ax-backup-strength-bar" }
            }
            span { "{label}" }
        }
    }
}

/// Password + confirm fields. Enter in the confirm field submits.
#[component]
pub fn NewPasswordFields(password: Signal<String>, confirm: Signal<String>, on_submit: EventHandler<()>, #[props(default)] autofocus: bool) -> Element {
    let pw = password();
    let hint_text = hint(&pw, &confirm());
    let id = dom::use_unique_id("ax-backup-pw");
    rsx! {
        div { class: "ax-backup-pw",
            TextField {
                id: "{id}-new",
                label: "Backup password",
                input_type: "password",
                value: pw.clone(),
                autofocus,
                hint: "At least 8 characters. A few unrelated words make a strong password.",
                oninput: move |v| password.set(v),
            }
            if !pw.is_empty() { StrengthMeter { password: pw.clone() } }
            TextField {
                id: "{id}-confirm",
                label: "Type it again",
                input_type: "password",
                value: confirm(),
                hint: hint_text.unwrap_or_default(),
                class: if hint_text.is_some() { "ax-backup-field-error" } else { "" },
                oninput: move |v| confirm.set(v),
                onkeydown: move |e: KeyboardEvent| if e.key() == Key::Enter { on_submit.call(()) },
            }
        }
    }
}

/// Asks for a new backup's password when none is remembered. Asks twice:
/// a typo would make the backup impossible to open.
#[component]
pub fn AskPasswordDialog(can_remember: bool, on_close: EventHandler<()>, on_submit: EventHandler<(Secret, bool)>) -> Element {
    let password = use_signal(String::new);
    let confirm = use_signal(String::new);
    let mut remember = use_signal(|| can_remember);
    let ok = backup::password_problem(&password(), &confirm()).is_none();
    let submit = move || {
        if backup::password_problem(&password.peek(), &confirm.peek()).is_none() {
            on_submit.call((Secret::new(password.peek().clone()), *remember.peek()));
        }
    };
    rsx! {
        Dialog {
            title: "Back up now",
            width: 460,
            on_close: move |_| on_close.call(()),
            actions: rsx! {
                Button {
                    label: "Back up",
                    variant: Variant::Primary,
                    disabled: !ok,
                    title: if ok { "Start the backup" } else { "Enter the same password twice (at least 8 characters)" },
                    onclick: move |_| submit(),
                }
                Button { label: "Cancel", onclick: move |_| on_close.call(()) }
            },
            div { class: "ax-backup-dialog-body",
                p { class: "ax-dialog-text", "Choose a password for this backup. You'll need it to restore. Without it, the backup can't be opened." }
                NewPasswordFields { password, confirm, autofocus: true, on_submit: move |_| submit() }
                if can_remember {
                    Checkbox {
                        label: "Remember it in my keyring",
                        description: "Needed for automatic monthly backups.",
                        checked: remember(),
                        onchange: move |v| remember.set(v),
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hints_wait_for_the_confirmation() {
        assert_eq!(hint("", ""), None);
        assert_eq!(hint("short", ""), Some("Use at least 8 characters."));
        assert_eq!(hint("long enough", ""), None);
        assert_eq!(hint("long enough", "long enou"), Some("The passwords don't match."));
        assert_eq!(hint("long enough", "long enough"), None);
        assert_eq!(strength_label(backup::password_strength("correct horse battery staple")), "Strong");
    }
}
