//! What can stop the mirror as a whole, as opposed to one file.

use axentra_clouddrive_provider::ProviderError;

#[derive(Debug, thiserror::Error)]
pub enum MirrorError {
    #[error("mirror manifest: {0}")]
    Db(#[from] rusqlite::Error),
    /// Written by a newer version of this program. Not touched, so that
    /// version can still read it.
    #[error(
        "the mirror manifest is version {found}, newer than this program understands \
         ({supported}); it was left unchanged"
    )]
    NewerManifest { found: i64, supported: i64 },
    #[error("the mirror manifest holds something unexpected: {0}")]
    Corrupt(String),
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error("staging folder: {0}")]
    Staging(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Invalid(String),
}

pub type MResult<T> = Result<T, MirrorError>;
