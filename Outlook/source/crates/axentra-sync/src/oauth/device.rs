//! Device authorization grant (§3.5, RFC 8628): show a code, the user signs
//! in on any device, we poll the token endpoint at the given interval.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::{OAuthClient, TokenSet, token_request};
use crate::error::{SyncError, SyncResult, copy, from_reqwest};
use crate::secrets::Secret;

#[derive(Clone, Debug)]
pub struct DeviceCode {
    pub user_code: String,
    pub verification_uri: String,
    pub expires_at: DateTime<Utc>,
    pub interval: Duration,
    device_code: Secret,
}

#[derive(Deserialize)]
struct DeviceResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    #[serde(default = "default_expiry")]
    expires_in: i64,
    #[serde(default = "default_interval")]
    interval: u64,
}

fn default_expiry() -> i64 {
    900
}

fn default_interval() -> u64 {
    5
}

/// Ask for a user code.
pub async fn start(http: &reqwest::Client, client: &OAuthClient, scopes: &str) -> SyncResult<DeviceCode> {
    let url = client.device_url.as_deref().ok_or_else(|| SyncError::User("This provider doesn't support sign-in with a code.".into()))?;
    let resp = http
        .post(url)
        .form(&[("client_id", client.client_id.as_str()), ("scope", scopes)])
        .send()
        .await
        .map_err(|e| from_reqwest(&e))?;
    let status = resp.status();
    let body = resp.bytes().await.map_err(|e| from_reqwest(&e))?;
    if !status.is_success() {
        let err: super::TokenError = serde_json::from_slice(&body).unwrap_or_default();
        if err.error_codes.contains(&53003) {
            return Err(SyncError::User(copy::DEVICE_CODE_BLOCKED.into()));
        }
        return Err(super::classify_token_error(&err));
    }
    let d: DeviceResponse = serde_json::from_slice(&body).map_err(|_| SyncError::protocol("The sign-in server sent an unexpected response."))?;
    Ok(DeviceCode {
        user_code: d.user_code,
        verification_uri: d.verification_uri,
        expires_at: Utc::now() + chrono::Duration::seconds(d.expires_in),
        interval: Duration::from_secs(d.interval.max(1)),
        device_code: Secret::new(d.device_code),
    })
}

/// What one poll said.
#[derive(Debug, PartialEq, Eq)]
pub enum PollStep {
    Pending,
    SlowDown,
    Declined,
    Expired,
    Failed,
}

/// Classify a polling `error` (Microsoft's list plus RFC 8628 `slow_down`).
pub fn poll_step(error: &str) -> PollStep {
    match error {
        "authorization_pending" => PollStep::Pending,
        "slow_down" => PollStep::SlowDown,
        "authorization_declined" | "access_denied" => PollStep::Declined,
        "expired_token" | "code_expired" => PollStep::Expired,
        _ => PollStep::Failed,
    }
}

/// Poll until the user finishes, declines or the code expires.
pub async fn poll(http: &reqwest::Client, client: &OAuthClient, code: &DeviceCode) -> SyncResult<TokenSet> {
    let mut interval = code.interval;
    loop {
        tokio::time::sleep(interval).await;
        if Utc::now() >= code.expires_at {
            return Err(SyncError::User(copy::CODE_EXPIRED.into()));
        }
        let result = token_request(
            http,
            client,
            &[("grant_type", "urn:ietf:params:oauth:grant-type:device_code"), ("device_code", code.device_code.expose())],
        )
        .await
        .map_err(|f| *f);
        match result {
            Ok(tokens) => return Ok(tokens),
            Err((_, Some(err))) => match poll_step(&err.error) {
                PollStep::Pending => {}
                PollStep::SlowDown => interval += Duration::from_secs(5),
                PollStep::Declined => return Err(SyncError::User(copy::CANCELLED.into())),
                PollStep::Expired => return Err(SyncError::User(copy::CODE_EXPIRED.into())),
                PollStep::Failed => {
                    if err.error_codes.contains(&53003) {
                        return Err(SyncError::User(copy::DEVICE_CODE_BLOCKED.into()));
                    }
                    return Err(super::classify_token_error(&err));
                }
            },
            // Network blips while polling: keep trying until expiry.
            Err((SyncError::Offline(_) | SyncError::Throttled { .. }, None)) => interval += Duration::from_secs(1),
            Err((e, None)) => return Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poll_errors() {
        assert_eq!(poll_step("authorization_pending"), PollStep::Pending);
        assert_eq!(poll_step("slow_down"), PollStep::SlowDown);
        assert_eq!(poll_step("authorization_declined"), PollStep::Declined);
        assert_eq!(poll_step("expired_token"), PollStep::Expired);
        assert_eq!(poll_step("bad_verification_code"), PollStep::Failed);
    }
}
