//! IMAP4rev1 client side (docs/spec/protocols.md §5): connection over
//! implicit TLS or STARTTLS, SASL PLAIN / LOGIN / XOAUTH2, capability
//! probing, folder discovery and the per-folder sync.

pub mod folders;
pub mod idle;
pub mod sync;

use std::collections::HashSet;
use std::fmt;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use async_imap::imap_proto::{Capability, Response, ResponseCode, Status};
use async_imap::types::UnsolicitedResponse;
use async_imap::{Client, Session};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;
use tokio_rustls::rustls::pki_types::ServerName;

use crate::config::{Security, ServerConfig};
use crate::error::{SyncError, SyncResult, copy, from_io};
use crate::secrets::Secret;

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);

/// A TCP stream before or after the TLS upgrade.
pub enum ImapStream {
    Plain(TcpStream),
    Tls(Box<TlsStream<TcpStream>>),
}

impl fmt::Debug for ImapStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ImapStream::Plain(_) => "ImapStream::Plain",
            ImapStream::Tls(_) => "ImapStream::Tls",
        })
    }
}

impl AsyncRead for ImapStream {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            ImapStream::Plain(s) => Pin::new(s).poll_read(cx, buf),
            ImapStream::Tls(s) => Pin::new(s.as_mut()).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for ImapStream {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        match self.get_mut() {
            ImapStream::Plain(s) => Pin::new(s).poll_write(cx, buf),
            ImapStream::Tls(s) => Pin::new(s.as_mut()).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            ImapStream::Plain(s) => Pin::new(s).poll_flush(cx),
            ImapStream::Tls(s) => Pin::new(s.as_mut()).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            ImapStream::Plain(s) => Pin::new(s).poll_shutdown(cx),
            ImapStream::Tls(s) => Pin::new(s.as_mut()).poll_shutdown(cx),
        }
    }
}

/// Anything async-imap can drive (TCP/TLS in the app, an in-memory duplex
/// pipe in tests).
pub trait Transport: AsyncRead + AsyncWrite + Unpin + fmt::Debug + Send + 'static {}
impl<T: AsyncRead + AsyncWrite + Unpin + fmt::Debug + Send + 'static> Transport for T {}

/// How to sign in.
#[derive(Clone, Debug)]
pub enum Credential {
    Password(Secret),
    /// OAuth access token for SASL XOAUTH2.
    Bearer(Secret),
}

/// An authenticated session plus what we learned about the server.
pub struct Imap<T: Transport> {
    pub session: Session<T>,
    /// Upper-cased capability atoms after authentication.
    pub caps: HashSet<String>,
}

impl<T: Transport> Imap<T> {
    pub fn has(&self, cap: &str) -> bool {
        self.caps.contains(cap)
    }
}

pub(crate) fn imap_err(e: async_imap::error::Error, host: &str) -> SyncError {
    use async_imap::error::Error as E;
    match e {
        E::Io(io) => from_io(&io, host),
        E::ConnectionLost => SyncError::Offline(copy::OFFLINE.into()),
        E::No(m) | E::Bad(m) => SyncError::protocol(format!("{host}: {m}")),
        other => SyncError::protocol(format!("{host}: {other}")),
    }
}

async fn tls_wrap(tcp: TcpStream, host: &str) -> SyncResult<TlsStream<TcpStream>> {
    let (config, rejected) = crate::tls::client_config();
    let name = ServerName::try_from(host.to_string()).map_err(|_| SyncError::User(format!("“{host}” isn't a valid server name.")))?;
    match tokio::time::timeout(CONNECT_TIMEOUT, TlsConnector::from(config).connect(name, tcp)).await {
        Err(_) => Err(SyncError::Offline(copy::OFFLINE.into())),
        Ok(Ok(s)) => Ok(s),
        Ok(Err(e)) => {
            if let Some(detail) = rejected.lock().unwrap_or_else(|p| p.into_inner()).take() {
                return Err(SyncError::Tls { message: copy::TLS_UNTRUSTED.into(), detail: format!("Server: {host}\n{detail}") });
            }
            Err(from_io(&e, host))
        }
    }
}

pub(crate) async fn read_greeting<T: Transport>(client: &mut Client<T>, host: &str) -> SyncResult<Vec<String>> {
    let greeting = tokio::time::timeout(COMMAND_TIMEOUT, client.read_response())
        .await
        .map_err(|_| SyncError::Offline(copy::OFFLINE.into()))?
        .map_err(|e| from_io(&e, host))?
        .ok_or_else(|| SyncError::Offline(copy::OFFLINE.into()))?;
    match greeting.parsed() {
        Response::Data { status: Status::Ok | Status::PreAuth, code, .. } => Ok(match code {
            Some(ResponseCode::Capabilities(caps)) => caps.iter().map(cap_name).collect(),
            _ => Vec::new(),
        }),
        Response::Data { information, .. } => Err(SyncError::protocol(format!(
            "{host} refused the connection: {}",
            information.as_deref().unwrap_or("no reason given")
        ))),
        _ => Err(SyncError::protocol(format!("{host} didn't send an IMAP greeting."))),
    }
}

fn cap_name(c: &Capability<'_>) -> String {
    match c {
        Capability::Imap4rev1 => "IMAP4REV1".into(),
        Capability::Auth(m) => format!("AUTH={}", m.to_ascii_uppercase()),
        Capability::Atom(a) => a.to_ascii_uppercase(),
    }
}

/// Open a connection (TLS from the first byte, or STARTTLS) and read the
/// greeting. Never authenticates in clear text.
pub async fn connect(server: &ServerConfig) -> SyncResult<(Client<ImapStream>, Vec<String>)> {
    let host = server.host.trim();
    if host.is_empty() {
        return Err(SyncError::User("Enter the incoming mail (IMAP) server.".into()));
    }
    let tcp = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect((host, server.port)))
        .await
        .map_err(|_| SyncError::Offline(copy::OFFLINE.into()))?
        .map_err(|e| from_io(&e, host))?;
    match server.security {
        Security::Tls => {
            let tls = tls_wrap(tcp, host).await?;
            let mut client = Client::new(ImapStream::Tls(Box::new(tls)));
            let caps = read_greeting(&mut client, host).await?;
            Ok((client, caps))
        }
        Security::StartTls => {
            let mut client = Client::new(ImapStream::Plain(tcp));
            read_greeting(&mut client, host).await?;
            client
                .run_command_and_check_ok("STARTTLS", None)
                .await
                .map_err(|_| SyncError::User(format!("{host} doesn't support STARTTLS. Choose SSL/TLS instead.")))?;
            let ImapStream::Plain(tcp) = client.into_inner() else { unreachable!("plain before STARTTLS") };
            let tls = tls_wrap(tcp, host).await?;
            // No greeting after STARTTLS; capabilities must be re-read.
            Ok((Client::new(ImapStream::Tls(Box::new(tls))), Vec::new()))
        }
    }
}

/// `CAPABILITY` on an unauthenticated connection.
async fn client_capabilities<T: Transport>(client: &mut Client<T>, host: &str) -> SyncResult<Vec<String>> {
    let (tx, rx) = async_channel::unbounded::<UnsolicitedResponse>();
    tokio::time::timeout(COMMAND_TIMEOUT, client.run_command_and_check_ok("CAPABILITY", Some(tx)))
        .await
        .map_err(|_| SyncError::Offline(copy::OFFLINE.into()))?
        .map_err(|e| imap_err(e, host))?;
    let mut out = Vec::new();
    while let Ok(UnsolicitedResponse::Other(r)) = rx.try_recv() {
        if let Response::Capabilities(caps) = r.parsed() {
            out.extend(caps.iter().map(cap_name));
        }
    }
    Ok(out)
}

struct Plain<'a> {
    user: &'a str,
    pass: &'a str,
}

impl async_imap::Authenticator for Plain<'_> {
    type Response = Vec<u8>;
    fn process(&mut self, _challenge: &[u8]) -> Vec<u8> {
        format!("\0{}\0{}", self.user, self.pass).into_bytes()
    }
}

/// SASL XOAUTH2. On failure Google sends a JSON challenge that must be
/// answered with an empty response before the tagged NO arrives (§5.2).
struct XOAuth2<'a> {
    user: &'a str,
    token: &'a str,
    sent: bool,
}

impl async_imap::Authenticator for XOAuth2<'_> {
    type Response = Vec<u8>;
    fn process(&mut self, _challenge: &[u8]) -> Vec<u8> {
        if self.sent {
            return Vec::new();
        }
        self.sent = true;
        xoauth2_string(self.user, self.token).into_bytes()
    }
}

/// The SASL XOAUTH2 initial client response (before base64).
pub fn xoauth2_string(user: &str, token: &str) -> String {
    format!("user={user}\x01auth=Bearer {token}\x01\x01")
}

/// Authenticate and return the session with post-auth capabilities.
pub async fn login<T: Transport>(mut client: Client<T>, mut pre_caps: Vec<String>, host: &str, user: &str, credential: &Credential) -> SyncResult<Imap<T>> {
    if pre_caps.is_empty() {
        pre_caps = client_capabilities(&mut client, host).await?;
    }
    let has = |c: &str| pre_caps.iter().any(|x| x == c);
    let result = match credential {
        Credential::Bearer(token) => {
            let auth = XOAuth2 { user, token: token.expose(), sent: false };
            tokio::time::timeout(COMMAND_TIMEOUT, client.authenticate("XOAUTH2", auth)).await
        }
        Credential::Password(pass) if has("AUTH=PLAIN") => {
            let auth = Plain { user, pass: pass.expose() };
            tokio::time::timeout(COMMAND_TIMEOUT, client.authenticate("PLAIN", auth)).await
        }
        Credential::Password(_) if has("LOGINDISABLED") => {
            return Err(SyncError::User(format!("{host} doesn't allow password sign-in on this connection.")));
        }
        Credential::Password(pass) => tokio::time::timeout(COMMAND_TIMEOUT, client.login(user, pass.expose())).await,
    };
    let mut session = match result {
        Err(_) => return Err(SyncError::Offline(copy::OFFLINE.into())),
        Ok(Ok(s)) => s,
        Ok(Err((e, _client))) => {
            return Err(match e {
                async_imap::error::Error::No(_) | async_imap::error::Error::Bad(_) => match credential {
                    Credential::Password(_) => SyncError::Auth(copy::IMAP_AUTH.into()),
                    Credential::Bearer(_) => SyncError::Auth("The server didn't accept your sign-in. Sign in again.".into()),
                },
                other => imap_err(other, host),
            });
        }
    };
    let caps: HashSet<String> = match session.capabilities().await {
        Ok(c) => c
            .iter()
            .map(|c| match c {
                async_imap::types::Capability::Imap4rev1 => "IMAP4REV1".to_string(),
                async_imap::types::Capability::Auth(m) => format!("AUTH={}", m.to_ascii_uppercase()),
                async_imap::types::Capability::Atom(a) => a.to_ascii_uppercase(),
            })
            .collect(),
        Err(e) => return Err(imap_err(e, host)),
    };
    if caps.contains("ID") {
        let id = format!("ID (\"name\" \"Axentra Outlook\" \"version\" \"{}\")", env!("CARGO_PKG_VERSION"));
        let _ = raw_command(&mut session, &id).await;
    }
    Ok(Imap { session, caps })
}

/// Connect and sign in.
pub async fn open(server: &ServerConfig, user: &str, credential: &Credential) -> SyncResult<Imap<ImapStream>> {
    let (client, caps) = connect(server).await?;
    login(client, caps, server.host.trim(), user, credential).await
}

/// What a raw command returned besides untagged data.
#[derive(Clone, Debug, Default)]
pub struct RawReply {
    /// `[COPYUID uidvalidity source-uids dest-uids]` (after UID MOVE / COPY).
    pub copy_uid: Option<(u32, Vec<u32>, Vec<u32>)>,
}

fn expand(set: &[async_imap::imap_proto::UidSetMember]) -> Vec<u32> {
    let mut out = Vec::new();
    for m in set {
        match m {
            async_imap::imap_proto::UidSetMember::Uid(u) => out.push(*u),
            async_imap::imap_proto::UidSetMember::UidRange(r) => out.extend(r.clone()),
        }
    }
    out
}

/// Run a command async-imap has no method for (ID, UID MOVE with COPYUID,
/// LIST … RETURN …) and hand every untagged response to `on_untagged`.
pub async fn raw_command_with<T: Transport>(
    session: &mut Session<T>,
    command: &str,
    on_untagged: impl FnMut(&Response<'_>),
) -> SyncResult<RawReply> {
    raw_command_timeout(session, command, COMMAND_TIMEOUT, on_untagged).await
}

/// [`raw_command_with`] with an explicit timeout (body fetches can be slow).
pub async fn raw_command_timeout<T: Transport>(
    session: &mut Session<T>,
    command: &str,
    timeout: Duration,
    mut on_untagged: impl FnMut(&Response<'_>),
) -> SyncResult<RawReply> {
    let fut = async {
        let tag = session.run_command(command).await.map_err(|e| imap_err(e, "server"))?;
        let mut reply = RawReply::default();
        loop {
            let Some(r) = session.read_response().await.map_err(|e| from_io(&e, "server"))? else {
                return Err(SyncError::Offline(copy::OFFLINE.into()));
            };
            let parsed = r.parsed();
            let (done, code, ok, info) = match parsed {
                Response::Done { tag: t, status, code, information } if *t == tag => {
                    (true, code.as_ref(), *status == Status::Ok, information.as_deref().map(str::to_string))
                }
                Response::Data { code, .. } => (false, code.as_ref(), true, None),
                _ => (false, None, true, None),
            };
            if let Some(ResponseCode::CopyUid(v, src, dst)) = code {
                reply.copy_uid = Some((*v, expand(src), expand(dst)));
            }
            if done {
                if !ok {
                    let verb = command.split(' ').take(2).collect::<Vec<_>>().join(" ");
                    return Err(SyncError::protocol(format!("The server rejected {verb}: {}", info.unwrap_or_default())));
                }
                return Ok(reply);
            }
            on_untagged(parsed);
        }
    };
    tokio::time::timeout(timeout, fut).await.map_err(|_| SyncError::Offline(copy::OFFLINE.into()))?
}

pub async fn raw_command<T: Transport>(session: &mut Session<T>, command: &str) -> SyncResult<RawReply> {
    raw_command_with(session, command, |_| {}).await
}

/// Quote a mailbox name or string for a raw command.
pub fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Compact a sorted UID list into an IMAP sequence set ("1:3,7,9:10").
pub fn uid_set(uids: &[u32]) -> String {
    let mut sorted = uids.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut out = String::new();
    let mut i = 0;
    while i < sorted.len() {
        let start = sorted[i];
        let mut end = start;
        while i + 1 < sorted.len() && sorted[i + 1] == end + 1 {
            i += 1;
            end = sorted[i];
        }
        if !out.is_empty() {
            out.push(',');
        }
        if start == end {
            out.push_str(&start.to_string());
        } else {
            out.push_str(&format!("{start}:{end}"));
        }
        i += 1;
    }
    out
}

/// Drop queued unsolicited responses (the channel is bounded).
pub fn drain_unsolicited<T: Transport>(session: &mut Session<T>) -> Vec<UnsolicitedResponse> {
    let mut out = Vec::new();
    while let Ok(r) = session.unsolicited_responses.try_recv() {
        out.push(r);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert_eq!(uid_set(&[9, 1, 2, 3, 7, 10]), "1:3,7,9:10");
        assert_eq!(uid_set(&[5]), "5");
        assert_eq!(uid_set(&[]), "");
        assert_eq!(quote("Sent \"Items\"\\x"), "\"Sent \\\"Items\\\"\\\\x\"");
        assert_eq!(xoauth2_string("a@b", "tok"), "user=a@b\x01auth=Bearer tok\x01\x01");
    }
}
