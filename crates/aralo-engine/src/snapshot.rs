//! The immutable matcher snapshot: a trie of reversed, case-folded abbreviations.
//!
//! The library builds a new [`Snapshot`] off the keystroke path whenever
//! snippets change. Lookup cost is bounded by the longest abbreviation, not by
//! the number of snippets.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::ops::Range;

use crate::buffer::{RingBuffer, CAPACITY};
use crate::case::{fold, is_word_char};

/// The longest abbreviation the engine accepts, in characters.
///
/// One less than the buffer so the character before a match is always
/// available to the whole-word rule.
pub const MAX_ABBREVIATION_CHARS: usize = CAPACITY - 1;

/// Delimiters used when a group does not define its own set.
pub const DEFAULT_DELIMITERS: &[char] = &[
    ' ', '\t', '\n', '\r', '.', ',', ';', ':', '!', '?', '(', ')', '[', ']', '{', '}', '<', '>',
    '/', '\\', '\'', '"', '-',
];

/// A snippet's ULID as a number. The engine never looks inside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SnippetId(pub u128);

/// When an abbreviation expands (PRD E1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Trigger {
    /// As soon as its last character is typed.
    Immediate,
    /// When a delimiter is typed after it.
    Delimiter,
}

/// How the typed case must relate to the defined abbreviation (PRD E3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CaseMode {
    /// Must be typed exactly as defined.
    Exact,
    /// Any case matches; the expansion is not changed.
    Ignore,
    /// Any case matches; the typed pattern is reported as a [`crate::CasePattern`].
    Adaptive,
}

/// Which apps an abbreviation is active in (PRD E8). App IDs are bundle IDs on
/// macOS and are compared without regard to ASCII case.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum Scope {
    #[default]
    Everywhere,
    Only(Vec<String>),
    Except(Vec<String>),
}

impl Scope {
    pub fn matches(&self, front_app: &str) -> bool {
        let listed = |apps: &[String]| apps.iter().any(|app| app.eq_ignore_ascii_case(front_app));
        match self {
            Scope::Everywhere => true,
            Scope::Only(apps) => listed(apps),
            Scope::Except(apps) => !listed(apps),
        }
    }

    /// Higher is more specific. Breaks ties between abbreviations of equal length.
    fn specificity(&self) -> u8 {
        match self {
            Scope::Everywhere => 0,
            Scope::Except(_) => 1,
            Scope::Only(_) => 2,
        }
    }

    fn normalised(self) -> Self {
        let tidy = |mut apps: Vec<String>| {
            for app in &mut apps {
                app.make_ascii_lowercase();
            }
            apps.sort();
            apps.dedup();
            apps
        };
        match self {
            Scope::Everywhere => Scope::Everywhere,
            Scope::Only(apps) => Scope::Only(tidy(apps)),
            Scope::Except(apps) => Scope::Except(tidy(apps)),
        }
    }
}

/// One abbreviation of one snippet, with its group's settings already resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Abbreviation {
    pub snippet_id: SnippetId,
    pub text: String,
    pub trigger: Trigger,
    pub case: CaseMode,
    /// The character before the abbreviation must not be a word character (PRD E2).
    pub whole_word: bool,
    /// Re-insert the delimiter after the expansion. Only read for [`Trigger::Delimiter`].
    pub keep_delimiter: bool,
    /// Only read for [`Trigger::Delimiter`].
    pub delimiters: Vec<char>,
    pub scope: Scope,
}

impl Abbreviation {
    /// A delimiter-triggered, adaptive-case, whole-word abbreviation that is active everywhere.
    pub fn new(snippet_id: SnippetId, text: impl Into<String>) -> Self {
        Self {
            snippet_id,
            text: text.into(),
            trigger: Trigger::Delimiter,
            case: CaseMode::Adaptive,
            whole_word: true,
            keep_delimiter: true,
            delimiters: DEFAULT_DELIMITERS.to_vec(),
            scope: Scope::Everywhere,
        }
    }
}

/// An abbreviation the builder refused. The rest of the snapshot is still built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejection {
    pub snippet_id: SnippetId,
    pub text: String,
    pub reason: RejectionReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectionReason {
    Empty,
    /// Longer than [`MAX_ABBREVIATION_CHARS`].
    TooLong {
        chars: usize,
    },
}

#[derive(Debug)]
pub(crate) struct Entry {
    pub(crate) snippet_id: SnippetId,
    /// The abbreviation as defined, in typing order.
    pub(crate) chars: Box<[char]>,
    pub(crate) trigger: Trigger,
    pub(crate) case: CaseMode,
    pub(crate) whole_word: bool,
    pub(crate) keep_delimiter: bool,
    delimiters: u32,
    scope: u32,
}

#[derive(Debug)]
struct Node {
    edges: Range<u32>,
    terminals: Range<u32>,
}

/// Which kind of match is being looked for on this key.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Pass {
    /// The key is the last character of the abbreviation and is already in the buffer.
    Immediate,
    /// The key is this delimiter and is not yet in the buffer.
    Delimiter(char),
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Hit {
    pub(crate) entry: u32,
    /// Length of the matched abbreviation in characters.
    pub(crate) depth: usize,
    pub(crate) specificity: u8,
}

/// An immutable set of abbreviations, ready for matching.
#[derive(Debug)]
pub struct Snapshot {
    nodes: Vec<Node>,
    /// Outgoing edges of every node, sorted by character within a node.
    edges: Vec<(char, u32)>,
    /// Entry indices of every node, best tie-break first is not assumed.
    terminals: Vec<u32>,
    entries: Vec<Entry>,
    delimiter_sets: Vec<Box<[char]>>,
    scopes: Vec<Scope>,
    /// Sorted union of every delimiter set: a cheap test before a delimiter pass.
    any_delimiter: Box<[char]>,
    longest: usize,
}

impl Snapshot {
    pub fn empty() -> Self {
        SnapshotBuilder::new().build().0
    }

    /// Number of abbreviations (not snippets).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Length of the longest abbreviation, in characters.
    pub fn longest_abbreviation(&self) -> usize {
        self.longest
    }

    pub(crate) fn entry(&self, index: u32) -> &Entry {
        &self.entries[index as usize]
    }

    pub(crate) fn is_delimiter(&self, c: char) -> bool {
        self.any_delimiter.binary_search(&c).is_ok()
    }

    fn child(&self, node: u32, c: char) -> Option<u32> {
        let range = &self.nodes[node as usize].edges;
        let edges = &self.edges[range.start as usize..range.end as usize];
        edges
            .binary_search_by_key(&c, |&(edge, _)| edge)
            .ok()
            .map(|found| edges[found].1)
    }

    /// Finds the best abbreviation that ends at the end of `buffer`.
    ///
    /// Longest wins, then the most specific scope, then the lowest snippet ID
    /// so the result never depends on build order.
    pub(crate) fn find(&self, buffer: &RingBuffer, pass: Pass, front_app: &str) -> Option<Hit> {
        if let Pass::Delimiter(c) = pass {
            if !self.is_delimiter(c) {
                return None;
            }
        }
        let mut best: Option<Hit> = None;
        let mut node = 0;
        for (back, c) in buffer.iter_back().enumerate() {
            let Some(next) = self.child(node, fold(c)) else {
                break;
            };
            node = next;
            let depth = back + 1;
            let range = &self.nodes[node as usize].terminals;
            let mut here: Option<Hit> = None;
            for &index in &self.terminals[range.start as usize..range.end as usize] {
                let entry = self.entry(index);
                if !self.accepts(entry, pass, buffer, depth, front_app) {
                    continue;
                }
                let specificity = self.scopes[entry.scope as usize].specificity();
                let better = here.is_none_or(|current| {
                    let current_id = self.entry(current.entry).snippet_id;
                    (specificity, core::cmp::Reverse(entry.snippet_id))
                        > (current.specificity, core::cmp::Reverse(current_id))
                });
                if better {
                    here = Some(Hit {
                        entry: index,
                        depth,
                        specificity,
                    });
                }
            }
            // Deeper means longer, and longest wins.
            if here.is_some() {
                best = here;
            }
        }
        best
    }

    fn accepts(
        &self,
        entry: &Entry,
        pass: Pass,
        buffer: &RingBuffer,
        depth: usize,
        front_app: &str,
    ) -> bool {
        match (pass, entry.trigger) {
            (Pass::Immediate, Trigger::Immediate) => {}
            (Pass::Delimiter(c), Trigger::Delimiter) => {
                let set = &self.delimiter_sets[entry.delimiters as usize];
                if set.binary_search(&c).is_err() {
                    return false;
                }
            }
            _ => return false,
        }
        if entry.case == CaseMode::Exact {
            let typed = (0..depth).map(|i| buffer.get_back(depth - 1 - i));
            if !typed.eq(entry.chars.iter().map(|&c| Some(c))) {
                return false;
            }
        }
        if entry.whole_word && buffer.get_back(depth).is_some_and(is_word_char) {
            return false;
        }
        self.scopes[entry.scope as usize].matches(front_app)
    }
}

/// Collects abbreviations and freezes them into a [`Snapshot`].
#[derive(Debug, Default)]
pub struct SnapshotBuilder {
    items: Vec<Abbreviation>,
}

impl SnapshotBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, abbreviation: Abbreviation) -> &mut Self {
        self.items.push(abbreviation);
        self
    }

    /// Builds the snapshot. Unusable abbreviations are reported, not fatal.
    pub fn build(self) -> (Snapshot, Vec<Rejection>) {
        struct TempNode {
            children: BTreeMap<char, u32>,
            terminals: Vec<u32>,
        }
        let new_node = || TempNode {
            children: BTreeMap::new(),
            terminals: Vec::new(),
        };

        let mut rejections = Vec::new();
        let mut temp = alloc::vec![new_node()];
        let mut entries = Vec::new();
        let mut delimiter_sets: Vec<Box<[char]>> = Vec::new();
        let mut delimiter_index: BTreeMap<Vec<char>, u32> = BTreeMap::new();
        let mut scopes: Vec<Scope> = Vec::new();
        let mut scope_index: BTreeMap<Scope, u32> = BTreeMap::new();
        let mut longest = 0;

        for item in self.items {
            let chars: Vec<char> = item.text.chars().collect();
            let reason = match chars.len() {
                0 => Some(RejectionReason::Empty),
                n if n > MAX_ABBREVIATION_CHARS => Some(RejectionReason::TooLong { chars: n }),
                _ => None,
            };
            if let Some(reason) = reason {
                rejections.push(Rejection {
                    snippet_id: item.snippet_id,
                    text: item.text,
                    reason,
                });
                continue;
            }

            let mut delimiters = item.delimiters;
            delimiters.sort_unstable();
            delimiters.dedup();
            let delimiters = *delimiter_index.entry(delimiters).or_insert_with_key(|set| {
                delimiter_sets.push(set.clone().into_boxed_slice());
                (delimiter_sets.len() - 1) as u32
            });
            let scope = *scope_index
                .entry(item.scope.normalised())
                .or_insert_with_key(|scope| {
                    scopes.push(scope.clone());
                    (scopes.len() - 1) as u32
                });

            let mut node = 0u32;
            for &c in chars.iter().rev() {
                let next = temp.len() as u32;
                let child = *temp[node as usize].children.entry(fold(c)).or_insert(next);
                if child == next {
                    temp.push(new_node());
                }
                node = child;
            }
            temp[node as usize].terminals.push(entries.len() as u32);
            longest = longest.max(chars.len());
            entries.push(Entry {
                snippet_id: item.snippet_id,
                chars: chars.into_boxed_slice(),
                trigger: item.trigger,
                case: item.case,
                whole_word: item.whole_word,
                keep_delimiter: item.keep_delimiter,
                delimiters,
                scope,
            });
        }

        let mut nodes = Vec::with_capacity(temp.len());
        let mut edges = Vec::new();
        let mut terminals = Vec::new();
        for node in temp {
            let edge_start = edges.len() as u32;
            edges.extend(node.children); // BTreeMap iterates in key order
            let terminal_start = terminals.len() as u32;
            terminals.extend(node.terminals);
            nodes.push(Node {
                edges: edge_start..edges.len() as u32,
                terminals: terminal_start..terminals.len() as u32,
            });
        }

        let mut any_delimiter: Vec<char> = entries
            .iter()
            .filter(|entry| entry.trigger == Trigger::Delimiter)
            .flat_map(|entry| delimiter_sets[entry.delimiters as usize].iter().copied())
            .collect();
        any_delimiter.sort_unstable();
        any_delimiter.dedup();

        let snapshot = Snapshot {
            nodes,
            edges,
            terminals,
            entries,
            delimiter_sets,
            scopes,
            any_delimiter: any_delimiter.into_boxed_slice(),
            longest,
        };
        (snapshot, rejections)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn rejects_empty_and_overlong_abbreviations_but_keeps_the_rest() {
        let mut builder = SnapshotBuilder::new();
        builder.add(Abbreviation::new(SnippetId(1), ""));
        builder.add(Abbreviation::new(
            SnippetId(2),
            "x".repeat(MAX_ABBREVIATION_CHARS + 1),
        ));
        builder.add(Abbreviation::new(
            SnippetId(3),
            "y".repeat(MAX_ABBREVIATION_CHARS),
        ));
        let (snapshot, rejections) = builder.build();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot.longest_abbreviation(), MAX_ABBREVIATION_CHARS);
        assert_eq!(rejections.len(), 2);
        assert_eq!(rejections[0].reason, RejectionReason::Empty);
        assert_eq!(
            rejections[1].reason,
            RejectionReason::TooLong {
                chars: MAX_ABBREVIATION_CHARS + 1
            }
        );
    }

    #[test]
    fn scope_matching_ignores_ascii_case() {
        let only = Scope::Only(alloc::vec!["com.apple.Mail".to_string()]);
        assert!(only.matches("com.apple.mail"));
        assert!(!only.matches("com.apple.Notes"));
        let except = Scope::Except(alloc::vec!["com.apple.Terminal".to_string()]);
        assert!(!except.matches("com.apple.terminal"));
        assert!(except.matches("com.apple.mail"));
    }

    #[test]
    fn identical_delimiter_sets_and_scopes_are_stored_once() {
        let mut builder = SnapshotBuilder::new();
        for id in 0..100 {
            builder.add(Abbreviation::new(SnippetId(id), alloc::format!(";a{id}")));
        }
        let (snapshot, _) = builder.build();
        assert_eq!(snapshot.delimiter_sets.len(), 1);
        assert_eq!(snapshot.scopes.len(), 1);
    }
}
