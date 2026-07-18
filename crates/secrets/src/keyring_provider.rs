//! OS-native keyring secret storage. See `docs/04-extensions/security.md` §1.

use crate::{SecretProvider, SecretWriter};
use async_trait::async_trait;
use common::{Result, WorkspaceError};
use secrecy::SecretString;

const DEFAULT_SERVICE: &str = "terminal-workspace";

/// Reads/writes secrets via the OS-native credential store: Windows
/// Credential Manager, macOS Keychain, or Linux Secret Service (via a
/// pure-Rust DBus client — not the C-binding `libdbus` flavor, keeping
/// ADR-0014's "no C compiler required" property intact).
pub struct KeyringProvider {
    service: String,
}

impl KeyringProvider {
    /// Create a provider scoped to the app's default keyring service name.
    #[must_use]
    pub fn new() -> Self {
        Self::with_service(DEFAULT_SERVICE)
    }

    /// Create a provider scoped to a specific keyring service name —
    /// primarily so tests don't read/write under the same service name a
    /// real run of the app would use.
    #[must_use]
    pub fn with_service(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    fn entry(&self, key: &str) -> Result<keyring::Entry> {
        keyring::Entry::new(&self.service, key)
            .map_err(|e| WorkspaceError::Security(format!("Keyring entry failed: {e}")))
    }
}

impl Default for KeyringProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SecretProvider for KeyringProvider {
    async fn get_secret(&self, key: &str) -> Result<Option<SecretString>> {
        match self.entry(key)?.get_password() {
            Ok(value) => Ok(Some(SecretString::from(value))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(WorkspaceError::Security(format!(
                "Keyring read failed: {e}"
            ))),
        }
    }
}

#[async_trait]
impl SecretWriter for KeyringProvider {
    async fn set_secret(&self, key: &str, value: &str) -> Result<()> {
        self.entry(key)?
            .set_password(value)
            .map_err(|e| WorkspaceError::Security(format!("Keyring write failed: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    /// A dedicated service name (not the real app's) so this doesn't touch
    /// whatever token a real run of the app may have stored on this machine.
    fn test_provider() -> KeyringProvider {
        KeyringProvider::with_service("terminal-workspace-tests")
    }

    /// Requires a real OS keyring backend to be reachable (Windows
    /// Credential Manager is always present; Linux CI without a DBus
    /// Secret Service session would need this ignored). Not run as part of
    /// the default `cargo test --workspace` sweep for that reason.
    ///
    /// Both assertions live in one test function rather than two, and
    /// deliberately in this order (missing-entry first) — `keyring`
    /// 4.1.5's `v1` compatibility shim has a real race condition in its
    /// lazy default-store initialization (`SET_CREDENTIAL_STORE.compare_exchange`
    /// flips to "done" *before* `keyring_core::set_default_store` actually
    /// runs), so two of these tests as separate `#[tokio::test]` functions
    /// can run on different threads and race: whichever loses sees
    /// `Error::NoDefaultStore` even though the backend itself is fine (this
    /// was confirmed by hand — the round-trip alone passes reliably).
    /// Keeping both checks sequential in one test sidesteps the upstream
    /// bug entirely rather than working around it in our own code.
    #[tokio::test]
    #[ignore = "requires a live OS keyring backend"]
    async fn round_trips_and_reports_missing_entries_via_the_real_os_keyring() {
        let provider = test_provider();

        let missing = provider
            .get_secret("TW_TEST_KEYRING_DEFINITELY_UNSET")
            .await
            .unwrap();
        assert!(missing.is_none());

        let key = "TW_TEST_KEYRING_ROUNDTRIP";
        provider.set_secret(key, "hunter2").await.unwrap();
        let secret = provider.get_secret(key).await.unwrap();
        assert_eq!(secret.unwrap().expose_secret(), "hunter2");
    }
}
