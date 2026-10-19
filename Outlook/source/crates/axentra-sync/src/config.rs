//! Account connection settings stored in `Account.config` (never secrets:
//! passwords and refresh tokens live in the OS keyring, see `secrets.rs`),
//! provider presets and the account health shown in the UI.

use axentra_core::{Account, AccountKind};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Which service an account talks to (docs/spec/protocols.md §1.2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    /// Microsoft 365 work or school account (Entra tenant).
    MicrosoftWork,
    /// Outlook.com, Hotmail, Live, MSN.
    MicrosoftPersonal,
    Google,
    ICloud,
    Yahoo,
    /// Any other IMAP + SMTP server.
    #[default]
    Imap,
}

impl Provider {
    pub fn label(self) -> &'static str {
        match self {
            Provider::MicrosoftWork => "Microsoft 365",
            Provider::MicrosoftPersonal => "Outlook.com",
            Provider::Google => "Gmail",
            Provider::ICloud => "iCloud",
            Provider::Yahoo => "Yahoo",
            Provider::Imap => "IMAP",
        }
    }

    pub fn is_microsoft(self) -> bool {
        matches!(self, Provider::MicrosoftWork | Provider::MicrosoftPersonal)
    }

    /// Value of the keyring `provider` attribute (§7.2).
    pub fn keyring_name(self) -> &'static str {
        match self {
            Provider::MicrosoftWork | Provider::MicrosoftPersonal => "microsoft",
            Provider::Google => "google",
            _ => "imap",
        }
    }
}

/// Transport security for IMAP / SMTP (§5.1). Plain text is never offered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Security {
    /// Implicit TLS (IMAP 993, SMTP 465).
    #[default]
    Tls,
    /// Plain connection upgraded with STARTTLS (IMAP 143, SMTP 587).
    StartTls,
}

impl Security {
    pub fn label(self) -> &'static str {
        match self {
            Security::Tls => "SSL/TLS",
            Security::StartTls => "STARTTLS",
        }
    }

    pub fn default_imap_port(self) -> u16 {
        match self {
            Security::Tls => 993,
            Security::StartTls => 143,
        }
    }

    pub fn default_smtp_port(self) -> u16 {
        match self {
            Security::Tls => 465,
            Security::StartTls => 587,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMethod {
    /// IMAP/SMTP password or app password (SASL PLAIN / LOGIN over TLS).
    #[default]
    Password,
    /// Microsoft Graph with a Microsoft identity platform token.
    OauthGraph,
    /// IMAP/SMTP XOAUTH2 with a Microsoft token (Graph-less fallback).
    Xoauth2Microsoft,
    /// IMAP/SMTP XOAUTH2 with a Google token.
    Xoauth2Google,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub security: Security,
    /// Sign-in name; empty means "the email address".
    pub username: String,
}

/// Whether the app must APPEND a copy of sent mail to Sent (§5.9).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SentCopyPolicy {
    /// The server files submitted mail itself (Gmail, Microsoft 365).
    Server,
    /// The app appends the transmitted message to Sent.
    ClientAppend,
    /// Unknown: check Sent after the first send.
    #[default]
    Detect,
}

/// Connection health (§2.6), persisted so the UI can show it at startup.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountState {
    #[default]
    Ok,
    Syncing,
    Offline,
    Throttled,
    NeedsReauth,
    KeyringLocked,
    Error,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AccountConfig {
    /// Stable id used as the keyring `account_id` attribute.
    pub uuid: String,
    pub provider: Provider,
    pub auth: AuthMethod,
    pub imap: Option<ServerConfig>,
    pub smtp: Option<ServerConfig>,
    /// SMTP signs in with the IMAP credentials.
    pub smtp_same_auth: bool,
    /// Microsoft: application (client) id of the Entra app registration.
    pub ms_client_id: Option<String>,
    /// Microsoft: authority segment for refreshes (`{tid}` for work
    /// accounts, `consumers` for personal ones).
    pub ms_tenant: Option<String>,
    pub ms_oid: Option<String>,
    pub google_client_id: Option<String>,
    pub sent_copy_policy: SentCopyPolicy,
    /// Post-authentication IMAP capabilities, for diagnostics.
    pub capabilities: Vec<String>,
    pub state: AccountState,
    /// Extra text for the state (e.g. the reason of an error).
    pub state_detail: Option<String>,
    /// How many days of mail to keep in sync (Settings > Offline default).
    pub days_to_sync: u32,
    /// The user dismissed the health message bar for this state.
    pub bar_dismissed: bool,
    pub added_at: Option<DateTime<Utc>>,
    /// When `state` last changed (throttling stays silent for a while).
    pub state_since: Option<DateTime<Utc>>,
}

/// Throttling shows a message bar only after this long (§2.6, estimate).
pub const THROTTLE_QUIET: chrono::Duration = chrono::Duration::minutes(5);

impl Default for AccountConfig {
    fn default() -> Self {
        Self {
            uuid: String::new(),
            provider: Provider::Imap,
            auth: AuthMethod::Password,
            imap: None,
            smtp: None,
            smtp_same_auth: true,
            ms_client_id: None,
            ms_tenant: None,
            ms_oid: None,
            google_client_id: None,
            sent_copy_policy: SentCopyPolicy::Detect,
            capabilities: Vec::new(),
            state: AccountState::Ok,
            state_detail: None,
            days_to_sync: 30,
            bar_dismissed: false,
            added_at: None,
            state_since: None,
        }
    }
}

impl AccountConfig {
    /// Parse `account.config` (missing or malformed fields take defaults).
    pub fn of(account: &Account) -> Self {
        serde_json::from_value(account.config.clone()).unwrap_or_default()
    }

    pub fn store_into(&self, account: &mut Account) {
        account.config = serde_json::to_value(self).unwrap_or_default();
    }

    /// The IMAP sign-in name (defaults to the address).
    pub fn imap_username<'a>(&'a self, email: &'a str) -> &'a str {
        match &self.imap {
            Some(s) if !s.username.is_empty() => &s.username,
            _ => email,
        }
    }

    pub fn smtp_username<'a>(&'a self, email: &'a str) -> &'a str {
        match &self.smtp {
            Some(s) if !s.username.is_empty() => &s.username,
            _ => self.imap_username(email),
        }
    }
}

// ---------------------------------------------------------------- presets

/// Well-known server settings (§5.1 "Provider presets").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preset {
    pub provider: Provider,
    pub imap: ServerConfig,
    pub smtp: ServerConfig,
    /// The IMAP user name is the part before `@` (iCloud).
    pub imap_local_part: bool,
    /// Where the provider lets users create an app password.
    pub app_password_url: Option<&'static str>,
}

fn server(host: &str, port: u16, security: Security) -> ServerConfig {
    ServerConfig { host: host.into(), port, security, username: String::new() }
}

pub fn preset(provider: Provider) -> Option<Preset> {
    let p = |imap, smtp, local, url| Preset { provider, imap, smtp, imap_local_part: local, app_password_url: url };
    match provider {
        Provider::Google => Some(p(
            server("imap.gmail.com", 993, Security::Tls),
            server("smtp.gmail.com", 465, Security::Tls),
            false,
            Some("https://myaccount.google.com/apppasswords"),
        )),
        Provider::ICloud => Some(p(
            server("imap.mail.me.com", 993, Security::Tls),
            server("smtp.mail.me.com", 587, Security::StartTls),
            true,
            Some("https://account.apple.com/account/manage"),
        )),
        Provider::Yahoo => Some(p(
            server("imap.mail.yahoo.com", 993, Security::Tls),
            server("smtp.mail.yahoo.com", 465, Security::Tls),
            false,
            Some("https://login.yahoo.com/account/security"),
        )),
        // IMAP fallback hosts for Microsoft accounts (XOAUTH2 only). Never
        // "upgrade" Microsoft SMTP to 465: it only speaks STARTTLS on 587.
        Provider::MicrosoftWork => Some(p(
            server("outlook.office365.com", 993, Security::Tls),
            server("smtp.office365.com", 587, Security::StartTls),
            false,
            None,
        )),
        Provider::MicrosoftPersonal => Some(p(
            server("outlook.office365.com", 993, Security::Tls),
            server("smtp-mail.outlook.com", 587, Security::StartTls),
            false,
            None,
        )),
        Provider::Imap => None,
    }
}

/// Lower-cased domain of an address, if it looks like one.
pub fn domain_of(email: &str) -> Option<String> {
    let (local, domain) = email.trim().rsplit_once('@')?;
    let domain = domain.trim().to_ascii_lowercase();
    (!local.is_empty() && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.')).then_some(domain)
}

/// Light validation used to enable "Continue": one `@`, a dotted domain, no spaces.
pub fn is_valid_email(email: &str) -> bool {
    let e = email.trim();
    !e.contains(char::is_whitespace) && e.matches('@').count() == 1 && domain_of(e).is_some()
}

/// Provider by domain (step 1 of account detection, §1.3).
pub fn provider_for_domain(domain: &str) -> Option<Provider> {
    let d = domain.to_ascii_lowercase();
    let first = d.split('.').next().unwrap_or_default();
    match d.as_str() {
        "gmail.com" | "googlemail.com" => Some(Provider::Google),
        "icloud.com" | "me.com" | "mac.com" => Some(Provider::ICloud),
        "msn.com" | "passport.com" => Some(Provider::MicrosoftPersonal),
        "ymail.com" | "rocketmail.com" => Some(Provider::Yahoo),
        _ => match first {
            "outlook" | "hotmail" | "live" | "windowslive" => Some(Provider::MicrosoftPersonal),
            "yahoo" => Some(Provider::Yahoo),
            _ => None,
        },
    }
}

/// Default manual-form guess for an unknown domain.
pub fn guess_servers(domain: &str) -> (ServerConfig, ServerConfig) {
    (server(&format!("imap.{domain}"), 993, Security::Tls), server(&format!("smtp.{domain}"), 465, Security::Tls))
}

// ---------------------------------------------------------------- health

/// What the UI shows for an account's connection (§2.6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountHealth {
    pub state: AccountState,
    /// Trailing glyph in the folder pane, `None` when healthy.
    pub icon: Option<&'static str>,
    /// Message bar text, `None` when no bar is needed.
    pub message: Option<String>,
    /// Message bar intent: "info", "warning" or "error".
    pub intent: &'static str,
    /// Primary action label ("Sign in", "Unlock", "Retry").
    pub action: Option<&'static str>,
}

impl AccountHealth {
    pub fn of(account: &Account) -> Self {
        Self::at(account, Utc::now())
    }

    pub fn at(account: &Account, now: DateTime<Utc>) -> Self {
        let cfg = AccountConfig::of(account);
        let name = if account.email.is_empty() { account.display_name.clone() } else { account.email.clone() };
        let state = if account.kind == AccountKind::Local { AccountState::Ok } else { cfg.state };
        let (icon, message, intent, action) = match state {
            AccountState::Ok | AccountState::Syncing => (None, None, "info", None),
            AccountState::Offline => (Some("cloud_off"), Some("You're offline. Changes will sync when you reconnect.".to_string()), "info", None),
            AccountState::Throttled => (
                None,
                // Silent unless it lasts (§2.6).
                cfg.state_since
                    .is_some_and(|t| now - t >= THROTTLE_QUIET)
                    .then(|| format!("{} is limiting requests. Syncing will resume shortly.", cfg.provider.label())),
                "warning",
                None,
            ),
            AccountState::NeedsReauth => (Some("warning"), Some(format!("Sign in again to keep {name} up to date.")), "warning", Some("Sign in")),
            AccountState::KeyringLocked => (Some("lock_closed"), Some(format!("Your keyring is locked. Unlock it to sync {name}.")), "warning", Some("Unlock")),
            AccountState::Error => {
                let reason = cfg.state_detail.clone().or_else(|| account.sync_error.clone()).unwrap_or_default();
                (Some("error_circle"), Some(format!("We can't connect to {name}. {reason}").trim().to_string()), "error", Some("Retry"))
            }
        };
        AccountHealth { state, icon, message, intent, action }
    }
}

/// Short status line for the accounts list, e.g. "Updated 5 minutes ago".
pub fn status_line(account: &Account, now: DateTime<Utc>) -> String {
    if account.kind == AccountKind::Local {
        return "Stored on this device only".into();
    }
    let cfg = AccountConfig::of(account);
    match cfg.state {
        AccountState::Syncing => "Syncing…".into(),
        AccountState::Offline => "Offline".into(),
        AccountState::Throttled => "Waiting for the server".into(),
        AccountState::NeedsReauth => "Sign-in needed".into(),
        AccountState::KeyringLocked => "Keyring locked".into(),
        AccountState::Error => "Can't connect".into(),
        AccountState::Ok => match account.last_sync_at {
            None => "Not synced yet".into(),
            Some(t) => {
                let mins = (now - t).num_minutes();
                match mins {
                    i64::MIN..=0 => "Updated just now".into(),
                    1 => "Updated 1 minute ago".into(),
                    2..=59 => format!("Updated {mins} minutes ago"),
                    60..=119 => "Updated 1 hour ago".into(),
                    120..=1439 => format!("Updated {} hours ago", mins / 60),
                    _ => format!("Updated {}", t.with_timezone(&chrono::Local).format("%-m/%-d/%Y")),
                }
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domains_and_presets() {
        assert_eq!(domain_of(" Ana@Example.ORG "), Some("example.org".into()));
        assert_eq!(domain_of("ana@localhost"), None);
        assert!(is_valid_email("ana@example.org"));
        assert!(!is_valid_email("ana@@example.org"));
        assert!(!is_valid_email("ana example@org.com"));
        assert_eq!(provider_for_domain("gmail.com"), Some(Provider::Google));
        assert_eq!(provider_for_domain("hotmail.co.uk"), Some(Provider::MicrosoftPersonal));
        assert_eq!(provider_for_domain("outlook.fr"), Some(Provider::MicrosoftPersonal));
        assert_eq!(provider_for_domain("yahoo.co.jp"), Some(Provider::Yahoo));
        assert_eq!(provider_for_domain("me.com"), Some(Provider::ICloud));
        assert_eq!(provider_for_domain("contoso.com"), None);
        let ms = preset(Provider::MicrosoftWork).unwrap();
        assert_eq!((ms.smtp.port, ms.smtp.security), (587, Security::StartTls));
        assert!(preset(Provider::ICloud).unwrap().imap_local_part);
        assert_eq!(Security::StartTls.default_smtp_port(), 587);
    }

    #[test]
    fn config_round_trip_and_health() {
        let mut acct = Account { kind: AccountKind::Imap, email: "ana@example.org".into(), ..Default::default() };
        let mut cfg = AccountConfig::of(&acct);
        assert_eq!(cfg.days_to_sync, 30);
        assert!(cfg.smtp_same_auth);
        cfg.imap = Some(ServerConfig { host: "imap.example.org".into(), port: 993, ..Default::default() });
        cfg.state = AccountState::NeedsReauth;
        cfg.store_into(&mut acct);
        assert_eq!(AccountConfig::of(&acct), cfg);
        assert_eq!(cfg.imap_username("ana@example.org"), "ana@example.org");
        let h = AccountHealth::of(&acct);
        assert_eq!(h.action, Some("Sign in"));
        assert_eq!(h.message.as_deref(), Some("Sign in again to keep ana@example.org up to date."));
        assert_eq!(status_line(&acct, Utc::now()), "Sign-in needed");
        cfg.state = AccountState::Ok;
        cfg.store_into(&mut acct);
        acct.last_sync_at = Some(Utc::now() - chrono::Duration::minutes(5));
        assert_eq!(status_line(&acct, Utc::now()), "Updated 5 minutes ago");
        assert_eq!(AccountHealth::of(&acct).message, None);

        // Throttling is quiet for a few minutes.
        cfg.state = AccountState::Throttled;
        cfg.state_since = Some(Utc::now());
        cfg.store_into(&mut acct);
        assert_eq!(AccountHealth::of(&acct).message, None);
        let later = Utc::now() + THROTTLE_QUIET;
        assert_eq!(AccountHealth::at(&acct, later).message.as_deref(), Some("IMAP is limiting requests. Syncing will resume shortly."));
    }
}
