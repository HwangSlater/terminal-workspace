# Implementation Plan - Phase 43: Calendar picker (`Ctrl+K`) search

Real feature request, direct follow-up to the enhancement-scan requested in
`step42.md`'s session ("고도화 할 작업 있는지 찾아볼래?"). One of four items
surfaced and reported back; the user asked for all four, this is the first.

## Context

`step37.md` added `/`-to-search to the Slack (`Ctrl+P`) and GitHub
(`Ctrl+R`) pickers, but deliberately left the Calendar picker (`Ctrl+K`)
out — the reasoning at the time was that a connected-calendar list stays
short (a handful of calendars, not hundreds of channels or repos), so
search wasn't worth the added complexity. That assumption held until it
didn't; the user asked for it directly once scrolling through their own
connected calendars got old.

## Change

`CalendarPickerState` (`crates/ui/src/state.rs`) gained the same
`filter_query`/`filtering` fields and `visible_indices()` method
`GitHubPickerState` already has — identical shape, since both are a
single flat list (unlike Slack's two-section channels-then-users split).

`capture_calendar_picker_input` (`crates/ui/src/keyboard.rs`) gained the
same `/`-filter sub-mode `capture_github_picker_input` already has:
`/` enters filter-typing, characters/`Backspace` edit `filter_query`,
`Enter` stops typing and returns to browsing. Navigation (`j`/`k`/arrows),
`Space` (toggle), `Enter` (submit), and `e` (rename) all now index through
`visible_indices()` first, translating a *visible* position back to the
real index into `calendars` before touching anything — same pattern the
Slack/GitHub pickers use. `Ctrl+K`'s open handler now also clears stale
filter state left over from a previous session, matching `Ctrl+P`/`Ctrl+R`.

`render_calendar_picker_overlay` (`crates/ui/src/render.rs`) gained the
same search-line row and filtered rendering `render_github_picker_overlay`
has, reusing the shared `search_line` helper.

Submitting (`Enter`) still submits every *checked* row regardless of the
current filter — searching only narrows what's visible, never what gets
saved, the same rule the other two pickers already established.

## Verification

- `cargo fmt --all -- --check` / `cargo clippy --workspace --all-targets --
  -D warnings` / `cargo test --workspace` all green.
- New tests: `slash_filters_the_calendar_picker_and_space_toggles_the_filtered_row`,
  `calendar_picker_e_opens_the_rename_prompt_for_the_filtered_row`,
  `ctrl_k_clears_a_filter_left_over_from_a_previous_session` — mirroring
  the equivalent Slack/GitHub tests from `step37.md`/`step38.md`.
- Manually verified: connected three calendars with overlapping name
  prefixes, confirmed `/` narrows the list, `Space`/`e` act on the
  filtered row shown (not whatever index it happened to occupy
  unfiltered), and `Enter` still saves the full checked set.
