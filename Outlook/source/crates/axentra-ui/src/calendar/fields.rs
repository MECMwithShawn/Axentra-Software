//! Form fields shared by quick create and the full editor: a typable date
//! picker, a 30-minute time combobox (end list shows durations) and the
//! attendee picker with address-book suggestions.

use axentra_core::*;
use chrono::{Duration, Local, NaiveDate, NaiveTime, Timelike};
use dioxus::prelude::*;

use super::mini_month::MiniMonth;
use super::range::first_of_month;
use super::text::{clock, duration_short, parse_date, parse_time, short_date};
use crate::kit::{Avatar, Icon, Menu, MenuItem, keys};
use crate::state::store;

static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

#[component]
pub fn DateField(
    value: NaiveDate,
    onchange: EventHandler<NaiveDate>,
    #[props(into)] label: String,
    #[props(default)] disabled: bool,
    #[props(default = chrono::Weekday::Sun)] week_start: chrono::Weekday,
) -> Element {
    let mut text = use_signal(|| short_date(value));
    let mut open = use_signal(|| false);
    let mut error = use_signal(|| false);
    let mut month = use_signal(|| first_of_month(value));
    use_effect(use_reactive!(|value| {
        text.set(short_date(value));
        error.set(false);
    }));
    let today = Local::now().date_naive();
    let mut commit = move || {
        let typed = text.peek().clone();
        match parse_date(&typed, today) {
            Some(d) => {
                error.set(false);
                text.set(short_date(d));
                if d != value {
                    onchange.call(d);
                }
            }
            None => error.set(true),
        }
    };
    rsx! {
        div { class: "ax-cal-datefield",
            div { class: "ax-cal-field-box",
                class: if error() { "ax-invalid" },
                input {
                    class: "ax-cal-field-input",
                    value: "{text}",
                    disabled,
                    "aria-label": "{label}",
                    "aria-invalid": if error() { "true" } else { "false" },
                    oninput: move |e| text.set(e.value()),
                    onkeydown: move |e: KeyboardEvent| {
                        keys::stop_typing_keys(&e);
                        if e.key() == Key::Enter {
                            e.prevent_default();
                            commit();
                        }
                    },
                    onblur: move |_| commit(),
                }
                button {
                    r#type: "button",
                    class: "ax-cal-field-btn",
                    title: "Choose a date",
                    "aria-label": "Choose a date",
                    disabled,
                    onclick: move |_| {
                        month.set(first_of_month(value));
                        open.set(!open());
                    },
                    Icon { name: "calendar_ltr", size: 16 }
                }
            }
            if error() { span { class: "ax-cal-field-error", role: "alert", "Enter a valid date" } }
            if open() {
                div { class: "ax-menu-backdrop", onclick: move |_| open.set(false) }
                div {
                    class: "ax-cal-flyout ax-cal-field-flyout",
                    onmousedown: move |e| e.stop_propagation(),
                    onkeydown: move |e: KeyboardEvent| if e.key() == Key::Escape { e.stop_propagation(); open.set(false) },
                    MiniMonth {
                        month: month(),
                        on_month: move |m| month.set(m),
                        on_pick: move |d| {
                            open.set(false);
                            error.set(false);
                            text.set(short_date(d));
                            onchange.call(d);
                        },
                        today,
                        week_start,
                        selected: Some(value),
                    }
                }
            }
        }
    }
}

/// Time combobox: 30-minute steps over the day. With `from`, the list starts
/// there and each entry shows its duration ("10:30 AM (1 hr)").
#[component]
pub fn TimeField(
    value: NaiveTime,
    onchange: EventHandler<NaiveTime>,
    #[props(into)] label: String,
    h24: bool,
    from: Option<NaiveTime>,
    #[props(default)] disabled: bool,
) -> Element {
    let id = use_hook(next_id);
    let mut text = use_signal(|| clock(value, h24));
    let mut open = use_signal(|| false);
    let mut error = use_signal(|| false);
    let mut active = use_signal(|| None::<usize>);
    use_effect(use_reactive!(|value, h24| {
        text.set(clock(value, h24));
        error.set(false);
    }));
    let start_min = from.map(|f| (f.hour() * 60 + f.minute()) as i64).unwrap_or(0);
    let first = if from.is_some() { start_min } else { 0 };
    let options: Vec<(NaiveTime, String)> = (0..48)
        .map(|k| first + k * 30)
        .filter(|m| *m < 24 * 60)
        .map(|m| {
            let t = NaiveTime::from_hms_opt((m / 60) as u32, (m % 60) as u32, 0).expect("valid time");
            let label = match from {
                Some(_) => format!("{} ({})", clock(t, h24), duration_short(Duration::minutes(m - start_min))),
                None => clock(t, h24),
            };
            (t, label)
        })
        .collect();
    let n = options.len();
    let mut commit_text = move || {
        let typed = text.peek().clone();
        match parse_time(&typed) {
            Some(t) => {
                error.set(false);
                text.set(clock(t, h24));
                if t != value {
                    onchange.call(t);
                }
            }
            None => error.set(true),
        }
    };
    let scroll_to_selected = move || {
        document::eval(&format!(
            "setTimeout(()=>{{const el=document.querySelector('[data-tf=\"{id}\"] .ax-selected');if(el)el.scrollIntoView({{block:'center'}});}},0)"
        ));
    };
    let opts = options.clone();
    rsx! {
        div { class: "ax-cal-timefield", "data-tf": "{id}",
            div { class: "ax-cal-field-box",
                class: if error() { "ax-invalid" },
                input {
                    class: "ax-cal-field-input",
                    value: "{text}",
                    disabled,
                    role: "combobox",
                    "aria-label": "{label}",
                    "aria-expanded": if open() { "true" } else { "false" },
                    "aria-invalid": if error() { "true" } else { "false" },
                    oninput: move |e| {
                        text.set(e.value());
                        open.set(false);
                    },
                    onfocus: move |_| {},
                    onkeydown: move |e: KeyboardEvent| {
                        keys::stop_typing_keys(&e);
                        match e.key() {
                            Key::ArrowDown | Key::ArrowUp => {
                                e.prevent_default();
                                if !open() {
                                    open.set(true);
                                    scroll_to_selected();
                                    return;
                                }
                                let cur = active().or_else(|| opts.iter().position(|(t, _)| *t == value)).unwrap_or(0);
                                let next = if e.key() == Key::ArrowDown { (cur + 1).min(n.saturating_sub(1)) } else { cur.saturating_sub(1) };
                                active.set(Some(next));
                                if let Some((t, _)) = opts.get(next) {
                                    text.set(clock(*t, h24));
                                }
                            }
                            Key::Enter => {
                                e.prevent_default();
                                open.set(false);
                                commit_text();
                            }
                            Key::Escape if open() => {
                                e.stop_propagation();
                                open.set(false);
                            }
                            _ => {}
                        }
                    },
                    onblur: move |_| {
                        open.set(false);
                        commit_text();
                    },
                }
                button {
                    r#type: "button",
                    class: "ax-cal-field-btn",
                    tabindex: "-1",
                    title: "Choose a time",
                    "aria-label": "Choose a time",
                    disabled,
                    onmousedown: move |e| e.prevent_default(),
                    onclick: move |_| {
                        open.set(!open());
                        scroll_to_selected();
                        // The field takes focus, so Escape, Tab and a click
                        // elsewhere close the list as they do after typing.
                        document::eval(&format!("document.querySelector('[data-tf=\"{id}\"] .ax-cal-field-input')?.focus()"));
                    },
                    Icon { name: "chevron_down", size: 12 }
                }
            }
            if error() { span { class: "ax-cal-field-error", role: "alert", "Enter a valid time" } }
            if open() {
                div { class: "ax-cal-timelist", role: "listbox", "aria-label": "{label}",
                    for (i, (t, l)) in options.into_iter().enumerate() {
                        div {
                            key: "{i}",
                            class: "ax-cal-timeopt",
                            class: if t == value { "ax-selected" },
                            class: if active() == Some(i) { "ax-active" },
                            role: "option",
                            "aria-selected": if t == value { "true" } else { "false" },
                            // Keep the input focused so blur doesn't fire first.
                            onmousedown: move |e| e.prevent_default(),
                            onclick: move |_| {
                                open.set(false);
                                error.set(false);
                                text.set(clock(t, h24));
                                onchange.call(t);
                            },
                            "{l}"
                        }
                    }
                }
            }
        }
    }
}

/// A plausible email address ("name@domain.tld").
pub fn valid_address(a: &str) -> bool {
    let a = a.trim();
    match a.split_once('@') {
        Some((local, domain)) => !local.is_empty() && !domain.contains('@') && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.') && !a.contains(' '),
        None => false,
    }
}

/// Turn typed text into an address: "Name <a@b.c>" or a bare address.
pub fn parse_address(text: &str) -> EmailAddress {
    let t = text.trim().trim_end_matches([';', ',']).trim();
    if let (Some(lt), Some(gt)) = (t.find('<'), t.rfind('>'))
        && lt < gt
    {
        return EmailAddress::new(t[..lt].trim().trim_matches('"'), t[lt + 1..gt].trim());
    }
    EmailAddress::new("", t)
}

/// Several addresses in the box at once (a pasted list, "a@x; b@y" or
/// "a@x, b@y"): each of them, split as compose's To box does. One entry (or
/// none) gives an empty list, so the box's own handling applies.
pub fn several_addresses(text: &str) -> Vec<EmailAddress> {
    let (entries, _) = crate::mail::compose::address::parse_list(text, true);
    if entries.len() > 1 { entries } else { Vec::new() }
}

/// A paste into an attendee box (`prev` to `now`: more than one character
/// at once) that holds an entry ended by ";" or ",": every entry in it, the
/// last one too, so an address typed straight on starts a new attendee.
/// The page empties the box after such a paste (kit/typed_text.js
/// `pasteEnds`, the same rule).
pub fn pasted_list(prev: &str, now: &str) -> Option<Vec<EmailAddress>> {
    let pasted = now.chars().count() > prev.chars().count() + 1;
    if !pasted || crate::mail::compose::address::parse_list(now, false).0.is_empty() {
        return None;
    }
    Some(crate::mail::compose::address::parse_list(now, true).0)
}

/// What text left in an attendee box adds when the event is saved (no
/// Enter, Tab or ";" pressed): each entry of a list, else the text as one
/// address, valid or not (an invalid one shows as a red chip to fix).
pub fn typed_attendees(text: &str) -> Vec<EmailAddress> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    match several_addresses(text) {
        list if !list.is_empty() => list,
        _ => vec![parse_address(text)],
    }
}

/// Add what is still typed in the attendee boxes (`typed`: each box's kind
/// and text) to `draft` and empty the boxes. Returns the addresses added,
/// so the caller can stop on an invalid one.
pub fn commit_typed(mut draft: Signal<axentra_core::Event>, typed: &[(AttendeeKind, Signal<String>)]) -> Vec<EmailAddress> {
    let mut added = Vec::new();
    for &(kind, mut text) in typed {
        let Ok(t) = text.try_peek().map(|t| t.clone()) else { continue };
        let entries = typed_attendees(&t);
        if entries.is_empty() {
            continue;
        }
        text.set(String::new());
        let mut d = draft.write();
        for addr in entries {
            if addr.address.trim().is_empty() || d.attendees.iter().any(|a| a.email.address.eq_ignore_ascii_case(&addr.address)) {
                continue;
            }
            added.push(addr.clone());
            d.attendees.push(Attendee { email: addr, kind, response: ResponseStatus::None });
        }
    }
    added
}

/// Address book suggestions for typed attendee text, without the people
/// already invited.
fn attendee_suggestions(typed: &str, attendees: &[Attendee]) -> Vec<EmailAddress> {
    if typed.trim().is_empty() {
        return Vec::new();
    }
    store()
        .suggest_addresses(typed.trim(), 8)
        .unwrap_or_default()
        .into_iter()
        .filter(|a| !attendees.iter().any(|x| x.email.address.eq_ignore_ascii_case(&a.address)))
        .collect()
}

/// Required or optional attendee well with suggestions. `onchange` receives
/// the whole attendee list (other kinds untouched). `typed` holds what is
/// typed in the box, for a form that adds it on save ([`commit_typed`]).
#[component]
pub fn AttendeePicker(
    attendees: Vec<Attendee>,
    kind: AttendeeKind,
    onchange: EventHandler<Vec<Attendee>>,
    #[props(into)] placeholder: String,
    #[props(default)] disabled: bool,
    #[props(default)] typed: Option<Signal<String>>,
) -> Element {
    let id = use_hook(next_id);
    let own = use_signal(String::new);
    let mut query = typed.unwrap_or(own);
    let mut active = use_signal(|| 0usize);
    let mut chip_menu = use_signal(|| None::<(usize, f64, f64)>);
    // The list as the handlers must see it: several entries can be added
    // before the parent renders again, and each must build on the last
    // (the `attendees` prop of this render would drop the one before).
    let mut latest = use_hook(|| CopyValue::new(attendees.clone()));
    latest.set(attendees.clone());
    let q = query();
    let suggestions = attendee_suggestions(&q, &attendees);
    let suggestions_len = suggestions.len();
    let mine: Vec<(usize, Attendee)> = attendees.iter().cloned().enumerate().filter(|(_, a)| a.kind == kind).collect();
    let mut change = move |list: Vec<Attendee>| {
        latest.set(list.clone());
        onchange.call(list);
    };
    let mut add = move |addr: EmailAddress| {
        if addr.address.trim().is_empty() {
            return;
        }
        let mut list = latest.peek().clone();
        if !list.iter().any(|a| a.email.address.eq_ignore_ascii_case(&addr.address)) {
            list.push(Attendee { email: addr, kind, response: ResponseStatus::None });
            change(list);
        }
        query.set(String::new());
        active.set(0);
    };
    // What Enter or Tab adds: the highlighted suggestion for the text in the
    // box now (keys can outrun the render that listed suggestions), else
    // the text itself.
    let shown_for = q.clone();
    let shown = suggestions.clone();
    let pick_for = move |typed: &str| -> EmailAddress {
        let (list, index) = if typed == shown_for { (shown.clone(), active()) } else { (attendee_suggestions(typed, &latest.peek()), 0) };
        list.get(index).cloned().unwrap_or_else(|| parse_address(typed))
    };
    rsx! {
        div { class: "ax-cal-people", class: if disabled { "ax-disabled" }, "data-ap": "{id}",
            Icon { name: if kind == AttendeeKind::Optional { "person_add" } else { "people" }, class: "ax-cal-row-icon" }
            div { class: "ax-cal-people-well",
                for (i, a) in mine {
                    {
                        let invalid = !valid_address(&a.email.address);
                        let name = a.email.display().to_string();
                        rsx! {
                            span {
                                key: "{i}",
                                class: "ax-cal-chip-person",
                                class: if invalid { "ax-invalid" },
                                title: if invalid { format!("{} isn't a valid address", a.email.address) } else { a.email.address.clone() },
                                button {
                                    r#type: "button",
                                    class: "ax-cal-chip-person-main",
                                    disabled,
                                    onclick: move |e: MouseEvent| {
                                        let c = e.client_coordinates();
                                        chip_menu.set(Some((i, c.x, c.y + 8.0)));
                                    },
                                    Avatar { name: a.email.name.clone(), email: a.email.address.clone(), size: 20 }
                                    span { class: "ax-ellipsis", "{name}" }
                                }
                                if !disabled {
                                    button {
                                        r#type: "button",
                                        class: "ax-cal-chip-person-x",
                                        title: "Remove {name}",
                                        "aria-label": "Remove {name}",
                                        onclick: move |_| {
                                            let mut list = latest.peek().clone();
                                            if i < list.len() {
                                                list.remove(i);
                                                change(list);
                                            }
                                        },
                                        Icon { name: "dismiss", size: 12 }
                                    }
                                }
                            }
                        }
                    }
                }
                if !disabled {
                    input {
                        class: "ax-cal-people-input",
                        placeholder: if attendees.iter().any(|a| a.kind == kind) { String::new() } else { placeholder.clone() },
                        value: "{q}",
                        // Enter, Tab, ";" and "," add the entry; the page
                        // empties the box as the key goes down
                        // (kit/typed_text.js).
                        "data-ax-address-list": "keys",
                        role: "combobox",
                        "aria-label": "{placeholder}",
                        "aria-expanded": if suggestions.is_empty() { "false" } else { "true" },
                        oninput: move |e| {
                            let v = e.value();
                            active.set(0);
                            let prev = query.peek().clone();
                            match pasted_list(&prev, &v) {
                                Some(list) => {
                                    list.into_iter().for_each(&mut add);
                                    query.set(String::new());
                                }
                                None => query.set(v),
                            }
                        },
                        onkeydown: move |e: KeyboardEvent| {
                            keys::stop_typing_keys(&e);
                            let typed = query.peek().clone();
                            // Shift+Tab is "Unidentified" in WebKitGTK; it commits like Tab.
                            match if keys::is_tab(&e) { Key::Tab } else { e.key() } {
                                Key::ArrowDown => {
                                    e.prevent_default();
                                    active.set((active() + 1).min(suggestions_len.saturating_sub(1)));
                                }
                                Key::ArrowUp => {
                                    e.prevent_default();
                                    active.set(active().saturating_sub(1));
                                }
                                Key::Enter | Key::Tab if !typed.trim().is_empty() => {
                                    e.prevent_default();
                                    match several_addresses(&typed) {
                                        list if !list.is_empty() => list.into_iter().for_each(&mut add),
                                        _ => add(pick_for(typed.trim())),
                                    }
                                }
                                Key::Character(ref c) if (c == ";" || c == ",") && !typed.trim().is_empty() => {
                                    e.prevent_default();
                                    match several_addresses(&typed) {
                                        list if !list.is_empty() => list.into_iter().for_each(&mut add),
                                        _ => add(parse_address(&typed)),
                                    }
                                }
                                Key::Backspace if typed.is_empty() => {
                                    let mut list = latest.peek().clone();
                                    if let Some(pos) = list.iter().rposition(|a| a.kind == kind) {
                                        list.remove(pos);
                                        change(list);
                                    }
                                }
                                Key::Escape if !typed.is_empty() => {
                                    e.stop_propagation();
                                    query.set(String::new());
                                }
                                _ => {}
                            }
                        },
                        onblur: move |_| {
                            // Leaving the box adds a list whole (an invalid
                            // entry as a red chip, as Enter does, never
                            // dropped) and a single valid address; a single
                            // invalid one stays in the box to fix. Once the
                            // keys typed before the click that took focus
                            // have landed in the box (a busy page delivers
                            // the click first), unless focus came back.
                            let mut add = add;
                            let sel = format!("[data-ap=\"{id}\"] input.ax-cal-people-input");
                            spawn(async move {
                                if crate::kit::dom::keys_landed(&sel).await {
                                    return;
                                }
                                let typed = query.peek().clone();
                                let several = several_addresses(&typed);
                                if !several.is_empty() {
                                    several.into_iter().for_each(&mut add);
                                } else if !typed.trim().is_empty() && valid_address(&parse_address(&typed).address) {
                                    add(parse_address(&typed));
                                }
                            });
                        },
                    }
                }
            }
            if !suggestions.is_empty() {
                div { class: "ax-cal-suggest", role: "listbox",
                    for (i, s) in suggestions.into_iter().enumerate() {
                        {
                            let pick = s.clone();
                            rsx! {
                                div {
                                    key: "{s.address}",
                                    class: "ax-cal-suggest-item",
                                    class: if active() == i { "ax-active" },
                                    role: "option",
                                    onmousedown: move |e| e.prevent_default(),
                                    onclick: move |_| add(pick.clone()),
                                    Avatar { name: s.name.clone(), email: s.address.clone(), size: 28 }
                                    div { class: "ax-col ax-grow",
                                        span { class: "ax-ellipsis", {s.display().to_string()} }
                                        span { class: "ax-ellipsis ax-muted", "{s.address}" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if let Some((i, x, y)) = chip_menu() {
                Menu {
                    items: vec![
                        if attendees.get(i).is_some_and(|a| a.kind == AttendeeKind::Optional) {
                            MenuItem::new("required", "Make required").icon("person")
                        } else {
                            MenuItem::new("optional", "Make optional").icon("person")
                        },
                        MenuItem::new("remove", "Remove").icon("dismiss"),
                    ],
                    x,
                    y,
                    on_select: move |cmd: String| {
                        let mut list = latest.peek().clone();
                        match cmd.as_str() {
                            "remove" if i < list.len() => {
                                list.remove(i);
                            }
                            "required" => if let Some(a) = list.get_mut(i) { a.kind = AttendeeKind::Required },
                            "optional" => if let Some(a) = list.get_mut(i) { a.kind = AttendeeKind::Optional },
                            _ => return,
                        }
                        change(list);
                    },
                    on_close: move |_| chip_menu.set(None),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_parsing_and_validation() {
        assert!(valid_address("priya.raman@northwind.example"));
        assert!(!valid_address("priya"));
        assert!(!valid_address("a@b"));
        assert!(!valid_address("a b@c.d"));
        let a = parse_address("Priya Raman <priya@x.example>;");
        assert_eq!((a.name.as_str(), a.address.as_str()), ("Priya Raman", "priya@x.example"));
        assert_eq!(parse_address(" sam@x.example ").address, "sam@x.example");
    }

    /// A pasted list becomes one attendee per address, as in compose's To.
    #[test]
    fn a_pasted_list_is_several_attendees() {
        let got = |t: &str| several_addresses(t).into_iter().map(|a| a.address).collect::<Vec<_>>();
        assert_eq!(got("dan@example.test; eve@example.test"), ["dan@example.test", "eve@example.test"]);
        assert_eq!(got("gus@example.test, hal@example.test"), ["gus@example.test", "hal@example.test"]);
        assert_eq!(got("Dan Kim <dan@example.test>, \"Lee, Eve\" <eve@example.test>;"), ["dan@example.test", "eve@example.test"]);
        assert!(got("dan@example.test").is_empty(), "one address: the box handles it");
        assert!(got("\"Lee, Eve\" <eve@example.test>").is_empty(), "a comma inside quotes is not a separator");
        assert!(got("").is_empty());
    }

    /// Pasting "x1; x2" and typing x3 straight on gives three attendees:
    /// the paste adds both at once instead of waiting for Enter, which then
    /// saw "x2...x3" as one invalid address.
    #[test]
    fn a_paste_adds_its_list_at_once() {
        let got = |prev: &str, now: &str| pasted_list(prev, now).map(|l| l.into_iter().map(|a| a.address).collect::<Vec<_>>());
        assert_eq!(got("", "x1@example.org; x2@example.org"), Some(vec!["x1@example.org".to_string(), "x2@example.org".to_string()]));
        assert_eq!(got("", "dan@example.test; eve@example").map(|l| l.len()), Some(2), "an invalid entry too (a red chip)");
        assert_eq!(got("", "x1@example.org"), None, "one address stays typed");
        assert_eq!(got("x1@example.or", "x1@example.org"), None, "typing is not a paste");
        assert_eq!(got("", "\"Lee, Eve\" <eve@example.test>"), None, "a comma in quotes ends nothing");
    }

    /// Text left in the box when the event is saved: a list, or one
    /// address as typed (valid or not).
    #[test]
    fn typed_text_becomes_attendees_on_save() {
        let got = |t: &str| typed_attendees(t).into_iter().map(|a| a.address).collect::<Vec<_>>();
        assert_eq!(got("a@example.org; b@example.org"), ["a@example.org", "b@example.org"]);
        assert_eq!(got(" pending@example.org "), ["pending@example.org"]);
        assert_eq!(got("eve@example"), ["eve@example"]);
        assert!(got("  ").is_empty());
    }
}
