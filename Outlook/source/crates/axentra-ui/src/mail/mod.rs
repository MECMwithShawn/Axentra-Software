//! Mail module: ribbon, folder pane, message list and reading pane
//! (docs/spec/mail.md). The module root owns the module state (`MailCtx`),
//! the list model, layout and splitters, background timers, navigation
//! requests, mark-as-read and compose hosting.

pub mod actions;
pub mod compose;
mod ctx;
mod dates;
mod dialogs;
mod files;
mod folders;
mod keyboard;
mod list;
mod menus;
mod model;
mod reading;
mod ribbon;

use std::rc::Rc;
use std::time::Duration;

use axentra_core::*;
use chrono::{Local, Utc};
use dioxus::prelude::*;

use crate::kit::Menu;
use crate::shell::ribbon::Ribbon;
use crate::state::{AppState, NavRequest, store, use_app};
use ctx::{ListMemo, MailCtx, MenuTarget, RowKey};

/// Folder pane width measured on the reference (216 px at 1440 px). The
/// model's default of 240 is treated as "not resized yet". MEASURED
const FOLDER_PANE_DEFAULT: u32 = 216;
const FOLDER_PANE_MIN: u32 = 180; // ESTIMATE
const FOLDER_PANE_MAX: u32 = 400; // ESTIMATE
const LIST_DEFAULT: u32 = 400;
const LIST_MIN: u32 = 300; // ESTIMATE
const LIST_MAX: u32 = 900;
const LIST_HEIGHT_DEFAULT: u32 = 45;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Splitter {
    Folders,
    List,
    ListHeight,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Resize {
    which: Splitter,
    start: f64,
    start_value: f64,
    value: f64,
}

fn folder_width(settings: &Settings) -> u32 {
    let w = if settings.folder_pane_width == Settings::default().folder_pane_width { FOLDER_PANE_DEFAULT } else { settings.folder_pane_width };
    w.clamp(FOLDER_PANE_MIN, FOLDER_PANE_MAX)
}

/// Mark the displayed messages read, if they aren't already.
fn mark_read(app: AppState, ids: &[MessageId]) {
    let s = store();
    let unread: Vec<MessageId> = s.messages_by_ids(ids).unwrap_or_default().into_iter().filter(|m| !m.is_read).map(|m| m.id).collect();
    if !unread.is_empty() && app.check(s.set_read(&unread, true)).is_some() {
        app.bump_mail();
    }
}

#[component]
pub fn MailModule() -> Element {
    let mut app = use_app();
    let mut ctx = use_context_provider(MailCtx::new);
    let memo = use_memo(move || Rc::new(model::build(app, ctx)));
    use_context_provider(|| ListMemo(memo));

    // Start in the primary account's Inbox.
    use_hook(move || {
        if ctx.folder.peek().is_none()
            && let Some(inbox) = store()
                .primary_account()
                .ok()
                .flatten()
                .and_then(|a| store().folder_by_role(a.id, FolderRole::Inbox).ok().flatten())
        {
            ctx.folder.set(Some(inbox.id));
        }
        document::eval(keyboard::HELPERS_JS);
        let icon = serde_json::to_string(crate::icons::icon_svg("mail").unwrap_or_default()).unwrap_or_default();
        document::eval(&format!("window.__axMailIcon = {icon};"));
    });

    // The shown folder disappeared (deleted for good, account removed): back to an Inbox.
    use_effect(move || {
        let _ = app.mail_rev();
        let current = *ctx.folder.peek();
        let s = store();
        if current.is_some_and(|f| s.folder(f).ok().flatten().is_none())
            && let Some(inbox) = s.primary_account().ok().flatten().and_then(|a| s.folder_by_role(a.id, FolderRole::Inbox).ok().flatten())
        {
            actions::go_to_folder(app, ctx, inbox.id);
        }
    });

    // Navigation requests from other modules (open a folder / message).
    use_effect(move || {
        let Some(req) = app.nav.read().clone() else { return };
        match req {
            NavRequest::Folder(id) => {
                app.nav.set(None);
                ctx.popout.set(None);
                actions::go_to_folder(app, ctx, id);
            }
            NavRequest::Message(id) => {
                app.nav.set(None);
                ctx.popout.set(None);
                let Some(m) = store().message(id).ok().flatten() else {
                    app.error("That message is no longer available.");
                    return;
                };
                actions::go_to_folder(app, ctx, m.folder_id);
                let settings = app.settings.peek().clone();
                if settings.focused_inbox && store().folder(m.folder_id).ok().flatten().is_some_and(|f| f.role == FolderRole::Inbox) {
                    ctx.tab.set(m.inference);
                }
                actions::open_message(ctx, &m, settings.conversation_view);
                let key = if settings.conversation_view { RowKey::Conv(m.conversation_id.clone()) } else { RowKey::Msg(m.id) };
                ctx.selection.set(vec![key.clone()]);
                ctx.anchor.set(Some(key.clone()));
                ctx.focus.set(Some(key.clone()));
                ctx.reveal(key);
            }
            _ => {}
        }
    });

    // Mark as read: on select, after a delay, or never (Settings > Mail).
    use_effect(move || {
        let Some(open) = ctx.open.read().clone() else { return };
        let (mode, delay) = {
            let s = app.settings.peek();
            (s.mark_as_read, s.mark_as_read_delay_secs)
        };
        match mode {
            MarkAsRead::Never => {}
            MarkAsRead::OnSelect => mark_read(app, &open.mark),
            MarkAsRead::AfterDelay => {
                spawn(async move {
                    tokio::time::sleep(Duration::from_secs(delay as u64)).await;
                    if ctx.open.peek().as_ref() == Some(&open) {
                        mark_read(app, &open.mark);
                    }
                });
            }
        }
    });

    // Background work: snoozed mail comes back, sweep rules run on launch
    // and daily, date groups roll over at midnight.
    use_future(move || async move {
        let s = store();
        if s.run_sweep_rules().unwrap_or(0) > 0 {
            app.bump_mail();
        }
        let mut last_sweep = std::time::Instant::now();
        loop {
            match s.wake_snoozed(Utc::now()) {
                Ok(n) if n > 0 => {
                    app.bump_mail();
                    app.toast(if n == 1 { "A snoozed message is back in your Inbox".to_string() } else { format!("{n} snoozed messages are back in your Inbox") });
                }
                _ => {}
            }
            let today = Local::now().date_naive();
            if *ctx.today.peek() != today {
                ctx.today.set(today);
            }
            if last_sweep.elapsed() > Duration::from_secs(24 * 3600) {
                last_sweep = std::time::Instant::now();
                if s.run_sweep_rules().unwrap_or(0) > 0 {
                    app.bump_mail();
                }
            }
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    });

    // Follow the OS dark preference for the System theme (message bodies).
    use_future(move || async move {
        let mut ev = document::eval(
            "const mq = window.matchMedia('(prefers-color-scheme: dark)');
             dioxus.send(mq.matches);
             mq.addEventListener('change', (e) => dioxus.send(e.matches));
             await new Promise(() => {});",
        );
        while let Ok(dark) = ev.recv::<bool>().await {
            if *ctx.system_dark.peek() != dark {
                ctx.system_dark.set(dark);
            }
        }
    });

    // Search: results replace the list and focus goes to the first result;
    // leaving search brings back the folder's selection (§7.5).
    {
        let mut last = use_signal(|| (false, 0u64));
        use_effect(move || {
            let (active, serial) = {
                let s = app.search.read();
                (s.active && !s.query.trim().is_empty(), s.serial)
            };
            let (was_active, last_serial) = *last.peek();
            if (active, serial) == (was_active, last_serial) || (!active && !was_active) {
                last.set((active, serial));
                return;
            }
            last.set((active, serial));
            let folder = *ctx.folder.peek();
            if active {
                if !was_active && let Some(f) = folder {
                    let memory = ctx::FolderMemory {
                        selection: ctx.selection.peek().clone(),
                        anchor: ctx.anchor.peek().clone(),
                        focus: ctx.focus.peek().clone(),
                        open: ctx.open.peek().clone(),
                        tab: *ctx.tab.peek(),
                        scroll: *ctx.list_scroll.peek(),
                    };
                    ctx.memory.write().insert(f, memory);
                }
                ctx.clear_selection();
                ctx.focus.set(None);
                ctx.expanded.set(None);
                ctx.full_view.set(false);
                ctx.request_list_focus();
            } else if let Some(m) = folder.and_then(|f| ctx.memory.peek().get(&f).cloned()) {
                ctx.selection.set(m.selection);
                ctx.anchor.set(m.anchor);
                ctx.focus.set(m.focus);
                ctx.open.set(m.open);
            }
        });
    }

    let settings = app.settings.read().clone();
    let compose = app.compose.read().clone();
    let compose_inline = compose.as_ref().is_some_and(|c| !c.popout);
    let searching = memo.read().searching;
    // Inline compose selects its Message tab, and a search its Search tab,
    // as Outlook does.
    use_effect(use_reactive!(|compose_inline, searching| {
        let tab = if compose_inline {
            "Message"
        } else if searching {
            "Search"
        } else {
            return;
        };
        document::eval(&format!(
            "setTimeout(() => {{ const t = [...document.querySelectorAll('.ax-mail .ax-ribbon-tab')].find(b => b.textContent.trim() === '{tab}'); if (t) t.click(); }}, 0);"
        ));
    }));

    let mut resize = use_signal(|| None::<Resize>);
    let mut main_height = use_signal(|| 600.0f64);
    let model_now = memo.read().clone();
    let tabs = ribbon::tabs(app, ctx, &model_now);
    let collapsed = settings.folder_pane_collapsed;
    let live = resize();
    let fw = match live {
        Some(r) if r.which == Splitter::Folders => r.value as u32,
        _ => folder_width(&settings),
    };
    let lw = match live {
        Some(r) if r.which == Splitter::List => r.value as u32,
        _ => settings.message_list_width.clamp(LIST_MIN, LIST_MAX),
    };
    let prefs = ctx.prefs.read().clone();
    let lh = match live {
        Some(r) if r.which == Splitter::ListHeight => r.value,
        _ => prefs.list_height_pct.unwrap_or(LIST_HEIGHT_DEFAULT) as f64,
    };
    let rp = settings.reading_pane;
    let full_view = *ctx.full_view.read() && ctx.open.read().is_some();

    let mut start_resize = move |which: Splitter, pos: f64, value: f64| {
        resize.set(Some(Resize { which, start: pos, start_value: value, value }));
    };
    let finish_resize = move |r: Resize| match r.which {
        Splitter::Folders => app.update_settings(|s| s.folder_pane_width = r.value.round() as u32),
        Splitter::List => app.update_settings(|s| s.message_list_width = r.value.round() as u32),
        Splitter::ListHeight => ctx.update_prefs(|p| p.list_height_pct = Some(r.value.round() as u32)),
    };

    let menu = ctx.menu.read().clone();
    let model_for_menu = memo;
    let popout = compose.clone().filter(|c| c.popout);
    let read_window = ctx.popout.read().clone();

    let list_pane = |columns: bool, style: String| rsx! { list::MessageList { columns, style } };

    rsx! {
        div {
            class: "ax-mail",
            onkeydown: move |e: KeyboardEvent| {
                if ctx.take_typing() {
                    return;
                }
                let m = memo.read().clone();
                if keyboard::module_key(app, ctx, &m, &e) {
                    e.prevent_default();
                    e.stop_propagation();
                }
            },
            // A fresh ribbon (Home selected) when the contextual compose or
            // search tabs come and go; the keyed list forces the remount.
            for ribbon_key in [format!("{compose_inline}-{searching}")] {
                Ribbon {
                    key: "{ribbon_key}",
                    tabs: tabs.clone(),
                    on_command: move |id: String| {
                        let m = memo.read().clone();
                        ribbon::on_command(app, ctx, &m, &id);
                    },
                    on_toggle_nav: move |_| app.update_settings(|s| s.folder_pane_collapsed = !s.folder_pane_collapsed),
                    nav_open: !collapsed,
                }
            }
            div { class: "ax-mail-panes",
                if !collapsed {
                    folders::FolderPane { width: fw }
                    SplitterBar {
                        vertical: true,
                        label: "Resize the folder pane",
                        onstart: move |x: f64| start_resize(Splitter::Folders, x, fw as f64),
                        onreset: move |_| app.update_settings(|s| s.folder_pane_width = FOLDER_PANE_DEFAULT),
                    }
                }
                match rp {
                    ReadingPanePosition::Right => rsx! {
                        {list_pane(false, format!("width:{lw}px"))}
                        SplitterBar {
                            vertical: true,
                            label: "Resize the message list",
                            onstart: move |x: f64| start_resize(Splitter::List, x, lw as f64),
                            onreset: move |_| app.update_settings(|s| s.message_list_width = LIST_DEFAULT),
                        }
                        reading::ReadingPane {}
                    },
                    ReadingPanePosition::Bottom => rsx! {
                        div {
                            class: "ax-mail-stack",
                            onresize: move |e: ResizeEvent| {
                                if let Ok(size) = e.data().get_content_box_size() {
                                    main_height.set(size.height);
                                }
                            },
                            {list_pane(true, format!("height:{lh}%"))}
                            SplitterBar {
                                vertical: false,
                                label: "Resize the message list",
                                onstart: move |y: f64| start_resize(Splitter::ListHeight, y, lh),
                                onreset: move |_| ctx.update_prefs(|p| p.list_height_pct = None),
                            }
                            reading::ReadingPane {}
                        }
                    },
                    ReadingPanePosition::Hidden => rsx! {
                        if full_view || compose_inline {
                            reading::ReadingPane { full: true }
                        } else {
                            {list_pane(true, String::new())}
                        }
                    },
                }
            }
            if let Some(rows_resize) = live.map(|r| r.which == Splitter::ListHeight) {
                div {
                    class: if rows_resize { "ax-mail-resize-overlay ax-rows" } else { "ax-mail-resize-overlay" },
                    onmousemove: move |e: MouseEvent| {
                        let c = e.client_coordinates();
                        let Some(mut r) = resize() else { return };
                        r.value = match r.which {
                            Splitter::Folders => (r.start_value + c.x - r.start).clamp(FOLDER_PANE_MIN as f64, FOLDER_PANE_MAX as f64),
                            Splitter::List => (r.start_value + c.x - r.start).clamp(LIST_MIN as f64, LIST_MAX as f64),
                            Splitter::ListHeight => (r.start_value + (c.y - r.start) / main_height().max(1.0) * 100.0).clamp(20.0, 80.0),
                        };
                        resize.set(Some(r));
                    },
                    onmouseup: move |_| {
                        if let Some(r) = resize() {
                            finish_resize(r);
                        }
                        resize.set(None);
                    },
                }
            }
            if let Some(open) = read_window {
                reading::ReadWindow { open }
            }
            if let Some(req) = popout {
                div {
                    class: "ax-mail-compose-popout",
                    onkeydown: move |_| ctx.mark_typing(),
                    compose::ComposePane { request: req }
                }
            }
            dialogs::MailDialogs {}
            if let Some(menu) = menu {
                Menu {
                    items: menu.items.clone(),
                    x: menu.x,
                    y: menu.y,
                    on_select: move |id: String| {
                        let m = model_for_menu.read().clone();
                        match &menu.target {
                            MenuTarget::Messages(ids) => {
                                actions::run(app, ctx, &m, &id, ids.clone());
                            }
                            target => folders::run_folder_command(app, ctx, &id, target),
                        }
                    },
                    on_close: move |_| ctx.menu.set(None),
                }
            }
        }
    }
}

/// A pane splitter: 1px line with an 8px hit area; drag to resize,
/// double-click to reset (docs/spec/mail.md §1.4).
#[component]
fn SplitterBar(vertical: bool, label: &'static str, onstart: EventHandler<f64>, onreset: EventHandler<()>) -> Element {
    rsx! {
        div {
            class: if vertical { "ax-mail-splitter" } else { "ax-mail-splitter ax-horizontal" },
            role: "separator",
            "aria-orientation": if vertical { "vertical" } else { "horizontal" },
            "aria-label": "{label}",
            title: "{label}",
            onmousedown: move |e: MouseEvent| {
                e.prevent_default();
                let c = e.client_coordinates();
                onstart.call(if vertical { c.x } else { c.y });
            },
            ondoubleclick: move |_| onreset.call(()),
        }
    }
}
