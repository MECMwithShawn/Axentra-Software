//! "Your accounts": one row per account with status and actions. The rows
//! form a roving-focus list: Up/Down/Home/End move between accounts, Enter
//! opens Manage, Delete asks to remove the account.

use axentra_core::{Account, AccountKind, Density};
use axentra_sync::{AccountConfig, AccountHealth, AccountState, AccountSyncInfo};
use dioxus::prelude::*;

use super::{Action, engine, provider_label};
use crate::kit::{Avatar, Icon, MenuButton, MenuItem, keys};
use crate::state::use_app;

/// Row status: spinner while syncing, a coloured glyph when unhealthy.
#[component]
pub fn StatusLine(account: Account, live: Option<AccountSyncInfo>) -> Element {
    let health = AccountHealth::of(&account);
    let syncing = live.as_ref().is_some_and(|l| l.syncing);
    let text = live
        .as_ref()
        .and_then(|l| l.progress.clone().filter(|_| syncing))
        .or_else(|| live.as_ref().map(|l| l.status_line.clone()))
        .unwrap_or_else(|| axentra_sync::config::status_line(&account, chrono::Utc::now()));
    let tone = match health.state {
        AccountState::Error => "ax-acct-tone-error",
        AccountState::NeedsReauth | AccountState::KeyringLocked | AccountState::Throttled => "ax-acct-tone-warning",
        _ => "",
    };
    rsx! {
        span { class: "ax-acct-status {tone}",
            if syncing {
                span { class: "ax-acct-spinner", "aria-hidden": "true" }
            } else if let Some(icon) = health.icon {
                Icon { name: icon, size: 16 }
            }
            span { class: "ax-ellipsis", "{text}" }
        }
    }
}

/// DOM id of a row's focus target (used to move focus between rows).
pub fn row_target_id(id: axentra_core::AccountId) -> String {
    format!("ax-acct-row-{id}")
}

/// Move keyboard focus to an account row that is on screen.
pub fn focus_row(id: axentra_core::AccountId) {
    let _ = document::eval(&format!("var e=document.getElementById('{}');if(e)e.focus();", row_target_id(id)));
}

/// Focus an account row after the next render (it is appearing again).
pub fn focus_row_later(id: axentra_core::AccountId) {
    let _ = document::eval(&format!("setTimeout(function(){{var e=document.getElementById('{}');if(e)e.focus()}},30)", row_target_id(id)));
}

#[component]
pub fn AccountRow(
    account: Account,
    primary: bool,
    live: Option<AccountSyncInfo>,
    on_action: EventHandler<Action>,
    /// This row is the list's tab stop (roving tabindex).
    #[props(default)]
    tab_stop: bool,
    /// Keys pressed on the row (the list moves focus).
    on_key: EventHandler<KeyboardEvent>,
    on_focus: EventHandler<()>,
) -> Element {
    let app = use_app();
    let id = account.id;
    let local = account.kind == AccountKind::Local;
    let cfg = AccountConfig::of(&account);
    let name = if account.nickname.trim().is_empty() { account.display_name.clone() } else { account.nickname.clone() };
    let syncing = live.as_ref().is_some_and(|l| l.syncing);
    let avatar = if app.settings.read().density == Density::Compact { 32 } else { 40 };
    let mut items = vec![
        MenuItem::new("sync", "Sync now")
            .icon("arrow_sync")
            .shortcut("F9")
            .disabled(local || syncing)
            .tooltip(if local { "This account is stored only on this device; there is nothing to sync" } else if syncing { "Syncing now" } else { "Sync this account now" }),
        MenuItem::new("primary", "Set as primary account").icon("star").disabled(primary),
        MenuItem::divider(),
        MenuItem::new("remove", "Remove").icon("delete").shortcut("Delete").danger(),
    ];
    if !local && matches!(cfg.state, AccountState::NeedsReauth | AccountState::Error) {
        items.insert(0, MenuItem::new("reauth", "Sign in again").icon("person_circle"));
    }
    let provider = provider_label(&account);
    let email = account.email.clone();
    let status = live.as_ref().map(|l| l.status_line.clone()).unwrap_or_else(|| axentra_sync::config::status_line(&account, chrono::Utc::now()));
    rsx! {
        li { class: "ax-acct-row", role: "listitem",
            div {
                id: row_target_id(id),
                class: "ax-acct-row-target",
                tabindex: if tab_stop { "0" } else { "-1" },
                "aria-label": if primary { format!("{name}, {email}, {provider}, primary account, {status}") } else { format!("{name}, {email}, {provider}, {status}") },
                "aria-keyshortcuts": "Enter Delete",
                onfocus: move |_| on_focus.call(()),
                onkeydown: move |e: KeyboardEvent| on_key.call(e),
                ondoubleclick: move |_| on_action.call(Action::Manage(id)),
                Avatar { name: account.display_name.clone(), email: account.email.clone(), size: avatar }
                div { class: "ax-acct-row-main",
                    div { class: "ax-acct-row-name",
                        span { class: "ax-ellipsis", "{name}" }
                        if primary { span { class: "ax-acct-pill", "Primary" } }
                    }
                    div { class: "ax-acct-row-email ax-ellipsis", "{email}" }
                    div { class: "ax-acct-row-meta",
                        span { class: "ax-acct-provider-name", "{provider}" }
                        span { class: "ax-acct-dot", "aria-hidden": "true", "·" }
                        StatusLine { account: account.clone(), live: live.clone() }
                    }
                }
            }
            div { class: "ax-acct-row-actions",
                button {
                    r#type: "button",
                    class: "ax-btn ax-acct-btn-row",
                    title: "Manage {email}",
                    onclick: move |_| on_action.call(Action::Manage(id)),
                    "Manage"
                }
                MenuButton {
                    label: "More actions for {email}",
                    icon: "more_horizontal",
                    icon_only: true,
                    items,
                    on_select: move |what: String| match what.as_str() {
                        "sync" => engine().sync_account(id),
                        "primary" => on_action.call(Action::SetPrimary(id)),
                        "remove" => on_action.call(Action::Remove(id)),
                        "reauth" => on_action.call(Action::Reauth(id)),
                        _ => {}
                    },
                }
            }
        }
    }
}

/// What a key on a row does, given the row's position among `len` rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKey {
    Focus(usize),
    Manage,
    Remove,
    Sync,
    Ignore,
}

pub fn row_key(chord: &str, index: usize, len: usize) -> RowKey {
    if len == 0 {
        return RowKey::Ignore;
    }
    match chord {
        "ArrowDown" => RowKey::Focus((index + 1).min(len - 1)),
        "ArrowUp" => RowKey::Focus(index.saturating_sub(1)),
        "Home" => RowKey::Focus(0),
        "End" => RowKey::Focus(len - 1),
        "Enter" | "Space" => RowKey::Manage,
        "Delete" => RowKey::Remove,
        "F9" => RowKey::Sync,
        _ => RowKey::Ignore,
    }
}

/// Handle a key on row `index`: returns true when the event was consumed.
pub fn handle_row_key(e: &KeyboardEvent, index: usize, ids: &[axentra_core::AccountId], mut on_action: impl FnMut(Action), mut focused: Signal<usize>) -> bool {
    let chord = keys::chord(e);
    match row_key(&chord, index, ids.len()) {
        RowKey::Focus(i) => {
            focused.set(i);
            focus_row(ids[i]);
        }
        RowKey::Manage => on_action(Action::Manage(ids[index])),
        RowKey::Remove => on_action(Action::Remove(ids[index])),
        RowKey::Sync => engine().sync_account(ids[index]),
        RowKey::Ignore => return false,
    }
    e.prevent_default();
    e.stop_propagation();
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roving_keys() {
        assert_eq!(row_key("ArrowDown", 0, 3), RowKey::Focus(1));
        assert_eq!(row_key("ArrowDown", 2, 3), RowKey::Focus(2), "stops at the last row");
        assert_eq!(row_key("ArrowUp", 0, 3), RowKey::Focus(0));
        assert_eq!(row_key("End", 0, 3), RowKey::Focus(2));
        assert_eq!(row_key("Home", 2, 3), RowKey::Focus(0));
        assert_eq!(row_key("Enter", 1, 3), RowKey::Manage);
        assert_eq!(row_key("Delete", 1, 3), RowKey::Remove);
        assert_eq!(row_key("KeyX", 1, 3), RowKey::Ignore);
        assert_eq!(row_key("ArrowDown", 0, 0), RowKey::Ignore);
    }
}
