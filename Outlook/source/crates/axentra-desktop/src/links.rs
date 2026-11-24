//! Links in a message open in the user's browser. The body is a sandboxed
//! frame whose links open a new window (`<base target="_blank">`), which
//! WebKitGTK asks the app for and Dioxus never answers, so clicks did
//! nothing. Answer it: hand web and mail links to the desktop, and make
//! no window.

use dioxus::desktop::DesktopService;
use dioxus::prelude::*;

/// Schemes handed to the desktop; anything else (file:, javascript:...) is dropped.
fn opens_outside(uri: &str) -> bool {
    let lower = uri.to_ascii_lowercase();
    ["http://", "https://", "mailto:"].iter().any(|s| lower.starts_with(s))
}

/// Open new-window requests (link clicks in message bodies) outside the app.
pub fn use_external_links(desktop: &DesktopService) {
    #[cfg(target_os = "linux")]
    {
        use dioxus::desktop::tao::platform::unix::WindowExtUnix;
        use dioxus::desktop::wry::WebViewExtUnix;
        use webkit2gtk::{URIRequestExt, WebViewExt};
        let webview = desktop.webview.webview();
        let window = desktop.window.gtk_window().clone();
        use_hook(move || {
            webview.connect_create(move |_, action| {
                if let Some(uri) = action.request().and_then(|r| r.uri()).filter(|u| opens_outside(u)) {
                    open(&window, &uri);
                }
                None
            });
        });
    }
}

/// Open `uri` in the user's browser (or mail app), in front. GTK asks the
/// desktop for a fresh activation token for this click: on GNOME Wayland a
/// browser started with the app's own, long-used launch token (what a plain
/// `xdg-open` child inherits) is kept behind other windows or not shown, so
/// clicks seemed to do nothing.
#[cfg(target_os = "linux")]
fn open(window: &gtk::ApplicationWindow, uri: &str) {
    if let Err(e) = gtk::show_uri_on_window(Some(window), uri, gtk::current_event_time()) {
        tracing::warn!("couldn't open a link through the desktop, trying xdg-open: {e}");
        let spawned = std::process::Command::new("xdg-open")
            .arg(uri)
            .env_remove("XDG_ACTIVATION_TOKEN")
            .env_remove("DESKTOP_STARTUP_ID")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        match spawned {
            // Collect it when it exits, so it doesn't linger as a zombie.
            Ok(mut child) => drop(std::thread::spawn(move || child.wait())),
            Err(e) => tracing::warn!("couldn't open a link: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_and_mail_links_leave_the_app() {
        assert!(opens_outside("https://example.com/a"));
        assert!(opens_outside("HTTP://example.com"));
        assert!(opens_outside("mailto:someone@example.com"));
        assert!(!opens_outside("file:///etc/passwd"));
        assert!(!opens_outside("javascript:alert(1)"));
        assert!(!opens_outside("about:blank"));
    }
}
