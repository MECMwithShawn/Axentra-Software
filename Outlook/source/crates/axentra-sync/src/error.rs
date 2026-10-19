//! Engine errors and the user-facing copy for them (docs/spec/protocols.md
//! §2.4, §3.6, §5.9). Messages are complete sentences in Outlook's voice so
//! the UI can show them as-is.

use std::time::Duration;

use crate::config::AccountState;

/// Copy shared by several call sites.
pub mod copy {
    pub const OFFLINE: &str = "You're offline. Connect to the internet and try again.";
    pub const IMAP_AUTH: &str = "We couldn't sign in. Check your password. Some providers need an app password.";
    pub const TLS_UNTRUSTED: &str = "The server's security certificate isn't trusted.";
    pub const ALREADY_PRESENT: &str = "This account is already in Axentra Outlook.";
    pub const SIGNIN_TIMEOUT: &str = "Sign-in timed out. Please try again.";
    pub const ADMIN_CONSENT: &str = "Your organization needs to approve Axentra Outlook before you can sign in.";
    pub const CANCELLED: &str = "Sign-in was cancelled.";
    pub const CA_BLOCKED: &str = "Your organization's sign-in policy blocked this app.";
    pub const DEVICE_CODE_BLOCKED: &str = "Your organization doesn't allow sign-in with a code. Use your browser instead.";
    pub const CODE_EXPIRED: &str = "The code expired. Get a new code.";
    pub const NO_OUTLOOK_COM_MAILBOX: &str = "This Microsoft account doesn't have an Outlook.com mailbox.";
    pub const KEYRING_UNAVAILABLE: &str = "Axentra Outlook can't find a keyring to store your sign-in securely. \
        Start a Secret Service keyring (such as GNOME Keyring, KWallet or KeePassXC) and try again. \
        Passwords are never saved in plain text.";
    pub const KEYRING_DENIED: &str = "Access to the keyring was denied, so your sign-in couldn't be saved.";
    pub const SMTP_DISABLED: &str = "Sending is turned off for this account by your organization.";
    pub const TOO_LARGE: &str = "This message is too large to send.";
    pub const MAILBOX_FULL: &str = "Your mailbox is full.";
    pub const NO_CLIENT_ID: &str = "Microsoft sign-in isn't set up yet. Enter the application (client) ID of your \
        Microsoft Entra app registration under Advanced, or set AXENTRA_MS_CLIENT_ID. See docs/ACCOUNTS.md.";

    pub fn no_mailbox(email: &str) -> String {
        format!("We couldn't find a mailbox for {email} in Microsoft 365.")
    }
    pub fn keyring_locked(email: &str) -> String {
        format!("Your keyring is locked. Unlock it to sync {email}.")
    }
    /// A typed server that refused the connection or doesn't resolve: most
    /// often a typo, sometimes no network.
    pub fn unreachable(host: &str) -> String {
        format!("We couldn't connect to {host}. Check the server name and port, and your internet connection.")
    }
    pub fn no_send_as(from: &str) -> String {
        format!("You don't have permission to send as {from}.")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyringError {
    /// No Secret Service provider (or no session bus).
    Unavailable,
    /// The collection holding the secret is locked and we may not prompt.
    Locked,
    /// The user dismissed the unlock / confirmation prompt.
    Denied,
}

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    /// No network, DNS failure, refused connection or timeout.
    #[error("{0}")]
    Offline(String),
    /// Credentials rejected (wrong password, revoked token): sign in again.
    #[error("{0}")]
    Auth(String),
    /// The server's certificate failed validation. `detail` holds host and
    /// SHA-256 fingerprint for the details expander.
    #[error("{message}")]
    Tls { message: String, detail: String },
    #[error("{}", keyring_text(.0))]
    Keyring(KeyringError),
    /// HTTP 429 / 503 (or IMAP/SMTP equivalents) with an optional hint.
    #[error("The server is limiting requests. Syncing will resume shortly.")]
    Throttled { retry_after: Option<Duration> },
    /// An item vanished on the server (Graph 404 / ErrorItemNotFound).
    #[error("not found on the server")]
    NotFound,
    /// A delta token expired (HTTP 410 / `syncStateNotFound`): resync that
    /// one collection from scratch.
    #[error("the server asked for a full resync")]
    ResyncRequired,
    /// A condition with fixed user-facing copy (setup errors, policy blocks).
    #[error("{0}")]
    User(String),
    /// Unexpected server behaviour; the text is shown after "We can't connect".
    #[error("{0}")]
    Protocol(String),
    #[error("{0}")]
    Store(#[from] axentra_core::Error),
    #[error("cancelled")]
    Cancelled,
}

fn keyring_text(e: &KeyringError) -> &'static str {
    match e {
        KeyringError::Unavailable => copy::KEYRING_UNAVAILABLE,
        KeyringError::Locked => "Your keyring is locked.",
        KeyringError::Denied => copy::KEYRING_DENIED,
    }
}

pub type SyncResult<T> = std::result::Result<T, SyncError>;

impl SyncError {
    pub fn protocol(msg: impl Into<String>) -> Self {
        SyncError::Protocol(msg.into())
    }

    /// The account state an error leaves the account in.
    pub fn account_state(&self) -> AccountState {
        match self {
            SyncError::Offline(_) => AccountState::Offline,
            SyncError::Auth(_) => AccountState::NeedsReauth,
            SyncError::Keyring(KeyringError::Locked) => AccountState::KeyringLocked,
            SyncError::Throttled { .. } => AccountState::Throttled,
            SyncError::Cancelled => AccountState::Ok,
            _ => AccountState::Error,
        }
    }

    /// Whether retrying later may help (network blips, throttling, 5xx).
    pub fn is_transient(&self) -> bool {
        matches!(self, SyncError::Offline(_) | SyncError::Throttled { .. })
    }

    /// Short reason used in "We can't connect to {email}. {reason}".
    pub fn short_reason(&self) -> String {
        match self {
            SyncError::Tls { message, .. } => message.clone(),
            other => other.to_string(),
        }
    }
}

/// Map an I/O error from a socket to the right class.
pub fn from_io(e: &std::io::Error, host: &str) -> SyncError {
    use std::io::ErrorKind::*;
    if let Some(inner) = e.get_ref() {
        let text = inner.to_string();
        if text.contains("certificate") || text.contains("UnknownIssuer") || text.contains("NotValidForName") {
            return SyncError::Tls { message: copy::TLS_UNTRUSTED.into(), detail: format!("Server: {host}\n{text}") };
        }
    }
    match e.kind() {
        ConnectionRefused | HostUnreachable => SyncError::Offline(copy::unreachable(host)),
        ConnectionReset | ConnectionAborted | NotConnected | TimedOut | NetworkUnreachable | NetworkDown | AddrNotAvailable => {
            SyncError::Offline(copy::OFFLINE.into())
        }
        _ => {
            let text = e.to_string();
            // getaddrinfo failures surface as `Other` / `Uncategorized`.
            if text.contains("failed to lookup address") || text.contains("Name or service not known") || text.contains("No address associated") {
                SyncError::Offline(copy::unreachable(host))
            } else if text.contains("Temporary failure in name resolution") {
                SyncError::Offline(copy::OFFLINE.into())
            } else {
                SyncError::Protocol(format!("{host}: {text}"))
            }
        }
    }
}

/// Map a reqwest transport error.
pub fn from_reqwest(e: &reqwest::Error) -> SyncError {
    if e.is_timeout() || e.is_connect() {
        let text = format!("{e:?}");
        if text.contains("certificate") || text.contains("UnknownIssuer") {
            return SyncError::Tls { message: copy::TLS_UNTRUSTED.into(), detail: text };
        }
        return SyncError::Offline(copy::OFFLINE.into());
    }
    if e.is_decode() {
        return SyncError::protocol(format!("Unexpected response from the server ({e})."));
    }
    SyncError::protocol(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_follow_error_class() {
        assert_eq!(SyncError::Auth("x".into()).account_state(), AccountState::NeedsReauth);
        assert_eq!(SyncError::Keyring(KeyringError::Locked).account_state(), AccountState::KeyringLocked);
        assert_eq!(SyncError::Offline(copy::OFFLINE.into()).account_state(), AccountState::Offline);
        assert_eq!(SyncError::protocol("bad").account_state(), AccountState::Error);
        assert!(SyncError::Throttled { retry_after: None }.is_transient());
        let refused = std::io::Error::from(std::io::ErrorKind::ConnectionRefused);
        let e = from_io(&refused, "imap.example.org");
        assert!(matches!(e, SyncError::Offline(_)) && e.is_transient());
        assert!(e.to_string().starts_with("We couldn't connect to imap.example.org."));
        let down = std::io::Error::from(std::io::ErrorKind::NetworkUnreachable);
        assert_eq!(from_io(&down, "x").to_string(), copy::OFFLINE);
        assert!(SyncError::Keyring(KeyringError::Unavailable).to_string().contains("keyring"));
    }
}
