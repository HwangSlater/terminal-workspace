# Event System Specification

This document details the design of the asynchronous **Event Bus** and the structural schema of events flowing through the Terminal Workspace.

> **Implementation Status (Phase 3, amended Phase 9)**: The `EventBus` ⇄ `EventDispatcher` split (Phase 2) and the Retry Policy & Backoff / Dead Letter Queue below (Phase 3, now that `crates/storage` backs `failed_events`) are implemented in `crates/events`. The multi-priority (`High`/`Medium`/`Low`) channel routing described later in this document is **still not implemented** — it's a larger EventBus-transport restructuring with no consumer yet (no TUI overlay, no plugin runtime), so it stays deferred until a phase that needs it. Until then, all events flow through one in-process broadcast channel, each independently retried/DLQ'd per handler. **The actual `enum Event` shipped is a strongly-typed Rust enum with one variant per event kind** (`SlackMessageReceived`, `SlackPresenceChanged`, `GitHubPRCreated`, `CalendarReminderTriggered`, `SystemAlert`, `PluginCustomEvent`, and — as of Phase 9, ADR-0016 — `IntegrationStatusChanged`), not the generic `{ id, timestamp, priority, producer, payload }` envelope sketched in "Core Interface" below; that shape was never built. See `crates/events/src/lib.rs` for the current, authoritative variant list rather than this document's payload-schema examples, which predate the real implementation.

## Event Bus vs. Event Dispatcher

The Event Bus's responsibility is deliberately kept narrow: **publish and subscribe only**. It does not know about handlers, retries, or routing rules.

```text
Event
   │
   ▼
EventBus (publish / subscribe)
   │
   ▼
EventDispatcher
   │
   ├── Notification Handler
   ├── UI Handler
   └── Plugin Handler
```

- **`EventBus`** (`crates/events`): wraps a `tokio::sync::broadcast::Sender<Event>`. `publish` sends, `subscribe` hands back a raw `Receiver<Event>`. It has no knowledge of `EventHandler`.
- **`EventDispatcher`** (`crates/events`): owns the list of registered `EventHandler`s, subscribes to an `EventBus`, and fans each received event out to every handler on its own spawned task.

This separation means the transport (`EventBus`) can later be swapped for an IPC or remote implementation without touching `EventDispatcher` or any registered handler — the dispatcher only depends on the `EventBus` trait, not on `InProcessEventBus`.

## Event Bus Design

The Event Bus acts as a central broker. In Rust, it is represented as a trait in the Domain layer, with its concrete implementation in the Infrastructure layer utilizing `tokio::sync::broadcast` and `tokio::sync::mpsc`.

### Core Interface (Domain Layer)

```rust
use async_trait::async_trait;
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EventPriority {
    High,    // Immediate dispatch, blocks lower priority if necessary (e.g., Calendar Reminder, Keyboard ESC)
    Medium,  // Standard user interaction events (e.g., Slack Message Received, PR Created)
    Low,     // Background indexing, offline caching updates
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: String,              // UUID v4
    pub timestamp: u64,          // Unix Timestamp in ms
    pub priority: EventPriority,
    pub producer: String,        // Sender ID (e.g., "service.slack", "ui.input")
    pub payload: EventPayload,
}

#[async_trait]
pub trait EventBus: Send + Sync {
    async fn publish(&self, event: Event) -> Result<(), EventError>;
    async fn subscribe(&self, topic: &str) -> Result<tokio::sync::broadcast::Receiver<Event>, EventError>;
}
```

---

## Event Payloads (Schema Definitions)

All events are serialized to JSON before being recorded or dispatched to WASM plugins.

### 1. Slack Messages
- **Event Type**: `SlackMessageReceived`
- **Schema**:
```json
{
  "channel_id": "C123456",
  "channel_name": "general",
  "sender_id": "U78910",
  "sender_name": "Alice",
  "text": "Hello Team!",
  "thread_ts": "1625097600.000200"
}
```

### 2. GitHub PR Activity
- **Event Type**: `GitHubPRCreated`
- **Schema**:
```json
{
  "pr_number": 42,
  "repository": "google/terminal-workspace",
  "title": "feat: integrate wasm runtime",
  "author": "octocat",
  "state": "open",
  "html_url": "https://github.com/google/terminal-workspace/pull/42"
}
```

### 3. Calendar Reminders
- **Event Type**: `CalendarReminder`
- **Schema**:
```json
{
  "event_id": "cal_9988",
  "title": "Sprint Planning Meeting",
  "start_time": 1781532000,
  "duration_minutes": 60,
  "attendees": ["alice@gmail.com", "bob@gmail.com"]
}
```

---

## Priority and Routing *(Planned — Phase 3)*

The system will maintain three internal broadcast channels corresponding to each `EventPriority`.
1. **High Priority Channel**: Inserts notifications into a TUI immediate overlay panel. High-priority handlers run with priority thread scheduling.
2. **Medium Priority Channel**: Dispatched to the active UI widget and active plugins.
3. **Low Priority Channel**: Polled during idle times. Storage synchronization tasks use this queue.

```text
               +---------------------------------------+
               |               Event Bus               |
               +---------------------------------------+
                                   |
           +-----------------------+-----------------------+
           |                       |                       |
           v                       v                       v
     [High Queue]            [Medium Queue]           [Low Queue]
           |                       |                       |
           v                       v                       v
    UI Notification         Plugin Dispatch         SQLite Database
    Overlay / Alerts        & UI Active Widget      Sync & Log Archive
```

---

## Retry Policy & Backoff

To handle intermittent network failures in third-party integrations (e.g., Slack rate limits, GitHub API timeouts), the Event Bus uses an **Exponential Backoff Retry Policy** for event consumer delivery.

- **Initial Delay**: 1 second
- **Multiplier**: 2.0x (1s -> 2s -> 4s -> 8s -> 16s)
- **Maximum Delay**: 60 seconds
- **Max Retries**: 5
- **Dead Letter Queue (DLQ)**: If an event fails to process after 5 retries, it is logged and written to the local SQLite `failed_events` table (see `docs/05-operations/storage.md`) for auditing.

**Implementation (Phase 3)**: `EventDispatcher::with_dlq(repo: Arc<dyn FailedEventRepository>)` opts a dispatcher into this behavior; the retry loop runs inside the per-handler task already spawned for fan-out (see "Event Bus vs. Event Dispatcher" above), so one slow/failing handler's retries never block delivery to other handlers. Without a configured DLQ repository, a dispatcher falls back to Phase 2 behavior (log and drop after the immediate failure, no retries) — this keeps `EventDispatcher::new()` callers unaffected. The "triggers a low-priority UI warning" consequence noted in the original design is deferred until a TUI consumer exists to receive it.
