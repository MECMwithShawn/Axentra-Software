//! `axentra-onedrive proton …` and `axentra-onedrive mirror …`: copying
//! OneDrive to Proton Drive one way, never deleting, moving, renaming or
//! replacing anything on either side.
//!
//! The work is done by `axentra-clouddrive-mirror` over two providers that
//! have no destructive methods at all: OneDrive read through [`GraphSource`]
//! (GET requests only), and Proton Drive through its official CLI,
//! [`ProtonCliDestination`], limited to an allowlist of commands. With
//! `--to <folder>` the destination is a folder on a local drive instead
//! ([`LocalFolderDestination`]), with its own manifest and staging folder.
//!
//! **The OneDrive sign-in.** The sync service refreshes `tokens.json` while
//! it runs, and these commands run beside it, so they read it with
//! [`TokenStore::open_read_only`]: they use the access token the service
//! keeps fresh and never refresh, save or remove it themselves. Two
//! processes refreshing one sign-in could save an older refresh token over
//! a newer one, rename each other's half-written `tokens.tmp` into place,
//! or (after a refused refresh) remove the file the service depends on. If
//! the token has run out (the service isn't running, say) a command stops
//! with "sign-in required" and asks for the service to be started; the file
//! is left exactly as it was.

use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use axentra_clouddrive_mirror::{
    plan_with_progress, ChangedSourcePolicy, ExecOptions, Executor, Manifest, MirrorError, Plan,
    PlanOptions, PlanProgress, Readiness, RunOutcome, RunSummary, SeedIndex,
};
use axentra_clouddrive_provider::proton_cli::{scrubbed_env, CheckedArgv, CliCommand};
use axentra_clouddrive_provider::{
    GraphSource, LocalFolderDestination, MirrorDestination, ProtonCliConfig, ProtonCliDestination,
    ProviderError, RelPath, RemotePath, SourceProvider,
};
use axentra_onedrive_core::config::{expand_home, Config, MirrorConfig, Paths};
use axentra_onedrive_graph::{Graph, TokenStore};
use clap::Subcommand;
use tokio::sync::watch;

/// The section of Proton Drive the mirror writes into.
const MY_FILES: &str = "/my-files";

/// The most a file mirrored to a local drive may be: the staging folder is
/// on the system disk, which has room for the biggest OneDrive files.
const LOCAL_STAGING_CAP: u64 = 64 << 30;

/// Files fetched ahead at once for a local drive. OneDrive answers each
/// request slowly but takes many at a time; most files are small.
const LOCAL_PARALLEL: usize = 8;

/// How long `proton login` waits for the browser sign-in.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(15 * 60);

#[derive(Subcommand)]
pub enum ProtonCmd {
    /// Check the Proton Drive CLI (pinned version and checksum) and whether
    /// it is signed in. Reads only; prints no account details.
    Status,
    /// Sign in to Proton Drive with Proton's official CLI (opens a browser
    /// and prints the sign-in URL).
    Login,
}

#[derive(Subcommand)]
pub enum MirrorCmd {
    /// Dry run: what a mirror run would do. Reads OneDrive and Proton Drive
    /// and changes nothing on either.
    Plan {
        /// Only this OneDrive folder, such as "/Documents/Taxes".
        #[arg(long, value_name = "ONEDRIVE PATH")]
        only: Option<String>,
        /// Mirror to this folder on a mounted drive instead of Proton Drive,
        /// such as "/media/me/Backup/OneDrive Mirror".
        #[arg(long, value_name = "FOLDER")]
        to: Option<PathBuf>,
        /// Print the plan as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Copy one OneDrive folder to Proton Drive. Adds folders and new files
    /// only; never deletes, moves, renames or replaces anything.
    Run {
        /// The OneDrive folder to mirror, such as "/Documents/Taxes".
        /// Required for now: mirroring the whole drive needs explicit
        /// approval and a future --all.
        #[arg(long, value_name = "ONEDRIVE PATH", conflicts_with = "all")]
        only: Option<String>,
        /// The whole OneDrive. Only with --to (a local drive) for now.
        #[arg(long)]
        all: bool,
        /// Mirror to this folder on a mounted drive instead of Proton Drive,
        /// such as "/media/me/Backup/OneDrive Mirror".
        #[arg(long, value_name = "FOLDER")]
        to: Option<PathBuf>,
        /// How many files to download from OneDrive at once (default 8 for
        /// a local drive, 1 for Proton Drive). Uploads stay one at a time.
        #[arg(long, value_name = "N", value_parser = clap::value_parser!(u16).range(1..=32))]
        parallel: Option<u16>,
        /// A folder on this computer that may hold copies of OneDrive files
        /// (an old Windows profile, say). A file whose copy there matches
        /// OneDrive's hash is taken from it instead of downloaded. Only
        /// read. Only with --to; may be given more than once.
        #[arg(long, value_name = "FOLDER", requires = "to")]
        seed_from: Vec<PathBuf>,
        /// Read each file back from the destination and compare it (L4).
        #[arg(long)]
        round_trip: bool,
        /// Don't ask before starting.
        #[arg(long, short)]
        yes: bool,
    },
    /// What has been mirrored, and how far each file was verified.
    Status {
        /// The local folder given to `run --to`, instead of Proton Drive.
        #[arg(long, value_name = "FOLDER")]
        to: Option<PathBuf>,
        /// Print the report as JSON.
        #[arg(long)]
        json: bool,
    },
}

/// Everything a mirror command needs from config.toml and [`Paths`].
#[derive(Debug, Clone)]
pub struct Settings {
    pub proton: ProtonCliConfig,
    /// The mirror's folder under `/my-files`, such as `OneDrive`.
    pub dest_root: String,
    pub staging_dir: PathBuf,
    pub staging_cap: u64,
    pub on_source_change: ChangedSourcePolicy,
    pub manifest: PathBuf,
    /// The destination folder as the user knows it.
    pub shown: String,
}

impl Settings {
    pub fn new(paths: &Paths, m: &MirrorConfig) -> Result<Self> {
        let dest_root = dest_root_under_my_files(&m.dest_root)?;
        let on_source_change = match m.changed_content.trim() {
            "diverged" => ChangedSourcePolicy::MarkDiverged,
            // Kept as a choice for later; not offered yet.
            "create-new-revision" => bail!(
                "[mirror] changed_content = \"create-new-revision\" isn't available yet; \
                 use \"diverged\""
            ),
            other => bail!("[mirror] changed_content = {other:?} isn't known; use \"diverged\""),
        };
        let binary = m.proton_cli_path();
        if !binary.is_absolute() {
            bail!(
                "[mirror] proton_cli must be an absolute path (or start with ~/), not {}",
                binary.display()
            );
        }
        let mut proton = ProtonCliConfig::new(binary);
        // The destination's own root is /my-files; the mirror's folder is a
        // path below it, so the planner lists it like any other folder.
        proton.root = MY_FILES.into();
        // Both live beside the sync service's files, in their own names:
        // ~/.local/share/axentra/onedrive/mirror.sqlite and
        // ~/.cache/axentra/onedrive/mirror-staging.
        let data_dir = paths
            .tokens_file
            .parent()
            .ok_or_else(|| anyhow!("no data folder"))?;
        let cache_dir = paths
            .cache_dir
            .parent()
            .ok_or_else(|| anyhow!("no cache folder"))?;
        let staging_dir = m
            .staging_path()
            .unwrap_or_else(|| cache_dir.join("mirror-staging"));
        // The run clears what it left there and makes the folder private;
        // none of that may reach anything else.
        if let Some(why) =
            staging_problem(&staging_dir, &expand_home("~"), data_dir, &paths.cache_dir)
        {
            bail!(
                "[mirror] staging_dir {}: {why}; use a folder of its own, such as \
                 ~/.cache/axentra/onedrive/mirror-staging",
                staging_dir.display()
            );
        }
        Ok(Self {
            proton,
            staging_dir,
            staging_cap: m.staging_cap_bytes,
            on_source_change,
            manifest: data_dir.join("mirror.sqlite"),
            shown: format!("{MY_FILES}/{dest_root}"),
            dest_root,
        })
    }

    /// The same settings for mirroring to `to`, a folder on a mounted drive
    /// (made if missing): its own manifest and staging folder, named after
    /// `to`, so it never mixes with the Proton Drive mirror's.
    pub fn for_local_folder(mut self, to: &Path) -> Result<(Self, LocalFolderDestination)> {
        let (mount, below) = mount_point_of(to)?;
        let dest = LocalFolderDestination::on_mount(&mount)
            .with_context(|| format!("--to {}", to.display()))?;
        if below.is_root() {
            bail!(
                "--to {}: give a folder on the drive, such as \"{}/OneDrive Mirror\"",
                to.display(),
                mount.display()
            );
        }
        let key: String = <sha1::Sha1 as sha1::Digest>::digest(to.as_os_str().as_bytes())
            .iter()
            .take(6)
            .map(|b| format!("{b:02x}"))
            .collect();
        let data_dir = self
            .manifest
            .parent()
            .ok_or_else(|| anyhow!("no data folder"))?;
        self.manifest = data_dir.join(format!("mirror-local-{key}.sqlite"));
        let mut staging = self.staging_dir.as_os_str().to_owned();
        staging.push(format!("-local-{key}"));
        self.staging_dir = staging.into();
        self.staging_cap = LOCAL_STAGING_CAP;
        self.dest_root = below.to_string();
        self.shown = to.display().to_string();
        Ok((self, dest))
    }

    pub fn plan_options(&self, only: Option<&str>, round_trip: bool) -> PlanOptions {
        PlanOptions {
            source_prefix: only.map(str::to_string),
            dest_root: self.dest_root.clone(),
            staging_cap: self.staging_cap,
            round_trip,
            on_source_change: self.on_source_change,
        }
    }

    /// The folder the mirror writes into.
    pub fn shown_dest(&self) -> String {
        self.shown.clone()
    }

    /// The staging folder must be outside the OneDrive folder the sync
    /// service keeps: whatever happens there is sent up to OneDrive.
    pub fn check_sync_root(&self, sync_root: &Path) -> Result<()> {
        if overlaps(&self.staging_dir, sync_root) {
            bail!(
                "[mirror] staging_dir {} overlaps the OneDrive folder {}; use a folder of its \
                 own, such as ~/.cache/axentra/onedrive/mirror-staging",
                self.staging_dir.display(),
                sync_root.display()
            );
        }
        Ok(())
    }
}

/// Why `staging` can't be the mirror's staging folder, or `None`.
fn staging_problem(
    staging: &Path,
    home: &Path,
    data_dir: &Path,
    files: &Path,
) -> Option<&'static str> {
    use std::path::Component;
    let mut parts = staging.components();
    if parts.next() != Some(Component::RootDir) || !parts.all(|c| matches!(c, Component::Normal(_)))
    {
        return Some("must be an absolute path without . or ..");
    }
    let shown = staging.to_string_lossy();
    if shown.contains(['*', '?', '[', '{']) || shown.chars().any(char::is_control) {
        return Some("must not contain *, ?, [, { or control characters");
    }
    if home.starts_with(staging) {
        return Some("is your home folder or holds it");
    }
    if data_dir.starts_with(staging) {
        return Some("is the sync service's data folder or holds it");
    }
    if overlaps(staging, files) {
        return Some("overlaps the sync service's file cache");
    }
    None
}

/// The mount point `to` is on, and the rest of `to` below it. `to` need
/// not exist yet; the nearest folder above it that does is where the
/// search starts.
fn mount_point_of(to: &Path) -> Result<(PathBuf, RelPath)> {
    let mut parts = to.components();
    if parts.next() != Some(Component::RootDir) || !parts.all(|c| matches!(c, Component::Normal(_)))
    {
        bail!("--to {}: must be an absolute path without . or ..", to.display());
    }
    let mut at = to.to_path_buf();
    let meta = loop {
        match std::fs::metadata(&at) {
            Ok(m) => break m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => match at.parent() {
                Some(p) => at = p.to_path_buf(),
                None => bail!("--to {}: no part of it exists", to.display()),
            },
            Err(e) => return Err(e).with_context(|| format!("--to {}", to.display())),
        }
    };
    let dev = meta.dev();
    while let Some(p) = at.parent() {
        if std::fs::metadata(p)?.dev() != dev {
            break;
        }
        at = p.to_path_buf();
    }
    let rest = to
        .strip_prefix(&at)
        .map_err(|_| anyhow!("--to {}: can't tell which drive it is on", to.display()))?;
    let rest = rest
        .to_str()
        .ok_or_else(|| anyhow!("--to {}: not UTF-8", to.display()))?;
    let below = RelPath::parse(rest).map_err(|e| anyhow!("--to {}: {e}", to.display()))?;
    Ok((at, below))
}

/// One of `a` and `b` is, or is inside, the other.
fn overlaps(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}

/// `/my-files/OneDrive` → `OneDrive`. Anything outside `/my-files`, or
/// `/my-files` itself, is refused.
fn dest_root_under_my_files(dest_root: &str) -> Result<String> {
    let parsed = RemotePath::parse(dest_root)
        .map_err(|e| anyhow!("[mirror] dest_root {dest_root:?}: {e}"))?;
    let below = parsed.segments();
    if below.is_empty() {
        bail!("[mirror] dest_root must be a folder inside /my-files, such as /my-files/OneDrive");
    }
    Ok(below.join("/"))
}

/// The OneDrive folder `--only` names, as `/A/B`; `None` for the whole
/// drive (no `--only`, or `--only /`).
pub fn scope(only: Option<&str>) -> Result<Option<String>> {
    let Some(only) = only else {
        return Ok(None);
    };
    let rel = RelPath::parse(only.trim_start_matches('/'))
        .map_err(|e| anyhow!("--only {only:?}: {e}"))?;
    Ok((!rel.is_root()).then(|| format!("/{rel}")))
}

/// A OneDrive folder to limit a run to. Required for now: a whole-drive
/// run is refused until it is approved and given its own `--all`.
pub fn require_only(only: Option<&str>) -> Result<String> {
    scope(only)?.ok_or_else(|| {
        anyhow!(
            "mirroring the whole OneDrive needs your explicit approval and a future --all flag; \
             for now give one folder with --only \"/Some Folder\" (try `mirror plan` first)"
        )
    })
}

/// The OneDrive half: read-only, with the sign-in the sync service keeps.
pub fn onedrive_source(paths: &Paths, cfg: &Config) -> Result<GraphSource> {
    if cfg.client_id.is_empty() {
        bail!("run `axentra-onedrive setup` and `axentra-onedrive login` first");
    }
    if !paths.tokens_file.exists() {
        bail!("not signed in to OneDrive (run `axentra-onedrive login`)");
    }
    // See the module docs: never refreshed or saved from this process.
    let tokens = Arc::new(TokenStore::open_read_only(&paths.tokens_file, cfg.auth()));
    Ok(GraphSource::new(Arc::new(Graph::new(
        &cfg.graph_base,
        tokens,
    ))))
}

/// The tools' own words, with what to do about the ones the user can fix.
fn explain(e: MirrorError) -> anyhow::Error {
    match e {
        MirrorError::Provider(ProviderError::AuthRequired) => anyhow!(
            "sign-in required. For OneDrive, start the sync service so it renews the sign-in \
             (systemctl --user start axentra-onedrive) or run `axentra-onedrive login`; for \
             Proton Drive, run `axentra-onedrive proton login`. Then try again."
        ),
        e => e.into(),
    }
}

/// How often, at most, planning says how far it has got.
const PROGRESS_EVERY: Duration = Duration::from_secs(2);

/// Planning's progress on stderr: a count of OneDrive items at most every
/// [`PROGRESS_EVERY`], and a line when the destination is listed. Counts only,
/// never a name.
struct ProgressLine {
    every: Duration,
    last: Mutex<Instant>,
    say: Box<dyn Fn(&str) + Send + Sync>,
}

impl ProgressLine {
    fn stderr() -> Self {
        Self::new(PROGRESS_EVERY, Box::new(|line| eprintln!("{line}")))
    }

    fn new(every: Duration, say: Box<dyn Fn(&str) + Send + Sync>) -> Self {
        Self {
            every,
            last: Mutex::new(Instant::now()),
            say,
        }
    }

    fn report(&self, p: PlanProgress) {
        match p {
            PlanProgress::SourceItems(n) => {
                let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
                if last.elapsed() >= self.every {
                    *last = Instant::now();
                    (self.say)(&format!("Listing OneDrive\u{2026} {} items", thousands(n)));
                }
            }
            PlanProgress::ListingDestination => (self.say)("Listing the destination\u{2026}"),
        }
    }
}

/// `12400` as `12,400`.
fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// [`plan_with_progress`], reporting on stderr.
async fn plan_reporting<S, D>(
    source: &S,
    dest: &D,
    m: &Manifest,
    opts: &PlanOptions,
) -> Result<Plan>
where
    S: SourceProvider,
    D: MirrorDestination,
{
    let line = ProgressLine::stderr();
    plan_with_progress(source, dest, &m.snapshot()?, opts, &|p| line.report(p))
        .await
        .map_err(explain)
}

/// Plan without changing anything anywhere, and note the dry run in the
/// manifest.
pub async fn dry_run<S, D>(
    source: &S,
    dest: &D,
    manifest: &Path,
    opts: &PlanOptions,
) -> Result<Plan>
where
    S: SourceProvider,
    D: MirrorDestination,
{
    let mut m = open_manifest(manifest)?;
    let p = plan_reporting(source, dest, &m, opts).await?;
    p.record_dry_run(&mut m)?;
    Ok(p)
}

/// Plan, ask with `confirm` (given the plan), and carry it out. `None` if
/// the answer was no.
pub async fn run<S, D>(
    source: &S,
    dest: &D,
    manifest: &Path,
    opts: &PlanOptions,
    exec: ExecOptions,
    confirm: impl FnOnce(&Plan) -> Result<bool>,
    stop: watch::Receiver<bool>,
) -> Result<Option<RunSummary>>
where
    S: SourceProvider,
    D: MirrorDestination,
{
    let mut m = open_manifest(manifest)?;
    let p = plan_reporting(source, dest, &m, opts).await?;
    if !confirm(&p)? {
        return Ok(None);
    }
    let sum = Executor::new(source, dest, &mut m, exec)
        .run(&p, stop)
        .await
        .map_err(explain)?;
    Ok(Some(sum))
}

fn open_manifest(path: &Path) -> Result<Manifest> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    Manifest::open(path).with_context(|| format!("opening {}", path.display()))
}

pub async fn proton(paths: &Paths, cmd: ProtonCmd) -> Result<()> {
    let cfg = Config::load(&paths.config_file)?;
    let settings = Settings::new(paths, &cfg.mirror)?;
    match cmd {
        ProtonCmd::Status => proton_status(&settings).await,
        ProtonCmd::Login => proton_login(&settings).await,
    }
}

async fn proton_status(settings: &Settings) -> Result<()> {
    let dest = ProtonCliDestination::with_process_runner(settings.proton.clone())?;
    dest.check_binary()
        .await
        .context("checking the Proton Drive CLI")?;
    println!(
        "Proton Drive CLI: ok ({}, checksum matches)",
        axentra_clouddrive_provider::proton_cli::PINNED_VERSION
    );
    // A read-only listing of /my-files; what's in it isn't shown.
    match dest.list(&RelPath::root()).await {
        Ok(_) => {
            println!("Proton Drive: signed in (ok)");
            Ok(())
        }
        Err(ProviderError::AuthRequired) => {
            println!("Proton Drive: not signed in (run `axentra-onedrive proton login`)");
            std::process::exit(1);
        }
        Err(e) => Err(e).context("checking the Proton Drive sign-in"),
    }
}

/// `proton-drive auth login`, with the terminal attached so the sign-in
/// URL it prints shows straight away. It is the same allowlisted argument
/// list and scrubbed environment the mirror uses, after the same binary
/// check; only the output isn't captured, since the command waits for the
/// browser.
async fn proton_login(settings: &Settings) -> Result<()> {
    let dest = ProtonCliDestination::with_process_runner(settings.proton.clone())?;
    dest.check_binary()
        .await
        .context("checking the Proton Drive CLI")?;
    let argv = CheckedArgv::new(CliCommand::AuthLogin.argv())?;
    let env = scrubbed_env(
        std::env::vars_os(),
        true,
        settings.proton.cache_dir.as_deref(),
    )?;
    println!(
        "This is a third-party application not officially supported by Proton.\n\n\
         Signing in with Proton's official Proton Drive CLI (a separate program, under Proton's \
         terms). It opens your browser; if it doesn't, open the URL it prints (any device \
         works). Keep this terminal open until it says it's done.\n"
    );
    let mut child = tokio::process::Command::new(&settings.proton.binary)
        .args(argv.args())
        .env_clear()
        .envs(env)
        .current_dir("/")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .context("starting proton-drive")?;
    let status = tokio::time::timeout(LOGIN_TIMEOUT, child.wait())
        .await
        .map_err(|_| anyhow!("the Proton sign-in didn't finish within 15 minutes"))??;
    if !status.success() {
        bail!("the Proton sign-in didn't complete");
    }
    println!("\nNext: axentra-onedrive proton status");
    Ok(())
}

pub async fn mirror(paths: &Paths, cmd: MirrorCmd) -> Result<()> {
    let cfg = Config::load(&paths.config_file)?;
    let settings = Settings::new(paths, &cfg.mirror)?;
    match cmd {
        MirrorCmd::Plan { only, to, json } => {
            let only = scope(only.as_deref())?;
            let source = onedrive_source(paths, &cfg)?;
            match to {
                None => {
                    let dest = ProtonCliDestination::with_process_runner(settings.proton.clone())?;
                    plan_cmd(&source, &dest, &settings, only.as_deref(), json).await
                }
                Some(to) => {
                    let (settings, dest) = settings.for_local_folder(&to)?;
                    plan_cmd(&source, &dest, &settings, only.as_deref(), json).await
                }
            }
        }
        MirrorCmd::Run {
            only,
            all,
            to,
            parallel,
            seed_from,
            round_trip,
            yes,
        } => {
            let only = if all {
                if to.is_none() {
                    bail!(
                        "--all is only for a local drive (--to) for now; mirroring the whole \
                         OneDrive to Proton Drive needs your explicit approval"
                    );
                }
                None
            } else {
                Some(require_only(only.as_deref())?)
            };
            settings.check_sync_root(&cfg.mount_path())?;
            let source = onedrive_source(paths, &cfg)?;
            let summary = match to {
                None => {
                    let dest = ProtonCliDestination::with_process_runner(settings.proton.clone())?;
                    let parallel = parallel.map_or(1, usize::from);
                    let exec = ExecOptions::new(settings.staging_dir.clone());
                    run_cmd(&source, &dest, &settings, only, exec, parallel, round_trip, yes)
                        .await?
                }
                Some(to) => {
                    let (settings, dest) = settings.for_local_folder(&to)?;
                    settings.check_sync_root(&cfg.mount_path())?;
                    let parallel = parallel.map_or(LOCAL_PARALLEL, usize::from);
                    let mut exec = ExecOptions::new(settings.staging_dir.clone());
                    // OneDrive limits requests per account; the lookup
                    // before each download is the one that can go.
                    exec.trust_plan = true;
                    // A pause beats ending a whole-drive run, whose next
                    // pass has to list OneDrive again first.
                    exec.wait_out_rate_limits = true;
                    if !seed_from.is_empty() {
                        exec.seed = Some(Arc::new(seed_index(&seed_from)?));
                    }
                    run_cmd(&source, &dest, &settings, only, exec, parallel, round_trip, yes)
                        .await?
                }
            };
            let Some(sum) = summary else {
                println!("Nothing was changed.");
                return Ok(());
            };
            print!("{}", summary_text(&sum));
            match sum.outcome {
                RunOutcome::Completed | RunOutcome::Cancelled => Ok(()),
                RunOutcome::AuthRequired => Err(explain(ProviderError::AuthRequired.into())),
                other => bail!("the run stopped: {}", other.as_str()),
            }
        }
        MirrorCmd::Status { to, json } => {
            let settings = match to {
                None => settings,
                Some(to) => settings.for_local_folder(&to)?.0,
            };
            if !settings.manifest.exists() {
                println!("Nothing has been mirrored yet (try `axentra-onedrive mirror plan`).");
                return Ok(());
            }
            let m = Manifest::open(&settings.manifest)?;
            let r = Readiness::from_manifest(&m)?;
            if json {
                println!("{}", r.to_json()?);
            } else {
                print!("{}", r.render_text());
            }
            Ok(())
        }
    }
}

/// Index the `--seed-from` folders, with a count on stderr as it goes.
fn seed_index(roots: &[PathBuf]) -> Result<SeedIndex> {
    for r in roots {
        if !r.is_absolute() {
            bail!("--seed-from {}: must be an absolute path", r.display());
        }
    }
    let index = SeedIndex::scan(roots, &|n| {
        eprintln!("Indexing local copies\u{2026} {} files", thousands(n))
    })
    .context("--seed-from")?;
    eprintln!("Local copies: {} files indexed", thousands(index.len()));
    Ok(index)
}

async fn plan_cmd<S, D>(
    source: &S,
    dest: &D,
    settings: &Settings,
    only: Option<&str>,
    json: bool,
) -> Result<()>
where
    S: SourceProvider,
    D: MirrorDestination,
{
    let p = dry_run(
        source,
        dest,
        &settings.manifest,
        &settings.plan_options(only, false),
    )
    .await?;
    if json {
        println!("{}", p.to_json()?);
    } else {
        print!("{}", p.render_text_for(&settings.shown_dest()));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run_cmd<S, D>(
    source: &S,
    dest: &D,
    settings: &Settings,
    only: Option<String>,
    mut exec: ExecOptions,
    parallel: usize,
    round_trip: bool,
    yes: bool,
) -> Result<Option<RunSummary>>
where
    S: SourceProvider,
    D: MirrorDestination,
{
    let (tx, rx) = watch::channel(false);
    let shown = settings.shown_dest();
    // Ctrl-C ends the process as usual while planning and at the
    // question; it asks for a stop only once the run has started.
    let confirm = move |p: &Plan| -> Result<bool> {
        print!("{}", p.render_text_for(&shown));
        let go = yes || {
            let answer = crate::prompt(&format!(
                "\nAdd these folders and files to {shown}? Nothing is deleted, moved or \
                 replaced on either side. [y/N] "
            ))?;
            matches!(answer.to_lowercase().as_str(), "y" | "yes")
        };
        if go {
            stop_on_ctrl_c(tx);
        }
        Ok(go)
    };
    let opts = settings.plan_options(only.as_deref(), round_trip);
    exec.parallel = parallel;
    run(source, dest, &settings.manifest, &opts, exec, confirm, rx).await
}

/// The first Ctrl-C asks the run to stop after the current step (an
/// upload in progress is let finish); a second one quits at once.
fn stop_on_ctrl_c(tx: watch::Sender<bool>) {
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_err() {
            return;
        }
        eprintln!(
            "\nStopping after the current step; an upload in progress is let finish. Press \
             Ctrl-C again to quit now (an interrupted upload is checked on the next run)."
        );
        let _ = tx.send(true);
        if tokio::signal::ctrl_c().await.is_ok() {
            eprintln!("Quitting now.");
            std::process::exit(130);
        }
    });
}

pub fn summary_text(s: &RunSummary) -> String {
    format!(
        "Run {}: {}\n  {} folders created, {} files uploaded ({} from local copies), {} adopted, \
         {} verified further\n  \
         {} diverged, {} skipped, {} failed, {} waiting to retry, {} gone from OneDrive \
         (kept at the destination)\n  {} uploaded, staging peaked at {}\n",
        s.run_id,
        s.outcome.as_str(),
        s.folders_created,
        s.files_mirrored,
        s.files_seeded,
        s.files_adopted,
        s.files_verified,
        s.files_diverged,
        s.files_skipped,
        s.files_failed,
        s.files_waiting,
        s.source_deleted,
        axentra_onedrive_core::status::human_size(s.bytes_uploaded),
        axentra_onedrive_core::status::human_size(s.staging_peak),
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use axentra_clouddrive_mirror::{Action, ItemState};
    use axentra_clouddrive_provider::proton_cli::{CliOutput, CliRunner, RunError, RunLimits};
    use axentra_onedrive_mock::MockServer;
    use parking_lot::Mutex;
    use sha1::{Digest, Sha1};

    /// Proton Drive as `proton-drive` 0.9.0 shows it, in memory: answers
    /// the allowlisted commands with the shapes the real CLI prints, and
    /// logs every argument list it was given.
    #[derive(Default)]
    struct ProtonFs {
        nodes: Mutex<BTreeMap<String, Option<Vec<u8>>>>,
        calls: Mutex<Vec<Vec<String>>>,
    }

    impl ProtonFs {
        fn new() -> Self {
            let fs = Self::default();
            fs.nodes.lock().insert("/my-files".into(), None);
            fs
        }

        fn content(&self, path: &str) -> Option<Vec<u8>> {
            self.nodes.lock().get(path).cloned().flatten()
        }

        fn snapshot(&self) -> BTreeMap<String, Option<Vec<u8>>> {
            self.nodes.lock().clone()
        }

        fn calls(&self) -> Vec<Vec<String>> {
            self.calls.lock().clone()
        }

        fn node(path: &str, content: &Option<Vec<u8>>) -> serde_json::Value {
            let name = path.rsplit('/').next().unwrap();
            let uid = format!("VOLUMEPLACEHOLDER0000A~{:x}", Sha1::digest(path.as_bytes()));
            let mut n = serde_json::json!({
                "uid": uid,
                "name": {"ok": true, "value": name},
                "ownedBy": {"email": "owner@example.invalid"},
                "type": if content.is_some() { "file" } else { "folder" },
            });
            if let Some(data) = content {
                n["activeRevision"] = serde_json::json!({
                    "uid": format!("{uid}~REVISION"),
                    "claimedSize": data.len(),
                    "claimedDigests": {"sha1": format!("{:x}", Sha1::digest(data)), "sha1Verified": true},
                });
            }
            n
        }

        fn answer(&self, args: &[String]) -> CliOutput {
            let ok = |v: serde_json::Value| CliOutput::new(0, v.to_string(), "");
            let missing = |p: &str| {
                CliOutput::new(
                    1,
                    "",
                    format!("Node not found: {}\n", p.rsplit('/').next().unwrap()),
                )
            };
            let summary = |moved: u64, bytes: u64, skipped: u64| {
                ok(
                    serde_json::json!({"transferredItems": moved, "transferredBytes": bytes,
                    "skippedItems": skipped, "failedItems": 0, "failures": []}),
                )
            };
            if args == ["version"] {
                return CliOutput::new(0, "Proton Drive CLI cli-drive@0.9.0+test\n", "");
            }
            let dash = args
                .iter()
                .position(|a| a == "--")
                .expect("a -- before paths");
            let pos = &args[dash + 1..];
            let mut nodes = self.nodes.lock();
            match args[1].as_str() {
                "info" => match nodes.get(&pos[0]) {
                    Some(c) => ok(Self::node(&pos[0], c)),
                    None => missing(&pos[0]),
                },
                "list" => {
                    if !nodes.contains_key(&pos[0]) {
                        return missing(&pos[0]);
                    }
                    let prefix = format!("{}/", pos[0]);
                    let children: Vec<_> = nodes
                        .iter()
                        .filter(|(p, _)| p.strip_prefix(&prefix).is_some_and(|r| !r.contains('/')))
                        .map(|(p, c)| Self::node(p, c))
                        .collect();
                    ok(children.into())
                }
                "create-folder" => {
                    let path = format!("{}/{}", pos[0], pos[1]);
                    if !nodes.contains_key(&pos[0]) {
                        return missing(&pos[0]);
                    }
                    assert!(!nodes.contains_key(&path), "folder made twice: {path}");
                    nodes.insert(path.clone(), None);
                    ok(Self::node(&path, &None))
                }
                "upload" => {
                    let local = Path::new(&pos[0]);
                    let path = format!(
                        "{}/{}",
                        pos[1],
                        local.file_name().unwrap().to_str().unwrap()
                    );
                    if !nodes.contains_key(&pos[1]) {
                        return missing(&pos[1]);
                    }
                    if nodes.contains_key(&path) {
                        return summary(0, 0, 1);
                    }
                    let data = std::fs::read(local).unwrap();
                    let n = data.len() as u64;
                    nodes.insert(path, Some(data));
                    summary(1, n, 0)
                }
                "download" => match nodes.get(&pos[0]) {
                    Some(Some(data)) => {
                        let name = pos[0].rsplit('/').next().unwrap();
                        std::fs::write(Path::new(&pos[1]).join(name), data).unwrap();
                        summary(1, data.len() as u64, 0)
                    }
                    _ => missing(&pos[0]),
                },
                other => panic!("unexpected subcommand {other}"),
            }
        }
    }

    impl CliRunner for &ProtonFs {
        async fn run(&self, argv: &CheckedArgv, _: RunLimits) -> Result<CliOutput, RunError> {
            let args: Vec<String> = argv
                .args()
                .iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            self.calls.lock().push(args.clone());
            Ok(self.answer(&args))
        }
    }

    /// Every argument list is one of the allowlisted read or add commands,
    /// and nothing in it could remove, replace or move anything.
    fn assert_allowed(calls: &[Vec<String>]) {
        const FORBIDDEN: [&str; 14] = [
            "delete",
            "trash",
            "empty-trash",
            "move",
            "rename",
            "restore",
            "copy",
            "replace",
            "remove",
            "sharing",
            "album",
            "photo",
            "takeout",
            "logout",
        ];
        for args in calls {
            match args.first().map(String::as_str) {
                Some("version") => assert_eq!(args.len(), 1),
                Some("filesystem") => {
                    assert!(
                        ["list", "info", "create-folder", "upload", "download"]
                            .contains(&args[1].as_str()),
                        "{args:?}"
                    );
                    assert!(args.contains(&"--json".to_string()), "{args:?}");
                }
                _ => panic!("unexpected command {args:?}"),
            }
            for a in args {
                assert!(!FORBIDDEN.contains(&a.as_str()), "{args:?}");
            }
            if let Some(i) = args.iter().position(|a| a == "--file-conflict-strategy") {
                assert_eq!(args[i + 1], "skip", "{args:?}");
            }
        }
    }

    struct Env {
        mock: MockServer,
        paths: Paths,
        cfg: Config,
        _dir: tempfile::TempDir,
    }

    impl Env {
        async fn new(expired: bool) -> Self {
            let mock = MockServer::start().await;
            let dir = tempfile::tempdir().unwrap();
            let paths = Paths::under(dir.path());
            std::fs::write(&paths.tokens_file, mock.tokens_json(expired)).unwrap();
            let cfg = Config {
                client_id: "test".into(),
                authority: mock.origin.clone(),
                graph_base: mock.graph_base.clone(),
                ..Default::default()
            };
            Env {
                mock,
                paths,
                cfg,
                _dir: dir,
            }
        }

        fn settings(&self) -> Settings {
            let mut s = Settings::new(&self.paths, &self.cfg.mirror).unwrap();
            // The fake runner has no binary to hash; the version is still checked.
            s.proton.expected_sha512 = None;
            s.proton.binary = "/nonexistent/proton-drive".into();
            s.staging_cap = 1 << 20;
            s
        }

        fn exec(&self, s: &Settings) -> ExecOptions {
            let mut o = ExecOptions::new(s.staging_dir.clone());
            o.min_free_bytes = 0;
            o
        }

        /// Only GET requests reached OneDrive.
        fn assert_read_only(&self) {
            for method in ["DELETE", "PATCH", "PUT", "POST"] {
                assert_eq!(self.mock.requests(&format!("{method} ")), 0, "{method}");
            }
        }
    }

    fn stop() -> watch::Receiver<bool> {
        watch::channel(false).1
    }

    #[test]
    fn settings_come_from_the_mirror_table() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        let s = Settings::new(&paths, &MirrorConfig::default()).unwrap();
        assert_eq!(s.proton.root, "/my-files");
        assert_eq!(s.dest_root, "OneDrive");
        assert_eq!(s.shown_dest(), "/my-files/OneDrive");
        assert_eq!(s.staging_cap, 2 << 30);
        assert_eq!(s.on_source_change, ChangedSourcePolicy::MarkDiverged);
        assert_eq!(s.manifest, dir.path().join("mirror.sqlite"));
        assert_eq!(s.staging_dir, dir.path().join("mirror-staging"));
        // Upload conflicts skip and revisions stay off.
        assert!(!s.proton.revisions);
        assert!(s.proton.expected_sha512.is_some());
        assert!(s
            .proton
            .binary
            .ends_with(".local/share/axentra/proton-drive-cli/0.9.0/proton-drive"));

        let with = |f: fn(&mut MirrorConfig)| {
            let mut m = MirrorConfig::default();
            f(&mut m);
            Settings::new(&paths, &m)
        };
        let deeper = with(|m| m.dest_root = "/my-files/Backups/OneDrive".into()).unwrap();
        assert_eq!(deeper.dest_root, "Backups/OneDrive");
        for bad in [
            "/my-files",
            "/my-files/",
            "/OneDrive",
            "my-files/OneDrive",
            "/my-files/a/../b",
        ] {
            let m = MirrorConfig {
                dest_root: bad.into(),
                ..Default::default()
            };
            assert!(Settings::new(&paths, &m).is_err(), "{bad}");
        }
        assert!(with(|m| m.changed_content = "create-new-revision".into()).is_err());
        assert!(with(|m| m.changed_content = "replace".into()).is_err());
        assert!(with(|m| m.proton_cli = "proton-drive".into()).is_err());
    }

    /// The staging folder is cleared and made private on every run, so it
    /// can't be anything holding other files.
    #[test]
    fn the_staging_folder_must_be_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        let home = expand_home("~");
        let staged = |s: &str| {
            let m = MirrorConfig {
                staging_dir: s.into(),
                ..Default::default()
            };
            Settings::new(&paths, &m)
        };
        let root = dir.path().to_str().unwrap();
        for bad in [
            "relative/stage".to_string(),
            "~".into(),
            "/".into(),
            home.parent().unwrap().to_str().unwrap().into(),
            format!("{root}/a/../stage"),
            format!("{root}/st[age"),
            root.into(),
            format!("{root}/files"),
            format!("{root}/files/m"),
        ] {
            assert!(staged(&bad).is_err(), "{bad}");
        }
        let ok = staged(&format!("{root}/stage")).unwrap();
        assert_eq!(ok.staging_dir, dir.path().join("stage"));
        assert!(staged("~/.cache/elsewhere/stage").is_ok());

        let s = Settings::new(&paths, &MirrorConfig::default()).unwrap();
        assert!(s.check_sync_root(&dir.path().join("OneDrive")).is_ok());
        assert!(s.check_sync_root(dir.path()).is_err());
        assert!(s.check_sync_root(&s.staging_dir.join("inner")).is_err());
        let inside = staged(&format!("{root}/OneDrive/m")).unwrap();
        assert!(inside
            .check_sync_root(&dir.path().join("OneDrive"))
            .is_err());
    }

    #[test]
    fn a_local_target_is_split_at_its_mount_point() {
        let dir = tempfile::tempdir().unwrap();
        let to = dir.path().join("not yet/Mirror");
        let (mount, below) = mount_point_of(&to).unwrap();
        assert!(dir.path().starts_with(&mount));
        assert!(below.to_string().ends_with("not yet/Mirror"));
        for bad in ["relative/x", "/a/../b"] {
            assert!(mount_point_of(Path::new(bad)).is_err(), "{bad}");
        }
        // A folder on the system disk (or not on a drive of its own) is
        // refused before anything is made.
        let s = Settings::new(&Paths::under(dir.path()), &MirrorConfig::default()).unwrap();
        assert!(s.for_local_folder(&to).is_err());
        assert!(!dir.path().join("not yet").exists());
    }

    #[test]
    fn a_run_needs_one_folder() {
        for whole in [None, Some("/"), Some(""), Some("//")] {
            let e = require_only(whole).unwrap_err().to_string();
            assert!(
                e.contains("explicit approval") && e.contains("--all"),
                "{e}"
            );
        }
        assert_eq!(
            require_only(Some("Mirror Test/Sub/")).unwrap(),
            "/Mirror Test/Sub"
        );
        assert!(require_only(Some("/a/../b")).is_err());
        assert_eq!(scope(None).unwrap(), None);
        assert_eq!(scope(Some("/")).unwrap(), None);
    }

    /// Progress lines carry only counts, and SourceItems lines are throttled.
    #[test]
    fn progress_is_counts_only_and_throttled() {
        let said = Arc::new(Mutex::new(Vec::<String>::new()));
        let line = |every| {
            let said = said.clone();
            ProgressLine::new(
                every,
                Box::new(move |l: &str| said.lock().push(l.to_string())),
            )
        };
        let quiet = line(Duration::from_secs(3600));
        for n in [10, 20, 30] {
            quiet.report(PlanProgress::SourceItems(n));
        }
        quiet.report(PlanProgress::ListingDestination);
        assert_eq!(*said.lock(), ["Listing the destination\u{2026}"]);

        said.lock().clear();
        let every_time = line(Duration::ZERO);
        every_time.report(PlanProgress::SourceItems(12_400));
        every_time.report(PlanProgress::SourceItems(654_321));
        assert_eq!(
            *said.lock(),
            [
                "Listing OneDrive\u{2026} 12,400 items",
                "Listing OneDrive\u{2026} 654,321 items"
            ]
        );
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1000), "1,000");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }

    /// The whole `mirror plan` / `mirror run` path: the OneDrive mock read
    /// through GraphSource with the read-only sign-in, Proton Drive through
    /// ProtonCliDestination with a fake `proton-drive`.
    #[tokio::test]
    async fn plan_and_run_add_only_and_never_write_to_onedrive() {
        let env = Env::new(false).await;
        env.mock.with(|d| d.page_size = 3);
        env.mock.add_folder("/", "Mirror Test");
        env.mock.add_folder("/Mirror Test", "Sub");
        env.mock.add_file("/Mirror Test", "a.txt", b"alpha");
        env.mock.add_file("/Mirror Test/Sub", "b.txt", b"bravo");
        env.mock.add_file("/Mirror Test/Sub", "c.txt", b"charlie");
        env.mock.add_file("/", "outside.txt", b"not mirrored");
        let s = env.settings();
        let fs = ProtonFs::new();
        let dest = ProtonCliDestination::new(s.proton.clone(), &fs).unwrap();
        let source = onedrive_source(&env.paths, &env.cfg).unwrap();
        let only = require_only(Some("/Mirror Test")).unwrap();

        // Dry run: reads only, on both sides.
        let before = fs.snapshot();
        let p = dry_run(
            &source,
            &dest,
            &s.manifest,
            &s.plan_options(Some(&only), false),
        )
        .await
        .unwrap();
        assert!(!p.dest_root_exists);
        assert_eq!(p.count(Action::CreateFolder), 2);
        assert_eq!(p.count(Action::UploadNew), 3);
        assert_eq!(fs.snapshot(), before);
        assert!(fs
            .calls()
            .iter()
            .all(|c| c[0] == "version" || ["list", "info"].contains(&c[1].as_str())));
        env.assert_read_only();
        // Only the folder was listed, not the whole drive's change feed.
        assert_eq!(env.mock.requests("/delta"), 0);
        assert!(p.source_cursor.is_none());

        // Answering no changes nothing.
        let opts = s.plan_options(Some(&only), true);
        let declined = run(
            &source,
            &dest,
            &s.manifest,
            &opts,
            env.exec(&s),
            |_| Ok(false),
            stop(),
        )
        .await
        .unwrap();
        assert!(declined.is_none());
        assert_eq!(fs.snapshot(), before);

        // A real run, verified to the round trip.
        let sum = run(
            &source,
            &dest,
            &s.manifest,
            &opts,
            env.exec(&s),
            |_| Ok(true),
            stop(),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(sum.outcome, RunOutcome::Completed);
        assert_eq!(sum.files_mirrored, 3);
        assert_eq!(
            fs.content("/my-files/OneDrive/Mirror Test/a.txt").unwrap(),
            b"alpha"
        );
        assert_eq!(
            fs.content("/my-files/OneDrive/Mirror Test/Sub/c.txt")
                .unwrap(),
            b"charlie"
        );
        assert!(fs.content("/my-files/OneDrive/outside.txt").is_none());
        let m = Manifest::open(&s.manifest).unwrap();
        let r = Readiness::from_manifest(&m).unwrap();
        assert_eq!(r.measured_round_trip.items, 3);
        drop(m);
        assert!(summary_text(&sum).contains("3 files uploaded"));

        // Edit, rename and delete on OneDrive: Proton keeps everything as it was.
        let mirrored = fs.snapshot();
        env.mock.modify("/Mirror Test/a.txt", b"alpha, edited");
        env.mock.rename("/Mirror Test/Sub/b.txt", "b-renamed.txt");
        env.mock.delete("/Mirror Test/Sub/c.txt");
        let sum = run(
            &source,
            &dest,
            &s.manifest,
            &opts,
            env.exec(&s),
            |p| {
                assert_eq!(p.count(Action::Diverged), 1);
                assert_eq!(p.count(Action::SourceMoved), 1);
                assert_eq!(p.count(Action::SourceDeleted), 1);
                Ok(true)
            },
            stop(),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(sum.files_mirrored, 0);
        assert_eq!(fs.snapshot(), mirrored);
        let states: Vec<ItemState> = Manifest::open(&s.manifest)
            .unwrap()
            .items()
            .unwrap()
            .into_iter()
            .filter(|r| r.source_path.ends_with(".txt"))
            .map(|r| r.state)
            .collect();
        assert!(states.contains(&ItemState::Diverged));
        assert!(states.contains(&ItemState::SourceDeleted));

        assert_allowed(&fs.calls());
        env.assert_read_only();
        // Nothing is left staged.
        let left = std::fs::read_dir(&s.staging_dir).unwrap().count();
        assert_eq!(left, 0);
    }

    /// The sync service owns tokens.json: an expired token here is not
    /// refreshed, the file is not touched, and a token the service renews
    /// meanwhile is picked up.
    #[tokio::test]
    async fn the_onedrive_sign_in_is_only_read() {
        let env = Env::new(true).await;
        env.mock.add_folder("/", "Mirror Test");
        env.mock.add_file("/Mirror Test", "a.txt", b"alpha");
        let s = env.settings();
        let fs = ProtonFs::new();
        let dest = ProtonCliDestination::new(s.proton.clone(), &fs).unwrap();
        let source = onedrive_source(&env.paths, &env.cfg).unwrap();
        let opts = s.plan_options(Some("/Mirror Test"), false);
        let before = std::fs::read(&env.paths.tokens_file).unwrap();

        let e = dry_run(&source, &dest, &s.manifest, &opts)
            .await
            .unwrap_err();
        assert!(e.to_string().contains("sign-in required"), "{e:#}");
        assert_eq!(std::fs::read(&env.paths.tokens_file).unwrap(), before);
        assert!(!env.paths.tokens_file.with_extension("tmp").exists());
        env.assert_read_only();

        // The service renews it: the next request uses the new one.
        std::fs::write(&env.paths.tokens_file, env.mock.tokens_json(false)).unwrap();
        let renewed = std::fs::read(&env.paths.tokens_file).unwrap();
        let p = dry_run(&source, &dest, &s.manifest, &opts).await.unwrap();
        assert_eq!(p.count(Action::UploadNew), 1);
        assert_eq!(std::fs::read(&env.paths.tokens_file).unwrap(), renewed);
        env.assert_read_only();

        // Nor can it be saved over or removed from here.
        let store = TokenStore::open_read_only(&env.paths.tokens_file, env.cfg.auth());
        assert!(store.is_read_only());
        assert!(store.clear().await.is_err());
        assert_eq!(std::fs::read(&env.paths.tokens_file).unwrap(), renewed);
    }
}
