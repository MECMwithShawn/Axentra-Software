//! The message body document rendered in a sandboxed iframe (`srcdoc`,
//! `sandbox="allow-popups"`, no scripts, no same-origin): sanitised HTML,
//! a Content-Security-Policy that blocks remote content unless allowed,
//! quoted history split off behind "Show message history", theme colours,
//! zoom and a height estimate (the parent can't measure a sandboxed frame).
//! On the dark body the email's colours are adapted the way Outlook's dark
//! mode does it ([`adapt_for_dark`]); the light body shows them as sent.

use axentra_core::MessageBody;
use axentra_core::html::{adapt_for_dark, light_scheme_only, sanitize_email_html};

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

/// Height in px of a blocked picture with alt text and no declared height
/// (the frame's style sheet). WebKitGTK would otherwise size its box by the
/// alt text's proportions: 480x140 for "Brand board" at width 480.
const BLOCKED_ALT_H: f64 = 24.0;

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
        // As the frame's style sheet picks them: no source, alt text, no height.
        let alt_box = !tag.contains(" src=") && !tag.contains(" height=") && tag.find(" alt=\"").is_some_and(|p| !tag[p + 6..].starts_with('"'));
        // A blocked picture (no source) counts at its declared size too: the
        // frame can't be measured, so too short would clip the end of the
        // message. With alt text and no height it is a one-line box (the
        // frame's style sheet), not a picture-sized one.
        images += match (w, h) {
            (_, Some(h)) if h <= 2.0 => 0.0,
            _ if alt_box => BLOCKED_ALT_H * z,
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
            // Dark body: the email's own colours are adapted (dark text
            // lightened, light boxes darkened) for display only. Whether
            // remote pictures load decides which backgrounds are pictures.
            // A failure there must never take the app down: the message
            // then shows with its own colours.
            // Light body: as sent, the sender's dark-mode rules never
            // applying (the frame's colour-scheme preference follows the
            // app's GTK theme, which may be dark).
            let html = if theme == BodyTheme::Dark {
                std::panic::catch_unwind(|| adapt_for_dark(&html, allow_remote)).unwrap_or_else(|_| {
                    tracing::warn!("dark-mode colour adaptation failed; showing the message's own colours");
                    html.clone()
                })
            } else {
                light_scheme_only(&html).into_owned()
            };
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
body{{color:{fg};font-family:\"Segoe UI Variable Text\",\"Segoe UI\",Selawik,Inter,\"Noto Sans\",sans-serif;font-size:14px;line-height:1.43;\
overflow-wrap:anywhere;zoom:{zoom}%;padding:2px 2px 8px}}\
a{{color:{link}}}img{{max-width:100%;height:auto}}img:not([src]):not([height])[alt]:not([alt=\"\"]){{height:24px}}table{{max-width:100%}}\
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

    /// Writes each corpus file's frame document, dark and light, for the
    /// WebKitGTK contrast check (`<name>.dark.html`, `<name>.light.html`):
    /// `AXENTRA_BODY_CORPUS=<dir of .html/.txt> AXENTRA_BODY_OUT=<dir>
    /// cargo test -p axentra-ui dump_corpus -- --ignored`
    /// (`AXENTRA_BODY_REMOTE=1`: as with remote content shown).
    #[test]
    #[ignore]
    fn dump_corpus() {
        let (Ok(dir), Ok(out)) = (std::env::var("AXENTRA_BODY_CORPUS"), std::env::var("AXENTRA_BODY_OUT")) else {
            panic!("set AXENTRA_BODY_CORPUS and AXENTRA_BODY_OUT");
        };
        let mut entries: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect();
        entries.sort();
        for path in entries {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_owned();
            if ext != "html" && ext != "txt" {
                continue;
            }
            let raw = std::fs::read_to_string(&path).unwrap();
            let body = if ext == "html" { MessageBody { html: Some(raw), ..Default::default() } } else { MessageBody { text: raw, ..Default::default() } };
            let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
            let remote = std::env::var("AXENTRA_BODY_REMOTE").is_ok_and(|v| v == "1");
            for (theme, name) in [(BodyTheme::Dark, "dark"), (BodyTheme::Light, "light")] {
                let doc = build(&body, remote, theme, 100, true, 700.0);
                std::fs::write(std::path::Path::new(&out).join(format!("{stem}.{name}.html")), doc.srcdoc).unwrap();
            }
        }
    }

    #[test]
    fn dark_body_adapts_the_emails_colours_light_body_keeps_them() {
        use axentra_core::html::color;
        let html = "<style>.muted{color:#8a8a8a}</style><table bgcolor=\"#ffffff\"><tr><td style=\"color:#333333;padding:4px\">Hi there</td></tr></table><p class=\"muted\">Footer</p>";
        let body = MessageBody { html: Some(html.into()), ..Default::default() };
        // Light: the sanitised HTML exactly as before.
        let light = build(&body, false, BodyTheme::Light, 100, false, 600.0);
        assert!(light.srcdoc.contains(&sanitize_email_html(html, false).html));
        assert!(light.srcdoc.contains("background:#ffffff;") && light.srcdoc.contains("color-scheme:light"));
        // Dark: same structure, the email's colours adapted, the frame's
        // own colours unchanged.
        let dark = build(&body, false, BodyTheme::Dark, 100, false, 600.0);
        for original in ["bgcolor=\"#ffffff\"", "color:#333333", "#8a8a8a"] {
            assert!(!dark.srcdoc.contains(original), "{original}: {}", dark.srcdoc);
        }
        assert!(dark.srcdoc.contains("background:#292929;") && dark.srcdoc.contains("body{color:#ffffff;") && dark.srcdoc.contains("a{color:#479ef5}"));
        assert!(dark.srcdoc.contains(";padding:4px\">Hi there</td>") && dark.srcdoc.contains("<p class=\"muted\">Footer</p>"));
        assert_eq!(dark.height, light.height);
        let cell = dark.srcdoc.split("<table bgcolor=\"").nth(1).unwrap();
        let bg = color::parse(&cell[..7]).unwrap();
        let fg = color::parse(&cell[cell.find("color:").unwrap() + 6..][..7]).unwrap();
        assert!(color::contrast(fg, bg) >= 4.5 && bg.lightness() < 0.35, "{cell}");
        // The adaptation targets the frame's dark canvas and link colour.
        assert_eq!(color::parse(DARK.0), Some(color::DARK_CANVAS));
        assert_eq!(color::parse(DARK.2), Some(color::DARK_LINK));
        // Plain text is never touched.
        let text = MessageBody { text: "Colour #333333 stays".into(), ..Default::default() };
        assert!(build(&text, false, BodyTheme::Dark, 100, false, 600.0).srcdoc.contains(">Colour #333333 stays</div>"));
    }

    #[test]
    fn line_height_scales_with_the_text() {
        // An absolute 20px would be inherited by 44px amounts and headlines.
        let body = MessageBody { html: Some("<p style=\"font-size:44px\">$84.17</p>".into()), ..Default::default() };
        for theme in [BodyTheme::Light, BodyTheme::Dark] {
            let doc = build(&body, false, theme, 100, false, 600.0);
            assert!(doc.srcdoc.contains("font-size:14px;line-height:1.43;") && !doc.srcdoc.contains("line-height:20px"));
        }
    }

    #[test]
    fn the_light_body_never_applies_the_senders_dark_rules() {
        // The frame's prefers-color-scheme follows the app's GTK theme: in a
        // dark app, "Light background" must still show the light design.
        let html = "<style>.bg{background:#ffffff}@media (prefers-color-scheme: dark){.bg{background:#121212!important}}</style><div class=\"bg\">x</div>";
        let body = MessageBody { html: Some(html.into()), ..Default::default() };
        let light = build(&body, false, BodyTheme::Light, 100, false, 600.0);
        assert!(light.srcdoc.contains("@media (max-width:0px){.bg{background:#121212!important}}"), "{}", light.srcdoc);
        // The dark body keeps the sender's dark design as it is.
        let dark = build(&body, false, BodyTheme::Dark, 100, false, 600.0);
        assert!(dark.srcdoc.contains("@media (prefers-color-scheme: dark){.bg{background:#121212!important}}"), "{}", dark.srcdoc);
    }

    #[test]
    fn dark_body_keeps_background_pictures_that_load() {
        // A light picture with dark text over it: once the picture loads
        // (remote content shown), the cell and its text keep their colours;
        // while it is blocked, the fallback colour shows and is adapted.
        let html = "<table><tr><td style=\"background:#f5efe6 url(https://cdn.example/paper.jpg)\"><h1 style=\"color:#2b2118\">Winter Blend</h1></td></tr></table>";
        let body = MessageBody { html: Some(html.into()), ..Default::default() };
        let shown = build(&body, true, BodyTheme::Dark, 100, false, 600.0);
        assert!(shown.srcdoc.contains("background:#f5efe6 url(https://cdn.example/paper.jpg)") && shown.srcdoc.contains("color:#2b2118"), "{}", shown.srcdoc);
        let blocked = build(&body, false, BodyTheme::Dark, 100, false, 600.0);
        assert!(!blocked.srcdoc.contains("#f5efe6") && !blocked.srcdoc.contains("color:#2b2118"), "{}", blocked.srcdoc);
        assert!(blocked.srcdoc.contains("url(https://cdn.example/paper.jpg)"));
        // The light body is the sanitised HTML either way.
        assert!(build(&body, true, BodyTheme::Light, 100, false, 600.0).srcdoc.contains(&sanitize_email_html(html, true).html));
    }

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
        let img = estimate_height("<img src=\"cid:a\" width=\"1200\" height=\"400\">", 600.0, 100);
        assert!((150..=260).contains(&img), "{img}");
    }

    #[test]
    fn blocked_pictures_keep_their_declared_room() {
        // The sanitiser drops a blocked picture's source but keeps the tag,
        // and WebKitGTK draws it as a box of its declared size (with or
        // without alt text): the frame must not come out shorter than that.
        let text = "<p>Hi Alex, the board is below.</p><p>Thanks, Elena</p>";
        let plain = estimate_height(text, 600.0, 100);
        for img in [
            "<img src=\"https://cdn.example/b.png\" width=\"480\" height=\"320\">",
            "<img src=\"https://cdn.example/b.png\" width=\"480\" height=\"320\" alt=\"\">",
            "<img src=\"https://cdn.example/b.png\" width=\"480\" height=\"320\" alt=\"Brand board\">",
        ] {
            let body = MessageBody { html: Some(format!("<p>{img}</p>{text}")), ..Default::default() };
            let blocked = build(&body, false, BodyTheme::Light, 100, false, 600.0);
            assert!(blocked.blocked, "{img}");
            assert!(!blocked.srcdoc.contains("cdn.example/b.png"), "{img}");
            assert!(blocked.height >= plain + 320, "{img}: {} vs {plain}", blocked.height);
            let shown = build(&body, true, BodyTheme::Light, 100, false, 600.0);
            assert_eq!(shown.height, blocked.height, "{img}");
        }
        // Width only, with alt text (Elena's brand board): a one-line box
        // while blocked, a picture once shown.
        let body = MessageBody { html: Some(format!("<p><img src=\"https://cdn.example/b.png\" width=\"480\" alt=\"Brand board\"></p>{text}")), ..Default::default() };
        let blocked = build(&body, false, BodyTheme::Light, 100, false, 600.0);
        assert!(blocked.srcdoc.contains("img:not([src]):not([height])[alt]:not([alt=\"\"]){height:24px}"));
        assert!(blocked.height >= plain + BLOCKED_ALT_H as u32, "{} vs {plain}", blocked.height);
        assert!(blocked.height < plain + 100, "{} vs {plain}", blocked.height);
        let shown = build(&body, true, BodyTheme::Light, 100, false, 600.0);
        assert!(shown.height >= plain + 280, "{} vs {plain}", shown.height);
        // No alt text: the estimate keeps room for a picture-sized box.
        let body = MessageBody { html: Some(format!("<p><img src=\"https://cdn.example/b.png\" width=\"480\"></p>{text}")), ..Default::default() };
        assert!(build(&body, false, BodyTheme::Light, 100, false, 600.0).height >= plain + 280);
    }
}
