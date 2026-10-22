//! Reading pane: the open conversation (or message), the multi-selection
//! summary, inline compose, and the empty state (docs/spec/mail.md §5).

pub mod attachments;
pub mod body;

use std::collections::HashSet;

use axentra_core::*;
use chrono::Local;
use dioxus::prelude::*;

pub use attachments::file_icon;

use super::actions;
use super::ctx::{MailCtx, Open, use_mail, use_model};
use super::menus::{self, Target};
use super::model::bare_subject;
use crate::kit::{Avatar, Icon, IconButton, Menu, MenuButton, MenuItem, SurfaceGrips, SurfaceSpec, dom, use_surface};
use crate::state::{AppState, NavRequest, store, use_app};
use attachments::AttachmentWell;
use body::BodyTheme;

/// True when the app currently renders dark.
pub fn is_dark(app: AppState, ctx: MailCtx) -> bool {
    match app.settings.read().theme {
        ThemeMode::Dark => true,
        ThemeMode::Light => false,
        ThemeMode::System => *ctx.system_dark.read(),
    }
}

#[component]
pub fn ReadingPane(#[props(default)] style: String, #[props(default)] full: bool) -> Element {
    let app = use_app();
    let mut ctx = use_mail();
    let compose = app.compose.read().clone().filter(|c| !c.popout);
    let sel = ctx.selection.read().len();
    let open = ctx.open.read().clone();
    // Focus requests (Enter in the list, F6): the scrolling part takes focus
    // so Home / End / Page Up / Page Down scroll the message.
    {
        let mut seen = use_signal(|| *ctx.focus_reading.peek());
        use_effect(move || {
            let req = *ctx.focus_reading.read();
            if req != *seen.peek() {
                seen.set(req);
                document::eval(
                    "setTimeout(() => { const p = document.querySelector('.ax-mail-reading'); \
                     (p && (p.querySelector('.ax-mail-reading-scroll') || p))?.focus({ preventScroll: true }); }, 0);",
                );
            }
        });
    }
    let content = if let Some(req) = compose {
        rsx! {
            div {
                class: "ax-mail-compose-host",
                // Keys typed in the compose form are not mail shortcuts.
                onkeydown: move |_| ctx.mark_typing(),
                crate::mail::compose::ComposePane { request: req }
            }
        }
    } else if sel >= 2 {
        rsx! { SelectionSummary { count: sel } }
    } else if let Some(open) = open {
        let k = format!("{}-{}", open.message, open.conversation.clone().unwrap_or_default());
        rsx! { ConversationView { key: "{k}", open, full, window: false } }
    } else {
        rsx! {
            div { class: "ax-mail-empty ax-mail-reading-empty",
                Icon { name: "mail_read", size: 96 }
                div { class: "ax-mail-empty-title", "Select an item to read" }
                div { class: "ax-mail-empty-body", "Nothing is selected." }
            }
        }
    };
    rsx! {
        section {
            class: "ax-mail-reading",
            class: if full { "ax-full" },
            role: "region",
            "aria-label": "Reading pane",
            "data-ax-region": "true",
            tabindex: "-1",
            style: "{style}",
            onresize: move |e: ResizeEvent| {
                if let Ok(size) = e.data().get_content_box_size()
                    && (size.width - *ctx.reading_width.peek()).abs() > 8.0
                {
                    ctx.reading_width.set(size.width);
                }
            },
            {content}
        }
    }
}

/// "{n} items selected" with bulk actions (§3.12).
#[component]
fn SelectionSummary(count: usize) -> Element {
    let app = use_app();
    let ctx = use_mail();
    let model = use_model();
    let m = model.read();
    let ids = actions::targets(ctx, &m);
    let msgs = store().messages_by_ids(&ids).unwrap_or_default();
    let settings = app.settings.read().clone();
    let account = msgs.first().map(|x| x.account_id);
    let folder = m.folder.as_ref().map(|f| f.id);
    let run = move |cmd: String| {
        let m = model.read();
        let ids = actions::targets(ctx, &m);
        actions::run(app, ctx, &m, &cmd, ids);
    };
    rsx! {
        div { class: "ax-mail-summary", role: "status",
            Icon { name: "mail_multiple", size: 96 }
            div { class: "ax-mail-empty-title", "{count} items selected" }
            div { class: "ax-mail-summary-actions",
                button { r#type: "button", class: "ax-btn", onclick: move |_| run("delete".into()), Icon { name: "delete" } span { "Delete" } }
                button { r#type: "button", class: "ax-btn", onclick: move |_| run("archive".into()), Icon { name: "archive" } span { "Archive" } }
                MenuButton {
                    label: "Move",
                    icon: "folder_arrow_right",
                    items: menus::move_items("move", account, folder, "Move to a different folder…"),
                    on_select: move |id: String| run(id),
                }
                button { r#type: "button", class: "ax-btn", onclick: move |_| run("read".into()), Icon { name: "mail_read" } span { "Mark as read" } }
                MenuButton {
                    label: "Flag",
                    icon: "flag",
                    items: menus::flag_items(&msgs, settings.week_start, &settings.work_days),
                    on_select: move |id: String| run(id),
                }
                MenuButton {
                    label: "Categorize",
                    icon: "tag",
                    items: menus::categorize_items(&msgs),
                    on_select: move |id: String| run(id),
                }
            }
            button { r#type: "button", class: "ax-mail-link", onclick: move |_| ctx.clear_selection(), "Deselect all" }
        }
    }
}

/// Folder roles whose messages are hidden from conversations shown elsewhere.
fn hidden_in_conversation(role: FolderRole, open_role: FolderRole) -> bool {
    matches!(role, FolderRole::Deleted | FolderRole::Junk) && role != open_role
}

/// Width a message body frame gets, for its height estimate: the reading
/// pane's (or the read window's) content width less the card padding.
fn frame_width(container: f64, window: bool) -> f64 {
    (container - if window { 78.0 } else { 72.0 }).max(240.0)
}

/// How long the read window's width has to stay put before the message
/// bodies are rebuilt for it (their height estimate follows the width).
const READ_WINDOW_SETTLE_MS: u64 = 150;

/// "Open in new window" (Shift+Enter): the conversation in a window over
/// the app, closed with Escape or its Close button. The window is resizable,
/// moves by its title bar, and remembers where it was ("read-window"). The
/// title bar carries the subject (the dialog's name) and the window's
/// commands, so the conversation below starts with the messages.
#[component]
pub fn ReadWindow(open: Open) -> Element {
    let app = use_app();
    let mut ctx = use_mail();
    let id = dom::use_unique_id("ax-mail-read-window");
    let title_id = format!("{id}-title");
    let surface = use_surface(&id, Some(SurfaceSpec::new("read-window", 900, Some(760)).min(Some(480), Some(360)).movable()));
    let k = format!("{}-{}", open.message, open.conversation.clone().unwrap_or_default());
    let subject = store().message(open.message).ok().flatten().map(|m| bare_subject(&m.subject).to_string()).unwrap_or_default();
    let subject = if subject.trim().is_empty() { "(No subject)".to_string() } else { subject };
    let dark = is_dark(app, ctx);
    let light_body = *ctx.light_body.read();
    // A width drag reports every frame; the bodies are rebuilt once it settles.
    let mut resize_gen = use_signal(|| 0u64);
    rsx! {
        // Keyed on the outer element: a key on a nested child is dropped in
        // release builds, and the window must remount when the message changes.
        div {
            key: "{k}",
            id: "{id}",
            class: "ax-mail-read-window ax-surface ax-surface-float",
            role: "dialog",
            "aria-labelledby": "{title_id}",
            // A press on the title bar focuses the window (kit/dom.js), so
            // Esc closes it.
            tabindex: "-1",
            style: surface.style(),
            onresize: move |e: ResizeEvent| {
                let Ok(size) = e.data().get_content_box_size() else { return };
                let generation = *resize_gen.peek() + 1;
                resize_gen.set(generation);
                if (size.width - *ctx.read_window_width.peek()).abs() <= 8.0 {
                    return;
                }
                spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(READ_WINDOW_SETTLE_MS)).await;
                    if *resize_gen.peek() == generation {
                        ctx.read_window_width.set(size.width);
                    }
                });
            },
            ..surface.attributes(),
            div { class: "ax-mail-read-window-bar", "data-ax-surface-move": "true",
                Icon { name: "mail", size: 16 }
                // A press on the subject moves the window, as on any title
                // bar, and brings focus into it when it was elsewhere.
                h2 { id: "{title_id}", class: "ax-mail-read-window-title", title: "{subject}", "{subject}" }
                if dark {
                    IconButton {
                        icon: if light_body { "weather_moon" } else { "weather_sunny" },
                        title: "Switch background",
                        toggled: light_body,
                        onclick: move |_| {
                            let v = !*ctx.light_body.peek();
                            ctx.light_body.set(v);
                        },
                    }
                }
                IconButton { icon: "print", title: "Print (Ctrl+P)", onclick: move |_| actions::print() }
                // As Esc does: focus goes back to the message list.
                IconButton {
                    icon: "dismiss",
                    title: "Close (Esc)",
                    onclick: move |_| {
                        ctx.popout.set(None);
                        ctx.request_list_focus();
                    },
                }
            }
            ConversationView { open, full: false, window: true }
            SurfaceGrips {}
        }
    }
}

/// DOM id of a message card or collapsed row in the reading pane.
fn card_dom_id(window: bool, id: MessageId) -> String {
    format!("ax-mail-card-{}{id}", if window { "w" } else { "" })
}

/// Bring a message of the open conversation into view.
fn scroll_to_card(window: bool, id: MessageId) {
    let dom = card_dom_id(window, id);
    document::eval(&format!(
        "setTimeout(() => document.getElementById('{dom}')?.scrollIntoView({{ block: 'start', behavior: 'smooth' }}), 30);"
    ));
}

#[component]
fn ConversationView(open: Open, full: bool, window: bool) -> Element {
    let app = use_app();
    let mut ctx = use_mail();
    let _ = app.mail_rev();
    let s = store();
    let settings = app.settings.read().clone();
    let folders = s.folders(None).unwrap_or_default();
    let role_of = |id: FolderId| folders.iter().find(|f| f.id == id).map(|f| f.role).unwrap_or_default();
    let Some(focus_msg) = s.message(open.message).ok().flatten() else {
        return rsx! {
            div { class: "ax-mail-empty ax-mail-reading-empty",
                Icon { name: "mail_read", size: 96 }
                div { class: "ax-mail-empty-title", "Select an item to read" }
                div { class: "ax-mail-empty-body", "Nothing is selected." }
            }
        };
    };
    let open_role = role_of(focus_msg.folder_id);
    let mut msgs: Vec<Message> = match &open.conversation {
        Some(c) => s.conversation(c).unwrap_or_default().into_iter().filter(|m| m.id == focus_msg.id || !hidden_in_conversation(role_of(m.folder_id), open_role)).collect(),
        None => vec![focus_msg.clone()],
    };
    if msgs.is_empty() {
        msgs.push(focus_msg.clone());
    }
    // Newest on top unless the user chose otherwise.
    msgs.sort_by_key(|m| std::cmp::Reverse(m.received_at));
    if settings.conversation_order == ConversationOrder::NewestOnBottom {
        msgs.reverse();
    }
    let initial: HashSet<MessageId> = {
        let mut set: HashSet<MessageId> = msgs.iter().filter(|m| !m.is_read && !m.is_draft).map(|m| m.id).collect();
        set.insert(focus_msg.id);
        set
    };
    let mut expanded = use_signal(|| initial);
    let mut show_more = use_signal(|| false);
    // Opening a message further down the conversation (a child row in the
    // list, a reply date link) scrolls to it.
    let first_id = msgs.first().map(|m| m.id);
    let focus_id = focus_msg.id;
    use_hook(move || {
        if first_id != Some(focus_id) {
            scroll_to_card(window, focus_id);
        }
    });
    let ids: Vec<MessageId> = msgs.iter().map(|m| m.id).collect();
    let responses = s.last_responses(&ids).unwrap_or_default();
    let subject = bare_subject(&focus_msg.subject).to_string();
    let subject = if subject.is_empty() { "(No subject)".to_string() } else { subject };
    let mut categories: Vec<String> = Vec::new();
    for m in &msgs {
        for c in &m.categories {
            if !categories.contains(c) {
                categories.push(c.clone());
            }
        }
    }
    let dark = is_dark(app, ctx);
    let light_body = *ctx.light_body.read();
    let newest_id = msgs.iter().max_by_key(|m| m.received_at).map(|m| m.id);
    let exp = expanded.read().clone();
    let more_open = show_more();
    let in_thread: HashSet<MessageId> = ids.iter().copied().collect();

    // Merge runs of more than three collapsed messages into "{n} more messages".
    enum Item {
        Card(Message),
        Collapsed(Message),
        More(usize),
    }
    let mut items: Vec<Item> = Vec::new();
    let mut run: Vec<Message> = Vec::new();
    let flush = |run: &mut Vec<Message>, items: &mut Vec<Item>| {
        if run.len() > 3 && !more_open {
            items.push(Item::Collapsed(run[0].clone()));
            items.push(Item::More(run.len() - 2));
            items.push(Item::Collapsed(run[run.len() - 1].clone()));
        } else {
            items.extend(run.drain(..).map(Item::Collapsed));
        }
        run.clear();
    };
    for m in msgs.iter().cloned() {
        if exp.contains(&m.id) {
            flush(&mut run, &mut items);
            items.push(Item::Card(m));
        } else {
            run.push(m);
        }
    }
    flush(&mut run, &mut items);
    let count = msgs.len();
    let open_window = {
        let m = focus_msg.clone();
        move |_| actions::open_window(app, ctx, &m)
    };

    rsx! {
        div { class: "ax-mail-reading-scroll", tabindex: "-1", "data-ax-default-focus": if !window { "true" },
            // The read window shows the subject and these commands in its title bar.
            if !window {
                div { class: "ax-mail-reading-head",
                    if full {
                        IconButton { icon: "arrow_left", title: "Back to the message list", onclick: move |_| ctx.full_view.set(false) }
                    }
                    h2 { class: "ax-mail-reading-subject ax-selectable", title: "{subject}", tabindex: "-1", "{subject}" }
                    div { class: "ax-mail-reading-head-actions",
                        if dark {
                            IconButton {
                                icon: if light_body { "weather_moon" } else { "weather_sunny" },
                                title: "Switch background",
                                toggled: light_body,
                                onclick: move |_| {
                                    let v = !*ctx.light_body.peek();
                                    ctx.light_body.set(v);
                                },
                            }
                        }
                        IconButton { icon: "open", title: "Open in new window (Shift+Enter)", onclick: open_window }
                        if full {
                            IconButton { icon: "dismiss", title: "Close", onclick: move |_| ctx.full_view.set(false) }
                        }
                    }
                }
            }
            if !categories.is_empty() {
                div { class: "ax-mail-reading-cats", super::list::row::CategoryPills { categories: categories.clone(), max: 6 } }
            }
            div { class: "ax-mail-thread",
                for item in items {
                    match item {
                        Item::Card(m) => {
                            let response = responses.get(&m.id).map(|r| (r.message_id, r.at, r.forwarded));
                            let in_thread = in_thread.clone();
                            rsx! {
                                MessageCard {
                                    key: "card-{m.id}",
                                    msg: m.clone(),
                                    newest: Some(m.id) == newest_id,
                                    collapsible: count > 1,
                                    window,
                                    response,
                                    folder: folders.iter().find(|f| f.id == m.folder_id).cloned(),
                                    theme: if dark && !light_body { BodyTheme::Dark } else { BodyTheme::Light },
                                    on_collapse: move |id: MessageId| {
                                        expanded.write().remove(&id);
                                    },
                                    on_show: move |id: MessageId| {
                                        if in_thread.contains(&id) {
                                            expanded.write().insert(id);
                                            show_more.set(true);
                                            scroll_to_card(window, id);
                                        } else {
                                            // The reply isn't shown here (conversations off): open it in its folder.
                                            ctx.popout.set(None);
                                            app.navigate(NavRequest::Message(id));
                                        }
                                    },
                                }
                            }
                        }
                        Item::Collapsed(m) => rsx! {
                            CollapsedMessage {
                                key: "row-{m.id}",
                                dom_id: card_dom_id(window, m.id),
                                msg: m.clone(),
                                folder_tag: (m.folder_id != focus_msg.folder_id).then(|| folders.iter().find(|f| f.id == m.folder_id).map(|f| f.name.clone())).flatten(),
                                on_expand: move |id: MessageId| {
                                    expanded.write().insert(id);
                                },
                            }
                        },
                        Item::More(n) => rsx! {
                            button { key: "more-{n}", r#type: "button", class: "ax-mail-more-messages", onclick: move |_| show_more.set(true),
                                "{n} more messages"
                            }
                        },
                    }
                }
            }
        }
    }
}

#[component]
fn CollapsedMessage(msg: Message, dom_id: String, folder_tag: Option<String>, on_expand: EventHandler<MessageId>) -> Element {
    let app = use_app();
    let h24 = app.settings.read().use_24h_time;
    let time = super::list::row::row_time(msg.received_at, Local::now(), h24, false);
    let id = msg.id;
    let draft = msg.is_draft;
    let who = if draft { "[Draft]".to_string() } else { msg.from.display().to_string() };
    let in_deleted = folder_tag.as_deref() == Some("Deleted Items");
    // Drafts in a conversation continue in compose; other rows expand.
    let activate = move || {
        if draft {
            app.compose(crate::state::ComposeRequest { draft: Some(id), ..crate::state::ComposeRequest::new_message() });
        } else {
            on_expand.call(id);
        }
    };
    rsx! {
        div {
            id: "{dom_id}",
            class: "ax-mail-collapsed",
            class: if !msg.is_read { "ax-unread" },
            class: if in_deleted { "ax-deleted" },
            role: "button",
            tabindex: "0",
            "aria-label": "{who}, {time}, {msg.preview}",
            onclick: move |_| activate(),
            onkeydown: move |e: KeyboardEvent| {
                if e.key() == Key::Enter || e.key() == Key::Character(" ".into()) {
                    e.prevent_default();
                    e.stop_propagation();
                    activate();
                }
            },
            Avatar { name: msg.from.name.clone(), email: msg.from.address.clone(), size: 32 }
            div { class: "ax-mail-collapsed-text",
                span { class: "ax-mail-collapsed-from ax-ellipsis", "{who}" }
                span { class: "ax-mail-collapsed-preview ax-ellipsis", "{msg.preview}" }
            }
            if let Some(tag) = folder_tag.clone() { span { class: "ax-mail-tag", "{tag}" } }
            if msg.has_attachments { span { class: "ax-mail-ico", Icon { name: "attach", size: 16 } } }
            if msg.flag == FlagStatus::Flagged { span { class: "ax-mail-ico ax-flag", Icon { name: "flag_filled", size: 16 } } }
            span { class: "ax-mail-collapsed-time", "{time}" }
        }
    }
}

fn names(list: &[EmailAddress]) -> Vec<String> {
    list.iter().map(|a| a.display().to_string()).collect()
}

#[component]
fn RecipientLine(label: &'static str, list: Vec<EmailAddress>) -> Element {
    let mut expanded = use_signal(|| false);
    const SHOW: usize = 3;
    let all = names(&list);
    let more = all.len().saturating_sub(SHOW);
    let shown = if expanded() || more == 0 { all.join("; ") } else { all[..SHOW].join("; ") };
    let full = list.iter().map(|a| if a.name.is_empty() { a.address.clone() } else { format!("{} <{}>", a.name, a.address) }).collect::<Vec<_>>().join("; ");
    rsx! {
        div { class: "ax-mail-recipients",
            span { class: "ax-mail-recipients-label", "{label}: " }
            span { class: "ax-mail-recipients-names ax-selectable", title: "{full}", "{shown}" }
            if more > 0 && !expanded() {
                button { r#type: "button", class: "ax-mail-link", onclick: move |_| expanded.set(true), "+{more} others" }
            }
        }
    }
}

#[component]
fn Banner(icon: &'static str, #[props(default)] tone: &'static str, children: Element) -> Element {
    rsx! {
        div { class: "ax-mail-banner {tone}", role: "note",
            Icon { name: icon, size: 16 }
            div { class: "ax-mail-banner-text", {children} }
        }
    }
}

fn response_icon(forwarded: bool) -> &'static str {
    if forwarded { "arrow_forward" } else { "arrow_reply" }
}

/// The user's reply or forward of a card's message: (that message, when, forwarded).
type Response = (MessageId, chrono::DateTime<chrono::Utc>, bool);

#[component]
fn MessageCard(
    msg: Message,
    newest: bool,
    collapsible: bool,
    /// Shown in the read window ("Open in new window").
    window: bool,
    response: Option<Response>,
    /// The folder the message is in.
    folder: Option<Folder>,
    theme: BodyTheme,
    on_collapse: EventHandler<MessageId>,
    /// Show another message of the conversation (the "You replied" link).
    on_show: EventHandler<MessageId>,
) -> Element {
    let app = use_app();
    let mut ctx = use_mail();
    let model = use_model();
    let s = store();
    let settings = app.settings.read().clone();
    let id = msg.id;
    let body = s.message_body(id).ok().flatten().unwrap_or_default();
    let role = folder.as_ref().map(|f| f.role);
    let me: Vec<String> = s.accounts().unwrap_or_default().into_iter().map(|a| a.email.to_lowercase()).collect();
    let from_me = me.contains(&msg.from.address.to_lowercase());
    let allowed = ctx.allowed_remote.read().contains(&id) || !settings.block_external_images || s.is_safe_sender(&msg.from.address).unwrap_or(false);
    let mut history = use_signal(|| false);
    let zoom = *ctx.zoom.read();
    let width = frame_width(if window { *ctx.read_window_width.read() } else { *ctx.reading_width.read() }, window);
    let doc = body::build(&body, allowed, theme, zoom, history(), width);
    let time = crate::fmt::full_time(msg.received_at, settings.use_24h_time);
    let long = format!("{} {}", msg.received_at.with_timezone(&Local).format("%A, %B %-d, %Y"), crate::fmt::time(msg.received_at, settings.use_24h_time));
    let sender_addr = msg.from.address.clone();
    let can_reply = !msg.is_draft;
    let today = Local::now().date_naive();
    let sender_name = msg.from.display().to_string();
    let in_deleted = role == Some(FolderRole::Deleted);
    let sender_addr2 = msg.from.address.clone();
    let sender_angle = format!("<{}>", msg.from.address);
    let due_text = msg.flag_due.map(|d| d.format("%a %-m/%-d/%Y").to_string()).unwrap_or_default();
    // The avatar and name open the sender's contact card when there is one.
    let contact = if from_me { None } else { s.contact_by_email(&msg.from.address).ok().flatten().map(|c| c.id) };
    // Senders with no saved contact get a small profile menu instead:
    // "Add to contacts" opens People's New contact form pre-filled.
    let mut profile_menu = use_signal(|| None::<(f64, f64)>);
    let open_contact = move |e: MouseEvent| {
        e.stop_propagation();
        if let Some(c) = contact {
            ctx.popout.set(None);
            app.navigate(NavRequest::Contact(c));
        } else if !from_me {
            let p = e.client_coordinates();
            profile_menu.set(Some((p.x, p.y + 4.0)));
        }
    };
    let profile_items = vec![
        MenuItem::header(if msg.from.name.is_empty() { msg.from.address.clone() } else { format!("{} <{}>", msg.from.name, msg.from.address) }),
        MenuItem::new("profile_add_contact", "Add to contacts").icon("person_add"),
        MenuItem::new("profile_email", "Send email").icon("mail_add"),
    ];
    let profile_from = msg.from.clone();
    let on_profile = move |cmd: String| match cmd.as_str() {
        "profile_add_contact" => {
            ctx.popout.set(None);
            app.navigate(NavRequest::NewContact { name: profile_from.name.clone(), email: profile_from.address.clone() });
        }
        "profile_email" => app.compose(crate::state::ComposeRequest::to(vec![profile_from.clone()])),
        _ => {}
    };
    let sender_tip = if contact.is_some() { "Open contact card" } else if from_me { "" } else { "Add to contacts or send email" };
    let reply = move |kind: ComposeKind| move |_: MouseEvent| actions::respond(app, ctx, kind, id, window);
    let card_id = card_dom_id(window, id);

    let more_items = {
        let t = Target::for_ids(&[id], role, folder.as_ref().map(|f| f.id), settings.focused_inbox, settings.use_24h_time);
        menus::message_menu(&t, folder.as_ref().map(|f| f.id))
    };
    let run = move |cmd: String| {
        let m = model.read();
        actions::run(app, ctx, &m, &cmd, vec![id]);
    };
    rsx! {
        article {
            id: "{card_id}",
            class: "ax-mail-card",
            class: if in_deleted { "ax-deleted" },
            "aria-label": format!("{sender_name}, {time}"),
            header {
                class: "ax-mail-card-head",
                onclick: move |_| {
                    if collapsible {
                        on_collapse.call(id);
                    }
                },
                span {
                    class: if contact.is_some() || !from_me { "ax-mail-card-avatar ax-link" } else { "ax-mail-card-avatar" },
                    title: sender_tip,
                    onclick: open_contact,
                    Avatar { name: msg.from.name.clone(), email: msg.from.address.clone(), size: 40 }
                }
                if let Some((x, y)) = profile_menu() {
                    Menu {
                        items: profile_items,
                        x,
                        y,
                        label: "Sender",
                        on_select: on_profile,
                        on_close: move |_| profile_menu.set(None),
                    }
                }
                div { class: "ax-mail-card-who",
                    div { class: "ax-mail-sender-line ax-selectable",
                        span {
                            class: if contact.is_some() || !from_me { "ax-mail-sender-name ax-link" } else { "ax-mail-sender-name" },
                            title: sender_tip,
                            onclick: open_contact,
                            "{sender_name}"
                        }
                        if !msg.from.name.is_empty() { span { class: "ax-mail-sender-addr", {sender_angle} } }
                    }
                    if !msg.to.is_empty() { RecipientLine { label: "To", list: msg.to.clone() } }
                    if !msg.cc.is_empty() { RecipientLine { label: "Cc", list: msg.cc.clone() } }
                    if from_me && !msg.bcc.is_empty() { RecipientLine { label: "Bcc", list: msg.bcc.clone() } }
                }
                div { class: "ax-mail-card-side", onclick: move |e: MouseEvent| e.stop_propagation(),
                    div { class: "ax-mail-card-actions", role: "toolbar", "aria-label": "Message actions",
                        if msg.is_draft {
                            IconButton { icon: "edit", title: "Edit draft", onclick: move |_| run("edit_draft".into()) }
                        }
                        if can_reply {
                            IconButton { icon: "arrow_reply", title: "Reply (Ctrl+R)", onclick: reply(ComposeKind::Reply) }
                            IconButton { icon: "arrow_reply_all", title: "Reply all (Ctrl+Shift+R)", onclick: reply(ComposeKind::ReplyAll) }
                            IconButton { icon: "arrow_forward", title: "Forward (Ctrl+F)", onclick: reply(ComposeKind::Forward) }
                        }
                        MenuButton {
                            label: "More actions",
                            icon: "more_horizontal",
                            icon_only: true,
                            items: more_items,
                            on_select: move |cmd: String| run(cmd),
                        }
                    }
                    div { class: "ax-mail-card-time", title: "{long}", "{time}" }
                }
            }
            div { class: "ax-mail-banners",
                if let Some((reply_id, at, forwarded)) = response {
                    Banner { icon: response_icon(forwarded),
                        span {
                            if forwarded { "You forwarded this message on " } else { "You replied to this message on " }
                            button {
                                r#type: "button",
                                class: "ax-mail-link",
                                title: if forwarded { "Show the forwarded message" } else { "Show your reply" },
                                onclick: move |_| on_show.call(reply_id),
                                {crate::fmt::full_time(at, settings.use_24h_time)}
                            }
                            "."
                        }
                    }
                }
                if role == Some(FolderRole::Junk) {
                    Banner { icon: "warning", tone: "ax-warning",
                        span { "This message is in Junk Email. Links and attachments may be unsafe." }
                        button { r#type: "button", class: "ax-mail-link", onclick: move |_| run("not_junk".into()), "It's not junk" }
                    }
                }
                match msg.flag {
                    FlagStatus::Flagged => rsx! {
                        Banner { icon: "flag", tone: "ax-flagged",
                            if let Some(due) = msg.flag_due {
                                span { class: if due < today { "ax-overdue" } else { "" }, "Follow up. Due by {due_text}." }
                            } else {
                                span { "Follow up." }
                            }
                        }
                    },
                    FlagStatus::Complete => rsx! { Banner { icon: "checkmark", tone: "ax-complete", span { "Completed." } } },
                    FlagStatus::NotFlagged => rsx! {},
                }
                match msg.importance {
                    Importance::High => rsx! { Banner { icon: "important", tone: "ax-important", span { "This message was sent with High importance." } } },
                    Importance::Low => rsx! { Banner { icon: "arrow_down", span { "This message was sent with Low importance." } } },
                    Importance::Normal => rsx! {},
                }
                if doc.blocked {
                    Banner { icon: "image_off",
                        span { "Pictures and other content from the internet weren't downloaded for this message." }
                        button {
                            r#type: "button",
                            class: "ax-mail-link",
                            onclick: move |_| {
                                ctx.allowed_remote.write().insert(id);
                            },
                            "Show content"
                        }
                        button {
                            r#type: "button",
                            class: "ax-mail-link",
                            onclick: move |_| {
                                if app.check(store().add_safe_sender(&sender_addr)).is_some() {
                                    app.toast(format!("Content from {sender_addr} will always be shown."));
                                    ctx.allowed_remote.write().insert(id);
                                }
                            },
                            "Always show content from {sender_addr2}"
                        }
                    }
                }
            }
            if let (Some(kind), Some(event)) = (msg.meeting_kind, msg.meeting_event_id) {
                MeetingBar { event_id: event, kind }
            }
            AttachmentWell { attachments: body.attachments.clone(), subject: msg.subject.clone() }
            iframe {
                class: "ax-mail-body",
                // A name for assistive technology, not a tooltip over the text.
                "aria-label": "Message body",
                "sandbox": "allow-popups",
                srcdoc: doc.srcdoc.clone(),
                style: format!("height:{}px", doc.height),
            }
            if doc.has_history {
                button { r#type: "button", class: "ax-mail-history", title: "Show message history", "aria-label": "Show message history", onclick: move |_| history.set(true),
                    Icon { name: "more_horizontal", size: 16 }
                }
            }
            if newest && can_reply {
                div { class: "ax-mail-reply-bar",
                    button { r#type: "button", class: "ax-btn", onclick: reply(ComposeKind::Reply), Icon { name: "arrow_reply" } span { "Reply" } }
                    button { r#type: "button", class: "ax-btn", onclick: reply(ComposeKind::ReplyAll), Icon { name: "arrow_reply_all" } span { "Reply all" } }
                    button { r#type: "button", class: "ax-btn", onclick: reply(ComposeKind::Forward), Icon { name: "arrow_forward" } span { "Forward" } }
                }
            }
        }
    }
}

/// RSVP bar for a meeting invitation linked to a calendar event.
#[component]
fn MeetingBar(event_id: EventId, kind: MeetingMessageKind) -> Element {
    let app = use_app();
    let _ = app.calendar_rev();
    let h24 = app.settings.read().use_24h_time;
    let Some(event) = store().event(event_id).ok().flatten() else {
        return rsx! {
            Banner { icon: "calendar_cancel", span { "This meeting is no longer on your calendar." } }
        };
    };
    let start = event.start.with_timezone(&Local);
    let when = if event.all_day {
        format!("{} (all day)", start.format("%a %-m/%-d/%Y"))
    } else {
        format!("{} {} – {}", start.format("%a %-m/%-d/%Y"), crate::fmt::time(event.start, h24), crate::fmt::time(event.end, h24))
    };
    let status = match event.my_response {
        ResponseStatus::Accepted => "You accepted this meeting.",
        ResponseStatus::Tentative => "You tentatively accepted this meeting.",
        ResponseStatus::Declined => "You declined this meeting.",
        ResponseStatus::Organizer => "You're the organizer.",
        _ => "You haven't responded yet.",
    };
    let title = event.title.clone();
    let respond = move |r: ResponseStatus| {
        let title = title.clone();
        move |_| {
            if app.check(store().respond_to_event(event_id, r)).is_some() {
                let verb = match r {
                    ResponseStatus::Accepted => "Accepted",
                    ResponseStatus::Tentative => "Tentatively accepted",
                    _ => "Declined",
                };
                app.toast(format!("{verb}: {title}"));
                app.bump_calendar();
                app.bump_mail();
            }
        }
    };
    let response = event.my_response;
    let (accepted, tentative, declined) =
        (response == ResponseStatus::Accepted, response == ResponseStatus::Tentative, response == ResponseStatus::Declined);
    let cancelled = kind == MeetingMessageKind::Cancellation || event.is_cancelled;
    rsx! {
        div { class: "ax-mail-meeting", role: "group", "aria-label": "Meeting invitation",
            div { class: "ax-mail-meeting-info",
                Icon { name: "calendar_ltr", size: 20 }
                div { class: "ax-mail-meeting-text",
                    div { class: "ax-mail-meeting-title", "{event.title}" }
                    div { class: "ax-mail-meeting-when", "{when}" }
                    if !event.location.is_empty() { div { class: "ax-mail-meeting-where", "{event.location}" } }
                    div { class: "ax-mail-meeting-status", if cancelled { "This meeting was cancelled." } else { "{status}" } }
                }
            }
            if !cancelled && response != ResponseStatus::Organizer {
                div { class: "ax-mail-meeting-actions",
                    button { r#type: "button", class: "ax-btn", class: if accepted { "ax-on" }, "aria-pressed": if accepted { "true" } else { "false" },
                        onclick: respond(ResponseStatus::Accepted), Icon { name: "checkmark" } span { "Accept" } }
                    button { r#type: "button", class: "ax-btn", class: if tentative { "ax-on" }, "aria-pressed": if tentative { "true" } else { "false" },
                        onclick: respond(ResponseStatus::Tentative), Icon { name: "question_circle" } span { "Tentative" } }
                    button { r#type: "button", class: "ax-btn", class: if declined { "ax-on" }, "aria-pressed": if declined { "true" } else { "false" },
                        onclick: respond(ResponseStatus::Declined), Icon { name: "dismiss" } span { "Decline" } }
                }
            }
            button { r#type: "button", class: "ax-mail-link", onclick: move |_| app.navigate(NavRequest::Event(event_id)), "View in calendar" }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_frames_are_estimated_at_the_width_they_get() {
        // The read window at its default 900px (898 inside its border)
        // keeps the old 820px estimate; resized, the estimate follows.
        assert_eq!(frame_width(898.0, true), 820.0);
        assert_eq!(frame_width(478.0, true), 400.0);
        assert!(body::estimate_height(&"<p>Lorem ipsum dolor sit amet, consectetur adipiscing elit.</p>".repeat(20), frame_width(478.0, true), 100) > body::estimate_height(&"<p>Lorem ipsum dolor sit amet, consectetur adipiscing elit.</p>".repeat(20), frame_width(898.0, true), 100));
        // Never narrower than 240px; the reading pane keeps its own padding.
        assert_eq!(frame_width(100.0, true), 240.0);
        assert_eq!(frame_width(712.0, false), 640.0);
    }
}
