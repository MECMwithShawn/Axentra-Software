//! Copies of source files that are already on this computer (another
//! drive, an old Windows profile), to take a file from instead of
//! downloading it.
//!
//! A file is only ever taken when its bytes hash to exactly what the source
//! says the file is (its QuickXorHash and size), so a seeded file is as
//! good as a downloaded one, wherever it was found. Candidates are looked
//! up by name and size; content decides. Seed folders are only read: they
//! are opened without following links and nothing in them is changed.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use axentra_clouddrive_provider::staging::StagedTemp;
use axentra_clouddrive_provider::ContentDigests;

/// At most this many same-name, same-size candidates are hashed per file.
const MAX_CANDIDATES: usize = 8;

/// Files under the seed folders, by name and size.
#[derive(Debug, Default)]
pub struct SeedIndex {
    files: HashMap<(String, u64), Vec<PathBuf>>,
    count: u64,
}

impl SeedIndex {
    /// Every regular file under `roots`, not following links. Folders that
    /// can't be read are passed over. `progress` is told the count so far
    /// now and then.
    pub fn scan(roots: &[PathBuf], progress: &dyn Fn(u64)) -> std::io::Result<Self> {
        let mut index = Self::default();
        for root in roots {
            let meta = std::fs::symlink_metadata(root)?;
            if !meta.is_dir() {
                return Err(std::io::Error::other(format!(
                    "{} is not a folder",
                    root.display()
                )));
            }
            let mut stack = vec![root.clone()];
            while let Some(dir) = stack.pop() {
                let Ok(entries) = std::fs::read_dir(&dir) else {
                    continue;
                };
                for ent in entries.flatten() {
                    let Ok(ft) = ent.file_type() else {
                        continue;
                    };
                    if ft.is_dir() {
                        stack.push(ent.path());
                    } else if ft.is_file() {
                        let (Ok(meta), Ok(name)) = (ent.metadata(), ent.file_name().into_string())
                        else {
                            continue;
                        };
                        index
                            .files
                            .entry((name, meta.len()))
                            .or_default()
                            .push(ent.path());
                        index.count += 1;
                        if index.count.is_multiple_of(50_000) {
                            progress(index.count);
                        }
                    }
                }
            }
        }
        progress(index.count);
        Ok(index)
    }

    /// How many files were found.
    pub fn len(&self) -> u64 {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    fn candidates(&self, name: &str, size: u64) -> &[PathBuf] {
        self.files
            .get(&(name.to_string(), size))
            .map_or(&[], Vec::as_slice)
    }

    /// Put a copy of a file called `name` whose content matches `want` at
    /// `dest` (which must not exist yet), if one of the candidates is that
    /// file. Returns the digests computed from the copied bytes. Anything
    /// that doesn't match, or can't be read, is passed over.
    pub async fn take(&self, name: &str, want: &ContentDigests, dest: &Path) -> Option<ContentDigests> {
        if want.qxh.is_none() && want.sha1.is_none() && want.sha256.is_none() {
            return None;
        }
        let candidates: Vec<PathBuf> = self
            .candidates(name, want.size)
            .iter()
            .take(MAX_CANDIDATES)
            .cloned()
            .collect();
        if candidates.is_empty() {
            return None;
        }
        let want = want.clone();
        let dest = dest.to_path_buf();
        tokio::task::spawn_blocking(move || {
            let dir = dest.parent()?;
            for from in candidates {
                let Ok(mut src) = OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                    .open(&from)
                else {
                    continue;
                };
                let Ok(mut tmp) = StagedTemp::create_in(dir) else {
                    return None;
                };
                if std::io::copy(&mut src, tmp.file()).is_err() {
                    continue;
                }
                let Ok(got) = tmp.digests() else {
                    continue;
                };
                if got.verify_against(&want).is_ok() && tmp.publish(&dest).is_ok() {
                    return Some(got);
                }
            }
            None
        })
        .await
        .ok()
        .flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn only_a_copy_with_the_right_content_is_taken() {
        let seed = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(seed.path().join("a/b")).unwrap();
        // Same name and size, other content: passed over.
        std::fs::write(seed.path().join("a/f.txt"), b"wrong").unwrap();
        std::fs::write(seed.path().join("a/b/f.txt"), b"right").unwrap();
        std::os::unix::fs::symlink(seed.path().join("a/b/f.txt"), seed.path().join("link.txt"))
            .unwrap();
        let idx = SeedIndex::scan(&[seed.path().to_path_buf()], &|_| {}).unwrap();
        assert_eq!(idx.len(), 2, "links aren't indexed");

        let stage = tempfile::tempdir().unwrap();
        let want = ContentDigests::of_bytes(b"right");
        let dest = stage.path().join("f.txt");
        let got = idx.take("f.txt", &want, &dest).await.unwrap();
        assert_eq!(got.sha256, want.sha256);
        assert_eq!(std::fs::read(&dest).unwrap(), b"right");

        // Nothing matching: nothing is made.
        let other = ContentDigests::of_bytes(b"other");
        let dest2 = stage.path().join("g.txt");
        assert!(idx.take("f.txt", &other, &dest2).await.is_none());
        assert!(idx.take("link.txt", &want, &dest2).await.is_none());
        assert_eq!(std::fs::read_dir(stage.path()).unwrap().count(), 1);
        // The seed folder is untouched.
        assert_eq!(std::fs::read(seed.path().join("a/f.txt")).unwrap(), b"wrong");
    }
}
