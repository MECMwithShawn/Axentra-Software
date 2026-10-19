//! Commands shared by the ribbon, context menus, the details pane and the
//! keyboard. Every mutation is followed by `app.bump_people()`.

use axentra_core::*;
use axentra_core::Store;
use chrono::{Duration, Local, Timelike, Utc};
use dioxus::prelude::*;

use super::ctx::{MenuTarget, PeopleCtx, PeopleDialog};
use super::io;
use super::view::{Item, Node, Row, SortBy};
use crate::kit::MenuItem;
use crate::state::{AppState, ComposeRequest, NavRequest, UndoAction, store};

/// What the current selection holds, read from the store.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Selection {
    pub contacts: Vec<Contact>,
    pub lists: Vec<ContactList>,
    pub suggested: Vec<String>,
}

impl Selection {
    /// The current selection (memoised in the module context).
    pub fn read(ctx: &PeopleCtx) -> Self {
        (**ctx.selected.read()).clone()
    }

    /// Look the items up in the store.
    pub fn from_items(items: &[Item]) -> Self {
        let mut out = Selection::default();
        let s = store();
        let contact_ids: Vec<ContactId> = items.iter().filter_map(|i| if let Item::Contact(id) = i { Some(*id) } else { None }).collect();
        out.contacts = s.contacts_by_ids(&contact_ids).unwrap_or_default();
        for item in items {
            match item {
                Item::Contact(_) => {}
                Item::List(id) => {
                    if let Ok(Some(l)) = s.contact_list(*id) {
                        out.lists.push(l);
                    }
                }
                Item::Suggested(a) => out.suggested.push(a.clone()),
            }
        }
        out
    }

    pub fn len(&self) -> usize {
        self.contacts.len() + self.lists.len() + self.suggested.len()
    }

    /// Exactly one contact or list (what Edit works on).
    pub fn single_editable(&self) -> bool {
        self.suggested.is_empty() && self.contacts.len() + self.lists.len() == 1
    }

    pub fn deletable(&self) -> bool {
        !self.contacts.is_empty() || !self.lists.is_empty()
    }

    pub fn all_favorite(&self) -> bool {
        self.deletable() && self.contacts.iter().all(|c| c.is_favorite) && self.lists.iter().all(|l| l.is_favorite)
    }

    /// Selected contacts that have an email address (only those can join lists).
    pub fn with_email(&self) -> Vec<EmailAddress> {
        self.contacts
            .iter()
            .filter_map(|c| c.primary_email().map(|e| EmailAddress::new(c.display_name.clone(), e)))
            .chain(self.suggested.iter().map(|a| EmailAddress::new("", a.clone())))
            .collect()
    }

    /// Recipients for "Email": contacts' primary addresses plus list members.
    pub fn recipients(&self) -> Vec<EmailAddress> {
        let mut out = self.with_email();
        for l in &self.lists {
            out.extend(l.members.iter().cloned());
        }
        let mut seen = std::collections::HashSet::new();
        out.retain(|a| seen.insert(a.address.to_lowercase()));
        out
    }

    /// The single contact's chat URI when it has an IM address and a handler.
    pub fn chat_uri(&self) -> std::result::Result<String, &'static str> {
        if self.contacts.len() != 1 || !self.lists.is_empty() {
            return Err("Select one contact to chat with");
        }
        chat_uri(&self.contacts[0])
    }
}

/// First IM address as a URI the desktop can open.
pub fn chat_uri(c: &Contact) -> std::result::Result<String, &'static str> {
    let Some(im) = c.im_addresses.first() else { return Err("No chat app available") };
    let uri = io::im_uri(im);
    match io::scheme(&uri) {
        Some(s) if io::has_handler(&s) => Ok(uri),
        _ => Err("No chat app available"),
    }
}

/// Folder new contacts go to from the current node.
pub fn current_folder(ctx: &PeopleCtx) -> String {
    match &*ctx.node.read() {
        Node::Folder(f) => f.clone(),
        _ => String::new(),
    }
}

pub fn new_contact(ctx: PeopleCtx) {
    let folder = current_folder(&ctx);
    ctx.open_dialog(PeopleDialog::NewContact(Box::new(Contact { folder, ..Default::default() })));
}

pub fn new_list(ctx: PeopleCtx, members: Vec<EmailAddress>) {
    ctx.open_dialog(PeopleDialog::EditList(None, members));
}

pub fn edit(ctx: PeopleCtx) {
    let sel = Selection::read(&ctx);
    if !sel.single_editable() {
        return;
    }
    if let Some(c) = sel.contacts.first() {
        let mut ctx = ctx;
        ctx.editing.set(Some(c.id));
        ctx.edit_dirty.set(false);
    } else if let Some(l) = sel.lists.first() {
        ctx.open_dialog(PeopleDialog::EditList(Some(l.id), Vec::new()));
    }
}

pub fn ask_delete(ctx: PeopleCtx) {
    let contacts = ctx.selected_contacts();
    let lists = ctx.selected_lists();
    if contacts.is_empty() && lists.is_empty() {
        return;
    }
    ctx.open_dialog(PeopleDialog::ConfirmDelete { contacts, lists });
}

/// Delete after confirmation: Undo toast, selection moves to the next row
/// (or the previous one when the last row went).
pub fn delete(mut ctx: PeopleCtx, app: AppState, contacts: Vec<ContactId>, lists: Vec<ContactListId>) {
    let model = ctx.model.peek().clone();
    let gone: Vec<Item> = contacts.iter().map(|id| Item::Contact(*id)).chain(lists.iter().map(|id| Item::List(*id))).collect();
    let items: Vec<Item> = model.items().collect();
    let first_pos = items.iter().position(|i| gone.contains(i));
    let next = first_pos.and_then(|p| {
        items[p..].iter().find(|i| !gone.contains(i)).or_else(|| items[..p].iter().rev().find(|i| !gone.contains(i))).cloned()
    });
    let Some(token) = app.check(store().delete_people(&contacts, &lists)) else { return };
    let text = match (contacts.len(), lists.len()) {
        (1, 0) => "Contact deleted".to_string(),
        (n, 0) => format!("{n} contacts deleted"),
        (0, 1) => "Contact list deleted".to_string(),
        (0, n) => format!("{n} contact lists deleted"),
        (c, l) => format!("{} items deleted", c + l),
    };
    ctx.editing.set(None);
    app.toast_with_undo(text, UndoAction::RestoreContact(token));
    app.bump_people();
    match next {
        Some(item) => ctx.select_only(item, true),
        None => ctx.clear_selection(),
    }
}

pub fn toggle_favorite(ctx: PeopleCtx, app: AppState) {
    let sel = Selection::read(&ctx);
    if !sel.deletable() {
        return;
    }
    let fav = !sel.all_favorite();
    set_favorite(app, &sel.contacts.iter().map(|c| c.id).collect::<Vec<_>>(), &sel.lists.iter().map(|l| l.id).collect::<Vec<_>>(), fav);
}

pub fn set_favorite(app: AppState, contacts: &[ContactId], lists: &[ContactListId], fav: bool) {
    if app.check(store().set_favorites(contacts, lists, fav)).is_some() {
        app.toast(if fav { "Added to favorites" } else { "Removed from favorites" });
        app.bump_people();
    }
}

pub fn email(app: AppState, recipients: Vec<EmailAddress>) {
    if !recipients.is_empty() {
        app.compose(ComposeRequest::to(recipients));
    }
}

/// Toggle membership of `members` in a list (all in it: remove; else add).
pub fn toggle_list_membership(app: AppState, list_id: ContactListId, members: &[EmailAddress]) {
    let Ok(Some(list)) = store().contact_list(list_id) else { return };
    let all_in = !members.is_empty() && members.iter().all(|m| list.members.iter().any(|x| x.address.eq_ignore_ascii_case(&m.address)));
    if all_in {
        for m in members {
            if app.check(store().remove_from_contact_list(list_id, &m.address)).is_none() {
                return;
            }
        }
        app.toast(format!("Removed from {}", list.name));
    } else {
        let Some(dups) = app.check(store().add_to_contact_list(list_id, members)) else { return };
        if dups.len() == members.len() && dups.len() == 1 {
            app.toast(format!("{} is already in this list", dups[0]));
        } else {
            app.toast(format!("Added to {}", list.name));
        }
    }
    app.bump_people();
}

/// "Add to list" menu: lists with a plus (not a member) or an X (member),
/// then "New contact list" (2.4).
pub fn add_to_list_menu(prefix: &str, members: &[EmailAddress]) -> Vec<MenuItem> {
    let lists = store().contact_lists().unwrap_or_default();
    let mut items: Vec<MenuItem> = lists
        .iter()
        .map(|l| {
            let all_in = !members.is_empty() && members.iter().all(|m| l.members.iter().any(|x| x.address.eq_ignore_ascii_case(&m.address)));
            MenuItem::new(format!("{prefix}list:{}", l.id), l.name.clone()).icon(if all_in { "dismiss" } else { "add" })
        })
        .collect();
    if !items.is_empty() {
        items.push(MenuItem::divider());
    }
    items.push(MenuItem::new(format!("{prefix}list:new"), "New contact list").icon("people_add"));
    items
}

/// "Categorize" menu: categories with check marks, then "New category".
pub fn category_menu(prefix: &str, contacts: &[Contact]) -> Vec<MenuItem> {
    let mut items: Vec<MenuItem> = store()
        .categories()
        .unwrap_or_default()
        .into_iter()
        .map(|cat| {
            let all = !contacts.is_empty() && contacts.iter().all(|c| c.categories.contains(&cat.name));
            let (light, _) = crate::kit::category_colors(&cat.color);
            MenuItem::new(format!("{prefix}cat:{}", cat.name), cat.name.clone()).checked(all).swatch(light)
        })
        .collect();
    if !items.is_empty() {
        items.push(MenuItem::divider());
    }
    items.push(MenuItem::new(format!("{prefix}cat:new"), "New category").icon("add"));
    items
}

/// "Move to" menu: the default folder and every user folder.
pub fn move_menu(prefix: &str, contacts: &[Contact]) -> Vec<MenuItem> {
    let current = contacts.first().map(|c| c.folder.clone()).filter(|f| contacts.iter().all(|c| &c.folder == f));
    std::iter::once(String::new())
        .chain(store().contact_folders().unwrap_or_default())
        .map(|f| {
            let label = if f.is_empty() { "Contacts".to_string() } else { f.clone() };
            MenuItem::new(format!("{prefix}move:{f}"), label).checked(current.as_deref() == Some(f.as_str()))
        })
        .collect()
}

/// Context menu for the current selection, in the order of 2.4.
pub fn selection_menu(ctx: &PeopleCtx) -> Vec<MenuItem> {
    let sel = Selection::read(ctx);
    if !sel.suggested.is_empty() {
        return vec![
            MenuItem::new("add_suggested", "Add to contacts").icon("person_add"),
            MenuItem::new("email", "Email").icon("mail"),
            MenuItem::divider(),
            MenuItem::new("dismiss_suggested", "Dismiss").icon("dismiss"),
        ];
    }
    let fav = sel.all_favorite();
    let with_email = sel.with_email();
    let mut items = vec![MenuItem::new("edit", "Edit").icon("edit").disabled(!sel.single_editable())];
    items.push(MenuItem::new("favorite", if fav { "Remove from favorites" } else { "Add to favorites" }).icon(if fav { "star_off" } else { "star" }));
    if !sel.contacts.is_empty() {
        items.push(MenuItem::new("add_to_list", "Add to list").icon("people_list").disabled(with_email.is_empty()).submenu(add_to_list_menu("", &with_email)));
    }
    items.push(MenuItem::new("email", "Email").icon("mail").disabled(sel.recipients().is_empty()));
    if !sel.contacts.is_empty() {
        items.push(MenuItem::new("chat", "Chat").icon("chat").disabled(sel.chat_uri().is_err()));
        items.push(MenuItem::new("categorize", "Categorize").icon("tag").submenu(category_menu("", &sel.contacts)));
        items.push(MenuItem::new("move_to", "Move to").icon("folder_arrow_right").submenu(move_menu("", &sel.contacts)));
    }
    items.push(MenuItem::divider());
    let export_label = if sel.len() == 1 && sel.lists.len() == 1 { "Export contact list (.vcf)" } else { "Export contact (.vcf)" };
    items.push(MenuItem::new("export_selection", export_label).icon("arrow_export"));
    items.push(MenuItem::divider());
    items.push(MenuItem::new("delete", "Delete").icon("delete").danger());
    items
}

/// Export the selected contacts/lists as one `.vcf` in Downloads.
pub fn export_selection(ctx: &PeopleCtx, app: AppState) {
    let sel = Selection::read(ctx);
    if !sel.deletable() {
        return;
    }
    let stem = if sel.len() == 1 {
        sel.contacts.first().map(|c| c.display_name.clone()).or_else(|| sel.lists.first().map(|l| l.name.clone())).unwrap_or_default()
    } else {
        "contacts".into()
    };
    let text = Store::contacts_to_vcard(&sel.contacts, &sel.lists, true);
    match io::save_download(if stem.trim().is_empty() { "contact" } else { &stem }, "vcf", &text) {
        Ok(path) => {
            app.toast(format!("Exported to {}", io::display_path(&path)));
        }
        Err(e) => {
            app.error(format!("Couldn't export: {e}"));
        }
    }
}

/// Run a ribbon or context-menu command id.
pub fn run(ctx: PeopleCtx, app: AppState, id: &str, target: &MenuTarget) {
    let mut ctx = ctx;
    match target {
        MenuTarget::YourContacts => {
            if id == "new_folder" {
                ctx.contacts_expanded.set(true);
                ctx.new_folder.set(true);
            }
            return;
        }
        MenuTarget::Folder(name) => {
            match id {
                "rename_folder" => ctx.renaming.set(Some(name.clone())),
                "delete_folder" => ctx.open_dialog(PeopleDialog::ConfirmDeleteFolder(name.clone())),
                "new_contact_here" => {
                    ctx.set_node(Node::Folder(name.clone()));
                    new_contact(ctx);
                }
                _ => {}
            }
            return;
        }
        MenuTarget::Category(name) => {
            if id == "remove_category"
                && let Some(n) = app.check(store().remove_category_from_contacts(name))
            {
                app.toast(format!("Removed {name} from {n} contact{}", if n == 1 { "" } else { "s" }));
                app.bump_people();
                if *ctx.node.peek() == Node::Category(name.clone()) {
                    ctx.set_node(Node::AllContacts);
                }
            }
            return;
        }
        MenuTarget::Selection => {}
    }
    let sel = || Selection::read(&ctx);
    match id {
        "new_contact" => new_contact(ctx),
        "new_list" => new_list(ctx, Vec::new()),
        "edit" => edit(ctx),
        "delete" => ask_delete(ctx),
        "favorite" => toggle_favorite(ctx, app),
        "email" => email(app, sel().recipients()),
        "chat" => match sel().chat_uri() {
            Ok(uri) => {
                if let Err(e) = io::open_uri(&uri) {
                    app.error(format!("Couldn't open the chat app: {e}"));
                }
            }
            Err(why) => {
                app.toast(why);
            }
        },
        "import" => ctx.open_dialog(PeopleDialog::Import),
        "export" => ctx.open_dialog(PeopleDialog::Export),
        "export_selection" => export_selection(&ctx, app),
        "shortcuts" => ctx.open_dialog(PeopleDialog::Shortcuts),
        "toggle_nav" => ctx.toggle_nav(),
        "toggle_suggested" => {
            let on = !*ctx.suggested_on.peek();
            ctx.set_suggested(on);
            if on {
                ctx.set_node(Node::Suggested);
            }
        }
        "add_suggested" => {
            if let Some(addr) = sel().suggested.first() {
                add_suggested(ctx, addr);
            }
        }
        "dismiss_suggested" => {
            for a in sel().suggested {
                app.check(store().dismiss_suggested_person(&a));
            }
            ctx.clear_selection();
            app.bump_people();
        }
        "list:new" => {
            let members = sel().with_email();
            new_list(ctx, members);
        }
        _ => {
            if let Some(sort) = id.strip_prefix("sort:") {
                ctx.set_sort(SortBy::from_key(sort));
            } else if let Some(order) = id.strip_prefix("display:") {
                ctx.set_last_first(order == "last");
            } else if let Some(d) = id.strip_prefix("density:") {
                let density = match d {
                    "roomy" => Density::Roomy,
                    "compact" => Density::Compact,
                    _ => Density::Cozy,
                };
                app.update_settings(|s| s.density = density);
            } else if let Some(list) = id.strip_prefix("list:").and_then(|v| v.parse::<ContactListId>().ok()) {
                toggle_list_membership(app, list, &sel().with_email());
            } else if let Some(cat) = id.strip_prefix("cat:") {
                let ids = ctx.selected_contacts();
                if cat == "new" {
                    ctx.open_dialog(PeopleDialog::NewCategory(ids));
                } else if !ids.is_empty() && app.check(store().toggle_contact_category(&ids, cat)).is_some() {
                    app.bump_people();
                }
            } else if let Some(folder) = id.strip_prefix("move:") {
                move_to_folder(app, &ctx.selected_contacts(), folder);
            }
        }
    }
}

/// Move contacts to a folder ("" = Contacts) and say so.
pub fn move_to_folder(app: AppState, ids: &[ContactId], folder: &str) {
    if !ids.is_empty() && app.check(store().move_contacts(ids, folder)).is_some() {
        app.toast(format!("Moved to {}", if folder.is_empty() { "Contacts" } else { folder }));
        app.bump_people();
    }
}

/// Open the New contact form for a "Suggested" person.
pub fn add_suggested(ctx: PeopleCtx, address: &str) {
    let found = ctx.model.peek().rows.iter().find_map(|r| match r {
        Row::Suggested(a, _) if a.address.eq_ignore_ascii_case(address) => Some((**a).clone()),
        _ => None,
    });
    let a = found.unwrap_or_else(|| EmailAddress::new("", address));
    ctx.open_dialog(PeopleDialog::NewContact(Box::new(prefill_from_address(&a))));
}

/// A new contact pre-filled from a name and address ("Add to contacts").
pub fn prefill_from_address(a: &EmailAddress) -> Contact {
    let mut parts = a.name.split_whitespace();
    let given = parts.next().unwrap_or_default().to_string();
    let surname = parts.collect::<Vec<_>>().join(" ");
    Contact {
        given_name: given,
        surname,
        emails: vec![LabeledValue { label: "Work".into(), value: a.address.clone() }],
        ..Default::default()
    }
}

/// Start time for "Schedule meeting": the next half hour.
pub fn next_half_hour() -> (chrono::DateTime<Utc>, chrono::DateTime<Utc>) {
    let now = Local::now();
    let add = 30 - (now.minute() % 30) as i64;
    let start = (now + Duration::minutes(add)).with_second(0).and_then(|t| t.with_nanosecond(0)).unwrap_or(now);
    let start = start.with_timezone(&Utc);
    (start, start + Duration::minutes(30))
}

/// Open the calendar's new-event form at the next half hour.
pub fn schedule_meeting(app: AppState) {
    let (start, end) = next_half_hour();
    app.navigate(NavRequest::NewEvent { start, end, all_day: false });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefill_splits_names() {
        let c = prefill_from_address(&EmailAddress::new("Ada King Lovelace", "ada@example.com"));
        assert_eq!((c.given_name.as_str(), c.surname.as_str()), ("Ada", "King Lovelace"));
        assert_eq!(c.primary_email(), Some("ada@example.com"));
        let (s, e) = next_half_hour();
        assert_eq!((e - s).num_minutes(), 30);
        assert!(s > Utc::now());
        assert_eq!(s.with_timezone(&Local).minute() % 30, 0);
    }

    #[test]
    fn selection_recipients_dedupe_and_expand_lists() {
        let sel = Selection {
            contacts: vec![Contact {
                display_name: "Ada".into(),
                emails: vec![LabeledValue { label: "Work".into(), value: "ada@example.com".into() }],
                ..Default::default()
            }],
            lists: vec![ContactList {
                name: "L".into(),
                members: vec![EmailAddress::new("Ada", "ADA@example.com"), EmailAddress::new("Grace", "grace@example.com")],
                ..Default::default()
            }],
            suggested: Vec::new(),
        };
        let r: Vec<_> = sel.recipients().into_iter().map(|a| a.address).collect();
        assert_eq!(r, ["ada@example.com", "grace@example.com"]);
        assert!(!sel.single_editable());
        assert_eq!(sel.chat_uri(), Err("Select one contact to chat with"));
    }
}
