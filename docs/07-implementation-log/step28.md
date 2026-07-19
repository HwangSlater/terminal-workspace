# Implementation Plan - Phase 28: Calendar visual polish

Small enough in scope (styling only, no new state/behavior) to skip the
full Decisions/AskUserQuestion cycle used for Phases 6-27 — implemented
directly per a direct request ("달력 캘린더는 디자인 예쁘게 해봐" — "make the
calendar's design pretty"), documented here after the fact per
`development.md`'s own rule that a design pass still needs a record even
when it didn't need up-front confirmation.

## Context

Live use of `step27.md`'s enlarged, centered grid surfaced that the grid
was structurally bigger but visually flat — every day number the same
color regardless of weekend/today/event status, a left-aligned unstyled
title, and a plain `"- HH:MM title"` event list with no visual hierarchy.
Standard calendar-app conventions (weekend coloring, a distinct "today"
marker separate from the current selection) were all missing.

## Changes

### `crates/ui/src/render.rs` — grid overlay (`render_calendar_grid_overlay`)

- **Title**: centered (`Title::from(...).alignment(Alignment::Center)`,
  bold), with the `Esc: 닫기` hint moved to a separate right-aligned title
  segment instead of being appended to the centered one — both are real
  `ratatui::widgets::block::Title`s on the same `Block`.
- **Weekend coloring**: Sunday (red) and Saturday (blue) — the same
  convention nearly every calendar app uses — applied to both the weekday
  header row and the day-number cells beneath it.
- **Today, distinct from the cursor**: a new `is_today` check (`grid.year`/`.month`
  match `chrono::Local::now()`'s real date, and the day number matches)
  renders bold cyan, independent of which day is currently selected — a
  real calendar always shows today, not just whatever's currently
  highlighted. Style priority: cursor (reversed+bold) > today (cyan+bold)
  > has-event (yellow) > weekend (red/blue) > default.
- **Event marker**: `•` → `●` (a more visible filled circle at this cell
  width).
- **Day-events heading**: `"{day}일:"` → `"{day}일 ({weekday})"` (bold),
  reading like an actual date instead of a bare number; each event line
  gets a yellow `●` bullet instead of a plain `-`.

### `crates/ui/src/render.rs` — right dock (`render_calendar_panel`)

- The leading timestamp (`"7/20 14:00"`) now renders dimmed (`Color::DarkGray`)
  relative to the title, when the line fits on one row without wrapping —
  the title is the part worth scanning, the timestamp is supporting
  detail. Falls back to the existing plain-text wrapped rendering
  (`step26.md`'s fix) for a title too long to fit on one line, since
  `wrap_to_width` operates on a flat string and re-deriving per-span
  boundaries across wrapped rows isn't worth it for what's already a
  fallback path.

## Verification

- `calendar_grid_overlay_colors_sunday_in_the_weekday_header`: Sunday's
  header label renders `Color::Red` (scans the buffer for the first `일`
  glyph top-to-bottom, which is unambiguous in a no-events fixture since
  the day-events list below doesn't repeat that character).
- `calendar_grid_overlay_shows_the_weekday_name_next_to_the_cursor_day`:
  the cursor day's heading includes its real weekday name, computed the
  same way the production code does rather than hardcoded, so the
  assertion can't silently drift onto the wrong weekday if the fixture
  date ever changes.
- Existing grid/panel tests (title text, event text, wrapping, centering,
  content-sized Team dock) all still pass unmodified — this phase changed
  presentation, not the underlying data or layout math those tests cover.
- `cargo fmt --all --check` / `cargo check --workspace --all-targets` /
  `cargo clippy --workspace --all-targets -- -D warnings` /
  `cargo test --workspace` all green. `crates/ui`: 161 tests (up from 159).
