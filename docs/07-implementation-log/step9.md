# Implementation Plan - Phase 9: Command Bar Dispatch + Connection Status

Design document for review, matching the process used for Phases 4-8.

## Context

Two gaps flagged in the last review, both real:

1. **`Command::SendSlackMessage` has no way to be triggered.** It's been fully implemented since Phase 6 (`chat.postMessage`, honest error when Slack isn't configured), but the command bar (`:`) still only pushes typed text to history without parsing it — a gap noted as far back as `step5.md` ("no text-to-Command parser... building one now would have nothing meaningful to parse into") and never closed since. It has something to parse into now.
2. **No persistent connection-status indicator.** If Slack's background poll loop degrades to `Reconnecting`/`Failed`, nothing in the main UI shows it — the only way to find out is reopening `Ctrl+S`/`Ctrl+P` and noticing.

## Decisions (confirmed)

1. **Scope**: `/send` (`SendSlackMessage`) plus presence shortcuts (`/away`, `/active`, `/offline`, `/meeting`, `/lunch`, mapping to `SetPresence` — with the trailing text after the status word, if any, as `custom_text`).
2. **Channel targeting**: `/send #name message text`, resolved against `WorkspaceState.slack_picker.channels` (populated by the `Ctrl+P` picker) — a clear error if the name isn't found there.
3. **Connection status**: real live updates via an `EventBus` subscription in the render loop, not just polled-on-redraw. This is a bigger change than a status badge alone — see Decision 3 below for what it actually touches — but it also fixes something not explicitly asked about: today, a new Slack message or presence change already lands in `DashboardReadModel` in the background, but the panel showing it doesn't visually update until the user's next keypress (the render loop only redraws on input/resize). Subscribing the event loop to the bus fixes both at once.

The reasoning for each is unchanged from the options below — kept for reference.

### 1. Command bar syntax and scope

`docs/02-architecture/ui.md`'s original mockup shows `/slack-send @bob I'm on it!` inside the command bar. The actual command is channel-based, not DM-based (`SendSlackMessage { channel_id, text }` — no per-user DM send exists), so the real syntax is closer to `/send <channel_id> <message text>`.

**Recommendation**: wire up `/send` only this phase — it's the specific gap identified. `SetPresence` (e.g. a hypothetical `:away`) and `MarkNotificationRead` are real commands too but weren't flagged as missing, and adding parseable syntax for them now would be scope creep beyond what was actually asked for.

### 2. How does `/send` identify a channel — raw ID, or a name?

**Option A**: `/send C0123456789 message text` — the raw Slack channel ID. Always works, no dependency on anything else having happened first, but the user has to know/copy the ID (not shown anywhere in the UI as configured — `config.toml` has it, the picker shows names not IDs).
**Option B**: `/send #general message text` — resolved against whatever channel list the `Ctrl+P` picker last fetched into `WorkspaceState.slack_picker.channels` (already in memory if the picker's been opened this session). Friendlier, but silently depends on the picker having been opened at least once, and the mapping goes stale if channels changed since.

**Recommendation**: **B**, with a clear error ("먼저 Ctrl+P로 채널 목록을 불러와주세요") if the name isn't found in that cached list — reuses data already being fetched for the picker rather than adding a second lookup path, and matches how a user would naturally discover channel names (through the picker, not by hunting for raw IDs).

### 3. Connection status: polled-on-redraw (simple) vs. event-pushed (live)

The render loop is deliberately event-driven, not a timer tick (`docs/02-architecture/ui.md`: "reacts... not polling") — it only redraws in response to a keypress or terminal resize. Two ways to show status:

**Option A (recommended)**: add a narrow `SlackStatusReporter` port (same one-capability-per-trait pattern as `SlackMessenger`/`SlackConnector`/`SlackPicker`). `TuiRenderer::draw()` calls its `health_check()` (an in-memory read on the adapter's side, not a network call) right before every redraw and stores the result in `WorkspaceState`, shown in the header. Correct as of the *last* keypress/resize, not truly live — if the user goes idle and the connection drops in the background, they won't see it until they press something. No architecture change needed.
**Option B**: also subscribe the UI's event loop directly to the `EventBus` (`tokio::select!` between the existing input channel and a new event-bus receiver), redrawing on relevant events even with no key pressed. Genuinely live, but is a bigger change to the core loop (a second stream to select over, `Event::IntegrationStatusChanged` would need to be a new frozen-adjacent variant since `Event` isn't Architecture-Freeze-locked but is still a shared contract, more testing surface) — and it's not just a status-indicator problem once opened up, the same mechanism would make new messages/presence appear live too, which is a materially bigger scope than "show connection status."

**Recommendation**: **A** for this phase. **B** is a legitimate, separate future phase ("live background redraw," not just a status badge) if the polled-on-interaction behavior turns out to feel stale in practice — flagging it here so it's a deliberate deferral, not a forgotten one.

---

## Design

### `crates/events`
- New `IntegrationConnectionStatus` enum (`Disconnected`/`Connecting`/`Connected`/`Reconnecting`/`Failed(String)`) — a structural duplicate of `integration::ConnectionStatus`, not a re-export of it. `crates/events` cannot depend on `crates/integration`: `integration` already depends on `events` for `EventBus`/`Event`, so the reverse would be circular. `crates/integration` maps its own `ConnectionStatus` into this type when publishing, same way it already maps Slack's wire JSON into domain types.
- New `Event::IntegrationStatusChanged { source: IntegrationSource, status: IntegrationConnectionStatus }` variant.

### `crates/integration`
- `SlackPoller::run_loop` publishes `Event::IntegrationStatusChanged` on every status transition (not just the existing Failed-threshold `SystemAlert`, which stays as-is — a distinct "raise an alert" signal, not replaced by this). `SlackAdapter::connect` publishes one immediately after setting `Connecting`, for instant UI feedback on submit rather than waiting for the first poll cycle.

### `crates/ui`
- New direct dependency on `crates/events` (needed for the `Event`/`IntegrationConnectionStatus` types flowing through the subscription — `ui` already depends on `commands`, which depends on `events`, but Rust doesn't let a crate use a transitive dependency's types without declaring it directly).
- `capture_command_text`'s `Enter` handler gains a real parse step for `/send #name text...` and `/away`/`/active`/`/offline`/`/meeting`/`/lunch [text...]` → `KeyOutcome::SubmitCommand(Command)` (new variant, same "sync `handle_key` can't dispatch, hand off to the async event loop" pattern as `SubmitSlackToken`/`SubmitSlackSelection`). Channel names resolve against `state.slack_picker.channels`; unparseable/unmatched input stays exactly as it is today — pushed to history, not dispatched, no error the user has to acknowledge for plain chat-style typing.
- `WorkspaceState` gains `slack_connection_status: events::IntegrationConnectionStatus`, set from an initial value at construction (`TuiRenderer` computes it once via `SlackAdapter::health_check` at boot, since nothing's been published to the bus yet at that point) and kept current after that purely by the event subscription below.
- `TuiRenderer` gains an `event_bus: Arc<dyn EventBus>` field. `event_loop` restructures around `tokio::select!` between the existing input `mpsc` channel and a new `broadcast::Receiver<Event>` (`event_bus.subscribe()`) — any event received triggers a redraw (picking up whatever `DashboardReadModel` change or status change came with it), and `Event::IntegrationStatusChanged` additionally updates `state.slack_connection_status` first.
- `render_header` shows the status (Korean labels: 연결됨/재연결중/연결 안 됨/실패, same style as `presence_status_label`).

### `crates/commands`
- `Projector::handle` (the `EventHandler` impl) gains a match arm for `Event::IntegrationStatusChanged` — not surfaced in `DashboardReadModel` (the UI's direct bus subscription handles it, no read-model role needed), but the match has to be exhaustive.

### `crates/app/src/main.rs`
- Compute the initial status via `slack_adapter.health_check().await` (already available — `SlackAdapter` already implements `IntegrationAdapter`, no new port needed) and pass it, plus `Arc::clone(&event_bus)`, into `TuiRenderer::new`.

---

## Verification Plan

- `crates/ui` keyboard tests: `/send #general hi` with `general` in `slack_picker.channels` → `SubmitCommand(Command::SendSlackMessage{..})` with the resolved ID; unknown channel name → stays as plain history text, not dispatched, no crash; `/away brb` → `SubmitCommand(Command::SetPresence{status: Away, custom_text: Some("brb")})`; plain text with no leading `/` unchanged from today.
- `crates/ui` render tests: header shows the right Korean label for each `IntegrationConnectionStatus` variant.
- `crates/integration` tests: a status transition during `run_loop` publishes `Event::IntegrationStatusChanged` with the right mapped status (subscribe a test receiver to a real `InProcessEventBus`, drive a poll cycle, assert on what's received) — same style as the existing `SystemAlert`-on-Failed test, extended.
- Manual: `/send` a real message to a real channel after `Ctrl+P`-selecting it; confirm the header status updates in the background (no keypress) by watching it during a period of inactivity.

---

## Implementation Notes (what actually happened)

- `docs/06-development/decisions/0016-event-enum-extension.md`: `enum Event` is explicitly frozen under Architecture Freeze v1 (`docs/06-development/development.md` §3 — "Event Contracts... cannot be altered" without an ADR). Adding `IntegrationStatusChanged` needed one; written before the code, per the project's own rule.
- `crates/events`: `IntegrationConnectionStatus` (structurally identical to `integration::ConnectionStatus`, not a re-export — `events` can't depend on `integration`, which already depends on `events`) and the new `Event` variant.
- `crates/integration::slack`: `SlackPoller::run_loop` publishes on every transition (previously only the Failed-threshold `SystemAlert` existed); `SlackAdapter::connect` publishes immediately on submit for instant setup-overlay feedback.
- `crates/commands`: `Projector`'s exhaustive match gained a no-op arm for the new variant — its job is done entirely by `crates/ui`'s own bus subscription.
- `crates/ui`: this ended up touching more than a status badge, per the confirmed scope:
  - `Command` gained `PartialEq`/`Eq` (needed for `KeyOutcome` to stay comparable in tests once it could hold a `Command`).
  - `capture_command_text` now returns `Option<Command>` (previously `()`) via a new pure `parse_command` function — `/send #name text`, `/away`/`/active`/`/offline`/`/meeting`/`/lunch [text]`. Channel names resolve against `WorkspaceState.slack_picker.channels`, whatever the `Ctrl+P` picker last fetched. A recognized-but-unresolvable attempt (leading `/`) sets `CommandBufferState.last_error`, shown in the command bar even after `Esc`; plain chat-style text is completely unchanged from before this phase.
  - `event_loop` restructured around `tokio::select!` between the input channel and a new `broadcast::Receiver<Event>` — any event now triggers a redraw, not just keypresses/resizes. This was the point of Decision 3's "bigger" option: a new Slack message or presence change arriving while the user is idle now shows up without them touching a key, not just the connection-status header.
  - `TuiRenderer::new` gained two new parameters (`event_bus`, `initial_slack_status`) — `main.rs` computes the initial status once via `SlackAdapter::health_check()` (already available, no new port needed) since nothing's been published to the bus yet at that point.
- **Verification reality**: `cargo check/clippy/fmt --workspace` and `cargo test --workspace` all ran and passed (133 tests total by the end of this phase). Manually confirmed live: the header correctly showed "Slack: 연결 안 됨" on a real run against the reporter's actual `config.toml`.
