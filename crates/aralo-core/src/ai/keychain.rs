//! The default [`SecretStore`]: the system keychain, through the `keyring`
//! crate. The app and the command line read the same items, so a key saved
//! in one works in the other.

use aralo_ai::{Secret, SecretError, SecretStore};

/// The name every Aralo key is filed under in the keychain. The account is
/// the profile's `key_ref`.
pub const KEYCHAIN_SERVICE: &str = "Aralo AI key";

#[derive(Debug, Clone)]
pub struct KeychainStore {
    service: String,
}

impl Default for KeychainStore {
    fn default() -> Self {
        Self {
            service: KEYCHAIN_SERVICE.into(),
        }
    }
}

impl KeychainStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn entry(&self, key_ref: &str) -> Result<keyring::Entry, SecretError> {
        keyring::Entry::new(&self.service, key_ref).map_err(describe)
    }
}

impl SecretStore for KeychainStore {
    fn get(&self, key_ref: &str) -> Result<Option<Secret>, SecretError> {
        match self.entry(key_ref)?.get_password() {
            Ok(key) => Ok(Some(Secret::new(key))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(describe(error)),
        }
    }

    fn set(&self, key_ref: &str, key: &Secret) -> Result<(), SecretError> {
        self.entry(key_ref)?
            .set_password(key.expose())
            .map_err(describe)
    }

    fn delete(&self, key_ref: &str) -> Result<(), SecretError> {
        match self.entry(key_ref)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(describe(error)),
        }
    }
}

/// The keyring crate's errors name the store and the reason, never the
/// password.
fn describe(error: keyring::Error) -> SecretError {
    SecretError(match error {
        keyring::Error::NoStorageAccess(_) => {
            "the keychain is locked or Aralo was not allowed to use it".into()
        }
        keyring::Error::PlatformFailure(inner) => format!("the keychain failed: {inner}"),
        other => other.to_string(),
    })
}
