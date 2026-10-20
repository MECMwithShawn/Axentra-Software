use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use chrono::{Duration, Local, NaiveDate, TimeZone, Utc};

use super::format::TEST_KDF;
use super::*;
use crate::model::*;
use crate::seed::seed_demo;

const PW: &str = "correct horse battery";

/// A fresh, empty directory under the system temp dir, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let p = std::env::temp_dir().join(format!("axentra-backup-test-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 19, 14, 30, 5).unwrap()
}

fn demo() -> Store {
    let store = Store::open_in_memory().unwrap();
    seed_demo(&store, Local::now()).unwrap();
    store
}

fn primary(store: &Store) -> AccountId {
    store.primary_account().unwrap().unwrap().id
}

/// Extra things the demo data doesn't have: a draft with an attachment, a
/// sweep rule, sender lists, account-scoped and app-wide preferences.
fn enrich(store: &Store) {
    let acct = primary(store);
    let mut d = store.new_draft(ComposeKind::New, Some(acct), None).unwrap();
    d.subject = "Unsent plan".into();
    d.body_html = "<p>Draft body</p>".into();
    d.to = vec![EmailAddress::new("Priya", "priya.raman@northwind.example")];
    store.add_draft_attachment(&mut d, "plan.txt", "text/plain", b"step one, step two").unwrap();
    let inbox = store.folder_by_role(acct, FolderRole::Inbox).unwrap().unwrap().id;
    let receipts = store.folders(Some(acct)).unwrap().into_iter().find(|f| f.name == "Receipts").unwrap().id;
    store.sweep(inbox, "deals@shop.example", "move_all_future", receipts).unwrap();
    store.block_sender(&EmailAddress::new("Spam", "spam@junk.example")).unwrap();
    store.add_safe_sender("friend@safe.example").unwrap();
    store.kv_set(&format!("shell.ribbon.{acct}.mail_home"), r#"{"groups":[]}"#).unwrap();
    store.kv_set("calendar.day_count", "3").unwrap();
    store.kv_set("shell.notifications", "{\"seen\":[1]}").unwrap();
    let mut s = store.settings();
    s.signature_html = "<b>Alex</b>".into();
    s.density = Density::Compact;
    s.backup_dir = Some("/somewhere/else".into());
    store.save_settings(&s).unwrap();
}

/// A comparable digest of a store: names, subjects, titles and counts.
#[derive(Debug, PartialEq)]
struct Digest {
    accounts: Vec<(AccountKind, String)>,
    folders: Vec<(String, FolderRole, Option<String>, bool)>,
    messages: BTreeMap<String, usize>,
    bodies: usize,
    attachments_with_data: Vec<(String, Vec<u8>)>,
    drafts: Vec<(String, usize)>,
    calendars: Vec<(String, String, bool)>,
    events: BTreeMap<String, usize>,
    exceptions: usize,
    contacts: Vec<(String, String)>,
    contact_lists: Vec<(String, usize)>,
    task_lists: Vec<(String, Option<String>, Option<String>)>,
    tasks: BTreeMap<String, usize>,
    categories: Vec<String>,
    sweep_rules: usize,
    special_folders: usize,
}

fn digest(store: &Store) -> Digest {
    let conn = store.conn();
    let count = |sql: &str| -> usize { conn.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap() as usize };
    let bodies = count("SELECT COUNT(*) FROM message_bodies");
    let sweep_rules = count("SELECT COUNT(*) FROM mail_sweep_rules");
    let special_folders = count("SELECT COUNT(*) FROM mail_special_folders");
    let exceptions = count("SELECT COUNT(*) FROM events WHERE json_extract(json, '$.series_id') IS NOT NULL");
    let mut attachments_with_data: Vec<(String, Vec<u8>)> = {
        let mut stmt = conn.prepare("SELECT json_extract(json, '$.name'), data FROM attachments WHERE data IS NOT NULL").unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(|r| r.unwrap()).collect()
    };
    attachments_with_data.sort();
    let mut drafts: Vec<(String, usize)> = {
        let mut stmt = conn.prepare("SELECT json FROM drafts").unwrap();
        stmt.query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(|j| serde_json::from_str::<Draft>(&j.unwrap()).unwrap())
            .map(|d| (d.subject, d.attachments.iter().filter(|a| a.path.starts_with("attachment:")).count()))
            .collect()
    };
    drafts.sort();
    drop(conn);

    let folders_all = store.folders(None).unwrap();
    let name_of = |id: Option<FolderId>| id.and_then(|i| folders_all.iter().find(|f| f.id == i)).map(|f| f.name.clone());
    let mut folders: Vec<_> = folders_all.iter().map(|f| (f.name.clone(), f.role, name_of(f.parent_id), f.is_favorite)).collect();
    folders.sort_by_key(|f| format!("{f:?}"));
    let mut messages = BTreeMap::new();
    for a in store.accounts().unwrap() {
        for m in store.account_messages(a.id).unwrap() {
            let folder = name_of(Some(m.folder_id)).unwrap_or_default();
            *messages.entry(format!("{folder}/{}", m.subject)).or_insert(0) += 1;
        }
    }
    let mut events = BTreeMap::new();
    let mut calendars = Vec::new();
    for c in store.calendars().unwrap() {
        calendars.push((c.name.clone(), c.color.clone(), c.is_visible));
        for e in store.calendar_events(c.id).unwrap() {
            *events.entry(format!("{}/{}/{}", c.name, e.title, e.start.timestamp())).or_insert(0) += 1;
        }
    }
    calendars.sort();
    let mut contacts: Vec<_> = store.contacts().unwrap().into_iter().map(|c| (c.display_name, c.folder)).collect();
    contacts.sort();
    let mut contact_lists: Vec<_> = store.contact_lists().unwrap().into_iter().map(|l| (l.name, l.members.len())).collect();
    contact_lists.sort();
    let mut task_lists = Vec::new();
    let mut tasks = BTreeMap::new();
    for l in store.task_lists().unwrap() {
        task_lists.push((l.name.clone(), l.group.clone(), l.emoji.clone()));
        for t in store.tasks_in_list(l.id).unwrap() {
            *tasks.entry(format!("{}/{}/{}", l.name, t.title, t.steps.len())).or_insert(0) += 1;
        }
    }
    task_lists.sort();
    let categories = store.categories().unwrap().into_iter().map(|c| format!("{}:{}", c.name, c.color)).collect();
    Digest {
        accounts: store.accounts().unwrap().into_iter().map(|a| (a.kind, a.email)).collect(),
        folders,
        messages,
        bodies,
        attachments_with_data,
        drafts,
        calendars,
        events,
        exceptions,
        contacts,
        contact_lists,
        task_lists,
        tasks,
        categories,
        sweep_rules,
        special_folders,
    }
}

#[test]
fn round_trip_into_a_fresh_store() {
    let src = demo();
    enrich(&src);
    let dir = TempDir::new();
    let written = create_backup_with(&src, &dir.0, PW, now(), 12, TEST_KDF).unwrap();
    assert_eq!(written.path.file_name().unwrap(), "axentra-outlook-backup-20261019-143005.axobk");
    assert_eq!(written.size, std::fs::metadata(&written.path).unwrap().len());
    assert!(written.counts.messages >= 20 && written.counts.drafts >= 1 && written.counts.events > 0);
    // The partial file is gone and the backup is private to the user.
    let names: Vec<String> = std::fs::read_dir(&dir.0).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(names, vec!["axentra-outlook-backup-20261019-143005.axobk".to_string()]);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&written.path).unwrap().permissions().mode() & 0o777, 0o600);
    }

    let payload = open_backup(&written.path, PW).unwrap();
    assert_eq!(payload.manifest.format, "axentra-outlook-backup");
    assert_eq!(payload.manifest.format_version, 1);
    assert_eq!(payload.manifest.created_at, now());
    assert_eq!(payload.manifest.counts, written.counts);

    let dst = Store::open_in_memory().unwrap();
    let p = preview(&payload, Some(&dst));
    assert_eq!(p.accounts.len(), 1);
    assert!(p.accounts[0].complete && !p.accounts[0].exists);
    let summary = restore(&dst, &payload, RestoreOptions { settings: true }, now()).unwrap();
    assert_eq!(summary.added.accounts, 1);
    assert!(summary.needs_sign_in.is_empty(), "local accounts need no sign-in");
    assert_eq!(summary.added.messages, written.counts.messages);
    assert_eq!(summary.added.drafts, written.counts.drafts);
    assert_eq!(summary.added.events, written.counts.events);
    assert_eq!(summary.added.contacts, written.counts.contacts);
    assert_eq!(summary.added.tasks, written.counts.tasks);

    let expected = digest(&src);
    // The demo data exercises every part of the comparison.
    assert!(expected.folders.iter().any(|(n, _, parent, _)| n == "Q4 Launch" && parent.as_deref() == Some("Projects")));
    assert!(!expected.attachments_with_data.is_empty() && !expected.drafts.is_empty() && expected.bodies >= 20);
    assert!(expected.events.len() > 3 && expected.contacts.iter().any(|(_, f)| !f.is_empty()));
    assert!(!expected.contact_lists.is_empty() && expected.task_lists.len() > 1 && expected.tasks.len() > 3);
    assert_eq!(expected.sweep_rules, 1);
    assert_eq!(digest(&dst), expected);

    // Settings came back, except where this device saves backups.
    let s = dst.settings();
    assert_eq!(s.signature_html, "<b>Alex</b>");
    assert_eq!(s.density, Density::Compact);
    assert_eq!(s.backup_dir, None);
    // Preferences and account-scoped keys moved to the new account id.
    let acct = primary(&dst);
    assert_eq!(dst.kv_get(&format!("shell.ribbon.{acct}.mail_home")).unwrap().as_deref(), Some(r#"{"groups":[]}"#));
    assert_eq!(dst.kv_get("calendar.day_count").unwrap().as_deref(), Some("3"));
    assert_eq!(dst.kv_get("shell.notifications").unwrap(), None, "not on the allowlist");
    assert!(dst.is_sender_blocked("spam@junk.example").unwrap());
    assert!(dst.is_safe_sender("friend@safe.example").unwrap());
    // The restored draft still opens with its stored attachment.
    let draft_id = dst.account_messages(acct).unwrap().into_iter().find(|m| m.subject == "Unsent plan").unwrap().id;
    let draft = dst.draft(draft_id).unwrap().unwrap();
    assert_eq!(dst.draft_attachment_data(&draft.attachments[0]).unwrap().unwrap(), b"step one, step two");
    // Search works on restored mail (the full-text index was filled).
    let found = dst.messages(&MessageQuery { search: Some("Unsent".into()), ..Default::default() }).unwrap();
    assert_eq!(found.len(), 1);
}

#[test]
fn restoring_twice_adds_nothing_the_second_time() {
    let src = demo();
    enrich(&src);
    let dir = TempDir::new();
    let w = create_backup_with(&src, &dir.0, PW, now(), 12, TEST_KDF).unwrap();
    let payload = open_backup(&w.path, PW).unwrap();

    let dst = Store::open_in_memory().unwrap();
    let first = restore(&dst, &payload, RestoreOptions::default(), now()).unwrap();
    assert!(first.added.total_items() > 0);
    let after_first = digest(&dst);
    let second = restore(&dst, &payload, RestoreOptions::default(), now()).unwrap();
    assert_eq!(second.added, Counts::default(), "{second:?}");
    assert_eq!(second.skipped.accounts, 1);
    assert_eq!(second.skipped.messages + second.skipped.drafts, first.added.messages + first.added.drafts);
    assert_eq!(digest(&dst), after_first);

    // Restoring into the store the backup came from adds nothing either.
    let before = digest(&src);
    let back = restore(&src, &payload, RestoreOptions::default(), now()).unwrap();
    assert_eq!(back.added, Counts::default(), "{back:?}");
    assert_eq!(digest(&src), before);
}

#[test]
fn restore_brings_back_what_was_deleted_and_keeps_the_rest() {
    let store = demo();
    let dir = TempDir::new();
    let w = create_backup_with(&store, &dir.0, PW, now(), 12, TEST_KDF).unwrap();
    let before = digest(&store);
    let contact = store.contacts().unwrap()[0].clone();
    store.delete_contact(contact.id).unwrap();
    let task = store.all_open_tasks().unwrap()[0].clone();
    store.delete_task(task.id).unwrap();
    let payload = open_backup(&w.path, PW).unwrap();
    let s = restore(&store, &payload, RestoreOptions::default(), now()).unwrap();
    assert_eq!((s.added.contacts, s.added.tasks), (1, 1));
    assert_eq!(s.added.messages + s.added.events, 0);
    assert_eq!(digest(&store), before);
}

#[test]
fn wrong_password_and_tampering_are_refused() {
    let store = demo();
    let dir = TempDir::new();
    let w = create_backup_with(&store, &dir.0, PW, now(), 12, TEST_KDF).unwrap();
    let err = open_backup(&w.path, "not the password").unwrap_err();
    assert!(matches!(err, BackupError::CantOpen));
    assert_eq!(err.to_string(), "This backup couldn't be opened. Check the password.");

    let bytes = std::fs::read(&w.path).unwrap();
    // A header byte (inside the salt) and a body byte.
    for i in [30, bytes.len() / 2] {
        let mut bad = bytes.clone();
        bad[i] ^= 0x40;
        assert!(matches!(open_bytes(&bad, PW), Err(BackupError::CantOpen)), "byte {i}");
    }
    let mut truncated = bytes.clone();
    truncated.truncate(bytes.len() - 1);
    assert!(matches!(open_bytes(&truncated, PW), Err(BackupError::CantOpen)));
    assert!(matches!(open_bytes(b"hello world, not a backup", PW), Err(BackupError::NotABackup)));
    assert_eq!(BackupError::NotABackup.to_string(), "This file isn't an Axentra Outlook backup.");
}

#[test]
fn newer_versions_are_reported() {
    let store = demo();
    let dir = TempDir::new();
    let w = create_backup_with(&store, &dir.0, PW, now(), 12, TEST_KDF).unwrap();
    let mut bytes = std::fs::read(&w.path).unwrap();
    bytes[7] = 2;
    let err = open_bytes(&bytes, PW).unwrap_err();
    assert!(matches!(err, BackupError::NewerVersion));
    assert_eq!(err.to_string(), "This backup was made by a newer version of Axentra Outlook.");

    // A newer document inside a version-1 container is refused as well.
    let mut payload = collect(&store, now()).unwrap();
    payload.manifest.format_version = 7;
    let sealed = format::seal(&serde_json::to_vec(&payload).unwrap(), PW, TEST_KDF).unwrap();
    assert!(matches!(open_bytes(&sealed, PW), Err(BackupError::NewerVersion)));
    assert!(matches!(restore(&store, &payload, RestoreOptions::default(), now()), Err(BackupError::NewerVersion)));
    // And an unrelated encrypted JSON document isn't a backup.
    let sealed = format::seal(br#"{"manifest":{"format":"something-else"}}"#, PW, TEST_KDF).unwrap();
    assert!(matches!(open_bytes(&sealed, PW), Err(BackupError::NotABackup)));
}

#[test]
fn short_passwords_are_refused() {
    let store = demo();
    let dir = TempDir::new();
    assert!(matches!(create_backup_with(&store, &dir.0, "short", now(), 12, TEST_KDF), Err(BackupError::WeakPassword)));
    assert!(list_backups(&dir.0).unwrap().is_empty());
}

/// An IMAP account with a server mailbox and some device-only items.
fn add_imap_account(store: &Store) -> AccountId {
    let mut a = Account {
        kind: AccountKind::Imap,
        display_name: "Ana Work".into(),
        email: "ana@work.example".into(),
        color: "#c50f1f".into(),
        config: serde_json::json!({
            "uuid": "keyring-uuid-1",
            "provider": "imap",
            "imap": { "host": "imap.work.example", "port": 993, "username": "ana" },
            "password": "S3CRET-PW",
            "oauth": { "refresh_token": "S3CRET-RT", "AccessToken": "S3CRET-AT", "scopes": ["mail"] },
            "nested": [ { "client_secret": "S3CRET-CS", "keep": "visible-value" } ],
            "cookie_jar": "S3CRET-CK",
            "state": "ok",
        }),
        ..Default::default()
    };
    let id = store.add_account(&mut a).unwrap();
    // Mail that lives on the server is not backed up...
    let inbox = store.folder_by_role(id, FolderRole::Inbox).unwrap().unwrap().id;
    let mut server = Message {
        account_id: id,
        folder_id: inbox,
        remote_id: Some("uid-1".into()),
        internet_message_id: "server-1@work.example".into(),
        subject: "SERVER-MAIL".into(),
        received_at: now(),
        ..Default::default()
    };
    store.insert_message(&mut server, None).unwrap();
    // ...but an unsent draft is.
    let mut d = store.new_draft(ComposeKind::New, Some(id), None).unwrap();
    d.subject = "Device-only draft".into();
    store.save_draft(&mut d).unwrap();
    // Server calendars only contribute their local-only events.
    let server_cal = store.calendars().unwrap().into_iter().find(|c| c.account_id == id).unwrap();
    let server_cal = Calendar { remote_id: Some("cal-remote".into()), ..server_cal };
    store.update_calendar(&server_cal).unwrap();
    let ev = |title: &str, remote: Option<&str>, cal: CalendarId| Event {
        calendar_id: cal,
        title: title.into(),
        remote_id: remote.map(Into::into),
        start: now(),
        end: now() + Duration::hours(1),
        ..Default::default()
    };
    store.insert_event(&mut ev("SERVER-EVENT", Some("ev-1"), server_cal.id)).unwrap();
    store.insert_event(&mut ev("Unsynced event", None, server_cal.id)).unwrap();
    let local_cal = store.create_calendar(id, "Ana's local calendar", "#13a10e", "My calendars").unwrap();
    store.insert_event(&mut ev("Local calendar event", None, local_cal)).unwrap();
    store.insert_contact(&mut Contact { account_id: id, display_name: "Server Person".into(), remote_id: Some("c-1".into()), ..Default::default() }).unwrap();
    store
        .insert_contact(&mut Contact {
            account_id: id,
            display_name: "Local Person".into(),
            emails: vec![LabeledValue { label: "Work".into(), value: "local.person@x.example".into() }],
            ..Default::default()
        })
        .unwrap();
    // Sync state with secrets in it never leaves the device.
    store.set_sync_cursor(id, "delta", Some("S3CRET-CURSOR")).unwrap();
    store.sync_put(id, "message", server.id, "uid-1", "INBOX", &serde_json::json!({ "etag": "S3CRET-ITEM" })).unwrap();
    store.kv_set("sync.token.cache", "S3CRET-KV").unwrap();
    store.kv_set("todo.prefs.access_token", "S3CRET-KV2").unwrap();
    store.kv_set(&format!("shell.ribbon.{id}.refresh_token"), "S3CRET-KV3").unwrap();
    id
}

#[test]
fn secrets_and_server_data_never_reach_the_payload() {
    let store = demo();
    add_imap_account(&store);
    let plain = plaintext(&store, now()).unwrap();
    let text = String::from_utf8(plain.to_vec()).unwrap();
    assert!(!text.contains("S3CRET"), "a secret leaked into the backup");
    assert!(!text.contains("SERVER-MAIL") && !text.contains("SERVER-EVENT") && !text.contains("Server Person"));
    assert!(text.contains("Device-only draft") && text.contains("Unsynced event") && text.contains("Local calendar event"));
    assert!(text.contains("Local Person") && text.contains("imap.work.example") && text.contains("visible-value"));

    let payload = collect(&store, now()).unwrap();
    let imap = payload.accounts.iter().find(|a| a.account.kind == AccountKind::Imap).unwrap();
    assert!(!imap.complete && imap.folders.is_empty());
    assert_eq!(imap.account.config["oauth"], serde_json::json!({ "scopes": ["mail"] }));
    assert_eq!(imap.account.config["nested"], serde_json::json!([{ "keep": "visible-value" }]));
    let server_cal = imap.calendars.iter().find(|c| c.calendar.remote_id.is_some()).unwrap();
    assert!(server_cal.container_only);
    assert_eq!(server_cal.events.len(), 1);
}

#[test]
fn strip_secrets_handles_nesting_and_case() {
    let v = serde_json::json!({
        "Password": 1, "smtp_password": 2, "refreshToken": 3, "x": { "ACCESS": 4, "y": [ { "SessionCookie": 5, "ok": 6 } ] },
        "client_secret": 7, "ms_client_id": "id", "host": "h"
    });
    assert_eq!(strip_secrets(&v), serde_json::json!({ "x": { "y": [ { "ok": 6 } ] }, "ms_client_id": "id", "host": "h" }));
    assert!(kv_allowed("todo.groups") && kv_allowed("people.ui.sort") && kv_allowed("todo.prefs.views"));
    assert!(!kv_allowed("todo.groupsx") && !kv_allowed("sync.cursor") && !kv_allowed("backup.last") && !kv_allowed("todo.prefs.token"));
    assert!(!kv_allowed("backup.written"), "this device's own file list stays here");
}

#[test]
fn synced_accounts_come_back_needing_sign_in_with_a_new_keyring_id() {
    let store = demo();
    add_imap_account(&store);
    let payload = collect(&store, now()).unwrap();

    let dst = Store::open_in_memory().unwrap();
    let s = restore(&dst, &payload, RestoreOptions::default(), now()).unwrap();
    assert_eq!(s.needs_sign_in, vec!["ana@work.example".to_string()]);
    let acct = dst.accounts().unwrap().into_iter().find(|a| a.kind == AccountKind::Imap).unwrap();
    assert_eq!(acct.config["state"], "needs_reauth");
    assert_ne!(acct.config["uuid"], "keyring-uuid-1");
    assert!(acct.config["uuid"].as_str().is_some_and(|u| u.len() >= 32));
    assert_eq!(acct.config["imap"]["host"], "imap.work.example");
    assert!(acct.last_sync_at.is_none());
    let subjects: Vec<String> = dst.account_messages(acct.id).unwrap().into_iter().map(|m| m.subject).collect();
    assert_eq!(subjects, vec!["Device-only draft".to_string()]);
    let drafts = dst.folder_by_role(acct.id, FolderRole::Drafts).unwrap().unwrap();
    assert_eq!(drafts.total_count, 1);
    let titles: Vec<String> = dst
        .calendars()
        .unwrap()
        .into_iter()
        .filter(|c| c.account_id == acct.id)
        .flat_map(|c| dst.calendar_events(c.id).unwrap())
        .map(|e| e.title)
        .collect();
    assert_eq!(titles.len(), 2);
    assert!(titles.contains(&"Unsynced event".to_string()) && titles.contains(&"Local calendar event".to_string()));
    // Restored synced-account items are local until the account syncs.
    assert!(dst.calendars().unwrap().iter().all(|c| c.remote_id.is_none()));

    // When the account already exists, its setup and keyring id are left alone.
    let s = restore(&store, &payload, RestoreOptions::default(), now()).unwrap();
    assert!(s.needs_sign_in.is_empty() && s.added.accounts == 0 && s.added.total_items() == 0, "{s:?}");
    let existing = store.accounts().unwrap().into_iter().find(|a| a.kind == AccountKind::Imap).unwrap();
    assert_eq!(existing.config["uuid"], "keyring-uuid-1");
}

#[test]
fn retention_deletes_only_matching_backup_files() {
    let dir = TempDir::new();
    let d = &dir.0;
    let backups = [
        "axentra-outlook-backup-20260101-000000.axobk",
        "axentra-outlook-backup-20260201-000000.axobk",
        "axentra-outlook-backup-20260301-000000.axobk",
        "axentra-outlook-backup-20260401-000000.axobk",
        "axentra-outlook-backup-20260501-120000.axobk",
    ];
    for b in backups {
        std::fs::write(d.join(b), b"x").unwrap();
    }
    let decoys = [
        "axentra-outlook-backup-20250101-000000.axobk.partial",
        "notes.txt",
        "axentra-outlook-backup-2025.axobk",
        "AXENTRA-OUTLOOK-BACKUP-20250101-000000.AXOBK",
        "axentra-outlook-backup-20251301-000000.axobk",
        "axentra-outlook-backup-20250101-000000.axobk.bak",
        "my-axentra-outlook-backup-20250101-000000.axobk",
        "axentra-outlook-backup-2025010-1000000.axobk",
    ];
    for x in decoys {
        std::fs::write(d.join(x), b"keep me").unwrap();
    }
    // A folder with a backup's name is never removed either.
    std::fs::create_dir(d.join("axentra-outlook-backup-20240101-000000.axobk")).unwrap();

    let listed: Vec<String> = list_backups(d).unwrap().into_iter().map(|b| b.name).collect();
    assert_eq!(listed, backups.iter().rev().map(|s| s.to_string()).collect::<Vec<_>>());

    let newest = d.join(backups[4]);
    let deleted = apply_retention(d, 2, &newest, |_| true).unwrap();
    assert_eq!(deleted.len(), 3);
    for (i, b) in backups.iter().enumerate() {
        assert_eq!(d.join(b).exists(), i >= 3, "{b}");
    }
    for x in decoys {
        assert!(d.join(x).exists(), "{x} was touched");
    }
    assert!(d.join("axentra-outlook-backup-20240101-000000.axobk").is_dir());
    // keep = 0 still keeps the one just written.
    apply_retention(d, 0, &newest, |_| true).unwrap();
    assert_eq!(list_backups(d).unwrap().len(), 1);
    // Deleting by hand only works on backup files.
    assert!(matches!(delete_backup(&d.join("notes.txt")), Err(BackupError::NotABackup)));
    assert!(d.join("notes.txt").exists());
    delete_backup(&d.join(backups[4])).unwrap();
    assert!(list_backups(d).unwrap().is_empty());
    // A missing folder lists nothing.
    assert!(list_backups(&d.join("missing")).unwrap().is_empty());
}

#[test]
fn backups_apply_retention_and_get_unique_names() {
    let store = demo();
    let dir = TempDir::new();
    std::fs::write(dir.0.join("unrelated.pdf"), b"%PDF").unwrap();
    for _ in 0..3 {
        create_backup_with(&store, &dir.0, PW, now(), 2, TEST_KDF).unwrap();
    }
    let names: Vec<String> = list_backups(&dir.0).unwrap().into_iter().map(|b| b.name).collect();
    // Same second three times: the names step forward, the oldest went.
    assert_eq!(names, vec!["axentra-outlook-backup-20261019-143007.axobk", "axentra-outlook-backup-20261019-143006.axobk"]);
    assert!(dir.0.join("unrelated.pdf").exists());
}

#[test]
fn file_names_round_trip() {
    let t = now();
    assert_eq!(parse_file_name(&file_name(t)), Some(t));
    assert_eq!(parse_file_name("axentra-outlook-backup-20261019-143005.axobk"), Some(t));
    assert_eq!(parse_file_name("axentra-outlook-backup-20261019-253005.axobk"), None);
    assert_eq!(parse_file_name("axentra-outlook-backup-+2026101-143005.axobk"), None);
}

#[test]
fn due_date_logic_with_an_injected_clock() {
    let t0 = now();
    // Never backed up: due right away, if automatic backups can run.
    assert!(is_due(true, true, None, t0));
    assert!(!is_due(false, true, None, t0));
    assert!(!is_due(true, false, None, t0), "no password available, nothing runs unattended");
    // Monthly.
    let last = Some(t0);
    assert!(!is_due(true, true, last, t0 + Duration::days(29) + Duration::hours(23)));
    assert!(is_due(true, true, last, t0 + Duration::days(30)));
    assert!(is_due(true, true, last, t0 + Duration::days(400)));
    // A clock that went back a little doesn't trigger a backup...
    assert!(!is_due(true, true, last, t0 - Duration::hours(3)));
    assert_eq!(next_due(last, t0), t0 + Duration::days(30));
    assert_eq!(next_due(None, t0), t0);
    // ...but a last success dated well in the future (recorded while the
    // clock was ahead) doesn't hold backups off until then.
    let ahead = Some(t0 + Duration::days(365));
    assert!(is_due(true, true, ahead, t0));
    assert!(is_due(true, true, last, t0 - Duration::days(3)));
    assert_eq!(next_due(ahead, t0), t0);
}

#[test]
fn status_is_remembered_and_errors_toast_once_a_day() {
    let store = demo();
    assert!(last_backup(&store).is_none());
    let day = NaiveDate::from_ymd_opt(2026, 10, 19).unwrap();
    assert!(record_failure(&store, now(), "disk full", day).unwrap());
    assert!(!record_failure(&store, now() + Duration::hours(1), "disk full", day).unwrap());
    assert!(record_failure(&store, now() + Duration::days(1), "disk full", day.succ_opt().unwrap()).unwrap());
    assert_eq!(last_failure(&store).unwrap().message, "disk full");
    let w = Written { path: "/x/y.axobk".into(), size: 42, created_at: now(), counts: Counts::default(), removed: 0 };
    record_success(&store, &w).unwrap();
    assert_eq!(last_backup(&store).unwrap(), LastBackup { at: now(), size: 42, path: "/x/y.axobk".into() });
    assert!(last_failure(&store).is_none());
    // The status itself is never part of a backup.
    let text = String::from_utf8(plaintext(&store, now()).unwrap().to_vec()).unwrap();
    assert!(!text.contains("/x/y.axobk"));
}

#[test]
fn password_hints() {
    assert_eq!(password_problem("short", "short"), Some("Use at least 8 characters."));
    assert_eq!(password_problem("longenough", "longenougH"), Some("The passwords don't match."));
    assert_eq!(password_problem("longenough", "longenough"), None);
    assert_eq!(password_strength("abc"), 0);
    assert_eq!(password_strength("aaaaaaaaaa"), 1);
    assert_eq!(password_strength("abcdefgh"), 1);
    assert_eq!(password_strength("abcdefgh12"), 2);
    assert_eq!(password_strength("Tr0ub4dor&3x"), 3);
    assert_eq!(password_strength("correct horse battery staple"), 3);
}

#[test]
fn old_settings_rows_get_backup_defaults() {
    let s: Settings = serde_json::from_str(r#"{"theme":"Dark"}"#).unwrap();
    assert!(s.backup_auto);
    assert_eq!((s.backup_dir, s.backup_keep), (None, DEFAULT_KEEP));
}

#[test]
fn restored_drafts_are_never_scheduled_or_queued_to_send() {
    let t0 = now();
    let store = demo();
    let imap = add_imap_account(&store);
    let boss = EmailAddress::new("Boss", "boss@work.example");
    let mut d = store.new_draft(ComposeKind::New, Some(imap), None).unwrap();
    d.subject = "Quarterly numbers".into();
    d.to = vec![boss.clone()];
    store.schedule_send(&mut d, t0 + Duration::hours(1)).unwrap();
    // A local account's message waiting out "Undo send" in Outbox.
    let local = primary(&store);
    let mut q = store.new_draft(ComposeKind::New, Some(local), None).unwrap();
    q.subject = "Held for undo".into();
    q.to = vec![boss];
    store.queue_send(&mut q, t0 + Duration::seconds(10)).unwrap();
    let payload = collect(&store, t0).unwrap();

    // The scheduled message goes out as planned.
    assert_eq!(store.send_due_scheduled(t0 + Duration::hours(2)).unwrap().len(), 1);
    let later = t0 + Duration::days(20);
    assert!(store.due_scheduled_drafts(later).unwrap().is_empty());

    // Restoring weeks later brings the draft back, but not its schedule.
    let s = restore(&store, &payload, RestoreOptions::default(), later).unwrap();
    assert_eq!((s.added.drafts, s.unscheduled), (1, 1), "{s:?}");
    assert!(store.due_scheduled_drafts(later).unwrap().is_empty());
    assert!(store.send_due_scheduled(later).unwrap().is_empty(), "sent once, never again");
    let back = store.account_messages(imap).unwrap().into_iter().find(|m| m.is_draft && m.subject == "Quarterly numbers").unwrap();
    assert_eq!(back.scheduled_send_at, None);
    assert_eq!(back.folder_id, store.folder_by_role(imap, FolderRole::Drafts).unwrap().unwrap().id);
    let draft = store.draft(back.id).unwrap().unwrap();
    assert_eq!((draft.scheduled_send_at, draft.send_pending_until), (None, None));

    // A new device (or a second one) doesn't send anything either.
    let dst = Store::open_in_memory().unwrap();
    let s = restore(&dst, &payload, RestoreOptions::default(), later).unwrap();
    assert_eq!(s.unscheduled, 2);
    let far = later + Duration::days(3650);
    assert!(dst.due_scheduled_drafts(far).unwrap().is_empty());
    assert!(dst.overdue_queued_sends(far).unwrap().is_empty());
    let new_local = dst.accounts().unwrap().into_iter().find(|a| a.kind == AccountKind::Local).unwrap().id;
    let held = dst.account_messages(new_local).unwrap().into_iter().find(|m| m.subject == "Held for undo").unwrap();
    assert!(held.is_draft);
    assert_eq!(held.folder_id, dst.folder_by_role(new_local, FolderRole::Drafts).unwrap().unwrap().id);
    assert_eq!(dst.draft(held.id).unwrap().unwrap().send_pending_until, None);
}

#[test]
fn a_draft_edited_since_the_backup_is_not_restored_twice() {
    let store = demo();
    let acct = primary(&store);
    let mut d = store.new_draft(ComposeKind::New, Some(acct), None).unwrap();
    d.subject = "Plan".into();
    d.body_html = "<p>v1</p>".into();
    let id = store.save_draft(&mut d).unwrap();
    let created = d.created_at.expect("set on the first save");
    let mut payload = collect(&store, now()).unwrap();
    // Every save moves the message's time: the edit below happens an hour
    // after the backup.
    for md in payload.accounts.iter_mut().flat_map(|a| a.messages.iter_mut()).filter(|m| m.message.subject == "Plan") {
        md.message.received_at -= Duration::hours(1);
    }
    d.body_html = "<p>v2</p>".into();
    store.save_draft(&mut d).unwrap();
    assert_eq!(d.created_at, Some(created));
    // A copy without the field (opened before it existed) keeps it too.
    let mut stale = store.draft(id).unwrap().unwrap();
    stale.created_at = None;
    store.save_draft(&mut stale).unwrap();
    assert_eq!(stale.created_at, Some(created));

    let s = restore(&store, &payload, RestoreOptions::default(), now()).unwrap();
    assert_eq!(s.added.drafts, 0, "{s:?}");
    assert_eq!(store.account_messages(acct).unwrap().iter().filter(|m| m.subject == "Plan").count(), 1);
}

#[test]
fn items_the_sync_engine_pushed_are_not_device_only() {
    let store = Store::open_in_memory().unwrap();
    let mut a = Account {
        kind: AccountKind::Microsoft,
        display_name: "Ana".into(),
        email: "ana@contoso.example".into(),
        config: serde_json::json!({ "uuid": "keyring-uuid" }),
        ..Default::default()
    };
    let id = store.add_account(&mut a).unwrap();
    // Like the Graph engine: items it pulled or pushed get a sync record,
    // pushed ones without a remote id on the row itself.
    let synced = |kind: &str, local: i64, remote: &str| {
        store.sync_put(id, kind, local, remote, "c", &serde_json::json!({ "remote_id": remote })).unwrap();
    };
    let cal = store.calendars().unwrap().into_iter().find(|c| c.account_id == id).unwrap();
    synced("calendar", cal.id, "cal-1");
    let list = store.default_task_list(Some(id)).unwrap().unwrap();
    synced("task_list", list.id, "list-1");
    let event = |cal: CalendarId, title: &str| Event { calendar_id: cal, title: title.into(), start: now(), end: now() + Duration::hours(1), ..Default::default() };
    let pushed = store.insert_event(&mut event(cal.id, "PUSHED-MEETING")).unwrap();
    synced("event", pushed, "ev-1");
    store.insert_event(&mut event(cal.id, "Not pushed yet")).unwrap();
    let local_cal = store.create_calendar(id, "Only here", "#13a10e", "My calendars").unwrap();
    store.insert_event(&mut event(local_cal, "On the local calendar")).unwrap();
    let pushed = store.insert_contact(&mut Contact { account_id: id, display_name: "PUSHED-CONTACT".into(), ..Default::default() }).unwrap();
    synced("contact", pushed, "c-1");
    store.insert_contact(&mut Contact { account_id: id, display_name: "Offline contact".into(), ..Default::default() }).unwrap();
    let pushed = store.insert_task(&mut Task { list_id: list.id, title: "PUSHED-TASK".into(), ..Default::default() }).unwrap();
    synced("task", pushed, "t-1");
    store.insert_task(&mut Task { list_id: list.id, title: "Offline task".into(), ..Default::default() }).unwrap();

    let payload = collect(&store, now()).unwrap();
    let text = serde_json::to_string(&payload).unwrap();
    assert!(!text.contains("PUSHED-"), "already on the server, so not device-only");
    for kept in ["Not pushed yet", "On the local calendar", "Offline contact", "Offline task"] {
        assert!(text.contains(kept), "{kept} is missing");
    }
    let data = &payload.accounts[0];
    assert!(data.calendars.iter().find(|c| c.calendar.id == cal.id).unwrap().container_only);
    assert!(!data.calendars.iter().find(|c| c.calendar.id == local_cal).unwrap().container_only);
    assert!(data.task_lists.iter().all(|l| l.container_only));

    // On a new device only the device-only items come back, so the first
    // sync creates nothing a second time on the server.
    let dst = Store::open_in_memory().unwrap();
    let s = restore(&dst, &payload, RestoreOptions::default(), now()).unwrap();
    assert_eq!((s.added.events, s.added.contacts, s.added.tasks), (2, 1, 1), "{s:?}");
}

#[test]
fn a_new_account_gets_the_backed_up_look_of_its_default_containers() {
    let src = demo();
    let acct = primary(&src);
    let mut cal = src.calendars().unwrap().into_iter().find(|c| c.account_id == acct && c.is_default).unwrap();
    cal.name = "Family".into();
    cal.color = "#c50f1f".into();
    cal.group = "Home".into();
    src.update_calendar(&cal).unwrap();
    let archive = src.folder_by_role(acct, FolderRole::Archive).unwrap().unwrap();
    let inbox = src.folder_by_role(acct, FolderRole::Inbox).unwrap().unwrap();
    src.set_folder_favorite(archive.id, true).unwrap();
    src.set_folder_favorite(inbox.id, false).unwrap();
    let mut list = src.default_task_list(Some(acct)).unwrap().unwrap();
    list.emoji = Some("🏠".into());
    list.group = Some("Home".into());
    src.update_task_list(&list).unwrap();
    let payload = collect(&src, now()).unwrap();

    let dst = Store::open_in_memory().unwrap();
    restore(&dst, &payload, RestoreOptions::default(), now()).unwrap();
    let new_acct = primary(&dst);
    let c = dst.calendars().unwrap().into_iter().find(|c| c.account_id == new_acct && c.is_default).unwrap();
    assert_eq!((c.name.as_str(), c.color.as_str(), c.group.as_str()), ("Family", "#c50f1f", "Home"));
    assert!(dst.folder_by_role(new_acct, FolderRole::Archive).unwrap().unwrap().is_favorite);
    assert!(!dst.folder_by_role(new_acct, FolderRole::Inbox).unwrap().unwrap().is_favorite);
    let l = dst.default_task_list(Some(new_acct)).unwrap().unwrap();
    assert_eq!((l.emoji.as_deref(), l.group.as_deref()), (Some("🏠"), Some("Home")));
    assert_eq!(digest(&dst), digest(&src));

    // An account that is already here keeps its own look.
    let mut mine = c.clone();
    mine.color = "#0000ff".into();
    dst.update_calendar(&mine).unwrap();
    restore(&dst, &payload, RestoreOptions::default(), now()).unwrap();
    assert_eq!(dst.calendars().unwrap().into_iter().find(|x| x.id == c.id).unwrap().color, "#0000ff");
}

#[test]
fn restored_preferences_point_at_the_restored_lists_and_accounts() {
    use serde_json::{Value, json};
    let src = Store::open_in_memory().unwrap();
    let mut ana = Account { kind: AccountKind::Local, display_name: "Ana".into(), email: "ana@home.example".into(), ..Default::default() };
    let ana_id = src.add_account(&mut ana).unwrap();
    let groceries = src.insert_task_list(&mut TaskList { account_id: ana_id, name: "Groceries".into(), ..Default::default() }).unwrap();
    let views = json!({ format!("list:{groceries}"): { "theme": "red" }, "MyDay": { "theme": "blue" } });
    src.kv_set("todo.prefs", &json!({ "confirm_delete": false, "last_view": { "List": groceries }, "views": views }).to_string()).unwrap();
    src.kv_set("mail.folder_pane", &json!({ "favorites_pinned": true, "collapsed_accounts": [ana_id] }).to_string()).unwrap();
    let payload = collect(&src, now()).unwrap();

    // A new computer where the work account was added first.
    let dst = Store::open_in_memory().unwrap();
    let mut work = Account { kind: AccountKind::Local, display_name: "Work".into(), email: "ana@work.example".into(), ..Default::default() };
    let work_id = dst.add_account(&mut work).unwrap();
    let work_tasks = dst.default_task_list(Some(work_id)).unwrap().unwrap().id;
    dst.kv_set("todo.prefs", &json!({ "views": { format!("list:{work_tasks}"): { "theme": "green" } } }).to_string()).unwrap();
    dst.kv_set("mail.folder_pane", &json!({ "collapsed_accounts": [work_id] }).to_string()).unwrap();
    restore(&dst, &payload, RestoreOptions { settings: true }, now()).unwrap();

    let new_groceries = dst.task_lists().unwrap().into_iter().find(|l| l.name == "Groceries").unwrap().id;
    assert_ne!(new_groceries, groceries, "the test needs the old id to be taken");
    assert!(dst.task_list(groceries).unwrap().is_some_and(|l| l.name == "Tasks"));
    let new_ana = dst.accounts().unwrap().into_iter().find(|a| a.email == "ana@home.example").unwrap().id;
    let prefs: Value = serde_json::from_str(&dst.kv_get("todo.prefs").unwrap().unwrap()).unwrap();
    assert_eq!(prefs["confirm_delete"], false);
    assert_eq!(prefs["last_view"], json!({ "List": new_groceries }));
    assert_eq!(prefs["views"][format!("list:{new_groceries}")]["theme"], "red");
    assert_eq!(prefs["views"][format!("list:{work_tasks}")]["theme"], "green", "the work list keeps its own theme");
    assert_eq!(prefs["views"]["MyDay"]["theme"], "blue");
    assert_eq!(prefs["views"].as_object().unwrap().len(), 3, "{prefs}");
    let pane: Value = serde_json::from_str(&dst.kv_get("mail.folder_pane").unwrap().unwrap()).unwrap();
    assert_eq!(pane["favorites_pinned"], true);
    let mut collapsed: Vec<i64> = pane["collapsed_accounts"].as_array().unwrap().iter().filter_map(Value::as_i64).collect();
    collapsed.sort();
    let mut expected = vec![new_ana, work_id];
    expected.sort();
    assert_eq!(collapsed, expected);
}

#[test]
fn retention_never_deletes_the_new_backup_or_another_computers() {
    let store = demo();
    let dir = TempDir::new();
    // From a computer whose clock was a year ahead, and from another
    // computer that shares the OneDrive folder.
    let future = dir.0.join("axentra-outlook-backup-20271019-000000.axobk");
    let other = dir.0.join("axentra-outlook-backup-20261001-090000.axobk");
    std::fs::write(&future, b"x").unwrap();
    std::fs::write(&other, b"x").unwrap();
    let w = create_backup_with(&store, &dir.0, PW, now(), 1, TEST_KDF).unwrap();
    assert!(w.path.exists(), "the backup just made is kept");
    assert_eq!(w.removed, 0);
    assert!(future.exists() && other.exists(), "not made here, so never deleted");

    // This computer's own backups rotate.
    let w2 = create_backup_with(&store, &dir.0, PW, now() + Duration::days(30), 1, TEST_KDF).unwrap();
    assert!(w2.path.exists() && !w.path.exists());
    assert_eq!(w2.removed, 1);

    // One it wrote while its clock was ahead doesn't push a new one out.
    let ahead = create_backup_with(&store, &dir.0, PW, now() + Duration::days(800), 2, TEST_KDF).unwrap();
    assert_eq!(ahead.removed, 0);
    let w3 = create_backup_with(&store, &dir.0, PW, now() + Duration::days(60), 1, TEST_KDF).unwrap();
    assert!(w3.path.exists());
    assert!(!ahead.path.exists() && !w2.path.exists());
    assert_eq!(w3.removed, 2);
    assert!(future.exists() && other.exists());
}
