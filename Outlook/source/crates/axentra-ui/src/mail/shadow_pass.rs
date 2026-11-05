//! When Local AI review runs: one message at a time, off the UI thread, and
//! only while it's switched on. Ollama being away isn't a fault: the review
//! says so in Settings and looks again later. Only errors reach the log,
//! never message content.

use std::net::SocketAddr;
use std::sync::Mutex;
use std::time::Duration;

use axentra_core::Store;
use axentra_sync::ollama;

const BUSY: Duration = Duration::from_secs(1);
const IDLE: Duration = Duration::from_secs(15);
const RETRY: Duration = Duration::from_secs(60);

/// What Settings shows when the review can't run.
static PROBLEM: Mutex<Option<Problem>> = Mutex::new(None);

/// Why the review is waiting.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Problem {
    NotRunning,
    MissingModel,
    Failed,
}

impl Problem {
    /// The full explanation, for Settings.
    pub fn text(self) -> &'static str {
        match self {
            Problem::NotRunning => "Ollama isn't running on this computer, so the review is waiting. Mail stays as it is.",
            Problem::MissingModel => "Ollama is missing a model. Install qwen3:4b and deepseek-r1:14b, and the review will carry on.",
            Problem::Failed => "The review couldn't finish a message and will try again.",
        }
    }

    /// A menu line.
    pub fn short(self) -> &'static str {
        match self {
            Problem::NotRunning => "Waiting: Ollama isn't running",
            Problem::MissingModel => "Waiting: Ollama is missing a model",
            Problem::Failed => "Couldn't finish a message, will retry",
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum Step {
    Idle,
    Reviewed,
    Recommended,
    Failed,
}

impl Step {
    pub fn wait(&self) -> Duration {
        match self {
            Step::Idle => IDLE,
            Step::Reviewed | Step::Recommended => BUSY,
            Step::Failed => RETRY,
        }
    }
}

pub fn addr() -> SocketAddr {
    ollama::ADDR.parse().expect("a valid address")
}

pub fn problem() -> Option<Problem> {
    *PROBLEM.lock().unwrap_or_else(|e| e.into_inner())
}

fn set_problem(p: Option<Problem>) {
    *PROBLEM.lock().unwrap_or_else(|e| e.into_inner()) = p;
}

/// Review the next message, if there is one.
pub fn run_one(s: &Store, addr: SocketAddr) -> Step {
    let (id, prompt) = match s.shadow_next() {
        Ok(Some(next)) => next,
        Ok(None) => return Step::Idle,
        Err(e) => {
            eprintln!("axentra: local AI review incomplete, retrying later: {e}");
            return Step::Failed;
        }
    };
    let review = match ollama::review(addr, &prompt) {
        Ok(r) => r,
        Err(e) => {
            set_problem(Some(match e {
                ollama::Error::NotRunning => Problem::NotRunning,
                ollama::Error::MissingModel => Problem::MissingModel,
                ollama::Error::Failed(_) => Problem::Failed,
            }));
            return Step::Failed;
        }
    };
    set_problem(None);
    if let Err(e) = s.save_shadow_review(id, &review) {
        eprintln!("axentra: local AI review not saved, retrying later: {e}");
        return Step::Failed;
    }
    if review.recommendation().is_some() { Step::Recommended } else { Step::Reviewed }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axentra_core::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn store_with_new_mail() -> (Store, MessageId, FolderId) {
        let s = Store::open_in_memory().unwrap();
        let mut a = Account { kind: AccountKind::Local, email: "me@axentra.example".into(), ..Default::default() };
        let acct = s.add_account(&mut a).unwrap();
        let mut settings = s.settings();
        settings.junk_shadow = true;
        settings.junk_filter = false;
        s.save_settings(&settings).unwrap();
        s.start_shadow_review().unwrap();
        let inbox = s.folders(Some(acct)).unwrap().into_iter().find(|f| f.role == FolderRole::Inbox).unwrap().id;
        let mut m = Message {
            account_id: acct,
            folder_id: inbox,
            from: EmailAddress::new("", "x@stranger.example"),
            subject: "Your account".into(),
            received_at: chrono::Utc::now() + chrono::Duration::seconds(5),
            ..Default::default()
        };
        let id = s.insert_message(&mut m, Some(&MessageBody { text: "hello".into(), ..Default::default() })).unwrap();
        s.assess_pending(10).unwrap();
        (s, id, inbox)
    }

    /// Answers `n` requests, each with `label`.
    fn ollama_says(label: &'static str, n: usize) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for _ in 0..n {
                let (mut c, _) = listener.accept().unwrap();
                let mut raw = Vec::new();
                let mut buf = [0u8; 8192];
                loop {
                    let got = c.read(&mut buf).unwrap();
                    raw.extend_from_slice(&buf[..got]);
                    let Some(end) = raw.windows(4).position(|w| w == b"\r\n\r\n") else { continue };
                    let head = String::from_utf8_lossy(&raw[..end]).to_lowercase();
                    let len: usize = head.lines().find_map(|l| l.strip_prefix("content-length: ")).unwrap().parse().unwrap();
                    if raw.len() >= end + 4 + len {
                        break;
                    }
                }
                let reply = serde_json::json!({ "message": { "content": format!("{{\"label\":\"{label}\"}}") } }).to_string();
                write!(c, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{reply}", reply.len()).unwrap();
            }
        });
        addr
    }

    #[test]
    fn a_recommendation_leaves_the_mail_where_it_is() {
        let (s, id, inbox) = store_with_new_mail();
        assert_eq!(run_one(&s, ollama_says("phishing", 2)), Step::Recommended);
        assert_eq!(s.shadow_recommendation(id).unwrap(), Some(junk::shadow::Recommendation::Phishing));
        assert_eq!(s.message(id).unwrap().unwrap().folder_id, inbox);
        assert_eq!(run_one(&s, addr()), Step::Idle, "each message is reviewed once");
    }

    #[test]
    fn with_ollama_away_the_message_waits_for_it() {
        let (s, id, _) = store_with_new_mail();
        let free = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
        assert_eq!(run_one(&s, free), Step::Failed);
        assert_eq!(problem(), Some(Problem::NotRunning));
        assert_eq!(s.shadow_next().unwrap().map(|n| n.0), Some(id));
        assert_eq!(run_one(&s, ollama_says("legitimate", 1)), Step::Reviewed);
        assert_eq!(problem(), None);
    }
}
