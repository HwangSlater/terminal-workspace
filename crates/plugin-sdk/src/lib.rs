//! Plugin SDK interface contracts.

use async_trait::async_trait;
use common::Result;

/// Host capabilities requested in manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginCapability {
    /// Connect to remote networks.
    NetworkConnect(String),
    /// Read local folders.
    FsRead(String),
    /// Write local folders.
    FsWrite(String),
    /// Request Slack reading.
    SlackRead,
}

/// Dynamic guest WASM Plugin interface contracts.
#[async_trait]
pub trait GuestPlugin: Send + Sync {
    /// Initialize config state.
    async fn initialize(&self, config_toml: &str) -> Result<()>;

    /// Handle event broadcasted from host.
    async fn on_event(&self, event_type: &str, payload_json: &str) -> Result<()>;

    /// Cleanup allocations.
    async fn shutdown(&self) -> Result<()>;
}
