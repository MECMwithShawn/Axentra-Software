//! Manage one account: nickname and display name, sync status and server
//! settings, sign in again, set as primary, remove (with confirmation).

use axentra_core::{AccountId, AccountKind};
use axentra_sync::config::AuthMethod;
use axentra_sync::{AccountConfig, AccountHealth, AccountState};
use dioxus::prelude::*;

use super::list::StatusLine;
use super::widgets::{AcctDialog, DialogButton, Field, MessageBar};
use super::{Action, engine, provider_label};
use crate::kit::{Avatar, Icon};
use crate::state::{store, use_app};

#[component]
pub fn ManagePanel(account_id: AccountId, live_rev: u64, on_action: EventHandler<Action>, on_back: EventHandler<()>) -> Element {
    let app = use_app();
    let _ = app.accounts_rev();
    let _ = live_rev;
    // Hooks first (they must run on every render).
    let initial = use_hook(|| store().account(account_id).ok().flatten());
    let mut nickname = use_signal(|| initial.as_ref().map(|a| a.nickname.clone()).unwrap_or_default());
    let mut display_name = use_signal(|| initial.as_ref().map(|a| a.display_name.clone()).unwrap_or_default());
    let Some(account) = store().account(account_id).ok().flatten() else {
        return rsx! {
            div { class: "ax-acct-manage",
                button { r#type: "button", class: "ax-acct-back", onclick: move |_| on_back.call(()), Icon { name: "chevron_left", size: 16 } span { "Email accounts" } }
                p { class: "ax-acct-text", "This account was removed." }
            }
        };
    };
    let cfg = AccountConfig::of(&account);
    let health = AccountHealth::of(&account);
    let local = account.kind == AccountKind::Local;
    let primary = store().primary_account().ok().flatten().is_some_and(|p| p.id == account_id);
    let live = engine().account_status(account_id);
    let syncing = live.as_ref().is_some_and(|l| l.syncing);

    let dirty = nickname() != account.nickname || display_name().trim() != account.display_name;
    let save = move |_| {
        let nick = nickname.peek().trim().to_string();
        let name = display_name.peek().trim().to_string();
        let r = store().update_account_with(account_id, |a| {
            a.nickname = nick.clone();
            if !name.is_empty() {
                a.display_name = name.clone();
            }
        });
        if app.check(r).is_some() {
            app.bump_accounts();
            app.toast("Account details saved");
        }
    };

    let servers = match (&cfg.imap, &cfg.smtp) {
        (Some(i), Some(s)) if account.kind == AccountKind::Imap => Some((
            format!("{} · port {} · {}", i.host, i.port, i.security.label()),
            format!("{} · port {} · {}", s.host, s.port, s.security.label()),
        )),
        _ => None,
    };
    let sign_in_kind = match cfg.auth {
        AuthMethod::Password => "Password (saved in your keyring)",
        AuthMethod::OauthGraph => "Microsoft account (Microsoft Graph)",
        AuthMethod::Xoauth2Microsoft => "Microsoft account (IMAP with OAuth)",
        AuthMethod::Xoauth2Google => "Google account (IMAP with OAuth)",
    };
    let email = account.email.clone();
    let name = if account.nickname.is_empty() { account.display_name.clone() } else { account.nickname.clone() };
    let provider = provider_label(&account);
    let last_sync = account
        .last_sync_at
        .map(|t| t.with_timezone(&chrono::Local).format("%-m/%-d/%Y %-I:%M %p").to_string())
        .unwrap_or_else(|| "Never".into());

    rsx! {
        div { class: "ax-acct-manage",
            button {
                r#type: "button",
                class: "ax-acct-back",
                title: "Back to Email accounts",
                onmounted: move |e| async move {
                    let _ = e.set_focus(true).await;
                },
                onclick: move |_| on_back.call(()),
                Icon { name: "chevron_left", size: 16 }
                span { "Email accounts" }
            }
            div { class: "ax-acct-manage-head",
                Avatar { name: account.display_name.clone(), email: account.email.clone(), size: 48 }
                div { class: "ax-acct-manage-id",
                    div { class: "ax-acct-manage-name",
                        span { class: "ax-ellipsis", "{name}" }
                        if primary { span { class: "ax-acct-pill", "Primary" } }
                    }
                    div { class: "ax-acct-row-email ax-ellipsis", "{email}" }
                    div { class: "ax-acct-row-meta", span { class: "ax-acct-provider-name", "{provider}" } }
                }
            }
            if let Some(text) = health.message.clone() {
                MessageBar {
                    intent: health.intent,
                    text,
                    actions: rsx! {
                        if let Some(label) = health.action {
                            button {
                                r#type: "button",
                                class: "ax-btn ax-small",
                                onclick: move |_| match label {
                                    "Sign in" => on_action.call(Action::Reauth(account_id)),
                                    "Unlock" => on_action.call(Action::Unlock(account_id)),
                                    _ => engine().sync_account(account_id),
                                },
                                "{label}"
                            }
                        }
                    },
                }
            }

            section { class: "ax-acct-section", "aria-labelledby": "ax-acct-details-h",
                h3 { id: "ax-acct-details-h", class: "ax-acct-section-title", "Account details" }
                div { class: "ax-acct-form ax-acct-form-narrow",
                    Field {
                        label: "Your name",
                        value: display_name(),
                        hint: "Shown to people you send mail to",
                        oninput: move |v: String| display_name.set(v),
                    }
                    Field {
                        label: "Nickname",
                        value: nickname(),
                        placeholder: "For example, Work or Personal",
                        hint: "Shown instead of the address in the folder pane",
                        oninput: move |v: String| nickname.set(v),
                    }
                    div { class: "ax-acct-inline-actions",
                        span { title: if dirty { "" } else { "No changes to save" },
                            DialogButton { label: "Save", primary: true, disabled: !dirty, onclick: save }
                        }
                    }
                }
            }

            section { class: "ax-acct-section", "aria-labelledby": "ax-acct-sync-h",
                h3 { id: "ax-acct-sync-h", class: "ax-acct-section-title", "Sync" }
                dl { class: "ax-acct-facts",
                    dt { "Status" }
                    dd { StatusLine { account: account.clone(), live: live.clone() } }
                    if !local {
                        dt { "Last synced" }
                        dd { "{last_sync}" }
                        dt { "Sign-in" }
                        dd { "{sign_in_kind}" }
                    }
                    if let Some((incoming, outgoing)) = servers {
                        dt { "Incoming mail (IMAP)" }
                        dd { class: "ax-selectable", "{incoming}" }
                        dt { "Outgoing mail (SMTP)" }
                        dd { class: "ax-selectable", "{outgoing}" }
                    }
                    if !cfg.capabilities.is_empty() {
                        dt { "Server features" }
                        dd { class: "ax-acct-caps ax-selectable", {cfg.capabilities.join(" ")} }
                    }
                }
                div { class: "ax-acct-inline-actions",
                    span { title: if local { "This account is stored on this device only, so there's nothing to sync." } else if syncing { "Syncing now…" } else { "" },
                        DialogButton { label: "Sync now", disabled: local || syncing, onclick: move |_| engine().sync_account(account_id) }
                    }
                    if !local {
                        DialogButton { label: "Sign in again", onclick: move |_| on_action.call(Action::Reauth(account_id)) }
                    }
                    if matches!(cfg.state, AccountState::KeyringLocked) {
                        DialogButton { label: "Unlock keyring", onclick: move |_| on_action.call(Action::Unlock(account_id)) }
                    }
                }
            }

            section { class: "ax-acct-section", "aria-labelledby": "ax-acct-manage-h",
                h3 { id: "ax-acct-manage-h", class: "ax-acct-section-title", "Manage" }
                div { class: "ax-acct-manage-rows",
                    div { class: "ax-acct-manage-row",
                        div {
                            div { class: "ax-acct-strong", "Primary account" }
                            div { class: "ax-acct-hint", "New mail, events, contacts and tasks are created in the primary account." }
                        }
                        span { title: if primary { "This is already your primary account" } else { "" },
                            DialogButton { label: "Set as primary", disabled: primary, onclick: move |_| on_action.call(Action::SetPrimary(account_id)) }
                        }
                    }
                    div { class: "ax-acct-manage-row",
                        div {
                            div { class: "ax-acct-strong", "Remove account" }
                            div { class: "ax-acct-hint",
                                if local { "Deletes this account's items from this device." } else { "Removes the account from Axentra Outlook. Nothing is deleted from the server." }
                            }
                        }
                        DialogButton { label: "Remove", danger: true, onclick: move |_| on_action.call(Action::Remove(account_id)) }
                    }
                }
            }
        }
    }
}

/// Destructive confirmation (role="alertdialog", initial focus on Cancel).
#[component]
pub fn RemoveDialog(account_id: AccountId, on_done: EventHandler<bool>) -> Element {
    let app = use_app();
    let mut busy = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let Some(account) = store().account(account_id).ok().flatten() else {
        return rsx! {};
    };
    let local = account.kind == AccountKind::Local;
    let counts = store().account_item_counts(account_id).unwrap_or_default();
    let what = item_summary(counts);
    let email = account.email.clone();
    let body = match (local, what.is_empty()) {
        (true, false) => format!("{email} is stored only on this device. Its {what} will be permanently deleted."),
        (true, true) => format!("{email} is stored only on this device and will be deleted."),
        (false, false) => format!(
            "{email} and its {what} will be removed from Axentra Outlook on this device. Nothing is deleted from the server, and you can add the account again later."
        ),
        (false, true) => format!(
            "{email} will be removed from Axentra Outlook on this device. Nothing is deleted from the server, and you can add the account again later."
        ),
    };
    let remove = move |_| {
        busy.set(true);
        spawn(async move {
            match engine().remove_account(account_id).await {
                Ok(()) => {
                    app.bump_accounts();
                    app.toast("Account removed");
                    on_done.call(true);
                }
                Err(e) => {
                    busy.set(false);
                    error.set(Some(e.to_string()));
                }
            }
        });
    };
    rsx! {
        AcctDialog {
            title: "Remove account?",
            role: "alertdialog",
            described_by: "ax-acct-remove-body",
            on_cancel: move |_| on_done.call(false),
            actions: rsx! {
                DialogButton { label: "Cancel", autofocus: true, disabled: busy(), onclick: move |_| on_done.call(false) }
                DialogButton { label: if busy() { "Removing…" } else { "Remove" }, danger: true, disabled: busy(), onclick: remove }
            },
            div { class: "ax-acct-step",
                if let Some(e) = error() { MessageBar { intent: "error", text: e } }
                p { id: "ax-acct-remove-body", class: "ax-acct-text", "{body}" }
            }
        }
    }
}

/// "12 messages, 3 events and 1 task" (zero counts left out; empty when
/// the account holds nothing).
pub fn item_summary((messages, events, contacts, tasks): (i64, i64, i64, i64)) -> String {
    let parts: Vec<String> = [(messages, "message", "messages"), (events, "event", "events"), (contacts, "contact", "contacts"), (tasks, "task", "tasks")]
        .into_iter()
        .filter(|(n, _, _)| *n > 0)
        .map(|(n, one, many)| format!("{n} {}", if n == 1 { one } else { many }))
        .collect();
    match parts.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::item_summary;

    #[test]
    fn removal_summary_skips_empty_counts() {
        assert_eq!(item_summary((0, 0, 0, 0)), "");
        assert_eq!(item_summary((1, 0, 0, 0)), "1 message");
        assert_eq!(item_summary((12, 3, 0, 1)), "12 messages, 3 events and 1 task");
        assert_eq!(item_summary((0, 0, 2, 5)), "2 contacts and 5 tasks");
    }
}
