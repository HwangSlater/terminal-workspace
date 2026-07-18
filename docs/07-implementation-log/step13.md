# Implementation Plan - Phase 13: Command Bar Autocomplete

This is a **design document for review — nothing described below has been implemented yet**, per the same process used for Phases 6-12.

## Context

`CommandBufferState.autocomplete_suggestions: Vec<String>` and `selected_suggestion_index: Option<usize>` have existed since Phase 5, always empty/`None` — `step5.md`'s scope note said the `Command` enum was too small to make completion useful yet. That's no longer true: `parse_command` (`crates/ui/src/keyboard.rs`, `step9.md`) now recognizes six command heads (`/send`, `/away`, `/active`, `/offline`, `/meeting`, `/lunch`), and `/send`'s first argument resolves against `state.slack_picker.channels` — a real cache of real channel names, populated by `Ctrl+P` (`step8.md`). Both are worth completing: remembering exact command spelling and exact channel names is the actual friction today.

**A related gap noticed while reading `capture_command_text`, not part of this phase's scope**: `Tab` inside the command bar is currently a complete no-op (falls through `capture_command_text`'s `_ => None` arm), and `history_index`/`history` are populated on every `Enter` but nothing ever reads `history_index` back — `Up`/`Down` don't browse history at all despite the field existing for exactly that. Flagging this now since it's adjacent, but out of scope here; `Tab` is what this phase claims.

## Decisions (confirmed)

### 1. What gets completed

**Confirmed**: both the command head (`/a` → `/active`, `/away`) and, specifically for `/send`'s first argument, the channel name (`/send #gen` → `/send #general`) against `state.slack_picker.channels` — the same cache `resolve_channel_id` already trusts. Nothing else (the free-text message body, presence custom-text) is completable — there's no finite candidate set for free text. Both cases share the same underlying mechanism (prefix-match a word against a candidate list), so implementing one and not the other wouldn't have saved meaningful complexity.

### 2. Interaction model

**Confirmed**: classic shell Tab-completion. Candidates are computed fresh against the current word every time the buffer's text changes (so they're always in sync with what's typed, no staleness); `Tab` replaces the current word with the first candidate and remembers the candidate list; pressing `Tab` again (before typing anything else) cycles to the next candidate, wrapping around. Any other key ends the cycle (next `Tab` press recomputes fresh). No new keybinding beyond `Tab`, which is currently unused in Input mode — and the interaction model most terminal users already have muscle memory for.

### 3. Rendering the candidate list

**Confirmed (accepted default, entailed by Decision 2)**: an inline dim-gray hint appended after the typed text in the existing one-row command bar (`  (Tab: /active, /away)`), with the currently-cycled-to candidate visually distinguished (bold/cyan) from the rest. No new layout row, no new overlay — extends `render_command_bar` the same way the existing `last_error` suffix already does.

---

## Proposed Changes (pending the decisions above)

#### [MODIFY] `crates/ui/src/keyboard.rs`
- New pure function `fn compute_suggestions(text: &str, cursor: usize, picker: &SlackPickerState) -> Vec<String>`: inspects `text[..cursor]`'s word count/position to decide mode (first word starting with `/` → command-head candidates; second word starting with `#` when the first word is exactly `/send` → channel-name candidates prefixed with `#`; anything else → empty), unit-testable against fixture strings with no `WorkspaceState` needed.
- New pure function `fn word_start(text: &str, cursor: usize) -> usize`: finds the byte offset where the word under/before `cursor` begins (last space before `cursor`, or `0`). Deliberately separate from `compute_suggestions` — safe to call against text that Tab has already partially completed (word boundaries don't move just because the word's *content* changed), whereas re-running `compute_suggestions` against already-completed text would match against itself and break cycling (a real design trap caught while drafting this doc, not found by trial and error).
- `capture_command_text`'s `Char`/`Backspace` arms call `compute_suggestions` after mutating `raw_text`, storing the result and resetting `selected_suggestion_index` to `None`.
- `capture_command_text` gains a `Tab` arm: no-ops if `autocomplete_suggestions` is empty; otherwise advances `selected_suggestion_index` (wrapping), splices the selected candidate into `raw_text` at `word_start(...)..cursor_position`, and moves the cursor to the end of the inserted text. Does **not** call `compute_suggestions` again (per the note above).
- `Enter` clears `autocomplete_suggestions`/`selected_suggestion_index` alongside the existing `raw_text`/`cursor_position` reset.

#### [MODIFY] `crates/ui/src/render.rs`
`render_command_bar`'s `FocusMode::Input` branch gains a second `Span` when `autocomplete_suggestions` is non-empty: the joined candidate list in dim gray, with the `selected_suggestion_index`-th entry (if any) styled distinctly.

#### [NO CHANGE] `crates/ui/src/state.rs`, `crates/commands`, `crates/integration`
`autocomplete_suggestions`/`selected_suggestion_index` already exist with the right shape. This phase populates and reads them; it doesn't need new fields, new `Command` variants, or anything below the UI layer — completion candidates come entirely from static command-head strings and the already-in-memory `slack_picker` cache.

---

## Verification Plan

- `compute_suggestions`/`word_start` unit-tested directly (pure functions, no `WorkspaceState` needed): command-head prefix matching (`/a` → `["/away", "/active"]`, order matches `COMMAND_HEADS`' declaration order, not alphabetical), channel-name matching scoped to `/send`'s argument position only (typing `#gen` as a *third* word, or after a non-`/send` command, yields no suggestions), case-insensitive channel matching (mirrors `resolve_channel_id`'s existing behavior), empty prefix / no match cases.
- `handle_key`-level tests: first `Tab` completes to candidate 0 and updates `raw_text`; second consecutive `Tab` cycles to candidate 1 without reverting to the typed prefix; typing a character between two `Tab` presses starts a fresh cycle instead of continuing the old one; `Tab` with no candidates is a no-op (doesn't submit, doesn't crash); `Enter` clears both fields.
- `render_command_bar` test: the dim-gray hint appears when suggestions exist and doesn't appear when they don't (mirrors the existing `last_error` visibility tests).

---

## Implementation Notes (what actually happened)

- **Landed close to the Proposed Changes draft**, with the two helper functions (`compute_suggestions`, `word_start`) and the `Tab`/`Char`/`Backspace`/`Enter` wiring exactly as planned. `COMMAND_HEADS` (a new module-level constant listing `/send`, `/away`, `/active`, `/offline`, `/meeting`, `/lunch`) became the single source of truth `compute_suggestions` filters against, so `parse_command`'s recognized heads and the completion candidates can't silently drift apart in a future edit.
- **A real test-authoring mistake caught by actually running the tests, not by re-reading the code**: several first-draft tests assumed `/a` would complete to `/active` first, reasoning (wrongly) that `COMMAND_HEADS` was alphabetical. It isn't — the array is declared `["/send", "/away", "/active", ...]`, so prefix-filtering preserves that order and `/away` comes first. The tests were wrong, not the implementation; fixed by correcting the expected order rather than reordering the array to match a wrong assumption (the declaration order matching `parse_command`'s own match-arm order is the more useful invariant to keep).
- **Two of the new tests initially had out-of-bounds cursor positions** (byte-counting slipped by one on `"/send #general"` and `"/send #general #gen"`), caught immediately by a real panic on the first `cargo test` run rather than silently passing with the wrong assertion. Fixed by deriving the cursor from `text.len()` instead of a hand-counted literal.
- **Another test-design flaw, same shape as two bugs found in `step10.md`/`step12.md`'s own review**: an initial `command_bar_shows_no_hint_when_there_are_no_suggestions` test asserted the whole rendered screen didn't contain `"Tab:"` — but the footer *already* legitimately renders `"Tab:다음 패널"` regardless of command-bar state, so the assertion failed on unrelated text, not a real bug. Fixed by asserting on `"(Tab:"` (the hint's own opening paren), the actual distinguishing marker.
- **Verification reality**: `cargo check --workspace --all-targets`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`, and `cargo test --workspace` all ran and passed (MinGW toolchain from `step12.md` still on `PATH` this session). `ui` crate: 89 tests (up from 74 before this phase — 16 new `keyboard` tests for `compute_suggestions`/`word_start`/Tab-cycling, 2 new `render` tests for the hint's visibility).

### Bundled in the same session, not originally scoped by this doc

Two real UI bugs were found and fixed while working in this area, at the user's request after noticing them live:
- **The Calendar dock panel (`render_right_dock_placeholder`) was still a hardcoded "not implemented" stub** after `step12.md` shipped a real Calendar adapter — the panel function didn't even take `state`/`model` parameters, so it *couldn't* have shown real data no matter what. Renamed to `render_calendar_panel`, filters `DashboardReadModel.unread_notifications` by `IntegrationSource::Calendar` (the same list Slack/GitHub notifications already flow through via the shared `Projector` — no new data-model field needed), and shows an accurate "not connected, Ctrl+L" message when appropriate. `apply_pane_action`'s Right-dock case (previously hardcoded to `len = 0`, a permanent no-op) now computes its length from the same filter, shared via a new `pub(crate) fn calendar_notifications` in `render.rs` so the two call sites can't drift apart.
- **Team and Calendar panels were completely unreachable below the 120-column sidebar-collapse width** — the collapsed body was hardcoded to always show the Notification panel regardless of `focused_dock`. Fixed so the single visible panel below that width follows `focused_dock`, exactly the same `Tab`/`Shift+Tab`/`Ctrl+1~3` shortcuts already used to move focus on a wide terminal — no new keybinding, no config option, no new `WorkspaceState` field. `docs/01-product/screen-spec.md` §3 also had a stale, never-actually-correct claim that `Ctrl+1`/`Ctrl+4` toggled Team/Calendar specifically (the real bindings are `Ctrl+1`=Team, `Ctrl+2`=Notification, `Ctrl+3`=Calendar, `Ctrl+4`=Bottom/Log); corrected alongside the code fix.
