//! Vault's notification-area (tray) icon. Its menu holds only actions, never vault data:
//! Show, Lock vault and Exit. The tooltip says only whether the vault is locked.
//!
//! Axentra Command can host one tray icon for every Axentra app. While it does, it holds the
//! named mutex `Local\Axentra.TrayHost` and shows Vault's actions in its own menu, so Vault
//! hides its icon; once the mutex is gone (Command closed or grouping turned off) the icon comes
//! back. See axentra-command/docs/TRAY-HOST.md.

use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use eframe::egui;
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::instance::Command;

const TRAY_HOST_MUTEX: &str = "Local\\Axentra.TrayHost";
const SYNCHRONIZE: u32 = 0x0010_0000;
const HOST_CHECK: Duration = Duration::from_secs(2);
const ICON_PX: u32 = 32;

#[cfg(windows)]
#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn OpenMutexW(access: u32, inherit: i32, name: *const u16) -> *mut std::ffi::c_void;
    fn CloseHandle(h: *mut std::ffi::c_void) -> i32;
}

/// True while Axentra Command is hosting the suite's tray icons.
#[cfg(windows)]
pub fn command_is_hosting() -> bool {
    let name: Vec<u16> = TRAY_HOST_MUTEX
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let h = OpenMutexW(SYNCHRONIZE, 0, name.as_ptr());
        if h.is_null() {
            false
        } else {
            CloseHandle(h);
            true
        }
    }
}

pub struct Tray {
    icon: TrayIcon,
    lock: MenuItem,
    shown: bool,
    unlocked: Option<bool>,
    checked: Option<Instant>,
}

impl Tray {
    /// Creates the icon on the UI thread (it needs that thread's message loop). Menu clicks and
    /// a left click on the icon arrive as [`Command`]s on `commands`, waking the UI.
    pub fn new(ctx: &egui::Context, commands: Sender<Command>) -> Option<Self> {
        let show = MenuItem::new("Show Axentra Vault", true, None);
        let lock = MenuItem::new("Lock vault", false, None);
        let exit = MenuItem::new("Exit Axentra Vault", true, None);
        let menu = Menu::new();
        menu.append_items(&[&show, &lock, &PredefinedMenuItem::separator(), &exit])
            .ok()?;
        let ids: [(MenuId, Command); 3] = [
            (show.id().clone(), Command::Show),
            (lock.id().clone(), Command::Lock),
            (exit.id().clone(), Command::Exit),
        ];

        let icon =
            tray_icon::Icon::from_rgba(crate::icon::tray_rgba(ICON_PX), ICON_PX, ICON_PX).ok()?;
        let hosted = command_is_hosting();
        let tray = TrayIconBuilder::new()
            .with_tooltip("Axentra Vault: locked")
            .with_icon(icon)
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build()
            .ok()?;
        let _ = tray.set_visible(!hosted);

        let (to_app, wake) = (commands.clone(), ctx.clone());
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            if let Some((_, c)) = ids.iter().find(|(id, _)| *id == e.id) {
                let _ = to_app.send(*c);
                wake.request_repaint();
            }
        }));
        let wake = ctx.clone();
        TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = e
            {
                let _ = commands.send(Command::Show);
                wake.request_repaint();
            }
        }));

        Some(Tray {
            icon: tray,
            lock,
            shown: !hosted,
            unlocked: None,
            checked: Some(Instant::now()),
        })
    }

    /// Called every frame: keeps the tooltip and the Lock item in step with the vault, and the
    /// icon hidden while Command hosts it (checked every couple of seconds).
    pub fn update(&mut self, ctx: &egui::Context, unlocked: bool) {
        if self.unlocked != Some(unlocked) {
            self.lock.set_enabled(unlocked);
            let _ = self.icon.set_tooltip(Some(if unlocked {
                "Axentra Vault: unlocked"
            } else {
                "Axentra Vault: locked"
            }));
            self.unlocked = Some(unlocked);
        }
        if self.checked.is_none_or(|t| t.elapsed() >= HOST_CHECK) {
            let show = !command_is_hosting();
            if show != self.shown && self.icon.set_visible(show).is_ok() {
                self.shown = show;
            }
            self.checked = Some(Instant::now());
        }
        ctx.request_repaint_after(HOST_CHECK);
    }
}

#[cfg(target_os="linux")]
pub fn command_is_hosting()->bool { crate::linux::tray_hosted() }
