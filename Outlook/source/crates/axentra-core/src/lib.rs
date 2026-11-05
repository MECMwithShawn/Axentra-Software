//! Axentra Outlook core: domain model, local SQLite store, demo data.
//!
//! See `docs/ARCHITECTURE.md` for how the crates fit together.

pub mod backup;
pub mod compose_request;
pub mod error;
pub mod html;
pub mod junk;
pub mod model;
pub mod orders;
pub mod recurrence;
pub(crate) mod scan;
pub mod seed;
pub mod store;
pub mod text;
pub mod topics;

pub use error::{Error, Result};
pub use model::*;
pub use store::{MoveRecord, QueryEmbedder, Store, meaning_text};
pub use store::calendar_time::{local_date, local_to_utc};
pub use store::mail_search::parse_search;
