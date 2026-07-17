# Integration Adapter Contract Specification

All external platform integrations (Slack, GitHub, Calendar, Jira) must implement the lifecycle, connection, and formatting contract defined in this document.

---

## 1. The Integration Adapter Interface

Integrations act strictly as **Infrastructure Adapters** (translating external APIs to internal Domain Entities). They are driven by the Application layer.

```rust
use async_trait::async_trait;

#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionStatus {
    Disconnected,
    OfflineMode, // Active fallback when configuration or tokens are missing
    Connecting,
    Connected,
    Reconnecting,
    Failed(String),
}

#[async_trait]
pub trait IntegrationAdapter: Send + Sync {
    /// Initialize credentials, establish initial handshakes (e.g. WebSocket or HTTPS start).
    async fn initialize(&self, secret_provider: &dyn SecretProvider) -> Result<(), AdapterError>;
    
    /// Starts the background sync worker loop. Should spawn tokio tasks internally.
    async fn start(&self, event_publisher: Box<dyn EventBus>) -> Result<(), AdapterError>;
    
    /// Checks connection health and returns active stats.
    async fn health_check(&self) -> Result<ConnectionStatus, AdapterError>;
    
    /// Closes network sockets and flushes local logs.
    async fn shutdown(&self) -> Result<(), AdapterError>;
}
```

---

## 2. Standard Behaviors

### 1. Reconnection & Backoff Policy
Adapters must not panic on connection drop. They must transition to `Reconnecting` and attempt automatic reconnection.
- **Initial Delay**: 5 seconds.
- **Backoff factor**: Exponential 1.5x up to a maximum of 5 minutes.
- **Fail Boundary**: If reconnection fails 10 times consecutively, the adapter transitions to `Failed` and raises a high-priority `SystemAlert` Event.

### 2. Rate Limiting Protection
Adapters must respect HTTP header rate limits (e.g., `Retry-After` headers on Slack/GitHub):
- Outgoing requests must pass through an internal adaptive rate limiter.
- If a `429 Too Many Requests` is received, the adapter must pause all outbound calls for the designated duration and queue outgoing Commands in-memory.

### 3. Zero-Config Fallback (Offline Mode)
If `initialize()` cannot locate any token or credential via the `SecretProvider` chain:
- The adapter **must not return an error or abort the thread**.
- It transitions to `ConnectionStatus::OfflineMode`.
- In `OfflineMode`, it periodically pushes static mock data (e.g., "Offline Workspace Demo", "GitHub Connection Offline") to the Event Bus. This ensures a **Zero-Config portable execution** where the user can test the UI interface without configuring tokens.
