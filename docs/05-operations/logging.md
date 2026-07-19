# Logging Standards Specification

This document defines the logging categories, span contexts, trace propagation rules, and correlation ID formats used inside the Terminal Workspace.

> **Implementation Status (amended `step35.md`)**: §1's destination table below was written up front (Phase 2) and never actually implemented as file destinations until `step35.md` — the real Phase 2 implementation wrote the formatted-text copy to stderr instead, which worked fine until the TUI (Phase 5+) started sharing that same physical terminal: any `WARN`/`ERROR` emitted mid-session (e.g. a background poller) tore straight through the rendered frame. `step35.md` finally routes that layer to a daily-rotating `app.log` file (`crates/logging::init_logger`, via `tracing-appender`) instead, closing the original gap rather than just working around the TUI-corruption symptom. The `ERROR` → dedicated `error.log` / "UI Dialog" split and `WARN` → "UI Info" dialog in the table are still **not** implemented as literally described — `WARN`/`INFO`/`ERROR` all currently land in the same `app.log`, and the closest thing to "UI Info" that exists is the TUI's own `Ctrl+c` log viewer (`step17.md`, `Ctrl+4` before `step38.md` dropped that numeric alias), which shows everything in the buffer regardless of level, not a level-specific popup.

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

**Implementation (Phase 2, amended `step35.md`)**: `crates/logging::RedactingWriter` wraps the `fmt` layer's output sink (`std::io::Write`) so every already-formatted event line is scanned and secret-shaped substrings (`xoxb-...`, `ghp_...`) are replaced with `[REDACTED_SECRET]` before reaching the underlying writer (a daily-rotating `app.log` file in production as of `step35.md`, was stderr before that; the test writer in tests). `init_logger` composes `fmt::layer().with_writer(RedactingMakeWriter::new(inner))` instead of writing to the destination directly — `RedactingMakeWriter` itself is generic over `inner` as of `step35.md` (previously hardcoded to stderr) so the same scrubbing applies regardless of destination. This closes a gap that predated Phase 2 — the requirement was already documented here and in `docs/04-extensions/security.md` §2 but had no implementation.
