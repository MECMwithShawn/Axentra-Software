//! Push for INBOX with IMAP IDLE (RFC 2177): one dedicated connection that
//! re-issues IDLE every 25 minutes and pokes the engine whenever the server
//! reports EXISTS / EXPUNGE / FETCH. Reconnects with backoff (1 s → 60 s).

use std::future::Future;
use std::time::Duration;

use async_imap::extensions::idle::IdleResponse;

use super::{Credential, drain_unsolicited, imap_err, open};
use crate::config::ServerConfig;
use crate::error::{SyncError, SyncResult};

const IDLE_RENEW: Duration = Duration::from_secs(25 * 60);

/// Why the watcher stopped for good.
#[derive(Debug)]
pub enum IdleEnd {
    /// The server has no IDLE: poll instead.
    Unsupported,
    /// Credentials stopped working; the next full sync reports it.
    Auth(SyncError),
}

/// Watch INBOX until the credentials fail or the server lacks IDLE.
/// `credential` is asked for a fresh credential on every (re)connect.
pub async fn watch<C, CF>(server: ServerConfig, user: String, mut credential: C, mut on_change: impl FnMut()) -> IdleEnd
where
    C: FnMut() -> CF,
    CF: Future<Output = SyncResult<Credential>>,
{
    let mut backoff = Duration::from_secs(1);
    loop {
        let result = async {
            let cred = credential().await?;
            let imap = open(&server, &user, &cred).await?;
            if !imap.has("IDLE") {
                return Ok(false);
            }
            let mut session = imap.session;
            session.select("INBOX").await.map_err(|e| imap_err(e, &server.host))?;
            drain_unsolicited(&mut session);
            loop {
                let mut handle = session.idle();
                handle.init().await.map_err(|e| imap_err(e, &server.host))?;
                let (wait, _stop) = handle.wait_with_timeout(IDLE_RENEW);
                let response = wait.await.map_err(|e| imap_err(e, &server.host))?;
                session = handle.done().await.map_err(|e| imap_err(e, &server.host))?;
                drain_unsolicited(&mut session);
                if matches!(response, IdleResponse::NewData(_)) {
                    on_change();
                }
            }
            #[allow(unreachable_code)]
            Ok::<bool, SyncError>(true)
        }
        .await;
        match result {
            Ok(false) => return IdleEnd::Unsupported,
            Ok(true) => backoff = Duration::from_secs(1),
            Err(e @ (SyncError::Auth(_) | SyncError::Keyring(_))) => return IdleEnd::Auth(e),
            Err(e) => {
                tracing::debug!("IDLE connection to {} dropped: {e}", server.host);
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(60));
            }
        }
    }
}
