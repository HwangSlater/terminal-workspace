# Implementation Plan - Phase 19: UI Polish Pass

This is a **design document for review — nothing described below has been implemented yet**, per the same process used for Phases 6-18.

## Context

Before starting the AI Assistant panel (item 4 of the post-v1.0 sequential plan), the user asked for a general review of the existing UI: add what's missing, fix what needs fixing, change the design where it helps, with an explicit "make it user-friendly" (사용자 친화적으로) framing. This isn't a new domain feature — it's a cross-cutting polish pass over `crates/ui`, informed by re-reading `render.rs`/`keyboard.rs` end to end with fresh eyes.

Six concrete findings, all additive or low-risk:

1. **The `/pomodoro` command is undocumented in the in-app help overlay.** `HELP_CATEGORIES` (`render.rs`) has categories for 탐색/명령줄/Slack/GitHub/Calendar/기타, but nothing for Pomodoro — a shipped, working feature (`step18.md`) that a user pressing `?` would never learn exists. Real gap, not a design choice.
2. **The log panel is nearly useless at its current height.** The bottom dock row is `Constraint::Length(3)` — 2 rows for borders, 1 for content. `step17.md` built a real 200-line ring buffer and a live tail, but only the single most recent line is ever visible, defeating the point of a "log panel" (you can't see what happened, only what's happening *right now*). `docs/01-product/screen-spec.md` §1 describes the Bottom Dock only as "fluid width container for logs" with no fixed height, so there's no spec constraint against growing it.
3. **No color coding for team presence status.** The header already color-codes Slack/GitHub/Calendar connection status (green/yellow/red/gray), but the Team panel's `[활동중]`/`[자리비움]`/etc. labels are always plain white — the one place presence is actually listed doesn't use the same at-a-glance color language the header established.
4. **No color coding for log lines by level.** `tracing`'s compact formatter writes `INFO`/`WARN`/`ERROR`/`DEBUG`/`TRACE` as a plain-text prefix on every line (ANSI disabled for the buffer, per `step17.md`'s `with_ansi(false)`); the log panel renders them all in the default color, so a `WARN`/`ERROR` doesn't stand out from routine `INFO` noise in the one place you'd want it to.
5. **No color coding for notification priority.** `NotificationItem.priority: PriorityLevel` (Low/Medium/High) exists and is stored, but the Notification panel never reads it — every item renders identically regardless of urgency.
6. **Dock titles don't show counts.** "팀"/"알림"/"캘린더" give no indication of how many items are inside without focusing the panel — a quick glance can't tell "3 unread" from "0 unread."

---

## Decisions

### 1. Help overlay gains a Pomodoro category

**Proposed**: add a 7th `HELP_CATEGORIES` entry, "Pomodoro", listing `/pomodoro start|pause|reset` the same way other command-bar syntax is documented in the existing "명령줄" category. No trade-off — pure gap-fill.

### 2. Log panel grows from 1 visible content row to 6

**Superseded during user review — see Implementation Notes at the bottom of this doc.** Kept here as the original proposal for context; the actual shipped design removes the always-visible row entirely in favor of a `Ctrl+4` overlay.

**Proposed**: change the bottom dock's layout constraint from `Constraint::Length(3)` to `Constraint::Length(8)` (6 content rows + 2 border rows). This shrinks the body row (Team/Notification/Calendar) by 5 rows on an unchanged terminal height — real estate has to come from somewhere. 6 was picked as "enough to actually read a short recent history without dominating the screen on an 24-line-minimum terminal" (`MIN_HEIGHT = 24`: header 1 + body 5+ + log 8 + command bar 1 + footer 1 = 16, leaving the body's `Constraint::Min(5)` comfortably satisfiable). Flagging this one for explicit confirmation since it's the only change here with a real space trade-off.

### 3. Presence status colored like connection status

**Proposed**: reuse the same semantic colors already established for connection status — `Active`→Green, `Meeting`/`Lunch`→Yellow, `Away`→Yellow (dimmer distinction not worth a 5th color), `Offline`→DarkGray. New `presence_status_color(PresenceStatus) -> Color` function alongside the existing `presence_status_label`.

### 4. Log lines colored by level, parsed from the line prefix

**Proposed**: `tracing`'s compact format puts the level right after the timestamp (`2026-... INFO ...`, `2026-... WARN ...`, `2026-... ERROR ...`). A small `log_line_color(&str) -> Color` scans for `" ERROR "`/`" WARN "`/`" DEBUG "`/`" TRACE "` substrings (in that priority order, since e.g. "WARN" doesn't contain "ERROR") and defaults to the existing plain color for `INFO`/anything unrecognized — Red/Yellow/DarkGray/DarkGray/default respectively. Rendered per-line as styled `Line`s instead of one joined `Paragraph` string (ratatui supports per-line styling directly).

### 5. Notification title colored by priority

**Proposed**: `PriorityLevel::High`→Red, `Medium`→default (no color change, most notifications are Medium and shouldn't all shout), `Low`→DarkGray. Mirrors the existing `connection_status_label`/`presence_status_label` pattern with a new `priority_color(PriorityLevel) -> Color`.

### 6. Dock titles show a count when non-empty

**Proposed**: `dock_block` gains an optional count that formats as `"팀 (3)"` when non-zero, plain `"팀"` when zero (avoids permanent visual noise like "팀 (0)" on an empty workspace). Applied to Team (member count), Notification (unread count), Calendar (upcoming count) — Log panel doesn't use `dock_block` (no focus-highlight border currently) so it's out of scope here.

---

## Proposed Changes

#### [MODIFY] `crates/ui/src/render.rs`
- `HELP_CATEGORIES`: new Pomodoro entry (Decision 1).
- `render()`: bottom dock constraint `Length(3)` → `Length(8)` (Decision 2).
- `presence_status_color()` (new) + `render_team_panel` uses it per-row (Decision 3).
- `render_log_panel`: switch from joined `Paragraph` text to a `List`/styled `Line`s per row, colored via new `log_line_color()` (Decision 4).
- `priority_color()` (new) + `render_notification_panel` uses it per-row (Decision 5).
- `dock_block`: new optional count parameter; call sites in `render_team_panel`/`render_notification_panel`/`render_calendar_panel` pass their real counts (Decision 6).

#### [MODIFY] `docs/01-product/screen-spec.md`
Note the Bottom Dock's actual current height (8 rows / 6 content rows) now that it's a deliberate number, not just "fluid."

---

## Verification Plan

- Existing render tests updated where the log-panel-height and dock-title-count changes touch known assertions (e.g. `log_panel_shows_only_the_most_recent_lines_that_fit` needs updating for 6 rows instead of 1).
- New tests: help overlay shows the Pomodoro category; team panel colors an Active member green and an Offline member gray (via style inspection, not text scraping, following this file's existing `TestBackend` pattern where color matters); log panel colors an ERROR line distinctly from an INFO line; notification panel colors a High-priority item distinctly from Low; dock titles show correct counts including the zero-count "no suffix" case.
- `cargo fmt --all --check` / `cargo check --workspace --all-targets` / `cargo clippy --workspace --all-targets -- -D warnings` / `cargo test --workspace` all green.

---

## Implementation Notes (what actually happened)

Decisions 1, 3, 4 (partially), 5, 6 shipped close to as designed. **Decision 2 changed substantially during review** — worth documenting since the design doc above still describes the original (superseded) proposal.

**Decision 2 superseded**: growing the always-visible strip to 6 rows was rejected during user review in favor of removing the persistent bottom row entirely and replacing it with a `Ctrl+4`-triggered overlay (`OverlayKind::LogViewer`, mirroring how `Ctrl+S`/`Ctrl+G`/`Ctrl+L` open their setup overlays directly rather than "focus a dock, then Enter"). The deciding fact that came out of that discussion: `UiDockSlot::Bottom` was already fully inert when focused (`apply_pane_action`'s `Bottom => 0` arm made `j`/`k`/`Enter` all no-ops), so there was no real interactive behavior being displaced. Net effect: the body panels (Team/Notification/Calendar) get 100% of the vertical space back instead of losing 5 rows, and the log viewer itself is far more useful (up to ~20 lines in a `centered_rect(80, 70)` popup vs. the originally-proposed 6). `Tab`/`Shift+Tab` dropped from a 4-element to a 3-element `DOCK_CYCLE`; `UiDockSlot::Bottom` itself was kept (unused by the visible layout now, but `docking_registry`/future plugin panel registration per ADR-0012 still reference it, and removing the enum variant would have widened the blast radius for no benefit this phase).

**Decision 4's per-line log coloring absorbed into the overlay.** The design doc's per-decision split (Decision 2 = height, Decision 4 = color) collapsed into one implementation unit once Decision 2 became "build an overlay" — `render_log_viewer_overlay` does both the layout and the `log_line_color` styling in one function, since there's no longer a separate always-visible-strip code path to keep the color logic apart from.

**Bug found in the test helper, not the product code**: the first version of `fg_color_of` (the new style-inspection test helper this phase needed) matched a color needle by its *first character only*. Two tests failed with wrong-but-plausible colors — `assert_eq!(high_color, Color::Red)` got `DarkGray`, `assert_eq!(error_color, Color::Red)` got `Reset`. Root cause: `centered_rect` popups don't cover the full screen, so header/footer text stays visible around them, and a single-letter search found an unrelated earlier match — the 'u' in the header's "Workspace" (before "urgent"), the 's' in "Workspace" itself (before "something"). Fixed by matching the *whole* needle as a contiguous run. That in turn hit a second issue: Korean glyphs render with a padding cell (a literal space) after every double-width character (the same fact `contains_ignoring_whitespace` was already written around), so a naive contiguous-cell match failed on `"활동중"`. Fixed by building a whitespace-stripped candidate string per row with an index map back to original cell positions — same whitespace-stripping convention `contains_ignoring_whitespace` already established, extended to also recover a cell index for color lookup.

Final state: 8 new/rewritten tests in `crates/ui/src/render.rs`'s color/count/overlay coverage plus the 3 rewritten log tests (109 total in `ui`, up from 101), 2 new tests in `crates/ui/src/keyboard.rs` (`Ctrl+4` opens the overlay without touching `focused_dock`; `Esc` closes it). `docs/01-product/screen-spec.md`, `docs/02-architecture/keyboard.md` (which had a pre-existing, never-actually-true "Ctrl+4 focuses CI/CD Build Status Panel" line predating this phase — corrected while already touching that row), `docs/03-domain/workspace-state.md`, and ADR-0012 all updated. Full `cargo fmt`/`check`/`clippy -D warnings`/`test --workspace` green with no regressions outside `crates/ui`.
