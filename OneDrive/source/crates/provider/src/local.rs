//! A folder on a local drive (an external USB or Thunderbolt disk, say) as a
//! [`MirrorDestination`].
//!
//! Adds only, like every destination: a file is copied to a temporary name
//! beside where it goes, flushed, and hard-linked into place, which never
//! replaces anything already there. Nothing is ever deleted, renamed over
//! or truncated; the only file removed is the temporary name this process
//! just made.
//!
//! **The mount guard.** An external drive can drop off mid-run. Its mount
//! point then becomes an empty folder on the system disk (or goes away),
//! and writes meant for the drive would quietly land there. So the
//! destination remembers the device and inode of its root when opened, and
//! before every call checks the root is still that same folder on that same
//! device; each new folder and file is checked again after it is made.
//! Anything else is [`ProviderError::Offline`], which stops a run without
//! marking files failed. [`LocalFolderDestination::on_mount`] also refuses a
//! root that isn't a mount point, or is on the same filesystem as `/`.
//!
//! "Claimed" digests here are not taken on trust: [`stat`] reads the file
//! back from the drive (after asking the kernel to drop its cached pages)
//! and hashes it.
//!
//! [`stat`]: MirrorDestination::stat

use std::fs::{File, OpenOptions};
use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use crate::digests::ContentDigests;
use crate::error::{PResult, ProviderError};
use crate::path::{check_name, PathError, RelPath};
use crate::staging::{parent_dir, verify_and_publish, StagedTemp, TEMP_PREFIX};
use crate::traits::{
    Account, Capabilities, DestEntry, ItemKind, MirrorDestination, RemoteStat,
};

pub struct LocalFolderDestination {
    root: PathBuf,
    /// Device and inode of `root` when opened.
    dev: u64,
    ino: u64,
}

impl LocalFolderDestination {
    /// `root` must be the mount point of a filesystem other than the one
    /// `/` is on, such as `/media/me/Backup`.
    pub fn on_mount(root: &Path) -> PResult<Self> {
        let d = Self::new(root)?;
        let parent = root
            .parent()
            .ok_or_else(|| ProviderError::Other("/ can't be a mirror destination".into()))?;
        if std::fs::metadata(parent)?.dev() == d.dev {
            return Err(ProviderError::Other(format!(
                "{} is not where a drive is mounted",
                root.display()
            )));
        }
        if std::fs::metadata("/")?.dev() == d.dev {
            return Err(ProviderError::Other(format!(
                "{} is on the system disk",
                root.display()
            )));
        }
        Ok(d)
    }

    /// Any existing, absolute, real folder, with no mount checks beyond
    /// the guard (for tests, and folders on the system disk on purpose).
    pub fn new(root: &Path) -> PResult<Self> {
        if !root.is_absolute() {
            return Err(ProviderError::Other(format!(
                "{} is not an absolute path",
                root.display()
            )));
        }
        let meta = std::fs::symlink_metadata(root)?;
        if meta.file_type().is_symlink() {
            return Err(PathError::Symlink(root.into()).into());
        }
        if !meta.is_dir() {
            return Err(PathError::NotAFolder(root.into()).into());
        }
        Ok(Self {
            root: root.to_path_buf(),
            dev: meta.dev(),
            ino: meta.ino(),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The root is still the folder it was when opened.
    fn guard(&self) -> PResult<()> {
        match std::fs::symlink_metadata(&self.root) {
            Ok(m) if m.dev() == self.dev && m.ino() == self.ino => Ok(()),
            _ => Err(self.gone()),
        }
    }

    fn gone(&self) -> ProviderError {
        ProviderError::Offline(format!(
            "the destination drive is no longer mounted at {}",
            self.root.display()
        ))
    }

    /// Something just made (or opened) is on the destination's device.
    fn on_device(&self, m: &std::fs::Metadata) -> PResult<()> {
        if m.dev() == self.dev {
            Ok(())
        } else {
            Err(self.gone())
        }
    }

    fn io(&self, e: std::io::Error) -> ProviderError {
        match e.raw_os_error() {
            Some(libc::EIO | libc::ENOTCONN | libc::ENODEV | libc::ENXIO) => {
                ProviderError::Offline(format!(
                    "the destination drive at {} stopped responding: {e}",
                    self.root.display()
                ))
            }
            _ if e.kind() == std::io::ErrorKind::NotFound => {
                ProviderError::NotFound(e.to_string())
            }
            _ => ProviderError::Local(e),
        }
    }

    /// Open the file at `rel` for reading, without following a link.
    fn open_file(&self, rel: &RelPath) -> PResult<File> {
        let path = rel.under(&self.root);
        let f = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&path)
            .map_err(|e| match e.raw_os_error() {
                Some(libc::ELOOP) => ProviderError::Conflict(format!("{rel} is a symbolic link")),
                _ => self.io(e),
            })?;
        let m = f.metadata().map_err(|e| self.io(e))?;
        self.on_device(&m)?;
        if !m.is_file() {
            return Err(ProviderError::Conflict(format!("{rel} is not a plain file")));
        }
        drop_cache(&f);
        Ok(f)
    }
}

/// Ask the kernel to forget its cached pages of `f`, so what is read next
/// comes from the drive. Advice only; failures don't matter.
fn drop_cache(f: &File) {
    // SAFETY: a valid open descriptor; the call only reads its arguments.
    unsafe {
        libc::posix_fadvise(f.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED);
    }
}

fn hash_file(mut f: File) -> std::io::Result<ContentDigests> {
    ContentDigests::of_reader(&mut f)
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> std::io::Result<T> + Send + 'static,
) -> std::io::Result<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(std::io::Error::other)?
}

impl MirrorDestination for LocalFolderDestination {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            change_feed: false,
            quick_xor_hash: true,
            sha1: true,
            sha256: true,
            create_folder: true,
            revisions: false,
            round_trip: true,
        }
    }

    fn name_problem(&self, name: &str, _kind: ItemKind) -> Option<&'static str> {
        if name.starts_with(TEMP_PREFIX) {
            return Some("names starting with .axentra-part- are kept for temporary files");
        }
        None
    }

    async fn account(&self) -> PResult<Account> {
        self.guard()?;
        Ok(Account {
            provider: "local".into(),
            id: format!("{}:{}", self.dev, self.ino),
            display_name: Some(self.root.display().to_string()),
            email: None,
            kind: None,
        })
    }

    /// Leftover temporary files are left out.
    async fn list(&self, folder: &RelPath) -> PResult<Vec<DestEntry>> {
        self.guard()?;
        let dir = folder.under(&self.root);
        let meta = std::fs::symlink_metadata(&dir).map_err(|e| self.io(e))?;
        self.on_device(&meta)?;
        if !meta.is_dir() {
            return Err(ProviderError::Other(format!("{folder} is not a folder")));
        }
        let mut out = Vec::new();
        for ent in std::fs::read_dir(&dir).map_err(|e| self.io(e))? {
            let ent = ent.map_err(|e| self.io(e))?;
            let Ok(name) = ent.file_name().into_string() else {
                // Not UTF-8: no source name can be the same.
                continue;
            };
            if name.starts_with(TEMP_PREFIX) {
                continue;
            }
            let m = std::fs::symlink_metadata(ent.path()).map_err(|e| self.io(e))?;
            let (kind, size) = if m.is_dir() {
                (ItemKind::Folder, None)
            } else if m.is_file() {
                (ItemKind::File, Some(m.len()))
            } else {
                // A link or device in the way: a "file" that can't be compared.
                (ItemKind::File, None)
            };
            out.push(DestEntry {
                name,
                uid: m.ino().to_string(),
                kind,
                size,
            });
        }
        Ok(out)
    }

    async fn ensure_folder(&self, folder: &RelPath) -> PResult<String> {
        self.guard()?;
        let mut at = self.root.clone();
        let mut ino = self.ino;
        for seg in folder.segments() {
            check_name(seg)?;
            at.push(seg);
            match std::fs::DirBuilder::new().mode(0o755).create(&at) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(self.io(e)),
            }
            let m = std::fs::symlink_metadata(&at).map_err(|e| self.io(e))?;
            self.on_device(&m)?;
            if !m.is_dir() {
                return Err(ProviderError::Conflict(format!(
                    "{} is not a folder",
                    at.display()
                )));
            }
            ino = m.ino();
        }
        Ok(ino.to_string())
    }

    /// Copied to a temporary file, flushed, then linked in under `name`
    /// (never over anything). The file keeps `local`'s modification time.
    async fn upload_new(&self, folder: &RelPath, name: &str, local: &Path) -> PResult<RemoteStat> {
        self.guard()?;
        check_name(name)?;
        if let Some(why) = self.name_problem(name, ItemKind::File) {
            return Err(ProviderError::Other(why.into()));
        }
        let dir = folder.under(&self.root);
        let dest = dir.join(name);
        if std::fs::symlink_metadata(&dest).is_ok() {
            return Err(ProviderError::Conflict(format!("{folder}/{name} already exists")));
        }
        let mut tmp = StagedTemp::create_in(&dir).map_err(|e| self.io(e))?;
        let m = tmp.file().metadata().map_err(|e| self.io(e))?;
        self.on_device(&m)?;
        let local = local.to_path_buf();
        let (dev, root) = (self.dev, self.root.clone());
        let res = blocking(move || {
            let mut src = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(&local)?;
            let mtime = src.metadata()?.modified()?;
            let size = std::io::copy(&mut src, tmp.file())?;
            tmp.file().set_modified(mtime)?;
            let m = tmp.file().metadata()?;
            if m.dev() != dev {
                return Err(std::io::Error::other(format!(
                    "the destination drive is no longer mounted at {}",
                    root.display()
                )));
            }
            tmp.publish(&dest)?;
            let ino = std::fs::symlink_metadata(&dest)?.ino();
            Ok((size, ino))
        })
        .await;
        let (size, ino) = match res {
            Ok(v) => v,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(ProviderError::Conflict(format!(
                    "{folder}/{name} already exists"
                )))
            }
            Err(e) => return Err(self.io(e)),
        };
        Ok(RemoteStat {
            uid: ino.to_string(),
            claimed: ContentDigests {
                size,
                ..Default::default()
            },
            revision: None,
        })
    }

    async fn upload_revision(&self, _file: &RelPath, _local: &Path) -> PResult<RemoteStat> {
        Err(ProviderError::Unsupported(crate::Capability::Revisions))
    }

    /// Hashes of the file as read back from the drive now.
    async fn stat(&self, file: &RelPath) -> PResult<RemoteStat> {
        self.guard()?;
        let f = self.open_file(file)?;
        let ino = f.metadata().map_err(|e| self.io(e))?.ino();
        let claimed = blocking(move || hash_file(f))
            .await
            .map_err(|e| self.io(e))?;
        Ok(RemoteStat {
            uid: ino.to_string(),
            claimed,
            revision: None,
        })
    }

    async fn download_to(&self, file: &RelPath, dest: &Path) -> PResult<ContentDigests> {
        self.guard()?;
        let mut src = self.open_file(file)?;
        let mut tmp = StagedTemp::create_in(parent_dir(dest))?;
        let tmp = blocking(move || {
            let mut buf = vec![0u8; 1 << 20];
            loop {
                let n = src.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                std::io::Write::write_all(tmp.file(), &buf[..n])?;
            }
            Ok(tmp)
        })
        .await
        .map_err(|e| self.io(e))?;
        verify_and_publish(tmp, None, dest).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(s: &str) -> RelPath {
        RelPath::parse(s).unwrap()
    }

    fn source_file(dir: &Path, data: &[u8]) -> PathBuf {
        let p = dir.join("staged");
        std::fs::write(&p, data).unwrap();
        p
    }

    #[tokio::test]
    async fn adds_files_and_folders_and_never_replaces() {
        let root = tempfile::tempdir().unwrap();
        let stage = tempfile::tempdir().unwrap();
        let d = LocalFolderDestination::new(root.path()).unwrap();
        d.ensure_folder(&rel("A/B")).await.unwrap();
        d.ensure_folder(&rel("A/B")).await.unwrap();

        let src = source_file(stage.path(), b"hello");
        let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
        File::options()
            .write(true)
            .open(&src)
            .unwrap()
            .set_modified(old)
            .unwrap();
        let st = d.upload_new(&rel("A/B"), "f.txt", &src).await.unwrap();
        assert_eq!(st.claimed.size, 5);
        let placed = root.path().join("A/B/f.txt");
        assert_eq!(std::fs::read(&placed).unwrap(), b"hello");
        assert_eq!(std::fs::metadata(&placed).unwrap().modified().unwrap(), old);

        // Taken: a conflict, and the file is untouched.
        let other = source_file(stage.path(), b"other");
        let e = d.upload_new(&rel("A/B"), "f.txt", &other).await.unwrap_err();
        assert!(matches!(e, ProviderError::Conflict(_)), "{e}");
        assert_eq!(std::fs::read(&placed).unwrap(), b"hello");
        // No temporary files left behind.
        let names: Vec<_> = std::fs::read_dir(root.path().join("A/B"))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("f.txt")]);

        let stat = d.stat(&rel("A/B/f.txt")).await.unwrap();
        assert_eq!(stat.claimed, ContentDigests::of_bytes(b"hello"));
        let list = d.list(&rel("A/B")).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].size, Some(5));

        let back = stage.path().join("back");
        let got = d.download_to(&rel("A/B/f.txt"), &back).await.unwrap();
        assert_eq!(got, ContentDigests::of_bytes(b"hello"));
        assert!(matches!(
            d.stat(&rel("A/B/missing")).await,
            Err(ProviderError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn a_file_in_the_way_of_a_folder_is_a_conflict() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("A"), b"x").unwrap();
        let d = LocalFolderDestination::new(root.path()).unwrap();
        let e = d.ensure_folder(&rel("A/B")).await.unwrap_err();
        assert!(matches!(e, ProviderError::Conflict(_)), "{e}");
        assert_eq!(std::fs::read(root.path().join("A")).unwrap(), b"x");
    }

    #[tokio::test]
    async fn links_are_not_followed() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), b"s").unwrap();
        std::os::unix::fs::symlink(outside.path().join("secret"), root.path().join("l")).unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("dl")).unwrap();
        let d = LocalFolderDestination::new(root.path()).unwrap();
        assert!(matches!(
            d.stat(&rel("l")).await,
            Err(ProviderError::Conflict(_))
        ));
        let list = d.list(&RelPath::root()).await.unwrap();
        assert!(list.iter().all(|e| e.kind == ItemKind::File && e.size.is_none()));
        assert!(matches!(
            d.ensure_folder(&rel("dl/x")).await,
            Err(ProviderError::Conflict(_))
        ));
        assert!(!outside.path().join("x").exists());
    }

    #[tokio::test]
    async fn a_root_that_is_no_longer_the_same_folder_is_offline() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("mnt");
        std::fs::create_dir(&root).unwrap();
        let d = LocalFolderDestination::new(&root).unwrap();
        // Made while the old one still exists, so it can't reuse its inode.
        let other = parent.path().join("other");
        std::fs::create_dir(&other).unwrap();
        // The drive "unmounts": the mount point goes away.
        std::fs::remove_dir(&root).unwrap();
        assert!(matches!(
            d.list(&RelPath::root()).await,
            Err(ProviderError::Offline(_))
        ));
        // Then a different folder at the same path.
        std::fs::rename(&other, &root).unwrap();
        let stage = tempfile::tempdir().unwrap();
        let src = source_file(stage.path(), b"x");
        assert!(matches!(
            d.upload_new(&RelPath::root(), "f", &src).await,
            Err(ProviderError::Offline(_))
        ));
        assert!(matches!(
            d.ensure_folder(&rel("A")).await,
            Err(ProviderError::Offline(_))
        ));
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
    }

    #[test]
    fn on_mount_refuses_a_plain_folder() {
        let root = tempfile::tempdir().unwrap();
        assert!(LocalFolderDestination::on_mount(root.path()).is_err());
        assert!(LocalFolderDestination::on_mount(Path::new("relative")).is_err());
    }
}
