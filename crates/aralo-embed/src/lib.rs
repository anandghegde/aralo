//! Embeddings: what a text means, as a vector, with nothing leaving the
//! machine (PRD A4).
//!
//! A [`Model`] is loaded from a folder the app ships or the user points at,
//! and turns text into a vector; [`Vectors`] holds the vectors of a library
//! and finds the ones nearest a query's. That is all of semantic search that
//! is not bookkeeping: which snippets need a vector, where vectors are kept
//! and how their hits sit beside literal ones belong to the library and the
//! core.
//!
//! Nothing here opens a socket, and the dependency check keeps it that way
//! (`scripts/check-deps.sh`): this crate's dependencies are a short list with
//! no networking in it, so no model, however it was built, can call out.

mod model;
mod safetensors;
mod tokenizer;
mod vectors;

use std::path::PathBuf;

pub use model::{Model, CONFIG_FILE, TENSORS_FILE, TOKENIZER_FILE};
pub use vectors::{from_bytes, to_bytes, Vectors};

#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    #[error("cannot read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("the model's tokenizer cannot be used: {0}")]
    Tokenizer(String),
    #[error("the model's weights cannot be used: {0}")]
    Tensors(String),
    #[error("the model's config.json cannot be read: {0}")]
    Config(String),
}
