//! Thumbnails without downloads, and who is reading.
//!
//! Browsing a folder of online-only photos would otherwise make thumbnailers,
//! the file chooser portal and image viewers read (and so download) every
//! file. Instead OneDrive's own previews are written into the freedesktop
//! thumbnail cache (`~/.cache/thumbnails/{normal,large,x-large,xx-large}`,
//! named by the MD5 of the file's URI, with `Thumb::URI` and `Thumb::MTime`
//! matching what the mount reports), which GIO, GTK, Nautilus and Eye of
//! GNOME look up before reading a file. Thumbnailers' reads are refused; once
//! a file is downloaded for real, any thumbnail "failed" marker is cleared.

use std::io::BufWriter;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use md5::{Digest, Md5};

/// Set (to anything) through the mount after a cloud thumbnail is saved: the
/// change notification makes file managers and file choosers look for it.
pub const NUDGE_XATTR: &str = "user.axentra.onedrive.thumbnail";

fn comm(pid: u32) -> Option<String> {
    Some(
        std::fs::read_to_string(format!("/proc/{pid}/comm"))
            .ok()?
            .trim()
            .to_string(),
    )
}

fn parent(pid: u32) -> Option<u32> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    status
        .lines()
        .find_map(|l| l.strip_prefix("PPid:"))?
        .trim()
        .parse()
        .ok()
}

/// "program[pid] from parent[pid]", to say who asked for a download.
pub fn describe(pid: u32) -> String {
    let me = comm(pid).unwrap_or_else(|| "unknown".into());
    match parent(pid).and_then(|pp| Some((pp, comm(pp)?))) {
        Some((pp, name)) => format!("{me}[{pid}] from {name}[{pp}]"),
        None => format!("{me}[{pid}]"),
    }
}

/// GLib names its worker threads "pool-<program>".
fn program(comm: &str) -> &str {
    comm.strip_prefix("pool-").unwrap_or(comm)
}

fn is_thumbnailer(comm: &str) -> bool {
    let p = program(comm);
    p.contains("thumb") || p == "tumblerd"
}

fn is_indexer(comm: &str) -> bool {
    let p = program(comm);
    p.starts_with("tracker") || p.starts_with("localsearch")
}

/// The program's own name when the kernel's 15-character one is cut short
/// ("totem-video-thu").
fn argv0(pid: u32) -> Option<String> {
    let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let first = cmdline.split(|b| *b == 0).next()?;
    let name = first.rsplit(|b| *b == b'/').next()?;
    Some(String::from_utf8_lossy(name).into_owned())
}

fn is_thumbnailer_itself(pid: u32) -> bool {
    comm(pid).is_some_and(|c| {
        is_thumbnailer(&c) || (c.len() >= 15 && argv0(pid).is_some_and(|a| is_thumbnailer(&a)))
    })
}

/// The thumbnailer `pid` is, or works for: some run a helper that does the
/// reading (`xapp-raw-thumbnailer` runs `dcraw`).
pub fn thumbnailer_of(pid: u32) -> Option<u32> {
    let mut cur = pid;
    for _ in 0..3 {
        if is_thumbnailer_itself(cur) {
            return Some(cur);
        }
        cur = parent(cur).filter(|p| *p > 1)?;
    }
    None
}

/// Whether `pid` is a thumbnailer (or its helper), which a cloud thumbnail
/// can stand in for.
pub fn is_thumbnailer_pid(pid: u32) -> bool {
    thumbnailer_of(pid).is_some()
}

/// Whether `pid` is a thumbnailer or desktop indexer.
pub fn is_background_reader(pid: u32) -> bool {
    comm(pid).is_some_and(|c| is_indexer(&c)) || is_thumbnailer_pid(pid)
}

/// The desktop portal's worker threads: they read images for the file
/// chooser's previews.
pub fn is_portal(pid: u32) -> bool {
    comm(pid).is_some_and(|c| c.starts_with("pool-xdg-deskto") || c.starts_with("xdg-desktop-por"))
}

/// Whether a folder listing is for showing the folder to someone: a file
/// manager, an image viewer's gallery, or a GTK app's file chooser (which
/// lists on GLib's worker threads). Scripts, `find`, backup tools and
/// indexers walk whole trees, and their listings fetch nothing.
pub fn lists_for_display(pid: u32) -> bool {
    const BROWSERS: [&str; 10] = [
        "nautilus",
        "nemo",
        "caja",
        "thunar",
        "pcmanfm",
        "eog",
        "EogJobScheduler",
        "gthumb",
        "loupe",
        "shotwell",
    ];
    let Some(c) = comm(pid) else { return false };
    if is_indexer(&c) || is_thumbnailer_pid(pid) || c == "pool-spawner" {
        return false;
    }
    c.starts_with("pool-") || BROWSERS.contains(&c.as_str())
}

/// The size a thumbnailer was asked for (`-s 256` in its arguments).
pub fn requested_size(pid: u32) -> Option<u32> {
    let pid = thumbnailer_of(pid).unwrap_or(pid);
    let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let mut args = cmdline.split(|b| *b == 0);
    while let Some(a) = args.next() {
        if a == b"-s" || a == b"--size" {
            return std::str::from_utf8(args.next()?).ok()?.parse().ok();
        }
    }
    None
}

/// Photos and videos, which OneDrive makes previews of.
pub fn is_media(name: &str) -> bool {
    const MEDIA: [&str; 31] = [
        "jpg", "jpeg", "jpe", "jfif", "png", "gif", "bmp", "webp", "tif", "tiff", "heic", "heif",
        "avif", "dng", "cr2", "cr3", "nef", "arw", "orf", "rw2", "raf", "mp4", "m4v", "mov", "avi",
        "mkv", "webm", "wmv", "3gp", "mts", "m2ts",
    ];
    name.rsplit_once('.').is_some_and(|(stem, ext)| {
        !stem.is_empty() && MEDIA.contains(&ext.to_ascii_lowercase().as_str())
    })
}

/// The freedesktop thumbnail sizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Bucket {
    Normal,
    Large,
    XLarge,
    XXLarge,
}

impl Bucket {
    pub const ALL: [Bucket; 4] = [
        Bucket::Normal,
        Bucket::Large,
        Bucket::XLarge,
        Bucket::XXLarge,
    ];

    pub fn px(self) -> u32 {
        128 << self as u32
    }

    pub fn dir(self) -> &'static str {
        ["normal", "large", "x-large", "xx-large"][self as usize]
    }

    pub fn bit(self) -> u32 {
        1 << self as u32
    }

    /// The smallest size at least `px` pixels.
    pub fn fitting(px: u32) -> Bucket {
        Self::ALL
            .into_iter()
            .find(|b| b.px() >= px)
            .unwrap_or(Bucket::XXLarge)
    }

    /// "normal", "large", "x-large", "xx-large", or a pixel size; "" is large.
    pub fn parse(s: &str) -> Option<Bucket> {
        match s {
            "" => Some(Bucket::Large),
            _ => Self::ALL
                .into_iter()
                .find(|b| b.dir() == s)
                .or_else(|| s.parse().ok().map(Self::fitting)),
        }
    }
}

/// GLib's g_filename_to_uri escaping for paths.
pub fn file_uri(path: &Path) -> String {
    let mut out = String::from("file://");
    for &b in path.as_os_str().as_encoded_bytes() {
        let keep = b.is_ascii_alphanumeric() || b"!$&'()*+,-./:=@_~".contains(&b);
        if keep {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The cache file name (without ".png") for a file's URI.
pub fn cache_name(uri: &str) -> String {
    Md5::digest(uri.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn path_in(root: &Path, bucket: Bucket, name: &str) -> PathBuf {
    root.join(bucket.dir()).join(format!("{name}.png"))
}

/// What the cache entry has to say to be used for a file.
pub struct Original<'a> {
    pub uri: &'a str,
    /// Unix seconds, as `stat` reports them.
    pub mtime: i64,
    pub size: u64,
}

/// Whether the PNG at `path` is a current thumbnail of `of`, by the checks
/// GIO makes (`thumbnail::is-valid`).
pub fn is_valid(path: &Path, of: &Original<'_>) -> bool {
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    let Ok(reader) = png::Decoder::new(std::io::BufReader::new(file)).read_info() else {
        return false;
    };
    let text = |k: &str| {
        reader
            .info()
            .uncompressed_latin1_text
            .iter()
            .find(|c| c.keyword == k)
            .map(|c| c.text.as_str())
    };
    text("Thumb::URI") == Some(of.uri)
        && text("Thumb::MTime") == Some(of.mtime.to_string().as_str())
        && text("Thumb::Size").is_none_or(|s| s == of.size.to_string())
}

/// Write `img` (scaled to fit `bucket`) as the cache entry for `of`.
pub fn write(
    root: &Path,
    bucket: Bucket,
    of: &Original<'_>,
    img: &image::DynamicImage,
) -> Result<PathBuf> {
    let px = bucket.px();
    let img = if img.width() > px || img.height() > px {
        img.resize(px, px, image::imageops::FilterType::Triangle)
    } else {
        img.clone()
    };
    let (color, pixels, (w, h)) = if img.color().has_alpha() {
        let p = img.to_rgba8();
        (png::ColorType::Rgba, p.as_raw().clone(), p.dimensions())
    } else {
        let p = img.to_rgb8();
        (png::ColorType::Rgb, p.as_raw().clone(), p.dimensions())
    };
    let dir = root.join(bucket.dir());
    create_private_dir(&dir)?;
    let name = cache_name(of.uri);
    let dest = dir.join(format!("{name}.png"));
    let tmp = dir.join(format!(".{name}.axentra-{}.png", std::process::id()));
    let result = (|| -> Result<()> {
        use std::os::unix::fs::OpenOptionsExt;
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        let mut enc = png::Encoder::new(BufWriter::new(file), w, h);
        enc.set_color(color);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Balanced);
        enc.add_text_chunk("Thumb::URI".into(), of.uri.into())?;
        enc.add_text_chunk("Thumb::MTime".into(), of.mtime.to_string())?;
        enc.add_text_chunk("Thumb::Size".into(), of.size.to_string())?;
        enc.add_text_chunk("Software".into(), "Axentra OneDrive".into())?;
        let mut w = enc.write_header()?;
        w.write_image_data(&pixels)?;
        w.finish()?;
        Ok(())
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(e).context("writing a thumbnail");
    }
    std::fs::rename(&tmp, &dest)?;
    Ok(dest)
}

fn create_private_dir(dir: &Path) -> Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    match std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
    {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Delete the cache entries (every size, and "failed" markers) for these names.
pub fn remove(root: &Path, names: &[String]) {
    if names.is_empty() {
        return;
    }
    let fail_dirs: Vec<PathBuf> = std::fs::read_dir(root.join("fail"))
        .map(|d| d.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    for name in names {
        for b in Bucket::ALL {
            let _ = std::fs::remove_file(path_in(root, b, name));
        }
        for d in &fail_dirs {
            let _ = std::fs::remove_file(d.join(format!("{name}.png")));
        }
    }
}

/// Forget that thumbnailing a file failed (it was online-only then), so
/// file managers try again now that it's here.
pub fn clear_failure(root: &Path, abs_path: &Path) {
    let name = cache_name(&file_uri(abs_path));
    let Ok(entries) = std::fs::read_dir(root.join("fail")) else {
        return;
    };
    for e in entries.flatten() {
        let _ = std::fs::remove_file(e.path().join(format!("{name}.png")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_escaping_matches_glib() {
        assert_eq!(
            file_uri(Path::new("/home/a/One Drive/x#1;é.jpg")),
            "file:///home/a/One%20Drive/x%231%3B%C3%A9.jpg"
        );
        // `printf %s file:///home/a/b.jpg | md5sum`
        assert_eq!(
            cache_name("file:///home/a/b.jpg"),
            "c8734c87070a10e5c70318cc0c92fb89"
        );
    }

    #[test]
    fn sizes() {
        assert_eq!(Bucket::Normal.px(), 128);
        assert_eq!(Bucket::XXLarge.px(), 1024);
        assert_eq!(Bucket::XLarge.dir(), "x-large");
        assert_eq!(Bucket::fitting(256), Bucket::Large);
        assert_eq!(Bucket::fitting(300), Bucket::XLarge);
        assert_eq!(Bucket::fitting(4000), Bucket::XXLarge);
        assert_eq!(Bucket::parse(""), Some(Bucket::Large));
        assert_eq!(Bucket::parse("xx-large"), Some(Bucket::XXLarge));
        assert_eq!(Bucket::parse("96"), Some(Bucket::Normal));
        assert_eq!(Bucket::parse("huge"), None);
    }

    #[test]
    fn media_names() {
        assert!(is_media("IMG_0001.JPG"));
        assert!(is_media("clip.mov"));
        assert!(is_media("a.b.heic"));
        assert!(!is_media("IMG_0001.AAE"));
        assert!(!is_media(".jpg"));
        assert!(!is_media("notes.txt"));
    }

    #[test]
    fn readers() {
        // The kernel's names are cut to 15 characters, so these are only
        // recognised by their full names (`is_thumbnailer_pid` looks at both).
        assert!(!is_thumbnailer("gdk-pixbuf-thum"));
        assert!(is_thumbnailer("gdk-pixbuf-thumbnailer"));
        assert!(is_thumbnailer("totem-video-thumbnailer"));
        assert!(is_thumbnailer("ffmpegthumbnai"));
        assert!(is_indexer("pool-tracker-mi"));
        assert!(is_indexer("localsearch-3"));
        assert!(!is_thumbnailer("pool-xdg-deskto"));
    }

    #[test]
    fn written_thumbnails_are_valid_for_their_file_only() {
        let dir = tempfile::tempdir().unwrap();
        let uri = "file:///home/a/OneDrive/IMG%201.jpg";
        let of = Original {
            uri,
            mtime: 1_700_000_000,
            size: 12345,
        };
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            400,
            300,
            image::Rgb([10, 20, 30]),
        ));
        let p = write(dir.path(), Bucket::Normal, &of, &img).unwrap();
        assert_eq!(p, path_in(dir.path(), Bucket::Normal, &cache_name(uri)));
        assert_eq!(image::image_dimensions(&p).unwrap(), (128, 96));
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(is_valid(&p, &of));
        assert!(!is_valid(
            &p,
            &Original {
                mtime: 1_700_000_001,
                ..of
            }
        ));
        assert!(!is_valid(&p, &Original { size: 1, ..of }));
        assert!(!is_valid(
            &p,
            &Original {
                uri: "file:///other.jpg",
                ..of
            }
        ));

        // Small originals aren't scaled up.
        let small = image::DynamicImage::ImageRgb8(image::RgbImage::new(60, 40));
        let p = write(dir.path(), Bucket::Large, &of, &small).unwrap();
        assert_eq!(image::image_dimensions(&p).unwrap(), (60, 40));

        remove(dir.path(), &[cache_name(uri)]);
        assert!(!p.exists());
    }
}
