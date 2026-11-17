use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::error::{CollectorError, CollectorResult};

const SERVICE_NAME: &str = "TokenVane";
const LEGACY_SERVICE_NAME: &str = "TokenTracker";

/// Abstraction over OS native credential storage (Windows Credential Manager / macOS Keychain)
/// and an in-memory stub for automated tests.
pub trait CredentialManager: Send + Sync {
    fn set_api_key(&self, provider_id: &str, api_key: &str) -> CollectorResult<()>;
    fn get_api_key(&self, provider_id: &str) -> CollectorResult<Option<String>>;
    fn delete_api_key(&self, provider_id: &str) -> CollectorResult<()>;
    fn has_api_key(&self, provider_id: &str) -> CollectorResult<bool>;
}

/// OS native keyring backed by Windows Credential Manager on Windows.
#[derive(Debug, Default, Clone)]
pub struct OsKeyringStore;

impl OsKeyringStore {
    pub fn new() -> Self {
        Self
    }

    fn entry(provider_id: &str) -> CollectorResult<keyring::Entry> {
        keyring::Entry::new(SERVICE_NAME, provider_id)
            .map_err(|e| CollectorError::Credential(format!("Failed to initialize keyring entry: {e}")))
    }
}

impl CredentialManager for OsKeyringStore {
    fn set_api_key(&self, provider_id: &str, api_key: &str) -> CollectorResult<()> {
        let trimmed = api_key.trim();
        if trimmed.is_empty() {
            return Err(CollectorError::Credential(
                "API key must not be empty".to_string(),
            ));
        }
        let entry = Self::entry(provider_id)?;
        entry
            .set_password(trimmed)
            .map_err(|e| CollectorError::Credential(format!("Failed to save API key to OS keyring: {e}")))
    }

    fn get_api_key(&self, provider_id: &str) -> CollectorResult<Option<String>> {
        let entry = Self::entry(provider_id)?;
        match entry.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => {
                if let Ok(legacy_entry) = keyring::Entry::new(LEGACY_SERVICE_NAME, provider_id) {
                    if let Ok(secret) = legacy_entry.get_password() {
                        let _ = entry.set_password(&secret);
                        return Ok(Some(secret));
                    }
                }
                Ok(None)
            }
            Err(e) => Err(CollectorError::Credential(format!(
                "Failed to read API key from OS keyring: {e}"
            ))),
        }
    }

    fn delete_api_key(&self, provider_id: &str) -> CollectorResult<()> {
        let entry = Self::entry(provider_id)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(CollectorError::Credential(format!(
                "Failed to delete API key from OS keyring: {e}"
            ))),
        }
    }

    fn has_api_key(&self, provider_id: &str) -> CollectorResult<bool> {
        Ok(self.get_api_key(provider_id)?.is_some())
    }
}

/// Thread-safe in-memory credential store for deterministic integration tests.
#[derive(Debug, Default, Clone)]
pub struct MemoryCredentialStore {
    inner: Arc<RwLock<HashMap<String, String>>>,
}

impl MemoryCredentialStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl CredentialManager for MemoryCredentialStore {
    fn set_api_key(&self, provider_id: &str, api_key: &str) -> CollectorResult<()> {
        let trimmed = api_key.trim();
        if trimmed.is_empty() {
            return Err(CollectorError::Credential(
                "API key must not be empty".to_string(),
            ));
        }
        let mut guard = self
            .inner
            .write()
            .map_err(|_| CollectorError::Credential("Credential lock poisoned".to_string()))?;
        guard.insert(provider_id.to_string(), trimmed.to_string());
        Ok(())
    }

    fn get_api_key(&self, provider_id: &str) -> CollectorResult<Option<String>> {
        let guard = self
            .inner
            .read()
            .map_err(|_| CollectorError::Credential("Credential lock poisoned".to_string()))?;
        Ok(guard.get(provider_id).cloned())
    }

    fn delete_api_key(&self, provider_id: &str) -> CollectorResult<()> {
        let mut guard = self
            .inner
            .write()
            .map_err(|_| CollectorError::Credential("Credential lock poisoned".to_string()))?;
        guard.remove(provider_id);
        Ok(())
    }

    fn has_api_key(&self, provider_id: &str) -> CollectorResult<bool> {
        let guard = self
            .inner
            .read()
            .map_err(|_| CollectorError::Credential("Credential lock poisoned".to_string()))?;
        Ok(guard.contains_key(provider_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_credential_store_lifecycle() {
        let store = MemoryCredentialStore::new();
        assert!(!store.has_api_key("claude_code").expect("check"));
        store
            .set_api_key("claude_code", "sk-ant-admin-test-123")
            .expect("set");
        assert!(store.has_api_key("claude_code").expect("has"));
        assert_eq!(
            store.get_api_key("claude_code").expect("get").as_deref(),
            Some("sk-ant-admin-test-123")
        );
        store.delete_api_key("claude_code").expect("del");
        assert!(!store.has_api_key("claude_code").expect("absent"));
    }
}
