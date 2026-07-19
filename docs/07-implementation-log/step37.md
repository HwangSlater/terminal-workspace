# Implementation Plan - Phase 37: Help overlay goes horizontal, pickers get search

Requested directly, four items in one message. This phase covers the two
that had one clear, low-risk implementation with no real tradeoff to
weigh; the other two (add more commands so everything is reachable via
`:` alone, and a full keybinding reorganization) are genuine design
decisions with several viable directions each — deferred to a follow-up
`AskUserQuestion` round rather than guessed at, since a wrong guess there
means redoing real implementation work across many files, not just a
string tweak.

## Context

1. "도움말 무조건 세로배치 말고 가로배치로" — the `?` help overlay
   (`step36.md` split it into a 단축키/커맨드 section) should lay out
   horizontally, not as one ever-taller vertical list.
2. "슬랙 채널이나 사용자 선택은 검색 지원 가능해?" — does the Slack
   channel/user picker (`Ctrl+P`) support search? (It didn't. Neither did
   GitHub's repository picker, `Ctrl+R`, same underlying gap.)
3. "커맨드 여러 종류 넣어서 커맨드만으로도 원하는걸 할 수 있게" — add
   enough `/commands` that everything is reachable without touching a
   `Ctrl+` shortcut at all. **Deferred** — see below.
4. "단축키도 Ctrl+2부터 시작하고 뒤죽박죽인거 같아. 재정리 해야할 거
   같아" — the `Ctrl+` scheme feels disorganized. **Deferred** — see below.

## Decision 1: two-column help overlay

`render_help_overlay` (`crates/ui/src/render.rs`) previously built one
`Vec<ListItem>` for the whole overlay and rendered it as a single `List`
inside one bordered `Block` titled "도움말" — every category, from either
section, stacked in one column. Reworked into `help_section_column`,
called once per `HelpSection` (`step36.md`'s Shortcuts/Commands tag),
each producing its own items + natural content width/height; `Layout::
horizontal` splits the popup into two bordered columns side by side,
titled "단축키" and "커맨드" directly (the section title is now the
column's own `Block` title, not a row inside a shared list). Popup width
is now the sum of both columns' natural widths plus borders/gap; popup
height is the taller of the two columns' natural heights — the reverse
of before, where height was the sum of everything and width was whatever
the single longest line needed.

No new keybinding, no state change — this is a pure rendering
reorganization, and it directly reuses the section tagging `step36.md`
already added rather than inventing a second axis to split on.

## Decision 2: `/` search in the Slack and GitHub pickers

`SlackPickerState`/`GitHubPickerState` (`crates/ui/src/state.rs`) each
gained `filter_query: String` and `filtering: bool`. `PickerRow` gained a
private `matches_filter` (case-insensitive substring match against the
row's `label`, empty query matches everything). Each picker state gained
a `visible_indices()` method returning the real indices (into `channels`/
`users`/`repositories`) that currently match, in the same order the
unfiltered list already used — Slack's version is channels-first-then-
users, matching the existing combined-cursor convention.

**Interaction model**: `/` (`crates/ui/src/keyboard.rs`,
`capture_slack_picker_input`/`capture_github_picker_input`) enters a
filter-typing sub-mode -- while `filtering` is true, `Char`/`Backspace`
edit `filter_query` and `Enter` exits back to browsing; while `filtering`
is false, `j`/`k`/arrows/`Space`/`Enter` mean exactly what they meant
before, now operating on `visible_indices()` instead of the raw list.
Chosen over an always-on "any letter typed is a filter" scheme because
`j`/`k`/`Space` are already bound to navigation/toggle in this same
overlay — a channel or person's name can legitimately contain any of
those characters, so typing and browsing need to be mutually exclusive
states, not simultaneously-live keys with ambiguous meaning. This mirrors
the app's existing Normal/Input modal split rather than inventing a new
interaction shape. `Esc` still always closes the whole overlay regardless
of `filtering` (intercepted globally in `handle_key`, before either
picker's capture function ever runs) — not changed, and not worth
changing just for this: a `filtering`-aware Esc would need touching the
global pipeline's meaning for every overlay, not just these two.

`cursor` now indexes the *visible* list, not the raw one -- `Space`
toggling row 0 while a filter is applied toggles the first *visible* row,
not raw index 0. Selecting rows is unaffected by the filter beyond what's
browsable: `Enter`'s final submit (outside filtering mode) still walks
the full, unfiltered `channels`/`users`/`repositories` for `.selected`
rows, so narrowing the view to find something doesn't silently drop an
earlier selection that's no longer visible.

`Ctrl+P`/`Ctrl+R` (the global shortcuts that open each picker) now also
clear `filter_query`/`filtering` on open, same "don't let a previous
session's typed state silently carry into a fresh one" reasoning already
applied to the setup overlays' token fields.

Calendar's `Ctrl+K` picker was **not** given search — it lists already-
connected calendars for removal, typically a handful at most (unlike
Slack channels/GitHub repos, which can run into the hundreds on a large
workspace/account), so the actual problem search solves doesn't really
exist there yet.

## Deferred: more commands, keybinding reorganization

Both are real, substantial design decisions:

- **Commands**: making "everything reachable via `:` alone" real means
  deciding, for each shortcut/overlay flow (Slack/GitHub/Calendar
  connect, channel/repo/calendar selection, calendar rename, ...),
  whether it gets a `/command` equivalent, what its argument shape is,
  and how a multi-field overlay flow (e.g. Calendar's label-then-URL
  setup) collapses into one command line. Several of these have more
  than one reasonable syntax.
- **Keybindings**: "재정리" could mean renumbering from `Ctrl+1` again,
  switching the Notification/Calendar focus keys to mnemonic letters to
  match the `Ctrl+S`/`Ctrl+P`, `Ctrl+G`/`Ctrl+R` pairs, or something else
  entirely — and any change here touches muscle memory directly, the same
  category of decision `step32.md` used a live mockup Artifact for rather
  than guessing.

Both need the user's direction before real implementation starts, not a
guess that might need a full rewrite if wrong. Asked directly as a
follow-up to this phase rather than forced into it.

## Verification

- `cargo fmt --all -- --check` / `cargo clippy --workspace --all-targets --
  -D warnings` / `cargo test --workspace` all green.
- New tests, `crates/ui/src/keyboard.rs`: `slash_starts_filtering_the_slack_picker`,
  `typing_while_filtering_does_not_move_the_cursor_or_toggle_a_row`,
  `enter_while_filtering_stops_filtering_without_submitting`,
  `space_toggles_the_correct_underlying_row_even_when_filtered` (the real
  bug class this feature could have introduced -- cursor position vs. raw
  index confusion -- pinned directly), `ctrl_p_clears_a_filter_left_over_from_a_previous_session`,
  and the GitHub-picker mirrors of the search-then-toggle and
  clear-on-reopen cases.
- New tests, `crates/ui/src/render.rs`:
  `slack_picker_overlay_hides_rows_that_do_not_match_the_filter`,
  `slack_picker_overlay_shows_a_trailing_cursor_mark_while_actively_filtering`,
  `help_overlay_separates_shortcuts_from_commands_as_distinct_sections`
  (already added in `step36.md`, still passes unchanged against the new
  two-column layout since it only checks text presence/ordering, not a
  specific arrangement).
- Manually ran the app: confirmed the help overlay renders as two
  side-by-side bordered columns; confirmed `/` in both pickers narrows
  the list live, `Enter` returns to browsing, and a stale search from a
  previous `Ctrl+P` session doesn't survive a reopen.
