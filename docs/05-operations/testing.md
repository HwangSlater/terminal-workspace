# Test Strategy Specification

The Terminal Workspace requires test-driven development (TDD) validation across all Clean Architecture layers. This document outlines our testing layers, mocking tools, and validation rules.

---

## 1. Test Architecture Layers

```text
+-----------------------------------------------------------+
|                  Virtual Terminal Tests                   |  <- System / E2E (Ratatui Snapshot)
+-----------------------------------------------------------+
|             Database & Async Integration Tests            |  <- Application Integration (SQLite, Tokio Channels)
+-----------------------------------------------------------+
|              Domain Logic & WASM Boundary Tests           |  <- Unit Tests (Mockall, Wasmtime harness)
+-----------------------------------------------------------+
```

---

## 2. Unit Testing & Mocking

Domain-level components are tested in isolation using the native Rust test framework.

- **Interface Mocking (`mockall`)**:
  Infrastructure adapters (like SQLite databases and HTTP clients) are mocked using the `mockall` crate.
  ```rust
  #[cfg(test)]
  mockall::mock! {
      pub StorageService {}
      #[async_trait]
      impl StorageService for StorageService {
          async fn set_kv(&self, key: &str, value: &str) -> Result<(), StorageError>;
          async fn get_kv(&self, key: &str) -> Result<Option<String>, StorageError>;
      }
  }
  ```
- **Network API Mocking (`wiremock`)**:
  Integration clients (Slack client, GitHub client) are validated against a local mock HTTP server spawned inside the test using `wiremock`. This ensures correct HTTP header parsing, token transport, and JSON deserialization under various HTTP error codes (429 Rate Limited, 500 Server Error).

---

## 3. Integration Testing

Integration tests run inside the `tests/` directory of their respective crates.

- **Database Rollback Tests**:
  All database integration tests operate on an in-memory SQLite database (`sqlite::memory:`). A transaction is started before each test case and rolled back during the `tear_down` phase, ensuring test cases do not pollute one another.
- **Asynchronous Event Bus Tests**:
  Tokio's test scheduler (`#[tokio::test]`) is configured to test race conditions in Event Bus pub/sub channels. We utilize `tokio::time::pause()` to fast-forward timeouts and test exponential backoffs.

---

## 4. TUI Virtual Terminal Testing

Testing terminal interfaces is historically error-prone. The presentation layer uses a **Virtual Backend** snapshot approach:

- **Buffer Assertions**:
  Instead of rendering to the physical screen via stdout, Ratatui's `TestBackend` is instantiated. The UI draws to a virtual grid buffer of size $W \times H$.
  ```rust
  #[test]
  fn test_render_team_panel() {
      let backend = TestBackend::new(20, 5);
      let mut terminal = Terminal::new(backend).unwrap();
      let state = TuiState::default();
      
      terminal.draw(|f| {
          draw_team_panel(f, f.size(), &state);
      }).unwrap();
      
      // Assert visual coordinates directly
      terminal.backend().assert_buffer(&Buffer::with_lines(vec![
          "┌─Team─────────────┐",
          "│ • @alice (Active)│",
          "│ o @bob (Away)    │",
          "│                  │",
          "└──────────────────┘",
      ]));
  }
  ```

---

## 5. WASM Host-Guest FFI Testing

To test plugins without compiling real WASM binaries:
- The Host provides a Mock Linker inside Rust unit tests.
- We test guest lifecycle transitions by compiling a simple `mock_plugin.wasm` target and executing it via `wasmtime::Engine` to ensure CPU/memory quotas and permissions are successfully locked down.
