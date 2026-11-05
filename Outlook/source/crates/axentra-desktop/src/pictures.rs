//! Right-clicking a picture in a message. WebKitGTK's own menu offers Save
//! Image As and Copy Image, but the first never asks where to save and the
//! second can't reach into the sandboxed body frame, so neither did anything
//! you could see. Replace them with items the app carries out itself. The
//! picture comes through the webview, which already holds it: as a data:
//! URL when it was sent inside the message, from its site when you chose to
//! load it.

/// The file name to offer for a picture.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn picture_name(uri: &str) -> String {
    const PLAIN: &str = "picture";
    if let Some(data) = uri.strip_prefix("data:") {
        let mime = data.split([';', ',']).next().unwrap_or("").to_ascii_lowercase();
        let ext = match mime.as_str() {
            "image/png" => "png",
            "image/jpeg" | "image/jpg" => "jpg",
            "image/gif" => "gif",
            "image/webp" => "webp",
            "image/bmp" => "bmp",
            "image/tiff" => "tif",
            "image/heic" => "heic",
            "image/svg+xml" => "svg",
            _ => return PLAIN.to_string(),
        };
        return format!("{PLAIN}.{ext}");
    }
    let path = uri.split(['?', '#']).next().unwrap_or("");
    let last = path.rsplit('/').next().unwrap_or("");
    let plain = last.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
    match last.rsplit_once('.') {
        Some((stem, ext)) if plain && !stem.is_empty() && (1..=5).contains(&ext.len()) => last.to_string(),
        _ => PLAIN.to_string(),
    }
}

#[cfg(target_os = "linux")]
pub use linux::use_picture_menu;

#[cfg(target_os = "linux")]
mod linux {
    use std::cell::Cell;
    use std::path::{Path, PathBuf};
    use std::rc::Rc;
    use std::sync::atomic::{AtomicU32, Ordering};

    use dioxus::desktop::DesktopService;
    use dioxus::prelude::*;
    use gtk::prelude::*;
    use gtk::{gdk, gdk_pixbuf, gio, glib};
    use webkit2gtk::{ContextMenuAction, ContextMenuExt, ContextMenuItem, ContextMenuItemExt, Download, DownloadExt, HitTestResultExt, WebView, WebViewExt};

    use super::picture_name;

    /// Put Save picture as… and Copy picture on a picture's menu.
    pub fn use_picture_menu(desktop: &DesktopService) {
        use dioxus::desktop::tao::platform::unix::WindowExtUnix;
        use dioxus::desktop::wry::WebViewExtUnix;
        let webview = desktop.webview.webview();
        let window = desktop.window.gtk_window().clone();
        use_hook(move || {
            webview.connect_context_menu(move |view, menu, _, hit| {
                let Some(uri) = hit.image_uri().filter(|_| hit.context_is_image()).map(|u| u.to_string()) else {
                    return false;
                };
                for item in menu.items() {
                    use ContextMenuAction::*;
                    if matches!(item.stock_action(), OpenImageInNewWindow | DownloadImageToDisk | CopyImageToClipboard | CopyImageUrlToClipboard) {
                        menu.remove(&item);
                    }
                }
                if menu.first().is_some_and(|i| !i.is_separator()) {
                    menu.prepend(&ContextMenuItem::new_separator());
                }
                let item = |name: &str, label: &str, run: Box<dyn Fn()>| {
                    let action = gio::SimpleAction::new(name, None);
                    action.connect_activate(move |_, _| run());
                    ContextMenuItem::from_gaction(&action, label, None)
                };
                let (v, w, u) = (view.clone(), window.clone(), uri.clone());
                menu.prepend(&item("axentra-copy-picture", "Copy picture", Box::new(move || copy(&v, &w, &u))));
                let (v, w, u) = (view.clone(), window.clone(), uri);
                menu.prepend(&item("axentra-save-picture", "Save picture as\u{2026}", Box::new(move || save(&v, &w, &u))));
                false
            });
        });
    }

    fn save(view: &WebView, window: &gtk::ApplicationWindow, uri: &str) {
        let chooser = gtk::FileChooserNative::new(Some("Save picture"), Some(window), gtk::FileChooserAction::Save, Some("_Save"), Some("_Cancel"));
        chooser.set_current_name(&picture_name(uri));
        chooser.set_do_overwrite_confirmation(true);
        if let Some(dir) = glib::user_special_dir(glib::UserDirectory::Downloads) {
            let _ = chooser.set_current_folder(dir);
        }
        if chooser.run() != gtk::ResponseType::Accept {
            return;
        }
        let Some(path) = chooser.file().and_then(|f| f.path()) else { return };
        let w = window.clone();
        fetch(view, uri, &path, move |got| {
            if !got {
                tell(&w, "Couldn't save the picture.");
            }
        });
    }

    fn copy(view: &WebView, window: &gtk::ApplicationWindow, uri: &str) {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let name = format!("axentra-picture-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
        // Only this user can read this folder, and the file goes right after.
        let path: PathBuf = glib::user_runtime_dir().join(name);
        let (w, file) = (window.clone(), path.clone());
        fetch(view, uri, &path, move |got| {
            let picture = if got { gdk_pixbuf::Pixbuf::from_file(&file).ok() } else { None };
            let _ = std::fs::remove_file(&file);
            match picture {
                Some(p) => gtk::Clipboard::get(&gdk::SELECTION_CLIPBOARD).set_image(&p),
                None => tell(&w, "Couldn't copy the picture."),
            }
        });
    }

    /// Have the webview write the picture to `path`; `done` says if it did.
    fn fetch(view: &WebView, uri: &str, path: &Path, done: impl Fn(bool) + 'static) {
        let Some(download) = view.download_uri(uri) else { return done(false) };
        let Ok(target) = glib::filename_to_uri(path, None) else {
            download.cancel();
            return done(false);
        };
        download.set_allow_overwrite(true);
        download.connect_decide_destination(move |d: &Download, _| {
            d.set_destination(&target);
            true
        });
        let failed = Rc::new(Cell::new(false));
        let f = failed.clone();
        download.connect_failed(move |_, e| {
            tracing::warn!("picture download failed: {e}");
            f.set(true);
        });
        download.connect_finished(move |_| done(!failed.get()));
    }

    fn tell(window: &gtk::ApplicationWindow, text: &str) {
        let dialog = gtk::MessageDialog::new(Some(window), gtk::DialogFlags::MODAL, gtk::MessageType::Warning, gtk::ButtonsType::Ok, text);
        dialog.run();
        dialog.close();
    }
}

#[cfg(test)]
mod tests {
    use super::picture_name;

    #[test]
    fn a_picture_gets_a_sensible_name() {
        assert_eq!(picture_name("data:image/png;base64,iVBOR"), "picture.png");
        assert_eq!(picture_name("data:IMAGE/JPEG;base64,/9j/"), "picture.jpg");
        assert_eq!(picture_name("data:application/octet-stream;base64,AAAA"), "picture");
        assert_eq!(picture_name("https://cdn.example/a/logo-2.png?w=200#x"), "logo-2.png");
        assert_eq!(picture_name("https://cdn.example/a/b"), "picture");
        assert_eq!(picture_name("https://cdn.example/track.php%2F..%2Fx"), "picture");
        assert_eq!(picture_name("https://cdn.example/"), "picture");
    }
}
