//! Global bandwidth limit shared by all transfers in one direction, like the
//! "Limit upload/download rate" options in the Windows client.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tokio::time::Instant;

#[derive(Debug)]
pub struct RateLimiter {
    bytes_per_sec: AtomicU64,
    next_free: Mutex<Option<Instant>>,
}

impl RateLimiter {
    /// `bytes_per_sec == 0` means unlimited.
    pub fn new(bytes_per_sec: u64) -> Self {
        Self {
            bytes_per_sec: AtomicU64::new(bytes_per_sec),
            next_free: Mutex::new(None),
        }
    }

    pub fn set_rate(&self, bytes_per_sec: u64) {
        self.bytes_per_sec.store(bytes_per_sec, Ordering::Relaxed);
        *self.next_free.lock().unwrap() = None;
    }

    pub fn rate(&self) -> u64 {
        self.bytes_per_sec.load(Ordering::Relaxed)
    }

    /// Account for `n` bytes, sleeping long enough to keep the shared rate.
    pub async fn consume(&self, n: u64) {
        let rate = self.rate();
        if rate == 0 || n == 0 {
            return;
        }
        let until = {
            let mut next = self.next_free.lock().unwrap();
            let now = Instant::now();
            let start = next.map_or(now, |t| t.max(now));
            let end = start + Duration::from_secs_f64(n as f64 / rate as f64);
            *next = Some(end);
            end
        };
        tokio::time::sleep_until(until).await;
    }
}
