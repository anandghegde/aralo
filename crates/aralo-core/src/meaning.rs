//! Search by meaning (PRD A4): the embedding model, a vector for every
//! snippet, and the snippets nearest a query handed to search.
//!
//! Nothing here reaches the network. The model is files on this machine, the
//! vectors are arithmetic on them, and `aralo-embed` has nothing in its
//! dependencies that could open a socket (`scripts/check-deps.sh`).
//!
//! A [`Runtime`](crate::Runtime) embeds on its indexer thread and keeps the
//! vectors in the index, keyed by content hash and model, so a restart embeds
//! only what changed. A [`Core`](crate::Core) on its own, as the CLI opens
//! one, embeds its library in memory when it is given a model: a static model
//! does a library of thousands in well under a second.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

use aralo_embed::{Model, Vectors};
use aralo_library::{content_hash, meaning_text, Library};
use aralo_snippet::SnippetId;

/// A snippet this much like the query or more is offered by meaning. Cosine
/// similarity: 1 is the same meaning, 0 unrelated. A static model averages
/// every token of a snippet into one vector, so a two-word query and the
/// paragraph that answers it rarely score high; the ranking matters more
/// than the number, and this only keeps the unrelated out.
pub const MIN_SIMILARITY: f32 = 0.25;

/// How many candidates search is handed before its filters run. It shows
/// fewer ([`aralo_library::Searcher::search_with_meaning`]); these are enough
/// that a filter on group or type still leaves some.
const CANDIDATES: usize = 50;

/// A query needs this many letters or digits before its meaning is looked
/// for. One or two are the start of an abbreviation, not a thought.
const MIN_QUERY_CHARS: usize = 3;

/// The model, and the vector of every snippet in the library under it.
pub struct Meaning {
    model: Arc<Model>,
    vectors: Vectors<SnippetId>,
}

impl fmt::Debug for Meaning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Meaning")
            .field("model", &self.model.id())
            .field("snippets", &self.vectors.len())
            .finish()
    }
}

impl Meaning {
    pub fn model(&self) -> &Arc<Model> {
        &self.model
    }

    /// How many snippets have a vector.
    pub fn len(&self) -> usize {
        self.vectors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.vectors.is_empty()
    }

    /// The snippets that mean what `text` means, most similar first, with
    /// their similarity. None for a query too short to mean anything.
    pub fn similar(&self, text: &str) -> Vec<(SnippetId, f32)> {
        if text.chars().filter(|c| c.is_alphanumeric()).count() < MIN_QUERY_CHARS {
            return Vec::new();
        }
        let query = self.model.embed(text);
        self.vectors
            .nearest(&query, CANDIDATES, MIN_SIMILARITY)
            .into_iter()
            .map(|(id, similarity)| (*id, similarity))
            .collect()
    }
}

/// What embedding a library takes, worked out under the library's lock so the
/// embedding itself can happen without it.
#[derive(Debug, Default)]
pub(crate) struct Plan {
    /// Every snippet and the hash of its content.
    snippets: Vec<(SnippetId, String)>,
    /// Content that has no vector yet, once per hash, with its text.
    missing: Vec<(String, String)>,
}

impl Plan {
    /// `known` says whether a vector for a content hash is already kept.
    pub(crate) fn of(library: &Library, known: impl Fn(&str) -> bool) -> Self {
        let mut plan = Self::default();
        let mut queued = HashSet::new();
        for snippet in library.snippets() {
            let hash = content_hash(snippet);
            if !known(&hash) && queued.insert(hash.clone()) {
                plan.missing.push((hash.clone(), meaning_text(snippet)));
            }
            plan.snippets.push((snippet.id, hash));
        }
        plan
    }

    /// The hashes the library holds now, for pruning what it no longer does.
    pub(crate) fn hashes(&self) -> HashSet<String> {
        self.snippets.iter().map(|(_, hash)| hash.clone()).collect()
    }

    /// Every snippet already has a vector, or has had its chance at one.
    pub(crate) fn is_settled(&self) -> bool {
        self.missing.is_empty()
    }

    pub(crate) fn snippets(&self) -> &Vec<(SnippetId, String)> {
        &self.snippets
    }

    pub(crate) fn into_snippets(self) -> Vec<(SnippetId, String)> {
        self.snippets
    }

    /// Embeds what is missing. The expensive part, and the part that needs no
    /// lock.
    pub(crate) fn embed(&self, model: &Model) -> Vec<(String, Vec<f32>)> {
        self.missing
            .iter()
            .map(|(hash, text)| (hash.clone(), model.embed(text)))
            .collect()
    }

    /// The meaning of the library, from the vectors kept and those just made.
    pub(crate) fn assemble(
        &self,
        model: Arc<Model>,
        vectors: &HashMap<String, Vec<f32>>,
    ) -> Meaning {
        let mut set = Vectors::new(model.dimensions());
        for (id, hash) in &self.snippets {
            if let Some(vector) = vectors.get(hash) {
                // A snippet with no word the model knows gets no vector, and
                // is found by its words alone.
                set.push(*id, vector);
            }
        }
        Meaning {
            model,
            vectors: set,
        }
    }
}

/// Embeds every snippet of `library` in memory.
pub(crate) fn embed_library(model: Arc<Model>, library: &Library) -> Meaning {
    let plan = Plan::of(library, |_| false);
    let made: HashMap<String, Vec<f32>> = plan.embed(&model).into_iter().collect();
    plan.assemble(model, &made)
}
