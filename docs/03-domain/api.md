# Internal API Specification

This document details the interface boundaries (Rust Traits) for the core domain services of the Terminal Workspace. These APIs enable high cohesion and low coupling by hiding implementation details (like HTTP clients or SQLite connections) behind abstractions.

---

## 1. Notification & Status Domain Interfaces

### `NotificationService`
Handles global notification aggregation and state synchronization.

```rust
use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct Notification {
    pub id: String,
    pub source: String, // "Slack", "GitHub", "Calendar"
    pub title: String,
    pub message: String,
    pub timestamp: u64,
    pub read: bool,
    pub action_url: Option<String>,
}

#[async_trait]
pub trait NotificationService: Send + Sync {
    async fn fetch_unread(&self) -> Result<Vec<Notification>, ServiceError>;
    async fn mark_as_read(&self, id: &str) -> Result<(), ServiceError>;
    async fn clear_all(&self) -> Result<(), ServiceError>;
}
```

---

## 2. Integration Core Services

These interfaces govern communication with external services. Their implementations live in the infrastructure layer.

### `SlackService`
Handles Slack interaction, syncing presences, and sending/receiving messages.

```rust
#[async_trait]
pub trait SlackService: Send + Sync {
    async fn send_message(&self, channel_id: &str, text: &str, thread_ts: Option<&str>) -> Result<(), ServiceError>;
    async fn set_user_presence(&self, presence: UserPresence) -> Result<(), ServiceError>;
    async fn fetch_channels(&self) -> Result<Vec<SlackChannel>, ServiceError>;
    async fn fetch_users(&self) -> Result<Vec<SlackUser>, ServiceError>;
}
```

### `GitHubService`
Handles pulling repository data, issue/PR management, and reviews.

```rust
#[async_trait]
pub trait GitHubService: Send + Sync {
    async fn fetch_pull_requests(&self, repo: &str) -> Result<Vec<PullRequest>, ServiceError>;
    async fn approve_pull_request(&self, repo: &str, pr_number: u32, body: &str) -> Result<(), ServiceError>;
    async fn fetch_workflow_runs(&self, repo: &str) -> Result<Vec<WorkflowRun>, ServiceError>;
}
```

---

## 3. Storage Layer Interfaces

Handles data persistence. Hides SQLite dependencies from the Domain layer.

```rust
#[async_trait]
pub trait StorageService: Send + Sync {
    // Key-Value style metadata
    async fn set_kv(&self, key: &str, value: &str) -> Result<(), StorageError>;
    async fn get_kv(&self, key: &str) -> Result<Option<String>, StorageError>;
    
    // Structured caching
    async fn cache_notifications(&self, notifications: &[Notification]) -> Result<(), StorageError>;
    async fn get_cached_notifications(&self) -> Result<Vec<Notification>, StorageError>;
    
    // Transaction support
    async fn run_in_transaction<F, T>(&self, f: F) -> Result<T, StorageError>
    where
        F: FnOnce() -> Result<T, StorageError> + Send;
}
```

---

## 4. Command Dispatcher & Async Queries

The UI communicates with the core services using an asynchronous **Command Dispatcher**. Instead of calling services directly (which would block the UI thread during network queries), the UI dispatches a Command, and the response is sent back asynchronously via an MPSC (Multi-Producer, Single-Consumer) response channel.

```rust
pub enum Command {
    SendSlackMessage { channel_id: String, text: String },
    ApprovePR { repo: String, pr_number: u32, comment: String },
    SyncCalendar,
}

pub enum CommandResponse {
    Success,
    Failure(String),
}

pub struct CommandEnvelope {
    pub id: String, // Trace ID
    pub command: Command,
    pub reply_to: tokio::sync::oneshot::Sender<CommandResponse>,
}

pub struct CommandDispatcher {
    sender: tokio::sync::mpsc::Sender<CommandEnvelope>,
}

impl CommandDispatcher {
    pub async fn dispatch(&self, command: Command) -> Result<CommandResponse, ServiceError> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let envelope = CommandEnvelope {
            id: uuid::Uuid::new_v4().to_string(),
            command,
            reply_to: tx,
        };
        self.sender.send(envelope).await.map_err(|_| ServiceError::DispatcherFailed)?;
        rx.await.map_err(|_| ServiceError::ResponseFailed)
    }
}
```
- This design ensures the Presentation layer (TUI) is decoupled from Application executors, maintaining high cohesion within the TUI rendering engine.
