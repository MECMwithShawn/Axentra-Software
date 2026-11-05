//! Settings > Mail (docs/spec/shell.md §7.4). Pages for features the mail
//! module owns (Rules, Sweep, ...) use the same layout and say what's there.

use axentra_core::*;
use dioxus::prelude::*;

use super::controls::{LinkButton, Note, Row, Section, Unavailable, use_settings};
use super::pages::PageId;
use crate::kit::{Button, Checkbox, ConfirmDialog, Icon, IconButton, RadioGroup, RadioOption, Select, Slider, Switch, TextField, dom};
use crate::state::{UndoAction, store, use_app};

/// Undo-send stops: Off, 5 … 30 seconds (January 2026 maximum).
pub const UNDO_SEND_STOPS: [u32; 7] = [0, 5, 10, 15, 20, 25, 30];

/// Whether Focused Inbox can work for the configured accounts.
fn focused_supported() -> bool {
    let accounts = store().accounts().unwrap_or_default();
    accounts.is_empty() || accounts.iter().any(|a| matches!(a.kind, AccountKind::Microsoft | AccountKind::Local))
}

#[component]
pub fn MessageListPage() -> Element {
    let ctx = use_settings();
    let s = ctx.get();
    let supported = focused_supported();
    rsx! {
        Section { title: "Focused Inbox", description: "Focused Inbox separates your most important mail from the rest.",
            Checkbox {
                label: "Sort messages into Focused and Other",
                checked: s.focused_inbox && supported,
                disabled: !supported,
                title: if supported { None } else { Some("Focused Inbox works with Microsoft 365 and Outlook.com accounts only".to_string()) },
                onchange: move |v| ctx.edit(|s| s.focused_inbox = v),
            }
            Note { text: "Only the Inbox is affected." }
        }
        Section { title: "Message list format",
            RadioGroup {
                label: "Message list format",
                value: if s.subject_first { "subject" } else { "sender" },
                options: vec![RadioOption::new("sender", "Sender name first"), RadioOption::new("subject", "Subject first")],
                onchange: move |v: String| ctx.edit(|s| s.subject_first = v == "subject"),
            }
        }
        Section { title: "Text preview",
            Checkbox { label: "Show preview text", checked: s.show_preview_text, onchange: move |v| ctx.edit(|s| s.show_preview_text = v) }
            Row { label: "Preview length",
                Select {
                    value: s.preview_lines.to_string(),
                    disabled: !s.show_preview_text,
                    options: vec![("2".into(), "2 lines".into()), ("1".into(), "1 line".into()), ("0".into(), "No extra line".into())],
                    aria_label: "Preview length",
                    onchange: move |v: String| ctx.edit(|s| s.preview_lines = v.parse().unwrap_or(1)),
                }
            }
        }
        Section { title: "Sender image",
            Checkbox { label: "Show sender images", checked: s.show_sender_images, onchange: move |v| ctx.edit(|s| s.show_sender_images = v) }
        }
        Section { title: "Attachment previews",
            Checkbox { label: "Show attachment previews", checked: s.show_attachment_previews, onchange: move |v| ctx.edit(|s| s.show_attachment_previews = v) }
        }
        Section { title: "Density",
            LinkButton { label: "Change density in Appearance", onclick: move |_| ctx.open(PageId::Appearance) }
        }
    }
}

#[component]
pub fn ReadingPanePage() -> Element {
    let ctx = use_settings();
    let s = ctx.get();
    let organization = match (s.conversation_view, s.conversation_branches) {
        (false, _) => "none",
        (true, true) => "branches",
        (true, false) => "conversation",
    };
    rsx! {
        Section { title: "Message organization", description: "How do you want your message list organized?",
            RadioGroup {
                label: "Message organization",
                value: organization,
                options: vec![
                    RadioOption::new("conversation", "Group messages by conversation"),
                    RadioOption::new("branches", "Group messages by branches within conversations"),
                    RadioOption::new("none", "Do not group messages"),
                ],
                onchange: move |v: String| ctx.edit(|s| {
                    s.conversation_view = v != "none";
                    s.conversation_branches = v == "branches";
                }),
            }
        }
        Section { title: "Reading pane position",
            RadioGroup {
                label: "Reading pane position",
                value: match s.reading_pane { ReadingPanePosition::Right => "right", ReadingPanePosition::Bottom => "bottom", ReadingPanePosition::Hidden => "fill" },
                options: vec![
                    RadioOption::new("right", "Right"),
                    RadioOption::new("bottom", "Bottom"),
                    RadioOption::new("fill", "Fill screen").description("No reading pane: an opened message replaces the list."),
                ],
                onchange: move |v: String| ctx.edit(|s| s.reading_pane = match v.as_str() {
                    "bottom" => ReadingPanePosition::Bottom,
                    "fill" => ReadingPanePosition::Hidden,
                    _ => ReadingPanePosition::Right,
                }),
            }
        }
        Section { title: "After moving or deleting an item",
            RadioGroup {
                label: "After moving or deleting an item",
                value: match s.after_move { AfterMoveAction::OpenNext => "next", AfterMoveAction::OpenPrevious => "previous", AfterMoveAction::ReturnToList => "list" },
                options: vec![
                    RadioOption::new("next", "Open the next item"),
                    RadioOption::new("previous", "Open the previous item"),
                    RadioOption::new("list", "Return to the message list"),
                ],
                onchange: move |v: String| ctx.edit(|s| s.after_move = match v.as_str() {
                    "previous" => AfterMoveAction::OpenPrevious,
                    "list" => AfterMoveAction::ReturnToList,
                    _ => AfterMoveAction::OpenNext,
                }),
            }
        }
        Section { title: "Conversation contents", description: "What do you want to see in your reading pane?",
            RadioGroup {
                label: "Conversation contents",
                value: if s.conversation_selected_only { "selected" } else { "all" },
                disabled: !s.conversation_view,
                options: vec![
                    RadioOption::new("all", "Show all messages from the selected conversation"),
                    RadioOption::new("selected", "Show only the selected message"),
                ],
                onchange: move |v: String| ctx.edit(|s| s.conversation_selected_only = v == "selected"),
            }
        }
        Section { title: "Conversation order",
            RadioGroup {
                label: "Conversation order",
                value: if s.conversation_order == ConversationOrder::NewestOnTop { "top" } else { "bottom" },
                disabled: !s.conversation_view,
                options: vec![RadioOption::new("top", "Newest on top"), RadioOption::new("bottom", "Newest on bottom")],
                onchange: move |v: String| ctx.edit(|s| s.conversation_order = if v == "top" { ConversationOrder::NewestOnTop } else { ConversationOrder::NewestOnBottom }),
            }
        }
        Section { title: "Deleted items in conversations", description: "Do you want to see deleted items in your conversations?",
            RadioGroup {
                label: "Deleted items in conversations",
                value: if s.conversation_show_deleted { "show" } else { "hide" },
                disabled: !s.conversation_view,
                options: vec![RadioOption::new("show", "Show deleted items"), RadioOption::new("hide", "Hide deleted items")],
                onchange: move |v: String| ctx.edit(|s| s.conversation_show_deleted = v == "show"),
            }
        }
    }
}

#[component]
pub fn FolderPanePage() -> Element {
    let ctx = use_settings();
    let s = ctx.get();
    rsx! {
        Section { title: "Folder pane",
            Checkbox { label: "Show the folder pane", checked: !s.folder_pane_collapsed, onchange: move |v: bool| ctx.edit(|s| s.folder_pane_collapsed = !v) }
        }
        Section { title: "Folder order",
            RadioGroup {
                label: "Folder order",
                value: if s.folders_sorted_az { "az" } else { "custom" },
                options: vec![RadioOption::new("custom", "Custom folder order").description("Drag folders to arrange them."), RadioOption::new("az", "Sort folders A to Z")],
                onchange: move |v: String| ctx.edit(|s| s.folders_sorted_az = v == "az"),
            }
        }
        Section { title: "Favorites",
            Checkbox { label: "Show favorites", checked: s.show_favorites, onchange: move |v| ctx.edit(|s| s.show_favorites = v) }
        }
    }
}

#[component]
pub fn ComposeReplyPage() -> Element {
    let ctx = use_settings();
    let s = ctx.get();
    let stop = UNDO_SEND_STOPS.iter().position(|v| *v >= s.undo_send_secs).unwrap_or(UNDO_SEND_STOPS.len() - 1) as u32;
    let undo_text = if s.undo_send_secs == 0 { "Off".to_string() } else { format!("{} seconds", s.undo_send_secs) };
    let fonts: Vec<(String, String)> = ["Aptos", "Calibri", "Arial", "Georgia", "Segoe UI", "Selawik", "Noto Sans", "Times New Roman", "Verdana"]
        .iter()
        .map(|f| (f.to_string(), f.to_string()))
        .collect();
    rsx! {
        Section { title: "Message format", description: "The font new messages start with.",
            div { class: "ax-row", style: "gap:12px;flex-wrap:wrap",
                Select { label: "Font", value: s.default_font.clone(), options: fonts, onchange: move |v| ctx.edit(|s| s.default_font = v) }
                Select {
                    label: "Size",
                    value: s.default_font_size_pt.to_string(),
                    options: [8u32, 9, 10, 11, 12, 14, 16, 18, 20, 24].iter().map(|p| (p.to_string(), format!("{p} pt"))).collect::<Vec<_>>(),
                    onchange: move |v: String| ctx.edit(|s| s.default_font_size_pt = v.parse().unwrap_or(12)),
                }
            }
            p {
                class: "ax-set-preview",
                style: "font-family:'{s.default_font}', var(--ax-font);font-size:{s.default_font_size_pt}pt",
                "This is what your message text will look like."
            }
        }
        Section { title: "Undo send", description: "Wait before sending, so you can take a message back.",
            Slider {
                value: stop,
                min: 0,
                max: (UNDO_SEND_STOPS.len() - 1) as u32,
                label: "Undo send delay",
                value_text: undo_text,
                onchange: move |i: u32| ctx.edit(|s| s.undo_send_secs = UNDO_SEND_STOPS[(i as usize).min(UNDO_SEND_STOPS.len() - 1)]),
            }
        }
        Section { title: "Email signature",
            LinkButton { label: "Edit your signature in Accounts > Signatures", onclick: move |_| ctx.open(PageId::Signatures) }
        }
    }
}

#[component]
pub fn AttachmentsPage() -> Element {
    let ctx = use_settings();
    let s = ctx.get();
    rsx! {
        Section { title: "Attachment previews", description: "Thumbnails of attached files in the message list.",
            Checkbox { label: "Show attachment previews", checked: s.show_attachment_previews, onchange: move |v| ctx.edit(|s| s.show_attachment_previews = v) }
        }
        Section { title: "Attachment reminder",
            Checkbox {
                label: "Warn me when I send a message that may be missing an attachment",
                checked: s.attachment_reminder,
                onchange: move |v| ctx.edit(|s| s.attachment_reminder = v),
            }
        }
        Section { title: "Opening attachments",
            Note { text: "Attachments open in the app your desktop uses for that file type." }
        }
    }
}

#[component]
pub fn MessageHandlingPage() -> Element {
    let ctx = use_settings();
    let s = ctx.get();
    let value = match s.mark_as_read {
        MarkAsRead::OnSelect => "open",
        MarkAsRead::AfterDelay => "delay",
        MarkAsRead::Never => "never",
    };
    rsx! {
        Section { title: "Mark as read",
            RadioGroup {
                label: "Mark as read",
                value,
                options: vec![
                    RadioOption::new("open", "Mark as read when the item is opened in the reading pane"),
                    RadioOption::new("delay", "Mark as read after the item has been in the reading pane for a few seconds"),
                    RadioOption::new("never", "Don't automatically mark items as read"),
                ],
                onchange: move |v: String| ctx.edit(|s| s.mark_as_read = match v.as_str() {
                    "delay" => MarkAsRead::AfterDelay,
                    "never" => MarkAsRead::Never,
                    _ => MarkAsRead::OnSelect,
                }),
            }
            if s.mark_as_read == MarkAsRead::AfterDelay {
                Row { label: "Mark as read after", description: "Seconds in the reading pane (0–60).",
                    TextField {
                        value: s.mark_as_read_delay_secs.to_string(),
                        input_type: "number",
                        min: "0",
                        max: "60",
                        oninput: move |v: String| {
                            if let Ok(n) = v.trim().parse::<u32>() {
                                ctx.edit(|s| s.mark_as_read_delay_secs = n.min(60));
                            }
                        },
                    }
                }
            }
        }
        Section { title: "Message options",
            Switch {
                label: "Empty Deleted Items when Axentra Outlook closes",
                checked: s.empty_deleted_on_exit,
                onchange: move |v| ctx.edit(|s| s.empty_deleted_on_exit = v),
            }
        }
    }
}

/// Whether `s` can go on a sender list: an address (`name@example.com`) or
/// a bare domain (`example.com`).
pub fn valid_sender(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() || s.chars().any(char::is_whitespace) {
        return false;
    }
    let domain = match s.split_once('@') {
        Some((local, domain)) => {
            if local.is_empty() || domain.contains('@') {
                return false;
            }
            domain
        }
        None => s,
    };
    domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.')
}

/// One saved Sweep rule in words.
pub fn sweep_rule_text(mode: &str, sender: &str, folder: &str, target: &str) -> String {
    match mode {
        "keep_latest" => format!("Keep only the latest message from {sender} in {folder}; move the rest to {target}"),
        "older_than_10_days" => format!("Move messages from {sender} in {folder} older than 10 days to {target}"),
        _ => format!("Move all messages from {sender} in {folder}, now and in the future, to {target}"),
    }
}

/// Settings > Mail > Junk email: block and allow rules (per account, or all
/// accounts), the separate picture permission, and the reports Axentra
/// learns from. Changes apply right away.
#[component]
pub fn JunkPage() -> Element {
    let app = use_app();
    let _ = app.mail_rev();
    let s = store();
    let accounts = s.accounts().unwrap_or_default();
    // Where new rules typed here apply: an account, or "all" on purpose.
    let mut scope = use_signal(|| accounts.first().map(|a| a.id.to_string()).unwrap_or_else(|| "all".into()));
    let scope_id: Option<AccountId> = scope().parse().ok();
    let scope_name = |a: Option<AccountId>| match a {
        None => "all accounts".to_string(),
        Some(id) => accounts.iter().find(|x| x.id == id).map(|x| x.email.clone()).unwrap_or_else(|| "a removed account".into()),
    };
    let rules = s.sender_rules().unwrap_or_default();
    let list = |action: junk::RuleAction| -> (Vec<(String, String, String)>, Vec<String>) {
        let mut items = Vec::new();
        let mut taken = Vec::new();
        for r in rules.iter().filter(|r| r.action() == action) {
            let who = if r.name.trim().is_empty() || r.domain { r.pattern.clone() } else { format!("{} <{}>", r.name, r.pattern) };
            let kind = match (r.domain, r.exact_domain) {
                (true, false) => " (whole domain)",
                (true, true) => " (this domain only)",
                _ => "",
            };
            let whose = match (r.account_id, r.source) {
                (None, junk::RuleSource::Legacy) => "all accounts, from an earlier version".to_string(),
                (a, _) => scope_name(a),
            };
            items.push((r.id.to_string(), r.pattern.clone(), format!("{who}{kind} \u{2014} {whose}")));
            if r.account_id == scope_id {
                taken.push(r.pattern.clone());
            }
        }
        (items, taken)
    };
    let (blocked, blocked_taken) = list(junk::RuleAction::Block);
    let (allowed, allowed_taken) = list(junk::RuleAction::Allow);
    let images: Vec<(String, String, String)> = s.image_senders().unwrap_or_default().into_iter().map(|a| (a.clone(), a.clone(), a)).collect();
    let image_taken: Vec<String> = images.iter().map(|i| i.1.clone()).collect();
    let mut scope_options: Vec<(String, String)> = accounts.iter().map(|a| (a.id.to_string(), a.email.clone())).collect();
    scope_options.push(("all".into(), "All accounts".into()));
    let remove_rule = move |key: String| {
        let Ok(id) = key.parse::<i64>() else { return };
        if let Some(Some(rule)) = app.check(store().remove_sender_rule(id)) {
            app.bump_mail();
            app.toast_with_undo(format!("Rule for {} removed", rule.pattern), UndoAction::RestoreRule(rule));
        }
    };
    rsx! {
        Section {
            title: "Filter junk",
            description: "Moves new mail that's clearly junk to Junk Email, also on the server. Only mail from senders you've never dealt with: not your contacts, people you've written to, or senders you marked Not junk. It looks for phishing signs, and for the attachment lure: a subject of only codes and numbers, marked urgent, with a document and next to no text.",
            Checkbox {
                label: "Move clearly junk mail to Junk Email",
                checked: app.settings.read().junk_filter,
                onchange: move |v| crate::mail::actions::set_junk_filter(app, v),
            }
            Note { text: "Moved by mistake? Open it in Junk Email and choose \u{201c}It's not junk\u{201d}. Mail from that sender then stays in your Inbox." }
        }
        ShadowReviewSection {}
        Section { title: "Where new rules apply", description: "Rules you add on this page apply to the account you choose here. Rules made from a message apply to that message's account.",
            Select {
                value: scope(),
                options: scope_options,
                aria_label: "Account for new rules",
                onchange: move |v: String| scope.set(v),
            }
        }
        SenderList {
            title: "Blocked senders and domains",
            description: "New mail from these senders goes to Junk Email, also on the server. A domain covers its subdomains, except domains carried over from an earlier version, which cover that domain only. Phishing warnings still show.",
            empty: "You haven't blocked anyone. Block a sender from a message (Home > More options > Block).",
            add_label: format!("Block a sender or domain in {}", scope_name(scope_id)),
            items: blocked,
            taken: blocked_taken,
            on_add: move |addr: String| {
                if let Some(change) = app.check(store().set_sender_rule(scope_id, &addr, "", junk::RuleAction::Block)) {
                    app.bump_mail();
                    app.toast_with_undo(format!("Blocked {addr}"), UndoAction::Junk { events: Vec::new(), rules: vec![change], moves: Vec::new() });
                }
            },
            on_remove: remove_rule,
        }
        SenderList {
            title: "Never block",
            description: "Block rules for a domain don't apply to these senders, and Axentra doesn't suggest Junk for them. They still get phishing warnings, and pictures don't load automatically.",
            empty: "No one yet. Choose \u{201c}Never block sender\u{201d} on a message to add one.",
            add_label: format!("Never block a sender or domain in {}", scope_name(scope_id)),
            items: allowed,
            taken: allowed_taken,
            on_add: move |addr: String| {
                if let Some(change) = app.check(store().set_sender_rule(scope_id, &addr, "", junk::RuleAction::Allow)) {
                    app.bump_mail();
                    app.toast_with_undo(format!("{addr} will never be blocked"), UndoAction::Junk { events: Vec::new(), rules: vec![change], moves: Vec::new() });
                }
            },
            on_remove: remove_rule,
        }
        SenderList {
            title: "Load pictures automatically",
            description: "Pictures and other content from the internet load in mail from these senders and domains, in every account. This doesn't change how their mail is filtered.",
            empty: "No senders yet. Choose \u{201c}Always show content from\u{2026}\u{201d} on a message to add one.",
            add_label: "Add a sender or domain",
            items: images,
            taken: image_taken,
            on_add: move |addr: String| {
                if app.check(store().allow_images_from(&addr)).is_some() {
                    app.bump_mail();
                    dom::announce(&format!("Pictures from {addr} will load"), false);
                }
            },
            on_remove: move |addr: String| {
                if app.check(store().remove_image_sender(&addr)).is_some() {
                    app.bump_mail();
                    app.toast(format!("Pictures from {addr} won't load automatically"));
                }
            },
        }
        JunkReports {}
    }
}

/// What Axentra learned from your reports: the switch for suggestions, the
/// model in use and how it was checked, Roll back, and Forget.
#[component]
fn JunkReports() -> Element {
    let app = use_app();
    let _ = app.mail_rev();
    let s = store();
    let history = s.label_history(25).unwrap_or_default();
    let status = s.learn_status().unwrap_or_default();
    let mut confirm = use_signal(|| false);
    let model_text = match &status.active {
        Some(m) if m.manifest.kind == "personal" => format!(
            "In use: a model learned from {} junk and {} not-junk reports (version {}). It passed every check before it was used.",
            m.manifest.examples_unwanted, m.manifest.examples_wanted, m.digest
        ),
        Some(m) => format!("In use: the model that came with Axentra (version {}).", m.digest),
        None => "In use: rules and warning signs only. Axentra suggests junk once it has learned from enough of your reports and the result passes its checks.".to_string(),
    };
    let personal = status.active.as_ref().is_some_and(|m| m.manifest.kind == "personal");
    rsx! {
        Section {
            title: "Learning from your reports",
            description: "Report junk, Report phishing and Not junk are the only things Axentra learns from. Learning happens on this device; your reports aren't sent anywhere and aren't included in backups. Suggestions never move mail.",
            Checkbox {
                label: "Suggest when a message looks like junk",
                checked: app.settings.read().junk_suggestions,
                onchange: move |v| crate::mail::actions::set_junk_suggestions(app, v),
            }
            Note { text: "Phishing and other warning signs are always shown." }
            p { class: "ax-set-desc", "{model_text}" }
            p { class: "ax-set-desc", "Your reports: {status.unwanted} junk or phishing, {status.wanted} not junk." }
            if !status.last.is_empty() {
                p { class: "ax-set-desc", "Last update: {status.last}" }
            }
            div { class: "ax-row", style: "gap:8px;margin-top:8px;flex-wrap:wrap",
                if personal {
                    Button {
                        label: "Roll back",
                        icon: "arrow_undo",
                        title: "Go back to the version before this one. Your reports and rules stay.",
                        onclick: move |_| {
                            if app.check(store().roll_back_model()).is_some() {
                                app.bump_mail();
                                app.toast("Rolled back to the previous version");
                            }
                        },
                    }
                }
                if confirm() {
                    Button {
                        label: "Forget all reports",
                        variant: crate::kit::Variant::Primary,
                        onclick: move |_| {
                            confirm.set(false);
                            if app.check(store().forget_learning()).is_some() {
                                app.bump_mail();
                                app.toast("Axentra forgot your junk reports and what it learned. Your block and never-block rules stay.");
                            }
                        },
                    }
                    Button { label: "Cancel", onclick: move |_| confirm.set(false) }
                } else {
                    Button { label: "Forget learned data\u{2026}", icon: "delete", onclick: move |_| confirm.set(true) }
                }
            }
        }
        Section { title: "Recent reports", description: "Undo a report from the message's notification, or report it again the other way.",
            if history.is_empty() {
                Note { text: "No reports yet." }
            } else {
                div { class: "ax-cat-list", role: "list", "aria-label": "Recent reports",
                    for e in history.into_iter() {
                        div { key: "{e.id}", class: "ax-cat-row", role: "listitem",
                            Icon { name: match e.label { junk::Label::Spam => "mail_prohibited", junk::Label::Phishing => "shield_error", junk::Label::NotSpam => "mail_inbox" }, size: 16 }
                            span { class: "ax-grow ax-ellipsis", title: "{e.subject}",
                                {match e.label { junk::Label::Spam => "Junk", junk::Label::Phishing => "Phishing", junk::Label::NotSpam => "Not junk" }}
                                ": {e.subject} \u{2014} {e.sender}"
                                if e.undone { " (undone)" }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The line under a sender box: what to type, or, once Enter was refused,
/// why (marked invalid), or that the entry is listed already.
fn sender_hint(text: &str, ok: bool, dup: bool, rejected: bool) -> (String, bool) {
    if !text.is_empty() && ok && dup {
        ("That's already on the list.".into(), rejected)
    } else if rejected && !text.is_empty() && !ok {
        (format!("\u{201c}{text}\u{201d} isn't an email address or a domain."), true)
    } else {
        ("Enter an email address, like name@example.com, or a domain, like example.com.".into(), false)
    }
}

/// A sender list with an add field and a remove button per row.
#[component]
fn SenderList(
    #[props(into)] title: String,
    #[props(into)] description: String,
    #[props(into)] empty: String,
    #[props(into)] add_label: String,
    /// (key for removal, pattern, shown text)
    items: Vec<(String, String, String)>,
    /// Patterns already listed where a new entry would go.
    taken: Vec<String>,
    on_add: EventHandler<String>,
    on_remove: EventHandler<String>,
) -> Element {
    let mut draft = use_signal(String::new);
    // Enter found the entry invalid (or listed already); until it is edited.
    let mut rejected = use_signal(|| false);
    let text = draft().trim().to_lowercase();
    let ok = valid_sender(&text);
    let dup = taken.iter().any(|a| a.eq_ignore_ascii_case(text.trim_start_matches('@')));
    let (hint, invalid) = sender_hint(&text, ok, dup, rejected());
    let hint_id = crate::kit::dom::use_unique_id("ax-sender-hint");
    // True when the address was added (the page then empties the box).
    let mut add = move || -> bool {
        let text = draft().trim().to_lowercase();
        if valid_sender(&text) && !dup {
            on_add.call(text);
            draft.set(String::new());
            return true;
        }
        false
    };
    rsx! {
        Section { title: title.clone(), description,
            // The hint is always there, under the row, so typing the first
            // character (or a hint changing) doesn't move the box, the Add
            // button or the next section.
            div { class: "ax-sender-add-wrap",
                div { class: "ax-row ax-sender-add", style: "gap:8px;align-items:flex-end",
                    TextField {
                        label: add_label.clone(),
                        placeholder: "name@example.com or example.com",
                        value: draft(),
                        invalid,
                        describedby: hint_id.clone(),
                        enter_clears: true,
                        oninput: move |v| {
                            draft.set(v);
                            rejected.set(false);
                        },
                        onkeydown: move |e: KeyboardEvent| {
                            // Taken only when added: an invalid entry stays for a
                            // fix, marked, with the reason under it.
                            if e.key() == Key::Enter {
                                if add() {
                                    e.prevent_default();
                                } else if !draft.peek().trim().is_empty() {
                                    rejected.set(true);
                                }
                            }
                        },
                    }
                    Button {
                        label: "Add",
                        icon: "add",
                        disabled: !ok || dup,
                        title: if ok && !dup { "Add to the list" } else { "Type an address or domain first" },
                        onclick: move |_| {
                            add();
                        },
                    }
                }
                span { id: "{hint_id}", class: "ax-field-hint ax-sender-hint", class: if invalid { "ax-danger" }, "aria-live": "polite", "{hint}" }
            }
            if items.is_empty() {
                Note { text: empty }
            } else {
                div { class: "ax-cat-list", role: "list", "aria-label": "{title}",
                    for (key, addr, label) in items.iter().cloned() {
                        div { key: "{key}", class: "ax-cat-row", role: "listitem",
                            span { class: "ax-grow ax-ellipsis ax-selectable", title: "{label}", "{label}" }
                            IconButton {
                                icon: "delete",
                                title: "Remove {addr}",
                                small: true,
                                onclick: move |_| on_remove.call(key.clone()),
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Settings > Mail > Sweep: the standing Sweep rules (they run once a day).
#[component]
pub fn SweepPage() -> Element {
    let app = use_app();
    let _ = app.mail_rev();
    let s = store();
    let name_of = |id: FolderId| s.folder(id).ok().flatten().map(|f| f.name).unwrap_or_else(|| "a deleted folder".into());
    let rules: Vec<(i64, String)> =
        s.sweep_rules().unwrap_or_default().into_iter().map(|(id, folder, sender, mode, target)| (id, sweep_rule_text(&mode, &sender, &name_of(folder), &name_of(target)))).collect();
    rsx! {
        Section { title: "Sweep rules", description: "Rules you set up from the Sweep button. Standing Sweep rules run once a day.",
            if rules.is_empty() {
                Unavailable { icon: "broom", title: "No Sweep rules", text: "Choose Sweep on a message to set one up." }
            } else {
                div { class: "ax-cat-list ax-sweep-list", role: "list", "aria-label": "Sweep rules",
                    for (id, text) in rules.into_iter() {
                        div { key: "{id}", class: "ax-cat-row", role: "listitem",
                            Icon { name: "broom", size: 16 }
                            span { class: "ax-grow", "{text}" }
                            IconButton {
                                icon: "delete",
                                title: "Delete this rule",
                                small: true,
                                onclick: move |_| {
                                    if app.check(store().delete_sweep_rule(id)).is_some() {
                                        app.bump_mail();
                                        app.toast("Sweep rule deleted");
                                    }
                                },
                            }
                        }
                    }
                }
            }
        }
    }
}

/// "Archived 12 messages older than 30 days" (the auto-archive toast).
pub fn auto_archive_text(moved: usize, days: u32, by_topic: bool) -> String {
    let what = if moved == 1 { "1 message".to_string() } else { format!("{moved} messages") };
    let sorted = if by_topic { ", sorted by topic" } else { "" };
    format!("Archived {what} older than {}{sorted}", age_text(days))
}

/// Run auto-archive now, with Undo: daily, when turned on, or "Archive
/// now". Off the UI thread (sorting by topic may first work out the topics
/// of thousands of messages), and one run at a time. `manual` says when
/// there was nothing to archive.
pub async fn run_auto_archive(app: crate::state::AppState, manual: bool) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static RUNNING: AtomicBool = AtomicBool::new(false);
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    let s = app.settings.peek().clone();
    let (days, unread, by_topic) = (s.auto_archive_age(), s.auto_archive_unread, s.auto_archive_by_topic);
    let now = chrono::Utc::now();
    // Once a day even when it fails: not again every 30 seconds.
    app.update_settings(|s| s.auto_archive_last_run = Some(now));
    let result = tokio::task::spawn_blocking(move || store().auto_archive(days, unread, by_topic, now))
        .await
        .unwrap_or_else(|_| Err(Error::Invalid("auto-archive stopped unexpectedly".into())));
    RUNNING.store(false, Ordering::SeqCst);
    match result {
        Ok(records) if records.is_empty() => {
            if manual {
                app.toast("Nothing to archive");
            }
        }
        Ok(records) => {
            app.bump_mail();
            app.toast_with_undo(auto_archive_text(records.len(), days, by_topic), UndoAction::Moves(records));
        }
        Err(e) if manual => {
            app.error(e.to_string());
        }
        Err(e) => tracing::warn!("auto-archive: {e}"),
    }
}

/// "30 days", "1 year"…
fn age_text(days: u32) -> String {
    match days {
        365 => "1 year".into(),
        1 => "1 day".into(),
        d => format!("{d} days"),
    }
}

/// Auto-archive: Inbox mail older than the chosen age moves to Archive
/// once a day. Acts at once (no Save): turning it on archives what is
/// already old, with Undo.
#[component]
pub fn AutoArchivePage() -> Element {
    let app = use_app();
    let _ = app.mail_rev();
    let s = app.settings.read().clone();
    let days = s.auto_archive_age();
    let now = chrono::Utc::now();
    let waiting = store().auto_archive_candidates(days, s.auto_archive_unread, now).map(|v| v.len()).unwrap_or(0);
    let has_archive = store()
        .accounts()
        .unwrap_or_default()
        .iter()
        .any(|a| store().folder_by_role(a.id, FolderRole::Archive).ok().flatten().is_some());
    let mut busy = use_signal(|| false);
    let mut run = move || {
        busy.set(true);
        spawn(async move {
            run_auto_archive(app, true).await;
            busy.set(false);
        });
    };
    let mut options: Vec<(String, String)> = AUTO_ARCHIVE_DAYS.iter().map(|d| (d.to_string(), age_text(*d))).collect();
    if !AUTO_ARCHIVE_DAYS.contains(&days) {
        options.push((days.to_string(), age_text(days)));
    }
    let preview = match waiting {
        0 => format!("No messages in your Inbox are older than {} right now.", age_text(days)),
        1 => format!("1 message in your Inbox is older than {}.", age_text(days)),
        n => format!("{n} messages in your Inbox are older than {}.", age_text(days)),
    };
    let last = s.auto_archive_last_run.map(|t| t.with_timezone(&chrono::Local).format("%b %-d at %-I:%M %p").to_string());
    rsx! {
        Section {
            title: "Archive older mail",
            description: "Keep your Inbox to recent mail. Once a day, older messages move to each account's Archive folder, here and on the server. Pinned, flagged and snoozed messages stay, and so does any conversation with a message newer than that.",
            if !has_archive {
                Note { text: "None of your accounts has an Archive folder, so there's nowhere to archive to." }
            }
            Switch {
                label: "Archive older Inbox mail automatically",
                checked: s.auto_archive,
                disabled: !has_archive,
                onchange: move |on: bool| {
                    app.update_settings(|s| s.auto_archive = on);
                    if on {
                        run();
                    }
                },
            }
            Row { label: "Archive messages older than",
                Select {
                    value: days.to_string(),
                    options,
                    aria_label: "Archive messages older than",
                    onchange: move |v: String| {
                        if let Ok(d) = v.parse::<u32>() {
                            app.update_settings(|s| s.auto_archive_days = d);
                        }
                    },
                }
            }
            Checkbox {
                label: "Include unread messages",
                checked: s.auto_archive_unread,
                onchange: move |v: bool| app.update_settings(|s| s.auto_archive_unread = v),
            }
            Checkbox {
                label: "Sort into topic folders",
                description: "Archived mail goes into a folder for its topic inside Archive: People, Orders & shipping, Receipts & bills, Finance, Travel and so on. A folder is made the first time it's needed. Mail that fits no topic stays in Archive itself.",
                checked: s.auto_archive_by_topic,
                onchange: move |v: bool| app.update_settings(|s| s.auto_archive_by_topic = v),
            }
            Row { label: "Archive now", description: "{preview}",
                Button {
                    label: if busy() { "Archiving…" } else { "Archive now" },
                    icon: "archive",
                    disabled: waiting == 0 || busy(),
                    onclick: move |_| run(),
                }
            }
            if s.auto_archive {
                if let Some(last) = last {
                    Note { text: "Last run {last}. It runs again a day later while Axentra Outlook is open." }
                }
            }
        }
    }
}

/// "1 week", "30 days", "1 year" (how long delivered packages stay).
fn keep_text(days: u32) -> String {
    match days {
        7 => "1 week".into(),
        d => age_text(d),
    }
}

/// Settings > Mail > Packages: order tracking on this device. Acts at once
/// (no Save); switching it off forgets what was found.
#[component]
pub fn PackagesPage() -> Element {
    let app = use_app();
    let _ = app.mail_rev();
    let st = app.settings.read().clone();
    let s = store();
    let ignores = s.order_sender_ignores().unwrap_or_default();
    let hidden = s.hidden_order_count().unwrap_or(0);
    let mut confirm_off = use_signal(|| false);
    let items: Vec<(String, String, String)> = ignores
        .iter()
        .map(|i| (i.pattern.clone(), i.pattern.clone(), if i.is_domain { format!("{} (whole domain)", i.pattern) } else { i.pattern.clone() }))
        .collect();
    let taken: Vec<String> = ignores.iter().map(|i| i.pattern.clone()).collect();
    let keep_options: Vec<(String, String)> = orders::KEEP_DAYS.iter().map(|d| (d.to_string(), keep_text(*d))).collect();
    let hidden_text = format!("Hidden orders: {hidden}");
    rsx! {
        Section {
            title: "Package tracking",
            description: "Order confirmations and shipping emails are listed in Mail > Packages, with where each order is.",
            Switch {
                label: "Find orders and deliveries in my mail",
                checked: st.order_tracking,
                onchange: move |on: bool| {
                    if on {
                        app.update_settings(|s| s.order_tracking = true);
                        app.bump_mail();
                    } else {
                        confirm_off.set(true);
                    }
                },
            }
            Switch {
                label: "Show package info at the top of messages",
                checked: st.order_banner,
                disabled: !st.order_tracking,
                onchange: move |on: bool| app.update_settings(|s| s.order_banner = on),
            }
            Note {
                icon: "shield",
                text: "Orders are found on this device. Nothing is sent to stores or carriers. Track opens the carrier's website only when you click it.",
            }
        }
        Section { title: "Delivered packages", description: "Delivered, cancelled and returned orders stay in Packages for a while, then leave the list.",
            Row { label: "Keep delivered packages for",
                Select {
                    value: st.orders_keep_days().to_string(),
                    options: keep_options,
                    aria_label: "Keep delivered packages for",
                    onchange: move |v: String| {
                        if let Ok(d) = v.parse::<u32>() {
                            app.update_settings(|s| s.orders_keep_delivered_days = d);
                        }
                    },
                }
            }
        }
        SenderList {
            title: "Senders not tracked",
            description: "Orders from these senders and domains aren't added to Packages. A domain covers its subdomains.",
            empty: "No one yet. Choose \u{201c}Don't track orders from\u{2026}\u{201d} on an order to add one.",
            add_label: "Don't track orders from",
            items,
            taken,
            on_add: move |pattern: String| {
                if app.check(store().ignore_order_sender(&pattern)).is_some() {
                    app.bump_mail();
                    app.toast_with_undo(format!("Orders from {pattern} won't be tracked"), UndoAction::OrderIgnore(pattern));
                }
            },
            on_remove: move |pattern: String| {
                if app.check(store().remove_order_sender_ignore(&pattern)).is_some() {
                    app.bump_mail();
                    app.toast(format!("Orders from {pattern} will be tracked again"));
                }
            },
        }
        Section { title: "Hidden orders", description: "Orders you removed from Packages stay hidden, also when new mail about them comes in.",
            Row { label: hidden_text,
                Button {
                    label: "Show hidden orders again",
                    disabled: hidden == 0,
                    title: if hidden == 0 { "No orders are hidden" } else { "Put every hidden order back in Packages" },
                    onclick: move |_| {
                        if let Some(n) = app.check(store().unhide_all_orders()) {
                            app.bump_mail();
                            app.toast(if n == 1 { "1 order is back in Packages".to_string() } else { format!("{n} orders are back in Packages") });
                        }
                    },
                }
            }
            LinkButton {
                label: "Look through my mail again",
                disabled: !st.order_tracking,
                title: if st.order_tracking { "Find orders again from the mail on this device. Your changes and senders not tracked stay." } else { "Turn on package tracking first" },
                onclick: move |_| {
                    if app.check(store().clear_orders()).is_some() {
                        app.bump_mail();
                        app.toast("Looking through your mail again");
                    }
                },
            }
        }
        if confirm_off() {
            ConfirmDialog {
                title: "Turn off package tracking?",
                message: "Packages found so far are removed from this device. Your mail isn't changed.",
                confirm_label: "Turn off",
                on_cancel: move |_| confirm_off.set(false),
                on_confirm: move |_| {
                    confirm_off.set(false);
                    app.update_settings(|s| s.order_tracking = false);
                    app.check(store().clear_orders());
                    app.bump_mail();
                },
            }
        }
    }
}

/// A topic correction as listed: its address or domain, and the account
/// it is for (none: all accounts), so an account's correction and an
/// all-accounts one for the same sender can be told apart.
fn correction_text(pattern: &str, is_domain: bool, account: Option<&str>) -> String {
    let target = if is_domain { format!("{pattern} (whole domain)") } else { pattern.to_string() };
    match account {
        Some(email) => format!("{target} \u{b7} {email}"),
        None => target,
    }
}

/// A topic correction's target: a domain, or an address.
fn topic_target(pattern: &str, is_domain: bool) -> axentra_core::store::TopicTarget {
    if is_domain {
        axentra_core::store::TopicTarget::Domain(pattern.to_string())
    } else {
        axentra_core::store::TopicTarget::Sender(pattern.to_string())
    }
}

/// Settings > Mail > Topics: sorting on this device, which topics the
/// folder pane shows, and the user's corrections. Acts at once (no Save).
#[component]
pub fn TopicsPage() -> Element {
    let app = use_app();
    let _ = app.mail_rev();
    let st = app.settings.read().clone();
    let s = store();
    let rules = s.topic_rules().unwrap_or_default();
    let accounts = s.accounts().unwrap_or_default();
    let pending = if st.topics_view { s.topics_pending_count(10_000).unwrap_or(0) } else { 0 };
    let mut confirm_off = use_signal(|| false);
    let mut confirm_forget = use_signal(|| false);
    let mut draft = use_signal(String::new);
    let mut draft_topic = use_signal(|| topics::Topic::Newsletters.id().to_string());
    let topic_options: Vec<(String, String)> = topics::Topic::ALL.into_iter().map(|t| (t.id().to_string(), t.label().to_string())).collect();
    let text = draft().trim().trim_start_matches('@').to_lowercase();
    let is_domain = !text.contains('@');
    let freemail_domain = is_domain && topics::is_freemail(&text);
    let ok = valid_sender(&text) && !freemail_domain;
    let hint = if freemail_domain {
        format!("{text} is a free mail provider. Add an address there instead.")
    } else {
        "An address, like name@example.com, or a domain, like example.com.".to_string()
    };
    let add = move |_| {
        let text = draft.peek().trim().trim_start_matches('@').to_lowercase();
        let is_domain = !text.contains('@');
        if !valid_sender(&text) || (is_domain && topics::is_freemail(&text)) {
            return;
        }
        let Some(topic) = topics::Topic::parse(&draft_topic.peek()) else { return };
        if let Some(change) = app.check(store().set_topic(None, topic_target(&text, is_domain), Some(topic))) {
            draft.set(String::new());
            app.bump_mail();
            app.toast_with_undo(format!("Mail from {text} will show in {}", topic.label()), UndoAction::Topic(change));
        }
    };
    rsx! {
        Section {
            title: "Topic sorting",
            description: "Mail > Topics lists your mail by what it's about: orders, travel, bills, newsletters and more, across your folders.",
            Switch {
                label: "Sort mail by topic",
                checked: st.topics_view,
                onchange: move |on: bool| {
                    if on {
                        app.update_settings(|s| s.topics_view = true);
                        app.bump_mail();
                    } else {
                        confirm_off.set(true);
                    }
                },
            }
            Note { text: "Topics are worked out on this device and never move your mail or change anything on the server." }
            if pending > 0 {
                Note { text: "About {pending} messages still to sort." }
            }
            LinkButton {
                label: "Sort my mail again",
                disabled: !st.topics_view,
                title: if st.topics_view { "Work out every message's topic again. Your corrections stay." } else { "Turn on topic sorting first" },
                onclick: move |_| {
                    if app.check(store().resort_topics()).is_some() {
                        app.bump_mail();
                        app.toast("Sorting your mail again");
                    }
                },
            }
        }
        Section { title: "Shown topics", description: "Show in the folder pane: the topics listed under Views. Hidden topics are still sorted.",
            for t in topics::Topic::ALL {
                Checkbox {
                    key: "{t.id()}",
                    label: t.label(),
                    checked: !st.topics_hidden.iter().any(|h| h == t.id()),
                    disabled: !st.topics_view,
                    onchange: move |show: bool| {
                        app.update_settings(|s| {
                            s.topics_hidden.retain(|h| h != t.id());
                            if !show {
                                s.topics_hidden.push(t.id().to_string());
                            }
                        });
                    },
                }
            }
        }
        Section {
            title: "Your corrections",
            description: "Change topic on a message to teach Axentra where mail from that sender belongs. Corrections are kept in your backups.",
            div { class: "ax-sender-add-wrap",
                div { class: "ax-row ax-sender-add", style: "gap:8px;align-items:flex-end;flex-wrap:wrap",
                    TextField {
                        label: "Address or domain",
                        placeholder: "name@example.com or example.com",
                        value: draft(),
                        invalid: !text.is_empty() && !ok,
                        oninput: move |v| draft.set(v),
                    }
                    Select {
                        value: draft_topic(),
                        options: topic_options.clone(),
                        aria_label: "Topic for this sender",
                        onchange: move |v: String| draft_topic.set(v),
                    }
                    Button { label: "Add", icon: "add", disabled: !ok, title: if ok { "Add the correction" } else { "Type an address or domain first" }, onclick: add }
                }
                span { class: "ax-field-hint ax-sender-hint", class: if freemail_domain { "ax-danger" }, "aria-live": "polite", "{hint}" }
            }
            if rules.is_empty() {
                Note { text: "No corrections yet. Use Change topic on a message to add one." }
            } else {
                div { class: "ax-cat-list", role: "list", "aria-label": "Your corrections",
                    for r in rules.into_iter() {
                        {
                            let rule = r.rule.clone();
                            let account_email = accounts.iter().find(|a| a.id == rule.account_key).map(|a| a.email.as_str());
                            let shown = correction_text(&rule.pattern, rule.is_domain, account_email);
                            let account = (rule.account_key != 0).then_some(rule.account_key);
                            let pattern = rule.pattern.clone();
                            let id = r.id;
                            rsx! {
                                div { key: "{id}", class: "ax-cat-row ax-topic-rule", role: "listitem",
                                    span { class: "ax-grow ax-ellipsis ax-selectable", title: "{shown}", "{shown}" }
                                    Select {
                                        value: rule.topic.id(),
                                        options: topic_options.clone(),
                                        aria_label: "Topic for {pattern}",
                                        onchange: move |v: String| {
                                            if let Some(t) = topics::Topic::parse(&v)
                                                && let Some(change) = app.check(store().set_topic(account, topic_target(&rule.pattern, rule.is_domain), Some(t)))
                                            {
                                                app.bump_mail();
                                                app.toast_with_undo(format!("Mail from {} will show in {}", rule.pattern, t.label()), UndoAction::Topic(change));
                                            }
                                        },
                                    }
                                    IconButton {
                                        icon: "delete",
                                        title: "Remove {pattern}",
                                        small: true,
                                        onclick: move |_| {
                                            if app.check(store().remove_topic_rule(id)).is_some() {
                                                app.bump_mail();
                                            }
                                        },
                                    }
                                }
                            }
                        }
                    }
                }
                Button { label: "Forget all corrections\u{2026}", icon: "delete", onclick: move |_| confirm_forget.set(true) }
            }
        }
        if confirm_off() {
            ConfirmDialog {
                title: "Turn off topics?",
                message: "Topics worked out so far are removed from this device. Your corrections stay, and your mail isn't changed.",
                confirm_label: "Turn off",
                on_cancel: move |_| confirm_off.set(false),
                on_confirm: move |_| {
                    confirm_off.set(false);
                    app.update_settings(|s| s.topics_view = false);
                    app.check(store().clear_topics());
                    app.bump_mail();
                },
            }
        }
        if confirm_forget() {
            ConfirmDialog {
                title: "Forget all corrections?",
                message: "Mail goes back to the topics Axentra works out. This can't be undone.",
                confirm_label: "Forget",
                danger: true,
                on_cancel: move |_| confirm_forget.set(false),
                on_confirm: move |_| {
                    confirm_forget.set(false);
                    if app.check(store().clear_topic_rules()).is_some() {
                        app.bump_mail();
                        app.toast("Forgot your topic corrections");
                    }
                },
            }
        }
    }
}

/// Pages whose features live in the mail module.
#[component]
pub fn ModulePage(page: PageId) -> Element {
    let (icon, title, text) = match page {
        PageId::Rules => ("arrow_sort", "No rules yet", "Rules you create from a message (Home > Rules > Create rule) appear here."),
        PageId::QuickSteps => ("sparkle", "No quick steps yet", "Quick steps you create from Home > Quick steps appear here."),
        PageId::CustomizeActions => ("options", "Message actions", "Message actions shown in the reading pane and on hover follow the ribbon commands."),
        PageId::Forwarding => ("arrow_forward", "Forwarding is off", "Forwarding is set on the server for Microsoft 365 and Outlook.com accounts."),
        PageId::ConditionalFormatting => ("paint_brush", "No conditional formatting rules", "Rules that colour messages in the list appear here."),
        PageId::SearchFolders => ("folder", "No search folders", "Search folders you create from the folder pane appear here."),
        PageId::Smime => ("shield", "S/MIME isn't set up", "Signing and encrypting messages needs a certificate installed for your account."),
        PageId::RetentionPolicies => ("archive", "No retention policies", "Policies from your organization appear here for Microsoft 365 work or school accounts."),
        PageId::Groups => ("people_team", "No groups", "Microsoft 365 groups you belong to appear here for work or school accounts."),
        _ => ("settings", "Nothing to set", ""),
    };
    rsx! { Unavailable { icon, title, text } }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sender_entries_are_addresses_or_domains() {
        for ok in ["ana@example.com", "example.com", "news.example.co.uk", " Bob@Example.org "] {
            assert!(valid_sender(ok), "{ok}");
        }
        for bad in ["", "ana", "@example.com", "ana@", "a b@example.com", "ana@@example.com", ".com", "example."] {
            assert!(!valid_sender(bad), "{bad}");
        }
    }

    /// The line under a sender box: the same helper while typing (so the
    /// layout never jumps), a reason once Enter refused the entry.
    #[test]
    fn sender_hints_explain_a_refused_entry() {
        let helper = sender_hint("", false, false, false);
        assert!(!helper.1 && helper.0.starts_with("Enter an email address"));
        assert_eq!(sender_hint("m", false, false, false), helper, "typing: no error yet");
        let refused = sender_hint("mtypo", false, false, true);
        assert!(refused.1 && refused.0.contains("\u{201c}mtypo\u{201d} isn't an email address or a domain"), "{refused:?}");
        assert_eq!(sender_hint("a@b.example", true, true, false), ("That's already on the list.".to_string(), false));
        assert!(sender_hint("a@b.example", true, true, true).1);
        assert_eq!(sender_hint("a@b.example", true, false, false), helper);
    }

    #[test]
    fn sweep_rules_read_as_sentences() {
        assert_eq!(
            sweep_rule_text("keep_latest", "news@shop.example", "Inbox", "Deleted Items"),
            "Keep only the latest message from news@shop.example in Inbox; move the rest to Deleted Items"
        );
        assert!(sweep_rule_text("older_than_10_days", "a@b.c", "Inbox", "Archive").contains("older than 10 days"));
        assert!(sweep_rule_text("move_all_future", "a@b.c", "Inbox", "Archive").contains("in the future"));
    }

    #[test]
    fn corrections_say_which_account_they_are_for() {
        assert_eq!(correction_text("shop.example", true, None), "shop.example (whole domain)");
        assert_eq!(correction_text("shop.example", true, Some("me@x.example")), "shop.example (whole domain) \u{b7} me@x.example");
        assert_eq!(correction_text("a@shop.example", false, Some("me@x.example")), "a@shop.example \u{b7} me@x.example");
    }

    #[test]
    fn auto_archive_toast_text() {
        assert_eq!(auto_archive_text(1, 30, false), "Archived 1 message older than 30 days");
        assert_eq!(auto_archive_text(312, 365, true), "Archived 312 messages older than 1 year, sorted by topic");
    }

    #[test]
    fn undo_send_stops_cover_off_to_thirty() {
        assert_eq!(UNDO_SEND_STOPS.first(), Some(&0));
        assert_eq!(UNDO_SEND_STOPS.last(), Some(&30));
        assert!(UNDO_SEND_STOPS.windows(2).all(|w| w[0] < w[1]));
    }
}

/// Local AI review, a pilot: see `junk::shadow`.
#[component]
fn ShadowReviewSection() -> Element {
    let app = use_app();
    let on = app.settings.read().junk_shadow;
    let mut cleared = use_signal(|| 0);
    let _ = cleared();
    let tally = if on { store().shadow_tally().unwrap_or_default() } else { Default::default() };
    let problem = if on { crate::mail::shadow_pass::problem() } else { None };
    rsx! {
        Section {
            title: "Local AI review (pilot)",
            description: "Two AI models on this computer read new mail from senders you've never dealt with and suggest \u{201c}Looks like phishing\u{201d} or \u{201c}Probably bulk mail\u{201d} in the reading pane. Nothing moves because of a suggestion, and no mail leaves this computer. It needs Ollama with qwen3:4b and deepseek-r1:14b, which use your graphics card for about 10 seconds on each flagged message.",
            Checkbox {
                label: "Review new mail with local AI",
                checked: on,
                onchange: move |v| crate::mail::actions::set_junk_shadow(app, v),
            }
            if let Some(problem) = problem {
                Note { text: problem.text() }
            }
            if on {
                p { class: "ax-set-desc",
                    "{tally.reviewed} reviewed: {tally.phishing} looked like phishing, {tally.bulk} probably bulk. Your reports so far: {tally.confirmed} agreed, {tally.rejected} marked not junk, {tally.missed} junk the review missed."
                }
                Button {
                    label: "Clear results",
                    onclick: move |_| {
                        let _ = app.check(store().forget_shadow());
                        cleared += 1;
                    },
                }
            }
        }
    }
}
