//! RFC 5322 / MIME: parsing fetched mail into the model and building
//! outgoing mail from drafts.

pub mod build;
pub mod parse;
pub mod thread;

/// Strip surrounding angle brackets and whitespace from a Message-ID.
pub fn bare_id(id: &str) -> String {
    id.trim().trim_start_matches('<').trim_end_matches('>').trim().to_string()
}

/// All `<...>` ids in a References / In-Reply-To header value, in order.
pub fn split_ids(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = value;
    while let Some(start) = rest.find('<') {
        let Some(end) = rest[start..].find('>') else { break };
        let id = rest[start + 1..start + end].trim();
        if !id.is_empty() {
            out.push(id.to_string());
        }
        rest = &rest[start + end + 1..];
    }
    if out.is_empty() {
        out.extend(value.split_whitespace().map(bare_id).filter(|s| !s.is_empty()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids() {
        assert_eq!(bare_id(" <a@b> "), "a@b");
        assert_eq!(split_ids("<a@x> <b@y>\r\n <c@z>"), vec!["a@x", "b@y", "c@z"]);
        assert_eq!(split_ids("a@x"), vec!["a@x"]);
    }
}
