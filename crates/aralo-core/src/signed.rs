//! Signed data tables (plan section 9, "Signed data updates").
//!
//! The compatibility table ships inside the build, and a newer one can arrive
//! as a download published with each release: `apps.toml` beside
//! `apps.toml.sig`, an Ed25519 signature over the file's bytes, in hex. The
//! public key is compiled in from `data/keys/data-tables.pub`. A table whose
//! signature does not check against that key is refused, and the table in use
//! stays in use.
//!
//! The release workflow signs with `openssl pkeyutl -sign -rawin` (see
//! `scripts/release.sh`). A fixture signed that way is checked here by
//! `tests/signed_data.rs`, so the two ends agree on the format.

use aws_lc_rs::signature::{UnparsedPublicKey, ED25519};

use crate::compat::{CompatError, CompatTable};

/// The key file as committed. While it says `unset`, no download is accepted.
const BUNDLED_KEY: &str = include_str!("../../../data/keys/data-tables.pub");

/// An Ed25519 public key for data tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataKey([u8; 32]);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SignedDataError {
    #[error("this build has no data-table key, so no downloaded table is accepted")]
    NoKey,
    #[error("the data-table key is not 64 hex digits")]
    BadKey,
    #[error("the signature is not 128 hex digits")]
    BadSignatureEncoding,
    #[error("the signature does not match the table")]
    Mismatch,
    #[error(transparent)]
    Table(#[from] CompatError),
}

impl DataKey {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// 64 hex digits, surrounding white space ignored.
    pub fn from_hex(text: &str) -> Result<Self, SignedDataError> {
        decode_hex::<32>(text.trim())
            .map(Self)
            .ok_or(SignedDataError::BadKey)
    }

    /// Reads a key file: `#` lines are comments, and the first other line is
    /// the key, or `unset`.
    pub fn from_file_text(text: &str) -> Result<Option<Self>, SignedDataError> {
        let line = text
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with('#'));
        match line {
            None | Some("unset") => Ok(None),
            Some(key) => Self::from_hex(key).map(Some),
        }
    }

    /// The key compiled into this build, if one is set.
    pub fn bundled() -> Option<Self> {
        Self::from_file_text(BUNDLED_KEY).ok().flatten()
    }

    /// Checks `signature_hex` (128 hex digits) over `data`.
    pub fn verify(&self, data: &[u8], signature_hex: &str) -> Result<(), SignedDataError> {
        let signature =
            decode_hex::<64>(signature_hex.trim()).ok_or(SignedDataError::BadSignatureEncoding)?;
        UnparsedPublicKey::new(&ED25519, &self.0)
            .verify(data, &signature)
            .map_err(|_| SignedDataError::Mismatch)
    }
}

impl CompatTable {
    /// A downloaded table, accepted only when `signature_hex` checks against
    /// `key` and the text then parses. With no key, nothing is accepted.
    pub fn from_signed(
        text: &str,
        signature_hex: &str,
        key: Option<&DataKey>,
    ) -> Result<Self, SignedDataError> {
        let key = key.ok_or(SignedDataError::NoKey)?;
        key.verify(text.as_bytes(), signature_hex)?;
        Ok(Self::parse(text)?)
    }

    /// Replaces this table with a downloaded one if it is signed and parses.
    /// On any error this table is left exactly as it was: a bad download
    /// keeps the bundled copy, or whichever table was in use.
    pub fn update_from_signed(
        &mut self,
        text: &str,
        signature_hex: &str,
        key: Option<&DataKey>,
    ) -> Result<(), SignedDataError> {
        *self = Self::from_signed(text, signature_hex, key)?;
        Ok(())
    }
}

fn decode_hex<const N: usize>(text: &str) -> Option<[u8; N]> {
    let bytes = text.as_bytes();
    if bytes.len() != N * 2 {
        return None;
    }
    let mut out = [0u8; N];
    let (pairs, _) = bytes.as_chunks::<2>();
    for (index, [high, low]) in pairs.iter().enumerate() {
        let high = char::from(*high).to_digit(16)?;
        let low = char::from(*low).to_digit(16)?;
        out[index] = u8::try_from(high * 16 + low).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_committed_key_file_reads() {
        // `unset` or a key; either way the file itself must parse.
        DataKey::from_file_text(BUNDLED_KEY).unwrap();
    }

    #[test]
    fn hex_is_exact_length_and_digits_only() {
        assert!(decode_hex::<2>("0aFf").is_some());
        assert!(decode_hex::<2>("0aF").is_none());
        assert!(decode_hex::<2>("0aFfF0").is_none());
        assert!(decode_hex::<2>("0g00").is_none());
        assert!(decode_hex::<2>("+0ab").is_none());
    }
}
