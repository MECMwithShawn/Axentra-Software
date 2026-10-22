//! Menus (Fluent MenuList): dropdowns, context menus and submenus.
//!
//! Keyboard (APG menu pattern): Up/Down/Home/End move, first letters jump,
//! Right opens a submenu, Left or Escape closes it, Enter/Space activate,
//! Escape or Tab closes the menu. Focus goes back to the element that opened
//! the menu.

use std::rc::Rc;

use dioxus::prelude::*;

use super::button::{Icon, Variant};
use super::dom;

/// One entry of a dropdown / context menu.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MenuItem {
    pub id: String,
    pub label: String,
    pub icon: Option<String>,
    pub shortcut: Option<String>,
    /// `Some` renders a check column; `Some(true)` shows the check mark.
    pub checked: Option<bool>,
    pub disabled: bool,
    pub danger: bool,
    pub divider: bool,
    pub header: bool,
    /// Colour square instead of an icon (categories, calendars).
    pub swatch: Option<String>,
    pub children: Vec<MenuItem>,
    /// Tooltip, e.g. why the item is disabled.
    pub tooltip: Option<String>,
    /// Checked items of a single-choice group (`menuitemradio`).
    pub radio: bool,
}

impl MenuItem {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self { id: id.into(), label: label.into(), ..Default::default() }
    }
    pub fn divider() -> Self {
        Self { divider: true, ..Default::default() }
    }
    pub fn header(label: impl Into<String>) -> Self {
        Self { header: true, label: label.into(), ..Default::default() }
    }
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }
    pub fn shortcut(mut self, s: impl Into<String>) -> Self {
        self.shortcut = Some(s.into());
        self
    }
    pub fn checked(mut self, c: bool) -> Self {
        self.checked = Some(c);
        self
    }
    /// A radio item (one of a group), checked or not.
    pub fn radio(mut self, c: bool) -> Self {
        self.checked = Some(c);
        self.radio = true;
        self
    }
    pub fn disabled(mut self, d: bool) -> Self {
        self.disabled = d;
        self
    }
    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }
    pub fn swatch(mut self, color: impl Into<String>) -> Self {
        self.swatch = Some(color.into());
        self
    }
    pub fn submenu(mut self, children: Vec<MenuItem>) -> Self {
        self.children = children;
        self
    }
    pub fn tooltip(mut self, t: impl Into<String>) -> Self {
        self.tooltip = Some(t.into());
        self
    }
}

/// Whether any item opens a submenu (the menu then must not clip, so it
/// doesn't scroll either).
fn has_submenus(items: &[MenuItem]) -> bool {
    items.iter().any(|i| !i.children.is_empty())
}

/// A menu at viewport position (x, y). It shifts itself to stay on screen.
/// Clicking outside or pressing Escape calls `on_close`; choosing an item
/// calls `on_select(id)` and then `on_close`.
#[component]
pub fn Menu(
    items: Vec<MenuItem>,
    x: f64,
    y: f64,
    on_select: EventHandler<String>,
    on_close: EventHandler<()>,
    /// `x` is the menu's right edge instead of its left edge.
    #[props(default)]
    anchor_end: bool,
    /// Accessible name of the menu.
    #[props(into)]
    label: Option<String>,
    #[props(default = 200)] min_width: u32,
) -> Element {
    let id = dom::use_unique_id("ax-menu");
    let nested = has_submenus(&items);
    let shift_x = if anchor_end { "-100%".to_string() } else { format!("min(0px, calc(100vw - {x}px - 100% - 8px))") };
    // Lifted to fit above the window's bottom edge, but never above its top.
    // A menu with submenus taller than the window scrolls its items in an
    // inner box (dom.js `fit`), so the submenus aren't clipped.
    let style = format!(
        "left:{x}px;top:{y}px;min-width:{min_width}px;transform:translate({shift_x}, max(calc(8px - {y}px), min(0px, calc(100vh - {y}px - 100% - 8px))))"
    );
    rsx! {
        div {
            class: "ax-menu-backdrop",
            onclick: move |e| { e.stop_propagation(); on_close.call(()) },
            oncontextmenu: move |e| { e.prevent_default(); e.stop_propagation(); on_close.call(()) },
        }
        div {
            id: "{id}",
            class: "ax-menu",
            // A scrolling menu would clip its submenus.
            class: if nested { "ax-menu-nested" },
            role: "menu",
            "aria-label": label,
            tabindex: "-1",
            "data-ax-roving": "vertical",
            "data-ax-wrap": "true",
            "data-ax-typeahead": "true",
            style: "{style}",
            onmounted: {
                let id = id.clone();
                move |_| dom::open_popup(&id)
            },
            // Keys inside an open menu belong to the menu, never to module or
            // global shortcuts (keyboard spec §2.1 scope 2).
            onkeydown: move |e: KeyboardEvent| {
                e.stop_propagation();
                if e.key() == Key::Escape || super::keys::is_tab(&e) {
                    on_close.call(());
                }
            },
            oncontextmenu: move |e| e.prevent_default(),
            onclick: move |e| e.stop_propagation(),
            if nested {
                div { class: "ax-menu-scroller",
                    MenuItems { items: items.clone(), on_select, on_close }
                }
            } else {
                MenuItems { items: items.clone(), on_select, on_close }
            }
        }
    }
}

#[component]
fn MenuItems(items: Vec<MenuItem>, on_select: EventHandler<String>, on_close: EventHandler<()>) -> Element {
    let mut open_sub = use_signal(|| None::<String>);
    // A submenu opened from the keyboard takes focus; one opened by hover does not.
    let mut sub_by_key = use_signal(|| false);
    let has_checks = items.iter().any(|i| i.checked.is_some());
    rsx! {
        for item in items.iter().cloned() {
            if item.divider {
                div { class: "ax-menu-divider", role: "separator" }
            } else if item.header {
                div { class: "ax-menu-header", role: "presentation", "{item.label}" }
            } else {
                div {
                    key: "{item.id}",
                    class: "ax-menu-item-wrap",
                    onmouseenter: {
                        let id = item.id.clone();
                        let has_children = !item.children.is_empty();
                        move |_| {
                            sub_by_key.set(false);
                            open_sub.set(has_children.then(|| id.clone()));
                        }
                    },
                    button {
                        r#type: "button",
                        role: if item.radio { "menuitemradio" } else if item.checked.is_some() { "menuitemcheckbox" } else { "menuitem" },
                        "aria-checked": item.checked.map(|c| if c { "true" } else { "false" }),
                        "aria-disabled": if item.disabled { "true" },
                        "aria-haspopup": if !item.children.is_empty() { "menu" },
                        "aria-expanded": if !item.children.is_empty() { if open_sub() == Some(item.id.clone()) { "true" } else { "false" } },
                        "aria-keyshortcuts": item.shortcut.as_deref().map(super::keys_to_aria),
                        "data-ax-item": "true",
                        tabindex: "-1",
                        title: item.tooltip.clone(),
                        class: "ax-menu-item",
                        class: if item.disabled { "ax-disabled" },
                        class: if item.danger { "ax-danger" },
                        class: if open_sub() == Some(item.id.clone()) { "ax-active" },
                        onclick: {
                            let id = item.id.clone();
                            let has_children = !item.children.is_empty();
                            let disabled = item.disabled;
                            move |e: MouseEvent| {
                                e.stop_propagation();
                                if disabled {
                                    return;
                                }
                                if has_children {
                                    open_sub.set(Some(id.clone()));
                                } else {
                                    on_select.call(id.clone());
                                    on_close.call(());
                                }
                            }
                        },
                        onkeydown: {
                            let id = item.id.clone();
                            let has_children = !item.children.is_empty();
                            let disabled = item.disabled;
                            move |e: KeyboardEvent| {
                                if has_children && !disabled && matches!(e.key(), Key::ArrowRight | Key::Enter) {
                                    e.prevent_default();
                                    e.stop_propagation();
                                    sub_by_key.set(true);
                                    open_sub.set(Some(id.clone()));
                                }
                            }
                        },
                        if has_checks {
                            span { class: "ax-menu-check",
                                if item.checked == Some(true) {
                                    Icon { name: if item.radio { "circle_filled" } else { "checkmark" }, size: if item.radio { 8 } else { 16 } }
                                }
                            }
                        }
                        if let Some(color) = &item.swatch {
                            span { class: "ax-menu-swatch", style: "background:{color}" }
                        } else if let Some(icon) = &item.icon {
                            Icon { name: icon.clone() }
                        }
                        span { class: "ax-menu-label", "{item.label}" }
                        if let Some(s) = &item.shortcut { span { class: "ax-menu-shortcut", "{s}" } }
                        if !item.children.is_empty() { Icon { name: "chevron_right", size: 16, class: "ax-menu-chevron" } }
                    }
                    if !item.children.is_empty() && open_sub() == Some(item.id.clone()) {
                        Submenu {
                            items: item.children.clone(),
                            label: item.label.clone(),
                            take_focus: sub_by_key(),
                            on_select,
                            on_close,
                            on_back: move |_| open_sub.set(None),
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn Submenu(
    items: Vec<MenuItem>,
    label: String,
    take_focus: bool,
    on_select: EventHandler<String>,
    on_close: EventHandler<()>,
    on_back: EventHandler<()>,
) -> Element {
    let id = dom::use_unique_id("ax-submenu");
    let nested = has_submenus(&items);
    rsx! {
        div {
            id: "{id}",
            class: "ax-menu ax-submenu",
            class: if nested { "ax-menu-nested" },
            role: "menu",
            "aria-label": "{label}",
            "data-ax-roving": "vertical",
            "data-ax-wrap": "true",
            "data-ax-typeahead": "true",
            onmounted: {
                let id = id.clone();
                move |_| if take_focus { dom::open_popup(&id) } else { dom::fit(&id) }
            },
            onkeydown: move |e: KeyboardEvent| {
                if matches!(e.key(), Key::ArrowLeft | Key::Escape) {
                    e.prevent_default();
                    e.stop_propagation();
                    on_back.call(());
                }
            },
            MenuItems { items, on_select, on_close }
        }
    }
}

/// A command that opens a menu below itself (click, Enter, Space, Down or
/// Alt+Down).
#[component]
pub fn MenuButton(
    #[props(into)] label: String,
    #[props(into)] icon: Option<String>,
    items: Vec<MenuItem>,
    on_select: EventHandler<String>,
    #[props(default)] variant: Variant,
    #[props(default)] icon_only: bool,
    #[props(default = true)] chevron: bool,
    #[props(default)] disabled: bool,
    #[props(into, default)] class: String,
    /// Tooltip (defaults to the label).
    #[props(into)]
    title: Option<String>,
    /// Align the menu's right edge with the button's right edge.
    #[props(default)]
    align_end: bool,
) -> Element {
    let mut pos = use_signal(|| None::<(f64, f64)>);
    let mut mounted = use_signal(|| None::<Rc<MountedData>>);
    let v = variant.class();
    let open = move |fallback: (f64, f64)| {
        if disabled {
            return;
        }
        spawn(async move {
            let rect = match mounted() {
                Some(m) => m.get_client_rect().await.ok(),
                None => None,
            };
            pos.set(Some(match rect {
                Some(r) if align_end => (r.max_x(), r.max_y() + 4.0),
                Some(r) => (r.min_x(), r.max_y() + 4.0),
                None => fallback,
            }));
        });
    };
    rsx! {
        button {
            r#type: "button",
            class: if icon_only { "ax-icon-btn {class}" } else { "ax-btn {v} {class}" },
            title: title.clone().unwrap_or_else(|| label.clone()),
            "aria-label": "{label}",
            "aria-haspopup": "menu",
            "aria-expanded": if pos().is_some() { "true" } else { "false" },
            "aria-disabled": if disabled { "true" },
            onmounted: move |e| mounted.set(Some(e.data())),
            onclick: move |e: MouseEvent| {
                e.stop_propagation();
                let c = e.client_coordinates();
                open((c.x, c.y + 8.0));
            },
            onkeydown: move |e: KeyboardEvent| {
                if e.key() == Key::ArrowDown {
                    e.prevent_default();
                    e.stop_propagation();
                    open((0.0, 0.0));
                }
            },
            if let Some(i) = &icon { Icon { name: i.clone() } }
            if !icon_only && !label.is_empty() { span { class: "ax-btn-label", "{label}" } }
            if chevron && !icon_only { Icon { name: "chevron_down", size: 12, class: "ax-btn-chevron" } }
        }
        if let Some((x, y)) = pos() {
            Menu { items: items.clone(), x, y, anchor_end: align_end, label: label.clone(), on_select, on_close: move |_| pos.set(None) }
        }
    }
}
