# Domain Model Specification

This document defines the Aggregate Roots, Entities, Value Objects, and Repository interfaces for each Bounded Context.

---

## 1. Domain Entities & Value Objects

### Notification Context
- **Aggregate Root**: `NotificationGroup`
- **Entities**: `NotificationItem` (Value Object: `NotificationId`, `Source`, `Priority`)
```rust
pub struct NotificationItem {
    pub id: NotificationId,
    pub source: IntegrationSource,
    pub title: String,
    pub body: String,
    pub timestamp: EpochMs,
    pub priority: PriorityLevel,
    pub is_read: bool,
    pub action_link: Option<String>,
}
```

### Presence Context
- **Aggregate Root**: `TeamPresence`
- **Value Objects**: `UserId`, `PresenceStatus` (Active, Away, Offline, Meeting, Lunch)
```rust
pub struct MemberPresence {
    pub user_id: UserId,
    pub display_name: String,
    pub status: PresenceStatus,
    pub custom_status_text: Option<String>,
    pub last_updated: EpochMs,
}
```

### Scheduler Context
- **Aggregate Root**: `Agenda`
- **Entities**: `CalendarEvent`, `PomodoroTimer` (Value Object: `EventId`, `TimeRange`)

### Assistant Context (AI)
- **Aggregate Root**: `ChatSession`
- **Entities**: `ChatMessage` (Value Object: `Role`, `TokenCount`)

---

## 2. Granular Repository Contracts (Domain Interfaces)

We explicitly declare separate traits to adhere to the Single Responsibility Principle:

### `NotificationRepository`
```rust
use async_trait::async_trait;

#[async_trait]
pub trait NotificationRepository: Send + Sync {
    async fn save(&self, notification: &NotificationItem) -> Result<(), RepositoryError>;
    async fn find_by_id(&self, id: &NotificationId) -> Result<Option<NotificationItem>, RepositoryError>;
    async fn fetch_unread(&self) -> Result<Vec<NotificationItem>, RepositoryError>;
    async fn mark_read(&self, id: &NotificationId) -> Result<(), RepositoryError>;
}
```

### `PresenceRepository`
```rust
#[async_trait]
pub trait PresenceRepository: Send + Sync {
    async fn save_presence(&self, presence: &MemberPresence) -> Result<(), RepositoryError>;
    async fn fetch_all(&self) -> Result<Vec<MemberPresence>, RepositoryError>;
}
```

### `SettingsRepository`
```rust
#[async_trait]
pub trait SettingsRepository: Send + Sync {
    async fn get_value(&self, key: &str) -> Result<Option<String>, RepositoryError>;
    async fn set_value(&self, key: &str, value: &str) -> Result<(), RepositoryError>;
}
```

### `PluginRepository`
```rust
#[async_trait]
pub trait PluginRepository: Send + Sync {
    async fn register_plugin(&self, plugin_manifest: &PluginManifest) -> Result<(), RepositoryError>;
    async fn get_active_plugins(&self) -> Result<Vec<PluginManifest>, RepositoryError>;
}
```

### `WorkspaceRepository`
```rust
#[async_trait]
pub trait WorkspaceRepository: Send + Sync {
    async fn save_layout(&self, layout_json: &str) -> Result<(), RepositoryError>;
    async fn load_layout(&self) -> Result<Option<String>, RepositoryError>;
}
```

### `FailedEventRepository` *(Phase 3)*
Backs the `failed_events` Dead Letter Queue table (`docs/05-operations/storage.md`). Added as a new, additive contract — it does not modify any of the five repository contracts frozen by Architecture Freeze v1 (`docs/06-development/development.md` §3).
```rust
pub struct FailedEventRecord {
    pub id: Uuid,
    pub event_type: String,
    pub producer: String,
    pub payload_json: String,
    pub error_message: String,
    pub retry_count: u32,
    pub failed_at_ms: u64,
}

#[async_trait]
pub trait FailedEventRepository: Send + Sync {
    async fn save_failed(&self, record: &FailedEventRecord) -> Result<(), RepositoryError>;
    async fn list_failed(&self) -> Result<Vec<FailedEventRecord>, RepositoryError>;
}
```
