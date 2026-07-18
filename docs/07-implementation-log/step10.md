# Implementation Plan - Phase 10: GitHub Integration (v0.3)

This is a **design document for review — nothing described below has been implemented yet**, per the same process used for Phases 6-9.

## Context

`docs/01-product/roadmap.md` lists `v0.3 GitHub` as the next milestone after Slack (v0.1/v0.2, Phases 6-9). Unlike Slack, most of the groundwork already exists and doesn't need to be re-invented:

- `Event::GitHubPRCreated(NotificationItem)` already exists in the frozen `Event` enum (`crates/events/src/lib.rs`) — added speculatively before any producer existed. This phase's whole job, event-wise, is to finally publish it. **No ADR is needed** (Architecture Freeze v1 only gates *adding* a variant, not implementing a producer for one that's already there).
- `IntegrationSource::GitHub` already exists in `crates/domain`.
- `docs/05-operations/configuration.md` §1 already sketches `[integrations.github]` (`enabled`, `sync_interval_secs`, `repositories`) as the "target shape" — it was written aspirationally in Phase 2 and never implemented; `crates/config::IntegrationsToggle.github_enabled` is still just a flat bool today.
- `docs/04-extensions/integrations/github.md` is a bare outline (section headers only), same starting state `slack.md` was in before Phase 6.
- The `IntegrationAdapter` trait, `SecretProviderChain`, `EventBus`, polling failure-counter policy (`integration-contract.md` §2.1), and rate-limit policy (§2.2) are all integration-agnostic infrastructure already built for Slack — reusable as-is.

**What's genuinely new:** an HTTP client call shape against a different API (GitHub REST v3), and a second concrete `IntegrationAdapter` living alongside `SlackAdapter`.

**Scope discipline, informed by a real proliferation concern found while drafting this doc:** Slack's full treatment across Phases 6-9 grew to *four* narrow traits (`SlackMessenger`, `SlackConnector`, `SlackPicker`, `SlackSelectionApplier`) plus in-app token-entry UI, an EventBus-driven status indicator, and a Ctrl+P repo/channel picker. Giving GitHub the identical treatment means `WorkspaceCommandHandler` and `TuiRenderer` each grow a second full set of per-integration fields — flagged here as a known trade-off, not a blocker: **confirmed decision below is to build the full treatment now** (connect UI + repo picker), accepting that field growth for this phase. The natural point to revisit — generalizing to something like a `Vec<Box<dyn Integration>>` registry — is the *next* integration (Calendar, v0.4), once there's a third data point on the shape of the pattern, not this one.

---

## Decisions

### 1. Scope: Phase-6-equivalent (polling + env var only) vs. full treatment (+ in-app setup UI + repo picker)

- Option A — Phase-6-equivalent: polling + env var token + config.toml-only repo list, no in-app UI.
- **Option B — Full treatment now (confirmed)**: also builds an in-app "Ctrl+G to connect" token-entry overlay (mirroring Slack's Ctrl+S) and a repo picker (mirroring Ctrl+P, own keybinding since Ctrl+P is already Slack-specific), in this same phase. Still **read-only** — no `Command::CommentOnPR`/`ApprovePR` equivalent to `SendSlackMessage`; nothing in this decision asked for GitHub writes, only for connect/picker UX parity with Slack's Phase 7/8.

### 2. Auth: GitHub PAT via env var, which scope

A classic Personal Access Token (`ghp_...`) with the `repo` scope (needed to read PRs on private repos; public repos work with no scope on an authenticated request, but `repo` covers both). Read via a `GITHUB_TOKEN` env var through the existing `SecretProviderChain` — exact parallel to Slack's `SLACK_BOT_TOKEN`/Decision 2 in `step6.md`. (Note: `redact.rs`'s log-scrubbing already has a `ghp_` prefix hardcoded from Phase 2 — this phase's token literally already can't leak into logs, nothing to change there.)

**Recommendation**: as above — matches the established Slack pattern exactly, no alternative considered worth presenting.

### 3. What to poll, and how "new" is detected (confirmed: open-PR diff)

GitHub's REST API has no cursor/webhook-free "give me only what changed" primitive for PRs the way Slack's `conversations.history` has a `ts` cursor. **Confirmed**: poll `GET /repos/{owner}/{repo}/pulls?state=open` each cycle; keep an in-memory `HashSet<(repo, pr_number)>` of PR numbers already seen per adapter instance. Any PR number not in the set is "new" → publish `Event::GitHubPRCreated`, then add it to the set. Matches the event that already exists, keeps this phase ADR-free (no `Event::GitHubPRUpdated`/closed variant — out of scope, mirrors Slack Phase 6 not detecting message edits/deletes either), and is simple to verify with fixture JSON.

### 4. Rate limit handling

GitHub's authenticated REST quota (5,000 req/hr) is generous compared to Slack's, but abuse-detection responses still happen (`403` with a `Retry-After` header, or `429` with `Retry-After` on the (rarer) secondary rate limit). Same shape as Slack's `429`/`Retry-After` handling in `integration-contract.md` §2.2.

**Recommendation**: on `403` or `429` with a `Retry-After` header present, pause and skip the current poll cycle without counting it as a failure — identical policy to Slack's, just triggered on GitHub's actual status codes (`403` is GitHub's primary rate-limit signal, not `429`).

### 5. Keybindings for the new overlays

Ctrl+S (Slack connect) and Ctrl+P (Slack picker) are taken. Following the same one-letter-mnemonic convention: **Ctrl+G** opens GitHub token entry (mirrors Ctrl+S), **Ctrl+R** opens the repository picker (mirrors Ctrl+P, "R" for Repositories since "P" is already Slack's). Both free today (existing bindings: Ctrl+Q, Ctrl+S, Ctrl+P, Ctrl+1-4).

---

## Proposed Changes

#### [MODIFY] `docs/04-extensions/integrations/github.md`
Replace the bare outline with a real spec: PAT setup instructions + `repo` scope, `GET /repos/{owner}/{repo}/pulls?state=open` (PR polling) and `GET /user/repos` (picker listing) endpoints, polling interval, rate-limit handling, `NotificationItem` field mapping (title = PR title, body = PR number + author, action_link = PR HTML URL).

#### [MODIFY] `crates/config/src/lib.rs`
`IntegrationsToggle.github_enabled: bool` → nested `GitHubSettings { enabled, sync_interval_secs, repositories }`, TOML shape `[integrations.github]` — matching the shape `configuration.md` §1 already sketches. `#[serde(default)]` on every field and on `IntegrationsToggle.github` itself, same lesson as the Slack config crash fixed in Phase 6 (`step6.md` Implementation Notes).

#### [NEW] `crates/integration/src/github.rs`
- `GitHubConfig { repositories: Vec<String>, sync_interval_secs: u64 }` (owner/repo strings, e.g. `"rust-lang/rust"`), held in `Arc<RwLock<_>>` on the adapter — same shape as `SlackConfig`, needed so `update_selection` (picker apply) can change it live without restarting the process.
- `GitHubAdapter` implementing `IntegrationAdapter`, `GitHubConnector`, `GitHubPicker`: `initialize` resolves the PAT via `SecretProviderChain`; `connect(token)` persists it via `SecretWriter` and re-initializes (mirrors `SlackConnector::connect`); `start` spawns a polling loop reusing the exact failure-counter/`Reconnecting`/`Failed` state machine from `integration-contract.md` §2.1; `health_check`/`shutdown` mirror `SlackAdapter`; `list_repositories()` calls `GET /user/repos` (paginated, same cursor convention as Slack's `conversations.list`) for the picker.
- Pure mapping function `map_pull_request(repo: &str, pr: &GitHubPullRequestResponse) -> NotificationItem` (deterministic UUIDv5 id from `"{repo}#{pr_number}"`, same duplicate-safe-upsert approach as Slack's `channel_id:ts`).
- Rate-limit helper mirroring Slack's, adapted to GitHub's `403`/`Retry-After`.
- Publishes `Event::GitHubPRCreated` and `Event::IntegrationStatusChanged{ source: IntegrationSource::GitHub, .. }` (the latter already generic since Phase 9 — no change needed there).
- `update_selection(repositories: Vec<String>)`: swaps the `GitHubConfig.repositories` list live, same as `SlackAdapter::update_selection`.

#### [MODIFY] `crates/integration/src/lib.rs`
Re-export `GitHubAdapter`/`GitHubConfig`/`GitHubConnector`/`GitHubPicker`/`PickerRepo` from the new `github` module, same as `slack`'s exports today. `PickerRepo` can likely just reuse the existing `slack::PickerChannel`-shaped `{ id, label }` pair — check for a sensible shared type during implementation rather than a forced one, but don't force a shared type if the two ever need to diverge.

#### [MODIFY] `crates/commands/src/lib.rs`
- `Command::ConnectGitHub { token: String }` and `Command::ApplyGitHubSelection { repositories: Vec<String> }` — direct parallels to `ConnectSlack`/`ApplySlackSelection`.
- `GitHubSelectionApplier` trait, defined here for the same cross-context reason `SlackSelectionApplier` is (touches both `config.toml` and the live adapter).
- `WorkspaceCommandHandler` gains `github_connector: Option<Arc<dyn GitHubConnector>>` and `github_selection_applier: Option<Arc<dyn GitHubSelectionApplier>>` fields (no `GitHubMessenger` — read-only, Decision 1). `list_repositories()` is a **read** — reached through a direct `Arc<dyn GitHubPicker>` port on `TuiRenderer`, *not* through `Command`/`CommandHandler`, per the CQRS correction already learned and documented in `step8.md`.

#### [MODIFY] `crates/ui/src/state.rs`
- `OverlayKind` gains `GitHubSetup`, `GitHubPicker` variants.
- `GitHubSetupState` (token input + status) — structurally identical to `SlackSetupState`.
- `GitHubPickerState` (single `Vec<PickerRow>` of repos + cursor + status) — simpler than `SlackPickerState` since there's only one list, not two (channels + users).
- `WorkspaceState` gains `github_setup: GitHubSetupState`, `github_picker: GitHubPickerState`, `github_connection_status: events::IntegrationConnectionStatus` (seeded at `run_loop()` start, same pattern as the Slack field from Phase 9).

#### [MODIFY] `crates/ui/src/keyboard.rs`
`Ctrl+G` → open `OverlayKind::GitHubSetup` (mirrors the existing Ctrl+S handler exactly). `Ctrl+R` → open `OverlayKind::GitHubPicker`, `KeyOutcome::OpenGitHubPicker` (mirrors Ctrl+P/`OpenSlackPicker`). Picker-row navigation (`j`/`k`/`Space`/`Enter`) reuses `capture_slack_picker_input`'s logic generalized to operate on any `&mut Vec<PickerRow>` + cursor, rather than copy-pasting it — this is a real, small, justified generalization (the exact same nav semantics, not a new pattern) as opposed to the earlier explicit decision not to generalize the *port/overlay* structure itself.

#### [MODIFY] `crates/ui/src/render.rs`
`render_github_setup_overlay`/`render_github_picker_overlay` (mirror the Slack overlay renderers). Header status line extends to show both Slack and GitHub connection status side by side, reusing the existing generic `slack_status_label`-style helper (rename to something integration-agnostic like `connection_status_label` while touching it, since it now serves two integrations).

#### [MODIFY] `crates/app/src/main.rs`
Construct `GitHubAdapter` alongside `SlackAdapter` (always constructed regardless of `enabled`, same Phase 6 rationale — the setup overlay needs something to connect to); wire `github_connector`/`github_selection_applier` into `WorkspaceCommandHandler`; add a `ConfigFileGitHubSelectionApplier` (parallel to the existing `ConfigFileSlackSelectionApplier`); compute `initial_github_status` via `health_check()` before constructing `TuiRenderer`, same as the existing `initial_slack_status`.

#### [MODIFY] `docs/03-domain/workspace-state.md`, `docs/05-operations/configuration.md`, `README.md`
Document the new overlay state shapes, the real `[integrations.github]` schema, and a "GitHub 연동" usage section (Ctrl+G / Ctrl+R) mirroring the existing "Slack 연동" section.

---

## Verification Plan

- Unit tests for `map_pull_request` against fixture JSON (pure function, no live network).
- Unit tests for the rate-limit helper (mock a `403`/`Retry-After`, assert the adapter skips the cycle rather than counting a failure).
- `GitHubAdapter::initialize` with no token present — asserts `ConnectionStatus::Disconnected`, no synthetic data (same honest-empty contract as Slack, `integration-contract.md` §2.3).
- Config round-trip test: an old `config.toml` with the flat `github_enabled = false` and no `[integrations.github]` table must still parse (mirrors the exact Phase 6 crash-and-fix precedent for Slack).
- No live-network integration test (no GitHub PAT available in this environment) — manual verification: run with a real `GITHUB_TOKEN` and confirm PR notifications appear in the Notification panel for the first time.
- `crates/ui`: `Ctrl+G`/`Ctrl+R` open the correct overlay + focus mode (mirrors the existing `ctrl_s_opens_the_slack_setup_overlay`/`ctrl_p_opens_the_slack_picker_overlay` tests).
- `crates/commands`: `Command::ConnectGitHub`/`ApplyGitHubSelection` error honestly when no GitHub adapter is configured, and delegate correctly when one is (mirrors the existing `ConnectSlack`/`ApplySlackSelection` test pairs).

---

## Implementation Notes (what actually happened)

- **A real DRY finding, caught while drafting `github.rs`**: `slack.rs`'s consecutive-failure state machine (`next_status`, `to_event_status`, `retry_after_seconds`, `PollResult`, the `RECONNECTING_THRESHOLD`/`FAILED_THRESHOLD` constants) turned out to be fully generic — nothing Slack-specific in any of it. Rather than copy-paste ~90 lines (and their 11 unit tests) into `github.rs`, hoisted them into a new `crates/integration/src/polling.rs` shared module; both adapters now import from there. This is the kind of duplication the project's "rule of three" restraint doesn't apply to — it wasn't a design choice being duplicated, it was the same mechanism with zero variation.
- **Picker-row navigation was *not* generalized**, despite the original plan (this doc's Proposed Changes, since edited) suggesting it. Slack's picker indexes a combined two-list (`channels` then `users`) space; GitHub's indexes a single list (`repositories`). Forcing a shared helper across those two shapes would have meant restructuring `SlackPickerState` (breaking its existing tests) just to save ~10 straight-line lines of `j`/`k`/`Space` logic in `capture_github_picker_input`. Wrote GitHub's own dedicated version instead — a real design correction, not an oversight, and now documented here so a future reader doesn't wonder why the "shared nav helper" mentioned in an earlier commit never showed up.
- **A real latent bug found while wiring the event-routing switch in `crates/ui/src/lib.rs`**: the Phase 9 `event_loop` handler for `Event::IntegrationStatusChanged` destructured `{ status, .. }` — discarding `source` — and unconditionally wrote to `state.slack_connection_status`. This was correct by coincidence (Slack was the only producer), not by design. Fixed to match on `source` and route to `slack_connection_status`/`github_connection_status` accordingly; without this fix, a GitHub status change would have overwritten the Slack header line instead of the GitHub one.
- **A real correctness fix in the GitHub rate-limit check**: GitHub returns a plain `403` for both "rate limited" and "bad/expired/insufficient-scope token." An initial version of `is_rate_limited` trusted the status code alone, which would have let an expired PAT retry forever as a false "rate limit," never tripping the `Reconnecting`/`Failed` threshold that would actually surface the problem to the user. Fixed to require either a `Retry-After` header or `X-RateLimit-Remaining: 0` alongside a `403` before treating it as rate-limited; a plain `403` now correctly counts as a poll failure. `429` has no such ambiguity and is always treated as rate-limited.
- **Config schema**: `IntegrationsToggle.github_enabled: bool` → nested `GitHubSettings { enabled, sync_interval_secs, repositories }`, exactly as planned. `docs/05-operations/configuration.md`'s §1 example schema already showed this shape (written aspirationally in Phase 2) — the implementation now matches what was already documented there, nothing to reconcile.
- **Verification reality**: `cargo check --workspace --all-targets`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`, and `cargo test --workspace` all ran and passed on this machine — 54 tests in `crates/integration` (up from 32; includes the new `polling` module's 11 and `github`'s new tests), 22 in `crates/commands` (up from 14), 61 in `crates/ui` (up from 47). No live GitHub account was available to test the actual HTTP calls against a real API — that remains a manual verification step for whoever has a `GITHUB_TOKEN` to test with, same caveat as Slack's Phase 6.
