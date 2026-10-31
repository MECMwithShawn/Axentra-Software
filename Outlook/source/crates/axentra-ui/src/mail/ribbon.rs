//! The Mail ribbon: File, Home, View, Help, the contextual Search tab while
//! searching, and the compose tabs while a compose form is open
//! (docs/spec/shell.md §6.3–6.9).

use axentra_core::*;
use dioxus::prelude::*;

use super::actions;
use super::ctx::{MailCtx, SearchScope};
use super::menus::{self, Target};
use super::model::ListModel;
use crate::kit::MenuItem;
use crate::shell::ribbon::{Command, RibbonTab};
use crate::state::{AppState, SearchState};

/// Compose tab command ids are prefixed so they can't clash with mail ids.
pub const COMPOSE_PREFIX: &str = "compose::";

fn prefix_menu(items: Vec<MenuItem>) -> Vec<MenuItem> {
    items
        .into_iter()
        .map(|mut i| {
            if !i.divider && !i.header {
                i.id = format!("{COMPOSE_PREFIX}{}", i.id);
            }
            i.children = prefix_menu(std::mem::take(&mut i.children));
            i
        })
        .collect()
}

fn compose_tabs() -> Vec<RibbonTab> {
    crate::mail::compose::ribbon_tabs()
        .into_iter()
        .map(|mut t| {
            for group in &mut t.groups {
                for cmd in group.iter_mut() {
                    cmd.id = format!("{COMPOSE_PREFIX}{}", cmd.id);
                    cmd.menu = prefix_menu(std::mem::take(&mut cmd.menu));
                }
            }
            t
        })
        .collect()
}

/// Keywords the Search tab toggles inside the query.
const SEARCH_TOGGLES: [(&str, &str, &str, &str); 3] = [
    ("s_attachments", "Has attachments", "attach", "hasattachments:yes"),
    ("s_unread", "Unread", "mail_unread", "is:unread"),
    ("s_flagged", "Flagged", "flag", "is:flagged"),
];

fn has_keyword(query: &str, kw: &str) -> bool {
    query.split_whitespace().any(|w| w.eq_ignore_ascii_case(kw))
}

pub fn tabs(app: AppState, ctx: MailCtx, model: &ListModel) -> Vec<RibbonTab> {
    let settings = app.settings.read().clone();
    let ids = actions::targets(ctx, model);
    let none = ids.is_empty();
    let role = if model.searching { None } else { model.folder.as_ref().map(|f| f.role) };
    let folder = model.folder.as_ref().map(|f| f.id);
    let t = Target::for_ids(&ids, role, folder, settings.focused_inbox_shown(), settings.use_24h_time);
    let single = t.messages.len() == 1;
    let drafts = role == Some(FolderRole::Drafts) || (!t.messages.is_empty() && t.messages.iter().all(|m| m.is_draft));
    let all_read = !t.messages.is_empty() && t.messages.iter().all(|m| m.is_read);
    let all_flagged = !t.messages.is_empty() && t.messages.iter().all(|m| m.flag == FlagStatus::Flagged);
    let all_pinned = !t.messages.is_empty() && t.messages.iter().all(|m| m.is_pinned);
    let account = t.messages.first().map(|m| m.account_id).or(model.folder.as_ref().map(|f| f.account_id));
    let no_sel = "Select a message first";

    // ---------------------------------------------------------------- Home
    let mut delete_menu = vec![
        MenuItem::new("delete", "Delete").icon("delete").shortcut("Delete").disabled(none),
        MenuItem::new("delete_permanent", "Permanently delete").icon("delete").shortcut("Shift+Delete").disabled(none),
    ];
    if let Some(f) = model.folder.as_ref().filter(|_| !model.searching) {
        delete_menu.push(MenuItem::divider());
        match f.role {
            FolderRole::Deleted => delete_menu.push(MenuItem::new("folder_empty", "Empty folder").icon("delete").disabled(f.total_count == 0)),
            FolderRole::Drafts | FolderRole::Outbox => {
                delete_menu.pop();
            }
            _ => delete_menu.push(MenuItem::new("folder_delete_all", "Delete all").icon("delete").disabled(f.total_count == 0)),
        }
    }
    let report = if role == Some(FolderRole::Junk) {
        Command::new("not_junk", "Not junk", "mail_inbox")
            .split(vec![
                MenuItem::new("not_junk", "Not junk").icon("mail_inbox"),
                MenuItem::new("report_phishing", "Report phishing").icon("shield_error"),
            ])
            .disabled(none)
            .tooltip(if none { no_sel } else { "Not junk" })
    } else {
        Command::new("report", "Report", "shield_error")
            .menu(vec![
                MenuItem::new("report_junk", "Report junk").icon("mail_prohibited"),
                MenuItem::new("report_phishing", "Report phishing").icon("shield_error"),
            ])
            .disabled(none || drafts || role == Some(FolderRole::Sent))
            .tooltip(if none { no_sel } else { "Report junk or phishing" })
    };
    let sweep_ok = single && !matches!(role, Some(FolderRole::Junk | FolderRole::Drafts | FolderRole::Sent | FolderRole::Deleted)) && role.is_some();
    let mut move_menu = menus::move_items("move", account, folder, "Other folder…");
    move_menu.push(MenuItem::new("copy_picker", "Copy to…").icon("copy"));
    if settings.focused_inbox_shown() && role == Some(FolderRole::Inbox) && !none {
        let other = t.messages.iter().all(|m| m.inference == InferenceClass::Other);
        move_menu.push(MenuItem::divider());
        if other {
            move_menu.push(MenuItem::new("move_focused", "Move to Focused inbox").icon("arrow_move"));
            move_menu.push(MenuItem::new("always_focused", "Always move to Focused inbox").icon("arrow_move"));
        } else {
            move_menu.push(MenuItem::new("move_other", "Move to Other inbox").icon("arrow_move"));
            move_menu.push(MenuItem::new("always_other", "Always move to Other inbox").icon("arrow_move"));
        }
    }
    let respond_ok = single && !drafts;
    let snoozed_folder = t.snoozed_folder;
    let snooze_menu = if snoozed_folder { vec![MenuItem::new("unsnooze", "Unsnooze").icon("snooze")] } else { menus::snooze_items(settings.use_24h_time) };
    let more_menu = vec![
        MenuItem::new("block_menu", "Block").icon("person_prohibited").submenu(vec![
            MenuItem::new("block", "Block sender").icon("person_prohibited").disabled(!single || drafts),
            MenuItem::new("never_block", "Never block sender").icon("shield").disabled(none || drafts),
        ]),
        MenuItem::new("ignore", "Ignore conversation").icon("eye_off").disabled(!single || drafts),
        MenuItem::divider(),
        MenuItem::new("details", "View message details").icon("info").disabled(!single),
        MenuItem::new("save_as", "Save as").icon("save").disabled(!single),
    ];
    // Groups follow the Classic layout (spec 6.4); priorities are the
    // Simplified overflow order, 1 = last to overflow (spec 6.11).
    let home = RibbonTab::new(
        "home",
        "Home",
        vec![
            vec![Command::new("new_mail", "New mail", "mail_add")
                .primary()
                .split(vec![
                    MenuItem::new("new_mail", "Mail").icon("mail").shortcut("Ctrl+N"),
                    MenuItem::new("new_event", "Event").icon("calendar_add"),
                ])
                .tooltip("New mail (Ctrl+N)")
                .group("New")
                .priority(1)],
            vec![
                Command::new("delete", "Delete", "delete").split(delete_menu).disabled(none).tooltip(if none { no_sel } else { "Delete (Delete)" }).group("Delete").priority(2),
                Command::new("archive", "Archive", "archive")
                    .disabled(none || role == Some(FolderRole::Archive))
                    .tooltip(if none { no_sel } else { "Archive (Backspace)" })
                    .priority(3),
                report.priority(10),
                Command::new("sweep", "Sweep", "broom")
                    .disabled(!sweep_ok)
                    .tooltip(if sweep_ok { "Sweep messages from this sender" } else { "Select one message (not in Junk Email, Drafts, Sent Items or Deleted Items)" })
                    .priority(14),
            ],
            vec![
                Command::new("move_to", "Move to", "folder_arrow_right")
                    .menu(move_menu)
                    .disabled(none)
                    .tooltip(if none { no_sel } else { "Move to (Ctrl+Shift+V)" })
                    .group("Move")
                    .priority(7),
            ],
            vec![
                Command::new("reply_all", "Reply all", "arrow_reply_all")
                    .split(vec![
                        MenuItem::new("reply", "Reply").icon("arrow_reply").shortcut("Ctrl+R").disabled(!respond_ok),
                        MenuItem::new("reply_all", "Reply all").icon("arrow_reply_all").shortcut("Ctrl+Shift+R").disabled(!respond_ok),
                        MenuItem::new("forward", "Forward").icon("arrow_forward").shortcut("Ctrl+F").disabled(!respond_ok),
                        MenuItem::new("forward_attachment", "Forward as attachment").icon("attach").disabled(none || drafts),
                    ])
                    .disabled(none || drafts)
                    .tooltip(if respond_ok { "Reply all (Ctrl+Shift+R)" } else { "Select one message to reply" })
                    .group("Respond")
                    .priority(4),
            ],
            vec![
                Command::new("quick_steps", "Quick steps", "flash")
                    .split(vec![MenuItem::new("quick_steps_manage", "Manage quick steps").disabled(true)])
                    .disabled(true)
                    .tooltip("Quick steps aren't available in Axentra yet")
                    .group("Quick steps")
                    .priority(15),
            ],
            vec![
                Command::new("toggle_read", "Read / Unread", if all_read { "mail_unread" } else { "mail_read" })
                    .disabled(none)
                    .tooltip(if none { no_sel } else if all_read { "Mark as unread (Ctrl+U)" } else { "Mark as read (Ctrl+Q)" })
                    .group("Tags")
                    .priority(8),
                Command::new("categorize", "Categorize", "tag")
                    .menu(menus::categorize_items(&t.messages))
                    .disabled(none)
                    .tooltip(if none { no_sel } else { "Categorize" })
                    .priority(11),
                Command::new("flag_toggle", "Flag / Unflag", if all_flagged { "flag_filled" } else { "flag" })
                    .split(menus::flag_items(&t.messages, settings.week_start, &settings.work_days))
                    .toggled(all_flagged)
                    .disabled(none)
                    .tooltip(if none { no_sel } else if all_flagged { "Unflag (Insert)" } else { "Flag (Insert)" })
                    .priority(9),
                Command::new("toggle_pin", if all_pinned { "Unpin" } else { "Pin" }, if all_pinned { "pin_filled" } else { "pin" })
                    .toggled(all_pinned)
                    .disabled(none)
                    .tooltip(if none { no_sel } else if all_pinned { "Unpin" } else { "Pin to the top of the list" })
                    .priority(12),
                Command::new("snooze", if snoozed_folder { "Unsnooze" } else { "Snooze" }, "snooze")
                    .menu(snooze_menu)
                    .disabled(none || drafts || role == Some(FolderRole::Sent))
                    .tooltip(if none { no_sel } else { "Snooze" })
                    .priority(13),
            ],
            vec![
                Command::new("print", "Print", "print")
                    .disabled(!single)
                    .tooltip(if single { "Print (Ctrl+P)" } else { "Select one message to print" })
                    .group("Actions")
                    .priority(16),
                Command::new("more", "More options", "more_horizontal").menu(more_menu).icon_only().tooltip("More options").priority(17),
            ],
        ],
    );

    // ---------------------------------------------------------------- View
    let rp = settings.reading_pane;
    let density = settings.density;
    let prefs = ctx.prefs.read().clone();
    let zoom = *ctx.zoom.read();
    let view = RibbonTab::new(
        "view",
        "View",
        vec![
            vec![Command::new("view_settings", "View settings", "settings").tooltip("Open Mail settings").group("Settings").priority(1)],
            vec![
                Command::new("v_folder", "Folder pane", "panel_left").group("Layout").priority(3).menu(vec![
                    MenuItem::new("v_folder_show", "Show").checked(!settings.folder_pane_collapsed),
                    MenuItem::new("v_folder_hide", "Hide").checked(settings.folder_pane_collapsed),
                    MenuItem::divider(),
                    MenuItem::new("v_fav_show", "Show favorites").checked(!prefs.favorites_hidden),
                    MenuItem::new("v_fav_pin", "Pin favorites").checked(prefs.favorites_pinned),
                ]),
                Command::new("v_reading", "Reading pane", if rp == ReadingPanePosition::Bottom { "panel_bottom" } else { "panel_right" }).priority(2).menu(vec![
                    MenuItem::new("v_rp_right", "Show on the right").icon("panel_right").checked(rp == ReadingPanePosition::Right),
                    MenuItem::new("v_rp_bottom", "Show on the bottom").icon("panel_bottom").checked(rp == ReadingPanePosition::Bottom),
                    MenuItem::new("v_rp_hidden", "Hide").icon("eye_off").checked(rp == ReadingPanePosition::Hidden),
                ]),
                Command::new("v_density", "Density", "text_density").priority(4).menu(vec![
                    MenuItem::new("v_density_roomy", "Roomy").checked(density == Density::Roomy),
                    MenuItem::new("v_density_cozy", "Cozy").checked(density == Density::Cozy),
                    MenuItem::new("v_density_compact", "Compact").checked(density == Density::Compact),
                ]),
            ],
            vec![
                Command::new("v_conversations", "Conversations", "mail_list").group("Messages").priority(5).menu(vec![
                    MenuItem::new("v_conv_on", "Group messages by conversation").checked(settings.conversation_view),
                    MenuItem::new("v_conv_off", "Do not group messages").checked(!settings.conversation_view),
                    MenuItem::divider(),
                    MenuItem::header("Reading pane order"),
                    MenuItem::new("v_order_top", "Newest on top").checked(settings.conversation_order == ConversationOrder::NewestOnTop),
                    MenuItem::new("v_order_bottom", "Newest on bottom").checked(settings.conversation_order == ConversationOrder::NewestOnBottom),
                ]),
                Command::new("v_preview", "Message preview", "document_one_page").priority(6).menu(vec![
                    MenuItem::new("v_preview_show", "Show preview text").checked(settings.show_preview_text),
                    MenuItem::new("v_preview_hide", "Hide preview text").checked(!settings.show_preview_text),
                ]),
                Command::new("v_sender_image", "Sender image", "person_circle")
                    .toggled(settings.show_sender_images)
                    .tooltip("Show sender images in the message list")
                    .priority(7),
            ],
            vec![Command::new("v_zoom", "Zoom", "zoom_in").group("Zoom").priority(8).menu(vec![
                MenuItem::new("v_zoom_in", "Zoom in").icon("zoom_in").disabled(zoom >= 200),
                MenuItem::new("v_zoom_out", "Zoom out").icon("zoom_out").disabled(zoom <= 50),
                MenuItem::divider(),
                MenuItem::new("v_zoom_reset", "Reset").shortcut(format!("{zoom}%")).disabled(zoom == 100),
            ])],
        ],
    );

    let file = RibbonTab::new(
        "file",
        "File",
        vec![vec![
            Command::new("print", "Print", "print").disabled(!single).tooltip(if single { "Print (Ctrl+P)" } else { "Select one message to print" }),
            Command::new("save_as", "Save as", "save").disabled(!single).tooltip(if single { "Save the message as an .eml file in Downloads" } else { "Select one message to save" }),
            Command::new("manage_accounts", "Manage accounts", "person_circle"),
        ]],
    );
    let help = RibbonTab::new(
        "help",
        "Help",
        vec![vec![
            Command::new("help", "Help", "question_circle").tooltip("Tips for getting around Mail"),
            Command::new("shortcuts", "Keyboard shortcuts", "keyboard").tooltip("Keyboard shortcuts (?)"),
        ]],
    );

    let mut tabs = vec![file, home, view, help];
    if model.searching {
        let query = app.search.read().query.clone();
        let scope = *ctx.search_scope.read();
        let mut cmds = vec![Command::new("s_scope", if scope == SearchScope::AllFolders { "All folders" } else { "Current folder" }, "folder_search").menu(vec![
            MenuItem::new("s_scope_all", "All folders").checked(scope == SearchScope::AllFolders),
            MenuItem::new("s_scope_current", "Current folder").checked(scope == SearchScope::CurrentFolder).disabled(model.folder.is_none()),
        ])];
        for (id, label, icon, kw) in SEARCH_TOGGLES {
            cmds.push(Command::new(id, label, icon).toggled(has_keyword(&query, kw)));
        }
        let any = SEARCH_TOGGLES.iter().any(|(_, _, _, kw)| has_keyword(&query, kw));
        tabs.push(RibbonTab::new(
            "search",
            "Search",
            vec![
                cmds,
                vec![
                    Command::new("s_clear", "Clear filters", "filter_dismiss").disabled(!any).tooltip(if any { "Clear filters" } else { "No filters to clear" }),
                    Command::new("s_exit", "Exit search", "arrow_exit"),
                ],
            ],
        ));
    }
    if app.compose.read().as_ref().is_some_and(|c| !c.popout) {
        tabs.extend(compose_tabs());
    }
    tabs
}

/// Handle a ribbon pick.
pub fn on_command(mut app: AppState, mut ctx: MailCtx, model: &ListModel, id: &str) {
    if let Some(cmd) = id.strip_prefix(COMPOSE_PREFIX) {
        app.compose_command(cmd);
        return;
    }
    let step_zoom = |z: u32, up: bool| -> u32 {
        const STEPS: [u32; 9] = [50, 67, 80, 90, 100, 110, 125, 150, 200];
        if up { STEPS.into_iter().find(|s| *s > z).unwrap_or(200) } else { STEPS.into_iter().rev().find(|s| *s < z).unwrap_or(50) }
    };
    match id {
        "v_folder_show" => app.update_settings(|s| s.folder_pane_collapsed = false),
        "v_folder_hide" => app.update_settings(|s| s.folder_pane_collapsed = true),
        "v_fav_show" => ctx.update_prefs(|p| p.favorites_hidden = !p.favorites_hidden),
        "v_fav_pin" => ctx.update_prefs(|p| p.favorites_pinned = !p.favorites_pinned),
        "v_rp_right" => app.update_settings(|s| s.reading_pane = ReadingPanePosition::Right),
        "v_rp_bottom" => app.update_settings(|s| s.reading_pane = ReadingPanePosition::Bottom),
        "v_rp_hidden" => app.update_settings(|s| s.reading_pane = ReadingPanePosition::Hidden),
        "v_density_roomy" | "v_density_cozy" | "v_density_compact" => {
            let d = match id {
                "v_density_roomy" => Density::Roomy,
                "v_density_cozy" => Density::Cozy,
                _ => Density::Compact,
            };
            // Density sets the defaults for sender images and attachment previews
            // (docs/spec/shell.md §1.14); the toggles can still be changed afterwards.
            app.update_settings(|s| {
                s.density = d;
                s.show_sender_images = d == Density::Roomy;
                s.show_attachment_previews = d == Density::Roomy;
            });
        }
        "v_conv_on" => app.update_settings(|s| s.conversation_view = true),
        "v_conv_off" => {
            ctx.expanded.set(None);
            app.update_settings(|s| s.conversation_view = false);
        }
        "v_order_top" => app.update_settings(|s| s.conversation_order = ConversationOrder::NewestOnTop),
        "v_order_bottom" => app.update_settings(|s| s.conversation_order = ConversationOrder::NewestOnBottom),
        "v_preview_show" => app.update_settings(|s| s.show_preview_text = true),
        "v_preview_hide" => app.update_settings(|s| s.show_preview_text = false),
        "v_sender_image" => app.update_settings(|s| s.show_sender_images = !s.show_sender_images),
        "v_zoom_in" => {
            let z = *ctx.zoom.peek();
            ctx.zoom.set(step_zoom(z, true));
        }
        "v_zoom_out" => {
            let z = *ctx.zoom.peek();
            ctx.zoom.set(step_zoom(z, false));
        }
        "v_zoom_reset" => ctx.zoom.set(100),
        "s_scope_all" => ctx.search_scope.set(SearchScope::AllFolders),
        "s_scope_current" => ctx.search_scope.set(SearchScope::CurrentFolder),
        "s_clear" => {
            let mut s = app.search.peek().clone();
            s.query = s
                .query
                .split_whitespace()
                .filter(|w| !SEARCH_TOGGLES.iter().any(|(_, _, _, kw)| w.eq_ignore_ascii_case(kw)))
                .collect::<Vec<_>>()
                .join(" ");
            s.active = !s.query.trim().is_empty();
            s.serial += 1;
            app.search.set(s);
        }
        "s_exit" => {
            let serial = app.search.peek().serial + 1;
            app.search.set(SearchState { serial, ..Default::default() });
        }
        "folder_empty" => {
            if let Some(f) = &model.folder {
                ctx.dialog.set(Some(super::ctx::MailDialog::EmptyFolder(f.id)));
            }
        }
        "folder_delete_all" => {
            if let Some(f) = &model.folder {
                ctx.dialog.set(Some(super::ctx::MailDialog::DeleteAll(f.id)));
            }
        }
        other => {
            if let Some((_, _, _, kw)) = SEARCH_TOGGLES.iter().find(|(i, ..)| *i == other) {
                let mut s = app.search.peek().clone();
                if has_keyword(&s.query, kw) {
                    s.query = s.query.split_whitespace().filter(|w| !w.eq_ignore_ascii_case(kw)).collect::<Vec<_>>().join(" ");
                } else {
                    s.query = format!("{} {kw}", s.query.trim()).trim().to_string();
                }
                s.active = !s.query.trim().is_empty();
                s.serial += 1;
                app.search.set(s);
                return;
            }
            let ids = actions::targets(ctx, model);
            actions::run(app, ctx, model, other, ids);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_keywords_are_found_case_insensitively() {
        assert!(has_keyword("budget IS:UNREAD", "is:unread"));
        assert!(!has_keyword("budget is:unreadx", "is:unread"));
        let items = prefix_menu(vec![MenuItem::new("bold", "Bold").submenu(vec![MenuItem::new("x", "X")]), MenuItem::divider()]);
        assert_eq!(items[0].id, "compose::bold");
        assert_eq!(items[0].children[0].id, "compose::x");
        assert!(items[1].id.is_empty());
    }
}
