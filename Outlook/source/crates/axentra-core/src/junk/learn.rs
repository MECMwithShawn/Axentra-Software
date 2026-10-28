//! The local learner: logistic regression over hashed features, trained
//! with plain SGD in a fixed order (same data, same model, byte for byte).
//!
//! A model is never changed in place. Training makes a *candidate*; a
//! candidate becomes active only after [`gate`] compares it with the active
//! model on fixed checks and finds it no worse. Its score is advice: it never
//! moves mail.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::analyze::{self, Analysis};
use crate::model::{Message, MessageBody};

/// Bump when features change; models and stored features of another
/// version are not used together.
pub const FEATURE_VERSION: u32 = 1;
pub const DIM_BITS: u32 = 20;
const DIM: u32 = 1 << DIM_BITS;
const MAX_TOKENS: usize = 1500;
const MAX_BODY: usize = 16 * 1024;

/// Sorted, de-duplicated feature indices (all features are present /
/// absent; each counts 1/√n).
pub type Features = Vec<u32>;

fn fnv(s: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in s.bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    h & (DIM - 1)
}

fn words(s: &str, prefix: &str, out: &mut Vec<String>) {
    for w in s.split(|c: char| !(c.is_alphanumeric() || c == '$' || c == '€' || c == '£')) {
        if out.len() >= MAX_TOKENS {
            return;
        }
        let n = w.chars().count();
        if (2..=24).contains(&n) {
            let w = w.to_lowercase();
            let w = if w.chars().all(|c| c.is_ascii_digit()) { "#num".to_string() } else { w };
            out.push(format!("{prefix}{w}"));
        }
    }
}

/// Text the learner reads from a message: the plain-text body when there is
/// one, else the HTML with tags stripped (scanned as text, never rendered).
fn body_text(body: Option<&MessageBody>) -> String {
    let Some(b) = body else { return String::new() };
    let src = if b.text.trim().is_empty() { b.html.clone().unwrap_or_default() } else { b.text.clone() };
    let mut cut = src.len().min(MAX_BODY);
    while !src.is_char_boundary(cut) {
        cut -= 1;
    }
    let mut out = String::with_capacity(cut);
    let mut in_tag = false;
    for c in src[..cut].chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

/// Features of a message. Uses only what is stored locally; `analysis` adds
/// its warning signs (by kind) and link domains.
pub fn features(m: &Message, body: Option<&MessageBody>, analysis: &Analysis) -> Features {
    let mut toks: Vec<String> = Vec::new();
    words(&m.subject, "s:", &mut toks);
    let text = body_text(body);
    words(&text, "w:", &mut toks);
    let from = m.from.address.trim().to_lowercase();
    let domain = super::domain_of(&from).to_string();
    if !domain.is_empty() {
        toks.push(format!("fd:{}", analyze::registrable(&domain)));
    }
    // Warning signs by kind. No format features (HTML or not, link and "!"
    // counts, top-level domain, empty sender name): in the public corpora
    // they mostly tell old mailing lists from old spam, which says nothing
    // about today's mail.
    for f in &analysis.findings {
        let kind = format!("{f:?}");
        toks.push(format!("f:{}", kind.split([' ', '{']).next().unwrap_or("")));
    }
    let letters: Vec<char> = m.subject.chars().filter(|c| c.is_alphabetic()).collect();
    if letters.len() >= 8 && letters.iter().filter(|c| c.is_uppercase()).count() * 10 >= letters.len() * 7 {
        toks.push("s:shouting".into());
    }
    if let Some(b) = body {
        for a in b.attachments.iter().filter(|a| !a.is_inline) {
            toks.push(format!("ax:{}", a.name.rsplit('.').next().unwrap_or("").to_lowercase()));
        }
    }
    let mut f: Features = toks.iter().map(|t| fnv(t)).collect();
    f.sort_unstable();
    f.dedup();
    f
}

/// Features as stored with a confirmed label (so the learner can rebuild
/// after the message itself is gone).
pub fn features_to_json(f: &Features) -> String {
    serde_json::to_string(&serde_json::json!({ "v": FEATURE_VERSION, "f": f })).unwrap_or_default()
}

pub fn features_from_json(s: &str) -> Option<Features> {
    let v: serde_json::Value = serde_json::from_str(s).ok()?;
    if v["v"].as_u64() != Some(u64::from(FEATURE_VERSION)) {
        return None;
    }
    serde_json::from_value(v["f"].clone()).ok()
}

/// A trained model. `weights` is sparse (index → weight).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Model {
    pub feature_version: u32,
    pub bias: f32,
    pub weights: BTreeMap<u32, f32>,
    /// Score at or above which the model suggests junk, chosen on the
    /// development set (never on the final checks).
    pub threshold: f32,
    /// What it was trained on and how (shown in Settings, kept for audit).
    pub manifest: Manifest,
    /// SHA-256 of the weights, bias, threshold and manifest (first 16 hex).
    pub digest: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Manifest {
    /// "base" (shipped) or "personal".
    pub kind: String,
    /// The model this one started from.
    pub parent: Option<String>,
    pub label_revision: i64,
    pub examples_unwanted: usize,
    pub examples_wanted: usize,
    pub epochs: u32,
    pub learning_rate: f32,
    pub l2: f32,
    pub seed_data: String,
    /// "fingerprint:label" of every confirmed label trained on, so a later
    /// Undo or relabel can tell this model is affected.
    pub trained_on: Vec<String>,
    /// The promotion checks this model passed (or failed).
    pub gate: Option<Verdict>,
}

#[inline]
fn sigmoid(z: f32) -> f32 {
    1.0 / (1.0 + (-z).exp())
}

impl Model {
    /// "Rules only": flags nothing. The reference a first learned model is
    /// compared with.
    pub fn rules_only() -> Model {
        Model { feature_version: FEATURE_VERSION, bias: -50.0, threshold: 1.0, ..Default::default() }
    }

    pub fn score(&self, f: &Features) -> f32 {
        if f.is_empty() || self.feature_version != FEATURE_VERSION {
            return sigmoid(self.bias);
        }
        let x = 1.0 / (f.len() as f32).sqrt();
        let z: f32 = f.iter().filter_map(|i| self.weights.get(i)).map(|w| w * x).sum();
        sigmoid(self.bias + z)
    }

    pub fn compute_digest(&self) -> String {
        let mut h = Sha256::new();
        h.update(self.feature_version.to_le_bytes());
        h.update(self.bias.to_le_bytes());
        h.update(self.threshold.to_le_bytes());
        for (i, w) in &self.weights {
            h.update(i.to_le_bytes());
            h.update(w.to_le_bytes());
        }
        h.update(serde_json::to_vec(&self.manifest).unwrap_or_default());
        h.finalize().iter().take(8).map(|b| format!("{b:02x}")).collect()
    }

    pub fn seal(mut self) -> Self {
        self.digest = self.compute_digest();
        self
    }

    /// Compact form for the database and the shipped base model.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(16 + self.weights.len() * 8);
        out.extend_from_slice(b"AXJM1");
        let meta = serde_json::to_vec(&serde_json::json!({
            "feature_version": self.feature_version, "bias": self.bias, "threshold": self.threshold,
            "manifest": self.manifest, "digest": self.digest,
        }))
        .unwrap_or_default();
        out.extend_from_slice(&(meta.len() as u32).to_le_bytes());
        out.extend_from_slice(&meta);
        out.extend_from_slice(&(self.weights.len() as u32).to_le_bytes());
        for (i, w) in &self.weights {
            out.extend_from_slice(&i.to_le_bytes());
            out.extend_from_slice(&w.to_le_bytes());
        }
        out
    }

    pub fn from_bytes(b: &[u8]) -> Option<Model> {
        let rest = b.strip_prefix(b"AXJM1")?;
        let u32_at = |s: &[u8], at: usize| s.get(at..at + 4).map(|x| u32::from_le_bytes([x[0], x[1], x[2], x[3]]));
        let meta_len = u32_at(rest, 0)? as usize;
        let meta: serde_json::Value = serde_json::from_slice(rest.get(4..4 + meta_len)?).ok()?;
        let body = rest.get(4 + meta_len..)?;
        let n = u32_at(body, 0)? as usize;
        let mut weights = BTreeMap::new();
        for k in 0..n {
            let at = 4 + k * 8;
            let i = u32_at(body, at)?;
            let w = f32::from_le_bytes(body.get(at + 4..at + 8)?.try_into().ok()?);
            weights.insert(i, w);
        }
        let m = Model {
            feature_version: meta["feature_version"].as_u64()? as u32,
            bias: meta["bias"].as_f64()? as f32,
            threshold: meta["threshold"].as_f64()? as f32,
            manifest: serde_json::from_value(meta["manifest"].clone()).ok()?,
            digest: meta["digest"].as_str()?.to_string(),
            weights,
        };
        let finite = m.bias.is_finite() && !m.threshold.is_nan() && m.weights.values().all(|w| w.is_finite());
        (finite && m.compute_digest() == m.digest).then_some(m)
    }
}

/// A fixed check example shipped with Axentra: features only, no text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fixture {
    pub id: String,
    /// "dev" (thresholds), "regression" (the 358-message screening set) or
    /// "fresh" (validation never used for training or tuning).
    pub set: String,
    pub unwanted: bool,
    pub phishing: bool,
    pub f: Features,
}

impl Fixture {
    pub fn example(&self) -> Example {
        Example { features: self.f.clone(), unwanted: self.unwanted, phishing: self.phishing, id: self.id.clone() }
    }
}

/// Fixtures from their stored form (deflate-compressed JSON). Empty on any
/// problem, which makes every gate "insufficient evidence".
pub fn fixtures_from_bytes(b: &[u8]) -> Vec<Fixture> {
    use std::io::Read;
    if b.is_empty() {
        return Vec::new();
    }
    let mut json = Vec::new();
    if flate2::read::DeflateDecoder::new(b).read_to_end(&mut json).is_err() {
        return Vec::new();
    }
    serde_json::from_slice(&json).unwrap_or_default()
}

pub fn fixtures_to_bytes(f: &[Fixture]) -> Vec<u8> {
    use std::io::Write;
    let mut enc = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
    let _ = enc.write_all(&serde_json::to_vec(f).unwrap_or_default());
    enc.finish().unwrap_or_default()
}

/// One training example: features and whether the mail was unwanted.
#[derive(Clone, Debug, PartialEq)]
pub struct Example {
    pub features: Features,
    pub unwanted: bool,
    /// Phishing (a subset of unwanted), for the safety checks.
    pub phishing: bool,
    /// Stable id (fingerprint / fixture id) for the fixed order and splits.
    pub id: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrainParams {
    pub epochs: u32,
    pub learning_rate: f32,
    pub l2: f32,
}

impl Default for TrainParams {
    fn default() -> Self {
        TrainParams { epochs: 8, learning_rate: 0.5, l2: 1e-5 }
    }
}

/// Deterministic shuffle key: examples are visited in the order of a hash
/// of (epoch, id), the same on every machine.
fn order_key(epoch: u32, id: &str) -> [u8; 8] {
    let mut h = Sha256::new();
    h.update(epoch.to_le_bytes());
    h.update(id.as_bytes());
    let d = h.finalize();
    [d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]]
}

/// Train from `start` (or from zero) on `examples`. Classes are weighted so
/// both count equally. Rebuilt from scratch each time: no click is ever
/// "un-learned" by a reverse step.
pub fn train(start: Option<&Model>, examples: &[Example], p: TrainParams) -> Model {
    let mut weights: BTreeMap<u32, f32> = start.map(|m| m.weights.clone()).unwrap_or_default();
    let mut bias = start.map(|m| m.bias).unwrap_or(0.0);
    let pos = examples.iter().filter(|e| e.unwanted).count().max(1) as f32;
    let neg = examples.iter().filter(|e| !e.unwanted).count().max(1) as f32;
    let total = pos + neg;
    let (wpos, wneg) = (total / (2.0 * pos), total / (2.0 * neg));
    let mut idx: Vec<usize> = (0..examples.len()).collect();
    for epoch in 0..p.epochs {
        idx.sort_by_key(|&i| order_key(epoch, &examples[i].id));
        let lr = p.learning_rate / (1.0 + epoch as f32);
        for &i in &idx {
            let e = &examples[i];
            if e.features.is_empty() {
                continue;
            }
            let x = 1.0 / (e.features.len() as f32).sqrt();
            let z: f32 = bias + e.features.iter().filter_map(|f| weights.get(f)).map(|w| w * x).sum::<f32>();
            let y = if e.unwanted { 1.0 } else { 0.0 };
            let cw = if e.unwanted { wpos } else { wneg };
            let g = (sigmoid(z) - y) * cw;
            bias -= lr * g * 0.1;
            for f in &e.features {
                let w = weights.entry(*f).or_insert(0.0);
                *w -= lr * (g * x + p.l2 * *w);
            }
        }
    }
    weights.retain(|_, w| w.abs() > 1e-4);
    Model { feature_version: FEATURE_VERSION, bias, weights, threshold: 0.5, ..Default::default() }
}

/// Counts on one set of checks.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Eval {
    pub wanted: usize,
    pub unwanted: usize,
    pub phishing: usize,
    /// Wanted mail the model would flag (false flags).
    pub false_flags: usize,
    /// Unwanted mail it flags.
    pub caught: usize,
    /// Phishing it flags.
    pub phishing_caught: usize,
    /// Ids of flagged wanted mail and of caught phishing (for the
    /// "no new false flag / no lost phishing" comparison).
    pub flagged_wanted: Vec<String>,
    pub caught_phishing: Vec<String>,
}

pub fn evaluate(m: &Model, set: &[Example], threshold: f32) -> Eval {
    let mut e = Eval::default();
    for x in set {
        let flagged = m.score(&x.features) >= threshold;
        if x.unwanted {
            e.unwanted += 1;
            e.caught += usize::from(flagged);
            if x.phishing {
                e.phishing += 1;
                if flagged {
                    e.phishing_caught += 1;
                    e.caught_phishing.push(x.id.clone());
                }
            }
        } else {
            e.wanted += 1;
            if flagged {
                e.false_flags += 1;
                e.flagged_wanted.push(x.id.clone());
            }
        }
    }
    e.flagged_wanted.sort();
    e.caught_phishing.sort();
    e
}

/// The highest-recall threshold on `dev` that flags at most
/// `max_false_rate` of its wanted mail (never below 0.5, and never below
/// the score of an empty message plus one logit). The bounds are only ever
/// raised, never lowered to fit: when no score can clear them (scores
/// saturate at 1.0) the result is infinite, which suggests nothing.
pub fn choose_threshold(m: &Model, dev: &[Example], max_false_rate: f32) -> f32 {
    let mut wanted: Vec<f32> = dev.iter().filter(|e| !e.unwanted).map(|e| m.score(&e.features)).collect();
    if wanted.iter().any(|s| !s.is_finite()) || !m.bias.is_finite() {
        return f32::INFINITY;
    }
    wanted.sort_by(|a, b| b.total_cmp(a));
    let allowed = ((wanted.len() as f32) * max_false_rate).floor() as usize;
    // Strictly above the highest wanted score that must not be flagged.
    let above = |s: f32| (s + 1e-4).max(s.next_up());
    let t = wanted.get(allowed).map(|&s| above(s)).unwrap_or(0.5);
    // A suggestion needs evidence: at least one logit above what a message
    // with no known words scores.
    let t = t.max(sigmoid(m.bias + 1.0)).max(0.5);
    if t > 1.0 { f32::INFINITY } else { t }
}

/// Why a candidate may or may not become the active model.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verdict {
    Promote,
    Reject(Vec<String>),
    /// The checks lack wanted, unwanted or phishing examples: nothing can
    /// be shown either way, so the previous model stays.
    InsufficientEvidence(String),
}

/// One named set of checks (fixed regression fixtures, the user's held-out
/// labels…) evaluated for the active and the candidate model.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateSet {
    pub name: String,
    pub active: Eval,
    pub candidate: Eval,
    /// Phishing coverage is required on this set.
    pub needs_phishing: bool,
}

/// The promotion rule, the same for every candidate (no relaxing to make
/// one pass): on every set, no wanted message flagged that the active
/// model didn't flag, no phishing the active model caught missed, unwanted
/// mail caught not fewer. A set without both classes (or without phishing,
/// where required) is insufficient evidence.
pub fn gate(sets: &[GateSet]) -> Verdict {
    let mut problems = Vec::new();
    for s in sets {
        let c = &s.candidate;
        if c.wanted == 0 || c.unwanted == 0 || (s.needs_phishing && c.phishing == 0) {
            return Verdict::InsufficientEvidence(format!(
                "{}: {} wanted, {} unwanted, {} phishing",
                s.name, c.wanted, c.unwanted, c.phishing
            ));
        }
        let new_flags: Vec<&String> = c.flagged_wanted.iter().filter(|id| !s.active.flagged_wanted.contains(id)).collect();
        if !new_flags.is_empty() {
            problems.push(format!("{}: {} more wanted message(s) flagged", s.name, new_flags.len()));
        }
        let lost: Vec<&String> = s.active.caught_phishing.iter().filter(|id| !c.caught_phishing.contains(id)).collect();
        if !lost.is_empty() {
            problems.push(format!("{}: {} phishing message(s) no longer caught", s.name, lost.len()));
        }
        if c.caught < s.active.caught {
            problems.push(format!("{}: catches {} unwanted, was {}", s.name, c.caught, s.active.caught));
        }
    }
    if problems.is_empty() { Verdict::Promote } else { Verdict::Reject(problems) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::EmailAddress;

    fn ex(id: &str, text: &str, unwanted: bool) -> Example {
        let m = Message { subject: text.into(), from: EmailAddress::new("", "a@x.example"), ..Default::default() };
        Example { features: features(&m, None, &Analysis::default()), unwanted, phishing: false, id: id.into() }
    }

    fn data() -> Vec<Example> {
        let mut v = Vec::new();
        for i in 0..20 {
            v.push(ex(&format!("s{i}"), &format!("cheap pills casino winner prize {i}"), true));
            v.push(ex(&format!("h{i}"), &format!("meeting agenda project review notes {i}"), false));
        }
        v
    }

    #[test]
    fn features_are_bounded_stable_and_round_trip() {
        let m = Message { subject: "Hello World 123".into(), from: EmailAddress::new("A", "a@b.example"), ..Default::default() };
        let body = MessageBody { text: "word ".repeat(10_000), ..Default::default() };
        let f = features(&m, Some(&body), &Analysis::default());
        assert!(f.len() < 50 && f.windows(2).all(|w| w[0] < w[1]) && f.iter().all(|i| *i < DIM));
        assert_eq!(f, features(&m, Some(&body), &Analysis::default()));
        assert_eq!(features_from_json(&features_to_json(&f)), Some(f));
        assert_eq!(features_from_json(r#"{"v":999,"f":[1]}"#), None, "other feature versions aren't mixed in");
    }

    #[test]
    fn training_is_deterministic_and_learns() {
        let d = data();
        let a = train(None, &d, TrainParams::default()).seal();
        let b = train(None, &d, TrainParams::default()).seal();
        assert_eq!(a.digest, b.digest);
        let spam = ex("t1", "casino prize winner", true);
        let ham = ex("t2", "project meeting notes", false);
        assert!(a.score(&spam.features) > 0.8, "{}", a.score(&spam.features));
        assert!(a.score(&ham.features) < 0.2, "{}", a.score(&ham.features));
    }

    #[test]
    fn models_round_trip_and_reject_tampering() {
        let m = train(None, &data(), TrainParams::default()).seal();
        let bytes = m.to_bytes();
        assert_eq!(Model::from_bytes(&bytes), Some(m.clone()));
        let mut bad = bytes.clone();
        let last = bad.len() - 1;
        bad[last] ^= 1;
        assert_eq!(Model::from_bytes(&bad), None);
        assert_eq!(Model::from_bytes(b"nope"), None);
    }

    #[test]
    fn the_gate_rejects_new_false_flags_and_lost_phishing() {
        let set = |flagged: &[&str], phish: &[&str], caught: usize| Eval {
            wanted: 5,
            unwanted: 5,
            phishing: 2,
            caught,
            flagged_wanted: flagged.iter().map(|s| s.to_string()).collect(),
            caught_phishing: phish.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        };
        let g = |a: Eval, c: Eval| gate(&[GateSet { name: "fixtures".into(), active: a, candidate: c, needs_phishing: true }]);
        assert_eq!(g(set(&["h1"], &["p1"], 3), set(&["h1"], &["p1", "p2"], 4)), Verdict::Promote);
        assert!(matches!(g(set(&[], &["p1"], 3), set(&["h2"], &["p1"], 5)), Verdict::Reject(_)), "a new false flag");
        assert!(matches!(g(set(&[], &["p1"], 3), set(&[], &["p2"], 3)), Verdict::Reject(_)), "a lost phishing catch");
        assert!(matches!(g(set(&[], &["p1"], 3), set(&[], &["p1"], 2)), Verdict::Reject(_)), "lower recall");
        let empty = Eval { wanted: 3, unwanted: 0, ..Default::default() };
        assert!(matches!(g(empty.clone(), empty), Verdict::InsufficientEvidence(_)));
    }

    #[test]
    fn thresholds_are_never_lowered_below_a_wanted_score() {
        let wanted = |id: &str, f: Features| Example { features: f, unwanted: false, phishing: false, id: id.into() };
        // Saturated: an empty message already scores 0.99995; no threshold
        // above it is representable below 1, so nothing is suggested.
        let m = Model { feature_version: FEATURE_VERSION, bias: 10.0, ..Default::default() };
        let t = choose_threshold(&m, &[wanted("w", vec![])], 0.0);
        assert!(t > m.score(&vec![]) && t.is_infinite(), "{t}");
        // The empty-message floor near one isn't clamped down either.
        let m = Model { feature_version: FEATURE_VERSION, bias: 6.5, ..Default::default() };
        let t = choose_threshold(&m, &[], 0.0);
        assert!(t >= sigmoid(7.5), "{t}");
        // Every wanted dev example stays below the chosen threshold, even
        // with equal scores.
        let d = data();
        let m = train(None, &d, TrainParams::default());
        let mut dev: Vec<Example> = d.iter().filter(|e| !e.unwanted).cloned().collect();
        dev.push(dev[0].clone());
        let t = choose_threshold(&m, &dev, 0.0);
        assert!(dev.iter().all(|e| m.score(&e.features) < t));
        // Non-finite models suggest nothing and don't load.
        let nan = Model { feature_version: FEATURE_VERSION, bias: f32::NAN, ..Default::default() };
        assert!(choose_threshold(&nan, &dev, 0.0).is_infinite());
        let mut bad = train(None, &d, TrainParams::default());
        bad.weights.insert(1, f32::INFINITY);
        assert_eq!(Model::from_bytes(&bad.seal().to_bytes()), None);
    }

    #[test]
    fn thresholds_come_from_the_dev_set() {
        let d = data();
        let m = train(None, &d, TrainParams::default());
        let t = choose_threshold(&m, &d, 0.0);
        let e = evaluate(&m, &d, t);
        assert_eq!(e.false_flags, 0);
        assert!(e.caught >= 18, "{e:?}");
    }
}
