//! Settings > Mail (docs/spec/shell.md §7.4). Pages for features the mail
//! module owns (Rules, Sweep, ...) use the same layout and say what's there.

use axentra_core::*;
use dioxus::prelude::*;

use super::controls::{LinkButton, Note, Row, Section, Unavailable, use_settings};
use super::pages::PageId;
use crate::kit::{Checkbox, RadioGroup, RadioOption, Select, Slider, Switch, TextField};
use crate::state::store;

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

/// Pages whose features live in the mail module.
#[component]
pub fn ModulePage(page: PageId) -> Element {
    let (icon, title, text) = match page {
        PageId::Rules => ("arrow_sort", "No rules yet", "Rules you create from a message (Home > Rules > Create rule) appear here."),
        PageId::Sweep => ("broom", "No Sweep rules", "Sweep rules you set up from the Sweep button appear here. Standing Sweep rules run once a day."),
        PageId::Junk => ("mail_prohibited", "No blocked or safe senders", "Senders you block or mark as safe from a message appear here."),
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
    fn undo_send_stops_cover_off_to_thirty() {
        assert_eq!(UNDO_SEND_STOPS.first(), Some(&0));
        assert_eq!(UNDO_SEND_STOPS.last(), Some(&30));
        assert!(UNDO_SEND_STOPS.windows(2).all(|w| w[0] < w[1]));
    }
}
