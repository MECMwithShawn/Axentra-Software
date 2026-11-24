//! Meaning search: turns text into a vector with a small sentence model
//! (BAAI bge-small-en-v1.5, 384 numbers per text) that runs on the
//! processor, on this computer. Texts about the same thing get vectors that
//! point the same way, whatever words they use. Nothing here touches the
//! network: the model files ship with the app (see [`model_dir`]).

mod wordpiece;

use std::path::{Path, PathBuf};

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config};

pub use wordpiece::WordPiece;

/// Numbers per vector.
pub const DIM: usize = 384;
/// Which model made a vector: vectors of another model aren't comparable.
pub const MODEL_ID: &str = "bge-small-en-v1.5";
/// Tokens read per text. The model takes 512; the start of an email
/// (subject, greeting, first paragraph) says what it's about, and a
/// shorter input is several times faster.
const MAX_TOKENS: usize = 128;
/// Threads the model runs on. They run at low priority, so indexing in the
/// background doesn't slow down the rest of the computer.
const THREADS: usize = 4;
/// bge's instruction for short search queries (not for the mail itself).
const QUERY_PREFIX: &str = "Represent this sentence for searching relevant passages: ";

pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T> = std::result::Result<T, Error>;

/// Where the model files are: `$AXENTRA_MODEL_DIR` when set (tests,
/// development), else `models/bge-small-en-v1.5` next to the installed
/// program (`/opt/axentra/outlook/…`), else under
/// `/usr/share/axentra-outlook/models/`. `None` when they aren't there:
/// meaning search is then unavailable, keyword search works.
pub fn model_dir() -> Option<PathBuf> {
    let has_model = |d: &Path| d.join("model.safetensors").is_file() && d.join("vocab.txt").is_file() && d.join("config.json").is_file();
    if let Some(d) = std::env::var_os("AXENTRA_MODEL_DIR").map(PathBuf::from) {
        return has_model(&d).then_some(d);
    }
    let mut dirs = Vec::new();
    if let Some(app) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        dirs.push(app.join("models").join(MODEL_ID));
    }
    dirs.push(Path::new("/usr/share/axentra-outlook/models").join(MODEL_ID));
    dirs.into_iter().find(|d| has_model(d))
}

/// A loaded model. Loading reads ~65 MB; embedding takes a few
/// milliseconds per text.
pub struct Embedder {
    model: BertModel,
    vocab: WordPiece,
    device: Device,
    pool: rayon::ThreadPool,
}

impl Embedder {
    pub fn load(dir: &Path) -> Result<Self> {
        let device = Device::Cpu;
        let config: Config = serde_json::from_str(&std::fs::read_to_string(dir.join("config.json"))?)?;
        let vocab = WordPiece::from_vocab(&std::fs::read_to_string(dir.join("vocab.txt"))?)?;
        // SAFETY: the file is memory-mapped read-only and the app doesn't
        // change it while running (a package upgrade replaces it with a new
        // file, which leaves this mapping intact).
        let vb = unsafe { VarBuilder::from_mmaped_safetensors(&[dir.join("model.safetensors")], DType::F32, &device)? };
        let model = BertModel::load(vb, &config)?;
        let threads = std::thread::available_parallelism().map_or(1, |n| (n.get() / 2).clamp(1, THREADS));
        let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).thread_name(|i| format!("ax-embed-{i}")).start_handler(|_| lower_priority()).build()?;
        Ok(Self { model, vocab, device, pool })
    }

    /// Vectors for mail texts, one per text, each of length 1.
    pub fn embed_documents(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        self.embed(texts.iter().map(|t| t.to_string()).collect())
    }

    /// The vector of a search query.
    pub fn embed_query(&self, query: &str) -> Result<Vec<f32>> {
        Ok(self.embed(vec![format!("{QUERY_PREFIX}{query}")])?.remove(0))
    }

    fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        let ids: Vec<Vec<u32>> = texts.iter().map(|t| self.vocab.encode(t, MAX_TOKENS)).collect();
        // Similar lengths together: less padding to compute.
        let mut order: Vec<usize> = (0..ids.len()).collect();
        order.sort_by_key(|&i| ids[i].len());
        let mut out = vec![Vec::new(); ids.len()];
        for chunk in order.chunks(16) {
            let rows: Vec<&[u32]> = chunk.iter().map(|&i| ids[i].as_slice()).collect();
            for (&i, v) in chunk.iter().zip(self.pool.install(|| self.forward(&rows))?) {
                out[i] = v;
            }
        }
        Ok(out)
    }

    fn forward(&self, ids: &[&[u32]]) -> Result<Vec<Vec<f32>>> {
        let len = ids.iter().map(|r| r.len()).max().unwrap_or(2);
        let mut flat = Vec::with_capacity(ids.len() * len);
        let mut mask = Vec::with_capacity(ids.len() * len);
        for row in ids {
            flat.extend(row.iter().copied().chain(std::iter::repeat_n(0, len - row.len())));
            mask.extend(std::iter::repeat_n(1u32, row.len()).chain(std::iter::repeat_n(0, len - row.len())));
        }
        let input = Tensor::from_vec(flat, (ids.len(), len), &self.device)?;
        let mask = Tensor::from_vec(mask, (ids.len(), len), &self.device)?;
        let types = input.zeros_like()?;
        let out = self.model.forward(&input, &types, Some(&mask))?;
        // bge pools on the [CLS] token, then normalizes.
        let cls = out.narrow(1, 0, 1)?.squeeze(1)?;
        let rows: Vec<Vec<f32>> = cls.to_vec2()?;
        Ok(rows.into_iter().map(normalized).collect())
    }
}

/// Nice the calling thread (Linux applies nice per thread).
fn lower_priority() {
    #[cfg(target_os = "linux")]
    // SAFETY: plain syscalls on the calling thread's own id.
    unsafe {
        let tid = libc::syscall(libc::SYS_gettid) as libc::id_t;
        libc::setpriority(libc::PRIO_PROCESS, tid, 10);
    }
}

fn normalized(mut v: Vec<f32>) -> Vec<f32> {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0.0 {
        v.iter_mut().for_each(|x| *x /= n);
    }
    v
}

/// How alike two vectors of length 1 are: 1 is the same meaning.
pub fn similarity(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// A vector as stored (little-endian f32s).
pub fn to_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

pub fn from_bytes(b: &[u8]) -> Vec<f32> {
    b.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vectors_round_trip_and_compare() {
        let v = normalized(vec![3.0, 4.0]);
        assert_eq!(from_bytes(&to_bytes(&v)), v);
        assert!((similarity(&v, &v) - 1.0).abs() < 1e-6);
    }

    /// Needs the model files: `AXENTRA_MODEL_DIR=… cargo test -p axentra-embed -- --ignored`.
    #[test]
    #[ignore]
    fn mail_about_the_same_thing_is_closest() {
        let e = Embedder::load(&model_dir().expect("AXENTRA_MODEL_DIR")).unwrap();
        let docs = e
            .embed_documents(&[
                "Your package has shipped. Tracking number 1Z999 arrives Thursday.",
                "Team lunch on Friday at the Thai place, who's in?",
                "Invoice 2039 for October consulting is attached, due in 30 days.",
            ])
            .unwrap();
        let rank = |q: &str| {
            let qv = e.embed_query(q).unwrap();
            (0..docs.len()).max_by(|&a, &b| similarity(&qv, &docs[a]).total_cmp(&similarity(&qv, &docs[b]))).unwrap()
        };
        assert_eq!(rank("when will my order be delivered"), 0);
        assert_eq!(rank("food with coworkers"), 1);
        assert_eq!(rank("bill I need to pay"), 2);
    }
}
