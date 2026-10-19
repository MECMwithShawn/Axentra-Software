//! Outgoing mail (§5.9): `multipart/alternative` (text + HTML), wrapped in
//! `multipart/related` when the HTML references inline images, wrapped in
//! `multipart/mixed` when there are attachments. The transmitted copy has
//! no `Bcc` header; the Sent copy keeps it.

use axentra_core::{EmailAddress, Importance};
use base64::Engine as _;
use chrono::{DateTime, Utc};
use mail_builder::MessageBuilder;
use mail_builder::headers::address::Address;
use mail_builder::headers::date::Date;
use mail_builder::headers::message_id::MessageId;
use mail_builder::headers::raw::Raw;
use mail_builder::mime::MimePart;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutAttachment {
    pub name: String,
    pub content_type: String,
    pub data: Vec<u8>,
    /// Set for inline images referenced from the HTML as `cid:…`.
    pub content_id: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Outgoing {
    pub from: EmailAddress,
    pub to: Vec<EmailAddress>,
    pub cc: Vec<EmailAddress>,
    pub bcc: Vec<EmailAddress>,
    pub subject: String,
    pub html: String,
    pub importance: Importance,
    pub read_receipt: bool,
    /// Without angle brackets.
    pub message_id: String,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub date: DateTime<Utc>,
    pub attachments: Vec<OutAttachment>,
}

#[derive(Clone, Debug)]
pub struct BuiltMessage {
    /// Bytes handed to SMTP / Graph (no Bcc header).
    pub transmit: Vec<u8>,
    /// Bytes appended to Sent (with Bcc).
    pub sent_copy: Vec<u8>,
    pub envelope_from: String,
    /// Every To, Cc and Bcc address, once.
    pub recipients: Vec<String>,
}

fn address_list(list: &[EmailAddress]) -> Address<'static> {
    Address::new_list(
        list.iter()
            .map(|a| {
                let name = a.name.trim();
                Address::new_address((!name.is_empty()).then(|| name.to_string()), a.address.trim().to_string())
            })
            .collect(),
    )
}

/// Replace `src="data:<type>;base64,…"` images with `cid:` references and
/// return them as inline parts (compose may embed pasted images that way).
pub fn extract_data_images(html: &str) -> (String, Vec<OutAttachment>) {
    let mut out = String::with_capacity(html.len());
    let mut parts = Vec::new();
    let mut rest = html;
    while let Some(pos) = rest.find("data:image/") {
        let quote = rest[..pos].chars().last();
        let is_src = matches!(quote, Some('"') | Some('\'')) && rest[..pos.saturating_sub(1)].trim_end().ends_with('=');
        let Some(q) = quote.filter(|_| is_src) else {
            out.push_str(&rest[..pos + 5]);
            rest = &rest[pos + 5..];
            continue;
        };
        let Some(end) = rest[pos..].find(q) else { break };
        let uri = &rest[pos..pos + end];
        let decoded = uri
            .strip_prefix("data:")
            .and_then(|u| u.split_once(";base64,"))
            .and_then(|(ct, data)| base64::engine::general_purpose::STANDARD.decode(data.trim()).ok().map(|d| (ct.to_string(), d)));
        match decoded {
            Some((content_type, data)) => {
                let cid = format!("image{}.{}@axentra", parts.len() + 1, &uuid::Uuid::new_v4().simple().to_string()[..8]);
                out.push_str(&rest[..pos]);
                out.push_str("cid:");
                out.push_str(&cid);
                let ext = content_type.rsplit('/').next().unwrap_or("png").to_string();
                parts.push(OutAttachment { name: format!("image{}.{ext}", parts.len() + 1), content_type, data, content_id: Some(cid) });
            }
            None => out.push_str(&rest[..pos + end]),
        }
        rest = &rest[pos + end..];
    }
    out.push_str(rest);
    (out, parts)
}

fn body_tree(html: &str, attachments: &[OutAttachment]) -> MimePart<'static> {
    let (html, embedded) = extract_data_images(html);
    let text = axentra_core::html::html_to_text(&html);
    let alternative = MimePart::new("multipart/alternative", vec![MimePart::new("text/plain", text), MimePart::new("text/html", html)]);
    let inline: Vec<MimePart<'static>> = embedded
        .iter()
        .chain(attachments.iter().filter(|a| a.content_id.is_some()))
        .map(|a| {
            MimePart::new(a.content_type.clone(), a.data.clone()).inline().cid(a.content_id.clone().unwrap_or_default())
        })
        .collect();
    let related = if inline.is_empty() {
        alternative
    } else {
        let mut parts = vec![alternative];
        parts.extend(inline);
        MimePart::new("multipart/related", parts)
    };
    let files: Vec<MimePart<'static>> = attachments
        .iter()
        .filter(|a| a.content_id.is_none())
        .map(|a| MimePart::new(a.content_type.clone(), a.data.clone()).attachment(a.name.clone()))
        .collect();
    if files.is_empty() {
        related
    } else {
        let mut parts = vec![related];
        parts.extend(files);
        MimePart::new("multipart/mixed", parts)
    }
}

/// Build the transmitted and Sent copies of a message.
pub fn build(out: &Outgoing) -> std::io::Result<BuiltMessage> {
    let mut builder = MessageBuilder::new()
        .from(address_list(std::slice::from_ref(&out.from)))
        .subject(out.subject.clone())
        .date(Date::new(out.date.timestamp()))
        .message_id(MessageId::new(out.message_id.clone()));
    if !out.to.is_empty() {
        builder = builder.to(address_list(&out.to));
    }
    if !out.cc.is_empty() {
        builder = builder.cc(address_list(&out.cc));
    }
    if let Some(irt) = &out.in_reply_to {
        builder = builder.in_reply_to(MessageId::new(irt.clone()));
    }
    if !out.references.is_empty() {
        builder = builder.references(MessageId::new_list(out.references.clone().into_iter()));
    }
    match out.importance {
        Importance::High => builder = builder.header("Importance", Raw::new("high")).header("X-Priority", Raw::new("1")),
        Importance::Low => builder = builder.header("Importance", Raw::new("low")).header("X-Priority", Raw::new("5")),
        Importance::Normal => {}
    }
    if out.read_receipt {
        builder = builder.header("Disposition-Notification-To", address_list(std::slice::from_ref(&out.from)));
    }
    builder = builder.body(body_tree(&out.html, &out.attachments));
    let transmit = builder.clone().write_to_vec()?;
    let sent_copy = if out.bcc.is_empty() { transmit.clone() } else { builder.bcc(address_list(&out.bcc)).write_to_vec()? };
    let mut recipients: Vec<String> = Vec::new();
    for a in out.to.iter().chain(&out.cc).chain(&out.bcc) {
        let addr = a.address.trim().to_string();
        if !addr.is_empty() && !recipients.iter().any(|r| r.eq_ignore_ascii_case(&addr)) {
            recipients.push(addr);
        }
    }
    Ok(BuiltMessage { transmit, sent_copy, envelope_from: out.from.address.trim().to_string(), recipients })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mime::parse::parse_rfc822;

    fn outgoing() -> Outgoing {
        Outgoing {
            from: EmailAddress::new("Ana Ruiz", "ana@example.org"),
            to: vec![EmailAddress::new("Bob", "bob@example.org")],
            cc: vec![EmailAddress::new("", "carol@example.org")],
            bcc: vec![EmailAddress::new("Dan", "dan@example.org"), EmailAddress::new("Bob again", "BOB@example.org")],
            subject: "RE: Café plans".into(),
            html: "<p>See <b>you</b> there <img src=\"data:image/png;base64,iVBORw0KGgo=\"></p>".into(),
            importance: Importance::High,
            message_id: "1234@example.org".into(),
            in_reply_to: Some("orig@example.org".into()),
            references: vec!["root@example.org".into(), "orig@example.org".into()],
            date: DateTime::parse_from_rfc3339("2026-10-19T08:00:00Z").unwrap().with_timezone(&Utc),
            attachments: vec![OutAttachment {
                name: "notes.txt".into(),
                content_type: "text/plain".into(),
                data: b"hello".to_vec(),
                content_id: None,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn outbox_mime_structure_and_bcc_handling() {
        let built = build(&outgoing()).unwrap();
        let wire = String::from_utf8_lossy(&built.transmit);
        assert!(!wire.contains("dan@example.org"), "Bcc leaked into the transmitted copy");
        assert!(wire.contains("Message-ID: <1234@example.org>"));
        assert!(wire.contains("In-Reply-To: <orig@example.org>"));
        assert!(wire.contains("References: <root@example.org> <orig@example.org>"));
        assert!(wire.contains("multipart/mixed"));
        assert!(wire.contains("multipart/related"));
        assert!(wire.contains("multipart/alternative"));
        assert!(wire.contains("X-Priority: 1"));
        // Order: mixed > related > alternative.
        assert!(wire.find("multipart/mixed").unwrap() < wire.find("multipart/related").unwrap());
        assert!(wire.find("multipart/related").unwrap() < wire.find("multipart/alternative").unwrap());
        assert_eq!(built.recipients, vec!["bob@example.org", "carol@example.org", "dan@example.org"]);
        assert_eq!(built.envelope_from, "ana@example.org");

        // Round trip through the parser.
        let parsed = parse_rfc822(&built.transmit).unwrap();
        assert_eq!(parsed.message.subject, "RE: Café plans");
        assert_eq!(parsed.message.to[0].address, "bob@example.org");
        assert!(parsed.message.bcc.is_empty());
        let html = parsed.body.html.unwrap();
        assert!(html.contains("cid:image1."), "{html}");
        assert!(parsed.body.text.contains("See you there"));
        let inline = parsed.attachments.iter().find(|(a, _)| a.is_inline).unwrap();
        assert_eq!(inline.0.content_type, "image/png");
        let file = parsed.attachments.iter().find(|(a, _)| a.name == "notes.txt").unwrap();
        assert_eq!(file.1.as_deref(), Some(&b"hello"[..]));

        let sent = parse_rfc822(&built.sent_copy).unwrap();
        assert_eq!(sent.message.bcc.len(), 2);
    }

    #[test]
    fn plain_message_without_attachments_is_alternative_only() {
        let mut o = outgoing();
        o.attachments.clear();
        o.bcc.clear();
        o.html = "<p>Hi</p>".into();
        o.importance = Importance::Normal;
        let built = build(&o).unwrap();
        let wire = String::from_utf8_lossy(&built.transmit);
        assert!(wire.contains("multipart/alternative"));
        assert!(!wire.contains("multipart/mixed") && !wire.contains("multipart/related") && !wire.contains("X-Priority"));
        assert_eq!(built.transmit, built.sent_copy);
    }

    #[test]
    fn data_uri_extraction_ignores_text_mentions() {
        let (html, parts) = extract_data_images("<p>data:image/ is a scheme</p><img alt=x src='data:image/gif;base64,R0lGODlhAQABAAAAACw='>");
        assert_eq!(parts.len(), 1);
        assert!(html.starts_with("<p>data:image/ is a scheme</p>"));
        assert!(html.contains("src='cid:image1."));
        assert_eq!(parts[0].content_type, "image/gif");
    }
}
