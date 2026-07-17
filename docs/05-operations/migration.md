# Storage Schema Evolution & Rollback Strategy

This document specifies how the storage layer (`redb`, see `docs/05-operations/storage.md` and ADR-0014) evolves its schema over time and recovers from corruption. It replaces an earlier SQL-migration-file-based design from when the storage engine was SQLite — there is no `crates/storage/migrations/` directory anymore; `redb` tables are created on first write, not via `CREATE TABLE` scripts.

---

## 1. Additive Changes (the common case)

Every stored value is a JSON blob (`serde_json`) of a domain struct. Adding a new `Option<T>` field with `#[serde(default)]`:

```rust
#[derive(Serialize, Deserialize)]
pub struct NotificationItem {
    // ...existing fields...
    #[serde(default)]
    pub thread_id: Option<String>,  // new field
}
```

...requires **no migration code at all**. Old JSON blobs written before the field existed simply deserialize with `None` for `thread_id`. This is the expected, low-ceremony path for the large majority of schema changes and is why `redb`'s per-record JSON encoding was chosen over rigid columns in the first place (ADR-0014).

---

## 2. Breaking Changes (rare — not yet needed)

A field rename, removal, or restructuring that JSON's additive-deserialization can't absorb needs an explicit one-off transform. The mechanism (not yet implemented, since nothing has required it):

1. A small `meta` table (`redb::TableDefinition<&str, &[u8]>`) stores a single `schema_version` key.
2. On startup, `RedbStorageBackend::open` reads `schema_version` (defaulting to the current baseline if absent — i.e. a fresh database is always "up to date").
3. If the stored version is older than the compiled `CURRENT_SCHEMA_VERSION`, a version-specific transform function runs once: open the affected table(s), read each JSON value with the *old* shape, write it back in the *new* shape, then bump `schema_version`.
4. All transform steps for one version bump run inside a single `redb` write transaction, so a failure partway through rolls back cleanly rather than leaving a half-migrated table.

This intentionally mirrors the old SQL `PRAGMA user_version` gating flow in spirit (versioned, transactional, idempotent) without needing SQL DDL semantics that don't apply to a KV store.

---

## 3. Corruption Recovery

If `redb::Database::open` fails to load `workspace.redb` (e.g. the file is truncated or corrupted):

1. The storage service copies the corrupt file to `workspace.redb.corrupt.[unix_epoch]` as a backup, next to the original.
2. It creates a fresh `workspace.redb` (tables created lazily on first write, per §1 of `storage.md`).
3. It attempts to read whatever tables/records are still recoverable from the backup file (via a best-effort `redb::Database::open` in a recovery mode, or record-by-record salvage if the file is partially readable) and re-inserts them into the fresh database — preserving as much notification/presence history as possible without blocking startup on a full recovery.

This is the same intent as the original SQLite-era rollback protocol (isolate the corrupt file, don't lose more data than necessary, don't block startup indefinitely) adapted to `redb`'s single-file-database model.
