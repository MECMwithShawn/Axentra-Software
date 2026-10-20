//! The running tray app: Slint windows on the main thread, a tokio task
//! talking to the sync service over D-Bus, and the tray icon.

use std::cell::Cell;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::Result;
use axentra_onedrive_core::config::{Config, Paths};
use axentra_onedrive_core::db::Activity;
use axentra_onedrive_core::ipc::{AxentraOneDriveProxy, ChangedStream};
use axentra_onedrive_core::status::{Account, Status};
use futures_util::StreamExt;
use ksni::TrayMethods;
use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::winit_030::{winit, EventResult, WinitWindowAccessor};
use slint::{ComponentHandle, Weak};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use crate::tray::AxentraOneDriveTray;
use crate::{present, Flyout, SettingsWindow, Theme};

pub enum Cmd {
    /// From a tray click, with where it was clicked if the tray host said.
    ToggleFlyout(Option<(i32, i32)>),
    ShowFlyout(Option<(i32, i32)>),
    HideFlyout,
    ShowSettings,
    OpenFolder,
    ViewOnline,
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
    FreeUpSpace,
    Unlink,
    Reload,
    SetIconColor(String),
}

pub fn run(start_with_settings: bool, show_panel: bool) -> Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let _guard = rt.enter();

    let flyout = Flyout::new()?;
    let settings = SettingsWindow::new()?;
    let (tx, rx) = unbounded_channel();

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
    wire!(settings.on_set_icon_color(c) => Cmd::SetIconColor(c.to_string()));

    close_on_focus_loss(&flyout);
    if show_panel {
        let _ = tx.send(Cmd::ShowFlyout(None));
    }
    rt.spawn(
        Backend::new(tx.clone(), flyout.as_weak(), settings.as_weak()).run(rx, start_with_settings),
    );
    slint::run_event_loop_until_quit()?;
    rt.shutdown_timeout(Duration::from_secs(1));
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

struct Backend {
    paths: Paths,
    tx: UnboundedSender<Cmd>,
    flyout: Weak<Flyout>,
    settings: Weak<SettingsWindow>,
    proxy: Option<AxentraOneDriveProxy<'static>>,
    changed: Option<ChangedStream>,
    tray: Option<ksni::Handle<AxentraOneDriveTray>>,
    account: Account,
    started_daemon: bool,
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

fn clipboard() -> &'static Mutex<Option<arboard::Clipboard>> {
    static CB: OnceLock<Mutex<Option<arboard::Clipboard>>> = OnceLock::new();
    CB.get_or_init(|| Mutex::new(arboard::Clipboard::new().ok()))
}

impl Backend {
    fn new(tx: UnboundedSender<Cmd>, flyout: Weak<Flyout>, settings: Weak<SettingsWindow>) -> Self {
        Self {
            paths: Paths::user(),
            tx,
            flyout,
            settings,
            proxy: None,
            changed: None,
            tray: None,
            account: Account::default(),
            started_daemon: false,
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
        let c = color.to_string();
        if let Ok(Err(e)) = tokio::task::spawn_blocking(move || crate::icons::apply(&c)).await {
            tracing::warn!("couldn't update the desktop icon: {e}");
        }
    }

    fn config(&self) -> Config {
        Config::load(&self.paths.config_file).unwrap_or_default()
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
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        loop {
            if self.proxy.is_none() {
                self.connect().await;
            }
            tokio::select! {
                cmd = rx.recv() => match cmd {
                    Some(cmd) => {
                        if let Err(e) = self.handle(cmd).await {
                            tracing::warn!("{e:#}");
                            self.toast(format!("{e}"));
                        }
                    }
                    None => break,
                },
                Some(()) = next_change(&mut self.changed) => self.refresh().await,
                _ = tick.tick() => self.refresh().await,
            }
        }
    }

    async fn connect(&mut self) {
        let Ok(conn) = zbus::Connection::session().await else {
            return;
        };
        if let Ok(p) = AxentraOneDriveProxy::new(&conn).await {
            if p.status().await.is_ok() {
                self.changed = p.receive_changed().await.ok();
                self.proxy = Some(p);
                self.refresh().await;
                return;
            }
        }
        self.show_disconnected();
        if !self.started_daemon {
            self.started_daemon = true;
            let unit_ok = Command::new("systemctl")
                .args(["--user", "start", "axentra-onedrive.service"])
                .stderr(Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if !unit_ok {
                let _ = Command::new(sibling_binary("axentra-onedrive"))
                    .arg("daemon")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn();
            }
        }
    }

    fn show_disconnected(&self) {
        let st = Status {
            state: "starting".into(),
            message: "Starting Axentra OneDrive".into(),
            detail: "Waiting for the sync service to start".into(),
            ..Default::default()
        };
        let acct = self.account.clone();
        let _ = self
            .flyout
            .upgrade_in_event_loop(move |ui| present::apply_flyout(&ui, &st, &acct, &[]));
    }

    fn proxy(&self) -> Result<&AxentraOneDriveProxy<'static>> {
        self.proxy
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Axentra OneDrive isn't running yet"))
    }

    async fn refresh(&mut self) {
        let Some(p) = self.proxy.clone() else { return };
        let fetched = async {
            let st: Status = serde_json::from_str(&p.status().await?).unwrap_or_default();
            let acct: Account = serde_json::from_str(&p.account().await?).unwrap_or_default();
            let act: Vec<Activity> =
                serde_json::from_str(&p.activity(40).await?).unwrap_or_default();
            let usage = p.local_usage().await.unwrap_or(0);
            Ok::<_, zbus::Error>((st, acct, act, usage))
        }
        .await;
        let Ok((st, acct, act, usage)) = fetched else {
            // The service went away; reconnect on the next pass.
            self.proxy = None;
            self.changed = None;
            self.started_daemon = false;
            self.show_disconnected();
            return;
        };
        self.account = acct.clone();
        if let Some(tray) = &self.tray {
            let (state, tip) = (
                st.state.clone(),
                if st.detail.is_empty() {
                    st.message.clone()
                } else {
                    format!("{}\n{}", st.message, st.detail)
                },
            );
            tray.update(move |t| {
                t.state = state;
                t.tooltip = tip;
            })
            .await;
        }
        let cfg = self.config();
        let (st2, acct2) = (st.clone(), acct.clone());
        let _ = self
            .flyout
            .upgrade_in_event_loop(move |ui| present::apply_flyout(&ui, &st, &acct, &act));
        let _ = self.settings.upgrade_in_event_loop(move |ui| {
            // Live values only; editable fields are loaded when the window opens.
            let keep = (
                ui.get_notify_deletes(),
                ui.get_notify_conflicts(),
                ui.get_notify_errors(),
                ui.get_upload_mode(),
                ui.get_upload_kbps(),
                ui.get_download_mode(),
                ui.get_download_kbps(),
                ui.get_poll_index(),
                ui.get_files_on_demand(),
            );
            present::apply_settings(&ui, &cfg, &st2, &acct2, usage);
            ui.set_notify_deletes(keep.0);
            ui.set_notify_conflicts(keep.1);
            ui.set_notify_errors(keep.2);
            ui.set_upload_mode(keep.3);
            ui.set_upload_kbps(keep.4);
            ui.set_download_mode(keep.5);
            ui.set_download_kbps(keep.6);
            ui.set_poll_index(keep.7);
            ui.set_files_on_demand(keep.8);
        });
    }

    fn toast(&self, text: String) {
        let _ = self.flyout.upgrade_in_event_loop({
            let text = text.clone();
            move |ui| ui.set_toast(text.into())
        });
        let flyout = self.flyout.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(4)).await;
            let _ = flyout.upgrade_in_event_loop(move |ui| {
                if ui.get_toast() == text.as_str() {
                    ui.set_toast("".into());
                }
            });
        });
    }

    fn hide_flyout(&self) {
        let _ = self.flyout.upgrade_in_event_loop(|ui| {
            let _ = ui.hide();
        });
    }

    fn mount(&self) -> String {
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
                self.refresh().await;
            }
            Cmd::HideFlyout => {
                let _ = self.flyout.upgrade_in_event_loop(|ui| {
                    let _ = ui.hide();
                });
            }
            Cmd::ShowSettings => {
                let cfg = self.config();
                let _ = self.flyout.upgrade_in_event_loop(|ui| {
                    let _ = ui.hide();
                });
                let acct = self.account.clone();
                let _ = self.settings.upgrade_in_event_loop(move |ui| {
                    present::apply_settings(&ui, &cfg, &Status::default(), &acct, 0);
                    let _ = ui.show();
                });
                self.refresh().await;
            }
            Cmd::OpenFolder => {
                self.hide_flyout();
                open(&self.mount());
            }
            Cmd::ViewOnline => {
                let url = match self.proxy() {
                    Ok(p) => p.web_url(self.mount()).await.ok(),
                    Err(_) => None,
                };
                self.hide_flyout();
                open(&url.unwrap_or_else(|| "https://onedrive.live.com/my".into()));
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
            Cmd::Pause(h) => {
                self.proxy()?.pause(h * 3600).await?;
                self.refresh().await;
            }
            Cmd::Resume => {
                self.proxy()?.resume().await?;
                self.refresh().await;
            }
            Cmd::TogglePause(on) => {
                if on {
                    self.proxy()?.pause(2 * 3600).await?;
                } else {
                    self.proxy()?.resume().await?;
                }
                self.refresh().await;
            }
            Cmd::SignIn => {
                let cli = sibling_binary("axentra-onedrive");
                if self.config().client_id.is_empty() {
                    // First run: the app registration has to be entered once.
                    let script = format!(
                        "{0} setup && {0} login; echo; read -p 'Press Enter to close' _",
                        cli.display()
                    );
                    let _ = Command::new("gnome-terminal")
                        .args([
                            "--title",
                            "Set up Axentra OneDrive",
                            "--",
                            "bash",
                            "-c",
                            &script,
                        ])
                        .spawn()
                        .or_else(|_| {
                            Command::new("x-terminal-emulator")
                                .args(["-e", "bash", "-c", &script])
                                .spawn()
                        });
                } else {
                    let _ = Command::new(cli)
                        .arg("login")
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .spawn();
                    self.toast("Finish signing in in your browser".into());
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
            }
            Cmd::ShareItem(path) => {
                let link = self.proxy()?.share_link(path, false).await?;
                let copied = clipboard()
                    .lock()
                    .ok()
                    .and_then(|mut c| c.as_mut().map(|c| c.set_text(link.clone()).is_ok()))
                    .unwrap_or(false);
                self.toast(if copied {
                    "Link copied. Anyone with the link can view.".into()
                } else {
                    link
                });
            }
            Cmd::ItemOnline(path) => {
                self.hide_flyout();
                let url = self.proxy()?.web_url(path).await?;
                open(&url);
            }
            Cmd::SettingsChanged | Cmd::SetOnDemand(_) => {
                let pin_all = matches!(cmd, Cmd::SetOnDemand(false));
                let path = self.paths.config_file.clone();
                let tx = self.tx.clone();
                let _ = self.settings.upgrade_in_event_loop(move |ui| {
                    let mut cfg = Config::load(&path).unwrap_or_default();
                    present::read_settings(&ui, &mut cfg);
                    if let Err(e) = cfg.save(&path) {
                        tracing::warn!("couldn't save settings: {e:#}");
                    }
                    let _ = tx.send(Cmd::Reload);
                });
                if pin_all {
                    self.proxy()?.pin(vec![self.mount()]).await?;
                }
            }
            Cmd::SetIconColor(color) => {
                if !axentra_onedrive_core::config::ICON_COLORS.contains(&color.as_str()) {
                    anyhow::bail!("unknown icon colour {color}");
                }
                let mut cfg = self.config();
                cfg.icon_color = color.clone();
                cfg.save(&self.paths.config_file)?;
                self.show_icon_color(&color).await;
            }
            Cmd::Reload => {
                self.proxy()?.reload().await?;
                self.refresh().await;
            }
            Cmd::FreeUpSpace => {
                self.proxy()?.free_up_space(vec![self.mount()]).await?;
                self.refresh().await;
            }
            Cmd::Unlink => {
                self.proxy()?.sign_out().await?;
                self.refresh().await;
            }
        }
        Ok(())
    }
}
