use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use ulid::Ulid;

/// A snippet's identity: a ULID that survives renames and moves.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SnippetId(Ulid);

#[derive(Debug, thiserror::Error)]
#[error("`{0}` is not a ULID (26 Crockford base32 characters)")]
pub struct InvalidId(String);

impl SnippetId {
    /// A fresh ID for a new snippet.
    pub fn generate() -> Self {
        Self(Ulid::generate())
    }

    pub const fn from_u128(value: u128) -> Self {
        Self(Ulid(value))
    }

    /// The form the engine matches on.
    pub const fn as_u128(self) -> u128 {
        self.0 .0
    }
}

impl fmt::Display for SnippetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl fmt::Debug for SnippetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SnippetId({})", self.0)
    }
}

impl FromStr for SnippetId {
    type Err = InvalidId;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ulid::from_string(s.trim())
            .map(Self)
            .map_err(|_| InvalidId(s.to_owned()))
    }
}

impl Serialize for SnippetId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for SnippetId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_text() {
        let id: SnippetId = "01J8ZK3V5Q8W6T9X2N4R7M0ABC".parse().unwrap();
        assert_eq!(id.to_string(), "01J8ZK3V5Q8W6T9X2N4R7M0ABC");
        assert_eq!(SnippetId::from_u128(id.as_u128()), id);
    }

    #[test]
    fn generated_ids_are_distinct() {
        assert_ne!(SnippetId::generate(), SnippetId::generate());
    }

    #[test]
    fn rejects_anything_that_is_not_a_ulid() {
        assert!("refund".parse::<SnippetId>().is_err());
        assert!("".parse::<SnippetId>().is_err());
    }
}
