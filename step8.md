# Implementation Plan - Phase 8: In-App Channel/User Picker

Design document for review, matching the process used for Phases 4-7.

## Context

`step7.md` Decision 3 explicitly deferred this: "Channel IDs / watched-user IDs stay in `config.toml`, edited by hand, for now — a channel/user picker needs its own Slack API calls (`conversations.list`/`users.list`) and is a separate, later piece of work." That later piece is this phase.

**No new Slack scopes needed.** `channels:read` (already required, `docs/04-extensions/integrations/slack.md`) covers `conversations.list`; `users:read` (already required) covers `users.list`. Both are already granted by the time a user has connected via `Ctrl+S`.

**Two things this phase closes:**
- `crates/config` is currently read-only — `AppConfig::load_or_create_default()` exists, nothing writes a modified config back to `config.toml`. The picker needs to persist selections somewhere durable, and `config.toml` is where `channel_ids`/`watched_user_ids` already live (Phase 6).
- No Slack API call in `crates/integration` fetches a *list* of channels/users today — `SlackPoller` only ever calls `conversations.history`/`users.getPresence`/`users.info` for IDs already known from config. `conversations.list`/`users.list` are new.

---

## Decisions (confirmed)

1. **Trigger**: a new dedicated shortcut, `Ctrl+P`.
2. **Persistence**: write back to `config.toml`.
3. **Channel scope**: only channels the bot has already been invited to.

The reasoning for each is unchanged from the options below — kept for reference.

### 1. How is the picker opened?

**Option A**: extend the existing `Ctrl+S` flow — once connected, pressing `Ctrl+S` again opens the picker instead of the token overlay (token isn't needed twice).
**Option B**: a new dedicated shortcut (e.g. `Ctrl+P`) opening the picker directly, independent of the connect flow; if not yet connected, it shows a message telling the user to `Ctrl+S` first instead of trying to fetch anything.

Recommendation: **B**. Keeping "enter a credential" and "pick channels/people" as separate overlays with separate entry points avoids conditional branching inside one overlay's state machine (what does Enter do — submit a token, or confirm a selection? — depends on a mode flag either way, so a second shortcut is actually less state, not more).

### 2. Does a selection persist to `config.toml`, or stay in-memory for the session?

**Option A**: write back to `config.toml`. Requires a new `AppConfig` write path (doesn't exist yet). Round-tripping through `serde`+`toml` and overwriting the file loses hand-added comments/formatting — an honest limitation, same category as `EncryptedFileProvider`'s (`step7.md`), not hidden.
**Option B**: session-only — selections apply immediately (adapter restarts polling with the new lists) but revert to whatever's in `config.toml` on the next launch.

Recommendation: **A**. `step7.md`'s whole point was "set it once, it stays set" for the token; a picker that resets every restart would be a worse experience than the manual `config.toml` editing it's replacing. The comment-loss limitation is accepted, same as Phase 7's fallback storage.

### 3. Which channels appear in the list?

**Option A**: only channels the bot has already been invited to (`conversations.list`'s `is_member` field). Nothing selectable would fail to actually work once selected.
**Option B**: every public channel in the workspace, regardless of membership, with a visible "봇 미초대" marker on ones the bot hasn't joined.

Recommendation: **A**. `conversations.history` fails for a channel the bot isn't a member of — showing channels that would just error if selected invites a confusing dead end. The help text should say plainly: "채널에 봇을 초대해야 목록에 나타납니다" (invite the bot to a channel for it to show up here).

---

## Design (pending confirmation above)

### `crates/integration`
- `fetch_channel_list(http, token) -> Result<Vec<SlackChannel>>` (`conversations.list?types=public_channel`, filtered to `is_member: true`) and `fetch_user_list(http, token) -> Result<Vec<SlackUser>>` (`users.list`, filtered to `!is_bot && !deleted`). Both handle Slack's cursor-based pagination (`response_metadata.next_cursor`) — a workspace with >100 channels/users needs more than one page.
- New `SlackPicker` port (narrow, same pattern as `SlackMessenger`/`SlackConnector`): `async fn list_channels(&self) -> Result<Vec<PickerChannel>>`, `async fn list_users(&self) -> Result<Vec<PickerUser>>`, `async fn apply_selection(&self, channel_ids: Vec<String>, watched_user_ids: Vec<String>) -> Result<()>` (updates the adapter's live config and restarts polling — same shutdown-then-start idempotency as `connect`).

### `crates/config`
- `AppConfig::save_to(&self, path: &Path) -> Result<()>` — serializes via `toml::to_string_pretty` and overwrites. Used only by the picker's apply step, not by the normal boot path (`load_or_create_default` stays read-only for everything else).

### `crates/commands`
- Only `Command::ApplySlackSelection { channel_ids, watched_user_ids }` — this is the one operation that actually mutates anything (persists `config.toml`, restarts polling). **Correction made during implementation**: listing channels/users is a read, not a mutation — routing it through `Command`/`CommandHandler` would force `WorkspaceCommandHandler::Output` (fixed at `()` today) to somehow carry back a `Vec<PickerChannel>`/`Vec<PickerUser>` for this one variant only, which doesn't fit CQRS's own split (Commands mutate; Queries read) let alone the trait shape. `TuiRenderer` instead holds a direct `Arc<dyn SlackPicker>` reference alongside its `CommandDispatcher` and calls `list_channels()`/`list_users()` directly — a read, bypassing the command/event machinery entirely, same category as `read_model` already being read directly rather than "queried" through a Command.

### `crates/ui`
- `Ctrl+P`: if not connected, show an inline message; if connected, fetch (via the direct `SlackPicker` reference) and open a picker overlay populated once results return.
- Picker state: a single scrollable list mixing both sections (channel rows, then a divider, then user rows) with a checkbox per row — less state than two separately-focused panes, and nothing about selecting a channel vs. a user needs different key handling. `Space` toggles the row under the cursor, `j`/`k` move, `Enter` confirms and dispatches `Command::ApplySlackSelection` through the `CommandDispatcher`.

---

## Verification Plan

- `crates/integration`: pagination handling (fixture JSON with `next_cursor` across two fake pages), `is_member`/`is_bot`/`deleted` filtering, unit-tested against fixture JSON — no live network, same pattern as Phase 6.
- `crates/config`: `save_to` round-trips (`load` after `save` reproduces the same struct).
- `crates/ui`: keyboard tests for `Space` toggling a row, `Enter` producing the right dispatch payload; render test asserting checkbox state (`[x]`/`[ ]`) reflects selection.
- Manual: `Ctrl+S` connect, `Ctrl+P`, select real channels/users, confirm `config.toml` updates and the adapter starts polling the newly selected ones without a restart.

---

## Implementation Notes (what actually happened)

- `crates/integration::slack`: `fetch_channel_list`/`fetch_user_list` (cursor-paginated `conversations.list`/`users.list`), each split into a network wrapper plus a pure `extract_channel_page`/`extract_user_page` function so pagination and filtering (`is_member`, `!is_bot && !deleted`) are unit-tested against fixture JSON, no live network — same pattern as Phase 6's message/presence mapping. `SlackAdapter.config` changed from a plain `SlackConfig` to `Arc<RwLock<SlackConfig>>` so `update_selection` can replace it live and have `start()` pick up the new value on its next (re)start.
- **Real correction made mid-implementation, not just an implementation detail**: the original design sketched `Command::FetchSlackPickerLists` alongside `Command::ApplySlackSelection`. Building it made the mismatch concrete — `CommandHandler::Output` is fixed at `()` for `WorkspaceCommandHandler`, and forcing a `Vec<PickerChannel>`/`Vec<PickerUser>` through it for one variant only doesn't fit CQRS's own split (commands mutate, queries read). Fixed by giving `TuiRenderer` a direct `Arc<dyn SlackPicker>` reference alongside `CommandDispatcher`, bypassing the command/event machinery entirely for the read side — only `ApplySlackSelection` (a real mutation: `config.toml` + live poll loop) goes through `Command`.
- `crates/commands`: `SlackSelectionApplier` trait, deliberately defined here rather than alongside `SlackConnector`/`SlackMessenger` in `crates/integration` — applying a selection is cross-context (touches both `config.toml` and the adapter's poll loop), so neither `crates/config` nor `crates/integration` should know about the other. The concrete implementation (`ConfigFileSlackSelectionApplier`) lives at the composition root (`crates/app/src/main.rs`), where both halves are already available.
- `crates/config`: `AppConfig::save_to` (round-trips via `serde`+`toml`, losing hand-added comments — accepted, documented, same category as `EncryptedFileProvider`'s tradeoff) and `resolve_config_path` (exposes the exact path `load_or_create_default` resolved, including a `--config` override, so the picker's write-back targets the same file, not a guess).
- `crates/ui`: `Ctrl+P` opens a picker overlay; `j`/`k`/`Space`/`Enter` navigate/toggle/confirm a single combined channel-then-user list (one cursor, one set of key handlers — no separate Tab-focused panes needed). New `KeyOutcome::OpenSlackPicker`/`SubmitSlackSelection` variants hand the network I/O and dispatch off to the async event loop, same pattern as `SubmitSlackToken` in Phase 7.
- **Verification reality**: `cargo check/clippy/fmt --workspace` and `cargo test --workspace` all ran and passed (119 tests total by the end of this phase). No live Slack workspace was available to test the real `conversations.list`/`users.list` calls end-to-end — that remains a manual verification step, same caveat as Phase 6.
