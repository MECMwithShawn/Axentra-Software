//! vCard reading and writing: 4.0 (RFC 6350) and 3.0 (RFC 2426), plus the
//! vCard 2.1 basics (bare TYPE parameters, QUOTED-PRINTABLE) that old
//! exports still contain. Contact lists map to `KIND:group` + `MEMBER` in
//! 4.0 and to Apple's `X-ADDRESSBOOKSERVER-KIND` convention in 3.0
//! (docs/spec/people-todo.md 2.15).

use std::collections::HashMap;

use chrono::{DateTime, Datelike, NaiveDate, Utc};

use super::text_codec::{base64_decode, decode_bytes};
use crate::model::*;

/// Year stored for a birthday entered without one (Apple's
/// `X-APPLE-OMIT-YEAR=1604` convention, see 2.8 of the spec).
pub(crate) const NO_YEAR: i32 = 1604;

/// One content line: `[group.]NAME;PARAM=a,b:value`.
#[derive(Debug)]
struct Prop {
    name: String,
    /// Upper-cased parameter names with their values (quotes removed).
    params: Vec<(String, Vec<String>)>,
    value: String,
}

impl Prop {
    fn param(&self, name: &str) -> Option<&str> {
        self.params.iter().find(|(k, _)| k == name).and_then(|(_, v)| v.first()).map(String::as_str)
    }

    /// Lower-cased TYPE values, including vCard 2.1 bare parameters (`TEL;CELL:`).
    fn types(&self) -> Vec<String> {
        self.params
            .iter()
            .filter(|(k, _)| k == "TYPE")
            .flat_map(|(_, v)| v.iter().map(|t| t.to_ascii_lowercase()))
            .collect()
    }

    fn has_type(&self, t: &str) -> bool {
        self.types().iter().any(|x| x == t)
    }

    /// The value with transfer encodings undone (QUOTED-PRINTABLE / CHARSET),
    /// but vCard text escapes still in place.
    fn raw_value(&self) -> String {
        let qp = self.param("ENCODING").is_some_and(|e| e.eq_ignore_ascii_case("QUOTED-PRINTABLE"));
        if !qp {
            return self.value.clone();
        }
        let bytes = quoted_printable_decode(&self.value);
        let charset = self.param("CHARSET").unwrap_or("UTF-8");
        if charset.eq_ignore_ascii_case("UTF-8") {
            decode_bytes(&bytes).0
        } else {
            bytes.iter().map(|&b| super::text_codec::cp1252_char(b)).collect()
        }
    }

    /// Unescaped single text value.
    fn text(&self) -> String {
        unescape(&self.raw_value())
    }

    /// Unescaped components of a structured value (`N`, `ADR`, `ORG`).
    fn components(&self) -> Vec<String> {
        split_escaped(&self.raw_value(), ';').iter().map(|c| unescape(c)).collect()
    }

    /// Unescaped items of a comma-separated list (`CATEGORIES`, `NICKNAME`).
    fn list(&self) -> Vec<String> {
        split_escaped(&self.raw_value(), ',').iter().map(|c| unescape(c).trim().to_string()).filter(|s| !s.is_empty()).collect()
    }
}

// ---------------------------------------------------------------- reading

/// Parse every `BEGIN:VCARD` ... `END:VCARD` block in `text`. Group cards
/// become contact lists; their `MEMBER`s are resolved against the cards in
/// the same file (by `UID`) when they are `urn:uuid:` references.
pub(crate) fn parse(text: &str) -> (Vec<Contact>, Vec<ContactList>) {
    let mut cards: Vec<Vec<Prop>> = Vec::new();
    let mut current: Option<Vec<Prop>> = None;
    for line in unfold(text) {
        let Some(prop) = parse_line(&line) else { continue };
        match prop.name.as_str() {
            "BEGIN" if prop.value.trim().eq_ignore_ascii_case("VCARD") => current = Some(Vec::new()),
            "END" if prop.value.trim().eq_ignore_ascii_case("VCARD") => {
                if let Some(card) = current.take() {
                    cards.push(card);
                }
            }
            _ => {
                if let Some(card) = current.as_mut() {
                    card.push(prop);
                }
            }
        }
    }

    let mut contacts = Vec::new();
    let mut uid_to_address: HashMap<String, EmailAddress> = HashMap::new();
    let mut groups: Vec<&Vec<Prop>> = Vec::new();
    for card in &cards {
        if is_group(card) {
            groups.push(card);
            continue;
        }
        let contact = contact_from_props(card);
        if let (Some(uid), Some(email)) = (find(card, "UID"), contact.primary_email()) {
            uid_to_address.insert(normalise_uid(&uid.text()), EmailAddress::new(contact.display_name.clone(), email));
        }
        if !contact.display_name.is_empty() || contact.primary_email().is_some() || !contact.phones.is_empty() {
            contacts.push(contact);
        }
    }
    let mut lists = Vec::new();
    for card in groups {
        let name = find(card, "FN").map(|p| p.text()).filter(|s| !s.trim().is_empty()).or_else(|| {
            find(card, "N").and_then(|p| p.components().into_iter().find(|c| !c.trim().is_empty()))
        });
        let Some(name) = name else { continue };
        let mut list = ContactList {
            name: name.trim().to_string(),
            description: find(card, "NOTE").map(|p| p.text()).unwrap_or_default(),
            is_favorite: is_favorite(card),
            ..Default::default()
        };
        for p in card.iter().filter(|p| p.name == "MEMBER" || p.name == "X-ADDRESSBOOKSERVER-MEMBER") {
            let v = p.text();
            let v = v.trim();
            let member = if let Some(addr) = strip_prefix_ci(v, "mailto:") {
                let addr = addr.split('?').next().unwrap_or(addr).trim();
                uid_to_address
                    .values()
                    .find(|a| a.address.eq_ignore_ascii_case(addr))
                    .cloned()
                    .unwrap_or_else(|| EmailAddress::new(p.param("CN").unwrap_or_default(), addr))
            } else if let Some(found) = uid_to_address.get(&normalise_uid(v)) {
                found.clone()
            } else {
                continue;
            };
            if !member.address.is_empty() && !list.members.iter().any(|m| m.address.eq_ignore_ascii_case(&member.address)) {
                list.members.push(member);
            }
        }
        lists.push(list);
    }
    (contacts, lists)
}

fn find<'a>(card: &'a [Prop], name: &str) -> Option<&'a Prop> {
    card.iter().find(|p| p.name == name)
}

fn is_group(card: &[Prop]) -> bool {
    card.iter().any(|p| {
        (p.name == "KIND" || p.name == "X-ADDRESSBOOKSERVER-KIND") && p.value.trim().eq_ignore_ascii_case("group")
    })
}

fn is_favorite(card: &[Prop]) -> bool {
    find(card, "X-AXENTRA-FAVORITE").is_some_and(|p| matches!(p.value.trim(), "1" | "TRUE" | "true"))
}

fn normalise_uid(uid: &str) -> String {
    let uid = uid.trim();
    strip_prefix_ci(uid, "urn:uuid:").unwrap_or(uid).to_ascii_lowercase()
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    (s.len() >= prefix.len() && s.is_char_boundary(prefix.len()) && s[..prefix.len()].eq_ignore_ascii_case(prefix))
        .then(|| &s[prefix.len()..])
}

fn contact_from_props(card: &[Prop]) -> Contact {
    let mut c = Contact::default();
    for p in card {
        match p.name.as_str() {
            "FN" => c.display_name = p.text().trim().to_string(),
            "N" => {
                let parts = p.components();
                let get = |i: usize| parts.get(i).map(|s| s.trim().to_string()).unwrap_or_default();
                c.surname = get(0);
                c.given_name = get(1);
                c.middle_name = get(2);
                c.more.title = get(3);
                c.more.suffix = get(4);
            }
            "NICKNAME" => c.nickname = p.list().join(", "),
            "EMAIL" => {
                let value = p.text();
                let value = strip_prefix_ci(value.trim(), "mailto:").unwrap_or(value.trim()).to_string();
                if !value.is_empty() && c.emails.len() < 3 {
                    let label = if p.has_type("work") {
                        "Work"
                    } else if p.has_type("home") {
                        "Personal"
                    } else {
                        "Other"
                    };
                    let item = LabeledValue { label: label.into(), value };
                    if p.has_type("pref") || p.param("PREF").is_some_and(|v| v.trim() == "1") {
                        c.emails.insert(0, item);
                    } else {
                        c.emails.push(item);
                    }
                }
            }
            "TEL" => {
                let value = p.text();
                let value = strip_prefix_ci(value.trim(), "tel:").unwrap_or(value.trim()).to_string();
                if !value.is_empty() {
                    let label = if p.has_type("cell") || p.has_type("mobile") || p.has_type("iphone") {
                        "Mobile"
                    } else if p.has_type("home") && !p.has_type("work") {
                        "Home"
                    } else {
                        "Work"
                    };
                    c.phones.push(LabeledValue { label: label.into(), value });
                }
            }
            "ADR" => {
                let parts = p.components();
                let get = |i: usize| parts.get(i).map(|s| s.trim().to_string()).unwrap_or_default();
                // pobox; extended; street; locality; region; code; country
                let street = [get(0), get(1), get(2)].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n");
                let label = if p.has_type("home") {
                    "Home"
                } else if p.has_type("work") {
                    "Work"
                } else {
                    "Other"
                };
                let addr = PostalAddress {
                    label: label.into(),
                    street,
                    city: get(3),
                    state: get(4),
                    postal_code: get(5),
                    country: get(6),
                };
                let empty = addr.street.is_empty() && addr.city.is_empty() && addr.state.is_empty() && addr.postal_code.is_empty() && addr.country.is_empty();
                if !empty && !c.addresses.iter().any(|a| a.label == addr.label) {
                    c.addresses.push(addr);
                }
            }
            "BDAY" => {
                let omit_year = p.param("X-APPLE-OMIT-YEAR").and_then(|y| y.trim().parse::<i32>().ok());
                c.birthday = parse_date(&p.text(), omit_year);
            }
            "ORG" => {
                let parts = p.components();
                c.company = parts.first().map(|s| s.trim().to_string()).unwrap_or_default();
                c.department = parts.get(1).map(|s| s.trim().to_string()).unwrap_or_default();
            }
            "TITLE" => c.job_title = p.text().trim().to_string(),
            "ROLE" => c.more.profession = p.text().trim().to_string(),
            "NOTE" => c.notes = p.text(),
            "URL" => {
                let url = p.text().trim().to_string();
                if !url.is_empty() {
                    c.websites.push(url);
                }
            }
            "IMPP" | "X-JABBER" | "X-SKYPE" | "X-MS-IMADDRESS" => {
                let v = p.text().trim().to_string();
                if !v.is_empty() {
                    let v = match p.name.as_str() {
                        "X-JABBER" if !v.contains(':') => format!("xmpp:{v}"),
                        "X-SKYPE" if !v.contains(':') => format!("skype:{v}"),
                        _ => v,
                    };
                    c.im_addresses.push(v);
                }
            }
            "RELATED" => {
                let v = p.text().trim().to_string();
                if p.has_type("spouse") {
                    c.more.spouse = v;
                } else if p.has_type("child") {
                    if !c.more.children.is_empty() {
                        c.more.children.push_str(", ");
                    }
                    c.more.children.push_str(&v);
                }
            }
            "X-SPOUSE" | "X-MS-SPOUSE" => c.more.spouse = p.param("NAME").map(str::to_string).unwrap_or_else(|| p.text()).trim().to_string(),
            "X-AXENTRA-CHILDREN" | "X-MS-CHILD" => c.more.children = p.text().trim().to_string(),
            "X-MS-MANAGER" | "X-MANAGER" => c.more.manager = p.text().trim().to_string(),
            "X-MS-ASSISTANT" | "X-ASSISTANT" => c.more.assistant = p.text().trim().to_string(),
            "X-AXENTRA-OFFICE" | "X-OFFICE" => c.office_location = p.text().trim().to_string(),
            "X-AXENTRA-FILE-AS" => c.more.file_as = p.text().trim().to_string(),
            "X-PHONETIC-FIRST-NAME" => c.more.phonetic_given_name = p.text().trim().to_string(),
            "X-PHONETIC-LAST-NAME" => c.more.phonetic_surname = p.text().trim().to_string(),
            "X-PHONETIC-ORG" => c.more.phonetic_company = p.text().trim().to_string(),
            "CATEGORIES" => {
                for cat in p.list() {
                    if !c.categories.contains(&cat) {
                        c.categories.push(cat);
                    }
                }
            }
            "X-AXENTRA-FAVORITE" => c.is_favorite = matches!(p.value.trim(), "1" | "TRUE" | "true"),
            "PHOTO" => c.photo = parse_photo(p),
            "REV" => {
                if let Some(t) = parse_timestamp(&p.text()) {
                    c.updated_at = t;
                }
            }
            _ => {}
        }
    }
    if c.display_name.is_empty() {
        c.display_name = [c.given_name.as_str(), c.surname.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" ");
    }
    c
}

/// `19901023`, `1990-10-23`, `--1023`, `--10-23`, `1990-10-23T00:00:00Z`.
/// A year equal to `omit_year` (or [`NO_YEAR`]) means "no year".
fn parse_date(s: &str, omit_year: Option<i32>) -> Option<NaiveDate> {
    let s = s.trim();
    let date = s.split('T').next().unwrap_or(s);
    if let Some(rest) = date.strip_prefix("--") {
        let digits: String = rest.chars().filter(|c| c.is_ascii_digit()).collect();
        if digits.len() == 4 {
            let m = digits[..2].parse().ok()?;
            let d = digits[2..].parse().ok()?;
            return NaiveDate::from_ymd_opt(NO_YEAR, m, d);
        }
        return None;
    }
    let digits: String = date.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() != 8 {
        return None;
    }
    let y: i32 = digits[..4].parse().ok()?;
    let m = digits[4..6].parse().ok()?;
    let d = digits[6..].parse().ok()?;
    let y = if Some(y) == omit_year { NO_YEAR } else { y };
    NaiveDate::from_ymd_opt(y, m, d)
}

fn parse_timestamp(s: &str) -> Option<DateTime<Utc>> {
    let s = s.trim();
    if let Ok(t) = DateTime::parse_from_rfc3339(s) {
        return Some(t.with_timezone(&Utc));
    }
    chrono::NaiveDateTime::parse_from_str(s.trim_end_matches('Z'), "%Y%m%dT%H%M%S").ok().map(|n| n.and_utc())
}

fn parse_photo(p: &Prop) -> Option<String> {
    let value = p.value.trim();
    if strip_prefix_ci(value, "data:").is_some() {
        return value.contains(";base64,").then(|| value.to_string());
    }
    let encoding = p.param("ENCODING").unwrap_or_default();
    if !(encoding.eq_ignore_ascii_case("b") || encoding.eq_ignore_ascii_case("BASE64")) {
        // Remote URIs are not fetched (no network access from the address book).
        return None;
    }
    let cleaned: String = value.chars().filter(|c| !c.is_whitespace()).collect();
    base64_decode(&cleaned)?;
    let kind = p.types().into_iter().find(|t| matches!(t.as_str(), "jpeg" | "jpg" | "png" | "gif" | "webp")).unwrap_or_else(|| "jpeg".into());
    let mime = match kind.as_str() {
        "jpg" => "jpeg".to_string(),
        other => other.to_string(),
    };
    Some(format!("data:image/{mime};base64,{cleaned}"))
}

/// Join folded lines (RFC 6350 3.2) and vCard 2.1 quoted-printable soft breaks.
fn unfold(text: &str) -> Vec<String> {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines: Vec<String> = Vec::new();
    let mut qp_soft_break = false;
    for raw in text.split('\n') {
        match lines.last_mut() {
            Some(last) if qp_soft_break => {
                last.pop();
                last.push_str(raw);
            }
            Some(last) if raw.starts_with(' ') || raw.starts_with('\t') => last.push_str(&raw[1..]),
            _ => lines.push(raw.to_string()),
        }
        qp_soft_break = lines.last().is_some_and(|l| {
            l.ends_with('=') && l.split(':').next().is_some_and(|head| head.to_ascii_uppercase().contains("QUOTED-PRINTABLE"))
        });
    }
    lines.retain(|l| !l.trim().is_empty());
    lines
}

fn parse_line(line: &str) -> Option<Prop> {
    let mut in_quotes = false;
    let mut colon = None;
    for (i, ch) in line.char_indices() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ':' if !in_quotes => {
                colon = Some(i);
                break;
            }
            _ => {}
        }
    }
    let colon = colon?;
    let head = &line[..colon];
    let value = line[colon + 1..].to_string();
    let mut parts = split_unquoted(head, ';').into_iter();
    let name = parts.next()?.trim().to_string();
    // Drop the optional group prefix ("item1.EMAIL").
    let name = name.rsplit('.').next().unwrap_or(&name).to_ascii_uppercase();
    if name.is_empty() {
        return None;
    }
    let mut params: Vec<(String, Vec<String>)> = Vec::new();
    for part in parts {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (key, raw) = match part.split_once('=') {
            Some((k, v)) => (k.trim().to_ascii_uppercase(), v),
            // vCard 2.1 bare parameters: TEL;CELL;PREF / PHOTO;BASE64.
            None => {
                let upper = part.to_ascii_uppercase();
                match upper.as_str() {
                    "QUOTED-PRINTABLE" | "BASE64" | "B" => ("ENCODING".to_string(), part),
                    _ => ("TYPE".to_string(), part),
                }
            }
        };
        let values: Vec<String> = split_unquoted(raw, ',')
            .into_iter()
            .map(|v| v.trim().trim_matches('"').to_string())
            .flat_map(|v| v.split(',').map(|s| s.trim().to_string()).collect::<Vec<_>>())
            .filter(|v| !v.is_empty())
            .collect();
        match params.iter_mut().find(|(k, _)| *k == key) {
            Some((_, existing)) => existing.extend(values),
            None => params.push((key, values)),
        }
    }
    Some(Prop { name, params, value })
}

fn split_unquoted(s: &str, sep: char) -> Vec<&str> {
    let mut out = Vec::new();
    let mut in_quotes = false;
    let mut start = 0;
    for (i, ch) in s.char_indices() {
        if ch == '"' {
            in_quotes = !in_quotes;
        } else if ch == sep && !in_quotes {
            out.push(&s[start..i]);
            start = i + ch.len_utf8();
        }
    }
    out.push(&s[start..]);
    out
}

/// Split on `sep` where it is not escaped with a backslash. Escapes are kept
/// so each piece can be unescaped afterwards.
fn split_escaped(s: &str, sep: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = s.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            cur.push(ch);
            if let Some(next) = chars.next() {
                cur.push(next);
            }
        } else if ch == sep {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(ch);
        }
    }
    out.push(cur);
    out
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('n') | Some('N') => out.push('\n'),
                Some(other) => out.push(other),
                None => out.push('\\'),
            }
        } else {
            out.push(ch);
        }
    }
    out
}

fn quoted_printable_decode(s: &str) -> Vec<u8> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'=' {
            let hex = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
            if let Some(b) = hex {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

// ---------------------------------------------------------------- writing

/// Serialise contacts and contact lists to a `.vcf` document (CRLF line
/// endings, lines folded at 75 octets).
pub(crate) fn serialize(contacts: &[Contact], lists: &[ContactList], v4: bool) -> String {
    let mut out = String::new();
    for c in contacts {
        write_contact(&mut out, c, v4);
    }
    for l in lists {
        write_list(&mut out, l, v4);
    }
    out
}

fn write_contact(out: &mut String, c: &Contact, v4: bool) {
    let mut w = CardWriter::new(out, v4);
    let fn_value = if c.display_name.trim().is_empty() {
        [c.given_name.as_str(), c.surname.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" ")
    } else {
        c.display_name.clone()
    };
    w.line("FN", &escape(&fn_value));
    w.line(
        "N",
        &[&c.surname, &c.given_name, &c.middle_name, &c.more.title, &c.more.suffix].iter().map(|s| escape(s)).collect::<Vec<_>>().join(";"),
    );
    if !c.nickname.is_empty() {
        w.line("NICKNAME", &escape(&c.nickname));
    }
    for (i, e) in c.emails.iter().enumerate() {
        let t = match e.label.as_str() {
            "Work" => Some("work"),
            "Personal" | "Home" => Some("home"),
            _ => None,
        };
        let mut params = String::new();
        if let Some(t) = t {
            params.push_str(&format!(";TYPE={t}"));
        }
        if i == 0 && c.emails.len() > 1 {
            params.push_str(if v4 { ";PREF=1" } else { ";TYPE=pref" });
        }
        if !v4 {
            params.push_str(";TYPE=internet");
        }
        w.line(&format!("EMAIL{params}"), &escape(&e.value));
    }
    for p in &c.phones {
        let t = match p.label.as_str() {
            "Mobile" => "cell",
            "Home" => "home",
            _ => "work",
        };
        let t = if v4 { format!("{t},voice") } else { format!("{t};TYPE=voice") };
        w.line(&format!("TEL;TYPE={t}"), &escape(&p.value));
    }
    for a in &c.addresses {
        let t = match a.label.as_str() {
            "Home" => ";TYPE=home",
            "Work" => ";TYPE=work",
            _ => "",
        };
        let parts = ["", "", a.street.as_str(), a.city.as_str(), a.state.as_str(), a.postal_code.as_str(), a.country.as_str()];
        w.line(&format!("ADR{t}"), &parts.iter().map(|s| escape(s)).collect::<Vec<_>>().join(";"));
    }
    if let Some(b) = c.birthday {
        if b.year() == NO_YEAR {
            if v4 {
                w.line("BDAY", &format!("--{:02}{:02}", b.month(), b.day()));
            } else {
                w.line(&format!("BDAY;X-APPLE-OMIT-YEAR={NO_YEAR}"), &format!("{NO_YEAR}-{:02}-{:02}", b.month(), b.day()));
            }
        } else if v4 {
            w.line("BDAY", &b.format("%Y%m%d").to_string());
        } else {
            w.line("BDAY", &b.format("%Y-%m-%d").to_string());
        }
    }
    if !c.company.is_empty() || !c.department.is_empty() {
        let org = if c.department.is_empty() { escape(&c.company) } else { format!("{};{}", escape(&c.company), escape(&c.department)) };
        w.line("ORG", &org);
    }
    if !c.job_title.is_empty() {
        w.line("TITLE", &escape(&c.job_title));
    }
    if !c.more.profession.is_empty() {
        w.line("ROLE", &escape(&c.more.profession));
    }
    if !c.notes.is_empty() {
        w.line("NOTE", &escape(&c.notes));
    }
    for url in &c.websites {
        w.line("URL", url);
    }
    for im in &c.im_addresses {
        w.line("IMPP", im);
    }
    if !c.more.spouse.is_empty() {
        if v4 {
            w.line("RELATED;TYPE=spouse;VALUE=text", &escape(&c.more.spouse));
        } else {
            w.line("X-SPOUSE", &escape(&c.more.spouse));
        }
    }
    if !c.more.children.is_empty() {
        if v4 {
            for child in c.more.children.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                w.line("RELATED;TYPE=child;VALUE=text", &escape(child));
            }
        } else {
            w.line("X-AXENTRA-CHILDREN", &escape(&c.more.children));
        }
    }
    if !c.more.manager.is_empty() {
        w.line("X-MS-MANAGER", &escape(&c.more.manager));
    }
    if !c.more.assistant.is_empty() {
        w.line("X-MS-ASSISTANT", &escape(&c.more.assistant));
    }
    if !c.office_location.is_empty() {
        w.line("X-AXENTRA-OFFICE", &escape(&c.office_location));
    }
    if !c.more.file_as.is_empty() {
        w.line("X-AXENTRA-FILE-AS", &escape(&c.more.file_as));
    }
    for (prop, v) in [
        ("X-PHONETIC-FIRST-NAME", &c.more.phonetic_given_name),
        ("X-PHONETIC-LAST-NAME", &c.more.phonetic_surname),
        ("X-PHONETIC-ORG", &c.more.phonetic_company),
    ] {
        if !v.is_empty() {
            w.line(prop, &escape(v));
        }
    }
    if !c.categories.is_empty() {
        w.line("CATEGORIES", &c.categories.iter().map(|s| escape(s)).collect::<Vec<_>>().join(","));
    }
    if c.is_favorite {
        w.line("X-AXENTRA-FAVORITE", "1");
    }
    if let Some(photo) = c.photo.as_deref().filter(|p| p.starts_with("data:image/")) {
        if v4 {
            w.line("PHOTO", photo);
        } else if let Some((meta, data)) = photo.split_once(";base64,") {
            let kind = meta.trim_start_matches("data:image/").to_ascii_uppercase();
            w.line(&format!("PHOTO;ENCODING=b;TYPE={kind}"), data);
        }
    }
    if c.updated_at.timestamp() > 0 {
        w.line("REV", &c.updated_at.format("%Y%m%dT%H%M%SZ").to_string());
    }
    w.finish();
}

fn write_list(out: &mut String, l: &ContactList, v4: bool) {
    let mut w = CardWriter::new(out, v4);
    if v4 {
        w.line("KIND", "group");
    } else {
        w.line("X-ADDRESSBOOKSERVER-KIND", "group");
    }
    w.line("FN", &escape(&l.name));
    w.line("N", &format!("{};;;;", escape(&l.name)));
    if !l.description.is_empty() {
        w.line("NOTE", &escape(&l.description));
    }
    for m in &l.members {
        let prop = if v4 { "MEMBER" } else { "X-ADDRESSBOOKSERVER-MEMBER" };
        w.line(prop, &format!("mailto:{}", m.address));
    }
    if l.is_favorite {
        w.line("X-AXENTRA-FAVORITE", "1");
    }
    w.finish();
}

struct CardWriter<'a> {
    out: &'a mut String,
}

impl<'a> CardWriter<'a> {
    fn new(out: &'a mut String, v4: bool) -> Self {
        let mut w = Self { out };
        w.line("BEGIN", "VCARD");
        w.line("VERSION", if v4 { "4.0" } else { "3.0" });
        let uid = uuid::Uuid::new_v4();
        w.line("UID", &if v4 { format!("urn:uuid:{uid}") } else { uid.to_string() });
        w
    }

    fn line(&mut self, name: &str, value: &str) {
        fold_into(self.out, &format!("{name}:{value}"));
    }

    fn finish(self) {
        fold_into(self.out, "END:VCARD");
    }
}

/// Append `line` folded at 75 octets without splitting a UTF-8 sequence.
fn fold_into(out: &mut String, line: &str) {
    let mut width = 0;
    for ch in line.chars() {
        let len = ch.len_utf8();
        if width + len > 75 {
            out.push_str("\r\n ");
            width = 1;
        }
        out.push(ch);
        width += len;
    }
    out.push_str("\r\n");
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            ',' => out.push_str("\\,"),
            ';' => out.push_str("\\;"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn parses_vcard_4_with_folding_escapes_and_types() {
        let text = "BEGIN:VCARD\r\nVERSION:4.0\r\nUID:urn:uuid:4fbe8971-0bc3-424c-9c26-36c3e1eff6b1\r\nFN:Ada Lovelace\r\n\
N:Lovelace;Ada;Augusta;Countess;\r\nNICKNAME:Ada,The Enchantress\r\nEMAIL;TYPE=work:ada@analytical.example\r\n\
EMAIL;TYPE=home;PREF=1:ada@home.example\r\nTEL;VALUE=uri;TYPE=\"cell,voice\":tel:+44-20-7946-0000\r\n\
TEL;TYPE=home:020 7946 0001\r\nADR;TYPE=work:;;12 St James\\'s Square;London;;SW1Y 4JH;United Kingdom\r\n\
BDAY:18151210\r\nORG:Analytical Engines Ltd;Research\r\nTITLE:Mathematician\r\nROLE:Programmer\r\n\
NOTE:First line\\nSecond\\, with comma\\; and semicolon\r\nURL:https://ada.example/notes\r\n\
CATEGORIES:VIP,Research\r\nRELATED;TYPE=spouse;VALUE=text:William King\r\nIMPP:xmpp:ada@chat.example\r\n\
NOTE;LANGUAGE=en:This is a long note that is folded across\r\n  two lines by the writer\r\nEND:VCARD\r\n";
        let (contacts, lists) = parse(text);
        assert!(lists.is_empty());
        assert_eq!(contacts.len(), 1);
        let c = &contacts[0];
        assert_eq!(c.display_name, "Ada Lovelace");
        assert_eq!((c.given_name.as_str(), c.surname.as_str(), c.middle_name.as_str(), c.more.title.as_str()), ("Ada", "Lovelace", "Augusta", "Countess"));
        assert_eq!(c.nickname, "Ada, The Enchantress");
        // PREF=1 moves the home address to the front.
        assert_eq!(c.emails[0], LabeledValue { label: "Personal".into(), value: "ada@home.example".into() });
        assert_eq!(c.emails[1].label, "Work");
        assert_eq!(c.phones[0], LabeledValue { label: "Mobile".into(), value: "+44-20-7946-0000".into() });
        assert_eq!(c.phones[1].label, "Home");
        assert_eq!(c.addresses[0].street, "12 St James's Square");
        assert_eq!(c.addresses[0].city, "London");
        assert_eq!(c.addresses[0].postal_code, "SW1Y 4JH");
        assert_eq!(c.addresses[0].label, "Work");
        assert_eq!(c.birthday, Some(ymd(1815, 12, 10)));
        assert_eq!((c.company.as_str(), c.department.as_str()), ("Analytical Engines Ltd", "Research"));
        assert_eq!(c.job_title, "Mathematician");
        assert_eq!(c.more.profession, "Programmer");
        // The second NOTE wins; folding drops exactly one leading space.
        assert_eq!(c.notes, "This is a long note that is folded across two lines by the writer");
        assert_eq!(c.websites, vec!["https://ada.example/notes".to_string()]);
        assert_eq!(c.categories, vec!["VIP".to_string(), "Research".to_string()]);
        assert_eq!(c.more.spouse, "William King");
        assert_eq!(c.im_addresses, vec!["xmpp:ada@chat.example".to_string()]);
    }

    #[test]
    fn note_escapes_are_undone() {
        let (contacts, _) = parse("BEGIN:VCARD\nVERSION:3.0\nFN:X\nNOTE:a\\nb\\, c\\; d\\\\e\nEND:VCARD\n");
        assert_eq!(contacts[0].notes, "a\nb, c; d\\e");
    }

    #[test]
    fn parses_vcard_3_apple_style() {
        let text = "BEGIN:VCARD\nVERSION:3.0\nN:Hopper;Grace;Brewster Murray;;\nFN:Grace Hopper\n\
item1.EMAIL;type=INTERNET;type=WORK;type=pref:grace@navy.example\nitem1.X-ABLabel:_$!<Work>!$_\n\
TEL;type=CELL;type=VOICE;type=pref:+1 (555) 010-0100\nTEL;type=WORK;type=VOICE:+1 (555) 010-0200\n\
ADR;type=HOME;type=pref:;;1 Compiler Way;Arlington;VA;22201;USA\nBDAY;X-APPLE-OMIT-YEAR=1604:1604-12-09\n\
X-PHONETIC-FIRST-NAME:GRAY-s\nPHOTO;ENCODING=b;TYPE=JPEG:/9j/4AAQSkZJRg==\nEND:VCARD\n";
        let (contacts, _) = parse(text);
        let c = &contacts[0];
        assert_eq!(c.display_name, "Grace Hopper");
        assert_eq!(c.middle_name, "Brewster Murray");
        assert_eq!(c.emails[0], LabeledValue { label: "Work".into(), value: "grace@navy.example".into() });
        assert_eq!(c.phones[0].label, "Mobile");
        assert_eq!(c.phones[1].label, "Work");
        assert_eq!(c.addresses[0].label, "Home");
        assert_eq!(c.addresses[0].state, "VA");
        assert_eq!(c.birthday, Some(ymd(NO_YEAR, 12, 9)));
        assert_eq!(c.more.phonetic_given_name, "GRAY-s");
        assert_eq!(c.photo.as_deref(), Some("data:image/jpeg;base64,/9j/4AAQSkZJRg=="));
    }

    #[test]
    fn parses_vcard_2_1_quoted_printable_and_bare_types() {
        let text = "BEGIN:VCARD\r\nVERSION:2.1\r\nN;CHARSET=UTF-8;ENCODING=QUOTED-PRINTABLE:N=C3=BA=C3=B1ez;Jos=C3=A9;;;\r\n\
TEL;CELL;PREF:555-0100\r\nTEL;HOME:555-0101\r\nNOTE;ENCODING=QUOTED-PRINTABLE:Line one=0D=0ALine =\r\ntwo\r\nEND:VCARD\r\n";
        let (contacts, _) = parse(text);
        let c = &contacts[0];
        assert_eq!(c.given_name, "José");
        assert_eq!(c.surname, "Núñez");
        assert_eq!(c.display_name, "José Núñez");
        assert_eq!(c.phones[0].label, "Mobile");
        assert_eq!(c.phones[1].label, "Home");
        assert_eq!(c.notes, "Line one\r\nLine two");
    }

    #[test]
    fn reduced_birthday_forms() {
        assert_eq!(parse_date("--1023", None), Some(ymd(NO_YEAR, 10, 23)));
        assert_eq!(parse_date("--10-23", None), Some(ymd(NO_YEAR, 10, 23)));
        assert_eq!(parse_date("1990-10-23", None), Some(ymd(1990, 10, 23)));
        assert_eq!(parse_date("19901023", None), Some(ymd(1990, 10, 23)));
        assert_eq!(parse_date("1990-10-23T00:00:00Z", None), Some(ymd(1990, 10, 23)));
        assert_eq!(parse_date("2000-02-30", None), None);
        assert_eq!(parse_date("junk", None), None);
    }

    #[test]
    fn groups_resolve_uid_and_mailto_members() {
        let text = "BEGIN:VCARD\nVERSION:4.0\nUID:urn:uuid:aaaa\nFN:Ada Lovelace\nEMAIL:ada@example.com\nEND:VCARD\n\
BEGIN:VCARD\nVERSION:4.0\nKIND:group\nFN:Book club\nNOTE:Monthly\nMEMBER:urn:uuid:AAAA\nMEMBER:mailto:grace@example.com\n\
MEMBER:mailto:ADA@example.com\nMEMBER:urn:uuid:missing\nEND:VCARD\n\
BEGIN:VCARD\nVERSION:3.0\nX-ADDRESSBOOKSERVER-KIND:group\nFN:Apple group\nX-ADDRESSBOOKSERVER-MEMBER:urn:uuid:aaaa\nEND:VCARD\n";
        let (contacts, lists) = parse(text);
        assert_eq!(contacts.len(), 1);
        assert_eq!(lists.len(), 2);
        assert_eq!(lists[0].name, "Book club");
        assert_eq!(lists[0].description, "Monthly");
        assert_eq!(lists[0].members, vec![EmailAddress::new("Ada Lovelace", "ada@example.com"), EmailAddress::new("", "grace@example.com")]);
        assert_eq!(lists[1].members.len(), 1);
    }

    fn sample_contact() -> Contact {
        Contact {
            given_name: "José".into(),
            surname: "Núñez".into(),
            middle_name: "Luis".into(),
            display_name: "José Núñez".into(),
            nickname: "Pepe".into(),
            emails: vec![
                LabeledValue { label: "Work".into(), value: "jose@work.example".into() },
                LabeledValue { label: "Personal".into(), value: "jose@home.example".into() },
            ],
            phones: vec![
                LabeledValue { label: "Mobile".into(), value: "+34 600 000 000".into() },
                LabeledValue { label: "Home".into(), value: "+34 910 000 000".into() },
            ],
            addresses: vec![PostalAddress {
                label: "Work".into(),
                street: "Calle Mayor 1, 2ºB".into(),
                city: "Madrid".into(),
                state: "".into(),
                postal_code: "28013".into(),
                country: "Spain".into(),
            }],
            websites: vec!["https://jose.example/a;b,c".into()],
            company: "Acme; Iberia, S.L.".into(),
            job_title: "Engineer".into(),
            department: "R&D".into(),
            office_location: "Madrid HQ".into(),
            birthday: Some(ymd(NO_YEAR, 2, 29)),
            notes: "Line 1\nLine 2 with a much longer text so that the line has to be folded at seventy-five octets, ñ ñ ñ".into(),
            is_favorite: true,
            categories: vec!["VIP".into(), "Spain, EU".into()],
            im_addresses: vec!["xmpp:jose@chat.example".into()],
            more: Box::new(ContactMore {
                spouse: "Ana".into(),
                children: "Leo, Mia".into(),
                manager: "Carmen".into(),
                assistant: "Pablo".into(),
                profession: "Civil engineer".into(),
                phonetic_given_name: "Ho-SAY".into(),
                ..Default::default()
            }),
            photo: Some("data:image/png;base64,iVBORw0KGgo=".into()),
            ..Default::default()
        }
    }

    #[test]
    fn round_trips_v4_and_v3() {
        let original = sample_contact();
        for v4 in [true, false] {
            let text = serialize(std::slice::from_ref(&original), &[], v4);
            assert!(text.lines().all(|l| l.len() <= 76), "folded lines in\n{text}");
            assert!(text.contains(if v4 { "VERSION:4.0" } else { "VERSION:3.0" }));
            let (back, _) = parse(&text);
            let c = &back[0];
            assert_eq!(c.display_name, original.display_name);
            assert_eq!(c.given_name, original.given_name);
            assert_eq!(c.middle_name, original.middle_name);
            assert_eq!(c.emails, original.emails);
            assert_eq!(c.phones, original.phones);
            assert_eq!(c.addresses, original.addresses);
            assert_eq!(c.websites, original.websites);
            assert_eq!(c.company, original.company);
            assert_eq!(c.department, original.department);
            assert_eq!(c.office_location, original.office_location);
            assert_eq!(c.birthday, original.birthday, "v4={v4}");
            assert_eq!(c.notes, original.notes);
            assert!(c.is_favorite);
            assert_eq!(c.categories, original.categories);
            assert_eq!(c.more.spouse, original.more.spouse);
            assert_eq!(c.more.children, original.more.children);
            assert_eq!(c.more.manager, original.more.manager);
            assert_eq!(c.more.assistant, original.more.assistant);
            assert_eq!(c.more.profession, original.more.profession);
            assert_eq!(c.im_addresses, original.im_addresses);
            assert_eq!(c.more.phonetic_given_name, original.more.phonetic_given_name);
            assert_eq!(c.photo, original.photo);
        }
    }

    #[test]
    fn writes_reduced_birthday_per_version() {
        let c = Contact { display_name: "B".into(), birthday: Some(ymd(NO_YEAR, 10, 23)), ..Default::default() };
        assert!(serialize(std::slice::from_ref(&c), &[], true).contains("BDAY:--1023\r\n"));
        assert!(serialize(std::slice::from_ref(&c), &[], false).contains("BDAY;X-APPLE-OMIT-YEAR=1604:1604-10-23\r\n"));
        let dated = Contact { birthday: Some(ymd(1990, 1, 2)), ..c };
        assert!(serialize(std::slice::from_ref(&dated), &[], true).contains("BDAY:19900102\r\n"));
    }

    #[test]
    fn lists_round_trip_in_both_versions() {
        let list = ContactList {
            name: "Launch team".into(),
            description: "Ship it, together".into(),
            members: vec![EmailAddress::new("Ada", "ada@example.com"), EmailAddress::new("", "grace@example.com")],
            is_favorite: true,
            ..Default::default()
        };
        for v4 in [true, false] {
            let text = serialize(&[], std::slice::from_ref(&list), v4);
            assert!(text.contains(if v4 { "KIND:group" } else { "X-ADDRESSBOOKSERVER-KIND:group" }));
            let (contacts, lists) = parse(&text);
            assert!(contacts.is_empty());
            assert_eq!(lists[0].name, "Launch team");
            assert_eq!(lists[0].description, "Ship it, together");
            assert!(lists[0].is_favorite);
            let addrs: Vec<_> = lists[0].members.iter().map(|m| m.address.as_str()).collect();
            assert_eq!(addrs, ["ada@example.com", "grace@example.com"]);
        }
    }

    #[test]
    fn folding_never_splits_multibyte_characters() {
        let mut out = String::new();
        fold_into(&mut out, &format!("NOTE:{}", "é".repeat(100)));
        for line in out.split("\r\n").filter(|l| !l.is_empty()) {
            assert!(line.len() <= 75);
        }
        let (contacts, _) = parse(&format!("BEGIN:VCARD\r\nVERSION:4.0\r\nFN:x\r\n{out}END:VCARD\r\n"));
        assert_eq!(contacts[0].notes, "é".repeat(100));
    }

    #[test]
    fn ignores_garbage_and_empty_cards() {
        let (contacts, lists) = parse("hello\nBEGIN:VCARD\nVERSION:4.0\nEND:VCARD\nnot a card");
        assert!(contacts.is_empty() && lists.is_empty());
    }
}
