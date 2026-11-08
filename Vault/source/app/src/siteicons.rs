//! Site icons: fetches each website's own icon (Apple touch icon, declared icons, or favicon)
//! directly from that site, converts it to a 64 px PNG, and lets the session store it in the
//! database as a KeePass custom icon, so it works offline and in KeePass/KeePassXC too.
//!
//! Privacy: only the host name is used (never the path, query, username or anything else from
//! the entry), requests go straight to that host (no third-party icon service), no cookies are
//! sent, and fetching only happens when the user asks for it.

use std::collections::VecDeque;
use std::io::Cursor;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ureq::ResponseExt;

const ICON_PX: u32 = 64;
const MAX_PAGE: u64 = 1_500_000;
const MAX_IMAGE: u64 = 2_000_000;
const WORKERS: usize = 6;

/// The host to fetch an icon for: http(s) URLs only, public names only (no IPs, no localhost).
pub fn icon_host(url: &str) -> Option<String> {
    let u = url.trim();
    let lower = u.to_lowercase();
    if !(lower.starts_with("https://") || lower.starts_with("http://")) {
        return None;
    }
    let host = vault_core::search::host_of(u).to_lowercase();
    let host = host.trim_end_matches('.');
    if host.is_empty()
        || !host.contains('.')
        || host.ends_with(".local")
        || host.ends_with(".lan")
        || host.parse::<std::net::IpAddr>().is_ok()
        || host
            .chars()
            .any(|c| !(c.is_ascii_alphanumeric() || c == '.' || c == '-'))
    {
        return None;
    }
    Some(host.to_owned())
}

fn public_address(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !(a == 0
                || a == 10
                || a == 127
                || a >= 224
                || (a == 100 && (64..=127).contains(&b))
                || (a == 169 && b == 254)
                || (a == 172 && (16..=31).contains(&b))
                || (a == 192
                    && (b == 168 || (b == 0 && (c == 0 || c == 2)) || (b == 88 && c == 99)))
                || (a == 198 && (b == 18 || b == 19 || (b == 51 && c == 100)))
                || (a == 203 && b == 0 && c == 113))
        }
        IpAddr::V6(ip) => {
            let s = ip.segments();
            // Conservatively allow global unicast only. Exclude special-purpose,
            // documentation, transition and IPv4-mapped ranges.
            (s[0] & 0xe000) == 0x2000
                && !(s[0] == 0x2001 && (s[1] < 0x0200 || s[1] == 0x0db8))
                && s[0] != 0x2002
                && !(s[0] == 0x3fff && s[1] < 0x1000)
        }
    }
}

fn public_uri(url: &str) -> Option<ureq::http::Uri> {
    let uri: ureq::http::Uri = url.parse().ok()?;
    if uri.scheme_str() != Some("https") || uri.port_u16().unwrap_or(443) != 443 {
        return None;
    }
    if uri.authority()?.as_str().contains('@') {
        return None;
    }
    icon_host(url)?;
    Some(uri)
}

/// The site a host belongs to: its last two labels, or three for two-letter country
/// second-level domains such as `co.uk` / `com.au`. A heuristic, not the Public Suffix List;
/// it only decides which hosts count as "the same site" for redirects and icon links. Every
/// request is still HTTPS-only and pinned to public addresses by `PublicResolver`.
fn site_of(host: &str) -> String {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let labels: Vec<&str> = host.split('.').collect();
    let n = labels.len();
    let keep = if n >= 3 && labels[n - 1].len() == 2 && labels[n - 2].len() <= 3 {
        3
    } else {
        2
    };
    labels[n.saturating_sub(keep)..].join(".")
}

/// Same site (e.g. `chase.com` and `www.chase.com`), both allowed public HTTPS URLs.
fn same_site(base: &str, url: &str) -> bool {
    match (public_uri(base), public_uri(url)) {
        (Some(a), Some(b)) => a
            .host()
            .zip(b.host())
            .is_some_and(|(a, b)| site_of(a) == site_of(b)),
        _ => false,
    }
}

/// ureq connects to the returned socket addresses; it does not resolve them again.
/// Reject the entire answer if any address is not public, including mixed answers.
#[derive(Debug, Default)]
struct PublicResolver<R = ureq::unversioned::resolver::DefaultResolver>(R);

impl<R: ureq::unversioned::resolver::Resolver> ureq::unversioned::resolver::Resolver
    for PublicResolver<R>
{
    fn resolve(
        &self,
        uri: &ureq::http::Uri,
        config: &ureq::config::Config,
        timeout: ureq::unversioned::transport::NextTimeout,
    ) -> Result<ureq::unversioned::resolver::ResolvedSocketAddrs, ureq::Error> {
        if public_uri(&uri.to_string()).is_none() {
            return Err(ureq::Error::HostNotFound);
        }
        let addresses = self.0.resolve(uri, config, timeout)?;
        if addresses.is_empty() || addresses.iter().any(|a| !public_address(a.ip())) {
            return Err(ureq::Error::HostNotFound);
        }
        Ok(addresses)
    }
}

fn agent() -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(8)))
        .max_redirects(0)
        .https_only(true)
        .proxy(None)
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AxentraVault-IconFetcher")
        .http_status_as_error(true)
        .build();
    ureq::Agent::with_parts(
        config,
        ureq::unversioned::transport::DefaultConnector::default(),
        PublicResolver(ureq::unversioned::resolver::DefaultResolver::default()),
    )
}

/// An icon link declared in a page, with a preference score (bigger is better).
#[derive(Debug, PartialEq)]
struct Candidate {
    url: String,
    score: u32,
}

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(name) {
        let start = from + i;
        let before_ok = start == 0
            || !lower.as_bytes()[start - 1].is_ascii_alphanumeric()
                && lower.as_bytes()[start - 1] != b'-';
        let rest = &lower[start + name.len()..];
        let trimmed = rest.trim_start();
        if before_ok && trimmed.starts_with('=') {
            let after_eq = start + name.len() + (rest.len() - trimmed.len()) + 1;
            let value = tag[after_eq..].trim_start();
            let offset = tag.len() - value.len();
            return match value.chars().next()? {
                q @ ('"' | '\'') => {
                    let end = value[1..].find(q)?;
                    Some(&tag[offset + 1..offset + 1 + end])
                }
                _ => {
                    let end = value
                        .find(|c: char| c.is_whitespace() || c == '>')
                        .unwrap_or(value.len());
                    Some(&tag[offset..offset + end])
                }
            };
        }
        from = start + name.len();
    }
    None
}

/// `<link rel="...icon..." href=...>` tags, best first: Apple touch icons and large sizes win,
/// SVG is good (scalable), monochrome mask icons are skipped.
fn icon_links(html: &str, base: &str) -> Vec<Candidate> {
    let mut out = Vec::new();
    let lower = html.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find("<link") {
        let start = from + i;
        let end = lower[start..]
            .find('>')
            .map(|e| start + e + 1)
            .unwrap_or(lower.len());
        let tag = &html[start..end];
        from = end;
        let Some(rel) = attr(tag, "rel").map(|r| r.to_ascii_lowercase()) else {
            continue;
        };
        if !rel.split_whitespace().any(|r| r.contains("icon")) || rel.contains("mask-icon") {
            continue;
        }
        let Some(href) = attr(tag, "href") else {
            continue;
        };
        let Some(url) = resolve(base, href.trim()) else {
            continue;
        };
        if !same_site(base, &url) {
            continue;
        }
        let size = attr(tag, "sizes")
            .and_then(|s| {
                s.split_whitespace()
                    .filter_map(|p| p.split_once(['x', 'X']))
                    .filter_map(|(w, _)| w.parse::<u32>().ok())
                    .max()
            })
            .unwrap_or(0);
        let is_svg = attr(tag, "type").is_some_and(|t| t.contains("svg"))
            || url.to_lowercase().ends_with(".svg");
        let score = if rel.contains("apple-touch-icon") {
            500u32.saturating_add(size.max(180))
        } else if is_svg {
            450
        } else {
            100 + size.min(400)
        };
        out.push(Candidate { url, score });
    }
    out.sort_by_key(|c| std::cmp::Reverse(c.score));
    out
}

/// Resolves an href against the page URL (absolute, protocol-relative, root or relative).
fn resolve(base: &str, href: &str) -> Option<String> {
    if href.is_empty() || href.starts_with("data:") || href.starts_with("javascript:") {
        return None;
    }
    let lower = href.to_lowercase();
    if lower.starts_with("https://") || lower.starts_with("http://") {
        return Some(href.to_owned());
    }
    let (scheme, rest) = base.split_once("://")?;
    let origin_end = rest.find('/').unwrap_or(rest.len());
    let origin = format!("{scheme}://{}", &rest[..origin_end]);
    if let Some(h) = href.strip_prefix("//") {
        return Some(format!("https://{h}"));
    }
    if href.starts_with('/') {
        return Some(format!("{origin}{href}"));
    }
    let path = &rest[origin_end..];
    let path = path.split(['?', '#']).next().unwrap_or("");
    let dir = path.rfind('/').map(|i| &path[..=i]).unwrap_or("/");
    Some(format!("{origin}{dir}{href}"))
}

/// Decodes an icon (PNG, ICO, JPEG, GIF, WebP or SVG) and returns a 64x64 PNG.
pub fn to_png(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).to_lowercase();
    let rgba = if head.contains("<svg") {
        // Website SVGs are untrusted: usvg's default image resolver opens local/UNC
        // files, and embedded raster data would bypass our image::Limits below.
        let mut options = resvg::usvg::Options::default();
        options.image_href_resolver.resolve_string = Box::new(|_, _| None);
        options.image_href_resolver.resolve_data = Box::new(|_, _, _| None);
        let tree = resvg::usvg::Tree::from_data(bytes, &options).map_err(|e| e.to_string())?;
        let mut pm = resvg::tiny_skia::Pixmap::new(ICON_PX, ICON_PX).ok_or("pixmap")?;
        let s = tree.size();
        let scale = ICON_PX as f32 / s.width().max(s.height()).max(1.0);
        resvg::render(
            &tree,
            resvg::tiny_skia::Transform::from_scale(scale, scale),
            &mut pm.as_mut(),
        );
        let data: Vec<u8> = pm
            .pixels()
            .iter()
            .flat_map(|p| {
                let c = p.demultiply();
                [c.red(), c.green(), c.blue(), c.alpha()]
            })
            .collect();
        image::RgbaImage::from_raw(ICON_PX, ICON_PX, data).ok_or("svg buffer")?
    } else {
        let mut reader = image::ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|e| e.to_string())?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(4096);
        limits.max_image_height = Some(4096);
        limits.max_alloc = Some(64 * 1024 * 1024);
        reader.limits(limits);
        let img = reader.decode().map_err(|e| e.to_string())?;
        if img.width() < 16 || img.height() < 16 {
            return Err("icon too small".into());
        }
        img.resize(ICON_PX, ICON_PX, image::imageops::FilterType::Lanczos3)
            .to_rgba8()
    };
    if rgba.pixels().all(|p| p.0[3] == 0) {
        return Err("icon is empty".into());
    }
    let rgba = backplate_if_dark(rgba);
    let mut out = Vec::new();
    image::DynamicImage::ImageRgba8(rgba)
        .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(out)
}

/// Dark logos on transparent backgrounds (made for light browser tabs, like GitHub's) disappear
/// on a dark UI. Those get a light rounded tile behind them.
fn backplate_if_dark(img: image::RgbaImage) -> image::RgbaImage {
    let (mut lum, mut opaque, mut clear) = (0.0f64, 0u32, 0u32);
    for p in img.pixels() {
        let [r, g, b, a] = p.0;
        if a > 128 {
            lum += (0.2126 * r as f64 + 0.7152 * g as f64 + 0.0722 * b as f64) / 255.0;
            opaque += 1;
        } else {
            clear += 1;
        }
    }
    let total = opaque + clear;
    if opaque == 0 || lum / (opaque as f64) > 0.32 || (clear as f64) < 0.25 * total as f64 {
        return img;
    }
    let (w, h) = img.dimensions();
    let r = (w as f32) * 0.2;
    let inside = |x: f32, y: f32| {
        let cx = x.clamp(r, w as f32 - r);
        let cy = y.clamp(r, h as f32 - r);
        (x - cx).powi(2) + (y - cy).powi(2) <= r * r
    };
    let mut out = image::RgbaImage::new(w, h);
    for (x, y, px) in out.enumerate_pixels_mut() {
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let bg = if inside(fx, fy) {
            [0xee, 0xf4, 0xf8, 255u8]
        } else {
            [0, 0, 0, 0]
        };
        // Logo scaled to 80% and centred on the tile.
        let sx = ((fx - w as f32 * 0.1) / 0.8) as i64;
        let sy = ((fy - h as f32 * 0.1) / 0.8) as i64;
        let fg = if (0..w as i64).contains(&sx) && (0..h as i64).contains(&sy) {
            img.get_pixel(sx as u32, sy as u32).0
        } else {
            [0, 0, 0, 0]
        };
        let a = fg[3] as f32 / 255.0;
        let mix = |i: usize| (fg[i] as f32 * a + bg[i] as f32 * (1.0 - a)).round() as u8;
        *px = image::Rgba([mix(0), mix(1), mix(2), bg[3].max(fg[3])]);
    }
    out
}

const MAX_REDIRECTS: usize = 5;

/// GETs `url`, following up to five redirects by hand so each hop is checked: HTTPS, port 443,
/// same site as the original URL, and (through the agent's resolver) public addresses only.
/// The body is capped after decompression, so a small gzip response cannot expand past `limit`.
fn get_bytes(agent: &ureq::Agent, url: &str, limit: u64) -> Result<(Vec<u8>, String), String> {
    public_uri(url).ok_or("Icon URL is not an allowed public HTTPS origin")?;
    let mut current = url.to_owned();
    for _ in 0..=MAX_REDIRECTS {
        let mut resp = agent.get(&current).call().map_err(|e| e.to_string())?;
        if resp.status().is_redirection() {
            let location = resp
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .ok_or("Redirect without a location")?;
            let next = resolve(&current, location.trim()).ok_or("Unusable redirect")?;
            if !same_site(url, &next) {
                return Err("Redirect leaves the site; not followed".into());
            }
            current = next;
            continue;
        }
        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status()));
        }
        let final_url = resp.get_uri().to_string();
        return Ok((read_capped(resp.body_mut(), limit)?, final_url));
    }
    Err("Too many redirects".into())
}

/// Reads a body, decompressed, refusing anything over `limit` bytes after decoding.
fn read_capped(body: &mut ureq::Body, limit: u64) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    body.with_config()
        .limit(limit.saturating_mul(64))
        .reader()
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("Response too large".into());
    }
    Ok(bytes)
}

/// Finds and converts the best icon for a host (trying its parent domain too).
pub fn fetch(host: &str) -> Result<Vec<u8>, String> {
    let agent = agent();
    let mut hosts = vec![host.to_owned()];
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() > 2 {
        hosts.push(labels[labels.len() - 2..].join("."));
    }
    let mut last_err = String::from("no icon found");
    for h in hosts {
        let page = format!("https://{h}/");
        let mut candidates = match get_bytes(&agent, &page, MAX_PAGE) {
            Ok((html, final_url)) => icon_links(&String::from_utf8_lossy(&html), &final_url),
            Err(e) => {
                last_err = e;
                Vec::new()
            }
        };
        candidates.push(Candidate {
            url: format!("https://{h}/apple-touch-icon.png"),
            score: 1,
        });
        candidates.push(Candidate {
            url: format!("https://{h}/favicon.ico"),
            score: 0,
        });
        for c in candidates.iter().take(6) {
            match get_bytes(&agent, &c.url, MAX_IMAGE).and_then(|(b, _)| to_png(&b)) {
                Ok(png) => return Ok(png),
                Err(e) => last_err = e,
            }
        }
    }
    Err(last_err)
}

/// Per-host outcome of a batch: a 64 px PNG or why there was none.
pub type IconResults = Vec<(String, Result<Vec<u8>, String>)>;

/// Progress shared with the UI while a batch runs.
#[derive(Default)]
pub struct Progress {
    pub done: AtomicUsize,
    pub total: AtomicUsize,
}

/// Fetches icons for many hosts on a small worker pool. Returns (host, result) pairs.
pub fn fetch_all(
    hosts: Vec<String>,
    progress: Arc<Progress>,
) -> Vec<(String, Result<Vec<u8>, String>)> {
    fetch_all_with(hosts, progress, fetch)
}

/// `fetch_all` with the per-host fetch passed in (tests).
fn fetch_all_with(
    hosts: Vec<String>,
    progress: Arc<Progress>,
    fetch: fn(&str) -> Result<Vec<u8>, String>,
) -> IconResults {
    progress.total.store(hosts.len(), Ordering::Relaxed);
    let queue = Arc::new(Mutex::new(hosts.into_iter().collect::<VecDeque<_>>()));
    let results = Arc::new(Mutex::new(Vec::new()));
    let workers: Vec<_> = (0..WORKERS)
        .map(|_| {
            let (queue, results, progress) = (queue.clone(), results.clone(), progress.clone());
            std::thread::spawn(move || {
                loop {
                    let Some(host) = queue.lock().unwrap().pop_front() else {
                        break;
                    };
                    // A panic while fetching or decoding counts as that site failing (never
                    // dropped silently), and the worker goes on to the next host.
                    let r = std::panic::catch_unwind(|| fetch(&host))
                        .unwrap_or_else(|_| Err("icon fetch stopped unexpectedly".into()));
                    results.lock().unwrap().push((host, r));
                    progress.done.fetch_add(1, Ordering::Relaxed);
                }
            })
        })
        .collect();
    for w in workers {
        let _ = w.join();
    }
    Arc::try_unwrap(results)
        .map(|m| m.into_inner().unwrap())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_whose_fetch_panics_counts_as_failed() {
        let hosts: Vec<String> = (0..20).map(|i| format!("site{i}.example")).collect();
        let progress = Arc::new(Progress::default());
        let results = fetch_all_with(hosts, progress.clone(), |host| {
            if host == "site3.example" {
                panic!("icon worker panic canary");
            }
            Ok(host.as_bytes().to_vec())
        });
        assert_eq!(results.len(), 20, "no host is dropped");
        assert_eq!(progress.done.load(Ordering::Relaxed), 20);
        let failed: Vec<_> = results.iter().filter(|(_, r)| r.is_err()).collect();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].0, "site3.example");
        assert!(!failed[0].1.as_ref().unwrap_err().contains("canary"));
    }

    #[test]
    fn hosts_are_public_web_names_only() {
        assert_eq!(
            icon_host("https://secure.bankofamerica.com/login/sign-in?x=1").as_deref(),
            Some("secure.bankofamerica.com")
        );
        assert_eq!(
            icon_host("http://www.chase.com").as_deref(),
            Some("www.chase.com")
        );
        assert_eq!(icon_host("http://192.168.50.1"), None);
        assert_eq!(icon_host("https://localhost:8080"), None);
        assert_eq!(icon_host("https://nas.local"), None);
        assert_eq!(icon_host("ftp://example.com"), None);
        assert_eq!(icon_host("cmd://evil"), None);
        assert_eq!(
            icon_host("https://user:pw@example.com/path").as_deref(),
            Some("example.com"),
            "credentials never used"
        );
    }

    #[test]
    fn finds_and_ranks_icon_links() {
        let html = r#"<html><head>
            <link rel="icon" href="/favicon-32.png" sizes="32x32">
            <link rel='apple-touch-icon' href='apple-icon.png'>
            <link rel="mask-icon" href="/mask.svg" color="black">
            <LINK REL="shortcut icon" HREF="//cdn.example.net/fav.ico">
            <link rel="stylesheet" href="/site.css">
        </head></html>"#;
        let c = icon_links(html, "https://www.example.com/en/home?a=1");
        let urls: Vec<&str> = c.iter().map(|c| c.url.as_str()).collect();
        assert_eq!(
            urls[0], "https://www.example.com/en/apple-icon.png",
            "apple touch icon first, relative to the page"
        );
        assert!(urls.contains(&"https://www.example.com/favicon-32.png"));
        assert!(!urls.contains(&"https://cdn.example.net/fav.ico"));
        assert!(
            !urls
                .iter()
                .any(|u| u.contains("mask") || u.contains(".css"))
        );
    }

    #[test]
    fn icon_requests_cannot_escape_the_public_origin() {
        let html = "<link rel='icon' href='http://127.0.0.1:8080/secret'><link rel='icon' href='https://cdn.other.example/icon'><link rel='icon' href='/safe.png'>";
        let links = icon_links(html, "https://public.example/");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].url, "https://public.example/safe.png");
        assert!(get_bytes(&agent(), "http://127.0.0.1:8080/", MAX_IMAGE).is_err());
        assert!(!same_site(
            "https://public.example/",
            "https://user:pw@public.example/a"
        ));
        assert!(!same_site(
            "https://public.example/",
            "https://public.example:8443/a"
        ));
        assert!(!same_site(
            "https://public.example/",
            "http://www.public.example/a"
        ));
        assert!(same_site(
            "https://public.example/",
            "https://www.public.example/a"
        ));
        assert!(!same_site("https://bank.co.uk/", "https://other.co.uk/"));
        assert!(same_site("https://bank.co.uk/", "https://www.bank.co.uk/"));
        assert_eq!(agent().config().max_redirects(), 0);
        assert!(agent().config().proxy().is_none());
    }

    #[test]
    fn dns_answers_are_filtered_and_pinned() {
        use ureq::unversioned::resolver::{ResolvedSocketAddrs, Resolver};
        use ureq::unversioned::transport::NextTimeout;
        #[derive(Debug)]
        struct Fixed(Vec<std::net::SocketAddr>);
        impl Resolver for Fixed {
            fn resolve(
                &self,
                _: &ureq::http::Uri,
                _: &ureq::config::Config,
                _: NextTimeout,
            ) -> Result<ResolvedSocketAddrs, ureq::Error> {
                let mut result = self.empty();
                for a in &self.0 {
                    result.push(*a);
                }
                Ok(result)
            }
        }
        let timeout = || NextTimeout {
            after: ureq::unversioned::transport::time::Duration::NotHappening,
            reason: ureq::Timeout::Global,
        };
        let uri = "https://public.example/".parse().unwrap();
        let config = ureq::config::Config::default();
        for ip in [
            "0.0.0.0",
            "10.0.0.1",
            "100.64.0.1",
            "127.0.0.1",
            "169.254.169.254",
            "172.16.0.1",
            "192.168.1.1",
            "198.18.0.1",
            "224.0.0.1",
            "::1",
            "::ffff:127.0.0.1",
            "fc00::1",
            "fe80::1",
            "2001:db8::1",
            "2002:7f00:1::",
            "3fff::1",
        ] {
            let address = std::net::SocketAddr::new(ip.parse().unwrap(), 443);
            // Mixed public/private DNS answers must also fail closed.
            let resolver = PublicResolver(Fixed(vec!["1.1.1.1:443".parse().unwrap(), address]));
            assert!(resolver.resolve(&uri, &config, timeout()).is_err(), "{ip}");
        }
        let expected = vec![
            "1.1.1.1:443".parse().unwrap(),
            "[2606:4700:4700::1111]:443".parse().unwrap(),
        ];
        let resolved = PublicResolver(Fixed(expected.clone()))
            .resolve(&uri, &config, timeout())
            .unwrap();
        assert_eq!(resolved.iter().copied().collect::<Vec<_>>(), expected);
    }

    /// One-shot local HTTP/1.1 server answering `responses` in order; returns its port.
    fn serve(responses: Vec<Vec<u8>>) -> u16 {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for body in responses {
                let Ok((mut s, _)) = listener.accept() else {
                    return;
                };
                let mut buf = [0u8; 4096];
                let _ = s.read(&mut buf);
                let _ = s.write_all(&body);
            }
        });
        port
    }

    #[test]
    fn decompressed_bodies_are_capped_and_large_icon_sizes_do_not_overflow() {
        use std::io::Write;
        // Plain-HTTP loopback is refused by policy, so exercise the capped reader directly
        // through an agent without the public-address policy.
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        gz.write_all(&vec![0u8; 8 * 1024 * 1024]).unwrap();
        let gz = gz.finish().unwrap();
        let mut head = format!(
            "HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            gz.len()
        )
        .into_bytes();
        head.extend_from_slice(&gz);
        let port = serve(vec![head]);
        let plain: ureq::Agent = ureq::Agent::config_builder().build().into();
        let mut resp = plain
            .get(&format!("http://127.0.0.1:{port}/"))
            .call()
            .unwrap();
        // 8 MB of zeros arrive as a few KB of gzip; the decoded size is what gets capped.
        assert_eq!(
            read_capped(resp.body_mut(), MAX_IMAGE),
            Err("Response too large".into())
        );

        let html = "<link rel='apple-touch-icon' sizes='4294967295x1' href='/a.png'>";
        assert_eq!(icon_links(html, "https://public.example/").len(), 1);
    }

    #[test]
    fn downloaded_svg_cannot_include_local_or_embedded_images() {
        let path = std::env::temp_dir().join(format!("axv-svg-{}.png", uuid::Uuid::new_v4()));
        image::RgbaImage::from_pixel(32, 32, image::Rgba([255, 0, 0, 255]))
            .save(&path)
            .unwrap();
        let svg = |href: &str| {
            format!(
                "<svg xmlns='http://www.w3.org/2000/svg' xmlns:xlink='http://www.w3.org/1999/xlink' width='64' height='64'><image width='64' height='64' xlink:href='{href}'/></svg>"
            )
        };
        assert!(to_png(svg(&path.display().to_string()).as_bytes()).is_err());
        assert!(to_png(svg("data:image/png;base64,aGVsbG8=").as_bytes()).is_err());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn converts_images_to_64px_png() {
        let mut src = Vec::new();
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            32,
            32,
            image::Rgba([10, 200, 90, 255]),
        ))
        .write_to(&mut Cursor::new(&mut src), image::ImageFormat::Png)
        .unwrap();
        let png = to_png(&src).unwrap();
        let back = image::load_from_memory(&png).unwrap();
        assert_eq!((back.width(), back.height()), (64, 64));

        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect width="10" height="10" fill="#f00"/></svg>"##;
        assert_eq!(
            image::load_from_memory(&to_png(svg).unwrap())
                .unwrap()
                .width(),
            64
        );
        assert!(to_png(b"not an image").is_err());

        // A black logo on transparency gets a light tile behind it.
        let mut dark = image::RgbaImage::new(64, 64);
        for (x, y, p) in dark.enumerate_pixels_mut() {
            if (20..44).contains(&x) && (20..44).contains(&y) {
                *p = image::Rgba([0, 0, 0, 255]);
            }
        }
        let mut src = Vec::new();
        image::DynamicImage::ImageRgba8(dark)
            .write_to(&mut Cursor::new(&mut src), image::ImageFormat::Png)
            .unwrap();
        let out = image::load_from_memory(&to_png(&src).unwrap())
            .unwrap()
            .to_rgba8();
        assert!(
            out.get_pixel(10, 32).0[0] > 200,
            "light tile behind a dark logo"
        );
    }
}

/// Live check against real sites (network; not run by default):
/// `cargo test -p axentra-vault live_fetch -- --ignored --nocapture`
#[cfg(test)]
#[test]
#[ignore]
fn live_fetch() {
    let out = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("assets")
        .join("preview")
        .join("icons");
    std::fs::create_dir_all(&out).unwrap();
    let hosts = [
        "github.com",
        "www.chase.com",
        "secure.bankofamerica.com",
        "outlook.live.com",
        "www.netflix.com",
    ];
    for (host, r) in fetch_all(
        hosts.iter().map(|h| h.to_string()).collect(),
        Arc::new(Progress::default()),
    ) {
        match r {
            Ok(png) => {
                std::fs::write(out.join(format!("{host}.png")), &png).unwrap();
                println!("ICON {host}: {} bytes", png.len());
            }
            Err(e) => println!("ICON {host}: FAILED {e}"),
        }
    }
}
