//! Notifications pane (docs/spec/shell.md §3.5): new mail (batched per
//! conversation), event reminders and task reminders, with read, seen and
//! dismissed state kept in the store's key/value table.

use std::collections::{BTreeSet, HashMap};

use axentra_core::*;
use chrono::{DateTime, Duration, Local, NaiveTime, Utc};
use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

use super::my_day::SidePaneFrame;
use super::{ShellCtx, SidePane, use_shell};
use crate::kit::{Avatar, Button, EmptyState, Icon, IconButton, Select, Variant, dom};
use crate::state::{NavRequest, store, use_app};

const STATE_KEY: &str = "shell.notifications";
/// New mail older than this does not notify.
const MAIL_WINDOW_HOURS: i64 = 48;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeKind {
    Mail,
    Event,
    Task,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    /// Stable key; changes when a thread gets a newer message.
    pub key: String,
    pub kind: NoticeKind,
    pub account: Option<AccountId>,
    pub title: String,
    pub snippet: String,
    pub when: DateTime<Utc>,
    pub important: bool,
    /// Messages batched into this card (mail).
    pub count: usize,
    pub sender: Option<EmailAddress>,
    pub target: NavRequest,
}

/// Which notices were read (clicked), seen (pane opened) or dismissed.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NoticeState {
    pub read: BTreeSet<String>,
    pub seen: BTreeSet<String>,
    pub dismissed: BTreeSet<String>,
}

impl NoticeState {
    pub fn load(store: &axentra_core::Store) -> Self {
        store.kv_get(STATE_KEY).ok().flatten().and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default()
    }

    fn save(&self) {
        if let Ok(json) = serde_json::to_string(self) {
            let _ = store().kv_set(STATE_KEY, &json);
        }
    }

    /// Forget keys of notices that no longer exist so the state stays small.
    fn prune(&mut self, live: &[String]) {
        let keep = |set: &mut BTreeSet<String>| set.retain(|k| live.contains(k));
        keep(&mut self.read);
        keep(&mut self.seen);
        keep(&mut self.dismissed);
    }
}

/// Collect current notices, newest first.
pub fn collect(store: &axentra_core::Store, settings: &Settings, now: DateTime<Local>) -> Vec<Notice> {
    let mut out = Vec::new();
    let now_utc = now.with_timezone(&Utc);
    if settings.notify_mail {
        for acct in store.accounts().unwrap_or_default() {
            let Some(inbox) = store.folder_by_role(acct.id, FolderRole::Inbox).ok().flatten() else { continue };
            let q = MessageQuery {
                folder_id: Some(inbox.id),
                filter: MessageFilter::Unread,
                inference: (settings.focused_inbox && settings.notify_focused_only).then_some(InferenceClass::Focused),
                limit: Some(100),
                ..Default::default()
            };
            let mut threads: HashMap<String, Notice> = HashMap::new();
            for m in store.messages(&q).unwrap_or_default() {
                if m.is_draft || now_utc - m.received_at > Duration::hours(MAIL_WINDOW_HOURS) {
                    continue;
                }
                let thread = if m.conversation_id.is_empty() { format!("m{}", m.id) } else { m.conversation_id.clone() };
                let important = m.importance == Importance::High || m.mentions_me;
                threads
                    .entry(thread.clone())
                    .and_modify(|n| {
                        n.count += 1;
                        n.important |= important;
                        if m.received_at > n.when {
                            n.when = m.received_at;
                            n.title = m.from.display().to_string();
                            n.snippet = m.subject.clone();
                            n.key = format!("mail:{thread}:{}", m.id);
                            n.target = NavRequest::Message(m.id);
                        }
                    })
                    .or_insert_with(|| Notice {
                        key: format!("mail:{thread}:{}", m.id),
                        kind: NoticeKind::Mail,
                        account: Some(acct.id),
                        title: m.from.display().to_string(),
                        snippet: if m.subject.is_empty() { "(No subject)".into() } else { m.subject.clone() },
                        when: m.received_at,
                        important,
                        count: 1,
                        sender: Some(m.from.clone()),
                        target: NavRequest::Message(m.id),
                    });
            }
            out.extend(threads.into_values());
        }
    }
    if settings.notify_calendar {
        let from = now_utc - Duration::hours(1);
        let to = local_to_utc(now.date_naive() + Duration::days(1), NaiveTime::MIN);
        for o in store.events_in_range(from, to).unwrap_or_default() {
            let Some(mins) = o.event.reminder_minutes else { continue };
            let due = o.start - Duration::minutes(mins as i64);
            if o.event.all_day || o.event.is_cancelled || o.event.my_response == ResponseStatus::Declined || now_utc < due || now_utc >= o.end {
                continue;
            }
            let starts = if o.start <= now_utc { "Now".to_string() } else { format!("Starts at {}", crate::fmt::time(o.start, settings.use_24h_time)) };
            let snippet = if o.event.location.is_empty() { starts } else { format!("{starts} · {}", o.event.location) };
            out.push(Notice {
                key: format!("event:{}:{}", o.event.id, o.start.timestamp()),
                kind: NoticeKind::Event,
                account: None,
                title: o.event.title.clone(),
                snippet,
                when: due,
                important: false,
                count: 1,
                sender: None,
                target: NavRequest::Event(o.event.id),
            });
        }
    }
    for list in store.task_lists().unwrap_or_default() {
        for t in store.tasks_in_list(list.id).unwrap_or_default() {
            let Some(r) = t.reminder else { continue };
            if t.is_completed || r > now_utc {
                continue;
            }
            let snippet = match t.due {
                Some(d) => format!("Reminder · Due {}", crate::fmt::short_date(d)),
                None => "Reminder".to_string(),
            };
            out.push(Notice {
                key: format!("task:{}:{}", t.id, r.timestamp()),
                kind: NoticeKind::Task,
                account: Some(list.account_id),
                title: t.title.clone(),
                snippet,
                when: r,
                important: t.is_important,
                count: 1,
                sender: None,
                target: NavRequest::Task(t.id),
            });
        }
    }
    out.sort_by_key(|n| std::cmp::Reverse(n.when));
    out
}

/// Count for the header badge: notices not yet seen and not dismissed.
pub fn use_unseen_count() -> usize {
    let app = use_app();
    let shell = use_shell();
    let _ = (app.mail_rev(), app.calendar_rev(), app.tasks_rev());
    let now = (shell.now)();
    let state = shell.notices.read();
    let settings = app.settings.read();
    collect(store(), &settings, now).iter().filter(|n| !state.seen.contains(&n.key) && !state.dismissed.contains(&n.key)).count()
}

/// "5 min ago", "2 hr ago", "Yesterday", "Mon".
pub fn relative(when: DateTime<Utc>, now: DateTime<Local>) -> String {
    let d = now.with_timezone(&Utc) - when;
    if d < Duration::minutes(1) {
        "Just now".into()
    } else if d < Duration::hours(1) {
        format!("{} min ago", d.num_minutes())
    } else if d < Duration::hours(24) && local_date(when) == now.date_naive() {
        format!("{} hr ago", d.num_hours())
    } else if local_date(when) == now.date_naive() - Duration::days(1) {
        "Yesterday".into()
    } else {
        crate::fmt::short_date(local_date(when))
    }
}

fn mark(mut shell: ShellCtx, f: impl FnOnce(&mut NoticeState)) {
    let mut state = shell.notices.write();
    f(&mut state);
    state.save();
}

#[component]
pub fn NotificationsPane() -> Element {
    let app = use_app();
    let shell = use_shell();
    let _ = (app.mail_rev(), app.calendar_rev(), app.tasks_rev(), app.accounts_rev());
    let now = (shell.now)();
    let mut account = use_signal(|| "all".to_string());
    let accounts = store().accounts().unwrap_or_default();
    let all = collect(store(), &app.settings.read(), now);
    let state = shell.notices.read().clone();
    let visible: Vec<Notice> = all
        .iter()
        .filter(|n| !state.dismissed.contains(&n.key))
        .filter(|n| account() == "all" || n.account.is_none() || n.account.map(|a| a.to_string()) == Some(account()))
        .cloned()
        .collect();

    // While the pane is open, everything it lists counts as seen (clears the badge).
    let live: Vec<String> = all.iter().map(|n| n.key.clone()).collect();
    use_effect(use_reactive((&live,), move |(live,)| {
        mark(shell, |s| {
            s.prune(&live);
            s.seen.extend(live.iter().cloned());
        });
    }));

    let mut options = vec![("all".to_string(), "All accounts".to_string())];
    options.extend(accounts.iter().map(|a| (a.id.to_string(), a.email.clone())));
    let close = move |_| {
        let mut shell = shell;
        shell.pane.set(None);
        dom::focus(".ax-header-end [aria-label^='Notifications']");
    };
    let keys: Vec<String> = visible.iter().map(|n| n.key.clone()).collect();
    rsx! {
        SidePaneFrame { title: "Notifications", close_label: "Close notifications", class: "ax-notifications", on_close: close,
            div { class: "ax-notif-toolbar",
                if accounts.len() > 1 {
                    Select { value: account(), options, aria_label: "Account", onchange: move |v| account.set(v), class: "ax-grow" }
                } else {
                    span { class: "ax-grow" }
                }
                Button {
                    label: "Dismiss all",
                    variant: Variant::Subtle,
                    disabled: visible.is_empty(),
                    title: if visible.is_empty() { "There are no notifications to dismiss" } else { "Dismiss all notifications" },
                    onclick: move |_| {
                        let keys = keys.clone();
                        mark(shell, |s| s.dismissed.extend(keys));
                        dom::announce("All notifications dismissed", false);
                    },
                }
            }
            div { class: "ax-sidepanel-body",
                if visible.is_empty() {
                    EmptyState { icon: "alert_badge", title: "You're all caught up" }
                } else {
                    div { class: "ax-notif-list", role: "list", "aria-label": "Notifications",
                        for n in visible {
                            NoticeCard { key: "{n.key}", unread: !state.read.contains(&n.key), notice: n, now }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn NoticeCard(notice: Notice, unread: bool, now: DateTime<Local>) -> Element {
    let app = use_app();
    let mut shell = use_shell();
    let n = notice.clone();
    let key = n.key.clone();
    let key2 = n.key.clone();
    let when = relative(n.when, now);
    let icon = match n.kind {
        NoticeKind::Mail => "mail",
        NoticeKind::Event => "calendar_clock",
        NoticeKind::Task => "task_list_square_ltr",
    };
    let label = format!(
        "{}{}{}, {}, {}",
        if unread { "Unread, " } else { "" },
        if n.important { "Important, " } else { "" },
        n.title,
        n.snippet,
        when
    );
    rsx! {
        div { class: "ax-notif-card", class: if unread { "ax-unread" }, role: "listitem",
            button {
                r#type: "button",
                class: "ax-notif-open",
                "aria-label": "{label}",
                onclick: move |_| {
                    let key = key.clone();
                    mark(shell, |s| {
                        s.read.insert(key);
                    });
                    shell.pane.set(None);
                    app.navigate(n.target.clone());
                },
                if let Some(from) = &notice.sender {
                    Avatar { name: from.name.clone(), email: from.address.clone(), size: 32 }
                } else {
                    span { class: "ax-notif-icon", Icon { name: icon, size: 20 } }
                }
                span { class: "ax-notif-text",
                    span { class: "ax-notif-title ax-ellipsis",
                        "{notice.title}"
                        if notice.count > 1 { span { class: "ax-muted", " ({notice.count})" } }
                    }
                    span { class: "ax-notif-snippet ax-ellipsis", "{notice.snippet}" }
                    span { class: "ax-notif-time",
                        if notice.important { Icon { name: "important", size: 12, class: "ax-notif-important" } }
                        "{when}"
                    }
                }
            }
            IconButton {
                icon: "dismiss",
                title: "Dismiss",
                small: true,
                class: "ax-notif-dismiss",
                onclick: move |_| {
                    let key = key2.clone();
                    mark(shell, |s| {
                        s.dismissed.insert(key);
                    });
                },
            }
        }
    }
}

/// Toggle the pane from elsewhere (e.g. a keyboard shortcut).
pub fn toggle(app: crate::state::AppState, shell: ShellCtx) {
    shell.toggle_pane(app, SidePane::Notifications);
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn mail_notices_batch_by_conversation_and_respect_settings() {
        let store = axentra_core::Store::open_in_memory().unwrap();
        let now = Local.with_ymd_and_hms(2026, 10, 19, 11, 30, 0).unwrap();
        axentra_core::seed::seed_demo(&store, now).unwrap();
        let settings = Settings::default();
        let notices = collect(&store, &settings, now);
        let mail: Vec<&Notice> = notices.iter().filter(|n| n.kind == NoticeKind::Mail).collect();
        assert!(!mail.is_empty(), "demo inbox has recent unread mail");
        let mut keys: Vec<&str> = mail.iter().map(|n| n.key.as_str()).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), mail.len(), "one card per conversation");
        assert!(notices.windows(2).all(|w| w[0].when >= w[1].when), "newest first");

        let off = Settings { notify_mail: false, notify_calendar: false, ..Settings::default() };
        assert!(collect(&store, &off, now).iter().all(|n| n.kind == NoticeKind::Task));
    }

    #[test]
    fn notice_state_prunes_stale_keys() {
        let mut s = NoticeState::default();
        s.read.insert("mail:a:1".into());
        s.dismissed.insert("gone".into());
        s.prune(&["mail:a:1".to_string()]);
        assert!(s.read.contains("mail:a:1"));
        assert!(s.dismissed.is_empty());
    }

    #[test]
    fn relative_times() {
        let now = Local.with_ymd_and_hms(2026, 10, 19, 11, 30, 0).unwrap();
        let ago = |m: i64| now.with_timezone(&Utc) - Duration::minutes(m);
        assert_eq!(relative(ago(0), now), "Just now");
        assert_eq!(relative(ago(5), now), "5 min ago");
        assert_eq!(relative(ago(120), now), "2 hr ago");
        assert_eq!(relative(ago(60 * 20), now), "Yesterday");
    }
}
