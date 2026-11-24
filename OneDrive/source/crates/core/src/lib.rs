//! Axentra CloudDrive core: local state, the sync engine, the Files On-Demand mount and
//! the D-Bus service the tray, flyout and file manager talk to.

pub mod cloud_thumbs;
pub mod config;
pub mod daemon;
pub mod db;
pub mod engine;
pub mod fs;
pub mod ipc;
pub mod names;
pub mod notify;
pub mod status;
pub mod thumbs;

pub use config::{Config, Paths};

/// The product name people see. Display text only: binaries, paths, D-Bus names,
/// xattrs and the icon keep their `axentra-onedrive` / `AxentraOneDrive` ids.
pub const BRAND: &str = "Axentra CloudDrive";
pub use engine::{Engine, Options};
