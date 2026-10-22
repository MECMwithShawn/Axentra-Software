//! Recipient text: splitting typed or pasted address lists (RFC 5322
//! `address-list`, simplified) and checking addresses.

use axentra_core::EmailAddress;

/// Split recipient text into entries. Entries end at `;`, `,` or a line
/// break outside quotes and angle brackets, and an entry of addresses
/// separated by spaces only ("a@x b@y", a column copied from a sheet) is
/// one entry each. With `all = false` the text after the last separator
/// is returned as still being typed; with `all = true` it becomes an entry
/// too. Group syntax (`Team: a@x, b@y;`) yields its members.
pub fn parse_list(text: &str, all: bool) -> (Vec<EmailAddress>, String) {
    let mut entries = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut angle = false;
    let mut escaped = false;
    for c in text.chars() {
        if escaped {
            current.push(c);
            escaped = false;
            continue;
        }
        match c {
            '\\' if quoted => {
                current.push(c);
                escaped = true;
            }
            '"' => {
                quoted = !quoted;
                current.push(c);
            }
            '<' if !quoted => {
                angle = true;
                current.push(c);
            }
            '>' if !quoted => {
                angle = false;
                current.push(c);
            }
            ':' if !quoted && !angle && !current.contains('@') => {
                // "Group name:" starts a group; its members follow.
                current.clear();
            }
            ';' | ',' | '\n' | '\r' if !quoted && !angle => {
                entries.extend(entries_in(&current));
                current.clear();
            }
            _ => current.push(c),
        }
    }
    if all {
        entries.extend(entries_in(&current));
        current.clear();
    }
    (entries, current.trim_start().to_string())
}

/// What the recipient box holds after an edit from `prev` to `now`:
/// entries ended by a separator become recipients; the rest stays typed.
/// A paste (several characters at once) also commits a complete final
/// address, so pasting "a@x.example; b@y.example" adds both.
pub fn split_typed(prev: &str, now: &str) -> (Vec<EmailAddress>, String) {
    let pasted = now.chars().count() > prev.chars().count() + 1;
    let (mut entries, rest) = parse_list(now, false);
    let last = entries_in(&rest);
    if pasted && !last.is_empty() && last.iter().all(|e| is_valid(&e.address)) {
        entries.extend(last);
        return (entries, String::new());
    }
    (entries, rest)
}

/// The entries in one entry's text: itself, or each address of a list
/// separated by spaces only (every word an address, no names).
fn entries_in(s: &str) -> Vec<EmailAddress> {
    let words: Vec<&str> = s.split_whitespace().collect();
    if words.len() > 1 && words.iter().all(|w| w.contains('@') && !w.contains(['<', '>', '"'])) {
        return words.into_iter().filter_map(parse_one).collect();
    }
    parse_one(s).into_iter().collect()
}

/// One entry: `Name <addr>`, `"Last, First" <addr>`, `<addr>`, `addr` or
/// a `mailto:` link. Invalid addresses are returned as typed (the UI shows
/// them as invalid chips).
pub fn parse_one(s: &str) -> Option<EmailAddress> {
    let s = s.trim().trim_matches(|c: char| c == ';' || c == ',').trim();
    if s.is_empty() {
        return None;
    }
    let (name, addr) = match (s.rfind('<'), s.rfind('>')) {
        (Some(open), Some(close)) if open < close => (unquote(&s[..open]), s[open + 1..close].trim().to_string()),
        _ => (String::new(), unquote(s)),
    };
    let addr = addr.strip_prefix("mailto:").or_else(|| addr.strip_prefix("MAILTO:")).unwrap_or(&addr).trim().to_string();
    if addr.is_empty() {
        return if name.is_empty() { None } else { Some(EmailAddress::new("", name)) };
    }
    // A display name that is just the address adds nothing.
    let name = if name.eq_ignore_ascii_case(&addr) { String::new() } else { name };
    Some(EmailAddress::new(name, addr))
}

fn unquote(s: &str) -> String {
    let t = s.trim();
    let t = t.strip_prefix('"').and_then(|t| t.strip_suffix('"')).unwrap_or(t);
    t.replace("\\\"", "\"").replace("\\\\", "\\").trim().to_string()
}

/// A deliverable-looking address: one `@`, a sensible local part and a
/// dotted domain (Unicode labels allowed; they go out as punycode).
pub fn is_valid(addr: &str) -> bool {
    let addr = addr.trim();
    let Some((local, domain)) = addr.rsplit_once('@') else { return false };
    if local.is_empty() || local.len() > 64 || domain.is_empty() || addr.len() > 254 {
        return false;
    }
    let bad = |c: char| c.is_whitespace() || c.is_control() || "<>()[]\\,;:\"@".contains(c);
    if local.chars().any(bad) || local.starts_with('.') || local.ends_with('.') || local.contains("..") {
        return false;
    }
    let labels: Vec<&str> = domain.split('.').collect();
    if labels.len() < 2 {
        return false;
    }
    let label_ok = |l: &&str| {
        !l.is_empty()
            && l.chars().count() <= 63
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.chars().all(|c| c.is_alphanumeric() || c == '-')
    };
    let tld = labels[labels.len() - 1];
    labels.iter().all(label_ok) && tld.chars().count() >= 2 && !tld.chars().all(|c| c.is_ascii_digit())
}

/// Text for editing a chip again: `Name <addr>` or just the address.
pub fn edit_text(a: &EmailAddress) -> String {
    if a.name.trim().is_empty() {
        a.address.clone()
    } else if a.name.contains([',', ';', '"', '<', '>']) {
        format!("\"{}\" <{}>", a.name.replace('"', "\\\""), a.address)
    } else {
        format!("{} <{}>", a.name, a.address)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addrs(list: &[EmailAddress]) -> Vec<(&str, &str)> {
        list.iter().map(|a| (a.name.as_str(), a.address.as_str())).collect()
    }

    #[test]
    fn splits_pasted_lists() {
        let (list, rest) = parse_list("Priya Raman <priya@northwind.example>; marcus@axentra.example, \"Bowen, Megan\" <megan@contoso.example>", true);
        assert_eq!(
            addrs(&list),
            vec![("Priya Raman", "priya@northwind.example"), ("", "marcus@axentra.example"), ("Bowen, Megan", "megan@contoso.example")]
        );
        assert_eq!(rest, "");
    }

    /// A column of addresses copied from a sheet (lines, tabs or spaces):
    /// one recipient each, not one merged address.
    #[test]
    fn splits_lists_separated_by_spaces_or_lines() {
        let (list, rest) = parse_list("a1@example.org\na2@example.org\r\na3@example.org", true);
        assert_eq!(addrs(&list), vec![("", "a1@example.org"), ("", "a2@example.org"), ("", "a3@example.org")]);
        assert_eq!(rest, "");
        let (list, _) = parse_list("a1@example.org a2@example.org\ta3@example.org", true);
        assert_eq!(list.len(), 3);
        // Names keep their spaces; a word that isn't an address keeps the entry whole.
        let (list, _) = parse_list("Priya Raman <priya@northwind.example>", true);
        assert_eq!(addrs(&list), vec![("Priya Raman", "priya@northwind.example")]);
        let (list, _) = parse_list("dan eve@example.org", true);
        assert_eq!(addrs(&list), vec![("", "dan eve@example.org")]);
        // Pasted (WebKit puts spaces for the lines in a text box): every one a chip.
        let (list, rest) = split_typed("", "a1@example.org a2@example.org a3@example.org");
        assert_eq!((list.len(), rest.as_str()), (3, ""));
        // Typed, the last one waits for Enter as before; an invalid one in a paste stays to fix.
        let (list, rest) = split_typed("a1@example.org a2@example.or", "a1@example.org a2@example.org");
        assert_eq!((list.len(), rest.as_str()), (0, "a1@example.org a2@example.org"));
        let (list, rest) = split_typed("", "a1@example.org a2@");
        assert_eq!((list.len(), rest.as_str()), (0, "a1@example.org a2@"));
    }

    #[test]
    fn keeps_unfinished_text() {
        let (list, rest) = parse_list("a@x.example; Pri", false);
        assert_eq!(addrs(&list), vec![("", "a@x.example")]);
        assert_eq!(rest, "Pri");
        // A comma inside quotes does not end the entry.
        let (list, rest) = parse_list("\"Bowen,", false);
        assert!(list.is_empty());
        assert_eq!(rest, "\"Bowen,");
    }

    #[test]
    fn paste_commits_the_last_address_typing_does_not() {
        // Typing one character at a time: the address stays in the box.
        let (list, rest) = split_typed("a@x.exampl", "a@x.example");
        assert!(list.is_empty());
        assert_eq!(rest, "a@x.example");
        // Pasting a list: every address becomes a chip.
        let (list, rest) = split_typed("", "Priya Raman <priya@northwind.example>; c@d.example");
        assert_eq!(addrs(&list), vec![("Priya Raman", "priya@northwind.example"), ("", "c@d.example")]);
        assert_eq!(rest, "");
        // A pasted fragment that is not an address yet stays as text.
        let (list, rest) = split_typed("", "a@x.example; Pri");
        assert_eq!(addrs(&list), vec![("", "a@x.example")]);
        assert_eq!(rest, "Pri");
    }

    #[test]
    fn group_syntax_and_mailto() {
        let (list, _) = parse_list("Team: a@x.example, b@y.example;", true);
        assert_eq!(addrs(&list), vec![("", "a@x.example"), ("", "b@y.example")]);
        assert_eq!(parse_one("mailto:c@z.example").unwrap().address, "c@z.example");
        assert_eq!(parse_one("<d@z.example>").unwrap().address, "d@z.example");
        assert_eq!(parse_one("e@z.example <e@z.example>").unwrap().name, "");
        assert!(parse_one("  ; ").is_none());
    }

    #[test]
    fn validates_addresses() {
        for ok in ["a@b.co", "first.last+tag@sub.example.org", "josé@exämple.de", "x_y@a-b.example"] {
            assert!(is_valid(ok), "{ok}");
        }
        for bad in ["", "plain", "a@b", "a@@b.co", "a b@c.co", ".a@b.co", "a..b@c.co", "a@-b.co", "a@b.c", "a@b.123", "a@b..co"] {
            assert!(!is_valid(bad), "{bad}");
        }
    }

    #[test]
    fn edit_text_quotes_when_needed() {
        assert_eq!(edit_text(&EmailAddress::new("Priya Raman", "p@x.example")), "Priya Raman <p@x.example>");
        assert_eq!(edit_text(&EmailAddress::new("Bowen, Megan", "m@x.example")), "\"Bowen, Megan\" <m@x.example>");
        assert_eq!(edit_text(&EmailAddress::new("", "m@x.example")), "m@x.example");
        let round = parse_one(&edit_text(&EmailAddress::new("Bowen, Megan", "m@x.example"))).unwrap();
        assert_eq!((round.name.as_str(), round.address.as_str()), ("Bowen, Megan", "m@x.example"));
    }
}
