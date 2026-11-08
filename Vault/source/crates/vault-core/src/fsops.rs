//! Windows file operations shared by the backup store and saves: moves that never (or always)
//! replace, unique temp files, deletes that clear read-only, free space, volume facts, and
//! folders with a private access list. `std::fs::rename` silently replaces its target, so
//! publishing a backup goes through `MoveFileExW` without `MOVEFILE_REPLACE_EXISTING`.
//!
//! Nothing here claims power-loss safety: that needs a power-cut test.

use std::ffi::c_void;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};

use crate::random_bytes;

const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
const ERROR_SHARING_VIOLATION: i32 = 32;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
const FILE_PERSISTENT_ACLS: u32 = 0x8;
const TOKEN_QUERY: u32 = 0x8;
/// TOKEN_INFORMATION_CLASS::TokenUser.
const TOKEN_USER: i32 = 1;
const SDDL_REVISION_1: u32 = 1;

#[repr(C)]
struct SecurityAttributes {
    length: u32,
    descriptor: *mut c_void,
    inherit: i32,
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    fn GetDiskFreeSpaceExW(
        dir: *const u16,
        available_to_caller: *mut u64,
        total: *mut u64,
        total_free: *mut u64,
    ) -> i32;
    fn GetVolumePathNameW(path: *const u16, out: *mut u16, len: u32) -> i32;
    fn GetVolumeInformationW(
        root: *const u16,
        name: *mut u16,
        name_len: u32,
        serial: *mut u32,
        max_component: *mut u32,
        flags: *mut u32,
        fs_name: *mut u16,
        fs_name_len: u32,
    ) -> i32;
    fn GetDriveTypeW(root: *const u16) -> u32;
    fn CreateDirectoryW(path: *const u16, attrs: *const SecurityAttributes) -> i32;
    fn GetCurrentProcess() -> *mut c_void;
    fn CloseHandle(h: *mut c_void) -> i32;
    fn LocalFree(mem: *mut c_void) -> *mut c_void;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn OpenProcessToken(process: *mut c_void, access: u32, token: *mut *mut c_void) -> i32;
    fn GetTokenInformation(
        token: *mut c_void,
        class: i32,
        info: *mut c_void,
        len: u32,
        returned: *mut u32,
    ) -> i32;
    fn ConvertSidToStringSidW(sid: *mut c_void, out: *mut *mut u16) -> i32;
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
        sddl: *const u16,
        revision: u32,
        sd: *mut *mut c_void,
        size: *mut u32,
    ) -> i32;
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn from_wide(buf: &[u16]) -> String {
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..n])
}

const TEMP_PREFIX: &str = ".~axv-";
const TEMP_SUFFIX: &str = ".tmp";

/// A fresh temp file name, `.~axv-<16 hex>.tmp`.
pub fn temp_name() -> String {
    let mut n = [0u8; 8];
    random_bytes(&mut n);
    format!("{TEMP_PREFIX}{}{TEMP_SUFFIX}", hex::encode(n))
}

/// Whether `name` is exactly a name [`temp_name`] makes. Anything else is left alone.
pub fn is_temp_name(name: &str) -> bool {
    name.strip_prefix(TEMP_PREFIX)
        .and_then(|rest| rest.strip_suffix(TEMP_SUFFIX))
        .is_some_and(|h| h.len() == 16 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
}

/// Creates `path` (failing if anything is there), writes `bytes` and flushes them to disk. A
/// partly written file is removed again.
pub fn write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut f = OpenOptions::new().write(true).create_new(true).open(path)?;
    if let Err(e) = f.write_all(bytes).and_then(|()| f.sync_all()) {
        drop(f);
        let _ = fs::remove_file(path);
        return Err(e);
    }
    Ok(())
}

/// Writes `bytes` to a new, uniquely named temp file in `dir` (see [`write_new`]).
pub fn write_temp_in(dir: &Path, bytes: &[u8]) -> io::Result<PathBuf> {
    let mut last = None;
    for _ in 0..4 {
        let path = dir.join(temp_name());
        match write_new(&path, bytes) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => last = Some(e),
            Err(e) => return Err(e),
        }
    }
    Err(last.unwrap_or_else(|| io::ErrorKind::AlreadyExists.into()))
}

/// Writes `bytes` to a new temp file beside `path`, named `<path>.~axv-<16 hex>.tmp` so a
/// leftover can be traced to the file it was for (see [`write_new`]).
pub fn write_temp_beside(path: &Path, bytes: &[u8]) -> io::Result<PathBuf> {
    let mut last = None;
    for _ in 0..4 {
        let mut name = path.as_os_str().to_owned();
        name.push(temp_name());
        let tmp = PathBuf::from(name);
        match write_new(&tmp, bytes) {
            Ok(()) => return Ok(tmp),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => last = Some(e),
            Err(e) => return Err(e),
        }
    }
    Err(last.unwrap_or_else(|| io::ErrorKind::AlreadyExists.into()))
}

fn move_file(from: &Path, to: &Path, flags: u32) -> io::Result<()> {
    let (from, to) = (wide(from), wide(to));
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), flags) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Renames `from` to `to` only if nothing exists at `to` (`AlreadyExists` otherwise; both files
/// are left as they were). Same volume only.
pub fn move_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    move_file(from, to, 0)
}

/// Renames `from` over `to`, replacing it, and returns only once the rename is on disk.
pub fn move_replace(from: &Path, to: &Path) -> io::Result<()> {
    move_file(from, to, MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)
}

/// Replaces the contents of `path` all at once: a synced temp file beside it, then
/// [`move_replace`]. Readers see the old file or the new one, never a mix.
pub fn write_replacing(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let tmp = write_temp_in(dir, bytes)?;
    move_replace(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

/// Sets or clears the read-only attribute. Backups are read-only so that another app (KeePass
/// "Save") cannot write over one.
pub fn set_readonly(path: &Path, readonly: bool) -> io::Result<()> {
    let mut perms = fs::metadata(path)?.permissions();
    if perms.readonly() != readonly {
        // On Windows this only toggles FILE_ATTRIBUTE_READONLY; the Unix mode concern behind
        // the lint does not apply.
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(readonly);
        fs::set_permissions(path, perms)?;
    }
    Ok(())
}

/// Deletes a file, clearing read-only first. A file that is already gone, or that another
/// program holds open (a sharing violation), is not an error: it returns `Ok(false)` and is
/// left for a later attempt.
pub fn remove_file(path: &Path) -> io::Result<bool> {
    let gone_or_busy = |e: &io::Error| {
        e.kind() == io::ErrorKind::NotFound || e.raw_os_error() == Some(ERROR_SHARING_VIOLATION)
    };
    match fs::symlink_metadata(path) {
        Err(e) if gone_or_busy(&e) => return Ok(false),
        Ok(m) if m.permissions().readonly() => match set_readonly(path, false) {
            Err(e) if gone_or_busy(&e) => return Ok(false),
            r => r?,
        },
        _ => {}
    }
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(e) if gone_or_busy(&e) => Ok(false),
        Err(e) => Err(e),
    }
}

/// Symbolic links, junctions and cloud placeholders are reparse points; the store never deletes
/// or prunes through one.
pub fn is_reparse_point(meta: &fs::Metadata) -> bool {
    meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

/// Bytes free for this user on the volume holding the existing folder `dir`.
pub fn free_space(dir: &Path) -> Option<u64> {
    let dir = wide(dir);
    let mut available = 0u64;
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            dir.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    (ok != 0).then_some(available)
}

/// The mount point of the volume holding `path`, such as `C:\`.
pub fn volume_path(path: &Path) -> Option<String> {
    let path = wide(path);
    let mut buf = vec![0u16; 1024];
    let ok = unsafe { GetVolumePathNameW(path.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) };
    (ok != 0).then(|| from_wide(&buf))
}

/// Whether two paths are on the same volume. Unknown counts as the same, which asks for more
/// free space rather than less.
pub fn same_volume(a: &Path, b: &Path) -> bool {
    match (volume_path(a), volume_path(b)) {
        (Some(a), Some(b)) => a.eq_ignore_ascii_case(&b),
        _ => true,
    }
}

/// The drive type (`GetDriveTypeW`) of the volume holding `path`.
fn drive_type(path: &Path) -> Option<u32> {
    let root = wide(Path::new(&volume_path(path)?));
    Some(unsafe { GetDriveTypeW(root.as_ptr()) })
}

/// Whether `path` is on a removable drive (a USB stick or memory card), by its volume's drive type.
pub fn is_removable(path: &Path) -> bool {
    const DRIVE_REMOVABLE: u32 = 2;
    drive_type(path) == Some(DRIVE_REMOVABLE)
}

/// Whether `path` is on a network share: a UNC path (`\\server\share`) or a mapped drive.
pub fn is_network(path: &Path) -> bool {
    const DRIVE_REMOTE: u32 = 4;
    let text = path.to_string_lossy();
    let unc = (text.starts_with(r"\\") && !text.starts_with(r"\\?\"))
        || text.to_ascii_uppercase().starts_with(r"\\?\UNC\");
    unc || drive_type(path) == Some(DRIVE_REMOTE)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeInfo {
    /// "NTFS", "ReFS", "FAT32", "exFAT", ...
    pub file_system: String,
    /// The file system stores access lists (NTFS, ReFS).
    pub persistent_acls: bool,
}

pub fn volume_info(path: &Path) -> Option<VolumeInfo> {
    let root = wide(Path::new(&volume_path(path)?));
    let mut flags = 0u32;
    let mut fs_name = [0u16; 64];
    let ok = unsafe {
        GetVolumeInformationW(
            root.as_ptr(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut flags,
            fs_name.as_mut_ptr(),
            fs_name.len() as u32,
        )
    };
    (ok != 0).then(|| VolumeInfo {
        file_system: from_wide(&fs_name),
        persistent_acls: flags & FILE_PERSISTENT_ACLS != 0,
    })
}

/// The current user's SID as a string (`S-1-5-21-…`).
fn current_user_sid() -> io::Result<String> {
    unsafe {
        let mut token = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut len = 0u32;
        GetTokenInformation(token, TOKEN_USER, std::ptr::null_mut(), 0, &mut len);
        // u64 cells keep the SID pointer at the start of the buffer aligned.
        let mut buf = vec![0u64; (len as usize).div_ceil(8)];
        let ok = GetTokenInformation(token, TOKEN_USER, buf.as_mut_ptr().cast(), len, &mut len);
        let err = io::Error::last_os_error();
        CloseHandle(token);
        if ok == 0 {
            return Err(err);
        }
        // TOKEN_USER starts with SID_AND_ATTRIBUTES, whose first field is the SID pointer.
        let sid = *buf.as_ptr().cast::<*mut c_void>();
        let mut text: *mut u16 = std::ptr::null_mut();
        if ConvertSidToStringSidW(sid, &mut text) == 0 {
            return Err(io::Error::last_os_error());
        }
        let n = (0..).take_while(|&i| *text.add(i) != 0).count();
        let sid = String::from_utf16_lossy(std::slice::from_raw_parts(text, n));
        LocalFree(text.cast());
        Ok(sid)
    }
}

/// Creates the folder `path` with a protected access list: full control for the current user
/// and SYSTEM only, inherited by everything created inside, nothing inherited from the parent.
/// Other users of a shared drive cannot read the backups. The access list is set as the folder
/// is created, so there is no moment with the parent's.
pub fn create_dir_private(path: &Path) -> io::Result<()> {
    let sid = current_user_sid()?;
    let sddl: Vec<u16> = format!("D:P(A;OICI;FA;;;{sid})(A;OICI;FA;;;SY)")
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let path = wide(path);
    unsafe {
        let mut sd: *mut c_void = std::ptr::null_mut();
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut sd,
            std::ptr::null_mut(),
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let attrs = SecurityAttributes {
            length: std::mem::size_of::<SecurityAttributes>() as u32,
            descriptor: sd,
            inherit: 0,
        };
        let ok = CreateDirectoryW(path.as_ptr(), &attrs);
        let err = io::Error::last_os_error();
        LocalFree(sd);
        if ok == 0 {
            return Err(err);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        let mut n = [0u8; 8];
        random_bytes(&mut n);
        let dir = std::env::temp_dir().join(format!("axvault-fsops-{}", hex::encode(n)));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn temp_names_are_recognised_exactly() {
        let name = temp_name();
        assert!(is_temp_name(&name), "{name}");
        assert_ne!(name, temp_name());
        for other in [
            ".~axv-.tmp",
            ".~axv-0123456789abcdeg.tmp",
            ".~axv-0123456789ABCDEF.tmp",
            ".~axv-0123456789abcdef0.tmp",
            "x.~axv-0123456789abcdef.tmp",
            ".~axv-0123456789abcdef.tmp.txt",
        ] {
            assert!(!is_temp_name(other), "{other}");
        }
    }

    #[test]
    fn no_replace_move_keeps_both_files() {
        let dir = temp_dir();
        let (a, b) = (dir.join("a"), dir.join("b"));
        fs::write(&a, b"new").unwrap();
        fs::write(&b, b"existing").unwrap();
        let err = move_no_replace(&a, &b).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&a).unwrap(), b"new");
        assert_eq!(fs::read(&b).unwrap(), b"existing");

        move_replace(&a, &b).unwrap();
        assert!(!a.exists());
        assert_eq!(fs::read(&b).unwrap(), b"new");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn remove_clears_readonly_and_ignores_missing() {
        let dir = temp_dir();
        let f = dir.join("ro");
        fs::write(&f, b"x").unwrap();
        set_readonly(&f, true).unwrap();
        assert!(fs::metadata(&f).unwrap().permissions().readonly());
        assert!(
            OpenOptions::new().write(true).open(&f).is_err(),
            "nothing can write over it"
        );
        assert!(remove_file(&f).unwrap());
        assert!(!f.exists());
        assert!(!remove_file(&f).unwrap());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn write_new_never_overwrites() {
        let dir = temp_dir();
        let f = dir.join("n");
        write_new(&f, b"one").unwrap();
        assert_eq!(
            write_new(&f, b"two").unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read(&f).unwrap(), b"one");
        write_replacing(&f, b"three").unwrap();
        assert_eq!(fs::read(&f).unwrap(), b"three");
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| is_temp_name(&e.file_name().to_string_lossy()))
            .collect();
        assert!(leftovers.is_empty());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn volume_facts_for_the_temp_folder() {
        let dir = temp_dir();
        assert!(free_space(&dir).is_some_and(|n| n > 0));
        assert!(volume_path(&dir).is_some_and(|v| v.ends_with('\\')));
        assert!(same_volume(&dir, &std::env::temp_dir()));
        assert!(volume_info(&dir).is_some_and(|v| !v.file_system.is_empty()));
        // The temp folder lives on a fixed disk.
        assert!(!is_removable(&dir) && !is_network(&dir));
        assert!(is_network(Path::new(r"\\server\share\Backups")));
        assert!(is_network(Path::new(r"\\?\UNC\server\share")));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn private_dir_is_created_and_usable() {
        let dir = temp_dir();
        let private = dir.join("private");
        create_dir_private(&private).unwrap();
        fs::write(private.join("f"), b"ok").unwrap();
        assert_eq!(fs::read(private.join("f")).unwrap(), b"ok");
        assert_eq!(
            create_dir_private(&private).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        fs::remove_dir_all(dir).unwrap();
    }
}
