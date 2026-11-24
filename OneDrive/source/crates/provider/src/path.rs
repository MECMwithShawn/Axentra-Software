//! Relative paths inside a provider's tree, checked before they are used.
//!
//! A remote name is data from someone else's server. Before it becomes part
//! of a local path it must not be able to climb out of the directory it is
//! joined to.

use std::fmt;
use std::path::{Path, PathBuf};

/// Longest name a Linux filesystem takes, in bytes.
const NAME_MAX: usize = 255;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PathError {
    #[error("{0:?} is an absolute path; a relative one is needed")]
    Absolute(String),
    #[error("{0:?} refers to a parent folder (\"..\")")]
    ParentRef(String),
    #[error("{0:?} contains a NUL byte")]
    Nul(String),
    #[error("{0:?} is not a single name")]
    NotAName(String),
    #[error("the name {0:?} is longer than {NAME_MAX} bytes")]
    TooLong(String),
    #[error("{} is a symbolic link; it won't be followed", .0.display())]
    Symlink(PathBuf),
    #[error("{} is not a folder", .0.display())]
    NotAFolder(PathBuf),
}

/// A normalized path relative to some root: no empty, `.` or `..`
/// segments, no NUL, every segment a usable file name. The root is the
/// path with no segments.
///
/// Names are kept byte for byte. Unicode is not normalized, so `é` as one
/// code point and as `e` plus a combining accent are different names, as
/// they are on Linux filesystems.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RelPath(Vec<String>);

impl RelPath {
    pub fn root() -> Self {
        Self::default()
    }

    /// Normalize `s`: repeated, leading-`./` and trailing slashes and `.`
    /// segments go; `..`, a leading `/`, NUL and over-long names are refused.
    pub fn parse(s: &str) -> Result<Self, PathError> {
        if s.contains('\0') {
            return Err(PathError::Nul(s.into()));
        }
        if s.starts_with('/') {
            return Err(PathError::Absolute(s.into()));
        }
        let mut parts = Vec::new();
        for seg in s.split('/') {
            match seg {
                "" | "." => {}
                ".." => return Err(PathError::ParentRef(s.into())),
                name if name.len() > NAME_MAX => return Err(PathError::TooLong(name.into())),
                name => parts.push(name.to_string()),
            }
        }
        Ok(Self(parts))
    }

    /// This path with one more segment, which must be a plain name.
    pub fn join(&self, name: &str) -> Result<Self, PathError> {
        check_name(name)?;
        let mut parts = self.0.clone();
        parts.push(name.to_string());
        Ok(Self(parts))
    }

    pub fn segments(&self) -> &[String] {
        &self.0
    }

    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    /// The last segment, `None` for the root.
    pub fn name(&self) -> Option<&str> {
        self.0.last().map(String::as_str)
    }

    /// `None` for the root.
    pub fn parent(&self) -> Option<Self> {
        let (_, rest) = self.0.split_last()?;
        Some(Self(rest.to_vec()))
    }

    /// Where this path lands under the local folder `root`. Lexical only:
    /// see [`crate::staging::ensure_dirs`] to create it without following
    /// symbolic links.
    pub fn under(&self, root: &Path) -> PathBuf {
        let mut p = root.to_path_buf();
        p.extend(&self.0);
        p
    }
}

impl fmt::Display for RelPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.join("/"))
    }
}

/// A single file name that is safe to join to a local folder.
pub fn check_name(name: &str) -> Result<(), PathError> {
    if name.contains('\0') {
        return Err(PathError::Nul(name.into()));
    }
    if name == ".." {
        return Err(PathError::ParentRef(name.into()));
    }
    if name.is_empty() || name == "." || name.contains('/') {
        return Err(PathError::NotAName(name.into()));
    }
    if name.len() > NAME_MAX {
        return Err(PathError::TooLong(name.into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segs(s: &str) -> Vec<String> {
        RelPath::parse(s).unwrap().segments().to_vec()
    }

    #[test]
    fn normalizes_slashes_and_dots() {
        assert_eq!(segs("a/b/c.txt"), ["a", "b", "c.txt"]);
        assert_eq!(segs("./a//b/./c/"), ["a", "b", "c"]);
        assert!(RelPath::parse("").unwrap().is_root());
        assert!(RelPath::parse("./.").unwrap().is_root());
        assert_eq!(RelPath::parse("a//b/").unwrap().to_string(), "a/b");
        // Dots inside a name are just part of it.
        assert_eq!(segs("..a/b../.c/..."), ["..a", "b..", ".c", "..."]);
    }

    #[test]
    fn refuses_escapes() {
        assert!(matches!(RelPath::parse(".."), Err(PathError::ParentRef(_))));
        assert!(matches!(
            RelPath::parse("a/../../etc/passwd"),
            Err(PathError::ParentRef(_))
        ));
        assert!(matches!(
            RelPath::parse("a/b/.."),
            Err(PathError::ParentRef(_))
        ));
        assert!(matches!(
            RelPath::parse("/etc"),
            Err(PathError::Absolute(_))
        ));
        assert!(matches!(RelPath::parse("//x"), Err(PathError::Absolute(_))));
        assert!(matches!(RelPath::parse("a\0b"), Err(PathError::Nul(_))));
        assert!(matches!(
            RelPath::parse(&"x".repeat(256)),
            Err(PathError::TooLong(_))
        ));
        assert!(RelPath::parse(&"x".repeat(255)).is_ok());
    }

    #[test]
    fn join_takes_one_plain_name() {
        let base = RelPath::parse("docs").unwrap();
        assert_eq!(base.join("a.txt").unwrap().to_string(), "docs/a.txt");
        for bad in ["", ".", "..", "a/b", "/a", "a\0"] {
            assert!(base.join(bad).is_err(), "{bad:?}");
        }
        // 255 bytes is the limit, counted in bytes, not characters.
        assert!(base.join(&"é".repeat(127)).is_ok());
        assert!(matches!(
            base.join(&"é".repeat(128)),
            Err(PathError::TooLong(_))
        ));
    }

    #[test]
    fn unicode_names_are_kept_as_they_are() {
        let p = RelPath::parse("Ünïcødé/日本語/файл 📄.txt").unwrap();
        assert_eq!(p.segments(), ["Ünïcødé", "日本語", "файл 📄.txt"]);
        assert_eq!(p.name(), Some("файл 📄.txt"));
        assert_eq!(p.parent().unwrap().to_string(), "Ünïcødé/日本語");
        // Precomposed and decomposed forms stay distinct.
        let nfc = RelPath::parse("caf\u{e9}").unwrap();
        let nfd = RelPath::parse("cafe\u{301}").unwrap();
        assert_ne!(nfc, nfd);
        // Right-to-left and zero-width characters are names like any other.
        assert!(RelPath::parse("\u{202e}gpj.exe/a\u{200b}b").is_ok());
    }

    #[test]
    fn under_joins_lexically() {
        let p = RelPath::parse("a/b").unwrap();
        assert_eq!(p.under(Path::new("/stage")), Path::new("/stage/a/b"));
        assert_eq!(
            RelPath::root().under(Path::new("/stage")),
            Path::new("/stage")
        );
        assert!(RelPath::root().parent().is_none());
    }
}
