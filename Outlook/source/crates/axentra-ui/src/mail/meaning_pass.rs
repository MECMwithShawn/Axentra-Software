//! Meaning search (Settings › Search): the sentence model reads each
//! message once in the background, newest first, and keeps its vector in
//! the profile; searches then also find mail about the same thing as the
//! words typed. Everything runs on this computer. Without the model files
//! (a development build, or a package without them) search is keyword only.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use axentra_core::Store;
use axentra_embed::{Embedder, MODEL_ID};

/// Messages read per batch (a few seconds of background work).
pub const BATCH: u32 = 32;

static ENABLED: AtomicBool = AtomicBool::new(false);

/// The model, loaded on first use; `None` when its files aren't installed
/// or don't load.
pub fn embedder() -> Option<&'static Embedder> {
    static E: OnceLock<Option<Embedder>> = OnceLock::new();
    E.get_or_init(|| {
        let dir = axentra_embed::model_dir()?;
        Embedder::load(&dir).map_err(|e| eprintln!("axentra: meaning search unavailable: {e}")).ok()
    })
    .as_ref()
}

/// Whether this build has the model files.
pub fn available() -> bool {
    axentra_embed::model_dir().is_some()
}

/// Turn meaning search on or off for searches and the background pass.
pub fn set_enabled(s: &Store, on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
    let embed: axentra_core::QueryEmbedder = Arc::new(|q: &str| {
        if !ENABLED.load(Ordering::Relaxed) {
            return None;
        }
        // One search reads the store more than once (as phrases, then as
        // typed): the model runs once for it.
        static LAST: Mutex<Option<(String, Vec<f32>)>> = Mutex::new(None);
        let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, v)) = last.as_ref().filter(|(text, _)| text == q) {
            return Some((MODEL_ID.to_string(), v.clone()));
        }
        let v = embedder()?.embed_query(q).map_err(|e| eprintln!("axentra: meaning search failed: {e}")).ok()?;
        *last = Some((q.to_string(), v.clone()));
        Some((MODEL_ID.to_string(), v))
    });
    s.set_query_embedder(on.then_some(embed));
}

/// One batch: read up to [`BATCH`] messages that have no vector yet.
/// Returns how many were read (0: nothing to do, off, or no model).
pub fn run_batch(s: &Store) -> Result<usize, String> {
    if !ENABLED.load(Ordering::Relaxed) {
        return Ok(0);
    }
    let Some(e) = embedder() else { return Ok(0) };
    let todo = s.messages_to_embed(MODEL_ID, BATCH).map_err(|e| e.to_string())?;
    if todo.is_empty() {
        return Ok(0);
    }
    let texts: Vec<&str> = todo.iter().map(|(_, t)| t.as_str()).collect();
    let vectors = e.embed_documents(&texts).map_err(|e| e.to_string())?;
    let rows: Vec<_> = todo.iter().map(|(id, _)| *id).zip(vectors).collect();
    s.save_vectors(MODEL_ID, &rows).map_err(|e| e.to_string())?;
    Ok(rows.len())
}

/// "4,210 of 12,800 messages read", for Settings.
pub fn progress(s: &Store) -> Option<(i64, i64)> {
    s.embedding_progress(MODEL_ID).ok()
}
