# ADR 0004: Relational SQLite Storage Engine Selection

> **⚠️ Superseded by [ADR-0014](0014-storage-engine-reconsideration.md).** The engine choice below (SQLite via `rusqlite` bundled) is no longer what's implemented — `crates/storage` now uses `redb`, a pure-Rust embedded key-value store, because the C-compiler build requirement this ADR accepted turned out to be real, costly friction against a relational-query capability the codebase never actually used. Kept below for historical record; do not treat the "Decision" section as current.

## Context
The Workspace must aggregate and display historical developer events (Slack messages, calendar events, GitHub PRs, workflow logs) and remain highly functional offline. 

Storage requirements:
- Complex relational query support (e.g., "Find all unread Slack DMs associated with the active GitHub PR branches").
- Low runtime overhead and swift cold start.
- Concurrent read/write safety: background sync tasks update tables while the UI thread displays content.
- Simple deployment: zero-setup local database.

---

## Decision
We select **SQLite** as the primary relational database storage engine. To maintain async-first architectures in Rust, we will use **`sqlx`** (using the sqlite driver) or **`tokio-rusqlite`** to handle database queries on a separate thread pool without blocking the main Tokio executor.

> **Amendment (Phase 3 Implementation)**: We chose **`rusqlite` (with the `bundled` feature) wrapped by `tokio-rusqlite`**, not `sqlx`. `bundled` statically compiles the SQLite C amalgamation into the binary — zero system dynamic-library dependency, satisfying `product-requirements.md` §2 "OS Independence" directly. `tokio-rusqlite` spawns one dedicated background thread owning the single `rusqlite::Connection` and exposes an async `.call(|conn| ...)` API, matching this ADR's "separate thread pool" intent without `sqlx`'s heavier async-driver surface or its compile-time query verification (which needs a live DB or an offline query cache at build time — unnecessary friction for the small, fixed schema in `docs/05-operations/storage.md`). Revisit if the schema grows complex enough to want compile-time-checked dynamic queries.

---

## Alternatives Considered

### 1. Key-Value Store (e.g., `sled`)
- *Pros*: Pure Rust implementation, extremely fast key-value lookup, very small dependency footprint.
- *Cons*: Lacks native indexes, joining schemas, and query filtering. To perform complex relational queries, the Application layer would need to load huge collections into memory and implement custom map-reduce filter functions, violating high-cohesion principles.

### 2. JSON Files / Directory Tree (Local File Cache)
- *Pros*: Human-readable, easy to edit with standard text tools.
- *Cons*: Suffers from file lock contention, poor crash-safety (corruption during writes on power-off), and lacks indexing, leading to high CPU usage during startup.

---

## Consequences

- **Relational Capability**: Complex queries and status aggregations are offloaded to SQLite, keeping our application code clean and highly cohesive.
- **Async Interfacing**: SQLite is single-connection writer-bound. We configure `sqlx` connection pools or `tokio-rusqlite` worker pools to wrap all DB statements in async futures, preventing disk IO from blocking the TUI thread.
- **Migration Pipeline**: We enforce structured SQLite database schema migrations on start (as detailed in `docs/05-operations/storage.md`).
- **Disk Overhead**: A lightweight SQLite database footprint is minimal (~수 MBs), satisfying our vision of minimal resource usage.
