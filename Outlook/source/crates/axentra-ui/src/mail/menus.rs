//! Menu contents: the message context menu (docs/spec/mail.md §4), its
//! submenus (also used by the ribbon) and the folder pane menus (§2.8).

use axentra_core::*;
use chrono::Local;

use super::dates::{self, FlagPreset, SnoozePreset};
use crate::kit::{MenuItem, category_colors};
use crate::state::store;

/// Facts about the messages a menu acts on.
#[derive(Clone, Debug, Default)]
pub struct Target {
    pub messages: Vec<Message>,
    /// Role of the folder the list shows (`None` while searching).
    pub role: Option<FolderRole>,
    pub snoozed_folder: bool,
    pub focused_inbox: bool,
    pub h24: bool,
}

impl Target {
    pub fn for_ids(ids: &[MessageId], role: Option<FolderRole>, folder: Option<FolderId>, focused_inbox: bool, h24: bool) -> Self {
        let s = store();
        let snoozed = s.snoozed_folder_ids().unwrap_or_default();
        Target {
            messages: s.messages_by_ids(ids).unwrap_or_default(),
            role,
            snoozed_folder: folder.is_some_and(|f| snoozed.contains(&f)),
            focused_inbox,
            h24,
        }
    }

    fn single(&self) -> bool {
        self.messages.len() == 1
    }

    fn all_drafts(&self) -> bool {
        !self.messages.is_empty() && self.messages.iter().all(|m| m.is_draft)
    }
}

/// Categories with check marks for the messages, then New / Clear / Manage.
pub fn categorize_items(messages: &[Message]) -> Vec<MenuItem> {
    let mut items: Vec<MenuItem> = store()
        .categories()
        .unwrap_or_default()
        .into_iter()
        .map(|c| {
            let checked = !messages.is_empty() && messages.iter().all(|m| m.categories.contains(&c.name));
            MenuItem::new(format!("cat:{}", c.name), c.name.clone()).swatch(category_colors(&c.color).0).checked(checked)
        })
        .collect();
    items.push(MenuItem::divider());
    items.push(MenuItem::new("cat_new", "New category").icon("add"));
    items.push(MenuItem::new("cat_clear", "Clear all categories").icon("dismiss").disabled(messages.iter().all(|m| m.categories.is_empty())));
    items.push(MenuItem::new("cat_manage", "Manage categories").icon("settings"));
    items
}

/// Flag presets with their resolved due day, then Custom / Mark complete / Clear flag.
pub fn flag_items(messages: &[Message], week_start: chrono::Weekday, work_days: &[chrono::Weekday]) -> Vec<MenuItem> {
    let today = Local::now().date_naive();
    let mut items: Vec<MenuItem> = FlagPreset::ALL
        .into_iter()
        .map(|p| {
            let mut item = MenuItem::new(format!("flag:{}", p.id()), p.label()).icon("flag");
            if let Some(d) = dates::flag_due(p, today, week_start, work_days) {
                item = item.shortcut(d.format("%a").to_string());
            }
            item
        })
        .collect();
    items.push(MenuItem::new("flag_custom", "Custom…").icon("calendar_ltr"));
    items.push(MenuItem::divider());
    let any_flagged = messages.iter().any(|m| m.flag == FlagStatus::Flagged);
    let any_set = messages.iter().any(|m| m.flag != FlagStatus::NotFlagged);
    items.push(MenuItem::new("flag_complete", "Mark complete").icon("checkmark").disabled(!any_flagged));
    items.push(MenuItem::new("flag_clear", "Clear flag").icon("flag_off").disabled(!any_set));
    items
}

/// Snooze presets that apply right now, with their times, then "Choose a date".
pub fn snooze_items(h24: bool) -> Vec<MenuItem> {
    let now = Local::now();
    let mut items: Vec<MenuItem> = SnoozePreset::ALL
        .into_iter()
        .filter_map(|p| {
            dates::snooze_until(p, now).map(|t| {
                MenuItem::new(format!("snooze:{}", p.id()), p.label()).icon("snooze").shortcut(dates::short_when(t, now, h24))
            })
        })
        .collect();
    items.push(MenuItem::divider());
    items.push(MenuItem::new("snooze_custom", "Choose a date").icon("calendar_ltr"));
    items
}

/// Recent folders and the Inbox, then the folder picker.
pub fn move_items(prefix: &str, account: Option<AccountId>, current: Option<FolderId>, picker_label: &str) -> Vec<MenuItem> {
    let s = store();
    let mut items = Vec::new();
    let mut seen = Vec::new();
    for f in s.recent_folders(5).unwrap_or_default() {
        if Some(f.id) != current && account.is_none_or(|a| a == f.account_id) {
            seen.push(f.id);
            items.push(MenuItem::new(format!("{prefix}:{}", f.id), f.name.clone()).icon(f.role.icon()));
        }
    }
    if let Some(a) = account {
        for role in [FolderRole::Inbox, FolderRole::Archive] {
            if let Ok(Some(f)) = s.folder_by_role(a, role)
                && Some(f.id) != current
                && !seen.contains(&f.id)
            {
                items.push(MenuItem::new(format!("{prefix}:{}", f.id), f.name.clone()).icon(role.icon()));
            }
        }
    }
    if !items.is_empty() {
        items.push(MenuItem::divider());
    }
    let picker = if prefix == "copy" { "copy_picker" } else { "move_picker" };
    items.push(MenuItem::new(picker, picker_label).icon("folder_open"));
    items
}

/// Right-click menu for messages in the list (and "More actions" in the
/// reading pane).
pub fn message_menu(t: &Target, folder: Option<FolderId>) -> Vec<MenuItem> {
    let settings_week = store().settings();
    let single = t.single();
    let role = t.role;
    let account = t.messages.first().map(|m| m.account_id);
    let mut items = Vec::new();

    if role == Some(FolderRole::Junk) {
        items.push(MenuItem::new("not_junk", "Not junk").icon("mail_inbox"));
        items.push(MenuItem::new("restore", "Restore").icon("arrow_undo"));
        items.push(MenuItem::divider());
    }
    if matches!(role, Some(FolderRole::Deleted)) {
        items.push(MenuItem::new("restore", "Restore").icon("arrow_undo"));
        items.push(MenuItem::divider());
    }
    if t.all_drafts() {
        items.push(MenuItem::new("edit_draft", "Edit").icon("edit").disabled(!single));
    } else {
        items.push(MenuItem::new("reply", "Reply").icon("arrow_reply").shortcut("Ctrl+R").disabled(!single));
        items.push(MenuItem::new("reply_all", "Reply all").icon("arrow_reply_all").shortcut("Ctrl+Shift+R").disabled(!single));
        items.push(MenuItem::new("forward_menu", "Forward").icon("arrow_forward").submenu(vec![
            MenuItem::new("forward", "Forward").icon("arrow_forward").shortcut("Ctrl+F").disabled(!single),
            MenuItem::new("forward_attachment", "Forward as attachment").icon("attach"),
        ]));
    }
    items.push(MenuItem::divider());
    items.push(MenuItem::new("delete", "Delete").icon("delete").shortcut("Delete"));
    items.push(MenuItem::new("archive", "Archive").icon("archive").shortcut("Backspace").disabled(role == Some(FolderRole::Archive)));
    items.push(MenuItem::new("move_menu", "Move").icon("folder_arrow_right").submenu(move_items("move", account, folder, "Move to a different folder…")));
    items.push(MenuItem::new("copy_menu", "Copy").icon("copy").submenu(move_items("copy", account, folder, "Copy to a different folder…")));
    items.push(MenuItem::divider());
    items.push(MenuItem::new("cat_menu", "Categorize").icon("tag").submenu(categorize_items(&t.messages)));
    let any_unread = t.messages.iter().any(|m| !m.is_read);
    let any_read = t.messages.iter().any(|m| m.is_read);
    if any_unread {
        items.push(MenuItem::new("read", "Mark as read").icon("mail_read").shortcut("Ctrl+Q"));
    }
    if any_read {
        items.push(MenuItem::new("unread", "Mark as unread").icon("mail_unread").shortcut("Ctrl+U"));
    }
    items.push(
        MenuItem::new("flag_menu", "Flag")
            .icon("flag")
            .shortcut("Insert")
            .submenu(flag_items(&t.messages, settings_week.week_start, &settings_week.work_days)),
    );
    let all_pinned = !t.messages.is_empty() && t.messages.iter().all(|m| m.is_pinned);
    items.push(if all_pinned { MenuItem::new("unpin", "Unpin").icon("pin_off") } else { MenuItem::new("pin", "Pin").icon("pin") });
    if t.snoozed_folder {
        items.push(MenuItem::new("unsnooze", "Unsnooze").icon("snooze"));
    } else if !matches!(role, Some(FolderRole::Drafts | FolderRole::Sent)) {
        items.push(MenuItem::new("snooze_menu", "Snooze").icon("snooze").submenu(snooze_items(t.h24)));
    }

    let hide_report = t.all_drafts() || matches!(role, Some(FolderRole::Sent | FolderRole::Drafts));
    let mut tail = Vec::new();
    if t.focused_inbox && role == Some(FolderRole::Inbox) && !t.messages.is_empty() {
        let other = t.messages.iter().all(|m| m.inference == InferenceClass::Other);
        if other {
            tail.push(MenuItem::new("move_focused", "Move to Focused inbox").icon("arrow_move"));
            tail.push(MenuItem::new("always_focused", "Always move to Focused inbox").icon("arrow_move"));
        } else {
            tail.push(MenuItem::new("move_other", "Move to Other inbox").icon("arrow_move"));
            tail.push(MenuItem::new("always_other", "Always move to Other inbox").icon("arrow_move"));
        }
    }
    if !t.all_drafts() {
        let ignored = t.messages.first().is_some_and(|m| store().is_conversation_ignored(&m.conversation_id).unwrap_or(false));
        tail.push(if ignored {
            MenuItem::new("ignore", "Stop ignoring").icon("eye")
        } else {
            MenuItem::new("ignore", "Ignore").icon("eye_off").disabled(!single)
        });
    }
    if !hide_report {
        if role == Some(FolderRole::Junk) {
            tail.push(MenuItem::new("report_menu", "Report").icon("shield_error").submenu(vec![
                MenuItem::new("not_junk", "Not junk").icon("mail_inbox"),
                MenuItem::new("report_phishing", "Report phishing").icon("shield_error"),
            ]));
        } else {
            tail.push(MenuItem::new("report_menu", "Report").icon("shield_error").submenu(vec![
                MenuItem::new("report_junk", "Report junk").icon("mail_prohibited"),
                MenuItem::new("report_phishing", "Report phishing").icon("shield_error"),
            ]));
        }
        tail.push(MenuItem::new("block_menu", "Block").icon("person_prohibited").submenu(vec![
            MenuItem::new("block", "Block sender").icon("person_prohibited").disabled(!single),
            MenuItem::new("never_block", "Never block sender").icon("shield"),
        ]));
    }
    let sweep_hidden = matches!(role, Some(FolderRole::Junk | FolderRole::Drafts | FolderRole::Sent | FolderRole::Deleted)) || role.is_none();
    if !sweep_hidden {
        tail.push(MenuItem::new("sweep", "Sweep…").icon("broom").disabled(!single));
    }
    if !tail.is_empty() {
        items.push(MenuItem::divider());
        items.extend(tail);
    }
    items.push(MenuItem::divider());
    items.push(MenuItem::new("view_menu", "View").icon("eye").submenu(vec![
        MenuItem::new("details", "View message details").icon("info").disabled(!single),
    ]));
    items.push(MenuItem::new("save_as", "Save as").icon("save").disabled(!single));
    items.push(MenuItem::new("print", "Print").icon("print").shortcut("Ctrl+P").disabled(!single));
    items.push(MenuItem::divider());
    items.push(MenuItem::new("open_window", "Open in new window").icon("open").shortcut("Shift+Enter").disabled(!single));
    items
}

/// Folder roles that can't be renamed, deleted or moved.
pub fn is_fixed(folder: &Folder, snoozed: &[FolderId]) -> bool {
    folder.role != FolderRole::Custom || snoozed.contains(&folder.id)
}

/// Right-click / "More options" menu for a folder (or a favourite).
pub fn folder_menu(folder: &Folder, as_favorite: bool, snoozed: &[FolderId]) -> Vec<MenuItem> {
    let fixed = is_fixed(folder, snoozed);
    let mut items = Vec::new();
    if as_favorite {
        items.push(MenuItem::new("f_fav_remove", "Remove from Favorites").icon("star_filled"));
        items.push(MenuItem::new("f_up", "Move up").icon("arrow_up"));
        items.push(MenuItem::new("f_down", "Move down").icon("arrow_down"));
        items.push(MenuItem::divider());
        items.push(MenuItem::new("f_mark_read", "Mark all as read").icon("mail_read").disabled(folder.unread_count == 0));
        items.push(MenuItem::new("f_new_sub", "Create new subfolder").icon("folder_add"));
        return items;
    }
    items.push(MenuItem::new("f_new_sub", "Create new subfolder").icon("folder_add"));
    if !fixed {
        items.push(MenuItem::new("f_rename", "Rename").icon("edit"));
        items.push(MenuItem::new("f_delete", "Delete folder").icon("delete"));
    }
    items.push(MenuItem::divider());
    items.push(MenuItem::new("f_mark_read", "Mark all as read").icon("mail_read").disabled(folder.unread_count == 0));
    match folder.role {
        FolderRole::Deleted => items.push(MenuItem::new("f_empty", "Empty folder").icon("delete").disabled(folder.total_count == 0)),
        FolderRole::Drafts | FolderRole::Outbox => {}
        _ => items.push(MenuItem::new("f_delete_all", "Delete all").icon("delete").disabled(folder.total_count == 0)),
    }
    items.push(MenuItem::divider());
    items.push(if folder.is_favorite {
        MenuItem::new("f_fav_remove", "Remove from Favorites").icon("star_filled")
    } else {
        MenuItem::new("f_fav_add", "Add to Favorites").icon("star")
    });
    if !fixed {
        items.push(MenuItem::new("f_move", "Move").icon("folder_arrow_right"));
    }
    items
}

pub fn favorites_header_menu(pinned: bool) -> Vec<MenuItem> {
    vec![
        if pinned { MenuItem::new("fav_unpin", "Unpin favorites").icon("pin_off") } else { MenuItem::new("fav_pin", "Pin favorites").icon("pin") },
        MenuItem::new("fav_hide", "Hide favorites").icon("eye_off"),
    ]
}

/// Menu for a favourite person under Favorites.
pub fn person_menu() -> Vec<MenuItem> {
    vec![
        MenuItem::new("p_email", "Send email").icon("mail_add"),
        MenuItem::new("p_contact", "Open contact card").icon("contact_card"),
        MenuItem::divider(),
        MenuItem::new("p_fav_remove", "Remove from Favorites").icon("star_filled"),
    ]
}

pub fn account_menu() -> Vec<MenuItem> {
    vec![
        MenuItem::new("a_new", "Create new folder").icon("folder_add"),
        MenuItem::divider(),
        MenuItem::new("a_mark_read", "Mark all as read").icon("mail_read"),
        MenuItem::new("a_settings", "Account settings").icon("settings"),
    ]
}
