//! Drafts, reply/forward pre-fill, draft attachments and sending (now,
//! after the "Undo send" delay, or at a scheduled time).
//!
//! A draft is a message in the Drafts folder plus an editable [`Draft`]
//! document in the `drafts` table. Files attached to a draft live in the
//! `attachments` table against the draft message, so they move with it to
//! Sent Items / Outbox; [`DraftAttachment::path`] then reads
//! `attachment:<id>`.

use std::collections::HashSet;

use chrono::{DateTime, Local, Utc};
use rusqlite::{OptionalExtension, params};

use super::{MoveRecord, Store};
use crate::html::html_to_text;
use crate::model::*;
use crate::text::preview;
use crate::{Error, Result};

pub(crate) const SCHEMA_EXTRA: &str = "
CREATE TABLE IF NOT EXISTS drafts (
    message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    json TEXT NOT NULL);
";

/// Prefix of [`DraftAttachment::path`] for attachments stored in the database.
const STORED: &str = "attachment:";

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// "Name <addr>; addr2", the way Outlook writes recipients in a quoted header.
fn addr_line(list: &[EmailAddress]) -> String {
    list.iter()
        .map(|a| if a.name.trim().is_empty() { a.address.clone() } else { format!("{} <{}>", a.name, a.address) })
        .collect::<Vec<_>>()
        .join("; ")
}

/// Subject with exactly one `RE:` / `FW:` prefix (en-US, Outlook's own).
/// Leading prefixes of the same kind collapse ("RE: Re: x" -> "RE: x");
/// other prefixes are kept ("FW: x" replied to -> "RE: FW: x").
fn with_prefix(prefix: &str, subject: &str) -> String {
    let same: &[&str] = if prefix.eq_ignore_ascii_case("RE:") { &["re:"] } else { &["fw:", "fwd:"] };
    let mut rest = subject.trim();
    loop {
        let lower = rest.to_ascii_lowercase();
        match same.iter().find(|p| lower.starts_with(**p)) {
            Some(p) => rest = rest[p.len()..].trim_start(),
            None => break,
        }
    }
    if rest.is_empty() { prefix.to_string() } else { format!("{prefix} {rest}") }
}

/// CSS font stack for a compose font name. Aptos (the new Outlook default)
/// is not installable on Linux, so it falls back to metric-compatible fonts.
pub(crate) fn font_stack(name: &str) -> String {
    let name = name.trim();
    let lower = name.to_ascii_lowercase();
    if name.is_empty() || lower == "aptos" {
        return "Aptos, Calibri, Carlito, 'Noto Sans', sans-serif".into();
    }
    if lower == "calibri" {
        return "Calibri, Carlito, sans-serif".into();
    }
    const SERIF: &[&str] = &["times new roman", "georgia", "cambria", "garamond", "palatino linotype", "book antiqua", "noto serif", "liberation serif", "dejavu serif", "constantia"];
    const MONO: &[&str] = &["courier new", "consolas", "lucida console", "dejavu sans mono", "liberation mono", "noto sans mono", "cascadia mono"];
    let generic = if SERIF.contains(&lower.as_str()) {
        "serif"
    } else if MONO.contains(&lower.as_str()) {
        "monospace"
    } else {
        "sans-serif"
    };
    if name.contains(' ') { format!("'{name}', {generic}") } else { format!("{name}, {generic}") }
}

/// Inline style every new paragraph starts with (default font and size).
fn body_style(settings: &Settings) -> String {
    format!("font-family: {}; font-size: {}pt;", font_stack(&settings.default_font), settings.default_font_size_pt.max(1))
}

/// The signature block. The data attribute lets the editor find (and swap)
/// the signature, and keeps it out of the attachment-reminder check.
pub(crate) fn signature_block(html: &str) -> String {
    format!("<div id=\"Signature\" data-axentra-signature=\"default\">{html}</div>")
}

/// Remove `<style>` / `<link>` blocks: quoted mail is edited inside the app
/// document, where a stylesheet would restyle the whole window.
fn strip_style_blocks(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len());
    let mut pos = 0;
    while let Some(rel) = lower[pos..].find("<style") {
        let start = pos + rel;
        out.push_str(&html[pos..start]);
        pos = match lower[start..].find("</style") {
            Some(end_rel) => {
                let close = start + end_rel;
                lower[close..].find('>').map(|g| close + g + 1).unwrap_or(html.len())
            }
            None => html.len(),
        };
    }
    out.push_str(&html[pos..]);
    // <link> is a void element: drop the tag itself.
    let lower = out.to_ascii_lowercase();
    let mut cleaned = String::with_capacity(out.len());
    let mut pos = 0;
    while let Some(rel) = lower[pos..].find("<link") {
        let start = pos + rel;
        cleaned.push_str(&out[pos..start]);
        pos = lower[start..].find('>').map(|g| start + g + 1).unwrap_or(out.len());
    }
    cleaned.push_str(&out[pos..]);
    cleaned
}

/// Standard base64 (RFC 4648), used for `data:` URIs of inline images.
pub(crate) fn base64(data: &[u8]) -> String {
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

/// Base64 folded into 76-character lines (MIME bodies, RFC 2045).
fn base64_lines(data: &[u8]) -> String {
    let flat = base64(data);
    let mut out = String::with_capacity(flat.len() + flat.len() / 76 * 2 + 2);
    for (i, chunk) in flat.as_bytes().chunks(76).enumerate() {
        if i > 0 {
            out.push_str("\r\n");
        }
        out.push_str(std::str::from_utf8(chunk).unwrap_or_default());
    }
    out
}

/// A header value: as is when it is plain ASCII, otherwise an RFC 2047
/// encoded word.
fn header_text(s: &str) -> String {
    let s = s.replace(['\r', '\n'], " ");
    if s.is_ascii() { s } else { format!("=?UTF-8?B?{}?=", base64(s.as_bytes())) }
}

/// `Name <addr>, addr2` for an address header.
fn header_addresses(list: &[EmailAddress]) -> String {
    list.iter()
        .map(|a| {
            let addr = a.address.trim();
            let name = a.name.trim();
            if name.is_empty() {
                addr.to_string()
            } else if name.is_ascii() {
                format!("\"{}\" <{addr}>", name.replace(['"', '\\'], ""))
            } else {
                format!("{} <{addr}>", header_text(name))
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// A parameter value (file name) safe inside quotes.
fn header_param(s: &str) -> String {
    header_text(&s.replace(['"', '\\'], "_"))
}

fn stored_id(path: &str) -> Option<AttachmentId> {
    path.strip_prefix(STORED)?.parse().ok()
}

fn same_address(a: &EmailAddress, b: &EmailAddress) -> bool {
    a.address.trim().eq_ignore_ascii_case(b.address.trim())
}

/// Most `References` entries a reply carries: the first one (the thread's
/// root) and the newest ones, like common mail clients trim long threads.
const MAX_REFERENCES: usize = 20;

/// `References` for a reply to a message (RFC 5322 3.6.4): the parent's
/// References (or, without them, its In-Reply-To), then the parent's own
/// Message-ID. Long lists keep the root and the most recent entries.
fn thread_references(parent_refs: &[String], parent_in_reply_to: Option<&str>, parent_id: &str) -> Vec<String> {
    let mut refs: Vec<String> = if parent_refs.is_empty() {
        parent_in_reply_to.filter(|s| !s.is_empty()).map(|s| vec![s.to_string()]).unwrap_or_default()
    } else {
        parent_refs.to_vec()
    };
    if !refs.iter().any(|r| r == parent_id) {
        refs.push(parent_id.to_string());
    }
    if refs.len() > MAX_REFERENCES {
        let tail = refs.split_off(refs.len() - (MAX_REFERENCES - 1));
        refs.truncate(1);
        refs.extend(tail);
    }
    refs
}

/// Keep the first occurrence of every address (case-insensitive).
fn dedup_addresses(list: Vec<EmailAddress>) -> Vec<EmailAddress> {
    let mut seen = HashSet::new();
    list.into_iter().filter(|a| seen.insert(a.address.trim().to_lowercase())).collect()
}

impl Store {
    /// A new draft, pre-filled for replies and forwards the way Outlook does:
    /// recipients, `RE:` / `FW:` subject, signature, and the original message
    /// quoted under a From/Sent/To/Cc/Subject header block. Every draft
    /// starts with an empty first line in the default compose font.
    pub fn new_draft(&self, kind: ComposeKind, account_id: Option<AccountId>, source: Option<MessageId>) -> Result<Draft> {
        let settings = self.settings();
        let source_msg = match source {
            Some(id) => Some(self.message(id)?.ok_or_else(|| Error::NotFound(format!("message {id}")))?),
            None => None,
        };
        let account_id = match (account_id, &source_msg) {
            (Some(a), _) => a,
            (None, Some(m)) => m.account_id,
            (None, None) => self.primary_account()?.map(|a| a.id).ok_or_else(|| Error::NotFound("account".into()))?,
        };
        let me = self.account(account_id)?.map(|a| a.email.to_lowercase()).unwrap_or_default();
        let not_me = |a: &&EmailAddress| a.address.trim().to_lowercase() != me;

        let mut draft = Draft { account_id, kind, source_message_id: source, ..Default::default() };
        let signature = if (kind == ComposeKind::New && settings.signature_on_new)
            || (kind != ComposeKind::New && settings.signature_on_reply)
        {
            settings.signature_html.trim().to_string()
        } else {
            String::new()
        };
        let style = body_style(&settings);
        let mut body = format!("<div style=\"{style}\"><br></div>");
        if !signature.is_empty() {
            body.push_str(&format!("<div style=\"{style}\"><br></div>{}", signature_block(&signature)));
        }

        if let Some(m) = &source_msg {
            // Replying to a message I sent goes back to its recipients.
            let from_me = m.from.address.trim().to_lowercase() == me;
            let reply_to: Vec<EmailAddress> = if from_me {
                m.to.clone()
            } else if m.reply_to.is_empty() {
                vec![m.from.clone()]
            } else {
                m.reply_to.clone()
            };
            match kind {
                ComposeKind::Reply => {
                    draft.to = dedup_addresses(reply_to);
                    draft.subject = with_prefix("RE:", &m.subject);
                }
                ComposeKind::ReplyAll => {
                    let mut to = reply_to;
                    to.extend(m.to.iter().cloned());
                    let to = dedup_addresses(to.iter().filter(not_me).cloned().collect());
                    let cc = m.cc.iter().filter(not_me).filter(|c| !to.iter().any(|t| same_address(t, c))).cloned().collect();
                    draft.to = to;
                    draft.cc = dedup_addresses(cc);
                    draft.subject = with_prefix("RE:", &m.subject);
                }
                ComposeKind::Forward => draft.subject = with_prefix("FW:", &m.subject),
                ComposeKind::New => {}
            }
            let src_body = self.message_body(m.id)?.unwrap_or_default();
            let mut original = match &src_body.html {
                Some(h) => strip_style_blocks(&crate::html::sanitize_email_html(h, true).html),
                None => format!("<div style=\"white-space:pre-wrap\">{}</div>", escape_html(&src_body.text)),
            };
            // Inline images (cid:) become data: URIs so they show while
            // editing and survive into the reply or forward.
            for a in src_body.attachments.iter().filter(|a| a.content_id.is_some()) {
                let cid = a.content_id.as_deref().unwrap_or_default().trim_matches(['<', '>']);
                let needle = format!("cid:{cid}");
                if !cid.is_empty()
                    && original.contains(&needle)
                    && let Some(data) = self.attachment_data(a.id)?
                {
                    original = original.replace(&needle, &format!("data:{};base64,{}", a.content_type, base64(&data)));
                }
            }
            let sent = m.sent_at.unwrap_or(m.received_at).with_timezone(&Local);
            let mut header = format!(
                "<b>From:</b> {}<br><b>Sent:</b> {}<br><b>To:</b> {}<br>",
                escape_html(&addr_line(std::slice::from_ref(&m.from))),
                sent.format("%A, %B %-d, %Y %-I:%M %p"),
                escape_html(&addr_line(&m.to)),
            );
            if !m.cc.is_empty() {
                header.push_str(&format!("<b>Cc:</b> {}<br>", escape_html(&addr_line(&m.cc))));
            }
            header.push_str(&format!("<b>Subject:</b> {}", escape_html(&m.subject)));
            body.push_str(&format!(
                "<hr style=\"display:inline-block;width:98%\" tabindex=\"-1\">\
                 <div id=\"divRplyFwdMsg\" dir=\"ltr\"><span style=\"font-family: {}; font-size: 11pt;\">{header}</span>\
                 <div>&nbsp;</div></div><div class=\"ax-quoted\">{original}</div>",
                font_stack(&settings.default_font),
            ));
            // Forward keeps the files; Reply drops them (inline images were
            // folded into the quoted body above).
            if kind == ComposeKind::Forward {
                for a in src_body.attachments.iter().filter(|a| !a.is_inline) {
                    draft.attachments.push(DraftAttachment {
                        name: a.name.clone(),
                        content_type: a.content_type.clone(),
                        size: a.size,
                        path: format!("{STORED}{}", a.id),
                    });
                }
            }
        }
        draft.body_html = body;
        Ok(draft)
    }

    /// Save (insert or update) a draft in the Drafts folder. Fills `draft.id`.
    /// Attachments taken from another message (forwarded) are copied onto
    /// the draft; stored attachments no longer listed are deleted. Changing
    /// the account moves the draft to that account's Drafts folder.
    pub fn save_draft(&self, draft: &mut Draft) -> Result<MessageId> {
        let account = self.account(draft.account_id)?.ok_or_else(|| Error::NotFound(format!("account {}", draft.account_id)))?;
        let drafts_folder = self.folder_for_role(draft.account_id, FolderRole::Drafts)?;
        let source = match draft.source_message_id {
            Some(id) => self.message(id)?,
            None => None,
        };
        let text = html_to_text(&draft.body_html);
        let now = Utc::now();
        let mut msg = match draft.id {
            Some(id) => self.message(id)?.unwrap_or_default(),
            None => Message::default(),
        };
        if draft.created_at.is_none() {
            // Keep the first-save time of a draft that already exists (a
            // stale in-memory copy may lack it); a draft saved before the
            // field existed gets its previous save time.
            let stored = match draft.id {
                Some(id) => self
                    .conn()
                    .query_row("SELECT json_extract(json, '$.created_at') FROM drafts WHERE message_id = ?1", [id], |r| {
                        r.get::<_, Option<String>>(0)
                    })
                    .optional()?
                    .flatten()
                    .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                    .map(|t| t.with_timezone(&Utc)),
                None => None,
            };
            draft.created_at = Some(stored.unwrap_or(if msg.id != 0 { msg.received_at } else { now }));
        }
        let account_changed = msg.id != 0 && msg.account_id != draft.account_id;
        msg.account_id = draft.account_id;
        if msg.folder_id == 0 || account_changed {
            msg.folder_id = drafts_folder;
        }
        msg.from = EmailAddress::new(account.display_name.clone(), account.email.clone());
        msg.to = draft.to.clone();
        msg.cc = draft.cc.clone();
        msg.bcc = draft.bcc.clone();
        msg.subject = draft.subject.clone();
        msg.preview = preview(&text, 200);
        msg.received_at = now;
        msg.is_read = true;
        msg.is_draft = true;
        msg.importance = draft.importance;
        msg.has_attachments = !draft.attachments.is_empty();
        msg.scheduled_send_at = draft.scheduled_send_at;
        if msg.conversation_id.is_empty() {
            msg.conversation_id = source
                .as_ref()
                .filter(|_| draft.kind != ComposeKind::Forward)
                .map(|s| s.conversation_id.clone())
                .unwrap_or_else(|| format!("draft-{}", uuid::Uuid::new_v4()));
        }
        msg.in_reply_to = source
            .as_ref()
            .filter(|_| draft.kind != ComposeKind::Forward)
            .map(|s| s.internet_message_id.clone())
            .filter(|s| !s.is_empty());
        msg.references = match (&source, &msg.in_reply_to) {
            (Some(s), Some(parent)) => thread_references(&s.references, s.in_reply_to.as_deref(), parent),
            _ => Vec::new(),
        };
        let id = if msg.id == 0 {
            self.insert_message(&mut msg, None)?
        } else {
            self.update_message(&msg)?;
            msg.id
        };
        draft.id = Some(id);
        self.sync_draft_attachments(id, draft)?;
        self.set_message_body(id, Some(&draft.body_html), &text)?;
        self.conn().execute(
            "INSERT INTO drafts (message_id, json) VALUES (?1, ?2) ON CONFLICT(message_id) DO UPDATE SET json = excluded.json",
            params![id, serde_json::to_string(draft)?],
        )?;
        Ok(id)
    }

    /// Make the attachments table for draft message `id` match `draft.attachments`.
    fn sync_draft_attachments(&self, id: MessageId, draft: &mut Draft) -> Result<()> {
        let existing: Vec<Attachment> = self.query_docs("WHERE message_id = ?1", [id])?;
        let mut keep = HashSet::new();
        for a in &mut draft.attachments {
            let Some(aid) = stored_id(&a.path) else { continue };
            if existing.iter().any(|e| e.id == aid) {
                keep.insert(aid);
                continue;
            }
            // Listed but owned by another message (a forwarded file): copy it.
            let Some(src) = self.get_doc::<Attachment>(aid)? else { continue };
            let data = self.attachment_data(aid)?;
            let mut copy = Attachment { id: 0, message_id: id, content_id: None, is_inline: false, ..src };
            let new_id = self.insert_attachment(&mut copy, data.as_deref())?;
            a.path = format!("{STORED}{new_id}");
            keep.insert(new_id);
        }
        for e in existing.iter().filter(|e| !keep.contains(&e.id) && !e.is_inline) {
            self.delete_doc::<Attachment>(e.id)?;
        }
        Ok(())
    }

    /// The editable draft behind a message in Drafts (or Outbox, while an
    /// "Undo send" delay runs).
    pub fn draft(&self, message_id: MessageId) -> Result<Option<Draft>> {
        let json: Option<String> = self
            .conn()
            .query_row("SELECT json FROM drafts WHERE message_id = ?1", [message_id], |r| r.get(0))
            .optional()?;
        if let Some(j) = json {
            let mut d: Draft = serde_json::from_str(&j)?;
            d.id = Some(message_id);
            return Ok(Some(d));
        }
        // A draft that arrived from the server: rebuild it from the message.
        let Some(m) = self.message(message_id)? else { return Ok(None) };
        if !m.is_draft {
            return Ok(None);
        }
        let body = self.message_body(message_id)?.unwrap_or_default();
        let body_html = match body.html {
            // It is edited inside the app document: sanitise like any mail.
            Some(h) => strip_style_blocks(&crate::html::sanitize_email_html(&h, true).html),
            None => format!("<div style=\"white-space:pre-wrap\">{}</div>", escape_html(&body.text)),
        };
        let attachments = body
            .attachments
            .iter()
            .filter(|a| !a.is_inline)
            .map(|a| DraftAttachment {
                name: a.name.clone(),
                content_type: a.content_type.clone(),
                size: a.size,
                path: format!("{STORED}{}", a.id),
            })
            .collect();
        Ok(Some(Draft {
            id: Some(m.id),
            account_id: m.account_id,
            to: m.to,
            cc: m.cc,
            bcc: m.bcc,
            subject: m.subject,
            body_html,
            importance: m.importance,
            attachments,
            scheduled_send_at: m.scheduled_send_at,
            ..Default::default()
        }))
    }

    /// Discard a draft: like Outlook's Discard it goes to Deleted Items, so
    /// it stays recoverable (or is removed for good when already there).
    pub fn discard_draft(&self, message_id: MessageId) -> Result<()> {
        self.discard_draft_undoable(message_id).map(|_| ())
    }

    /// [`Store::discard_draft`], returning the move to undo it.
    pub fn discard_draft_undoable(&self, message_id: MessageId) -> Result<Vec<MoveRecord>> {
        if let Some(mut d) = self.draft(message_id)?
            && (d.scheduled_send_at.is_some() || d.send_pending_until.is_some())
        {
            d.scheduled_send_at = None;
            d.send_pending_until = None;
            self.save_draft(&mut d)?;
        }
        self.delete_messages(&[message_id])
    }

    /// Send a draft. Local accounts deliver straight to Sent Items; real
    /// accounts queue in Outbox, where `axentra-sync` picks it up.
    pub fn send_draft(&self, draft: &mut Draft) -> Result<MessageId> {
        if draft.to.is_empty() && draft.cc.is_empty() && draft.bcc.is_empty() {
            return Err(Error::Invalid("add at least one recipient".into()));
        }
        draft.scheduled_send_at = None;
        draft.send_pending_until = None;
        let id = self.save_draft(draft)?;
        let account = self.account(draft.account_id)?.ok_or_else(|| Error::NotFound("account".into()))?;
        let mut msg = self.message(id)?.ok_or_else(|| Error::NotFound(format!("message {id}")))?;
        let now = Utc::now();
        msg.is_draft = false;
        msg.sent_at = Some(now);
        msg.received_at = now;
        msg.scheduled_send_at = None;
        msg.internet_message_id = format!("{}@axentra.local", uuid::Uuid::new_v4());
        let target = if account.kind == AccountKind::Local { FolderRole::Sent } else { FolderRole::Outbox };
        msg.folder_id = self.folder_for_role(draft.account_id, target)?;
        self.update_message(&msg)?;
        if account.kind == AccountKind::Local {
            self.conn().execute("DELETE FROM drafts WHERE message_id = ?1", [id])?;
        }
        let mut all = draft.to.clone();
        all.extend(draft.cc.iter().cloned());
        all.extend(draft.bcc.iter().cloned());
        self.record_recent_addresses(&all)?;
        Ok(id)
    }

    // ------------------------------------------------------------ attachments

    /// Attach a file to a draft. The draft is saved first when it is new; the
    /// bytes are stored against the draft message.
    pub fn add_draft_attachment(&self, draft: &mut Draft, name: &str, content_type: &str, data: &[u8]) -> Result<DraftAttachment> {
        let exists = match draft.id {
            Some(id) => self.message(id)?.is_some(),
            None => false,
        };
        let message_id = if exists { draft.id.unwrap_or_default() } else { self.save_draft(draft)? };
        let content_type = if content_type.trim().is_empty() { "application/octet-stream" } else { content_type };
        let mut a = Attachment {
            message_id,
            name: name.to_string(),
            content_type: content_type.to_string(),
            size: data.len() as i64,
            ..Default::default()
        };
        let aid = self.insert_attachment(&mut a, Some(data))?;
        let da = DraftAttachment { name: a.name, content_type: a.content_type, size: a.size, path: format!("{STORED}{aid}") };
        draft.attachments.push(da.clone());
        self.save_draft(draft)?;
        Ok(da)
    }

    /// Bytes of a draft attachment (stored, forwarded or a local file).
    pub fn draft_attachment_data(&self, attachment: &DraftAttachment) -> Result<Option<Vec<u8>>> {
        match stored_id(&attachment.path) {
            Some(id) => self.attachment_data(id),
            None if attachment.path.is_empty() => Ok(None),
            None => Ok(std::fs::read(&attachment.path).ok()),
        }
    }

    /// "Recent files" under Attach file: files the user attached to mail
    /// they wrote (drafts, Outbox, Sent Items), newest first, one entry per
    /// name and size, leaving out the files already on draft `exclude`.
    /// Attaching one copies it onto the draft ([`Store::save_draft`]).
    pub fn recent_attachments(&self, exclude: Option<MessageId>, limit: usize) -> Result<Vec<DraftAttachment>> {
        let rows: Vec<(AttachmentId, String)> = {
            let conn = self.conn();
            let mut stmt = conn.prepare_cached(
                "SELECT a.id, a.json FROM attachments a
                 JOIN messages m ON m.id = a.message_id JOIN folders f ON f.id = m.folder_id
                 WHERE a.data IS NOT NULL AND (m.is_draft = 1 OR f.role IN ('sent', 'outbox'))
                 ORDER BY m.received_at DESC, a.id DESC LIMIT 200",
            )?;
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<std::result::Result<_, _>>()?
        };
        let mut on_draft = HashSet::new();
        if let Some(id) = exclude {
            for a in self.query_docs::<Attachment>("WHERE message_id = ?1", [id])? {
                on_draft.insert((a.name.to_lowercase(), a.size));
            }
        }
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for (id, json) in rows {
            let a: Attachment = serde_json::from_str(&json)?;
            let key = (a.name.to_lowercase(), a.size);
            if a.is_inline || on_draft.contains(&key) || !seen.insert(key) {
                continue;
            }
            out.push(DraftAttachment { name: a.name, content_type: a.content_type, size: a.size, path: format!("{STORED}{id}") });
            if out.len() >= limit {
                break;
            }
        }
        Ok(out)
    }

    // ------------------------------------------------------------ forward as attachment

    /// A stored message as an RFC 5322 / MIME document (`message/rfc822`),
    /// rebuilt from what the store keeps: headers, the HTML or text body,
    /// and the attachments whose bytes are present.
    pub fn rfc822_for_forward(&self, message_id: MessageId) -> Result<Vec<u8>> {
        let m = self.message(message_id)?.ok_or_else(|| Error::NotFound(format!("message {message_id}")))?;
        let body = self.message_body(message_id)?.unwrap_or_default();
        let boundary = format!("axentra-{}", uuid::Uuid::new_v4().simple());
        let mut out = String::new();
        let mut header = |name: &str, value: String| {
            if !value.trim().is_empty() {
                out.push_str(&format!("{name}: {value}\r\n"));
            }
        };
        header("From", header_addresses(std::slice::from_ref(&m.from)));
        header("To", header_addresses(&m.to));
        header("Cc", header_addresses(&m.cc));
        header("Subject", header_text(&m.subject));
        header("Date", m.sent_at.unwrap_or(m.received_at).to_rfc2822());
        if !m.internet_message_id.is_empty() {
            header("Message-ID", format!("<{}>", m.internet_message_id.trim_matches(['<', '>'])));
        }
        header("MIME-Version", "1.0".into());
        let (body_type, body_bytes) = match &body.html {
            Some(h) => ("text/html", h.as_bytes()),
            None => ("text/plain", body.text.as_bytes()),
        };
        let files: Vec<(Attachment, Vec<u8>)> = body
            .attachments
            .iter()
            .filter_map(|a| self.attachment_data(a.id).ok().flatten().map(|d| (a.clone(), d)))
            .collect();
        if files.is_empty() {
            out.push_str(&format!("Content-Type: {body_type}; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n"));
            out.push_str(&base64_lines(body_bytes));
            out.push_str("\r\n");
            return Ok(out.into_bytes());
        }
        out.push_str(&format!("Content-Type: multipart/mixed; boundary=\"{boundary}\"\r\n\r\n"));
        out.push_str(&format!("--{boundary}\r\nContent-Type: {body_type}; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n"));
        out.push_str(&base64_lines(body_bytes));
        out.push_str("\r\n");
        for (a, data) in files {
            let name = header_param(&a.name);
            let disposition = if a.is_inline { "inline" } else { "attachment" };
            out.push_str(&format!("--{boundary}\r\nContent-Type: {}; name=\"{name}\"\r\n", a.content_type));
            out.push_str(&format!("Content-Disposition: {disposition}; filename=\"{name}\"\r\n"));
            if let Some(cid) = &a.content_id {
                out.push_str(&format!("Content-ID: <{}>\r\n", cid.trim_matches(['<', '>'])));
            }
            out.push_str("Content-Transfer-Encoding: base64\r\n\r\n");
            out.push_str(&base64_lines(&data));
            out.push_str("\r\n");
        }
        out.push_str(&format!("--{boundary}--\r\n"));
        Ok(out.into_bytes())
    }

    /// "Forward as attachment": a saved Forward draft (`FW:` subject, the
    /// signature, nothing quoted) with the original attached as an `.eml`
    /// file (`message/rfc822`, mail spec 6.10).
    pub fn new_forward_as_attachment(&self, source: MessageId) -> Result<Draft> {
        let mut draft = self.new_draft(ComposeKind::Forward, None, Some(source))?;
        let settings = self.settings();
        let style = body_style(&settings);
        let mut body = format!("<div style=\"{style}\"><br></div>");
        if settings.signature_on_reply && !settings.signature_html.trim().is_empty() {
            body.push_str(&format!("<div style=\"{style}\"><br></div>{}", signature_block(settings.signature_html.trim())));
        }
        draft.body_html = body;
        draft.attachments.clear();
        let m = self.message(source)?.ok_or_else(|| Error::NotFound(format!("message {source}")))?;
        let cleaned: String = m.subject.chars().filter(|c| !"/\\:*?\"<>|".contains(*c)).collect();
        let stem: String = cleaned.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(80).collect();
        let name = format!("{}.eml", if stem.is_empty() { "Message" } else { &stem });
        let eml = self.rfc822_for_forward(source)?;
        self.add_draft_attachment(&mut draft, &name, "message/rfc822", &eml)?;
        Ok(draft)
    }

    // ------------------------------------------------------------ delayed sending

    /// Schedule a draft to go out at `at`. Like Outlook it stays in Drafts
    /// until then; [`Store::send_due_scheduled`] sends it.
    pub fn schedule_send(&self, draft: &mut Draft, at: DateTime<Utc>) -> Result<MessageId> {
        if draft.to.is_empty() && draft.cc.is_empty() && draft.bcc.is_empty() {
            return Err(Error::Invalid("add at least one recipient".into()));
        }
        draft.scheduled_send_at = Some(at);
        draft.send_pending_until = None;
        self.save_draft(draft)
    }

    /// "Cancel send": turn a scheduled draft back into a normal draft.
    pub fn cancel_scheduled_send(&self, draft: &mut Draft) -> Result<()> {
        draft.scheduled_send_at = None;
        if draft.id.is_some() {
            self.save_draft(draft)?;
        }
        Ok(())
    }

    /// Scheduled drafts (still in a Drafts folder) whose time has come.
    pub fn due_scheduled_drafts(&self, now: DateTime<Utc>) -> Result<Vec<MessageId>> {
        let msgs: Vec<Message> = self.query_docs(
            "WHERE is_draft = 1 AND json_extract(json, '$.scheduled_send_at') IS NOT NULL
             AND folder_id IN (SELECT id FROM folders WHERE role = 'drafts')",
            [],
        )?;
        Ok(msgs.into_iter().filter(|m| m.scheduled_send_at.is_some_and(|t| t <= now)).map(|m| m.id).collect())
    }

    /// Send every scheduled draft that is due. Returns the sent message ids.
    pub fn send_due_scheduled(&self, now: DateTime<Utc>) -> Result<Vec<MessageId>> {
        let mut sent = Vec::new();
        for id in self.due_scheduled_drafts(now)? {
            if let Some(mut d) = self.draft(id)? {
                sent.push(self.send_draft(&mut d)?);
            }
        }
        Ok(sent)
    }

    /// Hold a draft for the "Undo send" delay: saved, moved to Outbox and
    /// stamped with the time it may go out. Undo is the returned move.
    pub fn queue_send(&self, draft: &mut Draft, send_at: DateTime<Utc>) -> Result<MoveRecord> {
        if draft.to.is_empty() && draft.cc.is_empty() && draft.bcc.is_empty() {
            return Err(Error::Invalid("add at least one recipient".into()));
        }
        draft.scheduled_send_at = None;
        draft.send_pending_until = Some(send_at);
        let id = self.save_draft(draft)?;
        let from_folder = self.message(id)?.map(|m| m.folder_id).unwrap_or_default();
        let outbox = self.folder_for_role(draft.account_id, FolderRole::Outbox)?;
        let record = self.move_messages(&[id], outbox)?.into_iter().next();
        Ok(record.unwrap_or(MoveRecord { message_id: id, from_folder }))
    }

    /// End the "Undo send" delay: send the held message if it is still
    /// waiting in Outbox. Returns `false` when it was taken back (undone).
    pub fn release_queued_send(&self, message_id: MessageId) -> Result<bool> {
        if !self.is_queued_send(message_id)? {
            return Ok(false);
        }
        let Some(mut d) = self.draft(message_id)? else { return Ok(false) };
        self.send_draft(&mut d)?;
        Ok(true)
    }

    /// After Undo moved a held message back: clear its send time and return
    /// the draft to reopen.
    pub fn unqueue_send(&self, message_id: MessageId) -> Result<Option<Draft>> {
        let Some(mut d) = self.draft(message_id)? else { return Ok(None) };
        if d.send_pending_until.take().is_some() {
            if self.is_queued_send(message_id)? {
                let drafts = self.folder_for_role(d.account_id, FolderRole::Drafts)?;
                self.move_messages(&[message_id], drafts)?;
            }
            self.save_draft(&mut d)?;
        }
        Ok(Some(d))
    }

    /// Held messages whose delay is over (e.g. the app quit during it).
    pub fn overdue_queued_sends(&self, now: DateTime<Utc>) -> Result<Vec<MessageId>> {
        let rows: Vec<(MessageId, String)> = {
            let conn = self.conn();
            let mut stmt = conn.prepare_cached(
                "SELECT d.message_id, d.json FROM drafts d JOIN messages m ON m.id = d.message_id
                 JOIN folders f ON f.id = m.folder_id
                 WHERE m.is_draft = 1 AND f.role = 'outbox'",
            )?;
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<std::result::Result<_, _>>()?
        };
        let mut out = Vec::new();
        for (id, json) in rows {
            let d: Draft = serde_json::from_str(&json)?;
            if d.send_pending_until.is_some_and(|t| t <= now) {
                out.push(id);
            }
        }
        Ok(out)
    }

    fn is_queued_send(&self, message_id: MessageId) -> Result<bool> {
        let Some(m) = self.message(message_id)? else { return Ok(false) };
        if !m.is_draft {
            return Ok(false);
        }
        Ok(self.folder_for_role(m.account_id, FolderRole::Outbox)? == m.folder_id)
    }

    // ------------------------------------------------------------ recipients

    /// Recipient autocomplete: [`Store::suggest_addresses`] plus whether each
    /// entry comes from the recent-recipients cache (those can be removed).
    pub fn recipient_suggestions(&self, query: &str, limit: usize) -> Result<Vec<(EmailAddress, bool)>> {
        let found = self.suggest_addresses(query, limit)?;
        let conn = self.conn();
        let mut stmt = conn.prepare_cached("SELECT 1 FROM recent_addresses WHERE address = lower(?1)")?;
        let mut out = Vec::with_capacity(found.len());
        for a in found {
            let recent = stmt.exists([a.address.trim()])?;
            out.push((a, recent));
        }
        Ok(out)
    }

    /// "Remove from suggestions": forget a recent recipient.
    pub fn forget_recent_address(&self, address: &str) -> Result<()> {
        self.conn().execute("DELETE FROM recent_addresses WHERE address = lower(?1)", [address.trim()])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_util::store_with_account;

    fn inbox_message(store: &Store, acct: AccountId, body: MessageBody) -> (MessageId, Message) {
        let inbox = store.folder_for_role(acct, FolderRole::Inbox).unwrap();
        let mut m = Message {
            account_id: acct,
            folder_id: inbox,
            internet_message_id: "orig-1@contoso.example".into(),
            from: EmailAddress::new("Megan Bowen", "megan@contoso.example"),
            to: vec![EmailAddress::new("Test User", "test@axentra.example"), EmailAddress::new("Alex", "alex@contoso.example")],
            cc: vec![EmailAddress::new("Lee", "lee@contoso.example"), EmailAddress::new("Alex again", "ALEX@contoso.example")],
            subject: "Budget".into(),
            received_at: Utc::now(),
            ..Default::default()
        };
        let id = store.insert_message(&mut m, Some(&body)).unwrap();
        (id, m)
    }

    #[test]
    fn reply_all_prefills_and_send_moves_to_sent() {
        let (store, acct) = store_with_account();
        let (src, m) = inbox_message(&store, acct, MessageBody { text: "Numbers attached".into(), ..Default::default() });
        let mut d = store.new_draft(ComposeKind::ReplyAll, None, Some(src)).unwrap();
        assert_eq!(d.subject, "RE: Budget");
        let to: Vec<_> = d.to.iter().map(|a| a.address.as_str()).collect();
        assert_eq!(to, vec!["megan@contoso.example", "alex@contoso.example"]);
        // Lee stays on Cc; the second Alex is already on To.
        assert_eq!(d.cc.iter().map(|a| a.address.as_str()).collect::<Vec<_>>(), vec!["lee@contoso.example"]);
        assert!(d.body_html.contains("Numbers attached"));
        assert!(d.body_html.contains("<b>Subject:</b> Budget"));
        assert!(d.body_html.contains("<b>Cc:</b> Lee &lt;lee@contoso.example&gt;"));
        assert!(d.body_html.contains("id=\"divRplyFwdMsg\""));

        let id = store.save_draft(&mut d).unwrap();
        let drafts = store.folder_for_role(acct, FolderRole::Drafts).unwrap();
        let saved = store.message(id).unwrap().unwrap();
        assert_eq!(saved.folder_id, drafts);
        assert_eq!(saved.in_reply_to.as_deref(), Some("orig-1@contoso.example"));
        assert_eq!(store.draft(id).unwrap().unwrap().subject, "RE: Budget");

        store.send_draft(&mut d).unwrap();
        let sent = store.folder_for_role(acct, FolderRole::Sent).unwrap();
        let sent_msg = store.message(id).unwrap().unwrap();
        assert_eq!(sent_msg.folder_id, sent);
        assert!(!sent_msg.is_draft);
        assert_eq!(sent_msg.conversation_id, m.conversation_id);
        assert!(store.suggest_addresses("lee", 3).unwrap().iter().any(|a| a.address == "lee@contoso.example"));
    }

    #[test]
    fn replies_carry_thread_references() {
        let (store, acct) = store_with_account();
        let (src, _) = inbox_message(&store, acct, MessageBody { text: "x".into(), ..Default::default() });
        let mut m = store.message(src).unwrap().unwrap();
        m.in_reply_to = Some("root@contoso.example".into());
        store.update_message(&m).unwrap();
        let mut d = store.new_draft(ComposeKind::Reply, None, Some(src)).unwrap();
        let id = store.save_draft(&mut d).unwrap();
        let saved = store.message(id).unwrap().unwrap();
        assert_eq!(saved.references, vec!["root@contoso.example".to_string(), "orig-1@contoso.example".to_string()]);
        // A forward starts a new thread.
        let mut f = store.new_draft(ComposeKind::Forward, None, Some(src)).unwrap();
        let fid = store.save_draft(&mut f).unwrap();
        let fwd = store.message(fid).unwrap().unwrap();
        assert!(fwd.references.is_empty() && fwd.in_reply_to.is_none());
    }

    #[test]
    fn forward_as_attachment_attaches_a_parsable_eml() {
        use mail_parser::{MessageParser, MimeHeaders};
        let (store, acct) = store_with_account();
        let body = MessageBody {
            html: Some("<p>Numbers for Zoë</p>".into()),
            text: "Numbers".into(),
            attachments: vec![Attachment { name: "Plan.pdf".into(), content_type: "application/pdf".into(), size: 4, ..Default::default() }],
            ..Default::default()
        };
        let (src, _) = inbox_message(&store, acct, body);
        let mut m = store.message(src).unwrap().unwrap();
        m.subject = "Budget: Q4 / Zoë".into();
        store.update_message(&m).unwrap();
        let att = store.message_body(src).unwrap().unwrap().attachments[0].id;
        store.conn().execute("UPDATE attachments SET data = ?1 WHERE id = ?2", params![b"%PDF".to_vec(), att]).unwrap();

        let raw = store.rfc822_for_forward(src).unwrap();
        let parsed = MessageParser::default().parse(&raw).expect("parses");
        assert_eq!(parsed.subject(), Some("Budget: Q4 / Zoë"));
        assert_eq!(parsed.from().and_then(|a| a.first()).and_then(|a| a.address()), Some("megan@contoso.example"));
        assert_eq!(parsed.message_id(), Some("orig-1@contoso.example"));
        assert!(parsed.body_html(0).unwrap().contains("Numbers for Zoë"));
        assert_eq!(parsed.attachment_count(), 1);
        let part = parsed.attachment(0).unwrap();
        assert_eq!(part.attachment_name(), Some("Plan.pdf"));
        assert_eq!(part.contents(), b"%PDF");

        let d = store.new_forward_as_attachment(src).unwrap();
        assert_eq!(d.kind, ComposeKind::Forward);
        assert_eq!(d.subject, "FW: Budget: Q4 / Zoë");
        assert!(!d.body_html.contains("divRplyFwdMsg"), "nothing quoted");
        assert_eq!(d.attachments.len(), 1);
        assert_eq!(d.attachments[0].name, "Budget Q4 Zoë.eml");
        assert_eq!(d.attachments[0].content_type, "message/rfc822");
        let saved = store.draft_attachment_data(&d.attachments[0]).unwrap().unwrap();
        let again = MessageParser::default().parse(&saved).expect("stored .eml parses");
        assert_eq!(again.subject(), Some("Budget: Q4 / Zoë"));
        assert!(store.message(d.id.unwrap()).unwrap().unwrap().has_attachments);
    }

    #[test]
    fn long_reference_lists_keep_root_and_newest() {
        let refs: Vec<String> = (0..30).map(|i| format!("m{i}@x")).collect();
        let out = thread_references(&refs, None, "parent@x");
        assert_eq!(out.len(), MAX_REFERENCES);
        assert_eq!(out[0], "m0@x");
        assert_eq!(out[MAX_REFERENCES - 1], "parent@x");
        assert_eq!(out[MAX_REFERENCES - 2], "m29@x");
        // No duplicate when the parent is already listed.
        assert_eq!(thread_references(&["a@x".into(), "b@x".into()], None, "b@x"), vec!["a@x".to_string(), "b@x".to_string()]);
        assert_eq!(thread_references(&[], Some(""), "p@x"), vec!["p@x".to_string()]);
    }

    #[test]
    fn reply_to_header_and_own_message() {
        let (store, acct) = store_with_account();
        let inbox = store.folder_for_role(acct, FolderRole::Inbox).unwrap();
        let mut list = Message {
            account_id: acct,
            folder_id: inbox,
            from: EmailAddress::new("Bot", "noreply@list.example"),
            reply_to: vec![EmailAddress::new("List", "list@list.example")],
            to: vec![EmailAddress::new("Test User", "test@axentra.example")],
            subject: "Re: RE: Weekly".into(),
            received_at: Utc::now(),
            ..Default::default()
        };
        let a = store.insert_message(&mut list, None).unwrap();
        let d = store.new_draft(ComposeKind::Reply, None, Some(a)).unwrap();
        assert_eq!(d.to, vec![EmailAddress::new("List", "list@list.example")]);
        assert_eq!(d.subject, "RE: Weekly");

        let sent = store.folder_for_role(acct, FolderRole::Sent).unwrap();
        let mut mine = Message {
            account_id: acct,
            folder_id: sent,
            from: EmailAddress::new("Test User", "test@axentra.example"),
            to: vec![EmailAddress::new("Priya", "priya@contoso.example")],
            subject: "Plan".into(),
            received_at: Utc::now(),
            ..Default::default()
        };
        let b = store.insert_message(&mut mine, None).unwrap();
        let d = store.new_draft(ComposeKind::Reply, None, Some(b)).unwrap();
        assert_eq!(d.to.iter().map(|a| a.address.as_str()).collect::<Vec<_>>(), vec!["priya@contoso.example"]);
    }

    #[test]
    fn subject_prefix_not_doubled() {
        assert_eq!(with_prefix("RE:", "RE: Budget"), "RE: Budget");
        assert_eq!(with_prefix("RE:", "RE: re:Budget"), "RE: Budget");
        assert_eq!(with_prefix("FW:", "RE: Budget"), "FW: RE: Budget");
        assert_eq!(with_prefix("FW:", "Fwd: Budget"), "FW: Budget");
        assert_eq!(with_prefix("RE:", "FW: Budget"), "RE: FW: Budget");
        assert_eq!(with_prefix("RE:", ""), "RE:");
    }

    #[test]
    fn send_requires_recipient() {
        let (store, acct) = store_with_account();
        let mut d = store.new_draft(ComposeKind::New, Some(acct), None).unwrap();
        assert!(store.send_draft(&mut d).is_err());
        assert!(store.queue_send(&mut d, Utc::now()).is_err());
        assert!(store.schedule_send(&mut d, Utc::now()).is_err());
    }

    #[test]
    fn new_draft_uses_default_font_and_signature() {
        let (store, acct) = store_with_account();
        let mut s = store.settings();
        s.signature_html = "<p>Cheers, Test</p>".into();
        s.signature_on_reply = false;
        s.default_font = "Georgia".into();
        s.default_font_size_pt = 14;
        store.save_settings(&s).unwrap();
        let d = store.new_draft(ComposeKind::New, Some(acct), None).unwrap();
        assert!(d.body_html.starts_with("<div style=\"font-family: Georgia, serif; font-size: 14pt;\"><br></div>"));
        assert!(d.body_html.contains("data-axentra-signature=\"default\"><p>Cheers, Test</p></div>"));
        let (src, _) = inbox_message(&store, acct, MessageBody { text: "hi".into(), ..Default::default() });
        let r = store.new_draft(ComposeKind::Reply, None, Some(src)).unwrap();
        assert!(!r.body_html.contains("data-axentra-signature"));
        assert_eq!(font_stack("Aptos"), "Aptos, Calibri, Carlito, 'Noto Sans', sans-serif");
        assert_eq!(font_stack("Courier New"), "'Courier New', monospace");
    }

    #[test]
    fn quoted_body_drops_styles_and_inlines_cid_images() {
        let (store, acct) = store_with_account();
        let body = MessageBody {
            html: Some("<style>body{color:red}</style><p>See <img src=\"cid:logo1\"></p><link rel=stylesheet href=x.css>".into()),
            text: "See".into(),
            attachments: vec![
                Attachment { name: "logo.png".into(), content_type: "image/png".into(), size: 3, content_id: Some("<logo1>".into()), is_inline: true, ..Default::default() },
                Attachment { name: "Plan.pdf".into(), content_type: "application/pdf".into(), size: 3, ..Default::default() },
            ],
            ..Default::default()
        };
        let (src, _) = inbox_message(&store, acct, body);
        let atts = store.message_body(src).unwrap().unwrap().attachments;
        store.conn().execute("UPDATE attachments SET data = ?1 WHERE id = ?2", params![b"abc".to_vec(), atts[0].id]).unwrap();
        let reply = store.new_draft(ComposeKind::Reply, None, Some(src)).unwrap();
        assert!(!reply.body_html.contains("<style"));
        assert!(!reply.body_html.contains("color:red"));
        assert!(!reply.body_html.to_lowercase().contains("<link"));
        assert!(reply.body_html.contains("src=\"data:image/png;base64,YWJj\""));
        assert!(reply.attachments.is_empty());
        let fwd = store.new_draft(ComposeKind::Forward, None, Some(src)).unwrap();
        assert_eq!(fwd.subject, "FW: Budget");
        assert!(fwd.to.is_empty());
        assert_eq!(fwd.attachments.len(), 1);
        assert_eq!(fwd.attachments[0].name, "Plan.pdf");
    }

    #[test]
    fn forwarded_and_added_attachments_are_stored_on_the_draft() {
        let (store, acct) = store_with_account();
        let body = MessageBody {
            text: "x".into(),
            attachments: vec![Attachment { name: "Plan.pdf".into(), content_type: "application/pdf".into(), size: 4, ..Default::default() }],
            ..Default::default()
        };
        let (src, _) = inbox_message(&store, acct, body);
        let src_att = store.message_body(src).unwrap().unwrap().attachments[0].id;
        store.conn().execute("UPDATE attachments SET data = ?1 WHERE id = ?2", params![b"%PDF".to_vec(), src_att]).unwrap();

        let mut d = store.new_draft(ComposeKind::Forward, None, Some(src)).unwrap();
        let id = store.save_draft(&mut d).unwrap();
        let copied = stored_id(&d.attachments[0].path).unwrap();
        assert_ne!(copied, src_att, "forwarded file is copied onto the draft");
        assert_eq!(store.draft_attachment_data(&d.attachments[0]).unwrap().unwrap(), b"%PDF");

        let added = store.add_draft_attachment(&mut d, "notes.txt", "", b"hello").unwrap();
        assert_eq!(added.content_type, "application/octet-stream");
        assert_eq!(added.size, 5);
        let stored = store.message_body(id).unwrap().unwrap().attachments;
        assert_eq!(stored.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), vec!["Plan.pdf", "notes.txt"]);
        assert!(store.message(id).unwrap().unwrap().has_attachments);
        // Reloading keeps both; removing one deletes its row.
        let mut again = store.draft(id).unwrap().unwrap();
        assert_eq!(again.attachments.len(), 2);
        again.attachments.remove(0);
        store.save_draft(&mut again).unwrap();
        let stored = store.message_body(id).unwrap().unwrap().attachments;
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].name, "notes.txt");
        // The source message still has its own file.
        assert_eq!(store.attachment_data(src_att).unwrap().unwrap(), b"%PDF");
    }

    #[test]
    fn new_draft_attachment_saves_the_draft_first() {
        let (store, acct) = store_with_account();
        let mut d = store.new_draft(ComposeKind::New, Some(acct), None).unwrap();
        assert!(d.id.is_none());
        store.add_draft_attachment(&mut d, "a.png", "image/png", &[1, 2, 3]).unwrap();
        let id = d.id.unwrap();
        assert_eq!(store.message_body(id).unwrap().unwrap().attachments.len(), 1);
        // Sending keeps the file with the sent message.
        d.to.push(EmailAddress::new("", "x@contoso.example"));
        store.send_draft(&mut d).unwrap();
        assert_eq!(store.message_body(id).unwrap().unwrap().attachments.len(), 1);
    }

    #[test]
    fn recent_attachments_offer_files_the_user_attached() {
        let (store, acct) = store_with_account();
        // A file someone sent me is not a "recent file".
        let body = MessageBody {
            text: "x".into(),
            attachments: vec![Attachment { name: "Theirs.pdf".into(), content_type: "application/pdf".into(), size: 2, ..Default::default() }],
            ..Default::default()
        };
        let (src, _) = inbox_message(&store, acct, body);
        let theirs = store.message_body(src).unwrap().unwrap().attachments[0].id;
        store.conn().execute("UPDATE attachments SET data = ?1 WHERE id = ?2", params![b"ab".to_vec(), theirs]).unwrap();
        assert!(store.recent_attachments(None, 10).unwrap().is_empty());

        // Sent mail and drafts are; the same file twice is listed once.
        let mut sent = store.new_draft(ComposeKind::New, Some(acct), None).unwrap();
        sent.to.push(EmailAddress::new("", "x@contoso.example"));
        store.add_draft_attachment(&mut sent, "Plan.pdf", "application/pdf", b"plan").unwrap();
        store.send_draft(&mut sent).unwrap();
        let mut other = store.new_draft(ComposeKind::New, Some(acct), None).unwrap();
        store.add_draft_attachment(&mut other, "plan.PDF", "application/pdf", b"plan").unwrap();
        store.add_draft_attachment(&mut other, "Notes.txt", "text/plain", b"n").unwrap();
        let recent = store.recent_attachments(None, 10).unwrap();
        let names: Vec<&str> = recent.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, vec!["Notes.txt", "plan.PDF"]);
        assert_eq!(store.recent_attachments(None, 1).unwrap().len(), 1);

        // Files already on the draft are left out; picking one copies it.
        let mut d = store.new_draft(ComposeKind::New, Some(acct), None).unwrap();
        let id = store.save_draft(&mut d).unwrap();
        d.attachments.push(recent[0].clone());
        store.save_draft(&mut d).unwrap();
        let copied = store.message_body(id).unwrap().unwrap().attachments;
        assert_eq!(copied.len(), 1);
        assert_eq!(store.attachment_data(copied[0].id).unwrap().unwrap(), b"n");
        let left: Vec<String> = store.recent_attachments(Some(id), 10).unwrap().into_iter().map(|a| a.name).collect();
        assert_eq!(left, vec!["plan.PDF".to_string()]);
    }

    #[test]
    fn scheduled_send_waits_in_drafts_until_due() {
        let (store, acct) = store_with_account();
        let mut d = store.new_draft(ComposeKind::New, Some(acct), None).unwrap();
        d.to.push(EmailAddress::new("Priya", "priya@contoso.example"));
        d.subject = "Later".into();
        let at = Utc::now() + chrono::Duration::hours(2);
        let id = store.schedule_send(&mut d, at).unwrap();
        let drafts = store.folder_for_role(acct, FolderRole::Drafts).unwrap();
        let m = store.message(id).unwrap().unwrap();
        assert_eq!(m.folder_id, drafts);
        assert_eq!(m.scheduled_send_at, Some(at));
        assert!(store.due_scheduled_drafts(Utc::now()).unwrap().is_empty());
        assert!(store.send_due_scheduled(Utc::now()).unwrap().is_empty());

        // Cancel send, then schedule again and let it come due.
        store.cancel_scheduled_send(&mut d).unwrap();
        assert_eq!(store.message(id).unwrap().unwrap().scheduled_send_at, None);
        store.schedule_send(&mut d, at).unwrap();
        assert_eq!(store.due_scheduled_drafts(at).unwrap(), vec![id]);
        assert_eq!(store.send_due_scheduled(at).unwrap(), vec![id]);
        let sent = store.folder_for_role(acct, FolderRole::Sent).unwrap();
        let m = store.message(id).unwrap().unwrap();
        assert_eq!((m.folder_id, m.is_draft, m.scheduled_send_at), (sent, false, None));
    }

    #[test]
    fn discarded_scheduled_draft_is_not_sent() {
        let (store, acct) = store_with_account();
        let mut d = store.new_draft(ComposeKind::New, Some(acct), None).unwrap();
        d.to.push(EmailAddress::new("", "priya@contoso.example"));
        let at = Utc::now();
        let id = store.schedule_send(&mut d, at).unwrap();
        let undo = store.discard_draft_undoable(id).unwrap();
        let deleted = store.folder_for_role(acct, FolderRole::Deleted).unwrap();
        assert_eq!(store.message(id).unwrap().unwrap().folder_id, deleted);
        assert!(store.due_scheduled_drafts(at + chrono::Duration::hours(1)).unwrap().is_empty());
        store.undo_moves(&undo).unwrap();
        let drafts = store.folder_for_role(acct, FolderRole::Drafts).unwrap();
        assert_eq!(store.message(id).unwrap().unwrap().folder_id, drafts);
        // A second discard from Deleted Items removes it for good.
        store.discard_draft(id).unwrap();
        store.discard_draft(id).unwrap();
        assert!(store.message(id).unwrap().is_none());
    }

    #[test]
    fn undo_send_holds_in_outbox_and_can_be_taken_back() {
        let (store, acct) = store_with_account();
        let outbox = store.folder_for_role(acct, FolderRole::Outbox).unwrap();
        let drafts = store.folder_for_role(acct, FolderRole::Drafts).unwrap();
        let sent = store.folder_for_role(acct, FolderRole::Sent).unwrap();
        let mut d = store.new_draft(ComposeKind::New, Some(acct), None).unwrap();
        d.to.push(EmailAddress::new("Priya", "priya@contoso.example"));
        let until = Utc::now() + chrono::Duration::seconds(5);

        // Undo: back to Drafts, nothing is sent.
        let rec = store.queue_send(&mut d, until).unwrap();
        let id = rec.message_id;
        assert_eq!(rec.from_folder, drafts);
        let held = store.message(id).unwrap().unwrap();
        assert!(held.is_draft && held.folder_id == outbox);
        assert!(store.overdue_queued_sends(Utc::now()).unwrap().is_empty());
        assert_eq!(store.overdue_queued_sends(until).unwrap(), vec![id]);
        store.undo_moves(&[rec]).unwrap();
        assert!(!store.release_queued_send(id).unwrap());
        let reopened = store.unqueue_send(id).unwrap().unwrap();
        assert_eq!(reopened.send_pending_until, None);
        assert_eq!(store.message(id).unwrap().unwrap().folder_id, drafts);

        // No undo: released after the delay and delivered.
        let mut d = reopened;
        store.queue_send(&mut d, until).unwrap();
        assert!(store.release_queued_send(id).unwrap());
        let m = store.message(id).unwrap().unwrap();
        assert_eq!((m.folder_id, m.is_draft), (sent, false));
        assert!(!store.release_queued_send(id).unwrap(), "only sent once");
    }

    #[test]
    fn unqueue_moves_a_held_message_back_to_drafts() {
        let (store, acct) = store_with_account();
        let drafts = store.folder_for_role(acct, FolderRole::Drafts).unwrap();
        let mut d = store.new_draft(ComposeKind::New, Some(acct), None).unwrap();
        d.to.push(EmailAddress::new("", "priya@contoso.example"));
        let id = store.queue_send(&mut d, Utc::now()).unwrap().message_id;
        store.unqueue_send(id).unwrap();
        assert_eq!(store.message(id).unwrap().unwrap().folder_id, drafts);
        assert!(store.overdue_queued_sends(Utc::now() + chrono::Duration::hours(1)).unwrap().is_empty());
    }

    #[test]
    fn changing_account_moves_draft() {
        let (store, acct) = store_with_account();
        let mut other = Account { display_name: "Work".into(), email: "me@work.example".into(), ..Default::default() };
        let other_id = store.add_account(&mut other).unwrap();
        let mut d = store.new_draft(ComposeKind::New, Some(acct), None).unwrap();
        let id = store.save_draft(&mut d).unwrap();
        d.account_id = other_id;
        store.save_draft(&mut d).unwrap();
        let m = store.message(id).unwrap().unwrap();
        assert_eq!(m.folder_id, store.folder_for_role(other_id, FolderRole::Drafts).unwrap());
        assert_eq!(m.from.address, "me@work.example");
    }

    #[test]
    fn server_draft_is_sanitised_when_rebuilt() {
        let (store, acct) = store_with_account();
        let drafts = store.folder_for_role(acct, FolderRole::Drafts).unwrap();
        let mut m = Message { account_id: acct, folder_id: drafts, is_draft: true, subject: "Srv".into(), received_at: Utc::now(), ..Default::default() };
        let body = MessageBody {
            html: Some("<p onclick=\"x()\">Hi</p><script>alert(1)</script><style>p{}</style>".into()),
            text: "Hi".into(),
            attachments: vec![Attachment { name: "a.txt".into(), size: 1, ..Default::default() }],
            ..Default::default()
        };
        let id = store.insert_message(&mut m, Some(&body)).unwrap();
        let d = store.draft(id).unwrap().unwrap();
        assert!(d.body_html.contains("Hi"));
        assert!(!d.body_html.contains("onclick") && !d.body_html.contains("script") && !d.body_html.contains("<style"));
        assert_eq!(d.attachments.len(), 1);
        assert!(d.attachments[0].path.starts_with(STORED));
    }

    #[test]
    fn suggestions_mark_and_forget_recent_addresses() {
        let (store, _) = store_with_account();
        store.record_recent_addresses(&[EmailAddress::new("Priya Raman", "Priya@Northwind.example")]).unwrap();
        let s = store.recipient_suggestions("priya", 5).unwrap();
        assert_eq!(s.len(), 1);
        assert!(s[0].1, "recent entry is marked");
        store.forget_recent_address("PRIYA@northwind.example").unwrap();
        assert!(store.recipient_suggestions("priya", 5).unwrap().is_empty());
    }

    #[test]
    fn base64_matches_rfc4648_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn strip_style_blocks_is_case_insensitive() {
        assert_eq!(strip_style_blocks("a<STYLE type=x>b</Style>c<link href=y>d"), "acd");
        assert_eq!(strip_style_blocks("<style>unterminated"), "");
    }
}
