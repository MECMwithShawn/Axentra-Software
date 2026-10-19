//! The engine's "something changed" counter, polled by the UI.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Debug, Default)]
pub struct Notifier(Arc<AtomicU64>);

impl Notifier {
    pub fn new(counter: Arc<AtomicU64>) -> Self {
        Notifier(counter)
    }

    /// Call after every write to the store.
    pub fn bump(&self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }

    pub fn get(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}
