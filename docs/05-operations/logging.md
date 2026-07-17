# Logging Standards Specification

This document defines the logging categories, span contexts, trace propagation rules, and correlation ID formats used inside the Terminal Workspace.

---

## 0. Span Hierarchy

Spans are defined up front (Phase 2) rather than added ad hoc later, so an OpenTelemetry exporter can attach to this hierarchy without restructuring call sites:

```text
Application
    ├── Command
    ├── Event
    ├── Integration
    └── Plugin
```

`crates/logging::spans` exposes one thin constructor per level, each wrapping `tracing::info_span!` and carrying the current `correlation_id` (from `TraceContext`) as a span field:

- `application_span()` — opened once at process startup; the root of every other span.
- `command_span(name: &str)` — opened when a `Command` (see `docs/02-architecture/command-model.md`) begins dispatch.
- `event_span(kind: &str)` — opened when an `Event` is published or handled (see `docs/02-architecture/events.md`).
- `integration_span(source: &str)` — opened around calls into a third-party adapter (Slack, GitHub, Gmail, Calendar, Jira).
- `plugin_span(plugin_id: &str)` — opened around a WASM guest invocation.

Call sites enter the span with `.in_scope(...)` or `#[tracing::instrument]`; the constructors only standardize naming and fields so every subsystem's logs are structurally comparable.

---

## 1. Logging Levels & Target Destinations

The logging framework is implemented using Rust's `tracing` crate. Logs are routed based on severity levels:

| Level | Destination | Target Use Case |
| :--- | :--- | :--- |
| `ERROR` | `error.log` / UI Dialog | Critical errors causing adapter drops, database lockouts, or WASM crashes. |
| `WARN` | `app.log` / UI Info | Non-fatal issues (e.g., connection timed out, retrying backoff, rate limits hit). |
| `INFO` | `app.log` | Administrative events: plugin loaded, token initialized, migration run. |
| `DEBUG` | `app.log` (Dev only) | Command dispatch traces, event routing logs. |
| `TRACE` | Disabled in prod | Frame rendering calculations, byte-level network buffers. |

---

## 2. Trace Propagation & Correlation ID

To trace an execution path across the asynchronous boundaries (from Slack API polling through the Event Bus and onto TUI projection), we propagate a **Correlation ID**:

- **Structure**: A UUID v4 string initialized on incoming events or command entry.
- **Trace Context**:
  ```rust
  #[derive(Clone, Debug)]
  pub struct TraceContext {
      pub correlation_id: String,
      pub span_id: String,
  }
  ```
- **Propagation**:
  Every dispatched `Command` or published `Event` must wrap a `TraceContext`. When logging, the adapter writes this correlation ID to the log entry:
  ```text
  [2026-07-17 16:05:08.123] [INFO] [corr_id: 9a8b7c6d...] [plugins] Loaded plugin pomodoro-timer
  ```

---

## 3. Log Scrubbing
As defined in `docs/04-extensions/security.md`, any payload containing keys matching regex structures like `xoxb-` (Slack) or `ghp_` (GitHub) must be intercepted at the tracing formatter layer and replaced with `[REDACTED_SECRET]`.

**Implementation (Phase 2)**: `crates/logging::RedactingWriter` wraps the `fmt` layer's output sink (`std::io::Write`) so every already-formatted event line is scanned and secret-shaped substrings (`xoxb-...`, `ghp_...`) are replaced with `[REDACTED_SECRET]` before reaching the underlying writer (stderr in production, the test writer in tests). `init_logger` composes `fmt::layer().with_writer(RedactingMakeWriter::default())` instead of writing to stderr directly. This closes a gap that predated Phase 2 — the requirement was already documented here and in `docs/04-extensions/security.md` §2 but had no implementation.
