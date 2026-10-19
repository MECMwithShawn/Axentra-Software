//! Side effects outside the store: files in the Downloads folder, the
//! desktop's URI handlers (`xdg-open`), the clipboard and file pickers
//! (both through small JavaScript snippets, which work in the desktop
//! webview and in liveview).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use dioxus::prelude::*;

/// `$XDG_DOWNLOAD_DIR`, else the `XDG_DOWNLOAD_DIR` entry of
/// `~/.config/user-dirs.dirs`, else `~/Downloads`.
pub fn download_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_DOWNLOAD_DIR").map(PathBuf::from).filter(|p| p.is_absolute()) {
        return dir;
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config"));
    if let Ok(text) = std::fs::read_to_string(config.join("user-dirs.dirs"))
        && let Some(dir) = parse_user_dirs(&text, &home)
    {
        return dir;
    }
    home.join("Downloads")
}

fn parse_user_dirs(text: &str, home: &Path) -> Option<PathBuf> {
    let line = text.lines().find(|l| l.trim_start().starts_with("XDG_DOWNLOAD_DIR="))?;
    let value = line.split_once('=')?.1.trim().trim_matches('"');
    let path = match value.strip_prefix("$HOME") {
        Some(rest) => home.join(rest.trim_start_matches('/')),
        None => PathBuf::from(value),
    };
    path.is_absolute().then_some(path)
}

/// `dir/stem.ext`, or `dir/stem (1).ext`, ... when taken.
pub fn unique_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let first = dir.join(format!("{stem}.{ext}"));
    if !first.exists() {
        return first;
    }
    (1..)
        .map(|n| dir.join(format!("{stem} ({n}).{ext}")))
        .find(|p| !p.exists())
        .unwrap_or(first)
}

/// Write an export into the Downloads folder; returns the path written.
pub fn save_download(stem: &str, ext: &str, content: &str) -> std::io::Result<PathBuf> {
    let dir = download_dir();
    std::fs::create_dir_all(&dir)?;
    let safe: String = stem.chars().map(|c| if c == '/' || c == '\\' || c.is_control() { '_' } else { c }).collect();
    let path = unique_path(&dir, safe.trim(), ext);
    std::fs::write(&path, content)?;
    Ok(path)
}

/// Path for display, with the home directory shortened to "~".
pub fn display_path(p: &Path) -> String {
    match std::env::var_os("HOME").map(PathBuf::from) {
        Some(home) if p.starts_with(&home) => format!("~/{}", p.strip_prefix(&home).map(|r| r.display().to_string()).unwrap_or_default()),
        _ => p.display().to_string(),
    }
}

/// URI scheme of `uri` ("tel", "xmpp", ...), lower-case.
pub fn scheme(uri: &str) -> Option<String> {
    let (s, _) = uri.split_once(':')?;
    (!s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))).then(|| s.to_ascii_lowercase())
}

/// IM address as a URI: bare handles become `xmpp:` URIs.
pub fn im_uri(value: &str) -> String {
    let v = value.trim();
    match scheme(v) {
        Some(s) if ["sip", "xmpp", "im", "msteams", "skype", "matrix"].contains(&s.as_str()) => v.to_string(),
        _ => format!("xmpp:{v}"),
    }
}

/// Whether the desktop has a handler for `scheme` (`xdg-mime`), cached.
pub fn has_handler(scheme: &str) -> bool {
    static CACHE: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(v) = cache.lock().ok().and_then(|c| c.get(scheme).copied()) {
        return v;
    }
    let found = std::process::Command::new("xdg-mime")
        .args(["query", "default", &format!("x-scheme-handler/{scheme}")])
        .output()
        .map(|o| o.status.success() && !String::from_utf8_lossy(&o.stdout).trim().is_empty())
        .unwrap_or(false);
    if let Ok(mut c) = cache.lock() {
        c.insert(scheme.to_string(), found);
    }
    found
}

/// Hand a URI to the desktop (`xdg-open`), detached.
pub fn open_uri(uri: &str) -> std::io::Result<()> {
    std::process::Command::new("xdg-open")
        .arg(uri)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
}

/// `tel:` URI for a phone number as typed.
pub fn tel_uri(number: &str) -> String {
    let cleaned: String = number.chars().filter(|c| c.is_ascii_digit() || matches!(c, '+' | '*' | '#')).collect();
    format!("tel:{cleaned}")
}

/// Copy text to the clipboard (async Clipboard API, falling back to
/// `execCommand` where the page is not a secure context).
pub fn copy_to_clipboard(text: &str) {
    let json = serde_json::to_string(text).unwrap_or_else(|_| "\"\"".into());
    document::eval(&format!(
        r#"const t = {json};
        const fallback = () => {{ const a = document.createElement('textarea'); a.value = t; a.style.position = 'fixed'; a.style.opacity = '0';
          document.body.appendChild(a); a.select(); try {{ document.execCommand('copy'); }} catch (e) {{}} a.remove(); }};
        if (navigator.clipboard && window.isSecureContext) {{ navigator.clipboard.writeText(t).catch(fallback); }} else {{ fallback(); }}"#
    ));
}

/// Focus the element with this id (and keep it visible).
pub fn focus_id(id: &str) {
    let json = serde_json::to_string(id).unwrap_or_default();
    document::eval(&format!("const el = document.getElementById({json}); if (el) {{ el.focus(); }}"));
}

/// Focus the first element matching a CSS selector once it exists.
pub fn focus_selector(selector: &str) {
    let json = serde_json::to_string(selector).unwrap_or_default();
    document::eval(&format!(
        "const go = (n) => {{ const el = document.querySelector({json}); if (el) el.focus(); else if (n > 0) requestAnimationFrame(() => go(n - 1)); }}; requestAnimationFrame(() => go(30));"
    ));
}

/// Remember the last focused element of the module outside dialogs and
/// menus, so focus can go back there when they close (keyboard spec 14).
/// Installed once from the module root.
pub const TRACK_FOCUS_JS: &str = r#"if (!window.axPeopleFocusTracked) { window.axPeopleFocusTracked = true;
  document.addEventListener('focusin', e => {
    const t = e.target;
    if (t && t.closest && t.closest('.ax-people') && !t.closest('.ax-dialog-overlay, .ax-menu, .ax-people-jump')) window.axPeopleLastFocus = t;
  }); }"#;

/// Put focus back where it was before a dialog or menu opened: the last
/// focused element if it still exists, else the list's tab stop, else the
/// module root (so module shortcuts keep working).
pub fn restore_focus() {
    document::eval(
        r#"const go = (tries) => {
          // Wait for the dialog / menu to leave the DOM (liveview patches arrive later).
          if (document.querySelector('.ax-dialog-overlay, .ax-menu') && tries > 0) { requestAnimationFrame(() => go(tries - 1)); return; }
          const a = document.activeElement;
          if (a && a !== document.body && a.isConnected) return;
          const last = window.axPeopleLastFocus;
          const el = (last && last.isConnected) ? last
            : (document.querySelector('#ax-people-scroll [tabindex="0"]') || document.querySelector('.ax-people'));
          if (el) el.focus({ preventScroll: true });
        };
        requestAnimationFrame(() => go(30));"#,
    );
}

/// A picked file: name and base64 contents.
#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct PickedFile {
    pub name: String,
    pub data: String,
}

/// Open the browser's file picker and read the chosen file as base64.
/// Resolves to `None` when the picker is cancelled.
pub async fn pick_file(accept: &str) -> Option<PickedFile> {
    let accept = serde_json::to_string(accept).unwrap_or_default();
    let mut eval = document::eval(&format!(
        r#"const input = document.createElement('input');
        input.type = 'file'; input.accept = {accept}; input.style.display = 'none';
        document.body.appendChild(input);
        input.addEventListener('cancel', () => {{ input.remove(); dioxus.send(null); }});
        input.addEventListener('change', () => {{
          const f = input.files && input.files[0]; input.remove();
          if (!f) {{ dioxus.send(null); return; }}
          const r = new FileReader();
          r.onload = () => {{ const s = String(r.result); dioxus.send({{ name: f.name, data: s.slice(s.indexOf(',') + 1) }}); }};
          r.onerror = () => dioxus.send(null);
          r.readAsDataURL(f);
        }});
        input.click();"#
    ));
    eval.recv::<Option<PickedFile>>().await.ok().flatten()
}

/// An image picked for a contact photo, as a JPEG data URL no larger than
/// [`PHOTO_WORK_PX`] on its long side, with its pixel size.
#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct PickedImage {
    pub url: String,
    pub width: f64,
    pub height: f64,
}

/// Long side of the working copy shown in the crop step.
pub const PHOTO_WORK_PX: u32 = 1296;
/// Side of the saved photo (docs/spec/people-todo.md 2.6.4).
pub const PHOTO_PX: u32 = 648;

/// Open the file picker for a JPEG, PNG or WebP image. `Ok(None)` when the
/// picker was cancelled, `Err(())` when the file isn't a readable image.
pub async fn pick_image() -> std::result::Result<Option<PickedImage>, ()> {
    let mut eval = document::eval(&format!(
        r#"const input = document.createElement('input');
        input.type = 'file'; input.accept = 'image/jpeg,image/png,image/webp'; input.style.display = 'none';
        document.body.appendChild(input);
        input.addEventListener('cancel', () => {{ input.remove(); dioxus.send(null); }});
        input.addEventListener('change', () => {{
          const f = input.files && input.files[0]; input.remove();
          if (!f) {{ dioxus.send(null); return; }}
          const url = URL.createObjectURL(f);
          const img = new Image();
          img.onload = () => {{
            const k = Math.min(1, {PHOTO_WORK_PX} / Math.max(img.naturalWidth, img.naturalHeight));
            const c = document.createElement('canvas');
            c.width = Math.max(1, Math.round(img.naturalWidth * k)); c.height = Math.max(1, Math.round(img.naturalHeight * k));
            const g = c.getContext('2d'); g.fillStyle = '#fff'; g.fillRect(0, 0, c.width, c.height);
            g.drawImage(img, 0, 0, c.width, c.height);
            URL.revokeObjectURL(url);
            dioxus.send({{ url: c.toDataURL('image/jpeg', 0.92), width: c.width, height: c.height }});
          }};
          img.onerror = () => {{ URL.revokeObjectURL(url); dioxus.send({{ url: '', width: 0, height: 0 }}); }};
          img.src = url;
        }});
        input.click();"#
    ));
    match eval.recv::<Option<PickedImage>>().await {
        Ok(Some(img)) if img.url.starts_with("data:image/") && img.width > 0.0 && img.height > 0.0 => Ok(Some(img)),
        Ok(Some(_)) => Err(()),
        _ => Ok(None),
    }
}

/// Cut the square `(x, y, side)` (in the image's pixels) out of `url` and
/// re-encode it as a [`PHOTO_PX`]-square JPEG data URL.
pub async fn crop_photo(url: &str, (x, y, side): (f64, f64, f64)) -> Option<String> {
    let url = serde_json::to_string(url).unwrap_or_default();
    let mut eval = document::eval(&format!(
        r#"const img = new Image();
        img.onload = () => {{
          const c = document.createElement('canvas'); c.width = {PHOTO_PX}; c.height = {PHOTO_PX};
          const g = c.getContext('2d'); g.fillStyle = '#fff'; g.fillRect(0, 0, {PHOTO_PX}, {PHOTO_PX});
          g.drawImage(img, {x}, {y}, {side}, {side}, 0, 0, {PHOTO_PX}, {PHOTO_PX});
          dioxus.send(c.toDataURL('image/jpeg', 0.85));
        }};
        img.onerror = () => dioxus.send('');
        img.src = {url};"#
    ));
    eval.recv::<String>().await.ok().filter(|u| u.starts_with("data:image/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_dirs_and_unique_names() {
        let home = Path::new("/home/x");
        assert_eq!(parse_user_dirs("# c\nXDG_DOWNLOAD_DIR=\"$HOME/Téléchargements\"\n", home), Some(PathBuf::from("/home/x/Téléchargements")));
        assert_eq!(parse_user_dirs("XDG_DOWNLOAD_DIR=\"/data/dl\"", home), Some(PathBuf::from("/data/dl")));
        assert_eq!(parse_user_dirs("XDG_MUSIC_DIR=\"$HOME/Music\"", home), None);
        let dir = std::env::temp_dir().join(format!("ax-people-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(unique_path(&dir, "contacts", "vcf"), dir.join("contacts.vcf"));
        std::fs::write(dir.join("contacts.vcf"), "x").unwrap();
        assert_eq!(unique_path(&dir, "contacts", "vcf"), dir.join("contacts (1).vcf"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn uris() {
        assert_eq!(scheme("tel:+1555"), Some("tel".into()));
        assert_eq!(scheme("no scheme"), None);
        assert_eq!(im_uri("ada@chat.example"), "xmpp:ada@chat.example");
        assert_eq!(im_uri("sip:ada@voip.example"), "sip:ada@voip.example");
        assert_eq!(tel_uri("+1 (415) 555-0100"), "tel:+14155550100");
    }
}
