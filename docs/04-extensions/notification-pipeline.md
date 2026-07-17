# Notification Pipeline Specification

This document details the stages through which raw events are processed, formatted, prioritized, and rendered as user notifications.

---

## 1. Pipeline Stages Flow

```text
  [Integration Event]
          │
          ▼
   [Event Bus Core]
          │
          ▼
   [Pipeline Input]
          │
          ▼
    [Rule Engine] ──────> Evaluates custom filters (e.g., Working Hours, White-lists)
          │
          ▼
   [Deduplicator] ──────> Filters identical alerts arriving in short intervals
          │
          ▼
   [Rate Limiter] ──────> Throttles notifications to prevent alert fatigue
          │
          ▼
    [Read Model]  ──────> Persists state to SQLite and triggers TUI render
```

---

## 2. Notification Rule Engine

The Rule Engine evaluates incoming events against user-defined matching conditions. Rules are configured in TOML (or JSON) and loaded into memory at startup.

### Rule Schema Representation:
```toml
# Rule 1: Elevate Slack DMs from Manager
[[rules]]
id = "slack_manager_dm"
match = "event.source == 'slack' && event.payload.sender == '@manager'"
action = "set_priority('High')"

# Rule 2: Silence build notifications outside working hours
[[rules]]
id = "ignore_ci_after_hours"
match = "event.source == 'github' && event.type == 'BuildFinished' && !is_working_hours()"
action = "suppress()"
```

---

## 3. Deduplication & Rate Limiting

- **Deduplication**:
  Events matching identical signatures (e.g., 5 quick GitHub Actions failure events for the same commit) are collapsed.
  - **Deduplication Hash**: `sha256(source + type + title)`
  - **Window**: 10 seconds. If an identical hash is received within the window, the count is incremented on the existing notification rather than creating a new item.
- **Rate Limiting**:
  Maximum notifications pushed to TUI overlay per second = 3. Any excess events are silently queued and flushed incrementally to prevent visual clutter.
