//! Integration Adapter abstractions.

use async_trait::async_trait;
use common::Result;
use events::EventBus;
use secrets::SecretProvider;
use std::sync::Arc;

/// Operational connection health status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionStatus {
    /// Zero connection active.
    Disconnected,
    /// Actively trying to map socket.
    Connecting,
    /// Safe connection running.
    Connected,
    /// Network dropped; attempting backoff.
    Reconnecting,
    /// Stopped; auth failed or critical error.
    Failed(String),
}

/// System adapter contract defining standard lifecycle interfaces.
#[async_trait]
pub trait IntegrationAdapter: Send + Sync {
    /// Configure connection credentials.
    async fn initialize(&self, secret_provider: &dyn SecretProvider) -> Result<()>;

    /// Spawns background tasks synchronizing states.
    async fn start(&self, event_bus: Arc<dyn EventBus>) -> Result<()>;

    /// Returns current connection metrics.
    async fn health_check(&self) -> Result<ConnectionStatus>;

    /// Gracefully shutdown connections.
    async fn shutdown(&self) -> Result<()>;
}

/// Slack Adapter implementation stub.
pub struct SlackAdapter;

#[async_trait]
impl IntegrationAdapter for SlackAdapter {
    async fn initialize(&self, _secret_provider: &dyn SecretProvider) -> Result<()> {
        Ok(())
    }

    async fn start(&self, _event_bus: Arc<dyn EventBus>) -> Result<()> {
        Ok(())
    }

    async fn health_check(&self) -> Result<ConnectionStatus> {
        Ok(ConnectionStatus::Connected)
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}
