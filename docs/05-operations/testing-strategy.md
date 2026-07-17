# Comprehensive Testing Strategy

This document details the automated validation layers, performance benchmarks, and chaos tests implemented to verify platform stability.

---

## 1. Testing Hierarchy

```text
  [ Chaos & Reliability Testing ]  <- Network loss emulation, panic injections
                 │
                 ▼
     [ System / E2E Testing ]       <- Ratatui TestBackend layout assert scripts
                 │
                 ▼
  [ Integration / Repository ]      <- In-memory SQLite transaction validations
                 │
                 ▼
      [ Unit / Domain Mocking ]     <- cargo test with mockall interface asserts
```

---

## 2. Test Execution Protocols

### 1. Unit Testing
- **Target**: Pure domain logic in `core/src/domain`.
- **Constraint**: No networking, filesystem accesses, or SQLite connections allowed. Must run in $< 20\text{ms}$ per test.
- **Tooling**: Rust standard test module, `mockall` for repository traits.

### 2. Integration Testing
- **Target**: SQLite adapter queries (`sqlx` scripts) and Tokio channel event broadcasts.
- **Execution**: Database tests must run against an in-memory SQLite database (`sqlite::memory:`). Wrap each test inside an SQL transaction that rolls back on complete to ensure isolation.

### 3. TUI Visual Snapshot Testing
- **Target**: Presentational widgets drawing inside the TUI panels.
- **Execution**: Instantiate Ratatui's `TestBackend`. Pass a pre-configured `DashboardReadModel` state, execute rendering, and compare the virtual terminal buffer with expected character grid vectors (ASCII snapshots).

### 4. E2E & Chaos Testing
- **Target**: Complete system run loops.
- **Emulating Packet Loss**: We use mock servers (`wiremock`) configured with latency spikes, HTTP 503 drops, and rate limits to verify that the `IntegrationAdapter` reconnection state machines and dead-letter queues (DLQ) work as designed.
- **Panic Trapping**: Induce deliberate panics inside a mock WASM plugin to verify that the `PluginManager` isolates the trap, suspends the plugin, and keeps the TUI running.

---

## 3. Performance & Load Tests
- **Startup Benchmarking**: Checked using `criterion` micro-benchmarks to profile cold-starts and ensure frame render speeds remain $< 16\text{ms}$ (60 FPS rendering).
- **Leak Detection**: Integrate `valgrind` or `cargo-heap` during CI runs to check for memory leaks in the Wasmtime instance pooling loops.
