//! Contacts over Graph (§4.10): the default Contacts folder is listed in full
//! on each contacts pass (30 min cadence), so deletions show up as absence;
//! local creates / edits / deletes are pushed back.

use std::collections::{HashMap, HashSet};

use axentra_core::{Account, Contact, ContactId, Store};

use super::client::{Graph, seg};
use super::map::{contact_from_graph, contact_to_graph};
use crate::diff::{ItemOp, KIND_CONTACT, Shadow, contact_fingerprint, item_op};
use crate::error::{SyncError, SyncResult};
use crate::notify::Notifier;

const CONTACTS: &str = "default";

pub async fn sync(graph: &Graph, store: &Store, account: &Account, notifier: &Notifier) -> SyncResult<usize> {
    let account_id = account.id;
    let remote = graph.get_all("/me/contacts?$top=100").await?;
    let mut locals: HashMap<ContactId, Contact> = store.account_contacts(account_id)?.into_iter().map(|c| (c.id, c)).collect();
    let mut changed = 0;
    let mut seen: HashSet<String> = HashSet::new();

    for v in &remote {
        let Some(rid) = v["id"].as_str() else { continue };
        seen.insert(rid.to_string());
        let change_key = v["changeKey"].as_str().map(str::to_string);
        match store.sync_find::<Shadow>(account_id, KIND_CONTACT, rid)? {
            Some((lid, mut sh)) => {
                let Some(local) = locals.get(&lid) else { continue };
                if sh.etag == change_key {
                    continue;
                }
                if contact_fingerprint(local) != sh.state {
                    // Local edit pending: it is pushed below.
                    sh.etag = change_key;
                    store.sync_put(account_id, KIND_CONTACT, lid, rid, CONTACTS, &sh)?;
                    continue;
                }
                let Some(c) = contact_from_graph(v, account_id, Some(local)) else { continue };
                store.update_contact(&c)?;
                let stored = store.contact(lid)?.unwrap_or(c);
                sh.state = contact_fingerprint(&stored);
                sh.etag = change_key;
                store.sync_put(account_id, KIND_CONTACT, lid, rid, CONTACTS, &sh)?;
                locals.insert(lid, stored);
                changed += 1;
            }
            None => {
                let Some(mut c) = contact_from_graph(v, account_id, None) else { continue };
                if c.display_name.trim().is_empty() && c.emails.is_empty() && c.company.is_empty() {
                    continue;
                }
                let lid = store.insert_contact(&mut c)?;
                let stored = store.contact(lid)?.unwrap_or(c);
                let sh = Shadow { remote_id: rid.to_string(), container: CONTACTS.into(), etag: change_key, state: contact_fingerprint(&stored), ..Default::default() };
                store.sync_put(account_id, KIND_CONTACT, lid, rid, CONTACTS, &sh)?;
                locals.insert(lid, stored);
                changed += 1;
            }
        }
    }

    // Deleted on the server.
    let shadows: HashMap<ContactId, Shadow> = store.sync_list::<Shadow>(account_id, KIND_CONTACT, None)?.into_iter().collect();
    for (lid, sh) in &shadows {
        if !seen.contains(&sh.remote_id) && locals.contains_key(lid) {
            store.delete_contact(*lid)?;
            store.sync_remove(account_id, KIND_CONTACT, *lid)?;
            locals.remove(lid);
            changed += 1;
        }
    }

    changed += push_with(graph, store, account, &locals).await?;
    if changed > 0 {
        notifier.bump();
    }
    Ok(changed)
}

/// Push local contact changes without reading the server list.
pub async fn push(graph: &Graph, store: &Store, account: &Account, notifier: &Notifier) -> SyncResult<usize> {
    let locals: HashMap<ContactId, Contact> = store.account_contacts(account.id)?.into_iter().map(|c| (c.id, c)).collect();
    let changed = push_with(graph, store, account, &locals).await?;
    if changed > 0 {
        notifier.bump();
    }
    Ok(changed)
}

async fn push_with(graph: &Graph, store: &Store, account: &Account, locals: &HashMap<ContactId, Contact>) -> SyncResult<usize> {
    let account_id = account.id;
    let mut changed = 0;
    let shadows: HashMap<ContactId, Shadow> = store.sync_list::<Shadow>(account_id, KIND_CONTACT, None)?.into_iter().collect();
    let mut ids: Vec<ContactId> = locals.keys().chain(shadows.keys()).copied().collect::<HashSet<_>>().into_iter().collect();
    ids.sort_unstable();
    for id in ids {
        let local = locals.get(&id);
        let shadow = shadows.get(&id);
        let fp = local.map(contact_fingerprint);
        let result: SyncResult<()> = match item_op(fp.as_ref(), shadow) {
            ItemOp::None => continue,
            ItemOp::Create => {
                let c = local.expect("create");
                match graph.post("/me/contacts", contact_to_graph(c)).await {
                    Ok(created) => {
                        let rid = created["id"].as_str().unwrap_or_default().to_string();
                        let sh = Shadow {
                            remote_id: rid.clone(),
                            container: CONTACTS.into(),
                            etag: created["changeKey"].as_str().map(str::to_string),
                            state: fp.clone().unwrap_or_default(),
                            ..Default::default()
                        };
                        store.sync_put(account_id, KIND_CONTACT, id, &rid, CONTACTS, &sh).map_err(SyncError::from)
                    }
                    Err(e) => Err(e),
                }
            }
            ItemOp::Update => {
                let (c, sh) = (local.expect("update"), shadow.expect("update"));
                match graph.patch(&format!("/me/contacts/{}", seg(&sh.remote_id)), contact_to_graph(c)).await {
                    Ok(updated) => {
                        let sh = Shadow { state: fp.clone().unwrap_or_default(), etag: updated["changeKey"].as_str().map(str::to_string), ..sh.clone() };
                        store.sync_put(account_id, KIND_CONTACT, id, &sh.remote_id, CONTACTS, &sh).map_err(SyncError::from)
                    }
                    Err(e) => Err(e),
                }
            }
            ItemOp::Delete => {
                let sh = shadow.expect("delete");
                match graph.delete(&format!("/me/contacts/{}", seg(&sh.remote_id))).await {
                    Ok(()) => store.sync_remove(account_id, KIND_CONTACT, id).map_err(SyncError::from),
                    Err(e) => Err(e),
                }
            }
        };
        match result {
            Ok(()) => changed += 1,
            Err(SyncError::NotFound) => {
                store.sync_remove(account_id, KIND_CONTACT, id)?;
            }
            Err(e) => return Err(e),
        }
    }
    Ok(changed)
}
