//! Clipboard copies that stay out of Windows clipboard history and cloud clipboard, and are
//! cleared after a timeout if nothing else has been copied since.

use std::ffi::c_void;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::GlobalFree;
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardSequenceNumber, OpenClipboard,
    RegisterClipboardFormatW, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows_sys::Win32::System::Ole::CF_UNICODETEXT;
use zeroize::Zeroize;

pub const CLEAR_AFTER: Duration = Duration::from_secs(12);

/// A pending auto-clear for something this app copied.
pub struct PendingClear {
    sequence: u32,
    at: Instant,
}

impl PendingClear {
    pub fn due(&self) -> bool {
        Instant::now() >= self.at
    }

    pub fn remaining(&self) -> Duration {
        self.at.saturating_duration_since(Instant::now())
    }

    /// Clears the clipboard only if it still holds what we copied.
    pub fn clear(self) {
        unsafe {
            if GetClipboardSequenceNumber() == self.sequence
                && OpenClipboard(std::ptr::null_mut()) != 0
            {
                EmptyClipboard();
                CloseClipboard();
            }
        }
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Moves `bytes` into a movable global block the clipboard can own.
unsafe fn global_from(bytes: &[u8]) -> Option<*mut c_void> {
    unsafe {
        let h = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1));
        if h.is_null() {
            return None;
        }
        let p = GlobalLock(h) as *mut u8;
        if p.is_null() {
            GlobalFree(h);
            return None;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
        GlobalUnlock(h);
        Some(h)
    }
}

unsafe fn set_marker(format_name: &str, data: &[u8]) {
    unsafe {
        let fmt = RegisterClipboardFormatW(wide(format_name).as_ptr());
        if fmt != 0
            && let Some(h) = global_from(data)
            && SetClipboardData(fmt, h).is_null()
        {
            GlobalFree(h);
        }
    }
}

/// Copies `text` as a secret. Returns the pending auto-clear, or None if the clipboard was busy.
pub fn copy_secret(text: &str) -> Option<PendingClear> {
    let mut utf16: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let mut bytes: Vec<u8> = utf16.iter().flat_map(|c| c.to_le_bytes()).collect();
    utf16.zeroize();

    let result = unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            None
        } else {
            EmptyClipboard();
            let ok = match global_from(&bytes) {
                Some(h) => {
                    if SetClipboardData(CF_UNICODETEXT as u32, h).is_null() {
                        GlobalFree(h);
                        false
                    } else {
                        true
                    }
                }
                None => false,
            };
            // Documented opt-outs: clipboard managers, Win+V history, and cloud clipboard sync.
            set_marker("ExcludeClipboardContentFromMonitorProcessing", &[0]);
            set_marker("CanIncludeInClipboardHistory", &0u32.to_le_bytes());
            set_marker("CanUploadToCloudClipboard", &0u32.to_le_bytes());
            CloseClipboard();
            ok.then(|| PendingClear {
                sequence: GetClipboardSequenceNumber(),
                at: Instant::now() + CLEAR_AFTER,
            })
        }
    };
    bytes.zeroize();
    result
}
