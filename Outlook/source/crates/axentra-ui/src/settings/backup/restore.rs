//! Restore… : password, preview of what the backup holds, confirmation,
//! then a summary of what was added and what was already there.

use std::sync::Arc;

use axentra_core::AccountKind;
use axentra_core::backup::{self, BackupFile, Counts, Payload, RestoreOptions, RestoreSummary};
use axentra_sync::Secret;
use chrono::Utc;
use dioxus::prelude::*;

use super::host::{REV, off_thread};
use super::{counts_text, vault, when};
use crate::kit::{Button, Checkbox, Dialog, Intent, MessageBar, Spinner, TextField, Variant};
use crate::state::{store, store_arc, use_app};

#[derive(Clone, Debug, PartialEq)]
enum Step {
    Password,
    Opening,
    Preview(Arc<Payload>),
    Restoring,
    Done(Box<RestoreSummary>),
}

#[component]
pub fn RestoreDialog(file: BackupFile, on_close: EventHandler<()>) -> Element {
    let mut app = use_app();
    let h24 = app.settings.read().use_24h_time;
    let mut step = use_signal(|| Step::Password);
    let mut password = use_signal(String::new);
    let mut prefilled = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let mut also_settings = use_signal(|| false);

    // Prefill the remembered password (never prompts for it).
    use_hook(move || {
        spawn(async move {
            if let Ok(Some(p)) = vault::get(false).await
                && password.peek().is_empty()
            {
                password.set(p.expose().to_string());
                prefilled.set(true);
            }
        });
    });

    let path = file.path.clone();
    let open = move || {
        if password.peek().is_empty() {
            error.set(Some("Enter the password for this backup.".into()));
            return;
        }
        let secret = Secret::new(password.peek().clone());
        let path = path.clone();
        error.set(None);
        step.set(Step::Opening);
        spawn(async move {
            match off_thread("axentra-backup-open", move || backup::open_backup(&path, secret.expose())).await {
                Some(Ok(payload)) => {
                    password.set(String::new());
                    step.set(Step::Preview(Arc::new(payload)));
                }
                Some(Err(e)) => {
                    error.set(Some(e.to_string()));
                    step.set(Step::Password);
                }
                None => {
                    error.set(Some("The backup couldn't be read. Try again.".into()));
                    step.set(Step::Password);
                }
            }
        });
    };
    let mut run_restore = move |payload: Arc<Payload>| {
        let opts = RestoreOptions { settings: *also_settings.peek() };
        step.set(Step::Restoring);
        spawn(async move {
            let store_handle = store_arc();
            let p = payload.clone();
            match off_thread("axentra-restore", move || backup::restore(&store_handle, &p, opts, Utc::now())).await {
                Some(Ok(summary)) => {
                    if summary.settings_restored {
                        app.settings.set(store().settings());
                    }
                    app.bump_accounts();
                    *REV.write() += 1;
                    step.set(Step::Done(Box::new(summary)));
                }
                Some(Err(e)) => {
                    error.set(Some(e.to_string()));
                    step.set(Step::Preview(payload));
                }
                None => {
                    error.set(Some("The restore stopped unexpectedly. Nothing was lost; try again.".into()));
                    step.set(Step::Preview(payload));
                }
            }
        });
    };

    let busy = matches!(step(), Step::Opening | Step::Restoring);
    let close = move || {
        if !busy {
            on_close.call(());
        }
    };
    let date = when(file.created_at, h24);
    let size = crate::fmt::file_size(file.size as i64);
    let current = step();
    let actions = match &current {
        Step::Password | Step::Opening => rsx! {
            Button {
                label: "Open backup",
                variant: Variant::Primary,
                disabled: busy || password().is_empty(),
                title: if password().is_empty() { "Enter the password first" } else { "Check the password and show what's in the backup" },
                onclick: {
                    let mut open = open.clone();
                    move |_| open()
                },
            }
            Button { label: "Cancel", disabled: busy, onclick: move |_| close() }
        },
        Step::Preview(p) => {
            let p = p.clone();
            rsx! {
                Button { label: "Restore", variant: Variant::Primary, onclick: move |_| run_restore(p.clone()) }
                Button { label: "Cancel", onclick: move |_| close() }
            }
        }
        Step::Restoring => rsx! {
            Button { label: "Restore", variant: Variant::Primary, disabled: true, title: "Restoring…" }
            Button { label: "Cancel", disabled: true, title: "Wait for the restore to finish" }
        },
        Step::Done(_) => rsx! {
            Button { label: "Close", variant: Variant::Primary, autofocus: true, onclick: move |_| on_close.call(()) }
        },
    };

    rsx! {
        Dialog { title: "Restore from backup", width: 560, modal_scrim: true, on_close: move |_| close(), actions,
            div { class: "ax-backup-dialog-body", "aria-busy": if busy { "true" } else { "false" },
                if let Some(e) = error() {
                    MessageBar { intent: Intent::Error, text: e }
                }
                match current {
                    Step::Password | Step::Opening => rsx! {
                        p { class: "ax-dialog-text", "Backup from {date} ({size})." }
                        TextField {
                            label: "Backup password",
                            input_type: "password",
                            value: password(),
                            autofocus: true,
                            disabled: busy,
                            hint: if prefilled() { "Filled in from your keyring. Backups made before a password change need the old password." } else { "" },
                            oninput: move |v| {
                                password.set(v);
                                prefilled.set(false);
                            },
                            onkeydown: {
                                let mut open = open.clone();
                                move |e: KeyboardEvent| if e.key() == Key::Enter && !busy { open() }
                            },
                        }
                        if busy { Spinner { label: "Opening the backup…" } }
                    },
                    Step::Preview(p) => rsx! { PreviewBody { payload: p.clone(), also_settings: also_settings(), on_settings: move |v| also_settings.set(v) } },
                    Step::Restoring => rsx! { Spinner { label: "Restoring… This can take a minute.", size: 32 } },
                    Step::Done(summary) => rsx! { SummaryBody { summary: *summary } },
                }
            }
        }
    }
}

#[component]
fn PreviewBody(payload: Arc<Payload>, also_settings: bool, on_settings: EventHandler<bool>) -> Element {
    let app = use_app();
    let h24 = app.settings.read().use_24h_time;
    let p = backup::preview(&payload, Some(store()));
    let made = when(p.created_at, h24);
    rsx! {
        p { class: "ax-dialog-text",
            "Made on {made} with Axentra Outlook {p.app_version}. Items you already have are skipped; nothing is deleted or replaced."
        }
        if p.accounts.is_empty() {
            p { class: "ax-muted", "This backup has settings only, no accounts." }
        }
        ul { class: "ax-backup-preview-accounts", "aria-label": "Accounts in this backup",
            for a in p.accounts.iter() {
                li { key: "{a.email}{a.kind:?}", class: "ax-backup-preview-account",
                    div { class: "ax-backup-row-date",
                        span { class: "ax-ellipsis", if a.display_name.is_empty() { "{a.email}" } else { "{a.display_name}" } }
                        span { class: "ax-backup-pill", "{backup::kind_label(a.kind)}" }
                    }
                    if !a.display_name.is_empty() { span { class: "ax-backup-row-meta", "{a.email}" } }
                    span { class: "ax-backup-preview-what",
                        if a.exists {
                            "Merges into your existing account."
                        } else if a.kind == AccountKind::Local {
                            "Will be added."
                        } else {
                            "Will be added. Sign in to it afterwards to start syncing; passwords aren't stored in backups."
                        }
                    }
                    span { class: "ax-backup-row-meta",
                        if !a.complete { "Only items saved on this device: " }
                        "{counts_or_none(&a.counts)}"
                    }
                }
            }
        }
        Checkbox {
            label: "Also restore settings",
            description: "Replaces your current settings, signature and view preferences with the ones in the backup.",
            checked: also_settings,
            onchange: move |v| on_settings.call(v),
        }
    }
}

fn counts_or_none(c: &Counts) -> String {
    let c = Counts { accounts: 0, ..c.clone() };
    let text = counts_text(&c);
    if text.is_empty() { "nothing besides the account setup".into() } else { text }
}

#[component]
fn SummaryBody(summary: RestoreSummary) -> Element {
    let added = counts_text(&summary.added);
    let skipped = counts_text(&summary.skipped);
    let accounts = summary.accounts_added.join(", ");
    rsx! {
        MessageBar {
            intent: Intent::Success,
            title: "Restore complete.",
            text: if summary.added.total_items() == 0 && summary.added.accounts == 0 {
                "Everything in this backup was already here.".to_string()
            } else {
                format!("Added {added}.")
            },
        }
        if !skipped.is_empty() {
            p { class: "ax-dialog-text", "Already here, so skipped: {skipped}." }
        }
        if summary.settings_restored {
            p { class: "ax-dialog-text", "Your settings were restored." }
        }
        if summary.unscheduled > 0 {
            p { class: "ax-dialog-text",
                if summary.unscheduled == 1 {
                    "1 draft was scheduled to send when the backup was made. It's back in Drafts and won't be sent unless you send it again."
                } else {
                    "{summary.unscheduled} drafts were scheduled to send when the backup was made. They're back in Drafts and won't be sent unless you send them again."
                }
            }
        }
        if !summary.accounts_added.is_empty() {
            p { class: "ax-dialog-text", "Accounts added: {accounts}." }
        }
        if !summary.needs_sign_in.is_empty() {
            MessageBar {
                intent: Intent::Warning,
                text: format!(
                    "Sign in to {} under Settings › Accounts › Email accounts to start syncing.",
                    summary.needs_sign_in.join(", ")
                ),
            }
        }
    }
}
