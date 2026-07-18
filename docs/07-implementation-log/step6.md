# Implementation Plan - Phase 6: Slack Integration (v0.1/v0.2)

This is a **design document for review — nothing described below has been implemented yet**, per the same process used for Phases 4-5.

## Context

`docs/01-product/roadmap.md` lists `v0.1 Slack` then `v0.2 Presence` as the next two milestones after the infrastructure/shell phases (2-5) already built. This phase closes both at once: the domain model, storage, CQRS read path, and TUI Team/Notification panels built in Phase 3/5 already fully support `NotificationItem` and `MemberPresence` — there is no seam that makes messages and presence separable adapter work, only separable *roadmap line items*. Splitting them into two phases would mean standing up the same adapter, HTTP client, and auth plumbing twice.

Three things this phase closes that earlier phases explicitly deferred:
- `crates/integration::SlackAdapter` is currently a stub — `initialize`/`start`/`health_check`/`shutdown` all no-op and return success immediately.
- `commands::WorkspaceCommandHandler` currently returns an honest `WorkspaceError::Integration("Slack integration not yet implemented")` for `Command::SendSlackMessage` (Phase 3's deliberate stand-in — see `crates/commands/src/lib.rs`).
- `docs/04-extensions/integrations/slack.md` and `docs/04-extensions/state-machine.md` are currently bare outlines (section headers only, no content) — this is the first phase that actually needs them to be real specs, not placeholders.

**What's already usable, unchanged:** `SecretProviderChain::default_chain()` (Env → Keyring → EncryptedFile, ADR-0006) already resolves a token from an environment variable today — `EnvProvider` is fully implemented, not a stub. `Event::SlackMessageReceived(NotificationItem)` and `Event::SlackPresenceChanged(MemberPresence)` already exist in `crates/events`. The `Projector` already upserts both into `DashboardReadModel` on receipt. Nothing in the write path, event path, or UI needs to change to consume real Slack data — only the adapter that produces it needs building.

**What's genuinely new:** no HTTP client exists anywhere in the workspace yet. `docs/04-extensions/security.md` §3 already names the intended choice (`reqwest` with `rustls`, TLS 1.3) — pure-Rust TLS, so this doesn't reopen the C-toolchain question ADR-0014 closed.

---

## Decisions (confirmed)

1. **Sync mechanism**: polling.
2. **Auth**: Bot Token via `SLACK_BOT_TOKEN` env var, not a full OAuth flow.
3. **No-token behavior**: honest-empty (`Disconnected`), not fake demo data.
4. **Presence scope**: a configured watch-list (`watched_user_ids`), not the whole workspace.

The reasoning for each is unchanged from the options below — kept for reference.

### 1. Sync mechanism: polling vs. a persistent connection

Slack offers three ways to receive updates: the deprecated RTM WebSocket API, **Socket Mode** (modern WebSocket, needs an app-level token + a persistent connection), or plain polling against the Web API (`conversations.history`, `users.getPresence`) on an interval. `docs/05-operations/configuration.md`'s existing (pre-written, Phase 2) config schema already has `[integrations.slack] sync_interval_secs = 30` — polling was the implicit original assumption, and it avoids adding a WebSocket client dependency on top of the HTTP one. Socket Mode would give faster presence/message updates but is meaningfully more code (persistent connection lifecycle, the `integration-contract.md` reconnect/backoff policy actually matters there) for a first cut.

**Recommendation**: polling for this phase, matching the existing config field. Socket Mode is a natural v0.2-and-a-half upgrade behind the same `IntegrationAdapter` interface, not a redesign, if 30s-latency presence turns out to feel stale in practice.

### 2. Auth: Bot Token via env var, not a full OAuth flow

A Slack **Bot Token** (`xoxb-...`, user creates a Slack App in their own workspace once and pastes the token) is one env var. A full OAuth authorization-code flow needs a registered Slack App with a client ID/secret *we* own, a redirect URI, and a local HTTP callback server — real work with no payoff until there's a public release for a wider (non-technical) audience to onboard through.

**Recommendation**: Bot Token via `SLACK_BOT_TOKEN` env var (works today, zero new secret-storage code) for this phase. Note: `KeyringProvider`/`EncryptedFileProvider` in `crates/secrets` are still stubs (`Ok(None)` always) — real OS-keyring storage is a separate, later piece of work, not blocking this one, since `EnvProvider` already satisfies the "how does a token reach the app" need for now.

### 3. No-token behavior: honest-empty vs. `integration-contract.md`'s "OfflineMode + fake demo data"

`docs/04-extensions/integration-contract.md` §2.3 currently specifies that when no token is found, the adapter should push **mock/demo data** ("Offline Workspace Demo") to the Event Bus so the UI has something to show. This directly contradicts the principle Phase 5 was explicitly built on (`step5.md`: *"an empty team list is correct, not a bug, until an integration exists to populate it"*) — showing fabricated Slack messages would be the fake-success pattern this project has otherwise deliberately avoided (`SendSlackMessage`'s honest error is the same instinct in the other direction).

**Recommendation**: drop the fake-data behavior from the spec. No token → adapter reports `ConnectionStatus::Disconnected` (or a new `OfflineMode` status if we want the UI to eventually distinguish "never configured" from "lost connection" — cosmetic, not required for this phase), panels stay honestly empty exactly as they do today. Update `integration-contract.md` §2.3 to match before implementing, per `docs/06-development/development.md`'s docs-first rule.

### 4. Presence scope: whole workspace vs. a configured watch-list

Slack's Web API has no single "give me presence for everyone" call — `users.getPresence` is one user per request. A workspace of any real size would mean dozens of API calls every `sync_interval_secs`, risking the 429 rate-limit path on every cycle.

**Recommendation**: presence polling is scoped to a small configured list of user IDs (a new `[integrations.slack] watched_user_ids = [...]` field), not the whole workspace roster. Message polling (`conversations.history`) is unaffected by this — it's already scoped to specific channel(s).

---

## Proposed Changes (pending the decisions above)

#### [MODIFY] `docs/04-extensions/integration-contract.md`
- Drop the OfflineMode-with-fake-data behavior (Decision 3). Reconcile the trait shown there with what's actually in `crates/integration` today (it currently specifies `AdapterError` and `Box<dyn EventBus>`; the shipped stub already uses `common::Result`/`Arc<dyn EventBus>`, matching every other trait in this codebase — the doc should match the code's established convention here, not the other way around, since `IntegrationAdapter` isn't in the Architecture Freeze v1 list).

#### [MODIFY] `docs/04-extensions/integrations/slack.md`
- Replace the bare outline with a real spec: Bot Token setup instructions, exact Web API endpoints used (`conversations.history`, `users.getPresence`), polling interval, rate-limit handling, the `NotificationItem`/`MemberPresence` field mapping.

#### [MODIFY] `docs/04-extensions/state-machine.md`
- Replace the bare outline with the actual states relevant to a polling adapter (simpler than the doc's original WebSocket-shaped `Disconnected → Connecting → Connected → Reconnecting` — polling doesn't really "connect", it either succeeds or fails per cycle).

#### [MODIFY] `docs/05-operations/configuration.md`
- Add `watched_user_ids` to the `[integrations.slack]` example schema (Decision 4).

#### [MODIFY] `crates/config` (found while cross-checking docs against code, not called out earlier)
- `IntegrationsToggle.slack_enabled: bool` is a flat flag — there's nowhere to put `channel_ids`/`watched_user_ids`/`sync_interval_secs`. Replace with a nested `SlackSettings { enabled, sync_interval_secs, channel_ids, watched_user_ids }` struct, TOML shape `[integrations.slack]` instead of the flat `[integrations] slack_enabled`. This is a breaking on-disk config schema change; acceptable given no public users exist yet (pre-v1.0) — old config files get the new default layout by deleting them and letting Zero-Config regenerate one.
- `github_enabled` stays a flat bool (no GitHub adapter exists yet — out of scope here).

#### [MODIFY] `crates/integration`
- Add `reqwest` (features: `json`, `rustls-tls`) and use the already-present `serde_json`.
- Real `SlackAdapter`: `initialize` resolves the token via `SecretProviderChain`; `start` spawns a polling loop (`tokio::time::interval`) that calls the Slack Web API, maps responses to `NotificationItem`/`MemberPresence`, and publishes `Event::SlackMessageReceived`/`Event::SlackPresenceChanged`; `health_check` reports last-poll outcome; `shutdown` stops the loop.
- Rate-limit handling per `integration-contract.md` §2.2 (respect `Retry-After`, skip a cycle rather than erroring).

#### [MODIFY] `crates/commands/src/lib.rs`
- `Command::SendSlackMessage` gets a real implementation (`chat.postMessage`) instead of the placeholder error, now that an adapter exists to call.

#### [MODIFY] `crates/app/src/main.rs`
- Construct `SlackAdapter`, call `initialize`/`start`, wire it into the boot sequence alongside the existing storage/event-bus/projector setup.

---

## Verification Plan

- Unit tests for the Slack API response → `NotificationItem`/`MemberPresence` mapping functions (pure functions, easy to test against fixture JSON — no live network needed).
- Unit tests for rate-limit handling (mock a 429 + `Retry-After`, assert the adapter skips rather than errors).
- `SlackAdapter::initialize` behavior with no token present (Decision 3) — asserts `Disconnected`, not fake data.
- No live-network integration test against real Slack (no test workspace / CI secret exists) — manual verification: run with a real `SLACK_BOT_TOKEN` and confirm messages/presence actually appear in the TUI panels for the first time.

---

## Implementation Notes (what actually happened)

- `crates/integration::slack`: real `SlackAdapter` (polling loop, `conversations.history`/`users.getPresence`/`users.info`/`chat.postMessage`), plus `SlackMessenger` — a narrow trait separate from `IntegrationAdapter` so `crates/commands` depends on just the one capability `SendSlackMessage` needs, not adapter lifecycle management. Deterministic UUIDv5 notification ids (from `channel_id:ts`) so re-polling the same message upserts instead of duplicating. 20 unit tests: pure mapping functions, the failure-counter state machine, rate-limit header parsing, no-token behavior.
- `crates/commands`: `WorkspaceCommandHandler` takes `Option<Arc<dyn SlackMessenger>>`; `SendSlackMessage` delegates to it when present, otherwise keeps the same honest "not configured" error as before.
- `crates/config`: `IntegrationsToggle.slack_enabled: bool` replaced with a nested `SlackSettings` struct (`enabled`, `sync_interval_secs`, `channel_ids`, `watched_user_ids`) — a breaking on-disk schema change, expected and acceptable pre-v1.0 (Decision context above).
- `crates/secrets`: found while wiring `SlackAdapter::initialize` — `SecretProviderChain` didn't itself implement `SecretProvider`, so nothing could pass a whole chain to code written against the trait (like `IntegrationAdapter::initialize`). Added the impl; not a scope change, a pre-existing gap this phase happened to be the first to hit.
- **A real ADR-0014-class finding, not just an implementation detail**: the original plan (and `docs/04-extensions/security.md` §3, written before any HTTP client existed) called for `reqwest` with `rustls`. Once actually wired up and built, `rustls`'s default crypto provider (`ring`) turned out to compile C/assembly at build time — reintroducing the exact C-toolchain requirement `redb` was chosen to eliminate. Caught by actually building the crate (not just reading `rustls`'s pitch) and confirmed via `cargo tree`/a clean rebuild showing `ring`/`cc` disappear entirely once switched to `reqwest`'s default `native-tls` backend (`schannel` on Windows, no compiler invoked). `security.md` §3 corrected to match, same pattern as ADR-0014's storage reconsideration.
- **Verification reality**: `cargo check --workspace --all-targets`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`, and `cargo test --workspace` all ran and passed on this machine (WinLibs GCC toolchain still on `PATH` from Phase 5, though — per the finding above — the `native-tls` switch means this phase's own new code no longer needs it; the GNU-target linker gap from earlier phases is unrelated and still open). No live Slack workspace was available to test the actual HTTP calls against a real API — that remains a manual verification step for whoever has a `SLACK_BOT_TOKEN` to test with.
