# Implementation Plan - Phase 23: AI Assistant (v1 — read-only Q&A)

> **Status: reviewed, then declined — not implemented.** This document was written in full (provider verified live against Anthropic's docs, key decisions on hosting/scope confirmed directly with the user) and then the user decided the AI Assistant feature isn't needed after all ("AI까지는 필요 없을 거 같아"). Kept as a record of what was actually considered and why it wasn't built, not as a live plan — nothing below should be treated as pending work. If this is revisited later, re-verify the provider details (model IDs, pricing, API shape) rather than trusting this snapshot, since Anthropic's model lineup moves.

This is a **design document for review — nothing described below has been implemented yet**, per the same process used for Phases 6-22.

This document is written to be read in full, not skimmed — per the user's explicit standing request for this specific feature ("AI는 따로 문서 만들어서 자세하게 제공해줘. 내가 알수있게"). Every design choice below is explained with *why*, not just stated.

## Context

`crates/assistant/src/lib.rs` is a 20-line stub: an `AssistantManager` whose `ask()` method ignores its input and always returns the literal string `"AI Assistant stub response."`. `docs/03-domain/assistant.md` sketches something much bigger than a sensible first version: a multi-turn conversational agent that can call five tools (including `dispatch_command(command_str)` — letting the LLM take real actions like sending Slack messages or changing your presence on your behalf), backed by a vector-embedding memory store. Building all of that as a first slice would repeat a mistake this project has deliberately avoided every other phase (Pomodoro shipped without `ShortBreak`/`LongBreak`, Scheduler shipped without generic `SchedulerEvent` — build what's needed now, not what might be needed later).

Two decisions were asked directly before writing anything, because they involve real cost, privacy, and security trade-offs that are the user's call, not an engineering default:

1. **Where does the model run?** → **Cloud API, user's own API key** (not a local model via Ollama). Same "bring your own credential" shape every other integration in this app already uses (Slack Bot Token, GitHub PAT, Calendar's private iCal URL) — the key lives in the OS keychain, never in `config.toml`, and the user pays their own API costs directly to the provider. A local model was the zero-cost, zero-data-leaves-the-machine alternative, but it would need the user to separately install and run something like Ollama (a real departure from this project's "Zero Setup" principle) and would answer noticeably worse.
2. **Can it act, or only read?** → **Read-only in v1.** The assistant answers questions using a snapshot of current workspace state as context; it cannot send a Slack message, change presence, or dispatch any other command on the user's behalf. `docs/03-domain/assistant.md`'s `dispatch_command` tool is explicitly **not** built in this phase — letting an LLM take real actions through misinterpreted natural language is a distinct trust boundary that deserves its own dedicated design conversation later, not a default bundled into "let me ask it things."

**Provider verified, not assumed** (this project's standing practice — same diligence as `notify-rust`/`interprocess`/`tray-icon`'s feasibility checks): fetched Anthropic's current API docs directly.
- Endpoint: `POST https://api.anthropic.com/v1/messages`
- Required headers: `content-type: application/json`, `x-api-key: <key>`, `anthropic-version: 2023-06-01`
- Request body: `{"model": "...", "max_tokens": N, "system": "...", "messages": [{"role": "user", "content": "..."}]}`
- Response body: `{"content": [{"type": "text", "text": "..."}], "stop_reason": "...", "usage": {...}, ...}`
- Current model IDs confirmed live from Anthropic's docs: `claude-haiku-4-5` (fastest/cheapest, "near-frontier intelligence" per Anthropic's own description), `claude-sonnet-5` (best speed/intelligence balance), `claude-opus-4-8` (most capable, priciest). No new architectural dependency: this is a plain HTTPS POST via `reqwest`, already a workspace dependency used identically by the GitHub adapter (`crates/integration/src/github.rs`'s bearer-token REST calls are the closest existing template).

---

## Decisions

### 1. Model: `claude-haiku-4-5` by default, configurable

**Proposed**: default to Haiku 4.5 — the workload here (glance at a small, bounded amount of workspace context and answer one short question) doesn't need Opus/Sonnet-level reasoning, and since the user pays per-token directly, cheapest-that-works is the right default. `[integrations.assistant].model` in `config.toml` lets anyone switch to `claude-sonnet-5`/`claude-opus-4-8` if they want more capability for harder questions. `max_tokens` capped at 1024 in the request — bounds both response length and cost per query; a short workspace Q&A answer doesn't need more.

### 2. What context gets sent: a compact text summary built from data already on screen

**Proposed**: every `/ask` call sends a `system` prompt built from the same `DashboardReadModel` the TUI already renders from — unread notifications (source, title, priority; capped at the 20 most recent to bound token cost), team presence (name + status), and upcoming Calendar events (title + start time). No new data collection, no additional API calls to Slack/GitHub/Calendar just to answer a question — if it's not already sitting in memory for the UI to draw, the assistant doesn't have it.

**This is a real privacy decision worth being explicit about, not glossed over**: asking a question sends a summary of your Slack/GitHub/Calendar activity to Anthropic's API, a third party this app has never sent anything to before (every other integration talks *to* Slack/GitHub/Calendar's own APIs, not to a fourth party about them). The setup overlay (Decision 4) states this plainly before the first question is ever sent, and it only happens when `/ask` is actually invoked — never in the background, never speculatively.

### 3. Not modeled as an "integration" — no `Command::Connect`, no `IntegrationSource`, no polling

**Proposed**: every existing integration (Slack/GitHub/Calendar) is built around a *persistent connection* — `IntegrationConnectionStatus` (Connected/Reconnecting/Failed/...), a background poll loop, a health check. None of that fits a stateless request/response API where there's nothing to "stay connected" to between questions. So the Assistant deliberately does **not** reuse `Command::Connect`/`IntegrationSource`/`IntegrationConnectionStatus` — those model a shape this feature doesn't have. Instead: a new, narrow `Command::SetAssistantApiKey { key: String }` stores the key via the existing `SecretWriter` (`crates/secrets`, OS keychain / encrypted-file fallback, exact same mechanism Slack/GitHub/Calendar tokens already use — just keyed by a new string, `"assistant_api_key"`, not by `IntegrationSource`, which doesn't need a new variant since `crates/secrets`' `get_secret`/`set_secret` already take a plain `&str` key) and makes one real validation call (a minimal `/v1/messages` request) to confirm the key actually works before reporting success.

### 4. UI: reuses three existing patterns, no new UI paradigm

**Proposed**: no new split-screen chat panel (the original `screen-spec.md` Screen 3 mockup) — that's real UI work with no proven need yet for a single-question-at-a-time v1. Instead:
- **`Ctrl+A`** opens a setup overlay for the API key — visually identical to the Slack/GitHub/Calendar setup overlays (masked input, Enter to save and validate), with one addition: a static line disclosing what gets sent on every question (Decision 2's privacy note), shown before any key is ever entered.
- **`/ask <질문>`** in the command bar — the exact same `/`-prefixed convention as `/pomodoro`/`/send`/`/away`. Not a `Command` dispatched through `CommandHandler` (no domain state changes happen — asking a question is a query, not a mutation, the same reasoning that already keeps `SlackPicker`/`Picker`'s `list_items()` out of the `Command` pipeline). A direct `AssistantClient` port, injected into `TuiRenderer` the same way `slack_picker`/`pickers` already are.
- **A new `OverlayKind::AssistantResponse`** opens immediately on `/ask` submission showing "질문 중..." (mirrors the picker overlays' `Loading` state), then replaces it with the answer or a clear error ("API 키가 설정되지 않았습니다 — Ctrl+A", a network failure message, etc.) once the call returns. Closed with `Esc`, same as every other overlay.

### 5. No conversation memory in v1

**Proposed**: each `/ask` is a single, stateless request — no multi-turn history, no `chat_history` table, no session concept. `docs/03-domain/assistant.md`'s `Conversation`/`Message`/vector-memory sketch is explicitly deferred; nothing in this phase needs "remember what I asked five minutes ago." Keeps the first real version small and removes an entire class of decisions (how long to retain history, where to store it, context-window management across turns) that aren't justified yet.

---

## Proposed Changes

#### [MODIFY] `crates/assistant/Cargo.toml`, `crates/assistant/src/lib.rs`
Replace the stub `AssistantManager` with a real `AssistantClient` trait + `AnthropicAssistantClient` implementation: builds the `system` prompt from a `DashboardReadModel`-shaped snapshot (Decision 2), POSTs to `/v1/messages` (Decision 1), parses the response, maps HTTP/API errors to `WorkspaceError` (same pattern `crates/integration/src/github.rs` already uses for its own error mapping). Add `reqwest`, `secrets`.

#### [MODIFY] `crates/commands/src/lib.rs`
New `Command::SetAssistantApiKey { key: String }` (Decision 3); `WorkspaceCommandHandler` dispatches it to `SecretWriter::set_secret("assistant_api_key", ...)` plus a real validation call.

#### [MODIFY] `crates/config/src/lib.rs`
New `[integrations.assistant]` table: `model: String` (default `"claude-haiku-4-5"`) — no token field, the key never lives here (Decision 3).

#### [MODIFY] `crates/ui/src/state.rs`, `crates/ui/src/keyboard.rs`, `crates/ui/src/render.rs`, `crates/ui/src/lib.rs`
`OverlayKind::AssistantSetup`/`OverlayKind::AssistantResponse` (Decision 4); `Ctrl+A` global shortcut; `/ask` added to `COMMAND_HEADS`/`parse_command`; `TuiRenderer` gains an injected `Arc<dyn AssistantClient>` field, called directly (not through `CommandDispatcher`) on `/ask` submission, same wiring shape `slack_picker` already has.

#### [MODIFY] `crates/app/src/main.rs`
Construct the real `AnthropicAssistantClient`, wire into both `WorkspaceCommandHandler::new` (for `SetAssistantApiKey`) and `TuiRenderer::new` (for `/ask`).

---

## Verification Plan

- Unit tests for the pure parts: building the `system` prompt string from a `DashboardReadModel` (does it actually cap at 20 notifications, does it include presence/calendar correctly), parsing a real Anthropic API response shape into the answer text, mapping HTTP error statuses (401 invalid key, 429 rate limited, 5xx) to clear `WorkspaceError` messages.
- A real, live end-to-end test against the actual Anthropic API is possible in this environment (unlike Linux/macOS-only claims elsewhere) if a real API key is available — this project's "verify empirically" discipline applies here too: don't just trust that `reqwest` compiles and the JSON shapes match documentation, actually send one real request and confirm the round trip.
- `cargo fmt --all --check` / `cargo check --workspace --all-targets` / `cargo clippy --workspace --all-targets -- -D warnings` / `cargo test --workspace` all green.

---

## Implementation Notes (what actually happened)

_Filled in during/after implementation._
