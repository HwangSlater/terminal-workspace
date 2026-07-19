//! Integration Adapter abstractions. See `docs/04-extensions/integration-contract.md`.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use common::Result;
use domain::NotificationItem;
use events::EventBus;
use secrets::SecretProvider;
use std::sync::Arc;

pub mod calendar;
pub mod github;
pub(crate) mod polling;
pub mod slack;

pub use calendar::{CalendarAdapter, CalendarConfig};
pub use github::{GitHubAdapter, GitHubConfig};
pub use slack::{
    PickerChannel, PickerUser, SlackAdapter, SlackConfig, SlackMessenger, SlackPicker,
};

/// Narrow port for connecting an integration with a bearer-style credential
/// (token). Identical in shape across every adapter built so far (Slack's
/// Bot Token, GitHub's PAT) — see `step11.md` for why this, unlike the
/// selection/picker ports below, generalizes with no loss of information.
/// Replaces the earlier per-integration `SlackConnector`/`GitHubConnector`
/// traits, which had byte-for-byte identical signatures.
#[async_trait]
pub trait IntegrationConnector: Send + Sync {
    /// Persist `token` durably (via the adapter's configured
    /// `SecretWriter`), then stop any running poll loop and start a fresh
    /// one with it — safe to call whether this is the first connection or
    /// a reconnect with a replacement token.
    async fn connect(&self, event_bus: Arc<dyn EventBus>, token: String) -> Result<()>;
}

/// One selectable item returned by [`Picker`] — an `id`/`label` pair. The
/// picker overlay decides how to render/select it; this crate has no
/// opinion beyond "here's what's available."
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerItem {
    /// Adapter-specific identifier, persisted into config on selection.
    pub id: String,
    /// Display label.
    pub label: String,
}

/// Narrow read-only port for integrations with a single selectable list
/// (GitHub's repositories, a future Calendar's calendar ids, ...). Slack's
/// two independent lists (channels + users) don't fit this shape and keep
/// their own dedicated [`SlackPicker`] port instead of being forced into
/// it (`step11.md`). Deliberately not routed through `Command`/
/// `CommandHandler` — listing is a read, not a mutation (the CQRS
/// correction made in `step8.md`).
#[async_trait]
pub trait Picker: Send + Sync {
    /// Items the authenticated account can access.
    async fn list_items(&self) -> Result<Vec<PickerItem>>;
}

/// Calendar-specific management operations (`step25.md`) that don't fit
/// `IntegrationConnector`/`Picker`/`SelectionApplier`'s shapes — defined
/// here (not `crates/commands`) so `CalendarAdapter` can implement it
/// directly with no `crates/app` bridge type, the same reasoning
/// `IntegrationConnector`/`Picker` already get away without one for
/// (`step24.md`'s Implementation Notes: only traits defined in
/// `crates/commands` need a bridge, since `crates/commands` depends on
/// `crates/integration` and not the other way around).
#[async_trait]
pub trait CalendarManager: Send + Sync {
    /// Updates how many hours ahead the reminder poll looks, then restarts
    /// polling with the new value (`CalendarPoller` snapshots its config
    /// once at `start()` time rather than re-reading it live).
    async fn set_lookahead_hours(&self, event_bus: Arc<dyn EventBus>, hours: u64) -> Result<()>;

    /// Renames a connected calendar (`id`, per [`PickerItem::id`]) without
    /// touching its URL or restarting polling — a label is cosmetic
    /// (prefixed onto reminder titles), not something the poll loop's
    /// fetch/parse logic depends on.
    async fn rename(&self, id: String, new_label: String) -> Result<()>;

    /// A fresh, on-demand fetch of every connected calendar's occurrences
    /// in `[after, before)` — independent of the reminder poll loop's
    /// `lookahead_hours` window and its "fire once" dedup state entirely.
    /// Backs the month grid view (`Ctrl+M`): a whole month's worth of
    /// "which days have something on them" has nothing to do with what
    /// the near-term reminder mechanism has already surfaced.
    async fn events_in_range(
        &self,
        after: DateTime<Utc>,
        before: DateTime<Utc>,
    ) -> Result<Vec<NotificationItem>>;
}

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
