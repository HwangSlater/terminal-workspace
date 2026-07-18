# Bounded Contexts Specification

This document details the boundaries, context mappings, and interaction definitions of the Domain-Driven Design (DDD) Bounded Contexts.

> **Implementation Status**: this is a **conceptual DDD context map**, not a claim about crate boundaries — the actual workspace is organized by technical layer (`domain`, `commands`, `events`, `storage`, `integration`, `ui`, `plugin-host`/`plugin-sdk`, `ipc`, ...), not by these 8 contexts, so there's no 1:1 crate to check "isolation" against (relevant to `product-requirements.md` §4's "Full Bounded Context isolation" v1.0.0 line, which this project can't concretely verify against this document as written). Per-context reality:
> - **Workspace, Notification, Presence, Integration**: real, in effect — their responsibilities as described exist in the code (`crates/domain`, `crates/commands`, `crates/integration`), just not as separate crates.
> - **Plugin Context**: real as of Phase 14 (`step14.md`) — `crates/plugin-host`/`plugin-sdk`. "Interacts strictly through the Unified Registries" is not yet true, though: plugins don't register commands or UI panels yet (deliberately deferred, `step14.md`'s scope note).
> - **Scheduler Context, Assistant Context**: **not built** — `crates/scheduler`/`crates/assistant` are unwired stubs; see those crates' own domain docs (`docs/03-domain/scheduler.md`, `assistant.md`) for their own status notes.
> - **Task Context**: **does not exist at all** — no crate, no stub, nothing. `domain::IntegrationSource` does have `Jira`/`Gmail` variants reserved for future integrations, but every place that matches on them is an explicit no-op (`IntegrationSource::Jira => {}` in `crates/ui/src/lib.rs`), not a Task context implementation.

---

## 1. Context Map

The system is decomposed into 8 isolated Bounded Contexts:

```text
                  +─────────────────────+
                  │  Workspace Context  │
                  +──────────┬──────────+
                             │
       ┌─────────────────────┼─────────────────────┐
       ▼                     ▼                     ▼
+──────────────+      +──────────────+      +──────────────+
│ Notification │      │   Presence   │      │  Scheduler   │
│   Context    │      │   Context    │      │   Context    │
+──────┬───────+      +──────┬───────+      +──────┬───────+
       │                     │                     │
       └──────────────┬──────┴─────────────────────┘
                      ▼
              +──────────────+
              │ Integration  │
              │   Context    │
              +──────┬───────+
                     │
       ┌─────────────┴─────────────┐
       ▼                           ▼
+──────────────+           +──────────────+
│    Plugin    │           │  Assistant   │
│   Context    │           │  Context     │
+──────────────+           +──────────────+
```

---

## 2. Context Definitions

### 1. Workspace Context
- **Responsibility**: Manages active terminal session state, screen focus layouts, configuration loading, and active themes.
- **Upstream/Downstream**: Upstream to all UI context projections.

### 2. Notification Context
- **Responsibility**: Aggregates unread message entities, applies rules, deduplicates, and manages message priorities.
- **Upstream/Downstream**: Downstream to Integration Context; Upstream to Workspace Context.

### 3. Presence Context
- **Responsibility**: Tracks developer presence status (`Active`, `Away`, `Offline`).
- **Upstream/Downstream**: Downstream to Integration Context.

### 4. Integration Context
- **Responsibility**: Orchestrates external service connections (Slack, GitHub, Calendar). Translates raw payload DTOs to pure Domain Entities.
- **Upstream/Downstream**: Upstream to Notification, Presence, and Scheduler.

### 5. Plugin Context
- **Responsibility**: Loads, initializes, executes, and teardown WASM plugin modules inside secure boundaries.
- **Upstream/Downstream**: Interacts strictly through the Unified Registries.

### 6. Scheduler Context
- **Responsibility**: Coordinates timer events, Pomodoro clocks, meeting alerts, and local agenda logs.
- **Upstream/Downstream**: Upstream to Notification Context.

### 7. Assistant Context (AI)
- **Responsibility**: Interfaces with Large Language Models (LLM), maps prompt templates, executes local tools, and stores conversation memory.
- **Upstream/Downstream**: Upstream to Workspace Context.

### 8. Task Context
- **Responsibility**: Tracks local/Jira todo issues, states (`Todo`, `Doing`, `Done`), and assignee indices.
