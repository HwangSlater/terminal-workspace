# Implementation Plan - Phase 16: Plugin `get-member-presence` + Real Capability Enforcement

This is a **design document for review — nothing described below has been implemented yet**, per the same process used for Phases 6-15.

## Context

`step14.md` (Phase 14, Plugin Runtime) deliberately deferred two things from the original `plugin-sdk.wit` draft (`docs/04-extensions/plugin-sdk.md`): the `get-member-presence` host function, and real enforcement of `PluginCapability` (`crates/plugin-sdk/src/lib.rs`'s four variants — `NetworkConnect`, `FsRead`, `FsWrite`, `SlackRead` — are defined but `PermissionManager::verify_capability` (`crates/plugin-host/src/lib.rs`) always returns `true`, honestly labeled as a stub). This was Decision 3's explicit scoping, not an oversight: no example plugin at the time needed either, and building enforcement with nothing to enforce would have been speculative.

A recent full-project review (docs-honesty pass, this session) flagged `docs/04-extensions/capability-system.md` as describing a permission system that doesn't exist. This phase makes a first real, narrow slice of that document true — not the whole document (no filesystem/network gating yet, no `NetworkConnect`/`FsRead`/`FsWrite`/`SlackRead` enforcement, since nothing exercises those either), but a complete, real, end-to-end proof that capability gating works: `get-member-presence` is the one host function worth adding right now, and it becomes the first capability-gated one.

`crates/plugin-host` already depends on `domain` (declared in `Cargo.toml` since Phase 14, never actually used) — this phase is what finally uses it.

---

## Decisions

### 1. WIT contract: add `get-member-presence` back, with a `presence-status` enum

**Confirmed (not separately asked -- entailed by the other two confirmed decisions)**: restore `get-member-presence: func(user-id: string) -> result<presence-status, string>` to the `host-services` interface in `crates/plugin-sdk/wit/plugin-sdk.wit`, plus a `presence-status` enum mapping `domain::PresenceStatus`'s five variants (`active`/`away`/`offline`/`meeting`/`lunch`) directly — matching the original `plugin-sdk.md` draft's shape, since nothing about that part of the draft was wrong, only unbuilt. The `Err(String)` case covers both "no such user" and "capability not granted" (Decision 4) — same pattern `initialize`/`on-event`/`shutdown` already use.

### 2. A new `PluginCapability::PresenceRead` variant, granted via a per-plugin manifest file

**Confirmed**: add `PresenceRead` to `PluginCapability` (`crates/plugin-sdk/src/lib.rs`). Capabilities are declared in a new sibling file next to each plugin's `.wasm`: `<plugin_id>.toml`, e.g. `hello.wasm` + `hello.toml`. Minimal schema — just the list, not the fuller `[plugin]`/`[capabilities.network]`/etc. shape `capability-system.md` sketched (that's more surface than this phase's one real capability needs):

```toml
capabilities = ["presence-read"]
```

A **missing** manifest file means **zero capabilities granted** (secure default — matches "zero-privilege sandbox by default," `capability-system.md` §1's framing, which this phase makes real for the first time). A manifest file that exists but fails to parse is logged as an error and also treated as zero capabilities granted (fail-closed, not a load-blocking error — a typo in a capabilities file shouldn't take the whole plugin down, but it must not silently grant anything either).

### 3. Presence data via an injected trait, not a direct `storage`/`commands` dependency

**Confirmed**: mirror `step15.md`'s `IpcStatusProvider` pattern. A new trait in `crates/plugin-host`:

```rust
#[async_trait]
pub trait PluginPresenceProvider: Send + Sync {
    async fn presence(&self, user_id: &str) -> Option<domain::PresenceStatus>;
}
```

implemented in `crates/app` using the `SharedReadModel` `main.rs` already holds (`read_model.read().await.team_presence`, matching `AppStatusProvider`'s existing pattern), and injected into `PluginHostManager` alongside the existing `EventBus`/`PermissionManager`. Keeps `crates/plugin-host` decoupled from `crates/storage`/`crates/commands` — it already depends on `domain` for the `PresenceStatus` type, nothing more.

### 4. Enforcement point: checked per-call against each `PluginState`'s parsed manifest

**Confirmed (not separately asked -- entailed by Decision 2)**: `PluginState` (`crates/plugin-host/src/lib.rs`) gains a `granted_capabilities: Vec<PluginCapability>` field, populated by parsing `<plugin_id>.toml` during `load_one`. `PermissionManager` stays the single decision point (in case a future capability needs more than exact-match, e.g. `NetworkConnect` domain-prefix matching) but becomes a real check instead of a stub:

```rust
impl PermissionManager {
    pub fn verify_capability(&self, granted: &[PluginCapability], requested: &PluginCapability) -> bool {
        granted.contains(requested)
    }
}
```

`HostServicesHost::get_member_presence` (the new WIT-generated trait method `PluginState` implements) checks `self.granted_capabilities` via `PermissionManager` before calling into the injected `PluginPresenceProvider`; on denial, returns `Err("capability not granted: presence-read".to_string())` — never even reaches the provider.

### 5. A new example plugin proving both outcomes for real

**Confirmed (not separately asked -- entailed by Decisions 2-4, matches step14.md's established verification pattern)**: `examples/plugins/presence-checker`, whose `on-event` calls `get-member-presence("local-user")` and logs the result via the existing `log` host function. Two integration tests in `crates/plugin-host` build on the same real-`.wasm` pattern Phase 14 established (`examples/plugins/hello` et al.): one with a `presence-checker.toml` granting `presence-read` (expect a real presence value logged), one without the manifest file at all (expect the denial error, and confirm the mock `PluginPresenceProvider` was never called) — the "was never called" assertion is what actually proves enforcement happens *before* reaching the provider, not just that the response differs.

---

## Proposed Changes (pending confirmation of Decisions 1-5 above)

#### [MODIFY] `crates/plugin-sdk/wit/plugin-sdk.wit`
Add `presence-status` enum + `get-member-presence` to `host-services` (Decision 1).

#### [MODIFY] `crates/plugin-sdk/src/lib.rs`
Add `PluginCapability::PresenceRead` (Decision 2). Re-export the new WIT-generated `presence-status` type alongside `log`/`publish_event` for guest authors (mirroring the existing pattern for those two).

#### [MODIFY] `crates/plugin-host/src/lib.rs`
- `PluginPresenceProvider` trait (Decision 3).
- `PluginState` gains `granted_capabilities: Vec<PluginCapability>`; `load_and_initialize`/`load_one` parse the sibling `.toml` manifest (Decision 2) alongside the `.wasm` file.
- `PermissionManager::verify_capability` becomes a real check (Decision 4), no longer a stub.
- `HostServicesHost` impl gains `get_member_presence`, gated per Decision 4.
- `PluginHostManager::new` gains a `presence_provider: Arc<dyn PluginPresenceProvider>` parameter.

#### [MODIFY] `crates/app/src/main.rs`
Implement `PluginPresenceProvider` using the existing `SharedReadModel` (Decision 3); pass it into `PluginHostManager::new`.

#### [NEW] `examples/plugins/presence-checker/`
Decision 5's example plugin.

#### [MODIFY] `docs/04-extensions/capability-system.md`, `docs/04-extensions/plugin-sdk.md`
Update their Implementation Status notes: `presence-read` is now real; every other capability/permission-map row in `capability-system.md` remains unbuilt (unchanged from the note added this session) until a plugin actually needs one of them, per the same "don't build for hypothetical needs" reasoning `step14.md` already used once.

---

## Verification Plan

- Unit tests for manifest TOML parsing (`#[serde(default)]`-style round-trip, matching every prior phase's config-parsing test pattern) — including a malformed-file case proving fail-closed, not load-blocking.
- The two real integration tests from Decision 5 (granted vs. not granted), against a real compiled `.wasm`, not mocked.
- `cargo fmt --all --check` / `cargo check --workspace --all-targets` / `cargo clippy --workspace --all-targets -- -D warnings` / `cargo test --workspace` all green, matching every prior phase's bar.

---

## Implementation Notes (what actually happened)

Every Proposed Change above was implemented and verified. `cargo fmt --all --check`, `cargo check --workspace --all-targets`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace` all pass clean, including a new real `.wasm` component (`examples/plugins/presence-checker`) built with `cargo-component` and loaded by real `PluginHostManager` tests (not mocked).

1. **A genuine, significant bug found by this phase's first real test of `get-member-presence`** — the single most important finding: `PluginHostManager::EventHandler::handle` (the `on-event` forwarding path) and `shutdown_all` were calling guest WASM functions **directly inside their async fn bodies**, not inside `tokio::task::spawn_blocking` the way `load_one` already did. `PluginState::publish_event`'s own doc comment claimed "every guest call in this crate runs inside a spawn_blocking closure" — true for `initialize` (via `load_one`), **false** for `on-event`/`shutdown`. This went unnoticed through all of Phase 14 because no example plugin (`hello`/`looper`/`memhog`) ever called a host function that itself calls back into async code (`publish-event`, and now `get-member-presence`) from inside `on-event` — `log` doesn't need `block_on`, so the bug had no way to surface. The first `get-member-presence`-granted test hit it immediately: `tokio::runtime::Handle::current().block_on(..)` panicked with "Cannot start a runtime from within a runtime" because it ran directly on a Tokio worker thread instead of a blocking-pool thread. Fixed by restructuring `handle`/`shutdown_all` to remove each `LoadedPlugin` from the map, move it into a `spawn_blocking` closure, run the guest call there, and reinsert (or drop, on trap) based on the outcome — making the documented invariant actually true everywhere, not just at load time. This also fixes a latent, never-triggered version of the same bug in `publish_event` for any future plugin that calls `publish-event` from `on-event`.

2. **`cargo-component`'s hyphen-to-underscore artifact naming bit the test helper.** `examples/plugins/presence-checker/` (hyphenated directory, matching the other example plugins' `[package] name`) builds `presence_checker.wasm` (underscore) — Rust's standard library-artifact naming convention, not something `cargo-component` does specially, but the existing `example_plugin_wasm_path` test helper assumed the plugin's directory name and its `.wasm` file's stem were always identical (true for `hello`/`looper`/`memhog`, all hyphen-free). Fixed by deriving the wasm file stem via `name.replace('-', "_")` inside the helper, rather than reusing `name` verbatim for both the directory and the file.

3. **The core capability-enforcement proof, verified via "was the provider ever called," not response content.** `get_member_presence_reaches_the_provider_when_the_capability_is_granted` and `get_member_presence_is_denied_before_reaching_the_provider_without_the_capability` both assert on `MockPresenceProvider.calls` (the exact `user_id`s it was invoked with) rather than parsing what the guest logged — this is what actually proves the `PermissionManager::verify_capability` check runs *before* `PluginPresenceProvider::presence` is ever reached, not merely that the two cases produce different guest-visible outcomes (which a bug that let the provider always run, but just discarded the result on denial, could also produce).

4. **The `<plugin-id>.toml` manifest and the WIT contract both landed exactly as designed** (Decisions 1-2) — `presence-status` maps `domain::PresenceStatus`'s five variants 1:1, `capabilities = ["presence-read"]` parses via a plain `#[derive(Deserialize)]` struct (`serde`/`toml`, both already workspace dependencies), and unknown capability names in a manifest are logged and ignored per-entry rather than failing the whole file, matching Decision 2's fail-closed-not-load-blocking design exactly as planned — no surprises here, unlike findings 1-2 above.

`crates/plugin-host`'s previously-unused `domain` dependency (declared since Phase 14, never imported) is now genuinely used, for `domain::PresenceStatus`.

Deferred, as scoped by this phase's Context: `NetworkConnect`/`FsRead`/`FsWrite`/`SlackRead` remain defined but unenforced — nothing exercises them yet, and wiring enforcement for capabilities nothing needs would repeat the exact mistake this project has twice now deliberately avoided (`step14.md`'s original scoping, and this phase's own narrow slice of `capability-system.md`).
