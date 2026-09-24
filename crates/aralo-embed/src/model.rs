//! A static embedding model: one vector per token, averaged.
//!
//! This is the Model2Vec way of embedding (ADR-0016). A transformer reads a
//! sentence and works out what each word means there; a static model has
//! already worked out, once, what each token means on average, so embedding a
//! text is a tokenizer pass and a sum. That is thousands of times cheaper,
//! which is what lets a whole library be embedded while the app starts, and a
//! query be embedded on every keystroke. What it gives up is context: "bank"
//! is one vector whatever the river says.
//!
//! [`Model::embed`] follows the reference implementation's `encode` to the
//! letter, so a model distilled and checked in Python means the same here:
//! truncate, tokenize with no special tokens around the text, drop the unknown
//! token, average the rows (through the vocabulary mapping and the token
//! weights when the model has them), and scale to unit length when the model
//! says to.

use std::fmt;
use std::path::Path;

use serde_json::Value;

use crate::tokenizer::Tokenizer;
use crate::{safetensors, ModelError};

/// Texts longer than this many tokens are cut, as the reference cuts them.
const MAX_TOKENS: usize = 512;

/// The three files of a model, in the folder [`Model::open`] reads.
pub const TOKENIZER_FILE: &str = "tokenizer.json";
pub const TENSORS_FILE: &str = "model.safetensors";
pub const CONFIG_FILE: &str = "config.json";

/// An embedding model, loaded and ready. It holds no file open and reaches for
/// nothing outside itself: embedding is arithmetic on what was loaded.
pub struct Model {
    id: String,
    tokenizer: Tokenizer,
    dimensions: usize,
    /// Row-major, `rows × dimensions`.
    embeddings: Vec<f32>,
    /// A weight per token id, for a model that stores them apart.
    weights: Option<Vec<f32>>,
    /// The row of each token id, for a model whose vocabulary was clustered.
    mapping: Option<Vec<u32>>,
    normalize: bool,
    /// Texts are cut to this many characters before tokenizing: `MAX_TOKENS`
    /// typical tokens, which is plenty and bounds the work.
    max_chars: usize,
}

impl fmt::Debug for Model {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Model")
            .field("id", &self.id)
            .field("dimensions", &self.dimensions)
            .field("rows", &(self.embeddings.len() / self.dimensions.max(1)))
            .finish_non_exhaustive()
    }
}

impl Model {
    /// Loads the model in `folder`: `tokenizer.json`, `model.safetensors` and,
    /// when there is one, `config.json`. The folder's name is the model's.
    pub fn open(folder: &Path) -> Result<Self, ModelError> {
        let read = |name: &str| {
            let path = folder.join(name);
            std::fs::read(&path).map_err(|source| ModelError::Read { path, source })
        };
        let tokenizer = read(TOKENIZER_FILE)?;
        let tensors = read(TENSORS_FILE)?;
        let config = match read(CONFIG_FILE) {
            Ok(config) => Some(config),
            Err(ModelError::Read { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                None
            }
            Err(error) => return Err(error),
        };
        let name = folder
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "model".to_owned());
        Self::from_bytes(&name, &tokenizer, &tensors, config.as_deref())
    }

    /// The same, from the three files' contents.
    pub fn from_bytes(
        name: &str,
        tokenizer: &[u8],
        tensors: &[u8],
        config: Option<&[u8]>,
    ) -> Result<Self, ModelError> {
        let normalize = match config {
            Some(config) => {
                let config: Value = serde_json::from_slice(config)
                    .map_err(|error| ModelError::Config(error.to_string()))?;
                // The reference's default when the file does not say.
                config["normalize"].as_bool().unwrap_or(false)
            }
            None => false,
        };

        let tokenizer_bytes = tokenizer;
        let tokenizer = Tokenizer::from_json(tokenizer)?;
        let read = safetensors::read(tensors)?;
        let matrix = ["embeddings", "embedding.weight", "0"]
            .iter()
            .find_map(|name| read.get(*name))
            .ok_or_else(|| ModelError::Tensors("there is no embedding matrix".to_owned()))?;
        let [rows, dimensions] = matrix.shape[..] else {
            return Err(ModelError::Tensors(format!(
                "the embedding matrix is {:?}, not rows by columns",
                matrix.shape
            )));
        };
        if rows == 0 || dimensions == 0 {
            return Err(ModelError::Tensors(
                "the embedding matrix is empty".to_owned(),
            ));
        }
        let embeddings = matrix.floats()?;
        let weights = read
            .get("weights")
            .map(|tensor| tensor.floats())
            .transpose()?;
        let mapping = read
            .get("mapping")
            .map(|tensor| tensor.indices())
            .transpose()?;

        let model = Self {
            id: format!("{name}@{}", fingerprint(tokenizer_bytes, tensors)),
            max_chars: MAX_TOKENS.saturating_mul(median_length(&tokenizer)),
            tokenizer,
            dimensions,
            embeddings,
            weights,
            mapping,
            normalize,
        };
        model.check(rows)?;
        Ok(model)
    }

    /// Every token the tokenizer can produce must have a row, so that
    /// embedding never has to decide what a missing one means.
    fn check(&self, rows: usize) -> Result<(), ModelError> {
        let largest = self.tokenizer.largest_id() as usize;
        let short = |what: &str, length: usize| {
            ModelError::Tensors(format!(
                "the {what} has {length} entries, and the vocabulary goes up to {largest}"
            ))
        };
        match &self.mapping {
            Some(mapping) => {
                if mapping.len() <= largest {
                    return Err(short("vocabulary mapping", mapping.len()));
                }
                if mapping.iter().any(|&row| row as usize >= rows) {
                    return Err(ModelError::Tensors(
                        "the vocabulary mapping points past the last row".to_owned(),
                    ));
                }
            }
            None if rows <= largest => return Err(short("embedding matrix", rows)),
            None => {}
        }
        if let Some(weights) = &self.weights {
            if weights.len() <= largest {
                return Err(short("list of token weights", weights.len()));
            }
        }
        Ok(())
    }

    /// What the model is known by where vectors are kept: its name and a
    /// fingerprint of its files. A vector made by one model means nothing to
    /// another, and this changes when the files do.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// How long each vector is.
    pub fn dimensions(&self) -> usize {
        self.dimensions
    }

    /// What `text` means, as a vector. A text with no token the model knows
    /// is all zeros.
    pub fn embed(&self, text: &str) -> Vec<f32> {
        let text = match text.char_indices().nth(self.max_chars) {
            Some((cut, _)) => &text[..cut],
            None => text,
        };
        let unknown = self.tokenizer.unknown();
        let mut sum = vec![0.0f32; self.dimensions];
        let mut count = 0usize;
        for id in self
            .tokenizer
            .encode(text)
            .into_iter()
            .filter(|&id| id != unknown)
            .take(MAX_TOKENS)
        {
            let id = id as usize;
            let row = match &self.mapping {
                Some(mapping) => mapping[id] as usize,
                None => id,
            };
            let weight = self.weights.as_ref().map_or(1.0, |weights| weights[id]);
            let start = row * self.dimensions;
            for (total, value) in sum
                .iter_mut()
                .zip(&self.embeddings[start..start + self.dimensions])
            {
                *total += value * weight;
            }
            count += 1;
        }
        if count == 0 {
            return sum;
        }
        for value in &mut sum {
            *value /= count as f32;
        }
        if self.normalize {
            let norm = sum.iter().map(|value| value * value).sum::<f32>().sqrt();
            if norm > 0.0 {
                for value in &mut sum {
                    *value /= norm;
                }
            }
        }
        sum
    }

    /// The token ids `text` becomes, before the unknown token is dropped. For
    /// tests and for explaining a result.
    pub fn tokenize(&self, text: &str) -> Vec<u32> {
        self.tokenizer.encode(text)
    }
}

/// The median length of a token in characters, rounded down, as the reference
/// works it out: the mean of the middle two when there is an even number.
fn median_length(tokenizer: &Tokenizer) -> usize {
    let mut lengths: Vec<usize> = tokenizer
        .tokens()
        .map(|token| token.chars().count())
        .collect();
    if lengths.is_empty() {
        return 1;
    }
    lengths.sort_unstable();
    let middle = lengths.len() / 2;
    let median = if lengths.len().is_multiple_of(2) {
        (lengths[middle - 1] + lengths[middle]) / 2
    } else {
        lengths[middle]
    };
    // A vocabulary of empty strings would otherwise cut every text to none.
    median.max(1)
}

/// Sixteen hex digits of a hash over both files, which is plenty to tell two
/// models apart and short enough to key a table on.
fn fingerprint(tokenizer: &[u8], tensors: &[u8]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&(tokenizer.len() as u64).to_le_bytes());
    hasher.update(tokenizer);
    hasher.update(tensors);
    hasher.finalize().to_hex()[..16].to_owned()
}
