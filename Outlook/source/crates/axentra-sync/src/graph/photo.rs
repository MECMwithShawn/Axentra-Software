//! The signed-in user's own profile photo (§4.5), shown instead of initials
//! on the account avatars. Best effort: nothing here fails a sync pass,
//! changes an account's state or asks for a new sign-in.
//!
//! `GET /me/photo` gives the metadata (`@odata.mediaEtag`, width, height);
//! a 404 (whatever the error code) or the 1×1 placeholder means "no photo",
//! and an unchanged etag skips the download. The bytes come from
//! `/me/photos/{n}x{n}/$value` (96 to 240 px: crisp at 48 px on HiDPI) when
//! the photo is that large, else from `/me/photo/$value` (the largest),
//! which is also the fallback when a sized request answers 404 / 400 (sizes
//! are documented for Microsoft 365, not for personal accounts). Only JPEG,
//! PNG, GIF and WebP up to 1 MiB are kept, recognised by their bytes; a
//! larger download stops at the limit, and a photo that wasn't kept isn't
//! downloaded again until its etag changes. Checked right after sign-in,
//! then at most every [`RECHECK`]; a failed check keeps the stored photo and
//! is tried again after [`RETRY`]. What a check finds is saved only for the
//! account it began with (see [`Store::set_account_photo`]).

use axentra_core::{AccountId, AccountPhoto, Store};
use base64::Engine as _;
use chrono::{DateTime, Duration, Utc};
use serde_json::Value;

use super::Graph;
use super::client::graph_error;
use crate::error::SyncError;

/// How often a photo is checked again.
pub const RECHECK: Duration = Duration::hours(6);
/// How soon a failed check is tried again.
pub const RETRY: Duration = Duration::minutes(30);
/// The largest photo kept, in bytes.
pub const MAX_BYTES: usize = 1024 * 1024;
/// Sized photos to ask for, largest first.
const SIZES: [u64; 3] = [240, 120, 96];

/// What a check did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Checked recently; nothing was requested.
    NotDue,
    /// The server still has the stored photo.
    Unchanged,
    /// A new or changed photo was stored.
    Stored,
    /// The photo was removed on the server, so the stored one went too.
    Removed,
    /// The account has no photo (and had none here).
    NoPhoto,
    /// The download wasn't a usable image, or is one that wasn't usable
    /// before; the stored photo was kept.
    Rejected,
    /// A request failed; the stored photo was kept.
    Failed,
    /// The account was removed during the check; nothing was saved.
    Gone,
}

impl Outcome {
    /// The avatars need to update.
    pub fn changed(self) -> bool {
        matches!(self, Outcome::Stored | Outcome::Removed)
    }
}

/// Whether a check is due at `now` for this stored record.
pub fn due(stored: Option<&AccountPhoto>, now: DateTime<Utc>) -> bool {
    let Some(p) = stored else { return true };
    // A time in the future (the clock moved back) counts as long ago.
    let older = |t: Option<DateTime<Utc>>, d: Duration| t.is_none_or(|t| t > now || now - t >= d);
    older(p.checked_at, RECHECK) && older(p.failed_at, RETRY)
}

/// The image type of `bytes`, from their magic number.
pub fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// Photo bytes as a `data:image/…;base64,…` URL, or why they can't be shown.
/// The server's content type is ignored: the bytes decide.
pub fn data_url(bytes: &[u8]) -> Result<String, String> {
    if bytes.len() > MAX_BYTES {
        return Err(format!("{} bytes is more than 1 MiB", bytes.len()));
    }
    let mime = sniff(bytes).ok_or("not a JPEG, PNG, GIF or WebP image")?;
    Ok(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

/// Graph answers `GET /me/photo` with a 1×1 placeholder when there is no photo.
fn is_placeholder(meta: &Value) -> bool {
    meta["id"].as_str() == Some("1x1") || (meta["width"].as_u64().is_some_and(|w| w <= 1) && meta["height"].as_u64().is_some_and(|h| h <= 1))
}

/// The sized photo to ask for, given the metadata of the largest one.
fn sized_path(meta: &Value) -> Option<String> {
    let side = meta["width"].as_u64()?.min(meta["height"].as_u64()?);
    SIZES.iter().find(|s| **s <= side).map(|s| format!("/me/photos/{s}x{s}/$value"))
}

enum Check {
    NoPhoto,
    Unchanged,
    Photo { etag: String, url: String },
    /// Downloaded and not usable.
    Rejected { etag: String, why: String },
    /// The photo that was rejected before: not downloaded again.
    StillRejected,
    Failed(SyncError),
}

/// Ask the server; compare with `stored`.
async fn check(graph: &Graph, stored: &AccountPhoto) -> Check {
    let meta = match graph.get_status("/me/photo").await {
        Ok((200..=299, body)) => serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null),
        Ok((404, _)) => return Check::NoPhoto,
        Ok((status, body)) => return Check::Failed(graph_error(status, &body)),
        Err(e) => return Check::Failed(e),
    };
    if is_placeholder(&meta) {
        return Check::NoPhoto;
    }
    let etag = meta["@odata.mediaEtag"].as_str().unwrap_or_default().to_string();
    if !etag.is_empty() && etag == stored.etag && stored.data_url.is_some() {
        return Check::Unchanged;
    }
    if !etag.is_empty() && etag == stored.rejected_etag {
        return Check::StillRejected;
    }
    if let Some(path) = sized_path(&meta) {
        match graph.get_status_max(&path, MAX_BYTES).await {
            Ok((200..=299, bytes)) => return photo(etag, bytes),
            // No sized photos for this account: take the full one.
            Ok((400 | 404, _)) => {}
            Ok((status, body)) => return Check::Failed(graph_error(status, &body.unwrap_or_default())),
            Err(e) => return Check::Failed(e),
        }
    }
    match graph.get_status_max("/me/photo/$value", MAX_BYTES).await {
        Ok((200..=299, bytes)) => photo(etag, bytes),
        Ok((404, _)) => Check::NoPhoto,
        Ok((status, body)) => Check::Failed(graph_error(status, &body.unwrap_or_default())),
        Err(e) => Check::Failed(e),
    }
}

/// A downloaded photo; `None` when it was larger than [`MAX_BYTES`].
fn photo(etag: String, bytes: Option<Vec<u8>>) -> Check {
    match bytes.ok_or_else(|| "more than 1 MiB".to_string()).and_then(|b| data_url(&b)) {
        Ok(url) => Check::Photo { etag, url },
        Err(why) => Check::Rejected { etag, why },
    }
}

/// Check the account's photo against the server (when due, or always with
/// `force`, right after sign-in) and store the result. Never fails: errors
/// are logged and keep the stored photo.
pub async fn refresh(graph: &Graph, store: &Store, account_id: AccountId, now: DateTime<Utc>, force: bool) -> Outcome {
    // The account this check is for: if it is removed meanwhile (and maybe
    // replaced by a new account with the same id), nothing is saved.
    let Some(account) = store.account(account_id).ok().flatten() else { return Outcome::Gone };
    let stored = store.account_photo(account_id).ok().flatten();
    if !force && !due(stored.as_ref(), now) {
        return Outcome::NotDue;
    }
    let mut record = stored.unwrap_or_default();
    let before = record.data_url.clone();
    let outcome = match check(graph, &record).await {
        Check::NoPhoto => {
            tracing::debug!("account {account_id}: no profile photo");
            record.etag.clear();
            record.data_url = None;
            record.rejected_etag.clear();
            if before.is_some() { Outcome::Removed } else { Outcome::NoPhoto }
        }
        Check::Unchanged => Outcome::Unchanged,
        Check::Photo { etag, url } => {
            record.etag = etag;
            record.rejected_etag.clear();
            let same = before.as_deref() == Some(url.as_str());
            record.data_url = Some(url);
            if same { Outcome::Unchanged } else { Outcome::Stored }
        }
        Check::Rejected { etag, why } => {
            // Not downloaded again until the server has another photo.
            tracing::warn!("account {account_id}: profile photo ignored ({why})");
            record.rejected_etag = etag;
            Outcome::Rejected
        }
        Check::StillRejected => {
            tracing::debug!("account {account_id}: the profile photo is still the one that was ignored");
            Outcome::Rejected
        }
        Check::Failed(e) => {
            if e.is_transient() {
                tracing::debug!("account {account_id}: couldn't check the profile photo: {e}");
            } else {
                tracing::warn!("account {account_id}: couldn't check the profile photo: {e}");
            }
            Outcome::Failed
        }
    };
    if outcome == Outcome::Failed {
        record.failed_at = Some(now);
    } else {
        record.checked_at = Some(now);
        record.failed_at = None;
    }
    match store.set_account_photo(&account, &record) {
        Ok(true) => outcome,
        Ok(false) => {
            tracing::debug!("account {account_id} was removed during its profile photo check");
            Outcome::Gone
        }
        Err(e) => {
            tracing::warn!("account {account_id}: couldn't save the profile photo: {e}");
            Outcome::Failed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(h: i64) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-20T00:00:00Z").unwrap().with_timezone(&Utc) + Duration::hours(h)
    }

    #[test]
    fn sniffing_and_limits() {
        assert_eq!(sniff(&[0xff, 0xd8, 0xff, 0xe0, 0]), Some("image/jpeg"));
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n...."), Some("image/png"));
        assert_eq!(sniff(b"GIF89a.."), Some("image/gif"));
        assert_eq!(sniff(b"RIFF\x10\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(sniff(b"RIFF\x10\0\0\0WAVEfmt "), None);
        assert_eq!(sniff(b"<svg xmlns='http://www.w3.org/2000/svg'/>"), None);
        assert_eq!(sniff(b""), None);
        assert_eq!(data_url(b"GIF87a").unwrap(), "data:image/gif;base64,R0lGODdh");
        let mut big = b"\x89PNG\r\n\x1a\n".to_vec();
        big.resize(MAX_BYTES, 0);
        assert!(data_url(&big).is_ok());
        big.push(0);
        assert!(data_url(&big).unwrap_err().contains("1 MiB"));
    }

    #[test]
    fn schedule() {
        assert!(due(None, at(0)));
        let p = AccountPhoto { checked_at: Some(at(0)), ..Default::default() };
        assert!(!due(Some(&p), at(5)) && due(Some(&p), at(6)));
        // A failure is retried after 30 minutes, not 6 hours.
        let failed = AccountPhoto { checked_at: Some(at(-24)), failed_at: Some(at(0)), ..Default::default() };
        assert!(!due(Some(&failed), at(0) + Duration::minutes(29)) && due(Some(&failed), at(0) + Duration::minutes(30)));
        // A check "in the future" doesn't block checks until then.
        assert!(due(Some(&AccountPhoto { checked_at: Some(at(48)), ..Default::default() }), at(0)));
    }

    #[test]
    fn placeholder_and_sizes() {
        let placeholder = serde_json::json!({"id":"1x1","width":1,"height":1,"@odata.mediaContentType":"image/gif","@odata.mediaEtag":""});
        assert!(is_placeholder(&placeholder));
        assert!(!is_placeholder(&serde_json::json!({"id":"648X648","width":648,"height":648})));
        assert_eq!(sized_path(&serde_json::json!({"width":648,"height":648})).as_deref(), Some("/me/photos/240x240/$value"));
        assert_eq!(sized_path(&serde_json::json!({"width":200,"height":150})).as_deref(), Some("/me/photos/120x120/$value"));
        assert_eq!(sized_path(&serde_json::json!({"width":96,"height":96})).as_deref(), Some("/me/photos/96x96/$value"));
        assert_eq!(sized_path(&serde_json::json!({"width":64,"height":64})), None, "smaller than 96: the full photo");
        assert_eq!(sized_path(&serde_json::json!({})), None);
    }
}
