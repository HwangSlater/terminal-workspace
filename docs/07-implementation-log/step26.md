# Implementation Plan - Phase 26: Calendar grid polish & configurable dock widths

This is a **design document for review — nothing described below has been implemented yet**, per the same process used for Phases 6-25, *except* where a section is explicitly marked already-shipped below (three small, unambiguous fixes were done immediately; the dock-width feature is the part still under review).

## Context

Follow-up from live use of `step25.md`'s month grid view (`Ctrl+M`), plus a direct question about whether panel sizing can be made configurable at all:

1. The grid accepted both `h`/`j`/`k`/`l` and arrow keys, matching every other overlay in this app — but for a *date* grid specifically, the letters read as ordinary text rather than clearly-a-shortcut. Requested: arrow keys only.
2. The highlighted day's event list showed only titles, no time — a real gap once more than one event lands on the same day.
3. The grid popup (`centered_rect(70, 80, area)`) should be bigger.
4. Whether dock widths (Team/Calendar panel widths on the main dashboard) can be made user-configurable at all, so a user who cares more about Calendar than Team can give it more room without a code change.

## Already shipped (small, unambiguous — no design review needed)

- `crates/ui/src/keyboard.rs`: `capture_calendar_grid_input` now matches `KeyCode::Left/Right/Up/Down` only. `h`/`j`/`k`/`l` fall through to the default no-op arm, same as any other unbound key on this overlay.
- `crates/ui/src/render.rs`: the highlighted day's event list now reads `"- {HH:MM} {title}"` via a new `format_occurrence_clock` helper (parallel to the right dock's existing `format_occurrence_time`, but time-only — the date is already the visible heading, so repeating it would be redundant). The status line hint changed from `"h/j/k/l: 날짜 이동  ..."` to `"방향키: 날짜 이동  ..."`.
- `README.md` / `docs/04-extensions/integrations/calendar.md`: updated to describe arrow-key-only navigation and the time-annotated day list.

## Decisions

### 1. Grid popup size: enlarge, still a floating overlay (not a layout redesign)

**Confirmed** (asked directly, two options offered): keep the current model — a centered popup drawn on top of the dashboard, Team/Notification/Calendar docks unchanged underneath — and just increase its share of the screen (from `centered_rect(70, 80, area)` toward something close to full-screen, e.g. `92, 90`). The alternative (swap the *main* dashboard's own layout into a calendar-first arrangement when the grid opens, shrinking Team/Notification specifically for that mode) was explicitly not chosen — bigger change, no clear benefit over an overlay that already fully covers the content that matters once it's this size.

### 2. Dock width customization: config-file scope, confirmed to start now

**Confirmed** (asked directly: design now vs. just answer the feasibility question): yes, start the design now. Scope is **`config.toml`-driven, read at startup, no in-app live-resize command** — the same shape every other numeric tuning knob in this app already has (`sync_interval_secs`, `refresh_rate_ms`, `lookahead_hours`'s *config* default). `lookahead_hours` is the one exception with a live command (`/calendar-range`, `step25.md`), and that was a deliberate, separately-asked-for exception, not the default pattern — dock widths don't need the same treatment unless asked.

New `[layout]` table in `AppConfig` (`crates/config`), two fields:

```toml
[layout]
left_dock_width = 24   # Team panel, default unchanged
right_dock_width = 32  # Calendar panel, default unchanged
```

Both `#[serde(default)]` (existing values keep working unmodified — same "config evolution must not break old files" rule every other settings struct here already follows). `AppConfig::validate()` gets two new checks, same function/pattern that already rejects `refresh_rate_ms < 16`:
- Each field individually: `10..=60`. Below 10, a dock can't show anything legible (checkbox + a few characters); above 60, a single dock would dominate even a wide terminal for no real benefit over the fluid center pane.
- `left_dock_width + right_dock_width <= 60`. `screen-spec.md`'s minimum supported terminal is 80 columns wide (`MIN_WIDTH`); this bound guarantees the center pane still gets at least 20 columns on that smallest supported terminal, rather than being squeezed to near-nothing by two large docks. (On a wider terminal the center pane gets proportionally more, same as today.)

`SIDEBAR_COLLAPSE_WIDTH` (120, the width below which the dashboard shows one panel at a time) stays a fixed constant, **not** derived from the configured widths — the two are only loosely related (collapse is about *total* terminal width being too narrow for three columns at all, not about which specific widths those columns are), and deriving it adds a second thing to reason about for a case (someone setting genuinely extreme widths right at that boundary) that doesn't come up in practice.

`crates/ui` does not gain a dependency on `crates/config` — `TuiRenderer::new` gains two more `u16` constructor parameters (`left_dock_width`, `right_dock_width`), matching the existing "already-resolved values injected in, not raw config structs" pattern every other `TuiRenderer` field already follows (e.g. `initial_slack_status`, `scheduler`). `crates/app/src/main.rs` reads `config.layout.left_dock_width`/`.right_dock_width` and passes them through. `render::render(...)` gains the same two parameters, threaded down into the two `Constraint::Length(...)` call sites that currently reference the `LEFT_DOCK_WIDTH`/`RIGHT_DOCK_WIDTH` constants (which become fallback/default values only, via `AppConfig`'s `#[serde(default = ...)]`, not dead code — every test/call site that doesn't care about custom widths keeps using them).

---

## Proposed Changes

#### [MODIFY] `crates/ui/src/render.rs`
`render_calendar_grid_overlay`: `centered_rect(70, 80, area)` → `centered_rect(92, 90, area)`. `render(...)` and the dashboard's body-layout function gain `left_dock_width: u16, right_dock_width: u16` parameters, replacing the two `Constraint::Length(LEFT_DOCK_WIDTH)`/`Constraint::Length(RIGHT_DOCK_WIDTH)` call sites' literal constants with the passed-in values (the constants remain, as the values `crates/config`'s `#[serde(default = ...)]` resolve to).

#### [MODIFY] `crates/ui/src/lib.rs`
`TuiRenderer` gains `left_dock_width: u16, right_dock_width: u16` fields + constructor parameters, threaded into every `render::render(...)` call site (`draw`, wherever it's invoked from `event_loop`).

#### [MODIFY] `crates/config/src/lib.rs`
New `LayoutSettings` struct (`left_dock_width`, `right_dock_width`, both `#[serde(default = ...)]` matching today's constants), a new `pub layout: LayoutSettings` field on `AppConfig` (`#[serde(default)]`), and two new checks in `AppConfig::validate()`.

#### [MODIFY] `crates/app/src/main.rs`
Read `config.layout.left_dock_width`/`.right_dock_width` where `AppConfig` is already loaded, pass both into `TuiRenderer::new(...)`.

#### [MODIFY] `docs/01-product/screen-spec.md`, `docs/05-operations/configuration.md`
§1's "Fixed width (default: 24/32 columns)" language updated to note these are now configurable via `[layout]`, with the same default values.

---

## Verification Plan

- `crates/config`: `validate()` rejects a dock width below 10, above 60, and a combination whose sum exceeds 60; accepts the existing defaults (24/32) and other in-bounds combinations; an old `config.toml` with no `[layout]` table at all still parses (via `#[serde(default)]`) and resolves to today's 24/32 defaults.
- `crates/ui`: `layout_adapts_across_a_range_of_terminal_sizes`-style `TestBackend` tests, but varying the configured dock widths instead of (or alongside) the terminal size, confirming a custom width actually reaches the rendered buffer's column boundaries.
- Manual: set `right_dock_width = 50` in a real `config.toml`, confirm the Calendar dock visibly widens and long titles (`step26.md`'s wrap fix from the prior session) wrap less aggressively as a result.
- `cargo fmt --all --check` / `cargo check --workspace --all-targets` / `cargo clippy --workspace --all-targets -- -D warnings` / `cargo test --workspace` all green.

---

## Implementation Notes (what actually happened)

Both decisions shipped as designed, no deviations.

**Grid popup**: `centered_rect(70, 80, area)` → `centered_rect(92, 90, area)` in `render_calendar_grid_overlay` -- a one-line change. The week-grid row above the day-event list stayed `Constraint::Length(6)` (unchanged, still exactly enough for 6 week rows); the extra height entirely goes to the day-event list's `Constraint::Min(1)` row, which just has more room to show a day with several events without scrolling.

**Dock widths**: `crates/config` gained `LayoutSettings { left_dock_width, right_dock_width }` (both `#[serde(default)]`, matching the pre-existing `24`/`32` constants) and two new `AppConfig::validate()` checks (per-field `10..=60`, sum `<=60`) -- exactly as designed, following `refresh_rate_ms`'s existing validation pattern. `crates/ui`'s `LEFT_DOCK_WIDTH`/`RIGHT_DOCK_WIDTH` module-level constants were deleted rather than kept as fallback values, once it became clear nothing in non-test code needed them any more (`crates/config` is now the single source of truth for the defaults); they live on only as `DEFAULT_LEFT_DOCK_WIDTH`/`DEFAULT_RIGHT_DOCK_WIDTH` test-local constants in `render.rs`'s test module, so the ~40 existing render tests that don't care about custom widths keep exercising the ordinary 24/32 layout without each hardcoding those numbers.

`render::render(...)` and `TuiRenderer` both gained two plain `u16` parameters rather than `crates/ui` taking a dependency on `crates/config` -- `crates/app/src/main.rs` reads `config.layout.left_dock_width`/`.right_dock_width` once and passes them into `TuiRenderer::new(...)`, the same "already-resolved value injected in" shape every other `TuiRenderer` field already follows.

`SIDEBAR_COLLAPSE_WIDTH` was left untouched, exactly as decided -- still a fixed `120` regardless of configured dock widths.

Test coverage added: `crates/config` gained 6 tests (`parses_real_layout_config`, `a_pre_step26_config_toml_without_a_layout_table_still_parses`, and one rejection test per validation rule -- below the floor, above the ceiling, and a combination whose sum is too large despite each field being individually in-bounds), bringing that crate to 24 tests total (up from 18). `crates/ui` gained a cell-position test (`configured_dock_widths_change_where_the_body_panels_split`, checking exact buffer coordinates rather than text search, since the thing under test *is* where a column boundary lands) and a test tying the dock-width feature back to this same phase's wrap-width fix (`a_wider_right_dock_width_wraps_long_titles_less` -- a title that wraps at the default 32-column dock fits on one line at 60), bringing that crate to 157 tests (up from 155 after the arrow-key/time-display changes earlier in this phase, 150 before this phase started). Full `cargo fmt`/`check`/`clippy -D warnings`/`test --workspace` green throughout.
