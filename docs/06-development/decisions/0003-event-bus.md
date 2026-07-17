# ADR 0003: Asynchronous Event Bus Architecture Selection

## Context
The Workspace architecture requires decoupled, async-first interactions. Integrating Slack, GitHub, Gmail, and the TUI requires a design where services do not call each other directly. Instead, they communicate by publishing and subscribing to events. 

The Event Bus must satisfy:
- High-throughput, thread-safe asynchronous dispatch.
- Decoupled lifecycle: integrations can be added/removed without altering other systems.
- Zero local deployment overhead: the entire message broker must run in-process without requiring external database engines (like Redis or RabbitMQ).

---

## Decision
We select an in-process, lock-free **Pub/Sub Event Bus** leveraging **Tokio's Broadcast and MPSC Channels**. The Event Bus acts as an asynchronous mediator.

---

## Alternatives Considered

### 1. External Broker (Redis Pub/Sub)
- *Pros*: Extremely robust, handles persistence naturally, supports offloaded analysis.
- *Cons*: Heavy requirement. Demanding developers to install and run a Redis daemon locally to use a terminal workspace tool is a poor developer experience.

### 2. Rust Actor Model Frameworks (e.g., `actix` / `riker`)
- *Pros*: Provides native supervision trees, structured message loops, and strong mailbox isolation.
- *Cons*: Adds significant cognitive complexity, boilerplate, and macro overhead to the codebase, which impacts long-term maintainability for contributor plugins.

---

## Consequences

- **Performance**: In-memory Rust broadcast queues route events in sub-microsecond latency.
- **Backpressure Handling (Lagged Consumers)**: 
  Tokio's broadcast channel defaults to dropping old messages if a receiver falls too far behind. If a receiver lags (e.g., a slow plugin performing intensive disk IO), the bus returns `RecvError::Lagged(skipped_count)`.
  - **Remedy**: The Event Bus wrapper catches this error, queries the missed events from the local SQLite database, re-injects them to the specific lagging client queue, and throttles further notifications until caught up.
- **Strict Decoupling**: Since services communicate only through shared `Event` structures, we achieve clean separation of concerns matching clean architecture rules.

---

## Amendment (Phase 2 Implementation)

During Phase 2 review (see `step2_feedback.md`), we split the single "Event Bus" concept from this ADR into two collaborating types, without altering the `Event` enum contract frozen by Architecture Freeze v1:

- **`EventBus`**: publish/subscribe transport only (`crates/events`).
- **`EventDispatcher`**: owns handler registration and fan-out, subscribing to an `EventBus`.

This is additive clarification, not a reversal — `EventDispatcher` depends only on the `EventBus` trait, so the in-process broadcast transport described above can still be swapped for an IPC/remote transport later without changing dispatcher or handler code. The priority-channel routing consequence described above remains the target design but is not yet implemented; see `docs/02-architecture/events.md` for current phasing.

## Amendment (Phase 3: Retry / Backoff / DLQ)

The "Backpressure Handling (Lagged Consumers)" remedy above is implemented as retry/backoff, scoped to **handler delivery failures** (a handler's `.handle()` returning `Err`), not `RecvError::Lagged` specifically — lagged receivers still just log a warning and continue (re-querying missed events from storage, as originally described, is deferred; it requires an event replay/outbox design beyond this phase's scope).

`EventDispatcher::with_dlq(repo: Arc<dyn FailedEventRepository>)` (additive builder, `crates/events`) opts a dispatcher into: exponential backoff retry (1s→2s→4s→8s→16s, capped 60s, max 5 attempts, per `docs/02-architecture/events.md`) on handler failure, and persisting to the `failed_events` table (`docs/05-operations/storage.md`, `docs/03-domain/domain-model.md`'s new `FailedEventRepository`) if all retries are exhausted. `EventDispatcher::new()` without `with_dlq` keeps Phase 2 behavior unchanged (log-and-drop, no retries) — existing callers are unaffected.
