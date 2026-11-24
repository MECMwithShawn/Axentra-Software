//! The junk filter: which new Inbox mail is clearly junk and goes to Junk
//! Email by itself (Settings › Mail › Junk email, on by default). Unlike the
//! warning signs and the learner, this moves mail, so it is narrow: only
//! mail from someone you've never dealt with, and only on strong signs. The
//! store decides who is unknown and when a message is new; this module only
//! weighs the message.

use super::analyze::{Analysis, Severity};
use crate::model::{Attachment, Importance, Message};

/// Text shorter than this (characters, after tags) counts as no message.
const BARE_TEXT: usize = 80;

/// Why a message was filtered, in words for the reading pane. Empty when it
/// stays in the Inbox.
///
/// It moves on a Danger sign or the learned model alone. The look of a
/// message (a subject of only codes, marked urgent, a document and next to
/// nothing to read) is common in real mail from someone new too: "RE: PO
/// 4500123987" with a full reply, photos from family, "Signed contract",
/// urgent, with a PDF. So it counts only with a Caution sign, or with all
/// three together, the attachment lure ("SCW07598 26833.26833.865").
pub fn junk_reasons(m: &Message, text: &str, attachments: &[Attachment], analysis: &Analysis, model_says_junk: bool) -> Vec<String> {
    if analysis.severity() == Some(Severity::Danger) {
        return analysis.findings.iter().filter(|f| f.severity() == Severity::Danger).map(|f| f.describe()).collect();
    }
    let code = code_subject(&m.subject);
    let urgent = m.importance == Importance::High;
    let bare_document = attachments.iter().any(document) && text.split_whitespace().map(str::len).sum::<usize>() < BARE_TEXT;
    let caution: Vec<String> = analysis.findings.iter().filter(|f| f.severity() == Severity::Caution).map(|f| f.describe()).collect();
    let looks = code || urgent || bare_document;
    if !(model_says_junk || code && urgent && bare_document || looks && !caution.is_empty()) {
        return Vec::new();
    }
    let mut reasons = Vec::new();
    if model_says_junk {
        reasons.push("It's like mail you reported as junk.".to_string());
    }
    if code {
        reasons.push("The subject is only codes and numbers.".to_string());
    }
    if urgent {
        reasons.push("It's marked urgent.".to_string());
    }
    if bare_document {
        reasons.push("It has a document attached and next to no text.".to_string());
    }
    reasons.extend(caution);
    reasons
}

/// A file to open, not a photo, a recording or an invitation.
fn document(a: &Attachment) -> bool {
    const MEDIA: &[&str] = &["jpg", "jpeg", "png", "gif", "heic", "heif", "webp", "bmp", "mp4", "mov", "m4a", "mp3", "wav", "ics"];
    let ty = a.content_type.to_ascii_lowercase();
    let ext = a.name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    !a.is_inline && !["image/", "audio/", "video/", "text/calendar"].iter().any(|p| ty.starts_with(p)) && !MEDIA.contains(&ext.as_str())
}

/// A subject with no words at all, just codes and numbers ("SCW07598
/// 26833.26833.865"): invoice and order numbers come with a word ("Invoice
/// 2039"), and an empty subject doesn't count.
pub fn code_subject(subject: &str) -> bool {
    let mut s = subject.trim();
    loop {
        let lower = s.to_lowercase();
        match ["re:", "fw:", "fwd:", "aw:", "tr:"].iter().find(|p| lower.starts_with(**p)) {
            Some(p) => s = s[p.len()..].trim_start(),
            None => break,
        }
    }
    let mut digits = 0;
    for token in s.split_whitespace() {
        let t: String = token.chars().filter(|c| c.is_alphanumeric()).collect();
        let letters = t.chars().filter(|c| c.is_alphabetic()).count();
        if letters == t.chars().count() && (letters >= 3 || (letters == 2 && t.chars().any(char::is_lowercase))) {
            return false;
        }
        digits += t.chars().filter(char::is_ascii_digit).count();
    }
    digits >= 4
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::junk::analyze::Finding;

    #[test]
    fn subjects_of_only_codes_and_numbers() {
        for s in ["SCW07598 26833.26833.865", "26833.26833.865", "RE: 4471-AX 0093", "#0042 / 99812"] {
            assert!(code_subject(s), "{s}");
        }
        for s in ["", "Invoice 2039", "Your code 4821", "Hi 2024", "Q3 2025 numbers", "Fwd:", "AB 12", "Заказ 12345", "MTG 0930"] {
            assert!(!code_subject(s), "{s}");
        }
    }

    #[test]
    fn the_look_of_a_message_needs_a_warning_sign_or_the_whole_lure() {
        let clean = Analysis::default();
        let pdf = Attachment { name: "SCW07598.pdf".into(), content_type: "application/pdf".into(), ..Default::default() };
        let photo = Attachment { name: "20261102_183045.jpg".into(), content_type: "image/jpeg".into(), ..Default::default() };
        let lure = Message { subject: "SCW07598 26833.26833.865".into(), importance: Importance::High, ..Default::default() };
        let r = junk_reasons(&lure, "", std::slice::from_ref(&pdf), &clean, false);
        assert_eq!(r.len(), 3, "{r:?}");

        // Real mail from strangers that has part of the look.
        let reply = Message { subject: "RE: PO 4500123987".into(), ..Default::default() };
        let full = "Confirming we received the purchase order. The filters ship on Thursday and should arrive early next week.";
        assert!(junk_reasons(&reply, full, &[], &clean, false).is_empty(), "a code subject alone");
        let photos = Message { subject: "Fwd: 20261102_183045".into(), ..Default::default() };
        assert!(junk_reasons(&photos, "pics from saturday", std::slice::from_ref(&photo), &clean, false).is_empty(), "photos are not documents");
        let signed = Message { subject: "Signed contract".into(), importance: Importance::High, ..Default::default() };
        assert!(junk_reasons(&signed, "Here you go, signed.", std::slice::from_ref(&pdf), &clean, false).is_empty(), "urgent + bare document");
        let not_urgent = Message { subject: "8841-AX-2026".into(), ..Default::default() };
        assert!(junk_reasons(&not_urgent, "", std::slice::from_ref(&pdf), &clean, false).is_empty(), "two of three");
        assert!(junk_reasons(&lure, "", std::slice::from_ref(&photo), &clean, false).is_empty(), "a photo isn't the lure");

        // A Caution sign plus any part of the look moves it; either alone doesn't.
        let caution = Analysis { findings: vec![Finding::HtmlAttachment { name: "Remittance.html".into() }], ..Default::default() };
        assert!(!junk_reasons(&not_urgent, "", std::slice::from_ref(&pdf), &caution, false).is_empty());
        let plain = Message { subject: "Remittance advice".into(), ..Default::default() };
        assert!(junk_reasons(&plain, full, &[], &caution, false).is_empty(), "a Caution sign alone");

        assert!(!junk_reasons(&plain, "hello", &[], &clean, true).is_empty(), "the learned model is enough");
        let danger = Analysis { findings: vec![Finding::UnsafeScheme { scheme: "javascript".into() }], ..Default::default() };
        assert!(!junk_reasons(&plain, "hello", &[], &danger, false).is_empty());
    }
}
