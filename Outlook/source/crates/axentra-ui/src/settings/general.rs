//! Settings > General (docs/spec/shell.md §7.3).

use axentra_core::*;
use dioxus::prelude::*;

use super::controls::{LinkButton, Note, Row, Section, Unavailable, use_settings};
use crate::kit::{Button, Checkbox, Icon, RadioGroup, RadioOption, Select, Switch};
use crate::shell::{keymap, search, theme, use_shell};
use crate::state::{store, sync, use_app};

/// Tri-state settings (follow system / on / off) as radio values.
fn tri_value(v: Option<bool>) -> String {
    match v {
        None => "system".into(),
        Some(true) => "on".into(),
        Some(false) => "off".into(),
    }
}

fn tri_parse(v: &str) -> Option<bool> {
    match v {
        "on" => Some(true),
        "off" => Some(false),
        _ => None,
    }
}

/// Density presets also set the list toggles they imply (§1.14): Roomy
/// shows sender images and attachment previews, Cozy and Compact don't.
pub fn apply_density(s: &mut Settings, d: Density) {
    s.density = d;
    let roomy = d == Density::Roomy;
    s.show_sender_images = roomy;
    s.show_attachment_previews = roomy;
    s.show_preview_text = true;
}

/// IANA time zone of this machine, when it can be found.
pub fn local_time_zone() -> Option<String> {
    if let Ok(tz) = std::env::var("TZ") {
        let tz = tz.trim_start_matches(':').to_string();
        if !tz.is_empty() {
            return Some(tz);
        }
    }
    if let Ok(link) = std::fs::read_link("/etc/localtime") {
        let s = link.to_string_lossy().to_string();
        if let Some(i) = s.find("zoneinfo/") {
            return Some(s[i + 9..].to_string());
        }
    }
    std::fs::read_to_string("/etc/timezone").ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

#[component]
pub fn LanguageTimePage() -> Element {
    let ctx = use_settings();
    let s = ctx.get();
    let offset = chrono::Local::now().format("%:z").to_string();
    let zone = local_time_zone().unwrap_or_else(|| "System time zone".into());
    let sample = chrono::Local::now();
    rsx! {
        Section { title: "Language",
            Row { label: "Language (Country/Region)",
                Select {
                    value: "en-US",
                    options: vec![("en-US".into(), "English (United States)".into())],
                    aria_label: "Language",
                    disabled: true,
                    title: "English (United States) is the only language available in this version",
                    onchange: move |_| {},
                }
            }
            Row { label: "Date format",
                span { class: "ax-set-value", "M/D/YYYY ({sample.format(\"%-m/%-d/%Y\")})" }
            }
        }
        Section { title: "Time format",
            RadioGroup {
                label: "Time format",
                value: if s.use_24h_time { "24" } else { "12" },
                options: vec![
                    RadioOption::new("12", format!("12-hour ({})", sample.format("%-I:%M %p"))),
                    RadioOption::new("24", format!("24-hour ({})", sample.format("%H:%M"))),
                ],
                onchange: move |v: String| ctx.edit(|s| s.use_24h_time = v == "24"),
            }
        }
        Section { title: "Current time zone",
            Row { label: "Time zone", description: "Axentra Outlook uses your system's time zone.",
                span { class: "ax-set-value", "(UTC{offset}) {zone}" }
            }
        }
    }
}

#[component]
pub fn AppearancePage() -> Element {
    let ctx = use_settings();
    let s = ctx.get();
    let current_theme = theme::color_theme(&s.color_theme).id;
    let dark_in_effect = s.theme != ThemeMode::Light;
    rsx! {
        Section { title: "Theme", description: "Colour themes change the header, buttons and highlights.",
            div { class: "ax-theme-grid", role: "radiogroup", "aria-label": "Theme", "data-ax-roving": "grid", "data-ax-cols": "5",
                for t in theme::COLOR_THEMES {
                    button {
                        key: "{t.id}",
                        r#type: "button",
                        role: "radio",
                        class: "ax-theme-tile",
                        class: if t.id == current_theme { "ax-selected" },
                        "aria-checked": if t.id == current_theme { "true" } else { "false" },
                        "aria-label": "{t.label}",
                        "data-ax-item": "true",
                        tabindex: if t.id == current_theme { "0" } else { "-1" },
                        title: "{t.label}",
                        onclick: move |_| ctx.edit(|s| {
                            s.color_theme = t.id.to_string();
                            s.accent = None;
                        }),
                        span { class: "ax-theme-swatch", style: "--ax-swatch:{theme::swatch(t)}",
                            span { class: "ax-theme-swatch-bar" }
                            if t.id == current_theme { span { class: "ax-theme-check", Icon { name: "checkmark", size: 12 } } }
                        }
                        span { class: "ax-theme-name", "{t.label}" }
                    }
                }
            }
        }
        Section { title: "Dark mode",
            RadioGroup {
                label: "Dark mode",
                value: match s.theme { ThemeMode::Light => "light", ThemeMode::Dark => "dark", ThemeMode::System => "system" },
                options: vec![
                    RadioOption::new("light", "Light"),
                    RadioOption::new("dark", "Dark"),
                    RadioOption::new("system", "Use system settings").description("Follow your desktop's light or dark style."),
                ],
                onchange: move |v: String| ctx.edit(|s| s.theme = match v.as_str() { "light" => ThemeMode::Light, "dark" => ThemeMode::Dark, _ => ThemeMode::System }),
            }
            if dark_in_effect {
                Checkbox {
                    label: "Use a light background for messages",
                    description: "Keeps message bodies light while the rest of the app is dark.",
                    checked: s.light_message_background,
                    onchange: move |v| ctx.edit(|s| s.light_message_background = v),
                }
            }
        }
        Section { title: "Density", description: "Density sets how much space messages take and which details show in the list.",
            div { class: "ax-density-options", role: "radiogroup", "aria-label": "Density", "data-ax-roving": "horizontal",
                for (d, label, text, lines) in [
                    (Density::Roomy, "Roomy", "Sender images, attachment previews and preview text", 3),
                    (Density::Cozy, "Cozy", "Preview text, with some spacing", 2),
                    (Density::Compact, "Compact", "Preview text, with minimal spacing", 2),
                ] {
                    button {
                        key: "{label}",
                        r#type: "button",
                        role: "radio",
                        class: "ax-density-tile ax-density-{label.to_lowercase()}",
                        class: if s.density == d { "ax-selected" },
                        "aria-checked": if s.density == d { "true" } else { "false" },
                        "data-ax-item": "true",
                        tabindex: if s.density == d { "0" } else { "-1" },
                        onclick: move |_| ctx.edit(|s| apply_density(s, d)),
                        span { class: "ax-density-preview", "aria-hidden": "true",
                            for i in 0..3 {
                                span { key: "{i}", class: "ax-density-line",
                                    if d == Density::Roomy { span { class: "ax-density-dot" } }
                                    span { class: "ax-density-bars",
                                        for l in 0..lines { span { key: "{l}", class: "ax-density-bar" } }
                                    }
                                }
                            }
                        }
                        span { class: "ax-choice-text",
                            span { class: "ax-semibold", "{label}" }
                            span { class: "ax-choice-desc", "{text}" }
                        }
                    }
                }
            }
        }
        Section { title: "Layout style",
            Switch {
                label: "Classic look",
                checked: s.classic_look,
                onchange: move |v| ctx.edit(|s| s.classic_look = v),
            }
            Note { text: "Shows panes edge to edge, separated by thin lines, instead of floating cards." }
        }
        Section { title: "Accessibility",
            Row { label: "High contrast", description: "Black-and-white colours with strong outlines.",
                Select {
                    value: tri_value(s.high_contrast),
                    options: vec![("system".into(), "Follow system".into()), ("on".into(), "On".into()), ("off".into(), "Off".into())],
                    aria_label: "High contrast",
                    onchange: move |v: String| ctx.edit(|s| s.high_contrast = tri_parse(&v)),
                }
            }
        }
    }
}

#[component]
pub fn NotificationsPage() -> Element {
    let ctx = use_settings();
    let s = ctx.get();
    rsx! {
        Section { title: "Notify me about",
            Switch { label: "Mail", checked: s.notify_mail, onchange: move |v| ctx.edit(|s| s.notify_mail = v) }
            Switch { label: "Calendar", checked: s.notify_calendar, onchange: move |v| ctx.edit(|s| s.notify_calendar = v) }
        }
        Section { title: "Mail", description: "New email notifications.",
            Checkbox {
                label: "Show desktop notifications",
                checked: s.notifications,
                disabled: !s.notify_mail,
                onchange: move |v| ctx.edit(|s| s.notifications = v),
            }
            Checkbox {
                label: "Play a sound",
                description: "Uses your desktop's \"new email\" sound.",
                checked: s.notification_sound,
                disabled: !s.notify_mail,
                onchange: move |v| ctx.edit(|s| s.notification_sound = v),
            }
            Checkbox {
                label: "Only for messages in Focused Inbox",
                checked: s.notify_focused_only,
                disabled: !s.notify_mail || !s.focused_inbox,
                title: if !s.focused_inbox { Some("Turn on Focused Inbox in Mail > Message list to use this".to_string()) } else { None },
                onchange: move |v| ctx.edit(|s| s.notify_focused_only = v),
            }
        }
        Section { title: "Calendar", description: "Event reminders.",
            Checkbox {
                label: "Send notifications when Axentra Outlook is closed",
                checked: false,
                disabled: true,
                title: "Reminders need Axentra Outlook to be running. A background service isn't available yet.",
                onchange: move |_| {},
            }
        }
        Section { title: "Batching",
            Checkbox {
                label: "Combine multiple new-mail notifications into one summary",
                checked: s.notify_batch,
                onchange: move |v| ctx.edit(|s| s.notify_batch = v),
            }
        }
    }
}

#[component]
pub fn AccessibilityPage() -> Element {
    let ctx = use_settings();
    let shell = use_shell();
    let s = ctx.get();
    let mode_value = |m: KeyboardMode| match m {
        KeyboardMode::Outlook => "outlook",
        KeyboardMode::Classic => "classic",
        KeyboardMode::Gmail => "gmail",
        KeyboardMode::Yahoo => "yahoo",
        KeyboardMode::Off => "off",
    };
    rsx! {
        Section { title: "Keyboard shortcuts", description: "Choose the shortcut set you're used to.",
            RadioGroup {
                label: "Keyboard shortcuts",
                value: mode_value(s.keyboard_mode),
                options: vec![
                    RadioOption::new("outlook", keymap::mode_label(KeyboardMode::Outlook)).description("Ctrl-based shortcuts, like Outlook for Windows (default)."),
                    RadioOption::new("classic", keymap::mode_label(KeyboardMode::Classic)).description("Adds classic Outlook keys such as Ctrl+Shift+M and F9."),
                    RadioOption::new("gmail", keymap::mode_label(KeyboardMode::Gmail)).description("Adds single-key shortcuts such as / to search."),
                    RadioOption::new("yahoo", keymap::mode_label(KeyboardMode::Yahoo)).description("Adds single-key shortcuts such as S to search."),
                    RadioOption::new("off", "Turn off keyboard shortcuts").description("Tab, arrows, Enter, Esc and F6 still work."),
                ],
                onchange: move |v: String| ctx.edit(|s| s.keyboard_mode = match v.as_str() {
                    "classic" => KeyboardMode::Classic,
                    "gmail" => KeyboardMode::Gmail,
                    "yahoo" => KeyboardMode::Yahoo,
                    "off" => KeyboardMode::Off,
                    _ => KeyboardMode::Outlook,
                }),
            }
            LinkButton {
                label: "View keyboard shortcuts",
                onclick: move |_| {
                    let mut sh = shell;
                    sh.shortcuts_open.set(true);
                },
            }
            Checkbox {
                label: "Show key tips when Alt is pressed",
                checked: s.show_key_tips,
                onchange: move |v| ctx.edit(|s| s.show_key_tips = v),
            }
        }
        Section { title: "Motion",
            Row { label: "Reduce motion", description: "Turns slides and fades into instant changes.",
                Select {
                    value: tri_value(s.reduce_motion),
                    options: vec![("system".into(), "Follow system".into()), ("on".into(), "On".into()), ("off".into(), "Off".into())],
                    aria_label: "Reduce motion",
                    onchange: move |v: String| ctx.edit(|s| s.reduce_motion = tri_parse(&v)),
                }
            }
        }
        Section { title: "Contrast",
            Row { label: "High contrast", description: "Also in Appearance.",
                Select {
                    value: tri_value(s.high_contrast),
                    options: vec![("system".into(), "Follow system".into()), ("on".into(), "On".into()), ("off".into(), "Off".into())],
                    aria_label: "High contrast",
                    onchange: move |v: String| ctx.edit(|s| s.high_contrast = tri_parse(&v)),
                }
            }
        }
        Section { title: "Screen readers",
            Checkbox {
                label: "Announce new messages with screen readers",
                checked: s.announce_new_messages,
                onchange: move |v| ctx.edit(|s| s.announce_new_messages = v),
            }
        }
    }
}

#[component]
pub fn SearchPage() -> Element {
    let ctx = use_settings();
    let app = use_app();
    let s = ctx.get();
    let mut history = use_signal(|| search::recent_searches().len());
    rsx! {
        Section { title: "Search results",
            Row { label: "Search scope", description: "Where searches look first.",
                Select {
                    value: format!("{:?}", s.search_scope),
                    options: vec![
                        ("AllFolders".into(), "All folders".into()),
                        ("CurrentFolder".into(), "Current folder".into()),
                        ("CurrentMailbox".into(), "Current mailbox".into()),
                    ],
                    aria_label: "Search scope",
                    onchange: move |v: String| ctx.edit(|s| s.search_scope = match v.as_str() {
                        "CurrentFolder" => SearchScope::CurrentFolder,
                        "CurrentMailbox" => SearchScope::CurrentMailbox,
                        _ => SearchScope::AllFolders,
                    }),
                }
            }
            Checkbox {
                label: "Include results from Deleted Items",
                checked: s.search_include_deleted,
                onchange: move |v| ctx.edit(|s| s.search_include_deleted = v),
            }
        }
        Section { title: "Search history", description: "Recent searches appear as suggestions in the search box.",
            div { class: "ax-row", style: "gap:12px",
                Button {
                    label: "Clear search history",
                    icon: "delete",
                    disabled: history() == 0,
                    title: if history() == 0 { "There's no search history" } else { "Delete your recent searches" },
                    onclick: move |_| {
                        if app.check(search::clear_recent_searches()).is_some() {
                            history.set(0);
                            app.toast("Search history cleared");
                        }
                    },
                }
                span { class: "ax-muted", if history() == 1 { "1 recent search" } else { "{history()} recent searches" } }
            }
        }
    }
}

#[component]
pub fn OfflinePage() -> Element {
    let ctx = use_settings();
    let s = ctx.get();
    let size = store().path().and_then(|p| std::fs::metadata(p).ok()).map(|m| m.len() as i64);
    rsx! {
        Section { title: "Offline",
            Switch {
                label: "Enable offline email, calendar, and people",
                checked: s.offline_enabled,
                onchange: move |v| ctx.edit(|s| s.offline_enabled = v),
            }
            Row { label: "Folders to save",
                Select {
                    value: format!("{:?}", s.offline_folders),
                    disabled: !s.offline_enabled,
                    options: vec![
                        ("Default".into(), "Default (Inbox, sent, drafts, archive, deleted and snoozed)".into()),
                        ("DefaultAndFavorites".into(), "Default and favorites".into()),
                        ("DefaultFavoritesAndRecent".into(), "Default, favorites, and recently used".into()),
                    ],
                    aria_label: "Folders to save",
                    onchange: move |v: String| ctx.edit(|s| s.offline_folders = match v.as_str() {
                        "Default" => OfflineFolders::Default,
                        "DefaultFavoritesAndRecent" => OfflineFolders::DefaultFavoritesAndRecent,
                        _ => OfflineFolders::DefaultAndFavorites,
                    }),
                }
            }
            Row { label: "Days of email to save",
                Select {
                    value: s.offline_days.to_string(),
                    disabled: !s.offline_enabled,
                    options: [7u32, 30, 90, 180, 365, 730].iter().map(|d| (d.to_string(), match d {
                        365 => "1 year".to_string(),
                        730 => "2 years".to_string(),
                        d => format!("{d} days"),
                    })).collect::<Vec<_>>(),
                    aria_label: "Days of email to save",
                    onchange: move |v: String| ctx.edit(|s| s.offline_days = v.parse().unwrap_or(30)),
                }
            }
            Switch {
                label: "Sync in the background when the app is closed",
                checked: s.background_sync,
                onchange: move |v| ctx.edit(|s| s.background_sync = v),
            }
        }
        Section { title: "Storage on this device",
            Row {
                label: "Saved data",
                description: match size {
                    Some(b) => format!("Using {} on this device.", crate::fmt::file_size(b)),
                    None => "This session keeps its data in memory.".to_string(),
                },
                Button {
                    label: "Clear offline data",
                    disabled: true,
                    title: "Local accounts keep all their data on this device, so it can't be cleared here.",
                }
            }
        }
    }
}

#[component]
pub fn PrivacyPage() -> Element {
    let ctx = use_settings();
    let s = ctx.get();
    rsx! {
        Section { title: "External images", description: "Pictures stored on other servers can tell the sender when you read a message.",
            RadioGroup {
                label: "External images",
                value: if s.block_external_images { "block" } else { "load" },
                options: vec![
                    RadioOption::new("block", "Don't load external images").description("Show them only when you choose to, message by message."),
                    RadioOption::new("load", "Always load external images"),
                ],
                onchange: move |v: String| ctx.edit(|s| s.block_external_images = v == "block"),
            }
        }
        Section { title: "Your data", description: "Axentra Outlook keeps your mail, calendar and contacts on this device and talks only to your email providers.",
            Note { text: "No usage data is collected." }
        }
    }
}

#[component]
pub fn DistributionGroupsPage() -> Element {
    rsx! {
        Unavailable {
            icon: "people_team",
            title: "No distribution groups",
            text: "Distribution groups you own or belong to appear here for Microsoft 365 work or school accounts.",
        }
    }
}

#[component]
pub fn AboutPage() -> Element {
    let app = use_app();
    let _ = app.accounts_rev();
    let accounts = store().accounts().map(|a| a.len()).unwrap_or(0);
    let engine = if sync().is_some() { "Running" } else { "Not running (preview)" };
    let db = store().path().map(|p| p.display().to_string()).unwrap_or_else(|| "In memory".into());
    rsx! {
        div { class: "ax-about",
            div { class: "ax-about-head",
                span { class: "ax-about-logo", "aria-hidden": "true", "A" }
                div {
                    div { class: "ax-subtitle-1", "{crate::APP_NAME}" }
                    div { class: "ax-muted", "Version {env!(\"CARGO_PKG_VERSION\")}" }
                }
            }
            p { "A mail, calendar, people and to-do app for Linux. Works with Outlook.com, Microsoft 365 and IMAP accounts." }
            p { class: "ax-muted ax-caption", "Not affiliated with or endorsed by Microsoft. Outlook and Microsoft 365 are trademarks of Microsoft Corporation." }
        }
        Section { title: "About",
            Row { label: "Version", span { class: "ax-set-value", "{env!(\"CARGO_PKG_VERSION\")}" } }
            Row { label: "Accounts", span { class: "ax-set-value", "{accounts}" } }
            Row { label: "Sync engine", span { class: "ax-set-value", "{engine}" } }
            Row { label: "Data file", span { class: "ax-set-value ax-selectable", "{db}" } }
        }
        Section { title: "Licenses",
            ul { class: "ax-licenses",
                li { "Fluent UI System Icons — MIT License, © Microsoft Corporation" }
                li { "Dioxus — MIT or Apache-2.0" }
                li { "SQLite (via rusqlite) — public domain / MIT" }
                li { "Fonts: Segoe UI or Selawik when installed on your system (not bundled)" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn density_presets_set_list_toggles() {
        let mut s = Settings::default();
        apply_density(&mut s, Density::Roomy);
        assert!(s.show_sender_images && s.show_attachment_previews && s.show_preview_text);
        apply_density(&mut s, Density::Compact);
        assert_eq!(s.density, Density::Compact);
        assert!(!s.show_sender_images && !s.show_attachment_previews && s.show_preview_text);
    }

    #[test]
    fn tri_state_round_trips() {
        for v in [None, Some(true), Some(false)] {
            assert_eq!(tri_parse(&tri_value(v)), v);
        }
    }
}
