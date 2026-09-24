//! Finding a snippet in a library.
//!
//! Two kinds of matching, because a snippet holds two kinds of text. The short
//! fields — abbreviation, label, tag, group — are matched fuzzily, the way an
//! interactive picker does, so `bregs` finds "Best regards". Bodies are matched
//! literally, because fuzzy matching over a paragraph finds the query's letters
//! scattered across it and calls that a hit. The literal body search is also
//! what [ADR-0013] leans on: a macro no importer could convert stays in the
//! body as the text it was, and searching for it is how every one of them is
//! found again.
//!
//! Ranking is by field first and score second, so every abbreviation hit comes
//! before every label hit and body hits come last. That order can be explained
//! in one sentence, which a blend of two scores that share no scale could not.
//!
//! Search by meaning (PRD A4) keeps to that sentence. The core works out which
//! snippets mean what the query means ([`meaning_text`] is what it embeds) and
//! hands them in, most similar first; each one the words did not already find
//! is a [`Field::Meaning`] hit, after every literal one. Typing a word that is
//! in a snippet finds it where it always did, and "money back" still finds the
//! refund reply that never says either word.
//!
//! [ADR-0013]: ../../../docs/adr/0013-import-and-export.md

use std::collections::HashSet;
use std::fmt;

use aralo_snippet::{SnippetId, SnippetKind};
use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use crate::{Library, LoadedSnippet};

/// The longest body line a hit carries back. A snippet body may be a megabyte
/// on one line, and a search result is something a list has to draw.
const MAX_EXCERPT: usize = 120;

/// How much of a long line to keep before the match, so the excerpt reads as
/// part of a sentence rather than starting at the query.
const EXCERPT_LEAD: usize = 24;

/// A body hit counts down from here by how far into the body it sits: the same
/// words in the first line of a snippet rank above them in the twentieth.
const BODY_SCORE: u32 = 100_000;

/// At most this many hits by meaning. They come after every literal hit, and
/// past the first few a model's idea of "similar" is not worth a row.
const MEANING_HITS: usize = 5;

/// What to look for.
///
/// The default query matches every snippet, which is the list view the editor
/// opens with.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    /// What the user typed. Empty matches everything.
    pub text: String,
    /// Only snippets in this group or in a group inside it. Folder names are
    /// compared without case, because the file systems Aralo runs on do.
    pub group: Option<Vec<String>>,
    /// Only snippets carrying this tag, compared without case.
    pub tag: Option<String>,
    /// Leave out snippets that are switched off.
    pub enabled_only: bool,
    /// Only snippets of this type, such as the `text` ones a palette can
    /// insert.
    pub kind: Option<SnippetKind>,
    /// At most this many hits, after ranking.
    pub limit: Option<usize>,
}

/// Where a query matched. The order of these variants is the ranking order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Field {
    Abbreviation,
    Label,
    Tag,
    Group,
    Body,
    /// None of its words, but what it means: the embedding model found it.
    Meaning,
}

/// One snippet a query found, and the text that found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub id: SnippetId,
    pub field: Field,
    /// The text that matched: one abbreviation, the label, one tag, the group
    /// path, or the body line the match sits on, cut to a readable length. A
    /// hit by meaning carries the body's first line, since no word matched.
    pub text: String,
    /// Character positions in `text` the query matched, in order, for an
    /// editor to highlight. Empty when the query was empty.
    pub matched: Vec<u32>,
    /// How well it matched. Comparable only with hits in the same field; for
    /// a hit by meaning, the similarity in ten-thousandths.
    pub score: u32,
}

/// Searches a library. It holds the matcher's scratch buffers, so keep one and
/// search with it repeatedly rather than building one per keystroke.
pub struct Searcher {
    matcher: Matcher,
    haystack: Vec<char>,
    indices: Vec<u32>,
}

impl Default for Searcher {
    fn default() -> Self {
        Self::new()
    }
}

// `Matcher` has no `Debug`, and the buffers are scratch space nobody wants
// printed.
impl fmt::Debug for Searcher {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Searcher").finish_non_exhaustive()
    }
}

impl Searcher {
    pub fn new() -> Self {
        Self {
            matcher: Matcher::new(Config::DEFAULT),
            haystack: Vec::new(),
            indices: Vec::new(),
        }
    }

    /// Every snippet the query matches, best first. At most one hit per
    /// snippet: a snippet whose abbreviation and body both match is reported
    /// against its abbreviation, the field that ranks highest.
    pub fn search(&mut self, library: &Library, query: &Query) -> Vec<Hit> {
        self.search_with_meaning(library, query, &[])
    }

    /// The same, with the snippets that mean what the query means, most
    /// similar first, each with its similarity from 0 to 1. Those the words
    /// found keep their literal hit; the rest come after every literal hit, in
    /// the order given, through the same filters.
    pub fn search_with_meaning(
        &mut self,
        library: &Library,
        query: &Query,
        similar: &[(SnippetId, f32)],
    ) -> Vec<Hit> {
        let text = query.text.trim();
        // `Pattern::new` and not `Pattern::parse`: parse reads `^`, `$`, `'`
        // and `!` as operators, and an abbreviation is exactly the kind of
        // string that begins with punctuation.
        let pattern = (!text.is_empty()).then(|| {
            Pattern::new(
                text,
                CaseMatching::Smart,
                Normalization::Smart,
                AtomKind::Fuzzy,
            )
        });

        let mut hits = Vec::new();
        for snippet in library.snippets() {
            if !query.allows(snippet) {
                continue;
            }
            match &pattern {
                Some(pattern) => {
                    if let Some(hit) = self.hit(snippet, pattern, text) {
                        hits.push(hit);
                    }
                }
                // An empty query is the list view: every snippet, nothing
                // highlighted, under the name the editor shows it by.
                None => hits.push(Hit {
                    id: snippet.id,
                    field: Field::Label,
                    text: snippet.display_name().to_owned(),
                    matched: Vec::new(),
                    score: 0,
                }),
            }
        }

        // A stable sort, so hits that rank the same keep the library's own
        // order, which is by path.
        hits.sort_by(|a, b| a.field.cmp(&b.field).then(b.score.cmp(&a.score)));

        if !text.is_empty() {
            let found: HashSet<SnippetId> = hits.iter().map(|hit| hit.id).collect();
            let mut seen = HashSet::new();
            for &(id, similarity) in similar {
                if seen.len() == MEANING_HITS {
                    break;
                }
                if found.contains(&id) || seen.contains(&id) {
                    continue;
                }
                let Some(snippet) = library.snippet(id) else {
                    continue;
                };
                if !query.allows(snippet) {
                    continue;
                }
                seen.insert(id);
                hits.push(Hit {
                    id,
                    field: Field::Meaning,
                    text: first_line(&snippet.file.body),
                    matched: Vec::new(),
                    score: (similarity.clamp(0.0, 1.0) * 10_000.0).round() as u32,
                });
            }
        }
        if let Some(limit) = query.limit {
            hits.truncate(limit);
        }
        hits
    }

    /// The best hit for one snippet, or `None` if the query misses it.
    fn hit(&mut self, snippet: &LoadedSnippet, pattern: &Pattern, text: &str) -> Option<Hit> {
        let front = &snippet.file.front;

        // Field order is ranking order, so the first field that matches wins
        // and the ones below it are never scored.
        if let Some((matched, score)) = self.best(pattern, front.abbr.iter().map(String::as_str)) {
            // The abbreviation you typed out in full is the one you meant.
            let score = if equal_ignore_case(&matched, text) {
                u32::MAX
            } else {
                score
            };
            return Some(self.hit_on(snippet.id, Field::Abbreviation, matched, score, pattern));
        }
        if let Some((matched, score)) = self.best(pattern, [snippet.display_name()]) {
            return Some(self.hit_on(snippet.id, Field::Label, matched, score, pattern));
        }
        if let Some((matched, score)) = self.best(pattern, front.tags.iter().map(String::as_str)) {
            return Some(self.hit_on(snippet.id, Field::Tag, matched, score, pattern));
        }
        let group = snippet.group.join("/");
        if let Some((matched, score)) = self.best(pattern, [group.as_str()]) {
            return Some(self.hit_on(snippet.id, Field::Group, matched, score, pattern));
        }

        let body = &snippet.file.body;
        let (start, end) = find_ignore_case(body, text)?;
        let (excerpt, offset, length) = excerpt(body, start, end);
        Some(Hit {
            id: snippet.id,
            field: Field::Body,
            text: excerpt,
            matched: (offset..offset + length).collect(),
            // `start` is a byte offset, which is close enough to "how far in"
            // for a tie-break between two body hits.
            score: BODY_SCORE.saturating_sub(start.try_into().unwrap_or(u32::MAX)),
        })
    }

    /// The best-scoring of several candidate strings, with its score.
    fn best<'a>(
        &mut self,
        pattern: &Pattern,
        candidates: impl IntoIterator<Item = &'a str>,
    ) -> Option<(String, u32)> {
        let mut best: Option<(String, u32)> = None;
        for candidate in candidates {
            if candidate.is_empty() {
                continue;
            }
            let haystack = Utf32Str::new(candidate, &mut self.haystack);
            if let Some(score) = pattern.score(haystack, &mut self.matcher) {
                if best.as_ref().is_none_or(|(_, best)| score > *best) {
                    best = Some((candidate.to_owned(), score));
                }
            }
        }
        best
    }

    /// A hit on a fuzzy field, with the characters the pattern used.
    fn hit_on(
        &mut self,
        id: SnippetId,
        field: Field,
        text: String,
        score: u32,
        pattern: &Pattern,
    ) -> Hit {
        self.indices.clear();
        let haystack = Utf32Str::new(&text, &mut self.haystack);
        pattern.indices(haystack, &mut self.matcher, &mut self.indices);
        // Several atoms can claim the same character, and they arrive in the
        // order the atoms ran rather than the order they are drawn in.
        self.indices.sort_unstable();
        self.indices.dedup();
        Hit {
            id,
            field,
            text,
            matched: self.indices.clone(),
            score,
        }
    }
}

impl Query {
    /// A query for `text` and nothing else.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Self::default()
        }
    }

    /// Whether the filters let this snippet through, before any matching.
    fn allows(&self, snippet: &LoadedSnippet) -> bool {
        if self.enabled_only && !snippet.settings.enabled {
            return false;
        }
        if self
            .kind
            .is_some_and(|kind| kind != snippet.file.front.kind)
        {
            return false;
        }
        if let Some(group) = &self.group {
            if group.len() > snippet.group.len()
                || !group
                    .iter()
                    .zip(&snippet.group)
                    .all(|(wanted, part)| equal_ignore_case(wanted, part))
            {
                return false;
            }
        }
        if let Some(tag) = &self.tag {
            if !snippet
                .file
                .front
                .tags
                .iter()
                .any(|carried| equal_ignore_case(carried, tag))
            {
                return false;
            }
        }
        true
    }
}

/// What a snippet means, as the embedding model is given it: the label, the
/// tags and the body, without the placeholders. `{{clipboard}}` says where
/// text goes, not what the snippet is about, and a date format is noise.
///
/// Only what the content hash covers goes in, so a snippet's vector is still
/// its own after a rename or a move, exactly as its hash is.
pub fn meaning_text(snippet: &LoadedSnippet) -> String {
    let front = &snippet.file.front;
    let mut text = String::new();
    for part in std::iter::once(front.label.as_str()).chain(front.tags.iter().map(String::as_str)) {
        if !part.is_empty() {
            text.push_str(part);
            text.push_str(".\n");
        }
    }
    let mut body = snippet.file.body.as_str();
    while let Some(open) = body.find("{{") {
        text.push_str(&body[..open]);
        match body[open..].find("}}") {
            Some(close) => {
                text.push(' ');
                body = &body[open + close + 2..];
            }
            None => {
                body = "";
            }
        }
    }
    text.push_str(body);
    text
}

/// The first line of a body with anything on it, cut to what a list can draw.
fn first_line(body: &str) -> String {
    let line = body
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    line.chars().take(MAX_EXCERPT).collect()
}

fn equal_ignore_case(left: &str, right: &str) -> bool {
    left == right || left.to_lowercase() == right.to_lowercase()
}

/// Whether two characters are the same once case is set aside. Folded one
/// character at a time: a search box wants the obvious answer, not the full
/// Unicode case-folding algorithm.
fn same_ignore_case(left: char, right: char) -> bool {
    left == right || left.to_lowercase().eq(right.to_lowercase())
}

/// The first place `needle` appears in `haystack`, ignoring case, as byte
/// offsets.
fn find_ignore_case(haystack: &str, needle: &str) -> Option<(usize, usize)> {
    let first = needle.chars().next()?;
    for (start, character) in haystack.char_indices() {
        if !same_ignore_case(character, first) {
            continue;
        }
        let mut left = haystack[start..].chars();
        let mut right = needle.chars();
        let mut end = start;
        loop {
            match (left.next(), right.next()) {
                (_, None) => return Some((start, end)),
                (None, Some(_)) => break,
                (Some(left), Some(right)) => {
                    if !same_ignore_case(left, right) {
                        break;
                    }
                    end += left.len_utf8();
                }
            }
        }
    }
    None
}

/// The body line a match sits on, cut to something a list can draw, with the
/// match's position and length inside it in characters.
fn excerpt(body: &str, start: usize, end: usize) -> (String, u32, u32) {
    let line_start = body[..start].rfind('\n').map_or(0, |index| index + 1);
    let line_end = body[end..]
        .find('\n')
        .map_or(body.len(), |index| end + index);
    let line = body[line_start..line_end].trim_end_matches('\r');
    let before = body[line_start..start].chars().count();
    let length = body[start..end].chars().count();

    if line.chars().count() <= MAX_EXCERPT {
        return (line.to_owned(), before as u32, length as u32);
    }
    // A window around the match, so a long line still shows why it matched.
    let from = before.saturating_sub(EXCERPT_LEAD);
    let text: String = line.chars().skip(from).take(MAX_EXCERPT).collect();
    let offset = before - from;
    let length = length.min(text.chars().count() - offset);
    (text, offset as u32, length as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_needle_is_found_whatever_the_case() {
        assert_eq!(find_ignore_case("Best Regards", "regards"), Some((5, 12)));
        assert_eq!(find_ignore_case("Grüße", "GRÜSSE"), None);
        assert_eq!(find_ignore_case("Grüße", "grüß"), Some((0, 6)));
        assert_eq!(find_ignore_case("abc", "d"), None);
        assert_eq!(find_ignore_case("abc", ""), None);
    }

    #[test]
    fn a_needle_is_found_after_a_false_start() {
        assert_eq!(find_ignore_case("aab", "ab"), Some((1, 3)));
    }

    #[test]
    fn an_excerpt_is_the_line_the_match_is_on() {
        let body = "first line\nthe %key:tab% macro\nthird line";
        let (start, end) = find_ignore_case(body, "%key:tab%").unwrap();
        assert_eq!(
            excerpt(body, start, end),
            ("the %key:tab% macro".to_owned(), 4, 9)
        );
    }

    #[test]
    fn a_long_line_is_cut_around_the_match() {
        let body = format!("{}NEEDLE{}", "x".repeat(400), "y".repeat(400));
        let (start, end) = find_ignore_case(&body, "needle").unwrap();
        let (text, offset, length) = excerpt(&body, start, end);
        assert_eq!(text.chars().count(), MAX_EXCERPT);
        assert_eq!(offset, EXCERPT_LEAD as u32);
        assert_eq!(length, 6);
        let matched: String = text
            .chars()
            .skip(offset as usize)
            .take(length as usize)
            .collect();
        assert_eq!(matched, "NEEDLE");
    }

    #[test]
    fn a_match_at_the_end_of_a_long_line_stays_inside_the_excerpt() {
        let body = format!("{}NEEDLE", "x".repeat(400));
        let (start, end) = find_ignore_case(&body, "needle").unwrap();
        let (text, offset, length) = excerpt(&body, start, end);
        assert!(offset + length <= text.chars().count() as u32);
    }
}
