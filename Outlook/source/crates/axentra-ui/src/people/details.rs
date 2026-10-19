//! Details pane: one contact, one contact list, a multi-selection, or a
//! Suggested person (docs/spec/people-todo.md 2.7).

use std::rc::Rc;

use axentra_core::*;
use axentra_core::Store;
use chrono::{Datelike, Duration, Local, Utc};
use dioxus::prelude::*;

use super::actions::{self, Selection};
use super::avatar::{PeopleAvatar, contact_color_key, list_color_key};
use super::ctx::{MenuTarget, PeopleCtx, PeopleDialog};
use super::form::InlineEditor;
use super::io;
use super::view::{Item, Node};
use crate::kit::{EmptyState, Icon, IconButton, Menu, MenuButton, MenuItem};
use crate::state::{AppState, NavRequest, store, use_app};

#[component]
pub fn DetailsPane() -> Element {
    let app = use_app();
    let _ = app.people_rev();
    let ctx = use_context::<PeopleCtx>();
    let selection = ctx.selection.read().clone();
    let editing = *ctx.editing.read();
    let content = match selection.as_slice() {
        [] => rsx! {
            EmptyState { icon: "contact_card", title: "Select a contact", subtitle: "Their details show up here." }
        },
        [Item::Contact(id)] if editing == Some(*id) => rsx! { InlineEditor { key: "edit-{id}", id: *id } },
        [Item::Contact(id)] => rsx! { ContactCard { key: "c-{id}", id: *id } },
        [Item::List(id)] => rsx! { ListCard { key: "l-{id}", id: *id } },
        [Item::Suggested(a)] => rsx! { SuggestedCard { address: a.clone() } },
        _ => rsx! { MultiCard {} },
    };
    rsx! {
        main {
            id: "ax-people-details",
            class: "ax-people-details ax-pane ax-grow",
            "aria-label": "Contact details",
            if !selection.is_empty() {
                div { class: "ax-people-back",
                    button {
                        r#type: "button",
                        class: "ax-btn ax-subtle",
                        onclick: move |_| { ctx.select_guarded(None, false); },
                        Icon { name: "arrow_left" }
                        span { "Back" }
                    }
                }
            }
            {content}
        }
    }
}

/// A 64 x 56 action button (icon above label). Disabled buttons stay
/// focusable with `aria-disabled` so the tooltip can be read (2.7.2).
#[component]
fn ActionButton(
    #[props(into)] label: String,
    #[props(into)] icon: String,
    #[props(into)] tooltip: String,
    #[props(default)] disabled: bool,
    #[props(default)] menu: Vec<MenuItem>,
    on_select: EventHandler<String>,
) -> Element {
    let mut menu_at = use_signal(|| None::<(f64, f64)>);
    let mut anchor = use_signal(|| None::<Rc<MountedData>>);
    let has_menu = !menu.is_empty();
    rsx! {
        button {
            r#type: "button",
            class: "ax-people-action",
            class: if disabled { "ax-disabled" },
            title: "{tooltip}",
            "aria-label": "{label}",
            "aria-disabled": if disabled { "true" } else { "false" },
            "aria-haspopup": if has_menu { "menu" } else { "false" },
            onmounted: move |e| anchor.set(Some(e.data())),
            onclick: move |e: MouseEvent| {
                if disabled {
                    return;
                }
                if has_menu {
                    let fallback = e.client_coordinates();
                    spawn(async move {
                        let rect = match anchor() {
                            Some(m) => m.get_client_rect().await.ok(),
                            None => None,
                        };
                        menu_at.set(Some(rect.map(|r| (r.min_x(), r.max_y() + 4.0)).unwrap_or((fallback.x, fallback.y))));
                    });
                } else {
                    on_select.call(String::new());
                }
            },
            Icon { name: icon.clone() }
            span { "{label}" }
        }
        if let Some((x, y)) = menu_at() {
            Menu { items: menu.clone(), x, y, on_select: move |id| on_select.call(id), on_close: move |_| menu_at.set(None) }
        }
    }
}

/// A label above a value, with a copy button on hover/focus (2.7.3).
#[component]
fn Field(#[props(into)] label: String, #[props(into)] value: String, #[props(default)] link: FieldLink) -> Element {
    let app = use_app();
    let copy_value = value.clone();
    let open_value = value.clone();
    let body = match link {
        FieldLink::None => rsx! { span { class: "ax-people-field-value ax-selectable", "{value}" } },
        FieldLink::Email => rsx! {
            button {
                r#type: "button",
                class: "ax-people-field-value ax-people-field-link",
                title: "Send email",
                onclick: move |_| actions::email(app, vec![EmailAddress::new("", open_value.clone())]),
                "{value}"
            }
        },
        FieldLink::Phone => rsx! {
            button {
                r#type: "button",
                class: "ax-people-field-value ax-people-field-link",
                title: if io::has_handler("tel") { "Call" } else { "No calling app available" },
                onclick: move |_| open_or_report(app, &io::tel_uri(&open_value), "tel"),
                "{value}"
            }
        },
        FieldLink::Web => {
            let href = web_href(&value);
            match href {
                Some(href) => rsx! {
                    a { class: "ax-people-field-value ax-people-field-link", href: "{href}", target: "_blank", rel: "noopener noreferrer", "{value}" }
                },
                None => rsx! { span { class: "ax-people-field-value ax-selectable", "{value}" } },
            }
        }
        FieldLink::Im => rsx! {
            button {
                r#type: "button",
                class: "ax-people-field-value ax-people-field-link",
                title: "Chat",
                onclick: move |_| {
                    let uri = io::im_uri(&open_value);
                    let scheme = io::scheme(&uri).unwrap_or_default();
                    open_or_report(app, &uri, &scheme);
                },
                "{value}"
            }
        },
    };
    rsx! {
        div { class: "ax-people-field",
            div { class: "ax-people-field-text",
                span { class: "ax-people-field-label", "{label}" }
                {body}
            }
            IconButton {
                icon: "copy",
                title: "Copy {label}",
                small: true,
                class: "ax-people-copy",
                onclick: move |_| {
                    io::copy_to_clipboard(&copy_value);
                    app.toast("Copied to clipboard");
                },
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum FieldLink {
    #[default]
    None,
    Email,
    Phone,
    Web,
    Im,
}

/// Websites open in the browser only for http(s) (2.7.3); bare domains get https.
fn web_href(value: &str) -> Option<String> {
    let v = value.trim();
    match io::scheme(v).as_deref() {
        Some("http") | Some("https") => Some(v.to_string()),
        None if v.contains('.') && !v.contains(' ') => Some(format!("https://{v}")),
        _ => None,
    }
}

fn open_or_report(app: AppState, uri: &str, scheme: &str) {
    if !io::has_handler(scheme) {
        app.error(match scheme {
            "tel" => "No calling app available".to_string(),
            _ => "No chat app available".to_string(),
        });
        return;
    }
    if let Err(e) = io::open_uri(uri) {
        app.error(format!("Couldn't open {uri}: {e}"));
    }
}

fn birthday_text(d: chrono::NaiveDate) -> String {
    if d.year() == Store::birthday_no_year() { d.format("%B %-d").to_string() } else { d.format("%B %-d, %Y").to_string() }
}

fn address_text(a: &PostalAddress) -> String {
    let line2 = [a.city.as_str(), a.state.as_str(), a.postal_code.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" ");
    [a.street.as_str(), line2.as_str(), a.country.as_str()].iter().filter(|s| !s.trim().is_empty()).copied().collect::<Vec<_>>().join("\n")
}

#[component]
fn ContactCard(id: ContactId) -> Element {
    let app = use_app();
    let _ = app.people_rev();
    let _ = app.mail_rev();
    let _ = app.calendar_rev();
    let ctx = use_context::<PeopleCtx>();
    let mut notes_open = use_signal(|| false);
    let Some(c) = store().contact(id).ok().flatten() else {
        return rsx! { EmptyState { icon: "contact_card", title: "This contact was deleted" } };
    };
    let name = super::view::display_name(&c, false);
    let org = [c.company.as_str(), c.department.as_str()].iter().filter(|s| !s.trim().is_empty()).copied().collect::<Vec<_>>().join(" · ");
    let emails: Vec<String> = c.emails.iter().map(|e| e.value.clone()).collect();

    // Email: a menu when there are several addresses.
    let email_menu: Vec<MenuItem> =
        if c.emails.len() > 1 { c.emails.iter().map(|e| MenuItem::new(e.value.clone(), e.value.clone()).icon("mail")).collect() } else { Vec::new() };
    let first_email = c.primary_email().map(str::to_string);
    let display_for_mail = c.display_name.clone();
    // Call: a menu with "Mobile" / "Work" / "Home" when there are several numbers.
    let has_tel = io::has_handler("tel");
    let call_menu: Vec<MenuItem> = if c.phones.len() > 1 {
        c.phones.iter().map(|p| MenuItem::new(p.value.clone(), format!("{} {}", p.label, p.value)).icon("call")).collect()
    } else {
        Vec::new()
    };
    let first_phone = c.phones.first().map(|p| p.value.clone());
    let (call_disabled, call_tip) = match (&first_phone, has_tel) {
        (None, _) => (true, "No phone number".to_string()),
        (Some(_), false) => (true, "No calling app available".to_string()),
        (Some(p), true) => (false, format!("Call {p}")),
    };
    let chat = actions::chat_uri(&c);

    let fav = c.is_favorite;
    let more = {
        let with_email: Vec<EmailAddress> = c.primary_email().map(|e| EmailAddress::new(c.display_name.clone(), e)).into_iter().collect();
        vec![
            MenuItem::new("add_to_list", "Add to list")
                .icon("people_list")
                .disabled(with_email.is_empty())
                .submenu(actions::add_to_list_menu("", &with_email)),
            MenuItem::new("categorize", "Categorize").icon("tag").submenu(actions::category_menu("", std::slice::from_ref(&c))),
            MenuItem::new("export_selection", "Export contact (.vcf)").icon("arrow_export"),
            MenuItem::divider(),
            MenuItem::new("delete", "Delete").icon("delete").danger(),
        ]
    };

    // Contact information, in the order of 2.7.3.
    let mut info: Vec<(String, String, FieldLink)> = Vec::new();
    for e in &c.emails {
        info.push(("Email".into(), e.value.clone(), FieldLink::Email));
    }
    for (label, kind) in [("Mobile", "Mobile"), ("Work phone", "Work"), ("Home phone", "Home")] {
        for p in c.phones.iter().filter(|p| p.label == kind) {
            info.push((label.into(), p.value.clone(), FieldLink::Phone));
        }
    }
    for p in c.phones.iter().filter(|p| !["Mobile", "Work", "Home"].contains(&p.label.as_str())) {
        info.push(("Phone".into(), p.value.clone(), FieldLink::Phone));
    }
    for im in &c.im_addresses {
        info.push(("IM address".into(), im.clone(), FieldLink::Im));
    }
    for (label, kind) in [("Home address", "Home"), ("Work address", "Work"), ("Other address", "Other")] {
        for a in c.addresses.iter().filter(|a| a.label == kind) {
            info.push((label.into(), address_text(a), FieldLink::None));
        }
    }
    if let Some(b) = c.birthday {
        info.push(("Birthday".into(), birthday_text(b), FieldLink::None));
    }
    for w in &c.websites {
        info.push(("Website".into(), w.clone(), FieldLink::Web));
    }
    for (label, v) in [("Nickname", &c.nickname), ("Spouse/partner", &c.more.spouse), ("Children", &c.more.children)] {
        if !v.trim().is_empty() {
            info.push((label.into(), v.clone(), FieldLink::None));
        }
    }
    let org_fields: Vec<(&str, String)> = [
        ("Company", &c.company),
        ("Job title", &c.job_title),
        ("Department", &c.department),
        ("Office location", &c.office_location),
        ("Manager", &c.more.manager),
        ("Assistant", &c.more.assistant),
        ("Profession", &c.more.profession),
    ]
    .into_iter()
    .filter(|(_, v)| !v.trim().is_empty())
    .map(|(l, v)| (l, v.clone()))
    .collect();
    let long_notes = c.notes.lines().count() > 6 || c.notes.len() > 420;

    let recent = store().recent_messages_with(&emails, 5).unwrap_or_default();
    let meetings = upcoming_meetings(&emails);
    let folder_label = if c.folder.is_empty() { "Contacts".to_string() } else { c.folder.clone() };
    let lists: Vec<ContactList> = store()
        .contact_lists()
        .unwrap_or_default()
        .into_iter()
        .filter(|l| l.members.iter().any(|m| emails.iter().any(|e| e.eq_ignore_ascii_case(&m.address))))
        .collect();

    rsx! {
        div { class: "ax-people-card",
            div { class: "ax-people-card-header",
                PeopleAvatar { name: c.display_name.clone(), color_key: contact_color_key(&c), size: 96, photo: c.photo.clone() }
                div { class: "ax-people-card-titles",
                    h1 { class: "ax-people-card-name ax-selectable", "{name}" }
                    if !c.job_title.trim().is_empty() { div { class: "ax-people-card-job ax-selectable", "{c.job_title}" } }
                    if !org.is_empty() { div { class: "ax-people-card-org ax-selectable", "{org}" } }
                    if !c.categories.is_empty() {
                        div { class: "ax-people-card-cats",
                            for cat in c.categories.iter() {
                                crate::kit::CategoryPill { key: "{cat}", name: cat.clone(), color: category_color(cat) }
                            }
                        }
                    }
                }
                div { class: "ax-people-card-tools",
                    button {
                        r#type: "button",
                        class: "ax-btn ax-subtle",
                        title: "Edit contact (Shift+E)",
                        onclick: move |_| actions::edit(ctx),
                        Icon { name: "edit" }
                        span { "Edit" }
                    }
                    IconButton {
                        icon: if fav { "star_filled" } else { "star" },
                        title: if fav { "Remove from favorites" } else { "Add to favorites" },
                        toggled: fav,
                        onclick: move |_| actions::set_favorite(app, &[id], &[], !fav),
                    }
                    MenuButton {
                        label: "More options",
                        icon: "more_horizontal",
                        icon_only: true,
                        items: more,
                        on_select: move |cmd: String| actions::run(ctx, app, &cmd, &MenuTarget::Selection),
                    }
                }
            }
            div { class: "ax-people-actions", role: "toolbar", "aria-label": "Contact actions",
                ActionButton {
                    label: "Email",
                    icon: "mail",
                    tooltip: if first_email.is_some() { "Send email" } else { "No email address" },
                    disabled: first_email.is_none(),
                    menu: email_menu,
                    on_select: move |addr: String| {
                        let to = if addr.is_empty() { first_email.clone().unwrap_or_default() } else { addr };
                        actions::email(app, vec![EmailAddress::new(display_for_mail.clone(), to)]);
                    },
                }
                ActionButton {
                    label: "Chat",
                    icon: "chat",
                    tooltip: match &chat { Ok(_) => "Chat".to_string(), Err(why) => why.to_string() },
                    disabled: chat.is_err(),
                    on_select: move |_| {
                        if let Ok(uri) = &chat && let Err(e) = io::open_uri(uri) {
                            app.error(format!("Couldn't open the chat app: {e}"));
                        }
                    },
                }
                ActionButton {
                    label: "Call",
                    icon: "call",
                    tooltip: call_tip,
                    disabled: call_disabled,
                    menu: if has_tel { call_menu } else { Vec::new() },
                    on_select: move |number: String| {
                        let n = if number.is_empty() { first_phone.clone().unwrap_or_default() } else { number };
                        open_or_report(app, &io::tel_uri(&n), "tel");
                    },
                }
                ActionButton { label: "Video", icon: "video", tooltip: "No video app available", disabled: true, on_select: move |_| {} }
                ActionButton { label: "Meeting", icon: "calendar_add", tooltip: "Schedule a meeting", on_select: move |_| actions::schedule_meeting(app) }
            }
            if !info.is_empty() {
                Section { title: "Contact information",
                    for (i, (label, value, link)) in info.into_iter().enumerate() {
                        Field { key: "{i}", label, value, link }
                    }
                }
            }
            if !org_fields.is_empty() {
                Section { title: "Organization",
                    for (label, value) in org_fields {
                        Field { key: "{label}", label, value }
                    }
                }
            }
            if !c.notes.trim().is_empty() {
                Section { title: "Notes",
                    div {
                        class: "ax-people-notes ax-selectable",
                        class: if long_notes && !notes_open() { "ax-clamped" },
                        "{c.notes}"
                    }
                    if long_notes {
                        button { r#type: "button", class: "ax-people-link", onclick: move |_| notes_open.set(!notes_open()),
                            if notes_open() { "Show less" } else { "Show more" }
                        }
                    }
                }
            }
            if !lists.is_empty() {
                Section { title: "Contact lists",
                    for l in lists {
                        {
                            let lid = l.id;
                            rsx! {
                                button {
                                    key: "{l.id}",
                                    r#type: "button",
                                    class: "ax-people-mini-row",
                                    onclick: move |_| open_item(ctx, Item::List(lid)),
                                    PeopleAvatar { name: l.name.clone(), color_key: list_color_key(&l), size: 24, square: true, fallback_icon: "people" }
                                    span { class: "ax-ellipsis", "{l.name}" }
                                }
                            }
                        }
                    }
                }
            }
            if !meetings.is_empty() {
                Section { title: "Upcoming meetings",
                    for (eid, title, when) in meetings {
                        button {
                            key: "{eid}-{when}",
                            r#type: "button",
                            class: "ax-people-mini-row ax-two-line",
                            onclick: move |_| app.navigate(NavRequest::Event(eid)),
                            Icon { name: "calendar_ltr", size: 16 }
                            span { class: "ax-people-mini-text",
                                span { class: "ax-ellipsis", "{title}" }
                                span { class: "ax-people-mini-sub ax-ellipsis", "{when}" }
                            }
                        }
                    }
                }
            }
            if !recent.is_empty() {
                Section { title: "Recent emails",
                    for m in recent {
                        {
                            let mid = m.id;
                            let who = if emails.iter().any(|e| e.eq_ignore_ascii_case(&m.from.address)) {
                                format!("From {}", m.from.display())
                            } else {
                                format!("To {}", m.to.iter().map(|a| a.display().to_string()).collect::<Vec<_>>().join(", "))
                            };
                            let when = crate::fmt::list_time(m.received_at, Local::now(), app.settings.read().use_24h_time);
                            let subject = if m.subject.trim().is_empty() { "(No subject)".to_string() } else { m.subject.clone() };
                            rsx! {
                                button {
                                    key: "{mid}",
                                    r#type: "button",
                                    class: "ax-people-mini-row ax-two-line",
                                    onclick: move |_| app.navigate(NavRequest::Message(mid)),
                                    Icon { name: "mail", size: 16 }
                                    span { class: "ax-people-mini-text",
                                        span { class: "ax-ellipsis", "{subject}" }
                                        span { class: "ax-people-mini-sub ax-ellipsis", "{who} · {when}" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            div { class: "ax-people-source", "Saved in {folder_label} · On this device" }
        }
    }
}

fn category_color(name: &str) -> String {
    store().categories().unwrap_or_default().into_iter().find(|c| c.name == name).map(|c| c.color).unwrap_or_else(|| "gray".into())
}

/// The next meetings (60 days) that include one of these addresses.
fn upcoming_meetings(emails: &[String]) -> Vec<(EventId, String, String)> {
    if emails.is_empty() {
        return Vec::new();
    }
    let now = Utc::now();
    let has = |a: &EmailAddress| emails.iter().any(|e| e.eq_ignore_ascii_case(&a.address));
    store()
        .events_in_range(now, now + Duration::days(60))
        .unwrap_or_default()
        .into_iter()
        .filter(|o| o.event.organizer.as_ref().is_some_and(has) || o.event.attendees.iter().any(|a| has(&a.email)))
        .take(3)
        .map(|o| {
            let l = o.start.with_timezone(&Local);
            let when = if o.event.all_day { l.format("%a, %b %-d").to_string() } else { l.format("%a, %b %-d, %-I:%M %p").to_string() };
            (o.event.id, o.event.title.clone(), when)
        })
        .collect()
}

#[component]
fn Section(#[props(into)] title: String, children: Element) -> Element {
    rsx! {
        section { class: "ax-people-section", "aria-label": "{title}",
            h3 { class: "ax-people-section-title", "{title}" }
            div { class: "ax-people-section-body", {children} }
        }
    }
}

/// Select an item, switching to a view that shows it when needed.
/// Checked against the store, not the list model, which may not have been
/// rebuilt yet for an item that was just created.
pub fn open_item(ctx: PeopleCtx, item: Item) {
    let mut ctx = ctx;
    let node = ctx.node.peek().clone();
    let filter = ctx.filter.peek().clone();
    let (shown, matches, fallback) = match &item {
        Item::Contact(id) => match store().contact(*id).ok().flatten() {
            Some(c) => (node.shows_contact(&c), super::view::contact_matches(&c, &filter), Node::AllContacts),
            None => (false, true, Node::AllContacts),
        },
        Item::List(id) => match store().contact_list(*id).ok().flatten() {
            Some(l) => (node.shows_list(&l), filter.trim().is_empty() || super::view::list_matches(&l, &filter), Node::AllLists),
            None => (false, true, Node::AllLists),
        },
        Item::Suggested(_) => (node == Node::Suggested, true, Node::Suggested),
    };
    if !matches {
        ctx.filter.set(String::new());
    }
    if !shown {
        ctx.set_node(fallback);
    }
    ctx.select_guarded(Some(item), false);
}

#[component]
fn ListCard(id: ContactListId) -> Element {
    let app = use_app();
    let _ = app.people_rev();
    let ctx = use_context::<PeopleCtx>();
    let Some(l) = store().contact_list(id).ok().flatten() else {
        return rsx! { EmptyState { icon: "people_team", title: "This contact list was deleted" } };
    };
    let n = l.members.len();
    let members_label = format!("{n} member{}", if n == 1 { "" } else { "s" });
    let members = l.members.clone();
    let fav = l.is_favorite;
    rsx! {
        div { class: "ax-people-card",
            div { class: "ax-people-card-header",
                PeopleAvatar { name: l.name.clone(), color_key: list_color_key(&l), size: 96, square: true, fallback_icon: "people" }
                div { class: "ax-people-card-titles",
                    h1 { class: "ax-people-card-name ax-selectable", "{l.name}" }
                    div { class: "ax-people-card-job", "{members_label}" }
                    if !l.description.trim().is_empty() { div { class: "ax-people-card-org ax-selectable", "{l.description}" } }
                }
                div { class: "ax-people-card-tools",
                    IconButton {
                        icon: if fav { "star_filled" } else { "star" },
                        title: if fav { "Remove from favorites" } else { "Add to favorites" },
                        toggled: fav,
                        onclick: move |_| actions::set_favorite(app, &[], &[id], !fav),
                    }
                }
            }
            div { class: "ax-people-actions", role: "toolbar", "aria-label": "Contact list actions",
                ActionButton {
                    label: "Email",
                    icon: "mail",
                    tooltip: if n == 0 { "This list has no members" } else { "Email everyone in this list" },
                    disabled: n == 0,
                    on_select: move |_| actions::email(app, members.clone()),
                }
                ActionButton { label: "Edit", icon: "edit", tooltip: "Edit contact list (Shift+E)", on_select: move |_| ctx.open_dialog(PeopleDialog::EditList(Some(id), Vec::new())) }
                ActionButton {
                    label: "Delete",
                    icon: "delete",
                    tooltip: "Delete contact list",
                    on_select: move |_| ctx.open_dialog(PeopleDialog::ConfirmDelete { contacts: Vec::new(), lists: vec![id] }),
                }
            }
            Section { title: "Members",
                if l.members.is_empty() {
                    div { class: "ax-people-muted", "No members yet. Choose Edit to add people." }
                }
                for m in l.members.iter().cloned() {
                    {
                        let saved = store().contact_by_email(&m.address).ok().flatten();
                        let name = saved.as_ref().map(|c| c.display_name.clone()).filter(|s| !s.is_empty()).unwrap_or_else(|| m.display().to_string());
                        let cid = saved.as_ref().map(|c| c.id);
                        let photo = saved.as_ref().and_then(|c| c.photo.clone());
                        rsx! {
                            button {
                                key: "{m.address}",
                                r#type: "button",
                                class: "ax-people-member",
                                class: if cid.is_none() { "ax-unsaved" },
                                title: if cid.is_some() { "Open contact" } else { "Not saved as a contact" },
                                onclick: move |_| match cid {
                                    Some(cid) => open_item(ctx, Item::Contact(cid)),
                                    None => actions::email(app, vec![m.clone()]),
                                },
                                PeopleAvatar { name: name.clone(), color_key: m.address.to_lowercase(), size: 32, photo }
                                span { class: "ax-people-mini-text",
                                    span { class: "ax-ellipsis", "{name}" }
                                    span { class: "ax-people-mini-sub ax-ellipsis", "{m.address}" }
                                }
                            }
                        }
                    }
                }
            }
            div { class: "ax-people-source", "On this device" }
        }
    }
}

#[component]
fn MultiCard() -> Element {
    let app = use_app();
    let _ = app.people_rev();
    let ctx = use_context::<PeopleCtx>();
    let sel = Selection::read(&ctx);
    let n = sel.len();
    let noun = if sel.lists.is_empty() { "contacts" } else { "items" };
    let fav = sel.all_favorite();
    let with_email = sel.with_email();
    let recipients = sel.recipients();
    rsx! {
        div { class: "ax-people-multi",
            div { class: "ax-people-multi-title", "{n} {noun} selected" }
            div { class: "ax-people-actions ax-people-multi-actions", role: "toolbar", "aria-label": "Selection actions",
                ActionButton {
                    label: "Email",
                    icon: "mail",
                    tooltip: if recipients.is_empty() { "None of these have an email address" } else { "Email everyone selected" },
                    disabled: recipients.is_empty(),
                    on_select: move |_| actions::email(app, recipients.clone()),
                }
                ActionButton {
                    label: "Add to list",
                    icon: "people_list",
                    tooltip: if with_email.is_empty() { "Contacts without an email address can't be added to a list" } else { "Add to list" },
                    disabled: with_email.is_empty(),
                    menu: actions::add_to_list_menu("", &with_email),
                    on_select: move |id: String| actions::run(ctx, app, &id, &MenuTarget::Selection),
                }
                ActionButton {
                    label: if fav { "Remove from favorites" } else { "Add to favorites" },
                    icon: if fav { "star_off" } else { "star" },
                    tooltip: if fav { "Remove from favorites" } else { "Add to favorites" },
                    disabled: !sel.deletable(),
                    on_select: move |_| actions::toggle_favorite(ctx, app),
                }
                ActionButton {
                    label: "Categorize",
                    icon: "tag",
                    tooltip: if sel.contacts.is_empty() { "Select contacts to categorize" } else { "Categorize" },
                    disabled: sel.contacts.is_empty(),
                    menu: actions::category_menu("", &sel.contacts),
                    on_select: move |id: String| actions::run(ctx, app, &id, &MenuTarget::Selection),
                }
                ActionButton { label: "Delete", icon: "delete", tooltip: "Delete", disabled: !sel.deletable(), on_select: move |_| actions::ask_delete(ctx) }
            }
        }
    }
}

#[component]
fn SuggestedCard(address: String) -> Element {
    let app = use_app();
    let ctx = use_context::<PeopleCtx>();
    let model = ctx.model.read().clone();
    let found = model.rows.iter().find_map(|r| match r {
        super::view::Row::Suggested(a, n) if a.address.eq_ignore_ascii_case(&address) => Some(((**a).clone(), *n)),
        _ => None,
    });
    let Some((person, n)) = found else { return rsx! {} };
    let count_label = format!("{n} email{} in the last 180 days", if n == 1 { "" } else { "s" });
    let add = address.clone();
    let mail_to = person.clone();
    rsx! {
        div { class: "ax-people-card",
            div { class: "ax-people-card-header",
                PeopleAvatar { name: person.display().to_string(), color_key: address.to_lowercase(), size: 96 }
                div { class: "ax-people-card-titles",
                    h1 { class: "ax-people-card-name ax-selectable", "{person.display()}" }
                    div { class: "ax-people-card-job ax-selectable", "{person.address}" }
                    div { class: "ax-people-card-org", "{count_label}" }
                }
            }
            div { class: "ax-people-actions", role: "toolbar", "aria-label": "Suggested contact actions",
                ActionButton { label: "Add", icon: "person_add", tooltip: "Add to contacts", on_select: move |_| actions::add_suggested(ctx, &add) }
                ActionButton { label: "Email", icon: "mail", tooltip: "Send email", on_select: move |_| actions::email(app, vec![mail_to.clone()]) }
            }
            div { class: "ax-people-source", "Suggested from your mail on this device" }
        }
    }
}
