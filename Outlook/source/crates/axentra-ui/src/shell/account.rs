//! Account Manager flyout under the header avatar (docs/spec/shell.md §3.6).

use axentra_core::*;
use dioxus::prelude::*;

use super::launcher::FlyoutPanel;
use super::use_shell;
use crate::kit::{Avatar, Button, Variant};
use crate::settings::PageId;
use crate::state::{NavRequest, store, use_app};

/// Unread messages in an account's Inbox.
fn inbox_unread(account: AccountId) -> i64 {
    store().folder_by_role(account, FolderRole::Inbox).ok().flatten().map(|f| f.unread_count).unwrap_or(0)
}

#[component]
pub fn AccountFlyout() -> Element {
    let app = use_app();
    let mut shell = use_shell();
    let _ = app.accounts_rev();
    let _ = app.mail_rev();
    let accounts = store().accounts().unwrap_or_default();
    let Some((me, others)) = accounts.split_first() else {
        return rsx! {
            FlyoutPanel { label: "Account manager", class: "ax-account-flyout", on_close: move |_| shell.flyout.set(None),
                div { class: "ax-account-empty",
                    p { "No account is set up yet." }
                    Button {
                        label: "Add account",
                        icon: "person_add",
                        variant: Variant::Primary,
                        onclick: move |_| shell.open_settings(app, PageId::EmailAccounts),
                    }
                }
            }
        };
    };
    let kind = match me.kind {
        AccountKind::Local => "Local account",
        AccountKind::Imap => "IMAP account",
        AccountKind::Microsoft => "Microsoft account",
    };
    rsx! {
        FlyoutPanel { label: "Account manager", class: "ax-account-flyout", on_close: move |_| shell.flyout.set(None),
            div { class: "ax-account-me",
                Avatar { name: me.display_name.clone(), email: me.email.clone(), size: 48 }
                div { class: "ax-account-me-text",
                    div { class: "ax-account-name ax-ellipsis", "{me.display_name}" }
                    div { class: "ax-account-email ax-ellipsis", "{me.email}" }
                    div { class: "ax-account-kind", "{kind}" }
                }
            }
            if !others.is_empty() {
                div { class: "ax-account-section", "Other accounts" }
                div { class: "ax-account-list", "data-ax-roving": "vertical",
                    for a in others.iter().cloned() {
                        button {
                            key: "{a.id}",
                            r#type: "button",
                            class: "ax-account-row",
                            "data-ax-item": "true",
                            title: "Go to {a.email}",
                            onclick: {
                                let id = a.id;
                                move |_| {
                                    shell.flyout.set(None);
                                    if let Some(f) = store().folder_by_role(id, FolderRole::Inbox).ok().flatten() {
                                        app.navigate(NavRequest::Folder(f.id));
                                    }
                                }
                            },
                            Avatar { name: a.display_name.clone(), email: a.email.clone(), size: 32 }
                            div { class: "ax-account-me-text",
                                div { class: "ax-ellipsis", "{a.display_name}" }
                                div { class: "ax-account-email ax-ellipsis", "{a.email}" }
                            }
                            if inbox_unread(a.id) > 0 {
                                span { class: "ax-count", "aria-label": "{inbox_unread(a.id)} unread", "{inbox_unread(a.id)}" }
                            }
                        }
                    }
                }
            }
            div { class: "ax-account-actions",
                Button {
                    label: "Add account",
                    icon: "person_add",
                    variant: Variant::Subtle,
                    onclick: move |_| shell.open_settings(app, PageId::EmailAccounts),
                }
                Button {
                    label: "Manage accounts",
                    icon: "settings",
                    variant: Variant::Subtle,
                    onclick: move |_| shell.open_settings(app, PageId::EmailAccounts),
                }
            }
        }
    }
}
