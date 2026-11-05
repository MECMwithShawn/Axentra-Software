//! New messages started from outside the app: `axentra-outlook --compose`
//! and `mailto:` links. Each one only opens a filled-in draft for the user
//! to review; nothing here sends.
//!
//! The launch writes the request into a private spool folder next to the
//! database (`<db>.compose/`, mode 0700, one `*.json` file per request,
//! mode 0600) and the running app picks it up ([`take`]), whether that is
//! itself or a copy that was already open (it shows its window when asked).
//! Requests older than [`MAX_AGE`] are dropped unopened, so one left behind
//! by a launch that never started the app does not pop up days later.
//!
//! A `mailto:` link never attaches files: a web page could otherwise use a
//! link to slip a private file into a message (`attach=` and friends are
//! ignored). Only `--attach` on the command line does.
//!
//! `--send` (command line only, never a link) asks the app to send the
//! message instead of opening it: the request carries an [`ComposeRequest::id`]
//! and the app answers with a [`SendResult`] in `<id>.result` in the same
//! folder ([`write_result`]), which the command waits for ([`wait_result`]).

use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use crate::model::EmailAddress;

/// How long a request waits in the spool for the app to open.
pub const MAX_AGE: Duration = Duration::from_secs(10 * 60);
/// Largest request file read (the body is text; files are referenced by path).
const MAX_REQUEST: u64 = 1024 * 1024;
/// Longest body taken from a `mailto:` link or a body file.
pub const MAX_BODY: usize = 512 * 1024;

/// What to fill in. Attachments are absolute paths, read when the draft is made.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ComposeRequest {
    #[serde(default)]
    pub to: Vec<EmailAddress>,
    #[serde(default)]
    pub cc: Vec<EmailAddress>,
    #[serde(default)]
    pub bcc: Vec<EmailAddress>,
    #[serde(default)]
    pub subject: String,
    /// Plain text.
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub attachments: Vec<PathBuf>,
    /// Send instead of opening a draft (`--send`; never from a link).
    #[serde(default)]
    pub send: bool,
    /// Names the request's [`SendResult`] file (set by [`submit`]).
    #[serde(default)]
    pub id: String,
}

/// How a `--send` request ended, for the command to print.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SendResult {
    pub sent: bool,
    /// One line: what was sent, or why not.
    pub text: String,
}

/// Why a message can't be sent as asked: no recipient, no subject, no body,
/// or an attachment that can't be read. Empty when it can go.
pub fn send_problems(req: &ComposeRequest) -> Vec<String> {
    let mut out = Vec::new();
    if req.to.is_empty() && req.cc.is_empty() && req.bcc.is_empty() {
        out.push("no recipient".to_string());
    }
    if let Some(bad) = req.to.iter().chain(&req.cc).chain(&req.bcc).find(|a| !looks_like_address(&a.address)) {
        out.push(format!("\"{}\" isn't an email address", bad.address));
    }
    if req.subject.trim().is_empty() {
        out.push("the subject is empty".to_string());
    }
    if req.body.trim().is_empty() {
        out.push("the body is empty".to_string());
    }
    for p in &req.attachments {
        match fs::File::open(p).and_then(|f| f.metadata()) {
            Ok(m) if m.is_file() => {}
            Ok(_) => out.push(format!("{} isn't a file", p.display())),
            Err(e) => out.push(format!("can't read {}: {e}", p.display())),
        }
    }
    out
}

fn looks_like_address(a: &str) -> bool {
    match a.rsplit_once('@') {
        Some((local, domain)) => !local.is_empty() && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.') && !a.chars().any(char::is_whitespace),
        None => false,
    }
}

/// The spool folder for the database at `db`.
pub fn spool_dir(db: &Path) -> PathBuf {
    let name = db.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "axentra.db".into());
    db.with_file_name(format!("{name}.compose"))
}

/// Hand a request to the app on `db` (picked up by [`take`]).
/// Returns the request file (gone once the app has taken it); its stem is
/// the request's id.
pub fn submit(db: &Path, req: &ComposeRequest) -> io::Result<PathBuf> {
    let dir = spool_dir(db);
    if let Some(parent) = dir.parent() {
        fs::create_dir_all(parent)?;
    }
    match DirBuilder::new().mode(0o700).create(&dir) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    check_dir(&dir)?;
    let stamp = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_nanos();
    let stem = format!("{stamp:032}-{}", std::process::id());
    let tmp = dir.join(format!(".{stem}.tmp"));
    let path = dir.join(format!("{stem}.json"));
    let req = ComposeRequest { id: stem.clone(), ..req.clone() };
    let mut f = OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?;
    f.write_all(&serde_json::to_vec(&req).map_err(io::Error::other)?)?;
    f.sync_all()?;
    drop(f);
    // Renamed into place whole, so [`take`] never reads half a request.
    fs::rename(&tmp, &path)?;
    Ok(path)
}

/// The spool must be our own private folder, not a link to somewhere else.
fn check_dir(dir: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(dir)?;
    // SAFETY: getuid has no preconditions and cannot fail.
    let uid = unsafe { libc_getuid() };
    if !meta.is_dir() || meta.uid() != uid || meta.mode() & 0o077 != 0 {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("{} is not a private folder", dir.display())));
    }
    Ok(())
}

unsafe extern "C" {
    #[link_name = "getuid"]
    fn libc_getuid() -> u32;
}

/// Take every waiting request for `db`, oldest first, and remove them.
/// Requests older than [`MAX_AGE`], unreadable or malformed are removed
/// without being returned.
pub fn take(db: &Path) -> Vec<ComposeRequest> {
    let dir = spool_dir(db);
    if check_dir(&dir).is_err() {
        return Vec::new();
    }
    let Ok(entries) = fs::read_dir(&dir) else { return Vec::new() };
    let all: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    let now = SystemTime::now();
    // Answers nobody waited for.
    for p in all.iter().filter(|p| p.extension().is_some_and(|x| x == "result")) {
        if fs::symlink_metadata(p).ok().and_then(|m| m.modified().ok()).and_then(|m| now.duration_since(m).ok()).is_some_and(|age| age > MAX_AGE * 6) {
            let _ = fs::remove_file(p);
        }
    }
    let mut files: Vec<PathBuf> = all.into_iter().filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
    files.sort();
    let mut out = Vec::new();
    for path in files {
        // Claimed by renaming, so a command that gives up waiting (and
        // removes its request) and the app never both act on it.
        let taken = path.with_extension("taken");
        if fs::rename(&path, &taken).is_err() {
            continue;
        }
        let req = read_request(&taken, now);
        if let Err(e) = fs::remove_file(&taken) {
            tracing::warn!("compose request {}: {e}", taken.display());
        }
        if let Some(req) = req {
            out.push(req);
        }
    }
    out
}

/// The app's answer to a `--send` request (see [`wait_result`]).
pub fn write_result(db: &Path, id: &str, result: &SendResult) -> io::Result<()> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "bad request id"));
    }
    let dir = spool_dir(db);
    check_dir(&dir)?;
    let tmp = dir.join(format!(".{id}.result.tmp"));
    let mut f = OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
    f.write_all(&serde_json::to_vec(result).map_err(io::Error::other)?)?;
    f.sync_all()?;
    drop(f);
    fs::rename(&tmp, dir.join(format!("{id}.result")))
}

/// How waiting for a `--send` request ended.
#[derive(Debug, PartialEq)]
pub enum Waited {
    /// The app's answer.
    Done(SendResult),
    /// Nobody took the request within `pickup`; it was withdrawn, so
    /// nothing will be sent later.
    NotTaken,
    /// Taken, but no answer within `total`: the message may still go.
    NoAnswer,
}

/// Wait for the answer to the request `submit` wrote at `request`.
/// `on_wait` runs about every half second (e.g. to start the app once).
pub fn wait_result(request: &Path, pickup: Duration, total: Duration, mut on_wait: impl FnMut(Duration)) -> Waited {
    let id = request.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let answer = request.with_file_name(format!("{id}.result"));
    let start = std::time::Instant::now();
    loop {
        if let Ok(bytes) = fs::read(&answer) {
            let _ = fs::remove_file(&answer);
            return Waited::Done(serde_json::from_slice(&bytes).unwrap_or(SendResult { sent: false, text: "Axentra Outlook gave an answer that couldn't be read.".into() }));
        }
        let waited = start.elapsed();
        if waited >= pickup && request.exists() && fs::remove_file(request).is_ok() {
            return Waited::NotTaken;
        }
        if waited >= total {
            return Waited::NoAnswer;
        }
        on_wait(waited);
        std::thread::sleep(Duration::from_millis(500));
    }
}

fn read_request(path: &Path, now: SystemTime) -> Option<ComposeRequest> {
    let meta = fs::symlink_metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_REQUEST {
        return None;
    }
    let age = meta.modified().ok().and_then(|m| now.duration_since(m).ok()).unwrap_or_default();
    if age > MAX_AGE {
        tracing::info!("compose request {} is too old; dropped", path.display());
        return None;
    }
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

/// Parse a `mailto:` link (RFC 6068): addresses before `?`, then `to`,
/// `cc`, `bcc`, `subject` and `body`. Anything else (`attach`,
/// `attachment`, `in-reply-to`…) is ignored. `None` when it isn't a
/// `mailto:` link.
pub fn parse_mailto(uri: &str) -> Option<ComposeRequest> {
    let uri = uri.trim();
    let rest = uri.get(..7).filter(|s| s.eq_ignore_ascii_case("mailto:")).map(|_| &uri[7..])?;
    let rest = rest.split('#').next().unwrap_or_default();
    let (addrs, query) = rest.split_once('?').unwrap_or((rest, ""));
    let mut req = ComposeRequest::default();
    push_addresses(&mut req.to, &percent_decode(addrs));
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let v = percent_decode(v);
        match percent_decode(k).to_ascii_lowercase().as_str() {
            "to" => push_addresses(&mut req.to, &v),
            "cc" => push_addresses(&mut req.cc, &v),
            "bcc" => push_addresses(&mut req.bcc, &v),
            "subject" if req.subject.is_empty() => req.subject = one_line(&v),
            "body" if req.body.is_empty() => req.body = truncate(v.replace("\r\n", "\n"), MAX_BODY),
            _ => {}
        }
    }
    Some(req)
}

/// Add each of "a@x, B <b@y>; c@z" to `list`, skipping repeats and blanks.
pub fn push_addresses(list: &mut Vec<EmailAddress>, text: &str) {
    for part in text.split([',', ';']) {
        let t = part.trim();
        if t.is_empty() {
            continue;
        }
        let a = if let (Some(lt), Some(gt)) = (t.find('<'), t.rfind('>'))
            && lt < gt
        {
            EmailAddress::new(one_line(t[..lt].trim().trim_matches('"')), one_line(t[lt + 1..gt].trim()))
        } else {
            EmailAddress::new("", one_line(t))
        };
        if a.address.is_empty() || list.iter().any(|x| x.address.eq_ignore_ascii_case(&a.address)) {
            continue;
        }
        list.push(a);
    }
}

/// Control characters (line breaks among them) out of a header value.
pub fn one_line(s: &str) -> String {
    s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect::<String>().trim().to_string()
}

/// At most `max` bytes, cut at a character boundary.
pub fn truncate(mut s: String, max: usize) -> String {
    if s.len() > max {
        let mut end = max;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
    }
    s
}

/// `%XX` escapes decoded as UTF-8 (invalid sequences become U+FFFD). `+`
/// stays a plus: in a `mailto:` link a space is `%20`.
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2]))
        {
            out.push(h << 4 | l);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(c: u8) -> Option<u8> {
    (c as char).to_digit(16).map(|d| d as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addrs(list: &[EmailAddress]) -> Vec<&str> {
        list.iter().map(|a| a.address.as_str()).collect()
    }

    #[test]
    fn mailto_links() {
        let r = parse_mailto("mailto:a@example.com,b@example.com?cc=c@example.com&subject=Hello%20there&body=Line%201%0D%0ALine%202").unwrap();
        assert_eq!(addrs(&r.to), ["a@example.com", "b@example.com"]);
        assert_eq!(addrs(&r.cc), ["c@example.com"]);
        assert_eq!(r.subject, "Hello there");
        assert_eq!(r.body, "Line 1\nLine 2");
        assert!(r.attachments.is_empty());

        let r = parse_mailto("MAILTO:?to=x@example.com&TO=x@example.com&bcc=y@example.com").unwrap();
        assert_eq!(addrs(&r.to), ["x@example.com"]);
        assert_eq!(addrs(&r.bcc), ["y@example.com"]);

        // A plus is a plus; the subject is one line.
        let r = parse_mailto("mailto:me+tag@example.com?subject=a%0Ab+c").unwrap();
        assert_eq!(addrs(&r.to), ["me+tag@example.com"]);
        assert_eq!(r.subject, "a b+c");

        // Named address, UTF-8, broken escapes.
        let r = parse_mailto("mailto:%22Z%C3%BC%22%20%3Cz@example.com%3E?subject=%E2%9C%93%zz%4").unwrap();
        assert_eq!(r.to, vec![EmailAddress::new("Zü", "z@example.com")]);
        assert_eq!(r.subject, "✓%zz%4");

        assert!(parse_mailto("https://example.com").is_none());
        assert!(parse_mailto("mail").is_none());
    }

    #[test]
    fn mailto_never_attaches() {
        let r = parse_mailto("mailto:a@example.com?attach=/etc/passwd&attachment=file:///home/me/.ssh/id_rsa&body=hi").unwrap();
        assert!(r.attachments.is_empty());
        assert_eq!(r.body, "hi");
    }

    #[test]
    fn send_requests_are_checked_and_answered() {
        let dir = std::env::temp_dir().join(format!("axentra-send-{}-{}", std::process::id(), line!()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.zip");
        fs::write(&file, b"zip").unwrap();
        let ok = ComposeRequest {
            to: vec![EmailAddress::new("", "a@example.com")],
            subject: "S".into(),
            body: "B".into(),
            attachments: vec![file.clone()],
            send: true,
            ..Default::default()
        };
        assert!(send_problems(&ok).is_empty());
        let bad = ComposeRequest {
            to: vec![EmailAddress::new("", "not an address")],
            subject: " ".into(),
            attachments: vec![dir.join("missing.zip"), dir.clone()],
            ..Default::default()
        };
        let p = send_problems(&bad);
        assert_eq!(p.len(), 5, "{p:?}");
        assert!(send_problems(&ComposeRequest::default()).iter().any(|p| p == "no recipient"));

        let db = dir.join("t.db");
        // Answered by the app.
        let req = submit(&db, &ok).unwrap();
        let taken = take(&db);
        assert!(taken[0].send && !taken[0].id.is_empty() && !req.exists());
        let answer = SendResult { sent: true, text: "Sent".into() };
        write_result(&db, &taken[0].id, &answer).unwrap();
        assert_eq!(wait_result(&req, Duration::from_secs(5), Duration::from_secs(5), |_| {}), Waited::Done(answer));
        assert!(fs::read_dir(spool_dir(&db)).unwrap().next().is_none());
        // Nobody takes it: withdrawn, never sent later.
        let req = submit(&db, &ok).unwrap();
        assert_eq!(wait_result(&req, Duration::ZERO, Duration::from_secs(5), |_| {}), Waited::NotTaken);
        assert!(take(&db).is_empty());
        // Taken but unanswered.
        let req = submit(&db, &ok).unwrap();
        take(&db);
        assert_eq!(wait_result(&req, Duration::ZERO, Duration::ZERO, |_| {}), Waited::NoAnswer);
        assert!(write_result(&db, "../x", &SendResult::default()).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn spool_round_trip() {
        let dir = std::env::temp_dir().join(format!("axentra-spool-{}-{}", std::process::id(), line!()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let db = dir.join("test.db");
        assert!(take(&db).is_empty());
        let a = ComposeRequest { subject: "one".into(), ..Default::default() };
        let b = ComposeRequest { subject: "two".into(), attachments: vec!["/tmp/x.zip".into()], ..Default::default() };
        let pa = submit(&db, &a).unwrap();
        submit(&db, &b).unwrap();
        assert_eq!(fs::metadata(&pa).unwrap().mode() & 0o777, 0o600);
        assert_eq!(fs::metadata(spool_dir(&db)).unwrap().mode() & 0o777, 0o700);
        let got = take(&db);
        assert_eq!(got.iter().map(|r| r.subject.as_str()).collect::<Vec<_>>(), ["one", "two"]);
        assert_eq!(got[1].attachments, b.attachments);
        assert!(got.iter().all(|r| !r.id.is_empty() && !r.send));
        let _ = a;
        assert!(take(&db).is_empty());

        // Too old, or junk: removed, not returned.
        let old = submit(&db, &ComposeRequest::default()).unwrap();
        let f = fs::File::options().write(true).open(&old).unwrap();
        f.set_modified(SystemTime::now() - MAX_AGE - Duration::from_secs(5)).unwrap();
        fs::write(spool_dir(&db).join("junk.json"), b"{not json").unwrap();
        assert!(take(&db).is_empty());
        assert_eq!(fs::read_dir(spool_dir(&db)).unwrap().count(), 0);

        // A spool others can read is not used.
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(spool_dir(&db), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(submit(&db, &ComposeRequest::default()).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
