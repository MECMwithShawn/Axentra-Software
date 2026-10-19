//! Conversation ids for mail that doesn't come with one (IMAP). Graph sends
//! `conversationId`; Gmail sends `X-GM-THRID`; everything else is threaded
//! from References / In-Reply-To, Outlook's Thread-Index, and finally the
//! subject with reply prefixes removed.

use base64::Engine as _;

/// The threading headers of one message.
#[derive(Clone, Debug, Default)]
pub struct ThreadInfo {
    /// Own Message-ID (no angle brackets).
    pub message_id: String,
    pub in_reply_to: Option<String>,
    /// References, oldest (thread root) first.
    pub references: Vec<String>,
    /// Raw `Thread-Index` header (base64).
    pub thread_index: Option<String>,
    pub subject: String,
    /// Gmail `X-GM-THRID`.
    pub gmail_thread_id: Option<u64>,
}

/// True when the subject starts with a reply / forward prefix.
pub fn has_reply_prefix(subject: &str) -> bool {
    let lower = subject.trim_start().to_ascii_lowercase();
    ["re:", "fw:", "fwd:", "aw:", "wg:", "sv:", "vs:", "antw:", "tr:", "re["].iter().any(|p| lower.starts_with(p))
}

/// The conversation key Outlook encodes in `Thread-Index`: the first 22
/// bytes (6-byte FILETIME + 16-byte GUID) of the base64 value.
pub fn thread_index_key(value: &str) -> Option<String> {
    let compact: String = value.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = base64::engine::general_purpose::STANDARD.decode(compact.as_bytes()).ok()?;
    if bytes.len() < 22 {
        return None;
    }
    Some(bytes[..22].iter().map(|b| format!("{b:02x}")).collect())
}

/// Derive the conversation id.
///
/// * `known` looks up the conversation of an already stored message by
///   Message-ID (any of the given ids).
/// * `by_subject` looks up a recent conversation with the same normalised
///   subject (only consulted for replies without references).
pub fn conversation_id(
    info: &ThreadInfo,
    mut known: impl FnMut(&[String]) -> Option<String>,
    mut by_subject: impl FnMut(&str) -> Option<String>,
) -> String {
    if let Some(thr) = info.gmail_thread_id {
        return format!("gm:{thr:x}");
    }
    // Newest reference first: the closest ancestor we already have.
    let mut ancestors: Vec<String> = info.references.iter().rev().cloned().collect();
    if let Some(irt) = &info.in_reply_to
        && !ancestors.contains(irt)
    {
        ancestors.insert(0, irt.clone());
    }
    if !ancestors.is_empty()
        && let Some(c) = known(&ancestors)
    {
        return c;
    }
    // A child of something we've seen before (it may arrive out of order).
    if !info.message_id.is_empty()
        && let Some(c) = known(std::slice::from_ref(&info.message_id))
    {
        return c;
    }
    if let Some(root) = info.references.first().or(info.in_reply_to.as_ref()) {
        return format!("mid:{root}");
    }
    if let Some(key) = info.thread_index.as_deref().and_then(thread_index_key) {
        return format!("ti:{key}");
    }
    if has_reply_prefix(&info.subject)
        && let Some(c) = by_subject(&info.subject)
    {
        return c;
    }
    if !info.message_id.is_empty() {
        return format!("mid:{}", info.message_id);
    }
    format!("local-{}", uuid::Uuid::new_v4())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn none(_: &[String]) -> Option<String> {
        None
    }
    fn no_subject(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn gmail_thread_id_wins() {
        let info = ThreadInfo { gmail_thread_id: Some(0x1a2b), message_id: "x@y".into(), ..Default::default() };
        assert_eq!(conversation_id(&info, none, no_subject), "gm:1a2b");
    }

    #[test]
    fn known_ancestor_then_root_reference() {
        let known: HashMap<&str, &str> = HashMap::from([("b@x", "conv-B")]);
        let lookup = |ids: &[String]| ids.iter().find_map(|i| known.get(i.as_str()).map(|s| s.to_string()));
        let reply = ThreadInfo {
            message_id: "c@x".into(),
            in_reply_to: Some("b@x".into()),
            references: vec!["a@x".into(), "b@x".into()],
            subject: "RE: Plan".into(),
            ..Default::default()
        };
        assert_eq!(conversation_id(&reply, lookup, no_subject), "conv-B");
        // Nothing stored yet: thread on the root reference.
        assert_eq!(conversation_id(&reply, none, no_subject), "mid:a@x");
        // Only In-Reply-To.
        let irt = ThreadInfo { message_id: "d@x".into(), in_reply_to: Some("a@x".into()), ..Default::default() };
        assert_eq!(conversation_id(&irt, none, no_subject), "mid:a@x");
        // The root itself gets the same id as its replies.
        let root = ThreadInfo { message_id: "a@x".into(), subject: "Plan".into(), ..Default::default() };
        assert_eq!(conversation_id(&root, none, no_subject), "mid:a@x");
    }

    #[test]
    fn thread_index_and_subject_fallbacks() {
        // 22 bytes of 0x01..0x16 plus a 5-byte child block.
        let raw: Vec<u8> = (1u8..=27).collect();
        let ti = base64::engine::general_purpose::STANDARD.encode(&raw);
        let key = thread_index_key(&ti).unwrap();
        assert_eq!(key.len(), 44);
        assert!(key.starts_with("0102"));
        let info = ThreadInfo { message_id: "m@x".into(), thread_index: Some(ti), ..Default::default() };
        assert_eq!(conversation_id(&info, none, no_subject), format!("ti:{key}"));
        assert_eq!(thread_index_key("AAA="), None);

        let reply = ThreadInfo { message_id: "r@x".into(), subject: "Re: Lunch?".into(), ..Default::default() };
        assert_eq!(conversation_id(&reply, none, |s| (s == "Re: Lunch?").then(|| "conv-L".to_string())), "conv-L");
        // Subject matching only applies to replies.
        let fresh = ThreadInfo { message_id: "f@x".into(), subject: "Lunch?".into(), ..Default::default() };
        assert_eq!(conversation_id(&fresh, none, |_| Some("conv-L".into())), "mid:f@x");
        assert!(conversation_id(&ThreadInfo::default(), none, no_subject).starts_with("local-"));
        assert!(has_reply_prefix("AW: Termin") && has_reply_prefix("Fwd: x") && !has_reply_prefix("Report"));
    }
}
