# Error Catalog Specification

This document catalogues all system errors, recovery mechanisms, and recovery policies.

> **Implementation Status**: this whole page is a **design sketch that was never built as specified** — the real error handling took a different, simpler shape, and some of the recovery protocols below are partially real under different names while others don't exist at all.
>
> - **No `ErrorCode` enum, no `ERR_*` class codes anywhere.** The real error type is `common::WorkspaceError` (`crates/common/src/lib.rs`) — six `String`-payload variants (`Storage`, `Integration`, `Plugin`, `Configuration`, `Security`, `Internal`), not a catalog of specific named codes. There is no `error.log` file; errors surface through the unified `tracing` logging pipeline (`docs/05-operations/logging.md`).
> - **Storage is `redb`, not SQL** (ADR-0014, superseding whatever this page assumed) — "DB rebuild" on lock/corruption isn't a real trigger; `redb`'s failure modes and this project's actual response to them aren't documented here at all yet.
> - **Rate limiting is partially real, under different names**: Slack/GitHub adapters do recognize `429`/rate-limit headers and back off via a shared consecutive-failure state machine (`crates/integration/src/polling.rs`, `step9.md`/`step10.md`) — but there's no literal `INTEGRATION_RATE_LIMITED` code, no `Retry-After`-driven 30-second-start exponential backoff as specifically described, and the TUI shows this as the existing `Reconnecting` connection status (yellow "재연결 중...", `crates/ui/src/render.rs`), not a `[Throttled]` indicator.
> - **Plugin trap handling is real, but simpler than described** (Phase 14, `step14.md`): `crates/plugin-host/src/lib.rs` does catch a WASM trap (fuel exhaustion, OOM, guest panic), log it via `tracing` (with the guest's own wasm backtrace, not a fixed `PLUGIN_PANIC`/`PLUGIN_CPU_LIMIT_EXCEEDED` code), and drop the plugin instance from its loaded set (the `Suspended` state exists in effect, if not by that literal name). What's **not** real: no `CommandRegistry` removal step (plugins don't register commands yet — `step14.md`'s deliberately narrow scope), no `SystemAlert` event published on a plugin trap, and no `/plugin reload <id>` command (no plugin-facing commands exist at all yet). A trapped plugin stays unloaded until the next full restart.
> - **Auth failures**: real (a missing/invalid Slack/GitHub token surfaces as a `WorkspaceError::Integration`, and the in-app setup overlays — `Ctrl+S`/`Ctrl+G` — are exactly the "prompt setup modal" this page describes), but again not through a named `SLACK_AUTH_FAILED`/`GITHUB_AUTH_FAILED` code.

---

## 1. System Error Classifications

Errors are categorized into four classes:

| Class Code | Category | Severity | Action Policy |
| :--- | :--- | :--- | :--- |
| `ERR_AUTH_*` | Authentication Errors | High | Prompt setup modal, clear cached session token. |
| `ERR_NET_*` | Network Connection | Medium | Trigger reconnection backoff loop. |
| `ERR_PLUG_*` | Plugin Failure | Medium | Catch WASM trap, suspend plugin, log crash metrics. |
| `ERR_STOR_*` | Storage & SQL Issues | High | If database lock/corruption occurs, trigger DB rebuild. |

---

## 2. Catalog of Errors

```rust
pub enum ErrorCode {
    // Auth class
    SLACK_AUTH_FAILED,          // Auth Token is expired or revoked.
    GITHUB_AUTH_FAILED,         // GitHub Personal Access Token invalid.
    
    // Network class
    INTEGRATION_RATE_LIMITED,   // API returned HTTP 429.
    CONNECTION_LOST,            // WebSocket stream dropped.
    
    // Plugin class
    PLUGIN_PANIC,               // Guest code panicked.
    PLUGIN_OUT_OF_MEM,          // Wasm module exceeded 64MB allocator block.
    PLUGIN_CPU_LIMIT_EXCEEDED,  // Wasm ran out of fuel instructions.
    
    // Storage class
    DB_MIGRATION_FAILED,        // DB schema setup failed during initialization (Fatal).
    DB_IO_ERROR,                // Disk full or read block failure.
}
```

---

## 3. Recovery Protocols

### 1. Integration Rate Limits (`INTEGRATION_RATE_LIMITED`)
- **Protocol**: Inspect `Retry-After` header. If absent, set exponential backoff pause starting at 30 seconds.
- **TUI Indication**: The status bar integration dot turns yellow, showing `[Throttled]`.

### 2. Plugin Crash Recovery (`PLUGIN_PANIC`)
- **Protocol**:
  1. The host catches the Wasm Panic trap.
  2. The host logs the stack trace to `error.log`.
  3. The `PluginManager` sets plugin state to `Suspended`.
  4. The host removes the plugin's commands from `CommandRegistry` to prevent further dispatcher routing.
  5. The host sends a `SystemAlert` notification event.
  6. The plugin is NOT restarted automatically. The user must manually issue `/plugin reload <id>` to retry.
