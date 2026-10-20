//! End-to-end IMAP round trips against an in-process fake server:
//! greeting → AUTHENTICATE PLAIN → LIST → SELECT → UID SEARCH → UID FETCH,
//! then a local edit pushed with UID STORE and a server-side expunge
//! mirrored locally; and folder creates / renames / deletes made here
//! pushed with CREATE / RENAME / DELETE.

use std::sync::{Arc, Mutex};

use async_imap::Client;
use axentra_core::{Account, AccountKind, FlagStatus, FolderRole, Store};
use base64::Engine as _;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream};

use crate::config::AccountConfig;
use crate::diff::{KIND_MESSAGE, Shadow};
use crate::imap::sync::MailSync;
use crate::imap::{self, Credential, folders};
use crate::notify::Notifier;
use crate::secrets::Secret;

#[derive(Default)]
struct FakeServer {
    /// (uid, flags, raw message) in INBOX.
    inbox: Vec<(u32, Vec<String>, String)>,
    /// Every command line received (tag stripped).
    log: Vec<String>,
    /// Mailbox names (empty = INBOX, "Sent Messages", Trash).
    mailboxes: Vec<String>,
}

impl FakeServer {
    fn boxes(&mut self) -> &mut Vec<String> {
        if self.mailboxes.is_empty() {
            self.mailboxes = vec!["INBOX".into(), "Sent Messages".into(), "Trash".into()];
        }
        &mut self.mailboxes
    }
}

/// The first (possibly quoted) argument of a command and the rest.
fn arg(s: &str) -> (String, &str) {
    let s = s.trim_start();
    if let Some(rest) = s.strip_prefix('"') {
        let end = rest.find('"').unwrap_or(rest.len());
        (rest[..end].to_string(), rest.get(end + 1..).unwrap_or_default())
    } else {
        let end = s.find(' ').unwrap_or(s.len());
        (s[..end].to_string(), &s[end..])
    }
}

fn raw(subject: &str, mid: &str, refs: &str) -> String {
    format!(
        "From: Megan Bowen <megan@contoso.example>\r\nTo: ana@example.org\r\nSubject: {subject}\r\n\
Date: Mon, 19 Oct 2026 09:00:00 +0000\r\nMessage-ID: <{mid}>\r\n{refs}MIME-Version: 1.0\r\n\
Content-Type: text/plain; charset=utf-8\r\n\r\nBody of {subject}\r\n"
    )
}

fn uid_matches(set: &str, uid: u32, max: u32) -> bool {
    set.split(',').any(|part| match part.split_once(':') {
        Some((a, b)) => {
            let a: u32 = a.parse().unwrap_or(0);
            let b: u32 = if b == "*" { max } else { b.parse().unwrap_or(0) };
            (a.min(b)..=a.max(b)).contains(&uid)
        }
        None => part.parse::<u32>().ok() == Some(uid),
    })
}

async fn serve(io: DuplexStream, state: Arc<Mutex<FakeServer>>) {
    let (r, mut w) = tokio::io::split(io);
    let mut lines = BufReader::new(r).lines();
    let caps = "IMAP4rev1 AUTH=PLAIN UIDPLUS MOVE";
    w.write_all(format!("* OK [CAPABILITY {caps}] Fake IMAP ready\r\n").as_bytes()).await.unwrap();
    while let Ok(Some(line)) = lines.next_line().await {
        let Some((tag, cmd)) = line.split_once(' ') else { continue };
        let cmd = cmd.to_string();
        state.lock().unwrap().log.push(cmd.clone());
        let upper = cmd.to_ascii_uppercase();
        let mut out = String::new();
        if upper == "CAPABILITY" {
            out.push_str(&format!("* CAPABILITY {caps}\r\n{tag} OK done\r\n"));
        } else if upper == "AUTHENTICATE PLAIN" {
            w.write_all(b"+ \r\n").await.unwrap();
            let creds = lines.next_line().await.unwrap().unwrap();
            let decoded = base64::engine::general_purpose::STANDARD.decode(creds.trim()).unwrap();
            if decoded == b"\0ana@example.org\0secret" {
                out.push_str(&format!("{tag} OK Logged in\r\n"));
            } else {
                out.push_str(&format!("{tag} NO [AUTHENTICATIONFAILED] Invalid credentials\r\n"));
            }
        } else if upper.starts_with("LIST") {
            for name in state.lock().unwrap().boxes().iter() {
                let attrs = match name.as_str() {
                    "Sent Messages" => "\\HasNoChildren \\Sent",
                    "Trash" => "\\Trash",
                    _ => "\\HasNoChildren",
                };
                out.push_str(&format!("* LIST ({attrs}) \"/\" \"{name}\"\r\n"));
            }
            out.push_str(&format!("{tag} OK LIST completed\r\n"));
        } else if let Some(rest) = cmd.strip_prefix("CREATE ") {
            let (name, _) = arg(rest);
            let mut s = state.lock().unwrap();
            if s.boxes().contains(&name) {
                out.push_str(&format!("{tag} NO [ALREADYEXISTS] Mailbox exists\r\n"));
            } else {
                s.boxes().push(name);
                out.push_str(&format!("{tag} OK CREATE completed\r\n"));
            }
        } else if let Some(rest) = cmd.strip_prefix("RENAME ") {
            let (old, rest) = arg(rest);
            let (new, _) = arg(rest);
            let mut s = state.lock().unwrap();
            let prefix = format!("{old}/");
            let mut found = false;
            for b in s.boxes().iter_mut() {
                if *b == old {
                    *b = new.clone();
                    found = true;
                } else if let Some(tail) = b.strip_prefix(&prefix) {
                    *b = format!("{new}/{tail}");
                }
            }
            out.push_str(&if found { format!("{tag} OK RENAME completed\r\n") } else { format!("{tag} NO [NONEXISTENT] No such mailbox\r\n") });
        } else if let Some(rest) = cmd.strip_prefix("DELETE ") {
            let (name, _) = arg(rest);
            let mut s = state.lock().unwrap();
            let before = s.boxes().len();
            s.boxes().retain(|b| *b != name);
            out.push_str(&if s.boxes().len() < before { format!("{tag} OK DELETE completed\r\n") } else { format!("{tag} NO [NONEXISTENT] No such mailbox\r\n") });
        } else if upper.starts_with("SUBSCRIBE ") {
            out.push_str(&format!("{tag} OK SUBSCRIBE completed\r\n"));
        } else if upper.starts_with("SELECT") || upper.starts_with("EXAMINE") {
            let s = state.lock().unwrap();
            let next = s.inbox.iter().map(|m| m.0).max().unwrap_or(0) + 1;
            out.push_str(&format!(
                "* FLAGS (\\Seen \\Flagged \\Deleted \\Draft)\r\n* {} EXISTS\r\n* OK [UIDVALIDITY 7] UIDs valid\r\n* OK [UIDNEXT {next}] next\r\n\
* OK [PERMANENTFLAGS (\\Seen \\Flagged \\Deleted \\*)] ok\r\n{tag} OK [READ-WRITE] SELECT completed\r\n",
                s.inbox.len()
            ));
        } else if let Some(q) = upper.strip_prefix("UID SEARCH ") {
            let s = state.lock().unwrap();
            let max = s.inbox.iter().map(|m| m.0).max().unwrap_or(0);
            let hits: Vec<String> = s
                .inbox
                .iter()
                .filter(|(uid, flags, _)| {
                    if let Some(set) = q.strip_prefix("UID ") {
                        let set = set.split(' ').next().unwrap_or_default();
                        let deleted_only = q.ends_with(" DELETED");
                        uid_matches(set, *uid, max) && (!deleted_only || flags.iter().any(|f| f == "\\Deleted"))
                    } else {
                        !q.contains("DELETED")
                    }
                })
                .map(|(uid, _, _)| uid.to_string())
                .collect();
            out.push_str(&format!("* SEARCH {}\r\n{tag} OK SEARCH completed\r\n", hits.join(" ")).replace("SEARCH \r\n", "SEARCH\r\n"));
        } else if let Some(rest) = cmd.strip_prefix("UID FETCH ") {
            let (set, items) = rest.split_once(' ').unwrap();
            let s = state.lock().unwrap();
            let max = s.inbox.iter().map(|m| m.0).max().unwrap_or(0);
            for (i, (uid, flags, body)) in s.inbox.iter().enumerate() {
                if !uid_matches(set, *uid, max) {
                    continue;
                }
                let seq = i + 1;
                let flags = flags.join(" ");
                if items.contains("BODY.PEEK[]") {
                    out.push_str(&format!(
                        "* {seq} FETCH (UID {uid} FLAGS ({flags}) INTERNALDATE \"19-Oct-2026 09:00:00 +0000\" RFC822.SIZE {} BODY[] {{{}}}\r\n{body})\r\n",
                        body.len(),
                        body.len()
                    ));
                } else if items.contains("RFC822.SIZE") {
                    out.push_str(&format!("* {seq} FETCH (UID {uid} RFC822.SIZE {})\r\n", body.len()));
                } else {
                    out.push_str(&format!("* {seq} FETCH (UID {uid} FLAGS ({flags}))\r\n"));
                }
            }
            out.push_str(&format!("{tag} OK FETCH completed\r\n"));
        } else if let Some(rest) = cmd.strip_prefix("UID STORE ") {
            let mut parts = rest.splitn(3, ' ');
            let (set, op, flags) = (parts.next().unwrap(), parts.next().unwrap(), parts.next().unwrap());
            let flags: Vec<String> = flags.trim_matches(['(', ')']).split(' ').map(str::to_string).collect();
            let mut s = state.lock().unwrap();
            let max = s.inbox.iter().map(|m| m.0).max().unwrap_or(0);
            for (uid, f, _) in s.inbox.iter_mut() {
                if uid_matches(set, *uid, max) {
                    if op.starts_with('+') {
                        for fl in &flags {
                            if !f.contains(fl) {
                                f.push(fl.clone());
                            }
                        }
                    } else {
                        f.retain(|x| !flags.contains(x));
                    }
                }
            }
            out.push_str(&format!("{tag} OK STORE completed\r\n"));
        } else if upper == "LOGOUT" {
            out.push_str(&format!("* BYE\r\n{tag} OK LOGOUT completed\r\n"));
            w.write_all(out.as_bytes()).await.unwrap();
            break;
        } else {
            out.push_str(&format!("{tag} BAD unknown command\r\n"));
        }
        w.write_all(out.as_bytes()).await.unwrap();
    }
}

async fn connect(state: &Arc<Mutex<FakeServer>>, password: &str) -> crate::error::SyncResult<imap::Imap<DuplexStream>> {
    let (client_io, server_io) = tokio::io::duplex(1 << 20);
    tokio::spawn(serve(server_io, state.clone()));
    let mut client = Client::new(client_io);
    let caps = imap::read_greeting(&mut client, "fake").await?;
    imap::login(client, caps, "fake", "ana@example.org", &Credential::Password(Secret::new(password))).await
}

fn account(store: &Store) -> Account {
    let mut a = Account { kind: AccountKind::Imap, email: "ana@example.org".into(), display_name: "Ana".into(), ..Default::default() };
    let cfg = AccountConfig { days_to_sync: 3650, ..Default::default() };
    cfg.store_into(&mut a);
    store.add_account(&mut a).unwrap();
    a
}

#[tokio::test]
async fn imap_round_trip_against_fake_server() {
    let store = Store::open_in_memory().unwrap();
    let acct = account(&store);
    let cfg = AccountConfig::of(&acct);
    let notifier = Notifier::default();
    let state = Arc::new(Mutex::new(FakeServer {
        inbox: vec![
            (1, vec!["\\Seen".into()], raw("Budget", "root@contoso.example", "")),
            (2, vec!["\\Flagged".into()], raw("RE: Budget", "reply@contoso.example", "In-Reply-To: <root@contoso.example>\r\nReferences: <root@contoso.example>\r\n")),
        ],
        ..Default::default()
    }));

    // Wrong password: the server says NO and we report an auth error.
    assert!(matches!(connect(&state, "nope").await, Err(crate::error::SyncError::Auth(_))));

    // First sync: LIST → roles, SELECT → FETCH bodies.
    let mut session = connect(&state, "secret").await.unwrap();
    let mut ms = MailSync::new(&store, &acct, &cfg, &notifier).unwrap();
    let remote = folders::list(&mut session).await.unwrap();
    let mapped = ms.map_folders(&remote).unwrap();
    let inbox = mapped.iter().find(|m| m.remote.role == Some(FolderRole::Inbox)).unwrap().clone();
    assert!(mapped.iter().any(|m| m.remote.role == Some(FolderRole::Sent) && m.remote.name == "Sent Messages"));
    let report = ms.sync_folder(&mut session, inbox.folder_id, "INBOX").await.unwrap();
    assert_eq!(report.new, 2);
    let msgs = store.folder_messages(inbox.folder_id).unwrap();
    assert_eq!(msgs.len(), 2);
    let budget = msgs.iter().find(|m| m.subject == "Budget").unwrap().clone();
    let reply = msgs.iter().find(|m| m.subject == "RE: Budget").unwrap().clone();
    assert!(budget.is_read && !reply.is_read);
    assert_eq!(reply.flag, FlagStatus::Flagged);
    assert_eq!(budget.conversation_id, reply.conversation_id, "threaded by References");
    assert_eq!(store.message_body(budget.id).unwrap().unwrap().text.trim(), "Body of Budget");
    assert!(state.lock().unwrap().log.iter().any(|c| c.contains("BODY.PEEK[]")), "never sets \\Seen while syncing");
    let _ = session.session.logout().await;

    // Local edit: mark the reply read and unflag it; another client expunges uid 1.
    store.set_read(&[reply.id], true).unwrap();
    store.set_flag(&[reply.id], FlagStatus::NotFlagged, None).unwrap();
    state.lock().unwrap().inbox.retain(|m| m.0 != 1);

    let mut session = connect(&state, "secret").await.unwrap();
    let mut ms = MailSync::new(&store, &acct, &cfg, &notifier).unwrap();
    let remote = folders::list(&mut session).await.unwrap();
    ms.map_folders(&remote).unwrap();
    let report = ms.sync_folder(&mut session, inbox.folder_id, "INBOX").await.unwrap();
    assert_eq!(report.deleted, 1);
    assert!(report.pushed >= 2, "{report:?}");
    assert!(store.message(budget.id).unwrap().is_none(), "expunged on the server → gone locally");
    let server_flags = state.lock().unwrap().inbox[0].1.clone();
    assert!(server_flags.contains(&"\\Seen".to_string()) && !server_flags.contains(&"\\Flagged".to_string()), "{server_flags:?}");
    let log = state.lock().unwrap().log.clone();
    assert!(log.iter().any(|c| c == "UID STORE 2 +FLAGS.SILENT (\\Seen)"), "{log:?}");
    assert!(log.iter().any(|c| c == "UID STORE 2 -FLAGS.SILENT (\\Flagged)"), "{log:?}");
    let shadow: Shadow = store.sync_get(acct.id, KIND_MESSAGE, reply.id).unwrap().unwrap();
    assert_eq!((shadow.uid, shadow.container.as_str()), (Some(2), "INBOX"));

    // Nothing left to push: a third pass sends no STORE.
    let before = state.lock().unwrap().log.len();
    let mut session = connect(&state, "secret").await.unwrap();
    let mut ms = MailSync::new(&store, &acct, &cfg, &notifier).unwrap();
    ms.map_folders(&folders::list(&mut session).await.unwrap()).unwrap();
    assert!(ms.pending_containers().unwrap().is_empty());
    ms.sync_folder(&mut session, inbox.folder_id, "INBOX").await.unwrap();
    assert!(!state.lock().unwrap().log[before..].iter().any(|c| c.starts_with("UID STORE")));
}

/// One folder pass: LIST, push local folder changes, LIST again, mirror.
async fn sync_folders(store: &Store, state: &Arc<Mutex<FakeServer>>, account_id: axentra_core::AccountId) -> usize {
    let mut session = connect(state, "secret").await.unwrap();
    let mut remote = folders::list(&mut session).await.unwrap();
    let pushed = folders::push_changes(&mut session, store, account_id, &remote).await.unwrap();
    if pushed > 0 {
        remote = folders::list(&mut session).await.unwrap();
    }
    folders::apply_to_store(store, account_id, &remote).unwrap();
    let _ = session.session.logout().await;
    pushed
}

#[tokio::test]
async fn folder_changes_made_here_reach_the_server() {
    let store = Store::open_in_memory().unwrap();
    let acct = account(&store);
    let state = Arc::new(Mutex::new(FakeServer::default()));
    assert_eq!(sync_folders(&store, &state, acct.id).await, 0);

    // Create a folder and a subfolder here.
    let projects = store.create_folder(acct.id, None, "Projects").unwrap();
    let q4 = store.create_folder(acct.id, Some(projects), "Q4").unwrap();
    assert_eq!(sync_folders(&store, &state, acct.id).await, 2);
    assert!(state.lock().unwrap().boxes().contains(&"Projects/Q4".to_string()));
    assert_eq!(store.folder(q4).unwrap().unwrap().remote_id.as_deref(), Some("Projects/Q4"));
    assert_eq!(sync_folders(&store, &state, acct.id).await, 0, "nothing left to push");

    // Rename the parent: the server renames the subtree, local ids stay.
    store.rename_folder(projects, "Clients").unwrap();
    assert_eq!(sync_folders(&store, &state, acct.id).await, 1);
    assert!(state.lock().unwrap().boxes().contains(&"Clients/Q4".to_string()));
    assert_eq!(store.folder(q4).unwrap().unwrap().remote_id.as_deref(), Some("Clients/Q4"));
    assert_eq!(store.folder(projects).unwrap().unwrap().name, "Clients");

    // Delete it here (Outlook moves it into Deleted Items first).
    store.delete_folder(projects).unwrap();
    assert_eq!(sync_folders(&store, &state, acct.id).await, 1);
    assert!(state.lock().unwrap().boxes().contains(&"Trash/Clients/Q4".to_string()), "{:?}", state.lock().unwrap().mailboxes);
    store.delete_folder(projects).unwrap(); // inside Deleted Items: gone for good
    assert!(store.folder(q4).unwrap().is_none());
    sync_folders(&store, &state, acct.id).await;
    let boxes = state.lock().unwrap().boxes().clone();
    assert!(!boxes.iter().any(|b| b.contains("Clients")), "{boxes:?}");
    let log = state.lock().unwrap().log.clone();
    assert!(log.iter().any(|c| c == "CREATE \"Projects\"") && log.iter().any(|c| c == "RENAME \"Projects\" \"Clients\""), "{log:?}");
}

// ---------------------------------------------------------------- Microsoft Graph

/// A minimal HTTP/1.1 server: `route(method, path, body)` answers every
/// request; the log keeps "METHOD path body" lines.
async fn fake_graph(route: fn(&str, &str, &str, &str) -> (u16, String)) -> (String, Arc<Mutex<Vec<String>>>) {
    use tokio::io::AsyncReadExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let log = Arc::new(Mutex::new(Vec::new()));
    let (b, l) = (base.clone(), log.clone());
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = listener.accept().await else { return };
            let (b, l) = (b.clone(), l.clone());
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 8192];
                let (head, body) = loop {
                    let n = s.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&buf).to_string();
                    if let Some((head, body)) = text.split_once("\r\n\r\n") {
                        let len = head
                            .lines()
                            .find_map(|h| h.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                            .unwrap_or(0);
                        if body.len() >= len {
                            break (head.to_string(), body.to_string());
                        }
                    }
                };
                let mut first = head.lines().next().unwrap_or_default().split(' ');
                let (method, path) = (first.next().unwrap_or_default().to_string(), first.next().unwrap_or_default().to_string());
                l.lock().unwrap().push(format!("{method} {path} {body}").trim().to_string());
                let (status, reply) = route(&method, &path, &body, &b);
                let resp = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                    reply.len()
                );
                let _ = s.write_all(resp.as_bytes()).await;
            });
        }
    });
    (base, log)
}

fn graph_routes(method: &str, path: &str, _body: &str, base: &str) -> (u16, String) {
    let not_found = (404, r#"{"error":{"code":"ErrorItemNotFound","message":"nope"}}"#.to_string());
    match (method, path) {
        ("GET", p) if p.starts_with("/me/mailFolders/inbox?") => (200, r#"{"id":"INBOX-ID"}"#.into()),
        ("GET", p) if p.starts_with("/me/mailFolders/sentitems?") => (200, r#"{"id":"SENT-ID"}"#.into()),
        ("GET", p) if p.starts_with("/me/mailFolders/") && p.ends_with("?$select=id") => not_found,
        ("GET", p) if p.starts_with("/me/mailFolders?") => (
            200,
            r#"{"value":[{"id":"INBOX-ID","displayName":"Inbox","parentFolderId":"ROOT","childFolderCount":1},
                {"id":"SENT-ID","displayName":"Sent Items","parentFolderId":"ROOT","childFolderCount":0},
                {"id":"F1","displayName":"Projects","parentFolderId":"ROOT","childFolderCount":0}]}"#
                .into(),
        ),
        ("GET", p) if p.starts_with("/me/mailFolders/INBOX-ID/childFolders") => {
            (200, r#"{"value":[{"id":"F2","displayName":"Receipts","parentFolderId":"INBOX-ID","childFolderCount":0}]}"#.into())
        }
        ("GET", p) if p.starts_with("/me/mailFolders/INBOX-ID/messages/delta") => (
            200,
            format!(
                r#"{{"value":[{{"id":"M1","subject":"Quarterly numbers","isRead":false,"parentFolderId":"INBOX-ID","conversationId":"C1",
                "internetMessageId":"<q@contoso.example>","receivedDateTime":"2026-10-19T09:00:00Z","changeKey":"ck1",
                "from":{{"emailAddress":{{"name":"Megan Bowen","address":"megan@contoso.example"}}}},
                "body":{{"contentType":"html","content":"<p>Numbers</p>"}},"flag":{{"flagStatus":"flagged"}}}}],
                "@odata.deltaLink":"{base}/inbox-delta-2"}}"#
            ),
        ),
        ("GET", "/inbox-delta-2") => (200, format!(r#"{{"value":[{{"id":"M1","@removed":{{"reason":"deleted"}}}}],"@odata.deltaLink":"{base}/inbox-delta-3"}}"#)),
        ("GET", p) if p.contains("/messages/delta") => (200, format!(r#"{{"value":[],"@odata.deltaLink":"{base}/empty"}}"#)),
        ("PATCH", "/me/messages/M1") => (200, "{}".into()),
        ("POST", "/me/mailFolders") => (201, r#"{"id":"F3","displayName":"Clients"}"#.into()),
        ("PATCH", "/me/mailFolders/F1") => (200, "{}".into()),
        _ => not_found,
    }
}

#[tokio::test]
async fn graph_mail_round_trip_against_fake_server() {
    use crate::graph::client::{Graph, StaticToken};
    use crate::graph::mail::GraphMail;

    let (base, log) = fake_graph(graph_routes).await;
    let store = Store::open_in_memory().unwrap();
    let mut acct = Account { kind: AccountKind::Microsoft, email: "megan@contoso.example".into(), display_name: "Megan".into(), ..Default::default() };
    let cfg = AccountConfig { provider: crate::config::Provider::MicrosoftWork, auth: crate::config::AuthMethod::OauthGraph, ..Default::default() };
    cfg.store_into(&mut acct);
    store.add_account(&mut acct).unwrap();
    let notifier = Notifier::default();
    let graph = Graph::with_base(reqwest::Client::new(), Arc::new(StaticToken(Secret::new("tok"))), &base);

    // Folders: well-known ids, the tree (with a child of Inbox), messages by delta.
    let mut mail = GraphMail::new(&graph, &store, &acct, &cfg, &notifier).unwrap();
    mail.sync_folders().await.unwrap();
    let folders = store.folders(Some(acct.id)).unwrap();
    let inbox = folders.iter().find(|f| f.role == FolderRole::Inbox).unwrap().clone();
    assert_eq!(inbox.remote_id.as_deref(), Some("INBOX-ID"));
    let receipts = folders.iter().find(|f| f.name == "Receipts").unwrap();
    assert_eq!(receipts.parent_id, Some(inbox.id));
    let projects = folders.iter().find(|f| f.name == "Projects").unwrap().clone();
    assert_eq!(mail.sync_messages(Some(&[FolderRole::Inbox])).await.unwrap(), 1);
    let m = store.folder_messages(inbox.id).unwrap().pop().unwrap();
    assert_eq!((m.subject.as_str(), m.is_read, m.flag), ("Quarterly numbers", false, FlagStatus::Flagged));
    assert_eq!(store.message_body(m.id).unwrap().unwrap().html.as_deref(), Some("<p>Numbers</p>"));

    // Local edits: read + unflag, a renamed folder and a new one.
    store.set_read(&[m.id], true).unwrap();
    store.set_flag(&[m.id], FlagStatus::NotFlagged, None).unwrap();
    store.rename_folder(projects.id, "Projects 2026").unwrap();
    let clients = store.create_folder(acct.id, None, "Clients").unwrap();
    let mut mail = GraphMail::new(&graph, &store, &acct, &cfg, &notifier).unwrap();
    assert!(mail.folders_pending().unwrap());
    assert_eq!(mail.push_folders().await.unwrap(), 2);
    assert_eq!(mail.push().await.unwrap(), 1);
    assert_eq!(store.folder(clients).unwrap().unwrap().remote_id.as_deref(), Some("F3"));
    assert!(!mail.folders_pending().unwrap());
    let requests = log.lock().unwrap().clone();
    let patch = requests.iter().find(|r| r.starts_with("PATCH /me/messages/M1")).expect("message patch");
    assert!(patch.contains(r#""isRead":true"#) && patch.contains(r#""flagStatus":"notFlagged""#), "{patch}");
    assert!(requests.iter().any(|r| r.starts_with("PATCH /me/mailFolders/F1") && r.contains("Projects 2026")), "{requests:?}");
    assert!(requests.iter().any(|r| r.starts_with("POST /me/mailFolders ") && r.contains(r#""displayName":"Clients""#)), "{requests:?}");
    assert!(requests.iter().all(|r| !r.contains("/beta/")));

    // Next delta round: the message was deleted on the server.
    let mut mail = GraphMail::new(&graph, &store, &acct, &cfg, &notifier).unwrap();
    mail.sync_messages(Some(&[FolderRole::Inbox])).await.unwrap();
    assert!(store.message(m.id).unwrap().is_none(), "@removed → gone locally");
    assert!(log.lock().unwrap().iter().any(|r| r.starts_with("GET /inbox-delta-2")), "follows the stored deltaLink");
}

fn calendar_routes(method: &str, path: &str, _body: &str, _base: &str) -> (u16, String) {
    match (method, path) {
        ("GET", p) if p.starts_with("/me/calendars?") => {
            (200, r#"{"value":[{"id":"CAL1","name":"Calendar","isDefaultCalendar":true,"canEdit":true,"changeKey":"c1"}]}"#.into())
        }
        ("GET", p) if p.starts_with("/me/calendars/CAL1/calendarView?") => (
            200,
            r#"{"value":[{"id":"E1","type":"singleInstance","subject":"Design review","isAllDay":false,
                "start":{"dateTime":"2026-10-21T15:00:00.0000000","timeZone":"UTC"},"end":{"dateTime":"2026-10-21T16:00:00.0000000","timeZone":"UTC"},
                "organizer":{"emailAddress":{"name":"Megan Bowen","address":"megan@contoso.example"}},"isOrganizer":false,
                "responseStatus":{"response":"notResponded"},"attendees":[],"showAs":"busy","changeKey":"k1"}]}"#
                .into(),
        ),
        ("POST", "/me/calendars/CAL1/events") => (201, r#"{"id":"E2","changeKey":"k2"}"#.into()),
        ("PATCH", "/me/events/E1") => (200, r#"{"id":"E1","changeKey":"k3"}"#.into()),
        ("POST", "/me/events/E1/accept") => (202, String::new()),
        ("DELETE", "/me/events/E2") => (204, String::new()),
        _ => (404, r#"{"error":{"code":"ErrorItemNotFound","message":"nope"}}"#.into()),
    }
}

#[tokio::test]
async fn graph_calendar_round_trip_against_fake_server() {
    use crate::graph::calendar::GraphCalendar;
    use crate::graph::client::{Graph, StaticToken};
    use axentra_core::{Event, ResponseStatus};

    let (base, log) = fake_graph(calendar_routes).await;
    let store = Store::open_in_memory().unwrap();
    let mut acct = Account { kind: AccountKind::Microsoft, email: "ana@contoso.example".into(), display_name: "Ana".into(), ..Default::default() };
    store.add_account(&mut acct).unwrap();
    let notifier = Notifier::default();
    let graph = Graph::with_base(reqwest::Client::new(), Arc::new(StaticToken(Secret::new("tok"))), &base);
    let cal = GraphCalendar { graph: &graph, store: &store, account: &acct, notifier: &notifier };

    // The account's default calendar adopts the server's default calendar.
    let cals = cal.sync_calendars().await.unwrap();
    assert_eq!(cals.len(), 1);
    let (cid, rid) = cals[0].clone();
    assert!(store.calendar(cid).unwrap().unwrap().is_default);
    cal.sync_calendar(cid, &rid).await.unwrap();
    let mut invite = store.calendar_events(cid).unwrap().into_iter().find(|e| e.title == "Design review").expect("pulled");
    assert_eq!(invite.my_response, ResponseStatus::NotResponded);

    // Here: accept the invite "without email" and change my reminder (an
    // attendee can only change their own copy); create a new event.
    store.respond_to_event_with(invite.id, ResponseStatus::Accepted, false, "").unwrap();
    invite = store.event(invite.id).unwrap().unwrap();
    invite.reminder_minutes = Some(30);
    store.update_event(&invite).unwrap();
    let start = chrono::Utc::now() + chrono::Duration::days(2);
    let mut lunch = Event { calendar_id: cid, title: "Lunch".into(), start, end: start + chrono::Duration::hours(1), ..Default::default() };
    let lunch_id = store.insert_event(&mut lunch).unwrap();
    assert_eq!(cal.push_only().await.unwrap(), 3);
    let requests = log.lock().unwrap().clone();
    let create = requests.iter().find(|r| r.starts_with("POST /me/calendars/CAL1/events")).expect("create");
    assert!(create.contains(r#""subject":"Lunch""#) && create.contains("transactionId"), "{create}");
    let patch = requests.iter().find(|r| r.starts_with("PATCH /me/events/E1")).expect("patch");
    assert!(patch.contains(r#""reminderMinutesBeforeStart":30"#) && !patch.contains("subject") && !patch.contains("attendees"), "{patch}");
    assert!(requests.iter().any(|r| r.starts_with("POST /me/events/E1/accept") && r.contains(r#""sendResponse":false"#)), "{requests:?}");

    // Settled: nothing more to push (the update and the response both stuck).
    assert_eq!(cal.push_only().await.unwrap(), 0);

    // Delete the new event here: DELETE on the server.
    store.delete_event(lunch_id).unwrap();
    assert_eq!(cal.push_only().await.unwrap(), 1);
    assert!(log.lock().unwrap().iter().any(|r| r.starts_with("DELETE /me/events/E2")));
}
