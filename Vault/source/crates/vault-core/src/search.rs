//! Ranked search. Every word in the query must match somewhere; matches are scored by where they
//! land (title beats website beats username beats notes) and whether they hit the start of a word.
//! Notes are searched only on request, and archived or recycled entries are left out by default,
//! so "outlook" brings up the Outlook login, not every entry that mentions Outlook in passing.

use keepass::Database;
use keepass::db::{Entry, EntryId, fields};

use crate::organize;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SearchOptions {
    pub include_notes: bool,
    pub include_archived: bool,
    /// Also list entries that match only through the domain of an email username
    /// (searching "outlook" when most logins are name@outlook.com).
    pub include_email_domains: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Field {
    Title,
    Website,
    Username,
    Tag,
    Custom,
    Notes,
    /// The part of an email username after the "@".
    EmailDomain,
}

impl Field {
    pub fn label(self) -> &'static str {
        match self {
            Field::Title => "title",
            Field::Website => "website",
            Field::Username => "username",
            Field::Tag => "tag",
            Field::Custom => "field",
            Field::Notes => "notes",
            Field::EmailDomain => "email domain",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub id: EntryId,
    pub score: u32,
    /// Where each query word matched best, deduplicated.
    pub matched: Vec<Field>,
}

/// Host part of a URL: "https://user@login.live.com:443/x?y" -> "login.live.com".
pub fn host_of(url: &str) -> &str {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let rest = rest.split(['/', '?', '#']).next().unwrap_or("");
    let rest = rest.rsplit_once('@').map(|(_, h)| h).unwrap_or(rest);
    rest.split(':').next().unwrap_or(rest)
}

/// 3 = whole value, 2 = start of value, 1 = start of a word, 0 = no match. Substrings inside a
/// word ("look" in "outlook") do not count, which is what keeps results relevant.
fn word_match(hay_lower: &str, term: &str) -> u32 {
    if hay_lower == term {
        return 3;
    }
    if hay_lower.starts_with(term) {
        return 2;
    }
    let bytes = hay_lower.as_bytes();
    let found = hay_lower
        .match_indices(term)
        .any(|(i, _)| i > 0 && !(bytes[i - 1] as char).is_alphanumeric());
    if found { 1 } else { 0 }
}

fn score_term(e: &Entry, term: &str, opts: SearchOptions) -> Option<(u32, Field)> {
    let lower = |s: Option<&str>| s.unwrap_or("").to_lowercase();
    let mut best: Option<(u32, Field)> = None;
    let mut consider = |score: u32, field: Field| {
        if score > 0 && best.is_none_or(|(b, _)| score > b) {
            best = Some((score, field));
        }
    };

    let title = lower(e.get_title());
    consider(
        [0, 60, 90, 100][word_match(&title, term) as usize],
        Field::Title,
    );

    let host = host_of(e.get_url().unwrap_or("")).to_lowercase();
    // Hosts split on dots and dashes, so "outlook" matches "outlook.live.com" and "login.outlook.com".
    let host_hit = host
        .split(['.', '-'])
        .map(|part| word_match(part, term))
        .max()
        .unwrap_or(0);
    consider([0, 45, 55, 70][host_hit as usize], Field::Website);

    // An email username is two things: the name part identifies the account; the domain part
    // ("outlook.com") is shared by most logins, so it only counts as a weak match.
    let user = lower(e.get_username());
    let (local, domain) = match user.rsplit_once('@') {
        Some((l, d)) => (l.to_owned(), d.to_owned()),
        None => (user.clone(), String::new()),
    };
    consider(
        [0, 30, 40, 50][word_match(&local, term) as usize],
        Field::Username,
    );
    if !domain.is_empty() {
        let hit = domain
            .split(['.', '-'])
            .map(|part| word_match(part, term))
            .max()
            .unwrap_or(0);
        consider([0, 2, 3, 4][hit as usize], Field::EmailDomain);
    }

    for tag in &e.tags {
        consider(
            [0, 30, 40, 60][word_match(&tag.to_lowercase(), term) as usize],
            Field::Tag,
        );
    }

    for (name, value) in &e.fields {
        if fields::KNOWN_FIELDS.contains(&name.as_str()) || name == fields::OTP {
            continue;
        }
        consider(
            [0, 15, 20, 25][word_match(&name.to_lowercase(), term) as usize],
            Field::Custom,
        );
        if !value.is_protected() {
            consider(
                [0, 10, 15, 20][word_match(&value.get().to_lowercase(), term) as usize],
                Field::Custom,
            );
        }
    }

    if opts.include_notes {
        consider(
            [0, 5, 8, 10][word_match(&lower(e.get(fields::NOTES)), term) as usize],
            Field::Notes,
        );
    }
    best
}

/// Ranked hits, plus entries left out because they matched only by email domain.
#[derive(Debug, Default)]
pub struct SearchReport {
    pub hits: Vec<Hit>,
    /// How many entries matched only through an email username's domain and were hidden.
    pub hidden_email_matches: usize,
    /// The most common such domain, for a "N more use an @outlook.com email" note.
    pub hidden_domain: Option<String>,
}

pub fn search(db: &Database, query: &str, opts: SearchOptions) -> Vec<Hit> {
    search_report(db, query, opts).hits
}

/// Searches, then (unless asked not to) hides email-domain-only matches when there are better
/// ones. If the email domain is all that matched anywhere, those entries are kept.
pub fn search_report(db: &Database, query: &str, opts: SearchOptions) -> SearchReport {
    let mut hits = rank(db, query, opts);
    let weak = |h: &Hit| h.matched.iter().all(|f| *f == Field::EmailDomain);
    if opts.include_email_domains || hits.iter().all(weak) {
        return SearchReport {
            hits,
            ..Default::default()
        };
    }
    let mut domains: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut hidden = 0;
    hits.retain(|h| {
        if !weak(h) {
            return true;
        }
        hidden += 1;
        let domain = db.entry(h.id).and_then(|e| {
            e.get_username()
                .and_then(|u| u.rsplit_once('@'))
                .map(|(_, d)| d.to_lowercase())
        });
        if let Some(d) = domain {
            *domains.entry(d).or_default() += 1;
        }
        false
    });
    let hidden_domain = domains.into_iter().max_by_key(|(_, n)| *n).map(|(d, _)| d);
    SearchReport {
        hits,
        hidden_email_matches: hidden,
        hidden_domain,
    }
}

fn rank(db: &Database, query: &str, opts: SearchOptions) -> Vec<Hit> {
    let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if terms.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<(Hit, String)> = db
        .iter_all_entries()
        .filter(|e| !organize::in_recycle_bin(e))
        .filter(|e| opts.include_archived || !organize::is_archived(e))
        .filter_map(|e| {
            let mut score = 0;
            let mut matched = Vec::new();
            for t in &terms {
                let (s, f) = score_term(&e, t, opts)?;
                score += s;
                if !matched.contains(&f) {
                    matched.push(f);
                }
            }
            matched.sort();
            Some((
                Hit {
                    id: e.id(),
                    score,
                    matched,
                },
                e.get_title().unwrap_or("").to_lowercase(),
            ))
        })
        .collect();

    hits.sort_by(|(a, at), (b, bt)| b.score.cmp(&a.score).then_with(|| at.cmp(bt)));
    hits.into_iter().map(|(h, _)| h).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> (Database, Vec<EntryId>) {
        let mut db = Database::new();
        let mut ids = Vec::new();
        let mut root = db.root_mut();
        for (title, user, url, notes) in [
            (
                "Outlook",
                "shawn@outlook.com",
                "https://outlook.live.com/mail",
                "",
            ),
            (
                "Microsoft account",
                "shawn@outlook.com",
                "https://login.live.com",
                "Same login as Outlook",
            ),
            (
                "Bank",
                "shawn",
                "https://bank.example",
                "Statements also go to my outlook inbox",
            ),
            ("Lookout security", "shawn", "https://lookout.com", ""),
            ("Work email", "s.brooks", "https://mail.work.example", ""),
        ] {
            let mut e = root.add_entry();
            e.set_unprotected(fields::TITLE, title);
            e.set_unprotected(fields::USERNAME, user);
            e.set_unprotected(fields::URL, url);
            e.set_unprotected(fields::NOTES, notes);
            ids.push(e.id());
        }
        (db, ids)
    }

    #[test]
    fn title_match_ranks_first_and_notes_are_ignored() {
        let (db, ids) = db();
        let hits = search(&db, "outlook", SearchOptions::default());
        let found: Vec<EntryId> = hits.iter().map(|h| h.id).collect();
        assert_eq!(found[0], ids[0], "Outlook entry should be first");
        assert!(
            !found.contains(&ids[1]),
            "an @outlook.com login alone is hidden when there are real matches"
        );
        assert!(
            !found.contains(&ids[2]),
            "a passing mention in notes must not match by default"
        );
        assert!(
            !found.contains(&ids[3]),
            "'look' inside 'lookout' is not a match for 'outlook'"
        );
    }

    #[test]
    fn email_domain_matches_are_reported_and_can_be_shown() {
        let (db, ids) = db();
        let report = search_report(&db, "outlook", SearchOptions::default());
        assert!(report.hidden_email_matches >= 1);
        assert_eq!(report.hidden_domain.as_deref(), Some("outlook.com"));
        let opts = SearchOptions {
            include_email_domains: true,
            ..Default::default()
        };
        let all: Vec<EntryId> = search(&db, "outlook", opts).iter().map(|h| h.id).collect();
        assert!(all.contains(&ids[1]), "shown when asked");
        assert_eq!(all[0], ids[0], "still ranked below the real match");
    }

    #[test]
    fn username_name_part_still_matches() {
        let (db, _) = db();
        assert!(!search(&db, "shawn", SearchOptions::default()).is_empty());
    }

    #[test]
    fn notes_only_when_asked() {
        let (db, ids) = db();
        let opts = SearchOptions {
            include_notes: true,
            ..Default::default()
        };
        let found: Vec<EntryId> = search(&db, "outlook", opts).iter().map(|h| h.id).collect();
        assert!(found.contains(&ids[2]));
        let pos = |id: &EntryId| found.iter().position(|x| x == id).unwrap();
        assert!(
            pos(&ids[2]) > pos(&ids[0]),
            "notes matches rank below title matches"
        );
    }

    #[test]
    fn all_words_must_match() {
        let (db, ids) = db();
        let found: Vec<EntryId> = search(&db, "work mail", SearchOptions::default())
            .iter()
            .map(|h| h.id)
            .collect();
        assert_eq!(found, vec![ids[4]]);
    }

    #[test]
    fn mid_word_substrings_do_not_match() {
        let (db, _) = db();
        assert!(search(&db, "utlook", SearchOptions::default()).is_empty());
    }

    #[test]
    fn archived_and_recycled_hidden_by_default() {
        let (mut db, ids) = db();
        organize::set_archived(&mut db.entry_mut(ids[0]).unwrap(), true);
        let found: Vec<EntryId> = search(&db, "outlook", SearchOptions::default())
            .iter()
            .map(|h| h.id)
            .collect();
        assert!(!found.contains(&ids[0]));
        let opts = SearchOptions {
            include_archived: true,
            ..Default::default()
        };
        assert!(search(&db, "outlook", opts).iter().any(|h| h.id == ids[0]));
    }

    #[test]
    fn hosts() {
        assert_eq!(
            host_of("https://user@login.live.com:443/x?y"),
            "login.live.com"
        );
        assert_eq!(host_of("outlook.com/mail"), "outlook.com");
        assert_eq!(host_of(""), "");
    }
}
