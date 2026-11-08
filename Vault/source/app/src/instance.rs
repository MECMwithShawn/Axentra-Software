//! One running Vault per Windows user and settings folder, plus the commands that other programs
//! may send it: a second launch, or Axentra Command's tray menu (`--show`, `--lock`, `--exit`).
//!
//! The process access list set by `hardening.rs` blocks every way into this process, so commands
//! travel over named kernel events that this process waits on. The channel carries only three
//! signals: show the window, lock, or exit (which still asks about unsaved changes). Nothing sent
//! this way can unlock the vault or read anything from it.

use std::ffi::c_void;

type Handle = *mut c_void;

const ERROR_ALREADY_EXISTS: u32 = 183;
const EVENT_MODIFY_STATE: u32 = 0x0002;
const INFINITE: u32 = 0xFFFF_FFFF;
const WAIT_OBJECT_0: u32 = 0;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateMutexW(attrs: *const c_void, initial_owner: i32, name: *const u16) -> Handle;
    fn CreateEventW(
        attrs: *const c_void,
        manual_reset: i32,
        initial_state: i32,
        name: *const u16,
    ) -> Handle;
    fn OpenEventW(access: u32, inherit: i32, name: *const u16) -> Handle;
    fn SetEvent(event: Handle) -> i32;
    fn WaitForMultipleObjects(count: u32, handles: *const Handle, wait_all: i32, ms: u32) -> u32;
    fn CloseHandle(h: Handle) -> i32;
    fn GetLastError() -> u32;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn AllowSetForegroundWindow(process_id: u32) -> i32;
}

/// ASFW_ANY: let the running copy bring its window to the front for this request.
const ASFW_ANY: u32 = u32::MAX;

/// What another program asked the running Vault to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Show,
    Lock,
    Exit,
}

const COMMANDS: [Command; 3] = [Command::Show, Command::Lock, Command::Exit];

impl Command {
    fn suffix(self) -> &'static str {
        match self {
            Command::Show => "Show",
            Command::Lock => "Lock",
            Command::Exit => "Exit",
        }
    }
}

/// The command given on the command line, if any (`--show`, `--lock`, `--exit`). Other
/// arguments are ignored.
pub fn command_from_args(args: impl IntoIterator<Item = String>) -> Option<Command> {
    args.into_iter().find_map(|a| match a.as_str() {
        "--show" => Some(Command::Show),
        "--lock" => Some(Command::Lock),
        "--exit" => Some(Command::Exit),
        _ => None,
    })
}

/// Object names are scoped to the settings folder, so a copy started with a different
/// `%APPDATA%` (tests, an auditor's throwaway profile) is a separate instance and never signals
/// the owner's running copy.
fn base_name() -> String {
    let dir = std::env::var_os("APPDATA")
        .map(|d| d.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    // FNV-1a: stable across runs and builds (std's hasher is randomly keyed).
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in dir.bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("Local\\Axentra.Vault.{hash:016x}")
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// This process owns the instance: its mutex and the three command events.
pub struct Instance {
    _mutex: Handle,
    events: [Handle; 3],
}

// The handles are only waited on and closed by the listener thread.
unsafe impl Send for Instance {}

pub enum Start {
    /// No other copy is running; this one carries on.
    Primary(Instance),
    /// Another copy is running and was sent the command; this one should exit.
    Forwarded,
}

/// Claims the instance, or forwards `command` (default: show the window) to the copy already
/// running.
pub fn start(command: Option<Command>) -> Start {
    let base = base_name();
    let mutex = unsafe {
        CreateMutexW(
            std::ptr::null(),
            0,
            wide(&format!("{base}.Instance")).as_ptr(),
        )
    };
    let already = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    if mutex.is_null() || already {
        if !mutex.is_null() {
            unsafe { CloseHandle(mutex) };
        }
        signal(&base, command.unwrap_or(Command::Show));
        return Start::Forwarded;
    }
    let events = COMMANDS.map(|c| unsafe {
        // Auto-reset, not signalled.
        CreateEventW(
            std::ptr::null(),
            0,
            0,
            wide(&format!("{base}.{}", c.suffix())).as_ptr(),
        )
    });
    Start::Primary(Instance {
        _mutex: mutex,
        events,
    })
}

/// Signals the running copy. The primary creates its events just after its mutex, so retry
/// briefly in case it is still starting.
fn signal(base: &str, command: Command) {
    let name = wide(&format!("{base}.{}", command.suffix()));
    for _ in 0..20 {
        let h = unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr()) };
        if !h.is_null() {
            unsafe {
                if command == Command::Show {
                    AllowSetForegroundWindow(ASFW_ANY);
                }
                SetEvent(h);
                CloseHandle(h);
            }
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

impl Instance {
    /// Waits for commands on a background thread for the life of the process (which keeps the
    /// instance claimed) and hands each one to `on_command`.
    pub fn listen(self, on_command: impl Fn(Command) + Send + 'static) {
        std::thread::Builder::new()
            .name("instance-commands".into())
            .spawn(move || {
                // Move the whole Instance (it is Send); edition-2024 closures would otherwise
                // capture just the raw handle array.
                let instance = self;
                if instance.events.iter().any(|h| h.is_null()) {
                    return;
                }
                loop {
                    let r =
                        unsafe { WaitForMultipleObjects(3, instance.events.as_ptr(), 0, INFINITE) };
                    match r.checked_sub(WAIT_OBJECT_0) {
                        Some(i) if (i as usize) < COMMANDS.len() => {
                            on_command(COMMANDS[i as usize])
                        }
                        _ => return,
                    }
                }
            })
            .expect("spawn instance listener");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tray_commands() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(command_from_args(args(&["vault.exe"])), None);
        assert_eq!(
            command_from_args(args(&["vault.exe", "--lock"])),
            Some(Command::Lock)
        );
        assert_eq!(
            command_from_args(args(&["--other", "--exit"])),
            Some(Command::Exit)
        );
        assert_eq!(command_from_args(args(&["--show"])), Some(Command::Show));
    }

    #[test]
    fn names_are_scoped_and_stable() {
        // Other tests point APPDATA at a temp folder once; do it first so it can't change
        // between the two calls.
        crate::ui_tests::isolate_settings();
        let a = base_name();
        assert_eq!(a, base_name());
        assert!(a.starts_with("Local\\Axentra.Vault."));
    }

    /// A second "launch" in the same namespace reaches the first one's listener.
    #[test]
    fn second_launch_forwards_its_command() {
        // Unique namespace for this test: a dedicated APPDATA value would race other tests, so
        // drive the primitives directly with a test-only base name.
        let base = format!("Local\\Axentra.Vault.test.{}", std::process::id());
        let events = COMMANDS.map(|c| unsafe {
            CreateEventW(
                std::ptr::null(),
                0,
                0,
                wide(&format!("{base}.{}", c.suffix())).as_ptr(),
            )
        });
        assert!(events.iter().all(|h| !h.is_null()));
        signal(&base, Command::Lock);
        let r = unsafe { WaitForMultipleObjects(3, events.as_ptr(), 0, 2000) };
        assert_eq!(r, 1, "the Lock event was signalled");
        for h in events {
            unsafe { CloseHandle(h) };
        }
    }
}
