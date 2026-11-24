//! The Axentra Vault window: the unlock screen and switching between locked and open states.
//! The open vault lives in `session.rs`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use eframe::egui::{self, Key, RichText};
use vault_core::envelope;
use vault_core::header::{Kdf, KdfBudget};
use vault_core::vault::{SealedVault, Vault};
use zeroize::Zeroizing;

use crate::ambient::Ambient;
use crate::chrome;
use crate::clipboard::{self, PendingClear};
use crate::icon::Icon;
use crate::intro::{self, Intro};
use crate::jobs::{self, Job, KeySource, UnlockError, WindowHandle};
use crate::session::{Session, SessionAction};
use crate::settings::{self, Layout as UiLayout};

/// Window size while locked (unlock card plus glow margin) in the Floating layout.
pub const COMPACT_SIZE: egui::Vec2 = egui::vec2(548.0, 800.0);
/// Default size of the open vault in the Floating layout.
const EXPANDED_SIZE: egui::Vec2 = egui::vec2(1440.0, 860.0);
/// The checkbox that approves a refused database's heavy key derivation settings.
pub(crate) const HEAVY_KDF_OVERRIDE: &str = "Unlock anyway: I chose these key derivation settings";
/// Shown under the database path when it is a file in a backup folder.
pub(crate) const BACKUP_UNLOCK_HINT: &str = "Backups open with your master password + backup key file; passkeys unlock only the main vault.";

#[derive(Clone, Copy, PartialEq, Eq)]
enum WindowMode {
    /// Just the unlock card, not resizable.
    Compact,
    /// The full layout, resizable.
    Expanded,
}

/// The window's new size and the commands that turn it into the unlock card or the full layout.
///
/// Wayland closes a window whose minimum size is larger than its maximum. winit pins the maximum
/// to the window's size while it isn't resizable and keeps that maximum when resizing is turned
/// back on, so the limits are loosened before they're tightened: opening lifts the maximum before
/// raising the minimum, and locking lowers the minimum before the maximum.
fn window_commands(
    mode: WindowMode,
    monitor_h: Option<f32>,
    expanded: egui::Vec2,
) -> (egui::Vec2, Vec<egui::ViewportCommand>) {
    use egui::ViewportCommand as V;
    match mode {
        WindowMode::Compact => {
            let height = COMPACT_SIZE.y.min(monitor_h.unwrap_or(900.0) - 60.0).max(560.0);
            let size = egui::vec2(COMPACT_SIZE.x, height);
            let commands = vec![
                V::Maximized(false),
                V::Resizable(false),
                V::MinInnerSize(size),
                V::MaxInnerSize(size),
                V::InnerSize(size),
            ];
            (size, commands)
        }
        WindowMode::Expanded => {
            let commands = vec![
                V::Resizable(true),
                V::MaxInnerSize(egui::Vec2::INFINITY),
                V::MinInnerSize(egui::vec2(980.0, 620.0)),
                V::InnerSize(expanded),
            ];
            (expanded, commands)
        }
    }
}

pub struct VaultApp {
    screen: Screen,
    clipboard: Option<PendingClear>,
    layout: UiLayout,
    /// Layout whose window decorations were last applied.
    applied: Option<UiLayout>,
    /// False on Remote Desktop or with high contrast: paint an opaque background instead.
    transparent: bool,
    /// Window size mode last applied, and the size to restore when the vault opens.
    window: Option<WindowMode>,
    expanded_size: egui::Vec2,
    /// When window capture exclusion was last applied.
    capture_applied: Option<std::time::Instant>,
    /// Show / Lock / Exit from the tray icon, a second launch or Axentra Command.
    commands: Option<std::sync::mpsc::Receiver<crate::instance::Command>>,
    tray: Option<crate::tray::Tray>,
    /// A Lock request to carry out on the next frame.
    lock_requested: bool,
}

enum Screen {
    Locked(Box<Unlock>),
    Open(Box<Session>),
}

impl VaultApp {
    /// The real app: also the tray icon and the listener for commands from other programs.
    /// (Tests use `with_layout`, which creates neither.)
    pub fn new(cc: &eframe::CreationContext<'_>, instance: crate::instance::Instance) -> Self {
        let mut app = Self::with_layout(cc, settings::layout());
        let (tx, rx) = std::sync::mpsc::channel();
        let (to_app, wake) = (tx.clone(), cc.egui_ctx.clone());
        instance.listen(move |c| {
            let _ = to_app.send(c);
            wake.request_repaint();
        });
        app.tray = crate::tray::Tray::new(&cc.egui_ctx, tx);
        app.commands = Some(rx);
        app
    }

    pub fn with_layout(cc: &eframe::CreationContext<'_>, layout: UiLayout) -> Self {
        // SVG support for the icon set and the vault mark.
        egui_extras::install_image_loaders(&cc.egui_ctx);
        crate::theme::apply(&cc.egui_ctx);
        VaultApp {
            screen: Screen::Locked(Box::new(Unlock::new(settings::last_database(), true))),
            clipboard: None,
            layout,
            applied: None,
            transparent: chrome::transparency_available(),
            window: None,
            expanded_size: EXPANDED_SIZE,
            capture_applied: None,
            commands: None,
            tray: None,
            lock_requested: false,
        }
    }

    /// Resizes the window around its current centre: compact card while locked, full layout
    /// once open.
    fn apply_window(&mut self, ctx: &egui::Context, mode: WindowMode) {
        use egui::ViewportCommand as V;
        let monitor_h = ctx.input(|i| i.viewport().monitor_size).map(|m| m.y);
        let (size, commands) = window_commands(mode, monitor_h, self.expanded_size);
        for command in commands {
            ctx.send_viewport_cmd(command);
        }
        if let Some(outer) = ctx.input(|i| i.viewport().outer_rect) {
            let pos = outer.center() - size / 2.0;
            ctx.send_viewport_cmd(V::OuterPosition(egui::pos2(pos.x.max(0.0), pos.y.max(0.0))));
        }
        // The open vault fills the screen by default; the size above is what "Restore" returns to.
        if mode == WindowMode::Expanded {
            ctx.send_viewport_cmd(V::Maximized(true));
        }
    }

    fn copy(&mut self, text: &str) -> Option<String> {
        if let Some(old) = self.clipboard.take() {
            old.clear();
        }
        self.clipboard = clipboard::copy_secret(text);
        if self.clipboard.is_some() {
            None
        } else {
            Some("The clipboard is busy; try again.".into())
        }
    }
}

impl Drop for VaultApp {
    fn drop(&mut self) {
        if let Some(p) = self.clipboard.take() {
            p.clear();
        }
    }
}

impl eframe::App for VaultApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        if self.layout == UiLayout::Floating && self.transparent {
            [0.0, 0.0, 0.0, 0.0]
        } else {
            egui::Rgba::from(egui::Color32::from_rgb(0x05, 0x09, 0x11)).to_array()
        }
    }

    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        crate::dialogs::remember(frame);
        #[cfg(target_os="linux")]
        if gtk::is_initialized_main_thread() { while gtk::events_pending() { gtk::main_iteration_do(false); } ctx.request_repaint_after(std::time::Duration::from_millis(100)); }
        // Re-applied every second so new windows (the entry editor) are covered too.
        #[cfg(windows)]
        if self
            .capture_applied
            .is_none_or(|t| t.elapsed() >= Duration::from_secs(1))
        {
            crate::hardening::exclude_from_capture(crate::hardening::capture_hidden());
            self.capture_applied = Some(std::time::Instant::now());
            ctx.request_repaint_after(Duration::from_secs(1));
        }
        if let Some(rx) = &self.commands {
            while let Ok(command) = rx.try_recv() {
                use crate::instance::Command;
                match command {
                    Command::Show => {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    }
                    Command::Lock => self.lock_requested = true,
                    // The normal close path: it still asks about unsaved changes.
                    Command::Exit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                }
            }
        }
        let unlocked = matches!(self.screen, Screen::Open(_));
        if let Some(tray) = &mut self.tray {
            tray.update(ctx, unlocked);
        }
        if self.clipboard.as_ref().is_some_and(PendingClear::due) {
            self.clipboard.take().unwrap().clear();
        }
        if self.clipboard.is_some() || matches!(self.screen, Screen::Open(_)) {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let mut next: Option<Screen> = None;
        let mut copy_request: Option<Zeroizing<String>> = None;
        let mut new_layout: Option<UiLayout> = None;

        // Both layouts use integrated window controls.
        let floating = self.layout == UiLayout::Floating;
        if self.applied != Some(self.layout) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(false));
            self.applied = Some(self.layout);
        }
        let full = ui.max_rect();
        if floating && !self.transparent {
            chrome::opaque_backdrop(ui, full);
        }
        let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        let area = full.shrink(if maximized { 6.0 } else { chrome::GAP });

        // Floating: a compact card while locked, the full layout (maximized) once open. Classic: maximized.
        let locked = matches!(self.screen, Screen::Locked(_));
        let want = if floating && locked {
            WindowMode::Compact
        } else {
            WindowMode::Expanded
        };
        if self.window != Some(want) {
            if self.window.is_some() || want == WindowMode::Compact {
                self.apply_window(&ctx, want);
            }
            self.window = Some(want);
        } else if want == WindowMode::Expanded
            && floating
            && !maximized
            && let Some(r) = ctx.input(|i| i.viewport().inner_rect)
            && r.width() >= 980.0
            && r.height() >= 620.0
        {
            self.expanded_size = r.size();
        }

        match &mut self.screen {
            Screen::Locked(unlock) => {
                self.lock_requested = false;
                unlock.layout = self.layout;
                let opened = if floating {
                    unlock.show_floating(ui, area)
                } else {
                    unlock.show(ui)
                };
                if let Some(l) = unlock.layout_request.take() {
                    new_layout = Some(l);
                }
                if let Some(vault) = opened {
                    // A backup opened as a vault is never the one to reopen next time.
                    if !vault.is_read_only() {
                        settings::remember_database(&vault.path().display().to_string());
                    }
                    let mut session = Session::new(vault);
                    // Old copies beside the database are offered once per unlock, until moved
                    // or "Don't ask again"; then the first message.
                    session.after_unlock(unlock.recovery.is_some());
                    next = Some(Screen::Open(Box::new(session)));
                }
            }
            Screen::Open(session) => {
                let clip_left = self.clipboard.as_ref().map(PendingClear::remaining);
                let action = if std::mem::take(&mut self.lock_requested) {
                    // Lock from the tray or another program: the same path as auto-lock.
                    session.lock_now()
                } else if floating {
                    session.show_floating(ui, area, clip_left)
                } else {
                    session.show(ui, clip_left)
                };
                match action {
                    SessionAction::None => {}
                    SessionAction::Copy(text) => copy_request = Some(text),
                    SessionAction::Lock => {
                        let path = session.vault.path().display().to_string();
                        let mut unlock = Unlock::new(Some(path), false);
                        unlock.recovery = session.recovery.take();
                        unlock.lock_notice = session.lock_notice.take();
                        next = Some(Screen::Locked(Box::new(unlock)));
                    }
                    SessionAction::Close => {
                        session.allow_close = true;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    SessionAction::SetLayout(l) => new_layout = Some(l),
                }
            }
        }

        if let Some(text) = copy_request {
            let err = self.copy(&text);
            if let Screen::Open(s) = &mut self.screen {
                s.error = err;
            }
        }
        if let Some(screen) = next {
            // Discard widget state across authentication boundaries, including old editor undo stacks.
            ctx.data_mut(|data| data.clear());
            // Protected database values, credentials and secret form buffers zeroize on drop.
            self.screen = screen;
        }
        if let Some(l) = new_layout {
            settings::set_layout(l);
            self.layout = l;
        }
        if !floating || !matches!(self.screen, Screen::Locked(_)) {
            chrome::resize_grips(ui, full);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Unlock screen

#[derive(Clone, Copy, PartialEq, Eq)]
enum KeyMode {
    PasswordOnly,
    KeyFile,
    Passkey,
}

struct Unlock {
    db_path: String,
    recovery: Option<SealedVault>,
    lock_notice: Option<String>,
    password: Zeroizing<String>,
    mode: KeyMode,
    key_file: String,
    error: Option<String>,
    job: Option<Job<Result<Vault, UnlockError>>>,
    focus_password: bool,
    intro: Intro,
    ambient: Ambient,
    /// Running unlock: when it started, what it is waiting for, and the expected duration.
    started: Option<std::time::Instant>,
    phase: std::sync::Arc<std::sync::atomic::AtomicU8>,
    estimate_ms: Option<u64>,
    kdf: Option<String>,
    /// The opened vault, held for the short lock-opening flourish.
    opened: Option<(std::time::Instant, Vault)>,
    /// Over-budget key derivation settings this database's header asks for, once refused, and
    /// whether the user ticked "Unlock anyway" for exactly those settings.
    heavy_kdf: Option<Kdf>,
    allow_heavy_kdf: bool,
    /// The database path is inside a backup folder: it opens as a read-only backup.
    is_backup: bool,
    /// Current layout (set by the app each frame) and a requested switch from the unlock card.
    pub layout: UiLayout,
    pub layout_request: Option<UiLayout>,
}

impl Unlock {
    /// `intro` plays the startup animation (on launch, not after locking).
    fn new(db_path: Option<String>, intro: bool) -> Self {
        let mut u = Unlock {
            db_path: db_path.unwrap_or_default(),
            recovery: None,
            lock_notice: None,
            password: Zeroizing::new(String::new()),
            mode: KeyMode::PasswordOnly,
            key_file: String::new(),
            error: None,
            job: None,
            focus_password: true,
            intro: Intro::new(intro),
            ambient: Ambient::new(intro::system_animations_enabled()),
            started: None,
            phase: Default::default(),
            estimate_ms: None,
            kdf: None,
            opened: None,
            heavy_kdf: None,
            allow_heavy_kdf: false,
            is_backup: false,
            layout: UiLayout::Floating,
            layout_request: None,
        };
        u.mode = u.default_mode();
        u.is_backup = u.in_backup_folder();
        u
    }

    /// Whether the database path is a backup in a vault's backup folder (reads the backup
    /// settings; see `BackupConfig::backup_root_of`).
    fn in_backup_folder(&self) -> bool {
        let path = self.db_path.trim();
        !path.is_empty()
            && settings::backup_config()
                .backup_root_of(Path::new(path))
                .is_some()
    }

    fn has_passkey_file(&self) -> bool {
        !self.db_path.trim().is_empty()
            && envelope::path_for(Path::new(self.db_path.trim())).exists()
    }

    fn default_mode(&self) -> KeyMode {
        if self.has_passkey_file() {
            KeyMode::Passkey
        } else {
            KeyMode::PasswordOnly
        }
    }

    /// A different database: its own key options, and no approval carried over.
    fn database_changed(&mut self) {
        self.mode = self.default_mode();
        self.is_backup = self.in_backup_folder();
        self.heavy_kdf = None;
        self.allow_heavy_kdf = false;
    }

    /// Refuses to derive `kdf` and offers "Unlock anyway". Approval is for exact settings: if they
    /// changed since the box was ticked, it is cleared.
    fn refuse_heavy_kdf(&mut self, kdf: Kdf) {
        self.error = Some(jobs::heavy_kdf_message(&kdf));
        if self.heavy_kdf.as_ref() != Some(&kdf) {
            self.allow_heavy_kdf = false;
        }
        self.heavy_kdf = Some(kdf);
    }

    /// Returns the opened vault once unlocking succeeds.
    fn show(&mut self, ui: &mut egui::Ui) -> Option<Vault> {
        if self.recovery.is_some() && ui.input(|i| i.viewport().close_requested()) {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.error = Some(
                "Unlock to save or explicitly discard the recovered changes before closing.".into(),
            );
        }
        // Hand the vault over once the lock-opening flourish has played.
        if self
            .opened
            .as_ref()
            .is_some_and(|(at, _)| at.elapsed() >= intro::OPEN_HOLD)
        {
            return self.opened.take().map(|(_, v)| v);
        }
        if let Some(result) = self.job.as_ref().and_then(Job::poll) {
            self.job = None;
            let took = self.started.map(|s| s.elapsed());
            match result.unwrap_or_else(|jobs::WorkerDied| {
                Err(UnlockError::Failed(
                    "Unlocking stopped unexpectedly. Try again.".into(),
                ))
            }) {
                Ok(vault) => {
                    // A backup's path is not kept anywhere, not even with its unlock time.
                    if let Some(took) = took.filter(|_| !vault.is_read_only()) {
                        settings::set_unlock_ms(self.db_path.trim(), took.as_millis() as u64);
                    }
                    if !intro::system_animations_enabled() {
                        return Some(vault);
                    }
                    self.opened = Some((std::time::Instant::now(), vault));
                    ui.ctx().request_repaint();
                }
                Err(UnlockError::Cancelled) => {
                    self.error = Some("Passkey request cancelled.".into())
                }
                Err(UnlockError::Failed(msg)) => self.error = Some(msg),
                Err(UnlockError::HeavyKdf(kdf)) => self.refuse_heavy_kdf(kdf),
            }
            if self.opened.is_none() {
                self.started = None;
                self.focus_password = true;
            }
        }
        let busy = self.job.is_some() || self.opened.is_some();
        let activity = self.activity();
        let mut submit = false;

        if self.layout == UiLayout::Classic {
            egui::Panel::top("classic_unlock_chrome").show(ui, |ui| {
                crate::window_chrome::header(ui, "Axentra Vault");
            });
        }
        // No panel frame, so the backdrop runs edge to edge.
        egui::CentralPanel::no_frame().show(ui, |ui| {
            self.ambient.paint(ui, ui.max_rect());
            // Circuit Halo emblem: 392 px on a roomy window, scaling down so the form always fits
            // (including the longer key derivation warning and its "Unlock anyway" box).
            let form = if self.heavy_kdf.is_some() { 420.0 } else { 300.0 }
                + if self.is_backup { 36.0 } else { 0.0 };
            let top = (ui.available_height() * 0.04).min(36.0);
            let fit_h = |e: f32| Intro::region_height(e) + form + top + 24.0 <= ui.available_height();
            let mut emblem = 392.0_f32.min(ui.available_width() - 32.0);
            while emblem > 140.0 && !fit_h(emblem) {
                emblem -= 8.0;
            }
            ui.add_space(top);
            // The region is allocated at full size from frame zero; the form below never moves.
            self.intro.show(ui, emblem, activity);
            ui.add_space(16.0);

            let width = 520.0_f32.min(ui.available_width() - 32.0);
            ui.vertical_centered(|ui| {
                ui.set_max_width(width);
                // Glass card over the backdrop.
                egui::Frame::new()
                    .fill(egui::Color32::from_rgba_unmultiplied(0x0b, 0x16, 0x22, 225))
                    .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(0x5f, 0xe3, 0xff, 48)))
                    .corner_radius(14.0)
                    .inner_margin(20.0)
                    .show(ui, |ui| {
                    ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_width(width - 36.0);
                    ui.add_enabled_ui(!busy, |ui| {
                        if let Some(notice) = &self.lock_notice {
                            ui.colored_label(ui.visuals().warn_fg_color, notice);
                        }
                        ui.label("Database");
                        ui.add_enabled_ui(self.recovery.is_none(), |ui| { ui.horizontal(|ui| {
                            let edit = ui.add(egui::TextEdit::singleline(&mut self.db_path).desired_width(ui.available_width() - 90.0));
                            if edit.changed() {
                                self.database_changed();
                            }
                            if ui.button("Browse…").clicked()
                                && let Some(p) = crate::dialogs::file().add_filter("KeePass database", &["kdbx"]).pick_file() {
                                    self.db_path = p.display().to_string();
                                    self.database_changed();
                                    self.focus_password = true;
                                }
                        }); });
                        if self.is_backup {
                            ui.label(RichText::new(BACKUP_UNLOCK_HINT).small().color(ui.visuals().warn_fg_color));
                        }
                        ui.add_space(10.0);

                        ui.label("Master password");
                        let pw = crate::secret_edit::show(ui, &mut self.password, false, false, f32::INFINITY);
                        if self.focus_password {
                            pw.request_focus();
                            self.focus_password = false;
                        }
                        if pw.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                            submit = true;
                        }
                        ui.add_space(10.0);

                        ui.horizontal(|ui| {
                            ui.add(Icon::Key.image(14.0));
                            ui.label("Second factor");
                        });
                        let has_passkey = self.has_passkey_file();
                        ui.horizontal_wrapped(|ui| {
                            ui.radio_value(&mut self.mode, KeyMode::PasswordOnly, "None");
                            ui.radio_value(&mut self.mode, KeyMode::KeyFile, "Key file");
                            ui.add_enabled_ui(has_passkey, |ui| {
                                ui.radio_value(&mut self.mode, KeyMode::Passkey, "Passkey / security key")
                                    .on_disabled_hover_text("No passkey is set up for this database (no .axvault file next to it).");
                            });
                        });
                        if self.mode == KeyMode::KeyFile {
                            ui.horizontal(|ui| {
                                ui.add(egui::TextEdit::singleline(&mut self.key_file).desired_width(ui.available_width() - 90.0));
                                if ui.button("Browse…").clicked()
                                    && let Some(p) = crate::dialogs::file().pick_file() {
                                        self.key_file = p.display().to_string();
                                    }
                            });
                        }
                        ui.add_space(14.0);
                        if ui.add_sized([ui.available_width(), 32.0], egui::Button::new(RichText::new("Unlock vault").strong().color(egui::Color32::from_rgb(0x04, 0x13, 0x1c)))
                                .fill(egui::Color32::from_rgb(0x22, 0xd3, 0xee))).clicked() {
                            submit = true;
                        }
                    });

                    if busy {
                        ui.add_space(10.0);
                        self.status_line(ui, activity);
                    }
                    if let Some(err) = &self.error {
                        ui.add_space(8.0);
                        ui.colored_label(ui.visuals().error_fg_color, err);
                    }
                    // The explicit override for a refused key derivation, for exactly these settings.
                    if self.heavy_kdf.is_some() {
                        ui.add_space(6.0);
                        ui.add_enabled(!busy, egui::Checkbox::new(&mut self.allow_heavy_kdf, HEAVY_KDF_OVERRIDE));
                    }
                    // Layout choice is available before unlocking, so no one gets stuck in one.
                    ui.add_space(10.0);
                    ui.vertical_centered(|ui| {
                        let (label, target) = match self.layout {
                            UiLayout::Floating => ("Switch to Classic layout", UiLayout::Classic),
                            UiLayout::Classic => ("Switch to Floating layout", UiLayout::Floating),
                        };
                        if ui.add_enabled(!busy, egui::Button::new(RichText::new(label).small().weak()).frame(false)).clicked() {
                            self.layout_request = Some(target);
                        }
                    });
                    });
                });
            });
        });

        if submit && !busy {
            self.start(ui.ctx());
        }
        None
    }

    fn activity(&self) -> intro::Activity {
        let working = self.started.map(|s| s.elapsed().as_secs_f32());
        intro::Activity {
            working,
            // Ease toward the last unlock time, holding short of full until it really finishes.
            progress: match (self.opened.is_some(), working, self.estimate_ms) {
                (true, _, _) => Some(1.0),
                (false, Some(w), Some(ms)) if ms > 0 => {
                    let x = (w / (ms as f32 / 1000.0)).min(1.0);
                    Some((1.0 - (1.0 - x).powi(2)) * 0.95)
                }
                _ => None,
            },
            opening: self
                .opened
                .as_ref()
                .map(|(at, _)| at.elapsed().as_secs_f32()),
        }
    }

    /// One honest line about what the unlock is doing, with progress when it can be measured.
    fn status_line(&self, ui: &mut egui::Ui, activity: intro::Activity) {
        let phase = self.phase.load(std::sync::atomic::Ordering::Relaxed);
        let (text, color) = if self.opened.is_some() {
            (
                "Unlocked".to_owned(),
                egui::Color32::from_rgb(0x34, 0xd3, 0x99),
            )
        } else if phase == jobs::PHASE_PASSKEY && self.mode == KeyMode::Passkey {
            ("Approve on your phone".to_owned(), crate::theme::CYAN)
        } else {
            let what = match &self.kdf {
                Some(k) => format!("Stretching your key  ·  {k}"),
                None => "Unlocking your vault".to_owned(),
            };
            (what, crate::theme::CYAN)
        };
        let dots = match activity.working {
            // Dots only on messages that end in words; the halo itself shows motion.
            Some(w)
                if self.opened.is_none()
                    && intro::system_animations_enabled()
                    && !text.contains('·') =>
            {
                ".".repeat(1 + (w * 2.5) as usize % 3)
            }
            _ => String::new(),
        };
        ui.vertical_centered(|ui| {
            ui.label(RichText::new(format!("{text}{dots}")).color(color));
            if let (Some(p), Some(w)) = (activity.progress, activity.working)
                && self.opened.is_none()
            {
                let (rect, _) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width() * 0.6, 3.0),
                    egui::Sense::hover(),
                );
                ui.painter().rect_filled(
                    rect,
                    1.5,
                    egui::Color32::from_rgba_unmultiplied(0x5f, 0xe3, 0xff, 30),
                );
                let fill = egui::Rect::from_min_size(
                    rect.min,
                    egui::vec2(rect.width() * p, rect.height()),
                );
                ui.painter().rect_filled(fill, 1.5, crate::theme::CYAN);
                let est = self.estimate_ms.unwrap_or(0) as f32 / 1000.0;
                ui.label(
                    RichText::new(format!("{w:.1} s  ·  usually about {est:.1} s"))
                        .small()
                        .weak(),
                );
            }
        });
    }

    /// The locked screen in the Floating layout: the whole (compact) window is the unlock card,
    /// with subtle minimise and close buttons. Drag anywhere on the card to move it.
    fn show_floating(&mut self, ui: &mut egui::Ui, card: egui::Rect) -> Option<Vault> {
        let ctx = ui.ctx().clone();
        let drag = ui.interact(
            card,
            egui::Id::new("unlock_card_drag"),
            egui::Sense::click_and_drag(),
        );
        if drag.drag_started_by(egui::PointerButton::Primary) {
            ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        let painter = ui.painter().clone();
        chrome::module_body(&painter, card, 0.0);
        let inner = card.shrink(2.0);
        let opened = ui
            .scope_builder(
                egui::UiBuilder::new()
                    .max_rect(inner)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
                |ui| {
                    ui.set_clip_rect(inner);
                    self.show(ui)
                },
            )
            .inner;
        chrome::module_edges(&painter, card, 0.0);

        // Small "locked" mark and the window buttons along the top edge.
        let lock = egui::Rect::from_min_size(
            card.left_top() + egui::vec2(16.0, 14.0),
            egui::vec2(14.0, 14.0),
        );
        Icon::Lock.image(14.0).paint_at(ui, lock);
        painter.text(
            egui::pos2(lock.right() + 7.0, lock.center().y),
            egui::Align2::LEFT_CENTER,
            "LOCKED",
            egui::FontId::proportional(10.5),
            egui::Color32::from_rgba_unmultiplied(0x5f, 0xe3, 0xff, 170),
        );
        chrome::window_controls(
            ui,
            card.right() - 8.0,
            card.top() + 21.0,
            egui::vec2(34.0, 28.0),
            &[chrome::Ctl::Close, chrome::Ctl::Minimize],
            true,
        );
        opened
    }

    fn start(&mut self, ctx: &egui::Context) {
        let path = PathBuf::from(self.db_path.trim());
        if self.recovery.is_none() && !path.is_file() {
            self.error = Some("Choose a database file (.kdbx).".into());
            return;
        }
        let source = match self.mode {
            KeyMode::PasswordOnly => KeySource::None,
            KeyMode::KeyFile if self.key_file.trim().is_empty() => {
                self.error = Some("Choose a key file.".into());
                return;
            }
            KeyMode::KeyFile => KeySource::KeyFile(PathBuf::from(self.key_file.trim())),
            KeyMode::Passkey => KeySource::Passkey,
        };
        let password = (!self.password.is_empty()).then(|| self.password.clone());
        if password.is_none() && matches!(source, KeySource::None) {
            self.error = Some("Enter the master password.".into());
            return;
        }
        let budget = match &self.heavy_kdf {
            Some(kdf) if self.allow_heavy_kdf => KdfBudget::DEFAULT.allowing(kdf),
            _ => KdfBudget::DEFAULT,
        };
        let header_kdf = vault_core::header::kdf_of(&path);
        // Refused here too, before a passkey prompt that would be wasted; the worker checks the
        // exact bytes it parses either way.
        if self.recovery.is_none()
            && let Some(kdf) = header_kdf.as_ref().filter(|k| !budget.allows(k))
        {
            self.refuse_heavy_kdf(kdf.clone());
            return;
        }
        self.error = None;
        let hwnd = WindowHandle::foreground();
        self.started = Some(std::time::Instant::now());
        self.phase = Default::default();
        self.estimate_ms = settings::unlock_ms(self.db_path.trim());
        self.kdf = header_kdf.map(|k| k.summary());
        let phase = self.phase.clone();
        let recovery = self.recovery.clone();
        self.job = Some(Job::spawn(ctx, move || {
            let backups = settings::backup_config();
            jobs::unlock(
                path, recovery, password, source, hwnd, phase, budget, backups,
            )
        }));
    }
}

#[cfg(test)]
mod window_tests {
    use super::*;

    /// How winit 0.30 keeps a Wayland window's size limits: one stored minimum and maximum,
    /// both pinned to the current size when the window stops being resizable.
    struct Wayland {
        size: egui::Vec2,
        min: egui::Vec2,
        max: Option<egui::Vec2>,
        resizable: bool,
    }

    impl Wayland {
        fn apply(&mut self, command: &egui::ViewportCommand) {
            use egui::ViewportCommand as V;
            match *command {
                V::Resizable(r) if r != self.resizable => {
                    self.resizable = r;
                    if !r {
                        self.min = self.size;
                        self.max = Some(self.size);
                    }
                }
                V::MinInnerSize(s) => self.min = s,
                V::MaxInnerSize(s) => self.max = (s != egui::Vec2::INFINITY).then_some(s),
                V::InnerSize(s) => self.size = s,
                _ => {}
            }
            if let Some(max) = self.max {
                assert!(
                    self.min.x <= max.x && self.min.y <= max.y,
                    "the compositor would close the window: minimum {:?} > maximum {:?} after {command:?}",
                    self.min,
                    max
                );
            }
        }
    }

    #[test]
    fn unlocking_and_locking_never_set_a_minimum_above_the_maximum() {
        // The Floating layout starts as the unlock card, not resizable.
        let mut window = Wayland {
            size: COMPACT_SIZE,
            min: COMPACT_SIZE,
            max: Some(COMPACT_SIZE),
            resizable: false,
        };
        for (mode, monitor_h) in [
            (WindowMode::Expanded, Some(1080.0)),
            (WindowMode::Compact, Some(1080.0)),
            (WindowMode::Expanded, Some(1080.0)),
            (WindowMode::Compact, Some(700.0)),
            (WindowMode::Expanded, None),
        ] {
            let (size, commands) = window_commands(mode, monitor_h, EXPANDED_SIZE);
            for command in &commands {
                window.apply(command);
            }
            assert_eq!(window.size, size);
            assert_eq!(window.resizable, mode == WindowMode::Expanded);
            // Open: resizable from 980x620 up, with no maximum. Locked: exactly the card.
            match mode {
                WindowMode::Expanded => assert_eq!((window.min, window.max), (egui::vec2(980.0, 620.0), None)),
                WindowMode::Compact => assert_eq!((window.min, window.max), (size, Some(size))),
            }
            // A maximized open vault is what locking shrinks from.
            window.size = egui::vec2(1920.0, 1032.0);
        }
    }
}

#[cfg(test)]
mod security_tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};
    use vault_core::keepass::{Database, DatabaseKey, config::KdfConfig};
    #[test]
    fn master_password_cannot_be_undone_after_lock() {
        let dir = std::env::temp_dir().join(format!("audit-master-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("synthetic.kdbx");
        let mut db = Database::new();
        db.config.kdf_config = KdfConfig::Aes { rounds: 1000 };
        db.save(
            &mut std::fs::File::create(&path).unwrap(),
            DatabaseKey::new().with_password("audit-master-canary"),
        )
        .unwrap();
        let mut h = Harness::builder()
            .with_size(egui::vec2(1280.0, 800.0))
            .build_eframe(|cc| VaultApp::with_layout(cc, UiLayout::Classic));
        let mut unlock = Unlock::new(Some(path.display().to_string()), false);
        *unlock.password = "audit-master-canary".into();
        h.state_mut().screen = Screen::Locked(Box::new(unlock));
        h.run_steps(4);
        h.get_by_label("Unlock vault").click();
        for _ in 0..200 {
            h.run_steps(1);
            if matches!(h.state().screen, Screen::Open(_)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(matches!(h.state().screen, Screen::Open(_)));
        h.run_steps(3);
        h.get_by_label("Lock").click();
        h.run_steps(3);
        let Screen::Locked(u) = &h.state().screen else {
            panic!("not locked")
        };
        assert!(u.password.is_empty());
        h.get_by_role(egui::accesskit::Role::PasswordInput).focus();
        h.run_steps(2);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
        h.run_steps(2);
        let Screen::Locked(u) = &h.state().screen else {
            panic!("not locked")
        };
        assert!(u.password.is_empty());
    }

    #[test]
    fn idle_lock_recovery_uses_the_real_unlock_screen_even_if_file_disappears() {
        use vault_core::keepass::db::fields;
        use vault_core::vault::Credentials;
        let dir = std::env::temp_dir().join(format!("axv-recovery-ui-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("synthetic.kdbx");
        let credentials = Credentials {
            password: Some(Zeroizing::new("recovery-master".into())),
            key_file: None,
        };
        let mut db = Database::new();
        db.config.kdf_config = KdfConfig::Aes { rounds: 1000 };
        db.save(
            &mut std::fs::File::create(&path).unwrap(),
            credentials.to_key().unwrap(),
        )
        .unwrap();
        let mut session = Session::new(Vault::open(&path, credentials).unwrap());
        session
            .vault
            .db
            .root_mut()
            .add_entry()
            .set_protected(fields::PASSWORD, "recovered-value");
        session.vault.mark_dirty();
        std::fs::remove_file(&path).unwrap();
        let mut h = Harness::builder()
            .with_size(egui::vec2(1280.0, 800.0))
            .build_eframe(|cc| VaultApp::with_layout(cc, UiLayout::Classic));
        h.state_mut().screen = Screen::Open(Box::new(session));
        h.run_steps(3);
        let Screen::Open(session) = &mut h.state_mut().screen else {
            panic!("expected open")
        };
        session.expire_idle_timer();
        h.run_steps(3);
        let Screen::Locked(unlock) = &mut h.state_mut().screen else {
            panic!("expected auto-lock")
        };
        assert!(unlock.recovery.is_some());
        assert!(unlock.password.is_empty());
        *unlock.password = "wrong-password".into();
        h.get_by_label("Unlock vault").click();
        for _ in 0..200 {
            h.run_steps(1);
            if let Screen::Locked(u) = &h.state().screen
                && u.error.is_some()
                && u.job.is_none()
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let Screen::Locked(unlock) = &mut h.state_mut().screen else {
            panic!("wrong password opened recovery")
        };
        assert!(unlock.error.is_some());
        assert!(unlock.recovery.is_some());
        *unlock.password = "recovery-master".into();
        h.get_by_label("Unlock vault").click();
        for _ in 0..200 {
            h.run_steps(1);
            if matches!(h.state().screen, Screen::Open(_)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let Screen::Open(session) = &h.state().screen else {
            panic!("recovery did not open")
        };
        assert!(session.vault.is_dirty());
        assert_eq!(
            session
                .vault
                .db
                .iter_all_entries()
                .next()
                .unwrap()
                .get_password(),
            Some("recovered-value")
        );
        assert!(session.error.is_some());
        assert!(
            !path.exists(),
            "recovery did not overwrite or recreate the file"
        );
    }

    /// An unlock worker that panics ends the attempt with a generic error (never the panic
    /// text) and gives the form back, instead of leaving it busy forever.
    #[test]
    fn panicked_unlock_job_shows_a_generic_error_and_frees_the_form() {
        crate::ui_tests::isolate_settings();
        let mut h = Harness::builder()
            .with_size(egui::vec2(1280.0, 800.0))
            .build_eframe(|cc| VaultApp::with_layout(cc, UiLayout::Classic));
        let job = Job::spawn(&h.ctx, || -> Result<Vault, UnlockError> {
            panic!("unlock-panic-canary")
        });
        assert!(matches!(
            job.wait(Duration::from_secs(10)),
            Some(Err(jobs::WorkerDied))
        ));
        let mut unlock = Unlock::new(None, false);
        unlock.job = Some(job);
        unlock.started = Some(std::time::Instant::now());
        h.state_mut().screen = Screen::Locked(Box::new(unlock));
        h.run_steps(2);
        let Screen::Locked(u) = &h.state().screen else {
            panic!("a dead unlock job must not open anything")
        };
        assert!(u.job.is_none(), "the unlock form is usable again");
        assert!(u.started.is_none());
        let error = u.error.as_deref().expect("the failure is reported");
        assert!(!error.contains("canary"), "panic text shown: {error}");
    }

    /// A heavy key derivation found by the worker (not by the display header read) still offers
    /// the override, unticked; choosing another database withdraws it.
    #[test]
    fn heavy_kdf_from_the_worker_offers_the_override() {
        crate::ui_tests::isolate_settings();
        let mut h = Harness::builder()
            .with_size(egui::vec2(1280.0, 800.0))
            .build_eframe(|cc| VaultApp::with_layout(cc, UiLayout::Classic));
        let kdf = Kdf::Aes {
            rounds: 200_000_000,
        };
        let found = kdf.clone();
        let mut unlock = Unlock::new(None, false);
        unlock.job = Some(Job::spawn(&h.ctx, move || {
            Err(UnlockError::HeavyKdf(found))
        }));
        unlock.started = Some(std::time::Instant::now());
        h.state_mut().screen = Screen::Locked(Box::new(unlock));
        for _ in 0..200 {
            h.run_steps(1);
            if let Screen::Locked(u) = &h.state().screen
                && u.job.is_none()
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        h.run_steps(2);
        assert!(h.query_by_label(HEAVY_KDF_OVERRIDE).is_some());
        let Screen::Locked(u) = &mut h.state_mut().screen else {
            panic!("nothing may open")
        };
        assert_eq!(u.heavy_kdf, Some(kdf));
        assert!(!u.allow_heavy_kdf, "never pre-approved");
        let error = u.error.as_deref().unwrap_or_default();
        assert!(error.contains("200,000,000 rounds"), "{error}");
        u.allow_heavy_kdf = true;
        u.database_changed();
        assert!(u.heavy_kdf.is_none() && !u.allow_heavy_kdf);
    }

    /// A Lock request from the tray, a second launch or Axentra Command locks an open vault
    /// through the auto-lock path: unsaved changes are saved first, without a prompt.
    #[test]
    fn external_lock_request_locks_and_saves() {
        use vault_core::keepass::db::fields;
        use vault_core::vault::Credentials;
        let dir = std::env::temp_dir().join(format!("axv-tray-lock-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("synthetic.kdbx");
        let credentials = || Credentials {
            password: Some(Zeroizing::new("tray-lock-master".into())),
            key_file: None,
        };
        let mut db = Database::new();
        db.config.kdf_config = KdfConfig::Aes { rounds: 1000 };
        db.save(
            &mut std::fs::File::create(&path).unwrap(),
            credentials().to_key().unwrap(),
        )
        .unwrap();
        let mut session = Session::new(Vault::open(&path, credentials()).unwrap());
        session
            .vault
            .db
            .root_mut()
            .add_entry()
            .set_unprotected(fields::TITLE, "added before tray lock");
        session.vault.mark_dirty();
        let mut h = Harness::builder()
            .with_size(egui::vec2(1280.0, 800.0))
            .build_eframe(|cc| VaultApp::with_layout(cc, UiLayout::Classic));
        h.state_mut().screen = Screen::Open(Box::new(session));
        h.run_steps(2);
        h.state_mut().lock_requested = true;
        h.run_steps(3);
        assert!(
            matches!(h.state().screen, Screen::Locked(_)),
            "the request locked the vault"
        );
        assert!(!h.state().lock_requested, "the request was consumed");
        let reopened = Vault::open(&path, credentials()).unwrap();
        assert_eq!(
            reopened.db.num_entries(),
            1,
            "unsaved changes were saved first"
        );
        std::fs::remove_dir_all(dir).ok();
    }
}
