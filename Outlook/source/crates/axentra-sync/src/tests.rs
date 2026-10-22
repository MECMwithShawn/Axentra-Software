//! End-to-end IMAP round trips against an in-process fake server:
//! greeting → AUTHENTICATE PLAIN → LIST → SELECT → UID SEARCH → UID FETCH,
//! then a local edit pushed with UID STORE and a server-side expunge
//! mirrored locally; and folder creates / renames / deletes made here
//! pushed with CREATE / RENAME / DELETE. Microsoft Graph mail, calendar and
//! the account's profile photo against a fake HTTP server.

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
/// request with JSON; the log keeps "METHOD path body" lines.
async fn fake_graph(route: fn(&str, &str, &str, &str) -> (u16, String)) -> (String, Arc<Mutex<Vec<String>>>) {
    fake_http(move |method, path, body, base| {
        let (status, reply) = route(method, path, body, base);
        Some((status, "application/json", reply.into_bytes()))
    })
    .await
}

/// One answer of [`fake_http`]: status, content type and body; `None`
/// drops the connection without answering (a transport error).
type Reply = Option<(u16, &'static str, Vec<u8>)>;

/// [`fake_graph`] with any content type, binary bodies and state.
async fn fake_http(route: impl Fn(&str, &str, &str, &str) -> Reply + Send + Sync + 'static) -> (String, Arc<Mutex<Vec<String>>>) {
    serve_http(move |r: Request| {
        let answer = route(&r.method, &r.path, &r.body, &r.base)
            .map(|(status, content_type, body)| Answer { status, headers: vec![("Content-Type", content_type.into())], body, ..Default::default() });
        std::future::ready(answer)
    })
    .await
}

/// One request to [`serve_http`].
struct Request {
    method: String,
    path: String,
    body: String,
    /// The `Authorization` header.
    auth: String,
    /// The server's own base URL.
    base: String,
}

/// How [`serve_http`] answers one request.
#[derive(Default)]
struct Answer {
    status: u16,
    headers: Vec<(&'static str, String)>,
    body: Vec<u8>,
    /// The declared Content-Length instead of the body's own; `Some(None)`
    /// declares none (the body ends when the connection closes).
    length: Option<Option<u64>>,
    /// Keep the connection open after the body (a stalled download).
    stall: bool,
}

/// A minimal HTTP/1.1 server whose answers may wait (`None` drops the
/// connection without answering); the log keeps "METHOD path body" lines.
async fn serve_http<F, Fut>(route: F) -> (String, Arc<Mutex<Vec<String>>>)
where
    F: Fn(Request) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Option<Answer>> + Send + 'static,
{
    use tokio::io::AsyncReadExt;
    let route = Arc::new(route);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let log = Arc::new(Mutex::new(Vec::new()));
    let (b, l) = (base.clone(), log.clone());
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = listener.accept().await else { return };
            let (b, l, route) = (b.clone(), l.clone(), route.clone());
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
                let auth = head
                    .lines()
                    .find_map(|h| h.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case("authorization")).map(|(_, v)| v.trim().to_string()))
                    .unwrap_or_default();
                l.lock().unwrap().push(format!("{method} {path} {body}").trim().to_string());
                let Some(a) = route(Request { method, path, body, auth, base: b }).await else { return };
                let mut resp = format!("HTTP/1.1 {} X\r\nConnection: close\r\n", a.status);
                if let Some(len) = a.length.unwrap_or(Some(a.body.len() as u64)) {
                    resp.push_str(&format!("Content-Length: {len}\r\n"));
                }
                for (k, v) in &a.headers {
                    resp.push_str(&format!("{k}: {v}\r\n"));
                }
                resp.push_str("\r\n");
                let mut resp = resp.into_bytes();
                resp.extend_from_slice(&a.body);
                let _ = s.write_all(&resp).await;
                if a.stall {
                    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                }
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

// ---------------------------------------------------------------- profile photo

/// What the fake server answers for the photo endpoints; `None` drops the
/// connection.
struct PhotoServer {
    meta: Reply,
    sized: Reply,
    full: Reply,
}

const JPEG: &[u8] = &[0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, b'J', b'F', b'I', b'F', 0x00, 0x01];
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR";

fn json(status: u16, body: &str) -> Reply {
    Some((status, "application/json", body.as_bytes().to_vec()))
}

fn image(content_type: &'static str, bytes: &[u8]) -> Reply {
    Some((200, content_type, bytes.to_vec()))
}

fn meta(etag: &str) -> Reply {
    json(200, &format!(r#"{{"id":"240X240","width":240,"height":240,"@odata.mediaContentType":"image/jpeg","@odata.mediaEtag":"{etag}"}}"#))
}

fn not_found() -> Reply {
    json(404, r#"{"error":{"code":"ImageNotFound","message":"The photo wasn't found."}}"#)
}

/// A fake server for `/me/photo…` (anything else: `other`), a store with a
/// Microsoft account, and a Graph client pointed at the server.
async fn photo_setup(
    state: PhotoServer,
    other: fn(&str, &str, &str, &str) -> (u16, String),
) -> (Arc<Mutex<PhotoServer>>, Arc<Mutex<Vec<String>>>, String, Arc<Store>, Account) {
    let state = Arc::new(Mutex::new(state));
    let st = state.clone();
    let (base, log) = fake_http(move |method, path, body, base| {
        let s = st.lock().unwrap();
        match path {
            "/me/photo" => s.meta.clone(),
            "/me/photo/$value" => s.full.clone(),
            p if p.starts_with("/me/photos/") => s.sized.clone(),
            _ => {
                let (status, reply) = other(method, path, body, base);
                Some((status, "application/json", reply.into_bytes()))
            }
        }
    })
    .await;
    let store = Arc::new(Store::open_in_memory().unwrap());
    let acct = microsoft_account(&store, "megan@contoso.example");
    (state, log, base, store, acct)
}

/// Add a Microsoft (Graph) account to `store`.
fn microsoft_account(store: &Store, email: &str) -> Account {
    let mut acct = Account { kind: AccountKind::Microsoft, email: email.into(), display_name: email.into(), ..Default::default() };
    let cfg = AccountConfig { provider: crate::config::Provider::MicrosoftWork, auth: crate::config::AuthMethod::OauthGraph, ..Default::default() };
    cfg.store_into(&mut acct);
    store.add_account(&mut acct).unwrap();
    acct
}

fn data_url(bytes: &[u8]) -> String {
    crate::graph::photo::data_url(bytes).unwrap()
}

fn photo_requests(log: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
    log.lock().unwrap().iter().filter(|r| r.starts_with("GET /me/photo")).cloned().collect()
}

#[tokio::test]
async fn profile_photo_is_fetched_kept_and_cleared() {
    use crate::graph::client::{Graph, StaticToken};
    use crate::graph::photo::{Outcome, refresh};
    use chrono::{Duration, Utc};

    // The server calls the bytes PNG; they are a JPEG, and the bytes win.
    let (state, log, base, store, acct) =
        photo_setup(PhotoServer { meta: meta("E1"), sized: image("image/png", JPEG), full: image("image/jpeg", PNG) }, |_, _, _, _| (404, String::new())).await;
    let graph = Graph::with_base(reqwest::Client::new(), Arc::new(StaticToken(Secret::new("tok"))), &base);
    let t0 = Utc::now();
    assert_eq!(refresh(&graph, &store, acct.id, t0, false).await, Outcome::Stored);
    let jpeg = format!("data:image/jpeg;base64,{}", base64::engine::general_purpose::STANDARD.encode(JPEG));
    assert_eq!(store.account_photos().unwrap().get(&acct.id), Some(&jpeg));
    assert_eq!(photo_requests(&log), vec!["GET /me/photo", "GET /me/photos/240x240/$value"], "a 240 px copy, not the full photo");
    let saved = store.account_photo(acct.id).unwrap().unwrap();
    assert_eq!((saved.etag.as_str(), saved.checked_at, saved.failed_at), ("E1", Some(t0), None));

    // Within 6 hours nothing is asked; after that, an unchanged etag skips the download.
    log.lock().unwrap().clear();
    assert_eq!(refresh(&graph, &store, acct.id, t0 + Duration::hours(5), false).await, Outcome::NotDue);
    assert!(photo_requests(&log).is_empty());
    assert_eq!(refresh(&graph, &store, acct.id, t0 + Duration::hours(6), false).await, Outcome::Unchanged);
    assert_eq!(photo_requests(&log), vec!["GET /me/photo"]);
    assert_eq!(store.account_photo(acct.id).unwrap().unwrap().checked_at, Some(t0 + Duration::hours(6)));

    // A sign-in checks right away. A changed photo without sized copies
    // (a personal account) comes from /me/photo/$value.
    log.lock().unwrap().clear();
    {
        let mut s = state.lock().unwrap();
        s.meta = meta("E2");
        s.sized = json(400, r#"{"error":{"code":"ErrorInvalidImageSize","message":"x"}}"#);
    }
    assert_eq!(refresh(&graph, &store, acct.id, t0 + Duration::hours(7), true).await, Outcome::Stored);
    assert_eq!(photo_requests(&log), vec!["GET /me/photo", "GET /me/photos/240x240/$value", "GET /me/photo/$value"]);
    assert!(store.account_photos().unwrap()[&acct.id].starts_with("data:image/png;base64,"));
    state.lock().unwrap().sized = not_found();
    state.lock().unwrap().full = image("image/jpeg", JPEG);
    state.lock().unwrap().meta = meta("E3");
    assert_eq!(refresh(&graph, &store, acct.id, t0 + Duration::hours(8), true).await, Outcome::Stored, "a sized 404 falls back too");
    assert_eq!(store.account_photos().unwrap()[&acct.id], jpeg);

    // Unusable downloads keep the stored photo: not an image, too large.
    state.lock().unwrap().meta = meta("E4");
    state.lock().unwrap().full = image("image/jpeg", b"<html>Sign in</html>");
    assert_eq!(refresh(&graph, &store, acct.id, t0 + Duration::hours(9), true).await, Outcome::Rejected);
    let mut huge = JPEG.to_vec();
    huge.resize(crate::graph::photo::MAX_BYTES + 1, 0);
    state.lock().unwrap().meta = meta("E4-big");
    state.lock().unwrap().full = image("image/jpeg", &huge);
    assert_eq!(refresh(&graph, &store, acct.id, t0 + Duration::hours(10), true).await, Outcome::Rejected);
    let kept = store.account_photo(acct.id).unwrap().unwrap();
    assert_eq!((kept.etag.as_str(), kept.data_url.as_ref(), kept.rejected_etag.as_str()), ("E3", Some(&jpeg), "E4-big"));
    // While the server keeps that photo it isn't downloaded again, not even by a sign-in.
    log.lock().unwrap().clear();
    assert_eq!(refresh(&graph, &store, acct.id, t0 + Duration::hours(11), true).await, Outcome::Rejected);
    assert_eq!(photo_requests(&log), vec!["GET /me/photo"]);
    state.lock().unwrap().full = image("image/jpeg", JPEG);

    // Failures (status, dropped connection) keep the photo and retry after 30 minutes.
    state.lock().unwrap().meta = json(500, r#"{"error":{"code":"generalException","message":"x"}}"#);
    let t1 = t0 + Duration::hours(20);
    assert_eq!(refresh(&graph, &store, acct.id, t1, false).await, Outcome::Failed);
    state.lock().unwrap().meta = None;
    assert_eq!(refresh(&graph, &store, acct.id, t1 + Duration::minutes(10), false).await, Outcome::NotDue);
    assert_eq!(refresh(&graph, &store, acct.id, t1 + Duration::minutes(30), false).await, Outcome::Failed);
    let kept = store.account_photo(acct.id).unwrap().unwrap();
    assert_eq!((kept.data_url.as_ref(), kept.failed_at), (Some(&jpeg), Some(t1 + Duration::minutes(30))));
    // So do 401 / 403 (nothing asks for a new sign-in) and a failed download.
    for reply in [
        json(401, r#"{"error":{"code":"InvalidAuthenticationToken","message":"x"}}"#),
        json(403, r#"{"error":{"code":"ErrorAccessDenied","message":"x"}}"#),
    ] {
        state.lock().unwrap().meta = reply;
        assert_eq!(refresh(&graph, &store, acct.id, t1 + Duration::minutes(30), true).await, Outcome::Failed);
    }
    {
        let mut s = state.lock().unwrap();
        s.meta = meta("E9");
        s.sized = json(500, r#"{"error":{"code":"generalException","message":"x"}}"#);
    }
    assert_eq!(refresh(&graph, &store, acct.id, t1 + Duration::minutes(30), true).await, Outcome::Failed);
    let kept = store.account_photo(acct.id).unwrap().unwrap();
    assert_eq!((kept.etag.as_str(), kept.data_url.as_ref()), ("E3", Some(&jpeg)));
    state.lock().unwrap().sized = not_found();

    // The 1×1 placeholder means no photo: back to initials.
    state.lock().unwrap().meta = json(200, r#"{"id":"1x1","width":1,"height":1,"@odata.mediaContentType":"image/gif","@odata.mediaEtag":""}"#);
    assert_eq!(refresh(&graph, &store, acct.id, t1 + Duration::hours(1), false).await, Outcome::Removed);
    assert!(store.account_photos().unwrap().is_empty());
    let none = store.account_photo(acct.id).unwrap().unwrap();
    assert_eq!((none.etag.as_str(), none.data_url.as_ref(), none.failed_at, none.rejected_etag.as_str()), ("", None, None, ""));

    // A photo again, then a 404 on the metadata (any error code) removes it.
    state.lock().unwrap().meta = meta("E5");
    assert_eq!(refresh(&graph, &store, acct.id, t1 + Duration::hours(2), true).await, Outcome::Stored);
    state.lock().unwrap().meta = json(404, r#"{"error":{"code":"MailboxNotEnabledForRESTAPI","message":"x"}}"#);
    assert_eq!(refresh(&graph, &store, acct.id, t1 + Duration::hours(3), true).await, Outcome::Removed);
    assert!(store.account_photos().unwrap().is_empty());
    assert_eq!(refresh(&graph, &store, acct.id, t1 + Duration::hours(4), true).await, Outcome::NoPhoto);

    // Metadata but no bytes (404 on $value) is no photo as well.
    state.lock().unwrap().meta = json(200, r#"{"id":"48X48","width":48,"height":48,"@odata.mediaEtag":"E6"}"#);
    state.lock().unwrap().full = not_found();
    log.lock().unwrap().clear();
    assert_eq!(refresh(&graph, &store, acct.id, t1 + Duration::hours(5), true).await, Outcome::NoPhoto);
    assert_eq!(photo_requests(&log), vec!["GET /me/photo", "GET /me/photo/$value"], "smaller than 96 px: no sized request");
}

#[tokio::test]
async fn invalid_bytes_without_a_stored_photo_keep_the_initials() {
    use crate::graph::client::{Graph, StaticToken};
    use crate::graph::photo::{Outcome, refresh};

    let (_state, _log, base, store, acct) =
        photo_setup(PhotoServer { meta: meta("E1"), sized: image("image/jpeg", b"GIF8"), full: None }, |_, _, _, _| (404, String::new())).await;
    let graph = Graph::with_base(reqwest::Client::new(), Arc::new(StaticToken(Secret::new("tok"))), &base);
    assert_eq!(refresh(&graph, &store, acct.id, chrono::Utc::now(), false).await, Outcome::Rejected);
    assert!(store.account_photos().unwrap().is_empty());
    assert!(store.account_photo(acct.id).unwrap().unwrap().checked_at.is_some(), "not downloaded again on every pass");
}

/// Mail as in the mail round trip; a quick pass also pushes a new
/// account's task list.
fn quick_pass_routes(method: &str, path: &str, body: &str, base: &str) -> (u16, String) {
    match (method, path) {
        ("POST", "/me/todo/lists") => (201, r#"{"id":"LIST-1","displayName":"Tasks"}"#.into()),
        ("GET", p) if p.starts_with("/me/todo/lists/LIST-1/tasks?") => (200, r#"{"value":[]}"#.into()),
        _ => graph_routes(method, path, body, base),
    }
}

/// A sync pass whose photo check fails still succeeds, and the account's
/// state and stored photo stay as they were.
#[tokio::test]
async fn a_failing_photo_check_never_fails_the_pass() {
    use crate::config::AccountState;
    use crate::engine::{Inner, Resource, Scope};
    use crate::graph::photo::RECHECK;
    use axentra_core::AccountPhoto;
    use chrono::Utc;

    let (_state, log, base, store, acct) = photo_setup(PhotoServer { meta: None, sized: None, full: None }, quick_pass_routes).await;
    let old = Utc::now() - RECHECK - chrono::Duration::minutes(1);
    let photo = AccountPhoto { etag: "E1".into(), data_url: Some("data:image/png;base64,iVBORw0KGgo=".into()), checked_at: Some(old), ..Default::default() };
    store.set_account_photo(&acct, &photo).unwrap();
    let status = Arc::new(Mutex::new(crate::SyncStatus::default()));
    let inner = Inner::with_graph_base(store.clone(), Notifier::default(), status.clone(), crate::secrets::Keyring::memory(), &base);
    inner.cache_token(acct.id, Resource::Graph, Secret::new("tok"), Utc::now() + chrono::Duration::hours(1));

    inner.pass(acct.id, Scope::Quick).await.expect("the pass succeeds");
    inner.request(acct.id, Scope::Quick);
    for _ in 0..100 {
        let failed = store.account_photo(acct.id).unwrap().unwrap().failed_at.is_some();
        if failed && !status.lock().unwrap().running {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let after = store.account_photo(acct.id).unwrap().unwrap();
    assert!(after.failed_at.is_some(), "the check ran and failed");
    assert_eq!((after.data_url, after.checked_at), (photo.data_url, photo.checked_at), "the stored photo stays");
    let a = store.account(acct.id).unwrap().unwrap();
    assert_eq!((AccountConfig::of(&a).state, a.sync_error), (AccountState::Ok, None));
    assert!(a.last_sync_at.is_some(), "the recorded pass succeeded");
    let photo_calls = photo_requests(&log);
    assert_eq!(photo_calls, vec!["GET /me/photo"], "one check: the second pass finds it running or waiting 30 minutes to retry");
}

/// Wait (up to 5 s) until `done` holds.
async fn eventually(what: &str, done: impl Fn() -> bool) {
    for _ in 0..100 {
        if done() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("timed out waiting until {what}");
}

/// Right after a sign-in the engine checks the photo at once and tells the
/// UI through the notifier; a check that isn't due asks nothing; a photo
/// removed on the server goes here too.
#[tokio::test]
async fn a_sign_in_check_reaches_the_avatars() {
    use crate::engine::{Inner, Resource};
    use chrono::Utc;
    use std::sync::atomic::AtomicU64;

    let (state, log, base, store, acct) =
        photo_setup(PhotoServer { meta: meta("E1"), sized: image("image/jpeg", JPEG), full: None }, |_, _, _, _| (404, String::new())).await;
    let notifier = Notifier::new(Arc::new(AtomicU64::new(0)));
    let status = Arc::new(Mutex::new(crate::SyncStatus::default()));
    let inner = Inner::with_graph_base(store.clone(), notifier.clone(), status, crate::secrets::Keyring::memory(), &base);
    inner.cache_token(acct.id, Resource::Graph, Secret::new("tok"), Utc::now() + chrono::Duration::hours(1));

    inner.check_photo(acct.id, true);
    eventually("the UI is told", || notifier.get() > 0).await;
    assert!(store.account_photos().unwrap()[&acct.id].starts_with("data:image/jpeg;base64,"));
    assert_eq!(photo_requests(&log).len(), 2);

    // Checked a moment ago: a pass asks nothing.
    inner.check_photo(acct.id, false);
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(photo_requests(&log).len(), 2);

    // Removed on the server: the next sign-in check takes it away here too.
    state.lock().unwrap().meta = not_found();
    let seen = notifier.get();
    inner.check_photo(acct.id, true);
    eventually("the removal is reported", || notifier.get() > seen).await;
    assert!(store.account_photos().unwrap().is_empty());
}

/// [`fake_http`]'s reply as an [`Answer`].
fn answer(reply: Reply) -> Option<Answer> {
    reply.map(|(status, content_type, body)| Answer { status, headers: vec![("Content-Type", content_type.into())], body, ..Default::default() })
}

/// A photo larger than the limit is never downloaded past it, whether its
/// size is declared or not, and isn't downloaded again while it stays.
#[tokio::test]
async fn an_oversized_photo_stops_at_the_limit_and_is_not_fetched_again() {
    use crate::graph::client::{Graph, StaticToken};
    use crate::graph::photo::{MAX_BYTES, Outcome, refresh};
    use chrono::{Duration, Utc};

    // How the full photo comes: (etag, declared Content-Length). Either way
    // 2 MiB arrive and then nothing more, so a client reading it all would hang.
    let serve: Arc<Mutex<(&'static str, Option<u64>)>> = Arc::new(Mutex::new(("BIG1", Some(64 << 20))));
    let s = serve.clone();
    let (base, log) = serve_http(move |r: Request| {
        let (etag, length) = *s.lock().unwrap();
        std::future::ready(match r.path.as_str() {
            "/me/photo" => answer(json(200, &format!(r#"{{"id":"64X64","width":64,"height":64,"@odata.mediaEtag":"{etag}"}}"#))),
            "/me/photo/$value" if etag == "SMALL" => answer(image("image/jpeg", JPEG)),
            "/me/photo/$value" => {
                let mut body = JPEG.to_vec();
                body.resize(2 * MAX_BYTES, 0);
                Some(Answer { status: 200, headers: vec![("Content-Type", "image/jpeg".into())], body, length: Some(length), stall: true })
            }
            _ => answer(not_found()),
        })
    })
    .await;
    let store = Arc::new(Store::open_in_memory().unwrap());
    let acct = microsoft_account(&store, "megan@contoso.example");
    let graph = Graph::with_base(reqwest::Client::new(), Arc::new(StaticToken(Secret::new("tok"))), &base);
    let t0 = Utc::now();
    let check = |at, force| {
        let (graph, store) = (graph.clone(), store.clone());
        async move { tokio::time::timeout(std::time::Duration::from_secs(10), refresh(&graph, &store, acct.id, at, force)).await.expect("not read to the end") }
    };

    // Declared 64 MiB: not read at all.
    assert_eq!(check(t0, false).await, Outcome::Rejected);
    assert_eq!(store.account_photo(acct.id).unwrap().unwrap().rejected_etag, "BIG1");
    // Unchanged 6 hours later: only the metadata is asked for.
    log.lock().unwrap().clear();
    assert_eq!(check(t0 + Duration::hours(6), false).await, Outcome::Rejected);
    assert_eq!(photo_requests(&log), vec!["GET /me/photo"]);

    // No declared size (as with a compressed body): reading stops at the limit.
    *serve.lock().unwrap() = ("BIG2", None);
    assert_eq!(check(t0 + Duration::hours(12), false).await, Outcome::Rejected);
    assert_eq!(store.account_photo(acct.id).unwrap().unwrap().rejected_etag, "BIG2");

    // A usable photo later is stored as usual.
    *serve.lock().unwrap() = ("SMALL", None);
    assert_eq!(check(t0 + Duration::hours(18), false).await, Outcome::Stored);
    let rec = store.account_photo(acct.id).unwrap().unwrap();
    assert_eq!((rec.etag.as_str(), rec.data_url, rec.rejected_etag.as_str()), ("SMALL", Some(data_url(JPEG)), ""));
}

/// A photo server for two accounts: the token "tokA" (Alice) gets a JPEG
/// with etag EA, any other token a PNG with etag EB. The first sized
/// download waits for `release` (a slow connection); `asked` counts them.
struct GatedPhotos {
    base: String,
    log: Arc<Mutex<Vec<String>>>,
    release: Arc<tokio::sync::Notify>,
    asked: Arc<std::sync::atomic::AtomicUsize>,
}

async fn gated_photos() -> GatedPhotos {
    use std::sync::atomic::Ordering::SeqCst;
    let release = Arc::new(tokio::sync::Notify::new());
    let asked = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (r, a) = (release.clone(), asked.clone());
    let (base, log) = serve_http(move |req: Request| {
        let (release, asked) = (r.clone(), a.clone());
        async move {
            let alice = req.auth.ends_with("tokA");
            match req.path.as_str() {
                "/me/photo" => answer(meta(if alice { "EA" } else { "EB" })),
                p if p.starts_with("/me/photos/") => {
                    if asked.fetch_add(1, SeqCst) == 0 {
                        release.notified().await;
                    }
                    answer(if alice { image("image/jpeg", JPEG) } else { image("image/png", PNG) })
                }
                _ => answer(not_found()),
            }
        }
    })
    .await;
    GatedPhotos { base, log, release, asked }
}

/// Removing an account stops its photo check: a late answer never lands on
/// a newer account that SQLite gave the same id, nor stops that account's
/// own check.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_removed_accounts_photo_never_reaches_a_newer_account_with_its_id() {
    use crate::engine::{Inner, Resource};
    use chrono::{Duration, Utc};
    use std::sync::atomic::Ordering::SeqCst;

    let server = gated_photos().await;
    let store = Arc::new(Store::open_in_memory().unwrap());
    let alice = microsoft_account(&store, "alice@contoso.example");
    let inner = Inner::with_graph_base(store.clone(), Notifier::default(), Arc::default(), crate::secrets::Keyring::memory(), &server.base);
    inner.cache_token(alice.id, Resource::Graph, Secret::new("tokA"), Utc::now() + Duration::hours(1));
    inner.check_photo(alice.id, true);
    eventually("Alice's photo is downloading", || server.asked.load(SeqCst) == 1).await;

    inner.forget_account(&alice).await.unwrap();
    let bob = microsoft_account(&store, "bob@contoso.example");
    assert_eq!(bob.id, alice.id, "SQLite reuses the rowid");
    inner.cache_token(bob.id, Resource::Graph, Secret::new("tokB"), Utc::now() + Duration::hours(1));
    inner.check_photo(bob.id, true);
    let png = data_url(PNG);
    eventually("Bob's photo is stored", || store.account_photos().unwrap().get(&bob.id) == Some(&png)).await;

    server.release.notify_one();
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let rec = store.account_photo(bob.id).unwrap().unwrap();
    assert_eq!((rec.etag.as_str(), rec.data_url.as_ref()), ("EB", Some(&png)));
}

/// The store keeps the same promise when a check outlives its account
/// before it could be stopped: nothing is saved.
#[tokio::test]
async fn a_check_that_outlives_its_account_saves_nothing() {
    use crate::graph::client::{Graph, StaticToken};
    use crate::graph::photo::{Outcome, refresh};
    use axentra_core::AccountPhoto;
    use chrono::Utc;
    use std::sync::atomic::Ordering::SeqCst;

    let server = gated_photos().await;
    let store = Arc::new(Store::open_in_memory().unwrap());
    let alice = microsoft_account(&store, "alice@contoso.example");
    let graph = Graph::with_base(reqwest::Client::new(), Arc::new(StaticToken(Secret::new("tokA"))), &server.base);
    let (s, id) = (store.clone(), alice.id);
    let check = tokio::spawn(async move { refresh(&graph, &s, id, Utc::now(), true).await });
    eventually("Alice's photo is downloading", || server.asked.load(SeqCst) == 1).await;

    store.remove_account(alice.id).unwrap();
    let bob = microsoft_account(&store, "bob@contoso.example");
    assert_eq!(bob.id, alice.id, "SQLite reuses the rowid");
    let bobs = AccountPhoto { etag: "EB".into(), data_url: Some(data_url(PNG)), checked_at: Some(Utc::now()), ..Default::default() };
    store.set_account_photo(&bob, &bobs).unwrap();

    server.release.notify_one();
    assert_eq!(check.await.unwrap(), Outcome::Gone);
    assert_eq!(store.account_photo(bob.id).unwrap(), Some(bobs));
}

/// A sign-in while a check runs isn't dropped: its check follows; a pass
/// meanwhile adds nothing.
#[tokio::test]
async fn a_sign_in_during_a_check_checks_again_after_it() {
    use crate::engine::{Inner, Resource};
    use chrono::{Duration, Utc};
    use std::sync::atomic::Ordering::SeqCst;

    let server = gated_photos().await;
    let store = Arc::new(Store::open_in_memory().unwrap());
    let acct = microsoft_account(&store, "alice@contoso.example");
    let inner = Inner::with_graph_base(store.clone(), Notifier::default(), Arc::default(), crate::secrets::Keyring::memory(), &server.base);
    inner.cache_token(acct.id, Resource::Graph, Secret::new("tokA"), Utc::now() + Duration::hours(1));
    inner.check_photo(acct.id, false);
    eventually("the first check is downloading", || server.asked.load(SeqCst) == 1).await;
    inner.check_photo(acct.id, true);
    inner.check_photo(acct.id, false);
    server.release.notify_one();
    eventually("the sign-in's check ran", || photo_requests(&server.log).len() == 3).await;
    assert_eq!(photo_requests(&server.log), vec!["GET /me/photo", "GET /me/photos/240x240/$value", "GET /me/photo"], "then the etag is unchanged");
    assert_eq!(store.account_photos().unwrap()[&acct.id], data_url(JPEG));
    // Done: the next sign-in starts a check of its own.
    inner.check_photo(acct.id, true);
    eventually("another check", || photo_requests(&server.log).len() == 4).await;
}

/// Throttling (429 with Retry-After) is retried within the check like any
/// Graph request, then counts as a failed check: the pass, the account and
/// the stored photo are untouched, and it is tried again in 30 minutes.
#[tokio::test]
async fn a_throttled_photo_check_changes_nothing() {
    use crate::config::AccountState;
    use crate::engine::{Inner, Resource, Scope};
    use crate::graph::photo::{RECHECK, RETRY, due};
    use axentra_core::AccountPhoto;
    use chrono::Utc;

    let (base, log) = serve_http(|r: Request| {
        std::future::ready(match r.path.as_str() {
            "/me/photo" => Some(Answer {
                status: 429,
                headers: vec![("Retry-After", "0".into()), ("Content-Type", "application/json".into())],
                body: br#"{"error":{"code":"TooManyRequests","message":"x"}}"#.to_vec(),
                ..Default::default()
            }),
            p if p.starts_with("/me/photo") => answer(not_found()),
            p => {
                let (status, reply) = quick_pass_routes(&r.method, p, &r.body, &r.base);
                answer(json(status, &reply))
            }
        })
    })
    .await;
    let store = Arc::new(Store::open_in_memory().unwrap());
    let acct = microsoft_account(&store, "megan@contoso.example");
    let old = Utc::now() - RECHECK - chrono::Duration::minutes(1);
    let photo = AccountPhoto { etag: "E1".into(), data_url: Some(data_url(PNG)), checked_at: Some(old), ..Default::default() };
    store.set_account_photo(&acct, &photo).unwrap();
    let inner = Inner::with_graph_base(store.clone(), Notifier::default(), Arc::default(), crate::secrets::Keyring::memory(), &base);
    inner.cache_token(acct.id, Resource::Graph, Secret::new("tok"), Utc::now() + chrono::Duration::hours(1));

    inner.pass(acct.id, Scope::Quick).await.expect("the pass succeeds");
    eventually("the check gave up", || store.account_photo(acct.id).unwrap().unwrap().failed_at.is_some()).await;
    assert_eq!(photo_requests(&log).len(), 6, "six attempts, as for any Graph request");
    let after = store.account_photo(acct.id).unwrap().unwrap();
    assert_eq!((after.etag.as_str(), after.data_url.as_ref(), after.checked_at), ("E1", photo.data_url.as_ref(), Some(old)));
    let failed = after.failed_at.unwrap();
    assert!(!due(Some(&after), failed + RETRY - chrono::Duration::minutes(1)) && due(Some(&after), failed + RETRY));
    let a = store.account(acct.id).unwrap().unwrap();
    assert_eq!((AccountConfig::of(&a).state, a.sync_error), (AccountState::Ok, None));
}

/// The photo is checked by quick and full passes, never by the Push and
/// Outbox passes that run every few seconds.
#[tokio::test]
async fn only_quick_and_full_passes_check_the_photo() {
    use crate::engine::{Inner, Resource, Scope};
    use chrono::Utc;

    let (_state, log, base, store, acct) = photo_setup(PhotoServer { meta: not_found(), sized: None, full: None }, quick_pass_routes).await;
    let inner = Inner::with_graph_base(store.clone(), Notifier::default(), Arc::default(), crate::secrets::Keyring::memory(), &base);
    inner.cache_token(acct.id, Resource::Graph, Secret::new("tok"), Utc::now() + chrono::Duration::hours(1));
    let _ = inner.pass(acct.id, Scope::Push).await;
    let _ = inner.pass(acct.id, Scope::Outbox).await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(photo_requests(&log).is_empty(), "{:?}", photo_requests(&log));
    inner.pass(acct.id, Scope::Quick).await.expect("the pass succeeds");
    eventually("the quick pass checked the photo", || photo_requests(&log) == ["GET /me/photo"]).await;
    assert!(store.account_photo(acct.id).unwrap().unwrap().checked_at.is_some());
}

/// Finishing a sign-in checks a Microsoft account's photo at once, for a new
/// account and when signing in again (even right after a check); an IMAP
/// account's sign-in asks nothing.
#[tokio::test]
async fn signing_in_checks_the_photo_at_once() {
    use crate::accounts::{SignInKind, finish, park_sign_in};
    use crate::config::{AuthMethod, Provider, Security, ServerConfig};
    use crate::engine::{Inner, Resource};
    use crate::oauth::OAuthClient;
    use axentra_core::AccountPhoto;
    use chrono::Utc;
    use std::sync::atomic::AtomicU64;

    let (state, log, base, store, megan) = photo_setup(PhotoServer { meta: meta("E1"), sized: image("image/jpeg", JPEG), full: None }, |_, _, _, _| {
        (404, r#"{"error":{"code":"ErrorItemNotFound","message":"x"}}"#.into())
    })
    .await;
    let notifier = Notifier::new(Arc::new(AtomicU64::new(0)));
    let inner = Inner::with_graph_base(store.clone(), notifier.clone(), Arc::default(), crate::secrets::Keyring::memory(), &base);
    let microsoft = OAuthClient::microsoft("organizations", "client-id");

    // A new account.
    let kind = SignInKind::Microsoft { tenant: "organizations".into(), client_id: None };
    let id = finish(&inner, park_sign_in("adele@contoso.example", kind, microsoft.clone(), "tenant-1", "tokAdele")).await.unwrap();
    eventually("the new account's photo", || store.account_photos().unwrap().get(&id) == Some(&data_url(JPEG))).await;
    assert_eq!(photo_requests(&log), vec!["GET /me/photo", "GET /me/photos/240x240/$value"]);

    // Signed in again moments after a check: checked anyway.
    let recent = AccountPhoto { etag: "E0".into(), data_url: Some(data_url(PNG)), checked_at: Some(Utc::now()), ..Default::default() };
    store.set_account_photo(&megan, &recent).unwrap();
    state.lock().unwrap().meta = meta("E2");
    let seen = notifier.get();
    finish(&inner, park_sign_in(&megan.email, SignInKind::Reauth(megan.id), microsoft, "tenant-1", "tokMegan")).await.unwrap();
    eventually("Megan's new photo", || store.account_photo(megan.id).unwrap().is_some_and(|p| p.etag == "E2")).await;
    assert_eq!(store.account_photos().unwrap()[&megan.id], data_url(JPEG));
    assert!(notifier.get() > seen);

    // An IMAP (Gmail) account: no photo request. Its servers don't answer,
    // and a Graph token is at hand, so a wrong check would reach the fake
    // server, not a real one.
    let unreachable = ServerConfig { host: "127.0.0.1".into(), port: 1, security: Security::Tls, username: String::new() };
    let mut gmail = Account { kind: AccountKind::Imap, email: "ana@gmail.example".into(), display_name: "Ana".into(), ..Default::default() };
    let cfg = AccountConfig {
        provider: Provider::Google,
        auth: AuthMethod::Xoauth2Google,
        imap: Some(unreachable.clone()),
        smtp: Some(unreachable),
        ..Default::default()
    };
    cfg.store_into(&mut gmail);
    store.add_account(&mut gmail).unwrap();
    inner.cache_token(gmail.id, Resource::Graph, Secret::new("tokAna"), Utc::now() + chrono::Duration::hours(1));
    let before = photo_requests(&log).len();
    let google = OAuthClient::google("google-client", None);
    finish(&inner, park_sign_in(&gmail.email, SignInKind::Reauth(gmail.id), google, "", "tokAna")).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(photo_requests(&log).len(), before);
    assert!(!store.account_photos().unwrap().contains_key(&gmail.id));
}
