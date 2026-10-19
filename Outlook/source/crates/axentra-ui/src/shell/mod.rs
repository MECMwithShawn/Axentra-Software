//! App frame: header, app rail, active module, side panes, toasts, dialogs.
//!
//! The shell keeps its own state in [`ShellCtx`] (provided at the root, read
//! with [`use_shell`]); app-wide state stays in `AppState`. Theme, density,
//! high contrast, motion and chrome are attributes on `.ax-app`, so changing
//! them never rebuilds the tree (docs/spec/shell.md §12.2).

pub mod account;
pub mod header;
pub mod help;
pub mod keymap;
pub mod launcher;
pub mod my_day;
pub mod notifications;
pub mod rail;
pub mod ribbon;
pub mod search;
pub mod shortcuts;
pub mod theme;
pub mod toasts;

use axentra_core::*;
use chrono::{DateTime, Local};
use dioxus::prelude::*;

use crate::kit::{dom, keys};
use crate::settings::PageId;
use crate::state::{AppState, ComposeRequest, Module, NavRequest, SettingsPage, store, sync, use_app};

/// Reports `navigator.onLine` now and on every change.
const ONLINE_JS: &str = r#"
const send = () => dioxus.send(navigator.onLine !== false);
send();
window.addEventListener("online", send);
window.addEventListener("offline", send);
await new Promise(() => {});
"#;

/// Side panes docked at the right of the content (mutually exclusive with
/// My Day, whose open state lives in `AppState::my_day_open`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidePane {
    Notifications,
    Help,
}

/// Header and rail flyouts (one at a time).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flyout {
    Launcher,
    Account,
    MoreApps,
}

/// Shell-local state, provided by [`Shell`].
#[derive(Clone, Copy)]
pub struct ShellCtx {
    pub pane: Signal<Option<SidePane>>,
    pub flyout: Signal<Option<Flyout>>,
    pub shortcuts_open: Signal<bool>,
    pub whats_new_open: Signal<bool>,
    /// Ribbon collapsed to its tab row (per window).
    pub ribbon_collapsed: Signal<bool>,
    /// Settings page to show when the dialog opens next.
    pub settings_page: Signal<Option<PageId>>,
    /// Last page shown in the Settings dialog (the gear reopens it).
    pub settings_last: Signal<Option<PageId>>,
    /// Appearance being previewed by the open Settings dialog.
    pub preview: Signal<Option<Settings>>,
    /// Undo stack for Ctrl+Z (toast id, action), newest last, at most 20.
    pub undo: Signal<Vec<(u64, crate::state::UndoAction)>>,
    /// Wall clock, refreshed every 30 s (agenda, reminders, midnight reset).
    pub now: Signal<DateTime<Local>>,
    /// Notification read / seen / dismissed keys.
    pub notices: Signal<notifications::NoticeState>,
    /// Network reachable (the page's online / offline events, §9.3).
    pub online: Signal<bool>,
    /// Bumped when a ribbon customisation is saved.
    pub ribbon_rev: Signal<u64>,
}

pub fn use_shell() -> ShellCtx {
    use_context::<ShellCtx>()
}

/// The shell context when rendered inside the shell (always, except in
/// isolated component tests).
pub fn try_shell() -> Option<ShellCtx> {
    try_use_context::<ShellCtx>()
}

fn provide_shell() -> ShellCtx {
    use_context_provider(|| ShellCtx {
        pane: Signal::new(None),
        flyout: Signal::new(None),
        shortcuts_open: Signal::new(false),
        whats_new_open: Signal::new(false),
        ribbon_collapsed: Signal::new(false),
        settings_page: Signal::new(None),
        settings_last: Signal::new(None),
        preview: Signal::new(None),
        undo: Signal::new(Vec::new()),
        now: Signal::new(Local::now()),
        notices: Signal::new(notifications::NoticeState::load(store())),
        online: Signal::new(true),
        ribbon_rev: Signal::new(0),
    })
}

impl ShellCtx {
    /// Open the Settings dialog on a page.
    pub fn open_settings(mut self, mut app: AppState, page: PageId) {
        self.settings_page.set(Some(page));
        self.flyout.set(None);
        app.settings_dialog.set(Some(page.category().settings_page()));
    }

    /// Open (or close) a side pane; My Day closes when another pane opens.
    pub fn toggle_pane(mut self, mut app: AppState, pane: SidePane) {
        let open = (self.pane)() == Some(pane);
        self.pane.set(if open { None } else { Some(pane) });
        if !open {
            app.my_day_open.set(false);
        }
    }

    pub fn toggle_flyout(mut self, f: Flyout) {
        let open = (self.flyout)() == Some(f);
        self.flyout.set(if open { None } else { Some(f) });
    }

    /// Undo the most recent reversible action (Ctrl+Z, §8.1).
    pub fn undo_last(mut self, app: AppState) {
        let last = self.undo.write().pop();
        match last {
            Some((toast, action)) => {
                app.run_undo(&action);
                app.dismiss_toast(toast);
                dom::announce("Undone", false);
            }
            None => dom::announce("Nothing to undo", false),
        }
    }
}

/// Attribute values for `.ax-app` derived from settings.
struct Look {
    theme: &'static str,
    density: &'static str,
    color_theme: String,
    chrome: &'static str,
    contrast: &'static str,
    motion: &'static str,
    ribbon: &'static str,
}

impl Look {
    fn of(s: &Settings) -> Self {
        let system = theme::system_prefs();
        let color_theme = if s.color_theme == "blue" && s.accent.as_deref().and_then(theme::parse_hex).is_some() {
            "custom".to_string()
        } else {
            theme::color_theme(&s.color_theme).id.to_string()
        };
        Look {
            theme: match s.theme {
                ThemeMode::System => "ax-theme-system",
                ThemeMode::Light => "ax-theme-light",
                ThemeMode::Dark => "ax-theme-dark",
            },
            density: match s.density {
                Density::Roomy => "ax-density-roomy",
                Density::Cozy => "ax-density-cozy",
                Density::Compact => "ax-density-compact",
            },
            color_theme,
            chrome: if s.classic_look { "flat" } else { "cards" },
            contrast: theme::follow_system(s.high_contrast, system.high_contrast, "high", "normal"),
            motion: theme::follow_system(s.reduce_motion, system.reduce_motion, "reduce", "normal"),
            ribbon: if s.simplified_ribbon { "simplified" } else { "classic" },
        }
    }
}

#[component]
pub fn Shell() -> Element {
    let app = use_app();
    let mut shell = provide_shell();
    let saved = app.settings.read().clone();
    // While Settings previews appearance, the frame shows the draft.
    let previewing = (app.settings_dialog)().is_some();
    let look = Look::of(shell.preview.read().as_ref().filter(|_| previewing).unwrap_or(&saved));
    let accent = saved.accent.clone();
    let theme_css = use_memo(move || theme::theme_css(accent.as_deref()));

    // Install the page runtime once, then keep it in sync with the settings.
    use_effect(move || {
        dom::install();
    });
    use_effect(move || {
        let s = app.settings.read();
        let (always, outside) = keymap::prevent_lists(s.keyboard_mode);
        dom::set_prevent(&always, &outside);
        dom::set_keytips(s.show_key_tips);
    });

    // Pick up changes written by the sync engine.
    use_future(move || async move {
        let Some(s) = sync() else { return };
        let mut seen = s.change_counter();
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(750)).await;
            let now = s.change_counter();
            if now != seen {
                seen = now;
                app.bump_accounts();
            }
            // Send failures and "n messages sent" from the engine.
            for n in s.take_notices() {
                if n.error { app.error(n.text) } else { app.toast(n.text) };
            }
        }
    });

    // Online / offline (§9.3): the webview's network state, live.
    use_future(move || async move {
        let mut ev = document::eval(ONLINE_JS);
        while let Ok(on) = ev.recv::<bool>().await {
            if *shell.online.peek() != on {
                shell.online.set(on);
                dom::announce(if on { "You're back online" } else { "You're offline" }, !on);
            }
        }
    });

    // Clock for the agenda, reminders and the My Day midnight reset.
    use_future(move || async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            shell.now.set(Local::now());
        }
    });

    // My Day shares the side-pane slot: opening it closes the other panes.
    use_effect(move || {
        if (app.my_day_open)() && shell.pane.peek().is_some() {
            shell.pane.set(None);
        }
    });

    // Every undoable toast also goes on the Ctrl+Z stack.
    use_effect(move || {
        let toasts = app.toasts.read();
        let mut stack = shell.undo.write();
        for t in toasts.iter() {
            if let Some(u) = &t.undo
                && !stack.iter().any(|(id, _)| *id == t.id)
            {
                stack.push((t.id, u.clone()));
            }
        }
        let excess = stack.len().saturating_sub(20);
        stack.drain(..excess);
    });

    let module = (app.module)();
    let pane = (shell.pane)();
    rsx! {
        document::Title { "{module.label()} - {crate::APP_NAME}" }
        style { {theme_css()} }
        div {
            class: "ax-app {look.theme} {look.density}",
            "data-color-theme": "{look.color_theme}",
            "data-chrome": look.chrome,
            "data-contrast": look.contrast,
            "data-motion": look.motion,
            "data-ribbon": look.ribbon,
            "data-light-messages": if saved.light_message_background { "true" },
            tabindex: "-1",
            onkeydown: move |e: KeyboardEvent| on_global_key(app, shell, &e),
            button {
                r#type: "button",
                class: "ax-skip-link",
                onclick: move |_| dom::enter_region(".ax-module [data-ax-region], .ax-module .ax-panes > .ax-pane"),
                "Skip to main content"
            }
            header::Header {}
            div { class: "ax-body",
                rail::AppRail {}
                main { id: "ax-main", class: "ax-module", tabindex: "-1", "aria-label": module.label(),
                    match module {
                        Module::Mail => rsx! { crate::mail::MailModule {} },
                        Module::Calendar => rsx! { crate::calendar::CalendarModule {} },
                        Module::People => rsx! { crate::people::PeopleModule {} },
                        Module::Tasks => rsx! { crate::tasks::TasksModule {} },
                    }
                }
                if (app.my_day_open)() && module != Module::Tasks {
                    my_day::MyDayPane {}
                } else if pane == Some(SidePane::Notifications) {
                    notifications::NotificationsPane {}
                } else if pane == Some(SidePane::Help) {
                    help::HelpPane {}
                }
            }
            toasts::Toasts {}
            // App-wide background work owned by other areas: event and task
            // reminders, scheduled sends and undo-send holds.
            crate::calendar::ReminderHost {}
            crate::tasks::TaskReminders {}
            crate::mail::compose::ComposeBackground {}
            if (shell.shortcuts_open)() { shortcuts::ShortcutsDialog {} }
            if (shell.whats_new_open)() { help::WhatsNewDialog {} }
            if let Some(page) = (app.settings_dialog)() {
                crate::settings::SettingsDialog { page }
            }
            div { id: "ax-live-polite", class: "ax-visually-hidden", role: "status", "aria-live": "polite" }
            div { id: "ax-live-assertive", class: "ax-visually-hidden", role: "alert", "aria-live": "assertive" }
            div { id: "ax-overlay-root", class: "ax-overlay-root", "aria-hidden": "true" }
        }
    }
}

/// Global shortcuts (keyboard spec §3). Modules handle their own keys in
/// their containers first; text fields stop typing keys from reaching here
/// (`kit::keys::stop_typing_keys`), and kit dialogs and menus keep their keys.
fn on_global_key(app: AppState, shell: ShellCtx, e: &KeyboardEvent) {
    if e.is_composing() {
        return;
    }
    let chord = keys::chord(e);
    let mode = app.settings.read().keyboard_mode;
    let Some(cmd) = keymap::command_for(&chord, mode) else { return };
    let one_shot = !matches!(cmd, keymap::GlobalCmd::NextRegion | keymap::GlobalCmd::PrevRegion | keymap::GlobalCmd::Undo);
    e.prevent_default();
    e.stop_propagation();
    if one_shot && e.is_auto_repeating() {
        return;
    }
    run_command(app, shell, cmd);
}

/// Run a global command (also used by menus and the Help pane).
pub fn run_command(app: AppState, mut shell: ShellCtx, cmd: keymap::GlobalCmd) {
    use keymap::GlobalCmd as G;
    match cmd {
        G::GoMail => app.go(Module::Mail),
        G::GoCalendar => app.go(Module::Calendar),
        G::GoPeople => app.go(Module::People),
        G::GoTasks => app.go(Module::Tasks),
        G::NextRegion => dom::cycle_regions(1),
        G::PrevRegion => dom::cycle_regions(-1),
        G::Shortcuts => shell.shortcuts_open.set(true),
        G::Search => dom::focus(search::INPUT_SELECTOR),
        G::Undo => shell.undo_last(app),
        G::NewMessage => app.compose(ComposeRequest::new_message()),
        G::Sync if !*shell.online.peek() => {
            app.toast("You're offline. Your accounts will sync when you reconnect.");
        }
        G::Sync => match sync() {
            Some(s) => {
                s.sync_now();
                app.toast("Syncing your accounts…");
            }
            None => {
                app.toast("Everything is up to date. This preview has no accounts to sync.");
            }
        },
        G::Settings => {
            let page = (shell.settings_last)().unwrap_or(PageId::EmailAccounts);
            shell.open_settings(app, page);
        }
        G::Help => {
            if (shell.pane)() != Some(SidePane::Help) {
                shell.toggle_pane(app, SidePane::Help);
            }
        }
    }
}

/// Open the account's Inbox (app name click, "Go to Inbox").
pub fn go_to_inbox(app: AppState) {
    let s = store();
    let inbox = s.primary_account().ok().flatten().and_then(|a| s.folder_by_role(a.id, FolderRole::Inbox).ok().flatten());
    match inbox {
        Some(f) => app.navigate(NavRequest::Folder(f.id)),
        None => app.go(Module::Mail),
    }
}

/// The settings page the module's "View settings" opens.
pub fn settings_page_for(module: Module) -> SettingsPage {
    match module {
        Module::Mail => SettingsPage::Mail,
        Module::Calendar => SettingsPage::Calendar,
        Module::People => SettingsPage::People,
        Module::Tasks => SettingsPage::General,
    }
}
