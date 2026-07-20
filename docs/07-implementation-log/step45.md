# Implementation Plan - Phase 45: Argument autocomplete for the `step41.md` commands

Third of four items from the `step42.md`/`step43.md` enhancement scan.

## Context

`step13.md` gave `Tab` two candidate sources: the command head
(`/a` → `/away`/`/active`) and, for `/send` specifically, its channel
argument. `step41.md` then added four more commands whose arguments are
themselves names resolved against a picker (`/slack-watch #channel`,
`/repo-watch owner/repo`, `/calendar-rename`/`/calendar-remove <label>`)
— but autocomplete was never extended to them, so typing one out meant
either remembering the exact name or alt-tabbing to the picker overlay to
check, exactly the friction those commands existed to remove in the first
place.

## Change

`compute_suggestions` (`crates/ui/src/keyboard.rs`) widened from
`(text, cursor, picker: &SlackPickerState)` to
`(text, cursor, state: &WorkspaceState)` — the same widening
`parse_command` already went through in `step41.md`, for the same reason
(resolving GitHub/Calendar names needs their own pickers, not just
Slack's). Three new match arms, each delegating to a small per-source
helper (`channel_suggestions`, `repo_suggestions`, `calendar_suggestions`)
that prefix-matches case-insensitively against the picker's rows:

- `/slack-watch #chan...` — reuses `channel_suggestions` (factored out of
  what was inline `/send`-only logic before). Unlike `/send`, which only
  completes its one channel argument (word 2; word 3+ is the free-text
  message body), `/slack-watch` completes *every* word from position 2
  onward, since it accepts an open-ended channel list.
- `/repo-watch owner/rep...` — same open-ended-list treatment, against
  `GitHubPickerState::repositories`.
- `/calendar-rename`/`/calendar-remove <name...>` — only the *first*
  argument (word 2) completes; `/calendar-rename`'s second argument is the
  new label being typed in, not a lookup, so offering candidates there
  would be actively wrong.

### A latent bug this surfaced: multi-word candidates broke `Tab` cycling

`apply_next_suggestion` recomputed `word_start(&raw_text, cursor)` fresh
on *every* `Tab` press, on the documented assumption that "none of the
candidates contain spaces." Calendar labels can (`개인 일정`), so once a
multi-word candidate had already been spliced in, a second `Tab` press
(cycling to the next candidate) would recompute `word_start` against the
*already-completed* text, find the space **inside** the just-inserted
label instead of the real original word boundary, and silently replace
only the label's last word — corrupting the line instead of cycling
correctly.

Fixed by pinning the replacement span instead of recomputing it:
`CommandBufferState` gained `suggestion_anchor: Option<usize>`
(`crates/ui/src/state.rs`), set once via `word_start` on the first `Tab`
of a cycle and reused for every later press in that same cycle. Reset to
`None` alongside `selected_suggestion_index` wherever that already was
(`refresh_suggestions` on every real edit, and the command-bar `Enter`
handler).

`render.rs`'s help overlay `/read-all` and Tab rows updated to reflect
the newly-completable arguments (`step44.md`'s command included in the
same pass since it landed just before this one).

## Verification

- `cargo fmt --all -- --check` / `cargo clippy --workspace --all-targets --
  -D warnings` / `cargo test --workspace` all green.
- New tests (`crates/ui/src/keyboard.rs`):
  `compute_suggestions_matches_slack_watch_channel_argument_by_prefix`,
  `compute_suggestions_offers_slack_watch_channels_past_the_first_argument`,
  `compute_suggestions_matches_repo_watch_argument_by_prefix`,
  `compute_suggestions_offers_repo_watch_repos_past_the_first_argument`,
  `compute_suggestions_matches_calendar_rename_argument_by_prefix`,
  `compute_suggestions_matches_calendar_remove_argument_by_prefix`,
  `compute_suggestions_does_not_offer_calendar_names_past_the_first_argument`,
  `tab_completes_a_calendar_name_for_calendar_remove`, and
  `cycling_past_a_multi_word_candidate_replaces_the_whole_original_word_not_just_its_tail`
  (the anchor-fix regression test — two calendars sharing a prefix, one
  multi-word, asserts the second `Tab` press produces the correct full
  replacement instead of a corrupted one).
- Manually verified: connected two calendars named "회사" and "회사 백업",
  typed `/calendar-remove 회`, confirmed the first `Tab` completed to
  "회사" and a second `Tab` cycled to the full "회사 백업" rather than
  mangling the line.
