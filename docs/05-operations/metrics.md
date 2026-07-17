# System Telemetry & Metrics Specification

This document details the telemetry metrics recorded in-memory and compiled to log streams for system monitoring.

---

## 1. Monitored Metrics Catalog

The core workspace tracks the following performance metrics:

### 1. Plugin Performance
- **Plugin Load Latency (ms)**: Time taken by `wasmtime` to compile and initialize a plugin WASM module. Expected: $< 50\text{ms}$.
- **Plugin Instruction Count (Fuel)**: Tracked per-event dispatch to ensure plugins stay within CPU budget.
- **Plugin Memory Usage (MB)**: Checked via host memory manager. Limit: $64\text{MB}$ per plugin.

### 2. Event Bus Health
- **Notification Latency (ms)**: Time taken from Integration Event arrival to TUI ReadModel updating. Expected: $< 10\text{ms}$.
- **Dropped Events Count**: Tracks events dropped from Low Priority queues during peak loads.
- **Queue Backlog Size**: Number of pending events waiting in the MPSC channels.

### 3. Connection Diagnostics
- **Reconnect Count**: Number of times an adapter disconnected and retried.
- **API Request Latency (ms)**: Duration of outgoing HTTP requests to Slack/GitHub endpoints.

---

## 2. Telemetry Reporting & Storage
- **Memory Buffer**: Metrics are held in a circular sliding-window buffer (`ringbuf`) in the core runtime.
- **Logging Out**: Metrics are flushed to `metrics.log` every 5 minutes in a JSON format suitable for local plotting or offline monitoring:
```json
{
  "timestamp": 1781532300,
  "metrics": {
    "plugin.todo-tracker.mem_bytes": 12451800,
    "event_bus.latency_ms": 1.25,
    "slack.reconnect_count": 0
  }
}
```
