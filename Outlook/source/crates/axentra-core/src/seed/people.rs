//! Demo contacts, contact folders and contact lists.

use chrono::{Duration, NaiveDate, Utc};

use super::{PEOPLE, SeedCtx, person};
use crate::model::*;
use crate::{Result, Store};

/// Extras for one demo person: (work phone, birthday (y, m, d; year 1604 =
/// no year), nickname, notes, days since added).
type Extra = (&'static str, Option<(i32, u32, u32)>, &'static str, &'static str, i64);

/// Per-person extras for the shared demo people, by index into `PEOPLE`.
const EXTRAS: &[Extra] = &[
    ("+1 (206) 555-0142", Some((1990, 10, 23)), "", "Main contact for the Northwind rollout. Prefers Teams calls before 10 AM Pacific.", 0),
    ("+1 (415) 555-0110", Some((1604, 3, 14)), "", "", 1),
    ("+49 30 5550 1234", None, "Leni", "Prefers morning meetings. Loves good coffee.\nAsk about the Berlin studio move.", 3),
    ("+1 (415) 555-0111", Some((1994, 7, 2)), "Tom", "", 12),
    ("+1 (415) 555-0112", None, "", "Owns the launch messaging. Out the last week of November.", 20),
    ("+1 (206) 555-0177", None, "", "", 26),
    ("+39 02 5550 8812", Some((1988, 12, 1)), "", "", 40),
    ("+1 (415) 555-0113", None, "Dan", "", 55),
    ("+1 (312) 555-0190", None, "", "Outside counsel. NDA template lives in the shared drive.", 70),
    ("+1 (415) 555-0114", None, "", "", 90),
    ("+1 (206) 555-0123", Some((1604, 5, 30)), "", "", 120),
    ("+971 4 555 0199", None, "", "Books all team travel. Needs passport details two weeks ahead.", 200),
];

fn split_name(name: &str) -> (String, String) {
    let mut parts = name.splitn(2, ' ');
    (parts.next().unwrap_or_default().into(), parts.next().unwrap_or_default().into())
}

fn labeled(label: &str, value: &str) -> LabeledValue {
    LabeledValue { label: label.into(), value: value.into() }
}

pub(super) fn seed(store: &Store, ctx: &SeedCtx) -> Result<()> {
    let now = Utc::now();
    let family = store.create_contact_folder("Family")?;

    for (i, (name, email, company, title)) in PEOPLE.iter().enumerate() {
        let (given, surname) = split_name(name);
        let (work_phone, birthday, nickname, notes, days_ago) = EXTRAS[i % EXTRAS.len()];
        let axentra = *company == "Axentra";
        let mut c = Contact {
            account_id: ctx.account_id,
            given_name: given.clone(),
            surname,
            display_name: name.to_string(),
            nickname: nickname.into(),
            emails: vec![labeled("Work", email)],
            phones: vec![labeled("Mobile", &format!("+1 (415) 555-{:04}", 1200 + i * 37)), labeled("Work", work_phone)],
            company: company.to_string(),
            job_title: title.to_string(),
            department: match *company {
                "Axentra" => "Product & Engineering".into(),
                "Northwind Traders" => "Operations".into(),
                "Lumen Studio" => "Creative".into(),
                _ => String::new(),
            },
            office_location: if axentra { "San Francisco · Floor 4".into() } else { String::new() },
            more: Box::new(ContactMore {
                manager: if axentra && i != 1 { "Marcus Lee".into() } else { String::new() },
                ..Default::default()
            }),
            is_favorite: matches!(i, 0 | 1 | 4),
            birthday: birthday.and_then(|(y, m, d)| NaiveDate::from_ymd_opt(y, m, d)),
            notes: notes.into(),
            websites: match *company {
                "Northwind Traders" => vec!["https://northwind.example".into()],
                "Lumen Studio" => vec!["https://lumen.example/studio".into()],
                _ => Vec::new(),
            },
            im_addresses: if axentra { vec![format!("xmpp:{email}")] } else { Vec::new() },
            categories: match i {
                0 => vec!["Blue category".into()],
                2 | 6 => vec!["Purple category".into()],
                8 => vec!["Red category".into()],
                _ => Vec::new(),
            },
            created_at: now - Duration::days(days_ago) - Duration::hours(i as i64),
            ..Default::default()
        };
        match i {
            0 => {
                c.addresses = vec![PostalAddress {
                    label: "Work".into(),
                    street: "1 Northwind Way".into(),
                    city: "Seattle".into(),
                    state: "WA".into(),
                    postal_code: "98101".into(),
                    country: "United States".into(),
                }];
                c.emails.push(labeled("Personal", "priya.r@outlook.example"));
            }
            1 => {
                c.addresses = vec![PostalAddress {
                    label: "Work".into(),
                    street: "500 Market St".into(),
                    city: "San Francisco".into(),
                    state: "CA".into(),
                    postal_code: "94105".into(),
                    country: "United States".into(),
                }];
                c.more.assistant = "Daniel Kim".into();
            }
            2 => {
                c.addresses = vec![PostalAddress {
                    label: "Work".into(),
                    street: "Torstraße 109".into(),
                    city: "Berlin".into(),
                    state: String::new(),
                    postal_code: "10119".into(),
                    country: "Germany".into(),
                }];
            }
            8 => c.more.title = "Ms.".into(),
            _ => {}
        }
        store.insert_contact(&mut c)?;
    }

    // Personal contacts: (given, surname, display name, email, mobile, folder, days since added).
    let personal: &[(&str, &str, &str, &str, &str, &str, i64)] = &[
        ("Jordan", "Reyes", "Jordan Reyes", "jordan.reyes@gmail.example", "+1 (510) 555-0134", "", 2),
        ("Mina", "Sato", "Mina Sato", "mina.sato@outlook.example", "+81 3 5550 0101", "", 6),
        ("Ben", "Carter", "Ben Carter", "ben@carterbuilds.example", "+1 (650) 555-0188", "", 15),
        ("Rosa", "Morgan", "Rosa Morgan", "rosa.morgan@family.example", "+1 (503) 555-0119", family.as_str(), 400),
        ("Chris", "Morgan", "Chris Morgan", "chris.morgan@family.example", "+1 (503) 555-0120", family.as_str(), 400),
        ("Émile", "Dubois", "Émile Dubois", "emile.dubois@atelier.example", "+33 1 5550 4411", "", 33),
        ("Zoë", "Ångström", "Zoë Ångström", "zoe@nordlys.example", "", "", 60),
        // Family name first, as written in Chinese.
        ("雷", "李", "李雷", "li.lei@example.cn", "+86 10 5550 0168", "", 75),
    ];
    for (given, surname, display, email, mobile, folder, days_ago) in personal {
        let mut c = Contact {
            account_id: ctx.account_id,
            given_name: given.to_string(),
            surname: surname.to_string(),
            display_name: display.to_string(),
            emails: vec![labeled("Personal", email)],
            phones: if mobile.is_empty() { Vec::new() } else { vec![labeled("Mobile", mobile)] },
            folder: folder.to_string(),
            created_at: now - Duration::days(*days_ago),
            ..Default::default()
        };
        match *given {
            "Rosa" => {
                c.nickname = "Mom".into();
                c.birthday = NaiveDate::from_ymd_opt(1962, 10, 30);
                c.addresses = vec![PostalAddress {
                    label: "Home".into(),
                    street: "2210 SE Ash St".into(),
                    city: "Portland".into(),
                    state: "OR".into(),
                    postal_code: "97214".into(),
                    country: "United States".into(),
                }];
                c.more.spouse = "Chris Morgan".into();
                c.is_favorite = true;
            }
            "Chris" => {
                c.more.spouse = "Rosa Morgan".into();
                c.birthday = NaiveDate::from_ymd_opt(1960, 4, 8);
            }
            "Ben" => {
                c.company = "Carter Builds".into();
                c.job_title = "Owner".into();
                c.notes = "Kitchen remodel quote due in November.".into();
                c.websites = vec!["https://carterbuilds.example".into()];
            }
            "Émile" => {
                c.company = "Atelier Dubois".into();
                c.job_title = "Illustrator".into();
            }
            _ => {}
        }
        store.insert_contact(&mut c)?;
    }

    // A phone-only contact and a company-only contact exercise the name fallbacks.
    store.insert_contact(&mut Contact {
        account_id: ctx.account_id,
        company: "Bayview Dental".into(),
        phones: vec![labeled("Work", "+1 (415) 555-0161")],
        addresses: vec![PostalAddress {
            label: "Work".into(),
            street: "88 Bayview Ave".into(),
            city: "San Francisco".into(),
            state: "CA".into(),
            postal_code: "94124".into(),
            country: "United States".into(),
        }],
        notes: "Cleaning every six months.".into(),
        created_at: now - Duration::days(300),
        ..Default::default()
    })?;

    let mut launch = ContactList {
        account_id: ctx.account_id,
        name: "Launch team".into(),
        description: "Everyone shipping the Q4 launch.".into(),
        members: vec![person(0), person(1), person(4), person(5), person(3)],
        is_favorite: true,
        created_at: now - Duration::days(10),
        ..Default::default()
    };
    store.insert_contact_list(&mut launch)?;
    let mut book_club = ContactList {
        account_id: ctx.account_id,
        name: "Book club".into(),
        description: "First Thursday of the month.".into(),
        members: vec![
            EmailAddress::new("Jordan Reyes", "jordan.reyes@gmail.example"),
            EmailAddress::new("Mina Sato", "mina.sato@outlook.example"),
            EmailAddress::new("Sam Patel", "sam.patel@bookworms.example"),
        ],
        created_at: now - Duration::days(45),
        ..Default::default()
    };
    store.insert_contact_list(&mut book_club)?;
    Ok(())
}
