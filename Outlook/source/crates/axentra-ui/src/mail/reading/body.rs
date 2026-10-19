//! The message body document rendered in a sandboxed iframe (`srcdoc`,
//! `sandbox="allow-popups"`, no scripts, no same-origin): sanitised HTML,
//! a Content-Security-Policy that blocks remote content unless allowed,
//! quoted history split off behind "Show message history", theme colours,
//! zoom and a height estimate (the parent can't measure a sandboxed frame).

use axentra_core::MessageBody;
use axentra_core::html::sanitize_email_html;

/// Body colours. The frame is a separate document and can't see the app's
/// CSS variables, so these mirror the `--ax-bg-1` / `--ax-fg-1` /
/// `--ax-fg-link` token values of each theme.
const LIGHT: (&str, &str, &str) = ("#ffffff", "#242424", "#115ea3");
const DARK: (&str, &str, &str) = ("#292929", "#ffffff", "#479ef5");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyTheme {
    Light,
    Dark,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BodyDoc {
    pub srcdoc: String,
    pub height: u32,
    /// Remote content was present and blocked.
    pub blocked: bool,
    /// There is quoted history that is currently hidden.
    pub has_history: bool,
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Plain text to HTML: escaped, URLs linked, `>` quotes split off.
fn text_to_html(text: &str) -> (String, Option<String>) {
    let mut main = Vec::new();
    let mut quoted = Vec::new();
    let mut in_quote = false;
    for line in text.lines() {
        if line.trim_start().starts_with('>') || in_quote && line.trim().is_empty() {
            in_quote = true;
            quoted.push(line);
        } else if in_quote {
            quoted.push(line);
        } else {
            main.push(line);
        }
    }
    let render = |lines: &[&str]| -> String {
        let linked: Vec<String> = lines
            .iter()
            .map(|l| {
                l.split(' ')
                    .map(|w| {
                        if w.starts_with("http://") || w.starts_with("https://") {
                            let e = escape(w);
                            format!("<a href=\"{e}\">{e}</a>")
                        } else {
                            escape(w)
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect();
        format!("<div style=\"white-space:pre-wrap\">{}</div>", linked.join("\n"))
    };
    let q = (!quoted.is_empty()).then(|| render(&quoted));
    (render(&main), q)
}

/// Split raw HTML before the quoted history Outlook and others insert
/// (`divRplyFwdMsg` with its rule, `blockquote type=cite`, `gmail_quote`).
pub fn split_history(html: &str) -> (&str, Option<&str>) {
    let lower = html.to_ascii_lowercase();
    let mut cut: Option<usize> = None;
    if let Some(i) = lower.find("id=\"divrplyfwdmsg\"").or_else(|| lower.find("id=divrplyfwdmsg")) {
        let tag_start = lower[..i].rfind('<').unwrap_or(i);
        // Include the horizontal rule just before it.
        let start = match lower[..tag_start].rfind("<hr") {
            Some(h) if tag_start - h < 200 => h,
            _ => tag_start,
        };
        cut = Some(start);
    }
    for marker in ["<blockquote type=\"cite\"", "<div class=\"gmail_quote\""] {
        if let Some(i) = lower.find(marker) {
            cut = Some(cut.map_or(i, |c| c.min(i)));
        }
    }
    match cut {
        Some(c) if c > 0 && !html[..c].trim().is_empty() => (&html[..c], Some(&html[c..])),
        _ => (html, None),
    }
}

/// Drop 1x1 images (tracking pixels), even when remote content is allowed.
pub fn strip_tracking_pixels(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len());
    let mut i = 0;
    while let Some(rel) = lower[i..].find("<img") {
        let start = i + rel;
        let end = lower[start..].find('>').map(|e| start + e + 1).unwrap_or(html.len());
        let tag = &lower[start..end];
        let tiny = |attr: &str| tag.contains(&format!("{attr}=\"1\"")) || tag.contains(&format!("{attr}=1 ")) || tag.contains(&format!("{attr}=\"0\""));
        out.push_str(&html[i..start]);
        if !(tiny("width") && tiny("height")) {
            out.push_str(&html[start..end]);
        }
        i = end;
    }
    out.push_str(&html[i..]);
    out
}

/// True when the HTML references remote resources the sanitiser doesn't
/// count (CSS `url()`, `background` attributes, `srcset`).
fn has_other_remote(html: &str) -> bool {
    let l = html.to_ascii_lowercase();
    ["url(http", "url('http", "url(\"http", "background=\"http", "srcset=\"http"].iter().any(|p| l.contains(p))
}

fn count(hay: &str, needle: &str) -> usize {
    hay.matches(needle).count()
}

/// Height estimate in px for the frame: text lines at the available width,
/// paragraph spacing and images (explicit heights scaled to the width).
pub fn estimate_height(html: &str, width: f64, zoom: u32) -> u32 {
    let z = zoom as f64 / 100.0;
    let usable = (width - 8.0).max(160.0);
    let cpl = (usable / (7.4 * z)).max(16.0);
    let text = axentra_core::html::html_to_text(html);
    let mut lines = 0.0;
    for line in text.lines() {
        let n = line.trim().chars().count() as f64;
        lines += if n == 0.0 { 0.35 } else { (n / cpl).ceil() };
    }
    let lower = html.to_ascii_lowercase();
    let blocks = count(&lower, "<p") + count(&lower, "<h") + count(&lower, "<li") + count(&lower, "<tr");
    let mut images = 0.0;
    let mut i = 0;
    while let Some(rel) = lower[i..].find("<img") {
        let start = i + rel;
        let end = lower[start..].find('>').map(|e| start + e).unwrap_or(lower.len());
        let tag = &lower[start..end];
        let num = |attr: &str| -> Option<f64> {
            let p = tag.find(&format!("{attr}=\""))? + attr.len() + 2;
            tag[p..].split('"').next()?.trim_end_matches("px").parse().ok()
        };
        let (w, h) = (num("width"), num("height"));
        images += match (w, h) {
            (_, Some(h)) if h <= 2.0 => 0.0,
            (Some(w), Some(h)) => h * (usable / (w * z)).min(1.0) * z,
            (Some(w), None) => 0.6 * w.min(usable / z) * z,
            (None, Some(h)) => h * z,
            (None, None) => 180.0 * z,
        };
        i = end.max(start + 4);
    }
    let h = lines * 20.0 * z + blocks as f64 * 12.0 * z + images + 24.0;
    h.clamp(64.0, 8000.0).round() as u32
}

/// Build the frame document for a body.
pub fn build(body: &MessageBody, allow_remote: bool, theme: BodyTheme, zoom: u32, show_history: bool, width: f64) -> BodyDoc {
    let (main_html, history_html, blocked) = match &body.html {
        Some(raw) => {
            let (main, history) = split_history(raw);
            let source = if show_history { raw.as_str() } else { main };
            let clean = sanitize_email_html(source, allow_remote);
            let blocked_any = !allow_remote && (sanitize_email_html(raw, false).blocked_remote_content || has_other_remote(raw));
            let html = if allow_remote { strip_tracking_pixels(&clean.html) } else { clean.html };
            (html, history.map(|_| ()), blocked_any)
        }
        None => {
            let (main, quoted) = text_to_html(&body.text);
            let html = match (&quoted, show_history) {
                (Some(q), true) => format!("{main}{q}"),
                _ => main,
            };
            (html, quoted.map(|_| ()), false)
        }
    };
    let (bg, fg, link) = match theme {
        BodyTheme::Light => LIGHT,
        BodyTheme::Dark => DARK,
    };
    let scheme = if theme == BodyTheme::Dark { "dark" } else { "light" };
    let remote = if allow_remote { " https: http:" } else { "" };
    let srcdoc = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'; img-src data: cid:{remote}; font-src data:{remote}; media-src data:\">\
<base target=\"_blank\"><style>\
:root{{color-scheme:{scheme}}}\
html,body{{margin:0;padding:0;background:{bg};}}\
body{{color:{fg};font-family:\"Segoe UI Variable Text\",\"Segoe UI\",Selawik,Inter,\"Noto Sans\",sans-serif;font-size:14px;line-height:20px;\
overflow-wrap:anywhere;zoom:{zoom}%;padding:2px 2px 8px}}\
a{{color:{link}}}img{{max-width:100%;height:auto}}table{{max-width:100%}}\
blockquote{{margin:0 0 0 8px;padding-left:12px;border-left:3px solid {link}}}\
p{{margin:0 0 12px}}pre{{white-space:pre-wrap}}\
</style></head><body>{main_html}</body></html>"
    );
    let height = estimate_height(&main_html, width, zoom);
    BodyDoc { srcdoc, height, blocked, has_history: history_html.is_some() && !show_history }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_is_split_at_the_reply_header() {
        let html = "<p>Thanks!</p><hr style=\"x\"><div id=\"divRplyFwdMsg\"><b>From:</b> A</div><p>old</p>";
        let (main, hist) = split_history(html);
        assert_eq!(main, "<p>Thanks!</p>");
        assert!(hist.unwrap().starts_with("<hr"));
        assert_eq!(split_history("<p>Just this</p>").1, None);
        // Never hide the whole message.
        assert_eq!(split_history("<blockquote type=\"cite\">all quoted</blockquote>").1, None);
        // An ordinary quotation is not history.
        assert_eq!(split_history("<p>a</p><blockquote>quote</blockquote><p>b</p>").1, None);
    }

    #[test]
    fn remote_content_is_blocked_by_csp_and_flagged() {
        let body = MessageBody { html: Some("<p>Hi</p><img src=\"https://t.example/a.png\" width=\"300\" height=\"100\">".into()), ..Default::default() };
        let doc = build(&body, false, BodyTheme::Light, 100, false, 600.0);
        assert!(doc.blocked);
        assert!(!doc.srcdoc.contains("t.example"));
        assert!(doc.srcdoc.contains("img-src data: cid:;"));
        let allowed = build(&body, true, BodyTheme::Dark, 100, false, 600.0);
        assert!(!allowed.blocked);
        assert!(allowed.srcdoc.contains("t.example"));
        assert!(allowed.srcdoc.contains("img-src data: cid: https: http:"));
        assert!(allowed.srcdoc.contains("color-scheme:dark"));
        // Background images in CSS also count as remote content.
        let css = MessageBody { html: Some("<div style=\"background:url(https://x.example/bg.png)\">x</div>".into()), ..Default::default() };
        assert!(build(&css, false, BodyTheme::Light, 100, false, 600.0).blocked);
    }

    #[test]
    fn tracking_pixels_are_removed() {
        let out = strip_tracking_pixels("<p>a</p><img src=\"https://t/p.gif\" width=\"1\" height=\"1\"><img src=\"https://t/b.png\" width=\"20\" height=\"1\">");
        assert!(!out.contains("p.gif"));
        assert!(out.contains("b.png"));
    }

    #[test]
    fn plain_text_bodies_link_and_quote() {
        let body = MessageBody { html: None, text: "See https://x.example/a?b=1&c=2\n\n> old line\n> more".into(), ..Default::default() };
        let doc = build(&body, false, BodyTheme::Light, 100, false, 600.0);
        assert!(doc.srcdoc.contains("<a href=\"https://x.example/a?b=1&amp;c=2\">"));
        assert!(doc.has_history);
        assert!(!doc.srcdoc.contains("old line"));
        assert!(build(&body, false, BodyTheme::Light, 100, true, 600.0).srcdoc.contains("old line"));
    }

    #[test]
    fn heights_grow_with_content_and_zoom() {
        let short = estimate_height("<p>Hello</p>", 600.0, 100);
        let long = estimate_height(&"<p>Lorem ipsum dolor sit amet, consectetur adipiscing elit.</p>".repeat(20), 600.0, 100);
        let zoomed = estimate_height(&"<p>Lorem ipsum dolor sit amet, consectetur adipiscing elit.</p>".repeat(20), 600.0, 150);
        assert!(short < long && long < zoomed);
        let img = estimate_height("<img width=\"1200\" height=\"400\">", 600.0, 100);
        assert!((150..=260).contains(&img), "{img}");
    }
}
