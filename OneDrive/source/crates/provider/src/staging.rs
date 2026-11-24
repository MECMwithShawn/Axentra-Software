//! Writing downloads into a staging folder without following symbolic links.
//!
//! A download goes to a fresh temporary file (`O_CREAT|O_EXCL|O_NOFOLLOW`,
//! mode 0600) next to its destination, and is written and hashed through
//! the handle opened then, never by path. Only once its digests check out is
//! it linked into place, and never over anything already there. A temporary
//! file that isn't published is removed when it goes out of scope.

use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::digests::ContentDigests;
use crate::error::{PResult, ProviderError};
use crate::path::{PathError, RelPath};

/// Temporary files and folders start with this, so a leftover one is easy
/// to tell.
pub const TEMP_PREFIX: &str = ".axentra-part-";

/// A name for the next temporary file or folder, unique within this process.
fn next_temp_name(suffix: &str) -> String {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{TEMP_PREFIX}{}-{n}{suffix}", std::process::id())
}

/// A temporary file being filled. Removed on drop unless published.
#[derive(Debug)]
pub struct StagedTemp {
    path: PathBuf,
    file: File,
    published: bool,
}

impl StagedTemp {
    /// A new, empty file in `dir`, which must be a real folder (not a link).
    pub fn create_in(dir: &Path) -> std::io::Result<Self> {
        real_dir(dir)?;
        loop {
            let path = dir.join(next_temp_name(""));
            let opened = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(&path);
            match opened {
                Ok(file) => {
                    return Ok(Self {
                        path,
                        file,
                        published: false,
                    })
                }
                // Left by an earlier run with the same pid: pick another.
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn file(&mut self) -> &mut File {
        &mut self.file
    }

    /// Digests of what the file holds now, read through the handle opened
    /// at creation, so a file swapped in at the same path isn't what's read.
    pub fn digests(&mut self) -> std::io::Result<ContentDigests> {
        self.file.seek(SeekFrom::Start(0))?;
        ContentDigests::of_reader(&mut self.file)
    }

    /// Link the file in at `dest`, which must not exist yet (not even as a
    /// dangling link), then drop the temporary name. Fails if the path no
    /// longer leads to the file this created.
    pub fn publish(mut self, dest: &Path) -> std::io::Result<()> {
        self.file.sync_all()?;
        let ours = self.file.metadata()?;
        let now = std::fs::symlink_metadata(&self.path)?;
        if (now.dev(), now.ino()) != (ours.dev(), ours.ino()) {
            return Err(std::io::Error::other(
                "the staged file was replaced while it was being written",
            ));
        }
        // link(2) never follows or replaces whatever is at `dest`.
        std::fs::hard_link(&self.path, dest)?;
        self.published = true;
        std::fs::remove_file(&self.path)
    }
}

impl Drop for StagedTemp {
    fn drop(&mut self) {
        if !self.published {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// A new, empty folder (mode 0700) made by this process. Removed, with
/// everything in it, on drop.
#[derive(Debug)]
pub struct StagedDir {
    path: PathBuf,
}

impl StagedDir {
    /// A fresh folder in `dir`, which must be a real folder (not a link).
    /// Never an existing one: a name already taken is skipped.
    pub fn create_in(dir: &Path) -> std::io::Result<Self> {
        real_dir(dir)?;
        loop {
            let path = dir.join(next_temp_name(".d"));
            match std::fs::DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for StagedDir {
    fn drop(&mut self) {
        // Doesn't follow links inside: a link is removed, not what it points to.
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// The folder `dest` goes in: `.` for a bare file name, whose parent is
/// otherwise the empty path.
pub(crate) fn parent_dir(dest: &Path) -> &Path {
    match dest.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    }
}

/// `dir` is a folder in itself, not a link to one.
fn real_dir(dir: &Path) -> std::io::Result<()> {
    let meta = std::fs::symlink_metadata(dir)?;
    if meta.file_type().is_symlink() {
        return Err(std::io::Error::other(PathError::Symlink(dir.into())));
    }
    if !meta.is_dir() {
        return Err(std::io::Error::other(PathError::NotAFolder(dir.into())));
    }
    Ok(())
}

/// Create `rel` under the staging folder `root` (mode 0700), one level at a
/// time, refusing any level that is a symbolic link or not a folder. `root`
/// itself is trusted.
pub fn ensure_dirs(root: &Path, rel: &RelPath) -> PResult<PathBuf> {
    let mut at = root.to_path_buf();
    for seg in rel.segments() {
        at.push(seg);
        match std::fs::DirBuilder::new().mode(0o700).create(&at) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
        let meta = std::fs::symlink_metadata(&at)?;
        if meta.file_type().is_symlink() {
            return Err(PathError::Symlink(at).into());
        }
        if !meta.is_dir() {
            return Err(PathError::NotAFolder(at).into());
        }
    }
    Ok(at)
}

/// Check what `tmp` holds against `expected` and publish it at `dest` if it
/// matches. On a mismatch the temporary file is removed and nothing appears
/// at `dest`. Returns the digests computed from the bytes.
pub async fn verify_and_publish(
    mut tmp: StagedTemp,
    expected: Option<&ContentDigests>,
    dest: &Path,
) -> PResult<ContentDigests> {
    let (tmp, got) = tokio::task::spawn_blocking(move || {
        let got = tmp.digests();
        (tmp, got)
    })
    .await
    .map_err(|e| ProviderError::Other(format!("hashing the download: {e}")))?;
    let got = got?;
    if let Some(want) = expected {
        if let Err(why) = got.verify_against(want) {
            return Err(ProviderError::Integrity(format!(
                "{}: {why}",
                dest.display()
            )));
        }
    }
    let dest = dest.to_path_buf();
    tokio::task::spawn_blocking(move || tmp.publish(&dest).map_err(|e| (e, dest)))
        .await
        .map_err(|e| ProviderError::Other(format!("saving the download: {e}")))?
        .map_err(|(e, dest)| match e.kind() {
            std::io::ErrorKind::AlreadyExists => {
                ProviderError::Conflict(format!("{} already exists", dest.display()))
            }
            _ => ProviderError::Local(e),
        })?;
    Ok(got)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::fs::{symlink, PermissionsExt};

    fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn a_temp_file_is_private_and_goes_away_unless_published() {
        let dir = tempfile::tempdir().unwrap();
        let mut t = StagedTemp::create_in(dir.path()).unwrap();
        assert!(t
            .path()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with(TEMP_PREFIX));
        let mode = std::fs::metadata(t.path()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        t.file().write_all(b"x").unwrap();
        drop(t);
        assert!(names(dir.path()).is_empty());

        let mut t = StagedTemp::create_in(dir.path()).unwrap();
        t.file().write_all(b"kept").unwrap();
        let dest = dir.path().join("файл 📄.txt");
        t.publish(&dest).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"kept");
        assert_eq!(names(dir.path()), ["файл 📄.txt"]);
    }

    #[test]
    fn publishing_never_replaces_or_follows_what_is_there() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let victim = outside.path().join("victim");
        std::fs::write(&victim, b"precious").unwrap();

        // A link at the destination, to a file and to nowhere.
        let dest = dir.path().join("a");
        symlink(&victim, &dest).unwrap();
        let mut t = StagedTemp::create_in(dir.path()).unwrap();
        t.file().write_all(b"evil").unwrap();
        let err = t.publish(&dest).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&victim).unwrap(), b"precious");

        let dangling = dir.path().join("b");
        symlink(outside.path().join("nothing"), &dangling).unwrap();
        let t = StagedTemp::create_in(dir.path()).unwrap();
        assert!(t.publish(&dangling).is_err());
        assert!(!outside.path().join("nothing").exists());

        // A plain file is not replaced either.
        let plain = dir.path().join("c");
        std::fs::write(&plain, b"old").unwrap();
        let t = StagedTemp::create_in(dir.path()).unwrap();
        assert!(t.publish(&plain).is_err());
        assert_eq!(std::fs::read(&plain).unwrap(), b"old");
        // And no temporary files are left behind.
        assert_eq!(names(dir.path()), ["a", "b", "c"]);
    }

    #[test]
    fn a_bare_name_is_staged_in_the_current_folder() {
        assert_eq!(parent_dir(Path::new("x")), Path::new("."));
        assert_eq!(parent_dir(Path::new("/")), Path::new("."));
        assert_eq!(parent_dir(Path::new("d/x")), Path::new("d"));
        assert_eq!(parent_dir(Path::new("/x")), Path::new("/"));
    }

    #[test]
    fn a_linked_staging_folder_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let link = dir.path().join("stage");
        symlink(outside.path(), &link).unwrap();
        assert!(StagedTemp::create_in(&link).is_err());
        assert!(names(outside.path()).is_empty());
    }

    #[test]
    fn ensure_dirs_refuses_links_and_files_on_the_way() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let rel = RelPath::parse("Ünïcødé/日本語").unwrap();
        let made = ensure_dirs(root.path(), &rel).unwrap();
        assert!(made.is_dir());
        assert_eq!(made, rel.under(root.path()));
        let mode = std::fs::metadata(&made).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
        // Again: already there is fine.
        ensure_dirs(root.path(), &rel).unwrap();

        symlink(outside.path(), root.path().join("out")).unwrap();
        let err = ensure_dirs(root.path(), &RelPath::parse("out/x").unwrap()).unwrap_err();
        assert!(
            matches!(err, ProviderError::InvalidPath(PathError::Symlink(_))),
            "{err}"
        );
        assert!(names(outside.path()).is_empty());

        std::fs::write(root.path().join("f"), b"").unwrap();
        let err = ensure_dirs(root.path(), &RelPath::parse("f/x").unwrap()).unwrap_err();
        assert!(
            matches!(err, ProviderError::InvalidPath(PathError::NotAFolder(_))),
            "{err}"
        );
    }

    #[test]
    fn a_staged_dir_is_fresh_private_and_cleaned_up() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("keep"), b"keep").unwrap();
        let a = StagedDir::create_in(dir.path()).unwrap();
        let b = StagedDir::create_in(dir.path()).unwrap();
        assert_ne!(a.path(), b.path());
        let mode = std::fs::metadata(a.path()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
        std::fs::write(a.path().join("f"), b"x").unwrap();
        std::fs::create_dir(a.path().join("sub")).unwrap();
        // A link inside is removed, not followed.
        symlink(outside.path(), a.path().join("out")).unwrap();
        drop(a);
        drop(b);
        assert!(names(dir.path()).is_empty());
        assert_eq!(names(outside.path()), ["keep"]);

        let link = dir.path().join("stage");
        symlink(outside.path(), &link).unwrap();
        assert!(StagedDir::create_in(&link).is_err());
        assert_eq!(names(outside.path()), ["keep"]);
    }

    #[tokio::test]
    async fn a_mismatch_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        let mut t = StagedTemp::create_in(dir.path()).unwrap();
        t.file().write_all(b"hello").unwrap();
        let want = ContentDigests::of_bytes(b"hellp");
        let dest = dir.path().join("out");
        let err = verify_and_publish(t, Some(&want), &dest).await.unwrap_err();
        assert!(matches!(err, ProviderError::Integrity(_)), "{err}");
        assert!(names(dir.path()).is_empty());

        let mut t = StagedTemp::create_in(dir.path()).unwrap();
        t.file().write_all(b"hello").unwrap();
        let want = ContentDigests::of_bytes(b"hello");
        let got = verify_and_publish(t, Some(&want), &dest).await.unwrap();
        assert_eq!(got, want);
        assert_eq!(names(dir.path()), ["out"]);
    }
}
