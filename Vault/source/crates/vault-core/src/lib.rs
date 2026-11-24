//! Axentra Vault core: KeePass (KDBX4) databases via the `keepass` crate, the passkey key
//! envelope shared with the KeePass plugin, backup key files, the versioned backup store and
//! password generation.

pub mod backups;
pub mod categorize;
pub mod envelope;
pub mod fidelity;
pub mod form;
#[cfg(windows)]
pub mod fsops;
#[cfg(target_os="linux")]
#[path="fsops_linux.rs"]
pub mod fsops;
pub mod generation;
pub mod generator;
pub mod header;
pub mod keyfile;
pub mod limits;
pub mod manage;
pub mod organize;
pub mod rekey;
pub mod search;
pub mod store;
pub mod vault;

pub use keepass;

/// Fills `buf` from the operating system CSPRNG. Panics only if the OS RNG is unavailable,
/// in which case no secret material can be generated safely anyway.
pub fn random_bytes(buf: &mut [u8]) {
    getrandom::fill(buf).expect("operating system random number generator failed");
}
