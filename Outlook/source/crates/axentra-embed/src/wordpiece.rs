//! BERT's uncased WordPiece tokenizer, as `BertTokenizer` does it for
//! bge-small: clean up the text, lower-case it and drop accents, split off
//! punctuation and CJK characters, then cut each word into the longest
//! pieces the vocabulary has.

use std::collections::HashMap;

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

/// Longer words become one [UNK], as in BERT.
const MAX_WORD_CHARS: usize = 100;

pub struct WordPiece {
    ids: HashMap<String, u32>,
    cls: u32,
    sep: u32,
    unk: u32,
}

impl WordPiece {
    /// From `vocab.txt` (one token per line, the line number is its id).
    pub fn from_vocab(vocab: &str) -> super::Result<Self> {
        let ids: HashMap<String, u32> = vocab.lines().enumerate().map(|(i, t)| (t.to_string(), i as u32)).collect();
        let get = |t: &str| ids.get(t).copied().ok_or_else(|| format!("vocab.txt has no {t}"));
        Ok(Self { cls: get("[CLS]")?, sep: get("[SEP]")?, unk: get("[UNK]")?, ids })
    }

    /// Token ids for `text`: [CLS], at most `max - 2` pieces, [SEP].
    pub fn encode(&self, text: &str, max: usize) -> Vec<u32> {
        let mut out = vec![self.cls];
        'words: for word in basic_tokens(text) {
            for id in self.pieces(&word) {
                if out.len() + 1 >= max {
                    break 'words;
                }
                out.push(id);
            }
        }
        out.push(self.sep);
        out
    }

    fn pieces(&self, word: &str) -> Vec<u32> {
        let chars: Vec<char> = word.chars().collect();
        if chars.len() > MAX_WORD_CHARS {
            return vec![self.unk];
        }
        let mut out = Vec::new();
        let mut start = 0;
        while start < chars.len() {
            let mut end = chars.len();
            let mut found = None;
            while start < end {
                let mut piece: String = chars[start..end].iter().collect();
                if start > 0 {
                    piece.insert_str(0, "##");
                }
                if let Some(&id) = self.ids.get(&piece) {
                    found = Some(id);
                    break;
                }
                end -= 1;
            }
            match found {
                Some(id) => out.push(id),
                None => return vec![self.unk],
            }
            start = end;
        }
        out
    }
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0x20000..=0x2A6DF | 0x2A700..=0x2B73F
        | 0x2B740..=0x2B81F | 0x2B820..=0x2CEAF | 0xF900..=0xFAFF | 0x2F800..=0x2FA1F)
}

/// BERT's punctuation: ASCII non-alphanumerics, and Unicode punctuation.
fn is_punct(c: char) -> bool {
    c.is_ascii_punctuation()
        || (!c.is_ascii() && !c.is_alphanumeric() && !c.is_whitespace() && !c.is_control() && !is_combining_mark(c) && !is_symbol(c))
}

/// Currency and other symbols aren't punctuation for BERT (they stay part
/// of a word); a rough test for the common ones.
fn is_symbol(c: char) -> bool {
    matches!(c as u32, 0x20A0..=0x20CF | 0x2100..=0x214F | 0x2190..=0x23FF | 0x2500..=0x27BF | 0x1F000..=0x1FAFF) || "£¥¢©®°±×÷¬¦§¶µ".contains(c)
}

/// Lower-cased, accent-free words and punctuation marks.
fn basic_tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut Vec<String>| {
        if !word.is_empty() {
            out.push(std::mem::take(word));
        }
    };
    for c in text.chars() {
        if c == '\0' || c == '\u{fffd}' || (c.is_control() && !c.is_whitespace()) {
            continue;
        }
        if c.is_whitespace() {
            flush(&mut word, &mut out);
            continue;
        }
        if is_cjk(c) || is_punct(c) {
            flush(&mut word, &mut out);
            out.push(c.to_string());
            continue;
        }
        for l in c.to_lowercase() {
            for d in std::iter::once(l).nfd().filter(|d| !is_combining_mark(*d)) {
                if is_punct(d) {
                    flush(&mut word, &mut out);
                    out.push(d.to_string());
                } else {
                    word.push(d);
                }
            }
        }
    }
    flush(&mut word, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_split_into_known_pieces() {
        let vocab = "[PAD]\n[UNK]\n[CLS]\n[SEP]\nhello\n,\nworld\nun\n##aff\n##able\ncafe\n!";
        let wp = WordPiece::from_vocab(vocab).unwrap();
        assert_eq!(basic_tokens("Héllo, WORLD!  Café\u{0}"), ["hello", ",", "world", "!", "cafe"]);
        assert_eq!(wp.encode("Hello, unaffable world", 512), [2, 4, 5, 7, 8, 9, 6, 3]);
        assert_eq!(wp.encode("zzz", 512), [2, 1, 3], "unknown word");
        assert_eq!(wp.encode("hello world hello", 4), [2, 4, 6, 3], "capped, [SEP] kept");
        assert_eq!(basic_tokens("日本x"), ["日", "本", "x"]);
    }
}
