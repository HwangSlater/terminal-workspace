# Error Catalog Specification

This document catalogues all system errors, recovery mechanisms, and recovery policies.

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
