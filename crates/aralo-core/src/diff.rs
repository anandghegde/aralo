//! Word-level differences, for the preview a command on selected text shows
//! before it replaces anything (plan 7.3, PRD A3).
//!
//! The spans are the core's so that every shell draws the same diff: the
//! removed spans joined up are the text before, and the added spans joined up
//! are the text after, with the unchanged spans in both.

use similar::{ChangeTag, TextDiff};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Same,
    Removed,
    Added,
}

/// A run of text that is in both versions, or only in one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffSpan {
    pub change: Change,
    pub text: String,
}

/// The difference between `before` and `after`, word by word.
///
/// Within a changed stretch the removed text comes first and the added text
/// second, so a rewritten phrase reads as one strike-through and one
/// insertion rather than alternating words. A space between two changed
/// words belongs to the change, for the same reason.
pub fn diff_words(before: &str, after: &str) -> Vec<DiffSpan> {
    let diff = TextDiff::configure().diff_words(before, after);
    let mut pieces: Vec<(Change, &str)> = diff
        .iter_all_changes()
        .map(|change| {
            let kind = match change.tag() {
                ChangeTag::Equal => Change::Same,
                ChangeTag::Delete => Change::Removed,
                ChangeTag::Insert => Change::Added,
            };
            (kind, change.value())
        })
        .collect();

    // White space alone between two changes joins them.
    let mut spaced: Vec<(Change, &str)> = Vec::with_capacity(pieces.len());
    for index in 0..pieces.len() {
        let (kind, text) = pieces[index];
        let between_changes = kind == Change::Same
            && text.chars().all(char::is_whitespace)
            && index > 0
            && index + 1 < pieces.len()
            && pieces[index - 1].0 != Change::Same
            && pieces[index + 1].0 != Change::Same;
        if between_changes {
            spaced.push((Change::Removed, text));
            spaced.push((Change::Added, text));
        } else {
            spaced.push((kind, text));
        }
    }
    pieces = spaced;

    let mut spans = Vec::new();
    let mut removed = String::new();
    let mut added = String::new();
    let mut same = String::new();
    for (kind, text) in pieces {
        match kind {
            Change::Same => {
                flush_change(&mut spans, &mut removed, &mut added);
                same.push_str(text);
            }
            Change::Removed | Change::Added => {
                flush(&mut spans, Change::Same, &mut same);
                if kind == Change::Removed {
                    removed.push_str(text);
                } else {
                    added.push_str(text);
                }
            }
        }
    }
    flush_change(&mut spans, &mut removed, &mut added);
    flush(&mut spans, Change::Same, &mut same);
    spans
}

fn flush_change(spans: &mut Vec<DiffSpan>, removed: &mut String, added: &mut String) {
    flush(spans, Change::Removed, removed);
    flush(spans, Change::Added, added);
}

fn flush(spans: &mut Vec<DiffSpan>, change: Change, text: &mut String) {
    if !text.is_empty() {
        spans.push(DiffSpan {
            change,
            text: std::mem::take(text),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn side(spans: &[DiffSpan], leave_out: Change) -> String {
        spans
            .iter()
            .filter(|span| span.change != leave_out)
            .map(|span| span.text.as_str())
            .collect()
    }

    fn render(spans: &[DiffSpan]) -> String {
        spans
            .iter()
            .map(|span| match span.change {
                Change::Same => span.text.clone(),
                Change::Removed => format!("[-{}]", span.text),
                Change::Added => format!("[+{}]", span.text),
            })
            .collect()
    }

    #[test]
    fn both_sides_come_back_exactly() {
        let cases = [
            ("", ""),
            ("", "new"),
            ("old", ""),
            ("their going home", "they're going home"),
            ("Pay now.\r\nThanks", "Please pay when you can.\r\nThanks"),
            ("  leading and trailing  ", "leading and trailing"),
            ("naïve café 日本語", "naive cafe 日本語です"),
        ];
        for (before, after) in cases {
            let spans = diff_words(before, after);
            assert_eq!(side(&spans, Change::Added), before, "{before:?}");
            assert_eq!(side(&spans, Change::Removed), after, "{after:?}");
        }
    }

    #[test]
    fn a_changed_word_is_one_removal_and_one_addition() {
        let spans = diff_words("their going home", "they're going home");
        assert_eq!(render(&spans), "[-their][+they're] going home");
    }

    #[test]
    fn a_rewritten_phrase_is_not_interleaved() {
        let spans = diff_words("Pay the invoice now.", "Please settle it today.");
        assert_eq!(
            render(&spans),
            "[-Pay the invoice now.][+Please settle it today.]"
        );
    }

    #[test]
    fn identical_text_is_one_unchanged_span() {
        assert_eq!(
            diff_words("same text", "same text"),
            [DiffSpan {
                change: Change::Same,
                text: "same text".into()
            }]
        );
    }
}
