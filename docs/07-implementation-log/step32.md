# Implementation Plan - Phase 32: Team moves to the header; Calendar never wraps

This is a **design document for review — nothing described below has been
implemented yet**, per the same process used for Phases 6-31. Unlike most
prior phases, the Decisions here were worked out through a set of visual
mockups (an Artifact, iterated live in conversation) rather than
`AskUserQuestion` text options — the user asked to see full-screen
renders of each candidate before picking. Confirmed directly: proceed
with the combined design below ("Option 5" in that mockup).

## Context

Follow-up from `step31.md`'s Team panel polish. Two things remained
unresolved even after that phase:

1. A real screenshot from live use showed the Team panel's box still
   stretching the full body height with a lot of visible blank space
   below just 2 members, even though its *width* now shrinks to content
   (`step27.md`). Explicitly reframed mid-conversation: rather than fill
   that blank space with more content, pull Team out of the three-column
   body row entirely.
2. Separately, but related: the Calendar panel's fixed 32-column width
   (competing with Team for the same row) routinely forced long event
   titles (especially with a `[label]` prefix, `step24.md`) to wrap —
   explicit request that this stop happening entirely, wrapping or not.

## Decisions

### 1. Team panel: out of the body row, into the header

**Confirmed**: Team is no longer one of the three body docks. A member
list this short (the user's own estimate: "realistically 5 people, max")
never needed a tall scrollable panel to begin with — confirmed by
checking `apply_pane_action`: selecting/moving the cursor within the Team
panel has never done anything beyond changing which row is highlighted,
so nothing functional is lost by dropping it from the focus-navigable
dock set. Team members render as a single compact header line instead
(colored presence dot + name per member, `step31.md`'s dot convention
reused directly), truncated with `truncate_with_ellipsis` if the whole
roster doesn't fit on one line.

`Tab`/`Shift+Tab`'s `DOCK_CYCLE` drops `Left`, leaving `[Center, Right]`
— mirrors the existing precedent for `Bottom` (already excluded from the
cycle since `step19.md`, its `UiDockSlot` variant kept but unreachable in
practice). `Ctrl+1` (previously "focus Team") is removed rather than
renumbered — `Ctrl+2`/`Ctrl+3` keep their existing meanings
(Notification/Calendar) unchanged, so muscle memory for those two
survives; only the now-meaningless binding goes away.

### 2. Calendar panel: content-sized width, truncate instead of wrap

**Confirmed**: the Calendar dock gets the same treatment `step27.md`
already gave the Team dock — `Constraint::Length` sized to
`calendar_panel_natural_width(model).min(right_dock_width).max(floor)`
instead of a fixed configured value, so a typical day's events fit on one
line without needing the ceiling at all. For the rare event whose title
is long even at the ceiling, `render_calendar_panel` switches from
`wrap_to_width` (`step26.md`) to `truncate_with_ellipsis` (`step31.md`) —
explicitly requested: no wrapping under any circumstance, not even as a
last resort for an extreme outlier.

Freed from competing with Team for the same row, `right_dock_width`'s
ceiling can be meaningfully larger than the old default (32) without
starving the Notification column — raised to 60.

### 3. `config.toml`: `left_dock_width` removed, `right_dock_width` stays (as Calendar's ceiling)

**Confirmed as a consequence of Decision 1**: `[layout].left_dock_width`
no longer configures anything (Team isn't a `Constraint::Length` dock
anymore) — removed from `LayoutSettings` rather than left as a silently
ignored field. `right_dock_width` keeps its name (Calendar is still the
visually-rightmost column) and its "ceiling, not exact value" semantics
from `step27.md`'s Team precedent, just retargeted to Calendar and
defaulted higher (60, was 32). `AppConfig::validate()`'s combined
`left_dock_width + right_dock_width <= 60` check is replaced by a
standalone `right_dock_width <= 60` bound (still leaving `MIN_WIDTH`'s
80-column floor at least 20 columns for the fluid Notification column,
same reasoning `step26.md` originally used).

---

## Proposed Changes

#### [MODIFY] `crates/config/src/lib.rs`
`LayoutSettings` loses `left_dock_width`; `right_dock_width`'s default
becomes 60. `AppConfig::validate()`'s bound simplifies to just
`right_dock_width`.

#### [MODIFY] `crates/ui/src/state.rs`
`DOCK_CYCLE` (or wherever `Tab` cycling is defined) drops `Left`.
`WorkspaceState::focused_dock`'s default becomes `Center` (was `Left`,
now unreachable as a focus target).

#### [MODIFY] `crates/ui/src/keyboard.rs`
Remove the `Ctrl+1` → focus-Team binding. `capture_calendar_panel`-adjacent
navigation logic (`apply_pane_action` in `crates/ui/src/lib.rs`) drops its
`UiDockSlot::Left` arm.

#### [MODIFY] `crates/ui/src/render.rs`
`render_header` gains the Team member line. `render(...)`'s body layout
becomes 2 columns (`Constraint::Min(0)` Notification, `Constraint::Length(calendar_width)`
Calendar) instead of 3. New `calendar_panel_natural_width`, mirroring
`team_panel_natural_width`. `render_calendar_panel` switches from
`wrap_to_width` to `truncate_with_ellipsis`. `render_team_panel` is
deleted (its logic moves into the header). The narrow-terminal
sidebar-collapse path (`collapse_sidebars`) drops its `UiDockSlot::Left`
arm — only Notification/Calendar are still real collapse targets.

#### [MODIFY] `crates/app/src/main.rs`
Drop `config.layout.left_dock_width` from the `TuiRenderer::new(...)` call
site (one fewer argument).

#### [MODIFY] `README.md`, `docs/02-architecture/keyboard.md`, `docs/01-product/screen-spec.md`, `docs/05-operations/configuration.md`
Reflect the removed `Ctrl+1` binding, the header's new Team line, the
2-column body, and `[layout]`'s narrowed schema.

---

## Verification Plan

- `crates/config`: `right_dock_width`'s bound tested the same way
  `step26.md`'s combined bound was; a `config.toml` with a stray
  `left_dock_width` key still parses (unknown TOML keys are just ignored
  by `toml`/`serde` — not an error) rather than crashing an old file.
- `crates/ui`: a render test proving the header shows the team roster; a
  cell-position test proving the body is 2 columns, not 3, at a given
  `right_dock_width`; `calendar_panel_natural_width`'s pure-function
  behavior (mirrors `team_panel_natural_width`'s existing tests); a
  regression test proving a long Calendar title is truncated, never
  wrapped, even at a narrow `right_dock_width`; `Tab`/`Shift+Tab` cycling
  test updated to 2 stops instead of 3; `Ctrl+1` no longer opens/focuses
  anything.
- `cargo fmt --all --check` / `cargo check --workspace --all-targets` /
  `cargo clippy --workspace --all-targets -- -D warnings` /
  `cargo test --workspace` all green.

---

## Implementation Notes (what actually happened)

All three decisions shipped as designed, no deviations.

**Team → header**: `render_team_panel` and `team_panel_natural_width`
deleted outright (not deprecated/kept-around) — their logic moved into a
new `team_header_line(model, width) -> Line` and `render_header` gained a
4th row for it (header height `Constraint::Length(3)` → `Length(4)`).
`team_header_line` doesn't reuse `truncate_with_ellipsis` directly — a
colored dot can't be cleanly split mid-name the way a plain string can,
so it checks each whole member "piece" (dot + name + separator) against
the running width budget and stops before the one that would overflow,
appending `…`. `DOCK_CYCLE` shrank from 3 stops to 2 (`[Center, Right]`);
`Ctrl+1`/`Ctrl+t` (the old "focus Team" binding) removed outright rather
than remapped; `WorkspaceState::focused_dock`'s default changed from
`Left` to `Center`. `apply_pane_action`'s `UiDockSlot::Left` arm and
`render`'s collapsed-sidebar `UiDockSlot::Left` arm both collapsed into
the same "unreachable, falls back to Notification" treatment `Bottom`
already had since `step19.md`.

**Calendar: content-sized, truncate-only**: `render_calendar_panel`
dropped its `wrap_to_width` branch entirely — every title now goes
through `truncate_with_ellipsis` unconditionally, with the budget
computed from the actual rendered time-prefix width so the ellipsis
lands exactly where the line would have overflowed. `wrap_to_width`
itself had no other call site left afterward and was deleted (along with
its two dedicated tests) rather than kept as unused dead code. New
`calendar_panel_natural_width` mirrors `team_panel_natural_width`'s old
shape exactly (longest `"{time}  {title}"` line, or the longest
empty-state string, +2 for borders) — same pattern, different panel.
`render(...)`'s body `Layout` dropped from 3 constraints to 2
(`Constraint::Min(0)` Notification, `Constraint::Length(calendar_width)`
Calendar), with the Calendar dock's floor set to 20 (was 10 for the old
Team dock) — a calendar row's minimum useful content (a timestamp alone
is already ~9-11 columns) is wider than a bare presence dot ever was.

**Config**: `LayoutSettings.left_dock_width` removed outright (not
deprecated) — `serde`'s default "ignore unknown keys" behavior (no
`#[serde(deny_unknown_fields)]` anywhere in this schema) means an old
`config.toml` with a stray `left_dock_width` key still parses cleanly,
confirmed by a dedicated test rather than assumed. `right_dock_width`'s
default raised from 32 to 60.

**A real, unplanned interaction surfaced during test fixes, not by
design review**: `WorkspaceState::default()`'s new `focused_dock: Center`
means any test that draws Notification content using a bare `..Default::default()`
now has that list's *first row* implicitly "selected" (`selected_index`
defaults to `0`), which replaces that row's priority color with the
selection highlight entirely (`theme::selected_style()` fully overrides
the item style, unlike the Team dot's separate `Span` whose own color
survives selection). One existing color-assertion test
(`notification_panel_colors_high_priority_differently_from_low`) broke
this way and was fixed by explicitly focusing a different dock in that
test — a real, if narrow, behavioral change worth knowing about for any
future test that inspects Notification-panel styling without setting
`focused_dock` itself.

Test churn was the bulk of this phase's actual work, not new logic: of
the ~15 tests touched, most were mechanical (dropping a
`left_dock_width` argument from a test helper call), but several needed
real rewrites where the *premise* no longer held — `a_short_roster_shrinks_the_team_dock_below_the_configured_ceiling`
and `a_wider_right_dock_width_wraps_long_titles_less` had no Team-dock or
wrapping behavior left to test at all and were replaced outright rather
than patched. Final counts: `crates/ui` 173 tests (up from 170),
`crates/config` 24 (down from 24 — one obsolete `left_dock_width`-floor
test removed, one new stray-key-compatibility test added, net even).
Full `cargo fmt`/`check`/`clippy -D warnings`/`test --workspace` green
throughout.
