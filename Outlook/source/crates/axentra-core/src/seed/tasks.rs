//! Demo To Do lists and tasks: grouped lists with emoji, steps, notes,
//! overdue and repeating tasks, reminders, an assigned task, a task linked
//! to an email, yesterday's My Day leftovers and some completed history.

use chrono::{Duration, NaiveTime, Utc, Weekday};

use super::SeedCtx;
use crate::model::*;
use crate::{Result, Store};

/// One demo task: the task, how many days ago it was created, and (for
/// completed tasks) how many days ago it was completed.
struct Spec {
    task: Task,
    created_days_ago: i64,
    completed_days_ago: Option<i64>,
}

fn spec(task: Task) -> Spec {
    Spec { task, created_days_ago: 1, completed_days_ago: None }
}

impl Spec {
    fn created(mut self, days_ago: i64) -> Self {
        self.created_days_ago = days_ago;
        self
    }
    fn done(mut self, days_ago: i64) -> Self {
        self.completed_days_ago = Some(days_ago);
        self
    }
}

pub(super) fn seed(store: &Store, ctx: &SeedCtx) -> Result<()> {
    let today = ctx.now.date_naive();
    let at = |days: i64, h: u32| crate::local_to_utc(today + Duration::days(days), NaiveTime::from_hms_opt(h, 0, 0).unwrap_or_default());
    let tasks_list = store.default_task_list(Some(ctx.account_id))?.expect("account has a task list");

    let work = store.create_task_group("Work")?;
    let personal = store.create_task_group("Personal")?;
    let list = |name: &str, group: Option<&str>| -> Result<TaskListId> {
        let mut l = TaskList { account_id: ctx.account_id, name: name.into(), group: group.map(str::to_string), ..Default::default() };
        store.insert_task_list(&mut l)
    };
    let groceries = list("🛒 Groceries", None)?;
    let launch = list("🚀 Launch prep", Some(&work))?;
    let q4 = list("📊 Q4 planning", Some(&work))?;
    let home = list("🏠 Home projects", Some(&personal))?;
    let reading = list("📚 Reading list", Some(&personal))?;

    let step = |id: i64, title: &str, done: bool| TaskStep { id, title: title.into(), is_completed: done };
    let weekly_on = |days: Vec<Weekday>| Recurrence { freq: RecurrenceFreq::Weekly, interval: 1, by_weekday: days, ..Default::default() };
    let every = |freq: RecurrenceFreq| Recurrence { freq, interval: 1, ..Default::default() };
    let t = |list_id: TaskListId, title: &str| Task { list_id, title: title.into(), ..Default::default() };
    let linked = store
        .messages(&MessageQuery { limit: Some(20), ..Default::default() })?
        .into_iter()
        .find(|m| !m.is_draft && !m.subject.is_empty());

    let mut specs: Vec<Spec> = vec![
        // Tasks
        spec(Task { due: Some(today + Duration::days(2)), is_important: true, my_day: Some(today), categories: vec!["Blue category".into()], ..t(tasks_list.id, "Review Q4 launch plan") }).created(4),
        spec(Task {
            due: Some(today + Duration::days(3)),
            my_day: Some(today),
            reminder: Some(at(0, 15)),
            steps: vec![step(1, "Collect merged PRs", true), step(2, "Write highlights", false), step(3, "Send to Marcus", false)],
            notes: "Keep it under one page. Link the migration guide at the end.".into(),
            ..t(tasks_list.id, "Draft release notes")
        })
        .created(3),
        spec(Task { due: Some(today), ..t(tasks_list.id, "Call the dentist to reschedule") }).created(2),
        spec(Task { due: Some(today), recurrence: Some(weekly_on(vec![today_weekday(ctx)])), ..t(tasks_list.id, "Water the office plants") }).created(30),
        spec(Task { due: Some(today + Duration::days(1)), notes: "Corporate rate code: AXN-24".into(), ..t(tasks_list.id, "Book Seattle hotel") }).created(5),
        spec(Task { due: Some(today - Duration::days(2)), is_important: true, reminder: Some(at(-2, 9)), ..t(tasks_list.id, "Expense report for September") }).created(12),
        spec(Task {
            my_day: Some(today - Duration::days(1)),
            steps: vec![step(1, "Career goals", false), step(2, "Feedback from the design review", false)],
            ..t(tasks_list.id, "Prepare 1:1 agenda for Marcus")
        })
        .created(2),
        spec(Task { assigned_to_me: true, due: Some(today + Duration::days(2)), ..t(tasks_list.id, "Send interview feedback") }).created(3),
        spec(Task { due: Some(today + Duration::days(12)), categories: vec!["Green category".into()], ..t(tasks_list.id, "Plan team offsite #team") }).created(6),
        spec(Task { due: Some(today + Duration::days(6)), reminder: Some(at(6, 9)), recurrence: Some(every(RecurrenceFreq::Monthly)), ..t(tasks_list.id, "Pay credit card bill") }).created(40),
        spec(Task { my_day: Some(today), ..t(tasks_list.id, "Update the team wiki") }).created(1).done(0),
        spec(t(tasks_list.id, "Renew passport")).created(20).done(3),
        // Launch prep
        spec(Task { due: Some(today + Duration::days(5)), is_important: true, ..t(launch, "Confirm venue deposit") }).created(9),
        spec(Task { assigned_to_me: true, due: Some(today + Duration::days(1)), categories: vec!["Orange category".into()], ..t(launch, "Approve booth design") }).created(2),
        spec(t(launch, "Finalize press list")).created(7),
        spec(Task { due: Some(today + Duration::days(9)), steps: vec![step(1, "Record backup video", false), step(2, "Dry run with Hannah", false)], ..t(launch, "Rehearse keynote demo") }).created(5),
        spec(t(launch, "Order swag")).created(10).done(1),
        // Q4 planning
        spec(Task { due: Some(today - Duration::days(1)), is_important: true, ..t(q4, "Draft OKRs for Q4") }).created(8),
        spec(Task { due: Some(today + Duration::days(7)), ..t(q4, "Collect budget requests") }).created(6),
        spec(Task { reminder: Some(at(4, 10)), ..t(q4, "Headcount review with finance") }).created(3),
        spec(Task { notes: "Owners: Priya (roadmap), Wei (budget), Grace (ops).".into(), ..t(q4, "Kickoff meeting notes") }).created(14).done(6),
        // Groceries
        spec(t(groceries, "Oat milk")).created(1),
        spec(t(groceries, "Coffee beans")).created(1),
        spec(t(groceries, "Basil")).created(1),
        spec(t(groceries, "Sourdough loaf")).created(2).done(0),
        // Home projects
        spec(Task { due: Some(today + Duration::days(3)), steps: vec![step(1, "Buy washers", true), step(2, "Turn off the water first", false)], ..t(home, "Fix the leaky faucet") }).created(4),
        spec(Task { notes: "Swatches: Harbor Mist or Sandstone. Need 2 litres.".into(), ..t(home, "Paint the spare room") }).created(15),
        spec(Task { due: Some(today + Duration::days(20)), recurrence: Some(every(RecurrenceFreq::Yearly)), ..t(home, "Replace smoke detector batteries") }).created(60),
        // Reading list
        spec(t(reading, "Finish the design systems handbook")).created(9),
        spec(Task { categories: vec!["Purple category".into()], ..t(reading, "Read the Q3 customer research report") }).created(3),
        spec(t(reading, "Skim the accessibility guidelines update")).created(20).done(4),
    ];
    if let Some(m) = &linked {
        specs.push(
            spec(Task { linked_message_id: Some(m.id), due: Some(today + Duration::days(1)), my_day: Some(today), ..t(tasks_list.id, &format!("Follow up: {}", m.subject)) })
                .created(0),
        );
    }

    let now = Utc::now();
    for s in specs {
        let mut task = s.task;
        task.is_completed = false;
        if task.my_day.is_some() {
            task.my_day_at = Some(now);
        }
        let id = store.insert_task_at(&mut task, false)?;
        let mut saved = store.task(id)?.expect("just inserted");
        saved.created_at = now - Duration::days(s.created_days_ago);
        if let Some(days) = s.completed_days_ago {
            saved.is_completed = true;
            saved.completed_at = Some(now - Duration::days(days) - Duration::minutes(30));
        }
        store.update_task(&saved)?;
    }
    Ok(())
}

fn today_weekday(ctx: &SeedCtx) -> Weekday {
    use chrono::Datelike;
    ctx.now.weekday()
}
