# Command Model (CQRS) Specification

This document details the CQRS design pattern implemented in the Terminal Workspace. State updates are strictly segregated into Write Commands and Read Projections.

> **Implementation Status**: the CQRS **pattern** in §1 (Command -> Dispatcher -> Handler -> Domain Event -> Event Bus -> Projector -> Read Model -> UI rerender) is real and matches `crates/commands`/`crates/events` closely — including §3's `CommandHandler<C>` trait, which the real `commands::CommandHandler<C>` matches almost exactly (`type Output` + a single `common::Result<Self::Output>`, vs. this page's separate `Response`/`Error` associated types). §2 and §4's **concrete contents are not real**, though: the actual `Command` enum (`crates/commands/src/lib.rs`) has entirely different variants — `SetPresence`, `SendSlackMessage`, `Connect { source, token }`, `ApplySlackSelection`, `ApplySelection { source, items }`, `MarkNotificationRead`, `SyncAllAdapters` — not `CreateCalendarEvent`/`CreateTask`/`SubmitAiQuery` (Calendar is read-only with no create-event command, per `step12.md`; there is no Task context or AI Assistant command at all — `crates/assistant` is an unwired stub, see `docs/03-domain/assistant.md`'s own status note). The real `DashboardReadModel` only has `unread_notifications: Vec<NotificationItem>` and `team_presence: Vec<MemberPresence>` — no `current_workspace_layout` or `assistant_chat_history` field exists.

---

## 1. CQRS Execution Path

```text
    [ TUI Input ]
          │
          ▼
   (Create Command)
          │
          ▼
  [Command Dispatcher] ──────> [Command Handler] (Write Model Mutation)
                                      │
                                      ▼
                             [Domain State Update]
                                      │
                                      ▼
                             [Domain Event Raised]
                                      │
                                      ▼
                             [Event Bus Router]
                                      │
                                      ▼
                            [Read Model Projector]
                                      │
                                      ▼
                            (Read Model Mutated)
                                      │
                                      ▼
                             [UI Rerender Trigger]
```

---

## 2. Command Definitions

All state mutations must be dispatched via a strongly typed Command:

```rust
pub enum Command {
    // Presence Context
    SetPresence { status: PresenceStatus, text: Option<String> },
    
    // Notification Context
    MarkNotificationRead { id: NotificationId },
    
    // Scheduler Context
    CreateCalendarEvent { title: String, time_range: TimeRange },
    
    // Task Context
    CreateTask { title: String, description: Option<String> },
    
    // AI Assistant Context
    SubmitAiQuery { query: String },
}
```

---

## 3. Command Handler Interface

Command handlers execute inside the Application Layer:

```rust
use async_trait::async_trait;

#[async_trait]
pub trait CommandHandler<C>: Send + Sync {
    type Response;
    type Error;
    
    async fn handle(&self, command: C) -> Result<Self::Response, Self::Error>;
}
```

---

## 4. Read Model Projections

The UI never queries aggregate roots directly. Instead, it reads optimized, in-memory **Projections**.

- **Read Model**:
```rust
pub struct DashboardReadModel {
    pub unread_notifications: Vec<NotificationSummary>,
    pub team_presence_list: Vec<TeamPresenceSummary>,
    pub current_workspace_layout: LayoutGrid,
    pub assistant_chat_history: Vec<ChatMessage>,
}
```
- **Projector Service**:
  Subscribes to Domain Events (e.g., `PresenceChanged`, `NotificationReceived`) and updates `DashboardReadModel` in-memory. Once updated, it issues a local `ReadModelUpdated` signal to the TUI thread.
- **Benefits**:
  - Zero DB query overhead during screen refreshes.
  - Highly cohesive UI rendering logic (TUI code strictly reads fields from the `DashboardReadModel`).
