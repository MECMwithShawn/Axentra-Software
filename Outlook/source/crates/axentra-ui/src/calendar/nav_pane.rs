//! Folder pane: New event, the mini month, "Add calendar" and the calendar
//! groups with coloured checkboxes and a per-calendar menu.

use axentra_core::*;
use chrono::Duration;
use dioxus::prelude::*;

use super::colors::{PALETTE, color_name};
use super::mini_month::MiniMonth;
use super::range::ViewKind;
use super::state::*;
use crate::kit::{Icon, IconButton, Menu, MenuButton, MenuItem, Variant, keys};
use crate::state::{AppState, store, use_app};

#[component]
pub fn NavPane() -> Element {
    let app = use_app();
    let mut cal = use_cal();
    let s = app.settings.read().clone();
    let _ = app.calendar_rev();
    let cals = store().calendars().unwrap_or_default();
    let view = (cal.view)();
    let days = cal.days(&s);
    let anchor = (cal.anchor)();
    let (range, selected) = if view == ViewKind::Month { (Vec::new(), Some(anchor)) } else { (days.clone(), None) };
    let new_menu: Vec<MenuItem> = std::iter::once(MenuItem::new("event", "Event").icon("calendar_add"))
        .chain(cals.iter().filter(|c| c.can_edit && !c.is_default).map(|c| MenuItem::new(format!("in:{}", c.id), format!("Event in {}", c.name)).swatch(c.color.clone())))
        .collect();
    let width = if s.show_week_numbers { 244 } else { 216 };
    rsx! {
        // An F6 stop of its own (with the grid, calendar/mod.rs).
        nav { class: "ax-cal-nav", style: "width:{width}px", "aria-label": "Navigation pane", "data-ax-region": "true",
            div { class: "ax-cal-newevent",
                button {
                    r#type: "button",
                    class: "ax-btn ax-primary ax-cal-newevent-main",
                    title: "New event (Ctrl+N)",
                    onclick: move |_| new_event_now(app, cal, None),
                    Icon { name: "calendar_add" }
                    span { "New event" }
                }
                MenuButton {
                    label: "More new event options",
                    icon: "chevron_down",
                    icon_only: true,
                    variant: Variant::Primary,
                    class: "ax-cal-newevent-chevron",
                    items: new_menu,
                    on_select: move |id: String| {
                        let target = id.strip_prefix("in:").and_then(|v| v.parse::<CalendarId>().ok());
                        new_event_now(app, cal, target);
                    },
                }
            }
            div { class: "ax-cal-nav-scroll",
                MiniMonth {
                    month: (cal.mini_month)(),
                    on_month: move |m| cal.mini_month.set(m),
                    on_pick: move |d| {
                        let s = app.settings.peek().clone();
                        cal.go_to(&s, d);
                    },
                    today: (cal.today)(),
                    week_start: s.week_start,
                    range,
                    selected,
                    week_numbers: s.show_week_numbers,
                    on_week: move |d| {
                        let s = app.settings.peek().clone();
                        cal.set_view(app, &s, ViewKind::Week);
                        cal.go_to(&s, d);
                    },
                }
                button {
                    r#type: "button",
                    class: "ax-cal-add-calendar",
                    onclick: move |_| cal.dialog.set(Some(CalDialog::AddCalendar)),
                    Icon { name: "calendar_add" }
                    span { "Add calendar" }
                }
                CalendarGroups { cals }
            }
        }
    }
}

/// New event from the button: the selected slot range if there is one,
/// otherwise the next half hour with the default length.
pub fn new_event_now(app: AppState, mut cal: Cal, calendar: Option<CalendarId>) {
    if let Some(id) = calendar {
        cal.active_calendar.set(Some(id));
    }
    let slot = *cal.slot.peek();
    match slot {
        Some(s) => cal.new_event(app, s.start(), s.end(), s.all_day),
        None => {
            let start = next_half_hour(chrono::Utc::now());
            let len = app.settings.peek().default_event_minutes.max(5) as i64;
            cal.new_event(app, start, start + Duration::minutes(len), false);
        }
    }
}

/// The group a calendar is listed under (blank means "My calendars").
fn group_of(c: &Calendar) -> &str {
    if c.group.trim().is_empty() { MY_CALENDARS } else { c.group.trim() }
}

const MY_CALENDARS: &str = "My calendars";

/// One account's groups in folder-pane order with their calendars (empty
/// groups made with "New calendar group" included).
fn grouped(account_id: AccountId, cals: &[Calendar]) -> Vec<(String, Vec<Calendar>)> {
    let names = store().calendar_groups(account_id).unwrap_or_else(|_| vec![MY_CALENDARS.to_string()]);
    names
        .into_iter()
        .map(|g| {
            let list = cals.iter().filter(|c| c.account_id == account_id && group_of(c) == g).cloned().collect();
            (g, list)
        })
        .collect()
}

/// Calendars listed before "Show all" appears.
const GROUP_PREVIEW: usize = 5;

/// What the inline name field in the calendar list is editing.
#[derive(Clone, Debug, PartialEq)]
enum GroupEdit {
    /// "New calendar group" for an account.
    New(AccountId),
    /// "Rename group".
    Rename(AccountId, String),
}

#[component]
fn CalendarGroups(cals: Vec<Calendar>) -> Element {
    // Group order and empty groups live in the store too.
    let _ = use_app().calendar_rev();
    let accounts = store().accounts().unwrap_or_default();
    // Accounts that have calendars, in calendar order.
    let mut account_ids: Vec<AccountId> = Vec::new();
    for c in &cals {
        if !account_ids.contains(&c.account_id) {
            account_ids.push(c.account_id);
        }
    }
    let multi = account_ids.len() > 1;
    let visible_total = cals.iter().filter(|c| c.is_visible).count();
    let edit = use_signal(|| None::<GroupEdit>);
    rsx! {
        div { class: "ax-cal-groups", role: "tree", "aria-label": "Calendars",
            for acct in account_ids {
                {
                    let groups = grouped(acct, &cals);
                    let email = accounts.iter().find(|a| a.id == acct).map(|a| a.email.clone()).unwrap_or_default();
                    let names: Vec<String> = groups.iter().map(|(g, _)| g.clone()).collect();
                    let count = groups.len();
                    rsx! {
                        if multi {
                            AccountHeader { key: "acct-{acct}", account_id: acct, email, edit }
                        }
                        for (i, (name, list)) in groups.into_iter().enumerate() {
                            GroupBlock {
                                key: "{acct}/{name}",
                                account_id: acct,
                                name,
                                list,
                                index: i,
                                count,
                                names: names.clone(),
                                visible_total,
                                edit,
                            }
                        }
                        if edit() == Some(GroupEdit::New(acct)) {
                            GroupNameField { key: "new-{acct}", account_id: acct, old: None, edit }
                        }
                    }
                }
            }
        }
    }
}

/// With several accounts: the account's address and its "New calendar group".
#[component]
fn AccountHeader(account_id: AccountId, email: String, edit: Signal<Option<GroupEdit>>) -> Element {
    let mut edit = edit;
    rsx! {
        div { class: "ax-cal-account-head",
            span { class: "ax-ellipsis ax-grow", title: "{email}", "{email}" }
            MenuButton {
                label: "More options",
                icon: "more_horizontal",
                icon_only: true,
                class: "ax-small ax-cal-row-more",
                items: vec![MenuItem::new("newgroup", "New calendar group").icon("add")],
                on_select: move |_| edit.set(Some(GroupEdit::New(account_id))),
            }
        }
    }
}

/// Inline name field for "New calendar group" / "Rename group".
#[component]
fn GroupNameField(account_id: AccountId, old: Option<String>, edit: Signal<Option<GroupEdit>>) -> Element {
    let app = use_app();
    let mut edit = edit;
    let mut text = use_signal(|| old.clone().unwrap_or_default());
    let label = if old.is_some() { "Group name" } else { "New calendar group" };
    let mut commit = move |old: Option<String>| {
        if edit.peek().is_none() {
            return;
        }
        edit.set(None);
        let name = text.peek().trim().to_string();
        if name.is_empty() || old.as_deref() == Some(name.as_str()) {
            return;
        }
        let r = match &old {
            Some(o) => store().rename_calendar_group(account_id, o, &name),
            None => store().add_calendar_group(account_id, &name),
        };
        if app.check(r).is_some() {
            app.bump_calendar();
        }
    };
    let (old1, old2) = (old.clone(), old.clone());
    rsx! {
        input {
            class: "ax-input ax-cal-group-input",
            value: "{text}",
            placeholder: "{label}",
            "aria-label": "{label}",
            onmounted: move |e| async move { let _ = e.set_focus(true).await; },
            oninput: move |e| text.set(e.value()),
            onkeydown: move |e: KeyboardEvent| {
                keys::stop_typing_keys(&e);
                match e.key() {
                    Key::Enter => commit(old1.clone()),
                    Key::Escape => {
                        e.stop_propagation();
                        edit.set(None);
                    }
                    _ => {}
                }
            },
            onblur: move |_| commit(old2.clone()),
        }
    }
}

#[component]
fn GroupBlock(
    account_id: AccountId,
    name: String,
    list: Vec<Calendar>,
    index: usize,
    count: usize,
    names: Vec<String>,
    visible_total: usize,
    edit: Signal<Option<GroupEdit>>,
) -> Element {
    let app = use_app();
    let mut cal = use_cal();
    let mut edit = edit;
    let key = format!("{account_id}/{name}");
    let is_collapsed = (cal.collapsed_groups)().contains(&key);
    let show_all = (cal.expanded_groups)().contains(&key);
    let all_on = !list.is_empty() && list.iter().all(|c| c.is_visible);
    let some_on = list.iter().any(|c| c.is_visible);
    let more = list.len() > GROUP_PREVIEW && !show_all;
    let shown: Vec<Calendar> = if more { list.iter().take(GROUP_PREVIEW).cloned().collect() } else { list.clone() };
    let ids: Vec<CalendarId> = list.iter().map(|c| c.id).collect();
    let others_visible = visible_total - list.iter().filter(|c| c.is_visible).count();
    let renaming = edit() == Some(GroupEdit::Rename(account_id, name.clone()));
    let mine = name == MY_CALENDARS;
    // Hover "More options" (spec §3.4): new / rename / delete / reorder.
    let menu = vec![
        MenuItem::new("new", "New calendar group").icon("add"),
        MenuItem::new("rename", "Rename group").icon("edit").disabled(mine),
        MenuItem::new("delete", "Delete group").icon("delete").disabled(mine || !list.is_empty()).shortcut(if !mine && !list.is_empty() { "Not empty" } else { "" }),
        MenuItem::divider(),
        MenuItem::new("up", "Move up").icon("arrow_up").disabled(mine || index <= 1),
        MenuItem::new("down", "Move down").icon("arrow_down").disabled(mine || index + 1 >= count),
    ];
    let (k1, k2, k3, n1) = (key.clone(), key.clone(), key.clone(), name.clone());
    rsx! {
        div { class: "ax-cal-group", role: "treeitem", "aria-expanded": if is_collapsed { "false" } else { "true" },
            div { class: "ax-cal-group-head",
                button {
                    r#type: "button",
                    class: "ax-cal-group-chevron",
                    title: if is_collapsed { format!("Expand {name}") } else { format!("Collapse {name}") },
                    onclick: move |_| {
                        let mut set = cal.collapsed_groups.peek().clone();
                        if !set.remove(&k1) {
                            set.insert(k1.clone());
                        }
                        cal.collapsed_groups.set(set);
                    },
                    Icon { name: if is_collapsed { "chevron_right" } else { "chevron_down" }, size: 12 }
                }
                ColorCheck {
                    checked: all_on,
                    mixed: some_on && !all_on,
                    color: None,
                    label: format!("Show all calendars in {name}"),
                    onchange: move |on: bool| {
                        if !on && others_visible == 0 {
                            app.toast("At least one calendar has to stay selected.");
                            return;
                        }
                        let r = ids.iter().try_for_each(|id| store().set_calendar_visible(*id, on));
                        if app.check(r).is_some() {
                            app.bump_calendar();
                        }
                    },
                }
                if renaming {
                    GroupNameField { account_id, old: Some(name.clone()), edit }
                } else {
                    span { class: "ax-cal-group-name ax-grow", "{name}" }
                    MenuButton {
                        label: "More options",
                        icon: "more_horizontal",
                        icon_only: true,
                        class: "ax-small ax-cal-row-more",
                        items: menu,
                        on_select: move |cmd: String| {
                            let r = match cmd.as_str() {
                                "new" => {
                                    edit.set(Some(GroupEdit::New(account_id)));
                                    return;
                                }
                                "rename" => {
                                    edit.set(Some(GroupEdit::Rename(account_id, n1.clone())));
                                    return;
                                }
                                "delete" => store().delete_calendar_group(account_id, &n1),
                                "up" => store().move_calendar_group(account_id, &n1, -1),
                                "down" => store().move_calendar_group(account_id, &n1, 1),
                                _ => return,
                            };
                            if app.check(r).is_some() {
                                app.bump_calendar();
                            }
                        },
                    }
                }
            }
            if !is_collapsed {
                div { role: "group",
                    if list.is_empty() {
                        div { class: "ax-cal-group-empty", "No calendars" }
                    }
                    for c in shown {
                        CalendarRow { key: "{c.id}", calendar: c, can_hide: visible_total > 1, groups: names.clone() }
                    }
                    if more {
                        button { r#type: "button", class: "ax-cal-show-all", onclick: move |_| { cal.expanded_groups.write().insert(k2.clone()); }, "Show all" }
                    } else if show_all {
                        button { r#type: "button", class: "ax-cal-show-all", onclick: move |_| { cal.expanded_groups.write().remove(&k3); }, "Show less" }
                    }
                }
            }
        }
    }
}

/// A 16px checkbox drawn in a calendar's colour.
#[component]
fn ColorCheck(checked: bool, #[props(default)] mixed: bool, color: Option<String>, label: String, onchange: EventHandler<bool>, #[props(default)] shake: u64) -> Element {
    let style = match (&color, checked) {
        (Some(c), true) => format!("background:{c};border-color:{c}"),
        (Some(c), false) => format!("border-color:{c}"),
        _ => String::new(),
    };
    rsx! {
        span {
            class: "ax-cal-check",
            class: if checked { "ax-checked" },
            class: if mixed { "ax-mixed" },
            class: if shake % 2 == 1 { "ax-shake-a" } else if shake > 0 { "ax-shake-b" },
            role: "checkbox",
            tabindex: "0",
            "aria-checked": if mixed { "mixed" } else if checked { "true" } else { "false" },
            "aria-label": "{label}",
            title: "{label}",
            style: "{style}",
            onclick: move |e| { e.stop_propagation(); onchange.call(!checked) },
            onkeydown: move |e: KeyboardEvent| {
                if e.key() == Key::Character(" ".into()) || e.key() == Key::Enter {
                    e.prevent_default();
                    e.stop_propagation();
                    onchange.call(!checked);
                }
            },
            if checked { Icon { name: "checkmark", size: 12 } } else if mixed { Icon { name: "subtract", size: 12 } }
        }
    }
}

#[component]
fn CalendarRow(calendar: Calendar, can_hide: bool, groups: Vec<String>) -> Element {
    let app = use_app();
    let mut cal = use_cal();
    let mut menu_at = use_signal(|| None::<(f64, f64)>);
    let mut draft = use_signal(String::new);
    let c = calendar.clone();
    let id = c.id;
    let active = (cal.active_calendar)() == Some(id);
    let renaming = (cal.renaming)() == Some(id);
    let shake = (cal.shake)().filter(|(sid, _)| *sid == id).map(|(_, n)| n).unwrap_or(0);
    let items = calendar_menu(&c, &groups);
    let visible = c.is_visible;
    rsx! {
        div {
            class: "ax-cal-row",
            class: if active { "ax-selected" },
            role: "treeitem",
            "aria-selected": if active { "true" } else { "false" },
            oncontextmenu: move |e: MouseEvent| {
                e.prevent_default();
                e.stop_propagation();
                let p = e.client_coordinates();
                menu_at.set(Some((p.x, p.y)));
            },
            ColorCheck {
                checked: visible,
                color: Some(c.color.clone()),
                label: format!("{} ({})", c.name, if visible { "shown" } else { "hidden" }),
                shake,
                onchange: move |on: bool| {
                    if !on && !can_hide {
                        // At least one calendar must stay checked.
                        let n = (*cal.shake.peek()).map(|(_, n)| n).unwrap_or(0) + 1;
                        cal.shake.set(Some((id, n)));
                        return;
                    }
                    if app.check(store().set_calendar_visible(id, on)).is_some() {
                        app.bump_calendar();
                    }
                },
            }
            if renaming {
                input {
                    class: "ax-input ax-cal-rename",
                    value: "{draft}",
                    "aria-label": "Calendar name",
                    onmounted: move |e| async move { let _ = e.set_focus(true).await; },
                    oninput: move |e| draft.set(e.value()),
                    onkeydown: move |e: KeyboardEvent| {
                        keys::stop_typing_keys(&e);
                        match e.key() {
                            Key::Enter => commit_rename(app, cal, id, &draft()),
                            Key::Escape => {
                                e.stop_propagation();
                                cal.renaming.set(None);
                            }
                            _ => {}
                        }
                    },
                    onblur: move |_| commit_rename(app, cal, id, &draft()),
                }
            } else {
                button {
                    r#type: "button",
                    class: "ax-cal-row-name",
                    title: "{c.name}",
                    onclick: move |_| {
                        cal.active_calendar.set(Some(id));
                        if !visible && app.check(store().set_calendar_visible(id, true)).is_some() {
                            app.bump_calendar();
                        }
                    },
                    span { class: "ax-ellipsis", "{c.name}" }
                }
                if !c.can_edit {
                    span { class: "ax-cal-row-badge", title: "Read-only calendar", Icon { name: "lock_closed", size: 12 } }
                }
                IconButton {
                    icon: "more_horizontal",
                    title: "More options",
                    small: true,
                    class: "ax-cal-row-more",
                    onclick: move |e: MouseEvent| {
                        let p = e.client_coordinates();
                        menu_at.set(Some((p.x, p.y + 8.0)));
                    },
                }
            }
            if let Some((x, y)) = menu_at() {
                Menu {
                    items,
                    x,
                    y,
                    on_select: move |cmd: String| {
                        if cmd == "rename" {
                            draft.set(calendar.name.clone());
                            cal.renaming.set(Some(id));
                        } else {
                            calendar_command(app, cal, id, &cmd);
                        }
                    },
                    on_close: move |_| menu_at.set(None),
                }
            }
        }
    }
}

fn commit_rename(app: AppState, mut cal: Cal, id: CalendarId, text: &str) {
    if cal.renaming.peek().is_none() {
        return;
    }
    cal.renaming.set(None);
    let unchanged = store().calendar(id).ok().flatten().is_some_and(|c| c.name == text.trim());
    if text.trim().is_empty() || unchanged {
        return;
    }
    if app.check(store().rename_calendar(id, text)).is_some() {
        app.bump_calendar();
    }
}

fn calendar_menu(c: &Calendar, groups: &[String]) -> Vec<MenuItem> {
    let colors: Vec<MenuItem> = PALETTE
        .iter()
        .map(|s| MenuItem::new(format!("color:{}", s.primary), s.name).swatch(s.primary).checked(color_name(&c.color) == s.name))
        .collect();
    let current = group_of(c);
    let mut move_to: Vec<MenuItem> = groups.iter().filter(|g| g.as_str() != current).map(|g| MenuItem::new(format!("move:{g}"), g.clone())).collect();
    if current != "Other calendars" && !groups.iter().any(|x| x == "Other calendars") {
        move_to.push(MenuItem::new("move:Other calendars", "Other calendars"));
    }
    let mut items = vec![
        MenuItem::new("color", "Color").icon("color").submenu(colors),
        MenuItem::new("rename", "Rename").icon("edit"),
        MenuItem::new("only", "Show only this calendar").icon("eye"),
        // Shown greyed with its reason: local calendars can't be shared.
        MenuItem::new("share", "Sharing and permissions").icon("share").disabled(true).shortcut("Local only"),
    ];
    if !move_to.is_empty() {
        items.push(MenuItem::new("move", "Move to").icon("folder_arrow_right").submenu(move_to));
    }
    items.push(MenuItem::divider());
    let remove_label = if c.can_edit { "Delete" } else { "Remove" };
    items.push(MenuItem::new("delete", remove_label).icon("delete").disabled(c.is_default).danger());
    items
}

fn calendar_command(app: AppState, mut cal: Cal, id: CalendarId, cmd: &str) {
    let r = if let Some(color) = cmd.strip_prefix("color:") {
        store().set_calendar_color(id, color)
    } else if let Some(group) = cmd.strip_prefix("move:") {
        store().calendar(id).and_then(|c| match c {
            Some(mut c) => {
                c.group = group.to_string();
                store().update_calendar(&c)
            }
            None => Ok(()),
        })
    } else {
        match cmd {
            "only" => store().show_only_calendar(id),
            "delete" => {
                cal.dialog.set(Some(CalDialog::DeleteCalendar(id)));
                return;
            }
            _ => return,
        }
    };
    if app.check(r).is_some() {
        app.bump_calendar();
    }
}
