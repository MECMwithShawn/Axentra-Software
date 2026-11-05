//! Packages and topics in the background: extract orders, regroup them,
//! sort topics, newest mail first, off the UI thread. Resumable: pending
//! work is whatever the store has no current row for. Scheduled like the
//! junk pass, with its [`JunkPass`](super::junk_pass::JunkPass).

use axentra_core::Store;

use super::junk_pass::Batch;

/// Messages extracted (and sorted) per batch.
pub const BATCH: u32 = 250;
/// Changed order keys regrouped per batch.
const REBUILD: u32 = 200;

/// One batch: orders found and regrouped (when Packages is on), then
/// topics sorted (when Topics is on). Both off: nothing to do.
pub fn run_batch(s: &Store) -> Result<Batch, String> {
    let st = s.settings();
    let now = chrono::Utc::now();
    let mut checked = 0;
    if st.order_tracking {
        checked += s.extract_orders_pending(BATCH, now).map_err(|e| e.to_string())?;
        checked += s.rebuild_dirty_orders(REBUILD).map_err(|e| e.to_string())?;
    }
    if st.topics_view {
        checked += s.classify_topics_pending(BATCH).map_err(|e| e.to_string())?;
    }
    Ok(Batch { model_changed: false, checked })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail::junk_pass::JunkPass;
    use axentra_core::*;

    fn store() -> (Store, AccountId) {
        let s = Store::open_in_memory().unwrap();
        let mut a = Account { kind: AccountKind::Local, email: "me@axentra.example".into(), ..Default::default() };
        let acct = s.add_account(&mut a).unwrap();
        (s, acct)
    }

    fn add(s: &Store, acct: AccountId, i: usize) -> MessageId {
        let inbox = s.folders(Some(acct)).unwrap().into_iter().find(|f| f.role == FolderRole::Inbox).unwrap().id;
        let mut m = Message {
            account_id: acct,
            folder_id: inbox,
            from: EmailAddress::new("", format!("p{i}@normal.example")),
            subject: format!("note {i}"),
            received_at: chrono::Utc::now() - chrono::Duration::minutes(i as i64),
            ..Default::default()
        };
        s.insert_message(&mut m, Some(&MessageBody { text: "hello".into(), ..Default::default() })).unwrap()
    }

    /// Drive the pass as the UI does, `rev` standing in for the mail
    /// revision (bumped when a batch did work, as the UI refreshes then).
    /// The junk pass runs alongside, as in the app: topics wait for its
    /// checks, and its work bumps the revision too.
    fn drive(s: &Store, pass: &mut JunkPass, rev: &mut u64, mut during: impl FnMut(usize, &mut u64)) -> usize {
        for batches in 0..1000 {
            if super::super::junk_pass::run_batch(s).is_ok_and(|b| b.checked > 0) {
                *rev += 1;
            }
            if !pass.due(*rev) {
                return batches;
            }
            let started = *rev;
            let out = run_batch(s);
            during(batches, rev);
            let worked = out.as_ref().is_ok_and(|b| b.checked > 0);
            pass.finished(started, &out);
            if worked {
                *rev += 1;
            }
        }
        panic!("the pass never settled");
    }

    fn pending(s: &Store) -> (u32, u32) {
        (s.orders_pending_count(10, chrono::Utc::now()).unwrap(), s.topics_pending_count(10).unwrap())
    }

    #[test]
    fn a_backlog_drains_without_further_input_and_then_rests() {
        let (s, acct) = store();
        let ids: Vec<MessageId> = (0..(BATCH as usize * 4 + 1)).map(|i| add(&s, acct, i)).collect();
        assert_eq!(pending(&s), (10, 0), "topics wait for the junk check");
        let (mut pass, mut rev) = (JunkPass::default(), 0);
        let batches = drive(&s, &mut pass, &mut rev, |_, _| {});
        assert!(batches >= 5, "{batches}");
        assert_eq!(pending(&s), (0, 0));
        // The oldest message too.
        assert!(s.extraction_for(ids[ids.len() - 1]).unwrap().is_some());
        assert!(s.topics_for(&[ids[ids.len() - 1]]).unwrap().contains_key(&ids[ids.len() - 1]));
        // Idle: no busy loop.
        assert!(!pass.due(rev));
        assert_eq!(drive(&s, &mut pass, &mut rev, |_, _| {}), 0);
        assert_eq!(run_batch(&s), Ok(Batch { model_changed: false, checked: 0 }));
    }

    #[test]
    fn mail_arriving_during_a_batch_is_handled_too() {
        let (s, acct) = store();
        for i in 0..10 {
            add(&s, acct, i);
        }
        let (mut pass, mut rev) = (JunkPass::default(), 0);
        let mut late = None;
        drive(&s, &mut pass, &mut rev, |n, rev| {
            if n == 0 {
                late = Some(add(&s, acct, 99));
                *rev += 1;
            }
        });
        assert_eq!(pending(&s), (0, 0), "the late message was handled");
        assert!(s.extraction_for(late.unwrap()).unwrap().is_some());
    }

    #[test]
    fn a_failed_batch_is_retried_not_acknowledged() {
        let (s, acct) = store();
        add(&s, acct, 0);
        let mut pass = JunkPass::default();
        pass.finished(3, &Err("disk full".into()));
        assert!(pass.due(3), "a failure doesn't count as done");
        // The retry does the work, then rests.
        let mut rev = 3;
        assert!(drive(&s, &mut pass, &mut rev, |_, _| {}) >= 1);
        assert_eq!(pending(&s), (0, 0));
        assert!(!pass.due(rev));
    }

    #[test]
    fn both_off_does_nothing() {
        let (s, acct) = store();
        add(&s, acct, 0);
        let mut st = s.settings();
        st.order_tracking = false;
        st.topics_view = false;
        s.save_settings(&st).unwrap();
        assert_eq!(run_batch(&s), Ok(Batch { model_changed: false, checked: 0 }));
        assert_eq!(pending(&s), (0, 0), "off: nothing is pending");
    }

    #[test]
    fn orders_off_writes_no_order_rows_while_topics_still_sort() {
        let (s, acct) = store();
        let id = add(&s, acct, 0);
        let mut st = s.settings();
        st.order_tracking = false;
        s.save_settings(&st).unwrap();
        let (mut pass, mut rev) = (JunkPass::default(), 0);
        drive(&s, &mut pass, &mut rev, |_, _| {});
        assert!(s.extraction_for(id).unwrap().is_none());
        assert!(s.topics_for(&[id]).unwrap().contains_key(&id));
    }
}
