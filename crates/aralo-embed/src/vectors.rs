//! The vector scan: every stored vector against the query, in one pass.
//!
//! A snippet library is thousands of vectors, not millions, and a dot product
//! of a few hundred numbers is nanoseconds, so the whole set is scanned on
//! every query. An approximate index would be faster only past a size no
//! library reaches, and would answer slightly differently from this.

use std::cmp::Ordering;

/// Vectors to search by what they mean, each under a key the caller chooses.
#[derive(Debug, Clone)]
pub struct Vectors<K> {
    dimensions: usize,
    keys: Vec<K>,
    /// Row-major, each row scaled to unit length.
    data: Vec<f32>,
}

impl<K> Vectors<K> {
    pub fn new(dimensions: usize) -> Self {
        Self {
            dimensions,
            keys: Vec::new(),
            data: Vec::new(),
        }
    }

    pub fn dimensions(&self) -> usize {
        self.dimensions
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Adds `vector` under `key`, scaled to unit length. False, and nothing
    /// added, when it is the wrong length or all zeros: a text the model knew
    /// nothing of points nowhere, and is similar to nothing.
    pub fn push(&mut self, key: K, vector: &[f32]) -> bool {
        if vector.len() != self.dimensions {
            return false;
        }
        let Some(norm) = norm(vector) else {
            return false;
        };
        self.keys.push(key);
        self.data.extend(vector.iter().map(|value| value / norm));
        true
    }

    /// The keys whose vectors point most nearly where `query` does, most
    /// similar first, each with its cosine similarity: 1 is the same meaning,
    /// 0 is unrelated. At most `limit`, and none below `at_least`.
    pub fn nearest(&self, query: &[f32], limit: usize, at_least: f32) -> Vec<(&K, f32)> {
        if query.len() != self.dimensions || limit == 0 {
            return Vec::new();
        }
        let Some(norm) = norm(query) else {
            return Vec::new();
        };
        let mut scored: Vec<(usize, f32)> = self
            .data
            .chunks_exact(self.dimensions.max(1))
            .enumerate()
            .map(|(row, vector)| {
                let dot: f32 = vector.iter().zip(query).map(|(a, b)| a * b).sum();
                (row, dot / norm)
            })
            .filter(|&(_, similarity)| similarity >= at_least)
            .collect();
        // Ties keep the order vectors were added in, so the same query over
        // the same set always answers the same way.
        scored.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(Ordering::Equal)
                .then(a.0.cmp(&b.0))
        });
        scored.truncate(limit);
        scored
            .into_iter()
            .map(|(row, similarity)| (&self.keys[row], similarity))
            .collect()
    }
}

fn norm(vector: &[f32]) -> Option<f32> {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    (norm > 0.0 && norm.is_finite()).then_some(norm)
}

/// A vector as it is stored: little-endian `f32`s, one after another.
pub fn to_bytes(vector: &[f32]) -> Vec<u8> {
    vector
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

/// The vector stored in `bytes`, or `None` when they are not a whole number of
/// `f32`s.
pub fn from_bytes(bytes: &[u8]) -> Option<Vec<f32>> {
    if !bytes.len().is_multiple_of(4) {
        return None;
    }
    Some(
        bytes
            .chunks_exact(4)
            .map(|four| f32::from_le_bytes([four[0], four[1], four[2], four[3]]))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_nearest_come_first_and_the_far_are_left_out() {
        let mut vectors = Vectors::new(2);
        assert!(vectors.push("east", &[1.0, 0.0]));
        assert!(vectors.push("north", &[0.0, 3.0]));
        assert!(vectors.push("north-east", &[1.0, 1.0]));
        assert!(vectors.push("west", &[-1.0, 0.0]));

        let found = vectors.nearest(&[2.0, 0.1], 10, 0.5);
        let keys: Vec<&str> = found.iter().map(|(key, _)| **key).collect();
        assert_eq!(keys, ["east", "north-east"]);
        assert!((found[0].1 - 0.998_75).abs() < 1e-4);

        assert_eq!(vectors.nearest(&[2.0, 0.1], 1, -1.0).len(), 1);
        assert!(vectors.nearest(&[0.0, 0.0], 10, -1.0).is_empty());
    }

    #[test]
    fn a_vector_that_points_nowhere_is_not_kept() {
        let mut vectors = Vectors::new(2);
        assert!(!vectors.push("nothing", &[0.0, 0.0]));
        assert!(!vectors.push("wrong length", &[1.0, 0.0, 0.0]));
        assert!(vectors.is_empty());
    }

    #[test]
    fn a_stored_vector_comes_back_as_it_went() {
        let vector = vec![0.5, -1.25, 3.0e-8];
        assert_eq!(from_bytes(&to_bytes(&vector)), Some(vector));
        assert_eq!(from_bytes(&[0, 0, 0]), None);
    }
}
