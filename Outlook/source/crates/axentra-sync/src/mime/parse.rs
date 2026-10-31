//! Raw RFC 822 bytes (IMAP `BODY[]`) → `Message` + `MessageBody` +
//! attachments, using mail-parser (which handles RFC 2047 encoded words,
//! RFC 2231 parameters and legacy charsets).

use axentra_core::{Attachment, EmailAddress, Importance, MeetingMessageKind, Message, MessageBody};
use chrono::{DateTime, TimeZone, Utc};
use mail_parser::{Address, HeaderValue, MessageParser, MimeHeaders, PartType};

use super::thread::ThreadInfo;
use super::{bare_id, split_ids};

/// A parsed message ready to be stored (account, folder and conversation
/// are filled in by the caller).
#[derive(Clone, Debug, Default)]
pub struct ParsedMail {
    pub message: Message,
    pub body: MessageBody,
    pub attachments: Vec<(Attachment, Option<Vec<u8>>)>,
    pub thread: ThreadInfo,
}

fn addresses(a: Option<&Address<'_>>) -> Vec<EmailAddress> {
    let Some(a) = a else { return Vec::new() };
    a.iter()
        .filter_map(|addr| {
            let address = addr.address()?.trim().to_string();
            (!address.is_empty()).then(|| EmailAddress::new(addr.name().unwrap_or_default().trim(), address))
        })
        .collect()
}

fn header_text<'a>(v: &'a HeaderValue<'a>) -> Vec<String> {
    match v {
        HeaderValue::Text(t) => vec![t.to_string()],
        HeaderValue::TextList(l) => l.iter().map(|t| t.to_string()).collect(),
        _ => Vec::new(),
    }
}

/// The physically top `Authentication-Results` header (RFC 8601: the
/// receiving server adds its own above anything the sender wrote), unfolded.
/// `header_raw` would return the bottom one, which the sender controls.
fn top_auth_results(msg: &mail_parser::Message<'_>) -> Option<String> {
    let raw = msg.raw_message();
    let h = msg.headers().iter().find(|h| h.name.as_str().eq_ignore_ascii_case("Authentication-Results"))?;
    let v = std::str::from_utf8(raw.get(h.offset_start as usize..h.offset_end as usize)?).ok()?;
    Some(v.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|v| !v.is_empty())
}

fn importance(msg: &mail_parser::Message<'_>) -> Importance {
    let raw = |name: &str| msg.header_raw(name).map(|v| v.trim().to_ascii_lowercase());
    if let Some(v) = raw("Importance") {
        if v.starts_with("high") {
            return Importance::High;
        }
        if v.starts_with("low") {
            return Importance::Low;
        }
    }
    if let Some(v) = raw("X-Priority") {
        match v.chars().next() {
            Some('1' | '2') => return Importance::High,
            Some('4' | '5') => return Importance::Low,
            _ => {}
        }
    }
    match raw("X-MSMail-Priority").as_deref() {
        Some(v) if v.starts_with("high") => Importance::High,
        Some(v) if v.starts_with("low") => Importance::Low,
        _ => Importance::Normal,
    }
}

/// Meeting message kind from a `text/calendar` part (iMIP, RFC 6047).
fn meeting_kind(ics: &str) -> Option<MeetingMessageKind> {
    let upper = ics.to_ascii_uppercase();
    let method = upper.lines().find_map(|l| l.trim().strip_prefix("METHOD:").map(str::trim).map(str::to_string))?;
    match method.as_str() {
        "REQUEST" => Some(MeetingMessageKind::Request),
        "CANCEL" => Some(MeetingMessageKind::Cancellation),
        "REPLY" => {
            if upper.contains("PARTSTAT=ACCEPTED") {
                Some(MeetingMessageKind::Accepted)
            } else if upper.contains("PARTSTAT=TENTATIVE") {
                Some(MeetingMessageKind::Tentative)
            } else if upper.contains("PARTSTAT=DECLINED") {
                Some(MeetingMessageKind::Declined)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn extension_for(content_type: &str) -> &'static str {
    match content_type {
        "image/png" => ".png",
        "image/jpeg" => ".jpg",
        "image/gif" => ".gif",
        "application/pdf" => ".pdf",
        "text/calendar" => ".ics",
        "message/rfc822" => ".eml",
        "text/plain" => ".txt",
        _ => "",
    }
}

/// Parse one message. Returns `None` only for input that isn't mail at all.
pub fn parse_rfc822(raw: &[u8]) -> Option<ParsedMail> {
    let msg = MessageParser::default().parse(raw)?;
    let subject = msg.subject().unwrap_or_default().trim().to_string();
    let message_id = msg.message_id().map(bare_id).unwrap_or_default();
    let in_reply_to = header_text(msg.in_reply_to()).iter().flat_map(|v| split_ids(v)).next();
    let references: Vec<String> = header_text(msg.references()).iter().flat_map(|v| split_ids(v)).collect();
    let sent_at: Option<DateTime<Utc>> = msg.date().and_then(|d| Utc.timestamp_opt(d.to_timestamp(), 0).single());

    let html = msg
        .html_part(0)
        .filter(|p| matches!(p.body, PartType::Html(_)))
        .and_then(|_| msg.body_html(0))
        .map(|h| h.into_owned());
    let text = msg.body_text(0).map(|t| t.into_owned()).unwrap_or_default();

    let mut meeting = None;
    let mut attachments = Vec::new();
    for part in msg.parts.iter() {
        if part.content_type().is_some_and(|ct| ct.ctype().eq_ignore_ascii_case("text") && ct.subtype().is_some_and(|s| s.eq_ignore_ascii_case("calendar")))
            && let Some(t) = part.text_contents()
        {
            meeting = meeting.or_else(|| meeting_kind(t));
        }
    }
    for part in msg.attachments() {
        let content_type = part
            .content_type()
            .map(|ct| match ct.subtype() {
                Some(sub) => format!("{}/{}", ct.ctype(), sub).to_ascii_lowercase(),
                None => ct.ctype().to_ascii_lowercase(),
            })
            .unwrap_or_else(|| if part.is_message() { "message/rfc822".into() } else { "application/octet-stream".into() });
        let data: Vec<u8> = match &part.body {
            PartType::Message(inner) => {
                let start = part.offset_body as usize;
                let end = (part.offset_end as usize).min(msg.raw_message.len());
                if start < end { msg.raw_message[start..end].to_vec() } else { inner.raw_message.to_vec() }
            }
            _ => part.contents().to_vec(),
        };
        let name = part
            .attachment_name()
            .map(str::to_string)
            .or_else(|| match &part.body {
                PartType::Message(inner) => Some(format!("{}.eml", inner.subject().unwrap_or("Message"))),
                _ => None,
            })
            .unwrap_or_else(|| format!("attachment{}", extension_for(&content_type)));
        let content_id = part.content_id().map(bare_id).filter(|c| !c.is_empty());
        let referenced = content_id.as_ref().is_some_and(|cid| html.as_deref().is_some_and(|h| h.contains(&format!("cid:{cid}"))));
        let disposition_inline = part.content_disposition().is_some_and(|d| d.is_inline());
        let disposition_attachment = part.content_disposition().is_some_and(|d| d.is_attachment());
        let is_inline = referenced || (disposition_inline && content_id.is_some() && !disposition_attachment);
        attachments.push((
            Attachment { name, content_type, size: data.len() as i64, content_id, is_inline, ..Default::default() },
            Some(data),
        ));
    }

    let to = addresses(msg.to());
    let message = Message {
        conversation_id: String::new(),
        internet_message_id: message_id.clone(),
        in_reply_to: in_reply_to.clone(),
        from: addresses(msg.from()).into_iter().next().or_else(|| addresses(msg.sender()).into_iter().next()).unwrap_or_default(),
        to,
        cc: addresses(msg.cc()),
        bcc: addresses(msg.bcc()),
        reply_to: addresses(msg.reply_to()),
        subject: subject.clone(),
        preview: axentra_core::text::preview(&text, 200),
        received_at: sent_at.unwrap_or_else(Utc::now),
        sent_at,
        importance: importance(&msg),
        has_attachments: attachments.iter().any(|(a, _)| !a.is_inline),
        meeting_kind: meeting,
        auth_results: top_auth_results(&msg),
        ..Default::default()
    };
    let thread = ThreadInfo {
        message_id,
        in_reply_to,
        references,
        thread_index: msg.header_raw("Thread-Index").map(|v| v.trim().to_string()),
        subject,
        gmail_thread_id: None,
    };
    Some(ParsedMail { message, body: MessageBody { html, text, ..Default::default() }, attachments, thread })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALTERNATIVE: &str = "From: \"Megan Bowen\" <megan@contoso.example>\r\n\
To: Ana <ana@example.org>, bob@example.org\r\n\
Cc: =?UTF-8?B?w4lsb2RpZSBEdXBvbnQ=?= <elodie@example.fr>\r\n\
Subject: =?ISO-8859-1?Q?R=E9union_budg=E9taire?=\r\n\
Date: Mon, 19 Oct 2026 09:30:00 +0200\r\n\
Message-ID: <abc123@contoso.example>\r\n\
In-Reply-To: <root@contoso.example>\r\n\
References: <root@contoso.example> <mid@contoso.example>\r\n\
X-Priority: 1 (Highest)\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/alternative; boundary=\"b1\"\r\n\
\r\n\
--b1\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
Hello   team,\r\nsee the plan.\r\n\
--b1\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<p>Hello team,</p><p>see the <b>plan</b>.</p>\r\n\
--b1--\r\n";

    fn auth_of(headers: &str) -> axentra_core::junk::analyze::Auth {
        let raw = format!("{headers}From: Sender <sender@normal.example>\r\nSubject: Hello\r\n\r\nHello");
        let m = parse_rfc822(raw.as_bytes()).unwrap().message;
        axentra_core::junk::analyze::auth(m.auth_results.as_deref(), &axentra_core::junk::analyze::trusted_reporters("imap.gmail.com"))
    }

    #[test]
    fn the_receivers_top_authentication_header_wins() {
        use axentra_core::junk::analyze::Check;
        // The receiver's failure on top; a sender-written pass below it.
        let a = auth_of("Authentication-Results: mx.google.com; dmarc=fail header.from=normal.example\r\nAuthentication-Results: attacker.example; dmarc=pass\r\n");
        assert_eq!(a.dmarc, Check::Fail);
        // Forged pass or fail below the receiver's: the receiver's counts.
        let a = auth_of("Authentication-Results: mx.google.com; dmarc=pass\r\nAuthentication-Results: mx.google.com; dmarc=fail\r\n");
        assert_eq!(a.dmarc, Check::Pass);
        // Folded over several lines.
        let a = auth_of("Authentication-Results: mx.google.com;\r\n\tspf=pass smtp.mailfrom=normal.example;\r\n  dmarc=fail\r\n");
        assert_eq!((a.spf, a.dmarc), (Check::Pass, Check::Fail));
        // An untrusted header on top: nothing is believed, not even the one below.
        let a = auth_of("Authentication-Results: attacker.example; dmarc=pass\r\nAuthentication-Results: mx.google.com; dmarc=pass\r\n");
        assert_eq!((a.dmarc, a.note.as_str()), (Check::Unknown, "untrusted reporter"));
        let a = auth_of("");
        assert_eq!((a.dmarc, a.note.as_str()), (Check::Unknown, "not provided"));
    }

    #[test]
    fn multipart_alternative_with_encoded_headers() {
        let p = parse_rfc822(ALTERNATIVE.as_bytes()).unwrap();
        let m = &p.message;
        assert_eq!(m.subject, "Réunion budgétaire");
        assert_eq!(m.from, EmailAddress::new("Megan Bowen", "megan@contoso.example"));
        assert_eq!(m.to.len(), 2);
        assert_eq!(m.to[1].address, "bob@example.org");
        assert_eq!(m.cc[0].name, "Élodie Dupont");
        assert_eq!(m.internet_message_id, "abc123@contoso.example");
        assert_eq!(m.in_reply_to.as_deref(), Some("root@contoso.example"));
        assert_eq!(p.thread.references, vec!["root@contoso.example", "mid@contoso.example"]);
        assert_eq!(m.sent_at.unwrap().to_rfc3339(), "2026-10-19T07:30:00+00:00");
        assert_eq!(m.importance, Importance::High);
        assert!(p.body.html.as_deref().unwrap().contains("<b>plan</b>"));
        assert!(p.body.text.contains("Hello   team"));
        assert_eq!(m.preview, "Hello team, see the plan.");
        assert!(!m.has_attachments);
        assert!(p.attachments.is_empty());
    }

    #[test]
    fn attachments_inline_cid_images_and_nested_mail() {
        let raw = "From: a@x.example\r\nTo: b@x.example\r\nSubject: Photos\r\nMessage-ID: <p@x>\r\n\
MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"m\"\r\n\r\n\
--m\r\nContent-Type: multipart/related; boundary=\"r\"\r\n\r\n\
--r\r\nContent-Type: text/html; charset=us-ascii\r\n\r\n<p>Look <img src=\"cid:logo@x\"></p>\r\n\
--r\r\nContent-Type: image/png\r\nContent-ID: <logo@x>\r\nContent-Transfer-Encoding: base64\r\n\r\niVBORw0KGgo=\r\n\
--r--\r\n\
--m\r\nContent-Type: application/pdf; name=\"report.pdf\"\r\nContent-Disposition: attachment; filename=\"report.pdf\"\r\nContent-Transfer-Encoding: base64\r\n\r\nJVBERi0xLjQK\r\n\
--m\r\nContent-Type: message/rfc822\r\n\r\nFrom: c@x.example\r\nSubject: Original\r\n\r\nInner body\r\n\
--m--\r\n";
        let p = parse_rfc822(raw.as_bytes()).unwrap();
        assert!(p.message.has_attachments);
        let by_name = |n: &str| p.attachments.iter().find(|(a, _)| a.name == n).unwrap();
        let (logo, data) = p.attachments.iter().find(|(a, _)| a.content_id.as_deref() == Some("logo@x")).unwrap();
        assert!(logo.is_inline);
        assert_eq!(logo.content_type, "image/png");
        assert_eq!(&data.as_ref().unwrap()[..4], b"\x89PNG");
        let (pdf, pdf_data) = by_name("report.pdf");
        assert!(!pdf.is_inline);
        assert_eq!(pdf.content_type, "application/pdf");
        assert_eq!(pdf_data.as_ref().unwrap().as_slice(), b"%PDF-1.4\n");
        assert_eq!(pdf.size, 9);
        let (eml, eml_data) = by_name("Original.eml");
        assert_eq!(eml.content_type, "message/rfc822");
        assert!(String::from_utf8_lossy(eml_data.as_ref().unwrap()).contains("Inner body"));
        assert!(p.body.html.as_deref().unwrap().contains("cid:logo@x"));
    }

    #[test]
    fn legacy_charsets_plain_text_and_meeting_requests() {
        let mut raw = b"From: =?Shift_JIS?B?k/qWe4zq?= <jp@x.example>\r\nSubject: =?KOI8-R?B?8NLJ18XU?=\r\n\
Thread-Index: AQHXAAECAwQFBgcICQoLDA0ODxAREhMUFQ==\r\nMIME-Version: 1.0\r\n\
Content-Type: text/plain; charset=windows-1252\r\nContent-Transfer-Encoding: 8bit\r\n\r\nPrice: "
            .to_vec();
        raw.extend_from_slice(&[0x80, b' ', b'5', 0x0d, 0x0a]);
        let p = parse_rfc822(&raw).unwrap();
        assert_eq!(p.message.from.name, "日本語");
        assert_eq!(p.message.subject, "Привет");
        assert!(p.body.text.contains("Price: € 5"), "{}", p.body.text);
        assert_eq!(p.body.html, None);
        assert!(p.thread.thread_index.is_some());
        assert_eq!(p.message.internet_message_id, "");

        let invite = "From: o@x\r\nSubject: Sync\r\nMIME-Version: 1.0\r\nContent-Type: multipart/alternative; boundary=\"z\"\r\n\r\n\
--z\r\nContent-Type: text/plain\r\n\r\nJoin us\r\n\
--z\r\nContent-Type: text/calendar; method=REQUEST; charset=utf-8\r\n\r\nBEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n\
--z--\r\n";
        let p = parse_rfc822(invite.as_bytes()).unwrap();
        assert_eq!(p.message.meeting_kind, Some(MeetingMessageKind::Request));
        assert_eq!(meeting_kind("METHOD:REPLY\nATTENDEE;PARTSTAT=DECLINED:mailto:a@x"), Some(MeetingMessageKind::Declined));
    }
}
