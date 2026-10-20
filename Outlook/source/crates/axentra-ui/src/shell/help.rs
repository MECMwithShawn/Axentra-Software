//! Help pane (More > Help, F1, ribbon Help tab), What's new and Feedback.
//! All help text is Axentra's own wording.

use axentra_core::EmailAddress;
use dioxus::prelude::*;

use super::my_day::SidePaneFrame;
use super::{ShellCtx, use_shell};
use crate::kit::{Button, Dialog, EmptyState, Icon, SearchField, Variant, dom};
use crate::settings::PageId;
use crate::state::{AppState, ComposeRequest, use_app};

/// Where feedback goes (a compose form opens with it filled in).
pub const FEEDBACK_ADDRESS: &str = "feedback@axentra.example";

/// What a help topic's button does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HelpAction {
    Settings(PageId),
    Shortcuts,
    MyDay,
    ToggleRibbon,
    WhatsNew,
}

pub struct Topic {
    pub title: &'static str,
    pub steps: &'static [&'static str],
    pub action: Option<(&'static str, HelpAction)>,
}

pub const TOPICS: &[Topic] = &[
    Topic {
        title: "Use keyboard shortcuts",
        steps: &[
            "Press ? anywhere outside a text box to see every shortcut.",
            "Press F6 to move between the header, the app bar, the ribbon and each pane.",
            "Press Alt to show key tips on the ribbon, then type the letters shown.",
        ],
        action: Some(("Show keyboard shortcuts", HelpAction::Shortcuts)),
    },
    Topic {
        title: "Change the theme or turn on dark mode",
        steps: &["Open Settings > General > Appearance.", "Pick a colour theme, then Light, Dark or Use system settings.", "Select Save."],
        action: Some(("Open Appearance", HelpAction::Settings(PageId::Appearance))),
    },
    Topic {
        title: "Make the message list roomier or more compact",
        steps: &["Open Settings > General > Appearance.", "Under Density, choose Roomy, Cozy or Compact."],
        action: Some(("Open Appearance", HelpAction::Settings(PageId::Appearance))),
    },
    Topic {
        title: "Turn Focused Inbox on or off",
        steps: &["Open Settings > Mail > Message list.", "Select or clear Sort messages into Focused and Other.", "Select Save."],
        action: Some(("Open Message list settings", HelpAction::Settings(PageId::MessageList))),
    },
    Topic {
        title: "Move or hide the reading pane",
        steps: &["Open Settings > Mail > Reading pane.", "Under Reading pane position, choose Right, Bottom or Fill screen."],
        action: Some(("Open Reading pane settings", HelpAction::Settings(PageId::ReadingPane))),
    },
    Topic {
        title: "Create an email signature",
        steps: &["Open Settings > Accounts > Signatures.", "Type your signature and choose when it's added.", "Select Save."],
        action: Some(("Open Signatures", HelpAction::Settings(PageId::Signatures))),
    },
    Topic {
        title: "Give yourself time to undo a send",
        steps: &["Open Settings > Mail > Compose and reply.", "Under Undo send, choose how many seconds to wait."],
        action: Some(("Open Compose and reply", HelpAction::Settings(PageId::ComposeReply))),
    },
    Topic {
        title: "Organise with categories",
        steps: &["Open Settings > Accounts > Categories to create, rename or recolour categories.", "Use Categorize on the ribbon to apply them."],
        action: Some(("Open Categories", HelpAction::Settings(PageId::Categories))),
    },
    Topic {
        title: "Set your work hours and first day of the week",
        steps: &["Open Settings > Calendar > View.", "Choose your work days and hours."],
        action: Some(("Open Calendar view settings", HelpAction::Settings(PageId::CalendarView))),
    },
    Topic {
        title: "Add another email account",
        steps: &["Select your picture at the top right, then Add account.", "Or open Settings > Accounts > Email accounts."],
        action: Some(("Open Email accounts", HelpAction::Settings(PageId::EmailAccounts))),
    },
    Topic {
        title: "Plan your day with My Day",
        steps: &["Select My Day at the top right to see today's events and tasks side by side with your mail."],
        action: Some(("Open My Day", HelpAction::MyDay)),
    },
    Topic {
        title: "Switch between the simplified and classic ribbon",
        steps: &["Select the arrow at the right end of the ribbon to switch layouts.", "Right-click a ribbon tab to collapse the ribbon to its tabs."],
        action: Some(("Switch ribbon layout", HelpAction::ToggleRibbon)),
    },
    Topic {
        title: "Choose the buttons on the ribbon",
        steps: &[
            "In Mail, on the Home or View tab, select More options (…) > Customize.",
            "Select or clear buttons, drag them into the order you like, and add or rename groups.",
            "Choose how labels show, then select Save. Reset all changes puts the tab back the way it was.",
        ],
        action: None,
    },
    Topic {
        title: "Use high contrast or reduce motion",
        steps: &["Open Settings > General > Accessibility.", "Choose High contrast and Reduce motion, or follow your system settings."],
        action: Some(("Open Accessibility", HelpAction::Settings(PageId::Accessibility))),
    },
];

/// Topics whose title or text contains every word of `query`.
pub fn search_topics(query: &str) -> Vec<&'static Topic> {
    let words: Vec<String> = query.split_whitespace().map(|w| w.to_lowercase()).collect();
    TOPICS
        .iter()
        .filter(|t| {
            let hay = format!("{} {}", t.title, t.steps.join(" ")).to_lowercase();
            words.iter().all(|w| hay.contains(w.as_str()))
        })
        .collect()
}

pub fn run_help_action(mut app: AppState, mut shell: ShellCtx, action: HelpAction) {
    match action {
        HelpAction::Settings(page) => shell.open_settings(app, page),
        HelpAction::Shortcuts => shell.shortcuts_open.set(true),
        HelpAction::MyDay => {
            shell.pane.set(None);
            app.my_day_open.set(true);
        }
        HelpAction::ToggleRibbon => app.update_settings(|s| s.simplified_ribbon = !s.simplified_ribbon),
        HelpAction::WhatsNew => shell.whats_new_open.set(true),
    }
}

/// Open a new message to the feedback address.
pub fn send_feedback(app: AppState) {
    let mut req = ComposeRequest::to(vec![EmailAddress::new(format!("{} feedback", crate::APP_NAME), FEEDBACK_ADDRESS)]);
    req.subject = Some(format!("Feedback on {} {}", crate::APP_NAME, env!("CARGO_PKG_VERSION")));
    app.compose(req);
}

#[component]
pub fn HelpPane() -> Element {
    let app = use_app();
    let mut shell = use_shell();
    let mut query = use_signal(String::new);
    let mut open = use_signal(|| None::<usize>);
    let topics = search_topics(&query());
    let close = move |_| {
        shell.pane.set(None);
        dom::focus(".ax-header-end [aria-label='More options']");
    };
    rsx! {
        SidePaneFrame { title: "Help", close_label: "Close Help", class: "ax-help", on_close: close,
            div { class: "ax-help-search",
                SearchField { value: query(), placeholder: "Search help", autofocus: true, oninput: move |v| { query.set(v); open.set(None); } }
            }
            div { class: "ax-sidepanel-body",
                if topics.is_empty() {
                    EmptyState { icon: "search", title: "We didn't find anything", subtitle: "Try different words." }
                }
                div { class: "ax-help-topics",
                    for (i, t) in topics.iter().enumerate() {
                        div { key: "{t.title}", class: "ax-help-topic",
                            button {
                                r#type: "button",
                                class: "ax-help-topic-head",
                                "aria-expanded": if open() == Some(i) { "true" } else { "false" },
                                onclick: move |_| open.set(if open() == Some(i) { None } else { Some(i) }),
                                Icon { name: if open() == Some(i) { "chevron_down" } else { "chevron_right" }, size: 16 }
                                span { "{t.title}" }
                            }
                            if open() == Some(i) {
                                div { class: "ax-help-topic-body",
                                    ol {
                                        for s in t.steps.iter() { li { "{s}" } }
                                    }
                                    if let Some((label, action)) = t.action {
                                        Button { label, small: true, onclick: move |_| run_help_action(app, shell, action) }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            div { class: "ax-sidepanel-foot ax-help-links",
                button {
                    r#type: "button",
                    class: "ax-link-btn",
                    "aria-keyshortcuts": super::keymap::aria_for(super::keymap::GlobalCmd::Shortcuts, app.settings.read().keyboard_mode),
                    onclick: move |_| run_help_action(app, shell, HelpAction::Shortcuts),
                    "Keyboard shortcuts"
                }
                button { r#type: "button", class: "ax-link-btn", onclick: move |_| run_help_action(app, shell, HelpAction::WhatsNew), "What's new" }
                button { r#type: "button", class: "ax-link-btn", onclick: move |_| send_feedback(app), "Send feedback" }
                button { r#type: "button", class: "ax-link-btn", onclick: move |_| shell.open_settings(app, PageId::About), "About {crate::APP_NAME}" }
            }
        }
    }
}

/// Features of this version, in Axentra's words.
const WHATS_NEW: &[(&str, &str, &str)] = &[
    ("color", "Colour themes", "Pick Blue, Classic blue, Neutral, Teal, Purple, Pink, Red, Orange, Green or Graphite in Settings > General > Appearance."),
    ("calendar_checkmark", "My Day", "See today's events and tasks next to your mail, add tasks and join online meetings."),
    ("alert", "Notifications pane", "New mail, event reminders and task reminders in one place. Dismiss them one by one or all at once."),
    ("keyboard", "Keyboard first", "F6 moves between regions, Alt shows ribbon key tips, and ? lists every shortcut."),
    ("accessibility", "High contrast and reduced motion", "Turn them on in Settings > General > Accessibility, or follow your desktop's settings."),
];

#[component]
pub fn WhatsNewDialog() -> Element {
    let mut shell = use_shell();
    rsx! {
        Dialog {
            title: "What's new",
            width: 520,
            resize_key: "whats-new", min_width: 400, min_height: 300,
            on_close: move |_| shell.whats_new_open.set(false),
            actions: rsx! { Button { label: "Close", variant: Variant::Primary, onclick: move |_| shell.whats_new_open.set(false) } },
            div { class: "ax-whats-new",
                for (icon, title, text) in WHATS_NEW.iter() {
                    div { key: "{title}", class: "ax-whats-new-item",
                        span { class: "ax-whats-new-icon", Icon { name: *icon, size: 20 } }
                        div {
                            div { class: "ax-semibold", "{title}" }
                            div { class: "ax-muted", "{text}" }
                        }
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
    fn help_search_matches_all_words() {
        assert!(search_topics("").len() == TOPICS.len());
        let dark = search_topics("dark mode");
        assert_eq!(dark.len(), 1);
        assert!(dark[0].title.contains("dark mode"));
        assert!(search_topics("signature").iter().any(|t| t.title.contains("signature")));
        assert!(search_topics("zzzz").is_empty());
    }
}
