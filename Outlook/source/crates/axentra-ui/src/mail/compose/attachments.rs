//! Attachments: the file pickers, adding files (picker, drop), the
//! attachment cards, and pictures inserted into the body.

use std::rc::Rc;

use axentra_core::DraftAttachment;
use dioxus::html::FileData;
use dioxus::prelude::*;
use serde_json::json;

use super::actions::{self, SIZE_LIMIT};
use super::state::{ComposeDialog, Form};
use crate::fmt;
use crate::kit::{Button, Dialog, Icon, Menu, MenuItem, Variant, keys};
use crate::state::{AppState, store, use_app};

/// Pictures larger than this are not inlined (mail spec 6.4, estimate).
const PICTURE_LIMIT: u64 = 5 * 1024 * 1024;

/// MIME type from a file name, for files the platform did not label.
pub fn guess_type(name: &str) -> String {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "txt" | "log" | "md" => "text/plain",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "ics" => "text/calendar",
        "eml" => "message/rfc822",
        "json" => "application/json",
        "zip" => "application/zip",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        _ => "application/octet-stream",
    }
    .to_string()
}

/// Fluent icon for an attachment card.
pub fn file_icon(name: &str, content_type: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "pdf" => "document_pdf",
        "xls" | "xlsx" | "csv" | "ods" => "document_table",
        "ppt" | "pptx" | "odp" | "key" => "slide_text",
        "zip" | "7z" | "rar" | "gz" | "tar" | "tgz" => "folder_zip",
        "txt" | "md" | "log" | "doc" | "docx" | "odt" | "rtf" => "document_text",
        "eml" | "msg" => "mail",
        "ics" => "calendar_ltr",
        _ if content_type.starts_with("image/") => "image",
        _ if content_type.starts_with("audio/") => "music_note_2",
        _ if content_type.starts_with("video/") => "video",
        _ => "document",
    }
}

/// Split "Quarterly report.final.pdf" into ("Quarterly report.final", ".pdf")
/// so the card can ellipsize the middle and keep the extension visible.
pub fn split_name(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 && name.len() - i <= 6 => (&name[..i], &name[i..]),
        _ => (name, ""),
    }
}

/// Standard base64, for pictures inserted as `data:` URIs.
pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Decode standard base64 (whitespace ignored); `None` on bad input.
pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let clean: Vec<u8> = s.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if !clean.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(clean.len() / 4 * 3);
    for chunk in clean.chunks(4) {
        let pad = chunk.iter().rev().take_while(|b| **b == b'=').count();
        if pad > 2 {
            return None;
        }
        let mut n = 0u32;
        for (i, b) in chunk.iter().enumerate() {
            let v = if i >= 4 - pad { 0 } else { val(*b)? };
            n = (n << 6) | v;
        }
        let bytes = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        out.extend_from_slice(&bytes[..3 - pad]);
    }
    Some(out)
}

/// Too big for the account's limit (alone or with what is attached)?
pub fn too_large(app: AppState, form: Form, name: &str, size: i64) -> bool {
    let total = actions::attachments_size(&form.draft.peek());
    if size > SIZE_LIMIT || total + size > SIZE_LIMIT {
        app.error(format!(
            "\"{name}\" is too large to attach ({}). The limit for this account is {}.",
            fmt::file_size(size),
            fmt::file_size(SIZE_LIMIT)
        ));
        return true;
    }
    false
}

/// Store one file on the draft (saved first if new).
pub fn add_bytes(app: AppState, mut form: Form, name: &str, content_type: &str, bytes: &[u8]) {
    if too_large(app, form, name, bytes.len() as i64) {
        return;
    }
    let content_type = if content_type.trim().is_empty() { guess_type(name) } else { content_type.to_string() };
    let mut d = actions::current(form);
    let first = d.id.is_none();
    match store().add_draft_attachment(&mut d, name, &content_type, bytes) {
        Ok(_) => {
            let id = d.id;
            form.draft.set(d);
            form.saved.set(*form.dirty.peek());
            if let Some(id) = id {
                if *form.saved_id.peek() != Some(id) {
                    form.saved_id.set(Some(id));
                }
                if first {
                    let mut compose = app.compose;
                    if let Some(r) = compose.write().as_mut() {
                        r.draft = Some(id);
                    }
                }
            }
            app.bump_mail();
        }
        Err(e) => {
            app.error(format!("Couldn't attach \"{name}\": {e}"));
        }
    }
}

/// Attach files from the file picker.
pub async fn add_files(app: AppState, form: Form, files: Vec<FileData>) {
    if files.is_empty() {
        return;
    }
    actions::refresh_body(form).await;
    for f in files {
        let name = f.name();
        if too_large(app, form, &name, f.size() as i64) {
            continue;
        }
        match f.read_bytes().await {
            Ok(bytes) => add_bytes(app, form, &name, &f.content_type().unwrap_or_default(), &bytes),
            Err(e) => {
                app.error(format!("Couldn't read \"{name}\": {e}"));
            }
        }
    }
}

/// Insert pictures into the body as inline images.
pub async fn insert_pictures(app: AppState, form: Form, files: Vec<FileData>) {
    for f in files {
        let name = f.name();
        let content_type = f.content_type().filter(|c| !c.trim().is_empty()).unwrap_or_else(|| guess_type(&name));
        if !content_type.starts_with("image/") {
            app.error(format!("\"{name}\" isn't a picture. Use Attach file to send it."));
            continue;
        }
        if f.size() > PICTURE_LIMIT {
            app.error(format!("\"{name}\" is larger than 5 MB. Attach it as a file instead."));
            continue;
        }
        match f.read_bytes().await {
            Ok(bytes) => {
                let src = format!("data:{content_type};base64,{}", base64(&bytes));
                form.exec("image", json!({ "src": src, "alt": name }));
            }
            Err(e) => {
                app.error(format!("Couldn't read \"{name}\": {e}"));
            }
        }
    }
}

/// Hidden file inputs behind Attach file and Pictures.
#[component]
pub fn FileInputs() -> Element {
    let app = use_app();
    let form = use_context::<Form>();
    rsx! {
        input {
            id: form.dom("file"),
            class: "ax-visually-hidden",
            r#type: "file",
            multiple: true,
            tabindex: "-1",
            "aria-hidden": "true",
            onchange: move |e| { form.spawn(add_files(app, form, e.files())); },
        }
        input {
            id: form.dom("picture"),
            class: "ax-visually-hidden",
            r#type: "file",
            multiple: true,
            accept: "image/*",
            tabindex: "-1",
            "aria-hidden": "true",
            onchange: move |e| { form.spawn(insert_pictures(app, form, e.files())); },
        }
    }
}

/// Attachment cards under the subject. One Tab stop; Left / Right move
/// between cards (keyboard spec 14), Enter previews, Delete removes,
/// Shift+F10 or the chevron opens the card's menu.
#[component]
pub fn AttachmentWell() -> Element {
    let form = use_context::<Form>();
    let list = form.draft.read().attachments.clone();
    let count = list.len();
    let mut current = use_signal(|| 0usize);
    if list.is_empty() {
        return rsx! {};
    }
    let active = current().min(count - 1);
    rsx! {
        div { class: "ax-compose-attachments", role: "list", "aria-label": "Attachments, {count}",
            for (i, a) in list.into_iter().enumerate() {
                AttachmentCard {
                    key: "{i}-{a.path}-{a.name}",
                    index: i,
                    count,
                    attachment: a,
                    tab_stop: i == active,
                    on_focus: move |i| current.set(i),
                }
            }
        }
    }
}

/// Remove attachment `index` from the form's draft.
fn remove(mut form: Form, index: usize) {
    let mut name = String::new();
    form.draft.with_mut(|d| {
        if index < d.attachments.len() {
            name = d.attachments.remove(index).name;
        }
    });
    form.touch();
    form.announce(format!("Removed {name}"));
    let left = form.draft.peek().attachments.len();
    if left > 0 {
        form.focus(&format!("att{}", index.min(left - 1)));
    } else {
        form.exec("focus", serde_json::Value::Null);
    }
}

/// Pictures and text files open in the preview dialog.
pub fn previewable(a: &DraftAttachment) -> bool {
    let t = a.content_type.to_ascii_lowercase();
    a.size <= PREVIEW_LIMIT && (t.starts_with("image/") || t.starts_with("text/") || t == "application/json")
}

/// Largest file the preview dialog loads.
const PREVIEW_LIMIT: i64 = 10 * 1024 * 1024;

#[component]
fn AttachmentCard(index: usize, count: usize, attachment: DraftAttachment, tab_stop: bool, on_focus: EventHandler<usize>) -> Element {
    let mut form = use_context::<Form>();
    let mut menu = use_signal(|| None::<(f64, f64)>);
    let mut anchor = use_signal(|| None::<Rc<MountedData>>);
    let (stem, ext) = split_name(&attachment.name);
    let icon = file_icon(&attachment.name, &attachment.content_type);
    let size = fmt::file_size(attachment.size);
    let name = attachment.name.clone();
    let can_preview = previewable(&attachment);
    let open_menu = move || {
        spawn(async move {
            let rect = match anchor() {
                Some(m) => m.get_client_rect().await.ok(),
                None => None,
            };
            menu.set(Some(rect.map(|r| (r.min_x(), r.max_y() + 4.0)).unwrap_or((0.0, 0.0))));
        });
    };
    let mut items = Vec::new();
    if can_preview {
        items.push(MenuItem::new("preview", "Preview").icon("eye"));
    }
    items.push(MenuItem::new("remove", "Remove attachment").icon("delete"));
    rsx! {
        div {
            id: form.dom(&format!("att{index}")),
            class: "ax-compose-attachment",
            role: "listitem",
            tabindex: if tab_stop { "0" } else { "-1" },
            title: "{name} ({size})",
            "aria-label": "{name}, {size}",
            onmounted: move |e| anchor.set(Some(e.data())),
            onfocus: move |_| on_focus.call(index),
            ondoubleclick: move |_| if can_preview { form.dialog.set(Some(ComposeDialog::Preview(index))) },
            oncontextmenu: move |e: MouseEvent| {
                e.prevent_default();
                e.stop_propagation();
                let p = e.client_coordinates();
                menu.set(Some((p.x, p.y)));
            },
            onkeydown: move |e: KeyboardEvent| {
                let chord = keys::chord(&e);
                match chord.as_str() {
                    "Delete" | "Backspace" => {
                        e.stop_propagation();
                        remove(form, index);
                    }
                    "ArrowLeft" | "ArrowRight" | "Home" | "End" => {
                        e.prevent_default();
                        e.stop_propagation();
                        let to = match chord.as_str() {
                            "ArrowLeft" => index.saturating_sub(1),
                            "ArrowRight" => (index + 1).min(count - 1),
                            "Home" => 0,
                            _ => count - 1,
                        };
                        form.focus(&format!("att{to}"));
                    }
                    "Enter" if can_preview => {
                        e.stop_propagation();
                        form.dialog.set(Some(ComposeDialog::Preview(index)));
                    }
                    "Shift+F10" | "ContextMenu" => {
                        e.prevent_default();
                        e.stop_propagation();
                        open_menu();
                    }
                    _ => {}
                }
            },
            Icon { name: icon, size: 24, class: "ax-compose-attachment-icon" }
            div { class: "ax-compose-attachment-text",
                div { class: "ax-compose-attachment-name",
                    span { class: "ax-compose-attachment-stem", "{stem}" }
                    span { "{ext}" }
                }
                span { class: "ax-compose-attachment-size", "{size}" }
            }
            div { class: "ax-compose-attachment-btns",
                button {
                    r#type: "button",
                    class: "ax-compose-attachment-btn",
                    tabindex: "-1",
                    title: "More actions",
                    "aria-label": "More actions for {name}",
                    "aria-haspopup": "menu",
                    onclick: move |e| { e.stop_propagation(); open_menu(); },
                    Icon { name: "chevron_down", size: 16 }
                }
                button {
                    r#type: "button",
                    class: "ax-compose-attachment-btn",
                    tabindex: "-1",
                    title: "Remove attachment",
                    "aria-label": "Remove {name}",
                    onclick: move |e| { e.stop_propagation(); remove(form, index); },
                    Icon { name: "dismiss", size: 16 }
                }
            }
        }
        if let Some((x, y)) = menu() {
            Menu {
                items,
                x,
                y,
                on_close: move |_| { menu.set(None); form.focus(&format!("att{index}")); },
                on_select: move |id: String| match id.as_str() {
                    "preview" => form.dialog.set(Some(ComposeDialog::Preview(index))),
                    "remove" => remove(form, index),
                    _ => {}
                },
            }
        }
    }
}

/// Look at a picture or text attachment without leaving the message.
#[component]
pub fn PreviewDialog(index: usize) -> Element {
    let mut form = use_context::<Form>();
    let Some(a) = form.draft.peek().attachments.get(index).cloned() else { return rsx! {} };
    let content = use_hook({
        let a = a.clone();
        move || store().draft_attachment_data(&a).ok().flatten()
    });
    let mut close = move || {
        form.dialog.set(None);
        form.focus(&format!("att{index}"));
    };
    let t = a.content_type.to_ascii_lowercase();
    let body = match content {
        None => rsx! { div { class: "ax-compose-preview-empty", "This file can't be previewed." } },
        Some(bytes) if t.starts_with("image/") => {
            let src = format!("data:{};base64,{}", a.content_type, base64(&bytes));
            rsx! { img { class: "ax-compose-preview-img", src: "{src}", alt: "{a.name}" } }
        }
        Some(bytes) => {
            const MAX_TEXT: usize = 200 * 1024;
            let text = String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_TEXT)]).into_owned();
            rsx! { pre { class: "ax-compose-preview-text", "{text}" } }
        }
    };
    rsx! {
        Dialog {
            title: "{a.name}",
            width: 720,
            on_close: move |_| close(),
            actions: rsx! {
                Button { label: "Remove attachment", variant: Variant::Subtle, icon: "delete", class: "ax-compose-dialog-left", onclick: move |_| { form.dialog.set(None); remove(form, index); } }
                Button { label: "Close", variant: Variant::Primary, onclick: move |_| close() }
            },
            div { class: "ax-compose-preview", {body} }
        }
    }
}

/// Attach one of the "Recent files" (a file stored in the app).
pub async fn add_recent(app: AppState, mut form: Form, path: String) {
    actions::refresh_body(form).await;
    let exclude = *form.saved_id.peek();
    let recent = store().recent_attachments(exclude, RECENT_FILES).unwrap_or_default();
    let Some(a) = recent.into_iter().find(|a| a.path == path) else {
        app.error("That file is no longer available.");
        return;
    };
    if too_large(app, form, &a.name, a.size) {
        return;
    }
    let name = a.name.clone();
    form.draft.with_mut(|d| d.attachments.push(a));
    form.touch();
    if actions::save_now(app, form).is_none() {
        app.error(format!("Couldn't attach \"{name}\"."));
    }
}

/// How many recent files Attach file lists (mail spec 6.4).
pub const RECENT_FILES: usize = 10;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_types_and_icons() {
        assert_eq!(split_name("Q4 Plan.final.pptx"), ("Q4 Plan.final", ".pptx"));
        assert_eq!(split_name("README"), ("README", ""));
        assert_eq!(split_name(".bashrc"), (".bashrc", ""));
        assert_eq!(guess_type("photo.JPG"), "image/jpeg");
        assert_eq!(guess_type("noext"), "application/octet-stream");
        assert_eq!(file_icon("a.pdf", "application/pdf"), "document_pdf");
        assert_eq!(file_icon("a.bin", "image/png"), "image");
        assert_eq!(base64(b"hello"), "aGVsbG8=");
    }

    #[test]
    fn base64_round_trip() {
        for s in [&b""[..], b"f", b"fo", b"foo", b"foobar", &[0, 255, 128, 7]] {
            assert_eq!(base64_decode(&base64(s)).unwrap(), s);
        }
        assert_eq!(base64_decode("aGVs\nbG8=").unwrap(), b"hello");
        assert!(base64_decode("abc").is_none());
        assert!(base64_decode("ab!d").is_none());
    }
}
