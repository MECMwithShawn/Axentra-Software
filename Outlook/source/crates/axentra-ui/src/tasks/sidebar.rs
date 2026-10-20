//! Sidebar (3.3): smart lists, custom lists and groups, "New list" and
//! "Create group", context menus, inline rename and drop targets.

use axentra_core::*;
use dioxus::prelude::*;

use super::actions;
use super::ctx::{Confirm, Drag, PaneEdge, TodoCtx, View, use_todo};
use super::splitter::Splitter;
use super::menus::list_label;
use crate::kit::{Icon, IconButton, Menu, MenuItem, keys};
use crate::state::{AppState, store, use_app};

/// One sidebar row's data.
#[derive(Clone, Debug, PartialEq)]
struct Entry {
    view: View,
    label: String,
    icon: &'static str,
    /// Leading emoji instead of the icon (custom lists).
    emoji: Option<String>,
    count: usize,
    /// Smart list that can be hidden from the sidebar.
    hideable: bool,
    /// Custom list (rename, move, duplicate, delete).
    list: Option<TaskList>,
    indent: bool,
}

/// "999+" above 999; hidden when 0.
fn count_label(n: usize) -> String {
    if n > 999 { "999+".into() } else { n.to_string() }
}

fn smart_entry(view: View, label: &str, icon: &'static str, count: usize, hideable: bool) -> Entry {
    Entry { view, label: label.into(), icon, emoji: None, count, hideable, list: None, indent: false }
}

fn list_entry(l: &TaskList, count: usize, indent: bool) -> Entry {
    Entry {
        view: View::List(l.id),
        label: l.name.clone(),
        icon: "text_bullet_list_ltr",
        emoji: l.emoji.clone(),
        count,
        hideable: false,
        list: Some(l.clone()),
        indent,
    }
}

/// Expand a collapsed sidebar (rename needs the full width).
fn expand(ctx: TodoCtx) {
    if !ctx.prefs.peek().sidebar_open {
        ctx.update_prefs(|p| p.sidebar_open = true);
    }
}

/// Create "Untitled list" in rename mode and open it (New list, Ctrl+L).
pub fn new_list(app: AppState, mut ctx: TodoCtx, group: Option<&str>) {
    expand(ctx);
    if let Some(id) = app.check(store().create_task_list("", group)) {
        app.bump_tasks();
        ctx.go(View::List(id));
        ctx.renaming_list.set(Some(id));
    }
}

/// Create "Untitled group" in rename mode.
pub fn new_group(app: AppState, mut ctx: TodoCtx) {
    expand(ctx);
    if let Some(name) = app.check(store().create_task_group("")) {
        ctx.update_prefs(|p| p.collapsed_groups.retain(|g| *g != name));
        ctx.renaming_group.set(Some(name));
        app.bump_tasks();
    }
}

#[component]
pub fn Sidebar() -> Element {
    let app = use_app();
    let ctx = use_todo();
    let _ = app.tasks_rev();
    let _ = app.mail_rev();
    let prefs = ctx.prefs.read().clone();
    let today = (ctx.today)();
    let s = store();
    let counts = s.smart_list_counts_with(today, prefs.due_today_in_my_day).unwrap_or_default();
    let list_counts = s.task_list_counts().unwrap_or_default();
    let lists = s.task_lists().unwrap_or_default();
    let groups = s.task_groups().unwrap_or_default();
    let count = |l: SmartList| counts.get(&l).copied().unwrap_or(0);

    let mut smart = vec![
        smart_entry(View::Smart(SmartList::MyDay), "My Day", "weather_sunny", count(SmartList::MyDay), false),
        smart_entry(View::Smart(SmartList::Important), "Important", "star", count(SmartList::Important), true),
        smart_entry(View::Smart(SmartList::Planned), "Planned", "calendar_ltr", count(SmartList::Planned), true),
    ];
    if prefs.show_all {
        smart.push(smart_entry(View::All, "All", "apps_list", s.all_open_tasks().map(|v| v.len()).unwrap_or(0), true));
    }
    if prefs.show_completed {
        smart.push(smart_entry(View::Completed, "Completed", "checkmark_circle", 0, true));
    }
    smart.extend([
        smart_entry(View::Smart(SmartList::Assigned), "Assigned to me", "person", count(SmartList::Assigned), true),
        smart_entry(View::Smart(SmartList::Flagged), "Flagged email", "flag", count(SmartList::Flagged), true),
    ]);
    if let Some(default) = lists.iter().find(|l| l.is_default) {
        smart.push(Entry { list: None, ..smart_entry(View::List(default.id), "Tasks", "home", count(SmartList::Tasks), false) });
    }
    smart.retain(|e| prefs.smart_visible(e.view) && !(prefs.auto_hide_empty && e.hideable && e.count == 0 && e.view != View::Completed));

    let top: Vec<Entry> = lists
        .iter()
        .filter(|l| !l.is_default && l.group.as_deref().is_none_or(|g| !groups.iter().any(|x| x == g)))
        .map(|l| list_entry(l, list_counts.get(&l.id).copied().unwrap_or(0), false))
        .collect();
    let grouped: Vec<(String, Vec<Entry>)> = groups
        .iter()
        .map(|g| {
            let members = lists
                .iter()
                .filter(|l| !l.is_default && l.group.as_deref() == Some(g))
                .map(|l| list_entry(l, list_counts.get(&l.id).copied().unwrap_or(0), true))
                .collect();
            (g.clone(), members)
        })
        .collect();
    let collapsed = prefs.collapsed_groups.clone();
    // Collapsed to the icon rail (3.2), except while a name is being edited.
    let compact = !prefs.sidebar_open && ctx.renaming_list.read().is_none() && ctx.renaming_group.read().is_none();
    if compact {
        let lists: Vec<Entry> = top.into_iter().chain(grouped.into_iter().flat_map(|(_, m)| m)).map(|e| Entry { indent: false, ..e }).collect();
        return rsx! {
            nav { class: "ax-pane ax-todo-sidebar ax-todo-rail", "aria-label": "To Do lists",
                div { class: "ax-todo-nav", onkeydown: move |e: KeyboardEvent| nav_arrows(&e),
                    for e in smart {
                        NavItem { key: "{e.view.key()}", entry: e, compact: true }
                    }
                    div { class: "ax-todo-nav-divider", role: "separator" }
                    for e in lists {
                        NavItem { key: "{e.view.key()}", entry: e, compact: true }
                    }
                }
                div { class: "ax-todo-nav-footer",
                    IconButton { icon: "add", title: "New list (Ctrl+L)", onclick: move |_| new_list(app, ctx, None) }
                }
            }
        };
    }

    rsx! {
        nav {
            class: "ax-pane ax-todo-sidebar",
            "aria-label": "To Do lists",
            Splitter { edge: PaneEdge::Sidebar }
            div {
                class: "ax-todo-nav",
                onkeydown: move |e: KeyboardEvent| nav_arrows(&e),
                for e in smart {
                    NavItem { key: "{e.view.key()}", entry: e }
                }
                div { class: "ax-todo-nav-divider", role: "separator" }
                for e in top {
                    NavItem { key: "{e.view.key()}", entry: e }
                }
                for (g, members) in grouped {
                    GroupItem { key: "group-{g}", name: g.clone(), collapsed: collapsed.contains(&g), members }
                }
                ListEndDrop {}
            }
            div { class: "ax-todo-nav-footer",
                button {
                    r#type: "button",
                    class: "ax-todo-newlist",
                    title: "New list (Ctrl+L)",
                    "aria-keyshortcuts": "Control+L",
                    onclick: move |_| new_list(app, ctx, None),
                    Icon { name: "add" }
                    span { "New list" }
                }
                IconButton { icon: "tab_add", title: "Create group", onclick: move |_| new_group(app, ctx) }
            }
        }
    }
}

/// Up / Down / Home / End between sidebar rows (roving focus).
fn nav_arrows(e: &KeyboardEvent) {
    let dir = match e.key() {
        Key::ArrowDown => "1",
        Key::ArrowUp => "-1",
        Key::Home => "first",
        Key::End => "last",
        _ => return,
    };
    e.prevent_default();
    document::eval(&format!(
        "const items = [...document.querySelectorAll('.ax-todo-nav .ax-todo-nav-item')];
         const i = items.indexOf(document.activeElement);
         const d = '{dir}';
         const n = d === 'first' ? 0 : d === 'last' ? items.length - 1 : Math.max(0, Math.min(items.length - 1, i + Number(d)));
         items[n]?.focus();"
    ));
}

/// Drop zone under the last list: dropping a list here moves it to the end
/// of the top level.
#[component]
fn ListEndDrop() -> Element {
    let app = use_app();
    let mut ctx = use_todo();
    let dragging_list = matches!(*ctx.drag.read(), Some(Drag::List(_)));
    let over = ctx.drop_target.read().as_deref() == Some("lists-end");
    rsx! {
        div {
            class: "ax-todo-nav-end",
            class: if dragging_list { "ax-todo-nav-end-active" },
            class: if over { "ax-todo-drop-line" },
            ondragover: move |_| if dragging_list && !over { ctx.drop_target.set(Some("lists-end".into())) },
            ondragleave: move |_| if over { ctx.drop_target.set(None) },
            ondrop: move |_| {
                if let Some(Drag::List(id)) = ctx.drag.peek().clone()
                    && app.check(store().move_task_list(id, None, None)).is_some() {
                        app.bump_tasks();
                    }
                ctx.drag.set(None);
                ctx.drop_target.set(None);
            },
        }
    }
}

/// What dropping the current drag on `view` would do, if anything.
fn drop_allowed(drag: &Option<Drag>, view: View) -> bool {
    match drag {
        Some(Drag::Tasks(_)) => matches!(view, View::Smart(SmartList::MyDay | SmartList::Important | SmartList::Planned) | View::List(_)),
        Some(Drag::List(src)) => matches!(view, View::List(id) if id != *src),
        None => false,
    }
}

fn drop_on(app: AppState, ctx: TodoCtx, view: View, target_list: Option<&TaskList>) {
    let drag = ctx.drag.peek().clone();
    match (drag, view) {
        (Some(Drag::Tasks(ids)), View::Smart(SmartList::MyDay)) => actions::set_my_day(app, ctx, &ids, true),
        (Some(Drag::Tasks(ids)), View::Smart(SmartList::Important)) => actions::set_important(app, &ids, true),
        (Some(Drag::Tasks(ids)), View::Smart(SmartList::Planned)) => actions::set_due(app, &ids, Some(*ctx.today.peek())),
        (Some(Drag::Tasks(ids)), View::List(list)) => actions::move_to(app, ctx, &ids, list),
        (Some(Drag::List(src)), View::List(dst)) if src != dst => {
            let group = target_list.and_then(|l| l.group.clone());
            let before = target_list.filter(|l| !l.is_default).map(|l| l.id);
            if app.check(store().move_task_list(src, before, group.as_deref())).is_some() {
                app.bump_tasks();
            }
        }
        _ => {}
    }
}

#[component]
fn NavItem(entry: Entry, #[props(default)] compact: bool) -> Element {
    let app = use_app();
    let mut ctx = use_todo();
    let mut menu = use_signal(|| None::<(f64, f64)>);
    let mut button = use_signal(|| None::<std::rc::Rc<MountedData>>);
    let view = entry.view;
    let selected = (ctx.view)() == view;
    let list = entry.list.clone();
    let list_id = list.as_ref().map(|l| l.id);
    let renaming = list_id.is_some() && *ctx.renaming_list.read() == list_id;
    let drag = ctx.drag.read().clone();
    let droppable = drop_allowed(&drag, view);
    let over = droppable && ctx.drop_target.read().as_deref() == Some(view.key().as_str());
    let icon = if selected { filled(entry.icon) } else { entry.icon.to_string() };
    let has_menu = !menu_is_empty(&entry);
    let groups = if menu().is_some() { store().task_groups().unwrap_or_default() } else { Vec::new() };
    let menu_items = context_items(&entry, &groups);
    let label = entry.label.clone();
    let emoji = entry.emoji.clone();
    let count = entry.count;
    let aria = if count > 0 { format!("{label}, {count} tasks") } else { label.clone() };
    let tip = if compact && count > 0 && view != View::Completed { format!("{label} ({})", count_label(count)) } else { label.clone() };
    let renamable = list.as_ref().is_some_and(|l| !l.is_default);
    let current_label = list.as_ref().map(list_label).unwrap_or_default();
    rsx! {
        div {
            class: "ax-todo-nav-row",
            class: if entry.indent { "ax-todo-nav-indent" },
            "data-ctx": "1",
            draggable: if renamable && !renaming { "true" } else { "false" },
            ondragstart: move |_| if let Some(id) = list_id { ctx.drag.set(Some(Drag::List(id))) },
            ondragend: move |_| {
                ctx.drag.set(None);
                ctx.drop_target.set(None);
            },
            ondragover: move |_| if droppable && !over { ctx.drop_target.set(Some(view.key())) },
            ondragleave: move |_| if over { ctx.drop_target.set(None) },
            ondrop: {
                let list = list.clone();
                move |_| {
                    if droppable {
                        drop_on(app, ctx, view, list.as_ref());
                    }
                    ctx.drag.set(None);
                    ctx.drop_target.set(None);
                }
            },
            if renaming {
                div { class: "ax-todo-nav-item ax-selected",
                    if let Some(e) = &emoji { span { class: "ax-todo-emoji", "{e}" } } else { Icon { name: icon.clone() } }
                    RenameInput {
                        initial: current_label.clone(),
                        label: "List name",
                        on_commit: move |name: String| {
                            if let Some(id) = list_id
                                && name.trim() != current_label && !name.trim().is_empty() && app.check(store().rename_task_list(id, &name)).is_some() {
                                    app.bump_tasks();
                                }
                            ctx.renaming_list.set(None);
                        },
                        on_cancel: move |_| ctx.renaming_list.set(None),
                    }
                }
            } else {
                button {
                    r#type: "button",
                    class: "ax-todo-nav-item",
                    class: if selected { "ax-selected" },
                    class: if over { "ax-todo-drop" },
                    tabindex: if selected { "0" } else { "-1" },
                    "aria-current": if selected { "page" } else { "false" },
                    "aria-label": "{aria}",
                    title: "{tip}",
                    onmounted: move |e| button.set(Some(e.data())),
                    onclick: move |_| ctx.go(view),
                    oncontextmenu: move |e: MouseEvent| {
                        e.prevent_default();
                        if has_menu {
                            let c = e.client_coordinates();
                            menu.set(Some((c.x, c.y)));
                        }
                    },
                    onkeydown: move |e: KeyboardEvent| {
                        if e.key() == Key::F2 && renamable {
                            e.prevent_default();
                            e.stop_propagation();
                            ctx.renaming_list.set(list_id);
                        }
                    },
                    if let Some(e) = &emoji {
                        span { class: "ax-todo-emoji", "aria-hidden": "true", "{e}" }
                    } else {
                        Icon { name: icon.clone() }
                    }
                    if !compact {
                        span { class: "ax-todo-nav-label", "{label}" }
                        if count > 0 && view != View::Completed {
                            span { class: "ax-todo-nav-count", "aria-hidden": "true", "{count_label(count)}" }
                        }
                    }
                }
            }
            if let Some((x, y)) = menu() {
                Menu {
                    items: menu_items.clone(),
                    x,
                    y,
                    on_select: move |id: String| run_list_command(app, ctx, view, &id),
                    on_close: move |_| {
                        menu.set(None);
                        refocus(ctx, button);
                    },
                }
            }
        }
    }
}

/// Give the focus back to a sidebar row after its menu closes, unless the
/// command started a rename or opened a dialog.
fn refocus(ctx: TodoCtx, button: Signal<Option<std::rc::Rc<MountedData>>>) {
    spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
        if !ctx.overlay_open()
            && let Some(m) = button.peek().clone()
        {
            let _ = m.set_focus(true).await;
        }
    });
}

/// Filled variant of a sidebar icon (selected rows).
fn filled(icon: &str) -> String {
    match icon {
        "checkmark_circle" | "weather_sunny" | "star" | "calendar_ltr" | "person" | "flag" | "home" | "text_bullet_list_ltr" | "apps_list" => {
            format!("{icon}_filled")
        }
        _ => icon.to_string(),
    }
}

fn menu_is_empty(e: &Entry) -> bool {
    e.list.is_none() && !e.hideable
}

/// Context menu items (3.3.4).
fn context_items(e: &Entry, groups: &[String]) -> Vec<MenuItem> {
    if let Some(l) = &e.list {
        let mut move_items: Vec<MenuItem> = groups
            .iter()
            .filter(|g| l.group.as_deref() != Some(g.as_str()))
            .map(|g| MenuItem::new(format!("group:{g}"), g.clone()).icon("folder"))
            .collect();
        if !move_items.is_empty() {
            move_items.push(MenuItem::divider());
        }
        move_items.push(MenuItem::new("group_new", "New group").icon("tab_add"));
        if l.group.is_some() {
            move_items.push(MenuItem::new("group_none", "Remove from group").icon("group_dismiss"));
        }
        return vec![
            MenuItem::new("rename", "Rename list").icon("rename").shortcut("F2"),
            // Sharing is out of scope for now (people-todo.md 3.4): shown disabled, with the reason.
            MenuItem::new("share", "Share list").icon("share").disabled(true).tooltip("Sharing lists isn't available in Axentra yet"),
            MenuItem::new("move", "Move list to...").icon("folder_arrow_right").submenu(move_items),
            MenuItem::new("duplicate", "Duplicate list").icon("copy"),
            MenuItem::new("print", "Print list").icon("print"),
            MenuItem::divider(),
            MenuItem::new("delete", "Delete list").icon("delete").danger(),
        ];
    }
    if e.hideable {
        return vec![MenuItem::new("hide", "Hide from sidebar").icon("eye_off")];
    }
    Vec::new()
}

/// Run a sidebar context-menu command for `view`.
fn run_list_command(app: AppState, mut ctx: TodoCtx, view: View, id: &str) {
    let s = store();
    let View::List(list_id) = view else {
        if id == "hide" {
            ctx.update_prefs(|p| match view {
                View::Smart(SmartList::Important) => p.show_important = false,
                View::Smart(SmartList::Planned) => p.show_planned = false,
                View::Smart(SmartList::Assigned) => p.show_assigned = false,
                View::Smart(SmartList::Flagged) => p.flagged_email = false,
                View::All => p.show_all = false,
                View::Completed => p.show_completed = false,
                _ => {}
            });
            if (ctx.view)() == view {
                ctx.go(View::Smart(SmartList::MyDay));
            }
        }
        return;
    };
    match id.split_once(':').unwrap_or((id, "")) {
        ("rename", _) => ctx.renaming_list.set(Some(list_id)),
        ("group", g) => {
            if app.check(s.set_task_list_group(list_id, Some(g))).is_some() {
                app.bump_tasks();
            }
        }
        ("group_new", _) => {
            if let Some(g) = app.check(s.create_task_group("")) {
                app.check(s.set_task_list_group(list_id, Some(&g)));
                ctx.renaming_group.set(Some(g));
                app.bump_tasks();
            }
        }
        ("group_none", _) => {
            if app.check(s.set_task_list_group(list_id, None)).is_some() {
                app.bump_tasks();
            }
        }
        ("duplicate", _) => {
            if let Some(copy) = app.check(s.duplicate_task_list(list_id)) {
                app.bump_tasks();
                ctx.go(View::List(copy));
            }
        }
        ("print", _) => {
            ctx.go(view);
            super::print(ctx);
        }
        ("delete", _) => ctx.confirm.set(Some(Confirm::DeleteList(list_id))),
        _ => {}
    }
}

#[component]
fn GroupItem(name: String, collapsed: bool, members: Vec<Entry>) -> Element {
    let app = use_app();
    let mut ctx = use_todo();
    let mut menu = use_signal(|| None::<(f64, f64)>);
    let mut button = use_signal(|| None::<std::rc::Rc<MountedData>>);
    let renaming = ctx.renaming_group.read().as_deref() == Some(name.as_str());
    let key = format!("group:{name}");
    let dragging_list = matches!(*ctx.drag.read(), Some(Drag::List(_)));
    let over = dragging_list && ctx.drop_target.read().as_deref() == Some(key.as_str());
    let n = name.clone();
    let toggle = move |_| {
        let n = n.clone();
        ctx.update_prefs(move |p| {
            if let Some(i) = p.collapsed_groups.iter().position(|g| *g == n) {
                p.collapsed_groups.remove(i);
            } else {
                p.collapsed_groups.push(n);
            }
        });
    };
    rsx! {
        div {
            class: "ax-todo-nav-row",
            "data-ctx": "1",
            ondragover: {
                let key = key.clone();
                move |_| if dragging_list && !over { ctx.drop_target.set(Some(key.clone())) }
            },
            ondragleave: move |_| if over { ctx.drop_target.set(None) },
            ondrop: {
                let name = name.clone();
                move |_| {
                    if let Some(Drag::List(id)) = ctx.drag.peek().clone()
                        && app.check(store().move_task_list(id, None, Some(&name))).is_some() {
                            app.bump_tasks();
                        }
                    ctx.drag.set(None);
                    ctx.drop_target.set(None);
                }
            },
            if renaming {
                div { class: "ax-todo-nav-item ax-todo-group",
                    Icon { name: "chevron_down", size: 12, class: "ax-todo-chevron" }
                    RenameInput {
                        initial: name.clone(),
                        label: "Group name",
                        on_commit: {
                            let old = name.clone();
                            move |new: String| {
                                if let Some(actual) = app.check(store().rename_task_group(&old, &new)) {
                                    if actual != old {
                                        let (o, a) = (old.clone(), actual.clone());
                                        ctx.update_prefs(move |p| {
                                            for g in &mut p.collapsed_groups {
                                                if *g == o {
                                                    *g = a.clone();
                                                }
                                            }
                                        });
                                    }
                                    app.bump_tasks();
                                }
                                ctx.renaming_group.set(None);
                            }
                        },
                        on_cancel: move |_| ctx.renaming_group.set(None),
                    }
                }
            } else {
                button {
                    r#type: "button",
                    class: "ax-todo-nav-item ax-todo-group",
                    class: if over { "ax-todo-drop" },
                    tabindex: "-1",
                    "aria-expanded": if collapsed { "false" } else { "true" },
                    title: "{name}",
                    onmounted: move |e| button.set(Some(e.data())),
                    onclick: toggle,
                    oncontextmenu: move |e: MouseEvent| {
                        e.prevent_default();
                        let c = e.client_coordinates();
                        menu.set(Some((c.x, c.y)));
                    },
                    onkeydown: {
                        let name = name.clone();
                        move |e: KeyboardEvent| {
                            if e.key() == Key::F2 {
                                e.prevent_default();
                                e.stop_propagation();
                                ctx.renaming_group.set(Some(name.clone()));
                            }
                        }
                    },
                    Icon { name: "chevron_down", size: 12, class: if collapsed { "ax-todo-chevron ax-collapsed" } else { "ax-todo-chevron" } }
                    span { class: "ax-todo-nav-label", "{name}" }
                }
            }
            if let Some((x, y)) = menu() {
                Menu {
                    items: vec![
                        MenuItem::new("rename_group", "Rename group").icon("rename").shortcut("F2"),
                        MenuItem::new("new_list", "New list in group").icon("add"),
                        MenuItem::new("ungroup", "Ungroup lists").icon("group_dismiss"),
                    ],
                    x,
                    y,
                    on_select: {
                        let name = name.clone();
                        move |id: String| match id.as_str() {
                            "rename_group" => ctx.renaming_group.set(Some(name.clone())),
                            "new_list" => new_list(app, ctx, Some(&name)),
                            "ungroup"
                                if app.check(store().ungroup_task_group(&name)).is_some() => {
                                    app.bump_tasks();
                                }
                            _ => {}
                        }
                    },
                    on_close: move |_| {
                        menu.set(None);
                        refocus(ctx, button);
                    },
                }
            }
        }
        if !collapsed {
            for e in members {
                NavItem { key: "{e.view.key()}", entry: e }
            }
        }
    }
}

/// Inline rename field: Enter or blur commits, Esc cancels (keeps the name).
#[component]
pub fn RenameInput(#[props(into)] initial: String, #[props(into)] label: String, on_commit: EventHandler<String>, on_cancel: EventHandler<()>) -> Element {
    let mut value = use_signal(|| initial.clone());
    let mut done = use_signal(|| false);
    rsx! {
        input {
            class: "ax-todo-rename",
            value: "{value}",
            "aria-label": "{label}",
            maxlength: "255",
            onmounted: move |e| async move {
                let _ = e.set_focus(true).await;
                document::eval("document.activeElement?.select?.()");
            },
            oninput: move |e| value.set(e.value()),
            onclick: move |e| e.stop_propagation(),
            onkeydown: move |e: KeyboardEvent| {
                keys::stop_typing_keys(&e);
                match e.key() {
                    Key::Enter => {
                        e.prevent_default();
                        done.set(true);
                        on_commit.call(value());
                    }
                    Key::Escape => {
                        e.stop_propagation();
                        done.set(true);
                        on_cancel.call(());
                    }
                    _ => {}
                }
            },
            onblur: move |_| if !done() {
                done.set(true);
                on_commit.call(value());
            },
        }
    }
}
