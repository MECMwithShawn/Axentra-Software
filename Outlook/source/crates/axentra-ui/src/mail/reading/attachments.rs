//! Attachment well under the message header (docs/spec/mail.md §5.5).

use axentra_core::*;
use dioxus::prelude::*;

use super::super::files;
use crate::kit::{Dialog, Icon, Menu, MenuItem};
use crate::state::{AppState, store, use_app};

/// File-type icon for an attachment.
pub fn file_icon(name: &str, content_type: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "doc" | "docx" | "odt" | "rtf" | "pdf" => "document",
        "txt" | "md" | "log" => "document_text",
        "xls" | "xlsx" | "ods" | "csv" | "tsv" => "document_table",
        "ppt" | "pptx" | "odp" | "key" => "slide_text",
        "zip" | "7z" | "rar" | "tar" | "gz" | "tgz" | "xz" | "bz2" => "folder_zip",
        "eml" | "msg" => "mail",
        "ics" => "calendar_ltr",
        "js" | "ts" | "rs" | "py" | "json" | "html" | "htm" | "xml" | "css" | "sh" => "code",
        _ if content_type.starts_with("image/") => "image",
        _ if content_type.starts_with("video/") => "video",
        _ if content_type.starts_with("audio/") => "music_note_2",
        _ => "document",
    }
}

/// "Quarterly report…draft.docx": shorten the middle so the extension stays visible.
pub fn middle_ellipsis(name: &str, max: usize) -> String {
    let chars: Vec<char> = name.chars().collect();
    if chars.len() <= max {
        return name.to_string();
    }
    let ext_len = name.rsplit_once('.').map(|(_, e)| e.chars().count() + 1).unwrap_or(0).min(max / 2);
    let tail = 4 + ext_len;
    let head = max.saturating_sub(tail + 1);
    let start: String = chars[..head].iter().collect();
    let end: String = chars[chars.len() - tail..].iter().collect();
    format!("{start}…{end}")
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { T[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[(n & 63) as usize] as char } else { '=' });
    }
    out
}

fn download(app: AppState, a: &Attachment) {
    match store().attachment_data(a.id) {
        Ok(Some(data)) => match files::save_to_downloads(&a.name, &data) {
            Ok(p) => {
                app.toast(format!("Downloaded {} to {}", a.name, p.parent().map(|d| d.display().to_string()).unwrap_or_default()));
            }
            Err(e) => {
                app.error(format!("Couldn't save {}: {e}", a.name));
            }
        },
        Ok(None) => {
            app.error(format!("{} hasn't been downloaded from the server yet.", a.name));
        }
        Err(e) => {
            app.error(e.to_string());
        }
    }
}

/// Open with the desktop's default app.
fn open_externally(app: AppState, a: &Attachment) {
    match store().attachment_data(a.id) {
        Ok(Some(data)) => {
            if let Err(e) = files::open_with_default_app(&a.name, &data) {
                app.error(format!("Couldn't open {}: {e}", a.name));
            }
        }
        Ok(None) => {
            app.error(format!("{} hasn't been downloaded from the server yet.", a.name));
        }
        Err(e) => {
            app.error(e.to_string());
        }
    }
}

/// "Download all": one zip of the downloaded attachments in Downloads.
fn download_all(app: AppState, list: &[Attachment], subject: &str) {
    let s = store();
    let mut files = Vec::new();
    let mut missing = 0;
    for a in list {
        match s.attachment_data(a.id) {
            Ok(Some(data)) => files.push((a.name.clone(), data)),
            _ => missing += 1,
        }
    }
    if files.is_empty() {
        app.error("None of these attachments have been downloaded from the server yet.");
        return;
    }
    let base = if subject.trim().is_empty() { "Attachments".to_string() } else { subject.trim().to_string() };
    match files::save_to_downloads(&format!("{base}.zip"), &files::zip_stored(&files)) {
        Ok(p) => {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if missing > 0 {
                app.toast(format!("Saved {name} to Downloads. {missing} not downloaded from the server yet were left out."));
            } else {
                app.toast(format!("Saved {name} to Downloads"));
            }
        }
        Err(e) => {
            app.error(format!("Couldn't save the attachments: {e}"));
        }
    }
}

fn previewable(a: &Attachment) -> bool {
    a.content_type.starts_with("image/") || a.content_type.starts_with("text/")
}

#[component]
pub fn AttachmentWell(attachments: Vec<Attachment>, #[props(into, default)] subject: String) -> Element {
    let app = use_app();
    let mut show_all = use_signal(|| false);
    let mut preview = use_signal(|| None::<Attachment>);
    let list: Vec<Attachment> = attachments.into_iter().filter(|a| !a.is_inline).collect();
    if list.is_empty() {
        return rsx! {};
    }
    let n = list.len();
    let downloaded = store().downloaded_attachments(&list.iter().map(|a| a.id).collect::<Vec<_>>()).unwrap_or_default();
    let available: Vec<Attachment> = list.iter().filter(|a| !files::is_blocked_type(&a.name) && downloaded.contains(&a.id)).cloned().collect();
    let shown: Vec<Attachment> = if show_all() { list.clone() } else { list.iter().take(6).cloned().collect() };
    rsx! {
        div { class: "ax-mail-attachments", role: "list", "aria-label": "Attachments, {n}",
            div { class: "ax-mail-attachment-cards",
                for a in shown {
                    AttachmentCard { key: "{a.id}", has_data: downloaded.contains(&a.id), attachment: a.clone(), on_preview: move |a| preview.set(Some(a)) }
                }
            }
            div { class: "ax-mail-attachment-actions",
                if n > 6 && !show_all() {
                    button { r#type: "button", class: "ax-mail-link", onclick: move |_| show_all.set(true), "Show all {n} attachments" }
                }
                if n >= 2 {
                    button {
                        r#type: "button",
                        class: "ax-mail-link",
                        disabled: available.is_empty(),
                        title: if available.is_empty() { "None of these attachments have been downloaded from the server yet" } else { "Save every attachment to Downloads as a .zip file" },
                        onclick: move |_| download_all(app, &available, &subject),
                        Icon { name: "arrow_download", size: 16 }
                        "Download all"
                    }
                }
            }
        }
        if let Some(a) = preview() {
            AttachmentPreview { attachment: a, on_close: move |_| preview.set(None) }
        }
    }
}

#[component]
fn AttachmentCard(attachment: Attachment, has_data: bool, on_preview: EventHandler<Attachment>) -> Element {
    let app = use_app();
    let mut menu = use_signal(|| None::<(f64, f64)>);
    let a = attachment.clone();
    let blocked = files::is_blocked_type(&a.name);
    let icon = if blocked { "shield" } else { file_icon(&a.name, &a.content_type) };
    let size = crate::fmt::file_size(a.size);
    let status = if blocked {
        "Blocked for safety".to_string()
    } else if has_data {
        size.clone()
    } else {
        format!("{size} · Not downloaded")
    };
    let name = middle_ellipsis(&a.name, 28);
    let can_preview = has_data && previewable(&a) && !blocked;
    let (a1, a2, a3) = (a.clone(), a.clone(), a.clone());
    let items = vec![
        MenuItem::new("preview", "Preview").icon("eye").disabled(!can_preview),
        MenuItem::new("download", "Download").icon("arrow_download").disabled(!has_data || blocked),
        MenuItem::new("open", "Open").icon("open").disabled(!has_data || blocked),
    ];
    rsx! {
        div {
            class: "ax-mail-attachment",
            class: if blocked { "ax-blocked" },
            class: if !has_data && !blocked { "ax-remote" },
            role: "listitem",
            button {
                r#type: "button",
                class: "ax-mail-attachment-main",
                title: if blocked { format!("{} — blocked for safety", a.name) } else if has_data { format!("{} ({size})", a.name) } else { format!("{} isn't downloaded from the server yet", a.name) },
                onclick: move |_| {
                    if blocked {
                        app.toast("This file type is blocked because it could harm your computer.");
                    } else if can_preview {
                        on_preview.call(a1.clone());
                    } else if has_data {
                        download(app, &a1);
                    } else {
                        app.toast(format!("{} hasn't been downloaded from the server yet.", a1.name));
                    }
                },
                span { class: "ax-mail-attachment-icon", Icon { name: icon, size: 32 } }
                span { class: "ax-mail-attachment-text",
                    span { class: "ax-mail-attachment-name", "{name}" }
                    span { class: "ax-mail-attachment-size", "{status}" }
                }
            }
            button {
                r#type: "button",
                class: "ax-mail-attachment-more",
                title: "More actions",
                "aria-label": "More actions for {a.name}",
                "aria-haspopup": "menu",
                onclick: move |e: MouseEvent| {
                    let c = e.client_coordinates();
                    menu.set(Some((c.x, c.y + 8.0)));
                },
                Icon { name: "chevron_down", size: 16 }
            }
            if let Some((x, y)) = menu() {
                Menu {
                    items: items.clone(),
                    x,
                    y,
                    on_select: move |id: String| match id.as_str() {
                        "preview" => on_preview.call(a2.clone()),
                        "download" => download(app, &a3),
                        "open" => open_externally(app, &a3),
                        _ => {}
                    },
                    on_close: move |_| menu.set(None),
                }
            }
        }
    }
}

/// In-app preview for images and text attachments.
#[component]
fn AttachmentPreview(attachment: Attachment, on_close: EventHandler<()>) -> Element {
    let app = use_app();
    let data = store().attachment_data(attachment.id).ok().flatten().unwrap_or_default();
    let is_image = attachment.content_type.starts_with("image/");
    let text = if is_image { String::new() } else { String::from_utf8_lossy(&data).into_owned() };
    let src = if is_image { format!("data:{};base64,{}", attachment.content_type, base64(&data)) } else { String::new() };
    let a = attachment.clone();
    rsx! {
        Dialog {
            title: attachment.name.clone(),
            width: 720,
            resize_key: "attachment-preview", min_width: 400, min_height: 240,
            on_close: move |_| on_close.call(()),
            actions: rsx! {
                button { r#type: "button", class: "ax-btn", onclick: move |_| download(app, &a), Icon { name: "arrow_download" } span { "Download" } }
                button { r#type: "button", class: "ax-btn ax-primary", onclick: move |_| on_close.call(()), "Close" }
            },
            div { class: "ax-mail-preview-body ax-surface-fill",
                if is_image {
                    img { src: "{src}", alt: "{attachment.name}" }
                } else {
                    pre { class: "ax-mail-preview-text ax-selectable", "{text}" }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons_and_names() {
        assert_eq!(file_icon("Plan.PPTX", ""), "slide_text");
        assert_eq!(file_icon("a.bin", "image/png"), "image");
        assert_eq!(file_icon("report", ""), "document");
        assert_eq!(middle_ellipsis("short.pdf", 28), "short.pdf");
        let m = middle_ellipsis("Notification service design document final.docx", 28);
        assert_eq!(m.chars().count(), 28);
        assert!(m.ends_with("l.docx"));
        assert_eq!(base64(b"hi!"), "aGkh");
    }
}
