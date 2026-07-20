# Integration Adapter Contract Specification

All external platform integrations (Slack, GitHub, Calendar, Jira) must implement the lifecycle and behavior contract defined in this document.

> **Implementation Status (Phase 6)**: this document was revised alongside the first real adapter (`SlackAdapter`, `crates/integration`) — see `step6.md` for the decisions behind the changes below. It previously specified a WebSocket-shaped reconnect policy and an "OfflineMode with fake demo data" fallback; both were replaced (§2.1, §2.3) once the actual sync mechanism (polling, not a persistent connection) and the project's established no-fake-data principle (`step5.md`) were factored in.

---

## 1. The Integration Adapter Interface

Integrations act strictly as **Infrastructure Adapters** (translating external APIs to internal Domain Entities). They are driven by the Application layer. This trait is **not** part of Architecture Freeze v1 (`docs/06-development/development.md` §3) — it may evolve via ordinary review, no ADR required.

```rust
use async_trait::async_trait;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionStatus {
    /// Never configured, or the last sync attempt failed and no earlier
    /// success exists to fall back on. Not an error state by itself.
    Disconnected,
    /// Currently establishing (only meaningful for adapters that hold a
    /// persistent connection, e.g. a future Socket Mode upgrade).
    Connecting,
    /// Last sync cycle succeeded.
    Connected,
    /// Attempting to recover after failures (see §2.1).
    Reconnecting,
    /// Exceeded the consecutive-failure threshold; a `SystemAlert` was raised.
    Failed(String),
}

#[async_trait]
pub trait IntegrationAdapter: Send + Sync {
    /// Resolve credentials via the `SecretProviderChain` (ADR-0006). Must
    /// not fail when no credential is found — see §2.3.
    async fn initialize(&self, secret_provider: &dyn SecretProvider) -> common::Result<()>;

    /// Starts the background sync loop. Spawns its own `tokio` task(s)
    /// internally; returns once the loop is running, not once it exits.
    async fn start(&self, event_bus: Arc<dyn EventBus>) -> common::Result<()>;

    /// Returns the adapter's current status.
    async fn health_check(&self) -> common::Result<ConnectionStatus>;

    /// Stops the sync loop and releases any resources.
    async fn shutdown(&self) -> common::Result<()>;

    /// Runs one poll cycle immediately, out-of-band from the interval
    /// loop's own schedule (`step46.md`, backing `Command::SyncAllAdapters`
    /// / `/sync`). A no-op returning `Ok(())` when there's nothing to poll
    /// with (no credential/connection) — same "not configured is not an
    /// error" rule §2.3 gives `start()`.
    async fn sync_now(&self, event_bus: Arc<dyn EventBus>) -> common::Result<()>;
}
```

**`step46.md`**: every implementation serializes `sync_now` against its own background loop via a shared per-adapter lock, and reuses the exact same poll-cycle logic (`poll_once` + the §2.1 failure-count state machine + status-change events) rather than restarting the loop. Restarting would have been the simpler-looking option (`shutdown()` then `start()`, which every existing selection-update method — `SlackAdapter::update_selection`, `GitHubAdapter::update_selection`, `CalendarAdapter::keep_only` — already does), but it resets any per-loop "is this the very first poll" tracking a concrete adapter keeps (GitHub's and Calendar's do; Slack's doesn't, since its per-channel cursor already persists that signal). A restarted loop would then treat anything a manual sync turns up as if it predated this session and mark it already-read, silently suppressing the desktop toast a manual sync exists to produce.

`common::Result` (i.e. `Result<T, WorkspaceError>`) is used here rather than a bespoke `AdapterError`, matching every other trait in this codebase (`NotificationRepository`, `SecretProvider`, etc.) — a per-adapter error type would be one more thing call sites need to convert, for no benefit `WorkspaceError::Integration(String)` doesn't already provide.

---

## 2. Standard Behaviors

### 1. Failure Handling (Polling Model)

`SlackAdapter` (and any future adapter built the same way) polls on an interval rather than holding a persistent connection, so there is no "drop" to reconnect from — only a poll cycle that succeeds or fails.

- A single failed poll (network error, non-2xx response) is logged and skipped; the next cycle tries again at the normal interval. This does **not** change `ConnectionStatus`.
- After **5 consecutive** failed cycles, status moves to `Reconnecting` and a `SystemAlert` Event is *not* yet raised (still recoverable).
- After **10 consecutive** failed cycles, status moves to `Failed(reason)` and a high-priority `SystemAlert` Event is raised.
- Any subsequent successful cycle resets the counter and returns status to `Connected`.

(A future adapter built on a persistent connection — e.g. Slack Socket Mode — would need real exponential backoff on reconnect attempts; that policy is deferred until such an adapter exists, rather than specified speculatively here.)

### 2. Rate Limiting Protection

Adapters must respect HTTP header rate limits (e.g. Slack's `Retry-After` on a `429`):
- On a `429`, the adapter pauses outbound calls for the header's duration and skips the current poll cycle rather than treating it as a failure under §2.1's counter.
- This is not optional politeness — hammering a rate limit only makes the outage longer for a locally-run, single-user tool with no request queue to smooth things out.

### 3. No-Credential Behavior (Zero-Config, Honest-Empty)

If `initialize()` cannot locate a token via the `SecretProviderChain`:
- The adapter **must not return an error or abort**. Returning an error here would surface as a boot failure, which contradicts Zero Configuration (`product-requirements.md` §2.1) — using the app without Slack configured must work.
- It reports `ConnectionStatus::Disconnected` and simply never starts a poll loop (or `start()` no-ops).
- It does **not** publish synthetic/demo data to the Event Bus. The corresponding UI panels (`docs/02-architecture/ui.md`) render their existing, real "no data yet" empty state — an empty Team Panel or Notification Panel is the correct and honest representation of "not configured," not a placeholder to be papered over with fabricated content. This mirrors `Command::SendSlackMessage`'s deliberate "not yet implemented" error (`crates/commands/src/lib.rs`) rather than faking a success.
