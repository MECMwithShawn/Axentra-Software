//! Proton Drive as a [`MirrorDestination`], through Proton's official
//! `proton-drive` command-line client, version 0.9.0.
//!
//! What may be run is limited in layers, each enough on its own:
//!
//! - [`CliCommand`] is the only way to say what to run. It has variants for
//!   `version`, `auth login` and `filesystem` `list`, `info`, `size`,
//!   `create-folder`, `upload` and `download`, and nothing else: there is no
//!   way to write a delete, trash, move, rename, restore, copy, sharing,
//!   album, photo or takeout command. Upload conflicts are a
//!   [`FileConflict`], which has no `replace` (trash the remote copy) and no
//!   `rename`; downloads always `skip`, never `remove`.
//! - [`CheckedArgv::new`] checks the finished argument list against the same
//!   allowlist again, and is the only way to make the [`CheckedArgv`] that a
//!   [`CliRunner`] runs. Option values, abbreviations (the CLI accepts any
//!   unique prefix, so `rep` would mean `replace`), paths outside
//!   `/my-files` and unexpected arguments are all refused there.
//! - [`ProcessRunner`] runs the binary itself, never a shell, with an
//!   environment built from an allowlist (see [`scrubbed_env`]), stdin closed,
//!   a time limit, capped output, and the whole process group killed on a
//!   timeout or when the call is dropped.
//! - Before its first command the destination checks the binary: `version`
//!   must report [`PINNED_VERSION`], and (by default) the file's SHA-512 must
//!   be [`PINNED_SHA512`]. Anything else fails closed.
//!
//! The command enum has no destructive variants, so this does not compile:
//!
//! ```compile_fail
//! use axentra_clouddrive_provider::proton_cli::FileConflict;
//! let _ = FileConflict::Replace;
//! ```
//!
//! ```compile_fail
//! use axentra_clouddrive_provider::proton_cli::{CliCommand, RemotePath};
//! let _ = CliCommand::Delete(RemotePath::my_files());
//! ```
//!
//! Output is parsed with the shapes the v0.9.0 source prints (`--json`
//! everywhere): `list` streams a JSON array of SDK nodes, `info` and
//! `create-folder` print one node, `size` prints `{size,
//! numberOfDescendants}`, and `upload` and `download` print one transfer
//! summary line. Output that doesn't parse is an error, never success and
//! never "not found".
//!
//! Remote paths are `/my-files/...`. The CLI treats `\/` as a slash inside a
//! name, treats a segment shaped like a node id as that id, and expands `*`,
//! `?`, `[` and `{` in local paths as globs; names that would trip any of
//! those are refused (see [`remote_name_problem`] and
//! [`upload_name_problem`]) rather than escaped.

use std::collections::VecDeque;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::OpenOptions;
use std::future::Future;
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use parking_lot::Mutex;
use serde::Deserialize;
use sha2::{Digest, Sha512};
use tokio::io::{AsyncRead, AsyncReadExt};

use crate::digests::{hex, ContentDigests};
use crate::error::{PResult, ProviderError};
use crate::path::{check_name, RelPath};
use crate::staging::{parent_dir, verify_and_publish, StagedDir, StagedTemp};
use crate::traits::{Account, Capabilities, DestEntry, ItemKind, MirrorDestination, RemoteStat};

/// The only `proton-drive` version this adapter has been checked against.
/// `version` must print exactly this, optionally followed by `+<build>`.
pub const PINNED_VERSION: &str = "cli-drive@0.9.0";

/// SHA-512 of the Linux x86-64 `proton-drive` 0.9.0 binary
/// (`cli-drive@0.9.0+75612299`).
pub const PINNED_SHA512: &str = "3533025ba69ae112b64e3e01fbcc1ad0688136a4043f6cf6a72886967d85fdcd9ec235479c2e113171614be5225bfba93427509a05ca5aa6071d924fa7e91ca8";

/// Where OneDrive is mirrored to unless configured otherwise.
pub const DEFAULT_ROOT: &str = "/my-files/OneDrive";

/// The section of the signed-in user's own files. Nothing outside it is
/// ever named.
const MY_FILES: &str = "my-files";

/// The fixed words a command line may contain besides the paths and names
/// it was given.
pub const ARGV_LITERALS: &[&str] = &[
    "version",
    "auth",
    "login",
    "filesystem",
    "list",
    "info",
    "size",
    "create-folder",
    "upload",
    "download",
    "--json",
    "--file-conflict-strategy",
    "skip",
    "create-new-revision",
    "--skip-thumbnails",
    "--",
];

// ----- names and paths ------------------------------------------------------------

/// Why `name` can't be a file or folder name in a `proton-drive` path, or
/// `None` if it can.
pub fn remote_name_problem(name: &str) -> Option<&'static str> {
    if check_name(name).is_err() {
        return Some("it is not a single plain name");
    }
    if name.contains('\\') {
        return Some("it contains a backslash, which proton-drive reads as an escape");
    }
    if name.chars().any(char::is_control) {
        return Some("it contains control characters");
    }
    if looks_like_node_uid(name) {
        return Some("it looks like a Proton node id, which proton-drive would look up instead");
    }
    None
}

/// Why a file called `name` can't be uploaded through `proton-drive`, or
/// `None` if it can. The CLI names an upload after the local file, and
/// expands a local path containing `*`, `?`, `[` or `{` as a pattern.
pub fn upload_name_problem(name: &str) -> Option<&'static str> {
    if let Some(why) = remote_name_problem(name) {
        return Some(why);
    }
    if name.contains(GLOB_CHARS) {
        return Some("it contains *, ?, [ or {, which proton-drive expands as a pattern");
    }
    None
}

const GLOB_CHARS: [char; 4] = ['*', '?', '[', '{'];

/// The CLI's `isNodeUid`: `^(X{88,108}|Y{22})~(X{88,108}|Y{22})$` with X
/// `[A-Za-z0-9=_-]` and Y `[A-Za-z0-9_-]`.
fn looks_like_node_uid(s: &str) -> bool {
    fn part(p: &str) -> bool {
        let long = (88..=108).contains(&p.len())
            && p.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"=_-".contains(&b));
        let short = p.len() == 22
            && p.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b));
        long || short
    }
    match s.split_once('~') {
        Some((a, b)) => part(a) && part(b),
        None => false,
    }
}

/// Why `path` can't be passed to `proton-drive` as a local path.
fn local_arg_problem(path: &str) -> Option<&'static str> {
    if !path.starts_with('/') {
        return Some("local paths must be absolute");
    }
    if path.contains(GLOB_CHARS) {
        return Some("the local path contains *, ?, [ or {, which proton-drive expands");
    }
    if path.chars().any(char::is_control) {
        return Some("the local path contains control characters");
    }
    None
}

fn bad_name(name: &str, why: &str) -> ProviderError {
    ProviderError::Other(format!("{name:?} can't be used on Proton Drive: {why}"))
}

/// A path in the signed-in user's own files: `/my-files` and names below
/// it, each passing [`remote_name_problem`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct RemotePath(Vec<String>);

impl RemotePath {
    /// `/my-files` itself.
    pub fn my_files() -> Self {
        Self::default()
    }

    /// `/my-files` or `/my-files/<name>/...`. Repeated or trailing slashes
    /// and anything outside `/my-files` are refused.
    pub fn parse(s: &str) -> PResult<Self> {
        let rest = s.strip_prefix('/').unwrap_or("");
        let mut segs = rest.split('/');
        if !s.starts_with('/') || segs.next() != Some(MY_FILES) {
            return Err(ProviderError::Other(format!(
                "{s:?} is not under /{MY_FILES}"
            )));
        }
        let mut p = Self::my_files();
        for seg in segs {
            p = p.join(seg)?;
        }
        Ok(p)
    }

    pub fn join(&self, name: &str) -> PResult<Self> {
        if let Some(why) = remote_name_problem(name) {
            return Err(bad_name(name, why));
        }
        let mut segs = self.0.clone();
        segs.push(name.to_string());
        Ok(Self(segs))
    }

    pub fn join_rel(&self, rel: &RelPath) -> PResult<Self> {
        let mut p = self.clone();
        for seg in rel.segments() {
            p = p.join(seg)?;
        }
        Ok(p)
    }

    /// The names below `/my-files`.
    pub fn segments(&self) -> &[String] {
        &self.0
    }

    pub fn name(&self) -> Option<&str> {
        self.0.last().map(String::as_str)
    }

    /// `None` for `/my-files`.
    pub fn parent(&self) -> Option<Self> {
        let (_, rest) = self.0.split_last()?;
        Some(Self(rest.to_vec()))
    }
}

impl fmt::Display for RemotePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "/{MY_FILES}")?;
        for seg in &self.0 {
            // No escaping needed: names with `/` or `\` are refused.
            write!(f, "/{seg}")?;
        }
        Ok(())
    }
}

// ----- commands -------------------------------------------------------------------

/// What to do when an upload's name is taken. There is deliberately no
/// `replace` (which trashes the remote copy) and no `rename`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileConflict {
    /// Leave the existing file alone and report the upload as skipped.
    Skip,
    /// Add the upload as a new revision of the existing file, keeping the
    /// earlier ones.
    CreateNewRevision,
}

impl FileConflict {
    fn as_arg(self) -> &'static str {
        match self {
            FileConflict::Skip => "skip",
            FileConflict::CreateNewRevision => "create-new-revision",
        }
    }
}

/// Every command this adapter can run. See the module docs for what is
/// left out, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliCommand {
    Version,
    /// Interactive sign-in in a browser; for a terminal, not the mirror.
    AuthLogin,
    List(RemotePath),
    Info(RemotePath),
    Size(RemotePath),
    CreateFolder {
        parent: RemotePath,
        name: String,
    },
    /// Upload the local file `local` into `parent`. The remote file is
    /// named after `local`'s file name.
    Upload {
        local: PathBuf,
        parent: RemotePath,
        on_conflict: FileConflict,
        thumbnails: bool,
    },
    /// Download the remote file `file` into the local folder `into`.
    Download {
        file: RemotePath,
        into: PathBuf,
    },
}

impl CliCommand {
    /// The arguments after the binary's name. Options always come before a
    /// `--`, so a name starting with `-` is never read as an option.
    pub fn argv(&self) -> Vec<OsString> {
        fn s(v: &[&str]) -> Vec<OsString> {
            v.iter().map(OsString::from).collect()
        }
        let path = |p: &RemotePath| OsString::from(p.to_string());
        match self {
            CliCommand::Version => s(&["version"]),
            CliCommand::AuthLogin => s(&["auth", "login"]),
            CliCommand::List(p) => {
                let mut v = s(&["filesystem", "list", "--json", "--"]);
                v.push(path(p));
                v
            }
            CliCommand::Info(p) => {
                let mut v = s(&["filesystem", "info", "--json", "--"]);
                v.push(path(p));
                v
            }
            CliCommand::Size(p) => {
                let mut v = s(&["filesystem", "size", "--json", "--"]);
                v.push(path(p));
                v
            }
            CliCommand::CreateFolder { parent, name } => {
                let mut v = s(&["filesystem", "create-folder", "--json", "--"]);
                v.push(path(parent));
                v.push(name.into());
                v
            }
            CliCommand::Upload {
                local,
                parent,
                on_conflict,
                thumbnails,
            } => {
                let mut v = s(&[
                    "filesystem",
                    "upload",
                    "--json",
                    "--file-conflict-strategy",
                    on_conflict.as_arg(),
                ]);
                if !thumbnails {
                    v.push("--skip-thumbnails".into());
                }
                v.push("--".into());
                v.push(local.into());
                v.push(path(parent));
                v
            }
            CliCommand::Download { file, into } => {
                let mut v = s(&[
                    "filesystem",
                    "download",
                    "--json",
                    "--file-conflict-strategy",
                    "skip",
                    "--",
                ]);
                v.push(path(file));
                v.push(into.into());
                v
            }
        }
    }

    /// Adds something to Proton Drive. Nothing this can express removes
    /// anything.
    pub fn is_mutating(&self) -> bool {
        matches!(
            self,
            CliCommand::CreateFolder { .. } | CliCommand::Upload { .. }
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Version,
    Login,
    List,
    Info,
    Size,
    CreateFolder,
    Upload,
    Download,
}

/// An argument list that passed the allowlist. The only thing a
/// [`CliRunner`] runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedArgv {
    args: Vec<OsString>,
    kind: Kind,
}

/// [`CheckedArgv::new`] refused an argument list.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("refusing to run proton-drive: {0}")]
pub struct ArgvRejected(pub String);

impl CheckedArgv {
    /// The one place the allowlist is enforced on what actually runs.
    pub fn new(args: Vec<OsString>) -> Result<Self, ArgvRejected> {
        let kind = check_argv(&args).map_err(ArgvRejected)?;
        Ok(Self { args, kind })
    }

    pub fn args(&self) -> &[OsString] {
        &self.args
    }

    /// `auth login`, which alone gets a display to open a browser on.
    pub fn is_login(&self) -> bool {
        self.kind == Kind::Login
    }

    pub fn is_mutating(&self) -> bool {
        matches!(self.kind, Kind::CreateFolder | Kind::Upload)
    }
}

fn check_argv(args: &[OsString]) -> Result<Kind, String> {
    let mut s = Vec::with_capacity(args.len());
    for a in args {
        s.push(a.to_str().ok_or("an argument is not valid UTF-8")?);
    }
    let (sub, rest) = match s.as_slice() {
        ["version"] => return Ok(Kind::Version),
        ["auth", "login"] => return Ok(Kind::Login),
        ["filesystem", sub, rest @ ..] => (*sub, rest),
        [] => return Err("no command".into()),
        [first, ..] => return Err(format!("the command {first:?} is not allowed")),
    };
    let kind = match sub {
        "list" => Kind::List,
        "info" => Kind::Info,
        "size" => Kind::Size,
        "create-folder" => Kind::CreateFolder,
        "upload" => Kind::Upload,
        "download" => Kind::Download,
        other => return Err(format!("filesystem {other:?} is not allowed")),
    };
    let split = rest
        .iter()
        .position(|a| *a == "--")
        .ok_or("the paths must follow a --")?;
    check_options(kind, &rest[..split])?;
    check_positionals(kind, &rest[split + 1..])?;
    Ok(kind)
}

fn check_options(kind: Kind, opts: &[&str]) -> Result<(), String> {
    let transfer = matches!(kind, Kind::Upload | Kind::Download);
    let (mut json, mut strategy, mut thumbs) = (false, false, false);
    let mut it = opts.iter();
    while let Some(opt) = it.next() {
        match *opt {
            "--json" if !json => json = true,
            "--file-conflict-strategy" if transfer && !strategy => {
                let v = it.next().ok_or("--file-conflict-strategy needs a value")?;
                let allowed: &[&str] = match kind {
                    Kind::Upload => &["skip", "create-new-revision"],
                    _ => &["skip"],
                };
                if !allowed.contains(v) {
                    return Err(format!("the conflict strategy {v:?} is not allowed"));
                }
                strategy = true;
            }
            "--skip-thumbnails" if kind == Kind::Upload && !thumbs => thumbs = true,
            other => return Err(format!("the option {other:?} is not allowed here")),
        }
    }
    if !json {
        return Err("--json is required".into());
    }
    if transfer && !strategy {
        return Err("a conflict strategy is required".into());
    }
    Ok(())
}

fn check_positionals(kind: Kind, pos: &[&str]) -> Result<(), String> {
    let remote = |p: &str| RemotePath::parse(p).map(drop).map_err(|e| e.to_string());
    let local = |p: &str| match local_arg_problem(p) {
        Some(why) => Err(why.to_string()),
        None => Ok(()),
    };
    match (kind, pos) {
        (Kind::List | Kind::Info | Kind::Size, [p]) => remote(p),
        (Kind::CreateFolder, [parent, name]) => {
            remote(parent)?;
            match remote_name_problem(name) {
                Some(why) => Err(format!("folder name: {why}")),
                None => Ok(()),
            }
        }
        (Kind::Upload, [file, parent]) => {
            local(file)?;
            let name = Path::new(file)
                .file_name()
                .and_then(OsStr::to_str)
                .ok_or("the upload has no file name")?;
            if let Some(why) = upload_name_problem(name) {
                return Err(format!("upload name: {why}"));
            }
            remote(parent)
        }
        (Kind::Download, [file, into]) => {
            remote(file)?;
            local(into)
        }
        _ => Err(format!("wrong number of paths ({})", pos.len())),
    }
}

// ----- running ----------------------------------------------------------------------

/// What a finished run printed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CliOutput {
    /// `None` if it was ended by a signal.
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// More was printed than the limits allowed; the rest was dropped.
    pub truncated: bool,
}

impl CliOutput {
    pub fn new(code: i32, stdout: impl Into<Vec<u8>>, stderr: impl Into<Vec<u8>>) -> Self {
        Self {
            code: Some(code),
            stdout: stdout.into(),
            stderr: stderr.into(),
            truncated: false,
        }
    }
}

/// How long a run may take and how much of its output is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunLimits {
    pub timeout: Duration,
    pub max_stdout: usize,
    pub max_stderr: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error("could not start proton-drive: {0}")]
    Spawn(std::io::Error),
    #[error("proton-drive took longer than {0:?} and was stopped")]
    TimedOut(Duration),
    #[error("talking to proton-drive: {0}")]
    Io(std::io::Error),
}

/// Runs `proton-drive` with an allowlisted argument list.
pub trait CliRunner: Send + Sync {
    fn run(
        &self,
        argv: &CheckedArgv,
        limits: RunLimits,
    ) -> impl Future<Output = Result<CliOutput, RunError>> + Send;
}

/// The environment `proton-drive` gets: only `HOME`, `USER`, `PATH`,
/// `LANG`, `LC_*`, `XDG_*` and `DBUS_SESSION_BUS_ADDRESS` from `parent`
/// (plus `DISPLAY` and `WAYLAND_DISPLAY` for `auth login`, to open a
/// browser), `PROTON_DRIVE_LOG_LEVEL=WARNING` always (its default, DEBUG,
/// logs too much), and `PROTON_DRIVE_CACHE_DIR` if `cache_dir` is given.
/// Everything else, `PROTON_DRIVE_BASE_URL` and `LD_PRELOAD` included, is
/// dropped.
///
/// Refuses outright if `parent` sets `PROTON_DRIVE_CREDENTIALS_STORE` to
/// `unsafe_file`: a Proton session kept in a plain file is not something
/// to carry on with quietly.
pub fn scrubbed_env(
    parent: impl IntoIterator<Item = (OsString, OsString)>,
    for_login: bool,
    cache_dir: Option<&Path>,
) -> PResult<Vec<(OsString, OsString)>> {
    let mut env = Vec::new();
    for (k, v) in parent {
        let Some(key) = k.to_str() else { continue };
        let unsafe_store = match v.to_str() {
            Some(v) => v.trim().eq_ignore_ascii_case("unsafe_file"),
            None => true,
        };
        if key == "PROTON_DRIVE_CREDENTIALS_STORE" && unsafe_store {
            return Err(ProviderError::Other(
                "PROTON_DRIVE_CREDENTIALS_STORE=unsafe_file keeps the Proton session in a plain \
                 file; refusing to run proton-drive with it set"
                    .into(),
            ));
        }
        let keep = matches!(
            key,
            "HOME" | "USER" | "PATH" | "LANG" | "DBUS_SESSION_BUS_ADDRESS"
        ) || key.starts_with("LC_")
            || key.starts_with("XDG_")
            || (for_login && matches!(key, "DISPLAY" | "WAYLAND_DISPLAY"));
        if keep {
            env.push((k, v));
        }
    }
    env.push(("PROTON_DRIVE_LOG_LEVEL".into(), "WARNING".into()));
    if let Some(dir) = cache_dir {
        env.push(("PROTON_DRIVE_CACHE_DIR".into(), dir.into()));
    }
    env.sort();
    Ok(env)
}

/// Runs the real binary: no shell, the scrubbed environment, stdin from
/// `/dev/null`, its own process group, and `/` as the working folder.
#[derive(Debug, Clone)]
pub struct ProcessRunner {
    binary: PathBuf,
    env: Vec<(OsString, OsString)>,
    login_env: Vec<(OsString, OsString)>,
}

impl ProcessRunner {
    /// `binary` must be an absolute path: `PATH` is never searched.
    pub fn new(binary: impl Into<PathBuf>, cache_dir: Option<&Path>) -> PResult<Self> {
        Self::with_env(binary, std::env::vars_os().collect(), cache_dir)
    }

    /// As [`ProcessRunner::new`], with `parent` in place of this process's
    /// environment.
    pub fn with_env(
        binary: impl Into<PathBuf>,
        parent: Vec<(OsString, OsString)>,
        cache_dir: Option<&Path>,
    ) -> PResult<Self> {
        let binary = binary.into();
        if !binary.is_absolute() {
            return Err(ProviderError::Other(format!(
                "the proton-drive path {} is not absolute",
                binary.display()
            )));
        }
        Ok(Self {
            env: scrubbed_env(parent.clone(), false, cache_dir)?,
            login_env: scrubbed_env(parent, true, cache_dir)?,
            binary,
        })
    }

    pub fn binary(&self) -> &Path {
        &self.binary
    }
}

/// Kills a process group when dropped, unless disarmed once its leader has
/// been reaped normally. The leader is not reaped while armed, so its id
/// (the group's) can't have been reused.
struct GroupKiller(Option<i32>);

impl GroupKiller {
    fn kill(&mut self) {
        if let Some(pgid) = self.0.take() {
            // SAFETY: a plain syscall on a group this runner created.
            unsafe {
                libc::killpg(pgid, libc::SIGKILL);
            }
        }
    }

    fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for GroupKiller {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Everything `r` yields, keeping at most `cap` bytes. Reading carries on
/// past the cap so the writer never blocks on a full pipe.
async fn read_capped(
    mut r: impl AsyncRead + Unpin,
    cap: usize,
) -> std::io::Result<(Vec<u8>, bool)> {
    let mut buf = Vec::new();
    let mut chunk = vec![0u8; 64 * 1024];
    let mut truncated = false;
    loop {
        let n = r.read(&mut chunk).await?;
        if n == 0 {
            return Ok((buf, truncated));
        }
        let room = cap.saturating_sub(buf.len());
        if n > room {
            truncated = true;
        }
        buf.extend_from_slice(&chunk[..n.min(room)]);
    }
}

impl CliRunner for ProcessRunner {
    async fn run(&self, argv: &CheckedArgv, limits: RunLimits) -> Result<CliOutput, RunError> {
        let env = if argv.is_login() {
            &self.login_env
        } else {
            &self.env
        };
        let mut cmd = tokio::process::Command::new(&self.binary);
        cmd.args(argv.args())
            .env_clear()
            .envs(env.iter().map(|(k, v)| (k, v)))
            .current_dir("/")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .kill_on_drop(true);
        let mut child = cmd.spawn().map_err(RunError::Spawn)?;
        let mut group = GroupKiller(child.id().and_then(|id| i32::try_from(id).ok()));
        let stdout = child.stdout.take().expect("stdout is piped");
        let stderr = child.stderr.take().expect("stderr is piped");
        let finished = tokio::time::timeout(limits.timeout, async {
            tokio::join!(
                read_capped(stdout, limits.max_stdout),
                read_capped(stderr, limits.max_stderr),
                child.wait(),
            )
        })
        .await;
        let Ok((out, err, status)) = finished else {
            group.kill();
            let _ = child.kill().await;
            return Err(RunError::TimedOut(limits.timeout));
        };
        // A failed wait leaves the child unreaped: `group` kills it on return.
        let status = status.map_err(RunError::Io)?;
        group.disarm();
        let (stdout, out_cut) = out.map_err(RunError::Io)?;
        let (stderr, err_cut) = err.map_err(RunError::Io)?;
        Ok(CliOutput {
            code: status.code(),
            stdout,
            stderr,
            truncated: out_cut || err_cut,
        })
    }
}

/// One answer queued on a [`FakeRunner`].
#[derive(Debug, Clone)]
pub enum FakeReply {
    Output(CliOutput),
    /// For a download: write each `(name, data)` in the folder it was asked
    /// to download into, then answer `output`.
    Download {
        files: Vec<(String, Vec<u8>)>,
        output: CliOutput,
    },
    TimedOut,
}

/// A file an upload was given, as it was when the command ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeUpload {
    /// The local file's name, which the CLI names the upload after.
    pub file_name: String,
    /// What else was in the local file's folder.
    pub siblings: Vec<String>,
    pub data: Vec<u8>,
}

#[derive(Debug, Default)]
struct FakeState {
    calls: Vec<Vec<String>>,
    limits: Vec<RunLimits>,
    mutating: usize,
    uploads: Vec<FakeUpload>,
    replies: VecDeque<FakeReply>,
}

/// A [`CliRunner`] for tests. Logs every argument list (which has passed
/// the allowlist like any other) and answers from a queue; with nothing
/// queued, a call fails.
#[derive(Debug, Default)]
pub struct FakeRunner {
    state: Mutex<FakeState>,
}

impl FakeRunner {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&self, reply: FakeReply) {
        self.state.lock().replies.push_back(reply);
    }

    /// Queue an exit code and what was printed.
    pub fn reply(&self, code: i32, stdout: impl Into<Vec<u8>>, stderr: impl Into<Vec<u8>>) {
        self.push(FakeReply::Output(CliOutput::new(code, stdout, stderr)));
    }

    /// Every argument list run, in order.
    pub fn calls(&self) -> Vec<Vec<String>> {
        self.state.lock().calls.clone()
    }

    /// The time limit each call was given.
    pub fn timeouts(&self) -> Vec<Duration> {
        self.state.lock().limits.iter().map(|l| l.timeout).collect()
    }

    /// Calls that would add to Proton Drive.
    pub fn mutating_calls(&self) -> usize {
        self.state.lock().mutating
    }

    pub fn uploads(&self) -> Vec<FakeUpload> {
        self.state.lock().uploads.clone()
    }

    /// Replies queued and not used yet.
    pub fn pending(&self) -> usize {
        self.state.lock().replies.len()
    }
}

fn file_names(dir: &Path) -> std::io::Result<Vec<String>> {
    let mut v = Vec::new();
    for e in std::fs::read_dir(dir)? {
        v.push(e?.file_name().to_string_lossy().into_owned());
    }
    v.sort();
    Ok(v)
}

impl CliRunner for FakeRunner {
    async fn run(&self, argv: &CheckedArgv, limits: RunLimits) -> Result<CliOutput, RunError> {
        let args: Vec<String> = argv
            .args()
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let mut s = self.state.lock();
        s.calls.push(args.clone());
        s.limits.push(limits);
        if argv.is_mutating() {
            s.mutating += 1;
        }
        let last = || PathBuf::from(args.last().expect("checked argv has paths"));
        if argv.kind == Kind::Upload {
            let local = PathBuf::from(&args[args.len() - 2]);
            s.uploads.push(FakeUpload {
                file_name: local
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                siblings: file_names(parent_dir(&local)).map_err(RunError::Io)?,
                data: std::fs::read(&local).map_err(RunError::Io)?,
            });
        }
        match s.replies.pop_front() {
            Some(FakeReply::Output(out)) => Ok(out),
            Some(FakeReply::Download { files, output }) => {
                for (name, data) in files {
                    std::fs::write(last().join(name), data).map_err(RunError::Io)?;
                }
                Ok(output)
            }
            Some(FakeReply::TimedOut) => Err(RunError::TimedOut(limits.timeout)),
            None => Err(RunError::Io(std::io::Error::other(
                "FakeRunner: no reply queued",
            ))),
        }
    }
}

// ----- parsing ----------------------------------------------------------------------

/// A file or folder as `info`, `list` and `create-folder` print it, with
/// only what the mirror uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtonNode {
    /// `<volume id>~<node id>`.
    pub uid: String,
    /// `None` if Proton couldn't decrypt it.
    pub name: Option<String>,
    pub kind: ItemKind,
    pub trashed: bool,
    /// The active revision's claimed (unencrypted) size.
    pub size: Option<u64>,
    /// The active revision's claimed SHA-1, lower-case hex.
    pub sha1: Option<String>,
    /// Whether the signature over the claimed SHA-1 verified.
    pub sha1_verified: bool,
    /// The active revision's uid.
    pub revision: Option<String>,
    pub owner_email: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NodeJson {
    uid: String,
    name: NameJson,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    trash_time: Option<serde_json::Value>,
    #[serde(default)]
    active_revision: Option<RevisionJson>,
    #[serde(default)]
    owned_by: Option<OwnedByJson>,
}

/// The SDK's `Result<string, Error>`.
#[derive(Debug, Deserialize)]
struct NameJson {
    ok: bool,
    #[serde(default)]
    value: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RevisionJson {
    uid: String,
    #[serde(default)]
    claimed_size: Option<u64>,
    #[serde(default)]
    claimed_digests: Option<DigestsJson>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DigestsJson {
    #[serde(default)]
    sha1: Option<String>,
    #[serde(default)]
    sha1_verified: bool,
}

#[derive(Debug, Deserialize)]
struct OwnedByJson {
    #[serde(default)]
    email: Option<String>,
}

fn unparseable(what: &str, e: serde_json::Error) -> ProviderError {
    // Only where it failed: the message could otherwise quote a file name.
    ProviderError::Other(format!(
        "proton-drive printed {what} this version of the mirror doesn't understand \
         ({:?} error at line {}, column {})",
        e.classify(),
        e.line(),
        e.column()
    ))
}

fn node_from(j: NodeJson) -> PResult<ProtonNode> {
    let kind = match j.kind.as_str() {
        "file" => ItemKind::File,
        "folder" => ItemKind::Folder,
        other => {
            return Err(ProviderError::Other(format!(
                "proton-drive reported a node of type {other:?}, which the mirror doesn't handle"
            )))
        }
    };
    let name = match (j.name.ok, j.name.value) {
        (true, Some(v)) => Some(v),
        (true, None) => {
            return Err(ProviderError::Other(
                "proton-drive reported a name without a value".into(),
            ))
        }
        (false, _) => None,
    };
    let rev = j.active_revision;
    let digests = rev.as_ref().and_then(|r| r.claimed_digests.as_ref());
    let sha1 = match digests.and_then(|d| d.sha1.as_deref()) {
        Some(h) if h.len() == 40 && h.bytes().all(|b| b.is_ascii_hexdigit()) => {
            Some(h.to_ascii_lowercase())
        }
        Some(_) => {
            return Err(ProviderError::Other(
                "proton-drive reported a SHA-1 that isn't 40 hex digits".into(),
            ))
        }
        None => None,
    };
    Ok(ProtonNode {
        uid: j.uid,
        name,
        kind,
        trashed: j.trash_time.is_some_and(|t| !t.is_null()),
        size: rev.as_ref().and_then(|r| r.claimed_size),
        sha1,
        sha1_verified: digests.is_some_and(|d| d.sha1_verified),
        revision: rev.map(|r| r.uid),
        owner_email: j.owned_by.and_then(|o| o.email),
    })
}

/// One node, as `info --json` and `create-folder --json` print it.
pub fn parse_node(stdout: &[u8]) -> PResult<ProtonNode> {
    let j: NodeJson =
        serde_json::from_slice(stdout.trim_ascii()).map_err(|e| unparseable("a node", e))?;
    node_from(j)
}

/// The array `list --json` prints.
pub fn parse_list(stdout: &[u8]) -> PResult<Vec<ProtonNode>> {
    let j: Vec<NodeJson> =
        serde_json::from_slice(stdout.trim_ascii()).map_err(|e| unparseable("a listing", e))?;
    j.into_iter().map(node_from).collect()
}

/// The summary line `upload --json` and `download --json` print.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TransferSummary {
    pub transferred_items: u64,
    pub transferred_bytes: u64,
    pub skipped_items: u64,
    pub failed_items: u64,
    pub failures: Vec<TransferFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TransferFailure {
    pub name: String,
    #[serde(default)]
    pub node_uid: Option<String>,
    /// `<error class>: <message>`, such as `IntegrityError: ...`.
    pub error: String,
}

pub fn parse_transfer_summary(stdout: &[u8]) -> PResult<TransferSummary> {
    let s: TransferSummary = serde_json::from_slice(stdout.trim_ascii())
        .map_err(|e| unparseable("a transfer summary", e))?;
    if s.failures.len() as u64 != s.failed_items {
        return Err(ProviderError::Other(
            "proton-drive's transfer summary doesn't add up".into(),
        ));
    }
    Ok(s)
}

/// What `size --json` prints: the folder's descendants, trashed ones
/// included.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FolderSize {
    pub size: u64,
    pub number_of_descendants: u64,
}

pub fn parse_folder_size(stdout: &[u8]) -> PResult<FolderSize> {
    serde_json::from_slice(stdout.trim_ascii()).map_err(|e| unparseable("a folder size", e))
}

/// The CLI's version from what `version` prints (its first line is
/// `Proton Drive CLI <version>`).
pub fn parse_version(stdout: &[u8]) -> PResult<String> {
    String::from_utf8_lossy(stdout)
        .lines()
        .find_map(|l| l.strip_prefix("Proton Drive CLI "))
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| ProviderError::Other("proton-drive didn't say which version it is".into()))
}

/// Fails unless `version` is [`PINNED_VERSION`], optionally with a build
/// suffix.
pub fn check_version(version: &str) -> PResult<()> {
    let rest = version.strip_prefix(PINNED_VERSION);
    if matches!(rest, Some(r) if r.is_empty() || r.starts_with('+')) {
        Ok(())
    } else {
        Err(ProviderError::Other(format!(
            "proton-drive reports version {version:?}, but only {PINNED_VERSION} has been checked \
             to work with this mirror; refusing to use it"
        )))
    }
}

// ----- errors -----------------------------------------------------------------------

/// The first line of `msg`, without control characters, at most 300
/// characters.
fn snippet(msg: &str) -> String {
    let line = msg.trim().lines().next().unwrap_or("");
    let clean: String = line.chars().filter(|c| !c.is_control()).collect();
    match clean.char_indices().nth(300) {
        Some((i, _)) => format!("{}...", &clean[..i]),
        None => clean,
    }
}

/// What a message from `proton-drive` means: a failed command's stderr,
/// or the `error` of a failed transfer (`<class>: <message>`). Anything
/// not recognised is [`ProviderError::Other`], which is not retried.
///
/// The phrases come from the CLI and SDK source. "Session expired" style
/// phrases are a best guess at the account library's messages, which are
/// not in the published source.
pub fn classify_message(msg: &str) -> ProviderError {
    let m = msg.trim();
    let lower = m.to_ascii_lowercase();
    let short = snippet(m);
    let has = |needles: &[&str]| needles.iter().any(|n| lower.contains(n));
    if m.contains("You need to login first")
        || has(&[
            "session expired",
            "session has expired",
            "invalid refresh token",
            "please log in again",
            "please login again",
        ])
    {
        return ProviderError::AuthRequired;
    }
    if m.starts_with("Failed to load session from secrets") {
        return ProviderError::Other(format!(
            "proton-drive couldn't read its saved session from the keyring (is the desktop \
             session's D-Bus reachable?): {short}"
        ));
    }
    if m.starts_with("RateLimitedError") || has(&["too many server requests", "too many requests"])
    {
        return ProviderError::RateLimited { retry_after: None };
    }
    if m.starts_with("IntegrityError")
        || m.starts_with("DecryptionError")
        || has(&["integrity verification failed"])
    {
        return ProviderError::Integrity(short);
    }
    if m.starts_with("NodeWithSameNameExistsValidationError") || has(&["already exists"]) {
        return ProviderError::Conflict(short);
    }
    if m.starts_with("ConnectionError")
        || has(&[
            "unable to connect",
            "network error",
            "fetch failed",
            "econnrefused",
            "econnreset",
            "enotfound",
            "eai_again",
            "offline",
        ])
    {
        return ProviderError::Offline(short);
    }
    if m.starts_with("ServerError")
        || m.starts_with("TimeoutError")
        || has(&["too many server errors", "timed out", "try again later"])
    {
        return ProviderError::Transient(short);
    }
    ProviderError::Other(format!("proton-drive failed: {short}"))
}

/// What a run that didn't succeed means. "Not found" only when the CLI
/// said, and said only, that a name on the path doesn't exist.
fn failure(out: &CliOutput) -> ProviderError {
    let stderr = String::from_utf8_lossy(&out.stderr);
    // The CLI writes a notice like these once per process when Proton
    // suggests a newer SDK (api/driveRequirementHook.ts). It says nothing
    // about why this command failed, so it is set aside before matching.
    let (notices, rest): (Vec<&str>, Vec<&str>) = stderr
        .trim()
        .lines()
        .partition(|l| UPDATE_NOTICES.iter().any(|p| l.trim_start().starts_with(p)));
    let rest = rest.join("\n");
    let msg = rest.trim();
    match out.code {
        None => {
            return ProviderError::Transient("proton-drive was ended by a signal".into());
        }
        Some(1)
            if out.stdout.trim_ascii().is_empty()
                && !msg.contains('\n')
                && msg.starts_with("Node not found: ") =>
        {
            return ProviderError::NotFound(snippet(msg));
        }
        _ => {}
    }
    match (classify_message(msg), notices.first()) {
        (ProviderError::Other(why), Some(notice)) => ProviderError::Other(format!(
            "{why} (proton-drive also said: {})",
            snippet(notice)
        )),
        (e, _) => e,
    }
}

/// How the CLI's update notices start.
const UPDATE_NOTICES: [&str; 3] = ["Update recommended:", "Update required:", "Update needed:"];

fn run_error(e: RunError, binary: &Path) -> ProviderError {
    match e {
        RunError::TimedOut(_) => ProviderError::Transient(e.to_string()),
        RunError::Spawn(io) if io.kind() == std::io::ErrorKind::NotFound => ProviderError::Other(
            format!("proton-drive was not found at {}", binary.display()),
        ),
        other => ProviderError::Other(other.to_string()),
    }
}

fn success(out: &CliOutput) -> PResult<&[u8]> {
    if out.code == Some(0) {
        Ok(&out.stdout)
    } else {
        Err(failure(out))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Transfer {
    Done,
    Skipped,
}

/// Exactly one item transferred or skipped, and nothing failed. Anything
/// else is an error.
fn transfer_outcome(out: &CliOutput) -> PResult<Transfer> {
    if out.stdout.trim_ascii().is_empty() {
        // It failed before it started, such as on a missing folder.
        return Err(failure(out));
    }
    let s = parse_transfer_summary(&out.stdout)?;
    match (out.code, s.failures.first()) {
        (Some(0), None) => match (s.transferred_items, s.skipped_items) {
            (1, 0) => Ok(Transfer::Done),
            (0, 1) => Ok(Transfer::Skipped),
            (t, k) => Err(ProviderError::Other(format!(
                "proton-drive transferred {t} and skipped {k} items where one was expected"
            ))),
        },
        (Some(_), Some(f)) if out.code != Some(0) => Err(classify_message(&f.error)),
        _ => Err(ProviderError::Other(
            "proton-drive's exit status and summary disagree".into(),
        )),
    }
}

// ----- the destination ----------------------------------------------------------

/// Time limits per command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timeouts {
    /// `version`, which also asks proton.me for the latest version (with
    /// its own 5 second limit).
    pub version: Duration,
    /// `list`, `info`, `size` and `create-folder`. The SDK waits out rate
    /// limits itself, so this is generous.
    pub metadata: Duration,
    /// An upload or download of an empty file.
    pub transfer_base: Duration,
    /// The slowest transfer rate, in bytes a second, before a transfer is
    /// given up on: a file of `n` bytes gets `transfer_base + n / this`.
    pub transfer_min_rate: u64,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            version: Duration::from_secs(60),
            metadata: Duration::from_secs(300),
            transfer_base: Duration::from_secs(300),
            transfer_min_rate: 128 * 1024,
        }
    }
}

impl Timeouts {
    pub fn transfer(&self, size: u64) -> Duration {
        self.transfer_base + Duration::from_secs(size / self.transfer_min_rate.max(1))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtonCliConfig {
    /// The `proton-drive` binary, an absolute path.
    pub binary: PathBuf,
    /// Checked against the binary before first use; `None` skips the check
    /// (the version is always checked).
    pub expected_sha512: Option<String>,
    /// The folder the mirror writes under, `/my-files/...`.
    pub root: String,
    /// `PROTON_DRIVE_CACHE_DIR`, if not the CLI's default.
    pub cache_dir: Option<PathBuf>,
    /// Let the CLI make thumbnails of uploads. Off by default: a thumbnail
    /// that fails to generate fails the upload.
    pub thumbnails: bool,
    /// Allow [`MirrorDestination::upload_revision`]. Off by default: the
    /// mirror records changed files as diverged instead.
    pub revisions: bool,
    pub timeouts: Timeouts,
    /// The most of a command's output kept; more is an error.
    pub max_stdout: usize,
    pub max_stderr: usize,
}

impl ProtonCliConfig {
    /// The defaults, for the binary at `binary`.
    pub fn new(binary: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
            expected_sha512: Some(PINNED_SHA512.into()),
            root: DEFAULT_ROOT.into(),
            cache_dir: None,
            thumbnails: false,
            revisions: false,
            timeouts: Timeouts::default(),
            max_stdout: 64 << 20,
            max_stderr: 1 << 20,
        }
    }

    /// Where Axentra keeps the pinned CLI:
    /// `$XDG_DATA_HOME/axentra/proton-drive-cli/0.9.0/proton-drive`.
    pub fn default_binary() -> Option<PathBuf> {
        let data = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .filter(|p| p.is_absolute())
                    .map(|h| h.join(".local/share"))
            })?;
        Some(data.join("axentra/proton-drive-cli/0.9.0/proton-drive"))
    }
}

/// Proton Drive, reached through `proton-drive`. See the module docs.
///
/// [`Capabilities`]: SHA-1 (claimed), creating folders, round trips, and
/// revisions only if [`ProtonCliConfig::revisions`] is set. The claimed
/// SHA-1 is reported only when its signature verified, so
/// [`VerifyLevel::L3RemoteClaimed`](crate::VerifyLevel::L3RemoteClaimed)
/// is unreachable for a file without one.
pub struct ProtonCliDestination<R: CliRunner = ProcessRunner> {
    runner: R,
    config: ProtonCliConfig,
    root: RemotePath,
    checked: tokio::sync::OnceCell<()>,
}

impl ProtonCliDestination<ProcessRunner> {
    /// The real thing: a [`ProcessRunner`] for `config.binary`.
    pub fn with_process_runner(config: ProtonCliConfig) -> PResult<Self> {
        let runner = ProcessRunner::new(&config.binary, config.cache_dir.as_deref())?;
        Self::new(config, runner)
    }
}

impl<R: CliRunner> ProtonCliDestination<R> {
    pub fn new(config: ProtonCliConfig, runner: R) -> PResult<Self> {
        let root = RemotePath::parse(&config.root)?;
        Ok(Self {
            runner,
            config,
            root,
            checked: tokio::sync::OnceCell::new(),
        })
    }

    pub fn runner(&self) -> &R {
        &self.runner
    }

    pub fn config(&self) -> &ProtonCliConfig {
        &self.config
    }

    /// The mirror's root, such as `/my-files/OneDrive`.
    pub fn root(&self) -> &RemotePath {
        &self.root
    }

    /// Where `rel` (relative to the root) is on Proton Drive.
    pub fn remote_path(&self, rel: &RelPath) -> PResult<RemotePath> {
        self.root.join_rel(rel)
    }

    /// Check the binary's SHA-512 (if configured) and version. Done before
    /// the first command and remembered once it passes.
    pub async fn check_binary(&self) -> PResult<()> {
        self.checked
            .get_or_try_init(|| self.verify_binary())
            .await
            .map(|_| ())
    }

    async fn verify_binary(&self) -> PResult<()> {
        if let Some(want) = &self.config.expected_sha512 {
            let path = self.config.binary.clone();
            let got = tokio::task::spawn_blocking(move || sha512_of(&path))
                .await
                .map_err(|e| ProviderError::Other(format!("hashing proton-drive: {e}")))??;
            if !got.eq_ignore_ascii_case(want.trim()) {
                return Err(ProviderError::Other(format!(
                    "{} is not the proton-drive build this mirror was checked with (its SHA-512 \
                     differs); refusing to use it",
                    self.config.binary.display()
                )));
            }
        }
        let out = self
            .run(CliCommand::Version, self.config.timeouts.version)
            .await?;
        check_version(&parse_version(success(&out)?)?)
    }

    /// Run `cmd` without the binary check.
    async fn run(&self, cmd: CliCommand, timeout: Duration) -> PResult<CliOutput> {
        let argv = CheckedArgv::new(cmd.argv()).map_err(|e| ProviderError::Other(e.to_string()))?;
        let limits = RunLimits {
            timeout,
            max_stdout: self.config.max_stdout,
            max_stderr: self.config.max_stderr,
        };
        let out = self
            .runner
            .run(&argv, limits)
            .await
            .map_err(|e| run_error(e, &self.config.binary))?;
        if out.truncated {
            return Err(ProviderError::Other(
                "proton-drive printed more than the output limit; none of it was trusted".into(),
            ));
        }
        Ok(out)
    }

    async fn exec(&self, cmd: CliCommand, timeout: Duration) -> PResult<CliOutput> {
        self.check_binary().await?;
        self.run(cmd, timeout).await
    }

    async fn info(&self, path: &RemotePath) -> PResult<ProtonNode> {
        let out = self
            .exec(
                CliCommand::Info(path.clone()),
                self.config.timeouts.metadata,
            )
            .await?;
        parse_node(success(&out)?)
    }

    /// `None` if nothing is at `path`.
    async fn lookup(&self, path: &RemotePath) -> PResult<Option<ProtonNode>> {
        match self.info(path).await {
            Ok(node) => Ok(Some(node)),
            Err(ProviderError::NotFound(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// The size of everything under `folder` (relative to the root),
    /// trashed files included.
    pub async fn folder_size(&self, folder: &RelPath) -> PResult<FolderSize> {
        let path = self.remote_path(folder)?;
        let out = self
            .exec(CliCommand::Size(path), self.config.timeouts.metadata)
            .await?;
        parse_folder_size(success(&out)?)
    }

    async fn create_folder(&self, parent: &RemotePath, name: &str) -> PResult<ProtonNode> {
        let path = parent.join(name)?;
        let cmd = CliCommand::CreateFolder {
            parent: parent.clone(),
            name: name.to_string(),
        };
        let out = self.exec(cmd, self.config.timeouts.metadata).await?;
        match success(&out).and_then(parse_node) {
            Ok(node) if node.name.as_deref() == Some(name) => Ok(node),
            Ok(_) => Err(ProviderError::Other(format!(
                "proton-drive created something other than {path}"
            ))),
            // Made by someone else meanwhile, or made but not reported:
            // what's there now decides.
            Err(e) => match self.lookup(&path).await {
                Ok(Some(node)) if node.kind == ItemKind::Folder && !node.trashed => Ok(node),
                _ => Err(e),
            },
        }
    }

    /// Put `local` in a fresh folder beside it under the exact name `name`
    /// (the CLI names uploads after the local file) and upload it to
    /// `parent`.
    async fn upload(
        &self,
        parent: &RemotePath,
        name: &str,
        local: &Path,
        on_conflict: FileConflict,
    ) -> PResult<Transfer> {
        if let Some(why) = upload_name_problem(name) {
            return Err(bad_name(name, why));
        }
        let meta = std::fs::symlink_metadata(local)?;
        if !meta.is_file() {
            return Err(ProviderError::Other(format!(
                "{} is not a regular file",
                local.display()
            )));
        }
        let dir = StagedDir::create_in(parent_dir(local))?;
        let staged = dir.path().join(name);
        // A second name for the same bytes; a copy if that can't be done.
        if std::fs::hard_link(local, &staged).is_err() {
            std::fs::copy(local, &staged)?;
        }
        let cmd = CliCommand::Upload {
            local: std::path::absolute(&staged)?,
            parent: parent.clone(),
            on_conflict,
            thumbnails: self.config.thumbnails,
        };
        let out = self
            .exec(cmd, self.config.timeouts.transfer(meta.len()))
            .await?;
        drop(dir);
        transfer_outcome(&out)
    }

    async fn stat_path(&self, path: &RemotePath) -> PResult<RemoteStat> {
        let node = self.info(path).await?;
        if node.trashed {
            return Err(ProviderError::Conflict(format!("{path} is in the trash")));
        }
        if node.kind != ItemKind::File {
            return Err(ProviderError::Other(format!("{path} is a folder")));
        }
        let size = node
            .size
            .ok_or_else(|| ProviderError::Other(format!("Proton claims no size for {path}")))?;
        Ok(RemoteStat {
            uid: node.uid,
            claimed: ContentDigests {
                size,
                // An unverified claim is no claim.
                sha1: node.sha1.filter(|_| node.sha1_verified),
                ..Default::default()
            },
            revision: node.revision,
        })
    }
}

fn sha512_of(path: &Path) -> PResult<String> {
    let mut f = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC)
        .open(path)
        .map_err(|e| {
            ProviderError::Other(format!(
                "can't read proton-drive at {}: {e}",
                path.display()
            ))
        })?;
    let mut h = Sha512::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            return Ok(hex(&h.finalize()));
        }
        h.update(&buf[..n]);
    }
}

fn folder_uid(path: &RemotePath, node: ProtonNode) -> PResult<String> {
    if node.trashed {
        return Err(ProviderError::Conflict(format!("{path} is in the trash")));
    }
    match node.kind {
        ItemKind::Folder => Ok(node.uid),
        ItemKind::File => Err(ProviderError::Conflict(format!("{path} is a file"))),
    }
}

impl<R: CliRunner> MirrorDestination for ProtonCliDestination<R> {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            sha1: true,
            create_folder: true,
            revisions: self.config.revisions,
            round_trip: true,
            ..Default::default()
        }
    }

    fn name_problem(&self, name: &str, kind: ItemKind) -> Option<&'static str> {
        match kind {
            ItemKind::File => upload_name_problem(name),
            ItemKind::Folder => remote_name_problem(name),
        }
    }

    async fn account(&self) -> PResult<Account> {
        let node = self.info(&RemotePath::my_files()).await?;
        Ok(Account {
            provider: "proton".into(),
            id: node.uid,
            display_name: None,
            email: node.owner_email,
            kind: None,
        })
    }

    /// Entries whose names Proton can't decrypt, and trashed ones, are
    /// left out: no name the mirror uploads can match them.
    async fn list(&self, folder: &RelPath) -> PResult<Vec<DestEntry>> {
        let path = self.remote_path(folder)?;
        let out = self
            .exec(CliCommand::List(path), self.config.timeouts.metadata)
            .await?;
        Ok(parse_list(success(&out)?)?
            .into_iter()
            .filter(|n| !n.trashed)
            .filter_map(|n| {
                Some(DestEntry {
                    size: if n.kind == ItemKind::File {
                        n.size
                    } else {
                        None
                    },
                    name: n.name?,
                    uid: n.uid,
                    kind: n.kind,
                })
            })
            .collect())
    }

    async fn ensure_folder(&self, folder: &RelPath) -> PResult<String> {
        let target = self.remote_path(folder)?;
        if let Some(node) = self.lookup(&target).await? {
            return folder_uid(&target, node);
        }
        let mut at = RemotePath::my_files();
        let mut uid = None;
        for seg in target.segments() {
            let next = at.join(seg)?;
            let node = match self.lookup(&next).await? {
                Some(node) => node,
                None => self.create_folder(&at, seg).await?,
            };
            uid = Some(folder_uid(&next, node)?);
            at = next;
        }
        uid.ok_or_else(|| ProviderError::Other(format!("{target} could not be looked up")))
    }

    async fn upload_new(&self, folder: &RelPath, name: &str, local: &Path) -> PResult<RemoteStat> {
        let parent = self.remote_path(folder)?;
        let file = parent.join(name)?;
        match self
            .upload(&parent, name, local, FileConflict::Skip)
            .await?
        {
            Transfer::Done => self.stat_path(&file).await,
            // Skipped: the name was taken (by identical content or not).
            Transfer::Skipped => Err(ProviderError::Conflict(format!("{file} already exists"))),
        }
    }

    async fn upload_revision(&self, file: &RelPath, local: &Path) -> PResult<RemoteStat> {
        self.capabilities()
            .require(crate::traits::Capability::Revisions)?;
        let path = self.remote_path(file)?;
        let (Some(parent), Some(name)) = (path.parent(), path.name()) else {
            return Err(ProviderError::Other("the root is not a file".into()));
        };
        // Must already be a file: a revision of nothing would be a new file.
        self.stat_path(&path).await?;
        // Skipped means the content was identical: nothing new to keep.
        self.upload(&parent, name, local, FileConflict::CreateNewRevision)
            .await?;
        self.stat_path(&path).await
    }

    async fn stat(&self, file: &RelPath) -> PResult<RemoteStat> {
        self.stat_path(&self.remote_path(file)?).await
    }

    async fn download_to(&self, file: &RelPath, dest: &Path) -> PResult<ContentDigests> {
        let path = self.remote_path(file)?;
        let size = self.stat_path(&path).await?.claimed.size;
        let parent = parent_dir(dest);
        let dir = StagedDir::create_in(parent)?;
        let cmd = CliCommand::Download {
            file: path.clone(),
            into: std::path::absolute(dir.path())?,
        };
        let out = self.exec(cmd, self.config.timeouts.transfer(size)).await?;
        if transfer_outcome(&out)? == Transfer::Skipped {
            return Err(ProviderError::Other(format!(
                "proton-drive skipped {path} (Proton Docs and Sheets can't be downloaded)"
            )));
        }
        let mut got = Vec::new();
        for e in std::fs::read_dir(dir.path())? {
            got.push(e?.path());
        }
        let [only] = got.as_slice() else {
            return Err(ProviderError::Other(format!(
                "expected proton-drive to download one file, found {}",
                got.len()
            )));
        };
        if !std::fs::symlink_metadata(only)?.is_file() {
            return Err(ProviderError::Other(
                "proton-drive's download is not a regular file".into(),
            ));
        }
        let mut src = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(only)?;
        let mut tmp = StagedTemp::create_in(parent)?;
        let tmp =
            tokio::task::spawn_blocking(move || std::io::copy(&mut src, tmp.file()).map(|_| tmp))
                .await
                .map_err(|e| ProviderError::Other(format!("copying the download: {e}")))??;
        drop(dir);
        verify_and_publish(tmp, None, dest).await
    }
}
