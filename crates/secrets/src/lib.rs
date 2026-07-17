//! SecretProvider Chain and Credential retrieval.

use async_trait::async_trait;
use common::Result;
use secrecy::SecretString;

/// Abstract Secret Provider interface retrieving access tokens safely.
#[async_trait]
pub trait SecretProvider: Send + Sync {
    /// Retrieve OAuth token or API key for targeted service.
    async fn get_secret(&self, key: &str) -> Result<Option<SecretString>>;
}

/// Keyring system secret provider.
pub struct KeyringProvider;

#[async_trait]
impl SecretProvider for KeyringProvider {
    async fn get_secret(&self, _key: &str) -> Result<Option<SecretString>> {
        // Stub implementation - will link keyring crate in Phase 3
        Ok(None)
    }
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

/// Encrypted local file secret provider for zero-dependency portability.
pub struct EncryptedFileProvider;

#[async_trait]
impl SecretProvider for EncryptedFileProvider {
    async fn get_secret(&self, _key: &str) -> Result<Option<SecretString>> {
        // Stub implementation - reads encrypted vault file on local disk
        Ok(None)
    }
}

/// Chain orchestrator routing secret lookup across multiple providers.
pub struct SecretProviderChain {
    providers: Vec<Box<dyn SecretProvider>>,
}

impl SecretProviderChain {
    /// Create new provider chain.
    #[must_use]
    pub fn new(providers: Vec<Box<dyn SecretProvider>>) -> Self {
        Self { providers }
    }

    /// Assemble the canonical provider order defined by ADR-0006:
    /// `Env -> Keyring -> EncryptedFile`. Additive convenience constructor;
    /// callers needing a different order (e.g. tests injecting a mock
    /// provider first) should keep using [`Self::new`] / [`Self::add_provider`].
    #[must_use]
    pub fn default_chain() -> Self {
        Self::new(vec![
            Box::new(EnvProvider),
            Box::new(KeyringProvider),
            Box::new(EncryptedFileProvider),
        ])
    }

    /// Add dynamic provider to the tail of the chain.
    pub fn add_provider(&mut self, provider: Box<dyn SecretProvider>) {
        self.providers.push(provider);
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

    #[tokio::test]
    async fn chain_short_circuits_on_first_hit() {
        let chain = SecretProviderChain::new(vec![
            Box::new(AlwaysHitProvider("first")),
            Box::new(AlwaysHitProvider("second")),
        ]);
        let secret = chain.get_secret("ANY_KEY").await.unwrap();
        assert_eq!(secret.unwrap().expose_secret(), "first");
    }

    #[tokio::test]
    async fn default_chain_prefers_env_over_stub_providers() {
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
}
