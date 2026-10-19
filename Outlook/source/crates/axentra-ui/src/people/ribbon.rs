//! People ribbon: Home, View and Help tabs (docs/spec/people-todo.md 2.4).

use axentra_core::Density;
use dioxus::prelude::*;

use super::actions::{Selection, add_to_list_menu, category_menu};
use super::ctx::PeopleCtx;
use super::view::SortBy;
use crate::kit::MenuItem;
use crate::shell::ribbon::{Command, RibbonTab};

pub fn tabs(ctx: &PeopleCtx, sel: &Selection, has_contacts: bool, density: Density) -> Vec<RibbonTab> {
    let with_email = sel.with_email();
    let fav = sel.all_favorite();
    let chat = sel.chat_uri();

    let new = Command::new("new_contact", "New contact", "person_add").primary().split(vec![
        MenuItem::new("new_contact", "New contact").icon("person_add").shortcut("Ctrl+N"),
        MenuItem::new("new_list", "New contact list").icon("people_add").shortcut("Ctrl+L"),
    ]);
    let edit = Command::new("edit", "Edit", "edit")
        .disabled(!sel.single_editable())
        .tooltip(if sel.single_editable() { "Edit (Shift+E)" } else { "Select one contact or contact list to edit" });
    let delete = Command::new("delete", "Delete", "delete")
        .disabled(!sel.deletable())
        .tooltip(if sel.deletable() { "Delete (Delete)" } else { "Select contacts or contact lists to delete" });
    let favorite = Command::new("favorite", if fav { "Remove from favorites" } else { "Add to favorites" }, if fav { "star_off" } else { "star" })
        .disabled(!sel.deletable())
        .tooltip(if sel.deletable() { if fav { "Remove from favorites" } else { "Add to favorites" } } else { "Select contacts to add to favorites" });
    let add_to_list = if sel.contacts.is_empty() || with_email.is_empty() {
        Command::new("add_to_list", "Add to list", "people_list").disabled(true).tooltip(if sel.contacts.is_empty() {
            "Select contacts to add to a list"
        } else {
            "Contacts without an email address can't be added to a list"
        })
    } else {
        Command::new("add_to_list", "Add to list", "people_list").menu(add_to_list_menu("", &with_email))
    };
    let recipients = sel.recipients();
    let email = Command::new("email", "Email", "mail")
        .disabled(recipients.is_empty())
        .tooltip(if recipients.is_empty() { "Select contacts that have an email address" } else { "Email" });
    let chat_cmd = match &chat {
        Ok(_) => Command::new("chat", "Chat", "chat"),
        Err(why) => Command::new("chat", "Chat", "chat").disabled(true).tooltip(*why),
    };
    let category = if sel.contacts.is_empty() {
        Command::new("categorize", "Add Category", "tag").disabled(true).tooltip("Select contacts to categorize")
    } else {
        Command::new("categorize", "Add Category", "tag").menu(category_menu("", &sel.contacts))
    };
    let import = Command::new("import", "Import contacts", "arrow_import");
    let export = Command::new("export", "Export contacts", "arrow_export")
        .disabled(!has_contacts)
        .tooltip(if has_contacts { "Export contacts" } else { "There are no contacts to export" });

    let sort = *ctx.sort.read();
    let mut sort_items: Vec<MenuItem> =
        SortBy::MENU.iter().map(|s| MenuItem::new(format!("sort:{}", s.key()), s.label()).checked(*s == sort)).collect();
    sort_items.push(MenuItem::divider());
    sort_items.extend(SortBy::EXTRA.iter().map(|s| MenuItem::new(format!("sort:{}", s.key()), s.label()).checked(*s == sort)));
    let last_first = *ctx.last_first.read();
    let display_items = vec![
        MenuItem::new("display:first", "First name").checked(!last_first),
        MenuItem::new("display:last", "Last name").checked(last_first),
    ];
    let density_items = vec![
        MenuItem::new("density:roomy", "Roomy").checked(density == Density::Roomy),
        MenuItem::new("density:cozy", "Cozy").checked(density == Density::Cozy),
        MenuItem::new("density:compact", "Compact").checked(density == Density::Compact),
    ];

    vec![
        RibbonTab::new("home", "Home", vec![vec![new], vec![edit, delete], vec![favorite, add_to_list, email, chat_cmd, category], vec![import, export]]),
        RibbonTab::new(
            "view",
            "View",
            vec![
                vec![
                    Command::new("sort_by", "Sort by", "arrow_sort").menu(sort_items),
                    Command::new("display_by", "Display contacts by", "text_sort_ascending").menu(display_items),
                ],
                vec![
                    Command::new("toggle_nav", "Show navigation pane", "panel_left").toggled(*ctx.nav_open.read()),
                    Command::new("density", "Density", "list").menu(density_items),
                ],
                vec![
                    Command::new("toggle_suggested", "Suggested contacts", "person_lightbulb")
                        .toggled(*ctx.suggested_on.read())
                        .tooltip("Show people you email who aren't saved as contacts. This stays on this device."),
                ],
            ],
        ),
        RibbonTab::new("help", "Help", vec![vec![Command::new("shortcuts", "Keyboard shortcuts", "keyboard").tooltip("Keyboard shortcuts (Shift+?)")]]),
    ]
}
