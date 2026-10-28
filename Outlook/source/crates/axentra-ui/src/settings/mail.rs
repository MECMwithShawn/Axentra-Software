//! Settings > Mail (docs/spec/shell.md §7.4). Pages for features the mail
//! module owns (Rules, Sweep, ...) use the same layout and say what's there.

use axentra_core::*;
use dioxus::prelude::*;

use super::controls::{LinkButton, Note, Row, Section, Unavailable, use_settings};
use super::pages::PageId;
use crate::kit::{Button, Checkbox, Icon, IconButton, RadioGroup, RadioOption, Select, Slider, Switch, TextField, dom};
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
    let ctx = use_settings();
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
                checked: ctx.get().junk_suggestions,
                onchange: move |v| {
                    ctx.edit(|s| s.junk_suggestions = v);
                    app.bump_mail();
                },
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
    fn undo_send_stops_cover_off_to_thirty() {
        assert_eq!(UNDO_SEND_STOPS.first(), Some(&0));
        assert_eq!(UNDO_SEND_STOPS.last(), Some(&30));
        assert!(UNDO_SEND_STOPS.windows(2).all(|w| w[0] < w[1]));
    }
}
