//! BERT's WordPiece tokenizer, read from a Hugging Face `tokenizer.json`.
//!
//! This is the one tokenizer the bundled model uses, written out rather than
//! pulled in: the `tokenizers` crate is a regex engine, a thread pool and five
//! models to run one of them. What is here follows that crate's
//! `BertNormalizer`, `BertPreTokenizer` and `WordPiece` step for step, and
//! `tests/tokenizer.rs` holds it to the ids the Python package gives for the
//! same file.
//!
//! Special tokens are found in the raw text first, as the reference does even
//! when it adds none of its own, so a snippet that contains `[SEP]` gets the
//! separator's id rather than five pieces of punctuation.
//!
//! One known difference: the reference classifies characters with Unicode 9
//! tables, and this uses current ones. A character assigned since then is
//! dropped as unassigned there and kept here, where it becomes the unknown
//! token, which the model leaves out of the average anyway. The two agree
//! unless such a character sits inside a word.

use std::collections::HashMap;

use serde_json::Value;
use unicode_normalization::UnicodeNormalization;
use unicode_properties::{GeneralCategory, GeneralCategoryGroup, UnicodeGeneralCategory};

use crate::ModelError;

/// A word longer than this, in characters, is the unknown token, whatever it
/// is made of. The reference's default, used when the file does not say.
const MAX_INPUT_CHARS_PER_WORD: usize = 100;

#[derive(Debug)]
pub(crate) struct Tokenizer {
    vocab: HashMap<String, u32>,
    /// The token a word becomes when no split of it is in the vocabulary.
    unk: u32,
    /// What marks a piece that continues a word, `##` in every BERT model.
    prefix: String,
    max_chars: usize,
    /// Tokens matched in the raw text before anything else, longest first.
    special: Vec<(String, u32)>,
    normalizer: Normalizer,
}

#[derive(Debug, Clone, Copy)]
struct Normalizer {
    clean_text: bool,
    chinese_chars: bool,
    strip_accents: bool,
    lowercase: bool,
}

impl Tokenizer {
    /// Reads a `tokenizer.json`. Anything but a BERT normalizer, a BERT
    /// pre-tokenizer and a WordPiece model is refused rather than tokenized
    /// some other way: a model fed the wrong ids answers, just wrongly.
    pub(crate) fn from_json(bytes: &[u8]) -> Result<Self, ModelError> {
        let file: Value = serde_json::from_slice(bytes)
            .map_err(|error| ModelError::Tokenizer(error.to_string()))?;

        let model = &file["model"];
        expect(model, "type", "WordPiece", "model")?;
        let vocab: HashMap<String, u32> = model["vocab"]
            .as_object()
            .ok_or_else(|| ModelError::Tokenizer("the model has no vocabulary".to_owned()))?
            .iter()
            .map(|(token, id)| {
                let id = id
                    .as_u64()
                    .and_then(|id| u32::try_from(id).ok())
                    .ok_or_else(|| ModelError::Tokenizer(format!("{token:?} has no id")))?;
                Ok((token.clone(), id))
            })
            .collect::<Result<_, ModelError>>()?;
        let unk_token = model["unk_token"].as_str().unwrap_or("[UNK]");
        let unk = *vocab.get(unk_token).ok_or_else(|| {
            ModelError::Tokenizer(format!(
                "the unknown token {unk_token:?} is not in the vocabulary"
            ))
        })?;
        let prefix = model["continuing_subword_prefix"]
            .as_str()
            .unwrap_or("##")
            .to_owned();
        let max_chars = model["max_input_chars_per_word"]
            .as_u64()
            .and_then(|max| usize::try_from(max).ok())
            .unwrap_or(MAX_INPUT_CHARS_PER_WORD);

        let normalizer = &file["normalizer"];
        expect(normalizer, "type", "BertNormalizer", "normalizer")?;
        let lowercase = normalizer["lowercase"].as_bool().unwrap_or(true);
        let normalizer = Normalizer {
            clean_text: normalizer["clean_text"].as_bool().unwrap_or(true),
            chinese_chars: normalizer["handle_chinese_chars"].as_bool().unwrap_or(true),
            // Unset means "do what lowercasing does", as in the reference.
            strip_accents: normalizer["strip_accents"].as_bool().unwrap_or(lowercase),
            lowercase,
        };
        expect(
            &file["pre_tokenizer"],
            "type",
            "BertPreTokenizer",
            "pre-tokenizer",
        )?;

        let mut special: Vec<(String, u32)> = file["added_tokens"]
            .as_array()
            .map(|tokens| {
                tokens
                    .iter()
                    .filter_map(|token| {
                        let content = token["content"].as_str()?;
                        let id = u32::try_from(token["id"].as_u64()?).ok()?;
                        (!content.is_empty()).then(|| (content.to_owned(), id))
                    })
                    .collect()
            })
            .unwrap_or_default();
        // Longest first, so the leftmost match is also the longest one.
        special.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.0.cmp(&b.0)));

        Ok(Self {
            vocab,
            unk,
            prefix,
            max_chars,
            special,
            normalizer,
        })
    }

    /// Every token in the vocabulary, for working out how long a typical one
    /// is.
    pub(crate) fn tokens(&self) -> impl Iterator<Item = &str> {
        self.vocab.keys().map(String::as_str)
    }

    pub(crate) fn unknown(&self) -> u32 {
        self.unk
    }

    /// The largest id [`Tokenizer::encode`] can give.
    pub(crate) fn largest_id(&self) -> u32 {
        self.vocab
            .values()
            .chain(self.special.iter().map(|(_, id)| id))
            .copied()
            .max()
            .unwrap_or(self.unk)
    }

    /// The ids of `text`, with no `[CLS]` or `[SEP]` added around them.
    pub(crate) fn encode(&self, text: &str) -> Vec<u32> {
        let mut ids = Vec::new();
        let mut rest = text;
        while !rest.is_empty() {
            match self.next_special(rest) {
                Some((at, token, id)) => {
                    self.encode_plain(&rest[..at], &mut ids);
                    ids.push(id);
                    rest = &rest[at + token.len()..];
                }
                None => {
                    self.encode_plain(rest, &mut ids);
                    break;
                }
            }
        }
        ids
    }

    /// The first special token in `text`, the longest where several start at
    /// the same place, with where it starts.
    fn next_special(&self, text: &str) -> Option<(usize, &str, u32)> {
        let mut found: Option<(usize, &str, u32)> = None;
        for (token, id) in &self.special {
            if let Some(at) = text.find(token.as_str()) {
                // `special` is longest first, so a later token at the same
                // place is shorter and loses.
                if found.is_none_or(|(best, _, _)| at < best) {
                    found = Some((at, token, *id));
                }
            }
        }
        found
    }

    fn encode_plain(&self, text: &str, ids: &mut Vec<u32>) {
        if text.is_empty() {
            return;
        }
        let normalized = self.normalizer.apply(text);
        for word in words(&normalized) {
            self.word_pieces(word, ids);
        }
    }

    /// Greedy longest-match-first, as WordPiece has always been: the longest
    /// prefix in the vocabulary, then the longest `##` piece after it, and so
    /// on. A word with any stretch that no piece covers is the unknown token
    /// as a whole, not in part.
    fn word_pieces(&self, word: &str, ids: &mut Vec<u32>) {
        if word.chars().count() > self.max_chars {
            ids.push(self.unk);
            return;
        }
        let first = ids.len();
        let mut piece = String::new();
        let mut start = 0;
        while start < word.len() {
            let mut end = word.len();
            let mut found = None;
            while start < end {
                piece.clear();
                if start > 0 {
                    piece.push_str(&self.prefix);
                }
                piece.push_str(&word[start..end]);
                if let Some(&id) = self.vocab.get(&piece) {
                    found = Some(id);
                    break;
                }
                end -= word[..end].chars().next_back().map_or(1, char::len_utf8);
            }
            let Some(id) = found else {
                ids.truncate(first);
                ids.push(self.unk);
                return;
            };
            ids.push(id);
            start = end;
        }
    }
}

impl Normalizer {
    fn apply(&self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        for character in text.chars() {
            if self.clean_text {
                if character == '\0' || character == '\u{fffd}' || is_control(character) {
                    continue;
                }
                if is_whitespace(character) {
                    out.push(' ');
                    continue;
                }
            }
            if self.chinese_chars && is_chinese(character) {
                out.push(' ');
                out.push(character);
                out.push(' ');
            } else {
                out.push(character);
            }
        }
        if self.strip_accents {
            out = out
                .nfd()
                .filter(|&character| {
                    character.general_category() != GeneralCategory::NonspacingMark
                })
                .collect();
        }
        if self.lowercase {
            // Character by character, as the reference does, so a final sigma
            // stays a plain one.
            out = out.chars().flat_map(char::to_lowercase).collect();
        }
        out
    }
}

/// Words split at whitespace, with every punctuation character a word of its
/// own.
fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(char::is_whitespace)
        .flat_map(|chunk| {
            let mut pieces = Vec::new();
            let mut start = 0;
            for (at, character) in chunk.char_indices() {
                if is_punctuation(character) {
                    pieces.push(&chunk[start..at]);
                    pieces.push(&chunk[at..at + character.len_utf8()]);
                    start = at + character.len_utf8();
                }
            }
            pieces.push(&chunk[start..]);
            pieces
        })
        .filter(|word| !word.is_empty())
}

fn expect(section: &Value, key: &str, wanted: &str, what: &str) -> Result<(), ModelError> {
    match section[key].as_str() {
        Some(found) if found == wanted => Ok(()),
        Some(found) => Err(ModelError::Tokenizer(format!(
            "the {what} is {found}, and only {wanted} is supported"
        ))),
        None => Err(ModelError::Tokenizer(format!(
            "the file names no {what}, and only {wanted} is supported"
        ))),
    }
}

/// Tab, newline and carriage return are control characters that the reference
/// counts as whitespace instead.
fn is_whitespace(character: char) -> bool {
    matches!(character, '\t' | '\n' | '\r') || character.is_whitespace()
}

fn is_control(character: char) -> bool {
    !matches!(character, '\t' | '\n' | '\r')
        && character.general_category_group() == GeneralCategoryGroup::Other
}

/// ASCII punctuation includes `$`, `+`, `<`, `=`, `>`, `^`, `` ` ``, `|` and
/// `~`, which Unicode calls symbols.
fn is_punctuation(character: char) -> bool {
    character.is_ascii_punctuation()
        || character.general_category_group() == GeneralCategoryGroup::Punctuation
}

/// The CJK blocks the reference spaces out, including its range that starts at
/// U+2B920 where the block starts at U+2B820.
fn is_chinese(character: char) -> bool {
    matches!(
        u32::from(character),
        0x4E00..=0x9FFF
            | 0x3400..=0x4DBF
            | 0x20000..=0x2A6DF
            | 0x2A700..=0x2B73F
            | 0x2B740..=0x2B81F
            | 0x2B920..=0x2CEAF
            | 0xF900..=0xFAFF
            | 0x2F800..=0x2FA1F
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokenizer() -> Tokenizer {
        let file = serde_json::json!({
            "added_tokens": [
                { "id": 0, "content": "[UNK]" },
                { "id": 1, "content": "[SEP]" },
            ],
            "normalizer": { "type": "BertNormalizer", "lowercase": true },
            "pre_tokenizer": { "type": "BertPreTokenizer" },
            "model": {
                "type": "WordPiece",
                "unk_token": "[UNK]",
                "continuing_subword_prefix": "##",
                "max_input_chars_per_word": 10,
                "vocab": {
                    "[UNK]": 0, "[SEP]": 1, "un": 2, "##aff": 3, "##able": 4,
                    "refund": 5, "!": 6, "cafe": 7, "a": 8, "##b": 9,
                },
            },
        });
        Tokenizer::from_json(file.to_string().as_bytes()).unwrap()
    }

    #[test]
    fn a_word_is_split_into_the_longest_pieces_it_has() {
        assert_eq!(tokenizer().encode("unaffable"), vec![2, 3, 4]);
    }

    #[test]
    fn a_word_with_a_gap_is_unknown_as_a_whole() {
        // `un` and `##aff` exist, but nothing covers the `x`.
        assert_eq!(tokenizer().encode("unaffx"), vec![0]);
        assert_eq!(tokenizer().encode("refund unaffx refund"), vec![5, 0, 5]);
    }

    #[test]
    fn case_accents_and_punctuation_are_normalized_away() {
        assert_eq!(tokenizer().encode("Café!REFUND"), vec![7, 6, 5]);
    }

    #[test]
    fn a_special_token_in_the_text_is_itself() {
        assert_eq!(tokenizer().encode("refund[SEP]cafe"), vec![5, 1, 7]);
        // Matched as written, before lowercasing.
        assert_ne!(tokenizer().encode("[sep]"), vec![1]);
    }

    #[test]
    fn a_long_word_is_unknown_whatever_it_holds() {
        assert_eq!(
            tokenizer().encode("abbbbbbbbb"),
            vec![8, 9, 9, 9, 9, 9, 9, 9, 9, 9]
        );
        assert_eq!(tokenizer().encode("abbbbbbbbbb"), vec![0]);
    }

    #[test]
    fn another_kind_of_tokenizer_is_refused() {
        let file = serde_json::json!({ "model": { "type": "BPE", "vocab": {} } });
        let error = Tokenizer::from_json(file.to_string().as_bytes()).unwrap_err();
        assert!(error.to_string().contains("BPE"), "{error}");
    }
}
