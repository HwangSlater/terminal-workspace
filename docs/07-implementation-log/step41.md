# Implementation Plan - Phase 41: Command-line equivalents for the picker overlays

Requested directly ("커맨드 확장 너가 알아서 해줘. 커맨드만으로 조작할 수
있게") as a follow-up to the scope `step37.md` deferred — full autonomy
given on which specific commands to add, so this doc records the actual
scoping decisions made rather than a pre-negotiated plan.

## Context

`step36.md`/`step37.md` already walked the full `Command` enum once and
found two genuinely unwired commands (`MarkNotificationRead`, wired to
`Enter` in `step36.md`; `SyncAllAdapters`, still a stub — see below). The
remaining gap the user is naming here is different: **the picker
overlays' selection-management (Slack channel/user list, GitHub repo
list, Calendar connections) had no command-line path at all** — only
`Ctrl+P`/`Ctrl+R`/`Ctrl+K` could change them. A user who wants to stay in
"type commands" mode couldn't fully avoid the overlays.

## Decisions

### What got a command, and why

Four new command heads, each a thin command-line front end for an
**existing** `Command` variant (`ApplySlackSelection`, `ApplySelection`,
`RenameCalendar` — none of these needed a new `Command` variant, and
`Command` isn't part of Architecture Freeze v1's frozen list to begin
with, `development.md` §3 — only `Event` is):

- **`/slack-watch #channel [#channel2 ...]`** → `Command::ApplySlackSelection`.
  Replaces the full channel-watch list in one call, mirroring `Ctrl+P`'s
  own "`Enter` submits exactly what's checked" semantics rather than
  inventing an "add to the list" meaning that would behave differently
  from the overlay doing the same job. Preserves whatever
  `watched_user_ids` (presence watch-list) the picker state already has
  selected — this command only touches channels, since scoping "watch
  this channel for messages" and "watch this person's presence" into one
  command would need two different argument shapes glued together.
- **`/repo-watch owner/repo [owner/repo2 ...]`** → `Command::ApplySelection`
  (GitHub). Same "replace the full list" shape as `/slack-watch`.
- **`/calendar-rename <기존 이름> <새 이름>`** → `Command::RenameCalendar`.
- **`/calendar-remove <이름>`** → `Command::ApplySelection` (Calendar) —
  Calendar's `ApplySelection` is "keep exactly these ids" (`step24.md`),
  so this resolves the target label then submits every *other* connected
  calendar's id.

All four resolve names against whatever the matching picker overlay
(`Ctrl+P`/`Ctrl+R`/`Ctrl+K`) last fetched into `WorkspaceState` — the
same dependency `/send`'s channel-name resolution already has and has
had since `step9.md`, not a new constraint this phase introduces. An
unresolvable name is a real error (`state.cmd_buffer.last_error`), same
as `/send #nope`, pointing at which overlay to open first.

### What deliberately did **not** get a command

- **Credential entry** (`Ctrl+S`/`Ctrl+G`/`Ctrl+L` — Slack Bot Token,
  GitHub PAT, Calendar's secret iCal URL): excluded on purpose, not an
  oversight. `CommandBufferState.history` is plain, unredacted text
  (unlike the setup overlays' masked input fields) — a hypothetical
  `/connect slack <token>` would put a live secret directly into that
  history, which nothing currently scrubs (`crates/logging`'s secret
  redaction covers *log output*, not the in-memory command bar history,
  a separate surface it has no visibility into). This was flagged as a
  real risk in `step37.md`'s deferred section and holds here too: the
  masked overlays exist specifically to keep a token off any plain-text
  surface, and a command equivalent would defeat that for no real
  ergonomic gain (pasting a token into `:` vs. into the overlay's own
  input field costs the same
  one paste either way).
- **`/sync` (forcing `Command::SyncAllAdapters`)**: still not real.
  `SyncAllAdapters`'s handler is a literal no-op (confirmed again this
  phase — `sync_all_adapters_is_a_noop_ok`, `crates/commands/src/lib.rs`,
  already documented this in `step36.md`). Making it real needs a new
  `IntegrationAdapter::poll_now()`-shaped method across all three
  adapters plus a decision about interaction with `step33.md`/`step39.md`'s
  `is_first_poll` suppression — genuinely separate, larger work, not
  "wire up an existing thing" like the four commands above.
- **Mark-all-read as a command**: `Enter` (`step36.md`) already marks one
  notification read at a time via a real, already-wired `Command`.
  Extending that to "all at once" needs either a new `Command` variant
  (small, but `parse_command` currently only ever returns a *single*
  `Command` per submitted line — dispatching N commands from one command
  line would need new plumbing in the async event loop, not just a new
  parse branch) or a client-side loop dispatched outside the normal
  single-command path. Judged not worth the new mechanism for this pass;
  flagged as a reasonable, self-contained follow-up if wanted.
- **Argument-level `Tab` autocomplete** for the four new commands'
  channel/repo/calendar-name arguments: `compute_suggestions`
  (`step13.md`) only completes command *heads* (works automatically for
  the four new ones, since it filters `COMMAND_HEADS`, which they were
  added to) and `/send`'s second word specifically. Extending it to also
  complete arbitrary-position arguments for four more commands is a
  real, contained follow-up, deliberately not bundled into this phase to
  keep it reviewable — the four commands are fully usable without it
  (an unresolved name is a clear, specific error either way), just less
  discoverable than `/send`'s existing channel completion.

### Multi-word Calendar labels

Calendar connection labels are free text and commonly contain spaces
(`"[회사] Design Review"`-style prefixes already exist per `step24.md`'s
example) — a naive `splitn(2, ' ')` on `/calendar-rename`'s arguments
would treat only the first word as the old label, breaking on anything
like `/calendar-rename 개인 일정 새 이름`. `split_calendar_label`
(`crates/ui/src/keyboard.rs`) instead finds the **longest** connected
label that prefixes the typed text, case-insensitively, and treats
everything after it as the new label. Longest-match (not first-match)
disambiguates the case where one label is itself a prefix of another
(e.g. "회사" vs. "회사 (백업)"). `/calendar-remove` doesn't have this
problem — it takes exactly one argument, the whole remainder of the line
is the label, no split needed.

## Verification

- `cargo fmt --all -- --check` / `cargo clippy --workspace --all-targets --
  -D warnings` / `cargo test --workspace` all green (201 `ui` tests, up
  from 189).
- New tests, `crates/ui/src/keyboard.rs`: three per command (success
  case, unknown-name error, usage error for missing arguments) —
  `slack_watch_replaces_the_channel_list_and_preserves_watched_users` /
  `_with_an_unknown_channel_is_a_real_error` / `_with_no_channels_is_a_usage_error`;
  the `repo_watch_*` mirrors; `calendar_rename_resolves_a_multi_word_label_and_renames_it`
  (the actual case `split_calendar_label`'s longest-prefix logic exists
  for) / `_with_an_unknown_label_is_a_real_error` /
  `_with_no_new_label_is_a_usage_error`; `calendar_remove_keeps_every_other_connected_calendar`
  / `_with_an_unknown_label_is_a_real_error` / `_with_no_label_is_a_usage_error`.
- Help overlay (`crates/ui/src/render.rs`, `HELP_CATEGORIES`'s "명령줄"
  category) updated with all four new commands, no new test needed
  beyond the existing overlay-content tests already exercising that
  category.
- Manually ran the app: connected Slack, opened `Ctrl+P` once to
  populate the channel cache, closed it, then used `/slack-watch
  #general` from the command line alone to change the watched channel
  without reopening the picker; same manual check for `/repo-watch` and
  `/calendar-rename`/`/calendar-remove` against a multi-word calendar
  label.
