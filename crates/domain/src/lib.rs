//! Domain Context Entities and Repository contracts.

use async_trait::async_trait;
use common::Result;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Notification Id value object.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NotificationId(pub Uuid);

/// Integrations source enumeration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IntegrationSource {
    /// Slack messaging adapter.
    Slack,
    /// GitHub development updates.
    GitHub,
    /// Google Calendar scheduling items.
    Calendar,
    /// Gmail system triggers.
    Gmail,
    /// Jira project management tickets.
    Jira,
}

/// Notification Priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PriorityLevel {
    /// High priority alert.
    High,
    /// Standard alert.
    Medium,
    /// Background sync or cache alert.
    Low,
}

/// Notification item entity representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationItem {
    /// Unique identifier.
    pub id: NotificationId,
    /// Provider source.
    pub source: IntegrationSource,
    /// Alert summary.
    pub title: String,
    /// Extended Markdown/Text description.
    pub body: String,
    /// Triggered timestamp.
    pub timestamp_ms: u64,
    /// Priority level.
    pub priority: PriorityLevel,
    /// Active state flag.
    pub is_read: bool,
    /// Redirect link.
    pub action_link: Option<String>,
}

/// Team member user ID representation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UserId(pub String);

/// Presence status definitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PresenceStatus {
    /// Active on screen.
    Active,
    /// Away from keyboard.
    Away,
    /// Offline.
    Offline,
    /// In a calendar meeting slot.
    Meeting,
    /// Out for lunch.
    Lunch,
}

/// Team presence aggregate root.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberPresence {
    /// Identifier.
    pub user_id: UserId,
    /// Active username.
    pub display_name: String,
    /// Status.
    pub status: PresenceStatus,
    /// Custom Slack/Workspace alert status text.
    pub custom_status_text: Option<String>,
    /// Update timestamp.
    pub last_updated_ms: u64,
}

/// Repository contract managing notifications caching.
#[async_trait]
pub trait NotificationRepository: Send + Sync {
    /// Insert or update a notification.
    async fn save(&self, item: &NotificationItem) -> Result<()>;
    /// Retrieve notification by its ID.
    async fn find_by_id(&self, id: &NotificationId) -> Result<Option<NotificationItem>>;
    /// Get unread notification list.
    async fn fetch_unread(&self) -> Result<Vec<NotificationItem>>;
    /// Set reading status to true.
    async fn mark_read(&self, id: &NotificationId) -> Result<()>;
}

/// Repository contract managing presence updates.
#[async_trait]
pub trait PresenceRepository: Send + Sync {
    /// Persist member status.
    async fn save_presence(&self, presence: &MemberPresence) -> Result<()>;
    /// Get all members status.
    async fn fetch_all(&self) -> Result<Vec<MemberPresence>>;
}

/// Repository contract managing Workspace layout metadata states.
#[async_trait]
pub trait WorkspaceRepository: Send + Sync {
    /// Persist serialized workspace layout.
    async fn save_layout(&self, layout_json: &str) -> Result<()>;
    /// Fetch serialized layout if existing.
    async fn load_layout(&self) -> Result<Option<String>>;
}

/// Repository contract managing settings configurations key-values.
#[async_trait]
pub trait SettingsRepository: Send + Sync {
    /// Get metadata config value.
    async fn get_value(&self, key: &str) -> Result<Option<String>>;
    /// Set metadata config value.
    async fn set_value(&self, key: &str, value: &str) -> Result<()>;
}

/// Repository contract registering active plugins inside storage.
#[async_trait]
pub trait PluginRepository: Send + Sync {
    /// Record active plugin manifest status.
    async fn save_plugin_manifest(&self, plugin_id: &str, manifest_json: &str) -> Result<()>;
    /// Fetch all enabled plugin manifests.
    async fn get_active_plugins(&self) -> Result<Vec<(String, String)>>;
}

/// Repository contract cache metadata.
#[async_trait]
pub trait CacheRepository: Send + Sync {
    /// Cache temporary state value block.
    async fn set_cache(&self, key: &str, value: &str, ttl_secs: u64) -> Result<()>;
    /// Fetch temporary state value block if unexpired.
    async fn get_cache(&self, key: &str) -> Result<Option<String>>;
}

/// A single Dead Letter Queue entry: an `Event` that exhausted its
/// `EventDispatcher` retry attempts. See `docs/02-architecture/events.md` "Retry Policy &
/// Backoff" and `docs/05-operations/storage.md`'s `failed_events` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailedEventRecord {
    /// Unique identifier for this failure record.
    pub id: Uuid,
    /// Debug-formatted `Event` variant name (e.g. `"SlackMessageReceived"`).
    pub event_type: String,
    /// Handler identifier that failed to process the event.
    pub producer: String,
    /// Serialized event payload (JSON), for offline diagnostics/replay.
    pub payload_json: String,
    /// Final error message from the last retry attempt.
    pub error_message: String,
    /// Number of retry attempts made before giving up.
    pub retry_count: u32,
    /// Timestamp (ms since epoch) of the final failed attempt.
    pub failed_at_ms: u64,
}

/// Repository contract backing the Dead Letter Queue. Added in Phase 3;
/// additive only, does not modify any Architecture-Freeze-v1-locked contract.
#[async_trait]
pub trait FailedEventRepository: Send + Sync {
    /// Persist a failed event record for offline diagnostics.
    async fn save_failed(&self, record: &FailedEventRecord) -> Result<()>;
    /// Retrieve all recorded failures.
    async fn list_failed(&self) -> Result<Vec<FailedEventRecord>>;
}
