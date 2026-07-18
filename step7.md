# Implementation Plan - Phase 7: In-App Slack Credential Setup

This is a **design document for review** — matching the process used for Phases 4-6. User-confirmed decisions are below; the rest is how those get built.

## Context

Phase 6 shipped a working `SlackAdapter`, but the only way to give it a token is `SLACK_BOT_TOKEN` — an environment variable the user has to set themselves, differently on every OS, every time, with no persistence built into the app. The user asked for the opposite: set the token once, from inside the TUI, stored permanently, OS-independent. This directly closes a gap flagged (not built) in `step6.md`'s Decision 2: `KeyringProvider`/`EncryptedFileProvider` in `crates/secrets` have been stubs (`Ok(None)` always) since Phase 2.

Two things this phase closes:
- `crates/secrets`: `SecretProvider` is read-only. There is no way for the app to *write* a secret anywhere durable.
- `crates/ui::TuiRenderer` has no reference to a `CommandDispatcher` at all — the TUI is currently pure CQRS *read* side (`SharedReadModel`) plus local keyboard/render state. Nothing typed into the command bar today can actually mutate anything (`step5.md`'s "Enter pushes to history but doesn't dispatch" gap). This phase is the first thing that needs the UI to actually write.

## Decisions (confirmed)

1. **Storage**: real `KeyringProvider` (OS keychain — Windows Credential Manager / macOS Keychain / Linux Secret Service) as the primary write target, with a real `EncryptedFileProvider` (AES-256-GCM, local file) as a fallback for environments with no keyring daemon (headless Linux, some containers/CI). Matches `security.md` §1's original intent — this phase is what actually builds it instead of leaving it a stub.
2. **UI**: a dedicated setup overlay (not the generic command bar) — token input is masked (`*`), never lands in command-bar history/scrollback.
3. **Scope**: token only. Channel IDs / watched-user IDs stay in `config.toml`, edited by hand, for now — a channel/user picker needs its own Slack API calls (`conversations.list`/`users.list`) and is a separate, later piece of work.
4. **Hot-connect**: saving the token immediately attempts to connect (spawns/restarts the poll loop) and shows success/failure inline in the same overlay — no restart required.

---

## Design

### 1. `crates/secrets`: a write path

New trait, separate from the existing read-only `SecretProvider` (which `EnvProvider` still only implements — you can't durably "set" an env var from inside the process in a way that survives restart, so it isn't a write target):

```rust
#[async_trait]
pub trait SecretWriter: Send + Sync {
    async fn set_secret(&self, key: &str, value: &str) -> Result<()>;
}
```

- `KeyringProvider`: real implementation via the `keyring` crate (`windows-native-keyring-store` / `apple-native-keyring-store` / `zbus-secret-service-keyring-store` features — the last is a pure-Rust DBus client, not the C-binding `libdbus` flavor, keeping the ADR-0014/step6.md "no C compiler" lesson intact). `keyring::Error::NoEntry` maps to `Ok(None)` on read (not found is not a failure), other errors propagate.
- `EncryptedFileProvider`: real implementation via `aes-gcm` (pure Rust, RustCrypto). A random 256-bit key is generated on first use and stored in `~/.config/terminal-workspace/secrets.key`; secrets themselves (a `HashMap<String, String>`, JSON-serialized) are AES-256-GCM-encrypted into `secrets.enc` alongside it. **Honest limitation, documented not hidden**: with no OS keyring backing it, the encryption key sits in a plain file next to the ciphertext — this protects against casual exposure (e.g. an accidental backup/sync of just `secrets.enc`) but not a determined attacker with full filesystem read access. It exists as a *fallback for when the real keyring is unavailable*, not a security-equivalent alternative to it.
- `SecretProviderChain` gets a second internal list (`writers: Vec<Arc<dyn SecretWriter>>`, populated with Keyring then EncryptedFile — no Env) alongside the existing read `providers` list, and its own `set_secret` trying each in order. Both lists share the same underlying provider instances via `Arc` (not duplicated) where a provider implements both traits.

### 2. `crates/integration`: a connect operation

New trait (separate from `IntegrationAdapter`'s generic lifecycle and `SlackMessenger`'s send capability — same "narrow port per capability" pattern as Phase 6):

```rust
#[async_trait]
pub trait SlackConnector: Send + Sync {
    async fn connect(&self, event_bus: Arc<dyn EventBus>, token: String) -> Result<()>;
}
```

`SlackAdapter::connect`: persists `token` via its stored `Arc<dyn SecretWriter>` (new constructor parameter), updates in-memory state, stops any already-running poll loop (`shutdown`), and starts a fresh one (`start`) — idempotent whether this is the first connection or a reconnect with a replacement token.

### 3. `crates/commands`: a command to trigger it

New `Command::ConnectSlack { token: String }`. `WorkspaceCommandHandler` gains `Option<Arc<dyn SlackConnector>>` (parallel to the existing `Option<Arc<dyn SlackMessenger>>` — `None` when Slack isn't constructed at all, same honest-error-if-absent pattern).

### 4. `crates/ui`: the setup overlay and the missing dispatch path

- `WorkspaceState` gains `active_overlay: OverlayKind` (`Help` | `SlackSetup`) and `slack_setup: SlackSetupState { token_input: String, status: SlackSetupStatus }` (`Idle` | `Connecting` | `Connected` | `Failed(String)`).
- New global shortcut `Ctrl+S` opens the Slack setup overlay (`FocusMode::Overlay`, `OverlayKind::SlackSetup`).
- While that overlay is open, keystrokes capture into `slack_setup.token_input` (same text-editing logic as the command bar, factored out rather than duplicated). Enter with non-empty input returns a new `KeyOutcome::SubmitSlackToken(String)` instead of `Handled`.
- `TuiRenderer` gains a `command_dispatcher: Arc<dyn CommandDispatcher>` field (new — didn't exist before this phase) and a constructor parameter. `event_loop` handles `SubmitSlackToken` by setting `Connecting`, redrawing immediately (so "연결 중..." shows before the network call), dispatching `Command::ConnectSlack`, then setting `Connected`/`Failed` from the result.
- Render: masked token input (`*` per character), inline status line, matching the existing help-overlay's popup idiom (`centered_rect` + `Clear`).

### 5. `crates/app/src/main.rs`

- `SlackAdapter` is now always constructed (not gated on `config.integrations.slack.enabled`) — the setup screen needs an adapter to hand a token to even if the user starts with nothing configured. `enabled`/an existing token still gates whether `start()` is called automatically at boot; the setup overlay's `connect()` path works regardless.
- `TuiRenderer::new` gets the `CommandDispatcher` passed through.

---

## Verification Plan

- `crates/secrets`: round-trip tests for `KeyringProvider`/`EncryptedFileProvider` (set then get returns the same value); `NoEntry`/missing-file maps to `Ok(None)` not `Err`; chain write-fallback order.
- `crates/integration`: `SlackAdapter::connect` persists via a mock `SecretWriter`, transitions status, and is safe to call twice (no duplicate poll loop — verified via the same `poll_task` abort-and-replace mechanism Phase 6 already has for `shutdown`).
- `crates/ui`: keyboard tests for `Ctrl+S` opening the overlay, text capture while it's open, `Enter` producing `SubmitSlackToken`; render tests for the masked input (asserting `*` characters appear, not the raw token text — a real regression to guard against, not a formality).
- Manual: run the app, `Ctrl+S`, paste a real Bot Token, confirm it connects without restarting, confirm the token survives a full process restart with no `SLACK_BOT_TOKEN` env var set.

---

## Implementation Notes (what actually happened)

- `crates/secrets`: `SecretWriter` trait; real `KeyringProvider` (`keyring` v4, `v1` compat feature — needed explicitly, the crate doesn't build without either `v1` or `cli`) and real `EncryptedFileProvider` (`aes-gcm`, pure Rust). `SecretProviderChain` now carries a second `writers` list alongside the existing read `providers` list, sharing the same `Arc`-backed `KeyringProvider`/`EncryptedFileProvider` instances rather than constructing them twice.
- **Verified against a real backend, not just mocked**: the OS keyring round-trip test actually ran against Windows Credential Manager on this machine and passed. In the course of that, found (and documented, not worked around) a genuine race condition in `keyring` 4.1.5's own `v1` compatibility shim — its lazy default-store initialization flips an atomic "done" flag *before* the store is actually registered, so two `Entry::new()` calls racing on separate threads can have one fail with `NoDefaultStore` even though the backend is fine. Fixed on our side by keeping the two round-trip assertions sequential in one test function instead of two, sidestepping the upstream bug rather than papering over it.
- `crates/integration`: `SlackConnector` trait, `SlackAdapter::connect` (persist token → reset failure counter → `shutdown` then `start`, safe to call repeatedly for reconnects).
- `crates/commands`: `Command::ConnectSlack { token }`; `WorkspaceCommandHandler` gained a fourth constructor parameter (`Option<Arc<dyn SlackConnector>>`). The raw token never needs special handling in logs — `crates/logging`'s existing secret-scrubbing writer already redacts any `xoxb-`-prefixed substring regardless of where it appears, a Phase 2-era mechanism that happened to already cover this.
- `crates/ui`: `OverlayKind` (`Help` | `SlackSetup`), `SlackSetupState`/`SlackSetupStatus`, `Ctrl+S` shortcut, masked token input (simple append/backspace, deliberately not reusing the command bar's cursor-aware editing — a pasted/typed-once token doesn't need mid-string cursor movement). New `KeyOutcome::SubmitSlackToken(String)` variant lets the synchronous `handle_key` hand off to the async event loop, which is also where `TuiRenderer` first gained a `CommandDispatcher` reference at all.
- `crates/app/src/main.rs`: the Slack adapter is now **always** constructed (previously gated on `config.integrations.slack.enabled`), because the setup overlay needs something to connect through before anything is configured. Auto-start at boot now triggers on `enabled == true` **or** a credential already being found via the chain — requiring both would mean a token saved through the UI on one run silently didn't reconnect on the next, defeating the point of persisting it.
- **Verification reality**: `cargo check/clippy/fmt --workspace` and `cargo test --workspace` all ran and passed (102 tests total across the workspace by the end of this phase). The one live-backend keyring test is `#[ignore]`d by default (environments without a reachable OS keyring would otherwise fail it) but was run explicitly and passed here.

