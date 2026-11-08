//! Breached-password check against Have I Been Pwned's free "Pwned Passwords" range API using
//! k-anonymity: each password is hashed with SHA-1 here, and only the first 5 hex characters of
//! the hash are sent. The service answers with every known hash suffix in that range (hundreds,
//! padded with decoys), and the match happens on this PC. Passwords and full hashes never leave
//! it. Runs only when the user asks.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use sha1::{Digest, Sha1};

use crate::siteicons::Progress;

const API: &str = "https://api.pwnedpasswords.com/range/";
const WORKERS: usize = 4;

/// Breached SHA-1 hash → times seen, or why the check failed.
pub type BreachResult = Result<HashMap<String, u64>, String>;

/// Uppercase hex SHA-1, the form the API uses. Kept in memory only for this session.
pub fn sha1_hex(password: &str) -> String {
    Sha1::digest(password.as_bytes())
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect()
}

/// Times each hash appears in known breaches, as listed in one range response.
/// Padding lines (count 0) are ignored.
fn parse_range(prefix: &str, body: &str) -> HashMap<String, u64> {
    body.lines()
        .filter_map(|line| {
            let (suffix, count) = line.trim().split_once(':')?;
            let count: u64 = count.trim().parse().ok()?;
            (count > 0).then(|| (format!("{prefix}{}", suffix.trim().to_uppercase()), count))
        })
        .collect()
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .https_only(true)
        .user_agent("AxentraVault-BreachCheck")
        .http_status_as_error(true)
        .build()
        .into()
}

/// Checks the given SHA-1 hashes. Returns breached hash → count. Fails if any range request
/// fails, so a partial check is never shown as "no breaches".
pub fn check(hashes: Vec<String>, progress: Arc<Progress>) -> BreachResult {
    let wanted: HashSet<String> = hashes.into_iter().collect();
    let prefixes: Vec<String> = wanted
        .iter()
        .map(|h| h[..5].to_owned())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    progress.total.store(prefixes.len(), Ordering::Relaxed);
    let queue = std::sync::Mutex::new(prefixes);
    let found = std::sync::Mutex::new(HashMap::new());
    let error = std::sync::Mutex::new(None::<String>);
    let agent = agent();
    std::thread::scope(|s| {
        for _ in 0..WORKERS {
            s.spawn(|| {
                loop {
                    let Some(prefix) = queue.lock().unwrap().pop() else {
                        return;
                    };
                    if error.lock().unwrap().is_some() {
                        return;
                    }
                    let result = agent
                        .get(&format!("{API}{prefix}"))
                        .header("Add-Padding", "true")
                        .call()
                        .and_then(|mut r| {
                            r.body_mut().with_config().limit(4 << 20).read_to_string()
                        });
                    match result {
                        Ok(body) => {
                            let mut f = found.lock().unwrap();
                            for (hash, count) in parse_range(&prefix, &body) {
                                if wanted.contains(&hash) {
                                    f.insert(hash, count);
                                }
                            }
                        }
                        Err(e) => {
                            *error.lock().unwrap() = Some(format!("Breach check failed: {e}"));
                            return;
                        }
                    }
                    progress.done.fetch_add(1, Ordering::Relaxed);
                }
            });
        }
    });
    match error.into_inner().unwrap() {
        Some(e) => Err(e),
        None => Ok(found.into_inner().unwrap()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_and_parses_ranges() {
        // SHA-1("password") = 5BAA61E4C9B93F3F0682250B6CF8331B7EE68FD8
        let h = sha1_hex("password");
        assert_eq!(h, "5BAA61E4C9B93F3F0682250B6CF8331B7EE68FD8");
        let body = "1E4C9B93F3F0682250B6CF8331B7EE68FD8:9545824\r\n\
                    0018A45C4D1DEF81644B54AB7F969B88D65:0\r\n\
                    00D4F6E8FA6EECAD2A3AA415EEC418D38EC:2";
        let ranges = parse_range("5BAA6", body);
        assert_eq!(ranges.get(&h), Some(&9_545_824));
        assert_eq!(ranges.len(), 2, "padding (count 0) ignored");
    }

    /// Live call (not run by default): only a 5-character prefix of a well-known password's
    /// hash is sent.
    #[test]
    #[ignore]
    fn live_check() {
        let h = sha1_hex("password");
        let found = check(vec![h.clone()], Arc::new(Progress::default())).unwrap();
        assert!(found[&h] > 1_000_000);
    }
}
