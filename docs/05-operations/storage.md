# Storage & Persistence Specification

The Terminal Workspace uses **[`redb`](https://github.com/cberner/redb)**, a pure-Rust embedded key-value store, for caching Slack messages, GitHub PR states, system logs, and notifications. This ensures offline access, ACID transactions, and data integrity — with no C compiler or native dependency required to build (`docs/06-development/decisions/0014-storage-engine-reconsideration.md`).

---

## 1. Table Design

The `redb` database file (`workspace.redb`) contains four tables. Each is a `redb::TableDefinition<&str, &[u8]>` — the value bytes are JSON, reusing the domain structs' existing `serde::Serialize`/`Deserialize` derives directly (no separate row-mapping layer, unlike a SQL column mapping).

### `notifications`
Stores aggregated messages and events from GitHub, Slack, Gmail, and Google Calendar.

- **Key**: `NotificationId` (UUID string).
- **Value**: JSON-encoded `domain::NotificationItem` (`id`, `source`, `title`, `body`, `timestamp_ms`, `priority`, `is_read`, `action_link` — see `docs/03-domain/domain-model.md`).
- **Access patterns**: `save` (upsert by key), `find_by_id` (point lookup), `fetch_unread` (full scan + filter on `is_read`, `mark_read` (read-modify-write by key). None of these are relational — see ADR-0014 for why a scan is an acceptable (and in fact already-implemented) replacement for a SQL `WHERE is_read = 0` query at this data volume (a local desktop tool's notification list, not a server-scale table).

### `team_presence`
Caches team member presence, minimizing API calls to Slack.

- **Key**: `UserId`.
- **Value**: JSON-encoded `domain::MemberPresence` (`user_id`, `display_name`, `status`, `custom_status_text`, `last_updated_ms`).
- **Access patterns**: `save_presence` (upsert), `fetch_all` (full scan — the whole point of this table is "show me everyone's status," so a full scan is the actual desired query, not a fallback).

### `key_value_store`
Acts as a simple local storage dictionary for configurations, access tokens (when not in keytar), and cache keys. Four of the six `domain-model.md` repository contracts share this one table via key-prefix namespacing — `SettingsRepository`, `CacheRepository`, `WorkspaceRepository`, and `PluginRepository` don't need dedicated tables of their own:

| Repository | Key prefix | Notes |
| :--- | :--- | :--- |
| `SettingsRepository` | `setting:<key>` | `expires_at` unused (`None`) |
| `CacheRepository` | `cache:<key>` | `expires_at` enforced; expired entries read as `None` and are lazily deleted |
| `WorkspaceRepository` | `workspace:layout` (fixed single key) | `expires_at` unused (`None`) |
| `PluginRepository` | `plugin:<plugin_id>` | value's `value` field is the manifest JSON; `get_active_plugins` scans the prefix |

- **Value**: JSON-encoded `{ value: String, expires_at: Option<u64> }` (Unix seconds) — this small wrapper struct is the one place this design adds a field beyond what SQL's `expires_at` column did directly, since a KV value can't have "extra columns" the way a SQL row can.

### `failed_events` (Dead Letter Queue)
Caches events that failed processing by consumers for offline diagnostics. Written to by `crates/events::EventDispatcher` (not a separate service) once its exponential-backoff retry loop exhausts its attempts — see `docs/02-architecture/events.md` §"Retry Policy & Backoff" and `docs/06-development/decisions/0003-event-bus.md`'s Phase 3 amendment.

- **Key**: record UUID string.
- **Value**: JSON-encoded `domain::FailedEventRecord` (`id`, `event_type`, `producer`, `payload_json`, `error_message`, `retry_count`, `failed_at_ms`).
- **Access patterns**: `save_failed` (insert), `list_failed` (full scan, ordered by `failed_at_ms` in application code after fetch — DLQ contents are inspected rarely, ordering cost at read time is fine).

---

## 2. Directory Layout & Paths

Data is placed in user-standard OS directories following the XDG Base Directory Specification:

- **Linux / macOS**:
  - Config: `~/.config/terminal-workspace/`
  - DB & Logs: `~/.local/share/terminal-workspace/`
- **Windows**:
  - Config: `%USERPROFILE%\.config\terminal-workspace\` (matches `crates/config`'s already-implemented path; see `docs/05-operations/configuration.md`)
  - DB & Logs: `%USERPROFILE%\AppData\Local\terminal-workspace\`

```text
AppData/Local/terminal-workspace/
├── workspace.redb           # Active redb database file
├── logs/
│   ├── app.log              # General debug / trace logging
│   └── error.log            # Critical warnings & panic reports
└── plugins/
    ├── manifests/           # Downloaded plugin definitions
    └── bin/
        ├── plugin_a.wasm    # Compiled plugin WASM binary files
        └── plugin_b.wasm
```

---

## 3. Schema Evolution

`redb` tables are created on first write — there's no `CREATE TABLE IF NOT EXISTS` migration step to run at startup, unlike the SQL-based design this superseded (see `docs/05-operations/migration.md` for the full strategy). In short:

- **Additive changes** (a new `Option<T>` field on a stored struct) are free: `#[serde(default)]` on the new field means old JSON blobs written before the change simply deserialize with the default, no migration code needed.
- **Breaking changes** (renaming/removing/restructuring a field) need a one-off transform, gated on a version marker. Not implemented yet — nothing has needed it — but the mechanism is documented in `migration.md` so it's a known, deliberate extension point rather than something to improvise under pressure later.

---

## 4. In-Memory Cache

For UI performance (60 FPS rendering), reading from the storage layer on every frame is prohibited, regardless of how fast `redb` itself is.
- **TuiState Cache**: An in-memory, thread-safe cache (`std::sync::Arc<tokio::sync::RwLock<CachedData>>`) is populated at startup from `redb`.
- **Write-Through Caching**: When an update arrives (e.g., new Slack DM), it is written to `redb` asynchronously (via `tokio::task::spawn_blocking`, since `redb`'s API is synchronous), and the in-memory cache is updated immediately, triggering a UI refresh event.
