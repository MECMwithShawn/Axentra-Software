//! Minimal Microsoft Graph client for OneDrive.
//!
//! Covers what a sync client needs: OAuth sign-in (browser + PKCE, or device
//! code), token refresh, the drive delta feed, chunked uploads, streamed
//! downloads, thumbnails, sharing links and version history.

pub mod auth;
pub mod client;
pub mod quickxor;
pub mod throttle;
pub mod types;

pub use auth::{AuthConfig, DeviceCode, TokenStore, Tokens};
pub use client::{DeltaPage, GResult, Graph, GraphError, UploadTarget};
pub use quickxor::QuickXorHash;
pub use throttle::RateLimiter;
pub use types::*;
