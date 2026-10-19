//! Settings > Accounts > Email accounts ("Your accounts", shell spec §7.2;
//! protocols spec §2): the account list with live sync status, Add account,
//! Manage (name, nickname, sync, sign in again, primary, remove), and the
//! pieces other areas embed: [`AddAccountDialog`] (folder pane "Add
//! account") and [`AccountHealthBar`] (message bar above the message list).

mod add;
mod form;
mod list;
mod manage;
mod widgets;

use std::sync::OnceLock;
use std::time::Duration;

use axentra_core::{Account, AccountId, AccountKind};
use axentra_sync::{AccountConfig, AccountHealth, Keyring, SyncHandle, SyncStatus};
use dioxus::prelude::*;

pub use add::AddAccountDialog;
use list::{AccountRow, handle_row_key};
use manage::{ManagePanel, RemoveDialog};
use widgets::MessageBar;

use crate::kit::{Button, EmptyState, Variant};
use crate::state::{SettingsPage, store, store_arc, sync, use_app};

/// The sync engine. The desktop app installs one at startup; the preview
/// server and tests don't, so one is started on first use with an
/// in-memory keyring (the preview's store is in memory too).
pub fn engine() -> &'static SyncHandle {
    if let Some(s) = sync() {
        return s;
    }
    static LOCAL: OnceLock<SyncHandle> = OnceLock::new();
    LOCAL.get_or_init(|| SyncHandle::start_with_keyring(store_arc(), Keyring::memory()))
}

/// Something a row or the manage panel asks the page to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Manage(AccountId),
    Remove(AccountId),
    SetPrimary(AccountId),
    Reauth(AccountId),
    Unlock(AccountId),
}

/// "Microsoft 365", "Gmail", "IMAP", or "On this device" for the demo account.
pub fn provider_label(a: &Account) -> &'static str {
    match a.kind {
        AccountKind::Local => "On this device",
        _ => AccountConfig::of(a).provider.label(),
    }
}

/// Live engine status for the page, polled while it is open.
fn use_live_status() -> (Signal<SyncStatus>, Signal<u64>) {
    let app = use_app();
    let mut live = use_signal(|| engine().status());
    let mut rev = use_signal(|| 0u64);
    use_future(move || async move {
        let e = engine();
        let mut seen = e.change_counter();
        loop {
            tokio::time::sleep(Duration::from_millis(700)).await;
            let st = e.status();
            if *live.peek() != st {
                live.set(st);
                rev += 1;
            }
            // The shell polls the installed engine; a locally started one is ours to watch.
            if sync().is_none() {
                let now = e.change_counter();
                if now != seen {
                    seen = now;
                    app.bump_accounts();
                }
            }
            // Notices are taken once, so draining here too (the shell does it
            // app-wide) never shows a toast twice.
            for n in e.take_notices() {
                if n.error {
                    app.error(n.text);
                } else {
                    app.toast(n.text);
                }
            }
        }
    });
    (live, rev)
}

#[component]
pub fn AccountsPage(
    /// Render the "Email accounts" page heading (off when the settings shell
    /// draws page titles itself).
    #[props(default = true)]
    show_title: bool,
) -> Element {
    let app = use_app();
    let _ = app.accounts_rev();
    let mut managing = use_signal(|| None::<AccountId>);
    let mut adding = use_signal(|| false);
    let mut reauth = use_signal(|| None::<AccountId>);
    let mut removing = use_signal(|| None::<AccountId>);
    let focused = use_signal(|| 0usize);
    let (live, live_rev) = use_live_status();

    let accounts = store().accounts().unwrap_or_default();
    let primary = accounts.first().map(|a| a.id);
    let ids: Vec<AccountId> = accounts.iter().map(|a| a.id).collect();
    let tab_stop = (*focused.read()).min(ids.len().saturating_sub(1));
    let on_action = move |a: Action| match a {
        Action::Manage(id) => managing.set(Some(id)),
        Action::Remove(id) => removing.set(Some(id)),
        Action::Reauth(id) => reauth.set(Some(id)),
        Action::SetPrimary(id) => {
            if app.check(store().set_primary_account(id)).is_some() {
                app.bump_accounts();
                if let Some(a) = store().account(id).ok().flatten() {
                    app.toast(format!("{} is now your primary account", a.email));
                }
            }
        }
        Action::Unlock(id) => {
            spawn(async move {
                if let Err(e) = engine().unlock_keyring(id).await {
                    app.error(e.to_string());
                }
            });
        }
    };
    let status = live();

    rsx! {
        div { class: "ax-acct-page",
            if show_title {
                h2 { class: "ax-acct-page-title", "Email accounts" }
            }
            if let Some(id) = managing() {
                ManagePanel {
                    key: "{id}",
                    account_id: id,
                    live_rev: live_rev(),
                    on_action,
                    on_back: move |_| {
                        managing.set(None);
                        list::focus_row_later(id);
                    },
                }
            } else {
                section { class: "ax-acct-section", "aria-labelledby": "ax-acct-your-h",
                    div { class: "ax-acct-section-head",
                        h3 { id: "ax-acct-your-h", class: "ax-acct-section-title", "Your accounts" }
                        // The empty state carries its own Add account button.
                        if !accounts.is_empty() {
                            Button { label: "Add account", icon: "add", variant: Variant::Primary, onclick: move |_| adding.set(true) }
                        }
                    }
                    p { class: "ax-acct-hint",
                        "Add, manage and remove email accounts. Mail, calendar, contacts and tasks stay in sync with each account's server."
                    }
                    if accounts.is_empty() {
                        EmptyState { icon: "person_add", title: "Add an account to get started",
                            Button { label: "Add account", variant: Variant::Primary, onclick: move |_| adding.set(true) }
                        }
                    } else {
                        ul { class: "ax-acct-list", role: "list", "aria-label": "Your accounts",
                            for (i, a) in accounts.into_iter().enumerate() {
                                AccountRow {
                                    key: "{a.id}",
                                    primary: Some(a.id) == primary,
                                    live: status.details.iter().find(|d| d.account_id == a.id).cloned(),
                                    account: a,
                                    on_action,
                                    tab_stop: i == tab_stop,
                                    on_focus: move |_| {
                                        let mut f = focused;
                                        f.set(i);
                                    },
                                    on_key: {
                                        let ids = ids.clone();
                                        move |e: KeyboardEvent| {
                                            let mut act = on_action;
                                            handle_row_key(&e, i, &ids, &mut act, focused);
                                        }
                                    },
                                }
                            }
                        }
                    }
                }
            }
            if adding() {
                AddAccountDialog { on_close: move |_| adding.set(false) }
            }
            if let Some(id) = reauth() {
                AddAccountDialog { reauth: Some(id), on_close: move |_| reauth.set(None) }
            }
            if let Some(id) = removing() {
                RemoveDialog {
                    account_id: id,
                    on_done: move |removed: bool| {
                        removing.set(None);
                        if removed && managing() == Some(id) {
                            managing.set(None);
                        }
                    },
                }
            }
        }
    }
}

/// Message bar for an account (§2.6): progress of the first sync, then
/// offline, throttled, sign-in needed, keyring locked or can't connect, with
/// its action. Renders nothing while the account is healthy or after the
/// user dismissed the bar (until the state changes). Embed it above the
/// message list (Mail) for the selected account.
#[component]
pub fn AccountHealthBar(account_id: AccountId) -> Element {
    let mut app = use_app();
    let _ = app.accounts_rev();
    let mut reauth = use_signal(|| false);
    let Some(account) = store().account(account_id).ok().flatten() else { return rsx! {} };
    if account.kind == AccountKind::Local {
        return rsx! {};
    }
    if let Some(progress) = engine().account_status(account_id).filter(|l| l.first_sync).and_then(|l| l.progress) {
        return rsx! {
            MessageBar { intent: "info", text: progress, class: "ax-acct-health" }
        };
    }
    let cfg = AccountConfig::of(&account);
    let health = AccountHealth::of(&account);
    let Some(text) = health.message.clone().filter(|_| !cfg.bar_dismissed) else {
        return rsx! {
            if reauth() { AddAccountDialog { reauth: Some(account_id), on_close: move |_| reauth.set(false) } }
        };
    };
    let dismiss = move |_| {
        let _ = store().update_account_with(account_id, |a| {
            let mut c = AccountConfig::of(a);
            c.bar_dismissed = true;
            c.store_into(a);
        });
        app.bump_accounts();
    };
    let action = health.action;
    let is_error = health.intent == "error";
    rsx! {
        MessageBar {
            intent: health.intent,
            text,
            class: "ax-acct-health",
            on_dismiss: dismiss,
            actions: rsx! {
                if let Some(label) = action {
                    button {
                        r#type: "button",
                        class: "ax-btn ax-small",
                        onclick: move |_| match label {
                            "Sign in" => reauth.set(true),
                            "Unlock" => {
                                spawn(async move {
                                    if let Err(e) = engine().unlock_keyring(account_id).await {
                                        app.error(e.to_string());
                                    }
                                });
                            }
                            _ => engine().sync_account(account_id),
                        },
                        "{label}"
                    }
                }
                if is_error {
                    button {
                        r#type: "button",
                        class: "ax-btn ax-small",
                        onclick: move |_| app.settings_dialog.set(Some(SettingsPage::Accounts)),
                        "Account settings"
                    }
                }
            },
        }
        if reauth() {
            AddAccountDialog { reauth: Some(account_id), on_close: move |_| reauth.set(false) }
        }
    }
}

