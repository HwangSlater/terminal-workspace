# Implementation Plan - Phase 3: Storage + CQRS Foundation

Phase 2 (Core Infrastructure) closed with `cargo check --workspace`, `clippy --all-targets -D warnings`, and `fmt --check` all clean. Its docs explicitly deferred two things to "Phase 3": Event retry/backoff + Dead Letter Queue (`docs/02-architecture/events.md`), and a real `CommandDispatcher` implementation (noted in `crates/app/tests/core_infra_test.rs`). Given a choice between Storage+CQRS, a first real Integration (Slack), and a TUI Docking shell, **Storage + CQRS** was selected — it keeps building the foundation layer before any UI or third-party adapter exists.

---

## User Review Required

> [!IMPORTANT]
> **Phase 3 Sub-system Implementations**:
> 1. **SQLite Storage**: Wire real persistence (via `rusqlite` bundled + `tokio-rusqlite`) behind the six already-frozen repository traits in `crates/domain`, plus a migration runner.
> 2. **CQRS Write Path**: `InMemoryCommandDispatcher` + `WorkspaceCommandHandler` routing the existing `Command` enum to repository mutations and, where a matching frozen `Event` variant exists, publishing resulting events.
> 3. **Event Reliability**: Exponential backoff retry + Dead Letter Queue in `EventDispatcher`, via a new additive `FailedEventRepository`.

**Explicitly out of scope**: the CQRS *read* path (`DashboardReadModel` + Projector, ADR-0007) — no consumer exists yet (no TUI); real Slack/GitHub/etc. adapters; any identity/auth system (a constant local-user placeholder is used for `SetPresence`).

**Architecture Freeze v1 compliance**: no changes to `NotificationRepository`/`PresenceRepository`/`WorkspaceRepository`/`SettingsRepository`/`PluginRepository` signatures or the `Event` enum. All new contracts (`FailedEventRepository`) are additive.

---

## Proposed Changes

### Crates Implementation

#### [MODIFY] `crates/storage/src/lib.rs` (+ new `crates/storage/migrations/*.sql`)
- Replace stub bodies in `SqliteStorageBackend` with real SQL against a `tokio_rusqlite::Connection`.
- `SqliteStorageBackend::open(path)` runs pending migrations (`0001_initial.sql`, `0002_add_failed_events.sql`) via `PRAGMA user_version` gating, transactionally.
- Implement `FailedEventRepository` for the new `failed_events` table.

#### [MODIFY] `crates/domain/src/lib.rs`
- Add `FailedEventRecord` + `FailedEventRepository` (additive; no existing trait touched).

#### [MODIFY] `crates/commands/src/lib.rs`
- Add `WorkspaceCommandHandler` (implements `CommandHandler<Command>`) and `InMemoryCommandDispatcher` (implements `CommandDispatcher`).

#### [MODIFY] `crates/events/src/lib.rs`
- Add `EventDispatcher::with_dlq(repo)` builder; retry/backoff loop inside the existing per-handler spawned task; DLQ write on exhaustion.

#### [MODIFY] `crates/app/src/main.rs`
- Wire `SqliteStorageBackend::open`, `InMemoryCommandDispatcher`, and `EventDispatcher::with_dlq` together at startup.

---

## Verification Plan

- **Storage Tests**: Migration-runs-once and CRUD round-trip per repository, against a temp-file SQLite DB.
- **CQRS Tests**: `WorkspaceCommandHandler` behavior per `Command` variant, using hand-rolled mock repositories (no new mocking-framework dependency).
- **Event Reliability Test**: A handler that always fails ends up in the DLQ after 5 attempts, using `tokio::time::pause()` to avoid real wall-clock delay.
- **Vertical Slice**: Extend the Phase 2 test to prove `Command -> WorkspaceCommandHandler -> Storage (temp SQLite) -> Event -> EventDispatcher` end-to-end.
- `cargo check --workspace`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`. `cargo test` execution is blocked in the assistant's sandbox (native linker limitation carried over from Phase 2 — `rusqlite`'s `bundled` feature compiles a C library via the same broken linker path); the user should run `cargo test --workspace` locally to confirm.
