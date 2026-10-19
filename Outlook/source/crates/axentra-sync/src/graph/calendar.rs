//! Calendars and events over Graph (§4.9). Each sync re-reads the rolling
//! window with `calendarView` (which works for every calendar, unlike
//! calendarView delta) and compares `changeKey`s. Series are stored as one
//! master with a local recurrence rule plus exception rows, so the Calendar
//! module keeps its "this event / the series" semantics; occurrences missing
//! on the server become exdates.

use std::collections::{HashMap, HashSet};

use axentra_core::recurrence::occurrence_dates;
use axentra_core::{Account, Calendar, CalendarId, Event, ResponseStatus, Store, local_date};
use chrono::{Duration, NaiveDate, NaiveTime, Utc};
use serde_json::json;

use super::client::{Graph, seg};
use super::map::{EventType, GraphEvent, calendar_from_graph, event_from_graph, event_to_graph};
use crate::diff::{EventExtra, EventOp, KIND_CALENDAR, KIND_EVENT, Shadow, event_fingerprint, event_ops};
use crate::error::{SyncError, SyncResult};
use crate::notify::Notifier;

/// Rolling window around today (estimate, §4.9).
const WINDOW_PAST_DAYS: i64 = 90;
const WINDOW_FUTURE_DAYS: i64 = 365;
const EVENT_SELECT: &str = "id,type,seriesMasterId,originalStart,subject,body,start,end,isAllDay,location,showAs,sensitivity,\
isReminderOn,reminderMinutesBeforeStart,organizer,isOrganizer,responseStatus,attendees,isOnlineMeeting,onlineMeeting,\
categories,isCancelled,originalStartTimeZone,changeKey";

pub struct GraphCalendar<'a> {
    pub graph: &'a Graph,
    pub store: &'a Store,
    pub account: &'a Account,
    pub notifier: &'a Notifier,
}

fn shadow_extra(s: &Shadow) -> EventExtra {
    serde_json::from_value(s.extra.clone()).unwrap_or_default()
}

impl GraphCalendar<'_> {
    /// Mirror the calendar list. Returns (local id, remote id) pairs.
    pub async fn sync_calendars(&self) -> SyncResult<Vec<(CalendarId, String)>> {
        let account_id = self.account.id;
        let remote = self.graph.get_all("/me/calendars?$select=id,name,color,hexColor,isDefaultCalendar,canEdit,owner,changeKey").await?;
        let mut locals: Vec<Calendar> = self.store.calendars()?.into_iter().filter(|c| c.account_id == account_id).collect();
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for (i, v) in remote.iter().enumerate() {
            let fresh = calendar_from_graph(v, account_id, i);
            let Some(rid) = fresh.remote_id.clone() else { continue };
            seen.insert(rid.clone());
            let existing = locals
                .iter_mut()
                .find(|c| c.remote_id.as_deref() == Some(rid.as_str()))
                .map(|c| c.id)
                .or_else(|| fresh.is_default.then(|| locals.iter().find(|c| c.is_default && c.remote_id.is_none()).map(|c| c.id)).flatten());
            let id = match existing.and_then(|id| locals.iter_mut().find(|c| c.id == id)) {
                Some(c) => {
                    // The user's visibility choice and colour stay local.
                    let changed = c.remote_id.as_deref() != Some(rid.as_str()) || c.name != fresh.name || c.can_edit != fresh.can_edit;
                    c.remote_id = Some(rid.clone());
                    c.name = fresh.name.clone();
                    c.can_edit = fresh.can_edit;
                    c.is_default = fresh.is_default;
                    if changed {
                        self.store.update_calendar(c)?;
                    }
                    c.id
                }
                None => {
                    let mut c = fresh;
                    self.store.add_calendar(&mut c)?
                }
            };
            self.store.sync_put(account_id, KIND_CALENDAR, id, &rid, "", &Shadow { remote_id: rid.clone(), ..Default::default() })?;
            out.push((id, rid));
        }
        for c in locals.iter().filter(|c| c.remote_id.as_ref().is_some_and(|r| !seen.contains(r))) {
            self.store.delete_calendar(c.id)?;
            self.store.sync_remove(account_id, KIND_CALENDAR, c.id)?;
        }
        self.notifier.bump();
        Ok(out)
    }

    /// Push local event changes of every synced calendar (no reads).
    pub async fn push_only(&self) -> SyncResult<usize> {
        let mut changed = 0;
        for (cid, sh) in self.store.sync_list::<Shadow>(self.account.id, KIND_CALENDAR, None)? {
            changed += self.push(cid, &sh.remote_id).await?;
        }
        if changed > 0 {
            self.notifier.bump();
        }
        Ok(changed)
    }

    /// Pull the window of one calendar, then push local changes.
    pub async fn sync_calendar(&self, calendar_id: CalendarId, remote: &str) -> SyncResult<usize> {
        let mut changed = self.pull(calendar_id, remote).await?;
        changed += self.push(calendar_id, remote).await?;
        if changed > 0 {
            self.notifier.bump();
        }
        Ok(changed)
    }

    async fn pull(&self, calendar_id: CalendarId, remote: &str) -> SyncResult<usize> {
        let account_id = self.account.id;
        let today = Utc::now().date_naive();
        let from = today - Duration::days(WINDOW_PAST_DAYS);
        let to = today + Duration::days(WINDOW_FUTURE_DAYS);
        let url = format!(
            "/me/calendars/{}/calendarView?startDateTime={from}T00:00:00Z&endDateTime={to}T00:00:00Z&$top=200&$select={EVENT_SELECT}",
            seg(remote)
        );
        let instances: Vec<GraphEvent> = self.graph.get_all(&url).await?.iter().filter_map(|v| event_from_graph(v, calendar_id)).collect();

        // Group series instances by master.
        let mut singles: Vec<GraphEvent> = Vec::new();
        let mut series: HashMap<String, Vec<GraphEvent>> = HashMap::new();
        for g in instances {
            match (&g.kind, &g.series_master_id) {
                (EventType::Occurrence | EventType::Exception, Some(m)) => series.entry(m.clone()).or_default().push(g),
                _ => singles.push(g),
            }
        }
        let mut seen: HashSet<String> = HashSet::new();
        let mut changed = 0;
        for g in singles {
            seen.insert(g.id.clone());
            changed += self.upsert(calendar_id, remote, g, EventExtra::default()).await? as usize;
        }
        for (master_id, inst) in series {
            let master = match self.graph.get(&format!("/me/events/{}?$select={EVENT_SELECT},recurrence", seg(&master_id))).await {
                Ok(v) => event_from_graph(&v, calendar_id),
                Err(SyncError::NotFound) => None,
                Err(e) => return Err(e),
            };
            match master {
                Some(m) if !m.recurrence_unsupported && m.event.recurrence.is_some() => {
                    let rule = m.event.recurrence.clone().expect("checked");
                    let start = local_date(m.event.start);
                    let week_start = self.store.settings().week_start;
                    let present: HashSet<NaiveDate> = inst
                        .iter()
                        .filter(|i| i.kind == EventType::Occurrence)
                        .filter_map(|i| i.original_start.or(Some(i.event.start)).map(local_date))
                        .collect();
                    let expected = occurrence_dates(&rule, start, week_start, &[], from.max(start), to - Duration::days(1));
                    let exdates: Vec<NaiveDate> = expected.into_iter().filter(|d| !present.contains(d)).collect();
                    let mut master_event = m;
                    master_event.event.exdates = exdates.clone();
                    seen.insert(master_event.id.clone());
                    changed += self.upsert(calendar_id, remote, master_event, EventExtra { exdates, ..Default::default() }).await? as usize;
                    for e in inst.into_iter().filter(|i| i.kind == EventType::Exception) {
                        seen.insert(e.id.clone());
                        let extra = EventExtra { series_master_id: Some(master_id.clone()), ..Default::default() };
                        changed += self.upsert(calendar_id, remote, e, extra).await? as usize;
                    }
                }
                // A pattern we can't express locally: keep the instances as single events.
                _ => {
                    for mut e in inst {
                        seen.insert(e.id.clone());
                        e.event.recurrence = None;
                        let extra = EventExtra { series_master_id: Some(master_id.clone()), ..Default::default() };
                        changed += self.upsert(calendar_id, remote, e, extra).await? as usize;
                    }
                }
            }
        }
        // Gone from the server (or outside the window now).
        for (lid, sh) in self.store.sync_list::<Shadow>(account_id, KIND_EVENT, Some(remote))? {
            if !seen.contains(&sh.remote_id) {
                self.store.delete_event(lid)?;
                self.store.sync_remove(account_id, KIND_EVENT, lid)?;
                changed += 1;
            }
        }
        Ok(changed)
    }

    /// Insert or refresh one event. Local edits not yet pushed win.
    async fn upsert(&self, calendar_id: CalendarId, remote: &str, g: GraphEvent, mut extra: EventExtra) -> SyncResult<bool> {
        let account_id = self.account.id;
        extra.response = g.event.my_response;
        match self.store.sync_find::<Shadow>(account_id, KIND_EVENT, &g.id)? {
            Some((lid, mut sh)) => {
                let Some(local) = self.store.event(lid)? else { return Ok(false) };
                let pending = event_fingerprint(&local) != sh.state || local.my_response != shadow_extra(&sh).response;
                if sh.etag == g.change_key && local.exdates == extra.exdates {
                    return Ok(false);
                }
                if pending {
                    // Pushed next; just remember the server version.
                    sh.etag = g.change_key.clone();
                    self.store.sync_put(account_id, KIND_EVENT, lid, &g.id, remote, &sh)?;
                    return Ok(false);
                }
                let mut e = g.event;
                e.id = lid;
                e.calendar_id = local.calendar_id;
                e.created_at = local.created_at;
                self.store.update_event(&e)?;
                let stored = self.store.event(lid)?.unwrap_or(e);
                sh.state = event_fingerprint(&stored);
                sh.etag = g.change_key;
                sh.extra = serde_json::to_value(&extra).unwrap_or_default();
                self.store.sync_put(account_id, KIND_EVENT, lid, &sh.remote_id.clone(), remote, &sh)?;
                Ok(true)
            }
            None => {
                let mut e = g.event;
                e.id = 0;
                e.calendar_id = calendar_id;
                let lid = self.store.insert_event(&mut e)?;
                let stored = self.store.event(lid)?.unwrap_or(e);
                let sh = Shadow {
                    remote_id: g.id.clone(),
                    container: remote.to_string(),
                    etag: g.change_key,
                    state: event_fingerprint(&stored),
                    extra: serde_json::to_value(&extra).unwrap_or_default(),
                    ..Default::default()
                };
                self.store.sync_put(account_id, KIND_EVENT, lid, &g.id, remote, &sh)?;
                Ok(true)
            }
        }
    }

    async fn push(&self, calendar_id: CalendarId, remote: &str) -> SyncResult<usize> {
        let account_id = self.account.id;
        let locals: HashMap<i64, Event> = self.store.calendar_events(calendar_id)?.into_iter().map(|e| (e.id, e)).collect();
        let shadows: HashMap<i64, Shadow> = self.store.sync_list::<Shadow>(account_id, KIND_EVENT, Some(remote))?.into_iter().collect();
        let mut ids: Vec<i64> = locals.keys().chain(shadows.keys()).copied().collect::<HashSet<_>>().into_iter().collect();
        ids.sort_unstable();
        let mut pushed = 0;
        for id in ids {
            let local = locals.get(&id);
            // Each op continues from the shadow the previous one produced
            // (an update followed by a response must keep both).
            let mut current = shadows.get(&id).cloned();
            for op in event_ops(local, current.as_ref()) {
                let result = self.apply(op, local, current.as_ref(), remote).await;
                match result {
                    Ok(Some(new_shadow)) => {
                        self.store.sync_put(account_id, KIND_EVENT, id, &new_shadow.remote_id, remote, &new_shadow)?;
                        current = Some(new_shadow);
                    }
                    Ok(None) => {
                        self.store.sync_remove(account_id, KIND_EVENT, id)?;
                        current = None;
                    }
                    Err(SyncError::NotFound) => {
                        self.store.sync_remove(account_id, KIND_EVENT, id)?;
                        if local.is_some() {
                            self.store.delete_event(id)?;
                        }
                        pushed += 1;
                        break;
                    }
                    Err(e) => return Err(e),
                }
                pushed += 1;
            }
        }
        Ok(pushed)
    }

    /// Run one push operation; returns the new shadow (`None` = forget it).
    async fn apply(&self, op: EventOp, local: Option<&Event>, shadow: Option<&Shadow>, remote: &str) -> SyncResult<Option<Shadow>> {
        let settled = |e: &Event, sh: Shadow, response: ResponseStatus| {
            let mut extra = shadow_extra(&sh);
            extra.response = response;
            extra.exdates = e.exdates.clone();
            Shadow { state: event_fingerprint(e), extra: serde_json::to_value(&extra).unwrap_or_default(), ..sh }
        };
        match op {
            EventOp::Create => {
                let e = local.expect("create has a local event");
                let mut body = event_to_graph(e, true);
                body["transactionId"] = json!(uuid::Uuid::new_v4().to_string());
                let created = self.graph.post(&format!("/me/calendars/{}/events", seg(remote)), body).await?;
                let rid = created["id"].as_str().ok_or_else(|| SyncError::protocol("Microsoft 365 didn't return the new event's id."))?;
                let sh = Shadow { remote_id: rid.to_string(), container: remote.to_string(), etag: created["changeKey"].as_str().map(str::to_string), ..Default::default() };
                Ok(Some(settled(e, sh, e.my_response)))
            }
            EventOp::Update => {
                let (e, sh) = (local.expect("update"), shadow.expect("update").clone());
                let updated = self.graph.patch(&format!("/me/events/{}", seg(&sh.remote_id)), event_to_graph(e, false)).await?;
                let sh = Shadow { etag: updated["changeKey"].as_str().map(str::to_string).or(sh.etag.clone()), ..sh };
                let response = shadow_extra(&sh).response;
                Ok(Some(settled(e, sh, response)))
            }
            EventOp::Respond(r) => {
                let sh = shadow.expect("respond").clone();
                let verb = match r {
                    ResponseStatus::Accepted => "accept",
                    ResponseStatus::Tentative => "tentativelyAccept",
                    _ => "decline",
                };
                self.graph.post(&format!("/me/events/{}/{verb}", seg(&sh.remote_id)), json!({"sendResponse": true})).await?;
                let mut extra = shadow_extra(&sh);
                extra.response = r;
                Ok(Some(Shadow { extra: serde_json::to_value(&extra).unwrap_or_default(), ..sh }))
            }
            EventOp::CancelOccurrences(dates) => {
                let (e, sh) = (local.expect("cancel"), shadow.expect("cancel").clone());
                for d in &dates {
                    let from = axentra_core::local_to_utc(*d - Duration::days(1), NaiveTime::MIN);
                    let to = axentra_core::local_to_utc(*d + Duration::days(2), NaiveTime::MIN);
                    let url = format!(
                        "/me/events/{}/instances?startDateTime={}&endDateTime={}&$select=id,originalStart,start",
                        seg(&sh.remote_id),
                        from.format("%Y-%m-%dT%H:%M:%SZ"),
                        to.format("%Y-%m-%dT%H:%M:%SZ")
                    );
                    for inst in self.graph.get_all(&url).await? {
                        let original = super::map::parse_instant(&inst["originalStart"]).or_else(|| super::map::parse_dtz(&inst["start"]));
                        if original.map(local_date) == Some(*d)
                            && let Some(iid) = inst["id"].as_str()
                        {
                            self.graph.delete(&format!("/me/events/{}", seg(iid))).await?;
                        }
                    }
                }
                let mut extra = shadow_extra(&sh);
                extra.exdates = e.exdates.clone();
                Ok(Some(Shadow { extra: serde_json::to_value(&extra).unwrap_or_default(), ..sh }))
            }
            EventOp::Delete => {
                let sh = shadow.expect("delete");
                self.graph.delete(&format!("/me/events/{}", seg(&sh.remote_id))).await?;
                Ok(None)
            }
        }
    }
}
