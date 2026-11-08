#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod ambient;
mod app;
mod breach;
mod chrome;
mod window_chrome;
#[cfg(windows)]
mod clipboard;
#[cfg(target_os="linux")]
#[path="clipboard_linux.rs"]
mod clipboard;
mod columns;
mod dialogs;
#[cfg(windows)]
mod hardening;
#[cfg(target_os="linux")]
#[path="hardening_linux.rs"]
mod hardening;
#[cfg(target_os="linux")]
mod linux;
mod icon;
#[cfg(windows)]
mod instance;
#[cfg(target_os="linux")]
#[path="instance_linux.rs"]
mod instance;
mod intro;
mod jobs;
mod secret_edit;
mod session;
mod settings;
mod siteicons;
mod theme;
mod tray;
#[cfg(test)]
mod ui_tests;

fn main() -> eframe::Result {
    #[cfg(target_os="linux")]
    // GTK is windowless here (eframe owns the window), and Wayland ignores clipboard offers from
    // a client without a focused surface, so Copy did nothing. XWayland lets the hidden owner hold
    // the clipboard and Mutter bridges it to Wayland apps; fall back to Wayland if X is absent.
    { unsafe {libc::umask(0o077);} gtk::gdk::set_allowed_backends("x11,*"); gtk::init().expect("Initialize Linux desktop services"); }
    // One copy per user and settings folder: a second launch (or Axentra Command's tray menu)
    // forwards --show / --lock / --exit to the running copy and exits.
    let command = instance::command_from_args(std::env::args().skip(1));
    let instance = match instance::start(command) {
        instance::Start::Forwarded => return Ok(()),
        // Nothing to lock or exit when Vault wasn't running.
        instance::Start::Primary(_)
            if matches!(
                command,
                Some(instance::Command::Lock | instance::Command::Exit)
            ) =>
        {
            return Ok(());
        }
        instance::Start::Primary(i) => i,
    };
    // Before any window or secret exists: other programs running as this user may not read
    // this process's memory or inject DLLs into it.
    if let Err(e) = hardening::restrict_process_access() {
        eprintln!("process access hardening failed: {e}");
    }
    for failure in hardening::apply_mitigations() {
        eprintln!("mitigation policy not applied: {failure}");
    }
    hardening::set_capture_hidden(settings::hide_from_capture());
    // Floating starts as the compact unlock card; Classic as a full workspace.
    let floating = settings::layout() == settings::Layout::Floating;
    let (size, min) = if floating {
        (app::COMPACT_SIZE, app::COMPACT_SIZE)
    } else {
        (
            eframe::egui::vec2(1120.0, 700.0),
            eframe::egui::vec2(780.0, 520.0),
        )
    };
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_app_id("axentra-vault")
            .with_title("Axentra Vault")
            .with_inner_size(size)
            .with_min_inner_size(min)
            .with_resizable(!floating)
            // Classic opens maximized; Floating maximizes once unlocked.
            .with_maximized(!floating)
            // Both layouts use in-app controls; Floating also uses transparent gaps.
            .with_decorations(false)
            .with_transparent(true)
            // No OS shadow or border around the transparent area of the floating window.
            .with_has_shadow(!floating)
            .with_icon(icon::app_icon()),
        centered: true,
        // OpenGL composites per-pixel alpha on Windows, so the gaps between the floating modules
        // show the desktop. (wgpu's DirectX/Vulkan swapchains present opaque on Windows.)
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "Axentra Vault",
        options,
        Box::new(|cc| Ok(Box::new(app::VaultApp::new(cc, instance)))),
    )
}
