//! Microsoft Graph JSON ↔ model (§8.2, §8.3). Pure functions, tested with
//! fixtures shaped like the Graph v1.0 reference responses.

use axentra_core::*;
use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc, Weekday};
use serde_json::{Value, json};

use crate::mime::bare_id;

// ---------------------------------------------------------------- shared

fn s(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

pub fn parse_instant(v: &Value) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(v.as_str()?).ok().map(|d| d.with_timezone(&Utc))
}

fn email(v: &Value) -> EmailAddress {
    EmailAddress::new(s(&v["emailAddress"]["name"]), s(&v["emailAddress"]["address"]))
}

fn emails(v: &Value) -> Vec<EmailAddress> {
    v.as_array().map(|a| a.iter().map(email).filter(|e| !e.address.is_empty()).collect()).unwrap_or_default()
}

/// IANA ↔ Windows zone names for the common zones (CLDR windowsZones).
const WINDOWS_ZONES: &[(&str, &str)] = &[
    ("UTC", "UTC"),
    ("Europe/London", "GMT Standard Time"),
    ("Europe/Dublin", "GMT Standard Time"),
    ("Europe/Lisbon", "GMT Standard Time"),
    ("Europe/Berlin", "W. Europe Standard Time"),
    ("Europe/Amsterdam", "W. Europe Standard Time"),
    ("Europe/Rome", "W. Europe Standard Time"),
    ("Europe/Stockholm", "W. Europe Standard Time"),
    ("Europe/Vienna", "W. Europe Standard Time"),
    ("Europe/Zurich", "W. Europe Standard Time"),
    ("Europe/Paris", "Romance Standard Time"),
    ("Europe/Madrid", "Romance Standard Time"),
    ("Europe/Brussels", "Romance Standard Time"),
    ("Europe/Copenhagen", "Romance Standard Time"),
    ("Europe/Warsaw", "Central European Standard Time"),
    ("Europe/Prague", "Central Europe Standard Time"),
    ("Europe/Budapest", "Central Europe Standard Time"),
    ("Europe/Athens", "GTB Standard Time"),
    ("Europe/Bucharest", "GTB Standard Time"),
    ("Europe/Helsinki", "FLE Standard Time"),
    ("Europe/Kiev", "FLE Standard Time"),
    ("Europe/Kyiv", "FLE Standard Time"),
    ("Europe/Istanbul", "Turkey Standard Time"),
    ("Europe/Moscow", "Russian Standard Time"),
    ("Africa/Cairo", "Egypt Standard Time"),
    ("Africa/Johannesburg", "South Africa Standard Time"),
    ("Africa/Lagos", "W. Central Africa Standard Time"),
    ("Africa/Nairobi", "E. Africa Standard Time"),
    ("Asia/Dubai", "Arabian Standard Time"),
    ("Asia/Jerusalem", "Israel Standard Time"),
    ("Asia/Karachi", "Pakistan Standard Time"),
    ("Asia/Kolkata", "India Standard Time"),
    ("Asia/Calcutta", "India Standard Time"),
    ("Asia/Dhaka", "Bangladesh Standard Time"),
    ("Asia/Bangkok", "SE Asia Standard Time"),
    ("Asia/Jakarta", "SE Asia Standard Time"),
    ("Asia/Singapore", "Singapore Standard Time"),
    ("Asia/Kuala_Lumpur", "Singapore Standard Time"),
    ("Asia/Shanghai", "China Standard Time"),
    ("Asia/Hong_Kong", "China Standard Time"),
    ("Asia/Taipei", "Taipei Standard Time"),
    ("Asia/Seoul", "Korea Standard Time"),
    ("Asia/Tokyo", "Tokyo Standard Time"),
    ("Australia/Perth", "W. Australia Standard Time"),
    ("Australia/Adelaide", "Cen. Australia Standard Time"),
    ("Australia/Brisbane", "E. Australia Standard Time"),
    ("Australia/Sydney", "AUS Eastern Standard Time"),
    ("Australia/Melbourne", "AUS Eastern Standard Time"),
    ("Pacific/Auckland", "New Zealand Standard Time"),
    ("Pacific/Honolulu", "Hawaiian Standard Time"),
    ("America/Anchorage", "Alaskan Standard Time"),
    ("America/Los_Angeles", "Pacific Standard Time"),
    ("America/Vancouver", "Pacific Standard Time"),
    ("America/Phoenix", "US Mountain Standard Time"),
    ("America/Denver", "Mountain Standard Time"),
    ("America/Edmonton", "Mountain Standard Time"),
    ("America/Chicago", "Central Standard Time"),
    ("America/Winnipeg", "Central Standard Time"),
    ("America/Mexico_City", "Central Standard Time (Mexico)"),
    ("America/New_York", "Eastern Standard Time"),
    ("America/Toronto", "Eastern Standard Time"),
    ("America/Detroit", "Eastern Standard Time"),
    ("America/Halifax", "Atlantic Standard Time"),
    ("America/St_Johns", "Newfoundland Standard Time"),
    ("America/Bogota", "SA Pacific Standard Time"),
    ("America/Lima", "SA Pacific Standard Time"),
    ("America/Sao_Paulo", "E. South America Standard Time"),
    ("America/Argentina/Buenos_Aires", "Argentina Standard Time"),
    ("America/Santiago", "Pacific SA Standard Time"),
];

pub fn windows_zone(iana: &str) -> Option<&'static str> {
    WINDOWS_ZONES.iter().find(|(i, _)| *i == iana).map(|(_, w)| *w)
}

pub fn iana_zone(windows: &str) -> Option<&'static str> {
    WINDOWS_ZONES.iter().find(|(_, w)| *w == windows).map(|(i, _)| *i)
}

/// The system's IANA zone (TZ, /etc/timezone or the /etc/localtime link).
pub fn local_iana() -> String {
    if let Ok(tz) = std::env::var("TZ") {
        let tz = tz.trim_start_matches(':').trim();
        if tz.contains('/') {
            return tz.to_string();
        }
    }
    if let Ok(s) = std::fs::read_to_string("/etc/timezone") {
        let s = s.trim();
        if !s.is_empty() {
            return s.to_string();
        }
    }
    std::fs::read_link("/etc/localtime")
        .ok()
        .and_then(|p| p.to_string_lossy().split("zoneinfo/").nth(1).map(str::to_string))
        .unwrap_or_else(|| "UTC".into())
}

/// A Graph `dateTimeTimeZone`. Requests ask for UTC; a zone matching the
/// local one is read as local time; anything else is taken as UTC.
pub fn parse_dtz(v: &Value) -> Option<DateTime<Utc>> {
    let raw = v["dateTime"].as_str()?;
    let naive = NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S%.f").or_else(|_| NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S")).ok()?;
    let tz = v["timeZone"].as_str().unwrap_or("UTC");
    let local = local_iana();
    if tz != "UTC" && (Some(local.as_str()) == iana_zone(tz) || tz == local) {
        return Local.from_local_datetime(&naive).earliest().map(|d| d.with_timezone(&Utc));
    }
    Some(Utc.from_utc_datetime(&naive))
}

/// Date part of a `dateTimeTimeZone` (all-day events, task due dates).
pub fn dtz_date(v: &Value) -> Option<NaiveDate> {
    let raw = v["dateTime"].as_str()?;
    NaiveDate::parse_from_str(raw.get(..10)?, "%Y-%m-%d").ok()
}

fn dtz_utc(t: DateTime<Utc>) -> Value {
    json!({"dateTime": t.format("%Y-%m-%dT%H:%M:%S").to_string(), "timeZone": "UTC"})
}

// ---------------------------------------------------------------- messages

/// A message from a list / delta response.
#[derive(Clone, Debug, Default)]
pub struct GraphMessage {
    pub id: String,
    pub removed: bool,
    pub message: Message,
    pub body: MessageBody,
    /// `parentFolderId`.
    pub folder: String,
    pub change_key: Option<String>,
}

pub fn message_from_graph(v: &Value) -> Option<GraphMessage> {
    let id = v["id"].as_str()?.to_string();
    if v.get("@removed").is_some() {
        return Some(GraphMessage { id, removed: true, ..Default::default() });
    }
    let flag = match v["flag"]["flagStatus"].as_str() {
        Some("flagged") => FlagStatus::Flagged,
        Some("complete") => FlagStatus::Complete,
        _ => FlagStatus::NotFlagged,
    };
    let body = &v["body"];
    let content = s(&body["content"]);
    let (html, text) = if body["contentType"].as_str() == Some("html") {
        let t = axentra_core::html::html_to_text(&content);
        (Some(content), t)
    } else {
        (None, content)
    };
    let meeting_kind = match v["meetingMessageType"].as_str() {
        Some("meetingRequest") => Some(MeetingMessageKind::Request),
        Some("meetingCancelled") => Some(MeetingMessageKind::Cancellation),
        Some("meetingAccepted") => Some(MeetingMessageKind::Accepted),
        // The published wire value is misspelled; accept both (§4.7).
        Some("meetingTenativelyAccepted" | "meetingTentativelyAccepted") => Some(MeetingMessageKind::Tentative),
        Some("meetingDeclined") => Some(MeetingMessageKind::Declined),
        _ => None,
    };
    let received = parse_instant(&v["receivedDateTime"]).unwrap_or_else(Utc::now);
    let message = Message {
        conversation_id: s(&v["conversationId"]),
        remote_id: Some(id.clone()),
        internet_message_id: bare_id(v["internetMessageId"].as_str().unwrap_or_default()),
        from: email(&v["from"]),
        to: emails(&v["toRecipients"]),
        cc: emails(&v["ccRecipients"]),
        bcc: emails(&v["bccRecipients"]),
        reply_to: emails(&v["replyTo"]),
        subject: s(&v["subject"]),
        preview: axentra_core::text::preview(v["bodyPreview"].as_str().unwrap_or_default(), 200),
        received_at: received,
        sent_at: parse_instant(&v["sentDateTime"]),
        is_read: v["isRead"].as_bool().unwrap_or(false),
        flag,
        flag_due: if flag == FlagStatus::Flagged { dtz_date(&v["flag"]["dueDateTime"]) } else { None },
        importance: match v["importance"].as_str() {
            Some("high") => Importance::High,
            Some("low") => Importance::Low,
            _ => Importance::Normal,
        },
        has_attachments: v["hasAttachments"].as_bool().unwrap_or(false),
        categories: v["categories"].as_array().map(|a| a.iter().filter_map(|c| c.as_str().map(str::to_string)).collect()).unwrap_or_default(),
        inference: if v["inferenceClassification"].as_str() == Some("other") { InferenceClass::Other } else { InferenceClass::Focused },
        is_draft: v["isDraft"].as_bool().unwrap_or(false),
        meeting_kind,
        ..Default::default()
    };
    Some(GraphMessage {
        id,
        removed: false,
        message,
        body: MessageBody { html, text, ..Default::default() },
        folder: s(&v["parentFolderId"]),
        change_key: v["changeKey"].as_str().map(str::to_string),
    })
}

/// PATCH body for a flag change (`dueDateTime` needs `startDateTime`).
pub fn flag_patch(status: FlagStatus, due: Option<NaiveDate>) -> Value {
    match status {
        FlagStatus::NotFlagged => json!({"flag": {"flagStatus": "notFlagged"}}),
        FlagStatus::Complete => json!({"flag": {"flagStatus": "complete"}}),
        FlagStatus::Flagged => match due {
            None => json!({"flag": {"flagStatus": "flagged"}}),
            Some(d) => {
                let today = Local::now().date_naive().min(d);
                json!({"flag": {
                    "flagStatus": "flagged",
                    "startDateTime": {"dateTime": format!("{today}T00:00:00"), "timeZone": "UTC"},
                    "dueDateTime": {"dateTime": format!("{d}T00:00:00"), "timeZone": "UTC"},
                }})
            }
        },
    }
}

// ---------------------------------------------------------------- events

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EventType {
    #[default]
    SingleInstance,
    Occurrence,
    Exception,
    SeriesMaster,
}

#[derive(Clone, Debug, Default)]
pub struct GraphEvent {
    pub id: String,
    pub removed: bool,
    pub event: Event,
    pub kind: EventType,
    pub series_master_id: Option<String>,
    pub original_start: Option<DateTime<Utc>>,
    pub change_key: Option<String>,
    /// The recurrence pattern couldn't be represented locally.
    pub recurrence_unsupported: bool,
}

fn weekday(name: &str) -> Option<Weekday> {
    Some(match name {
        "sunday" => Weekday::Sun,
        "monday" => Weekday::Mon,
        "tuesday" => Weekday::Tue,
        "wednesday" => Weekday::Wed,
        "thursday" => Weekday::Thu,
        "friday" => Weekday::Fri,
        "saturday" => Weekday::Sat,
        _ => return None,
    })
}

fn weekday_name(w: Weekday) -> &'static str {
    match w {
        Weekday::Sun => "sunday",
        Weekday::Mon => "monday",
        Weekday::Tue => "tuesday",
        Weekday::Wed => "wednesday",
        Weekday::Thu => "thursday",
        Weekday::Fri => "friday",
        Weekday::Sat => "saturday",
    }
}

fn index_of(name: &str) -> Option<i32> {
    Some(match name {
        "first" => 1,
        "second" => 2,
        "third" => 3,
        "fourth" => 4,
        "last" => -1,
        _ => return None,
    })
}

fn index_name(n: i32) -> &'static str {
    match n {
        1 => "first",
        2 => "second",
        3 => "third",
        4 => "fourth",
        _ => "last",
    }
}

/// `patternedRecurrence` → model. `None` when the pattern has no local
/// equivalent (e.g. "first weekday of the month").
pub fn recurrence_from_graph(v: &Value) -> Option<Recurrence> {
    let p = &v["pattern"];
    let r = &v["range"];
    let interval = p["interval"].as_u64().unwrap_or(1).max(1) as u32;
    let days: Vec<Weekday> = p["daysOfWeek"].as_array().map(|a| a.iter().filter_map(|d| d.as_str().and_then(weekday)).collect()).unwrap_or_default();
    let index = p["index"].as_str().and_then(index_of).unwrap_or(1);
    let day_of_month = p["dayOfMonth"].as_u64().filter(|d| *d > 0).map(|d| d as u32);
    let month = p["month"].as_u64().filter(|m| *m > 0).map(|m| m as u32);
    let mut rec = Recurrence { interval, ..Default::default() };
    match p["type"].as_str()? {
        "daily" => rec.freq = RecurrenceFreq::Daily,
        "weekly" => {
            rec.freq = RecurrenceFreq::Weekly;
            rec.by_weekday = days;
        }
        "absoluteMonthly" => {
            rec.freq = RecurrenceFreq::Monthly;
            rec.month_day = day_of_month;
        }
        "relativeMonthly" => {
            if days.len() != 1 {
                return None;
            }
            rec.freq = RecurrenceFreq::Monthly;
            rec.nth_weekday = Some((index, days[0]));
        }
        "absoluteYearly" => {
            rec.freq = RecurrenceFreq::Yearly;
            rec.month = month;
            rec.month_day = day_of_month;
        }
        "relativeYearly" => {
            if days.len() != 1 {
                return None;
            }
            rec.freq = RecurrenceFreq::Yearly;
            rec.month = month;
            rec.nth_weekday = Some((index, days[0]));
        }
        _ => return None,
    }
    rec.end = match r["type"].as_str() {
        Some("endDate") => r["endDate"].as_str().and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()).map(RecurrenceEnd::Until).unwrap_or_default(),
        Some("numbered") => RecurrenceEnd::Count(r["numberOfOccurrences"].as_u64().unwrap_or(1).max(1) as u32),
        _ => RecurrenceEnd::Never,
    };
    Some(rec)
}

/// Model → `patternedRecurrence` for a series starting on `start`.
pub fn recurrence_to_graph(rec: &Recurrence, start: NaiveDate, time_zone: &str) -> Value {
    let interval = rec.interval.max(1);
    let pattern = match rec.freq {
        RecurrenceFreq::Daily => json!({"type": "daily", "interval": interval}),
        RecurrenceFreq::Weekly => {
            let days: Vec<&str> = if rec.by_weekday.is_empty() { vec![weekday_name(start.weekday())] } else { rec.by_weekday.iter().map(|w| weekday_name(*w)).collect() };
            json!({"type": "weekly", "interval": interval, "daysOfWeek": days, "firstDayOfWeek": "sunday"})
        }
        RecurrenceFreq::Monthly => match rec.nth_weekday {
            Some((n, w)) => json!({"type": "relativeMonthly", "interval": interval, "daysOfWeek": [weekday_name(w)], "index": index_name(n)}),
            None => json!({"type": "absoluteMonthly", "interval": interval, "dayOfMonth": rec.month_day.unwrap_or(start.day())}),
        },
        RecurrenceFreq::Yearly => {
            let month = rec.month.unwrap_or(start.month());
            match rec.nth_weekday {
                Some((n, w)) => json!({"type": "relativeYearly", "interval": interval, "month": month, "daysOfWeek": [weekday_name(w)], "index": index_name(n)}),
                None => json!({"type": "absoluteYearly", "interval": interval, "month": month, "dayOfMonth": rec.month_day.unwrap_or(start.day())}),
            }
        }
    };
    let start_s = start.format("%Y-%m-%d").to_string();
    let range = match rec.end {
        RecurrenceEnd::Never => json!({"type": "noEnd", "startDate": start_s, "recurrenceTimeZone": time_zone}),
        RecurrenceEnd::Until(d) => json!({"type": "endDate", "startDate": start_s, "endDate": d.format("%Y-%m-%d").to_string(), "recurrenceTimeZone": time_zone}),
        RecurrenceEnd::Count(n) => json!({"type": "numbered", "startDate": start_s, "numberOfOccurrences": n, "recurrenceTimeZone": time_zone}),
    };
    json!({"pattern": pattern, "range": range})
}

fn show_as(v: &str) -> ShowAs {
    match v {
        "free" => ShowAs::Free,
        "tentative" => ShowAs::Tentative,
        "oof" => ShowAs::OutOfOffice,
        "workingElsewhere" => ShowAs::WorkingElsewhere,
        _ => ShowAs::Busy,
    }
}

fn show_as_name(s: ShowAs) -> &'static str {
    match s {
        ShowAs::Free => "free",
        ShowAs::Tentative => "tentative",
        ShowAs::Busy => "busy",
        ShowAs::OutOfOffice => "oof",
        ShowAs::WorkingElsewhere => "workingElsewhere",
    }
}

pub fn response(v: &str) -> ResponseStatus {
    match v {
        "organizer" => ResponseStatus::Organizer,
        "accepted" => ResponseStatus::Accepted,
        "tentativelyAccepted" => ResponseStatus::Tentative,
        "declined" => ResponseStatus::Declined,
        "notResponded" => ResponseStatus::NotResponded,
        _ => ResponseStatus::None,
    }
}

pub fn event_from_graph(v: &Value, calendar_id: CalendarId) -> Option<GraphEvent> {
    let id = v["id"].as_str()?.to_string();
    if v.get("@removed").is_some() {
        return Some(GraphEvent { id, removed: true, ..Default::default() });
    }
    let all_day = v["isAllDay"].as_bool().unwrap_or(false);
    let (start, end) = if all_day {
        let sd = dtz_date(&v["start"])?;
        let ed = dtz_date(&v["end"]).unwrap_or(sd.succ_opt()?);
        (axentra_core::local_to_utc(sd, NaiveTime::MIN), axentra_core::local_to_utc(ed, NaiveTime::MIN))
    } else {
        let st = parse_dtz(&v["start"])?;
        (st, parse_dtz(&v["end"]).unwrap_or(st))
    };
    let kind = match v["type"].as_str() {
        Some("occurrence") => EventType::Occurrence,
        Some("exception") => EventType::Exception,
        Some("seriesMaster") => EventType::SeriesMaster,
        _ => EventType::SingleInstance,
    };
    let recurrence = if kind == EventType::SeriesMaster && !v["recurrence"].is_null() { recurrence_from_graph(&v["recurrence"]) } else { None };
    let tz_win = v["originalStartTimeZone"].as_str().unwrap_or_default();
    let time_zone = iana_zone(tz_win).map(str::to_string).unwrap_or_else(|| if tz_win.contains('/') { tz_win.to_string() } else { local_iana() });
    let attendees = v["attendees"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|x| Attendee {
                    email: email(x),
                    kind: match x["type"].as_str() {
                        Some("optional") => AttendeeKind::Optional,
                        Some("resource") => AttendeeKind::Resource,
                        _ => AttendeeKind::Required,
                    },
                    response: response(x["status"]["response"].as_str().unwrap_or_default()),
                })
                .filter(|a| !a.email.address.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let organizer = Some(email(&v["organizer"])).filter(|e| !e.address.is_empty());
    let is_organizer = v["isOrganizer"].as_bool().unwrap_or(true);
    let my_response = match response(v["responseStatus"]["response"].as_str().unwrap_or_default()) {
        ResponseStatus::None if is_organizer => ResponseStatus::Organizer,
        r => r,
    };
    let event = Event {
        calendar_id,
        remote_id: Some(id.clone()),
        title: s(&v["subject"]),
        location: s(&v["location"]["displayName"]),
        body_html: s(&v["body"]["content"]),
        start,
        end,
        all_day,
        time_zone,
        recurrence,
        exdates: Vec::new(),
        show_as: show_as(v["showAs"].as_str().unwrap_or("busy")),
        is_private: matches!(v["sensitivity"].as_str(), Some("private" | "confidential")),
        reminder_minutes: if v["isReminderOn"].as_bool().unwrap_or(false) { v["reminderMinutesBeforeStart"].as_i64().map(|m| m as i32) } else { None },
        organizer,
        attendees,
        my_response,
        is_online_meeting: v["isOnlineMeeting"].as_bool().unwrap_or(false),
        online_meeting_url: v["onlineMeeting"]["joinUrl"].as_str().map(str::to_string).filter(|s| !s.is_empty()),
        categories: v["categories"].as_array().map(|a| a.iter().filter_map(|c| c.as_str().map(str::to_string)).collect()).unwrap_or_default(),
        is_cancelled: v["isCancelled"].as_bool().unwrap_or(false),
        created_at: parse_instant(&v["createdDateTime"]).unwrap_or_else(Utc::now),
        updated_at: parse_instant(&v["lastModifiedDateTime"]).unwrap_or_else(Utc::now),
        ..Default::default()
    };
    Some(GraphEvent {
        id,
        removed: false,
        recurrence_unsupported: kind == EventType::SeriesMaster && event.recurrence.is_none(),
        event,
        kind,
        series_master_id: v["seriesMasterId"].as_str().map(str::to_string),
        original_start: parse_instant(&v["originalStart"]),
        change_key: v["changeKey"].as_str().map(str::to_string),
    })
}

/// Event write payload (§4.9). `create` adds the fields that can only be
/// set once (online meeting).
pub fn event_to_graph(e: &Event, create: bool) -> Value {
    let local_tz = local_iana();
    let win = windows_zone(&local_tz);
    let (start, end) = if e.all_day {
        let sd = axentra_core::local_date(e.start);
        let ed = axentra_core::local_date(e.end).max(sd.succ_opt().unwrap_or(sd));
        let tz = win.unwrap_or("UTC");
        (json!({"dateTime": format!("{sd}T00:00:00"), "timeZone": tz}), json!({"dateTime": format!("{ed}T00:00:00"), "timeZone": tz}))
    } else if let (Some(_), Some(tz)) = (&e.recurrence, win) {
        // Series follow local wall-clock time across DST changes.
        let fmt = |t: DateTime<Utc>| t.with_timezone(&Local).format("%Y-%m-%dT%H:%M:%S").to_string();
        (json!({"dateTime": fmt(e.start), "timeZone": tz}), json!({"dateTime": fmt(e.end), "timeZone": tz}))
    } else {
        (dtz_utc(e.start), dtz_utc(e.end))
    };
    let mut v = json!({
        "subject": e.title,
        "body": {"contentType": "html", "content": e.body_html},
        "start": start,
        "end": end,
        "isAllDay": e.all_day,
        "location": {"displayName": e.location},
        "showAs": show_as_name(e.show_as),
        "sensitivity": if e.is_private { "private" } else { "normal" },
        "isReminderOn": e.reminder_minutes.is_some(),
        "categories": e.categories,
        "attendees": e.attendees.iter().map(|a| json!({
            "emailAddress": {"address": a.email.address, "name": a.email.name},
            "type": match a.kind { AttendeeKind::Required => "required", AttendeeKind::Optional => "optional", AttendeeKind::Resource => "resource" },
        })).collect::<Vec<_>>(),
    });
    if let Some(m) = e.reminder_minutes {
        v["reminderMinutesBeforeStart"] = json!(m);
    }
    v["recurrence"] = match &e.recurrence {
        Some(r) => recurrence_to_graph(r, axentra_core::local_date(e.start), win.unwrap_or("UTC")),
        None => Value::Null,
    };
    if create && e.is_online_meeting {
        v["isOnlineMeeting"] = json!(true);
        v["onlineMeetingProvider"] = json!("teamsForBusiness");
    }
    v
}

// ---------------------------------------------------------------- calendars

pub fn calendar_from_graph(v: &Value, account_id: AccountId, index: usize) -> Calendar {
    const PALETTE: &[&str] = &["#0f6cbd", "#13a10e", "#ca5010", "#8764b8", "#c50f1f", "#038387", "#986f0b", "#e3008c"];
    let hex = v["hexColor"].as_str().filter(|h| h.starts_with('#') && h.len() == 7).map(str::to_string);
    Calendar {
        account_id,
        name: s(&v["name"]),
        color: hex.unwrap_or_else(|| PALETTE[index % PALETTE.len()].to_string()),
        is_visible: true,
        is_default: v["isDefaultCalendar"].as_bool().unwrap_or(false),
        can_edit: v["canEdit"].as_bool().unwrap_or(true),
        remote_id: v["id"].as_str().map(str::to_string),
        group: "My calendars".into(),
        sort_order: index as i64,
        ..Default::default()
    }
}

// ---------------------------------------------------------------- contacts

/// Graph contact → model. `previous` keeps local labels for known addresses.
pub fn contact_from_graph(v: &Value, account_id: AccountId, previous: Option<&Contact>) -> Option<Contact> {
    let id = v["id"].as_str()?.to_string();
    let label_for = |addr: &str| {
        previous
            .and_then(|p| p.emails.iter().find(|e| e.value.eq_ignore_ascii_case(addr)).map(|e| e.label.clone()))
            .unwrap_or_else(|| "Email".into())
    };
    let mut emails: Vec<LabeledValue> = Vec::new();
    for e in v["emailAddresses"].as_array().into_iter().flatten() {
        let addr = s(&e["address"]);
        if !addr.is_empty() && !emails.iter().any(|x| x.value.eq_ignore_ascii_case(&addr)) {
            emails.push(LabeledValue { label: label_for(&addr), value: addr });
        }
    }
    let mut phones = Vec::new();
    for p in v["businessPhones"].as_array().into_iter().flatten().filter_map(|p| p.as_str()) {
        phones.push(LabeledValue { label: "Work".into(), value: p.to_string() });
    }
    for p in v["homePhones"].as_array().into_iter().flatten().filter_map(|p| p.as_str()) {
        phones.push(LabeledValue { label: "Home".into(), value: p.to_string() });
    }
    if let Some(m) = v["mobilePhone"].as_str().filter(|m| !m.is_empty()) {
        phones.push(LabeledValue { label: "Mobile".into(), value: m.to_string() });
    }
    let address = |a: &Value, label: &str| {
        let pa = PostalAddress {
            label: label.into(),
            street: s(&a["street"]),
            city: s(&a["city"]),
            state: s(&a["state"]),
            postal_code: s(&a["postalCode"]),
            country: s(&a["countryOrRegion"]),
        };
        (!(pa.street.is_empty() && pa.city.is_empty() && pa.state.is_empty() && pa.postal_code.is_empty() && pa.country.is_empty())).then_some(pa)
    };
    let addresses = [("businessAddress", "Work"), ("homeAddress", "Home"), ("otherAddress", "Other")]
        .iter()
        .filter_map(|(k, l)| address(&v[*k], l))
        .collect();
    Some(Contact {
        id: previous.map(|p| p.id).unwrap_or(0),
        account_id,
        remote_id: Some(id),
        given_name: s(&v["givenName"]),
        surname: s(&v["surname"]),
        display_name: s(&v["displayName"]),
        nickname: s(&v["nickName"]),
        emails,
        phones,
        addresses,
        websites: v["businessHomePage"].as_str().filter(|w| !w.is_empty()).map(|w| vec![w.to_string()]).unwrap_or_default(),
        company: s(&v["companyName"]),
        job_title: s(&v["jobTitle"]),
        department: s(&v["department"]),
        office_location: s(&v["officeLocation"]),
        birthday: v["birthday"].as_str().and_then(|b| NaiveDate::parse_from_str(b.get(..10)?, "%Y-%m-%d").ok()),
        notes: s(&v["personalNotes"]),
        is_favorite: previous.map(|p| p.is_favorite).unwrap_or(false),
        created_at: previous.map(|p| p.created_at).unwrap_or_else(Utc::now),
        updated_at: Utc::now(),
        ..previous.cloned().unwrap_or_default()
    })
}

pub fn contact_to_graph(c: &Contact) -> Value {
    let phones_with = |labels: &[&str]| -> Vec<String> {
        c.phones.iter().filter(|p| labels.iter().any(|l| p.label.eq_ignore_ascii_case(l))).map(|p| p.value.clone()).collect()
    };
    let mobile = c.phones.iter().find(|p| ["mobile", "cell"].iter().any(|l| p.label.eq_ignore_ascii_case(l))).map(|p| p.value.clone());
    let mut business = phones_with(&["work", "business"]);
    business.extend(
        c.phones
            .iter()
            .filter(|p| !["work", "business", "home", "mobile", "cell"].iter().any(|l| p.label.eq_ignore_ascii_case(l)))
            .map(|p| p.value.clone()),
    );
    let addr = |label: &str| {
        c.addresses.iter().find(|a| a.label.eq_ignore_ascii_case(label)).map(|a| {
            json!({"street": a.street, "city": a.city, "state": a.state, "postalCode": a.postal_code, "countryOrRegion": a.country})
        })
    };
    json!({
        "givenName": c.given_name,
        "surname": c.surname,
        // Always resend displayName so Graph doesn't regenerate it (§4.10).
        "displayName": c.display_name,
        "nickName": c.nickname,
        "emailAddresses": c.emails.iter().map(|e| json!({"address": e.value, "name": c.display_name})).collect::<Vec<_>>(),
        "businessPhones": business,
        "homePhones": phones_with(&["home"]),
        "mobilePhone": mobile,
        "businessAddress": addr("work").unwrap_or(json!({})),
        "homeAddress": addr("home").unwrap_or(json!({})),
        "otherAddress": addr("other").unwrap_or(json!({})),
        "businessHomePage": c.websites.first(),
        "companyName": c.company,
        "jobTitle": c.job_title,
        "department": c.department,
        "officeLocation": c.office_location,
        "birthday": c.birthday.map(|b| format!("{b}T11:59:00Z")),
        "personalNotes": c.notes,
    })
}

// ---------------------------------------------------------------- To Do

/// (list, wellknownListName) from a `todoTaskList`.
pub fn task_list_from_graph(v: &Value, account_id: AccountId) -> Option<(TaskList, String)> {
    let id = v["id"].as_str()?.to_string();
    let wellknown = v["wellknownListName"].as_str().unwrap_or("none").to_string();
    Some((
        TaskList {
            account_id,
            remote_id: Some(id),
            name: s(&v["displayName"]),
            is_default: wellknown == "defaultList",
            ..Default::default()
        },
        wellknown,
    ))
}

pub fn task_from_graph(v: &Value, list_id: TaskListId, previous: Option<&Task>) -> Option<Task> {
    let id = v["id"].as_str()?.to_string();
    let completed = v["status"].as_str() == Some("completed");
    let steps = v["checklistItems"]
        .as_array()
        .map(|a| {
            a.iter()
                .enumerate()
                .map(|(i, c)| TaskStep { id: i as i64 + 1, title: s(&c["displayName"]), is_completed: c["isChecked"].as_bool().unwrap_or(false) })
                .collect()
        })
        .unwrap_or_else(|| previous.map(|p| p.steps.clone()).unwrap_or_default());
    let reminder = if v["isReminderOn"].as_bool().unwrap_or(false) { parse_dtz(&v["reminderDateTime"]) } else { None };
    Some(Task {
        id: previous.map(|p| p.id).unwrap_or(0),
        list_id,
        remote_id: Some(id),
        title: s(&v["title"]),
        notes: match v["body"]["contentType"].as_str() {
            Some("html") => axentra_core::html::html_to_text(v["body"]["content"].as_str().unwrap_or_default()),
            _ => s(&v["body"]["content"]),
        },
        is_completed: completed,
        completed_at: if completed { parse_dtz(&v["completedDateTime"]).or(previous.and_then(|p| p.completed_at)).or_else(|| Some(Utc::now())) } else { None },
        is_important: v["importance"].as_str() == Some("high"),
        due: dtz_date(&v["dueDateTime"]),
        reminder,
        recurrence: if v["recurrence"].is_null() { None } else { recurrence_from_graph(&v["recurrence"]) },
        my_day: previous.and_then(|p| p.my_day),
        steps,
        categories: v["categories"].as_array().map(|a| a.iter().filter_map(|c| c.as_str().map(str::to_string)).collect()).unwrap_or_default(),
        assigned_to_me: previous.map(|p| p.assigned_to_me).unwrap_or(false),
        linked_message_id: previous.and_then(|p| p.linked_message_id),
        created_at: parse_instant(&v["createdDateTime"]).unwrap_or_else(Utc::now),
        updated_at: parse_instant(&v["lastModifiedDateTime"]).unwrap_or_else(Utc::now),
        sort_order: previous.map(|p| p.sort_order).unwrap_or(0),
        ..previous.cloned().unwrap_or_default()
    })
}

pub fn task_to_graph(t: &Task) -> Value {
    let mut v = json!({
        "title": t.title,
        "body": {"contentType": "text", "content": t.notes},
        "status": if t.is_completed { "completed" } else { "notStarted" },
        "importance": if t.is_important { "high" } else { "normal" },
        "categories": t.categories,
        "isReminderOn": t.reminder.is_some(),
    });
    v["dueDateTime"] = match t.due {
        Some(d) => json!({"dateTime": format!("{d}T00:00:00.0000000"), "timeZone": "UTC"}),
        None => Value::Null,
    };
    if let Some(r) = t.reminder {
        v["reminderDateTime"] = dtz_utc(r);
    }
    if let Some(c) = t.completed_at.filter(|_| t.is_completed) {
        v["completedDateTime"] = dtz_utc(c);
    }
    v["recurrence"] = match (&t.recurrence, t.due) {
        (Some(r), Some(d)) => recurrence_to_graph(r, d, "UTC"),
        _ => Value::Null,
    };
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_fixture() {
        let v = json!({
            "id": "AAkALgAAAAAAHYQDEapmEc2byACqAC-EWg0A",
            "conversationId": "AAQkAGI2TG93AAA=",
            "internetMessageId": "<BYAPR15MB2342@namprd15.prod.outlook.com>",
            "subject": "Q4 plan",
            "bodyPreview": "Hi team,\r\n\r\nAttached   is the plan.",
            "from": {"emailAddress": {"name": "Megan Bowen", "address": "MeganB@contoso.com"}},
            "toRecipients": [{"emailAddress": {"name": "Alex", "address": "alex@contoso.com"}}],
            "ccRecipients": [],
            "receivedDateTime": "2026-10-19T08:30:00Z",
            "sentDateTime": "2026-10-19T08:29:58Z",
            "isRead": false, "isDraft": false, "hasAttachments": true, "importance": "high",
            "flag": {"flagStatus": "flagged", "dueDateTime": {"dateTime": "2026-10-23T00:00:00.0000000", "timeZone": "UTC"}},
            "categories": ["Red category"],
            "inferenceClassification": "other",
            "parentFolderId": "AQMkAGI2-inbox",
            "changeKey": "CQAAABYAAAA",
            "body": {"contentType": "html", "content": "<p>Hi team,</p><p>Attached is the plan.</p>"}
        });
        let g = message_from_graph(&v).unwrap();
        let m = &g.message;
        assert_eq!(m.remote_id.as_deref(), Some("AAkALgAAAAAAHYQDEapmEc2byACqAC-EWg0A"));
        assert_eq!(m.internet_message_id, "BYAPR15MB2342@namprd15.prod.outlook.com");
        assert_eq!(m.conversation_id, "AAQkAGI2TG93AAA=");
        assert_eq!(m.from.address, "MeganB@contoso.com");
        assert_eq!(m.preview, "Hi team, Attached is the plan.");
        assert_eq!((m.flag, m.flag_due), (FlagStatus::Flagged, NaiveDate::from_ymd_opt(2026, 10, 23)));
        assert_eq!(m.importance, Importance::High);
        assert_eq!(m.inference, InferenceClass::Other);
        assert!(m.has_attachments && !m.is_read);
        assert_eq!(g.folder, "AQMkAGI2-inbox");
        assert!(g.body.html.as_deref().unwrap().contains("<p>Hi team,</p>"));
        assert!(g.body.text.contains("Attached is the plan."));
        let removed = message_from_graph(&json!({"id": "x", "@removed": {"reason": "deleted"}})).unwrap();
        assert!(removed.removed);
        let invite = message_from_graph(&json!({"id": "i", "meetingMessageType": "meetingTenativelyAccepted"})).unwrap();
        assert_eq!(invite.message.meeting_kind, Some(MeetingMessageKind::Tentative));
        assert_eq!(flag_patch(FlagStatus::Complete, None), json!({"flag": {"flagStatus": "complete"}}));
        let due = flag_patch(FlagStatus::Flagged, NaiveDate::from_ymd_opt(2030, 1, 2));
        assert!(due["flag"]["startDateTime"].is_object() && due["flag"]["dueDateTime"]["dateTime"] == "2030-01-02T00:00:00");
    }

    #[test]
    fn event_with_recurrence_fixture() {
        let v = json!({
            "id": "AAMkMaster", "type": "seriesMaster", "subject": "Team sync",
            "body": {"contentType": "html", "content": "<p>Agenda</p>"},
            "start": {"dateTime": "2026-10-19T16:00:00.0000000", "timeZone": "UTC"},
            "end": {"dateTime": "2026-10-19T16:30:00.0000000", "timeZone": "UTC"},
            "isAllDay": false, "showAs": "tentative", "sensitivity": "private",
            "isReminderOn": true, "reminderMinutesBeforeStart": 10,
            "location": {"displayName": "Room 4"},
            "organizer": {"emailAddress": {"name": "Megan", "address": "megan@contoso.com"}},
            "isOrganizer": false,
            "responseStatus": {"response": "tentativelyAccepted", "time": "2026-10-18T10:00:00Z"},
            "attendees": [
                {"type": "required", "status": {"response": "accepted"}, "emailAddress": {"name": "Alex", "address": "alex@contoso.com"}},
                {"type": "optional", "status": {"response": "none"}, "emailAddress": {"name": "Lee", "address": "lee@contoso.com"}}
            ],
            "isOnlineMeeting": true, "onlineMeeting": {"joinUrl": "https://teams.example/join/1"},
            "originalStartTimeZone": "Pacific Standard Time",
            "recurrence": {
                "pattern": {"type": "weekly", "interval": 1, "daysOfWeek": ["monday", "wednesday"], "firstDayOfWeek": "sunday"},
                "range": {"type": "endDate", "startDate": "2026-10-19", "endDate": "2026-12-31", "recurrenceTimeZone": "Pacific Standard Time"}
            },
            "changeKey": "DwAAABYAAAA"
        });
        let g = event_from_graph(&v, 3).unwrap();
        assert_eq!(g.kind, EventType::SeriesMaster);
        let e = &g.event;
        assert_eq!(e.title, "Team sync");
        assert_eq!(e.start.to_rfc3339(), "2026-10-19T16:00:00+00:00");
        assert_eq!((e.end - e.start).num_minutes(), 30);
        assert_eq!(e.show_as, ShowAs::Tentative);
        assert!(e.is_private);
        assert_eq!(e.reminder_minutes, Some(10));
        assert_eq!(e.my_response, ResponseStatus::Tentative);
        assert_eq!(e.attendees.len(), 2);
        assert_eq!(e.attendees[1].kind, AttendeeKind::Optional);
        assert_eq!(e.time_zone, "America/Los_Angeles");
        assert_eq!(e.online_meeting_url.as_deref(), Some("https://teams.example/join/1"));
        let r = e.recurrence.as_ref().unwrap();
        assert_eq!(r.freq, RecurrenceFreq::Weekly);
        assert_eq!(r.by_weekday, vec![Weekday::Mon, Weekday::Wed]);
        assert_eq!(r.end, RecurrenceEnd::Until(NaiveDate::from_ymd_opt(2026, 12, 31).unwrap()));

        // Round trip of the pattern.
        let back = recurrence_to_graph(r, NaiveDate::from_ymd_opt(2026, 10, 19).unwrap(), "Pacific Standard Time");
        assert_eq!(back["pattern"]["daysOfWeek"], json!(["monday", "wednesday"]));
        assert_eq!(back["range"]["type"], "endDate");
        assert_eq!(recurrence_from_graph(&back).as_ref(), Some(r));

        let monthly = json!({"pattern": {"type": "relativeMonthly", "interval": 2, "daysOfWeek": ["thursday"], "index": "last"}, "range": {"type": "numbered", "numberOfOccurrences": 6}});
        let m = recurrence_from_graph(&monthly).unwrap();
        assert_eq!((m.freq, m.interval, m.nth_weekday, m.end), (RecurrenceFreq::Monthly, 2, Some((-1, Weekday::Thu)), RecurrenceEnd::Count(6)));
        let weekdays = json!({"pattern": {"type": "relativeMonthly", "daysOfWeek": ["monday", "tuesday"], "index": "first"}, "range": {"type": "noEnd"}});
        assert_eq!(recurrence_from_graph(&weekdays), None);

        let payload = event_to_graph(e, true);
        assert_eq!(payload["subject"], "Team sync");
        assert_eq!(payload["showAs"], "tentative");
        assert_eq!(payload["attendees"][1]["type"], "optional");
        assert_eq!(payload["reminderMinutesBeforeStart"], 10);
        assert_eq!(payload["isOnlineMeeting"], true);
        assert!(payload["recurrence"]["pattern"].is_object());

        let all_day = json!({"id": "d", "type": "singleInstance", "isAllDay": true, "subject": "Holiday",
            "start": {"dateTime": "2026-12-25T00:00:00.0000000", "timeZone": "UTC"},
            "end": {"dateTime": "2026-12-26T00:00:00.0000000", "timeZone": "UTC"}});
        let d = event_from_graph(&all_day, 1).unwrap().event;
        assert!(d.all_day);
        assert_eq!(axentra_core::local_date(d.start), NaiveDate::from_ymd_opt(2026, 12, 25).unwrap());
        assert_eq!(axentra_core::local_date(d.end), NaiveDate::from_ymd_opt(2026, 12, 26).unwrap());
    }

    #[test]
    fn contact_fixture() {
        let v = json!({
            "id": "AAMkContact", "givenName": "Pavel", "surname": "Bansky", "displayName": "Pavel Bansky", "nickName": "Pav",
            "emailAddresses": [{"address": "pavelb@contoso.com", "name": "Pavel Bansky"}, {"address": "pavel@home.example", "name": "x"}],
            "businessPhones": ["+1 732 555 0102"], "homePhones": [], "mobilePhone": "+1 425 555 0101",
            "businessAddress": {"street": "1 Main St", "city": "Redmond", "state": "WA", "postalCode": "98052", "countryOrRegion": "United States"},
            "homeAddress": {}, "companyName": "Contoso", "jobTitle": "Engineer", "department": "R&D",
            "officeLocation": "B 4", "birthday": "1974-07-22T11:59:00Z", "personalNotes": "Likes tea",
            "businessHomePage": "https://contoso.example"
        });
        let prev = Contact { id: 9, emails: vec![LabeledValue { label: "Work".into(), value: "PAVELB@contoso.com".into() }], is_favorite: true, ..Default::default() };
        let c = contact_from_graph(&v, 2, Some(&prev)).unwrap();
        assert_eq!((c.id, c.is_favorite), (9, true));
        assert_eq!(c.emails[0].label, "Work");
        assert_eq!(c.emails[1].label, "Email");
        assert_eq!(c.phones.iter().map(|p| p.label.as_str()).collect::<Vec<_>>(), vec!["Work", "Mobile"]);
        assert_eq!(c.addresses.len(), 1);
        assert_eq!(c.addresses[0].city, "Redmond");
        assert_eq!(c.birthday, NaiveDate::from_ymd_opt(1974, 7, 22));
        assert_eq!(c.websites, vec!["https://contoso.example".to_string()]);
        let back = contact_to_graph(&c);
        assert_eq!(back["displayName"], "Pavel Bansky");
        assert_eq!(back["mobilePhone"], "+1 425 555 0101");
        assert_eq!(back["businessPhones"], json!(["+1 732 555 0102"]));
        assert_eq!(back["businessAddress"]["postalCode"], "98052");
        assert_eq!(back["birthday"], "1974-07-22T11:59:00Z");
    }

    #[test]
    fn task_fixture() {
        let v = json!({
            "id": "AAMkTask", "title": "Book flights", "status": "notStarted", "importance": "high",
            "body": {"content": "Aisle seat", "contentType": "text"},
            "dueDateTime": {"dateTime": "2026-10-24T00:00:00.0000000", "timeZone": "UTC"},
            "isReminderOn": true, "reminderDateTime": {"dateTime": "2026-10-23T09:00:00.0000000", "timeZone": "UTC"},
            "categories": ["Travel"],
            "checklistItems": [{"displayName": "Compare prices", "isChecked": true}, {"displayName": "Pay", "isChecked": false}],
            "recurrence": {"pattern": {"type": "absoluteMonthly", "interval": 1, "dayOfMonth": 24}, "range": {"type": "noEnd", "startDate": "2026-10-24"}},
            "createdDateTime": "2026-10-01T10:00:00Z", "lastModifiedDateTime": "2026-10-02T10:00:00Z"
        });
        let t = task_from_graph(&v, 5, None).unwrap();
        assert_eq!((t.title.as_str(), t.notes.as_str(), t.is_important, t.is_completed), ("Book flights", "Aisle seat", true, false));
        assert_eq!(t.due, NaiveDate::from_ymd_opt(2026, 10, 24));
        assert_eq!(t.reminder.unwrap().to_rfc3339(), "2026-10-23T09:00:00+00:00");
        assert_eq!(t.steps.len(), 2);
        assert!(t.steps[0].is_completed);
        assert_eq!(t.recurrence.as_ref().unwrap().month_day, Some(24));
        let done = task_from_graph(&json!({"id": "d", "title": "x", "status": "completed", "completedDateTime": {"dateTime": "2026-10-03T00:00:00", "timeZone": "UTC"}}), 5, None).unwrap();
        assert!(done.is_completed && done.completed_at.is_some());
        let back = task_to_graph(&t);
        assert_eq!(back["importance"], "high");
        assert_eq!(back["dueDateTime"]["dateTime"], "2026-10-24T00:00:00.0000000");
        assert_eq!(back["status"], "notStarted");
        let (list, wk) = task_list_from_graph(&json!({"id": "L", "displayName": "Tasks", "wellknownListName": "defaultList"}), 1).unwrap();
        assert!(list.is_default && wk == "defaultList");
    }
}
