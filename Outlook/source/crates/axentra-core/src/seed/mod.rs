//! Demo data for first run and for the preview server.
//!
//! Everything is anchored on `now` so the demo always looks current
//! (messages "Today", meetings "this week"). All people, companies and
//! domains are fictional (`*.example`).

mod calendar;
mod mail;
mod people;
mod tasks;

use chrono::{DateTime, Local};

use crate::model::*;
use crate::{Result, Store};

/// Shared context handed to each area's seeder.
pub struct SeedCtx {
    pub now: DateTime<Local>,
    pub account_id: AccountId,
    pub me: EmailAddress,
}

/// The fictional people used across demo mail, calendar and contacts:
/// (display name, email, company, job title).
pub const PEOPLE: &[(&str, &str, &str, &str)] = &[
    ("Priya Raman", "priya.raman@northwind.example", "Northwind Traders", "Product Manager"),
    ("Marcus Lee", "marcus.lee@axentra.example", "Axentra", "Engineering Lead"),
    ("Elena Fischer", "elena.fischer@lumen.example", "Lumen Studio", "Design Director"),
    ("Tomás Ortega", "tomas.ortega@axentra.example", "Axentra", "Software Engineer"),
    ("Hannah Okafor", "hannah.okafor@axentra.example", "Axentra", "Head of Marketing"),
    ("Wei Zhang", "wei.zhang@northwind.example", "Northwind Traders", "Finance Analyst"),
    ("Sofia Rossi", "sofia.rossi@lumen.example", "Lumen Studio", "Account Manager"),
    ("Daniel Kim", "daniel.kim@axentra.example", "Axentra", "QA Engineer"),
    ("Aisha Bello", "aisha.bello@harbor.example", "Harbor Legal", "Counsel"),
    ("Liam O'Connor", "liam.oconnor@axentra.example", "Axentra", "Recruiter"),
    ("Grace Liu", "grace.liu@northwind.example", "Northwind Traders", "Operations Manager"),
    ("Omar Haddad", "omar.haddad@skyline.example", "Skyline Air", "Travel Coordinator"),
];

pub fn person(i: usize) -> EmailAddress {
    let (name, addr, _, _) = PEOPLE[i % PEOPLE.len()];
    EmailAddress::new(name, addr)
}

/// Populate an empty store with the demo account and data. Does nothing if
/// any account already exists.
pub fn seed_demo(store: &Store, now: DateTime<Local>) -> Result<()> {
    if !store.is_empty()? {
        return Ok(());
    }
    let me = EmailAddress::new("Alex Morgan", "alex.morgan@axentra.example");
    let mut account = Account {
        kind: AccountKind::Local,
        display_name: me.name.clone(),
        email: me.address.clone(),
        color: "#0f6cbd".into(),
        config: serde_json::json!({}),
        ..Default::default()
    };
    let account_id = store.add_account(&mut account)?;
    for (name, color) in [
        ("Blue category", "blue"),
        ("Green category", "green"),
        ("Orange category", "orange"),
        ("Purple category", "purple"),
        ("Red category", "red"),
        ("Yellow category", "yellow"),
    ] {
        store.upsert_category(&Category { name: name.into(), color: color.into() })?;
    }
    let ctx = SeedCtx { now, account_id, me };
    people::seed(store, &ctx)?;
    calendar::seed(store, &ctx)?;
    mail::seed(store, &ctx)?;
    tasks::seed(store, &ctx)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn seed_is_complete_and_idempotent() {
        let store = Store::open_in_memory().unwrap();
        let now = Local.with_ymd_and_hms(2026, 10, 19, 11, 30, 0).unwrap();
        seed_demo(&store, now).unwrap();
        seed_demo(&store, now).unwrap();
        assert_eq!(store.accounts().unwrap().len(), 1);
        let inbox = store.folder_by_role(store.accounts().unwrap()[0].id, FolderRole::Inbox).unwrap().unwrap();
        assert!(inbox.total_count >= 20, "inbox has {}", inbox.total_count);
        assert!(inbox.unread_count >= 3);
        assert!(!store.contacts().unwrap().is_empty());
        assert!(!store.task_lists().unwrap().is_empty());
        let week = store
            .events_in_range(
                crate::local_to_utc(now.date_naive(), chrono::NaiveTime::MIN),
                crate::local_to_utc(now.date_naive() + chrono::Duration::days(7), chrono::NaiveTime::MIN),
            )
            .unwrap();
        assert!(week.len() >= 5);
    }
}
