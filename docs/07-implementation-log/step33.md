# Implementation Plan - Phase 33: Suppress notifications on the first poll after every launch

Real bug fix, root-caused via live use — skips the Decisions/AskUserQuestion
cycle (there was only one reasonable fix once the cause was clear),
documented here per `development.md`'s own rule that a change still needs a
record even when it didn't need up-front confirmation.

## Context

Reported directly: a desktop notification fires immediately, every single
time the app is launched — not just for genuinely new messages/PRs/events
received while it's running.

## Root cause

Each of the three pollers (`SlackAdapter`, `GitHubAdapter`,
`CalendarAdapter`) tracks "what have I already told the user about" purely
in memory, with no persistence:

- Slack: `channel_cursor: HashMap<channel_id, last_message_ts>`
- GitHub: `seen_prs: HashSet<(repo, pr_number)>`
- Calendar: `seen_occurrences: HashSet<(calendar_id, event_uid, occurrence_ms)>`

All three start empty on every process launch, and all three poll loops
call their poll function as the very first statement inside `loop { ... }`
— before the loop's own `sleep`, i.e. immediately at startup for any
integration that's already connected. With an empty cursor/seen-set, the
very first poll after every launch found *everything currently there*
(a channel's recent message history, every open PR, every occurrence
already inside the lookahead window) indistinguishable from genuinely new
activity, and published an event for each one. `DesktopNotifier` itself
was working exactly as designed (a narrow, correct set of `Event` variants
turn into real OS toasts) — the bug was upstream, in what got published in
the first place.

## Fix

Each poller now knows whether the current poll is the first one since its
`run_loop` started (a plain `bool` local to `run_loop`, threaded into
`poll_once`/`poll_one` as a parameter — no new persisted state, no new
struct field, no `Mutex` needed, since it's only ever read/written from
the single task that owns the loop). The first poll still does the real
fetch and still records everything it finds as seen (advancing
`channel_cursor`, populating `seen_prs`/`seen_occurrences`) — it just
skips the `event_bus.publish(...)` call for what it finds, since none of
it is actually new to the user. Every poll after the first behaves exactly
as before, unchanged.

This composes correctly with the existing selection-change paths
(`update_selection`/`connect`/`keep_only`/`set_lookahead_hours`), which
already clear the seen-state and restart the poller before this phase —
each restart naturally gets its own fresh `is_first_poll = true`, so
newly-added channels/repos/calendars also get a quiet "establish
baseline" first poll instead of immediately dumping their entire existing
backlog as notifications. That behavior wasn't the bug being fixed, but
falls out of the same mechanism for free.

Deliberately not fixed by persisting the cursors/seen-sets to `redb`
instead (the "more complete" alternative) — that would make the *exact*
same message/PR/occurrence never re-notify even across a real restart
days later, which is more machinery (a schema, load/save on every
adapter) for a difference that doesn't matter in practice: the poll
interval is what actually governs "how fresh is fresh" during normal
operation, and this phase's actual complaint was specifically about the
one-time startup burst, not steady-state behavior.

## Verification

- `cargo check`/`clippy`/`test --workspace` all green; no existing test
  broke, since none of the three poll functions had a direct unit test to
  begin with (consistent with this codebase's existing "live-network poll
  bodies aren't unit tested, only the pure mapping/filtering functions
  around them are" pattern — same reasoning `calendar.md`'s Testing
  section already documents for `fetch_calendar_feed`).
- Manual acceptance check (the actual regression this phase fixes): start
  the app with Slack/GitHub/Calendar already connected and existing
  activity in each, confirm no desktop notification fires on that first
  launch, then confirm a message/PR/event that shows up on a *later* poll
  still notifies normally.
- A real local `config.toml` issue was found and fixed by hand in the same
  session as this bug report, unrelated to this phase's code change: an
  already-saved `config.toml` had a stale explicit `right_dock_width = 32`
  from before `step32.md` raised that default to 60, because the
  Slack/GitHub/Calendar picker "save" flow round-trips the *entire*
  `AppConfig` (not just the field being changed) back to disk. That
  round-trip behavior is pre-existing (since `step8.md`), not introduced
  by this phase, and is flagged here as a known quirk worth a future look,
  not fixed as part of this change.
