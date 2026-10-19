//! Microsoft To Do over Graph (§4.11): task lists and tasks (with steps as
//! checklist items). Lists are re-read in full each pass; the built-in
//! "Tasks" list maps onto the account's default list, and "Flagged email"
//! is skipped (flagged mail comes from the messages themselves).

use std::collections::{HashMap, HashSet};

use axentra_core::{Account, Store, Task, TaskId, TaskList, TaskListId};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::client::{Graph, seg};
use super::map::{task_from_graph, task_list_from_graph, task_to_graph};
use crate::diff::{ItemOp, KIND_TASK, KIND_TASK_LIST, Shadow, item_op, task_fingerprint, task_list_fingerprint};
use crate::error::{SyncError, SyncResult};
use crate::notify::Notifier;

/// Extra state kept for a task: remote ids of its checklist items.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct TaskExtra {
    steps: Vec<String>,
}

pub struct GraphTodo<'a> {
    pub graph: &'a Graph,
    pub store: &'a Store,
    pub account: &'a Account,
    pub notifier: &'a Notifier,
}

impl GraphTodo<'_> {
    pub async fn sync(&self) -> SyncResult<usize> {
        let lists = self.sync_lists().await?;
        let mut changed = 0;
        for (lid, rid) in &lists {
            changed += self.sync_tasks(*lid, rid).await?;
        }
        changed += self.push_lists().await?;
        if changed > 0 {
            self.notifier.bump();
        }
        Ok(changed)
    }

    /// Push local list and task changes without reading the server.
    pub async fn push_only(&self) -> SyncResult<usize> {
        let mut changed = self.push_lists().await?;
        for (lid, sh) in self.store.sync_list::<Shadow>(self.account.id, KIND_TASK_LIST, None)? {
            let locals: HashMap<TaskId, Task> = self.store.tasks_in_list(lid)?.into_iter().map(|t| (t.id, t)).collect();
            changed += self.push_tasks(lid, &sh.remote_id, &locals).await?;
        }
        if changed > 0 {
            self.notifier.bump();
        }
        Ok(changed)
    }

    async fn sync_lists(&self) -> SyncResult<Vec<(TaskListId, String)>> {
        let account_id = self.account.id;
        let remote = self.graph.get_all("/me/todo/lists").await?;
        let locals: Vec<TaskList> = self.store.task_lists()?.into_iter().filter(|l| l.account_id == account_id).collect();
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for v in &remote {
            let Some((fresh, wellknown)) = task_list_from_graph(v, account_id) else { continue };
            if wellknown == "flaggedEmails" {
                continue;
            }
            let rid = fresh.remote_id.clone().unwrap_or_default();
            seen.insert(rid.clone());
            let existing = locals
                .iter()
                .find(|l| l.remote_id.as_deref() == Some(rid.as_str()))
                .or_else(|| fresh.is_default.then(|| locals.iter().find(|l| l.is_default && l.remote_id.is_none())).flatten())
                .cloned();
            let id = match existing {
                Some(mut l) => {
                    let pending = self
                        .store
                        .sync_get::<Shadow>(account_id, KIND_TASK_LIST, l.id)?
                        .is_some_and(|sh| task_list_fingerprint(&l) != sh.state);
                    if l.remote_id.as_deref() != Some(rid.as_str()) || (!pending && !l.is_default && l.name != fresh.name) {
                        l.remote_id = Some(rid.clone());
                        if !pending && !l.is_default {
                            l.name = fresh.name.clone();
                        }
                        self.store.update_task_list(&l)?;
                    }
                    l.id
                }
                None => {
                    let mut l = fresh.clone();
                    l.is_default = false;
                    self.store.insert_task_list(&mut l)?
                }
            };
            let stored = self.store.task_list(id)?.unwrap_or(fresh);
            let sh = Shadow { remote_id: rid.clone(), extra: json!({"wellknown": wellknown}), state: task_list_fingerprint(&stored), ..Default::default() };
            self.store.sync_put(account_id, KIND_TASK_LIST, id, &rid, "", &sh)?;
            out.push((id, rid));
        }
        for l in locals.iter().filter(|l| l.remote_id.as_ref().is_some_and(|r| !seen.contains(r))) {
            if !l.is_default {
                self.store.delete_task_list(l.id)?;
            }
            self.store.sync_remove(account_id, KIND_TASK_LIST, l.id)?;
        }
        Ok(out)
    }

    /// New, renamed and deleted lists made locally.
    async fn push_lists(&self) -> SyncResult<usize> {
        let account_id = self.account.id;
        let locals: HashMap<TaskListId, TaskList> =
            self.store.task_lists()?.into_iter().filter(|l| l.account_id == account_id).map(|l| (l.id, l)).collect();
        let shadows: HashMap<TaskListId, Shadow> = self.store.sync_list::<Shadow>(account_id, KIND_TASK_LIST, None)?.into_iter().collect();
        let mut pushed = 0;
        let mut ids: Vec<TaskListId> = locals.keys().chain(shadows.keys()).copied().collect::<HashSet<_>>().into_iter().collect();
        ids.sort_unstable();
        for id in ids {
            let local = locals.get(&id);
            let shadow = shadows.get(&id);
            let fp = local.map(task_list_fingerprint);
            match item_op(fp.as_ref(), shadow) {
                ItemOp::None => continue,
                ItemOp::Create => {
                    let l = local.expect("create");
                    let created = self.graph.post("/me/todo/lists", json!({"displayName": l.name})).await?;
                    let rid = created["id"].as_str().unwrap_or_default().to_string();
                    let mut l2 = l.clone();
                    l2.remote_id = Some(rid.clone());
                    self.store.update_task_list(&l2)?;
                    let sh = Shadow { remote_id: rid.clone(), state: fp.unwrap_or_default(), extra: json!({"wellknown": "none"}), ..Default::default() };
                    self.store.sync_put(account_id, KIND_TASK_LIST, id, &rid, "", &sh)?;
                    // Its tasks go up with the next task pass.
                    self.sync_tasks(id, &rid).await?;
                }
                ItemOp::Update => {
                    let (l, sh) = (local.expect("update"), shadow.expect("update"));
                    if sh.extra["wellknown"].as_str().unwrap_or("none") == "none" {
                        self.graph.patch(&format!("/me/todo/lists/{}", seg(&sh.remote_id)), json!({"displayName": l.name})).await?;
                    }
                    let sh = Shadow { state: fp.unwrap_or_default(), ..sh.clone() };
                    self.store.sync_put(account_id, KIND_TASK_LIST, id, &sh.remote_id, "", &sh)?;
                }
                ItemOp::Delete => {
                    let sh = shadow.expect("delete");
                    if sh.extra["wellknown"].as_str().unwrap_or("none") == "none" {
                        self.graph.delete(&format!("/me/todo/lists/{}", seg(&sh.remote_id))).await?;
                    }
                    self.store.sync_remove(account_id, KIND_TASK_LIST, id)?;
                }
            }
            pushed += 1;
        }
        Ok(pushed)
    }

    async fn sync_tasks(&self, list_id: TaskListId, remote: &str) -> SyncResult<usize> {
        let account_id = self.account.id;
        let base = format!("/me/todo/lists/{}/tasks", seg(remote));
        let items = self.graph.get_all(&format!("{base}?$top=100&$expand=checklistItems")).await?;
        let mut locals: HashMap<TaskId, Task> = self.store.tasks_in_list(list_id)?.into_iter().map(|t| (t.id, t)).collect();
        let mut seen = HashSet::new();
        let mut changed = 0;
        for v in &items {
            let Some(rid) = v["id"].as_str() else { continue };
            seen.insert(rid.to_string());
            let etag = v["@odata.etag"].as_str().or(v["lastModifiedDateTime"].as_str()).map(str::to_string);
            let step_ids: Vec<String> = v["checklistItems"].as_array().map(|a| a.iter().filter_map(|c| c["id"].as_str().map(str::to_string)).collect()).unwrap_or_default();
            let extra = serde_json::to_value(TaskExtra { steps: step_ids }).unwrap_or_default();
            match self.store.sync_find::<Shadow>(account_id, KIND_TASK, rid)? {
                Some((lid, mut sh)) => {
                    let Some(local) = locals.get(&lid) else { continue };
                    if sh.etag == etag {
                        continue;
                    }
                    if task_fingerprint(local) != sh.state || local.list_id != list_id {
                        sh.etag = etag;
                        self.store.sync_put(account_id, KIND_TASK, lid, rid, &sh.container.clone(), &sh)?;
                        continue;
                    }
                    let Some(t) = task_from_graph(v, list_id, Some(local)) else { continue };
                    self.store.update_task(&t)?;
                    let stored = self.store.task(lid)?.unwrap_or(t);
                    let sh = Shadow { state: task_fingerprint(&stored), etag, extra, container: remote.to_string(), ..sh };
                    self.store.sync_put(account_id, KIND_TASK, lid, rid, remote, &sh)?;
                    locals.insert(lid, stored);
                    changed += 1;
                }
                None => {
                    let Some(mut t) = task_from_graph(v, list_id, None) else { continue };
                    if t.title.trim().is_empty() {
                        continue;
                    }
                    let completed = t.is_completed;
                    let completed_at = t.completed_at;
                    t.is_completed = false;
                    let lid = self.store.insert_task(&mut t)?;
                    if completed && let Some(mut done) = self.store.task(lid)? {
                        done.is_completed = true;
                        done.completed_at = completed_at;
                        self.store.update_task(&done)?;
                    }
                    let stored = self.store.task(lid)?.unwrap_or(t);
                    let sh = Shadow { remote_id: rid.to_string(), container: remote.to_string(), etag, extra, state: task_fingerprint(&stored), ..Default::default() };
                    self.store.sync_put(account_id, KIND_TASK, lid, rid, remote, &sh)?;
                    locals.insert(lid, stored);
                    changed += 1;
                }
            }
        }
        for (lid, sh) in self.store.sync_list::<Shadow>(account_id, KIND_TASK, Some(remote))? {
            if !seen.contains(&sh.remote_id) && locals.contains_key(&lid) && locals[&lid].list_id == list_id {
                self.store.delete_task(lid)?;
                self.store.sync_remove(account_id, KIND_TASK, lid)?;
                locals.remove(&lid);
                changed += 1;
            }
        }
        changed += self.push_tasks(list_id, remote, &locals).await?;
        Ok(changed)
    }

    async fn push_tasks(&self, list_id: TaskListId, remote: &str, locals: &HashMap<TaskId, Task>) -> SyncResult<usize> {
        let account_id = self.account.id;
        let base = format!("/me/todo/lists/{}/tasks", seg(remote));
        let shadows: HashMap<TaskId, Shadow> = self.store.sync_list::<Shadow>(account_id, KIND_TASK, Some(remote))?.into_iter().collect();
        let mut ids: Vec<TaskId> = locals.keys().chain(shadows.keys()).copied().collect::<HashSet<_>>().into_iter().collect();
        ids.sort_unstable();
        let mut pushed = 0;
        for id in ids {
            let local = locals.get(&id).filter(|t| t.list_id == list_id);
            let shadow = shadows.get(&id);
            let fp = local.map(task_fingerprint);
            let op = item_op(fp.as_ref(), shadow);
            let result: SyncResult<Option<Shadow>> = async {
                match op {
                    ItemOp::None => Ok(shadow.cloned()),
                    ItemOp::Create => {
                        let t = local.expect("create");
                        // Moved here from another list: To Do can't move tasks, so
                        // delete the old copy and create a new one.
                        if let Some(old) = self.store.sync_get::<Shadow>(account_id, KIND_TASK, t.id)?
                            && old.container != remote
                        {
                            self.graph.delete(&format!("/me/todo/lists/{}/tasks/{}", seg(&old.container), seg(&old.remote_id))).await?;
                        }
                        let created = self.graph.post(&base, task_to_graph(t)).await?;
                        let rid = created["id"].as_str().unwrap_or_default().to_string();
                        let steps = self.replace_steps(&format!("{base}/{}", seg(&rid)), t, &[]).await?;
                        Ok(Some(Shadow {
                            remote_id: rid,
                            container: remote.to_string(),
                            etag: created["@odata.etag"].as_str().map(str::to_string),
                            state: fp.clone().unwrap_or_default(),
                            extra: serde_json::to_value(TaskExtra { steps }).unwrap_or_default(),
                            ..Default::default()
                        }))
                    }
                    ItemOp::Update => {
                        let (t, sh) = (local.expect("update"), shadow.expect("update"));
                        let path = format!("{base}/{}", seg(&sh.remote_id));
                        let updated = self.graph.patch(&path, task_to_graph(t)).await?;
                        let mut extra: TaskExtra = serde_json::from_value(sh.extra.clone()).unwrap_or_default();
                        let old: serde_json::Value = sh.state["steps"].clone();
                        if fp.as_ref().map(|f| f["steps"].clone()) != Some(old) {
                            extra.steps = self.replace_steps(&path, t, &extra.steps).await?;
                        }
                        Ok(Some(Shadow {
                            state: fp.clone().unwrap_or_default(),
                            etag: updated["@odata.etag"].as_str().map(str::to_string).or(sh.etag.clone()),
                            extra: serde_json::to_value(extra).unwrap_or_default(),
                            ..sh.clone()
                        }))
                    }
                    ItemOp::Delete => {
                        if let Some(sh) = shadow {
                            self.graph.delete(&format!("{base}/{}", seg(&sh.remote_id))).await?;
                        }
                        Ok(None)
                    }
                }
            }
            .await;
            match (op, result) {
                (ItemOp::None, _) => continue,
                (_, Ok(Some(sh))) => self.store.sync_put(account_id, KIND_TASK, id, &sh.remote_id.clone(), remote, &sh)?,
                (_, Ok(None)) | (_, Err(SyncError::NotFound)) => self.store.sync_remove(account_id, KIND_TASK, id)?,
                (_, Err(e)) => return Err(e),
            }
            pushed += 1;
        }
        Ok(pushed)
    }

    /// Steps become checklist items; on change, replace them all.
    async fn replace_steps(&self, task_path: &str, t: &Task, old: &[String]) -> SyncResult<Vec<String>> {
        for id in old {
            self.graph.delete(&format!("{task_path}/checklistItems/{}", seg(id))).await?;
        }
        let mut out = Vec::new();
        for s in &t.steps {
            let created = self.graph.post(&format!("{task_path}/checklistItems"), json!({"displayName": s.title, "isChecked": s.is_completed})).await?;
            if let Some(id) = created["id"].as_str() {
                out.push(id.to_string());
            }
        }
        Ok(out)
    }
}
