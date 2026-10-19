//! Header search box (docs/spec/shell.md §3.2, keyboard spec §9).
//!
//! The box only edits `AppState::search`; each module runs the search and
//! renders results itself. Here: placeholder, focus growth, clear and exit
//! buttons, suggestions (recent searches, people, top items), the mail
//! filter panel, and the Esc / Enter / Up / Down behaviour.

use axentra_core::*;
use dioxus::prelude::*;

use crate::kit::{Button, Checkbox, Icon, TextField, Variant, dom, keys};
use crate::state::{AppState, Module, NavRequest, SearchState, store, use_app};

pub const INPUT_SELECTOR: &str = "#ax-search-input";
const RECENT_KEY: &str = "shell.recent_searches";
const MAX_RECENT: usize = 10;

/// Recent searches, newest first (kept in the store's key/value table).
pub fn recent_searches() -> Vec<String> {
    store().kv_get(RECENT_KEY).ok().flatten().and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default()
}

pub fn clear_recent_searches() -> axentra_core::Result<()> {
    store().kv_set(RECENT_KEY, "[]")
}

/// `list` with `q` moved to the front (no duplicates, at most 10).
pub fn push_recent(mut list: Vec<String>, q: &str) -> Vec<String> {
    let q = q.trim();
    if q.is_empty() {
        return list;
    }
    list.retain(|s| !s.eq_ignore_ascii_case(q));
    list.insert(0, q.to_string());
    list.truncate(MAX_RECENT);
    list
}

fn remember(q: &str) {
    let list = push_recent(recent_searches(), q);
    if let Ok(json) = serde_json::to_string(&list) {
        let _ = store().kv_set(RECENT_KEY, &json);
    }
}

fn placeholder(module: Module) -> &'static str {
    match module {
        Module::Mail => "Search",
        Module::Calendar => "Search calendar",
        Module::People => "Search people",
        Module::Tasks => "Search tasks",
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Suggestion {
    Recent(String),
    Person { name: String, address: String, contact: Option<ContactId> },
    Message { id: MessageId, subject: String, from: String },
    Event { id: EventId, title: String, when: String },
}

impl Suggestion {
    fn section(&self) -> &'static str {
        match self {
            Suggestion::Recent(_) => "Recent searches",
            Suggestion::Person { .. } => "People",
            Suggestion::Message { .. } => "Mail",
            Suggestion::Event { .. } => "Events",
        }
    }
}

fn suggestions(module: Module, query: &str, recent: &[String]) -> Vec<Suggestion> {
    let q = query.trim();
    if q.is_empty() {
        return recent.iter().take(5).cloned().map(Suggestion::Recent).collect();
    }
    let lower = q.to_lowercase();
    let s = store();
    let mut out: Vec<Suggestion> = recent
        .iter()
        .filter(|r| r.to_lowercase().starts_with(&lower) && !r.eq_ignore_ascii_case(q))
        .take(2)
        .cloned()
        .map(Suggestion::Recent)
        .collect();
    if q.contains(':') {
        return out;
    }
    for c in s.search_contacts(q).unwrap_or_default().into_iter().take(3) {
        let address = c.primary_email().unwrap_or_default().to_string();
        out.push(Suggestion::Person { name: c.display_name.clone(), address, contact: Some(c.id) });
    }
    match module {
        Module::Mail => {
            let query = MessageQuery { search: Some(format!("subject:\"{q}\"")), limit: Some(3), ..Default::default() };
            for m in s.messages(&query).unwrap_or_default() {
                out.push(Suggestion::Message { id: m.id, subject: m.subject.clone(), from: m.from.display().to_string() });
            }
        }
        Module::Calendar => {
            for e in s.search_events(q).unwrap_or_default().into_iter().take(3) {
                let when = crate::fmt::short_date(e.start.with_timezone(&chrono::Local).date_naive());
                out.push(Suggestion::Event { id: e.id, title: e.title.clone(), when });
            }
        }
        _ => {}
    }
    out
}

fn submit(mut app: AppState, query: String) {
    let q = query.trim().to_string();
    remember(&q);
    let mut s = app.search.write();
    s.query = query;
    s.active = !q.is_empty();
    s.serial += 1;
    if s.active {
        dom::announce(&format!("Searching for {q}"), false);
    }
}

fn exit_search(mut app: AppState) {
    let serial = app.search.read().serial + 1;
    app.search.set(SearchState { serial, ..Default::default() });
}

#[component]
pub fn SearchBox() -> Element {
    let mut app = use_app();
    let module = (app.module)();
    let search = app.search.read().clone();
    let mut focused = use_signal(|| false);
    let mut list_open = use_signal(|| false);
    let mut active = use_signal(|| None::<usize>);
    let mut filters_open = use_signal(|| false);
    let items = if list_open() && focused() { suggestions(module, &search.query, &recent_searches()) } else { Vec::new() };
    let n = items.len();
    let active_id = active().filter(|i| *i < n).map(|i| format!("ax-search-opt-{i}"));
    let show_list = !items.is_empty();
    let mut pick = move |item: Suggestion| {
        let mut app = app;
        list_open.set(false);
        match item {
            Suggestion::Recent(q) => submit(app, q),
            Suggestion::Person { name, address, contact } => match ((app.module)(), contact) {
                (Module::People, Some(id)) => app.navigate(NavRequest::Contact(id)),
                (Module::Mail, _) if !address.is_empty() => submit(app, format!("from:\"{address}\"")),
                _ => submit(app, name),
            },
            Suggestion::Message { id, .. } => {
                app.search.write().query.clear();
                app.navigate(NavRequest::Message(id));
            }
            Suggestion::Event { id, .. } => app.navigate(NavRequest::Event(id)),
        }
    };
    let expanded = focused() || filters_open();
    rsx! {
        div {
            class: "ax-header-search",
            class: if expanded { "ax-expanded" },
            class: if search.active { "ax-searching" },
            role: "search",
            onfocusin: move |_| {
                focused.set(true);
                list_open.set(true);
            },
            onfocusout: move |_| {
                // Let a click on a suggestion land before the list closes.
                spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(180)).await;
                    focused.set(false);
                });
            },
            div { class: "ax-search",
                if search.active {
                    button {
                        r#type: "button",
                        class: "ax-search-icon-btn",
                        title: "Exit search",
                        "aria-label": "Exit search",
                        onclick: move |_| exit_search(app),
                        Icon { name: "arrow_left", size: 16 }
                    }
                } else {
                    Icon { name: "search", size: 16, class: "ax-search-glyph" }
                }
                input {
                    id: "ax-search-input",
                    r#type: "text",
                    role: "combobox",
                    autocomplete: "off",
                    spellcheck: "false",
                    placeholder: placeholder(module),
                    "aria-label": "Search",
                    "aria-expanded": if show_list { "true" } else { "false" },
                    "aria-controls": "ax-search-suggestions",
                    "aria-autocomplete": "list",
                    "aria-activedescendant": active_id,
                    "data-ax-default-focus": "true",
                    value: "{search.query}",
                    oninput: move |e| {
                        app.search.write().query = e.value();
                        list_open.set(true);
                        active.set(None);
                    },
                    onkeydown: {
                        let items = items.clone();
                        move |e: KeyboardEvent| {
                            keys::stop_typing_keys(&e);
                            match e.key() {
                                Key::ArrowDown if n > 0 => {
                                    e.prevent_default();
                                    active.set(Some(active().map(|i| (i + 1) % n).unwrap_or(0)));
                                }
                                Key::ArrowUp if n > 0 => {
                                    e.prevent_default();
                                    active.set(Some(active().map(|i| (i + n - 1) % n).unwrap_or(n - 1)));
                                }
                                Key::Enter => {
                                    e.prevent_default();
                                    match active().and_then(|i| items.get(i).cloned()) {
                                        Some(item) => pick(item),
                                        None => {
                                            list_open.set(false);
                                            let q = app.search.read().query.clone();
                                            submit(app, q);
                                        }
                                    }
                                    active.set(None);
                                }
                                Key::Escape => {
                                    e.stop_propagation();
                                    if show_list || filters_open() {
                                        list_open.set(false);
                                        filters_open.set(false);
                                    } else if !app.search.read().query.is_empty() || app.search.read().active {
                                        exit_search(app);
                                    } else {
                                        dom::enter_region(".ax-module [data-ax-region], .ax-module .ax-panes > .ax-pane");
                                    }
                                }
                                _ => {}
                            }
                        }
                    },
                }
                if !search.query.is_empty() {
                    button {
                        r#type: "button",
                        class: "ax-search-icon-btn",
                        title: "Clear",
                        "aria-label": "Clear search",
                        onclick: move |_| {
                            exit_search(app);
                            dom::focus(INPUT_SELECTOR);
                        },
                        Icon { name: "dismiss", size: 16 }
                    }
                }
                if module == Module::Mail && (expanded || search.active) {
                    button {
                        r#type: "button",
                        class: "ax-search-icon-btn",
                        class: if filters_open() { "ax-toggled" },
                        title: "Filters",
                        "aria-label": "Search filters",
                        "aria-expanded": if filters_open() { "true" } else { "false" },
                        onclick: move |_| {
                            filters_open.set(!filters_open());
                            list_open.set(false);
                        },
                        Icon { name: "filter", size: 16 }
                    }
                }
            }
            if show_list && !filters_open() {
                div { id: "ax-search-suggestions", class: "ax-search-flyout", role: "listbox", "aria-label": "Search suggestions",
                    for (i, item) in items.iter().cloned().enumerate() {
                        SuggestionRow {
                            key: "{i}",
                            index: i,
                            heading: (i == 0 || items[i - 1].section() != item.section()).then(|| item.section().to_string()),
                            item: item.clone(),
                            active: active() == Some(i),
                            onpick: pick,
                        }
                    }
                }
            }
            if filters_open() {
                FilterPanel {
                    query: search.query.clone(),
                    on_close: move |_| filters_open.set(false),
                    on_search: move |q: String| {
                        filters_open.set(false);
                        submit(app, q);
                    },
                }
            }
        }
    }
}

#[component]
fn SuggestionRow(index: usize, heading: Option<String>, item: Suggestion, active: bool, onpick: EventHandler<Suggestion>) -> Element {
    let (icon, title, sub) = match &item {
        Suggestion::Recent(q) => ("clock", q.clone(), String::new()),
        Suggestion::Person { name, address, .. } => ("person", name.clone(), address.clone()),
        Suggestion::Message { subject, from, .. } => ("mail", subject.clone(), from.clone()),
        Suggestion::Event { title, when, .. } => ("calendar_ltr", title.clone(), when.clone()),
    };
    rsx! {
        if let Some(h) = heading {
            div { class: "ax-search-section", role: "presentation", "{h}" }
        }
        div {
            id: "ax-search-opt-{index}",
            class: "ax-search-option",
            class: if active { "ax-active" },
            role: "option",
            "aria-selected": if active { "true" } else { "false" },
            onmousedown: move |e| e.prevent_default(),
            onclick: {
                let item = item.clone();
                move |_| onpick.call(item.clone())
            },
            if let Suggestion::Person { name, address, .. } = &item {
                crate::kit::Avatar { name: name.clone(), email: address.clone(), size: 24 }
            } else {
                Icon { name: icon, size: 16 }
            }
            span { class: "ax-search-option-text",
                span { class: "ax-ellipsis", "{title}" }
                if !sub.is_empty() { span { class: "ax-ellipsis ax-muted ax-caption", "{sub}" } }
            }
        }
    }
}

/// Mail search filters: From, To, Subject, Keywords, Has attachments,
/// Unread. Builds the keyword query the mail store understands.
#[component]
fn FilterPanel(query: String, on_close: EventHandler<()>, on_search: EventHandler<String>) -> Element {
    let parsed = axentra_core::parse_search(&query);
    let field = |k: &str| parsed.iter().filter(|(key, _)| key.as_deref() == Some(k)).map(|(_, v)| v.clone()).collect::<Vec<_>>().join(" ");
    let mut from = use_signal(|| field("from"));
    let mut to = use_signal(|| field("to"));
    let mut subject = use_signal(|| field("subject"));
    let mut words = use_signal(|| parsed.iter().filter(|(k, _)| k.is_none()).map(|(_, v)| v.clone()).collect::<Vec<_>>().join(" "));
    let mut attachments = use_signal(|| parsed.iter().any(|(k, v)| k.as_deref() == Some("hasattachments") && v == "yes"));
    let mut unread = use_signal(|| parsed.iter().any(|(k, v)| k.as_deref() == Some("is") && v == "unread"));
    let build = move || {
        let quote = |v: String| if v.contains(' ') { format!("\"{v}\"") } else { v };
        let mut parts = Vec::new();
        for (key, sig) in [("from", from), ("to", to), ("subject", subject)] {
            let v = sig.read().trim().to_string();
            if !v.is_empty() {
                parts.push(format!("{key}:{}", quote(v)));
            }
        }
        let w = words.read().trim().to_string();
        if !w.is_empty() {
            parts.push(w);
        }
        if attachments() {
            parts.push("hasattachments:yes".into());
        }
        if unread() {
            parts.push("is:unread".into());
        }
        parts.join(" ")
    };
    rsx! {
        div {
            class: "ax-search-flyout ax-search-filters",
            role: "dialog",
            "aria-label": "Search filters",
            onkeydown: move |e: KeyboardEvent| {
                if e.key() == Key::Escape {
                    e.stop_propagation();
                    on_close.call(());
                    dom::focus(INPUT_SELECTOR);
                }
            },
            TextField { label: "From", value: from(), oninput: move |v| from.set(v), autofocus: true, onkeydown: move |e: KeyboardEvent| if e.key() == Key::Enter { on_search.call(build()) } }
            TextField { label: "To", value: to(), oninput: move |v| to.set(v), onkeydown: move |e: KeyboardEvent| if e.key() == Key::Enter { on_search.call(build()) } }
            TextField { label: "Subject", value: subject(), oninput: move |v| subject.set(v), onkeydown: move |e: KeyboardEvent| if e.key() == Key::Enter { on_search.call(build()) } }
            TextField {
                label: "Keywords",
                value: words(),
                placeholder: "Words found in the message",
                oninput: move |v| words.set(v),
                onkeydown: move |e: KeyboardEvent| if e.key() == Key::Enter { on_search.call(build()) },
            }
            div { class: "ax-row", style: "gap:24px",
                Checkbox { label: "Has attachments", checked: attachments(), onchange: move |v| attachments.set(v) }
                Checkbox { label: "Unread", checked: unread(), onchange: move |v| unread.set(v) }
            }
            div { class: "ax-row ax-search-filters-actions",
                Button {
                    label: "Clear filters",
                    variant: Variant::Subtle,
                    onclick: move |_| {
                        from.set(String::new());
                        to.set(String::new());
                        subject.set(String::new());
                        words.set(String::new());
                        attachments.set(false);
                        unread.set(false);
                    },
                }
                span { class: "ax-grow" }
                Button { label: "Search", variant: Variant::Primary, onclick: move |_| on_search.call(build()) }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_searches_dedupe_and_cap() {
        let mut list = Vec::new();
        for i in 0..12 {
            list = push_recent(list, &format!("q{i}"));
        }
        assert_eq!(list.len(), MAX_RECENT);
        assert_eq!(list[0], "q11");
        let list = push_recent(list, "Q5");
        assert_eq!(list[0], "Q5");
        assert_eq!(list.iter().filter(|s| s.eq_ignore_ascii_case("q5")).count(), 1);
        assert_eq!(push_recent(vec!["a".into()], "  "), vec!["a".to_string()]);
    }
}
