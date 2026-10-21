//! The running tray app: Slint windows on the main thread, a tokio task
//! talking to the sync service over D-Bus, and the tray icon.

use std::cell::Cell;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::Result;
use axentra_onedrive_core::config::{Config, Paths};
use axentra_onedrive_core::db::Activity;
use axentra_onedrive_core::ipc::{AxentraOneDriveProxy, ChangedStream};
use axentra_onedrive_core::notify::Notifier;
use axentra_onedrive_core::status::{Account, Status};
use futures_util::StreamExt;
use ksni::TrayMethods;
use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::winit_030::{winit, EventResult, WinitWindowAccessor};
use slint::{ComponentHandle, Weak};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use crate::tray::AxentraOneDriveTray;
use crate::{present, Flyout, SettingsWindow, Theme};

/// What a refresh fetched from the sync service.
pub struct Snapshot {
    status: Status,
    account: Account,
    /// Only fetched while the panel is open.
    activity: Option<Vec<Activity>>,
    /// Only fetched while Settings is open.
    local_usage: Option<u64>,
}

pub enum Cmd {
    /// From a tray click, with where it was clicked if the tray host said.
    ToggleFlyout(Option<(i32, i32)>),
    ShowFlyout(Option<(i32, i32)>),
    HideFlyout,
    ShowSettings,
    OpenFolder,
    ViewOnline,
    OpenSource,
    RecycleBin,
    ManageStorage,
    Pause(u32),
    Resume,
    TogglePause(bool),
    SignIn,
    Quit,
    OpenItem(String),
    OpenPath(String),
    RevealItem(String),
    ShareItem(String),
    ItemOnline(String),
    SettingsChanged,
    SetOnDemand(bool),
    /// Turning Files On-Demand to `.0` didn't download or free up the
    /// files, so the choice goes back.
    UndoOnDemand(bool),
    FreeUpSpace,
    Unlink,
    SetIconColor(String),
    StartService,
    /// The service asked to start at `.0` stopped again (or couldn't start).
    StartFailed(Instant),
    /// Fetch the service's state again.
    Refresh,
    /// A refresh for connection number `.0` is done; `None` if the service
    /// went away.
    Refreshed(u64, Option<Box<Snapshot>>),
}

pub fn run(start_with_settings: bool, show_panel: bool) -> Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let _guard = rt.enter();

    // One tray app per session: starting it again (from the app menu or
    // Axentra Command) shows the running one's panel or Settings instead.
    let (tx, rx) = unbounded_channel();
    let _instance = match rt.block_on(claim_instance(tx.clone())) {
        Ok(Some(conn)) => Some(conn),
        Ok(None) => {
            let what = if start_with_settings {
                "settings"
            } else if show_panel {
                "panel"
            } else {
                ""
            };
            return rt.block_on(show_in_running(what));
        }
        Err(e) => {
            tracing::warn!("couldn't check for a running Axentra OneDrive: {e:#}");
            None
        }
    };

    let flyout = Flyout::new()?;
    let settings = SettingsWindow::new()?;

    macro_rules! wire {
        ($ui:ident . $on:ident => $cmd:expr) => {{
            let tx = tx.clone();
            $ui.$on(move || {
                let _ = tx.send($cmd);
            });
        }};
        ($ui:ident . $on:ident ($a:ident) => $cmd:expr) => {{
            let tx = tx.clone();
            $ui.$on(move |$a| {
                let _ = tx.send($cmd);
            });
        }};
    }
    wire!(flyout.on_open_folder => Cmd::OpenFolder);
    wire!(flyout.on_view_online => Cmd::ViewOnline);
    wire!(flyout.on_open_settings => Cmd::ShowSettings);
    wire!(flyout.on_recycle_bin => Cmd::RecycleBin);
    wire!(flyout.on_manage_storage => Cmd::ManageStorage);
    wire!(flyout.on_pause(h) => Cmd::Pause(h as u32));
    wire!(flyout.on_resume => Cmd::Resume);
    wire!(flyout.on_sign_in => Cmd::SignIn);
    wire!(flyout.on_start_service => Cmd::StartService);
    wire!(flyout.on_quit => Cmd::Quit);
    wire!(flyout.on_dismiss => Cmd::HideFlyout);
    wire!(flyout.on_open_item(p) => Cmd::OpenItem(p.to_string()));
    wire!(flyout.on_open_path(p) => Cmd::OpenPath(p.to_string()));
    wire!(flyout.on_reveal_item(p) => Cmd::RevealItem(p.to_string()));
    wire!(flyout.on_share_item(p) => Cmd::ShareItem(p.to_string()));
    wire!(flyout.on_item_online(p) => Cmd::ItemOnline(p.to_string()));
    wire!(settings.on_changed => Cmd::SettingsChanged);
    wire!(settings.on_toggle_pause(b) => Cmd::TogglePause(b));
    wire!(settings.on_set_on_demand(b) => Cmd::SetOnDemand(b));
    wire!(settings.on_open_folder => Cmd::OpenFolder);
    wire!(settings.on_free_up_space => Cmd::FreeUpSpace);
    wire!(settings.on_sign_in => Cmd::SignIn);
    wire!(settings.on_unlink => Cmd::Unlink);
    wire!(settings.on_manage_storage => Cmd::ManageStorage);
    wire!(settings.on_view_online => Cmd::ViewOnline);
    wire!(settings.on_open_source => Cmd::OpenSource);
    wire!(settings.on_set_icon_color(c) => Cmd::SetIconColor(c.to_string()));

    close_on_focus_loss(&flyout);
    flyout.set_show_close(!crate::panel::is_x11());
    drop_unshown_windows(flyout.as_weak(), settings.as_weak(), 0);
    if show_panel {
        let _ = tx.send(Cmd::ShowFlyout(None));
    }
    rt.spawn(
        Backend::new(tx.clone(), flyout.as_weak(), settings.as_weak()).run(rx, start_with_settings),
    );
    if let Err(e) = slint::run_event_loop_until_quit() {
        // The display connection closed, normally because the desktop
        // session ended. Leave at once: tearing down after that aborts in
        // the clipboard's thread. Exiting with 1 lets the tray service start
        // it again if the session is still running.
        tracing::warn!("the desktop session went away ({e}); exiting");
        unsafe { libc::_exit(1) };
    }
    rt.shutdown_timeout(Duration::from_secs(1));
    Ok(())
}

const INSTANCE_NAME: &str = "io.github.mecmwithshawn.AxentraOneDrive.Tray";
const INSTANCE_PATH: &str = "/io/github/mecmwithshawn/AxentraOneDrive/Tray";
const INSTANCE_IFACE: &str = "io.github.mecmwithshawn.AxentraOneDrive.Tray1";

/// What a second start of the tray app talks to.
struct Instance {
    tx: UnboundedSender<Cmd>,
}

#[zbus::interface(name = "io.github.mecmwithshawn.AxentraOneDrive.Tray1")]
impl Instance {
    /// Show "panel" or "settings"; anything else does nothing.
    fn show(&self, what: &str) {
        let cmd = match what {
            "panel" => Cmd::ShowFlyout(None),
            "settings" => Cmd::ShowSettings,
            _ => return,
        };
        let _ = self.tx.send(cmd);
    }
}

/// The session-bus connection that makes this the running tray app, or
/// `None` when another one already is.
async fn claim_instance(tx: UnboundedSender<Cmd>) -> Result<Option<zbus::Connection>> {
    // zbus lets a newer requester take a name over by default; refuse that,
    // so a second start finds this one instead of replacing it.
    let built = zbus::connection::Builder::session()?
        .allow_name_replacements(false)
        .replace_existing_names(false)
        .name(INSTANCE_NAME)?
        .serve_at(INSTANCE_PATH, Instance { tx })?
        .build()
        .await;
    match built {
        Ok(conn) => Ok(Some(conn)),
        Err(zbus::Error::NameTaken) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

async fn show_in_running(what: &str) -> Result<()> {
    if what.is_empty() {
        return Ok(());
    }
    let conn = zbus::Connection::session().await?;
    conn.call_method(
        Some(INSTANCE_NAME),
        INSTANCE_PATH,
        Some(INSTANCE_IFACE),
        "Show",
        &(what,),
    )
    .await?;
    Ok(())
}

/// The panel's size in Flyout (logical pixels).
const FLYOUT_SIZE: (f32, f32) = (360.0, 600.0);

thread_local! {
    /// When the panel last closed because it lost focus. Clicking the tray
    /// icon to close it takes the focus first, so that click mustn't reopen it.
    static FOCUS_CLOSED: Cell<Option<Instant>> = const { Cell::new(None) };
    /// Whether the panel has had the focus since it opened. Windows report
    /// losing a focus they never had while they are being created.
    static HAD_FOCUS: Cell<bool> = const { Cell::new(false) };
}

/// Like the Windows panel, close when another window gets the focus.
fn close_on_focus_loss(flyout: &Flyout) {
    let weak = flyout.as_weak();
    flyout.window().on_winit_window_event(move |_, event| {
        if let winit::event::WindowEvent::Focused(true) = event {
            HAD_FOCUS.with(|h| h.set(true));
        }
        if let winit::event::WindowEvent::Focused(false) = event {
            if !HAD_FOCUS.with(|h| h.replace(false)) {
                return EventResult::Propagate;
            }
            let weak = weak.clone();
            slint::Timer::single_shot(Duration::ZERO, move || {
                if let Some(ui) = weak.upgrade() {
                    if ui.window().is_visible() {
                        FOCUS_CLOSED.with(|c| c.set(Some(Instant::now())));
                        let _ = ui.hide();
                    }
                }
            });
        }
        EventResult::Propagate
    });
}

/// Slint creates every window when the event loop starts, hidden. Wayland
/// has no hidden windows, so the panel and Settings would sit in the
/// taskbar as empty windows until first used. Once they exist, show and
/// hide each one that isn't meant to be open in the same turn: on Wayland
/// hiding destroys the window, before it has drawn anything.
fn drop_unshown_windows(flyout: Weak<Flyout>, settings: Weak<SettingsWindow>, tries: u32) {
    if crate::panel::is_x11() {
        return;
    }
    slint::Timer::single_shot(Duration::from_millis(100), move || {
        let (Some(f), Some(s)) = (flyout.upgrade(), settings.upgrade()) else {
            return;
        };
        let exists = |w: &slint::Window| w.with_winit_window(|_| ()).is_some();
        if !(exists(f.window()) && exists(s.window())) {
            if tries < 50 {
                drop_unshown_windows(flyout, settings, tries + 1);
            }
            return;
        }
        for window in [f.window(), s.window()] {
            if !window.is_visible() {
                let _ = window.show();
                let _ = window.hide();
            }
        }
    });
}

fn x11_id(window: &slint::Window) -> Option<u32> {
    window
        .with_winit_window(|w| {
            w.window_handle().ok().and_then(|h| match h.as_raw() {
                RawWindowHandle::Xlib(x) => Some(x.window as u32),
                RawWindowHandle::Xcb(x) => Some(x.window.get()),
                _ => None,
            })
        })
        .flatten()
}

/// Open the panel above the tray icon at `click` (or the screen's
/// bottom-right corner), out of the taskbar, with the keyboard focus.
fn show_flyout(ui: &Flyout, click: Option<(i32, i32)>) {
    HAD_FOCUS.with(|h| h.set(false));
    ui.set_shown(false);
    ui.invoke_reset_focus();
    let window = ui.window();
    let x11 = crate::panel::is_x11();
    let mut spot = None;
    if x11 {
        let sf = window.scale_factor();
        let (w, h) = (
            (FLYOUT_SIZE.0 * sf).round() as i32,
            (FLYOUT_SIZE.1 * sf).round() as i32,
        );
        let anchor = click
            .or_else(|| crate::panel::primary_monitor().map(|m| (m.x + m.w - 1, m.y + m.h - 1)));
        let id = x11_id(window);
        if let Some(anchor) = anchor {
            let (x, y) = crate::panel::place(w, h, anchor, &crate::panel::work_areas());
            window.set_position(slint::PhysicalPosition::new(x, y));
            if let Some(id) = id {
                crate::panel::set_open_position(id, x, y);
            }
            spot = Some((x, y));
        }
        if let Some(id) = id {
            crate::panel::keep_out_of_taskbar(id);
        }
    }
    let _ = ui.show();
    finish_show(ui.as_weak(), x11.then_some(spot).flatten(), x11, 0);
}

/// Once the window manager has mapped the panel: keep it out of the
/// taskbar, move it into place (the first time it was just created where
/// the window manager chose), give it the focus and fade it in.
fn finish_show(weak: Weak<Flyout>, spot: Option<(i32, i32)>, x11: bool, tries: u32) {
    slint::Timer::single_shot(Duration::from_millis(25), move || {
        let Some(ui) = weak.upgrade() else { return };
        if !ui.window().is_visible() {
            return;
        }
        if x11 {
            let id = x11_id(ui.window());
            if !id.is_some_and(crate::panel::is_mapped) && tries < 40 {
                finish_show(weak, spot, x11, tries + 1);
                return;
            }
            if let Some(id) = id {
                crate::panel::keep_out_of_taskbar(id);
            }
            if let Some((x, y)) = spot {
                ui.window().with_winit_window(|w| {
                    w.set_outer_position(winit::dpi::PhysicalPosition::new(x, y));
                });
            }
            if let Some(id) = id {
                crate::panel::activate(id);
            }
        }
        ui.set_shown(true);
    });
}

/// The least time between refreshes that the service's change signal causes.
const REFRESH_EVERY: Duration = Duration::from_secs(1);
/// How long after asking the service to start the panel says it's starting.
const START_GRACE: Duration = Duration::from_secs(30);

struct Backend {
    paths: Paths,
    tx: UnboundedSender<Cmd>,
    flyout: Weak<Flyout>,
    settings: Weak<SettingsWindow>,
    bus: Option<zbus::Connection>,
    proxy: Option<AxentraOneDriveProxy<'static>>,
    changed: Option<ChangedStream>,
    /// Counts connections to the service, so a late refresh from an earlier
    /// one is ignored.
    generation: u64,
    tray: Option<ksni::Handle<AxentraOneDriveTray>>,
    /// The tray's state and tooltip, so it's only updated when they change.
    tray_shows: (String, String),
    /// As last fetched. Its `mount_point` is empty while not connected.
    status: Status,
    account: Account,
    local_usage: Option<u64>,
    /// config.toml as last read successfully.
    config: Config,
    /// Whether the tray app has looked for the service yet. Only that first
    /// look starts it if it isn't running: a service stopped on purpose
    /// stays stopped until the Start button, and systemd restarts one that
    /// crashed.
    looked: bool,
    /// When the service was last asked to start.
    started: Option<Instant>,
    /// Watches that start for the service stopping again at once.
    start_watch: Option<tokio::task::AbortHandle>,
    refreshing: bool,
    /// The service changed since the last refresh began.
    stale: bool,
    /// A command wants the next refresh without the usual wait.
    urgent: bool,
    last_refresh: Instant,
}

async fn next_change(s: &mut Option<ChangedStream>) -> Option<()> {
    match s {
        Some(s) => s.next().await.map(|_| ()),
        None => std::future::pending().await,
    }
}

fn open(target: &str) {
    let _ = Command::new("xdg-open")
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

fn sibling_binary(name: &str) -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(name)))
        .filter(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from(name))
}

/// Run `script` with bash in a new terminal window titled `title`.
fn open_terminal(title: &str, script: &str) -> std::io::Result<()> {
    Command::new("gnome-terminal")
        .args(["--title", title, "--", "bash", "-c", script])
        .spawn()
        .or_else(|_| {
            Command::new("x-terminal-emulator")
                .args(["-e", "bash", "-c", script])
                .spawn()
        })
        .map(|_| ())
}

/// Where the tray's Sign in went on.
#[derive(Debug, PartialEq)]
enum SignIn {
    /// A terminal, which shows whatever goes wrong.
    InTerminal,
    /// Straight to the browser.
    InBrowser,
}

/// Sign in the way Windows OneDrive does, by setting it up again: in a
/// terminal (`terminal(title, script)`), `cli setup` asks for the app
/// registration, where Enter keeps the one there is, so one pasted wrongly
/// (Microsoft's sign-in page then never comes back) can be replaced; then
/// `cli login` opens the browser. With no terminal to open, `login` signs
/// in with the app registration there is, if it was `registered`.
fn sign_in<T, L>(cli: &Path, registered: bool, terminal: T, login: L) -> Result<SignIn>
where
    T: FnOnce(&str, &str) -> std::io::Result<()>,
    L: FnOnce(),
{
    let script = format!(
        "{0} setup && {0} login; echo; read -p 'Press Enter to close' _",
        cli.display()
    );
    if terminal("Set up Axentra OneDrive", &script).is_ok() {
        return Ok(SignIn::InTerminal);
    }
    if !registered {
        anyhow::bail!(
            "Couldn't open a terminal. Run `axentra-onedrive setup` in one to set up Axentra OneDrive."
        );
    }
    login();
    Ok(SignIn::InBrowser)
}

/// The sync service's systemd user unit.
const SERVICE_UNIT: &str = "axentra-onedrive.service";

/// `systemctl --user <args>`: what it printed, or what it said went wrong.
async fn systemctl(args: &[&str]) -> Result<String> {
    let out = tokio::process::Command::new("systemctl")
        .arg("--user")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .await?;
    let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).trim().to_string();
    if !out.status.success() {
        anyhow::bail!("{}", text(&out.stderr));
    }
    Ok(text(&out.stdout))
}

/// Whether a unit's LoadState says it's installed (masked included: then
/// it's meant not to run).
fn unit_found(load_state: &str) -> bool {
    !load_state.is_empty() && load_state != "not-found"
}

/// How asking the service to start went.
#[derive(Debug, PartialEq)]
enum Started {
    Running,
    /// It stopped again; the service says why itself, in a notification.
    Stopped,
    /// It couldn't be started, for this reason.
    Refused(String),
}

/// Start the service: through systemd, or by running it directly
/// (`run_directly`, which gives what ends when it exits) when its unit
/// isn't installed or there's no systemd to ask. `asked`: the user clicked
/// Start, so an earlier failure is cleared first, as `axentra-onedrive`
/// advises; systemd's start limit counts failed starts too and would refuse
/// it. `systemctl` runs `systemctl --user` (tests stand in for it).
async fn start<S, F, D, E>(asked: bool, at: Instant, systemctl: S, run_directly: D) -> Started
where
    S: Fn(&'static [&'static str]) -> F,
    F: Future<Output = Result<String>>,
    D: FnOnce() -> std::io::Result<E>,
    E: Future<Output = ()>,
{
    if asked {
        let _ = systemctl(&["reset-failed", SERVICE_UNIT]).await;
    }
    let refused = match systemctl(&["start", SERVICE_UNIT]).await {
        Ok(_) if stops_soon(at, &systemctl).await => return Started::Stopped,
        Ok(_) => return Started::Running,
        Err(e) => e,
    };
    // Installed but refused (masked, say): it isn't to be run another way.
    let load_state = systemctl(&["show", "--property=LoadState", "--value", SERVICE_UNIT]).await;
    if load_state.is_ok_and(|s| unit_found(&s)) {
        return Started::Refused(refused.to_string());
    }
    match run_directly() {
        Ok(exited) => match tokio::time::timeout(START_GRACE, exited).await {
            Ok(()) => Started::Stopped,
            Err(_) => Started::Running,
        },
        Err(e) => Started::Refused(e.to_string()),
    }
}

/// Whether the unit, started at `at`, stops again within `START_GRACE`.
/// It counts as started once it runs (Type=simple), and one with a problem
/// only the user can fix exits at once, saying why in a notification.
async fn stops_soon<S, F>(at: Instant, systemctl: &S) -> bool
where
    S: Fn(&'static [&'static str]) -> F,
    F: Future<Output = Result<String>>,
{
    while at.elapsed() < START_GRACE {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let state = systemctl(&["show", "--property=ActiveState", "--value", SERVICE_UNIT]).await;
        if matches!(state.as_deref(), Ok("failed" | "inactive")) {
            return true;
        }
    }
    false
}

fn clipboard() -> &'static Mutex<Option<arboard::Clipboard>> {
    static CB: OnceLock<Mutex<Option<arboard::Clipboard>>> = OnceLock::new();
    CB.get_or_init(|| Mutex::new(arboard::Clipboard::new().ok()))
}

fn notifier() -> &'static Notifier {
    static N: OnceLock<Notifier> = OnceLock::new();
    N.get_or_init(|| Notifier::new(true))
}

async fn show_in_file_manager(path: String) -> Result<()> {
    let uri = url::Url::from_file_path(&path)
        .map(|u| u.to_string())
        .unwrap_or(path);
    let conn = zbus::Connection::session().await?;
    conn.call_method(
        Some("org.freedesktop.FileManager1"),
        "/org/freedesktop/FileManager1",
        Some("org.freedesktop.FileManager1"),
        "ShowItems",
        &(vec![uri], ""),
    )
    .await?;
    Ok(())
}

/// Shows what came of a command where the user is looking: in the panel if
/// it's open, else in Settings if that is, else as a desktop notification.
#[derive(Clone)]
struct Reporter {
    flyout: Weak<Flyout>,
    settings: Weak<SettingsWindow>,
}

impl Reporter {
    fn show(&self, text: String, error: bool) {
        let (flyout, settings) = (self.flyout.clone(), self.settings.clone());
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = flyout.upgrade().filter(|ui| ui.window().is_visible()) {
                ui.set_toast(text.clone().into());
                let weak = ui.as_weak();
                slint::Timer::single_shot(Duration::from_secs(4), move || {
                    if let Some(ui) = weak.upgrade() {
                        if ui.get_toast() == text.as_str() {
                            ui.set_toast("".into());
                        }
                    }
                });
            } else if let Some(ui) = settings.upgrade().filter(|ui| ui.window().is_visible()) {
                ui.set_message(text.into());
                ui.set_message_error(error);
            } else {
                tokio::spawn(async move { notifier().send(&text, "").await });
            }
        });
    }

    fn error(&self, e: &anyhow::Error) {
        tracing::warn!("{e:#}");
        self.show(present::error_text(e), true);
    }
}

impl Backend {
    fn new(tx: UnboundedSender<Cmd>, flyout: Weak<Flyout>, settings: Weak<SettingsWindow>) -> Self {
        Self {
            paths: Paths::user(),
            tx,
            flyout,
            settings,
            bus: None,
            proxy: None,
            changed: None,
            generation: 0,
            tray: None,
            tray_shows: Default::default(),
            status: Status::default(),
            account: Account::default(),
            local_usage: None,
            config: Config::default(),
            looked: false,
            started: None,
            start_watch: None,
            refreshing: false,
            stale: false,
            urgent: false,
            last_refresh: Instant::now(),
        }
    }

    fn reporter(&self) -> Reporter {
        Reporter {
            flyout: self.flyout.clone(),
            settings: self.settings.clone(),
        }
    }

    /// Show `color` in both windows and the tray, and in the desktop icon theme.
    async fn show_icon_color(&self, color: &str) {
        let c = color.to_string();
        let _ = self.flyout.upgrade_in_event_loop({
            let c = c.clone();
            move |ui| ui.global::<Theme>().set_icon_color(c.into())
        });
        let _ = self
            .settings
            .upgrade_in_event_loop(move |ui| ui.global::<Theme>().set_icon_color(c.into()));
        if let Some(tray) = &self.tray {
            let accent = axentra_onedrive_core::config::icon_accent(color);
            tray.update(move |t| t.accent = accent).await;
        }
        // Writing the icon theme takes a moment; nothing waits for it.
        let c = color.to_string();
        tokio::task::spawn_blocking(move || {
            if let Err(e) = crate::icons::apply(&c) {
                tracing::warn!("couldn't update the desktop icon: {e}");
            }
        });
    }

    /// Read config.toml, remembering it if it could be read.
    fn load_config(&mut self) -> Result<Config> {
        let cfg = Config::load(&self.paths.config_file)?;
        self.config = cfg.clone();
        Ok(cfg)
    }

    /// config.toml, or the last version of it that could be read: a hand
    /// edit with a mistake in it mustn't move the OneDrive folder.
    fn config(&mut self) -> Config {
        self.load_config().unwrap_or_else(|e| {
            tracing::warn!("{e:#}");
            self.config.clone()
        })
    }

    /// config.toml, to change and save. Writing defaults over a file that
    /// couldn't be read would lose the app registration, folder and ignore
    /// list in it, so that's an error to show instead.
    fn config_to_change(&mut self) -> Result<Config> {
        self.load_config()
            .map_err(|e| anyhow::anyhow!(present::config_problem(&e)))
    }

    async fn run(mut self, mut rx: UnboundedReceiver<Cmd>, start_with_settings: bool) {
        let color = self.config().icon_color().to_string();
        let tray = AxentraOneDriveTray {
            state: "starting".into(),
            tooltip: "Axentra OneDrive".into(),
            accent: axentra_onedrive_core::config::icon_accent(&color),
            tx: self.tx.clone(),
        };
        self.show_icon_color(&color).await;
        match tray.spawn().await {
            Ok(h) => self.tray = Some(h),
            Err(e) => {
                // No tray host (e.g. GNOME without the AppIndicator extension):
                // the flyout is the main window then.
                tracing::warn!("no system tray available ({e}); showing the panel instead");
                let _ = self.tx.send(Cmd::ShowFlyout(None));
            }
        }
        if start_with_settings {
            let _ = self.tx.send(Cmd::ShowSettings);
        }
        // Every command is quick here: calls to the service that can take a
        // while run in tasks of their own, so the tray and the windows never
        // wait behind them.
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        loop {
            if self.proxy.is_none() {
                self.connect().await;
            }
            let due = self.stale && !self.refreshing && self.proxy.is_some();
            let at = if self.urgent {
                Instant::now()
            } else {
                self.last_refresh + REFRESH_EVERY
            };
            // Look for the service often while it's starting.
            let starting =
                self.proxy.is_none() && self.started.is_some_and(|t| t.elapsed() < START_GRACE);
            tokio::select! {
                cmd = rx.recv() => match cmd {
                    Some(cmd) => {
                        if let Err(e) = self.handle(cmd).await {
                            self.reporter().error(&e);
                        }
                    }
                    None => break,
                },
                Some(()) = next_change(&mut self.changed) => self.stale = true,
                _ = tick.tick() => self.stale = true,
                _ = tokio::time::sleep_until(at.into()), if due => self.start_refresh(),
                _ = tokio::time::sleep(Duration::from_millis(500)), if starting => {}
            }
        }
    }

    async fn connect(&mut self) {
        if self.bus.is_none() {
            self.bus = zbus::Connection::session().await.ok();
        }
        let Some(bus) = self.bus.clone() else { return };
        let found = match AxentraOneDriveProxy::new(&bus).await {
            Ok(p) if p.status().await.is_ok() => Some(p),
            _ => None,
        };
        if self.should_auto_start(found.is_some()) {
            self.start_service(false);
        }
        let Some(p) = found else {
            self.show_disconnected().await;
            return;
        };
        self.changed = p.receive_changed().await.ok();
        self.proxy = Some(p);
        self.generation += 1;
        self.started = None;
        if let Some(watch) = self.start_watch.take() {
            watch.abort();
        }
        self.refreshing = false;
        self.start_refresh();
    }

    /// Whether to start the service, given whether it was `found` running:
    /// only at the tray app's first look for it, whatever that finds.
    fn should_auto_start(&mut self, found: bool) -> bool {
        !std::mem::replace(&mut self.looked, true) && !found
    }

    /// Start the service (see [`start`]); `asked`: the user clicked Start.
    fn start_service(&mut self, asked: bool) {
        let at = Instant::now();
        self.started = Some(at);
        let (tx, report) = (self.tx.clone(), self.reporter());
        let watch = tokio::spawn(async move {
            // Without the unit (or systemd), it runs here: its log goes
            // where the tray app's does, and the runtime reaps it.
            let run_directly = || {
                let mut daemon = tokio::process::Command::new(sibling_binary("axentra-onedrive"))
                    .arg("daemon")
                    .stdin(Stdio::null())
                    .spawn()?;
                Ok(async move {
                    let _ = daemon.wait().await;
                })
            };
            let started = start(asked, at, systemctl, run_directly).await;
            if let (true, Started::Refused(why)) = (asked, &started) {
                report.error(&anyhow::anyhow!("Couldn't start Axentra OneDrive: {why}"));
            }
            if started != Started::Running {
                let _ = tx.send(Cmd::StartFailed(at));
            }
        });
        if let Some(old) = self.start_watch.replace(watch.abort_handle()) {
            old.abort();
        }
    }

    /// No service: "Starting" just after asking it to start, otherwise that
    /// it isn't running, with a button to start it.
    async fn show_disconnected(&mut self) {
        let starting = self.started.is_some_and(|t| t.elapsed() < START_GRACE);
        let st = if starting {
            Status {
                state: "starting".into(),
                message: "Starting Axentra OneDrive".into(),
                detail: "Waiting for the sync service to start".into(),
                ..Default::default()
            }
        } else {
            Status {
                state: "stopped".into(),
                message: present::NOT_RUNNING.into(),
                detail: "Your files aren't syncing until you start it again.".into(),
                ..Default::default()
            }
        };
        self.update_tray(&st.state, st.message.clone()).await;
        self.status = st.clone();
        let acct = self.account.clone();
        let _ = self
            .flyout
            .upgrade_in_event_loop(move |ui| present::apply_flyout(&ui, &st, &acct, Some(&[])));
    }

    async fn update_tray(&mut self, state: &str, tooltip: String) {
        if self.tray_shows.0 == state && self.tray_shows.1 == tooltip {
            return;
        }
        self.tray_shows = (state.to_string(), tooltip.clone());
        if let Some(tray) = &self.tray {
            let state = state.to_string();
            tray.update(move |t| {
                t.state = state;
                t.tooltip = tooltip;
            })
            .await;
        }
    }

    fn proxy(&self) -> Result<&AxentraOneDriveProxy<'static>> {
        self.proxy
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!(present::NOT_RUNNING))
    }

    /// Whether the panel and Settings are open, asked on the UI thread.
    fn open_windows(&self) -> impl Future<Output = (bool, bool)> + Send + 'static {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let (flyout, settings) = (self.flyout.clone(), self.settings.clone());
        let _ = slint::invoke_from_event_loop(move || {
            let _ = tx.send((
                flyout.upgrade().is_some_and(|ui| ui.window().is_visible()),
                settings
                    .upgrade()
                    .is_some_and(|ui| ui.window().is_visible()),
            ));
        });
        async move { rx.await.unwrap_or((true, true)) }
    }

    /// Refresh now, or right after the refresh that is running.
    fn refresh(&mut self) {
        if self.refreshing {
            self.stale = true;
            self.urgent = true;
        } else {
            self.start_refresh();
        }
    }

    /// Fetch the service's state in a task of its own; `Cmd::Refreshed`
    /// brings it back. Each fetch costs the service scans of its database,
    /// so the recent files are only fetched while the panel is open and the
    /// space used on this device while Settings is.
    fn start_refresh(&mut self) {
        let Some(p) = self.proxy.clone() else { return };
        self.refreshing = true;
        self.stale = false;
        self.urgent = false;
        self.last_refresh = Instant::now();
        let (generation, tx, open) = (self.generation, self.tx.clone(), self.open_windows());
        tokio::spawn(async move {
            let (panel, settings) = open.await;
            let fetched = async {
                let status: Status = serde_json::from_str(&p.status().await?).unwrap_or_default();
                let account: Account =
                    serde_json::from_str(&p.account().await?).unwrap_or_default();
                let activity = match panel {
                    true => Some(serde_json::from_str(&p.activity(40).await?).unwrap_or_default()),
                    false => None,
                };
                let local_usage = match settings {
                    true => Some(p.local_usage().await?),
                    false => None,
                };
                Ok::<_, zbus::Error>(Snapshot {
                    status,
                    account,
                    activity,
                    local_usage,
                })
            }
            .await;
            let _ = tx.send(Cmd::Refreshed(generation, fetched.ok().map(Box::new)));
        });
    }

    async fn show(&mut self, s: Snapshot) {
        let tip = if s.status.detail.is_empty() {
            s.status.message.clone()
        } else {
            format!("{}\n{}", s.status.message, s.status.detail)
        };
        self.update_tray(&s.status.state, tip).await;
        self.status = s.status.clone();
        self.account = s.account.clone();
        let (st, acct, activity) = (s.status.clone(), s.account.clone(), s.activity);
        let _ = self.flyout.upgrade_in_event_loop(move |ui| {
            present::apply_flyout(&ui, &st, &acct, activity.as_deref())
        });
        if let Some(usage) = s.local_usage {
            self.local_usage = Some(usage);
            // Read again, quietly keeping the last good copy: `setup --folder`
            // may have moved the folder while Settings is open, and the
            // service restarted into it since.
            let cfg = self.load_config().unwrap_or_else(|_| self.config.clone());
            let (st, acct) = (s.status, s.account);
            let _ = self.settings.upgrade_in_event_loop(move |ui| {
                present::apply_live(&ui, &cfg, &st, &acct, Some(usage))
            });
        }
    }

    /// Run a call to the service in a task of its own, then show what came
    /// of it (a message on success, if `call` has one) and refresh.
    fn spawn_call<F, Fut>(&self, call: F) -> Result<()>
    where
        F: FnOnce(AxentraOneDriveProxy<'static>) -> Fut,
        Fut: Future<Output = Result<Option<String>>> + Send + 'static,
    {
        let done = call(self.proxy()?.clone());
        let (report, tx) = (self.reporter(), self.tx.clone());
        tokio::spawn(async move {
            match done.await {
                Ok(Some(text)) => report.show(text, false),
                Ok(None) => {}
                Err(e) => report.error(&e),
            }
            let _ = tx.send(Cmd::Refresh);
        });
        Ok(())
    }

    /// Run `f` on Settings on the UI thread and wait for what it returns.
    async fn with_settings<R: Send + 'static>(
        &self,
        f: impl FnOnce(&SettingsWindow) -> R + Send + 'static,
    ) -> Result<R> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.settings.upgrade_in_event_loop(move |ui| {
            let _ = tx.send(f(&ui));
        })?;
        Ok(rx.await?)
    }

    /// Show `cfg` in Settings, with the live values known so far.
    fn show_settings_from(&self, cfg: Config) {
        let (st, acct, usage) = (self.status.clone(), self.account.clone(), self.local_usage);
        let _ = self
            .settings
            .upgrade_in_event_loop(move |ui| present::apply_settings(&ui, &cfg, &st, &acct, usage));
    }

    /// Save what Settings shows to config.toml and have the service apply
    /// it. Turning Files On-Demand off downloads every file; turning it back
    /// on frees up the space they take.
    async fn save_settings(&mut self, on_demand: Option<bool>) -> Result<()> {
        let mut cfg = match self.config_to_change() {
            Ok(cfg) => cfg,
            Err(e) => {
                // Nothing was saved, so put the controls back.
                self.show_settings_from(self.config.clone());
                return Err(e);
            }
        };
        // Saving the choice without the service to download or free up the
        // files would leave Settings showing what didn't happen.
        if on_demand.is_some() {
            if let Err(e) = self.proxy() {
                self.show_settings_from(self.config.clone());
                return Err(e);
            }
        }
        let cfg = self
            .with_settings(move |ui| {
                present::read_settings(ui, &mut cfg);
                cfg
            })
            .await?;
        cfg.save(&self.paths.config_file)?;
        self.config = cfg;
        let (mount, tx) = (self.mount(), self.tx.clone());
        self.spawn_call(move |p| async move {
            let applied = async {
                p.reload().await?;
                Ok::<_, zbus::Error>(match on_demand {
                    Some(false) => {
                        p.pin(vec![mount]).await?;
                        None
                    }
                    Some(true) => present::kept_note(p.free_up_space(vec![mount]).await?),
                    None => None,
                })
            }
            .await;
            if let (Err(_), Some(on)) = (&applied, on_demand) {
                let _ = tx.send(Cmd::UndoOnDemand(on));
            }
            Ok(applied?)
        })
    }

    /// Put Files On-Demand back as it was before turning it to `on` failed,
    /// unless it has been changed again since.
    fn undo_on_demand(&mut self, on: bool) -> Result<()> {
        let mut cfg = self.config_to_change()?;
        if cfg.files_on_demand == on {
            cfg.files_on_demand = !on;
            cfg.save(&self.paths.config_file)?;
            self.config = cfg.clone();
            if let Some(p) = self.proxy.clone() {
                tokio::spawn(async move {
                    let _ = p.reload().await;
                });
            }
        }
        self.show_settings_from(cfg);
        Ok(())
    }

    fn hide_flyout(&self) {
        let _ = self.flyout.upgrade_in_event_loop(|ui| {
            let _ = ui.hide();
        });
    }

    /// The OneDrive folder. A new location in config.toml only takes effect
    /// when the service restarts, so while it runs it's where the service
    /// has it, and otherwise where config.toml puts it.
    fn mount(&mut self) -> String {
        if !self.status.mount_point.is_empty() {
            return self.status.mount_point.clone();
        }
        self.config().mount_path().display().to_string()
    }

    fn recycle_bin_url(&self) -> String {
        if self.account.drive_type == "business" && !self.account.web_url.is_empty() {
            let site = self
                .account
                .web_url
                .trim_end_matches('/')
                .trim_end_matches("/Documents");
            format!("{site}/_layouts/15/RecycleBin.aspx")
        } else {
            "https://onedrive.live.com/recycle".into()
        }
    }

    async fn handle(&mut self, cmd: Cmd) -> Result<()> {
        match cmd {
            Cmd::ToggleFlyout(click) | Cmd::ShowFlyout(click) => {
                let toggle = matches!(cmd, Cmd::ToggleFlyout(_));
                let _ = self.flyout.upgrade_in_event_loop(move |ui| {
                    let just_closed = FOCUS_CLOSED
                        .with(|c| c.get())
                        .is_some_and(|t| t.elapsed() < Duration::from_millis(400));
                    if toggle && (ui.window().is_visible() || just_closed) {
                        let _ = ui.hide();
                    } else {
                        show_flyout(&ui, click);
                    }
                });
                self.refresh();
            }
            Cmd::HideFlyout => self.hide_flyout(),
            Cmd::ShowSettings => {
                let loaded = self.load_config();
                let problem = loaded.as_ref().err().map(present::config_problem);
                let cfg = loaded.unwrap_or_else(|_| self.config.clone());
                self.hide_flyout();
                let (st, acct, usage) =
                    (self.status.clone(), self.account.clone(), self.local_usage);
                let _ = self.settings.upgrade_in_event_loop(move |ui| {
                    let window = ui.window();
                    // Already open, perhaps behind other windows: bring it to
                    // the front. Wayland only gives the focus to a new window.
                    let raise = window.is_visible();
                    let x11 = crate::panel::is_x11();
                    if raise && !x11 {
                        let _ = window.hide();
                    }
                    present::apply_settings(&ui, &cfg, &st, &acct, usage);
                    ui.set_message(problem.unwrap_or_default().into());
                    ui.set_message_error(true);
                    let _ = ui.show();
                    if raise && x11 {
                        window.with_winit_window(|w| w.set_minimized(false));
                        if let Some(id) = x11_id(window) {
                            crate::panel::activate(id);
                        }
                    }
                });
                self.refresh();
            }
            Cmd::OpenFolder => {
                self.hide_flyout();
                open(&self.mount());
            }
            Cmd::ViewOnline => {
                self.hide_flyout();
                let (mount, proxy) = (self.mount(), self.proxy.clone());
                tokio::spawn(async move {
                    let url = match proxy {
                        Some(p) => p.web_url(mount).await.ok(),
                        None => None,
                    };
                    open(&url.unwrap_or_else(|| "https://onedrive.live.com/my".into()));
                });
            }
            Cmd::OpenSource => {
                open("https://github.com/MECMwithShawn/Axentra-Software/tree/main/OneDrive")
            }
            Cmd::RecycleBin => {
                self.hide_flyout();
                open(&self.recycle_bin_url());
            }
            Cmd::ManageStorage => {
                self.hide_flyout();
                open(if self.account.drive_type == "business" {
                    "https://www.microsoft365.com/"
                } else {
                    "https://onedrive.live.com/?v=managestorage"
                });
            }
            Cmd::Pause(h) => self.spawn_call(move |p| async move {
                p.pause(h * 3600).await?;
                Ok(None)
            })?,
            Cmd::Resume | Cmd::TogglePause(false) => self.spawn_call(|p| async move {
                p.resume().await?;
                Ok(None)
            })?,
            Cmd::TogglePause(true) => self.spawn_call(|p| async move {
                p.pause(2 * 3600).await?;
                Ok(None)
            })?,
            Cmd::SignIn => {
                let cli = sibling_binary("axentra-onedrive");
                let registered = !self.config().client_id.is_empty();
                let login = || {
                    let _ = Command::new(&cli)
                        .arg("login")
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .spawn();
                };
                if sign_in(&cli, registered, open_terminal, login)? == SignIn::InBrowser {
                    self.reporter()
                        .show("Continue in your browser to sign in".into(), false);
                }
            }
            Cmd::Quit => {
                if let Ok(p) = self.proxy() {
                    let _ = p.quit().await;
                }
                if let Some(t) = &self.tray {
                    t.shutdown().await;
                }
                let _ = slint::invoke_from_event_loop(|| {
                    let _ = slint::quit_event_loop();
                });
            }
            Cmd::OpenItem(path) | Cmd::OpenPath(path) => {
                self.hide_flyout();
                open(&path);
            }
            Cmd::RevealItem(path) => {
                self.hide_flyout();
                let report = self.reporter();
                tokio::spawn(async move {
                    if let Err(e) = show_in_file_manager(path).await {
                        report.error(&e);
                    }
                });
            }
            Cmd::ShareItem(path) => self.spawn_call(move |p| async move {
                let link = p.share_link(path, false).await?;
                let copied = clipboard()
                    .lock()
                    .ok()
                    .and_then(|mut c| c.as_mut().map(|c| c.set_text(link.clone()).is_ok()))
                    .unwrap_or(false);
                Ok(Some(if copied {
                    "Link copied. Anyone with the link can view.".into()
                } else {
                    link
                }))
            })?,
            Cmd::ItemOnline(path) => {
                self.hide_flyout();
                self.spawn_call(move |p| async move {
                    open(&p.web_url(path).await?);
                    Ok(None)
                })?;
            }
            Cmd::SettingsChanged => self.save_settings(None).await?,
            Cmd::SetOnDemand(on) => self.save_settings(Some(on)).await?,
            Cmd::UndoOnDemand(on) => self.undo_on_demand(on)?,
            Cmd::SetIconColor(color) => {
                if !axentra_onedrive_core::config::ICON_COLORS.contains(&color.as_str()) {
                    anyhow::bail!("unknown icon colour {color}");
                }
                let mut cfg = self.config_to_change()?;
                cfg.icon_color = color.clone();
                cfg.save(&self.paths.config_file)?;
                self.config = cfg;
                self.show_icon_color(&color).await;
            }
            Cmd::FreeUpSpace => {
                let mount = self.mount();
                self.spawn_call(move |p| async move {
                    Ok(present::kept_note(p.free_up_space(vec![mount]).await?))
                })?;
            }
            Cmd::Unlink => self.spawn_call(|p| async move {
                p.sign_out().await?;
                Ok(None)
            })?,
            Cmd::StartService => {
                self.start_service(true);
                self.show_disconnected().await;
            }
            Cmd::StartFailed(at) => {
                // Not "Starting" any more: it isn't running, and says why.
                if self.proxy.is_none() && self.started == Some(at) {
                    self.started = None;
                    self.show_disconnected().await;
                }
            }
            Cmd::Refresh => self.refresh(),
            Cmd::Refreshed(generation, snapshot) => {
                if generation != self.generation {
                    return Ok(());
                }
                self.refreshing = false;
                match snapshot {
                    Some(s) => self.show(*s).await,
                    None => {
                        // The service went away. It isn't started again
                        // here: it was stopped on purpose, or it crashed
                        // and systemd restarts it.
                        self.proxy = None;
                        self.changed = None;
                        self.show_disconnected().await;
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn backend(paths: Paths) -> Backend {
        crate::shot::headless();
        let (tx, _rx) = unbounded_channel();
        let mut backend = Backend::new(tx, Weak::default(), Weak::default());
        backend.paths = paths;
        backend
    }

    fn block_on<T>(f: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(f)
    }

    #[test]
    fn a_config_with_a_mistake_is_left_alone() {
        let dir = std::env::temp_dir().join(format!("axentra-ui-app-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let paths = Paths::under(&dir);
        let text = "client_id = \"my-app\"\nmount_point = \"/data/OneDrive\"\nignore = \"*.tmp\"\n";
        std::fs::write(&paths.config_file, text).unwrap();
        let mut backend = backend(paths.clone());
        for cmd in [
            Cmd::SettingsChanged,
            Cmd::SetOnDemand(false),
            Cmd::SetIconColor("azure".into()),
        ] {
            let e = block_on(backend.handle(cmd)).unwrap_err();
            assert!(e.to_string().contains("mistake on line 3"), "{e}");
        }
        assert_eq!(std::fs::read_to_string(&paths.config_file).unwrap(), text);
        // What the app goes on using is the last config it could read.
        assert_eq!(backend.config().client_id, "");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn files_on_demand_waits_for_the_service() {
        let dir = std::env::temp_dir().join(format!("axentra-ui-fod-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let paths = Paths::under(&dir);
        let text = "client_id = \"my-app\"\nfiles_on_demand = false\n";
        std::fs::write(&paths.config_file, text).unwrap();
        let mut backend = backend(paths.clone());
        // Nothing could be freed up without the service, so nothing is saved.
        for on in [true, false] {
            let e = block_on(backend.handle(Cmd::SetOnDemand(on))).unwrap_err();
            assert_eq!(e.to_string(), present::NOT_RUNNING);
        }
        assert_eq!(std::fs::read_to_string(&paths.config_file).unwrap(), text);
        assert!(!backend.config.files_on_demand);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_stopped_service_stays_stopped() {
        for running_at_first in [true, false] {
            let mut backend = backend(Paths::under(&std::env::temp_dir()));
            // The tray app's first look: started only if it wasn't running.
            assert_eq!(
                backend.should_auto_start(running_at_first),
                !running_at_first
            );
            // Then stopped by the user.
            let generation = backend.generation;
            block_on(backend.handle(Cmd::Refreshed(generation, None))).unwrap();
            assert!(backend.proxy.is_none());
            assert!(backend.started.is_none());
            assert_eq!(backend.status.state, "stopped");
            assert_eq!(backend.status.message, present::NOT_RUNNING);
            // Looking for it again doesn't start it.
            assert!(!backend.should_auto_start(false));
        }
    }

    #[test]
    fn a_start_that_fails_stops_saying_starting() {
        let mut backend = backend(Paths::under(&std::env::temp_dir()));
        let at = Instant::now();
        backend.started = Some(at);
        block_on(backend.show_disconnected());
        assert_eq!(backend.status.state, "starting");
        // About a start asked for before this one: nothing changes.
        let earlier = at - Duration::from_secs(1);
        block_on(backend.handle(Cmd::StartFailed(earlier))).unwrap();
        assert_eq!(backend.status.state, "starting");
        block_on(backend.handle(Cmd::StartFailed(at))).unwrap();
        assert!(backend.started.is_none());
        assert_eq!(backend.status.state, "stopped");
    }

    /// `start` with systemctl answering each command from `answers` (by an
    /// argument in it; the rest succeed, saying nothing): how it went, the
    /// commands it ran, and whether it ran the service directly.
    fn start_with(
        asked: bool,
        answers: &[(&str, Result<&str, &str>)],
    ) -> (Started, Vec<String>, bool) {
        let ran = std::cell::RefCell::new(Vec::new());
        let systemctl = |args: &'static [&'static str]| {
            ran.borrow_mut().push(args.join(" "));
            let answer = answers
                .iter()
                .find(|(arg, _)| args.contains(arg))
                .map_or(Ok(""), |(_, a)| *a);
            async move {
                answer
                    .map(str::to_string)
                    .map_err(|e| anyhow::anyhow!("{e}"))
            }
        };
        let directly = Cell::new(false);
        let run_directly = || {
            directly.set(true);
            Ok(std::future::ready(()))
        };
        let started = block_on(start(asked, Instant::now(), systemctl, run_directly));
        (started, ran.into_inner(), directly.get())
    }

    #[test]
    fn the_service_is_run_directly_only_without_its_unit() {
        const RESET: &str = "reset-failed axentra-onedrive.service";
        const START: &str = "start axentra-onedrive.service";
        const LOAD_STATE: &str = "show --property=LoadState --value axentra-onedrive.service";
        const REFUSED: Result<&str, &str> = Err("Unit axentra-onedrive.service is masked.");
        for (asked, load_state) in [(true, "masked"), (false, "loaded")] {
            let answers = [("start", REFUSED), ("--property=LoadState", Ok(load_state))];
            let (started, ran, directly) = start_with(asked, &answers);
            // Clicking Start clears an earlier failure first.
            let expected = if asked {
                vec![RESET, START, LOAD_STATE]
            } else {
                vec![START, LOAD_STATE]
            };
            assert_eq!(ran, expected);
            // Installed but refused: said why, not run another way.
            assert_eq!(
                started,
                Started::Refused("Unit axentra-onedrive.service is masked.".into())
            );
            assert!(!directly);
        }
        // Not installed, or no systemd to ask: run here.
        for load_state in [Ok("not-found"), Ok(""), Err("Failed to connect to bus")] {
            let answers = [("start", REFUSED), ("--property=LoadState", load_state)];
            let (started, _, directly) = start_with(true, &answers);
            assert!(directly);
            // It exited at once.
            assert_eq!(started, Started::Stopped);
        }
        // Started by systemd, then stopped again.
        let answers = [("--property=ActiveState", Ok("failed"))];
        let (started, ran, directly) = start_with(false, &answers);
        assert_eq!(started, Started::Stopped);
        assert_eq!(ran[0], START);
        assert!(!directly);
    }

    #[test]
    fn the_folder_is_where_the_service_has_it() {
        let dir = std::env::temp_dir().join(format!("axentra-ui-mount-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let paths = Paths::under(&dir);
        // Moved in config.toml; the service keeps the old one until it restarts.
        std::fs::write(&paths.config_file, "mount_point = \"/data/New\"\n").unwrap();
        let mut backend = backend(paths);
        backend.status.mount_point = "/data/Old".into();
        assert_eq!(backend.mount(), "/data/Old");
        // Not running: where it will be.
        let generation = backend.generation;
        block_on(backend.handle(Cmd::Refreshed(generation, None))).unwrap();
        assert_eq!(backend.mount(), "/data/New");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn settings_follow_the_folder_moved_while_open() {
        let dir = std::env::temp_dir().join(format!("axentra-ui-moved-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let paths = Paths::under(&dir);
        std::fs::write(&paths.config_file, "mount_point = \"/data/Old\"\n").unwrap();
        let mut backend = backend(paths.clone());
        // Settings opens.
        block_on(backend.handle(Cmd::ShowSettings)).unwrap();
        let refreshed = |mount: &str| {
            Box::new(Snapshot {
                status: Status {
                    mount_point: mount.into(),
                    ..Default::default()
                },
                account: Account::default(),
                activity: None,
                local_usage: Some(0),
            })
        };
        // `setup --folder /data/New`, then the service restarts into it.
        std::fs::write(&paths.config_file, "mount_point = \"/data/New\"\n").unwrap();
        let generation = backend.generation;
        block_on(backend.handle(Cmd::Refreshed(generation, Some(refreshed("/data/Old"))))).unwrap();
        assert_eq!(backend.config.mount_path(), Path::new("/data/New"));
        block_on(backend.handle(Cmd::Refreshed(generation, Some(refreshed("/data/New"))))).unwrap();
        assert_eq!(backend.config.mount_path(), Path::new("/data/New"));
        // A mistake made meanwhile: the last good copy stays.
        std::fs::write(&paths.config_file, "mount_point = 7\n").unwrap();
        block_on(backend.handle(Cmd::Refreshed(generation, Some(refreshed("/data/New"))))).unwrap();
        assert_eq!(backend.config.mount_path(), Path::new("/data/New"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn sign_in_can_replace_the_app_registration() {
        let cli = Path::new("/opt/axentra/axentra-onedrive");
        for registered in [false, true] {
            // With one already too: it may be the wrong one (Enter keeps it).
            let ran = std::cell::RefCell::new(String::new());
            let terminal = |_: &str, script: &str| {
                *ran.borrow_mut() = script.to_string();
                Ok(())
            };
            let went = sign_in(cli, registered, terminal, || {
                panic!("signed in without setup")
            });
            assert_eq!(went.unwrap(), SignIn::InTerminal);
            let script = ran.into_inner();
            assert!(
                script.starts_with(
                    "/opt/axentra/axentra-onedrive setup && /opt/axentra/axentra-onedrive login;"
                ),
                "{script}"
            );
        }
        // No terminal: straight to the browser with the one there is...
        let no_terminal =
            |_: &str, _: &str| -> std::io::Result<()> { Err(std::io::ErrorKind::NotFound.into()) };
        let logged_in = Cell::new(false);
        let went = sign_in(cli, true, no_terminal, || logged_in.set(true));
        assert_eq!(went.unwrap(), SignIn::InBrowser);
        assert!(logged_in.get());
        // ...but on the first run there's none to sign in with.
        let e = sign_in(cli, false, no_terminal, || panic!("no app registration")).unwrap_err();
        assert!(e.to_string().contains("`axentra-onedrive setup`"), "{e}");
    }

    #[test]
    fn files_on_demand_goes_back_if_it_wasnt_applied() {
        let dir = std::env::temp_dir().join(format!("axentra-ui-undo-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let paths = Paths::under(&dir);
        let text = |on: bool| format!("# mine\nclient_id = \"my-app\"\nfiles_on_demand = {on}\n");
        let mut backend = backend(paths.clone());
        // "Download all files" was saved, then downloading them failed.
        std::fs::write(&paths.config_file, text(false)).unwrap();
        block_on(backend.handle(Cmd::UndoOnDemand(false))).unwrap();
        assert!(Config::load(&paths.config_file).unwrap().files_on_demand);
        assert!(backend.config.files_on_demand);
        // Changed back since: left alone.
        std::fs::write(&paths.config_file, text(true)).unwrap();
        block_on(backend.handle(Cmd::UndoOnDemand(false))).unwrap();
        assert_eq!(
            std::fs::read_to_string(&paths.config_file).unwrap(),
            text(true)
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
