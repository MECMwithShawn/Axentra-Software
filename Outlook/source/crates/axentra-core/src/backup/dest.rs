//! Where backups are saved.
//!
//! In order: `AXENTRA_BACKUP_DIR` (development and tests), the folder the
//! user picked, the OneDrive folder that the separate Axentra OneDrive app
//! syncs (its `mount_point` in `~/.config/axentra/onedrive/config.toml`),
//! `~/OneDrive` when it exists, else `~/Documents`. Backups go into
//! `Axentra/Outlook Backups` inside that folder.
//!
//! Axentra OneDrive shows the cloud as a FUSE file system mounted on its
//! `mount_point`. While it isn't running that is a plain local folder, and
//! Axentra OneDrive refuses to mount over a folder that has files in it, so
//! nothing is written there unless the mount is live ([`Destination::ready`]).

use std::path::{Path, PathBuf};

use super::{BackupError, BackupResult};

/// Environment variable that overrides every other choice.
pub const ENV_OVERRIDE: &str = "AXENTRA_BACKUP_DIR";

/// Why a destination was chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DestKind {
    /// `AXENTRA_BACKUP_DIR`.
    Override,
    /// A folder the user chose.
    Custom,
    /// The Axentra OneDrive folder (configured or `~/OneDrive`).
    OneDrive,
    /// `~/Documents`, because no OneDrive folder was found.
    Documents,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Destination {
    /// The chosen base folder.
    pub base: PathBuf,
    /// `<base>/Axentra/Outlook Backups`, where the files are.
    pub dir: PathBuf,
    pub kind: DestKind,
    /// The folder is inside the OneDrive folder, so Axentra OneDrive syncs it.
    pub in_onedrive: bool,
    /// The Axentra OneDrive mount point `dir` is under: backups are written
    /// only while it is mounted.
    pub needs_mount: Option<PathBuf>,
}

impl Destination {
    /// Whether the folder can take a backup now: always, unless it lies in
    /// the Axentra OneDrive mount and that isn't mounted.
    pub fn ready(&self) -> BackupResult<()> {
        match &self.needs_mount {
            Some(mount) if !is_mounted(mount) => Err(BackupError::Io(
                "Axentra OneDrive isn't running, so the backup wasn't saved. Start Axentra OneDrive and try again.".into(),
            )),
            _ => Ok(()),
        }
    }

    /// The mount this folder needs is missing right now.
    pub fn waiting_for_mount(&self) -> bool {
        self.needs_mount.as_deref().is_some_and(|m| !is_mounted(m))
    }
}

/// Whether a FUSE file system (Axentra OneDrive's) is mounted at `path`.
pub fn is_mounted(path: &Path) -> bool {
    match std::fs::read_to_string("/proc/self/mountinfo") {
        Ok(info) => {
            let canonical = std::fs::canonicalize(path).ok();
            fuse_mounted_at(&info, path) || canonical.is_some_and(|c| fuse_mounted_at(&info, &c))
        }
        // No /proc: a mount point is on another device than its parent.
        Err(_) => other_device_than_parent(path),
    }
}

#[cfg(unix)]
fn other_device_than_parent(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let (Some(parent), Ok(meta)) = (path.parent(), std::fs::metadata(path)) else { return false };
    std::fs::metadata(parent).is_ok_and(|p| p.dev() != meta.dev())
}

#[cfg(not(unix))]
fn other_device_than_parent(_path: &Path) -> bool {
    false
}

/// Whether `mountinfo` (the text of `/proc/self/mountinfo`) lists a FUSE
/// file system mounted exactly at `path`.
pub fn fuse_mounted_at(mountinfo: &str, path: &Path) -> bool {
    mountinfo.lines().any(|line| {
        // "36 35 98:0 /root /mount/point opts… - fstype source opts"
        let Some((left, right)) = line.split_once(" - ") else { return false };
        let Some(point) = left.split(' ').nth(4) else { return false };
        let fstype = right.split(' ').next().unwrap_or_default();
        (fstype == "fuse" || fstype.starts_with("fuse.")) && Path::new(&unescape_mount(point)) == path
    })
}

/// Undo mountinfo's octal escapes (`\040` for a space, …).
fn unescape_mount(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\'
            && i + 3 < b.len()
            && (b'0'..=b'3').contains(&b[i + 1])
            && b[i + 2..i + 4].iter().all(|c| (b'0'..=b'7').contains(c))
        {
            out.push((b[i + 1] - b'0') * 64 + (b[i + 2] - b'0') * 8 + (b[i + 3] - b'0'));
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The inputs to [`resolve`], read from the system by [`DestEnv::from_system`]
/// and built by hand in tests.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DestEnv {
    pub override_dir: Option<PathBuf>,
    pub home: Option<PathBuf>,
    /// `mount_point` from the Axentra OneDrive configuration.
    pub onedrive_mount: Option<PathBuf>,
    /// Whether `~/OneDrive` exists (checked by `from_system`).
    pub home_onedrive_exists: bool,
    /// `onedrive_mount` is Axentra OneDrive's FUSE mount point (it came
    /// from its configuration), so writing there needs the mount to be live.
    pub mount_is_fuse: bool,
}

impl DestEnv {
    /// Read the environment. The OneDrive configuration is only read when
    /// `AXENTRA_BACKUP_DIR` isn't set.
    pub fn from_system() -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from).filter(|p| p.is_absolute());
        let override_dir = std::env::var_os(ENV_OVERRIDE).map(PathBuf::from).filter(|p| !p.as_os_str().is_empty());
        if override_dir.is_some() {
            return DestEnv { override_dir, home, ..Default::default() };
        }
        let config_dir = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| home.as_ref().map(|h| h.join(".config")));
        let onedrive_mount = config_dir
            .map(|c| c.join("axentra").join("onedrive").join("config.toml"))
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| parse_mount_point(&text, home.as_deref()));
        let home_onedrive_exists = home.as_ref().is_some_and(|h| h.join("OneDrive").is_dir());
        let mount_is_fuse = onedrive_mount.is_some();
        DestEnv { override_dir: None, home, onedrive_mount, home_onedrive_exists, mount_is_fuse }
    }

    /// The OneDrive folder, if there is one.
    pub fn onedrive(&self) -> Option<PathBuf> {
        self.onedrive_mount
            .clone()
            .or_else(|| self.home.as_ref().filter(|_| self.home_onedrive_exists).map(|h| h.join("OneDrive")))
    }
}

/// `<base>/Axentra/Outlook Backups`.
pub fn backup_dir(base: &Path) -> PathBuf {
    base.join("Axentra").join("Outlook Backups")
}

/// Expand `~` / `~/…` and make relative paths relative to `home`.
pub fn expand(path: &str, home: Option<&Path>) -> Option<PathBuf> {
    let path = path.trim();
    if path.is_empty() {
        return None;
    }
    let expanded = if path == "~" {
        home?.to_path_buf()
    } else if let Some(rest) = path.strip_prefix("~/") {
        home?.join(rest)
    } else {
        PathBuf::from(path)
    };
    if expanded.is_absolute() { Some(expanded) } else { home.map(|h| h.join(expanded)) }
}

/// The `mount_point` value of an Axentra OneDrive `config.toml` (a simple
/// line parse: `mount_point = "~/OneDrive"`, single or double quotes or
/// bare, `#` comments). Only top-level keys count.
pub fn parse_mount_point(toml: &str, home: Option<&Path>) -> Option<PathBuf> {
    for line in toml.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            // A table starts: later keys belong to it.
            return None;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        if key.trim() != "mount_point" {
            continue;
        }
        let value = value.trim();
        let text = if let Some(rest) = value.strip_prefix('"') {
            // Basic string: honour \" and \\ escapes.
            let mut out = String::new();
            let mut chars = rest.chars();
            loop {
                match chars.next()? {
                    '"' => break,
                    '\\' => match chars.next()? {
                        'n' => out.push('\n'),
                        't' => out.push('\t'),
                        c => out.push(c),
                    },
                    c => out.push(c),
                }
            }
            out
        } else if let Some(rest) = value.strip_prefix('\'') {
            rest.split_once('\'')?.0.to_string()
        } else {
            value.split('#').next().unwrap_or_default().trim().to_string()
        };
        return expand(&text, home);
    }
    None
}

/// Pick the destination. `custom` is the folder from Settings, if any.
pub fn resolve(custom: Option<&str>, env: &DestEnv) -> Destination {
    let onedrive = env.onedrive();
    let in_onedrive = |p: &Path| onedrive.as_ref().is_some_and(|o| p.starts_with(o));
    // The FUSE mount a folder lies in (or *is*: writing the mount point
    // itself while unmounted would block the mount just the same).
    let fuse = env.onedrive_mount.as_ref().filter(|_| env.mount_is_fuse);
    let needs_mount = |p: &Path| fuse.filter(|m| p.starts_with(m)).cloned();
    let make = |base: PathBuf, kind: DestKind, in_onedrive: bool| Destination {
        dir: backup_dir(&base),
        needs_mount: needs_mount(&base),
        base,
        kind,
        in_onedrive,
    };
    if let Some(base) = env.override_dir.clone() {
        let in_od = in_onedrive(&base);
        return make(base, DestKind::Override, in_od);
    }
    if let Some(base) = custom.and_then(|c| expand(c, env.home.as_deref())) {
        let in_od = in_onedrive(&base);
        return make(base, DestKind::Custom, in_od);
    }
    if let Some(base) = onedrive {
        return make(base, DestKind::OneDrive, true);
    }
    let base = env.home.as_ref().map(|h| h.join("Documents")).unwrap_or_else(|| PathBuf::from("Documents"));
    make(base, DestKind::Documents, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> PathBuf {
        PathBuf::from("/home/ana")
    }

    #[test]
    fn mount_point_parsing() {
        let h = home();
        let h = Some(h.as_path());
        assert_eq!(parse_mount_point("mount_point = \"~/OneDrive\"\n", h), Some(PathBuf::from("/home/ana/OneDrive")));
        assert_eq!(parse_mount_point("client_id = \"x\"\nmount_point='/data/My OneDrive' # synced\n", h), Some(PathBuf::from("/data/My OneDrive")));
        assert_eq!(parse_mount_point("  mount_point   =   /mnt/od   # comment", h), Some(PathBuf::from("/mnt/od")));
        assert_eq!(parse_mount_point("mount_point = \"~\"", h), Some(PathBuf::from("/home/ana")));
        assert_eq!(parse_mount_point("mount_point = \"OneDrive\"", h), Some(PathBuf::from("/home/ana/OneDrive")));
        assert_eq!(parse_mount_point("mount_point = \"C:\\\\x\\\"y\"", h), Some(PathBuf::from("/home/ana/C:\\x\"y")));
        // Commented out, empty, inside a table, or missing.
        assert_eq!(parse_mount_point("# mount_point = \"/x\"\n", h), None);
        assert_eq!(parse_mount_point("mount_point = \"\"", h), None);
        assert_eq!(parse_mount_point("[sync]\nmount_point = \"/x\"", h), None);
        assert_eq!(parse_mount_point("mount_points = \"/x\"", h), None);
        assert_eq!(parse_mount_point("mount_point = \"/unterminated", h), None);
        assert_eq!(parse_mount_point("mount_point = \"~/OneDrive\"", None), None);
    }

    #[test]
    fn resolution_order() {
        let mut env = DestEnv { home: Some(home()), ..Default::default() };
        let d = resolve(None, &env);
        assert_eq!((d.kind, d.in_onedrive), (DestKind::Documents, false));
        assert_eq!(d.dir, PathBuf::from("/home/ana/Documents/Axentra/Outlook Backups"));

        env.home_onedrive_exists = true;
        let d = resolve(None, &env);
        assert_eq!((d.kind, d.base.clone()), (DestKind::OneDrive, PathBuf::from("/home/ana/OneDrive")));

        env.onedrive_mount = Some(PathBuf::from("/data/od"));
        let d = resolve(None, &env);
        assert_eq!((d.kind, d.base.clone(), d.in_onedrive), (DestKind::OneDrive, PathBuf::from("/data/od"), true));

        let d = resolve(Some("/data/od/Work"), &env);
        assert_eq!((d.kind, d.in_onedrive), (DestKind::Custom, true));
        let d = resolve(Some("~/Backups"), &env);
        assert_eq!((d.kind, d.base.clone(), d.in_onedrive), (DestKind::Custom, PathBuf::from("/home/ana/Backups"), false));
        // A blank custom folder means "the default".
        assert_eq!(resolve(Some("  "), &env).kind, DestKind::OneDrive);

        env.override_dir = Some(PathBuf::from("/tmp/bk"));
        let d = resolve(Some("/elsewhere"), &env);
        assert_eq!((d.kind, d.dir), (DestKind::Override, PathBuf::from("/tmp/bk/Axentra/Outlook Backups")));
    }

    const MOUNTINFO: &str = "\
22 1 8:2 / / rw,relatime shared:1 - ext4 /dev/sda2 rw
301 34 0:59 / /home/ana/OneDrive rw,nosuid,nodev,noatime shared:471 - fuse.axentra-onedrive axentra-onedrive rw,user_id=1000
302 22 8:3 / /data/od rw,relatime shared:2 - ext4 /dev/sda3 rw
303 22 0:60 / /home/ana/My\\040Drive rw shared:3 - fuse sshfs rw
";

    #[test]
    fn fuse_mounts_are_found_in_mountinfo() {
        assert!(fuse_mounted_at(MOUNTINFO, Path::new("/home/ana/OneDrive")));
        assert!(fuse_mounted_at(MOUNTINFO, Path::new("/home/ana/My Drive")), "octal escapes");
        // A folder inside the mount isn't the mount point.
        assert!(!fuse_mounted_at(MOUNTINFO, Path::new("/home/ana/OneDrive/Axentra")));
        // Mounted, but a local disk: Axentra OneDrive would still mount over it.
        assert!(!fuse_mounted_at(MOUNTINFO, Path::new("/data/od")));
        assert!(!fuse_mounted_at(MOUNTINFO, Path::new("/home/ana")));
        assert!(!fuse_mounted_at("garbage\n\n", Path::new("/home/ana/OneDrive")));
    }

    #[test]
    fn only_the_axentra_onedrive_mount_needs_to_be_live() {
        let fuse = PathBuf::from("/home/ana/OneDrive");
        let mut env = DestEnv { home: Some(home()), onedrive_mount: Some(fuse.clone()), mount_is_fuse: true, ..Default::default() };
        assert_eq!(resolve(None, &env).needs_mount, Some(fuse.clone()));
        assert_eq!(resolve(Some("~/OneDrive/Work"), &env).needs_mount, Some(fuse.clone()));
        assert_eq!(resolve(Some("/media/usb"), &env).needs_mount, None);
        // ~/OneDrive found on disk (another sync app) or the preview server's
        // stand-in folder: an ordinary folder.
        env.mount_is_fuse = false;
        assert_eq!(resolve(None, &env).needs_mount, None);
        let env = DestEnv { home: Some(home()), home_onedrive_exists: true, ..Default::default() };
        assert_eq!(resolve(None, &env).needs_mount, None);
    }

    #[test]
    fn nothing_is_written_while_axentra_onedrive_is_not_mounted() {
        let mount = std::env::temp_dir().join(format!("axentra-dest-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&mount);
        std::fs::create_dir_all(&mount).unwrap();
        let env = DestEnv { home: Some(home()), onedrive_mount: Some(mount.clone()), mount_is_fuse: true, ..Default::default() };
        let d = resolve(None, &env);
        assert!(!is_mounted(&mount));
        assert!(d.waiting_for_mount());
        let err = d.ready().unwrap_err();
        assert!(err.to_string().starts_with("Axentra OneDrive isn't running, so the backup wasn't saved."), "{err}");
        assert_eq!(std::fs::read_dir(&mount).unwrap().count(), 0, "the mount point stays empty");
        // Any other folder is always ready.
        let d = resolve(Some("/media/usb"), &env);
        assert!(d.ready().is_ok() && !d.waiting_for_mount());
        std::fs::remove_dir_all(&mount).unwrap();
    }
}
