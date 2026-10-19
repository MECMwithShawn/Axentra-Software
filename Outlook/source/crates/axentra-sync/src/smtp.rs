//! SMTP submission (§5.9) with lettre: 465 implicit TLS or 587 STARTTLS,
//! SASL PLAIN / LOGIN or XOAUTH2. The message bytes come from
//! `mime::build`, so what goes out is exactly what lands in Sent.

use std::time::Duration;

use lettre::address::{Address, Envelope};
use lettre::transport::smtp::authentication::{Credentials, Mechanism};
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::transport::smtp::extension::ClientId;
use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};

use crate::config::{Security, ServerConfig};
use crate::error::{SyncError, SyncResult, copy};
use crate::secrets::Secret;

const TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone, Debug)]
pub enum SmtpAuth {
    Password { user: String, password: Secret },
    Xoauth2 { user: String, token: Secret },
}

fn transport(server: &ServerConfig, auth: &SmtpAuth, plaintext_for_tests: bool) -> SyncResult<AsyncSmtpTransport<Tokio1Executor>> {
    let host = server.host.trim();
    if host.is_empty() {
        return Err(SyncError::User("Enter the outgoing mail (SMTP) server.".into()));
    }
    let tls = if plaintext_for_tests {
        Tls::None
    } else {
        let params = TlsParameters::new(host.to_string()).map_err(|e| SyncError::protocol(format!("{host}: {e}")))?;
        match server.security {
            Security::Tls => Tls::Wrapper(params),
            Security::StartTls => Tls::Required(params),
        }
    };
    let (credentials, mechanisms) = match auth {
        SmtpAuth::Password { user, password } => {
            (Credentials::new(user.clone(), password.expose().to_string()), vec![Mechanism::Plain, Mechanism::Login])
        }
        SmtpAuth::Xoauth2 { user, token } => (Credentials::new(user.clone(), token.expose().to_string()), vec![Mechanism::Xoauth2]),
    };
    let hello = hostname_id();
    Ok(AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host)
        .port(server.port)
        .tls(tls)
        .hello_name(hello)
        .credentials(credentials)
        .authentication(mechanisms)
        .timeout(Some(TIMEOUT))
        .build())
}

fn hostname_id() -> ClientId {
    // `EHLO {fqdn-or-[ip]}`: the default resolves the machine's host name.
    ClientId::default()
}

/// Map an SMTP failure to the user-facing classes of §5.9.
pub fn classify(e: &lettre::transport::smtp::Error, host: &str, from: &str) -> SyncError {
    let text = e.to_string();
    let code = e.status().map(|c| c.to_string()).unwrap_or_default();
    if e.is_tls() {
        return SyncError::Tls { message: copy::TLS_UNTRUSTED.into(), detail: format!("Server: {host}\n{text}") };
    }
    if text.contains("5.7.139") {
        return SyncError::User(copy::SMTP_DISABLED.into());
    }
    if text.contains("5.7.60") {
        return SyncError::User(copy::no_send_as(from));
    }
    if code == "552" || text.contains("5.3.4") {
        return SyncError::User(copy::TOO_LARGE.into());
    }
    if code == "535" || code == "534" || code == "530" {
        return SyncError::Auth(copy::IMAP_AUTH.into());
    }
    if e.is_transient() {
        return SyncError::Throttled { retry_after: None };
    }
    if e.is_permanent() {
        return SyncError::protocol(format!("The outgoing mail server didn't accept the message: {text}"));
    }
    if text.contains("Connection refused") || text.contains("lookup") || text.contains("resolve") {
        return SyncError::Offline(copy::unreachable(host));
    }
    if e.is_timeout() || text.contains("connection") {
        return SyncError::Offline(copy::OFFLINE.into());
    }
    SyncError::protocol(format!("{host}: {text}"))
}

async fn send_inner(server: &ServerConfig, auth: &SmtpAuth, from: &str, recipients: &[String], bytes: &[u8], plaintext_for_tests: bool) -> SyncResult<()> {
    let t = transport(server, auth, plaintext_for_tests)?;
    let from_addr: Address = from.parse().map_err(|_| SyncError::User(format!("“{from}” isn't a valid sender address.")))?;
    let mut to = Vec::new();
    for r in recipients {
        to.push(r.parse::<Address>().map_err(|_| SyncError::User(format!("“{r}” isn't a valid email address.")))?);
    }
    let envelope = Envelope::new(Some(from_addr), to).map_err(|_| SyncError::User("Add at least one recipient.".into()))?;
    t.send_raw(&envelope, bytes).await.map_err(|e| classify(&e, &server.host, from))?;
    Ok(())
}

/// Submit one message.
pub async fn send(server: &ServerConfig, auth: &SmtpAuth, from: &str, recipients: &[String], bytes: &[u8]) -> SyncResult<()> {
    send_inner(server, auth, from, recipients, bytes, false).await
}

/// Connect, (STARTTLS,) authenticate and quit: the "Test connection" check.
pub async fn test(server: &ServerConfig, auth: &SmtpAuth) -> SyncResult<()> {
    let t = transport(server, auth, false)?;
    match t.test_connection().await {
        Ok(true) => Ok(()),
        Ok(false) => Err(SyncError::Offline(copy::OFFLINE.into())),
        Err(e) => Err(classify(&e, &server.host, "")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpListener;

    /// A tiny SMTP server that accepts one message and returns what it got.
    async fn fake_server(listener: TcpListener, reject_auth: bool) -> (Vec<String>, String) {
        let (sock, _) = listener.accept().await.unwrap();
        let (r, mut w) = sock.into_split();
        let mut lines = BufReader::new(r).lines();
        let mut commands = Vec::new();
        let mut data = String::new();
        w.write_all(b"220 fake.example ESMTP\r\n").await.unwrap();
        while let Some(line) = lines.next_line().await.unwrap() {
            let upper = line.to_ascii_uppercase();
            commands.push(line.clone());
            let reply: &[u8] = if upper.starts_with("EHLO") {
                b"250-fake.example\r\n250-AUTH PLAIN LOGIN\r\n250 8BITMIME\r\n"
            } else if upper.starts_with("AUTH") {
                if reject_auth { b"535 5.7.8 Username and Password not accepted\r\n" } else { b"235 2.7.0 Accepted\r\n" }
            } else if upper.starts_with("MAIL") || upper.starts_with("RCPT") {
                b"250 OK\r\n"
            } else if upper == "DATA" {
                w.write_all(b"354 Go ahead\r\n").await.unwrap();
                while let Some(l) = lines.next_line().await.unwrap() {
                    if l == "." {
                        break;
                    }
                    data.push_str(&l);
                    data.push('\n');
                }
                b"250 2.0.0 Queued\r\n"
            } else if upper == "QUIT" {
                w.write_all(b"221 Bye\r\n").await.unwrap();
                break;
            } else {
                b"250 OK\r\n"
            };
            w.write_all(reply).await.unwrap();
        }
        (commands, data)
    }

    #[tokio::test]
    async fn submits_envelope_with_bcc_and_reports_auth_failures() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(fake_server(listener, false));
        let cfg = ServerConfig { host: "127.0.0.1".into(), port, security: Security::StartTls, username: String::new() };
        let auth = SmtpAuth::Password { user: "ana@example.org".into(), password: Secret::new("pw") };
        let msg = b"From: ana@example.org\r\nTo: bob@example.org\r\nSubject: Hi\r\n\r\nHello\r\n";
        let rcpts = vec!["bob@example.org".to_string(), "hidden@example.org".to_string()];
        send_inner(&cfg, &auth, "ana@example.org", &rcpts, msg, true).await.unwrap();
        let (commands, data) = server.await.unwrap();
        assert!(commands.iter().any(|c| c.starts_with("MAIL FROM:<ana@example.org>")));
        assert!(commands.iter().any(|c| c.starts_with("RCPT TO:<hidden@example.org>")), "Bcc recipients go in the envelope");
        assert!(data.contains("Subject: Hi"));
        assert!(!data.contains("hidden@example.org"));

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(fake_server(listener, true));
        let cfg = ServerConfig { port, ..cfg };
        let err = send_inner(&cfg, &auth, "ana@example.org", &rcpts, msg, true).await.unwrap_err();
        assert!(matches!(err, SyncError::Auth(_)), "{err:?}");
        server.abort();
    }
}
