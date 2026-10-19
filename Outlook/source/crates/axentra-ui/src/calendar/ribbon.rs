//! Calendar ribbon: Home (New event, Day ▾, Work week, Week, Month, Split
//! view, Filter, Share, Print), View (views, Time scale, Filter, Saved views,
//! Calendar settings) and Help.

use std::collections::BTreeSet;

use axentra_core::*;
use dioxus::prelude::*;

use super::nav_pane::new_event_now;
use super::range::ViewKind;
use super::state::*;
use super::text::{SHOW_AS_CHOICES, show_as_id, show_as_label};
use crate::kit::MenuItem;
use crate::shell::ribbon::{Command, Ribbon, RibbonTab};
use crate::state::{AppState, SettingsPage, store, use_app};

fn view_commands(view: ViewKind) -> Vec<Command> {
    let days = match view {
        ViewKind::Day(n) => n,
        _ => 0,
    };
    let day_menu: Vec<MenuItem> = (1..=7u8)
        .map(|n| MenuItem::new(format!("days:{n}"), if n == 1 { "1 day".to_string() } else { format!("{n} days") }).checked(days == n))
        .collect();
    vec![
        Command::new("view_day", "Day", "calendar_day").split(day_menu).toggled(days > 0).tooltip("Day (Ctrl+Alt+1)"),
        Command::new("view_workweek", "Work week", "calendar_work_week").toggled(view == ViewKind::WorkWeek).tooltip("Work week (Ctrl+Alt+2)"),
        Command::new("view_week", "Week", "calendar_week_start").toggled(view == ViewKind::Week).tooltip("Week (Ctrl+Alt+3)"),
        Command::new("view_month", "Month", "calendar_month").toggled(view == ViewKind::Month).tooltip("Month (Ctrl+Alt+4)"),
    ]
}

fn filter_menu(f: &EventFilter, cats: &[Category]) -> Vec<MenuItem> {
    let cat_on = |name: &str| f.categories.as_ref().is_some_and(|s| s.contains(name));
    let mut cat_items: Vec<MenuItem> = cats
        .iter()
        .map(|c| MenuItem::new(format!("f:cat:{}", c.name), c.name.clone()).swatch(crate::kit::category_colors(&c.color).0).checked(cat_on(&c.name)))
        .collect();
    cat_items.push(MenuItem::new("f:cat:", "Not categorized").checked(cat_on("")));
    let show_as: Vec<MenuItem> = SHOW_AS_CHOICES
        .iter()
        .map(|s| MenuItem::new(format!("f:showas:{}", show_as_id(*s)), show_as_label(*s)).checked(f.show_as.as_ref().is_some_and(|x| x.contains(show_as_id(*s)))))
        .collect();
    vec![
        MenuItem::new("f:all", "All").checked(!f.is_active()),
        MenuItem::new("f:appointments", "Appointments").checked(f.appointments),
        MenuItem::new("f:meetings", "Meetings").checked(f.meetings),
        MenuItem::new("f:categories", "Categories").icon("tag").submenu(cat_items),
        MenuItem::new("f:showas", "Show as").icon("calendar_clock").submenu(show_as),
        MenuItem::new("f:recurrence", "Recurrence").icon("arrow_repeat_all").submenu(vec![
            MenuItem::new("f:rec:yes", "Recurring").checked(f.recurring == Some(true)),
            MenuItem::new("f:rec:no", "Not recurring").checked(f.recurring == Some(false)),
        ]),
        MenuItem::new("f:privacy", "Privacy").icon("lock_closed").submenu(vec![
            MenuItem::new("f:priv:yes", "Private").checked(f.private == Some(true)),
            MenuItem::new("f:priv:no", "Not private").checked(f.private == Some(false)),
        ]),
        MenuItem::divider(),
        MenuItem::new("f:declined", "Hide declined events").checked(f.hide_declined),
    ]
}

fn filter_command(f: &EventFilter, cmd: &str) -> EventFilter {
    let mut f = f.clone();
    let toggle = |set: &mut Option<BTreeSet<String>>, v: &str| {
        let mut s = set.take().unwrap_or_default();
        if !s.remove(v) {
            s.insert(v.to_string());
        }
        *set = (!s.is_empty()).then_some(s);
    };
    if let Some(name) = cmd.strip_prefix("f:cat:") {
        toggle(&mut f.categories, name);
    } else if let Some(id) = cmd.strip_prefix("f:showas:") {
        if let Some(sa) = super::text::show_as_from_id(id) {
            let key = show_as_id(sa);
            let mut s = f.show_as.take().unwrap_or_default();
            if !s.remove(key) {
                s.insert(key);
            }
            f.show_as = (!s.is_empty()).then_some(s);
        }
    } else {
        match cmd {
            "f:all" => f = EventFilter { hide_declined: f.hide_declined, ..EventFilter::default() },
            "f:appointments" => f.appointments = !f.appointments,
            "f:meetings" => f.meetings = !f.meetings,
            "f:rec:yes" => f.recurring = if f.recurring == Some(true) { None } else { Some(true) },
            "f:rec:no" => f.recurring = if f.recurring == Some(false) { None } else { Some(false) },
            "f:priv:yes" => f.private = if f.private == Some(true) { None } else { Some(true) },
            "f:priv:no" => f.private = if f.private == Some(false) { None } else { Some(false) },
            "f:declined" => f.hide_declined = !f.hide_declined,
            _ => {}
        }
    }
    // Never hide both kinds.
    if !f.appointments && !f.meetings {
        f.appointments = true;
        f.meetings = true;
    }
    f
}

#[component]
pub fn CalendarRibbon() -> Element {
    let app = use_app();
    let cal = use_cal();
    let s = app.settings.read().clone();
    let _ = app.calendar_rev();
    let cals = store().calendars().unwrap_or_default();
    let cats = store().categories().unwrap_or_default();
    let view = (cal.view)();
    let filter = (cal.filter)();
    let scale = (cal.scale)();
    let checked = cals.iter().filter(|c| c.is_visible).count();
    let new_menu: Vec<MenuItem> = std::iter::once(MenuItem::new("new:event", "Event").icon("calendar_add"))
        .chain(cals.iter().filter(|c| c.can_edit && !c.is_default).map(|c| MenuItem::new(format!("new:{}", c.id), format!("Event in {}", c.name)).swatch(c.color.clone())))
        .collect();
    let split_tip = if checked < 2 { "Split view needs two or more calendars selected" } else { "Split view isn't available yet" };
    let filter_cmd = Command::new("filter", "Filter", "filter").menu(filter_menu(&filter, &cats)).toggled(filter.is_active()).tooltip(if filter.is_active() { "Filter (on)" } else { "Filter" });
    let home = RibbonTab::new(
        "home",
        "Home",
        vec![
            vec![Command::new("new_event", "New event", "calendar_add").primary().split(new_menu).tooltip("New event (Ctrl+N)")],
            view_commands(view),
            vec![
                Command::new("split", "Split view", "split_vertical").disabled(true).tooltip(split_tip),
                filter_cmd.clone(),
                Command::new("share", "Share", "share").disabled(true).tooltip("Sharing isn't available for local calendars"),
                Command::new("print", "Print", "print").tooltip("Print (Ctrl+P)"),
            ],
        ],
    );
    let scale_menu: Vec<MenuItem> = SCALES.iter().map(|m| MenuItem::new(format!("scale:{m}"), format!("{m} minutes")).checked(scale == *m)).collect();
    let view_tab = RibbonTab::new(
        "view",
        "View",
        vec![
            view_commands(view),
            vec![
                Command::new("split", "Split view", "split_vertical").disabled(true).tooltip(split_tip),
                Command::new("scale", "Time scale", "clock").menu(scale_menu).disabled(view == ViewKind::Month).tooltip(if view == ViewKind::Month { "Time scale applies to Day and Week views" } else { "Time scale" }),
                filter_cmd,
                Command::new("saved", "Saved views", "board").disabled(true).tooltip("Saved views aren't available yet"),
            ],
            vec![Command::new("settings", "Calendar settings", "settings")],
        ],
    );
    let help = RibbonTab::new("help", "Help", vec![vec![Command::new("shortcuts", "Keyboard shortcuts", "keyboard")]]);
    let collapsed = s.folder_pane_collapsed;
    rsx! {
        Ribbon {
            tabs: vec![home, view_tab, help],
            on_command: move |id: String| run_command(app, cal, &id),
            on_toggle_nav: move |_| {
                let c = app.settings.peek().folder_pane_collapsed;
                app.update_settings(|s| s.folder_pane_collapsed = !c);
            },
            nav_open: !collapsed,
        }
    }
}

/// Ribbon command (also used by keyboard shortcuts).
pub fn run_command(app: AppState, mut cal: Cal, id: &str) {
    let s = app.settings.peek().clone();
    if let Some(n) = id.strip_prefix("days:").and_then(|v| v.parse::<u8>().ok()) {
        cal.set_view(app, &s, ViewKind::Day(n));
        return;
    }
    if let Some(m) = id.strip_prefix("scale:").and_then(|v| v.parse::<u32>().ok()) {
        cal.set_scale(m);
        return;
    }
    if id.starts_with("f:") {
        let next = filter_command(&cal.filter.peek(), id);
        cal.filter.set(next);
        return;
    }
    if let Some(rest) = id.strip_prefix("new:") {
        new_event_now(app, cal, rest.parse::<CalendarId>().ok());
        return;
    }
    match id {
        "new_event" => new_event_now(app, cal, None),
        "view_day" => cal.set_view(app, &s, ViewKind::Day(1)),
        "view_workweek" => cal.set_view(app, &s, ViewKind::WorkWeek),
        "view_week" => cal.set_view(app, &s, ViewKind::Week),
        "view_month" => cal.set_view(app, &s, ViewKind::Month),
        "print" => {
            document::eval("window.print()");
        }
        "settings" => {
            let mut d = app.settings_dialog;
            d.set(Some(SettingsPage::Calendar));
        }
        "shortcuts" => cal.dialog.set(Some(CalDialog::Shortcuts)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_commands_toggle_and_reset() {
        let f = EventFilter::default();
        let f = filter_command(&f, "f:meetings");
        assert!(!f.meetings && f.is_active());
        // Turning off both kinds is refused.
        let g = filter_command(&f, "f:appointments");
        assert!(g.appointments && g.meetings);
        let f = filter_command(&f, "f:cat:Red category");
        assert!(f.categories.as_ref().unwrap().contains("Red category"));
        let f = filter_command(&f, "f:cat:Red category");
        assert!(f.categories.is_none());
        let f = filter_command(&f, "f:showas:free");
        assert!(f.show_as.as_ref().unwrap().contains("free"));
        let f = filter_command(&f, "f:rec:yes");
        assert_eq!(f.recurring, Some(true));
        let f = filter_command(&f, "f:declined");
        assert!(!f.hide_declined);
        let f = filter_command(&f, "f:all");
        assert_eq!(f, EventFilter { hide_declined: false, ..EventFilter::default() });
    }
}
