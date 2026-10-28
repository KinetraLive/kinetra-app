//! Secrets of API sources: the only place that touches the OS keychain.
//!
//! A secret header stores only a random secret id in the source file; the value lives in the
//! keychain under service [`SERVICE`], account `secret:<secretId>` (Windows Credential Manager).
//! Values are write-only from the UI (`set_source_secret`), read only by the poller and the test
//! request when they build a request, and never logged, put in an error message, written to a
//! file, exported or backed up.
//!
//! [`SecretStore`] abstracts the keychain so tests (and the e2e fixture server) use
//! [`MemorySecrets`] and never touch the real one.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

/// Keychain service name; canary builds keep their own, so they never share sign-in or secrets
/// with stable.
pub const SERVICE: &str = if crate::core::model::CANARY_BUILD {
    "Kinetra Canary"
} else {
    "Kinetra"
};

/// A keychain failure. The message never contains a secret value.
#[derive(Debug, thiserror::Error)]
#[error("The system keychain failed: {0}")]
pub struct SecretError(String);

impl SecretError {
    /// What failed, for messages that add their own context ("no access to the credential
    /// store").
    pub fn reason(&self) -> &str {
        &self.0
    }

    #[cfg(test)]
    pub(crate) fn for_tests(reason: &str) -> Self {
        Self(reason.to_string())
    }
}

impl From<SecretError> for crate::error::AppError {
    fn from(e: SecretError) -> Self {
        crate::error::AppError::Internal(e.to_string())
    }
}

/// Where secret values live (see the module docs).
pub trait SecretStore: Send + Sync {
    /// The value, or `None` when there is none on this machine.
    fn get(&self, id: &str) -> Result<Option<String>, SecretError>;
    fn set(&self, id: &str, value: &str) -> Result<(), SecretError>;
    /// Removes the value; no error when there is none.
    fn delete(&self, id: &str) -> Result<(), SecretError>;
}

/// The keychain service this process uses: [`SERVICE`], or its dev profile's
/// (`core::devprofile`, debug builds only).
pub fn service() -> String {
    crate::core::devprofile::keychain_service(SERVICE, crate::core::devprofile::active())
}

fn account(id: &str) -> String {
    format!("secret:{id}")
}

/// The OS keychain (`keyring`; Windows Credential Manager).
#[derive(Debug, Default)]
pub struct KeyringSecrets;

impl KeyringSecrets {
    fn entry(id: &str) -> Result<keyring::Entry, SecretError> {
        keyring::Entry::new(&service(), &account(id)).map_err(|e| SecretError(describe(&e)))
    }
}

/// A keyring error without anything that could hold a value.
fn describe(e: &keyring::Error) -> String {
    match e {
        keyring::Error::NoStorageAccess(_) => "no access to the credential store".into(),
        keyring::Error::PlatformFailure(_) => "the credential store failed".into(),
        keyring::Error::TooLong(what, max) => format!("{what} is longer than {max}"),
        keyring::Error::Invalid(what, _) => format!("invalid {what}"),
        keyring::Error::NoEntry => "no entry".into(),
        keyring::Error::BadEncoding(_) => "the stored value is not text".into(),
        keyring::Error::Ambiguous(_) => "more than one matching entry".into(),
        _ => "unknown error".into(),
    }
}

impl SecretStore for KeyringSecrets {
    fn get(&self, id: &str) -> Result<Option<String>, SecretError> {
        match Self::entry(id)?.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(SecretError(describe(&e))),
        }
    }

    fn set(&self, id: &str, value: &str) -> Result<(), SecretError> {
        Self::entry(id)?
            .set_password(value)
            .map_err(|e| SecretError(describe(&e)))
    }

    fn delete(&self, id: &str) -> Result<(), SecretError> {
        match Self::entry(id)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(SecretError(describe(&e))),
        }
    }
}

/// An in-memory store: tests and the e2e fixture server (never the real keychain).
#[derive(Debug, Default)]
pub struct MemorySecrets(Mutex<HashMap<String, String>>);

impl MemorySecrets {
    /// Number of stored values (for tests).
    pub fn len(&self) -> usize {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl SecretStore for MemorySecrets {
    fn get(&self, id: &str) -> Result<Option<String>, SecretError> {
        Ok(self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(id)
            .cloned())
    }

    fn set(&self, id: &str, value: &str) -> Result<(), SecretError> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id.to_string(), value.to_string());
        Ok(())
    }

    fn delete(&self, id: &str) -> Result<(), SecretError> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_round_trip() {
        let s = MemorySecrets::default();
        assert_eq!(s.get("a").unwrap(), None);
        s.set("a", "v").unwrap();
        assert_eq!(s.get("a").unwrap().as_deref(), Some("v"));
        s.delete("a").unwrap();
        s.delete("a").unwrap();
        assert!(s.is_empty());
    }

    #[test]
    fn accounts_are_namespaced() {
        assert_eq!(account("abc"), "secret:abc");
    }
}
