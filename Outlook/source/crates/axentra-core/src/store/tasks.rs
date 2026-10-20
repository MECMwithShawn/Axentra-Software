//! To Do lists and tasks.
//!
//! Smart lists (My Day, Important, Planned, Assigned to me) are views over
//! the stored tasks, never backing lists: a task added in a smart list lives
//! in the default "Tasks" list with the matching attribute set.
//!
//! List groups exist only locally: `TaskList::group` names the group a list
//! sits in, and the kv entry [`GROUPS_KEY`] keeps the order of the groups
//! (and groups that don't hold a list yet).

use std::collections::HashMap;

use chrono::{DateTime, Datelike, Duration, Months, NaiveDate, NaiveTime, SecondsFormat, Utc};

use super::Store;
use crate::model::*;
use crate::{Error, Result};

pub(crate) const SCHEMA_EXTRA: &str = "";

/// kv key holding the ordered task-list group names (JSON array).
const GROUPS_KEY: &str = "todo.groups";

/// Next due date of a repeating task after `due` (or after today if none).
pub fn next_due(rule: &Recurrence, due: NaiveDate) -> NaiveDate {
    let n = rule.interval.max(1);
    match rule.freq {
        RecurrenceFreq::Daily => due + Duration::days(n as i64),
        RecurrenceFreq::Weekly => {
            if rule.by_weekday.len() > 1 {
                // Next listed weekday after `due`, jumping intervals at week end.
                let mut d = due + Duration::days(1);
                for _ in 0..(7 * n as i64 + 7) {
                    if rule.by_weekday.contains(&d.weekday()) {
                        return d;
                    }
                    d += Duration::days(1);
                }
                due + Duration::weeks(n as i64)
            } else {
                due + Duration::weeks(n as i64)
            }
        }
        RecurrenceFreq::Monthly => due.checked_add_months(Months::new(n)).unwrap_or(due),
        RecurrenceFreq::Yearly => due.checked_add_months(Months::new(12 * n)).unwrap_or(due),
    }
}

/// Code points that start an emoji (pictographs, symbols, dingbats, flags).
fn is_pictographic(c: char) -> bool {
    matches!(c as u32, 0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0x2B00..=0x2BFF | 0x2300..=0x23FF | 0x3030 | 0x303D | 0x3297 | 0x3299)
}

/// Split a leading emoji off a list name, as To Do does for the list icon:
/// "🛒 Groceries" -> (Some("🛒"), "Groceries"). Handles variation selectors,
/// skin tones, ZWJ sequences and flags. A name that is only an emoji is kept
/// as the name.
pub fn split_list_emoji(name: &str) -> (Option<String>, String) {
    let name = name.trim();
    let mut end = 0;
    let mut after_zwj = false;
    let mut regional = 0;
    for (i, c) in name.char_indices() {
        let cp = c as u32;
        let is_regional = (0x1F1E6..=0x1F1FF).contains(&cp);
        let ok = if end == 0 || after_zwj {
            is_pictographic(c)
        } else {
            matches!(cp, 0xFE0F | 0x20E3 | 0x200D | 0x1F3FB..=0x1F3FF | 0xE0020..=0xE007F) || (is_regional && regional == 1)
        };
        if !ok {
            break;
        }
        after_zwj = cp == 0x200D;
        if is_regional {
            regional += 1;
        }
        end = i + c.len_utf8();
    }
    let rest = name[end..].trim_start();
    if end == 0 || rest.is_empty() {
        return (None, name.to_string());
    }
    (Some(name[..end].to_string()), rest.to_string())
}

/// `base`, or `base (1)`, `base (2)` … whichever isn't taken.
fn unique_name(base: &str, taken: &[String]) -> String {
    if !taken.iter().any(|t| t == base) {
        return base.to_string();
    }
    (1..).map(|n| format!("{base} ({n})")).find(|c| !taken.contains(c)).unwrap_or_else(|| base.to_string())
}

/// RFC 3339 UTC string of local midnight starting `day`, comparable with the
/// serialised `completed_at` values.
fn day_start_utc(day: NaiveDate) -> String {
    crate::local_to_utc(day, NaiveTime::MIN).to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// True when every whitespace-separated word of `q` (lower-cased) occurs in
/// the task's title, note, steps or categories.
fn task_matches(t: &Task, words: &[String]) -> bool {
    let mut hay = t.title.to_lowercase();
    hay.push('\n');
    hay.push_str(&t.notes.to_lowercase());
    for s in &t.steps {
        hay.push('\n');
        hay.push_str(&s.title.to_lowercase());
    }
    for c in &t.categories {
        hay.push('\n');
        hay.push_str(&c.to_lowercase());
    }
    words.iter().all(|w| hay.contains(w.as_str()))
}

impl Store {
    // ------------------------------------------------------------ lists

    pub fn task_lists(&self) -> Result<Vec<TaskList>> {
        self.query_docs("ORDER BY json_extract(json,'$.is_default') DESC, sort_order, id", [])
    }

    pub fn task_list(&self, id: TaskListId) -> Result<Option<TaskList>> {
        self.get_doc(id)
    }

    /// The built-in "Tasks" list of an account (or of the primary account).
    pub fn default_task_list(&self, account_id: Option<AccountId>) -> Result<Option<TaskList>> {
        let lists = self.task_lists()?;
        let account_id = match account_id {
            Some(a) => Some(a),
            None => self.primary_account()?.map(|a| a.id),
        };
        Ok(lists
            .iter()
            .find(|l| l.is_default && Some(l.account_id) == account_id)
            .or_else(|| lists.iter().find(|l| l.is_default))
            .or_else(|| lists.first())
            .cloned())
    }

    /// Insert a list. A leading emoji in the name becomes the list's emoji.
    pub fn insert_task_list(&self, list: &mut TaskList) -> Result<TaskListId> {
        if list.emoji.is_none() {
            let (emoji, name) = split_list_emoji(&list.name);
            list.emoji = emoji;
            list.name = name;
        }
        if list.name.trim().is_empty() {
            list.name = "Untitled list".into();
        }
        if list.account_id == 0 {
            list.account_id = self.primary_account()?.map(|a| a.id).ok_or_else(|| Error::NotFound("account".into()))?;
        }
        if list.sort_order == 0 {
            list.sort_order = self.task_lists()?.iter().map(|l| l.sort_order).max().unwrap_or(0) + 1;
        }
        self.insert_doc(list)
    }

    /// "New list": a list named `name` (made unique, e.g. "Untitled list (1)")
    /// at the end of the sidebar, optionally inside a group.
    pub fn create_task_list(&self, name: &str, group: Option<&str>) -> Result<TaskListId> {
        let taken: Vec<String> = self.task_lists()?.into_iter().map(|l| l.name).collect();
        let base = if name.trim().is_empty() { "Untitled list" } else { name.trim() };
        let mut list = TaskList { name: unique_name(base, &taken), group: group.map(str::to_string), ..Default::default() };
        self.insert_task_list(&mut list)
    }

    pub fn update_task_list(&self, list: &TaskList) -> Result<()> {
        self.update_doc(list)
    }

    /// Rename a custom list. `name` may start with an emoji, which becomes the
    /// list icon. The default "Tasks" list can't be renamed.
    pub fn rename_task_list(&self, id: TaskListId, name: &str) -> Result<()> {
        let mut list: TaskList = self.get_doc(id)?.ok_or_else(|| Error::NotFound(format!("task list {id}")))?;
        if list.is_default {
            return Err(Error::Invalid("the Tasks list cannot be renamed".into()));
        }
        let (emoji, rest) = split_list_emoji(name);
        if rest.trim().is_empty() {
            return Err(Error::Invalid("list name is empty".into()));
        }
        list.emoji = emoji;
        list.name = rest;
        self.update_doc(&list)
    }

    /// Delete a list and its tasks. The default list cannot be deleted.
    pub fn delete_task_list(&self, id: TaskListId) -> Result<()> {
        let list: TaskList = self.get_doc(id)?.ok_or_else(|| Error::NotFound(format!("task list {id}")))?;
        if list.is_default {
            return Err(Error::Invalid("the Tasks list cannot be deleted".into()));
        }
        self.delete_doc::<TaskList>(id)
    }

    /// "Duplicate list": a copy named "{name} (1)" with copies of all its
    /// tasks, at the end of the same group.
    pub fn duplicate_task_list(&self, id: TaskListId) -> Result<TaskListId> {
        let src: TaskList = self.get_doc(id)?.ok_or_else(|| Error::NotFound(format!("task list {id}")))?;
        let taken: Vec<String> = self.task_lists()?.into_iter().map(|l| l.name).collect();
        let tasks = self.tasks_in_list(id)?;
        let max_order = self.task_lists()?.iter().map(|l| l.sort_order).max().unwrap_or(0);
        self.tx(|tx| {
            let mut copy = TaskList {
                id: 0,
                remote_id: None,
                is_default: false,
                name: unique_name(&src.name, &taken),
                sort_order: max_order + 1,
                ..src.clone()
            };
            let new_id = Self::insert_doc_in(tx, &mut copy)?;
            let now = Utc::now();
            for t in tasks {
                let mut c = Task { id: 0, list_id: new_id, remote_id: None, linked_message_id: None, created_at: now, updated_at: now, ..t };
                Self::insert_doc_in(tx, &mut c)?;
            }
            Ok(new_id)
        })
    }

    /// Put a list into a group (`None` = top level), at the end of it.
    pub fn set_task_list_group(&self, id: TaskListId, group: Option<&str>) -> Result<()> {
        let mut list: TaskList = self.get_doc(id)?.ok_or_else(|| Error::NotFound(format!("task list {id}")))?;
        if list.is_default {
            return Err(Error::Invalid("the Tasks list cannot be grouped".into()));
        }
        let group = group.map(str::trim).filter(|g| !g.is_empty()).map(str::to_string);
        if let Some(g) = &group {
            let mut groups = self.task_groups()?;
            if !groups.contains(g) {
                groups.push(g.clone());
                self.save_task_groups(&groups)?;
            }
        }
        list.group = group;
        list.sort_order = self.task_lists()?.iter().map(|l| l.sort_order).max().unwrap_or(0) + 1;
        self.update_doc(&list)
    }

    /// Drag-reorder in the sidebar: move list `id` into `group` and before
    /// list `before` (or to the end).
    pub fn move_task_list(&self, id: TaskListId, before: Option<TaskListId>, group: Option<&str>) -> Result<()> {
        let mut lists: Vec<TaskList> = self.task_lists()?.into_iter().filter(|l| !l.is_default).collect();
        let Some(pos) = lists.iter().position(|l| l.id == id) else {
            return Err(Error::NotFound(format!("task list {id}")));
        };
        let mut moving = lists.remove(pos);
        moving.group = group.map(str::to_string).filter(|g| !g.is_empty());
        if let Some(g) = &moving.group {
            let mut groups = self.task_groups()?;
            if !groups.contains(g) {
                groups.push(g.clone());
                self.save_task_groups(&groups)?;
            }
        }
        let at = before.and_then(|b| lists.iter().position(|l| l.id == b)).unwrap_or(lists.len());
        lists.insert(at, moving);
        self.tx(|tx| {
            for (i, l) in lists.iter_mut().enumerate() {
                let order = i as i64 + 1;
                if l.sort_order != order || l.id == id {
                    l.sort_order = order;
                    Self::update_doc_in(tx, l)?;
                }
            }
            Ok(())
        })
    }

    // ------------------------------------------------------------ groups

    fn save_task_groups(&self, groups: &[String]) -> Result<()> {
        self.kv_set(GROUPS_KEY, &serde_json::to_string(groups)?)
    }

    /// Group names in sidebar order: the stored order, then any group only
    /// known from a list.
    pub fn task_groups(&self) -> Result<Vec<String>> {
        let mut out: Vec<String> = self.kv_get(GROUPS_KEY)?.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default();
        for l in self.task_lists()? {
            if let Some(g) = l.group.filter(|g| !g.is_empty() && !out.contains(g)) {
                out.push(g);
            }
        }
        Ok(out)
    }

    /// "Create group": a new, empty group (named uniquely, "Untitled group").
    pub fn create_task_group(&self, name: &str) -> Result<String> {
        let mut groups = self.task_groups()?;
        let base = if name.trim().is_empty() { "Untitled group" } else { name.trim() };
        let name = unique_name(base, &groups);
        groups.push(name.clone());
        self.save_task_groups(&groups)?;
        Ok(name)
    }

    /// Rename a group (its lists follow). Returns the name actually used.
    pub fn rename_task_group(&self, old: &str, new: &str) -> Result<String> {
        let new = new.trim();
        if new.is_empty() || new == old {
            return Ok(old.to_string());
        }
        let mut groups = self.task_groups()?;
        let others: Vec<String> = groups.iter().filter(|g| *g != old).cloned().collect();
        let name = unique_name(new, &others);
        for g in &mut groups {
            if g == old {
                *g = name.clone();
            }
        }
        self.save_task_groups(&groups)?;
        self.tx(|tx| {
            let lists: Vec<TaskList> = Self::query_docs_in(tx, "", [])?;
            for mut l in lists {
                if l.group.as_deref() == Some(old) {
                    l.group = Some(name.clone());
                    Self::update_doc_in(tx, &l)?;
                }
            }
            Ok(())
        })?;
        Ok(name)
    }

    /// "Ungroup lists": the group goes away, its lists move to the top level.
    pub fn ungroup_task_group(&self, name: &str) -> Result<()> {
        let mut groups = self.task_groups()?;
        groups.retain(|g| g != name);
        self.save_task_groups(&groups)?;
        let max_order = self.task_lists()?.iter().map(|l| l.sort_order).max().unwrap_or(0);
        self.tx(|tx| {
            let lists: Vec<TaskList> = Self::query_docs_in(tx, "ORDER BY sort_order, id", [])?;
            let mut n = max_order;
            for mut l in lists {
                if l.group.as_deref() == Some(name) {
                    n += 1;
                    l.group = None;
                    l.sort_order = n;
                    Self::update_doc_in(tx, &l)?;
                }
            }
            Ok(())
        })
    }

    // ------------------------------------------------------------ queries

    pub fn task(&self, id: TaskId) -> Result<Option<Task>> {
        self.get_doc(id)
    }

    /// Tasks of one list: incomplete first by sort order, then completed
    /// (most recently completed first).
    pub fn tasks_in_list(&self, list_id: TaskListId) -> Result<Vec<Task>> {
        self.query_docs(
            "WHERE list_id = ?1 ORDER BY is_completed, CASE WHEN is_completed = 0 THEN sort_order END, \
             json_extract(json,'$.completed_at') DESC, id DESC",
            [list_id],
        )
    }

    /// Incomplete tasks of a list in manual order.
    fn open_tasks_in_list(&self, list_id: TaskListId) -> Result<Vec<Task>> {
        self.query_docs("WHERE list_id = ?1 AND is_completed = 0 ORDER BY sort_order, id DESC", [list_id])
    }

    /// Tasks shown in a smart list. `Flagged` returns nothing here: flagged
    /// email comes from [`Store::flagged_messages`]. My Day includes tasks due
    /// today (see [`Store::my_day_tasks`]).
    pub fn tasks_for(&self, list: SmartList, today: NaiveDate) -> Result<Vec<Task>> {
        self.smart_list_tasks(list, today, true, false)
    }

    /// Tasks of a smart list for the To Do view. `include_due_today` is the
    /// "Show tasks due today in My Day" setting. `include_completed` adds
    /// completed tasks to Important, Planned and Assigned to me (My Day and
    /// Tasks always include theirs).
    pub fn smart_list_tasks(
        &self,
        list: SmartList,
        today: NaiveDate,
        include_due_today: bool,
        include_completed: bool,
    ) -> Result<Vec<Task>> {
        let done = if include_completed { "" } else { " AND is_completed = 0" };
        match list {
            SmartList::MyDay => self.my_day_tasks(today, include_due_today),
            SmartList::Important => self.query_docs(
                &format!("WHERE is_important = 1{done} ORDER BY is_completed, due IS NULL, due, id DESC"),
                [],
            ),
            SmartList::Planned => self.query_docs(
                &format!(
                    "WHERE (due IS NOT NULL OR json_extract(json,'$.reminder') IS NOT NULL){done} \
                     ORDER BY is_completed, due IS NULL, due, json_extract(json,'$.reminder'), id DESC"
                ),
                [],
            ),
            SmartList::Assigned => self.query_docs(
                &format!("WHERE json_extract(json,'$.assigned_to_me') = 1{done} ORDER BY is_completed, due IS NULL, due, id DESC"),
                [],
            ),
            SmartList::Flagged => Ok(Vec::new()),
            SmartList::Tasks => match self.default_task_list(None)? {
                Some(l) => self.tasks_in_list(l.id),
                None => Ok(Vec::new()),
            },
        }
    }

    /// My Day: tasks added to My Day today, plus (when `include_due_today`)
    /// tasks due today that weren't removed from My Day today. Completed
    /// tasks stay until the day ends.
    pub fn my_day_tasks(&self, today: NaiveDate, include_due_today: bool) -> Result<Vec<Task>> {
        let today_s = today.to_string();
        if !include_due_today {
            return self.query_docs("WHERE my_day = ?1 ORDER BY is_completed, sort_order, id DESC", [today_s]);
        }
        self.query_docs(
            "WHERE my_day = ?1 OR (due = ?1 AND IFNULL(json_extract(json,'$.my_day_hidden'), '') <> ?1 \
               AND (is_completed = 0 OR IFNULL(json_extract(json,'$.completed_at'), '') >= ?2)) \
             ORDER BY is_completed, sort_order, id DESC",
            [today_s, day_start_utc(today)],
        )
    }

    /// Every incomplete task (the optional "All" smart list).
    pub fn all_open_tasks(&self) -> Result<Vec<Task>> {
        self.query_docs("WHERE is_completed = 0 ORDER BY list_id, sort_order, id DESC", [])
    }

    /// Every completed task, most recently completed first (the optional
    /// "Completed" smart list).
    pub fn all_completed_tasks(&self) -> Result<Vec<Task>> {
        self.query_docs("WHERE is_completed = 1 ORDER BY json_extract(json,'$.completed_at') DESC, id DESC", [])
    }

    /// Incomplete counts per smart list (Flagged counts flagged messages).
    pub fn smart_list_counts(&self, today: NaiveDate) -> Result<HashMap<SmartList, usize>> {
        self.smart_list_counts_with(today, true)
    }

    /// [`Store::smart_list_counts`] honouring the "Show tasks due today in My
    /// Day" setting.
    pub fn smart_list_counts_with(&self, today: NaiveDate, include_due_today: bool) -> Result<HashMap<SmartList, usize>> {
        let mut out = HashMap::new();
        for l in SmartList::ALL {
            let n = match l {
                SmartList::Flagged => self.flagged_messages()?.len(),
                _ => self.smart_list_tasks(l, today, include_due_today, false)?.iter().filter(|t| !t.is_completed).count(),
            };
            out.insert(l, n);
        }
        Ok(out)
    }

    /// Incomplete task count per list id.
    pub fn task_list_counts(&self) -> Result<HashMap<TaskListId, usize>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached("SELECT list_id, COUNT(*) FROM tasks WHERE is_completed = 0 GROUP BY list_id")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)? as usize)))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Search tasks across all lists: every word must appear in the title,
    /// note, a step or a category (case-insensitive). Incomplete tasks first.
    pub fn search_tasks(&self, q: &str) -> Result<Vec<Task>> {
        let words: Vec<String> = q.split_whitespace().map(str::to_lowercase).collect();
        if words.is_empty() {
            return Ok(Vec::new());
        }
        let all: Vec<Task> = self.query_docs("ORDER BY is_completed, list_id, sort_order, id DESC", [])?;
        Ok(all.into_iter().filter(|t| task_matches(t, &words)).collect())
    }

    /// My Day "Suggestions", in order: ("overdue", …), ("today", …),
    /// ("upcoming", due within 7 days), ("earlier", in My Day on a past day),
    /// ("recent", created within 2 days of `now`). Only incomplete tasks that
    /// aren't in today's My Day; each task appears in its first section.
    pub fn my_day_suggestions(
        &self,
        today: NaiveDate,
        include_due_today: bool,
        now: DateTime<Utc>,
    ) -> Result<Vec<(&'static str, Vec<Task>)>> {
        let open: Vec<Task> = self.query_docs("WHERE is_completed = 0 ORDER BY due IS NULL, due, id DESC", [])?;
        let in_my_day =
            |t: &Task| t.my_day == Some(today) || (include_due_today && t.due == Some(today) && t.my_day_hidden != Some(today));
        let recent = now - Duration::days(2);
        let mut sections: Vec<(&'static str, Vec<Task>)> =
            ["overdue", "today", "upcoming", "earlier", "recent"].into_iter().map(|k| (k, Vec::new())).collect();
        for t in open.into_iter().filter(|t| !in_my_day(t)) {
            let idx = match t.due {
                Some(d) if d < today => 0,
                Some(d) if d == today => 1,
                Some(d) if d <= today + Duration::days(7) => 2,
                _ if t.my_day.is_some_and(|d| d < today) => 3,
                _ if t.created_at >= recent => 4,
                _ => continue,
            };
            sections[idx].1.push(t);
        }
        Ok(sections)
    }

    /// Incomplete tasks whose reminder falls in `(after, until]`, earliest
    /// first: what a reminder check that last ran at `after` must fire now.
    pub fn task_reminders_between(&self, after: DateTime<Utc>, until: DateTime<Utc>) -> Result<Vec<Task>> {
        let mut due: Vec<Task> = self
            .query_docs::<Task>("WHERE is_completed = 0 AND json_extract(json,'$.reminder') IS NOT NULL", [])?
            .into_iter()
            .filter(|t| t.reminder.is_some_and(|r| r > after && r <= until))
            .collect();
        due.sort_by_key(|t| (t.reminder, t.id));
        Ok(due)
    }

    /// "Snooze": move a task's reminder to `until`.
    pub fn snooze_task_reminder(&self, id: TaskId, until: DateTime<Utc>) -> Result<()> {
        self.edit_task(id, |t| {
            t.reminder = Some(until);
            Ok(())
        })
    }

    /// Messages whose follow-up flag is marked complete, newest first (the
    /// "Completed" section of Flagged email).
    pub fn completed_flagged_messages(&self) -> Result<Vec<Message>> {
        self.query_docs("WHERE flag = 'Complete' ORDER BY received_at DESC LIMIT 200", [])
    }

    // ------------------------------------------------------------ edits

    pub fn insert_task(&self, task: &mut Task) -> Result<TaskId> {
        task.title = task.title.trim().to_string();
        if task.title.is_empty() {
            return Err(Error::Invalid("task title is empty".into()));
        }
        if task.list_id == 0 {
            task.list_id = self.default_task_list(None)?.map(|l| l.id).ok_or_else(|| Error::NotFound("task list".into()))?;
        }
        let now = Utc::now();
        task.created_at = now;
        task.updated_at = now;
        if task.sort_order == 0 {
            // New tasks go on top, like To Do.
            let min: i64 = self.tasks_in_list(task.list_id)?.iter().map(|t| t.sort_order).min().unwrap_or(0);
            task.sort_order = min - 1;
        }
        self.insert_doc(task)
    }

    /// Insert at the top or the bottom of the list's manual order (the
    /// "Add new tasks on top" setting).
    pub fn insert_task_at(&self, task: &mut Task, on_top: bool) -> Result<TaskId> {
        if task.list_id == 0 {
            task.list_id = self.default_task_list(None)?.map(|l| l.id).ok_or_else(|| Error::NotFound("task list".into()))?;
        }
        let orders: Vec<i64> = self.open_tasks_in_list(task.list_id)?.iter().map(|t| t.sort_order).collect();
        let order = if on_top { orders.iter().min().copied().unwrap_or(0) - 1 } else { orders.iter().max().copied().unwrap_or(0) + 1 };
        task.sort_order = if order == 0 { if on_top { -1 } else { 1 } } else { order };
        self.insert_task(task)
    }

    pub fn update_task(&self, task: &Task) -> Result<()> {
        let mut t = task.clone();
        t.updated_at = Utc::now();
        self.update_doc(&t)
    }

    pub fn delete_task(&self, id: TaskId) -> Result<()> {
        self.delete_doc::<Task>(id)
    }

    /// Load a task, apply `f`, save it.
    fn edit_task<R>(&self, id: TaskId, f: impl FnOnce(&mut Task) -> Result<R>) -> Result<R> {
        let mut t: Task = self.get_doc(id)?.ok_or_else(|| Error::NotFound(format!("task {id}")))?;
        let out = f(&mut t)?;
        self.update_task(&t)?;
        Ok(out)
    }

    /// Complete or reopen a task. Completing a repeating task creates the
    /// next occurrence (returned) the way To Do does: same list and position,
    /// steps unchecked, next due date, reminder shifted by the same amount.
    pub fn set_task_completed(&self, id: TaskId, completed: bool) -> Result<Option<TaskId>> {
        let mut t: Task = self.get_doc(id)?.ok_or_else(|| Error::NotFound(format!("task {id}")))?;
        if t.is_completed == completed {
            return Ok(None);
        }
        t.is_completed = completed;
        t.completed_at = completed.then(Utc::now);
        self.update_task(&t)?;
        if completed && let Some(rule) = t.recurrence.clone() {
            let today = crate::local_date(Utc::now());
            let base = t.due.unwrap_or(today);
            let due = next_due(&rule, base);
            let mut next = t.clone();
            next.id = 0;
            next.remote_id = None;
            next.is_completed = false;
            next.completed_at = None;
            next.due = Some(due);
            next.reminder = t.reminder.map(|r| r + (due - base));
            next.my_day = (due == today).then_some(today);
            next.my_day_hidden = None;
            next.my_day_at = None;
            if next.sort_order == 0 {
                next.sort_order = 1;
            }
            for s in &mut next.steps {
                s.is_completed = false;
            }
            return Ok(Some(self.insert_task(&mut next)?));
        }
        Ok(None)
    }

    pub fn set_task_important(&self, id: TaskId, important: bool) -> Result<()> {
        self.edit_task(id, |t| {
            t.is_important = important;
            Ok(())
        })
    }

    /// Add to My Day for `today` (`Some`) or take the task out of it (`None`).
    pub fn set_task_my_day(&self, id: TaskId, today: Option<NaiveDate>) -> Result<()> {
        self.edit_task(id, |t| {
            t.my_day = today;
            if today.is_some() {
                t.my_day_at = Some(Utc::now());
                t.my_day_hidden = None;
            }
            Ok(())
        })
    }

    /// Mail dropped on My Day's To Do tab (people-todo.md 4.4): one task per
    /// message, titled with its subject, linked to the message and added to
    /// My Day for `today`. A message that already has an open task gets that
    /// task added to My Day instead of a duplicate. Returns the tasks.
    pub fn tasks_from_messages(&self, ids: &[MessageId], today: NaiveDate) -> Result<Vec<TaskId>> {
        let open_links: HashMap<MessageId, TaskId> = self
            .query_docs::<Task>("WHERE is_completed = 0", [])?
            .into_iter()
            .filter_map(|t| t.linked_message_id.map(|m| (m, t.id)))
            .collect();
        let mut out = Vec::new();
        for &id in ids {
            if let Some(&existing) = open_links.get(&id) {
                if !out.contains(&existing) {
                    self.set_task_my_day(existing, Some(today))?;
                    out.push(existing);
                }
                continue;
            }
            let Some(m) = self.message(id)? else { continue };
            let subject = m.subject.trim();
            let mut t = Task {
                title: if subject.is_empty() { "(No subject)".into() } else { subject.to_string() },
                linked_message_id: Some(id),
                my_day: Some(today),
                my_day_at: Some(Utc::now()),
                ..Default::default()
            };
            out.push(self.insert_task(&mut t)?);
        }
        Ok(out)
    }

    /// "Remove from My Day": also hides a task that My Day showed because it
    /// is due today, for the rest of the day.
    pub fn remove_task_from_my_day(&self, id: TaskId, today: NaiveDate) -> Result<()> {
        self.edit_task(id, |t| {
            t.my_day = None;
            t.my_day_hidden = Some(today);
            Ok(())
        })
    }

    /// Save a task's note, stamping when it changed.
    pub fn set_task_notes(&self, id: TaskId, notes: &str) -> Result<()> {
        self.edit_task(id, |t| {
            if t.notes != notes {
                t.notes = notes.to_string();
                t.notes_updated_at = Some(Utc::now());
            }
            Ok(())
        })
    }

    /// "Move task to…": move tasks to the end of another list.
    pub fn move_tasks_to_list(&self, ids: &[TaskId], list_id: TaskListId) -> Result<()> {
        if self.task_list(list_id)?.is_none() {
            return Err(Error::NotFound(format!("task list {list_id}")));
        }
        let mut order = self.open_tasks_in_list(list_id)?.iter().map(|t| t.sort_order).max().unwrap_or(0);
        let now = Utc::now();
        self.tx(|tx| {
            for &id in ids {
                let Some(mut t) = Self::get_doc_in::<Task>(tx, id)? else { continue };
                if t.list_id == list_id {
                    continue;
                }
                order += 1;
                t.list_id = list_id;
                t.sort_order = order;
                t.updated_at = now;
                Self::update_doc_in(tx, &t)?;
            }
            Ok(())
        })
    }

    /// Manual reorder inside a list: put task `id` before `before` (another
    /// task of the same list), or at the end when `before` is `None`.
    pub fn reorder_task(&self, id: TaskId, before: Option<TaskId>) -> Result<()> {
        let t: Task = self.get_doc(id)?.ok_or_else(|| Error::NotFound(format!("task {id}")))?;
        let mut order = self.open_tasks_in_list(t.list_id)?;
        order.retain(|x| x.id != id);
        let at = before.and_then(|b| order.iter().position(|x| x.id == b)).unwrap_or(order.len());
        order.insert(at, t);
        self.tx(|tx| {
            for (i, x) in order.iter_mut().enumerate() {
                let n = i as i64 + 1;
                if x.sort_order != n {
                    x.sort_order = n;
                    Self::update_doc_in(tx, x)?;
                }
            }
            Ok(())
        })
    }

    /// Ctrl+Up / Ctrl+Down: move a task one place up or down in manual
    /// order. Returns false when it is already at that end.
    pub fn shift_task(&self, id: TaskId, up: bool) -> Result<bool> {
        let t: Task = self.get_doc(id)?.ok_or_else(|| Error::NotFound(format!("task {id}")))?;
        let order = self.open_tasks_in_list(t.list_id)?;
        let Some(i) = order.iter().position(|x| x.id == id) else { return Ok(false) };
        let before = if up {
            if i == 0 {
                return Ok(false);
            }
            Some(order[i - 1].id)
        } else {
            if i + 1 >= order.len() {
                return Ok(false);
            }
            order.get(i + 2).map(|x| x.id)
        };
        self.reorder_task(id, before)?;
        Ok(true)
    }

    // ------------------------------------------------------------ steps

    /// Append a step; returns its id.
    pub fn add_task_step(&self, task_id: TaskId, title: &str) -> Result<i64> {
        let title = title.trim().to_string();
        if title.is_empty() {
            return Err(Error::Invalid("step title is empty".into()));
        }
        self.edit_task(task_id, |t| {
            let id = t.steps.iter().map(|s| s.id).max().unwrap_or(0) + 1;
            t.steps.push(TaskStep { id, title, is_completed: false });
            Ok(id)
        })
    }

    /// Rename and/or check a step.
    pub fn update_task_step(&self, task_id: TaskId, step_id: i64, title: Option<&str>, completed: Option<bool>) -> Result<()> {
        self.edit_task(task_id, |t| {
            let s = t.steps.iter_mut().find(|s| s.id == step_id).ok_or_else(|| Error::NotFound(format!("step {step_id}")))?;
            if let Some(title) = title.map(str::trim).filter(|s| !s.is_empty()) {
                s.title = title.to_string();
            }
            if let Some(c) = completed {
                s.is_completed = c;
            }
            Ok(())
        })
    }

    pub fn delete_task_step(&self, task_id: TaskId, step_id: i64) -> Result<()> {
        self.edit_task(task_id, |t| {
            t.steps.retain(|s| s.id != step_id);
            Ok(())
        })
    }

    /// Drag-reorder of steps: put step `step_id` before step `before` (or at
    /// the end when `before` is `None`).
    pub fn reorder_task_step(&self, task_id: TaskId, step_id: i64, before: Option<i64>) -> Result<()> {
        self.edit_task(task_id, |t| {
            let i = t.steps.iter().position(|s| s.id == step_id).ok_or_else(|| Error::NotFound(format!("step {step_id}")))?;
            let step = t.steps.remove(i);
            let at = before.and_then(|b| t.steps.iter().position(|s| s.id == b)).unwrap_or(t.steps.len());
            t.steps.insert(at, step);
            Ok(())
        })
    }

    /// "Promote to task": the step becomes a task in the same list, right
    /// below its parent. Returns the new task's id.
    pub fn promote_task_step(&self, task_id: TaskId, step_id: i64) -> Result<TaskId> {
        let step = self.edit_task(task_id, |t| {
            let i = t.steps.iter().position(|s| s.id == step_id).ok_or_else(|| Error::NotFound(format!("step {step_id}")))?;
            Ok(t.steps.remove(i))
        })?;
        let parent: Task = self.get_doc(task_id)?.ok_or_else(|| Error::NotFound(format!("task {task_id}")))?;
        let mut t = Task {
            list_id: parent.list_id,
            title: step.title,
            is_completed: step.is_completed,
            completed_at: step.is_completed.then(Utc::now),
            ..Default::default()
        };
        let id = self.insert_task_at(&mut t, false)?;
        let order = self.open_tasks_in_list(parent.list_id)?;
        if let Some(i) = order.iter().position(|x| x.id == task_id) {
            let before = order.iter().skip(i + 1).find(|x| x.id != id).map(|x| x.id);
            self.reorder_task(id, before)?;
        }
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_util::store_with_account;

    fn day(m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, d).unwrap()
    }

    fn add(store: &Store, t: Task) -> TaskId {
        let mut t = t;
        store.insert_task_at(&mut t, false).unwrap()
    }

    fn titles(tasks: &[Task]) -> Vec<&str> {
        tasks.iter().map(|t| t.title.as_str()).collect()
    }

    #[test]
    fn dropped_mail_becomes_my_day_tasks_once() {
        let (store, account) = store_with_account();
        let inbox = store.folder_by_role(account, FolderRole::Inbox).unwrap().unwrap();
        let mut a = Message { account_id: account, folder_id: inbox.id, subject: " Budget review ".into(), ..Default::default() };
        let mut b = Message { account_id: account, folder_id: inbox.id, ..Default::default() };
        let a_id = store.insert_message(&mut a, None).unwrap();
        let b_id = store.insert_message(&mut b, None).unwrap();
        let today = day(10, 19);
        let made = store.tasks_from_messages(&[a_id, b_id, 999_999], today).unwrap();
        assert_eq!(made.len(), 2, "unknown ids are skipped");
        let first = store.task(made[0]).unwrap().unwrap();
        assert_eq!((first.title.as_str(), first.linked_message_id, first.my_day), ("Budget review", Some(a_id), Some(today)));
        assert_eq!(store.task(made[1]).unwrap().unwrap().title, "(No subject)");
        // Dropping the same message again reuses its open task.
        store.set_task_my_day(made[0], None).unwrap();
        assert_eq!(store.tasks_from_messages(&[a_id, a_id], day(10, 20)).unwrap(), vec![made[0]]);
        assert_eq!(store.task(made[0]).unwrap().unwrap().my_day, Some(day(10, 20)));
        // Once completed, a new drop makes a new task.
        store.set_task_completed(made[0], true).unwrap();
        assert_ne!(store.tasks_from_messages(&[a_id], today).unwrap(), vec![made[0]]);
    }

    #[test]
    fn smart_lists_and_repeat_completion() {
        let (store, _) = store_with_account();
        let today = day(10, 19);
        let mut a = Task { title: "Pay rent".into(), due: Some(today), is_important: true, ..Default::default() };
        a.recurrence = Some(Recurrence { freq: RecurrenceFreq::Monthly, interval: 1, ..Default::default() });
        let a_id = store.insert_task(&mut a).unwrap();
        let mut b = Task { title: "Read report".into(), my_day: Some(today), ..Default::default() };
        store.insert_task(&mut b).unwrap();

        // My Day shows the task added to it plus the one due today.
        assert_eq!(store.tasks_for(SmartList::MyDay, today).unwrap().len(), 2);
        assert_eq!(store.my_day_tasks(today, false).unwrap().len(), 1);
        assert_eq!(store.tasks_for(SmartList::Important, today).unwrap().len(), 1);
        assert_eq!(store.tasks_for(SmartList::Planned, today).unwrap().len(), 1);
        assert_eq!(store.tasks_for(SmartList::Tasks, today).unwrap()[0].title, "Read report");

        let next = store.set_task_completed(a_id, true).unwrap().expect("repeat creates next");
        let next = store.task(next).unwrap().unwrap();
        assert_eq!(next.due, Some(day(11, 19)));
        assert_eq!(next.sort_order, a.sort_order, "next occurrence keeps its place");
        assert_eq!(store.smart_list_counts(today).unwrap()[&SmartList::Planned], 1);
        assert!(store.set_task_completed(a_id, true).unwrap().is_none());

        let default = store.default_task_list(None).unwrap().unwrap();
        assert!(store.delete_task_list(default.id).is_err());
        assert!(store.rename_task_list(default.id, "Inbox").is_err());
    }

    #[test]
    fn weekly_multi_day_next_due() {
        use chrono::Weekday::*;
        let r = Recurrence { freq: RecurrenceFreq::Weekly, interval: 1, by_weekday: vec![Mon, Thu], ..Default::default() };
        let mon = day(10, 19);
        assert_eq!(next_due(&r, mon), day(10, 22));
        let weekdays = Recurrence { by_weekday: vec![Mon, Tue, Wed, Thu, Fri], ..r };
        assert_eq!(next_due(&weekdays, day(10, 23)), day(10, 26), "Friday rolls to Monday");
    }

    #[test]
    fn repeat_shifts_reminder_with_due_date() {
        let (store, _) = store_with_account();
        let due = day(10, 19);
        let reminder = crate::local_to_utc(due, NaiveTime::from_hms_opt(9, 0, 0).unwrap());
        let id = add(
            &store,
            Task {
                title: "Stand-up notes".into(),
                due: Some(due),
                reminder: Some(reminder),
                recurrence: Some(Recurrence { freq: RecurrenceFreq::Daily, interval: 1, ..Default::default() }),
                steps: vec![TaskStep { id: 1, title: "Yesterday".into(), is_completed: true }],
                ..Default::default()
            },
        );
        let next = store.task(store.set_task_completed(id, true).unwrap().unwrap()).unwrap().unwrap();
        assert_eq!(next.due, Some(day(10, 20)));
        assert_eq!(next.reminder, Some(reminder + Duration::days(1)));
        assert!(!next.steps[0].is_completed);
    }

    #[test]
    fn my_day_due_today_can_be_removed_and_completed_ones_stay() {
        let (store, _) = store_with_account();
        let today = crate::local_date(Utc::now());
        let due = add(&store, Task { title: "Due today".into(), due: Some(today), ..Default::default() });
        let other = add(&store, Task { title: "Due tomorrow".into(), due: Some(today + Duration::days(1)), ..Default::default() });
        assert_eq!(titles(&store.my_day_tasks(today, true).unwrap()), ["Due today"]);

        store.set_task_completed(due, true).unwrap();
        assert_eq!(titles(&store.my_day_tasks(today, true).unwrap()), ["Due today"], "completed today stays");
        store.set_task_completed(due, false).unwrap();

        store.remove_task_from_my_day(due, today).unwrap();
        assert!(store.my_day_tasks(today, true).unwrap().is_empty());
        assert!(store.my_day_tasks(today + Duration::days(1), true).unwrap().iter().any(|t| t.id == other));

        store.set_task_my_day(due, Some(today)).unwrap();
        let t = store.task(due).unwrap().unwrap();
        assert!(t.my_day_hidden.is_none() && t.my_day_at.is_some());
        assert_eq!(store.my_day_tasks(today, false).unwrap().len(), 1);
    }

    #[test]
    fn planned_includes_reminder_only_and_completed_on_request() {
        let (store, _) = store_with_account();
        let today = day(10, 19);
        add(&store, Task { title: "Reminder only".into(), reminder: Some(Utc::now()), ..Default::default() });
        let done = add(&store, Task { title: "Done".into(), due: Some(today), is_important: true, ..Default::default() });
        store.set_task_completed(done, true).unwrap();
        assert_eq!(titles(&store.smart_list_tasks(SmartList::Planned, today, true, false).unwrap()), ["Reminder only"]);
        assert_eq!(store.smart_list_tasks(SmartList::Planned, today, true, true).unwrap().len(), 2);
        assert!(store.smart_list_tasks(SmartList::Important, today, true, false).unwrap().is_empty());
        assert_eq!(store.smart_list_tasks(SmartList::Important, today, true, true).unwrap().len(), 1);
        assert_eq!(store.all_open_tasks().unwrap().len(), 1);
        assert_eq!(titles(&store.all_completed_tasks().unwrap()), ["Done"]);
    }

    #[test]
    fn manual_order_insert_reorder_shift_and_move() {
        let (store, _) = store_with_account();
        let list = store.default_task_list(None).unwrap().unwrap().id;
        let a = add(&store, Task { title: "A".into(), ..Default::default() });
        let b = add(&store, Task { title: "B".into(), ..Default::default() });
        let mut top = Task { title: "Top".into(), ..Default::default() };
        let top = store.insert_task_at(&mut top, true).unwrap();
        let open = |s: &Store| titles(&s.tasks_in_list(list).unwrap()).into_iter().map(String::from).collect::<Vec<_>>();
        assert_eq!(open(&store), ["Top", "A", "B"]);

        store.reorder_task(top, None).unwrap();
        assert_eq!(open(&store), ["A", "B", "Top"]);
        store.reorder_task(top, Some(a)).unwrap();
        assert_eq!(open(&store), ["Top", "A", "B"]);
        assert!(store.shift_task(a, false).unwrap());
        assert_eq!(open(&store), ["Top", "B", "A"]);
        assert!(!store.shift_task(a, false).unwrap());
        assert!(store.shift_task(a, true).unwrap());
        assert_eq!(open(&store), ["Top", "A", "B"]);

        let other = store.create_task_list("Errands", None).unwrap();
        let c = add(&store, Task { title: "C".into(), list_id: other, ..Default::default() });
        store.move_tasks_to_list(&[a, b], other).unwrap();
        assert_eq!(open(&store), ["Top"]);
        assert_eq!(titles(&store.tasks_in_list(other).unwrap()), ["C", "A", "B"]);
        assert!(store.move_tasks_to_list(&[c], 9999).is_err());
    }

    #[test]
    fn completed_tasks_sort_newest_first() {
        let (store, _) = store_with_account();
        let list = store.default_task_list(None).unwrap().unwrap().id;
        for (title, days_ago) in [("Old", 5), ("New", 1), ("Middle", 3)] {
            let id = add(&store, Task { title: title.into(), ..Default::default() });
            let mut t = store.task(id).unwrap().unwrap();
            t.is_completed = true;
            t.completed_at = Some(Utc::now() - Duration::days(days_ago));
            store.update_task(&t).unwrap();
        }
        add(&store, Task { title: "Open".into(), ..Default::default() });
        assert_eq!(titles(&store.tasks_in_list(list).unwrap()), ["Open", "New", "Middle", "Old"]);
    }

    #[test]
    fn steps_add_update_delete_promote() {
        let (store, _) = store_with_account();
        let parent = add(&store, Task { title: "Plan offsite".into(), ..Default::default() });
        let after = add(&store, Task { title: "After".into(), ..Default::default() });
        let s1 = store.add_task_step(parent, "Pick venue").unwrap();
        let s2 = store.add_task_step(parent, "  Send invites ").unwrap();
        assert!(store.add_task_step(parent, "   ").is_err());
        store.update_task_step(parent, s1, Some("Book venue"), Some(true)).unwrap();
        let t = store.task(parent).unwrap().unwrap();
        assert_eq!((t.steps[0].title.as_str(), t.steps[0].is_completed), ("Book venue", true));
        assert_eq!(t.steps[1].title, "Send invites");

        let promoted = store.promote_task_step(parent, s2).unwrap();
        let list = t.list_id;
        assert_eq!(titles(&store.tasks_in_list(list).unwrap()), ["Plan offsite", "Send invites", "After"]);
        assert_eq!(store.task(parent).unwrap().unwrap().steps.len(), 1);
        assert_ne!(promoted, after);

        store.delete_task_step(parent, s1).unwrap();
        assert!(store.task(parent).unwrap().unwrap().steps.is_empty());
        assert!(store.update_task_step(parent, s1, None, Some(false)).is_err());
    }

    #[test]
    fn steps_reorder() {
        let (store, _) = store_with_account();
        let id = add(&store, Task { title: "Trip".into(), ..Default::default() });
        let a = store.add_task_step(id, "A").unwrap();
        let b = store.add_task_step(id, "B").unwrap();
        let c = store.add_task_step(id, "C").unwrap();
        let order = |s: &Store| s.task(id).unwrap().unwrap().steps.iter().map(|s| s.title.clone()).collect::<Vec<_>>().join("");
        store.reorder_task_step(id, c, Some(a)).unwrap();
        assert_eq!(order(&store), "CAB");
        store.reorder_task_step(id, c, None).unwrap();
        assert_eq!(order(&store), "ABC");
        store.reorder_task_step(id, a, Some(c)).unwrap();
        assert_eq!(order(&store), "BAC");
        store.reorder_task_step(id, b, Some(b)).unwrap();
        assert_eq!(order(&store), "ACB", "dropping on itself moves it to the end");
        assert!(store.reorder_task_step(id, 99, None).is_err());
    }

    #[test]
    fn reminders_fire_once_in_their_window_and_snooze() {
        let (store, _) = store_with_account();
        let now = Utc::now();
        let soon = add(&store, Task { title: "Soon".into(), reminder: Some(now + Duration::minutes(1)), ..Default::default() });
        let first = add(&store, Task { title: "First".into(), reminder: Some(now + Duration::seconds(30)), ..Default::default() });
        add(&store, Task { title: "Later".into(), reminder: Some(now + Duration::hours(2)), ..Default::default() });
        add(&store, Task { title: "Past".into(), reminder: Some(now - Duration::minutes(5)), ..Default::default() });
        let done = add(&store, Task { title: "Done".into(), reminder: Some(now + Duration::seconds(10)), ..Default::default() });
        store.set_task_completed(done, true).unwrap();
        add(&store, Task { title: "No reminder".into(), ..Default::default() });

        let window = store.task_reminders_between(now, now + Duration::minutes(1)).unwrap();
        assert_eq!(titles(&window), ["First", "Soon"]);
        assert!(store.task_reminders_between(now + Duration::minutes(1), now + Duration::minutes(2)).unwrap().is_empty());

        store.snooze_task_reminder(soon, now + Duration::minutes(11)).unwrap();
        let later = store.task_reminders_between(now + Duration::minutes(10), now + Duration::minutes(12)).unwrap();
        assert_eq!(later.iter().map(|t| t.id).collect::<Vec<_>>(), [soon]);
        assert_ne!(first, soon);
    }

    #[test]
    fn list_names_emoji_and_duplicates() {
        assert_eq!(split_list_emoji("🛒 Groceries"), (Some("🛒".into()), "Groceries".into()));
        assert_eq!(split_list_emoji("👩🏽‍💻Work"), (Some("👩🏽‍💻".into()), "Work".into()));
        assert_eq!(split_list_emoji("🇮🇪 Trip"), (Some("🇮🇪".into()), "Trip".into()));
        assert_eq!(split_list_emoji("❤️ Family"), (Some("❤️".into()), "Family".into()));
        assert_eq!(split_list_emoji("Plain"), (None, "Plain".into()));
        assert_eq!(split_list_emoji("🎉"), (None, "🎉".into()));

        let (store, _) = store_with_account();
        let a = store.create_task_list("", None).unwrap();
        let b = store.create_task_list("", None).unwrap();
        assert_eq!(store.task_list(b).unwrap().unwrap().name, "Untitled list (1)");
        store.rename_task_list(a, "🎯 Goals").unwrap();
        let l = store.task_list(a).unwrap().unwrap();
        assert_eq!((l.emoji.as_deref(), l.name.as_str()), (Some("🎯"), "Goals"));
        assert!(store.rename_task_list(a, "  ").is_err());

        add(&store, Task { title: "Run 5k".into(), list_id: a, steps: vec![TaskStep { id: 1, title: "Shoes".into(), is_completed: false }], ..Default::default() });
        let copy = store.duplicate_task_list(a).unwrap();
        let c = store.task_list(copy).unwrap().unwrap();
        assert_eq!((c.name.as_str(), c.emoji.as_deref()), ("Goals (1)", Some("🎯")));
        let copied = store.tasks_in_list(copy).unwrap();
        assert_eq!(titles(&copied), ["Run 5k"]);
        assert_eq!(copied[0].steps.len(), 1);
        assert_eq!(store.tasks_in_list(a).unwrap().len(), 1, "original untouched");
    }

    #[test]
    fn groups_create_rename_move_ungroup() {
        let (store, _) = store_with_account();
        let g1 = store.create_task_group("").unwrap();
        let g2 = store.create_task_group("").unwrap();
        assert_eq!((g1.as_str(), g2.as_str()), ("Untitled group", "Untitled group (1)"));
        let a = store.create_task_list("A", Some(&g1)).unwrap();
        let b = store.create_task_list("B", None).unwrap();
        store.set_task_list_group(b, Some(&g1)).unwrap();
        let renamed = store.rename_task_group(&g1, "Work").unwrap();
        assert_eq!(renamed, "Work");
        assert_eq!(store.task_groups().unwrap(), ["Work", "Untitled group (1)"]);
        assert!(store.task_lists().unwrap().iter().filter(|l| !l.is_default).all(|l| l.group.as_deref() == Some("Work")));
        assert_eq!(store.rename_task_group("Untitled group (1)", "Work").unwrap(), "Work (1)");

        store.move_task_list(b, Some(a), None).unwrap();
        let order: Vec<(String, Option<String>)> =
            store.task_lists().unwrap().into_iter().filter(|l| !l.is_default).map(|l| (l.name, l.group)).collect();
        assert_eq!(order, [("B".to_string(), None), ("A".to_string(), Some("Work".to_string()))]);

        store.ungroup_task_group("Work").unwrap();
        assert_eq!(store.task_groups().unwrap(), ["Work (1)"]);
        assert!(store.task_list(a).unwrap().unwrap().group.is_none());
    }

    #[test]
    fn search_covers_title_notes_steps_and_categories() {
        let (store, _) = store_with_account();
        add(&store, Task { title: "Book flights #travel".into(), ..Default::default() });
        add(&store, Task { title: "Pack".into(), notes: "Passport and CHARGERS".into(), ..Default::default() });
        add(&store, Task { title: "Budget".into(), steps: vec![TaskStep { id: 1, title: "Hotel quote".into(), is_completed: false }], ..Default::default() });
        add(&store, Task { title: "Review".into(), categories: vec!["Blue category".into()], ..Default::default() });
        assert_eq!(titles(&store.search_tasks("#travel").unwrap()), ["Book flights #travel"]);
        assert_eq!(titles(&store.search_tasks("chargers passport").unwrap()), ["Pack"]);
        assert_eq!(titles(&store.search_tasks("hotel").unwrap()), ["Budget"]);
        assert_eq!(titles(&store.search_tasks("blue").unwrap()), ["Review"]);
        assert!(store.search_tasks("  ").unwrap().is_empty());
    }

    #[test]
    fn suggestions_sections() {
        let (store, _) = store_with_account();
        let today = crate::local_date(Utc::now());
        add(&store, Task { title: "Late".into(), due: Some(today - Duration::days(2)), ..Default::default() });
        add(&store, Task { title: "Today".into(), due: Some(today), ..Default::default() });
        add(&store, Task { title: "Soon".into(), due: Some(today + Duration::days(3)), ..Default::default() });
        add(&store, Task { title: "Yesterday's".into(), my_day: Some(today - Duration::days(1)), ..Default::default() });
        add(&store, Task { title: "In My Day".into(), my_day: Some(today), ..Default::default() });
        let far = add(&store, Task { title: "Far".into(), due: Some(today + Duration::days(30)), ..Default::default() });

        let later = Utc::now() + Duration::days(3);
        let s = store.my_day_suggestions(today, true, later).unwrap();
        let get = |k: &str| s.iter().find(|(n, _)| *n == k).map(|(_, v)| titles(v).join(",")).unwrap();
        assert_eq!(get("overdue"), "Late");
        assert_eq!(get("today"), "", "due today is already in My Day");
        assert_eq!(get("upcoming"), "Soon");
        assert_eq!(get("earlier"), "Yesterday's");
        assert_eq!(get("recent"), "");

        let s = store.my_day_suggestions(today, false, Utc::now()).unwrap();
        let get = |k: &str| s.iter().find(|(n, _)| *n == k).map(|(_, v)| v.iter().map(|t| t.id).collect::<Vec<_>>()).unwrap();
        assert_eq!(get("today").len(), 1);
        assert_eq!(get("recent"), [far], "recently created, no other reason");
    }
}
