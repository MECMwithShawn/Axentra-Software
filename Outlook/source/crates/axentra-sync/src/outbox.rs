//! Outbox (§5.9, §9.3): `Store::send_draft` queues real accounts' mail in
//! the Outbox folder; the engine builds the MIME from the saved draft and
//! hands it to SMTP or Graph once the undo-send delay (and any scheduled
//! time) has passed. Sent messages move to Sent Items locally.

use axentra_core::{Account, EmailAddress, FolderRole, Message, MessageId, Store};
use chrono::{Duration, Utc};

use crate::config::domain_of;
use crate::error::{SyncError, SyncResult};
use crate::mime::build::{BuiltMessage, OutAttachment, Outgoing, build};
use crate::mime::split_ids;

/// A message ready to go out.
pub struct Ready {
    pub message: Message,
    pub built: BuiltMessage,
    /// Message-ID used on the wire (no angle brackets).
    pub message_id: String,
}

/// Outbox messages whose undo-send window and scheduled time have passed.
pub fn due(store: &Store, account: &Account) -> SyncResult<Vec<Message>> {
    let Some(outbox) = store.folder_by_role(account.id, FolderRole::Outbox)? else { return Ok(Vec::new()) };
    if outbox.total_count == 0 {
        return Ok(Vec::new());
    }
    let delay = Duration::seconds(store.settings().undo_send_secs as i64);
    let now = Utc::now();
    let mut out = Vec::new();
    for m in store.folder_messages(outbox.id)? {
        let queued_at = m.sent_at.unwrap_or(m.received_at);
        if queued_at + delay > now {
            continue;
        }
        if let Some(d) = store.draft(m.id)?
            && d.scheduled_send_at.is_some_and(|t| t > now)
        {
            continue;
        }
        out.push(m);
    }
    Ok(out)
}

/// A Message-ID with the sender's domain (§5.9). Keeps one the app already
/// generated unless it uses the placeholder `axentra.local` domain.
pub fn message_id_for(existing: &str, from: &str) -> String {
    let existing = existing.trim().trim_matches(['<', '>']);
    if !existing.is_empty() && !existing.ends_with("@axentra.local") {
        return existing.to_string();
    }
    let domain = domain_of(from).unwrap_or_else(|| "axentra.invalid".into());
    format!("{}@{domain}", uuid::Uuid::new_v4().simple())
}

/// Build the wire message for one queued item.
pub fn prepare(store: &Store, account: &Account, message: &Message) -> SyncResult<Ready> {
    let draft = store.draft(message.id)?;
    let body = store.message_body(message.id)?.unwrap_or_default();
    let html = draft
        .as_ref()
        .map(|d| d.body_html.clone())
        .filter(|h| !h.trim().is_empty())
        .or(body.html.clone())
        .unwrap_or_else(|| format!("<div style=\"white-space:pre-wrap\">{}</div>", body.text.replace('&', "&amp;").replace('<', "&lt;")));
    let mut attachments = Vec::new();
    for a in draft.as_ref().map(|d| d.attachments.clone()).unwrap_or_default() {
        let data = if let Some(id) = a.path.strip_prefix("attachment:") {
            id.parse().ok().and_then(|id| store.attachment_data(id).ok().flatten())
        } else {
            std::fs::read(&a.path).ok()
        };
        let Some(data) = data else {
            return Err(SyncError::User(format!("The attachment “{}” couldn't be read.", a.name)));
        };
        attachments.push(OutAttachment {
            name: a.name.clone(),
            content_type: if a.content_type.is_empty() { "application/octet-stream".into() } else { a.content_type.clone() },
            data,
            content_id: None,
        });
    }
    // Inline images embedded by the composer as stored attachments.
    for a in body.attachments.iter().filter(|a| a.is_inline && a.content_id.is_some()) {
        if html.contains(&format!("cid:{}", a.content_id.as_deref().unwrap_or_default()))
            && let Some(data) = store.attachment_data(a.id)?
        {
            attachments.push(OutAttachment { name: a.name.clone(), content_type: a.content_type.clone(), data, content_id: a.content_id.clone() });
        }
    }
    let source = match draft.as_ref().and_then(|d| d.source_message_id) {
        Some(id) => store.message(id)?,
        None => None,
    };
    let in_reply_to = source.as_ref().map(|s| s.internet_message_id.clone()).filter(|s| !s.is_empty());
    let mut references: Vec<String> = Vec::new();
    if let Some(s) = &source {
        if let Some(irt) = &s.in_reply_to {
            references.extend(split_ids(irt));
        }
        if !s.internet_message_id.is_empty() {
            references.push(s.internet_message_id.clone());
        }
    }
    let message_id = message_id_for(&message.internet_message_id, &account.email);
    let out = Outgoing {
        from: EmailAddress::new(account.display_name.clone(), account.email.clone()),
        to: draft.as_ref().map(|d| d.to.clone()).unwrap_or_else(|| message.to.clone()),
        cc: draft.as_ref().map(|d| d.cc.clone()).unwrap_or_else(|| message.cc.clone()),
        bcc: draft.as_ref().map(|d| d.bcc.clone()).unwrap_or_else(|| message.bcc.clone()),
        subject: message.subject.clone(),
        html,
        importance: message.importance,
        read_receipt: draft.as_ref().is_some_and(|d| d.request_read_receipt),
        message_id: message_id.clone(),
        in_reply_to: if draft.as_ref().is_some_and(|d| d.kind == axentra_core::ComposeKind::Forward) { None } else { in_reply_to },
        references,
        date: Utc::now(),
        attachments,
    };
    if out.to.is_empty() && out.cc.is_empty() && out.bcc.is_empty() {
        return Err(SyncError::User("Add at least one recipient.".into()));
    }
    let built = build(&out).map_err(|e| SyncError::protocol(format!("Couldn't build the message: {e}")))?;
    Ok(Ready { message: message.clone(), built, message_id })
}

/// Sent: move to Sent Items and record the Message-ID.
pub fn finish(store: &Store, account: &Account, id: MessageId, message_id: &str) -> SyncResult<()> {
    let sent = store.folder_by_role(account.id, FolderRole::Sent)?.ok_or_else(|| SyncError::protocol("This account has no Sent Items folder."))?;
    store.finish_sent_message(id, sent.id, message_id)?;
    Ok(())
}

/// A permanent failure: back to Drafts so nothing is lost (§18 "Saved in Drafts").
pub fn return_to_drafts(store: &Store, account: &Account, id: MessageId) -> SyncResult<()> {
    let Some(drafts) = store.folder_by_role(account.id, FolderRole::Drafts)? else { return Ok(()) };
    if let Some(mut m) = store.message(id)? {
        m.folder_id = drafts.id;
        m.is_draft = true;
        store.update_message(&m)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axentra_core::{AccountKind, ComposeKind, DraftAttachment};

    #[test]
    fn queued_message_becomes_mime_and_moves_to_sent() {
        let store = Store::open_in_memory().unwrap();
        let mut acct = Account { kind: AccountKind::Imap, email: "ana@example.org".into(), display_name: "Ana".into(), ..Default::default() };
        let id = store.add_account(&mut acct).unwrap();
        let mut settings = store.settings();
        settings.undo_send_secs = 0;
        store.save_settings(&settings).unwrap();
        let tmp = std::env::temp_dir().join(format!("axentra-outbox-{}.txt", uuid::Uuid::new_v4()));
        std::fs::write(&tmp, b"notes").unwrap();
        let mut draft = store.new_draft(ComposeKind::New, Some(id), None).unwrap();
        draft.to = vec![EmailAddress::new("Bob", "bob@example.org")];
        draft.bcc = vec![EmailAddress::new("", "boss@example.org")];
        draft.subject = "Plan".into();
        draft.body_html = "<p>Hello <b>Bob</b></p>".into();
        draft.attachments = vec![DraftAttachment { name: "notes.txt".into(), content_type: "text/plain".into(), size: 5, path: tmp.display().to_string() }];
        let mid = store.send_draft(&mut draft).unwrap();
        let acct = store.account(id).unwrap().unwrap();

        let queued = due(&store, &acct).unwrap();
        assert_eq!(queued.len(), 1);
        let ready = prepare(&store, &acct, &queued[0]).unwrap();
        assert!(ready.message_id.ends_with("@example.org"));
        let wire = String::from_utf8_lossy(&ready.built.transmit).to_string();
        assert!(wire.contains("Subject: Plan") && wire.contains("notes.txt") && !wire.contains("boss@example.org"));
        assert!(ready.built.recipients.contains(&"boss@example.org".to_string()));
        finish(&store, &acct, mid, &ready.message_id).unwrap();
        let sent = store.folder_by_role(id, FolderRole::Sent).unwrap().unwrap();
        let m = store.message(mid).unwrap().unwrap();
        assert_eq!((m.folder_id, m.internet_message_id.clone()), (sent.id, ready.message_id.clone()));
        assert!(due(&store, &acct).unwrap().is_empty());
        let _ = std::fs::remove_file(tmp);

        assert_eq!(message_id_for("abc@x.example", "a@b.c"), "abc@x.example");
        assert!(message_id_for("abc@axentra.local", "a@b.c").ends_with("@b.c"));
    }

    #[test]
    fn undo_send_delay_holds_messages_back() {
        let store = Store::open_in_memory().unwrap();
        let mut acct = Account { kind: AccountKind::Imap, email: "ana@example.org".into(), ..Default::default() };
        let id = store.add_account(&mut acct).unwrap();
        let mut draft = store.new_draft(ComposeKind::New, Some(id), None).unwrap();
        draft.to = vec![EmailAddress::new("Bob", "bob@example.org")];
        store.send_draft(&mut draft).unwrap();
        // Default undo-send delay is a few seconds: not yet due.
        assert!(due(&store, &store.account(id).unwrap().unwrap()).unwrap().is_empty());
    }
}
