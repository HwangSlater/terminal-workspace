# Implementation Plan - Phase 11: Integration Registry Generalization

This is a **design document for review — nothing described below has been implemented yet**, per the same process used for Phases 6-10.

## Context

`step10.md` explicitly flagged this: giving GitHub the same treatment Slack got meant `WorkspaceCommandHandler`, `Command`, and `TuiRenderer` each grew a second named field/variant set, and the doc noted "the natural point to revisit is the *next* integration (Calendar, v0.4), once there's a third data point on the shape of the pattern." The user has now confirmed we're not stopping at Calendar (Gmail, Jira are also on the product's stated scope, `docs/01-product/vision.md`), so refactoring now — before Calendar is built on top of the un-generalized pattern — avoids doing this same refactor twice.

**What's actually growing linearly today**, concretely:
- `commands::Command`: `ConnectSlack{token}` / `ConnectGitHub{token}` (identical shape) and `ApplySlackSelection{channel_ids, watched_user_ids}` / `ApplyGitHubSelection{repositories}` (different shape — see below) are separate variants per integration.
- `commands::WorkspaceCommandHandler`: 5 `Option<Arc<dyn _>>` fields today (`slack_messenger`, `slack_connector`, `slack_selection_applier`, `github_connector`, `github_selection_applier`), plus a `#[allow(clippy::too_many_arguments)]` on `new()` already needed at 8 constructor params. A third integration would push this to 7+ fields / 11+ params.
- `crates/app/src/main.rs`: each integration gets its own `let x_adapter = ...`, `let x_connector = Arc::clone(...)`, `ConfigFileXSelectionApplier` struct, wired individually into `WorkspaceCommandHandler::new(...)`'s growing argument list.
- `ui::TuiRenderer`: `slack_picker: Arc<dyn SlackPicker>` / `github_picker: Arc<dyn GitHubPicker>` — same shape, separate fields.

**What is NOT actually identical, and shouldn't be forced to look identical:**
- `SlackConnector::connect(&self, event_bus, token: String)` and `GitHubConnector::connect(&self, event_bus, token: String)` are **byte-for-byte the same signature** — this is a real, lossless generalization opportunity, not a stretch.
- `SlackSelectionApplier::apply(&self, event_bus, channel_ids: Vec<String>, watched_user_ids: Vec<String>)` takes **two** independent lists (channels to poll for messages, people to poll for presence). `GitHubSelectionApplier::apply(&self, event_bus, repositories: Vec<String>)` takes **one**. This is a genuine domain difference — Slack really does have two independent selectable dimensions, GitHub has one. Calendar (`calendar_ids`) and a hypothetical Jira (`project_keys`) both look like GitHub's one-list shape, not Slack's two-list shape. Forcing Slack into a generic `Vec<String>` shape would either lose the channels/users distinction or require a stringly-typed `HashMap<String, Vec<String>>`, both worse than just admitting Slack is the outlier here.
- Same split for the picker ports: `SlackPicker` has two methods (`list_channels`, `list_users`); `GitHubPicker` has one (`list_repositories`). Same reasoning — generalize the one-list shape, leave Slack's two-list shape alone.

So this isn't "extract one `Integration` trait that covers everything" — it's "generalize the two things that are actually identical across every integration built so far (connect-with-a-token, and single-list selection/picking), and leave Slack's genuinely two-dimensional shape as a documented, deliberate exception rather than distorting it to fit."

---

## Decisions (confirmed)

### 1. Scope: generalize now vs. only refactor `Command`/`WorkspaceCommandHandler`, leave `TuiRenderer`'s picker ports alone

**Confirmed**: Option A — generalize all three growth points identified above (`Command`, `WorkspaceCommandHandler`'s connector/single-list-applier fields, and `TuiRenderer`'s single-list picker port) in this phase, applied retroactively to Slack+GitHub's existing code (no behavior change, pure refactor) before Calendar is built on top of it. `TuiRenderer` gaining a new named field *and* a new `KeyOutcome`/overlay/state variant set per integration is the smaller of the two growth problems, but it's the same shape of problem, and doing it now costs little extra once the `Command` side is already being touched.

### 2. How to key the registries: `HashMap<IntegrationSource, Arc<dyn _>>` vs. a small custom `IntegrationRegistry` type

**Confirmed (accepted default)**: plain `HashMap<IntegrationSource, Arc<dyn IntegrationConnector>>` (and one more for `SelectionApplier`, one more for `Picker` in `TuiRenderer`), plus one small private helper function (`fn require<T>(map: &HashMap<...>, source: IntegrationSource) -> Result<&Arc<T>>`) for the repeated "not configured" error message — not a dedicated `IntegrationRegistry<T>` newtype, which would be speculative until the lookup needs behavior beyond "get or error." `IntegrationSource` already exists in `crates/domain`, already `Copy + PartialEq + Eq`; only needs `Hash` added (additive derive, not a frozen-contract change).

### 3. `Command` shape for the generalized cases

**Confirmed** (entailed by Decisions 1 and 2 above — Slack's two-list shape stays a deliberate exception, not folded into the generic single-list pattern):
- **`Connect`**: `Command::ConnectSlack{token}` / `Command::ConnectGitHub{token}` → single `Command::Connect{source: IntegrationSource, token: String}`.
- **Single-list selection**: `Command::ApplyGitHubSelection{repositories}` → `Command::ApplySelection{source: IntegrationSource, items: Vec<String>}`. `Command::ApplySlackSelection{channel_ids, watched_user_ids}` is **kept as its own variant, unchanged**.
- `Command::SendSlackMessage` is also kept as-is: only one integration sends anything today, so there's nothing to generalize *from* yet (generalizing a pattern with one data point is exactly the premature-abstraction failure mode this whole refactor is trying to avoid elsewhere).

---

## Proposed Changes (pending the decisions above)

#### [MODIFY] `crates/domain/src/lib.rs`
Add `Hash` to `IntegrationSource`'s derive list (needed as a `HashMap` key). Purely additive.

#### [MODIFY] `crates/integration/src/lib.rs`
- New generic traits, alongside the existing Slack/GitHub-specific ones (not replacing `SlackPicker`/`SlackSelectionApplier`, which stay):
  - `trait IntegrationConnector: Send + Sync { async fn connect(&self, event_bus: Arc<dyn EventBus>, token: String) -> Result<()>; }` — `SlackAdapter`/`GitHubAdapter` implement this instead of (not in addition to) their current bespoke `SlackConnector`/`GitHubConnector` traits, which are deleted (their `connect` signature was already identical, so this is a pure rename/merge, not new code).
  - `trait Picker: Send + Sync { async fn list_items(&self) -> Result<Vec<PickerItem>>; }` and `struct PickerItem { id: String, label: String }` — `GitHubAdapter` implements this instead of `GitHubPicker` (deleted, same reasoning). `SlackPicker` is untouched.
  - `trait SelectionApplier: Send + Sync { async fn apply(&self, event_bus: Arc<dyn EventBus>, items: Vec<String>) -> Result<()>; }` (lives in `crates/commands`, mirroring where `SlackSelectionApplier`/`GitHubSelectionApplier` live today, for the same cross-context reason). `GitHubAdapter`'s `update_selection` wrapper implements this instead of `GitHubSelectionApplier` (deleted). `SlackSelectionApplier` is untouched.

#### [MODIFY] `crates/commands/src/lib.rs`
- `Command::ConnectSlack`/`ConnectGitHub` → `Command::Connect { source: IntegrationSource, token: String }`.
- `Command::ApplyGitHubSelection` → `Command::ApplySelection { source: IntegrationSource, items: Vec<String> }`. `Command::ApplySlackSelection` unchanged.
- `WorkspaceCommandHandler`: `slack_connector`/`github_connector` fields → `connectors: HashMap<IntegrationSource, Arc<dyn IntegrationConnector>>`; `github_selection_applier` field → `selection_appliers: HashMap<IntegrationSource, Arc<dyn SelectionApplier>>` (`slack_selection_applier` stays a named field, unchanged). `slack_messenger` stays a named field, unchanged (only Slack sends).
- `new()`'s constructor collapses from 8 params to 6 (two `HashMap`s replace four of today's `Option` params) — `#[allow(clippy::too_many_arguments)]` likely no longer needed; verify during implementation instead of assuming.

#### [MODIFY] `crates/ui/src/lib.rs`
- `TuiRenderer.slack_picker`/`github_picker` → `slack_picker: Arc<dyn SlackPicker>` (unchanged) + `pickers: HashMap<IntegrationSource, Arc<dyn Picker>>`.
- `open_github_picker` (and any future single-list picker) become one shared `open_picker(source, terminal, state)` method; `open_slack_picker` stays separate (two-list shape).

#### [MODIFY] `crates/ui/src/keyboard.rs`
`KeyOutcome::SubmitGitHubToken`/`OpenGitHubPicker`/`SubmitGitHubSelection` → generalized to carry `IntegrationSource` (e.g. `SubmitToken(IntegrationSource, String)`) so a future Calendar connect/pick doesn't need three more `KeyOutcome` variants. `SubmitSlackToken`/`OpenSlackPicker`/`SubmitSlackSelection` stay separate (Slack's overlay is reached via its own dedicated `Ctrl+S`/`Ctrl+P`, and its selection is two-list).

#### [MODIFY] `crates/app/src/main.rs`
Each adapter's `ConfigFileXSelectionApplier`/wiring stays per-integration where the shape is genuinely different (`ConfigFileSlackSelectionApplier`), collapses to a single generic `ConfigFileSelectionApplier { adapter: Arc<dyn SelectionApplier>, config_field_setter: ... }`-style helper where the shape is now shared (GitHub, future Calendar) — exact shape to be worked out during implementation once the generic trait exists to write it against.

#### [NO CHANGE] `Event` enum, `IntegrationAdapter` trait, `ConnectionStatus`, the polling failure-counter (`crates/integration/src/polling.rs`, already shared since Phase 10)
None of this refactor touches the frozen `Event` enum or anything already-generalized.

---

## Verification Plan

- Every existing Slack/GitHub test (`crates/integration`, `crates/commands`, `crates/ui`) continues to pass with call sites updated for the renamed types — this is a refactor, not a behavior change, so no test's *assertions* should need to change, only its setup code (e.g. constructing a `HashMap` instead of passing `Some(x)`/`None` positionally).
- New tests: `Command::Connect`/`ApplySelection` dispatch correctly by `source` when multiple integrations are registered in the same `WorkspaceCommandHandler` (a case that didn't really exist before — the old positional `Option` params couldn't easily test "Slack connector present, GitHub connector absent" and vice versa in the same assertion as cleanly as a `HashMap` can).
- Manual check: run the app with both `SLACK_BOT_TOKEN` and `GITHUB_TOKEN` set, confirm `Ctrl+S`/`Ctrl+G` connect independently and `Ctrl+P`/`Ctrl+R` pick independently — proving the registry keys don't cross-wire.

---

## Implementation Notes (what actually happened)

- **`IntegrationConnector`/`Picker`/`SelectionApplier` landed exactly as designed** — `SlackConnector`/`GitHubConnector` deleted and merged into one `IntegrationConnector` trait (their signatures really were byte-for-byte identical); `GitHubPicker`/`PickerRepo` deleted, replaced by `Picker`/`PickerItem` (GitHub's `list_repositories` became `list_items`, same field shapes renamed `name`→`label` for consistency with `PickerRow`); `GitHubSelectionApplier` deleted, replaced by the generic `SelectionApplier`. `SlackPicker`/`SlackSelectionApplier`/`SlackMessenger` are all untouched, as planned.
- **A real, deliberate deviation from the Proposed Changes draft**: that section originally said `KeyOutcome::SubmitSlackToken` would stay separate from the generalized GitHub token-submit variant. Once actually building `crates/ui/src/keyboard.rs`, this turned out to be an inconsistency worth avoiding — `Command::Connect` is fully generalized (Decision 3), and Slack's token-submit shape (`String` in, no list involved) is exactly as identical to GitHub's as the `Command` itself is; only the *selection* shape differs (Slack's two lists vs. everyone else's one). Keeping a separate `SubmitSlackToken` variant next to a generic `SubmitToken` would have meant two ways to express the same outcome, and a future Calendar implementer would have had no clear signal which one to use. Fixed: `KeyOutcome::SubmitToken(IntegrationSource, String)` is used by **both** Slack's `Ctrl+S` and GitHub's `Ctrl+G` capture functions; only `OpenSlackPicker`/`SubmitSlackSelection` (the genuinely two-list-shaped ones) stayed separate from the generic `OpenPicker`/`SubmitSelection`.
- **A real latent bug caught while generalizing**: `crates/ui/src/lib.rs`'s `event_loop` handler for `Event::IntegrationStatusChanged` was *already* fixed to route by `source` back in Phase 10 (a bug found during that phase's own implementation) — re-verified here rather than re-broken, since this refactor touched the same function for the `KeyOutcome` match arms.
- **`WorkspaceCommandHandler::new`'s constructor went from 8 params to 7**, not the 6 the Proposed Changes section guessed — `slack_messenger` and `slack_selection_applier` both remain individually-named `Option` params (only `slack_connector`+`github_connector` collapsed into the one `connectors` map, and `github_selection_applier` became the `selection_appliers` map's first entry). `#[allow(clippy::too_many_arguments)]` was in fact no longer needed on `WorkspaceCommandHandler::new` — clippy stayed quiet at 7 params — but is still needed on `TuiRenderer::new` (8 params: `pickers` added one back).
- **`IntegrationSource` gained `Hash`** (`crates/domain/src/lib.rs`) — purely additive, no ADR needed (not on Architecture Freeze v1's list), verified no existing call site broke.
- **Verification reality**: `cargo check --workspace --all-targets`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`, and `cargo test --workspace` all ran and passed. Test counts: `commands` 21 (down from 22 — the old `ConnectSlack`/`ConnectGitHub`/`ApplyGitHubSelection`-specific tests collapsed into fewer, more general `Connect`/`ApplySelection` tests that cover the same behavior plus a new cross-source isolation test), `integration` 54 (unchanged — this crate's internal logic didn't change, only its public trait names), `ui` 62 (unchanged). No behavior changed for an end user — this was a pure internal refactor, confirmed by every pre-existing assertion still passing unmodified (only test *setup* code needed updating, per the Verification Plan's prediction).
