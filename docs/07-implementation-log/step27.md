# Implementation Plan - Phase 27: Grid navigation simplification, bigger centered grid, content-sized Team dock

This is a **design document for review — nothing described below has been implemented yet**, per the same process used for Phases 6-26.

## Context

Follow-up from live use of `step26.md`'s enlarged grid popup and configurable dock widths:

1. The popup (`centered_rect(92, 90, area)`) is big, but the actual day-number grid inside it stayed a small, fixed-size, left-aligned block — most of the popup is empty space around it, not around a bigger grid.
2. Up/Down still moves the day cursor by a week inside the grid overlay; only asked to be simplified once already (`step26.md`: `h`/`j`/`k`/`l` letters removed). This time: remove the week-jump behavior itself, regardless of which key drives it — Left/Right (day-by-day) only.
3. The Team dock always renders at the full configured `left_dock_width` (`step26.md`) even when the actual team roster needs far less — wasted width that could go to the Notification panel instead.

## Decisions

### 1. Grid overlay: enlarge and horizontally center the day-grid itself

**Confirmed**: the weekday header + day-number grid render into a fixed-width block computed from real content (7 columns × a wider per-day cell than today's, so each day gets more visual weight), horizontally centered within the popup's inner width via a nested `Layout` split (`[Min(0), Length(grid_width), Min(0)]`) rather than left-flush `Paragraph`s spanning the full inner width. Stays pinned to the top of the popup (not vertically centered) — only asked to fix "small and left-aligned," not to reposition it away from the top. Week rows also get more vertical breathing room (a blank spacer line between weeks) so the grid reads as genuinely bigger, not just wider.

### 2. Grid overlay: remove week-jump navigation entirely

**Confirmed** (asked directly: keep Up/Down as a week-jump, or drop it): drop it. `capture_calendar_grid_input` keeps `KeyCode::Left`/`KeyCode::Right` (day-by-day, clamped to the month, unchanged) and removes the `KeyCode::Up`/`KeyCode::Down` arms entirely — they fall through to the same no-op default arm `h`/`j`/`k`/`l` already do. `[`/`]` (month switching) are unaffected.

### 3. Team dock: content-sized width, freed width absorbed by the Notification panel

**Confirmed** (asked directly, two shapes offered: split the left dock vertically to show a Notification/Calendar preview underneath Team, vs. shrink Team's *width* and let the already-fluid Center panel absorb the difference): the second, simpler shape. `left_dock_width` from `config.toml` (`step26.md`) becomes a **ceiling**, not a fixed value — the Team dock actually renders at `min(configured left_dock_width, natural content width)`, floored at 10 (matching `AppConfig::validate()`'s own floor, so it never collapses to an unreadably thin sliver). Natural content width is computed from the real roster each frame: the longest `"{display_name} [{status_label}]"` line (or the empty-state text when there are no team members), plus 2 for the block's borders. No config changes needed — `left_dock_width`'s meaning shifts slightly (max, not exact) but every existing value in an existing `config.toml` keeps behaving the same for anyone whose roster is already wider than their configured width. The Notification (Center) panel needs no changes at all — it's already `Constraint::Min(0)` (fluid), so it automatically claims whatever width Team doesn't use. Calendar's `right_dock_width` is untouched (out of scope — nothing asked about it this phase).

---

## Proposed Changes

#### [MODIFY] `crates/ui/src/render.rs`
- `render_calendar_grid_overlay`: wider per-day cell format, spacer lines between weeks, a nested horizontal `Layout` centering the fixed-width grid block within the popup.
- `capture_calendar_grid_input` (`keyboard.rs`, not `render.rs` — see below): drop the `Up`/`Down` arms.
- `render(...)`: computes `team_panel_natural_width(model)` before building the body `Layout`, uses `left_dock_width.min(natural_width).max(10)` as the actual `Constraint::Length` for Team instead of `left_dock_width` directly. Notification/Calendar constraints unchanged (`Constraint::Min(0)` / `Constraint::Length(right_dock_width)`).

#### [MODIFY] `crates/ui/src/keyboard.rs`
`capture_calendar_grid_input`: remove the `KeyCode::Up`/`KeyCode::Down` match arms.

---

## Verification Plan

- `crates/ui`: grid-overlay render tests confirming the day-grid is horizontally centered (not flush-left) at a couple of popup widths; keyboard tests confirming Up/Down are now a no-op (mirroring `step26.md`'s existing `calendar_grid_h_j_k_l_letters_do_not_move_the_cursor` test, extended to include the arrow keys) while Left/Right still move the cursor; a render test confirming a short team roster renders a Team dock narrower than the configured `left_dock_width`, and the Notification panel's border shifts left to match (same cell-position-check technique `configured_dock_widths_change_where_the_body_panels_split` used).
- `cargo fmt --all --check` / `cargo check --workspace --all-targets` / `cargo clippy --workspace --all-targets -- -D warnings` / `cargo test --workspace` all green.

---

## Implementation Notes (what actually happened)

All three decisions shipped as designed, no deviations.

**Grid enlargement**: per-day cell width went from 4 to 6 columns (`" {d:>2}{marker}  "`), and a blank spacer `Line` was inserted after each of the 6 week rows, doubling the grid's vertical footprint (`GRID_HEIGHT = 12` vs. the old fixed `6`). Both the weekday header and the week-grid `Paragraph`s render into a horizontally-centered sub-`Rect` (`[Constraint::Min(0), Constraint::Length(GRID_WIDTH), Constraint::Min(0)]` split of their row), rather than spanning the popup's full inner width left-aligned. The weekday header is now built from a `Vec<Span>` (one per weekday, each padded to the same 6-column cell width as the day cells below it) instead of one hardcoded string, so the two rows stay pixel-aligned by construction rather than by coincidence.

**Navigation**: `KeyCode::Up`/`KeyCode::Down` match arms removed from `capture_calendar_grid_input` entirely (not redirected anywhere) — they fall through to the same no-op default arm `h`/`j`/`k`/`l` already used since `step26.md`. The status line hint changed from `"방향키: 날짜 이동"` to `"←/→: 날짜 이동"`.

**Team dock**: new `team_panel_natural_width(model)` computes the longest `"{display_name} [{status_label}]"` line's `unicode-width` (or the empty-state text's width when the roster is empty), `+2` for borders. `render(...)` uses `team_panel_natural_width(model).min(left_dock_width).max(10)` as the actual `Constraint::Length` for the Team dock instead of `left_dock_width` directly — `left_dock_width` (`step26.md`) keeps its meaning as the *configured* value/ceiling, it just no longer dictates the *rendered* width by itself. No changes needed to the Notification panel (`Constraint::Min(0)`, already fluid) or to `crates/config` (the config field's semantics shifted slightly — ceiling, not exact — but every existing `config.toml` value keeps behaving identically for a roster already wider than it).

Test coverage added: `crates/ui` gained a cell-position test proving a short roster renders the Team dock narrower than a wide configured ceiling (with the Notification panel's border shifted left to match, and explicitly *not* still sitting at the old ceiling position); a cell-scan test proving the weekday header renders well right of the popup's left edge (centered) rather than flush against it; and an Up/Down no-op test alongside the existing `h`/`j`/`k`/`l` one. One pre-existing test (`calendar_grid_down_and_up_arrows_move_by_a_week_clamped_to_the_month`) was replaced rather than deleted outright, since the behavior it exercised no longer exists — its replacement (`calendar_grid_up_and_down_arrows_no_longer_move_the_cursor`) asserts the opposite. `crates/ui` ends this phase at 159 tests (up from 157 after `step26.md`). Full `cargo fmt`/`check`/`clippy -D warnings`/`test --workspace` green throughout.
