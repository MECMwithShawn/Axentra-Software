//! One window by default: a second launch hands its files to the first over a unix socket.
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value as J};

pub fn socket_path(data: &Path) -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|d| d.is_absolute() && d.is_dir())
        .map(|d| d.join("axentra-notepad.sock"))
        .unwrap_or_else(|| data.join("instance.sock"))
}

pub enum Claim {
    /// Another window took the files.
    Forwarded,
    /// This process is the primary window and owns the socket.
    Primary(UnixListener),
    /// Couldn't forward or claim: run as a window of its own.
    Alone,
}

fn forward(mut stream: UnixStream, paths: &[String]) -> bool {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(4)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(4)));
    let line = format!("{}\n", json!({ "paths": paths }));
    if stream.write_all(line.as_bytes()).is_err() {
        return false;
    }
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply).is_ok() && reply.trim() == "ok"
}

pub fn claim(socket: &Path, paths: &[String]) -> Claim {
    if let Ok(stream) = UnixStream::connect(socket) {
        return if forward(stream, paths) { Claim::Forwarded } else { Claim::Alone };
    }
    if let Some(dir) = socket.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // Nobody answered, so any socket file left behind is stale.
    let _ = std::fs::remove_file(socket);
    match UnixListener::bind(socket) {
        Ok(listener) => Claim::Primary(listener),
        // Lost a race with another launch: hand over to it instead.
        Err(_) => match UnixStream::connect(socket).map(|s| forward(s, paths)) {
            Ok(true) => Claim::Forwarded,
            _ => Claim::Alone,
        },
    }
}

/// Answers other launches on a thread; `on_paths` runs for each request.
pub fn serve(listener: UnixListener, on_paths: impl Fn(Vec<String>) + Send + 'static) {
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(4)));
            let mut line = String::new();
            let Ok(reader_stream) = stream.try_clone() else { continue };
            if BufReader::new(reader_stream.take(4 << 20)).read_line(&mut line).is_err() {
                continue;
            }
            let req: J = serde_json::from_str(&line).unwrap_or(J::Null);
            let paths: Vec<String> = req["paths"].as_array().map(|a| a.iter().filter_map(|p| p.as_str().map(String::from)).collect()).unwrap_or_default();
            let mut stream = stream;
            let _ = stream.write_all(b"ok\n");
            on_paths(paths);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::tests::temp;

    #[test]
    fn second_launch_forwards_to_the_first() {
        let dir = temp("instance");
        let sock = dir.join("test.sock");
        // A stale socket file from a crash doesn't block claiming.
        std::fs::write(&sock, "").unwrap();
        let Claim::Primary(listener) = claim(&sock, &[]) else { panic!("should be primary") };
        let (tx, rx) = std::sync::mpsc::channel();
        serve(listener, move |p| tx.send(p).unwrap());
        assert!(matches!(claim(&sock, &["/tmp/a.txt".into(), "/tmp/b c.txt".into()]), Claim::Forwarded));
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), vec!["/tmp/a.txt".to_string(), "/tmp/b c.txt".to_string()]);
        std::fs::remove_dir_all(dir).ok();
    }
}
