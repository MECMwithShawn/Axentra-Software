//! Meaning search storage: one vector per message (made by the sentence
//! model in `axentra-embed`, which this crate doesn't depend on), and the
//! lookup search uses to add messages about the same thing as the words
//! typed. The app installs a [`QueryEmbedder`] at start; without one,
//! search is keyword only.

use std::sync::Arc;

use rusqlite::params;

use super::Store;
use super::mail::id_list;
use crate::Result;
use crate::model::*;

pub(crate) const SCHEMA_EXTRA: &str = r#"
CREATE TABLE IF NOT EXISTS message_vectors (
    message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    model TEXT NOT NULL,
    vec BLOB NOT NULL);
CREATE TRIGGER IF NOT EXISTS message_vectors_subject AFTER UPDATE OF json ON messages
    WHEN json_extract(OLD.json, '$.subject') IS NOT json_extract(NEW.json, '$.subject')
    BEGIN DELETE FROM message_vectors WHERE message_id = NEW.id; END;
CREATE TRIGGER IF NOT EXISTS message_vectors_body_insert AFTER INSERT ON message_bodies
    BEGIN DELETE FROM message_vectors WHERE message_id = NEW.message_id; END;
CREATE TRIGGER IF NOT EXISTS message_vectors_body_update AFTER UPDATE ON message_bodies
    WHEN OLD.text IS NOT NEW.text OR OLD.html IS NOT NEW.html
    BEGIN DELETE FROM message_vectors WHERE message_id = NEW.message_id; END;
"#;

/// Turns a search into (model id, vector of length 1), or `None` when the
/// model isn't available.
pub type QueryEmbedder = Arc<dyn Fn(&str) -> Option<(String, Vec<f32>)> + Send + Sync>;

/// Characters of a message given to the model (it reads the first 128
/// tokens; this leaves room for long words).
const TEXT_CHARS: usize = 1200;
/// Most messages a search adds by meaning.
const MAX_MATCHES: usize = 40;
/// How far above a typical message's score a match must be. The model
/// scores everything 0.45–0.75, so a match is judged against the rest of
/// the mailbox (its median), not a fixed number.
const ABOVE_TYPICAL: f32 = 0.13;
/// With too few messages for a median, this score at least.
const MIN_SIMILARITY: f32 = 0.65;
const FEW: usize = 50;
/// … and how close to the best match: a search with one clear answer
/// doesn't drag in everything vaguely near it.
const NEAR_BEST: f32 = 0.07;

/// The text the model reads for a message.
pub fn message_text(m: &Message, body: Option<&MessageBody>) -> String {
    let body = body.map(crate::junk::learn::visible_text).unwrap_or_default();
    let body = body.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut text = format!("{}\n{}", m.subject.trim(), body);
    if let Some((cut, _)) = text.char_indices().nth(TEXT_CHARS) {
        text.truncate(cut);
    }
    text
}

fn vec_from_bytes(b: &[u8]) -> Vec<f32> {
    b.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect()
}

fn vec_to_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

impl Store {
    /// Use `embed` for searches (see [`QueryEmbedder`]).
    pub fn set_query_embedder(&self, embed: Option<QueryEmbedder>) {
        *self.query_embedder.lock().unwrap_or_else(|e| e.into_inner()) = embed;
    }

    /// Messages that have no vector from `model` yet, newest first, with
    /// the text to give the model. Drafts and Junk Email are left out.
    pub fn messages_to_embed(&self, model: &str, limit: u32) -> Result<Vec<(MessageId, String)>> {
        let ids: Vec<MessageId> = {
            let conn = self.conn();
            let mut stmt = conn.prepare_cached(
                "SELECT m.id FROM messages m JOIN folders f ON f.id = m.folder_id
                 LEFT JOIN message_vectors v ON v.message_id = m.id AND v.model = ?1
                 WHERE v.message_id IS NULL AND f.role NOT IN ('drafts', 'junk', 'outbox') AND m.is_draft = 0
                 ORDER BY m.received_at DESC LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![model, limit], |r| r.get(0))?;
            rows.collect::<std::result::Result<_, _>>()?
        };
        let mut out = Vec::with_capacity(ids.len());
        for m in self.messages_by_ids(&ids)? {
            let body = self.message_body(m.id)?;
            out.push((m.id, message_text(&m, body.as_ref())));
        }
        Ok(out)
    }

    /// Store vectors made by `model`. Messages deleted meanwhile are skipped.
    pub fn save_vectors(&self, model: &str, vectors: &[(MessageId, Vec<f32>)]) -> Result<()> {
        self.tx(|tx| {
            let mut stmt = tx.prepare_cached(
                "INSERT OR REPLACE INTO message_vectors (message_id, model, vec) SELECT ?1, ?2, ?3 WHERE EXISTS (SELECT 1 FROM messages WHERE id = ?1)",
            )?;
            for (id, v) in vectors {
                stmt.execute(params![id, model, vec_to_bytes(v)])?;
            }
            Ok(())
        })
    }

    /// (messages with a vector from `model`, messages that should have one).
    pub fn embedding_progress(&self, model: &str) -> Result<(i64, i64)> {
        let conn = self.conn();
        Ok(conn.query_row(
            "SELECT (SELECT COUNT(*) FROM message_vectors v JOIN messages m ON m.id = v.message_id WHERE v.model = ?1),
                    (SELECT COUNT(*) FROM messages m JOIN folders f ON f.id = m.folder_id
                     WHERE f.role NOT IN ('drafts', 'junk', 'outbox') AND m.is_draft = 0)",
            [model],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?)
    }

    /// Drop every vector (Settings: meaning search turned off).
    pub fn forget_vectors(&self) -> Result<()> {
        self.conn().execute("DELETE FROM message_vectors", [])?;
        Ok(())
    }

    /// Messages about the same thing as `text`, best first, as an SQL id
    /// list for search; `None` without a model or a close match.
    pub(crate) fn meaning_matches(&self, text: &str) -> Result<Option<String>> {
        let embed = self.query_embedder.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let Some((model, query)) = embed.and_then(|f| f(text)) else { return Ok(None) };
        let ids = self.closest(&model, &query)?;
        Ok((!ids.is_empty()).then(|| id_list(&ids)))
    }

    /// The messages closest to `query` (see [`ABOVE_TYPICAL`], [`NEAR_BEST`]).
    pub fn closest(&self, model: &str, query: &[f32]) -> Result<Vec<MessageId>> {
        let mut scored: Vec<(f32, MessageId)> = Vec::new();
        {
            let conn = self.conn();
            let mut stmt = conn.prepare_cached("SELECT message_id, vec FROM message_vectors WHERE model = ?1")?;
            let mut rows = stmt.query([model])?;
            while let Some(r) = rows.next()? {
                let v = vec_from_bytes(r.get_ref(1)?.as_blob().map_err(rusqlite::Error::from)?);
                scored.push((v.iter().zip(query).map(|(a, b)| a * b).sum(), r.get(0)?));
            }
        }
        scored.sort_by(|a, b| b.0.total_cmp(&a.0));
        let Some(best) = scored.first().map(|s| s.0) else { return Ok(Vec::new()) };
        let floor = if scored.len() < FEW { MIN_SIMILARITY } else { scored[scored.len() / 2].0 + ABOVE_TYPICAL };
        let floor = floor.max(best - NEAR_BEST);
        Ok(scored.into_iter().take_while(|s| s.0 >= floor).take(MAX_MATCHES).map(|s| s.1).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_util::store_with_account;

    #[test]
    fn vectors_are_kept_per_model_and_dropped_when_the_text_changes() {
        let (store, acct) = store_with_account();
        let inbox = store.folder_for_role(acct, FolderRole::Inbox).unwrap();
        let mut ids = Vec::new();
        for subject in ["Invoice for October", "Team lunch"] {
            let mut m = Message { account_id: acct, folder_id: inbox, subject: subject.into(), ..Default::default() };
            ids.push(store.insert_message(&mut m, Some(&MessageBody { text: "Hello there".into(), ..Default::default() })).unwrap());
        }
        let todo = store.messages_to_embed("m1", 10).unwrap();
        assert_eq!(todo.len(), 2);
        assert!(todo.iter().any(|(_, t)| t == "Invoice for October\nHello there"));
        store.save_vectors("m1", &[(ids[0], vec![1.0, 0.0]), (ids[1], vec![0.0, 1.0]), (9999, vec![1.0, 0.0])]).unwrap();
        assert_eq!(store.embedding_progress("m1").unwrap(), (2, 2));
        assert!(store.messages_to_embed("m1", 10).unwrap().is_empty());
        assert_eq!(store.messages_to_embed("m2", 10).unwrap().len(), 2, "another model's vectors don't count");

        assert_eq!(store.closest("m1", &[0.9, 0.1]).unwrap(), vec![ids[0]]);
        assert!(store.closest("m1", &[0.5, -0.5]).unwrap().is_empty(), "nothing close enough");

        store.set_message_body(ids[0], None, "New text").unwrap();
        assert_eq!(store.embedding_progress("m1").unwrap(), (1, 2));
        let mut m = store.message(ids[1]).unwrap().unwrap();
        m.is_read = true;
        store.update_message(&m).unwrap();
        assert_eq!(store.embedding_progress("m1").unwrap(), (1, 2), "reading it changes nothing");
    }

    #[test]
    fn search_adds_messages_close_in_meaning() {
        let (store, acct) = store_with_account();
        let inbox = store.folder_for_role(acct, FolderRole::Inbox).unwrap();
        let add = |subject: &str| {
            let mut m = Message { account_id: acct, folder_id: inbox, subject: subject.into(), ..Default::default() };
            store.insert_message(&mut m, None).unwrap()
        };
        let bill = add("Amount due for October");
        let lunch = add("Team lunch Friday");
        let invoice = add("Invoice attached");
        store.save_vectors("m1", &[(bill, vec![1.0, 0.0]), (lunch, vec![0.0, 1.0])]).unwrap();
        let search = |q: &str| {
            let mut ids: Vec<MessageId> =
                store.messages(&MessageQuery { search: Some(q.into()), ..Default::default() }).unwrap().into_iter().map(|m| m.id).collect();
            ids.sort();
            ids
        };
        assert_eq!(search("invoice"), vec![invoice], "keywords only without a model");
        store.set_query_embedder(Some(Arc::new(|q: &str| Some(("m1".to_string(), if q.contains("invoice") { vec![0.95, 0.05] } else { vec![0.0, 0.0] })))));
        assert_eq!(search("invoice"), vec![bill, invoice], "the bill is about the same thing");
        assert_eq!(search("from:nobody@x.example invoice"), Vec::<MessageId>::new(), "filters still apply");
        assert_eq!(search("\"invoice\""), vec![invoice], "a quoted word is exact");
        assert_eq!(search("lunch"), vec![lunch]);
    }
}
