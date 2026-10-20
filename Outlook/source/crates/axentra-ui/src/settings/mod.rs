//! Settings dialog (docs/spec/shell.md §7): category nav with search, page
//! nav, page content, and a Save / Discard footer for pages that edit the
//! settings draft. Appearance changes preview live and revert on Discard.

pub mod accounts;
mod account_pages;
pub mod backup;
mod calendar;
pub mod controls;
mod general;
mod mail;
pub mod pages;
mod people;

use dioxus::prelude::*;

pub use controls::{SettingsCtx, use_settings};
pub use pages::{Category, PageId};

use crate::kit::{Button, ConfirmDialog, Dialog, Icon, IconButton, MessageBar, SearchField, Variant, dom};
use crate::shell::use_shell;
use crate::state::{SettingsPage, use_app};

/// Something waiting on "Discard unsaved changes?".
#[derive(Clone, Copy, Debug, PartialEq)]
enum Pending {
    Page(PageId),
    Close,
}

/// First page to show for a category: the requested page, else the last
/// visited page of that category, else its first page.
fn initial_page(cat: Category, requested: Option<PageId>, last: Option<PageId>) -> PageId {
    requested
        .filter(|p| p.category() == cat)
        .or(last.filter(|p| p.category() == cat))
        .unwrap_or(cat.pages()[0])
}

#[component]
pub fn SettingsDialog(page: SettingsPage) -> Element {
    let mut app = use_app();
    let mut shell = use_shell();
    let cat = Category::from_settings_page(page);
    let mut current = use_signal(|| initial_page(cat, *shell.settings_page.peek(), *shell.settings_last.peek()));
    let mut draft = use_signal(|| app.settings.peek().clone());
    let mut base = use_signal(|| app.settings.peek().clone());
    let mut go = use_signal(|| None::<PageId>);
    let mut pending = use_signal(|| None::<Pending>);
    let mut query = use_signal(String::new);
    let mut elsewhere = use_signal(|| false);
    use_context_provider(|| SettingsCtx { draft, base, go });

    let dirty = *draft.read() != *base.read();
    let page_id = current();
    let uses_draft = page_id.uses_draft();

    // These run inside effects too, so they only peek at `current`:
    // subscribing an effect to it would undo every page change.
    let mut show = move |p: PageId| {
        current.set(p);
        shell.settings_last.set(Some(p));
        query.set(String::new());
        let cat = p.category().settings_page();
        if *app.settings_dialog.peek() != Some(cat) {
            app.settings_dialog.set(Some(cat));
        }
    };
    let mut navigate = move |p: PageId| {
        if p == *current.peek() {
            return;
        }
        if *draft.peek() != *base.peek() {
            pending.set(Some(Pending::Page(p)));
        } else {
            show(p);
        }
    };
    let mut really_close = move || {
        shell.settings_last.set(Some(*current.peek()));
        shell.preview.set(None);
        app.settings_dialog.set(None);
    };
    let mut close = move || {
        if *draft.peek() != *base.peek() {
            pending.set(Some(Pending::Close));
        } else {
            really_close();
        }
    };

    // Requests from elsewhere while open (a page link, the gear again).
    use_effect(move || {
        if let Some(p) = (shell.settings_page)() {
            shell.settings_page.set(None);
            navigate(p);
        }
    });
    use_effect(move || {
        if let Some(p) = go() {
            go.set(None);
            navigate(p);
        }
    });
    // Another category asked for through AppState (e.g. a module's "View settings").
    use_effect(use_reactive((&page,), move |(page,)| {
        let cat = Category::from_settings_page(page);
        if current.peek().category() != cat {
            navigate(initial_page(cat, None, None));
        }
    }));
    // Saved settings changed elsewhere (View tab, another window).
    use_effect(move || {
        let saved = app.settings.read().clone();
        if *base.peek() != saved {
            if *draft.peek() == *base.peek() {
                draft.set(saved.clone());
                base.set(saved);
            } else {
                elsewhere.set(true);
            }
        }
    });
    // Live preview of appearance while editing.
    use_effect(move || {
        let d = draft.read().clone();
        let changed = d != *base.read();
        shell.preview.set(changed.then_some(d));
    });

    let save = move |_| {
        let d = draft();
        app.update_settings(|s| *s = d.clone());
        base.set(d);
        elsewhere.set(false);
        dom::announce("Settings saved", false);
    };
    let discard = move |_| {
        let saved = app.settings.peek().clone();
        draft.set(saved.clone());
        base.set(saved);
        elsewhere.set(false);
    };
    let hits = pages::search(&query());
    let searching = !query().trim().is_empty();
    let current_cat = page_id.category();
    rsx! {
        Dialog { title: "Settings", bare: true, width: 1080, height: 760, class: "ax-settings", modal_scrim: true, on_close: move |_| close(),
            div { class: "ax-settings-grid",
                nav { class: "ax-settings-cats", "aria-label": "Settings categories",
                    h2 { class: "ax-settings-title", "Settings" }
                    SearchField {
                        value: query(),
                        placeholder: "Search settings",
                        oninput: move |v| query.set(v),
                        onkeydown: move |e: KeyboardEvent| {
                            if e.key() == Key::Enter
                                && let Some(h) = pages::search(&query()).first()
                            {
                                let (p, anchor) = (h.page, PageId::anchor(&h.label));
                                navigate(p);
                                flash_later(anchor);
                            }
                        },
                    }
                    div { class: "ax-settings-navlist", role: "tablist", "aria-orientation": "vertical", "aria-label": "Categories", "data-ax-roving": "vertical",
                        for c in Category::ALL {
                            button {
                                key: "{c.label()}",
                                r#type: "button",
                                role: "tab",
                                class: "ax-settings-nav-item",
                                class: if c == current_cat && !searching { "ax-selected" },
                                "aria-selected": if c == current_cat { "true" } else { "false" },
                                "data-ax-item": "true",
                                "data-ax-autofocus": if c == current_cat { "true" },
                                tabindex: if c == current_cat { "0" } else { "-1" },
                                onclick: move |_| {
                                    query.set(String::new());
                                    // Copy out first: `show` writes `settings_last`.
                                    let last = *shell.settings_last.peek();
                                    if c != current().category() {
                                        navigate(initial_page(c, None, last));
                                    }
                                },
                                Icon { name: if c == current_cat { crate::kit::filled_icon(c.icon()) } else { c.icon().to_string() } }
                                span { "{c.label()}" }
                            }
                        }
                    }
                }
                if searching {
                    div { class: "ax-settings-results", role: "region", "aria-label": "Search results",
                        h2 { class: "ax-settings-page-title", "Search results" }
                        if hits.is_empty() {
                            p { class: "ax-muted", "No settings match \"{query}\"." }
                        }
                        div { class: "ax-settings-hits", role: "list", "data-ax-roving": "vertical",
                            for h in hits {
                                button {
                                    key: "{h.crumb}{h.label}",
                                    r#type: "button",
                                    class: "ax-settings-hit",
                                    role: "listitem",
                                    "data-ax-item": "true",
                                    onclick: {
                                        let anchor = PageId::anchor(&h.label);
                                        let p = h.page;
                                        move |_| {
                                            navigate(p);
                                            flash_later(anchor.clone());
                                        }
                                    },
                                    span { class: "ax-settings-hit-label", "{h.label}" }
                                    span { class: "ax-settings-hit-crumb", "{h.page.category().label()} › {h.crumb}" }
                                }
                            }
                        }
                    }
                } else {
                    nav { class: "ax-settings-pages", "aria-label": "{current_cat.label()} settings",
                        div { class: "ax-settings-navlist", role: "tablist", "aria-orientation": "vertical", "aria-label": "{current_cat.label()}", "data-ax-roving": "vertical",
                            for p in current_cat.pages().iter().copied() {
                                button {
                                    key: "{p:?}",
                                    r#type: "button",
                                    role: "tab",
                                    class: "ax-settings-nav-item",
                                    class: if p == page_id { "ax-selected" },
                                    "aria-selected": if p == page_id { "true" } else { "false" },
                                    "aria-controls": "ax-settings-panel",
                                    "data-ax-item": "true",
                                    tabindex: if p == page_id { "0" } else { "-1" },
                                    onclick: move |_| navigate(p),
                                    span { "{p.title()}" }
                                }
                            }
                        }
                    }
                    div { id: "ax-settings-panel", class: "ax-settings-content", role: "tabpanel", "aria-label": "{page_id.title()}",
                        h2 { class: "ax-settings-page-title", "{page_id.title()}" }
                        if elsewhere() && dirty {
                            MessageBar {
                                intent: crate::kit::Intent::Warning,
                                text: "This setting was changed elsewhere. Saving will replace that change.",
                                on_dismiss: move |_| elsewhere.set(false),
                            }
                        }
                        div { class: "ax-settings-scroll",
                            PageBody { page: page_id }
                        }
                        if uses_draft {
                            div { class: "ax-settings-footer",
                                Button { label: "Save", variant: Variant::Primary, disabled: !dirty, title: if dirty { "Save changes" } else { "No changes to save" }, onclick: save }
                                Button { label: "Discard", disabled: !dirty, title: if dirty { "Discard changes" } else { "No changes to discard" }, onclick: discard }
                            }
                        }
                    }
                }
                IconButton { icon: "dismiss", title: "Close", class: "ax-settings-close", onclick: move |_| close() }
            }
            if let Some(p) = pending() {
                ConfirmDialog {
                    title: "Discard unsaved changes?",
                    message: "You have changes on this page that haven't been saved.",
                    confirm_label: "Discard",
                    cancel_label: "Keep editing",
                    on_cancel: move |_| pending.set(None),
                    on_confirm: move |_| {
                        pending.set(None);
                        let saved = app.settings.peek().clone();
                        draft.set(saved.clone());
                        base.set(saved);
                        elsewhere.set(false);
                        match p {
                            Pending::Page(p) => show(p),
                            Pending::Close => really_close(),
                        }
                    },
                }
            }
        }
    }
}

/// Highlight a searched option once its page has rendered.
fn flash_later(anchor: String) {
    spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
        dom::flash(&format!("#{anchor}"));
    });
}

#[component]
fn PageBody(page: PageId) -> Element {
    use PageId::*;
    match page {
        EmailAccounts => rsx! { accounts::AccountsPage {} },
        Signatures => rsx! { account_pages::SignaturesPage {} },
        Categories => rsx! { account_pages::CategoriesPage {} },
        AutomaticReplies => rsx! { account_pages::AutomaticRepliesPage {} },
        SharedWithMe => rsx! { account_pages::SharedWithMePage {} },
        Storage => rsx! { account_pages::StoragePage {} },
        Backup => rsx! { backup::BackupPage {} },
        LanguageTime => rsx! { general::LanguageTimePage {} },
        Appearance => rsx! { general::AppearancePage {} },
        Notifications => rsx! { general::NotificationsPage {} },
        Accessibility => rsx! { general::AccessibilityPage {} },
        Search => rsx! { general::SearchPage {} },
        Offline => rsx! { general::OfflinePage {} },
        Privacy => rsx! { general::PrivacyPage {} },
        DistributionGroups => rsx! { general::DistributionGroupsPage {} },
        About => rsx! { general::AboutPage {} },
        MessageList => rsx! { mail::MessageListPage {} },
        ReadingPane => rsx! { mail::ReadingPanePage {} },
        FolderPane => rsx! { mail::FolderPanePage {} },
        ComposeReply => rsx! { mail::ComposeReplyPage {} },
        Attachments => rsx! { mail::AttachmentsPage {} },
        MessageHandling => rsx! { mail::MessageHandlingPage {} },
        Junk => rsx! { mail::JunkPage {} },
        Sweep => rsx! { mail::SweepPage {} },
        Rules | QuickSteps | CustomizeActions | Forwarding | ConditionalFormatting | SearchFolders | Smime | RetentionPolicies | Groups => {
            rsx! { mail::ModulePage { page } }
        }
        CalendarView => rsx! { calendar::CalendarViewPage {} },
        EventsInvitations => rsx! { calendar::EventsInvitationsPage {} },
        Weather | SharedCalendars | CalendarActions | EventsFromEmail => rsx! { calendar::CalendarModulePage { page } },
        Contacts => rsx! { people::ContactsPage {} },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_page_prefers_request_then_last_visit() {
        assert_eq!(initial_page(Category::Mail, Some(PageId::ReadingPane), None), PageId::ReadingPane);
        assert_eq!(initial_page(Category::Mail, Some(PageId::Appearance), Some(PageId::Rules)), PageId::Rules);
        assert_eq!(initial_page(Category::General, None, Some(PageId::Rules)), PageId::LanguageTime);
    }

    #[test]
    fn old_settings_rows_load_with_new_defaults() {
        // A settings row saved before the shell's fields existed.
        let old = r#"{"theme":"Dark","density":"Compact","focused_inbox":false}"#;
        let s: axentra_core::Settings = serde_json::from_str(old).unwrap();
        assert_eq!(s.theme, axentra_core::ThemeMode::Dark);
        assert!(!s.focused_inbox);
        assert_eq!(s.color_theme, "blue");
        assert_eq!(s.preview_lines, 1);
        assert!(s.show_key_tips && s.notify_mail && s.offline_enabled);
        assert_eq!(s.high_contrast, None);
        assert!(s.unpinned_apps.is_empty());
    }
}
