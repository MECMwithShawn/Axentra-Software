//! Outlook-compatible contacts CSV (RFC 4180 quoting). Headers follow the
//! list in docs/spec/people-todo.md 2.13 and are matched case-insensitively,
//! ignoring spaces and hyphens, so "E-mail Address" and "Email Address" both
//! work.

use chrono::{Datelike, NaiveDate};

use super::vcard::NO_YEAR;
use crate::model::*;

pub(crate) const HEADERS: &[&str] = &[
    "First Name", "Middle Name", "Last Name", "Title", "Suffix", "Nickname",
    "E-mail Address", "E-mail 2 Address", "E-mail 3 Address",
    "Mobile Phone", "Business Phone", "Home Phone",
    "Company", "Department", "Job Title", "Office Location",
    "Business Street", "Business City", "Business State", "Business Postal Code", "Business Country/Region",
    "Home Street", "Home City", "Home State", "Home Postal Code", "Home Country/Region",
    "Birthday", "Web Page", "Notes", "Categories",
];

/// "E-mail 2 Address" -> "email2address".
fn key(header: &str) -> String {
    header.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Split CSV text into records of fields (quoted fields may hold commas,
/// quotes as `""` and line breaks).
pub(crate) fn records(text: &str) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    let mut record = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if quoted {
            match ch {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => quoted = false,
                other => field.push(other),
            }
            continue;
        }
        match ch {
            '"' if field.is_empty() => quoted = true,
            ',' => record.push(std::mem::take(&mut field)),
            '\r' if chars.peek() == Some(&'\n') => {}
            '\n' | '\r' => {
                record.push(std::mem::take(&mut field));
                if record.iter().any(|f| !f.is_empty()) {
                    out.push(std::mem::take(&mut record));
                } else {
                    record.clear();
                }
            }
            other => field.push(other),
        }
    }
    record.push(field);
    if record.iter().any(|f| !f.is_empty()) {
        out.push(record);
    }
    out
}

/// Parse a contacts CSV whose first record is the header row.
pub(crate) fn parse(text: &str) -> Vec<Contact> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut rows = records(text).into_iter();
    let Some(header) = rows.next() else { return Vec::new() };
    let keys: Vec<String> = header.iter().map(|h| key(h)).collect();
    let mut out = Vec::new();
    for row in rows {
        let get = |names: &[&str]| -> String {
            names
                .iter()
                .find_map(|n| keys.iter().position(|k| k == n).and_then(|i| row.get(i)).filter(|v| !v.trim().is_empty()))
                .map(|v| v.trim().to_string())
                .unwrap_or_default()
        };
        let mut c = Contact {
            given_name: get(&["firstname", "givenname"]),
            middle_name: get(&["middlename", "additionalname"]),
            surname: get(&["lastname", "surname", "familyname"]),
            nickname: get(&["nickname"]),
            company: get(&["company", "organization"]),
            department: get(&["department"]),
            job_title: get(&["jobtitle"]),
            office_location: get(&["officelocation", "office"]),
            notes: get(&["notes"]),
            display_name: get(&["displayname", "name"]),
            more: Box::new(ContactMore { title: get(&["title"]), suffix: get(&["suffix"]), ..Default::default() }),
            ..Default::default()
        };
        for (names, label) in [
            (&["emailaddress", "email", "primaryemail"][..], "Work"),
            (&["email2address", "email2"][..], "Personal"),
            (&["email3address", "email3"][..], "Other"),
        ] {
            let v = get(names);
            if !v.is_empty() {
                c.emails.push(LabeledValue { label: label.into(), value: v });
            }
        }
        for (names, label) in [
            (&["mobilephone", "mobile"][..], "Mobile"),
            (&["businessphone"][..], "Work"),
            (&["businessphone2"][..], "Work"),
            (&["homephone"][..], "Home"),
            (&["homephone2"][..], "Home"),
        ] {
            let v = get(names);
            if !v.is_empty() {
                c.phones.push(LabeledValue { label: label.into(), value: v });
            }
        }
        for (prefix, label) in [("business", "Work"), ("home", "Home"), ("other", "Other")] {
            let f = |suffixes: &[&str]| {
                let names: Vec<String> = suffixes.iter().map(|s| format!("{prefix}{s}")).collect();
                get(&names.iter().map(String::as_str).collect::<Vec<_>>())
            };
            let addr = PostalAddress {
                label: label.into(),
                street: f(&["street"]),
                city: f(&["city"]),
                state: f(&["state"]),
                postal_code: f(&["postalcode", "zip"]),
                country: f(&["countryregion", "country"]),
            };
            if [&addr.street, &addr.city, &addr.state, &addr.postal_code, &addr.country].iter().any(|s| !s.is_empty()) {
                c.addresses.push(addr);
            }
        }
        c.birthday = parse_birthday(&get(&["birthday"]));
        let web = get(&["webpage", "website"]);
        if !web.is_empty() {
            c.websites.push(web);
        }
        c.categories = get(&["categories"]).split(';').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        if c.display_name.is_empty() {
            c.display_name = [c.given_name.as_str(), c.surname.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" ");
        }
        let has_identity = !c.display_name.is_empty() || !c.company.is_empty() || !c.emails.is_empty() || !c.phones.is_empty();
        if has_identity {
            out.push(c);
        }
    }
    out
}

/// "10/23/1990", "1990-10-23", "10/23" (no year). Outlook writes "0/0/00" for none.
fn parse_birthday(s: &str) -> Option<NaiveDate> {
    let s = s.trim();
    if s.is_empty() || s == "0/0/00" {
        return None;
    }
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Some(d);
    }
    let parts: Vec<&str> = s.split('/').collect();
    let num = |i: usize| parts.get(i).and_then(|p| p.trim().parse::<i32>().ok());
    match parts.len() {
        3 => {
            let mut y = num(2)?;
            if y < 100 {
                y += if y > 30 { 1900 } else { 2000 };
            }
            NaiveDate::from_ymd_opt(y, num(0)? as u32, num(1)? as u32)
        }
        2 => NaiveDate::from_ymd_opt(NO_YEAR, num(0)? as u32, num(1)? as u32),
        _ => None,
    }
}

fn quote(field: &str) -> String {
    if field.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

/// UTF-8 CSV with a BOM and CRLF line endings, Outlook header row first.
pub(crate) fn serialize(contacts: &[Contact]) -> String {
    let mut out = String::from("\u{feff}");
    out.push_str(&HEADERS.iter().map(|h| quote(h)).collect::<Vec<_>>().join(","));
    out.push_str("\r\n");
    for c in contacts {
        let email = |i: usize| c.emails.get(i).map(|e| e.value.clone()).unwrap_or_default();
        let phone = |label: &str| c.phones.iter().find(|p| p.label == label).map(|p| p.value.clone()).unwrap_or_default();
        let addr = |label: &str| c.addresses.iter().find(|a| a.label == label).cloned().unwrap_or_default();
        let (work, home) = (addr("Work"), addr("Home"));
        let birthday = c
            .birthday
            .map(|b| if b.year() == NO_YEAR { format!("{}/{}", b.month(), b.day()) } else { format!("{}/{}/{}", b.month(), b.day(), b.year()) })
            .unwrap_or_default();
        let fields: Vec<String> = vec![
            c.given_name.clone(), c.middle_name.clone(), c.surname.clone(), c.more.title.clone(), c.more.suffix.clone(), c.nickname.clone(),
            email(0), email(1), email(2),
            phone("Mobile"), phone("Work"), phone("Home"),
            c.company.clone(), c.department.clone(), c.job_title.clone(), c.office_location.clone(),
            work.street, work.city, work.state, work.postal_code, work.country,
            home.street, home.city, home.state, home.postal_code, home.country,
            birthday, c.websites.first().cloned().unwrap_or_default(), c.notes.clone(), c.categories.join(";"),
        ];
        out.push_str(&fields.iter().map(|f| quote(f)).collect::<Vec<_>>().join(","));
        out.push_str("\r\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc4180_quoting() {
        let rows = records("a,\"b,c\",\"say \"\"hi\"\"\"\r\n\"multi\nline\",,x\n\n");
        assert_eq!(rows, vec![vec!["a", "b,c", "say \"hi\""], vec!["multi\nline", "", "x"]]);
    }

    #[test]
    fn header_variants_and_values() {
        let text = "\u{feff}First Name,Last Name,Email Address,E-mail 2 Address,Mobile Phone,Business Phone,Company,Business City,Birthday,Categories,Notes\n\
Ada,Lovelace,ada@example.com,ada@home.example,555-0100,555-0101,Analytical,London,12/10/1815,VIP;Research,\"Hello, world\"\n\
,,,,,,,,0/0/00,,\n";
        let contacts = parse(text);
        assert_eq!(contacts.len(), 1, "blank row is skipped");
        let c = &contacts[0];
        assert_eq!(c.display_name, "Ada Lovelace");
        assert_eq!(c.emails.iter().map(|e| e.value.as_str()).collect::<Vec<_>>(), ["ada@example.com", "ada@home.example"]);
        assert_eq!(c.phones.iter().map(|p| p.label.as_str()).collect::<Vec<_>>(), ["Mobile", "Work"]);
        assert_eq!(c.addresses[0].city, "London");
        assert_eq!(c.birthday, NaiveDate::from_ymd_opt(1815, 12, 10));
        assert_eq!(c.categories, ["VIP", "Research"]);
        assert_eq!(c.notes, "Hello, world");
    }

    #[test]
    fn round_trip() {
        let c = Contact {
            given_name: "Grace".into(),
            surname: "Hopper".into(),
            display_name: "Grace Hopper".into(),
            emails: vec![LabeledValue { label: "Work".into(), value: "grace@navy.example".into() }],
            phones: vec![LabeledValue { label: "Home".into(), value: "+1 555 0100".into() }],
            company: "Navy, US".into(),
            notes: "Said \"it's easier to ask forgiveness\"\nSecond line".into(),
            birthday: NaiveDate::from_ymd_opt(NO_YEAR, 12, 9),
            addresses: vec![PostalAddress { label: "Home".into(), street: "1 Way".into(), city: "Arlington".into(), ..Default::default() }],
            categories: vec!["A".into(), "B".into()],
            websites: vec!["https://grace.example".into()],
            ..Default::default()
        };
        let text = serialize(std::slice::from_ref(&c));
        assert!(text.starts_with("\u{feff}First Name,Middle Name,Last Name"));
        let back = parse(&text);
        assert_eq!(back.len(), 1);
        let b = &back[0];
        assert_eq!(b.display_name, "Grace Hopper");
        assert_eq!(b.emails[0].value, "grace@navy.example");
        assert_eq!(b.phones, c.phones);
        assert_eq!(b.company, c.company);
        assert_eq!(b.notes, c.notes);
        assert_eq!(b.birthday, c.birthday);
        assert_eq!(b.addresses, c.addresses);
        assert_eq!(b.categories, c.categories);
        assert_eq!(b.websites, c.websites);
    }

    #[test]
    fn birthday_formats() {
        assert_eq!(parse_birthday("10/23/1990"), NaiveDate::from_ymd_opt(1990, 10, 23));
        assert_eq!(parse_birthday("10/23/90"), NaiveDate::from_ymd_opt(1990, 10, 23));
        assert_eq!(parse_birthday("1990-10-23"), NaiveDate::from_ymd_opt(1990, 10, 23));
        assert_eq!(parse_birthday("10/23"), NaiveDate::from_ymd_opt(NO_YEAR, 10, 23));
        assert_eq!(parse_birthday("0/0/00"), None);
        assert_eq!(parse_birthday("13/40/2000"), None);
    }
}
