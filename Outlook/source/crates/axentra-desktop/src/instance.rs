//! One running app per database: launching Axentra Outlook again (from
//! Axentra Command, the app menu or a terminal) shows the window that is
//! already open instead of starting a second copy.
//!
//! The first launch takes an `flock` on `instance.lock` in a private runtime
//! directory keyed by the database path (`$XDG_RUNTIME_DIR/axentra-outlook-<hash>`,
//! or `/tmp/axentra-<uid>/…` without one), holds it for the life of the
//! process (a crash releases it, so there is no stale lock) and binds a
//! datagram socket `show` next to it before anything slow starts. A later
//! launch that finds the lock taken sends `show` there, with its activation
//! token if the launcher gave it one, and waits for the answer: the running
//! app restores, raises and focuses its main window on the GTK thread
//! ([`present_main_window`]) and answers [`Answer::Shown`], or
//! [`Answer::Starting`] while its window is not up yet. An app that is
//! quitting answers [`Answer::Closing`], from the moment its close was
//! asked for ([`closing`]: its own Close button can take seconds to end
//! the process on a busy machine, the window listed all the while), and
//! the launch waits for the lock and runs the app itself. A holder that does not answer within [`WAIT`]
//! (frozen, or its queue full) makes the launch give up with
//! [`ClaimError::NotAnswering`]: never a second copy, never a hang.
//!
//! The lock only guards copies that look for it. [`other_copies`] finds any
//! other Axentra Outlook process with the database open: one started before
//! this version (0.1.2 took no lock), or one whose lock lives elsewhere
//! (another `XDG_RUNTIME_DIR`). [`start`] runs that check while holding the
//! lock and, when it refuses, lets go of the lock before the user is told
//! why: a launch made while that message shows (the old copy now closed, as
//! it says) runs the app instead of being answered by a process that is
//! about to exit.
//!
//! Two launches at the same instant with different runtime directories take
//! different locks, and neither has the database open yet when the other
//! looks. So every launch also takes an `flock` on `<db>.instance` next to
//! the database ([`lock_db`]): the one that finds it taken waits for the
//! holder to show up with the database open and is refused like any other
//! copy, instead of both running (or one failing on "database is locked").
//!
//! Another database (`AXENTRA_DB`, e.g. a test profile) is another key, so it
//! runs side by side.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{SocketAddr, UnixDatagram};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

/// How long a second launch waits for the running app to answer (or, when
/// it holds the lock but has not bound its socket yet, to bind it).
pub const WAIT: Duration = Duration::from_secs(3);
/// How long a launch waits for an app that answered [`Answer::Closing`] to
/// let go of the lock.
pub const CLOSING_WAIT: Duration = Duration::from_secs(10);
/// After an answer that the window was shown: how long the launch makes
/// sure the app stays. One asked just after its Close was clicked can
/// answer before it gets to the click (a busy machine: seconds later), and
/// then says it is closing after all ([`closing`]) or exits; the launch
/// then waits for the lock and runs the app instead of leaving none open.
const SHOWN_GRACE: Duration = Duration::from_millis(2000);

const LOCK_FILE: &str = "instance.lock";
const SOCKET: &str = "show";
const SHOW: &[u8] = b"show";
/// Longest activation token passed on (they are short ids in practice).
const MAX_TOKEN: usize = 256;

/// The outcome of [`claim`].
pub enum Claim {
    /// This process runs the app: it holds the lock and answers later launches.
    Primary(Primary),
    /// The app was already running and has shown its window (or will, as
    /// soon as it is up).
    Shown,
}

/// The running app's side: the lock and, normally, the bound `show`
/// socket. Without the socket (it could not be bound) the lock is still
/// held: later launches then give up instead of opening the database again.
pub struct Primary {
    lock: File,
    socket: Option<UnixDatagram>,
}

/// The lock, held until it is dropped (for the app: until the process exits).
pub struct Held {
    _lock: File,
}

/// What the running app tells a later launch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    /// The window was shown.
    Shown,
    /// The app is starting; its window shows when it is up.
    Starting,
    /// The app is quitting: the launch should wait for the lock and run.
    Closing,
}

impl Answer {
    fn bytes(self) -> &'static [u8] {
        match self {
            Answer::Shown => b"shown",
            Answer::Starting => b"starting",
            Answer::Closing => b"closing",
        }
    }

    fn parse(b: &[u8]) -> Option<Answer> {
        [Answer::Shown, Answer::Starting, Answer::Closing].into_iter().find(|a| a.bytes() == b)
    }
}

/// A later launch's request to show the window: its activation token, and
/// where to send the [`Answer`].
pub struct Request {
    pub token: Option<String>,
    reply: Option<(Arc<UnixDatagram>, SocketAddr)>,
}

/// Launches told [`Answer::Shown`] lately: [`closing`] tells them it is
/// closing after all.
static SHOWN_TO: std::sync::Mutex<Vec<(Arc<UnixDatagram>, SocketAddr, Instant)>> = std::sync::Mutex::new(Vec::new());

impl Request {
    /// Tell the launch what happened (once; later answers are ignored by
    /// it, except a [`closing`] within [`SHOWN_GRACE`] of "shown").
    pub fn answer(&self, a: Answer) {
        let Some((socket, to)) = &self.reply else { return };
        if let Err(e) = socket.send_to_addr(a.bytes(), to) {
            tracing::debug!("instance: answer not delivered: {e}");
        } else if a == Answer::Shown
            && let Ok(mut told) = SHOWN_TO.lock()
        {
            told.retain(|(_, _, at)| at.elapsed() < SHOWN_GRACE);
            told.push((socket.clone(), to.clone(), Instant::now()));
        }
    }
}

impl Primary {
    /// Answer later launches on a thread: `on_show` runs for each `show`
    /// request, which it answers (see [`Request::answer`]). Keep the
    /// returned [`Held`] alive as long as the app runs.
    pub fn serve(self, on_show: impl Fn(Request) + Send + 'static) -> Held {
        let Some(socket) = self.socket else {
            return Held { _lock: self.lock };
        };
        let replies = socket.try_clone().map(Arc::new).ok();
        let spawned = std::thread::Builder::new().name("instance".into()).spawn(move || {
            let mut buf = [0u8; 1024];
            loop {
                match socket.recv_from(&mut buf) {
                    Ok((n, from)) => match parse(&buf[..n]) {
                        // A panic here must not end the listener: later
                        // launches would find nobody answering.
                        Some(token) => {
                            let reply = replies.clone().filter(|_| !from.is_unnamed()).map(|s| (s, from));
                            let req = Request { token, reply };
                            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| on_show(req))).is_err() {
                                tracing::warn!("instance: show request failed");
                            }
                        }
                        None => tracing::debug!(len = n, "instance: ignored a malformed request"),
                    },
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(e) => {
                        tracing::warn!("instance: show socket: {e}");
                        return;
                    }
                }
            }
        });
        if let Err(e) = spawned {
            tracing::warn!("instance: listener thread: {e}");
        }
        Held { _lock: self.lock }
    }
}

/// Why [`claim`] could neither run the app nor reach the running one.
#[derive(Debug)]
pub enum ClaimError {
    /// The runtime directory or lock could not be set up; the app can still
    /// run, just without the single-instance lock (see [`other_copies`]).
    Setup(io::Error),
    /// Another process holds the lock but did not answer in time.
    NotAnswering,
}

impl std::fmt::Display for ClaimError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClaimError::Setup(e) => write!(f, "{e}"),
            ClaimError::NotAnswering => f.write_str("Axentra Outlook is already open on this profile but is not responding"),
        }
    }
}

/// Run as the app, or ask the one already running on `db` to show itself.
/// `token` is this launch's activation token ([`launch_token`]).
pub fn claim(db: &Path, token: Option<&str>) -> Result<Claim, ClaimError> {
    let dir = runtime_dir(db).map_err(ClaimError::Setup)?;
    claim_in(&dir, token, WAIT)
}

/// What a launch does (see [`start`]).
pub enum Start {
    /// Run the app, answering later launches through the [`Primary`] (none
    /// when the runtime directory could not be used; a warning was shown),
    /// and holding the database's own lock ([`DbLock`]) until exit.
    Run(Option<Primary>, DbLock),
    /// The running app has shown its window (or will once it is up).
    Shown,
    /// Another copy has the database open, one the lock can't see. The lock
    /// has been let go already.
    Refused(OtherCopy),
}

/// [`claim`], then the check for copies the lock can't see
/// ([`other_copies`]). Call first in `main`, before anything slow.
pub fn start(db: &Path, token: Option<&str>) -> Result<Start, ClaimError> {
    decide(claim(db, token), || other_copies(db), || lock_db(db, DB_LOCK_WAIT), WAIT)
}

fn decide(
    claimed: Result<Claim, ClaimError>,
    others: impl Fn() -> Vec<OtherCopy>,
    db_lock: impl FnOnce() -> DbLock,
    wait: Duration,
) -> Result<Start, ClaimError> {
    let primary = match claimed {
        Ok(Claim::Primary(p)) => Some(p),
        Ok(Claim::Shown) => return Ok(Start::Shown),
        // No usable runtime directory: run without the lock (the check for
        // other copies still stands).
        Err(ClaimError::Setup(e)) => {
            warn_visibly(&format!("single instance: {e}"));
            None
        }
        Err(e) => return Err(e),
    };
    let db = db_lock();
    if db.taken {
        // A copy whose instance lock is elsewhere got the database first;
        // it opens it straight after. Let go, then name it once it shows.
        drop(primary);
        let deadline = Instant::now() + wait;
        loop {
            if let Some(other) = others().into_iter().next() {
                return Ok(Start::Refused(other));
            }
            if Instant::now() >= deadline {
                return Err(ClaimError::NotAnswering);
            }
            std::thread::sleep(POLL.min(wait));
        }
    }
    // Checked while holding the locks (a launch that comes in meanwhile
    // waits), and not served yet: a refused launch must never answer
    // anyone. It lets go here, before it tells the user, so a launch made
    // while that message is up runs the app.
    match others().into_iter().next() {
        Some(other) => {
            drop(primary);
            drop(db);
            Ok(Start::Refused(other))
        }
        None => Ok(Start::Run(primary, db)),
    }
}

/// How long a launch waits for `<db>.instance` when it is taken: a copy
/// that is quitting lets go as it exits.
const DB_LOCK_WAIT: Duration = Duration::from_millis(1000);

/// The database's own lock (`<db>.instance`, see the module docs), held
/// while it is kept. `taken`: another process holds it. Neither held nor
/// taken: the file could not be used (a read-only folder), and the other
/// checks stand alone.
pub struct DbLock {
    _file: Option<File>,
    taken: bool,
}

/// `<db>.instance` next to the database's real path.
fn db_lock_path(db: &Path) -> Option<PathBuf> {
    let name = db.file_name()?;
    let dir = db.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    fs::create_dir_all(dir).ok()?;
    let real = fs::canonicalize(db).ok().or_else(|| fs::canonicalize(dir).ok().map(|d| d.join(name)))?;
    let mut lock = real.file_name()?.to_os_string();
    lock.push(".instance");
    Some(real.with_file_name(lock))
}

/// Take `<db>.instance`, waiting up to `wait` while another process has it.
pub fn lock_db(db: &Path, wait: Duration) -> DbLock {
    let unusable = DbLock { _file: None, taken: false };
    let Some(path) = db_lock_path(db) else { return unusable };
    let Ok(file) = OpenOptions::new().read(true).write(true).create(true).truncate(false).mode(0o600).custom_flags(libc::O_NOFOLLOW).open(&path) else {
        return unusable;
    };
    let deadline = Instant::now() + wait;
    loop {
        match file.try_lock() {
            Ok(()) => return DbLock { _file: Some(file), taken: false },
            Err(fs::TryLockError::WouldBlock) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            Err(fs::TryLockError::WouldBlock) => return DbLock { _file: None, taken: true },
            Err(fs::TryLockError::Error(_)) => return unusable,
        }
    }
}

/// [`claim`] in a given (private) directory, waiting up to `wait` for an
/// answer. The directory is used by its canonical path (an
/// `XDG_RUNTIME_DIR` reached through a symlink, such as
/// `/var/run/user/1000`), so every launch names the socket the same way and
/// recognises the running app's answer as coming from it.
pub fn claim_in(dir: &Path, token: Option<&str>, wait: Duration) -> Result<Claim, ClaimError> {
    let dir = fs::canonicalize(dir).map_err(ClaimError::Setup)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(dir.join(LOCK_FILE))
        .map_err(ClaimError::Setup)?;
    let socket = dir.join(SOCKET);
    let msg = message(token);
    let asker = Asker::new();
    let mut deadline = Instant::now() + wait;
    // The socket of a holder that said it is quitting: wait while it is
    // still the one there.
    let mut closing: Option<Option<SocketId>> = None;
    loop {
        match lock.try_lock() {
            Ok(()) => return Ok(Claim::Primary(bind(lock, &socket))),
            Err(fs::TryLockError::WouldBlock) => {}
            Err(fs::TryLockError::Error(e)) => return Err(ClaimError::Setup(e)),
        }
        let now = Instant::now();
        if now >= deadline {
            return Err(ClaimError::NotAnswering);
        }
        match closing {
            Some(quitting) if socket_id(&socket) == quitting => {}
            // The lock went to another launch (it bound a new socket): ask that one.
            Some(_) => {
                closing = None;
                deadline = now + wait;
                continue;
            }
            None => {
                // Held by a running app: its socket is bound right after the
                // lock, so a failed send means it is just starting (or just
                // crashed, and the lock is about to be free), or that its
                // queue is full because it has stopped reading.
                let asked = socket_id(&socket);
                // Stop waiting as soon as the holder has gone: a request that
                // reached an app busy closing is never answered (it exits
                // first), and the lock is free at once. (Taking it here keeps
                // it: the next try_lock on the same file succeeds.)
                let gone = || socket_id(&socket) != asked || lock.try_lock().is_ok();
                match asker.ask(&msg, &socket, deadline - now, gone) {
                    Asked::Answer(Answer::Shown | Answer::Starting) => match after_answer(&lock, &asker, &socket) {
                        AfterAnswer::Gone => return Ok(Claim::Primary(bind(lock, &socket))),
                        AfterAnswer::Closing => {
                            closing = Some(asked);
                            deadline = Instant::now() + CLOSING_WAIT;
                            continue;
                        }
                        AfterAnswer::Stays => return Ok(Claim::Shown),
                    },
                    Asked::Answer(Answer::Closing) => {
                        closing = Some(asked);
                        deadline = Instant::now() + CLOSING_WAIT;
                        continue;
                    }
                    // No answer: the app asked went away without one and
                    // another launch has the lock now (ask it), or one last
                    // look at the lock.
                    Asked::NoAnswer => {
                        deadline = if socket_id(&socket) != asked { Instant::now() + wait } else { Instant::now() };
                        continue;
                    }
                    Asked::NotSent => {}
                }
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// What the app that answered does within [`SHOWN_GRACE`]: lets go of the
/// lock (it quit; the caller then holds it), says it is closing after all,
/// or stays.
enum AfterAnswer {
    Gone,
    Closing,
    Stays,
}

fn after_answer(lock: &File, asker: &Asker, holder: &Path) -> AfterAnswer {
    let until = Instant::now() + SHOWN_GRACE;
    loop {
        if lock.try_lock().is_ok() {
            return AfterAnswer::Gone;
        }
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return AfterAnswer::Stays;
        }
        if asker.listen(holder, left.min(Duration::from_millis(25))) == Some(Answer::Closing) {
            return AfterAnswer::Closing;
        }
    }
}

/// One binding of the `show` socket: a new holder removes the old file and
/// binds a new one, so this changes when the lock has changed hands.
type SocketId = (u64, u64, i64, i64);

fn socket_id(path: &Path) -> Option<SocketId> {
    fs::symlink_metadata(path).ok().map(|m| (m.dev(), m.ino(), m.ctime(), m.ctime_nsec()))
}

/// How often a launch waiting for an answer checks whether the app it asked
/// has been replaced.
const POLL: Duration = Duration::from_millis(100);

/// The launch's end of the conversation: a socket the answer can come back
/// to (an abstract, autobound-style name, so nothing is left on disk).
struct Asker {
    socket: Option<UnixDatagram>,
}

enum Asked {
    Answer(Answer),
    /// Sent, but no answer in time.
    NoAnswer,
    /// Could not be sent (no socket bound yet, or the holder's queue is full).
    NotSent,
}

impl Asker {
    fn new() -> Self {
        Asker { socket: reply_socket().map_err(|e| tracing::debug!("instance: no answer socket: {e}")).ok() }
    }

    /// Send `msg` and wait up to `wait` for the answer, giving up early
    /// when `replaced()` (the holder asked has gone and another took over).
    fn ask(&self, msg: &[u8], to: &Path, wait: Duration, replaced: impl Fn() -> bool) -> Asked {
        let Some(s) = &self.socket else {
            // Nowhere for an answer to come back to: sending has to do.
            let sent = UnixDatagram::unbound().and_then(|u| {
                u.set_nonblocking(true)?;
                u.send_to(msg, to)
            });
            return if sent.is_ok() { Asked::Answer(Answer::Shown) } else { Asked::NotSent };
        };
        if s.set_nonblocking(true).and_then(|()| s.send_to(msg, to)).is_err() {
            return Asked::NotSent;
        }
        let deadline = Instant::now() + wait;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Asked::NoAnswer;
            }
            match self.recv(to, left.min(POLL)) {
                Ok(Some(a)) => return Asked::Answer(a),
                Ok(None) if replaced() => return Asked::NoAnswer,
                Ok(None) => {}
                Err(()) => return Asked::NoAnswer,
            }
        }
    }

    /// A later word from the holder at `from` (a [`closing`] after it said
    /// the window was shown), waiting up to `wait`.
    fn listen(&self, from: &Path, wait: Duration) -> Option<Answer> {
        if self.socket.is_none() {
            std::thread::sleep(wait);
            return None;
        }
        self.recv(from, wait).ok().flatten()
    }

    /// One answer from `from` within `wait` (`Ok(None)`: none came);
    /// `Err` when the socket fails.
    fn recv(&self, from: &Path, wait: Duration) -> Result<Option<Answer>, ()> {
        let Some(s) = &self.socket else { return Err(()) };
        s.set_nonblocking(false).and_then(|()| s.set_read_timeout(Some(wait.max(Duration::from_millis(1))))).map_err(|_| ())?;
        let mut buf = [0u8; 64];
        match s.recv_from(&mut buf) {
            // Only the holder's socket can answer: it lives in our private directory.
            Ok((n, addr)) if addr.as_pathname() == Some(from) => Ok(Answer::parse(&buf[..n])),
            Ok(_) => Ok(None),
            Err(e) if matches!(e.kind(), io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => Ok(None),
            Err(_) => Err(()),
        }
    }
}

#[cfg(target_os = "linux")]
fn reply_socket() -> io::Result<UnixDatagram> {
    use std::os::linux::net::SocketAddrExt;
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    let name = format!("axentra-outlook-launch-{}-{nanos}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed));
    UnixDatagram::bind_addr(&SocketAddr::from_abstract_name(name.as_bytes())?)
}

#[cfg(not(target_os = "linux"))]
fn reply_socket() -> io::Result<UnixDatagram> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "no abstract sockets"))
}

/// Bind the `show` socket for the lock's new holder. A socket file left by
/// an earlier run is stale: whoever bound it no longer holds the lock. When
/// binding fails the lock is kept anyway (see [`Primary`]).
fn bind(lock: File, path: &Path) -> Primary {
    let bound = (|| {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        let socket = UnixDatagram::bind(path)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        Ok(socket)
    })();
    match bound {
        Ok(socket) => Primary { lock, socket: Some(socket) },
        Err(e) => {
            warn_visibly(&format!("single instance: {}: {e}; later launches will not reach this window", path.display()));
            Primary { lock, socket: None }
        }
    }
}

/// A warning for the log and, since the log is off by default, for stderr.
pub fn warn_visibly(msg: &str) {
    tracing::warn!("{msg}");
    eprintln!("axentra-outlook: {msg}");
}

/// The request a later launch sends: `show`, then a space and the
/// activation token if it has a valid one.
pub fn message(token: Option<&str>) -> Vec<u8> {
    let mut m = SHOW.to_vec();
    if let Some(t) = token.filter(|t| valid_token(t)) {
        m.push(b' ');
        m.extend_from_slice(t.as_bytes());
    }
    m
}

/// A request's token: `Some(token)` for a `show` request (an invalid
/// token is dropped), `None` for anything else.
pub fn parse(msg: &[u8]) -> Option<Option<String>> {
    let rest = msg.strip_prefix(SHOW)?;
    if rest.is_empty() {
        return Some(None);
    }
    let token = std::str::from_utf8(rest.strip_prefix(b" ")?).ok()?;
    Some(valid_token(token).then(|| token.to_string()))
}

/// Startup ids and activation tokens are short printable ids, e.g.
/// `gnome-shell-1234-host-axentra-outlook-1_TIME5678` or a UUID.
pub fn valid_token(t: &str) -> bool {
    !t.is_empty() && t.len() <= MAX_TOKEN && t.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.:+=/~@".contains(&b))
}

/// This launch's activation token: Wayland's `XDG_ACTIVATION_TOKEN`, else
/// X11's startup-notification `DESKTOP_STARTUP_ID`.
pub fn launch_token() -> Option<String> {
    ["XDG_ACTIVATION_TOKEN", "DESKTOP_STARTUP_ID"]
        .into_iter()
        .filter_map(|k| std::env::var(k).ok())
        .find(|t| valid_token(t))
}

/// What identifies a profile: the database path made absolute, through
/// symlinks as far as it exists, the rest as written. A database that does
/// not exist yet, or a profile folder not created yet, keys the same as once
/// the store has created them.
pub fn key_path(db: &Path) -> PathBuf {
    let abs = if db.is_absolute() {
        db.to_path_buf()
    } else {
        std::env::current_dir().map(|d| d.join(db)).unwrap_or_else(|_| db.to_path_buf())
    };
    let mut head = abs.clone();
    let mut tail = Vec::new();
    loop {
        if let Ok(mut p) = fs::canonicalize(&head) {
            for c in tail.iter().rev() {
                if c == ".." {
                    p.pop();
                } else if c != "." {
                    p.push(c);
                }
            }
            return p;
        }
        let Some(last) = head.components().next_back().map(|c| c.as_os_str().to_owned()) else { return abs };
        if !head.pop() {
            return abs;
        }
        tail.push(last);
    }
}

/// The directory name for a profile: the app name and a hash of its key.
pub fn dir_name(db: &Path) -> String {
    let hash = Sha256::digest(key_path(db).as_os_str().as_encoded_bytes());
    let hex: String = hash.iter().take(10).map(|b| format!("{b:02x}")).collect();
    format!("axentra-outlook-{hex}")
}

/// The profile's private runtime directory, created if needed.
pub fn runtime_dir(db: &Path) -> io::Result<PathBuf> {
    let dir = runtime_base()?.join(dir_name(db));
    private_dir(&dir)?;
    Ok(dir)
}

fn euid() -> u32 {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

/// `$XDG_RUNTIME_DIR` when it is absolute, ours and not writable by others;
/// else a private `axentra-<uid>` directory in the temp dir (the suite's
/// shared fallback, as in Axentra Command).
fn runtime_base() -> io::Result<PathBuf> {
    if let Some(d) = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|p| p.is_absolute())
        && fs::metadata(&d).is_ok_and(|m| m.is_dir() && m.uid() == euid() && m.mode() & 0o022 == 0)
    {
        return Ok(d);
    }
    let base = std::env::temp_dir().join(format!("axentra-{}", euid()));
    private_dir(&base)?;
    Ok(base)
}

/// Create `path` as a 0700 directory, or check an existing one: it must be
/// a real directory (not a symlink) owned by this user. Looser permissions
/// are tightened through an open handle, so a swapped path cannot redirect
/// the change.
pub fn private_dir(path: &Path) -> io::Result<()> {
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    let dir = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY)
        .open(path)
        .map_err(|e| io::Error::new(e.kind(), format!("{}: not a private directory ({e})", path.display())))?;
    let m = dir.metadata()?;
    if m.uid() != euid() {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("{} belongs to another user", path.display())));
    }
    if m.mode() & 0o077 != 0 {
        dir.set_permissions(fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Another Axentra Outlook process that has the database open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OtherCopy {
    pub pid: u32,
    /// Its program file was replaced since it started (an update installed
    /// while it ran).
    pub outdated: bool,
}

/// Other Axentra Outlook processes of this user with `db` open, found
/// through `/proc` (read only). The lock cannot see them when they take
/// none (0.1.2 and earlier) or keep it elsewhere (another runtime
/// directory); either way a second session on the same mail must not start.
pub fn other_copies(db: &Path) -> Vec<OtherCopy> {
    copies_in(Path::new("/proc"), db, std::process::id(), |exe| exe.starts_with("axentra-outlook"))
}

fn copies_in(proc: &Path, db: &Path, me: u32, is_app: impl Fn(&str) -> bool) -> Vec<OtherCopy> {
    let Ok(db) = fs::canonicalize(db) else { return Vec::new() };
    let Ok(entries) = fs::read_dir(proc) else { return Vec::new() };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
        if pid == me || !entry.metadata().is_ok_and(|m| m.uid() == euid()) {
            continue;
        }
        let Ok(exe) = fs::read_link(entry.path().join("exe")) else { continue };
        let name = exe.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if !is_app(&name) {
            continue;
        }
        let Ok(fds) = fs::read_dir(entry.path().join("fd")) else { continue };
        if fds.flatten().any(|fd| fs::read_link(fd.path()).is_ok_and(|t| t == db)) {
            found.push(OtherCopy { pid, outdated: name.ends_with(" (deleted)") });
        }
    }
    found.sort_by_key(|c| c.pid);
    found
}

#[cfg(target_os = "linux")]
mod gtk_side {
    use std::sync::atomic::{AtomicBool, Ordering};

    /// The GTK main loop is running (set by its first idle).
    pub static LOOP_UP: AtomicBool = AtomicBool::new(false);
    /// The main window has existed: from then on, no window means quitting.
    pub static WINDOW_SEEN: AtomicBool = AtomicBool::new(false);
    /// The app's close was asked for ([`super::closing`]).
    pub static CLOSING: AtomicBool = AtomicBool::new(false);

    pub fn up() -> bool {
        LOOP_UP.load(Ordering::Acquire)
    }

    pub fn seen() -> bool {
        WINDOW_SEEN.load(Ordering::Acquire)
    }

    pub fn set_seen() {
        WINDOW_SEEN.store(true, Ordering::Release);
    }
}

/// The app has begun to close (its Close button, or the window manager's
/// close): from now on a launch is told it is quitting, waits for the lock
/// and opens the app, instead of being told the window was shown while it
/// goes.
pub fn closing() {
    #[cfg(target_os = "linux")]
    gtk_side::CLOSING.store(true, std::sync::atomic::Ordering::Release);
    // Told "shown" just before the close (its click still on its way): now
    // they wait for the lock too.
    let told = SHOWN_TO.lock().map(|mut t| std::mem::take(&mut *t)).unwrap_or_default();
    for (socket, to, at) in told {
        if at.elapsed() < SHOWN_GRACE {
            let _ = socket.send_to_addr(Answer::Closing.bytes(), &to);
        }
    }
}

/// What a `show` request is answered now: [`Answer::Closing`] once the app
/// began closing (without showing a window that is going) or after its
/// window has gone, [`Answer::Shown`] when `present` showed it, `None`
/// while it is not up yet.
fn show_answer(closing: bool, present: impl FnOnce() -> bool, seen: bool) -> Option<Answer> {
    if closing {
        Some(Answer::Closing)
    } else if present() {
        Some(Answer::Shown)
    } else if seen {
        Some(Answer::Closing)
    } else {
        None
    }
}

/// Note when the GTK main loop runs and when the main window exists, so
/// requests can be answered [`Answer::Starting`] or [`Answer::Closing`].
/// Call on the main thread before the event loop starts.
#[cfg(target_os = "linux")]
pub fn watch_gtk() {
    use std::sync::atomic::Ordering;
    gtk::glib::idle_add_once(|| {
        gtk_side::LOOP_UP.store(true, Ordering::Release);
        gtk::glib::timeout_add_local(Duration::from_millis(250), || {
            if main_window().is_some() {
                gtk_side::set_seen();
                gtk::glib::ControlFlow::Break
            } else {
                gtk::glib::ControlFlow::Continue
            }
        });
    });
}

/// Hand a `show` request to the GTK main thread, which answers it. While
/// the main loop is not running yet (the app is still opening its
/// database), the answer is [`Answer::Starting`] from here and the window
/// shows once it is up. An idle source on the default main context always
/// runs there (`MainContext::invoke` would not do: while the main loop is
/// between iterations it runs the function right away on the calling
/// thread).
#[cfg(target_os = "linux")]
pub fn show_on_gtk_thread(req: Request) {
    if !gtk_side::up() {
        req.answer(Answer::Starting);
    }
    gtk::glib::idle_add_once(move || present_when_ready(req, 0));
}

/// [`present_main_window`], retried for a few seconds while the window
/// does not exist yet (a request that came in while the app was starting).
#[cfg(target_os = "linux")]
fn present_when_ready(req: Request, tries: u32) {
    use std::sync::atomic::Ordering;
    let closing = gtk_side::CLOSING.load(Ordering::Acquire);
    let shown = || {
        let shown = present_main_window(req.token.as_deref());
        if shown {
            gtk_side::set_seen();
        }
        shown
    };
    if let Some(answer) = show_answer(closing, shown, gtk_side::seen()) {
        req.answer(answer);
        return;
    }
    if tries == 0 {
        req.answer(Answer::Starting);
    }
    if tries < 25 {
        gtk::glib::timeout_add_local_once(Duration::from_millis(200), move || present_when_ready(req, tries + 1));
    }
}

#[cfg(target_os = "linux")]
fn main_window() -> Option<gtk::ApplicationWindow> {
    use gtk::prelude::*;
    gtk::Window::list_toplevels()
        .into_iter()
        .filter_map(|w| w.downcast::<gtk::ApplicationWindow>().ok())
        .find(|w| w.window_type() == gtk::WindowType::Toplevel)
}

/// Restore, raise and focus the app's main window, on the GTK main thread.
/// False while there is no window yet.
#[cfg(target_os = "linux")]
fn present_main_window(token: Option<&str>) -> bool {
    use gtk::prelude::*;

    if !gtk::is_initialized_main_thread() {
        return false;
    }
    let Some(window) = main_window() else {
        tracing::debug!("instance: show requested while there is no window");
        return false;
    };
    let display = WidgetExt::display(&window);
    let backend = display.type_().name();
    tracing::debug!(backend, token = token.is_some(), "instance: presenting the main window");
    window.deiconify();
    window.show();
    match backend {
        // Wayland: focus needs an xdg-activation token from the launcher;
        // GDK activates with it on the next present. Without one the
        // compositor decides (GNOME shows "… is ready" and leaves a
        // minimized window minimized). A token that turns out stale
        // (inherited from an earlier launch) does nothing at all, so when
        // the window is still not active a moment later it is presented
        // again without it, which at least gets the "is ready" notice.
        "GdkWaylandDisplay" => {
            if let Some(t) = token.and_then(|t| std::ffi::CString::new(t).ok()) {
                // SAFETY: a live GdkWaylandDisplay (checked above) and a
                // NUL-terminated string; GDK copies it.
                unsafe { gdk_wayland_display_set_startup_notification_id(display.as_ptr().cast(), t.as_ptr()) };
                let w = window.clone();
                gtk::glib::timeout_add_local_once(Duration::from_millis(400), move || {
                    if !w.is_active() {
                        tracing::debug!("instance: activation token did nothing; presenting without it");
                        w.present_with_time(0);
                    }
                });
            }
            window.present_with_time(0);
        }
        // X11: the launcher's startup id ends its busy cursor and carries
        // the click's time; a fresh server time lets Mutter raise rather
        // than only flash the window.
        "GdkX11Display" => {
            if let Some(t) = token {
                window.set_startup_id(t);
            }
            let time = window.window().map_or(0, |w| {
                // SAFETY: the realized GdkX11Window of a window on an X11 display.
                unsafe { gdk_x11_get_server_time(w.as_ptr().cast()) }
            });
            window.present_with_time(time);
        }
        _ => window.present(),
    }
    true
}

#[cfg(target_os = "linux")]
unsafe extern "C" {
    fn gdk_wayland_display_set_startup_notification_id(display: *mut std::ffi::c_void, startup_id: *const std::ffi::c_char);
    fn gdk_x11_get_server_time(window: *mut std::ffi::c_void) -> u32;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::FileTypeExt;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;

    /// A fresh directory with a short path (socket paths are limited to
    /// 108 bytes), removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            static N: AtomicUsize = AtomicUsize::new(0);
            let base = if std::env::temp_dir().as_os_str().len() < 60 { std::env::temp_dir() } else { PathBuf::from("/tmp") };
            let p = base.join(format!("axo-inst-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(&p).unwrap();
            fs::set_permissions(&p, fs::Permissions::from_mode(0o700)).unwrap();
            TempDir(p)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn mode(p: &Path) -> u32 {
        fs::symlink_metadata(p).unwrap().mode() & 0o777
    }

    /// A holder that answers every request with `answer` and reports the
    /// tokens it got.
    fn answering(primary: Primary, answer: Answer) -> (Held, mpsc::Receiver<Option<String>>) {
        let (tx, rx) = mpsc::channel();
        let held = primary.serve(move |req| {
            let _ = tx.send(req.token.clone());
            req.answer(answer);
        });
        (held, rx)
    }

    fn primary(dir: &Path) -> Primary {
        match claim_in(dir, None, Duration::ZERO) {
            Ok(Claim::Primary(p)) => p,
            _ => panic!("the first launch runs the app"),
        }
    }

    #[test]
    fn the_key_is_the_database_path() {
        let t = TempDir::new();
        let a = t.0.join("a.db");
        let b = t.0.join("b.db");
        assert_eq!(dir_name(&a), dir_name(&a));
        assert_ne!(dir_name(&a), dir_name(&b));
        assert!(dir_name(&a).starts_with("axentra-outlook-") && dir_name(&a).len() == "axentra-outlook-".len() + 20);
        // Not created yet keys the same as created.
        let before = dir_name(&a);
        fs::write(&a, b"").unwrap();
        assert_eq!(dir_name(&a), before);
        // Through a symlinked directory and a symlinked file.
        std::os::unix::fs::symlink(&t.0, t.0.join("link")).unwrap();
        assert_eq!(dir_name(&t.0.join("link").join("a.db")), before);
        std::os::unix::fs::symlink(&a, t.0.join("alias.db")).unwrap();
        assert_eq!(dir_name(&t.0.join("alias.db")), before);
        // Relative paths are taken from the working directory.
        assert!(key_path(Path::new("rel.db")).is_absolute());
        assert_eq!(key_path(Path::new("rel.db")), std::env::current_dir().unwrap().join("rel.db"));
    }

    /// A fresh profile behind a symlink (HOME, ~/.local/share, Silverblue's
    /// /home): the folders the store creates later must not change the key.
    #[test]
    fn a_profile_not_created_yet_keys_the_same_through_a_symlink() {
        let t = TempDir::new();
        fs::create_dir(t.0.join("real")).unwrap();
        std::os::unix::fs::symlink(t.0.join("real"), t.0.join("home")).unwrap();
        let db = t.0.join("home/.local/share/axentra-outlook/axentra.db");
        let first = dir_name(&db);
        fs::create_dir_all(t.0.join("real/.local/share/axentra-outlook")).unwrap();
        assert_eq!(dir_name(&db), first, "after the folders exist");
        fs::write(t.0.join("real/.local/share/axentra-outlook/axentra.db"), b"").unwrap();
        assert_eq!(dir_name(&db), first, "after the database exists");
        assert_eq!(key_path(&db), fs::canonicalize(t.0.join("real")).unwrap().join(".local/share/axentra-outlook/axentra.db"));
        // Dots in the part that doesn't exist yet are resolved as written.
        assert_eq!(key_path(&t.0.join("home/x/../y/./z.db")), fs::canonicalize(t.0.join("real")).unwrap().join("y/z.db"));
    }

    #[test]
    fn private_dirs_are_created_tightened_or_refused() {
        let t = TempDir::new();
        let fresh = t.0.join("fresh");
        private_dir(&fresh).unwrap();
        assert_eq!(mode(&fresh), 0o700);
        let loose = t.0.join("loose");
        fs::create_dir(&loose).unwrap();
        fs::set_permissions(&loose, fs::Permissions::from_mode(0o755)).unwrap();
        private_dir(&loose).unwrap();
        assert_eq!(mode(&loose), 0o700, "tightened");
        // A symlink, even to a good directory, is refused and left alone.
        let link = t.0.join("link");
        std::os::unix::fs::symlink(&fresh, &link).unwrap();
        assert!(private_dir(&link).is_err());
        assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        // So is a plain file.
        let file = t.0.join("file");
        fs::write(&file, b"").unwrap();
        assert!(private_dir(&file).is_err());
    }

    #[test]
    fn the_lock_is_exclusive_until_dropped() {
        let t = TempDir::new();
        let held = primary(&t.0).serve(|_| {});
        let other = File::open(t.0.join(LOCK_FILE)).unwrap();
        assert!(matches!(other.try_lock(), Err(fs::TryLockError::WouldBlock)));
        drop(held);
        other.try_lock().expect("free once the app is gone");
    }

    #[test]
    fn messages_carry_only_valid_tokens() {
        assert_eq!(message(None), b"show");
        assert_eq!(message(Some("gnome-shell-2210-host-axentra-outlook-3_TIME1234")), b"show gnome-shell-2210-host-axentra-outlook-3_TIME1234");
        assert_eq!(message(Some("bad token")), b"show");
        assert_eq!(message(Some("")), b"show");
        assert_eq!(message(Some(&"x".repeat(MAX_TOKEN + 1))), b"show");
        assert_eq!(parse(b"show"), Some(None));
        assert_eq!(parse(b"show abc-123_TIME9"), Some(Some("abc-123_TIME9".into())));
        assert_eq!(parse(b"show a\nb"), Some(None), "a bad token is dropped, the request kept");
        assert_eq!(parse(b"show \xff"), None);
        assert_eq!(parse(b"shower"), None);
        assert_eq!(parse(b"quit"), None);
        assert_eq!(parse(b""), None);
        assert!(valid_token("0f3e4b2a-9c1d-4e5f-8a7b-6c5d4e3f2a1b"));
        assert!(!valid_token("a;rm -rf"));
        for a in [Answer::Shown, Answer::Starting, Answer::Closing] {
            assert_eq!(Answer::parse(a.bytes()), Some(a));
        }
        assert_eq!(Answer::parse(b"show"), None);
    }

    #[test]
    fn a_second_launch_shows_the_first_and_does_not_run() {
        let t = TempDir::new();
        // A socket file left by a crashed run does not get in the way.
        fs::write(t.0.join(SOCKET), b"stale").unwrap();
        let first = primary(&t.0);
        assert!(fs::symlink_metadata(t.0.join(SOCKET)).unwrap().file_type().is_socket(), "stale file replaced by the socket");
        assert_eq!(mode(&t.0.join(SOCKET)), 0o600);
        let (_held, rx) = answering(first, Answer::Shown);
        assert!(matches!(claim_in(&t.0, Some("tok-1"), WAIT), Ok(Claim::Shown)));
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap().as_deref(), Some("tok-1"));
        assert!(matches!(claim_in(&t.0, None, WAIT), Ok(Claim::Shown)));
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), None);
        // Junk on the socket is ignored and the listener keeps going.
        UnixDatagram::unbound().unwrap().send_to(b"junk", t.0.join(SOCKET)).unwrap();
        assert!(matches!(claim_in(&t.0, None, WAIT), Ok(Claim::Shown)));
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), None);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn an_app_still_starting_counts_as_shown() {
        let t = TempDir::new();
        let (_held, rx) = answering(primary(&t.0), Answer::Starting);
        assert!(matches!(claim_in(&t.0, None, WAIT), Ok(Claim::Shown)));
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), None);
    }

    #[test]
    fn a_launch_after_the_app_exits_runs_it_again() {
        let t = TempDir::new();
        drop(primary(&t.0).serve(|_| {}));
        // The old socket file is still there; the new holder rebinds it.
        assert!(t.0.join(SOCKET).exists());
        let (_held, rx) = answering(primary(&t.0), Answer::Shown);
        assert!(matches!(claim_in(&t.0, None, WAIT), Ok(Claim::Shown)));
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), None);
    }

    #[test]
    fn a_holder_that_never_binds_is_given_up_on_without_running() {
        let t = TempDir::new();
        // The lock taken, no socket: a first launch stuck before binding.
        let lock = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(t.0.join(LOCK_FILE)).unwrap();
        lock.try_lock().unwrap();
        let start = Instant::now();
        assert!(matches!(claim_in(&t.0, None, Duration::from_millis(300)), Err(ClaimError::NotAnswering)));
        let took = start.elapsed();
        assert!(took >= Duration::from_millis(300) && took < Duration::from_secs(2), "{took:?}");
        // Once the holder binds, the waiting launch gets through.
        let dir = t.0.clone();
        let waiter = std::thread::spawn(move || matches!(claim_in(&dir, None, WAIT), Ok(Claim::Shown)));
        std::thread::sleep(Duration::from_millis(200));
        let (_held, rx) = answering(bind(lock, &t.0.join(SOCKET)), Answer::Shown);
        assert!(waiter.join().unwrap());
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), None);
    }

    /// A stopped or hung app: the socket is bound but nobody reads it. The
    /// launch must neither claim it showed the window nor hang, even once
    /// the socket's queue is full.
    #[test]
    fn a_frozen_holder_is_given_up_on_without_hanging() {
        let t = TempDir::new();
        let _frozen = primary(&t.0);
        let start = Instant::now();
        assert!(matches!(claim_in(&t.0, None, Duration::from_millis(300)), Err(ClaimError::NotAnswering)));
        assert!(start.elapsed() < Duration::from_secs(2), "{:?}", start.elapsed());
        // Fill its queue, as hundreds of launches would.
        let filler = UnixDatagram::unbound().unwrap();
        filler.set_nonblocking(true).unwrap();
        let mut sent = 0;
        while filler.send_to(b"show", t.0.join(SOCKET)).is_ok() && sent < 100_000 {
            sent += 1;
        }
        assert!(sent < 100_000, "the queue never filled");
        let start = Instant::now();
        assert!(matches!(claim_in(&t.0, None, Duration::from_millis(300)), Err(ClaimError::NotAnswering)));
        assert!(start.elapsed() < Duration::from_secs(2), "{:?}", start.elapsed());
    }

    #[test]
    fn a_request_nobody_answers_is_not_called_shown() {
        let t = TempDir::new();
        let (tx, rx) = mpsc::channel();
        let _held = primary(&t.0).serve(move |req| tx.send(req.token).unwrap());
        assert!(matches!(claim_in(&t.0, None, Duration::from_millis(300)), Err(ClaimError::NotAnswering)));
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), None, "the request did arrive");
    }

    /// A launch in the ~0.1 s between the close click and the exit: the
    /// request is queued on the closing main loop and never answered. The
    /// launch runs the app as soon as the lock is free, not after `WAIT`.
    /// Once the app began closing (its own Close can take seconds on a busy
    /// machine, the window listed all along), a launch is told it is
    /// quitting, so it waits for the lock and opens the app, instead of
    /// "showing its window" as it goes and leaving nothing open.
    #[test]
    fn an_app_that_began_closing_answers_closing() {
        let mut shown = false;
        assert_eq!(show_answer(true, || { shown = true; true }, true), Some(Answer::Closing));
        assert!(!shown, "a closing window is not shown");
        assert_eq!(show_answer(false, || true, true), Some(Answer::Shown));
        assert_eq!(show_answer(false, || false, true), Some(Answer::Closing), "the window has gone");
        assert_eq!(show_answer(false, || false, false), None, "not up yet");
    }

    #[test]
    fn a_launch_the_quitting_app_never_answers_runs_it_at_once() {
        let t = TempDir::new();
        let (tx, rx) = mpsc::channel();
        let held = primary(&t.0).serve(move |req| tx.send(req.token).unwrap());
        let dir = t.0.clone();
        let start = Instant::now();
        let waiter = std::thread::spawn(move || matches!(claim_in(&dir, None, WAIT), Ok(Claim::Primary(_))));
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        drop(held);
        assert!(waiter.join().unwrap(), "runs the app");
        assert!(start.elapsed() < Duration::from_millis(1500), "{:?}", start.elapsed());
    }

    /// The request reached the app just before it handled the close click:
    /// it shows its window, answers, and exits. The launch notices and runs
    /// the app, instead of saying it showed a window that is gone.
    #[test]
    fn an_app_that_answers_and_then_quits_is_run_again() {
        let t = TempDir::new();
        let (held, rx) = answering(primary(&t.0), Answer::Shown);
        let dir = t.0.clone();
        let waiter = std::thread::spawn(move || matches!(claim_in(&dir, None, WAIT), Ok(Claim::Primary(_))));
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
        std::thread::sleep(Duration::from_millis(80));
        drop(held);
        assert!(waiter.join().unwrap(), "runs the app");
    }

    /// A busy app answered "shown" before it got to its Close click, then
    /// says it is closing after all, and takes longer than the grace to go:
    /// the launch waits for the lock and runs the app (it printed "showing
    /// its window" and left nothing open).
    #[test]
    fn a_launch_told_closing_after_shown_runs_the_app() {
        let t = TempDir::new();
        let (tx, rx) = mpsc::channel();
        let held = primary(&t.0).serve(move |req| {
            req.answer(Answer::Shown);
            let _ = tx.send(req);
        });
        let dir = t.0.clone();
        let waiter = std::thread::spawn(move || matches!(claim_in(&dir, None, WAIT), Ok(Claim::Primary(_))));
        let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        req.answer(Answer::Closing);
        std::thread::sleep(SHOWN_GRACE + Duration::from_millis(500));
        drop(held);
        assert!(waiter.join().unwrap(), "runs the app once the old one is gone");
    }

    /// A launch during the close click: the app answers that it is quitting,
    /// and the launch runs the app once it has gone.
    #[test]
    fn a_launch_while_the_app_quits_runs_it_afterwards() {
        let t = TempDir::new();
        let (held, _rx) = answering(primary(&t.0), Answer::Closing);
        let dir = t.0.clone();
        let waiter = std::thread::spawn(move || matches!(claim_in(&dir, None, WAIT), Ok(Claim::Primary(_))));
        std::thread::sleep(Duration::from_millis(400));
        drop(held);
        assert!(waiter.join().unwrap(), "runs the app once the old one is gone");
    }

    /// Two launches while the app quits (a double click, or Command and the
    /// menu): one runs the app and the other is answered by it, instead of
    /// waiting on the quitting app and then calling it "not responding".
    #[test]
    fn two_launches_while_the_app_quits_get_one_app() {
        let t = TempDir::new();
        let (held, _rx) = answering(primary(&t.0), Answer::Closing);
        let (keep, kept) = mpsc::channel();
        let waiters: Vec<_> = (0..2)
            .map(|_| {
                let dir = t.0.clone();
                let keep = keep.clone();
                std::thread::spawn(move || match claim_in(&dir, None, WAIT) {
                    Ok(Claim::Primary(p)) => {
                        let _ = keep.send(answering(p, Answer::Starting));
                        "runs the app"
                    }
                    Ok(Claim::Shown) => "shown",
                    Err(_) => "gave up",
                })
            })
            .collect();
        std::thread::sleep(Duration::from_millis(400));
        let start = Instant::now();
        drop(held);
        let mut got: Vec<&str> = waiters.into_iter().map(|w| w.join().unwrap()).collect();
        got.sort();
        assert_eq!(got, ["runs the app", "shown"]);
        assert!(start.elapsed() < WAIT, "{:?}", start.elapsed());
        drop(kept);
    }

    /// The app asked went away without answering and another launch took
    /// the lock meanwhile: the waiting launch asks the new holder instead of
    /// reporting "not responding".
    #[test]
    fn a_holder_replaced_while_asked_is_asked_again() {
        let t = TempDir::new();
        let silent = primary(&t.0).serve(|_| {});
        let dir = t.0.clone();
        let start = Instant::now();
        let waiter = std::thread::spawn(move || matches!(claim_in(&dir, None, WAIT), Ok(Claim::Shown)));
        std::thread::sleep(Duration::from_millis(300));
        drop(silent);
        let (_next, rx) = answering(primary(&t.0), Answer::Starting);
        assert!(waiter.join().unwrap(), "answered by the new holder");
        assert!(start.elapsed() < WAIT, "{:?}", start.elapsed());
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), None);
    }

    /// A launch whose runtime directory goes through a symlink reaches the
    /// app and takes its answer (the app reports the path it bound).
    #[test]
    fn a_runtime_dir_through_a_symlink_is_answered() {
        let t = TempDir::new();
        let real = t.0.join("real");
        private_dir(&real).unwrap();
        std::os::unix::fs::symlink(&real, t.0.join("link")).unwrap();
        let (_held, rx) = answering(primary(&real), Answer::Shown);
        assert!(matches!(claim_in(&t.0.join("link"), None, Duration::from_millis(500)), Ok(Claim::Shown)));
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), None);
    }

    /// A launch refused because an older copy has the database open lets go
    /// of the lock before it shows why: once that copy is closed (as the
    /// message says), launching again runs the app, instead of the refused
    /// launch answering "starting" from its dialog and nothing opening.
    #[test]
    fn a_refused_launch_lets_go_before_it_says_why() {
        let t = TempDir::new();
        let old = OtherCopy { pid: 4242, outdated: true };
        let db = t.0.join("axentra.db");
        let refused = decide(claim_in(&t.0, None, WAIT), || vec![old.clone()], || lock_db(&db, Duration::ZERO), WAIT);
        assert!(matches!(&refused, Ok(Start::Refused(o)) if *o == old));
        let start = Instant::now();
        assert!(matches!(claim_in(&t.0, None, WAIT), Ok(Claim::Primary(_))), "the next launch runs the app");
        assert!(start.elapsed() < Duration::from_secs(1), "{:?}", start.elapsed());
        drop(refused);
        // Nothing else open: the launch runs, holding the lock.
        assert!(!lock_db(&db, Duration::ZERO).taken, "a refused launch let go of the database lock too");
        let run = decide(claim_in(&t.0, None, WAIT), Vec::new, || lock_db(&db, Duration::ZERO), WAIT);
        assert!(matches!(&run, Ok(Start::Run(Some(_), _))));
        assert!(matches!(claim_in(&t.0, None, Duration::from_millis(300)), Err(ClaimError::NotAnswering)), "held, not served yet");
        // A launch that was answered never looks for other copies.
        let shown = decide(Ok(Claim::Shown), || panic!("not checked"), || panic!("not taken"), WAIT);
        assert!(matches!(shown, Ok(Start::Shown)));
    }

    /// Two launches at the same instant on one database, with different
    /// runtime directories: each takes its own instance lock, and neither
    /// has the database open when the other looks. The database's lock
    /// tells them apart: the second waits for the first to show up with the
    /// database open and is refused (0.1.4-1 as packaged ran both, or one
    /// panicked on "database is locked").
    #[test]
    fn simultaneous_launches_with_other_runtime_dirs_run_one_copy() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let t = TempDir::new();
        let (dir_a, dir_b) = (t.0.join("run"), t.0.join("run2"));
        private_dir(&dir_a).unwrap();
        private_dir(&dir_b).unwrap();
        let db = t.0.join("data").join("axentra.db");
        let a = decide(claim_in(&dir_a, None, WAIT), Vec::new, || lock_db(&db, Duration::ZERO), WAIT);
        assert!(matches!(&a, Ok(Start::Run(Some(_), l)) if l._file.is_some()), "the first runs");
        assert!(db.with_file_name("axentra.db.instance").exists());
        let a_open = Arc::new(AtomicBool::new(false));
        let seen = a_open.clone();
        let db_b = db.clone();
        let started = Instant::now();
        let b = std::thread::spawn(move || {
            let others = || if seen.load(Ordering::SeqCst) { vec![OtherCopy { pid: 4242, outdated: false }] } else { Vec::new() };
            match decide(claim_in(&dir_b, None, WAIT), others, || lock_db(&db_b, Duration::ZERO), WAIT) {
                Ok(Start::Refused(o)) => Some(o.pid),
                _ => None,
            }
        });
        std::thread::sleep(Duration::from_millis(300));
        a_open.store(true, Ordering::SeqCst);
        assert_eq!(b.join().unwrap(), Some(4242), "the second is refused, naming the first");
        assert!(started.elapsed() >= Duration::from_millis(300));
        // A holder that never shows up: the launch gives up, it never runs.
        let dir_c = t.0.join("run3");
        private_dir(&dir_c).unwrap();
        let c = decide(claim_in(&dir_c, None, WAIT), Vec::new, || lock_db(&db, Duration::ZERO), Duration::from_millis(200));
        assert!(matches!(c, Err(ClaimError::NotAnswering)));
        drop(a);
        assert!(!lock_db(&db, Duration::ZERO).taken, "let go when the app exits");
    }

    #[test]
    fn the_database_lock_waits_for_a_copy_that_is_quitting() {
        let t = TempDir::new();
        let db = t.0.join("axentra.db");
        let held = lock_db(&db, Duration::ZERO);
        assert!(held._file.is_some() && !held.taken);
        assert!(lock_db(&db, Duration::ZERO).taken);
        let db2 = db.clone();
        let waiter = std::thread::spawn(move || lock_db(&db2, Duration::from_secs(3)).taken);
        std::thread::sleep(Duration::from_millis(200));
        drop(held);
        assert!(!waiter.join().unwrap(), "taken once the holder let go");
        // Through a symlinked folder it is the same lock.
        std::os::unix::fs::symlink(&t.0, t.0.join("link")).unwrap();
        let a = lock_db(&db, Duration::ZERO);
        assert!(lock_db(&t.0.join("link").join("axentra.db"), Duration::ZERO).taken);
        drop(a);
    }

    #[test]
    fn a_crashed_holder_frees_the_lock_for_a_waiting_launch() {
        let t = TempDir::new();
        let lock = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(t.0.join(LOCK_FILE)).unwrap();
        lock.try_lock().unwrap();
        let dir = t.0.clone();
        let waiter = std::thread::spawn(move || matches!(claim_in(&dir, None, WAIT), Ok(Claim::Primary(_))));
        std::thread::sleep(Duration::from_millis(200));
        drop(lock);
        assert!(waiter.join().unwrap(), "the waiting launch runs the app");
    }

    /// When the socket can't be bound the app keeps the lock: later
    /// launches give up rather than open the database a second time.
    #[test]
    fn a_socket_that_cannot_be_bound_keeps_the_lock() {
        let t = TempDir::new();
        fs::create_dir(t.0.join(SOCKET)).unwrap();
        fs::write(t.0.join(SOCKET).join("x"), b"").unwrap();
        let first = primary(&t.0);
        assert!(first.socket.is_none());
        let _held = first.serve(|_| {});
        assert!(matches!(claim_in(&t.0, None, Duration::from_millis(300)), Err(ClaimError::NotAnswering)));
    }

    #[test]
    fn other_copies_are_found_by_their_open_database() {
        let t = TempDir::new();
        let db = t.0.join("p.db");
        let open = File::create(&db).unwrap();
        let me = std::process::id();
        // This test process stands in for the other copy (its own pid not
        // skipped); only its own /proc entry is looked into.
        let exe = std::env::current_exe().unwrap().file_name().unwrap().to_string_lossy().into_owned();
        let this = |name: &str| name == exe;
        let found = copies_in(Path::new("/proc"), &db, 0, this);
        assert_eq!(found, vec![OtherCopy { pid: me, outdated: false }]);
        // Through a symlinked path too.
        std::os::unix::fs::symlink(&t.0, t.0.join("link")).unwrap();
        assert_eq!(copies_in(Path::new("/proc"), &t.0.join("link/p.db"), 0, this).len(), 1);
        // Not another program, not ourselves, not another database, not one that doesn't exist.
        assert!(copies_in(Path::new("/proc"), &db, 0, |_| false).is_empty());
        assert!(copies_in(Path::new("/proc"), &db, me, this).is_empty());
        fs::write(t.0.join("q.db"), b"").unwrap();
        assert!(copies_in(Path::new("/proc"), &t.0.join("q.db"), 0, this).is_empty());
        assert!(copies_in(Path::new("/proc"), &t.0.join("none.db"), 0, this).is_empty());
        drop(open);
        assert!(copies_in(Path::new("/proc"), &db, 0, this).is_empty(), "closed");
    }
}
