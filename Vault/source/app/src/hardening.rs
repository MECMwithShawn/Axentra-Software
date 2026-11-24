//! Defences against other programs running as the same Windows user (the common case for
//! malware): they may not read or write this process's memory, inject code into it, or capture
//! its windows. Nothing here can stop malware running as administrator or SYSTEM.
//!
//! Also reads the Windows security state (antivirus health, Smart App Control) for display.
//! Everything is local; no network.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};

type Handle = *mut c_void;
type Hwnd = *mut c_void;

const DACL_SECURITY_INFORMATION: u32 = 0x4;
const PROTECTED_DACL_SECURITY_INFORMATION: u32 = 0x8000_0000;
const SDDL_REVISION_1: u32 = 1;

// PROCESS_MITIGATION_POLICY values.
const PROCESS_IMAGE_LOAD_POLICY: i32 = 10;
const PROCESS_EXTENSION_POINT_DISABLE_POLICY: i32 = 6;

const WDA_NONE: u32 = 0x0;
const WDA_EXCLUDEFROMCAPTURE: u32 = 0x11;

const WSC_SECURITY_PROVIDER_ANTIVIRUS: u32 = 0x4;
const HKEY_LOCAL_MACHINE: isize = 0x8000_0002_u32 as i32 as isize;
const RRF_RT_REG_DWORD: u32 = 0x10;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetCurrentProcess() -> Handle;
    fn GetCurrentProcessId() -> u32;
    fn SetProcessMitigationPolicy(policy: i32, buffer: *const c_void, len: usize) -> i32;
    fn LocalFree(mem: *mut c_void) -> *mut c_void;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
        sddl: *const u16,
        revision: u32,
        sd: *mut *mut c_void,
        size: *mut u32,
    ) -> i32;
    fn SetKernelObjectSecurity(handle: Handle, info: u32, sd: *const c_void) -> i32;
    fn RegGetValueW(
        key: isize,
        subkey: *const u16,
        value: *const u16,
        flags: u32,
        kind: *mut u32,
        data: *mut c_void,
        len: *mut u32,
    ) -> i32;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SetWindowDisplayAffinity(hwnd: Hwnd, affinity: u32) -> i32;
    fn EnumWindows(cb: extern "system" fn(Hwnd, isize) -> i32, param: isize) -> i32;
    fn GetWindowThreadProcessId(hwnd: Hwnd, pid: *mut u32) -> u32;
}

#[link(name = "wscapi", kind = "raw-dylib")]
unsafe extern "system" {
    fn WscGetSecurityProviderHealth(providers: u32, health: *mut i32) -> i32;
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Replaces this process's access list so other programs running as the same user can only
/// see that it exists, wait for it and end it: no memory reads or writes, no remote threads,
/// no handle duplication. SYSTEM keeps full access. The OWNER RIGHTS entry also removes the
/// owner's implicit right to rewrite this list. (KeePassXC uses the same approach.)
pub fn restrict_process_access() -> Result<(), String> {
    // 0x101001 = SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE.
    let sddl = wide("D:P(A;;GA;;;SY)(A;;0x101001;;;OW)");
    unsafe {
        let mut sd: *mut c_void = std::ptr::null_mut();
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut sd,
            std::ptr::null_mut(),
        ) == 0
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let ok = SetKernelObjectSecurity(
            GetCurrentProcess(),
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            sd,
        );
        let err = std::io::Error::last_os_error();
        LocalFree(sd);
        if ok == 0 {
            return Err(err.to_string());
        }
    }
    Ok(())
}

/// Process mitigations that block common DLL-injection routes without affecting the graphics
/// driver (which is not Microsoft-signed, so "Microsoft-signed DLLs only" is not used):
/// - legacy extension points off: AppInit_DLLs, Winsock layered providers, global window-hook
///   DLLs and legacy IMEs no longer load into this process;
/// - no DLLs from network shares or low-integrity locations, and System32 copies preferred.
pub fn apply_mitigations() -> Vec<String> {
    let mut failed = Vec::new();
    let set = |policy: i32, flags: u32| unsafe {
        SetProcessMitigationPolicy(
            policy,
            (&flags as *const u32).cast(),
            std::mem::size_of::<u32>(),
        ) != 0
    };
    // DisableExtensionPoints = bit 0.
    if !set(PROCESS_EXTENSION_POINT_DISABLE_POLICY, 0b1) {
        failed.push(format!(
            "extension points: {}",
            std::io::Error::last_os_error()
        ));
    }
    // NoRemoteImages | NoLowMandatoryLabelImages | PreferSystem32Images.
    if !set(PROCESS_IMAGE_LOAD_POLICY, 0b111) {
        failed.push(format!("image load: {}", std::io::Error::last_os_error()));
    }
    failed
}

extern "system" fn set_affinity(hwnd: Hwnd, param: isize) -> i32 {
    let mut pid = 0;
    unsafe {
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == GetCurrentProcessId() {
            SetWindowDisplayAffinity(hwnd, param as u32);
        }
    }
    1
}

static CAPTURE_HIDDEN: AtomicBool = AtomicBool::new(true);

/// The user's choice (Settings); applied to every window about once a second.
pub fn set_capture_hidden(hidden: bool) {
    CAPTURE_HIDDEN.store(hidden, Ordering::Relaxed);
}

pub fn capture_hidden() -> bool {
    CAPTURE_HIDDEN.load(Ordering::Relaxed)
}

/// Hides (or shows) every window of this app from screenshots, screen recording and screen
/// sharing. Captures see a black or empty area instead. Cheap; called about once a second so
/// new windows (the entry editor) are covered too.
pub fn exclude_from_capture(exclude: bool) {
    let affinity = if exclude {
        WDA_EXCLUDEFROMCAPTURE
    } else {
        WDA_NONE
    };
    unsafe {
        EnumWindows(set_affinity, affinity as isize);
    }
}

/// Health of the antivirus registered with Windows Security (Defender or another product).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Antivirus {
    Good,
    /// Off, out of date, or snoozed.
    Attention,
    Unknown,
}

/// Smart App Control (Windows 11): blocks untrusted apps before they run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmartAppControl {
    On,
    Evaluation,
    Off,
    /// Not available on this Windows version.
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SecurityStatus {
    pub antivirus: Antivirus,
    pub smart_app_control: SmartAppControl,
}

pub fn security_status() -> SecurityStatus {
    let antivirus = unsafe {
        let mut health = -1;
        if WscGetSecurityProviderHealth(WSC_SECURITY_PROVIDER_ANTIVIRUS, &mut health) != 0 {
            Antivirus::Unknown
        } else {
            // WSC_SECURITY_PROVIDER_HEALTH_GOOD = 0; NOTMONITORED = 1; POOR = 2; SNOOZE = 3.
            match health {
                0 => Antivirus::Good,
                1..=3 => Antivirus::Attention,
                _ => Antivirus::Unknown,
            }
        }
    };
    let subkey = wide(r"SYSTEM\CurrentControlSet\Control\CI\Policy");
    let value = wide("VerifiedAndReputablePolicyState");
    let mut data: u32 = 0;
    let mut len = std::mem::size_of::<u32>() as u32;
    let smart_app_control = unsafe {
        if RegGetValueW(
            HKEY_LOCAL_MACHINE,
            subkey.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&mut data as *mut u32).cast(),
            &mut len,
        ) != 0
        {
            SmartAppControl::Unavailable
        } else {
            match data {
                1 => SmartAppControl::On,
                2 => SmartAppControl::Evaluation,
                _ => SmartAppControl::Off,
            }
        }
    };
    SecurityStatus {
        antivirus,
        smart_app_control,
    }
}

impl SecurityStatus {
    /// One line for the Settings page.
    pub fn summary(&self) -> String {
        let av = match self.antivirus {
            Antivirus::Good => "Antivirus: on",
            Antivirus::Attention => "Antivirus: needs attention",
            Antivirus::Unknown => "Antivirus: unknown",
        };
        let sac = match self.smart_app_control {
            SmartAppControl::On => "Smart App Control: on",
            SmartAppControl::Evaluation => "Smart App Control: evaluation",
            SmartAppControl::Off => "Smart App Control: off",
            SmartAppControl::Unavailable => "Smart App Control: not available",
        };
        format!("{av} · {sac}")
    }

    /// A warning worth showing when the vault opens, if any.
    pub fn warning(&self) -> Option<&'static str> {
        (self.antivirus == Antivirus::Attention).then_some(
            "Windows Security reports that antivirus protection is off or needs attention. Malware running on this PC can read an unlocked vault.",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[link(name = "kernel32", kind = "raw-dylib")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
        fn CloseHandle(h: Handle) -> i32;
    }

    /// Runs in a child test process (see below) so the stricter access list does not leak into
    /// the rest of the test binary.
    #[test]
    #[ignore]
    fn child_restricted_process_refuses_memory_access() {
        const PROCESS_VM_READ: u32 = 0x10;
        const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
        let pid = unsafe { GetCurrentProcessId() };
        let open = |access| unsafe {
            let h = OpenProcess(access, 0, pid);
            if !h.is_null() {
                CloseHandle(h);
            }
            !h.is_null()
        };
        assert!(open(PROCESS_VM_READ), "readable before hardening");
        restrict_process_access().unwrap();
        assert!(!open(PROCESS_VM_READ), "memory reads refused");
        assert!(
            open(PROCESS_QUERY_LIMITED_INFORMATION),
            "Task Manager can still see it"
        );
        assert!(apply_mitigations().is_empty(), "{:?}", apply_mitigations());
    }

    #[test]
    fn hardening_blocks_memory_reads() {
        let exe = std::env::current_exe().unwrap();
        let status = std::process::Command::new(exe)
            .args([
                "hardening::tests::child_restricted_process_refuses_memory_access",
                "--ignored",
                "--exact",
                "--test-threads=1",
            ])
            .status()
            .unwrap();
        assert!(status.success());
    }

    #[test]
    fn security_status_reads_without_error() {
        let s = security_status();
        assert!(!s.summary().is_empty());
    }
}

pub fn capture_supported()->bool{true}
pub fn capture_hint()->&'static str{"Screen-capture malware, screenshots and screen sharing see a blank window"}
