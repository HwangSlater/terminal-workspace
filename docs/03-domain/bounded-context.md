# Bounded Contexts Specification

This document details the boundaries, context mappings, and interaction definitions of the Domain-Driven Design (DDD) Bounded Contexts.

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
