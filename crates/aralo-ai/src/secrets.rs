//! Where API keys live (PRD P13). A profile names its key by `key_ref`; the
//! key itself is only ever in a [`SecretStore`] and, for one request, in a
//! [`Secret`].
//!
//! The default store is the system keychain, in `aralo-core`. A shell may
//! bring its own, and the tests use [`MemorySecretStore`].

use std::collections::HashMap;
use std::sync::Mutex;

use crate::model::Secret;

/// Why the store could not do what was asked. The message never holds the
/// key.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the key store: {0}")]
pub struct SecretError(pub String);

pub trait SecretStore: Send + Sync {
    /// The key stored under `key_ref`, or `None` when there is none.
    fn get(&self, key_ref: &str) -> Result<Option<Secret>, SecretError>;

    /// Stores `key` under `key_ref`, replacing what was there.
    fn set(&self, key_ref: &str, key: &Secret) -> Result<(), SecretError>;

    /// Removes the key. Removing one that is not there is not an error.
    fn delete(&self, key_ref: &str) -> Result<(), SecretError>;
}

/// Keys in memory, for tests and for shells that keep none.
#[derive(Default)]
pub struct MemorySecretStore {
    keys: Mutex<HashMap<String, Secret>>,
}

impl std::fmt::Debug for MemorySecretStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemorySecretStore").finish_non_exhaustive()
    }
}

impl MemorySecretStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn keys(&self) -> std::sync::MutexGuard<'_, HashMap<String, Secret>> {
        self.keys
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl SecretStore for MemorySecretStore {
    fn get(&self, key_ref: &str) -> Result<Option<Secret>, SecretError> {
        Ok(self.keys().get(key_ref).map(Secret::duplicate))
    }

    fn set(&self, key_ref: &str, key: &Secret) -> Result<(), SecretError> {
        self.keys().insert(key_ref.to_owned(), key.duplicate());
        Ok(())
    }

    fn delete(&self, key_ref: &str) -> Result<(), SecretError> {
        self.keys().remove(key_ref);
        Ok(())
    }
}
