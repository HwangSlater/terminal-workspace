//! SecretProvider Chain and Credential retrieval.

use async_trait::async_trait;
use common::{Result, WorkspaceError};
use secrecy::SecretString;
use std::path::PathBuf;
use std::sync::Arc;

mod encrypted_file;
mod keyring_provider;

pub use encrypted_file::EncryptedFileProvider;
pub use keyring_provider::KeyringProvider;

/// Abstract Secret Provider interface retrieving access tokens safely.
#[async_trait]
pub trait SecretProvider: Send + Sync {
    /// Retrieve OAuth token or API key for targeted service.
    async fn get_secret(&self, key: &str) -> Result<Option<SecretString>>;
}

/// Abstract Secret Writer interface persisting tokens/API keys durably.
/// Separate from [`SecretProvider`] because not every provider can
/// meaningfully be written to — [`EnvProvider`] is read-only, since
/// setting a process environment variable from inside the app wouldn't
/// survive a restart, defeating the point of "durable" storage.
#[async_trait]
pub trait SecretWriter: Send + Sync {
    /// Persist `value` under `key`.
    async fn set_secret(&self, key: &str, value: &str) -> Result<()>;
}

/// Environment variable secret provider.
pub struct EnvProvider;

#[async_trait]
impl SecretProvider for EnvProvider {
    async fn get_secret(&self, key: &str) -> Result<Option<SecretString>> {
        // Retrieve secret from local shell env variable
        if let Ok(value) = std::env::var(key) {
            return Ok(Some(SecretString::from(value)));
        }
        Ok(None)
    }
}

/// `~/.config/terminal-workspace` (or `%USERPROFILE%\.config\terminal-workspace`
/// on Windows) — same base directory `crates/config` uses for `config.toml`,
/// per `docs/05-operations/configuration.md` §3.2's documented
/// `secrets.enc` location.
fn default_secrets_dir() -> PathBuf {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".config")
        .join("terminal-workspace")
}

/// Chain orchestrator routing secret lookup and, separately, secret
/// persistence across multiple providers.
pub struct SecretProviderChain {
    providers: Vec<Arc<dyn SecretProvider>>,
    writers: Vec<Arc<dyn SecretWriter>>,
}

impl SecretProviderChain {
    /// Create a new chain from explicit read and write provider lists.
    #[must_use]
    pub fn new(
        providers: Vec<Arc<dyn SecretProvider>>,
        writers: Vec<Arc<dyn SecretWriter>>,
    ) -> Self {
        Self { providers, writers }
    }

    /// Assemble the canonical order defined by ADR-0006: reads try
    /// `Env -> Keyring -> EncryptedFile`; writes try `Keyring -> EncryptedFile`
    /// (`Env` is read-only — see [`SecretWriter`]'s docs). Keyring and
    /// EncryptedFile each back both lists via the same `Arc` instance, not
    /// duplicated construction. Additive convenience constructor; callers
    /// needing a different order (e.g. tests injecting a mock provider
    /// first) should keep using [`Self::new`] / [`Self::add_provider`].
    #[must_use]
    pub fn default_chain() -> Self {
        let keyring: Arc<KeyringProvider> = Arc::new(KeyringProvider::new());
        let encrypted_file: Arc<EncryptedFileProvider> =
            Arc::new(EncryptedFileProvider::new(default_secrets_dir()));

        Self::new(
            vec![
                Arc::new(EnvProvider),
                Arc::clone(&keyring) as Arc<dyn SecretProvider>,
                Arc::clone(&encrypted_file) as Arc<dyn SecretProvider>,
            ],
            vec![
                keyring as Arc<dyn SecretWriter>,
                encrypted_file as Arc<dyn SecretWriter>,
            ],
        )
    }

    /// Add a dynamic provider to the tail of the read chain.
    pub fn add_provider(&mut self, provider: Arc<dyn SecretProvider>) {
        self.providers.push(provider);
    }

    /// Add a dynamic writer to the tail of the write chain.
    pub fn add_writer(&mut self, writer: Arc<dyn SecretWriter>) {
        self.writers.push(writer);
    }

    /// Query providers sequentially until secret is retrieved.
    pub async fn get_secret(&self, key: &str) -> Result<Option<SecretString>> {
        for provider in &self.providers {
            if let Ok(Some(secret)) = provider.get_secret(key).await {
                return Ok(Some(secret));
            }
        }
        Ok(None)
    }

    /// Persist `value` under `key`, trying each writer in order and
    /// returning on the first success — e.g. falling back to the encrypted
    /// file if no OS keyring backend is reachable.
    pub async fn set_secret(&self, key: &str, value: &str) -> Result<()> {
        let mut last_error = None;
        for writer in &self.writers {
            match writer.set_secret(key, value).await {
                Ok(()) => return Ok(()),
                Err(e) => last_error = Some(e),
            }
        }
        Err(last_error
            .unwrap_or_else(|| WorkspaceError::Security("No secret writer available".into())))
    }
}

/// The chain is itself a `SecretProvider` — anything written against the
/// trait (e.g. `IntegrationAdapter::initialize`) can take a whole chain
/// without knowing it's a chain. (Not delegating to the inherent
/// `get_secret` above to avoid relying on inherent-vs-trait method
/// resolution priority for the same name — the loop is 6 lines, just
/// repeated verbatim.)
#[async_trait]
impl SecretProvider for SecretProviderChain {
    async fn get_secret(&self, key: &str) -> Result<Option<SecretString>> {
        for provider in &self.providers {
            if let Ok(Some(secret)) = provider.get_secret(key).await {
                return Ok(Some(secret));
            }
        }
        Ok(None)
    }
}

/// The chain is itself a `SecretWriter` for the same reason — code that
/// only needs to persist a secret (`SlackAdapter::connect`) can depend on
/// the trait, not the concrete chain type. (Same non-delegation reasoning
/// as the `SecretProvider` impl above.)
#[async_trait]
impl SecretWriter for SecretProviderChain {
    async fn set_secret(&self, key: &str, value: &str) -> Result<()> {
        let mut last_error = None;
        for writer in &self.writers {
            match writer.set_secret(key, value).await {
                Ok(()) => return Ok(()),
                Err(e) => last_error = Some(e),
            }
        }
        Err(last_error
            .unwrap_or_else(|| WorkspaceError::Security("No secret writer available".into())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    struct AlwaysHitProvider(&'static str);

    #[async_trait]
    impl SecretProvider for AlwaysHitProvider {
        async fn get_secret(&self, _key: &str) -> Result<Option<SecretString>> {
            Ok(Some(SecretString::from(self.0.to_string())))
        }
    }

    struct RecordingWriter {
        should_fail: bool,
        received: tokio::sync::Mutex<Vec<(String, String)>>,
    }

    impl RecordingWriter {
        fn new(should_fail: bool) -> Self {
            Self {
                should_fail,
                received: tokio::sync::Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl SecretWriter for RecordingWriter {
        async fn set_secret(&self, key: &str, value: &str) -> Result<()> {
            if self.should_fail {
                return Err(WorkspaceError::Security("simulated failure".into()));
            }
            self.received
                .lock()
                .await
                .push((key.to_string(), value.to_string()));
            Ok(())
        }
    }

    #[tokio::test]
    async fn chain_short_circuits_on_first_hit() {
        let chain = SecretProviderChain::new(
            vec![
                Arc::new(AlwaysHitProvider("first")),
                Arc::new(AlwaysHitProvider("second")),
            ],
            vec![],
        );
        let secret = chain.get_secret("ANY_KEY").await.unwrap();
        assert_eq!(secret.unwrap().expose_secret(), "first");
    }

    #[tokio::test]
    async fn default_chain_prefers_env_over_other_providers() {
        let key = "TW_TEST_DEFAULT_CHAIN_SECRET";
        std::env::set_var(key, "from-env");
        let chain = SecretProviderChain::default_chain();
        let secret = chain.get_secret(key).await.unwrap();
        std::env::remove_var(key);
        assert_eq!(secret.unwrap().expose_secret(), "from-env");
    }

    #[tokio::test]
    async fn default_chain_returns_none_when_nothing_configured() {
        let chain = SecretProviderChain::default_chain();
        let secret = chain
            .get_secret("TW_TEST_DEFAULT_CHAIN_UNSET_KEY")
            .await
            .unwrap();
        assert!(secret.is_none());
    }

    #[tokio::test]
    async fn set_secret_falls_back_to_the_next_writer_on_failure() {
        let failing = Arc::new(RecordingWriter::new(true));
        let succeeding = Arc::new(RecordingWriter::new(false));
        let chain = SecretProviderChain::new(
            vec![],
            vec![
                Arc::clone(&failing) as Arc<dyn SecretWriter>,
                Arc::clone(&succeeding) as Arc<dyn SecretWriter>,
            ],
        );

        chain.set_secret("KEY", "value").await.unwrap();

        assert_eq!(
            succeeding.received.lock().await.as_slice(),
            [("KEY".to_string(), "value".to_string())]
        );
    }

    #[tokio::test]
    async fn set_secret_errors_when_every_writer_fails() {
        let chain = SecretProviderChain::new(
            vec![],
            vec![Arc::new(RecordingWriter::new(true)) as Arc<dyn SecretWriter>],
        );
        let result = chain.set_secret("KEY", "value").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn set_secret_errors_when_there_are_no_writers() {
        let chain = SecretProviderChain::new(vec![], vec![]);
        let result = chain.set_secret("KEY", "value").await;
        assert!(result.is_err());
    }
}
