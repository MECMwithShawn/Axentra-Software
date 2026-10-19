//! Axentra Outlook core: domain model, local SQLite store, demo data.
//!
//! See `docs/ARCHITECTURE.md` for how the crates fit together.

pub mod error;
pub mod html;
pub mod model;
pub mod recurrence;
pub mod seed;
pub mod store;
pub mod text;

pub use error::{Error, Result};
pub use model::*;
pub use store::{MoveRecord, Store};
pub use store::calendar_time::{local_date, local_to_utc};
pub use store::mail_search::parse_search;
