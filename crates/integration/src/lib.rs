//! Integration Adapter abstractions. See `docs/04-extensions/integration-contract.md`.

use async_trait::async_trait;
use common::Result;
use events::EventBus;
use secrets::SecretProvider;
use std::sync::Arc;

pub mod slack;

pub use slack::{SlackAdapter, SlackConfig, SlackConnector, SlackMessenger};

/// Operational connection health status. See
/// `docs/04-extensions/state-machine.md` for the transition rules of a
/// polling-based adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionStatus {
    /// Never configured (no credential found), or configured but no poll
    /// has completed yet under a credential-less run. Not an error state.
    Disconnected,
    /// Credential found; the first poll cycle hasn't completed yet.
    Connecting,
    /// Last poll cycle succeeded.
    Connected,
    /// 5-9 consecutive poll failures; still attempting recovery.
    Reconnecting,
    /// 10+ consecutive poll failures; a `SystemAlert` Event was raised.
    Failed(String),
}

/// System adapter contract defining standard lifecycle interfaces. Not part
/// of Architecture Freeze v1 (`docs/06-development/development.md` §3) —
/// may evolve via ordinary review.
#[async_trait]
pub trait IntegrationAdapter: Send + Sync {
    /// Resolve credentials via the `SecretProviderChain`. Must not fail
    /// when no credential is found (`docs/04-extensions/integration-contract.md` §2.3).
    async fn initialize(&self, secret_provider: &dyn SecretProvider) -> Result<()>;

    /// Spawns the background sync loop. Returns once the loop is running,
    /// not once it exits.
    async fn start(&self, event_bus: Arc<dyn EventBus>) -> Result<()>;

    /// Returns the adapter's current status.
    async fn health_check(&self) -> Result<ConnectionStatus>;

    /// Stops the sync loop and releases resources.
    async fn shutdown(&self) -> Result<()>;
}
