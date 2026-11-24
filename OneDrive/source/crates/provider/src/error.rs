//! One error type for every provider, sorted by what the caller can do about it.

use std::time::Duration;

use crate::path::PathError;
use crate::traits::Capability;

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    /// No route to the provider at all. Try again once online.
    #[error("offline: {0}")]
    Offline(String),
    /// A failure that the same request may get past later (a server fault,
    /// a dropped connection).
    #[error("temporary failure: {0}")]
    Transient(String),
    #[error("not found: {0}")]
    NotFound(String),
    /// The name is taken, or the item changed since it was last seen.
    #[error("conflict: {0}")]
    Conflict(String),
    /// The provider asked us to slow down, for this long if it said.
    #[error("rate limited")]
    RateLimited { retry_after: Option<Duration> },
    /// The saved change-feed cursor is too old (OneDrive's HTTP 410): start
    /// again from no cursor and treat everything seen as possibly changed.
    #[error("the change feed cursor expired; a full resync is needed")]
    ResyncRequired,
    /// Bytes did not match the digests they were meant to have. Whatever
    /// was being written has already been thrown away.
    #[error("integrity check failed: {0}")]
    Integrity(String),
    #[error("sign-in required")]
    AuthRequired,
    #[error("not supported by this provider: {0}")]
    Unsupported(Capability),
    #[error(transparent)]
    InvalidPath(#[from] PathError),
    /// A local file could not be read or written (a full disk, say).
    #[error("local file error: {0}")]
    Local(#[from] std::io::Error),
    #[error("{0}")]
    Other(String),
}

pub type PResult<T> = Result<T, ProviderError>;

impl ProviderError {
    /// Worth trying again later without changing anything.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            ProviderError::Offline(_)
                | ProviderError::Transient(_)
                | ProviderError::RateLimited { .. }
        )
    }
}
