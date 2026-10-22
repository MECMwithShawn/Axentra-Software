//! The ribbon (docs/spec/shell.md §5–§6.12): tab strip plus either the
//! Simplified command row (with "More options" overflow) or the Classic
//! layout (large/small buttons in labelled groups). It can collapse to its
//! tab row, and has key tips and arrow-key navigation.
//!
//! Modules describe their ribbon as data ([`RibbonTab`] / [`Command`]) and
//! receive the chosen command id (or menu item id) through `on_command`.
//! Ids starting with `shell.` are handled by the shell (see [`help_tab`]); a
//! Help tab is added when a module doesn't provide one.

mod classic;
mod command;
pub mod custom;
mod customize;
pub mod layout;
mod simplified;

use std::rc::Rc;

use axentra_core::AccountId;
use dioxus::prelude::*;

use crate::kit::{IconButton, Menu, MenuItem, dom};
use crate::state::{AppState, store, use_app};

use super::ShellCtx;

#[derive(Clone, Debug, PartialEq)]
pub struct RibbonTab {
    pub id: String,
    pub label: String,
    /// Groups of commands; a separator is drawn between groups. In the
    /// Classic layout each group gets a label (see [`Command::group`]).
    pub groups: Vec<Vec<Command>>,
}

impl RibbonTab {
    pub fn new(id: impl Into<String>, label: impl Into<String>, groups: Vec<Vec<Command>>) -> Self {
        Self { id: id.into(), label: label.into(), groups }
    }

    /// Label of group `i` in the Classic layout: the first `.group(..)` set
    /// on one of its commands.
    pub fn group_label(&self, i: usize) -> String {
        self.groups.get(i).and_then(|g| g.iter().find_map(|c| c.group.clone())).unwrap_or_default()
    }
}

/// Size of a command in the Classic layout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CommandSize {
    /// Large for the first commands of a group, small after (§6.4 shapes).
    #[default]
    Auto,
    /// 32px icon over a two-line label.
    Large,
    /// 16px icon beside the label, three per column.
    Small,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Command {
    pub id: String,
    pub label: String,
    pub icon: String,
    /// Non-empty: clicking opens this menu (or the chevron does, if `split`).
    pub menu: Vec<MenuItem>,
    /// Split button: the main part runs `id`, the chevron opens `menu`.
    pub split: bool,
    pub primary: bool,
    pub disabled: bool,
    pub toggled: bool,
    pub icon_only: bool,
    pub tooltip: Option<String>,
    /// Keyboard shortcut shown in the tooltip, e.g. "Ctrl+N".
    pub shortcut: Option<String>,
    /// Overflow priority, 1 = most important (default: ribbon order).
    pub priority: Option<u8>,
    /// Classic layout size.
    pub size: CommandSize,
    /// Key tip letters (default: derived from the label).
    pub keytip: Option<String>,
    /// Classic group label for the group this command is in.
    pub group: Option<String>,
    /// A press leaves focus where it is (no blur of the field being typed
    /// in before the command runs).
    pub keep_focus: bool,
}

impl Command {
    pub fn new(id: impl Into<String>, label: impl Into<String>, icon: impl Into<String>) -> Self {
        Self { id: id.into(), label: label.into(), icon: icon.into(), ..Default::default() }
    }
    pub fn menu(mut self, items: Vec<MenuItem>) -> Self {
        self.menu = items;
        self
    }
    pub fn split(mut self, items: Vec<MenuItem>) -> Self {
        self.menu = items;
        self.split = true;
        self
    }
    pub fn primary(mut self) -> Self {
        self.primary = true;
        self
    }
    pub fn disabled(mut self, d: bool) -> Self {
        self.disabled = d;
        self
    }
    pub fn toggled(mut self, t: bool) -> Self {
        self.toggled = t;
        self
    }
    pub fn icon_only(mut self) -> Self {
        self.icon_only = true;
        self
    }
    pub fn tooltip(mut self, t: impl Into<String>) -> Self {
        self.tooltip = Some(t.into());
        self
    }
    pub fn shortcut(mut self, s: impl Into<String>) -> Self {
        self.shortcut = Some(s.into());
        self
    }
    pub fn priority(mut self, p: u8) -> Self {
        self.priority = Some(p);
        self
    }
    pub fn large(mut self) -> Self {
        self.size = CommandSize::Large;
        self
    }
    pub fn small(mut self) -> Self {
        self.size = CommandSize::Small;
        self
    }
    pub fn keytip(mut self, k: impl Into<String>) -> Self {
        self.keytip = Some(k.into());
        self
    }
    pub fn group(mut self, label: impl Into<String>) -> Self {
        self.group = Some(label.into());
        self
    }
    pub fn keep_focus(mut self) -> Self {
        self.keep_focus = true;
        self
    }

    /// Tooltip text: explicit tooltip or label, plus the shortcut.
    pub fn title(&self) -> String {
        let base = self.tooltip.clone().unwrap_or_else(|| self.label.clone());
        match &self.shortcut {
            Some(s) if !base.contains('(') => format!("{base} ({s})"),
            _ => base,
        }
    }

    /// The command as a menu entry (overflow menu, collapsed Classic groups):
    /// split buttons become submenus whose first entry is the main action.
    pub fn as_menu_item(&self) -> MenuItem {
        let mut item = MenuItem::new(self.id.clone(), self.label.clone()).icon(self.icon.clone()).disabled(self.disabled);
        if let Some(t) = &self.tooltip {
            item = item.tooltip(t.clone());
        }
        if self.split && !self.menu.is_empty() {
            let mut children = vec![MenuItem::new(self.id.clone(), self.label.clone()).icon(self.icon.clone())];
            children.extend(self.menu.iter().cloned());
            item.submenu(children)
        } else if !self.menu.is_empty() {
            item.submenu(self.menu.clone())
        } else if self.toggled {
            item.checked(true)
        } else {
            match &self.shortcut {
                Some(s) => item.shortcut(s.clone()),
                None => item,
            }
        }
    }
}

/// The standard Help tab (§6.9). Commands with `shell.` ids are run by the
/// shell; modules can also use them in their own tabs.
pub fn help_tab() -> RibbonTab {
    RibbonTab::new(
        "help",
        "Help",
        vec![
            vec![
                Command::new("shell.help", "Help", "question_circle").shortcut("F1").group("Help").large(),
                Command::new("shell.whats_new", "What's new", "megaphone").large(),
                Command::new("shell.shortcuts", "Keyboard shortcuts", "keyboard").shortcut("?").large(),
            ],
            vec![
                Command::new("shell.feedback", "Feedback", "person_feedback").group("Contact").large(),
                Command::new("shell.support", "Support", "info").tooltip("Version and support information").large(),
                Command::new("shell.diagnostics", "Diagnostics", "document_text").tooltip("Copy diagnostic details to the clipboard").large(),
            ],
        ],
    )
}

/// Run a `shell.*` ribbon command. Returns false for module commands.
pub fn run_shell_command(app: AppState, mut shell: ShellCtx, id: &str) -> bool {
    use super::keymap::GlobalCmd;
    match id {
        "shell.help" => super::run_command(app, shell, GlobalCmd::Help),
        "shell.shortcuts" => shell.shortcuts_open.set(true),
        "shell.whats_new" => shell.whats_new_open.set(true),
        "shell.feedback" => super::help::send_feedback(app),
        "shell.support" => shell.open_settings(app, crate::settings::PageId::About),
        "shell.diagnostics" => {
            let s = crate::state::store();
            let text = format!(
                "{} {}\nDatabase: {}\nAccounts: {}\nSync engine: {}",
                crate::APP_NAME,
                env!("CARGO_PKG_VERSION"),
                s.path().map(|p| p.display().to_string()).unwrap_or_else(|| "in memory".into()),
                s.accounts().map(|a| a.len()).unwrap_or(0),
                if crate::state::sync().is_some() { "running" } else { "off" },
            );
            dom::run(&format!("navigator.clipboard && navigator.clipboard.writeText({})", dom::js_str(&text)));
            app.toast("Diagnostic details copied to the clipboard");
        }
        "shell.toggle_layout" => app.update_settings(|s| s.simplified_ribbon = !s.simplified_ribbon),
        _ => return false,
    }
    true
}

/// Key tip letters for well-known tabs (keyboard spec §22).
fn tab_keytip(id: &str) -> Option<&'static str> {
    Some(match id {
        "file" => "F",
        "home" => "H",
        "view" => "V",
        "help" => "Y",
        "search" => "S",
        "insert" => "N",
        "format_text" | "format" => "O",
        "draw" => "D",
        "options" => "P",
        "message" => "M",
        _ => return None,
    })
}

#[component]
pub fn Ribbon(
    tabs: Vec<RibbonTab>,
    on_command: EventHandler<String>,
    /// Hamburger at the far left ("Hide navigation pane").
    on_toggle_nav: Option<EventHandler<()>>,
    #[props(default)] nav_open: bool,
    /// Select this tab (e.g. the contextual Search tab); `None` = the user's choice.
    #[props(into)]
    selected: Option<String>,
    /// Called when the user picks a tab.
    on_tab_change: Option<EventHandler<String>>,
    /// Account whose ribbon customisation applies (Mail: the account of the
    /// selected folder); default: the primary account.
    account: Option<AccountId>,
) -> Element {
    let app = use_app();
    let shell = super::try_shell();
    let id = dom::use_unique_id("ax-ribbon");
    let mut tabs = tabs;
    if !tabs.iter().any(|t| t.id == "help") {
        tabs.push(help_tab());
    }
    let mut chosen = use_signal(|| tabs.iter().find(|t| t.id == "home").or(tabs.first()).map(|t| t.id.clone()).unwrap_or_default());
    // A controlled selection wins; an unknown id falls back to Home.
    let selected_id = selected
        .filter(|s| tabs.iter().any(|t| &t.id == s))
        .or_else(|| tabs.iter().any(|t| t.id == chosen()).then(&*chosen))
        .or_else(|| tabs.iter().find(|t| t.id == "home").or(tabs.first()).map(|t| t.id.clone()))
        .unwrap_or_default();
    let current = tabs.iter().find(|t| t.id == selected_id).cloned();
    let simplified = app.settings.read().simplified_ribbon;
    let collapsed = shell.map(|s| (s.ribbon_collapsed)()).unwrap_or(false);
    let mut overlay = use_signal(|| false);
    let mut tab_menu = use_signal(|| None::<(f64, f64)>);
    let mut customizing = use_signal(|| false);
    // Simplified-ribbon customisation (§6.10): Mail's Home and View tabs.
    let customizable = simplified && shell.is_some() && current.as_ref().is_some_and(|t| custom::customizable((app.module)(), &t.id));
    let account_id = account
        .or_else(|| {
            let _ = app.accounts_rev();
            store().primary_account().ok().flatten().map(|a| a.id)
        })
        .unwrap_or(0);
    let (shown, removed, labels) = match &current {
        Some(t) if customizable => {
            let _ = shell.map(|s| (s.ribbon_rev)());
            let saved = custom::load(account_id, &t.id);
            let (shown, removed) = saved.apply(t);
            (Some(shown), removed, saved.labels)
        }
        other => (other.clone(), Vec::new(), custom::LabelMode::default()),
    };
    let tab_tips = layout::assign_keytips(&tabs.iter().map(|t| (t.label.clone(), tab_keytip(&t.id).map(str::to_string))).collect::<Vec<_>>());
    let panel_id = format!("{id}-panel");
    let show_panel = !collapsed || overlay();

    let run = move |cmd: String| {
        overlay.set(false);
        if cmd == simplified::CUSTOMIZE_ID {
            customizing.set(true);
            return;
        }
        if let Some(sh) = shell
            && run_shell_command(app, sh, &cmd)
        {
            return;
        }
        on_command.call(cmd);
    };
    let mut select = move |tab: String| {
        chosen.set(tab.clone());
        if let Some(h) = &on_tab_change {
            h.call(tab);
        }
    };
    let mut toggle_collapsed = move || {
        if let Some(mut sh) = shell {
            let c = (sh.ribbon_collapsed)();
            sh.ribbon_collapsed.set(!c);
            overlay.set(false);
        }
    };

    rsx! {
        div {
            class: "ax-ribbon",
            class: if simplified { "ax-ribbon-simplified" } else { "ax-ribbon-classic" },
            class: if collapsed { "ax-ribbon-collapsed" },
            role: "region",
            "aria-label": "Ribbon",
            div { class: "ax-ribbon-tabs",
                if let Some(h) = on_toggle_nav {
                    IconButton {
                        icon: "navigation",
                        title: if nav_open { "Hide navigation pane" } else { "Show navigation pane" },
                        class: "ax-ribbon-nav",
                        onclick: move |_| h.call(()),
                    }
                }
                div { class: "ax-ribbon-tablist", role: "tablist", "aria-label": "Ribbon tabs", "data-ax-roving": "horizontal", "data-ax-wrap": "true",
                    for (i, tab) in tabs.iter().cloned().enumerate() {
                        RibbonTabButton {
                            key: "{tab.id}",
                            tab: tab.clone(),
                            selected: tab.id == selected_id,
                            keytip: tab_tips.get(i).cloned().unwrap_or_default(),
                            panel_id: panel_id.clone(),
                            ribbon_id: id.clone(),
                            on_select: {
                                let was = selected_id.clone();
                                move |t: String| {
                                    let same = t == was;
                                    select(t);
                                    if collapsed {
                                        overlay.set(!(same && overlay()));
                                    }
                                }
                            },
                            on_focus_select: move |t: String| select(t),
                            on_toggle_collapse: move |_| toggle_collapsed(),
                            on_menu: move |p| tab_menu.set(Some(p)),
                        }
                    }
                }
            }
            if let Some((x, y)) = tab_menu() {
                Menu {
                    x,
                    y,
                    label: "Ribbon options",
                    items: vec![
                        MenuItem::new("collapse", "Collapse the ribbon").checked(collapsed).disabled(shell.is_none()),
                        MenuItem::new("layout", if simplified { "Switch to classic ribbon" } else { "Switch to simplified ribbon" }).icon(if simplified { "chevron_down" } else { "chevron_up" }),
                    ],
                    on_close: move |_| tab_menu.set(None),
                    on_select: move |c: String| match c.as_str() {
                        "collapse" => toggle_collapsed(),
                        "layout" => app.update_settings(|s| s.simplified_ribbon = !s.simplified_ribbon),
                        _ => {}
                    },
                }
            }
            if collapsed && overlay() {
                div { class: "ax-ribbon-overlay-backdrop", onclick: move |_| overlay.set(false) }
            }
            if customizing() {
                if let Some(tab) = current.clone() {
                    customize::CustomizeDialog {
                        tab,
                        account: account_id,
                        on_close: move |_| customizing.set(false),
                        on_saved: move |_| {
                            if let Some(mut sh) = shell {
                                sh.ribbon_rev += 1;
                            }
                        },
                    }
                }
            }
            if show_panel {
                if let Some(tab) = current {
                    div {
                        id: "{panel_id}",
                        class: "ax-ribbon-panel",
                        class: if collapsed { "ax-ribbon-overlay" },
                        role: "tabpanel",
                        "aria-labelledby": "{id}-tab-{tab.id}",
                        onkeydown: move |e: KeyboardEvent| {
                            match e.key() {
                                // Up returns to the tab row (§6.12).
                                Key::ArrowUp if !e.modifiers().alt() => {
                                    e.prevent_default();
                                    dom::focus(&format!("#{}-tab-{}", id, selected_id));
                                }
                                Key::Escape if collapsed => {
                                    e.stop_propagation();
                                    overlay.set(false);
                                    dom::focus(&format!("#{}-tab-{}", id, selected_id));
                                }
                                _ => {}
                            }
                        },
                        if simplified {
                            simplified::SimplifiedRow {
                                tab: shown.clone().unwrap_or_else(|| tab.clone()),
                                labels,
                                removed: removed.clone(),
                                customizable,
                                on_command: run,
                            }
                        } else {
                            classic::ClassicRow { tab: tab.clone(), on_command: run }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn RibbonTabButton(
    tab: RibbonTab,
    selected: bool,
    keytip: String,
    panel_id: String,
    ribbon_id: String,
    on_select: EventHandler<String>,
    /// Arrow keys select the focused tab without opening a collapsed ribbon.
    on_focus_select: EventHandler<String>,
    on_toggle_collapse: EventHandler<()>,
    on_menu: EventHandler<(f64, f64)>,
) -> Element {
    let mut anchor = use_signal(|| None::<Rc<MountedData>>);
    let tab_id = tab.id.clone();
    let tab_id2 = tab.id.clone();
    let tab_id3 = tab.id.clone();
    let shortcut = crate::kit::keys_to_aria(&format!("Alt+{keytip}"));
    rsx! {
        button {
            id: "{ribbon_id}-tab-{tab.id}",
            r#type: "button",
            role: "tab",
            class: "ax-ribbon-tab",
            class: if selected { "ax-selected" },
            "aria-selected": if selected { "true" } else { "false" },
            "aria-controls": "{panel_id}",
            "aria-keyshortcuts": "{shortcut}",
            "data-ax-item": "true",
            "data-ax-keytip": "{keytip}",
            "data-ax-tab-id": "{tab.id}",
            tabindex: if selected { "0" } else { "-1" },
            onmounted: move |e| anchor.set(Some(e.data())),
            onclick: move |_| on_select.call(tab_id.clone()),
            ondoubleclick: move |_| on_toggle_collapse.call(()),
            oncontextmenu: move |e: MouseEvent| {
                e.prevent_default();
                let c = e.client_coordinates();
                on_menu.call((c.x, c.y));
            },
            onkeydown: move |e: KeyboardEvent| match e.key() {
                Key::ArrowDown => {
                    e.prevent_default();
                    on_focus_select.call(tab_id2.clone());
                    dom::focus_first_in(&format!("#{panel_id} [role=toolbar]"), "[data-ax-item]");
                }
                Key::ContextMenu => {
                    e.prevent_default();
                    spawn(async move {
                        let rect = match anchor() {
                            Some(m) => m.get_client_rect().await.ok(),
                            None => None,
                        };
                        on_menu.call(rect.map(|r| (r.min_x(), r.max_y())).unwrap_or((100.0, 80.0)));
                    });
                }
                Key::F10 if e.modifiers().shift() => {
                    e.prevent_default();
                    on_menu.call((100.0, 80.0));
                }
                _ => {}
            },
            onfocus: move |_| if !selected { on_focus_select.call(tab_id3.clone()) },
            span { class: "ax-ribbon-tab-label", "data-text": "{tab.label}", "{tab.label}" }
        }
    }
}
