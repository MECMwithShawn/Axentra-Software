//! Provider-neutral interfaces for Axentra CloudDrive's one-way mirror.
//!
//! A [`SourceProvider`] is read from (OneDrive, through [`GraphSource`]) and
//! a [`MirrorDestination`] is copied to. Neither trait has a method that
//! deletes, moves, renames or trashes anything, so a mirror built on them
//! cannot express a destructive call. [`ProtonCliDestination`] is Proton
//! Drive through its official CLI, limited to an allowlist of commands, and
//! [`LocalFolderDestination`] a folder on an external drive.
//! [`MemSource`] and [`MemDestination`] are in-memory fakes that log every
//! call and fail on demand.

pub mod digests;
pub mod error;
pub mod graph_source;
pub mod local;
pub mod mem;
pub mod path;
pub mod proton_cli;
pub mod staging;
pub mod traits;

pub use digests::{ContentDigests, VerifyLevel};
pub use error::{PResult, ProviderError};
pub use graph_source::GraphSource;
pub use local::LocalFolderDestination;
pub use mem::{MemDestination, MemSource, Op, MEM_ROOT};
pub use path::{PathError, RelPath};
pub use proton_cli::{
    CliRunner, FakeRunner, ProcessRunner, ProtonCliConfig, ProtonCliDestination, RemotePath,
};
pub use traits::{
    Account, Capabilities, Capability, ChangePage, DestEntry, ItemKind, MirrorDestination,
    RemoteStat, SourceItem, SourceProvider,
};
