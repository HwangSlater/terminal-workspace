# Implementation Plan - Phase 25: Calendar UX — lookahead range, rename, month grid view

This is a **design document for review — nothing described below has been implemented yet**, per the same process used for Phases 6-24.

## Context

Follow-up from `step24.md`'s multi-calendar work, plus a real bug found in live use (Calendar panel showed no date/time — fixed separately, `398f9c9`). Three requests came out of actually using the feature:

1. Change how far ahead the Calendar panel looks (`lookahead_hours`) without editing `config.toml` by hand.
2. Rename a connected calendar without removing and re-adding it.
3. A real month/week calendar grid, not just a flat list of upcoming reminders.

---

## Decisions

### 1. Lookahead range: `/calendar-range <hours>` command

**Confirmed, one detail changed during implementation**: a new command-bar command, `/calendar-range <hours>`, matching the existing `/pomodoro`-style syntax. Dispatches to a narrow port defined in `crates/integration`, implemented directly on `CalendarAdapter` — no `crates/app` bridge needed, same reasoning `IntegrationConnector`/`Picker` already established in `step24.md`'s Implementation Notes: only traits defined in `crates/commands` need a bridge, and this one doesn't have to be. Updates `CalendarConfig.lookahead_hours` and restarts polling with the new value — mirrors `CalendarAdapter::keep_only`'s existing shutdown-then-start pattern, since `CalendarPoller` snapshots its config once at `start()` time rather than re-reading it every cycle. Shipped as one method (`set_lookahead_hours`) on a single combined `CalendarManager` trait alongside Decisions 2/3's ports, rather than the separately-named `CalendarLookaheadSetter` originally sketched here — see Implementation Notes.

### 2. Rename: label-only edit from the `Ctrl+K` picker, not a full re-add

**Confirmed**: pressing `e` on the highlighted row in the `Ctrl+K` picker overlay opens a small rename prompt (plain text, pre-filled with the current label) — not the URL. Changing a calendar's *URL* still means remove (`Ctrl+K`, uncheck, save) and re-add (`Ctrl+L`) — a URL change is rare, and re-pasting a fresh secret address from Google Calendar is not meaningfully more work than an in-place "edit a masked field you can't see the original value of" flow would be, which is confusing UX for no real benefit. A label typo, by contrast, is common and shouldn't need a full disconnect/reconnect.

### 3. Month grid view: a new full-screen overlay, `Ctrl+M`

**Confirmed**: the current Calendar panel (right dock, 32 columns) is too narrow for a readable month grid — this is a new overlay, not a replacement for that panel, opened via `Ctrl+M` (unclaimed; `Ctrl+D` already means "focus the Calendar dock," an alias for `Ctrl+3`, so a different letter was needed). Read-only in this phase: navigate between months, see which days have at least one event (a marker), no in-overlay event creation/editing — that's a much bigger feature (writing to a calendar you only have read-only iCal access to isn't even possible under this integration's auth model at all, `step12.md` Decision 1).

### 4. Grid shape: month, not week

**Confirmed** (asked directly): a month grid, matching the general shape most calendar apps default to. Navigation: `h`/`j`/`k`/`l` (and arrows) move the day cursor within the displayed month, clamped to its real day count (no wraparound into an adjacent month — that would need a re-fetch mid-navigation, deliberately kept out of this phase); `[`/`]` explicitly change the displayed month and re-fetch.

---

## Proposed Changes

#### [MODIFY] `crates/integration/src/lib.rs`, `crates/integration/src/calendar.rs`
New `CalendarManager` trait (`set_lookahead_hours`, `rename`, `events_in_range`); `CalendarAdapter` implements it directly (no bridge, per Decision 1's note). `poll_one`/`events_in_range` share a `fetch_calendar_feed` helper extracted from the old single-purpose `poll_one` body.

#### [MODIFY] `crates/commands/src/lib.rs`
New `Command::SetCalendarLookaheadHours`/`Command::RenameCalendar`; `WorkspaceCommandHandler` gains a `calendar_manager: Option<Arc<dyn CalendarManager>>` field and dispatches both.

#### [MODIFY] `crates/ui/src/state.rs`, `crates/ui/src/keyboard.rs`, `crates/ui/src/render.rs`, `crates/ui/src/lib.rs`
`CalendarRenameState`/`OverlayKind::CalendarRename` (Decision 2); `CalendarGridState`/`OverlayKind::CalendarGrid`/`days_in_month`/`shift_month` (Decisions 3-4); `/calendar-range` command parsing; `Ctrl+M` global shortcut; `TuiRenderer` gains a `calendar_manager` field for the grid's on-demand fetch.

#### [MODIFY] `crates/app/src/main.rs`
Wire `calendar_adapter` into `WorkspaceCommandHandler::new`'s new `calendar_manager` argument and `TuiRenderer::new`'s new one.

---

## Verification Plan

- Unit tests: `set_lookahead_hours` actually updates the config and keeps polling; `rename` updates and persists the label without touching the URL or restarting polling; renaming an unknown id is a real error; `days_in_month`/`shift_month` across normal months, February in leap vs. non-leap years, and the December↔January year rollover in both directions; grid cursor movement clamps to the real month length without wrapping; `[`/`]` reset the cursor and produce a fresh fetch request.
- Manual verification: `/calendar-range 48` and confirm the panel's window actually changes; rename a calendar via `Ctrl+K`+`e` and confirm the new label shows immediately and survives a restart; open `Ctrl+M`, confirm real connected-calendar events show as markers on the correct days.
- `cargo fmt --all --check` / `cargo check --workspace --all-targets` / `cargo clippy --workspace --all-targets -- -D warnings` / `cargo test --workspace` all green.

---

## Implementation Notes (what actually happened)

Decisions 2 and 4 shipped exactly as designed. Decision 3 shipped as designed with one addition (Decision 4 formalized the month-vs-week question the original Decision 3 text left implicit). Decision 1's *outcome* (a `/calendar-range` command that updates config and restarts polling) shipped as designed; the *port shape* changed.

**Consolidated into one `CalendarManager` trait, not three separate ports.** The original per-decision framing implied `set_lookahead_hours` might get its own narrow trait (mirroring `IntegrationConnector`'s "one trait, one job" precedent). During implementation, `rename` and `events_in_range` (the grid view's fetch) turned out to need the exact same "defined in `crates/integration`, no bridge needed" shape for the exact same reason (`step24.md`'s Implementation Notes: `crates/commands` depends on `crates/integration`, not the reverse, so any trait `CalendarAdapter` implements directly — as opposed to through a bridge — must live in `crates/integration`). Three near-identical single-method traits would have been more ceremony than one `CalendarManager` trait with three methods, for no real separation-of-concerns benefit (all three are "Calendar-specific management operations," the same category).

**A real refactor, not just an addition**: `CalendarPoller::poll_one`'s fetch-and-parse logic (request, status check, rate-limit handling, body read, ICS parse — all with `tracing::warn!` diagnostics on every failure path, from the earlier live-bug fix) was extracted into a standalone `fetch_calendar_feed` function so `events_in_range` could reuse it exactly, rather than duplicating five different failure-logging call sites. `poll_one` itself shrank to "call the shared fetch, then do its own occurrence-expansion-and-dedup-and-publish work" — the part that's actually specific to the reminder poll loop, not shared with a one-shot grid-view fetch.

**Fetch range uses UTC month boundaries, not local-midnight**: computing a correct local-midnight-to-UTC conversion for the month's start/end would need `chrono`'s ambiguous/nonexistent-local-time handling across DST transitions (`TimeZone::from_local_datetime` returning `LocalResult::Ambiguous`/`None` in edge cases) — real complexity for a fetch range that only needs to be "generous enough to cover the displayed month." The part that actually has to be correct in local time — which day cell an event's marker lands in — is computed separately and correctly via `local_day_of`, unaffected by this simplification.

Final state: 8 new tests in `crates/integration` (71 total, up from 66 pre-`step24.md`) covering `set_lookahead_hours`/`rename`/the unknown-id error; 7 new pure-function tests in `crates/ui/src/state.rs` for `days_in_month`/`shift_month`/`CalendarGridState::default()`; 8 new keyboard tests for grid navigation and month-shift; 4 new render tests for the grid overlay's loading/failure/populated states. 150 tests in `crates/ui` (up from 130), 29 in `crates/commands` (up from 27). Full `cargo fmt`/`check`/`clippy -D warnings`/`test --workspace` green with no regressions.
