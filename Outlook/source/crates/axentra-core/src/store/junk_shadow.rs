//! Local AI review's log (see [`junk::shadow`]): one row for each message
//! that was reviewed or passed over, so each is weighed once. Rows go with
//! their message. Nothing here moves mail.

use chrono::Utc;
use rusqlite::{OptionalExtension, params};

use super::Store;
use crate::error::Result;
use crate::junk::shadow::{self, Recommendation, Review};
use crate::junk::{self, Label};
use crate::model::{AccountId, MessageId};

pub(crate) const SHADOW_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS junk_shadow (
    message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    account_id INTEGER NOT NULL,
    fingerprint TEXT NOT NULL,
    policy TEXT NOT NULL,
    skipped TEXT,
    first_label TEXT,
    second_label TEXT,
    recommendation TEXT,
    first_ms INTEGER NOT NULL DEFAULT 0,
    second_ms INTEGER NOT NULL DEFAULT 0,
    at INTEGER NOT NULL);
"#;

/// When the review started: mail that arrived before it isn't read.
const SINCE: &str = "junk.shadow_since";
/// Messages looked at per call, past those it can't read yet.
const SCAN: u32 = 200;

/// How the pilot is going, for Settings. `confirmed`, `rejected` and
/// `missed` compare the models with what you reported.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShadowTally {
    pub reviewed: i64,
    pub phishing: i64,
    pub bulk: i64,
    /// You reported a recommended message as unwanted.
    pub confirmed: i64,
    /// You marked a recommended message Not junk.
    pub rejected: i64,
    /// You reported a message the models let through.
    pub missed: i64,
}

impl Store {
    /// Start (or restart) the review now: mail already here isn't read.
    pub fn start_shadow_review(&self) -> Result<()> {
        self.kv_set(SINCE, &Utc::now().timestamp().to_string())
    }

    /// The next message to review and its prompt: new Inbox mail the junk
    /// filter has weighed and left, with no label from you, from someone
    /// you've never dealt with. Mail from contacts is passed over for good.
    pub fn shadow_next(&self) -> Result<Option<(MessageId, String)>> {
        let settings = self.settings();
        if !settings.junk_shadow {
            return Ok(None);
        }
        // Switched on by a restored backup: it starts from now, like any start.
        let Some(since) = self.kv_get(SINCE)?.and_then(|v| v.parse::<i64>().ok()) else {
            self.start_shadow_review()?;
            return Ok(None);
        };
        let ids: Vec<MessageId> = {
            let conn = self.conn();
            let mut stmt = conn.prepare_cached(
                "SELECT m.id FROM messages m JOIN folders f ON f.id = m.folder_id
                 JOIN junk_assessments a ON a.message_id = m.id
                 LEFT JOIN junk_filtered x ON x.message_id = m.id
                 LEFT JOIN junk_shadow r ON r.message_id = m.id
                 WHERE f.role = 'inbox' AND r.message_id IS NULL AND COALESCE(json_extract(m.json, '$.is_draft'), 0) = 0
                   AND (?1 = 0 OR x.moved = 0)
                 ORDER BY json_extract(m.json, '$.received_at') DESC LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![settings.junk_filter, SCAN], |r| r.get(0))?;
            rows.collect::<std::result::Result<_, _>>()?
        };
        let now = Utc::now().timestamp();
        for id in ids {
            let Some(m) = self.message(id)? else { continue };
            let arrived = m.received_at.timestamp();
            if arrived < since {
                // Newest first: the rest are older still.
                break;
            }
            // Not here yet: picked up once its body arrives.
            let Some(body) = self.message_body(id)? else { continue };
            let fingerprint = junk::fingerprint_of(&m, Some(&body));
            let skip = if arrived > now + 86_400 {
                Some("future")
            } else if self.message_label(id)?.is_some() {
                Some("labelled")
            } else if self.known_sender(m.account_id, &m.from.address)? {
                Some("contact")
            } else {
                None
            };
            match skip {
                Some(why) => self.write_shadow(id, m.account_id, &fingerprint, Some(why), None)?,
                None => return Ok(Some((id, shadow::prompt(&m, &body)))),
            }
        }
        Ok(None)
    }

    /// Record a review. A message deleted meanwhile, or a review switched
    /// off meanwhile, leaves nothing behind.
    pub fn save_shadow_review(&self, id: MessageId, review: &Review) -> Result<()> {
        if !self.settings().junk_shadow {
            return Ok(());
        }
        let Some(m) = self.message(id)? else { return Ok(()) };
        let body = self.message_body(id)?;
        self.write_shadow(id, m.account_id, &junk::fingerprint_of(&m, body.as_ref()), None, Some(review))
    }

    fn write_shadow(&self, id: MessageId, account: AccountId, fingerprint: &str, skipped: Option<&str>, review: Option<&Review>) -> Result<()> {
        let verdict = |v: Option<shadow::Verdict>| v.map(|v| v.as_str());
        self.conn().execute(
            "INSERT OR REPLACE INTO junk_shadow
             (message_id, account_id, fingerprint, policy, skipped, first_label, second_label, recommendation, first_ms, second_ms, at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                id,
                account,
                fingerprint,
                shadow::POLICY,
                skipped,
                verdict(review.and_then(|r| r.first)),
                verdict(review.and_then(|r| r.second)),
                review.and_then(|r| r.recommendation()).map(Recommendation::as_str),
                review.map_or(0, |r| r.first_ms as i64),
                review.map_or(0, |r| r.second_ms as i64),
                Utc::now().timestamp(),
            ],
        )?;
        Ok(())
    }

    /// What the review recommends for this message, if anything.
    pub fn shadow_recommendation(&self, id: MessageId) -> Result<Option<Recommendation>> {
        let r: Option<Option<String>> =
            self.conn().query_row("SELECT recommendation FROM junk_shadow WHERE message_id = ?1", [id], |r| r.get(0)).optional()?;
        Ok(r.flatten().as_deref().and_then(Recommendation::parse))
    }

    pub fn shadow_tally(&self) -> Result<ShadowTally> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT s.recommendation, l.label FROM junk_shadow s
             LEFT JOIN junk_labels l ON l.account_id = s.account_id AND l.fingerprint = s.fingerprint
             WHERE s.skipped IS NULL",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?)))?;
        let mut t = ShadowTally::default();
        for row in rows {
            let (recommendation, label) = row?;
            let recommendation = recommendation.as_deref().and_then(Recommendation::parse);
            let label = label.as_deref().and_then(Label::parse);
            t.reviewed += 1;
            match recommendation {
                Some(Recommendation::Phishing) => t.phishing += 1,
                Some(Recommendation::Bulk) => t.bulk += 1,
                None => {}
            }
            match (recommendation.is_some(), label) {
                (true, Some(Label::NotSpam)) => t.rejected += 1,
                (true, Some(_)) => t.confirmed += 1,
                (false, Some(l)) if l != Label::NotSpam => t.missed += 1,
                _ => {}
            }
        }
        Ok(t)
    }

    /// Clear the log. Your own reports stay.
    pub fn forget_shadow(&self) -> Result<()> {
        self.conn().execute("DELETE FROM junk_shadow", [])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::junk::shadow::Verdict;
    use crate::model::*;
    use crate::store::test_util::store_with_account;

    fn on(store: &Store) {
        let mut s = store.settings();
        s.junk_shadow = true;
        s.junk_filter = false;
        store.save_settings(&s).unwrap();
        store.start_shadow_review().unwrap();
        store.kv_set(SINCE, &(Utc::now().timestamp() - 60).to_string()).unwrap();
    }

    fn add(store: &Store, acct: AccountId, from: &str, age_secs: i64) -> MessageId {
        let folder_id = store.folder_for_role(acct, FolderRole::Inbox).unwrap();
        let mut m = Message {
            account_id: acct,
            folder_id,
            from: EmailAddress::new("", from),
            subject: format!("from {from}"),
            received_at: Utc::now() - chrono::Duration::seconds(age_secs),
            ..Default::default()
        };
        let id = store.insert_message(&mut m, Some(&MessageBody { text: "hello".into(), ..Default::default() })).unwrap();
        store.assess_pending(100).unwrap();
        id
    }

    const PHISHING: Review = Review { first: Some(Verdict::Phishing), second: Some(Verdict::Phishing), first_ms: 200, second_ms: 9000 };
    const CLEAR: Review = Review { first: Some(Verdict::Legitimate), second: None, first_ms: 200, second_ms: 0 };

    #[test]
    fn off_by_default_and_old_mail_is_never_read() {
        let (store, acct) = store_with_account();
        assert!(!store.settings().junk_shadow);
        add(&store, acct, "a@stranger.example", 0);
        assert_eq!(store.shadow_next().unwrap(), None);
        on(&store);
        add(&store, acct, "old@stranger.example", 3600);
        let (id, prompt) = store.shadow_next().unwrap().unwrap();
        assert!(prompt.contains("a@stranger.example"));
        store.save_shadow_review(id, &CLEAR).unwrap();
        assert_eq!(store.shadow_next().unwrap(), None);
    }

    #[test]
    fn a_review_never_moves_mail_and_each_message_is_read_once() {
        let (store, acct) = store_with_account();
        on(&store);
        let id = add(&store, acct, "a@stranger.example", 0);
        let before = store.message(id).unwrap().unwrap().folder_id;
        let (next, _) = store.shadow_next().unwrap().unwrap();
        assert_eq!(next, id);
        store.save_shadow_review(id, &PHISHING).unwrap();
        assert_eq!(store.message(id).unwrap().unwrap().folder_id, before);
        assert_eq!(store.message_label(id).unwrap(), None, "a model's verdict is never a confirmed label");
        assert_eq!(store.shadow_recommendation(id).unwrap(), Some(Recommendation::Phishing));
        assert_eq!(store.shadow_next().unwrap(), None);
    }

    #[test]
    fn contacts_are_passed_over_and_the_filter_goes_first() {
        let (store, acct) = store_with_account();
        on(&store);
        let mut c = Contact { account_id: acct, display_name: "Pat".into(), ..Default::default() };
        c.emails.push(LabeledValue { label: "Work".into(), value: "pat@friend.example".into() });
        store.insert_contact(&mut c).unwrap();
        add(&store, acct, "pat@friend.example", 0);
        assert_eq!(store.shadow_next().unwrap(), None);
        assert_eq!(store.shadow_tally().unwrap().reviewed, 0);

        let mut s = store.settings();
        s.junk_filter = true;
        store.save_settings(&s).unwrap();
        add(&store, acct, "b@stranger.example", 0);
        assert_eq!(store.shadow_next().unwrap(), None, "waits for the filter");
        store.restart_junk_filter().unwrap();
        store.filter_junk(50).unwrap();
        assert!(store.shadow_next().unwrap().is_some());
    }

    #[test]
    fn the_tally_compares_the_models_with_your_reports() {
        let (store, acct) = store_with_account();
        on(&store);
        let right = add(&store, acct, "a@stranger.example", 0);
        let wrong = add(&store, acct, "b@stranger.example", 1);
        let missed = add(&store, acct, "c@stranger.example", 2);
        let quiet = add(&store, acct, "d@stranger.example", 3);
        for (id, r) in [(right, PHISHING), (wrong, PHISHING), (missed, CLEAR), (quiet, CLEAR)] {
            store.save_shadow_review(id, &r).unwrap();
        }
        store.report_junk(&[right], Label::Phishing, false).unwrap();
        store.mark_not_junk(&[wrong]).unwrap();
        store.report_junk(&[missed], Label::Spam, false).unwrap();
        assert_eq!(store.shadow_tally().unwrap(), ShadowTally { reviewed: 4, phishing: 2, bulk: 0, confirmed: 1, rejected: 1, missed: 1 });
        store.forget_shadow().unwrap();
        assert_eq!(store.shadow_tally().unwrap(), ShadowTally::default());
        assert!(store.message_label(right).unwrap().is_some(), "your reports stay");
    }

    #[test]
    fn switching_off_mid_review_or_deleting_the_message_leaves_nothing() {
        let (store, acct) = store_with_account();
        on(&store);
        let id = add(&store, acct, "a@stranger.example", 0);
        let mut s = store.settings();
        s.junk_shadow = false;
        store.save_settings(&s).unwrap();
        store.save_shadow_review(id, &PHISHING).unwrap();
        assert_eq!(store.shadow_tally().unwrap().reviewed, 0);
        store.save_shadow_review(MessageId::default(), &PHISHING).unwrap();
    }
}
