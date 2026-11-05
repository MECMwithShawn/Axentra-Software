//! Labelled test mail: the repository's JSON sets (`tools/junk/*.json`)
//! and, optionally, the 358-message screening benchmark (JSON lines).

use axentra_core::{Attachment, EmailAddress, Importance, Message, MessageBody};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Label {
    Wanted,
    Spam,
    Phishing,
}

impl Label {
    pub fn unwanted(self) -> bool {
        self != Label::Wanted
    }
}

pub struct Item {
    pub id: String,
    pub label: Label,
    pub message: Message,
    pub body: MessageBody,
}

pub struct Set {
    pub name: String,
    pub items: Vec<Item>,
}

fn label_of(s: &str) -> Label {
    match s {
        "legitimate" => Label::Wanted,
        "phishing" => Label::Phishing,
        _ => Label::Spam,
    }
}

/// A `tools/junk/*.json` set: `{ "about": …, "messages": [ { id, label,
/// from_name, from, subject, html | text, attachments?, reply_to?,
/// importance? } ] }`.
pub fn json_set(path: &str) -> Result<Set, String> {
    let raw = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let v: Value = serde_json::from_str(&raw).map_err(|e| format!("{path}: {e}"))?;
    let messages = v["messages"].as_array().ok_or_else(|| format!("{path}: no \"messages\""))?;
    let items = messages
        .iter()
        .map(|m| {
            let html = m["html"].as_str().map(str::to_string);
            let text = m["text"].as_str().map(str::to_string).unwrap_or_else(|| axentra_core::html::html_to_text(html.as_deref().unwrap_or("")));
            let attachments: Vec<Attachment> = m["attachments"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|x| Attachment {
                            name: x["name"].as_str().unwrap_or("").into(),
                            content_type: x["type"].as_str().unwrap_or("").into(),
                            size: x["size"].as_i64().unwrap_or(100_000),
                            ..Default::default()
                        })
                        .collect()
                })
                .unwrap_or_default();
            Item {
                id: m["id"].as_str().unwrap_or("").into(),
                label: label_of(m["label"].as_str().unwrap_or("")),
                message: Message {
                    from: EmailAddress::new(m["from_name"].as_str().unwrap_or(""), m["from"].as_str().unwrap_or("")),
                    subject: m["subject"].as_str().unwrap_or("").into(),
                    reply_to: m["reply_to"].as_str().map(|r| vec![EmailAddress::new("", r)]).unwrap_or_default(),
                    has_attachments: !attachments.is_empty(),
                    importance: if m["importance"].as_str() == Some("high") { Importance::High } else { Importance::Normal },
                    ..Default::default()
                },
                body: MessageBody { html, text, attachments, ..Default::default() },
            }
        })
        .collect();
    Ok(Set { name: set_name(path), items })
}

fn parse_from(s: &str) -> EmailAddress {
    let s = s.trim();
    match (s.rfind('<'), s.rfind('>')) {
        (Some(a), Some(b)) if b > a => EmailAddress::new(s[..a].trim().trim_matches('"'), s[a + 1..b].trim()),
        _ => EmailAddress::new("", s),
    }
}

/// The screening benchmark (`evaluation.jsonl`): one `{ id, group, gold,
/// email: { From, Subject, Body } }` per line, links written as
/// `[link destination: …]`.
pub fn benchmark(path: &str) -> Result<Set, String> {
    let raw = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let mut items = Vec::new();
    for line in raw.lines().filter(|l| !l.trim().is_empty()) {
        let v: Value = serde_json::from_str(line).map_err(|e| format!("{path}: {e}"))?;
        let e = &v["email"];
        let group = v["group"].as_str().unwrap_or("");
        let gold = v["gold"].as_str().unwrap_or("");
        let label = if gold == "legitimate" || group.ends_with("ham") || group.ends_with("legitimate") {
            Label::Wanted
        } else if group == "fresh_phishing" || gold == "phishing" {
            Label::Phishing
        } else {
            Label::Spam
        };
        items.push(Item {
            id: format!("bench-{}", v["id"].as_str().unwrap_or("")),
            label,
            message: Message { from: parse_from(e["From"].as_str().unwrap_or("")), subject: e["Subject"].as_str().unwrap_or("").into(), ..Default::default() },
            body: MessageBody { text: e["Body"].as_str().unwrap_or("").replace("[link destination: ", " ").replace("] ", " "), ..Default::default() },
        });
    }
    Ok(Set { name: "benchmark-358".into(), items })
}

fn set_name(path: &str) -> String {
    std::path::Path::new(path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| path.into())
}
