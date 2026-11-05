//! Local AI review over Ollama on this computer (see
//! `axentra_core::junk::shadow`). Plain HTTP/1.1 to a loopback address: a
//! message can't reach another host from here.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use axentra_core::junk::shadow::{self, FIRST_MODEL, Review, SECOND_MODEL};
use serde_json::Value;

/// Where Ollama listens by default.
pub const ADDR: &str = "127.0.0.1:11434";
/// A slow model on a busy computer, not a stuck server.
const REPLY_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("Ollama isn't running on this computer")]
    NotRunning,
    #[error("Ollama doesn't have a model the review needs")]
    MissingModel,
    #[error("{0}")]
    Failed(String),
}

/// Review one message. The second model reads only what the first flags.
pub fn review(addr: SocketAddr, prompt: &str) -> Result<Review, Error> {
    let started = Instant::now();
    let first = shadow::parse_reply(&chat(addr, &shadow::request(FIRST_MODEL, false, prompt))?);
    let first_ms = started.elapsed().as_millis() as u64;
    let started = Instant::now();
    let second = match first {
        Some(v) if v.unwanted() => shadow::parse_reply(&chat(addr, &shadow::request(SECOND_MODEL, true, prompt))?),
        _ => None,
    };
    Ok(Review { first, second, first_ms, second_ms: if second.is_some() { started.elapsed().as_millis() as u64 } else { 0 } })
}

fn chat(addr: SocketAddr, request: &Value) -> Result<Value, Error> {
    if !addr.ip().is_loopback() {
        return Err(Error::Failed("only this computer can run the review".into()));
    }
    let failed = |e: std::io::Error| Error::Failed(e.to_string());
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).map_err(|_| Error::NotRunning)?;
    s.set_read_timeout(Some(REPLY_TIMEOUT)).map_err(failed)?;
    let payload = request.to_string();
    let head = format!("POST /api/chat HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", payload.len());
    s.write_all(head.as_bytes()).and_then(|_| s.write_all(payload.as_bytes())).map_err(failed)?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).map_err(failed)?;
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or_else(|| Error::Failed("malformed reply".into()))?;
    let head = String::from_utf8_lossy(&raw[..end]).to_ascii_lowercase();
    let status = head.split_whitespace().nth(1).unwrap_or("");
    match status {
        "200" => {}
        "404" => return Err(Error::MissingModel),
        other => return Err(Error::Failed(format!("Ollama answered {other}"))),
    }
    let rest = &raw[end + 4..];
    let body = if head.contains("transfer-encoding: chunked") { dechunk(rest) } else { rest.to_vec() };
    serde_json::from_slice(&body).map_err(|e| Error::Failed(format!("reply not JSON: {e}")))
}

fn dechunk(mut s: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(line) = s.windows(2).position(|w| w == b"\r\n") {
        let Some(n) = std::str::from_utf8(&s[..line]).ok().and_then(|v| usize::from_str_radix(v.trim(), 16).ok()) else { break };
        let rest = &s[line + 2..];
        if n == 0 || rest.len() < n {
            break;
        }
        out.extend_from_slice(&rest[..n]);
        s = rest[n..].strip_prefix(b"\r\n").unwrap_or(&rest[n..]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use axentra_core::junk::shadow::{Recommendation, Verdict};
    use std::net::TcpListener;

    /// Reads one whole request.
    fn read_request(s: &mut TcpStream) -> Vec<u8> {
        let mut raw = Vec::new();
        let mut buf = [0u8; 65536];
        loop {
            let n = s.read(&mut buf).unwrap();
            raw.extend_from_slice(&buf[..n]);
            let Some(end) = raw.windows(4).position(|w| w == b"\r\n\r\n") else { continue };
            let head = String::from_utf8_lossy(&raw[..end]).to_lowercase();
            let len: usize = head.lines().find_map(|l| l.strip_prefix("content-length: ")).unwrap().parse().unwrap();
            if raw.len() >= end + 4 + len {
                return raw[end + 4..].to_vec();
            }
        }
    }

    /// A stand-in for Ollama that answers each model with `label` and
    /// records who was asked.
    fn fake(first: &'static str, second: &'static str) -> (SocketAddr, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            listener.set_nonblocking(false).unwrap();
            let mut asked = Vec::new();
            while let Ok((mut s, _)) = listener.accept() {
                let body = read_request(&mut s);
                let v: Value = serde_json::from_slice(&body).unwrap();
                let model = v["model"].as_str().unwrap().to_string();
                if model == "stop" {
                    return asked;
                }
                let label = if model == FIRST_MODEL { first } else { second };
                let reply = serde_json::json!({ "message": { "content": format!("{{\"label\":\"{label}\"}}") } }).to_string();
                write!(s, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{reply}", reply.len()).unwrap();
                asked.push(model);
            }
            asked
        });
        (addr, handle)
    }

    fn stop(addr: SocketAddr) {
        let _ = chat(addr, &serde_json::json!({ "model": "stop" }));
    }

    #[test]
    fn the_second_model_reads_only_what_the_first_flags() {
        let (addr, h) = fake("legitimate", "phishing");
        let r = review(addr, "mail").unwrap();
        assert_eq!((r.first, r.second, r.recommendation()), (Some(Verdict::Legitimate), None, None));
        stop(addr);
        assert_eq!(h.join().unwrap(), vec![FIRST_MODEL]);

        let (addr, h) = fake("phishing", "phishing");
        assert_eq!(review(addr, "mail").unwrap().recommendation(), Some(Recommendation::Phishing));
        stop(addr);
        assert_eq!(h.join().unwrap(), vec![FIRST_MODEL, SECOND_MODEL]);
    }

    #[test]
    fn nothing_listening_and_other_hosts_are_errors() {
        let free = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
        assert_eq!(review(free, "mail"), Err(Error::NotRunning));
        let outside: SocketAddr = "192.0.2.1:11434".parse().unwrap();
        assert!(matches!(review(outside, "mail"), Err(Error::Failed(_))));
    }

    #[test]
    fn a_missing_model_is_named_as_such() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            read_request(&mut s);
            write!(s, "HTTP/1.1 404 Not Found\r\nContent-Length: 2\r\n\r\n{{}}").unwrap();
        });
        assert_eq!(review(addr, "mail"), Err(Error::MissingModel));
    }

    /// Needs Ollama with both models: `cargo test -p axentra-sync real_ollama -- --ignored`.
    #[test]
    #[ignore]
    fn real_ollama_reviews_invented_mail() {
        use axentra_core::{EmailAddress, Message, MessageBody};
        let mail = |from: &str, subject: &str, text: &str| {
            let m = Message { from: EmailAddress::new("", from), subject: subject.into(), ..Default::default() };
            shadow::prompt(&m, &MessageBody { text: text.into(), ..Default::default() })
        };
        let addr: SocketAddr = ADDR.parse().unwrap();
        let scam = review(addr, &mail("security@paypa1-billing.example", "Your account has been limited", "Confirm your password and card number within 24 hours at https://paypa1-billing.example/login or your account will be closed.")).unwrap();
        assert_eq!(scam.recommendation(), Some(Recommendation::Phishing), "{scam:?}");
        let note = review(addr, &mail("sam@friend.example", "Lunch on Thursday?", "Are you free for lunch on Thursday? The usual place at noon.")).unwrap();
        assert_eq!(note.recommendation(), None, "{note:?}");
    }

    #[test]
    fn chunks_join_even_inside_a_character() {
        let bytes = "é".as_bytes();
        let mut raw = b"1\r\n".to_vec();
        raw.push(bytes[0]);
        raw.extend_from_slice(b"\r\n1\r\n");
        raw.push(bytes[1]);
        raw.extend_from_slice(b"\r\n0\r\n\r\n");
        assert_eq!(dechunk(&raw), bytes);
    }
}
