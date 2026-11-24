//! File and message dialogs owned by the main window. Without a parent the desktop shows a
//! portal dialog as a separate app (its own taskbar entry and a generic icon); with one it
//! opens modal on top of Vault, as part of Vault.
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WindowHandle,
};
use std::cell::Cell;

thread_local! {
    static MAIN: Cell<Option<(RawWindowHandle, RawDisplayHandle)>> = const { Cell::new(None) };
}

/// Records the main window, every frame (the handle can change if the window is recreated).
pub fn remember(frame: &eframe::Frame) {
    let handles = frame
        .window_handle()
        .ok()
        .zip(frame.display_handle().ok())
        .map(|(w, d)| (w.as_raw(), d.as_raw()));
    MAIN.set(handles);
}

/// The main window as last seen by `remember`. Dialogs run on the UI thread while that window
/// exists, so the borrowed handles outlive every dialog that uses them.
struct Main(RawWindowHandle, RawDisplayHandle);

impl HasWindowHandle for Main {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        // Safety: see `Main`.
        Ok(unsafe { WindowHandle::borrow_raw(self.0) })
    }
}

impl HasDisplayHandle for Main {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        // Safety: see `Main`.
        Ok(unsafe { DisplayHandle::borrow_raw(self.1) })
    }
}

/// The desktop portal identifies its caller through `/proc/<pid>/root`, which a non-dumpable
/// process (`hardening::restrict_process_access`) hides from it, so it refuses the request and
/// rfd falls back to zenity: a separate app that can't attach to a Wayland window. Lift the flag
/// just long enough for the portal to look Vault up, then put it back. Yama's ptrace scope still
/// keeps other programs out of Vault's memory meanwhile.
#[cfg(target_os = "linux")]
fn let_portal_identify_us() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    const LOOKUP: std::time::Duration = std::time::Duration::from_millis(1500);
    // Lookups still in progress; protection comes back only when the last one ends, so a second
    // dialog opened within the window isn't cut short by the first one's timer.
    static LIFTED: AtomicUsize = AtomicUsize::new(0);
    // Safety: plain prctl calls on this process.
    if LIFTED.load(Ordering::SeqCst) == 0 && unsafe { libc::prctl(libc::PR_GET_DUMPABLE, 0, 0, 0, 0) } != 0 {
        return; // Protection was never applied; nothing to restore.
    }
    LIFTED.fetch_add(1, Ordering::SeqCst);
    unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 1, 0, 0, 0) };
    std::thread::spawn(|| {
        std::thread::sleep(LOOKUP);
        if LIFTED.fetch_sub(1, Ordering::SeqCst) == 1 {
            unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) };
        }
    });
}

pub fn file() -> rfd::FileDialog {
    #[cfg(target_os = "linux")]
    let_portal_identify_us();
    match MAIN.get() {
        Some((w, d)) => rfd::FileDialog::new().set_parent(&Main(w, d)),
        None => rfd::FileDialog::new(),
    }
}

pub fn message() -> rfd::MessageDialog {
    match MAIN.get() {
        Some((w, d)) => rfd::MessageDialog::new().set_parent(&Main(w, d)),
        None => rfd::MessageDialog::new(),
    }
}
