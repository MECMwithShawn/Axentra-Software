//! Email HTML handling.

use std::collections::HashSet;

/// Result of sanitising an email body for display.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SanitizedHtml {
    pub html: String,
    /// True when remote images were present and were blocked.
    pub blocked_remote_content: bool,
}

/// Sanitise untrusted email HTML for rendering inside a sandboxed iframe.
///
/// Scripts, event handlers, forms and iframes are always removed. When
/// `allow_remote` is false, `http(s)` image sources are dropped (Outlook's
/// "Some content in this message has been blocked" behaviour); `cid:` and
/// `data:` images are kept.
pub fn sanitize_email_html(raw: &str, allow_remote: bool) -> SanitizedHtml {
    let blocked = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = blocked.clone();
    let mut builder = ammonia::Builder::default();
    builder
        .add_generic_attributes(["style", "class", "align", "valign", "width", "height", "bgcolor", "color", "dir"])
        .add_tags(["font", "center", "style"])
        .rm_clean_content_tags(["style"])
        .add_tag_attributes("font", ["face", "size", "color"])
        .add_tag_attributes("table", ["border", "cellpadding", "cellspacing"])
        .add_tag_attributes("td", ["colspan", "rowspan", "nowrap"])
        .add_tag_attributes("th", ["colspan", "rowspan", "nowrap"])
        .url_schemes(HashSet::from(["http", "https", "mailto", "cid", "data", "tel"]))
        .link_rel(Some("noopener noreferrer"))
        .attribute_filter(move |element, attribute, value| {
            if element == "img" && attribute == "src" {
                let v = value.trim_start();
                let remote = v.starts_with("http:") || v.starts_with("https:") || v.starts_with("//");
                if remote && !allow_remote {
                    flag.store(true, std::sync::atomic::Ordering::Relaxed);
                    return None;
                }
            }
            Some(value.into())
        });
    let html = builder.clean(raw).to_string();
    SanitizedHtml { html, blocked_remote_content: blocked.load(std::sync::atomic::Ordering::Relaxed) }
}

/// Convert HTML to readable plain text (used for previews and search).
pub fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len() / 2);
    let mut in_tag = false;
    let mut tag = String::new();
    let mut skip_depth = 0usize;
    let mut chars = html.chars().peekable();
    while let Some(c) = chars.next() {
        if in_tag {
            if c == '>' {
                in_tag = false;
                let t = tag.trim().to_ascii_lowercase();
                let name: String = t.trim_start_matches('/').chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
                if matches!(name.as_str(), "style" | "script" | "head" | "title") {
                    if t.starts_with('/') { skip_depth = skip_depth.saturating_sub(1) } else { skip_depth += 1 }
                }
                if matches!(name.as_str(), "br" | "p" | "div" | "tr" | "li" | "h1" | "h2" | "h3" | "h4" | "table") {
                    out.push('\n');
                }
                tag.clear();
            } else {
                tag.push(c);
            }
            continue;
        }
        if c == '<' {
            in_tag = true;
            continue;
        }
        if skip_depth > 0 {
            continue;
        }
        if c == '&' {
            let mut ent = String::new();
            while let Some(&n) = chars.peek() {
                if n == ';' || ent.len() > 8 { break; }
                ent.push(n);
                chars.next();
            }
            if chars.peek() == Some(&';') { chars.next(); }
            out.push_str(match ent.as_str() {
                "amp" => "&",
                "lt" => "<",
                "gt" => ">",
                "quot" => "\"",
                "apos" | "#39" => "'",
                "nbsp" | "#160" => " ",
                _ => " ",
            });
            continue;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_scripts_and_blocks_remote_images() {
        let s = sanitize_email_html(
            r#"<p onclick="x()">Hi</p><script>alert(1)</script><img src="https://t.example/p.gif"><img src="cid:logo">"#,
            false,
        );
        assert!(!s.html.contains("script"));
        assert!(!s.html.contains("onclick"));
        assert!(!s.html.contains("t.example"));
        assert!(s.html.contains("cid:logo"));
        assert!(s.blocked_remote_content);
        let allowed = sanitize_email_html(r#"<img src="https://t.example/p.gif">"#, true);
        assert!(allowed.html.contains("t.example"));
        assert!(!allowed.blocked_remote_content);
    }

    #[test]
    fn html_to_text_drops_tags_and_styles() {
        let t = html_to_text("<style>p{}</style><p>Hello&nbsp;<b>there</b></p><p>A &amp; B</p>");
        assert_eq!(crate::text::preview(&t, 100), "Hello there A & B");
    }
}
