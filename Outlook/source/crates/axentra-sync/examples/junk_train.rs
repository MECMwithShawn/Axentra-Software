//! Train and evaluate Axentra's shipped junk model, reproducibly.
//!
//! ```text
//! cargo run --release -p axentra-sync --example junk_train -- \
//!     <train-pool.jsonl from tools/junk/prepare.py> \
//!     <benchmark evaluation.jsonl (358 messages)> \
//!     tools/junk/fresh-validation.json \
//!     <out dir>
//! ```
//!
//! Splits: the SpamAssassin pool (benchmark messages and their duplicates
//! already removed) is split by campaign group into training (85%) and a
//! development set (15%) used only to choose the threshold. The 358-message
//! benchmark is a regression check, the fresh validation set a separate
//! check; neither is trained on or used for tuning. Writes `base_model.bin`,
//! `gate_fixtures.bin` (features only, no text) and `report.json`.

use std::time::Instant;

use axentra_core::junk::analyze::{self, Analysis, Severity};
use axentra_core::junk::learn::{self, Example, Fixture, Manifest, TrainParams};
use axentra_core::{Attachment, EmailAddress, Message, MessageBody};
use base64::Engine as _;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn sha(s: &str) -> String {
    Sha256::digest(s.as_bytes()).iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// One message ready for the pipeline.
struct Item {
    id: String,
    unwanted: bool,
    phishing: bool,
    message: Message,
    body: MessageBody,
}

fn parse_from(s: &str) -> EmailAddress {
    let s = s.trim();
    match (s.rfind('<'), s.rfind('>')) {
        (Some(a), Some(b)) if b > a => EmailAddress::new(s[..a].trim().trim_matches('"'), s[a + 1..b].trim()),
        _ => EmailAddress::new("", s),
    }
}

/// The invented user of the fresh sets: works at Contoso, banks with
/// Northwind, and has these contacts.
fn known_people(familiar: &[String]) -> Vec<(String, Vec<String>)> {
    if familiar.is_empty() {
        return Vec::new();
    }
    [("daniel kim", "daniel.kim@contoso.example"), ("priya raman", "priya.raman@northwind.example"), ("hannah okafor", "hannah.okafor@contoso.example"), ("maya chen", "maya.chen@riverbend.example"), ("marcus lee", "marcus.lee@contoso.example")]
        .iter()
        .map(|(n, a)| (n.to_string(), vec![a.to_string()]))
        .collect()
}

fn analysis_of(it: &Item, familiar: &[String]) -> Analysis {
    analyze::analyze(&analyze::Input {
        message: &it.message,
        html: it.body.html.as_deref(),
        text: &it.body.text,
        attachments: &it.body.attachments,
        familiar_domains: familiar,
        trusted_reporters: &[],
        known_people: &known_people(familiar),
    })
}

fn features(it: &Item, familiar: &[String]) -> (Vec<u32>, Analysis) {
    let a = analysis_of(it, familiar);
    (learn::features(&it.message, Some(&it.body), &a), a)
}

fn pool(path: &str) -> Vec<(Item, String)> {
    let text = std::fs::read_to_string(path).expect("training pool");
    let mut out = Vec::new();
    for line in text.lines() {
        let v: Value = serde_json::from_str(line).expect("pool line");
        let raw = base64::engine::general_purpose::STANDARD.decode(v["raw"].as_str().unwrap_or("")).unwrap_or_default();
        let Some(p) = axentra_sync::mime::parse::parse_rfc822(&raw) else { continue };
        let mut body = p.body;
        body.attachments = p.attachments.into_iter().map(|(a, _)| a).collect();
        out.push((
            Item { id: v["id"].as_str().unwrap_or("").into(), unwanted: v["unwanted"].as_bool().unwrap_or(false), phishing: false, message: p.message, body },
            v["group"].as_str().unwrap_or("").to_string(),
        ));
    }
    out
}

fn benchmark(path: &str) -> Vec<Item> {
    let text = std::fs::read_to_string(path).expect("benchmark");
    text.lines()
        .map(|line| {
            let v: Value = serde_json::from_str(line).expect("benchmark line");
            let e = &v["email"];
            let group = v["group"].as_str().unwrap_or("");
            let gold = v["gold"].as_str().unwrap_or("");
            let unwanted = gold != "legitimate" && !group.ends_with("legitimate") && !group.ends_with("ham");
            Item {
                id: format!("bench-{}", v["id"].as_str().unwrap_or("")),
                unwanted,
                phishing: group == "fresh_phishing",
                message: Message {
                    from: parse_from(e["From"].as_str().unwrap_or("")),
                    subject: e["Subject"].as_str().unwrap_or("").into(),
                    ..Default::default()
                },
                body: MessageBody { text: e["Body"].as_str().unwrap_or("").replace("[link destination: ", " ").replace("] ", " "), ..Default::default() },
            }
        })
        .collect()
}

fn fresh(path: &str) -> Vec<Item> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path).expect("fresh set")).expect("fresh json");
    v["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .map(|m| {
            let label = m["label"].as_str().unwrap_or("");
            let html = m["html"].as_str().map(str::to_string);
            let text = m["text"].as_str().map(str::to_string).unwrap_or_else(|| axentra_core::html::html_to_text(html.as_deref().unwrap_or("")));
            let attachments = m["attachments"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|x| Attachment { name: x["name"].as_str().unwrap_or("").into(), content_type: x["type"].as_str().unwrap_or("").into(), size: 100_000, ..Default::default() })
                        .collect()
                })
                .unwrap_or_default();
            Item {
                id: m["id"].as_str().unwrap_or("").into(),
                unwanted: label != "legitimate",
                phishing: label == "phishing",
                message: Message {
                    from: EmailAddress::new(m["from_name"].as_str().unwrap_or(""), m["from"].as_str().unwrap_or("")),
                    subject: m["subject"].as_str().unwrap_or("").into(),
                    reply_to: m["reply_to"].as_str().map(|r| vec![EmailAddress::new("", r)]).unwrap_or_default(),
                    has_attachments: m["attachments"].is_array(),
                    ..Default::default()
                },
                body: MessageBody { html, text, attachments, ..Default::default() },
            }
        })
        .collect()
}

/// The whole advisory pipeline on one set: warning signs plus the model.
fn pipeline(model: &learn::Model, items: &[Item], familiar: &[String]) -> Value {
    let (mut legit, mut unwanted, mut phish) = (0, 0, 0);
    let (mut legit_any, mut legit_model, mut legit_danger, mut legit_caution) = (0, 0, 0, 0);
    let (mut unwanted_flagged, mut unwanted_model, mut unwanted_unnoticed) = (0, 0, 0);
    let (mut phish_warned, mut phish_only_model, mut phish_caution, mut phish_missed) = (0, 0, 0, 0);
    let mut times = Vec::new();
    let mut false_flags = Vec::new();
    let mut unwanted_items = Vec::new();
    for it in items {
        let t = Instant::now();
        let (f, a) = features(it, familiar);
        let score = model.score(&f);
        times.push(t.elapsed().as_secs_f64() * 1000.0);
        let flagged_model = score >= model.threshold;
        let sev = a.severity();
        let danger = sev == Some(Severity::Danger);
        let caution = sev == Some(Severity::Caution);
        if it.unwanted {
            unwanted_items.push(json!({ "id": it.id, "phishing": it.phishing, "score": score, "severity": format!("{sev:?}"), "signs": a.findings.iter().map(|f| f.describe()).collect::<Vec<_>>() }));
            unwanted += 1;
            unwanted_model += usize::from(flagged_model);
            if flagged_model || danger {
                unwanted_flagged += 1;
            } else if !caution {
                unwanted_unnoticed += 1;
            }
            if it.phishing {
                phish += 1;
                if danger {
                    phish_warned += 1;
                } else if flagged_model {
                    phish_only_model += 1;
                } else if caution {
                    phish_caution += 1;
                } else {
                    phish_missed += 1;
                }
            }
        } else {
            legit += 1;
            legit_model += usize::from(flagged_model);
            legit_danger += usize::from(danger);
            legit_caution += usize::from(caution && !flagged_model);
            if flagged_model || danger || caution {
                legit_any += 1;
                false_flags.push(json!({ "id": it.id, "score": score, "signs": a.findings.iter().map(|f| f.describe()).collect::<Vec<_>>() }));
            }
        }
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let pct = |p: f64| times.get(((times.len() as f64 - 1.0) * p).round() as usize).copied().unwrap_or(0.0);
    json!({
        "legitimate": legit,
        "legitimate_with_any_banner": legit_any,
        "legitimate_flagged_by_model": legit_model,
        "legitimate_with_phishing_warning": legit_danger,
        "legitimate_with_caution_only": legit_caution,
        "unwanted": unwanted,
        "unwanted_flagged": unwanted_flagged,
        "unwanted_flagged_by_model": unwanted_model,
        "unwanted_with_no_banner": unwanted_unnoticed,
        "phishing": phish,
        "phishing_with_phishing_warning": phish_warned,
        "phishing_only_junk_suggestion": phish_only_model,
        "phishing_caution_only": phish_caution,
        "phishing_missed": phish_missed,
        "latency_ms_median": pct(0.5),
        "latency_ms_p95": pct(0.95),
        "false_flags": false_flags,
        "unwanted_items": unwanted_items,
    })
}

fn gate_vs_rules_only(cand: &learn::Model, sets: &[(&str, &[Example])]) -> learn::Verdict {
    let reference = learn::Model::rules_only();
    let gs: Vec<learn::GateSet> = sets
        .iter()
        .map(|(name, xs)| learn::GateSet {
            name: name.to_string(),
            active: learn::evaluate(&reference, xs, reference.threshold),
            candidate: learn::evaluate(cand, xs, cand.threshold),
            needs_phishing: true,
        })
        .collect();
    learn::gate(&gs)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 6 {
        eprintln!("usage: junk_train <train-pool.jsonl> <benchmark.jsonl> <modern-dev.json> <fresh-validation.json> <out dir>");
        std::process::exit(2);
    }
    let out = std::path::Path::new(&args[5]);
    std::fs::create_dir_all(out).expect("out dir");
    let none: Vec<String> = Vec::new();
    // The synthetic sets' invented user works at Contoso and banks with Northwind.
    let familiar: Vec<String> =
        ["contoso.example", "northwind.example", "lumen.example", "northwindbank.example", "riverbend.example"].iter().map(|s| s.to_string()).collect();

    let pool = pool(&args[1]);
    let (mut train_items, mut dev_items) = (Vec::new(), Vec::new());
    for (it, group) in pool {
        // Split by campaign group so near-duplicates never straddle sets.
        if u8::from_str_radix(&sha(&format!("split:{group}"))[..2], 16).unwrap_or(0) < 38 { dev_items.push(it) } else { train_items.push(it) }
    }
    let bench = benchmark(&args[2]);
    let modern = fresh(&args[3]);
    let fresh = fresh(&args[4]);

    let ex = |items: &[Item], familiar: &[String]| -> Vec<Example> {
        items.iter().map(|it| Example { features: features(it, familiar).0, unwanted: it.unwanted, phishing: it.phishing, id: it.id.clone() }).collect()
    };
    let env = |k: &str, d: f32| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
    let d = TrainParams::default();
    let params = TrainParams { epochs: env("JUNK_EPOCHS", d.epochs as f32) as u32, learning_rate: env("JUNK_LR", d.learning_rate), l2: env("JUNK_L2", d.l2) };
    let train_ex = ex(&train_items, &none);
    let dev_ex = ex(&dev_items, &none);
    let modern_ex = ex(&modern, &familiar);
    let bench_ex = ex(&bench, &none);
    let fresh_ex = ex(&fresh, &familiar);
    let count = |xs: &[Example], u: bool| xs.iter().filter(|e| e.unwanted == u).count();

    // 1. The bootstrap candidate: SpamAssassin only.
    let t = Instant::now();
    let mut boot = learn::train(None, &train_ex, params);
    let train_secs = t.elapsed().as_secs_f64();
    boot.threshold = learn::choose_threshold(&boot, &dev_ex, 0.01);
    boot.manifest = Manifest {
        kind: "base".into(),
        examples_unwanted: count(&train_ex, true),
        examples_wanted: count(&train_ex, false),
        epochs: params.epochs,
        learning_rate: params.learning_rate,
        l2: params.l2,
        seed_data: "SpamAssassin public corpus (20030228 easy_ham, hard_ham; 20050311 spam_2), screening-benchmark messages and duplicates removed".into(),
        ..Default::default()
    };
    let boot_verdict = gate_vs_rules_only(&boot, &[("regression fixtures", &bench_ex), ("fresh validation", &fresh_ex)]);
    boot.manifest.gate = Some(boot_verdict.clone());
    let boot = boot.seal();
    std::fs::write(out.join("bootstrap_candidate.bin"), boot.to_bytes()).expect("write model");

    // 2. A first personal model as the app would build it from ~60 reports
    //    (the modern development set standing in for them): from scratch,
    //    threshold above every wanted message in the development data.
    let mut personal = learn::train(None, &modern_ex, params);
    let mut threshold_dev: Vec<Example> = dev_ex.clone();
    threshold_dev.extend(modern_ex.iter().filter(|e| !e.unwanted).cloned());
    personal.threshold = learn::choose_threshold(&personal, &threshold_dev, 0.0);
    let personal_verdict = gate_vs_rules_only(&personal, &[("regression fixtures", &bench_ex), ("fresh validation", &fresh_ex)]);

    // Shipped checks: development (SpamAssassin sample + modern dev),
    // regression (358) and fresh validation. Features only.
    let mut fixtures: Vec<Fixture> = Vec::new();
    let mut dev_sample: Vec<&Example> = dev_ex.iter().collect();
    dev_sample.sort_by_key(|e| sha(&format!("dev:{}", e.id)));
    for e in dev_sample.into_iter().take(400).chain(modern_ex.iter()) {
        fixtures.push(Fixture { id: e.id.clone(), set: "dev".into(), unwanted: e.unwanted, phishing: e.phishing, f: e.features.clone() });
    }
    for (set, xs) in [("regression", &bench_ex), ("fresh", &fresh_ex)] {
        for e in xs.iter() {
            fixtures.push(Fixture { id: e.id.clone(), set: set.into(), unwanted: e.unwanted, phishing: e.phishing, f: e.features.clone() });
        }
    }
    std::fs::write(out.join("gate_fixtures.bin"), learn::fixtures_to_bytes(&fixtures)).expect("write fixtures");

    let rules_only = learn::Model::rules_only();
    let report = json!({
        "features": learn::FEATURE_VERSION,
        "analysis": analyze::ANALYSIS_VERSION,
        "params": { "epochs": params.epochs, "learning_rate": params.learning_rate, "l2": params.l2 },
        "data": {
            "spamassassin_train": { "messages": train_ex.len(), "unwanted": count(&train_ex, true), "legitimate": count(&train_ex, false) },
            "spamassassin_dev": { "messages": dev_ex.len(), "unwanted": count(&dev_ex, true), "legitimate": count(&dev_ex, false) },
            "regression": { "messages": bench_ex.len(), "unwanted": count(&bench_ex, true), "phishing": bench_ex.iter().filter(|e| e.phishing).count() },
            "modern_dev": { "messages": modern_ex.len(), "unwanted": count(&modern_ex, true), "phishing": modern_ex.iter().filter(|e| e.phishing).count() },
            "fresh_validation": { "messages": fresh_ex.len(), "unwanted": count(&fresh_ex, true), "phishing": fresh_ex.iter().filter(|e| e.phishing).count() },
        },
        "rules_only": {
            "regression": pipeline(&rules_only, &bench, &none),
            "modern_dev": pipeline(&rules_only, &modern, &familiar),
            "fresh_validation": pipeline(&rules_only, &fresh, &familiar),
        },
        "bootstrap_spamassassin": {
            "digest": boot.digest,
            "threshold": boot.threshold,
            "train_seconds": train_secs,
            "nonzero_weights": boot.weights.len(),
            "dev": learn::evaluate(&boot, &dev_ex, boot.threshold),
            "regression": pipeline(&boot, &bench, &none),
            "modern_dev": pipeline(&boot, &modern, &familiar),
            "fresh_validation": pipeline(&boot, &fresh, &familiar),
            "gate_vs_rules_only": boot_verdict,
        },
        "simulated_first_personal_model": {
            "trained_on": "modern-dev.json (60 labelled messages standing in for a user's first reports)",
            "threshold": personal.threshold,
            "regression": pipeline(&personal, &bench, &none),
            "fresh_validation": pipeline(&personal, &fresh, &familiar),
            "gate_vs_rules_only": personal_verdict,
        },
    });
    std::fs::write(out.join("report.json"), serde_json::to_string_pretty(&report).unwrap()).expect("write report");
    println!("wrote {}", out.display());
}
