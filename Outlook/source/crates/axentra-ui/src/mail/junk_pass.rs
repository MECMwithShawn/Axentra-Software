//! When the background junk pass runs. It checks mail in batches until
//! nothing is left, without needing another change to keep going, and only
//! counts as seen the mail revision it started from: a change that arrives
//! while a batch runs gets a pass of its own. A batch that fails is retried
//! later, and the other messages still get checked (a single message that
//! can't be checked is recorded as incomplete by the store, not skipped).
//! It also schedules `insights_pass` (packages and topics), which runs the
//! same way with a [`JunkPass`] of its own.

use std::time::{Duration, Instant};

use axentra_core::Store;

/// Messages checked per batch.
pub const BATCH: u32 = 250;
/// Pause between batches while there is more to do (keeps the UI responsive).
const BUSY: Duration = Duration::from_millis(150);
/// Pause while idle, between looks at the mail revision.
const IDLE: Duration = Duration::from_secs(2);
/// Pause after a failed batch.
const RETRY: Duration = Duration::from_secs(30);

#[derive(Debug, Default)]
pub struct JunkPass {
    /// The mail revision whose changes are all checked.
    done: Option<u64>,
}

/// What one batch did.
#[derive(Debug, PartialEq)]
pub struct Batch {
    pub model_changed: bool,
    pub checked: usize,
}

impl JunkPass {
    /// True when a batch should run for mail revision `rev`.
    pub fn due(&self, rev: u64) -> bool {
        self.done != Some(rev)
    }

    /// Record a batch that started at `rev`; returns how long to wait before
    /// looking again. Work was done: another batch runs next (and the list
    /// refreshes, see [`Refresh`]). Nothing to do: `rev` is done. Failure:
    /// `rev` is not done, retried after a pause.
    pub fn finished(&mut self, rev: u64, outcome: &Result<Batch, String>) -> Duration {
        match outcome {
            Ok(b) if b.model_changed || b.checked > 0 => BUSY,
            Ok(_) => {
                self.done = Some(rev);
                IDLE
            }
            Err(_) => RETRY,
        }
    }
}

/// How often a background pass refreshes the mail view while it works. A
/// refresh rebuilds the message list and the folder pane: after every batch,
/// a large mailbox kept the window busy (no scrolling, drags that never
/// started) until all of it was checked.
pub const REFRESH_EVERY: Duration = Duration::from_secs(4);

/// When a pass's results show: the first batch at once, then at most every
/// [`REFRESH_EVERY`], and once more when it has finished.
#[derive(Debug, Default)]
pub struct Refresh {
    last: Option<Instant>,
    pending: bool,
}

impl Refresh {
    /// After a look at the pass (`changed`: a batch did something the view
    /// shows): whether to refresh now.
    pub fn now(&mut self, changed: bool, at: Instant) -> bool {
        self.pending |= changed;
        if !self.pending || self.last.is_some_and(|t| at.duration_since(t) < REFRESH_EVERY) {
            return false;
        }
        self.pending = false;
        self.last = Some(at);
        true
    }
}

/// One batch: retrain if the reports changed, check what is due, then
/// move what the junk filter finds clearly junk. A
/// failed training run doesn't hold up the checks; it is tried again with
/// the next batch.
pub fn run_batch(s: &Store) -> Result<Batch, String> {
    let retrain = s.retrain_if_needed();
    let checked = s.assess_pending(BATCH).map_err(|e| e.to_string())? + s.filter_junk(BATCH).map_err(|e| e.to_string())?;
    match retrain {
        Ok(model_changed) => Ok(Batch { model_changed, checked }),
        Err(e) if checked == 0 => Err(e.to_string()),
        Err(_) => Ok(Batch { model_changed: false, checked }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axentra_core::*;

    fn add(s: &Store, acct: AccountId, i: usize) -> MessageId {
        let inbox = s.folders(Some(acct)).unwrap().into_iter().find(|f| f.role == FolderRole::Inbox).unwrap().id;
        let mut m = Message {
            account_id: acct,
            folder_id: inbox,
            from: EmailAddress::new("", format!("p{i}@normal.example")),
            subject: format!("note {i}"),
            received_at: chrono::DateTime::from_timestamp(1_700_000_000 + i as i64, 0).unwrap(),
            ..Default::default()
        };
        s.insert_message(&mut m, Some(&MessageBody { text: "hello".into(), ..Default::default() })).unwrap()
    }

    /// Drive the pass as the UI does, `rev` standing in for the mail
    /// revision (bumped when a batch did work, as the UI refreshes then).
    fn drive(s: &Store, pass: &mut JunkPass, rev: &mut u64, mut during: impl FnMut(usize, &mut u64)) -> usize {
        for batches in 0..1000 {
            if !pass.due(*rev) {
                return batches;
            }
            let started = *rev;
            let out = run_batch(s);
            during(batches, rev);
            if pass.finished(started, &out) == BUSY {
                *rev += 1;
            }
        }
        panic!("the pass never settled");
    }

    #[test]
    fn results_show_at_once_then_every_few_seconds_and_at_the_end() {
        let t0 = Instant::now();
        let mut r = Refresh::default();
        assert!(!r.now(false, t0), "nothing to show");
        assert!(r.now(true, t0), "the first batch shows at once");
        assert!(!r.now(true, t0 + Duration::from_millis(300)));
        assert!(!r.now(true, t0 + Duration::from_secs(2)));
        assert!(r.now(true, t0 + REFRESH_EVERY), "a backlog shows every few seconds");
        // The last batch's results show once the wait is over, though the
        // pass is idle by then.
        assert!(!r.now(true, t0 + REFRESH_EVERY + Duration::from_secs(1)));
        assert!(!r.now(false, t0 + REFRESH_EVERY + Duration::from_secs(2)));
        assert!(r.now(false, t0 + REFRESH_EVERY * 2));
        assert!(!r.now(false, t0 + REFRESH_EVERY * 3), "then rests");
    }

    #[test]
    fn a_backlog_drains_without_further_input_and_then_rests() {
        let s = Store::open_in_memory().unwrap();
        let mut a = Account { kind: AccountKind::Local, email: "me@axentra.example".into(), ..Default::default() };
        let acct = s.add_account(&mut a).unwrap();
        let ids: Vec<MessageId> = (0..(BATCH as usize * 4 + 1)).map(|i| add(&s, acct, i)).collect();
        let (mut pass, mut rev) = (JunkPass::default(), 0);
        let batches = drive(&s, &mut pass, &mut rev, |_, _| {});
        assert!(batches >= 5, "{batches}");
        // Every message, the oldest included, has a current check.
        assert_eq!(s.assess_pending(10).unwrap(), 0);
        let oldest = ids[0];
        assert!(s.assessment(oldest).unwrap().is_some());
        // Idle: no busy loop.
        assert!(!pass.due(rev));
        assert_eq!(drive(&s, &mut pass, &mut rev, |_, _| {}), 0);
    }

    #[test]
    fn mail_arriving_during_a_batch_is_checked_too() {
        let s = Store::open_in_memory().unwrap();
        let mut a = Account { kind: AccountKind::Local, email: "me@axentra.example".into(), ..Default::default() };
        let acct = s.add_account(&mut a).unwrap();
        for i in 0..10 {
            add(&s, acct, i);
        }
        let (mut pass, mut rev) = (JunkPass::default(), 0);
        let mut late = None;
        drive(&s, &mut pass, &mut rev, |n, rev| {
            if n == 0 {
                // New mail lands while the first batch runs: the app bumps
                // the revision, the batch's own start revision isn't it.
                late = Some(add(&s, acct, 99));
                *rev += 1;
            }
        });
        assert_eq!(s.assess_pending(10).unwrap(), 0, "the late message was checked");
        assert!(late.is_some());
    }

    #[test]
    fn a_failed_batch_is_retried_not_acknowledged() {
        let mut pass = JunkPass::default();
        assert_eq!(pass.finished(3, &Err("disk full".into())), RETRY);
        assert!(pass.due(3), "a failure doesn't count as checked");
        assert_eq!(pass.finished(3, &Ok(Batch { model_changed: false, checked: 0 })), IDLE);
        assert!(!pass.due(3));
    }
}
