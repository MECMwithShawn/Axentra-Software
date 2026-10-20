//! People module (docs/spec/people-todo.md section 2): navigation pane,
//! contact list, details pane, ribbon, dialogs and keyboard.
//!
//! Module-local state lives in [`ctx::PeopleCtx`], provided here. The list
//! contents are one memo (`ctx.model`) rebuilt when people data, the node,
//! the sort or the search text change.

mod actions;
mod avatar;
mod compose_table;
mod ctx;
mod details;
mod dialogs;
mod form;
mod io;
mod list;
mod list_editor;
mod nav;
mod photo;
mod ribbon;
mod text;
mod view;

use std::rc::Rc;

use chrono::{Duration, Local, Utc};
use dioxus::prelude::*;

use actions::Selection;
use axentra_core::EmailAddress;
use ctx::{LIST_W, MenuTarget, NAV_W, PeopleCtx, PeopleDialog, Prefs, SUGGESTED_DAYS};
use view::{BuildInput, Item, Node};

use crate::kit::{Menu, keys};
use crate::shell::ribbon::Ribbon;
use crate::state::{NavRequest, store, use_app};

/// Window width below which the navigation pane hides (2.2); the matching
/// media queries are in people.css.
const HIDE_NAV_BELOW: u32 = 720;
/// Two splitter clicks within this many ms reset the pane width.
const DOUBLE_CLICK_MS: u64 = 450;

#[component]
pub fn PeopleModule() -> Element {
    let app = use_app();
    let prefs = use_hook(|| std::rc::Rc::new(Prefs::load()));
    let node = use_signal(|| prefs.node.clone());
    let sort = use_signal(|| prefs.sort);
    let last_first = use_signal(|| prefs.last_first);
    let nav_open = use_signal(|| prefs.nav_open);
    let suggested_on = use_signal(|| prefs.suggested_on);
    let tip_dismissed = use_signal(|| prefs.tip_dismissed);
    let nav_w = use_signal(|| prefs.nav_w);
    let list_w = use_signal(|| prefs.list_w);
    let filter = use_signal(String::new);

    let model = use_memo(move || {
        let _ = app.people_rev();
        let node = node();
        let local = filter();
        let query = ctx::effective_query(&local, &app.search.read());
        let s = store();
        let contacts = s.contacts().unwrap_or_default();
        let lists = s.contact_lists().unwrap_or_default();
        let suggested = if node == Node::Suggested {
            let _ = app.mail_rev();
            s.suggested_people(Utc::now() - Duration::days(SUGGESTED_DAYS), 200).unwrap_or_default()
        } else {
            Vec::new()
        };
        Rc::new(view::build(BuildInput {
            node: &node,
            contacts: &contacts,
            lists: &lists,
            suggested: &suggested,
            query: &query,
            sort: sort(),
            today: Local::now().date_naive(),
        }))
    });

    let selection = use_signal(Vec::new);
    // What the selection holds, re-read from the store only when the
    // selection or people data change (not on every render).
    let selected = use_memo(move || {
        let _ = app.people_rev();
        Rc::new(Selection::from_items(&selection.read()))
    });

    let mut ctx = use_context_provider(|| PeopleCtx {
        node,
        sort,
        last_first,
        nav_open,
        contacts_expanded: Signal::new(true),
        suggested_on,
        tip_dismissed,
        filter,
        selection,
        selected,
        anchor: Signal::new(None),
        focus: Signal::new(None),
        focus_req: Signal::new((0, false)),
        editing: Signal::new(None),
        edit_dirty: Signal::new(false),
        dialog: Signal::new(None),
        menu: Signal::new(None),
        nav_w,
        list_w,
        new_folder: Signal::new(false),
        renaming: Signal::new(None),
        drag: Signal::new(Vec::new()),
        model,
    });

    // Requests from other modules: open a contact (NavRequest::Contact) or
    // a pre-filled New contact form ("Add to contacts", NavRequest::NewContact).
    use_effect(move || match (app.nav)() {
        Some(NavRequest::Contact(id)) => {
            let mut nav = app.nav;
            nav.set(None);
            ctx.editing.set(None);
            details::open_item(ctx, Item::Contact(id));
        }
        Some(NavRequest::NewContact { name, email }) => {
            let mut nav = app.nav;
            nav.set(None);
            ctx.editing.set(None);
            let existing = (!email.trim().is_empty()).then(|| store().contact_by_email(email.trim()).ok().flatten()).flatten();
            match existing {
                // Already saved: show it instead of a duplicate form.
                Some(c) => details::open_item(ctx, Item::Contact(c.id)),
                None => {
                    let prefill = actions::prefill_from_address(&EmailAddress::new(name, email.trim()));
                    ctx.open_dialog(PeopleDialog::NewContact(Box::new(prefill)));
                }
            }
        }
        _ => {}
    });

    // Below 720 px the navigation pane is hidden (CSS) and the hamburger
    // shows it as an overlay instead of toggling the saved preference.
    let mut nav_peek = use_signal(|| false);
    use_effect(move || {
        let _ = (ctx.node)();
        nav_peek.set(false);
    });
    let _ = app.people_rev();
    let sel = ctx.selected.read().clone();
    let has_contacts = store().people_count().unwrap_or(0) > 0;
    let density = app.settings.read().density;
    let tabs = ribbon::tabs(&ctx, &sel, has_contacts, density);
    let nav_open = *ctx.nav_open.read();
    let has_selection = !ctx.selection.read().is_empty();

    let on_keys = move |e: KeyboardEvent| {
        if ctx.dialog.peek().is_some() || ctx.menu.peek().is_some() {
            return;
        }
        let chord = keys::chord(&e);
        let region = match chord.as_str() {
            "F6" | "Ctrl+F6" => Some(1),
            "Shift+F6" | "Ctrl+Shift+F6" => Some(-1),
            _ => None,
        };
        if let Some(dir) = region {
            e.prevent_default();
            e.stop_propagation();
            cycle_regions(dir);
            return;
        }
        if ctx.editing.peek().is_some() {
            return;
        }
        match chord.as_str() {
            "Ctrl+N" => actions::new_contact(ctx),
            "Ctrl+L" => actions::new_list(ctx, Vec::new()),
            "Shift+E" => actions::edit(ctx),
            "Delete" | "Ctrl+D" => actions::ask_delete(ctx),
            "Ctrl+E" => io::focus_id(list::SEARCH_ID),
            // `?` is Shift+/ on a US layout; some event sources report the
            // unshifted key.
            "Shift+?" | "?" | "Shift+/" => ctx.open_dialog(ctx::PeopleDialog::Shortcuts),
            "Ctrl+A" => {
                let items: Vec<Item> = ctx.model.peek().items().collect();
                ctx.selection.set(items);
            }
            "Escape" if !ctx.selection.peek().is_empty() => ctx.clear_selection(),
            // "Clear selection and refresh" (keyboard spec 7).
            "Shift+F5" => {
                ctx.clear_selection();
                app.bump_people();
            }
            _ => return,
        }
        e.prevent_default();
        e.stop_propagation();
    };

    rsx! {
        div {
            class: "ax-people",
            tabindex: "-1",
            onkeydown: on_keys,
            onmounted: move |_| {
                // Browser defaults for these chords must be cancelled in the
                // page, because liveview handles keys asynchronously.
                document::eval(
                    r#"const root = document.querySelector('.ax-people');
                    if (root && !root.dataset.keysReady) { root.dataset.keysReady = '1';
                      root.addEventListener('keydown', e => {
                        const k = (e.key || '').toLowerCase();
                        const editable = e.target.matches('input, textarea, select, [contenteditable="true"]');
                        if ((e.ctrlKey || e.metaKey) && !e.altKey && ['n', 'l', 'e', 'd', 's'].includes(k)) e.preventDefault();
                        if ((e.ctrlKey || e.metaKey) && k === 'a' && !editable) e.preventDefault();
                        if (e.key === 'F6') e.preventDefault();
                      }); }"#,
                );
                document::eval(io::TRACK_FOCUS_JS);
            },
            Ribbon {
                tabs,
                on_command: move |id: String| actions::run(ctx, app, &id, &MenuTarget::Selection),
                on_toggle_nav: move |_| {
                    spawn(async move {
                        let narrow = document::eval(&format!("return window.matchMedia('(max-width: {}px)').matches;", HIDE_NAV_BELOW - 1))
                            .join::<bool>()
                            .await
                            .unwrap_or(false);
                        if narrow { nav_peek.set(!nav_peek()) } else { ctx.toggle_nav() }
                    });
                },
                nav_open,
            }
            // Narrow windows are handled in CSS (2.2): under 1024 px the details
            // replace the list while something is selected; under 720 px the
            // navigation pane hides.
            div {
                class: "ax-panes ax-people-panes",
                class: if has_selection { "ax-has-selection" },
                class: if nav_peek() { "ax-nav-peek" },
                if nav_open || nav_peek() {
                    nav::NavPane {}
                    Splitter { nav: true }
                }
                list::ListPane {}
                Splitter { nav: false }
                details::DetailsPane {}
            }
            dialogs::PeopleDialogs {}
            if let Some(menu) = ctx.menu.read().clone() {
                Menu {
                    items: menu.items.clone(),
                    x: menu.x,
                    y: menu.y,
                    on_select: move |id: String| actions::run(ctx, app, &id, &menu.target),
                    on_close: move |_| {
                        ctx.menu.set(None);
                        io::restore_focus();
                    },
                }
            }
        }
    }
}

/// Move focus to the next (`dir` = 1) or previous region: ribbon, navigation
/// pane, contact list, details (2.2, keyboard spec 12.2).
fn cycle_regions(dir: i32) {
    document::eval(&format!(
        r#"const regions = ['.ax-people .ax-ribbon', '#ax-people-nav', '#ax-people-listpane', '#ax-people-details'];
        const targets = [
          '.ax-people .ax-ribbon-commands button:not([disabled])',
          '#ax-people-nav [tabindex="0"]',
          '#ax-people-scroll [tabindex="0"], #ax-people-search',
          '#ax-people-details button:not([disabled]), #ax-people-details input, #ax-people-details a'
        ];
        const active = document.activeElement;
        let cur = regions.findIndex(sel => {{ const r = document.querySelector(sel); return r && r.contains(active); }});
        const n = regions.length;
        for (let k = 1; k <= n; k++) {{
          const i = (((cur < 0 ? 0 : cur) + {dir} * k) % n + n) % n;
          const el = document.querySelector(targets[i]);
          if (el) {{ el.focus(); break; }}
        }}"#
    ));
}

/// Drag handle between panes; also resizable with the arrow keys
/// (16 px steps); double-click restores the default width.
#[component]
fn Splitter(nav: bool) -> Element {
    let ctx = use_context::<PeopleCtx>();
    let mut drag = use_signal(|| None::<(f64, u32)>);
    // The drag overlay swallows the second click, so double-clicks are
    // recognised from two quick clicks that didn't move.
    let mut last_click = use_signal(|| None::<std::time::Instant>);
    let (default, min, max) = if nav { NAV_W } else { LIST_W };
    let current = if nav { *ctx.nav_w.read() } else { *ctx.list_w.read() };
    let set_live = move |w: u32| {
        let w = w.clamp(min, max);
        if nav {
            let mut s = ctx.nav_w;
            s.set(w);
        } else {
            let mut s = ctx.list_w;
            s.set(w);
        }
    };
    let commit = move |w: u32| {
        if nav { ctx.set_nav_w(w) } else { ctx.set_list_w(w) }
    };
    let label = if nav { "Resize navigation pane" } else { "Resize contact list" };
    rsx! {
        div {
            class: "ax-people-splitter",
            class: if nav { "ax-people-splitter-nav" } else { "ax-people-splitter-list" },
            class: if drag().is_some() { "ax-dragging" },
            role: "separator",
            "aria-orientation": "vertical",
            "aria-label": "{label}",
            "aria-valuenow": "{current}",
            "aria-valuemin": "{min}",
            "aria-valuemax": "{max}",
            tabindex: "0",
            title: "{label}",
            onmousedown: move |e: MouseEvent| {
                e.prevent_default();
                drag.set(Some((e.client_coordinates().x, current)));
            },
            ondoubleclick: move |_| commit(default),
            onkeydown: move |e: KeyboardEvent| match e.key() {
                Key::ArrowLeft => {
                    e.stop_propagation();
                    commit(current.saturating_sub(16));
                }
                Key::ArrowRight => {
                    e.stop_propagation();
                    commit(current + 16);
                }
                _ => {}
            },
        }
        if let Some((start_x, start_w)) = drag() {
            div {
                class: "ax-people-drag-overlay",
                onmousemove: move |e: MouseEvent| {
                    let dx = e.client_coordinates().x - start_x;
                    set_live((start_w as f64 + dx).round().max(0.0) as u32);
                },
                onmouseup: move |e: MouseEvent| {
                    let dx = e.client_coordinates().x - start_x;
                    drag.set(None);
                    if dx.abs() < 2.0 {
                        let double = last_click().is_some_and(|t| t.elapsed() < std::time::Duration::from_millis(DOUBLE_CLICK_MS));
                        last_click.set(if double { None } else { Some(std::time::Instant::now()) });
                        commit(if double { default } else { start_w });
                    } else {
                        last_click.set(None);
                        commit(((start_w as f64 + dx).round().max(0.0) as u32).clamp(min, max));
                    }
                },
            }
        }
    }
}
