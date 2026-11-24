//! Junk filter bake-off: every candidate filter decides, for the same
//! labelled mail, whether it would move each message to Junk Email. The
//! report counts wanted mail moved (the number that must stay at zero),
//! spam and phishing caught, and the time per message. See
//! `docs/junk/BENCH.md`.
//!
//! ```text
//! cargo run --release -p axentra-junk-bench -- \
//!     [--reports tools/junk/modern-dev.json] \
//!     [--set tools/junk/fresh-validation.json --set tools/junk/tricky.json --set tools/junk/holdout.json] \
//!     [--benchmark …/evaluation.jsonl] [--llm qwen3:14b] [--model-dir DIR] [--out DIR]
//! ```
//!
//! "Reports" stands in for the mail you reported as junk / not junk: the
//! learning candidates train on it and only on it, and their cut-offs are
//! chosen on it, never on a test set.

mod data;
mod llm;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use axentra_core::junk::analyze::{self, Analysis, Severity};
use axentra_core::junk::{filter, learn};
use axentra_core::{Importance, meaning_text};
use axentra_embed::{Embedder, similarity};
use data::{Item, Label, Set};
use serde_json::{Value, json};

/// The invented user of the synthetic sets works at Contoso, banks with
/// Northwind and has these contacts (the senders the filters treat as known).
const FAMILIAR: [&str; 5] = ["contoso.example", "northwind.example", "lumen.example", "northwindbank.example", "riverbend.example"];
const CONTACTS: [(&str, &str); 5] = [
    ("daniel kim", "daniel.kim@contoso.example"),
    ("priya raman", "priya.raman@northwind.example"),
    ("hannah okafor", "hannah.okafor@contoso.example"),
    ("maya chen", "maya.chen@riverbend.example"),
    ("marcus lee", "marcus.lee@contoso.example"),
];
/// An LLM verdict counts at this confidence or more.
const LLM_CONFIDENT: f32 = 0.8;
/// Nearest reports each side the similarity vote looks at.
const K: usize = 3;

struct Args {
    reports: String,
    sets: Vec<String>,
    benchmark: Option<String>,
    llm: Option<String>,
    model_dir: Option<PathBuf>,
    out: PathBuf,
}

fn args() -> Args {
    let mut a = Args {
        reports: "tools/junk/modern-dev.json".into(),
        sets: Vec::new(),
        benchmark: None,
        llm: None,
        model_dir: None,
        out: PathBuf::from("target/junk-bench"),
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = || it.next().unwrap_or_else(|| usage(&format!("{flag} needs a value")));
        match flag.as_str() {
            "--reports" => a.reports = value(),
            "--set" => a.sets.push(value()),
            "--benchmark" => a.benchmark = Some(value()),
            "--llm" => a.llm = Some(value()),
            "--model-dir" => a.model_dir = Some(value().into()),
            "--out" => a.out = value().into(),
            "-h" | "--help" => usage(""),
            other => usage(&format!("unknown argument {other}")),
        }
    }
    if a.sets.is_empty() {
        a.sets = vec!["tools/junk/fresh-validation.json".into(), "tools/junk/tricky.json".into(), "tools/junk/holdout.json".into()];
    }
    a
}

fn usage(problem: &str) -> ! {
    if !problem.is_empty() {
        eprintln!("axentra-junk-bench: {problem}");
    }
    eprintln!("usage: axentra-junk-bench [--reports SET.json] [--set SET.json]… [--benchmark evaluation.jsonl] [--llm OLLAMA_MODEL] [--model-dir DIR] [--out DIR]");
    std::process::exit(2);
}

/// Everything worked out once per message.
struct Prepared {
    analysis: Analysis,
    features: learn::Features,
    /// The text the junk filter reads (visible body text).
    visible: String,
    vec: Option<Vec<f32>>,
    known: bool,
    /// Any sign at all (the corroboration an LLM verdict needs).
    any_sign: bool,
    llm: Option<llm::Verdict>,
    ms_checks: f64,
    ms_embed: f64,
    ms_llm: f64,
}

fn known_people() -> Vec<(String, Vec<String>)> {
    CONTACTS.iter().map(|(n, a)| (n.to_string(), vec![a.to_string()])).collect()
}

fn prepare(set: &Set, embedder: Option<&Embedder>, judge: Option<&llm::Judge>) -> Vec<Prepared> {
    let familiar: Vec<String> = FAMILIAR.iter().map(|s| s.to_string()).collect();
    let people = known_people();
    let mut out: Vec<Prepared> = set
        .items
        .iter()
        .map(|it| {
            let t = Instant::now();
            let analysis = analyze::analyze(&analyze::Input {
                message: &it.message,
                html: it.body.html.as_deref(),
                text: &it.body.text,
                attachments: &it.body.attachments,
                familiar_domains: &familiar,
                trusted_reporters: &[],
                known_people: &people,
                own_names: &[],
            });
            let features = learn::features(&it.message, Some(&it.body), &analysis);
            let visible = learn::visible_text(&it.body);
            let from = it.message.from.address.to_lowercase();
            let known = CONTACTS.iter().any(|(_, a)| *a == from);
            let bare = it.body.attachments.iter().any(|a| !a.is_inline) && visible.split_whitespace().map(str::len).sum::<usize>() < 80;
            let any_sign = analysis.severity() >= Some(Severity::Caution) || filter::code_subject(&it.message.subject) || it.message.importance == Importance::High || bare;
            Prepared { analysis, features, visible, vec: None, known, any_sign, llm: None, ms_checks: t.elapsed().as_secs_f64() * 1e3, ms_embed: 0.0, ms_llm: 0.0 }
        })
        .collect();
    if let Some(e) = embedder {
        let texts: Vec<String> = set.items.iter().map(|it| meaning_text(&it.message, Some(&it.body))).collect();
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        let t = Instant::now();
        match e.embed_documents(&refs) {
            Ok(vecs) => {
                let each = t.elapsed().as_secs_f64() * 1e3 / vecs.len().max(1) as f64;
                for (p, v) in out.iter_mut().zip(vecs) {
                    p.vec = Some(v);
                    p.ms_embed = each;
                }
            }
            Err(err) => eprintln!("embedding failed, similarity candidates skipped: {err}"),
        }
    }
    if let Some(j) = judge {
        for (n, (it, p)) in set.items.iter().zip(out.iter_mut()).enumerate() {
            eprint!("\r  {}: asking the LLM {}/{}", set.name, n + 1, set.items.len());
            let t = Instant::now();
            let files: Vec<String> = it.body.attachments.iter().map(|a| a.name.clone()).collect();
            let from = format!("{} <{}>", it.message.from.name, it.message.from.address);
            match j.judge(&from, &it.message.subject, &p.visible, &files, p.known) {
                Ok(v) => p.llm = Some(v),
                Err(err) => eprintln!("\n  {}: LLM failed: {err}", it.id),
            }
            p.ms_llm = t.elapsed().as_secs_f64() * 1e3;
        }
        eprintln!();
    }
    out
}

/// Similarity vote: how much closer a message is to the reported junk
/// than to the reported good mail (mean of the `K` nearest each side).
struct Knn {
    refs: Vec<(Vec<f32>, bool)>,
    margin: f32,
}

impl Knn {
    fn score(&self, v: &[f32], skip: Option<usize>) -> f32 {
        let mut junk = Vec::new();
        let mut good = Vec::new();
        for (i, (r, unwanted)) in self.refs.iter().enumerate() {
            if Some(i) == skip {
                continue;
            }
            if *unwanted { junk.push(similarity(v, r)) } else { good.push(similarity(v, r)) }
        }
        let top = |mut xs: Vec<f32>| {
            xs.sort_by(|a, b| b.total_cmp(a));
            let n = xs.len().clamp(1, K);
            xs.iter().take(n).sum::<f32>() / n as f32
        };
        top(junk) - top(good)
    }

    /// The cut-off: above every report of wanted mail, scored without
    /// itself (leave one out), plus a little room.
    fn fit(refs: Vec<(Vec<f32>, bool)>) -> Self {
        let mut k = Knn { refs, margin: 0.0 };
        let highest = (0..k.refs.len()).filter(|&i| !k.refs[i].1).map(|i| k.score(&k.refs[i].0.clone(), Some(i))).fold(f32::MIN, f32::max);
        k.margin = (highest + 0.02).max(0.03);
        k
    }
}

/// Logistic regression on the sentence vectors.
struct Lr {
    w: Vec<f32>,
    b: f32,
    threshold: f32,
}

impl Lr {
    fn train(xs: &[(&[f32], bool)]) -> (Vec<f32>, f32) {
        let dim = xs.first().map_or(0, |x| x.0.len());
        let (mut w, mut b) = (vec![0.0f32; dim], 0.0f32);
        let pos = xs.iter().filter(|x| x.1).count().max(1) as f32;
        let neg = xs.iter().filter(|x| !x.1).count().max(1) as f32;
        let (wp, wn) = ((pos + neg) / (2.0 * pos), (pos + neg) / (2.0 * neg));
        for _ in 0..300 {
            for (x, y) in xs {
                let p = sigmoid(b + dot(&w, x));
                let g = (p - if *y { 1.0 } else { 0.0 }) * if *y { wp } else { wn };
                for (wi, xi) in w.iter_mut().zip(x.iter()) {
                    *wi -= 0.5 * (g * xi + 1e-3 * *wi);
                }
                b -= 0.5 * g;
            }
        }
        (w, b)
    }

    fn p(&self, v: &[f32]) -> f32 {
        sigmoid(self.b + dot(&self.w, v))
    }

    /// Trained on all reports; the cut-off is above every wanted report
    /// scored by a model trained without it (leave one out).
    fn fit(refs: &[(Vec<f32>, bool)]) -> Self {
        let all: Vec<(&[f32], bool)> = refs.iter().map(|(v, y)| (v.as_slice(), *y)).collect();
        let (w, b) = Self::train(&all);
        let mut highest: f32 = 0.5;
        for i in (0..refs.len()).filter(|&i| !refs[i].1) {
            let rest: Vec<(&[f32], bool)> = all.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, x)| *x).collect();
            let (wi, bi) = Self::train(&rest);
            highest = highest.max(sigmoid(bi + dot(&wi, &refs[i].0)));
        }
        Lr { w, b, threshold: (highest + 0.02).min(0.99) }
    }
}

fn sigmoid(z: f32) -> f32 {
    1.0 / (1.0 + (-z).exp())
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// What was learned from the reports.
struct Learned {
    personal: learn::Model,
    knn: Option<Knn>,
    lr: Option<Lr>,
}

fn learn_from(reports: &Set, prepared: &[Prepared]) -> Learned {
    let examples: Vec<learn::Example> = reports
        .items
        .iter()
        .zip(prepared)
        .map(|(it, p)| learn::Example { features: p.features.clone(), unwanted: it.label.unwanted(), phishing: it.label == Label::Phishing, id: it.id.clone() })
        .collect();
    let mut personal = learn::train(None, &examples, learn::TrainParams::default());
    let wanted: Vec<learn::Example> = examples.iter().filter(|e| !e.unwanted).cloned().collect();
    personal.threshold = learn::choose_threshold(&personal, &wanted, 0.0);
    let refs: Option<Vec<(Vec<f32>, bool)>> = reports.items.iter().zip(prepared).map(|(it, p)| p.vec.clone().map(|v| (v, it.label.unwanted()))).collect();
    let (knn, lr) = match refs {
        Some(r) => (Some(Knn::fit(r.clone())), Some(Lr::fit(&r))),
        None => (None, None),
    };
    Learned { personal, knn, lr }
}

/// One candidate's decision: `Some(why)` moves the message to Junk Email.
type Decide = fn(&Item, &Prepared, &Learned) -> Option<String>;

struct Candidate {
    name: &'static str,
    about: &'static str,
    needs_vectors: bool,
    needs_llm: bool,
    decide: Decide,
}

fn shipped(it: &Item, p: &Prepared, model_says_junk: bool) -> Option<String> {
    if p.known {
        return None;
    }
    let r = filter::junk_reasons(&it.message, &p.visible, &it.body.attachments, &p.analysis, model_says_junk);
    (!r.is_empty()).then(|| r.join(" "))
}

fn knn_says(p: &Prepared, l: &Learned) -> Option<f32> {
    let (k, v) = (l.knn.as_ref()?, p.vec.as_ref()?);
    let s = k.score(v, None);
    (s >= k.margin).then_some(s)
}

fn lr_says(p: &Prepared, l: &Learned) -> Option<f32> {
    let (m, v) = (l.lr.as_ref()?, p.vec.as_ref()?);
    let s = m.p(v);
    (s >= m.threshold).then_some(s)
}

fn llm_says(p: &Prepared) -> Option<&llm::Verdict> {
    p.llm.as_ref().filter(|v| v.unwanted && v.confidence >= LLM_CONFIDENT)
}

fn kind(v: &llm::Verdict) -> &'static str {
    if v.phishing { "phishing" } else { "spam" }
}

fn candidates() -> Vec<Candidate> {
    vec![
        Candidate {
            name: "filter",
            about: "The junk filter as shipped: warning signs and points, strangers only",
            needs_vectors: false,
            needs_llm: false,
            decide: |it, p, _| shipped(it, p, false),
        },
        Candidate {
            name: "filter+learner",
            about: "Plus the personal word model trained on the reports",
            needs_vectors: false,
            needs_llm: false,
            decide: |it, p, l| shipped(it, p, l.personal.score(&p.features) >= l.personal.threshold),
        },
        Candidate {
            name: "similar",
            about: "Sentence model: closer to reported junk than to reported good mail",
            needs_vectors: true,
            needs_llm: false,
            decide: |_, p, l| (!p.known).then(|| knn_says(p, l).map(|s| format!("similar to reported junk ({s:.3})"))).flatten(),
        },
        Candidate {
            name: "filter+similar",
            about: "The filter, or the similarity vote",
            needs_vectors: true,
            needs_llm: false,
            decide: |it, p, l| shipped(it, p, false).or_else(|| (!p.known).then(|| knn_says(p, l).map(|s| format!("similar to reported junk ({s:.3})"))).flatten()),
        },
        Candidate {
            name: "vector-lr",
            about: "Sentence model plus a classifier trained on the reports",
            needs_vectors: true,
            needs_llm: false,
            decide: |_, p, l| (!p.known).then(|| lr_says(p, l).map(|s| format!("classifier {s:.2}"))).flatten(),
        },
        Candidate {
            name: "filter+vector-lr",
            about: "The filter, or the vector classifier",
            needs_vectors: true,
            needs_llm: false,
            decide: |it, p, l| shipped(it, p, false).or_else(|| (!p.known).then(|| lr_says(p, l).map(|s| format!("classifier {s:.2}"))).flatten()),
        },
        Candidate {
            name: "llm",
            about: "The local LLM's verdict alone (confident), strangers only",
            needs_vectors: false,
            needs_llm: true,
            decide: |_, p, _| (!p.known).then(|| llm_says(p).map(|v| format!("LLM ({}): {}", kind(v), v.reason))).flatten(),
        },
        Candidate {
            name: "filter+llm-backed",
            about: "The filter, or a confident LLM verdict backed by a sign or the similarity vote (Axentra Mail's rule)",
            needs_vectors: false,
            needs_llm: true,
            decide: |it, p, l| {
                shipped(it, p, false).or_else(|| {
                    let backed = p.any_sign || knn_says(p, l).is_some();
                    (!p.known && backed).then(|| llm_says(p).map(|v| format!("LLM ({}), backed: {}", kind(v), v.reason))).flatten()
                })
            },
        },
    ]
}

#[derive(Default)]
struct Tally {
    wanted: usize,
    spam: usize,
    phishing: usize,
    wanted_moved: Vec<(String, String)>,
    spam_moved: usize,
    phishing_moved: usize,
    missed: Vec<String>,
    ms: Vec<f64>,
}

fn run_set(set: &Set, prepared: &[Prepared], learned: &Learned, cands: &[Candidate], have_vectors: bool, have_llm: bool) -> Value {
    let wanted = set.items.iter().filter(|i| i.label == Label::Wanted).count();
    let spam = set.items.iter().filter(|i| i.label == Label::Spam).count();
    let phishing = set.items.iter().filter(|i| i.label == Label::Phishing).count();
    println!("\n{} ({} messages: {wanted} wanted, {spam} spam, {phishing} phishing)", set.name, set.items.len());
    println!("  {:<20} {:>13} {:>12} {:>16} {:>9}", "candidate", "wanted moved", "spam caught", "phishing caught", "ms/msg");
    let mut out = serde_json::Map::new();
    let mut decisions: HashMap<&str, Vec<Value>> = HashMap::new();
    for c in cands {
        if (c.needs_vectors && !have_vectors) || (c.needs_llm && !have_llm) {
            println!("  {:<20} {:>13}", c.name, "not run");
            continue;
        }
        let mut t = Tally { wanted, spam, phishing, ..Default::default() };
        for (it, p) in set.items.iter().zip(prepared) {
            let time = Instant::now();
            let moved = (c.decide)(it, p, learned);
            let mut ms = p.ms_checks + time.elapsed().as_secs_f64() * 1e3;
            if c.needs_vectors || c.name.contains("similar") || c.needs_llm {
                ms += p.ms_embed;
            }
            if c.needs_llm {
                ms += p.ms_llm;
            }
            t.ms.push(ms);
            match (&moved, it.label) {
                (Some(why), Label::Wanted) => t.wanted_moved.push((it.id.clone(), why.clone())),
                (Some(_), Label::Spam) => t.spam_moved += 1,
                (Some(_), Label::Phishing) => t.phishing_moved += 1,
                (None, Label::Spam | Label::Phishing) => t.missed.push(it.id.clone()),
                (None, Label::Wanted) => {}
            }
            decisions.entry(c.name).or_default().push(json!({ "id": it.id, "label": format!("{:?}", it.label), "moved": moved.is_some(), "why": moved }));
        }
        t.ms.sort_by(|a, b| a.total_cmp(b));
        let median = t.ms.get(t.ms.len() / 2).copied().unwrap_or(0.0);
        println!(
            "  {:<20} {:>13} {:>12} {:>16} {:>9.1}",
            c.name,
            format!("{}/{}", t.wanted_moved.len(), t.wanted),
            format!("{}/{}", t.spam_moved, t.spam),
            format!("{}/{}", t.phishing_moved, t.phishing),
            median
        );
        for (id, why) in &t.wanted_moved {
            let subject = set.items.iter().find(|i| &i.id == id).map(|i| i.message.subject.as_str()).unwrap_or("");
            println!("      ! moved wanted {id} \"{subject}\": {why}");
        }
        out.insert(
            c.name.to_string(),
            json!({
                "about": c.about,
                "wanted_moved": t.wanted_moved.iter().map(|(id, why)| json!({ "id": id, "why": why })).collect::<Vec<_>>(),
                "spam_caught": t.spam_moved,
                "phishing_caught": t.phishing_moved,
                "missed": t.missed,
                "median_ms": median,
                "decisions": decisions.remove(c.name).unwrap_or_default(),
            }),
        );
    }
    json!({ "messages": set.items.len(), "wanted": wanted, "spam": spam, "phishing": phishing, "candidates": out })
}

fn find_model(explicit: Option<&Path>) -> Option<PathBuf> {
    let ok = |d: &Path| d.join("model.safetensors").is_file() && d.join("vocab.txt").is_file();
    if let Some(d) = explicit {
        return ok(d).then(|| d.to_path_buf());
    }
    axentra_embed::model_dir().or_else(|| {
        ["target/models/bge-small-en-v1.5", "/opt/axentra/outlook/models/bge-small-en-v1.5"].iter().map(PathBuf::from).find(|d| ok(d))
    })
}

fn main() {
    let a = args();
    let load = |p: &str| data::json_set(p).unwrap_or_else(|e| usage(&e));
    let reports = load(&a.reports);
    let mut sets: Vec<Set> = a.sets.iter().map(|p| load(p)).collect();
    if let Some(b) = &a.benchmark {
        sets.push(data::benchmark(b).unwrap_or_else(|e| usage(&e)));
    }
    let embedder = match find_model(a.model_dir.as_deref()) {
        Some(dir) => Embedder::load(&dir).map_err(|e| eprintln!("sentence model didn't load ({e}); similarity candidates skipped")).ok(),
        None => {
            eprintln!("no sentence model found (--model-dir, AXENTRA_MODEL_DIR, target/models or /opt/axentra); similarity candidates skipped");
            None
        }
    };
    let judge = a.llm.as_deref().and_then(|m| {
        let j = llm::Judge::connect(m);
        if j.is_none() {
            eprintln!("no Ollama server on 127.0.0.1:11434; LLM candidates skipped");
        }
        j
    });
    println!("Junk filter bake-off. Learning candidates trained on: {} ({} messages).", reports.name, reports.items.len());
    println!("\"wanted moved\" must stay at 0: each one is real mail sent to Junk Email.");
    let report_prep = prepare(&reports, embedder.as_ref(), None);
    let learned = learn_from(&reports, &report_prep);
    let cands = candidates();
    let mut results = serde_json::Map::new();
    for set in &sets {
        let prepared = prepare(set, embedder.as_ref(), judge.as_ref());
        results.insert(set.name.clone(), run_set(set, &prepared, &learned, &cands, embedder.is_some(), judge.is_some()));
    }
    let report = json!({
        "reports": reports.name,
        "llm": a.llm.as_deref().filter(|_| judge.is_some()),
        "sentence_model": embedder.as_ref().map(|_| axentra_embed::MODEL_ID),
        "cutoffs": {
            "learner": learned.personal.threshold,
            "similar_margin": learned.knn.as_ref().map(|k| k.margin),
            "vector_lr": learned.lr.as_ref().map(|l| l.threshold),
            "llm_confidence": LLM_CONFIDENT,
        },
        "sets": results,
    });
    if std::fs::create_dir_all(&a.out).is_ok() {
        let path = a.out.join("report.json");
        match std::fs::write(&path, serde_json::to_string_pretty(&report).unwrap_or_default()) {
            Ok(()) => println!("\nFull report (every decision and its reason): {}", path.display()),
            Err(e) => eprintln!("couldn't write the report: {e}"),
        }
    }
}
