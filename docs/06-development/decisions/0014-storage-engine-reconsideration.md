# ADR 0014: Storage Engine Reconsideration — SQLite (rusqlite) → redb

## Status

**Supersedes** the engine choice in [ADR-0004](0004-storage.md) (SQLite selection remains there for historical record; its "Decision" section no longer reflects the current implementation).

## Context

ADR-0004 selected SQLite via `rusqlite` with the `bundled` feature (statically compiling SQLite's C source into the binary) to avoid an OS-level dynamic `libsqlite3` dependency. That trade-off has a cost ADR-0004 didn't fully price in: **every build now requires a C compiler**, on every developer machine and CI runner, for every supported OS.

That cost stopped being theoretical during Phase 3 implementation and rollout. Getting a single contributor (this project's own architect, on a bare Windows machine with only `rustup` installed) from a fresh clone to a working `cargo check --workspace` took multiple hours: no C compiler was present at all, `winget install`-ing Visual Studio Build Tools left behind zombie processes that blocked a retry, and the eventual install had to be killed mid-way, still unresolved. This happened in the same working session where "Zero Setup" was formalized as a core product principle (`docs/01-product/vision.md`, `product-requirements.md` §2) — living through the "Contributor Experience: One-Time Setup" cost that principle explicitly carved out made clear it isn't actually cheap.

Re-examining the ADR-0004 justification against what's actually built: SQLite was chosen over a KV store specifically for **relational query capability** — the example given was "find all unread Slack DMs associated with active GitHub PR branches." Auditing `crates/storage/src/lib.rs` as implemented in Phase 3: all six repository implementations (`NotificationRepository`, `PresenceRepository`, `WorkspaceRepository`, `SettingsRepository`, `PluginRepository`, `FailedEventRepository`) are simple key-based upsert/get, or full-table scan-and-filter (`fetch_unread`, `fetch_all`, `list_failed`, `get_active_plugins`). **There is no JOIN, and no relational query, anywhere in the codebase.** The capability SQLite was chosen for is not something the current implementation or the near-term roadmap (`docs/01-product/roadmap.md` through v0.5) actually exercises.

## Decision

Switch the storage engine to **[`redb`](https://github.com/cberner/redb)**: a pure-Rust, ACID-compliant, embedded key-value store. No C compiler, no build script, no native dependency of any kind — `cargo build` works identically on Windows/macOS/Linux with nothing beyond a `rustup`-installed Rust toolchain.

This is not a narrower/cheaper substitute for SQLite's capability — it's a match for what's actually being used. The four conceptual "tables" from ADR-0004's schema (`notifications`, `team_presence`, `key_value_store`, `failed_events`) map directly onto four `redb::TableDefinition`s, each storing JSON-serialized domain structs (reusing their existing `serde::Serialize`/`Deserialize` derives — no new DTO layer). See `docs/05-operations/storage.md` for the concrete table/key design.

**Repository contracts are unaffected.** `NotificationRepository`, `PresenceRepository`, `WorkspaceRepository`, `SettingsRepository`, `PluginRepository` (frozen by Architecture Freeze v1) and `FailedEventRepository` (Phase 3) keep their exact signatures — this is a swap of what implements them (`SqliteStorageBackend` → `RedbStorageBackend`), not a change to the seam other code depends on.

## Alternatives Considered

### Keep SQLite, mitigate the toolchain cost with tooling
`scripts/setup.ps1`/`setup.sh` (written earlier this session) automate the C toolchain install. This reduces friction but doesn't remove it — it's still a multi-GB download, still capable of hanging (as observed), and still a real "why do I need Visual Studio to run a terminal notification tool" moment for a first-time contributor. Rejected: treats a symptom the actual code doesn't need to have.

### Dynamically link system SQLite instead of `bundled`
Removes the C-*compile* step but trades it for an OS-*provisioning* problem (`libsqlite3` isn't preinstalled on Windows at all, and versions vary across Linux distros) — ADR-0004 already rejected this for the same reason, and it wouldn't fix the Windows case that caused this reconsideration in the first place.

### A different embedded SQL engine (e.g. `sqlx` with `sqlite` bundled feature)
Same root problem — any embedded *SQL* engine bundled from source needs a C compiler; `sqlx`'s SQLite backend is `libsqlite3-sys` underneath, identical to `rusqlite`'s.

## Consequences

- **Contributor Experience simplifies dramatically**: `product-requirements.md` §2.2's "One-Time Setup" cost is now just "install Rust" — no separate toolchain step. `scripts/setup.*` are simplified from installers into plain verification scripts (`docs/06-development/platform-support.md`).
- **No relational queries, by design.** If a genuine need for cross-entity relational queries materializes later (not hypothesized — actually needed by a concrete feature), that's a real signal to revisit storage again; migrating off a KV store at that point is a smaller, better-informed change than continuing to carry today's C-toolchain cost against a need that hasn't shown up.
- **Schema evolution changes shape**: no more SQL `ALTER TABLE`/`PRAGMA user_version` migrations. Additive field changes are free (JSON + `#[serde(default)]`); breaking changes need a manual one-off transform gated on a version marker — see `docs/05-operations/migration.md`.
- **Full local verification is possible again**: because `redb` has no build script requiring a C compiler, `cargo check`/`clippy`/`fmt`/`test --workspace` can now actually run to completion in *any* environment with just `rustup` — including the assistant sandbox that could not verify `crates/storage` at all under the SQLite-based design.
