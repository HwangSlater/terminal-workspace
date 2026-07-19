# Implementation Plan - Phase 31: Team panel — colored dot instead of a bracketed status word

Small enough in scope (one panel, one real bug fix + one direct design
request) to skip the full Decisions/AskUserQuestion cycle — implemented
directly, documented here after the fact per `development.md`'s own rule
that a change still needs a record even when it didn't need up-front
confirmation.

## Context

Two related reports from live use:

1. A long display name in the Team panel gets clipped with no indication
   anything was cut off — the same class of bug `wrap_to_width` fixed for
   the Calendar panel (`step26.md`), which never got the same treatment.
2. The Team panel realistically only ever holds a handful of people (a
   small team), so it doesn't need much width at all — but the
   `"{name} [{status_label}]"` format (e.g. `"Alice [활동중]"`) is wide
   enough, once a longer name is involved, that wrapping it onto a second
   row (the Calendar panel's fix) would look awkward for a list this
   short. Requested directly: shorten the line itself, using a color or
   an icon instead of the bracketed status word.

## Fixes

### Bracketed status word → colored dot

`render_team_panel`'s line format changed from `"{display_name} [{status_label}]"`
to `"● {display_name}"`, with the `●` colored via the already-existing
`presence_status_color` (the status *word* — "활동중"/"오프라인"/etc. — is
gone; the color alone carries the same signal, same reasoning the header's
connection-status dots already established). This alone fixes almost
every real case: dropping `" [상태]"` (up to 8 columns for the longest
label, "자리비움") down to `"● "` (2 columns) means most real display
names now fit comfortably even under a small `left_dock_width` ceiling.

`presence_status_label` (the function that produced the now-deleted
bracketed word) had no other call sites once this landed — deleted rather
than left as dead code.

### Long names: shorten in place, not wrap

For the (now much rarer) case where a name still doesn't fit — a
genuinely long display name under a small configured ceiling — added
`truncate_with_ellipsis(text, width)`, the Team panel's counterpart to
the Calendar panel's `wrap_to_width`: unicode-width-aware, appends `…`
when it shortens something, matching the direct request to shorten
rather than wrap. `team_panel_natural_width` (`step27.md`'s content-sized
Team dock) updated to reflect the new `"● {name}"` format when computing
how narrow the dock can get.

## Verification

- `truncate_with_ellipsis` pure-function tests: short text untouched, long
  text shortened with a trailing `…` and never exceeding the requested
  width, zero-width doesn't panic.
- `team_panel_truncates_a_long_name_instead_of_clipping_it`: a name much
  wider than a deliberately narrow configured `left_dock_width` renders
  with a `…` marker, not the full name and not silently clipped.
- `team_panel_colors_the_presence_dot_like_the_header_does` (replaces the
  old status-word color test, which scraped `Color` off "활동중"/"오프라인"
  text that no longer exists): each presence status drawn in its own
  single-member scenario (not two members in the same frame — every row's
  dot is the same character, so `fg_color_of`'s first-match search
  couldn't otherwise tell two members' colors apart) and checked directly.
- `a_short_roster_shrinks_the_team_dock_below_the_configured_ceiling`
  (`step27.md`) updated for the new, shorter natural-width formula — a
  2-character roster now computes to 6 columns, floored to `render()`'s
  existing 10-column minimum (unchanged from `step27.md`), rather than
  the old format's 13.
- `cargo fmt --all --check` / `cargo check --workspace --all-targets` /
  `cargo clippy --workspace --all-targets -- -D warnings` /
  `cargo test --workspace` all green. `crates/ui`: 174 tests (up from 170).
