//! Axentra CloudDrive's one-way mirror: a source copied to a destination,
//! never the other way, and never taking anything away from either.
//!
//! - [`manifest`]: what has been mirrored and how far it was verified, in
//!   its own SQLite file.
//! - [`planner`]: a dry run. Reads the source and the destination and says
//!   what a run would do, making no change anywhere.
//! - [`executor`]: carries a plan out, one file at a time, through a
//!   bounded staging folder, recording each verification level as it is
//!   reached so a run can stop at any point and resume without uploading
//!   anything twice.
//! - [`report`]: what the manifest says, split into what was measured and
//!   what was not.
//!
//! Everything here is generic over [`SourceProvider`] and
//! [`MirrorDestination`], which have no destructive methods at all.
//!
//! [`SourceProvider`]: axentra_clouddrive_provider::SourceProvider
//! [`MirrorDestination`]: axentra_clouddrive_provider::MirrorDestination

pub mod error;
pub mod executor;
pub mod manifest;
pub mod planner;
pub mod report;
pub mod seed;

use std::path::PathBuf;

pub use error::{MResult, MirrorError};
pub use executor::{ExecOptions, Executor, RunOutcome, RunSummary};
pub use manifest::{ItemRow, ItemState, Manifest, RunRow, MANIFEST_VERSION};
pub use planner::{
    plan, plan_with_progress, Action, ChangedSourcePolicy, Plan, PlanEntry, PlanOptions,
    PlanProgress, PlanTotals,
};
pub use report::Readiness;
pub use seed::SeedIndex;

/// Where mirrored files go, under the destination's own root (Proton
/// Drive's `/my-files`), with the source's paths kept beneath it.
pub const DEFAULT_DEST_ROOT: &str = "OneDrive";

/// The most the staging folder may hold at once. A file bigger than this
/// is skipped as too large.
pub const DEFAULT_STAGING_CAP: u64 = 2 << 30;

/// `$XDG_DATA_HOME/axentra/onedrive/mirror.sqlite`. Not created here.
pub fn default_manifest_path() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join("axentra/onedrive/mirror.sqlite"))
}

/// `$XDG_CACHE_HOME/axentra/onedrive/mirror-staging`. Not created here.
pub fn default_staging_dir() -> Option<PathBuf> {
    Some(dirs::cache_dir()?.join("axentra/onedrive/mirror-staging"))
}
