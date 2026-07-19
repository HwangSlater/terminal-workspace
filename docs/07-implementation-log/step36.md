# Implementation Plan - Phase 36: README refresh + a full shortcut/command review

Requested directly: bring `README.md` up to date, drop the "2등 시민" phrasing
from it, review whether the keyboard shortcuts need work, review command-bar
usage and add anything straightforward and missing, and — if anything gets
added — split the `?` help overlay into a shortcuts section and a commands
section so the two don't stay interleaved.

## Context

Four asks in one message. Handled together since the shortcut/command
review directly determines what (if anything) the help-overlay split needs
to show, and the README refresh needed to reflect whatever came out of
that review anyway.

## Review: keyboard shortcuts

Walked `docs/02-architecture/keyboard.md`, `crates/ui/src/keyboard.rs`'s
`DOCK_CYCLE`/global-shortcut match, and `render.rs`'s `HELP_CATEGORIES`
end to end. Found:

- **`Enter` did nothing.** Documented since Phase 5 as "Activate item
  (e.g., open thread, edit event)," but `apply_pane_action`'s
  `PaneAction::Activate` arm was a literal no-op comment the entire time
  ("No expandable tree nodes or activatable detail view yet"). Meanwhile
  `Command::MarkNotificationRead` has been fully implemented on the write
  side (repository call, a passing unit test) since Phase 3 — nothing in
  the UI ever dispatched it. This is the one real, concrete gap the
  shortcut review surfaced; see Decision 1.
- Two leftover `Ctrl+1~3` strings from `step32.md`'s Team-to-header move
  were already caught and fixed in `step35.md` (help overlay, status
  footer) — re-verified here, nothing left stale.
- No collisions, no dead bindings, no missing `j`/`k`-vs-arrow
  inconsistency beyond what `step29.md` already intentionally decided
  (arrows advertised, `j`/`k` still functionally accepted where it always
  was). Nothing else warranted a change — the shortcut *scheme* itself
  (modal Vim-inspired input, global vs. pane-specific precedence) is
  sound; the one gap was a wiring miss, not a design problem.

## Review: command-bar (`:`) usage

Walked the full `Command` enum (`crates/commands/src/lib.rs`) against
`COMMAND_HEADS` and every keybinding to find what exists on the write
side but isn't reachable from anywhere in the UI:

- **`Command::MarkNotificationRead`** — the same dead command found above.
  Deliberately **not** exposed as a `/command` — it targets "whichever
  notification is currently highlighted," which is inherently a
  keystroke-on-a-selection action (the same shape `Enter`/`Space` already
  are for every picker overlay in this app), not something that reads
  naturally as typed syntax. Wired to `Enter` instead (Decision 1) rather
  than invented as e.g. `/read`.
- **`Command::SyncAllAdapters`** — also dead (a pre-existing test,
  `sync_all_adapters_is_a_noop_ok`, already documents that dispatching it
  does nothing and its log line is stale: `"no integration adapters
  registered yet"` was true when written, before Slack/GitHub/Calendar
  adapters existed, and never updated). Making this real would need a new
  `IntegrationAdapter::poll_now()`-shaped method implemented across all
  three adapters, plus a decision about how it interacts with `step33.md`'s
  `is_first_poll` suppression (does a manual sync count as "first"? would
  a forced sync right after startup re-trigger the burst-notification bug
  `step33.md` fixed?). That's a real, separate feature with its own design
  question, not a "wire the existing thing up" fix — **out of scope for
  this phase**, left as a documented gap rather than forced through.
- No other command was found missing an obvious, low-risk implementation.
  The 8-entry `COMMAND_HEADS` list (`/send`, presence x5, `/pomodoro`,
  `/calendar-range`) already covers everything else the `Command` enum
  can express.

## Decisions

### Decision 1: `Enter` marks the highlighted notification/reminder read

Confirmed via investigation (no `AskUserQuestion` round — closing a
documented-but-never-implemented gap has one reasonable fix, not several
tradeoffs to weigh):

- `apply_pane_action`'s `PaneAction::Activate` arm (`crates/ui/src/lib.rs`)
  now looks up the highlighted row's `NotificationId` (Notification dock:
  `model.unread_notifications[selected_index]`; Calendar dock: the same
  `render::calendar_notifications` filter `Up`/`Down` already share) and
  dispatches `Command::MarkNotificationRead { id }`.
- `Command::MarkNotificationRead`'s handler (`crates/commands/src/lib.rs`)
  previously only called `notification_repo.mark_read(&id)` and published
  no `Event` (correctly — no frozen `Event` variant fits "notification
  read," and nothing else in the system needs to react to it the way the
  `Projector`'s integration-originated events do). But that meant the live
  `DashboardReadModel.unread_notifications` — which `Projector` only ever
  populates from storage once, at startup, then keeps current purely via
  `Event`s — never found out. Marking something read would persist
  correctly but keep showing it as unread in the panel until a full
  restart. Fixed by giving `WorkspaceCommandHandler` a `SharedReadModel`
  handle too and having this one command remove the item directly
  (`unread_notifications.retain(|n| n.id != id)`) after the repository
  write succeeds — a deliberate, narrow exception to "only the `Projector`
  writes the read model," justified because this is the one command whose
  entire effect is "stop showing this specific item," with no other
  subscriber that could ever care.
- `WorkspaceCommandHandler::new`'s constructor gained a `read_model:
  SharedReadModel` parameter (10th argument now, still under the existing
  `#[allow(clippy::too_many_arguments)]`). `crates/app/src/main.rs` builds
  the `Projector`/`SharedReadModel` pair earlier than before (moved from
  its original spot right before the IPC-bind step up to right before the
  command handler's construction) so the handler can receive a clone of
  it — a reordering, not new machinery; `Projector::new` only ever needed
  `presence_repo`/`notification_repo`, both available from the very start
  of `main`.
- After a successful mark-read, `apply_pane_action` re-reads the (now
  one-shorter) list and clamps `selected_index` so the highlight never
  points past the new end — a small polish pass so marking the last
  visible row read doesn't leave nothing highlighted until the next
  arrow-key press.
- The read guard is explicitly dropped before the `dispatch(...).await`
  call in `apply_pane_action` — the command handler takes a *write* lock
  on the same `SharedReadModel` Arc to remove the item, so holding the
  read guard across that await would deadlock. Called out with an inline
  comment since it's the one genuinely easy way to get this wrong.

### Decision 2: split the `?` help overlay into "단축키" and "커맨드" sections

`HELP_CATEGORIES` (`crates/ui/src/render.rs`) previously interleaved a
single `"명령줄"` (commands) category among five shortcut categories in a
flat list, in category-registration order — command-bar syntax and
keystrokes read as two different kinds of lookup ("what key do I press"
vs. "what can I type"), so burying one command category in the middle of
five shortcut ones made the overlay harder to scan than it needed to be,
independent of whether any new command ever got added.

- Each `HelpCategory` now carries a `HelpSection` tag (`Shortcuts` or
  `Commands`). `render_help_overlay` renders all `Shortcuts` categories
  first under one bold+underlined "단축키" header, then all `Commands`
  categories under a "커맨드" header — `명령줄`'s five entries moved from
  2nd position to last overall, but its own internal category header and
  row shape are unchanged.
- No new command actually shipped this phase (Decision 1's
  `MarkNotificationRead` is a keystroke, not a `/command`; Decision 2's
  restructuring stands on its own regardless), so `HELP_CATEGORIES` still
  has exactly one `Commands`-tagged category (`명령줄`) — but the section
  split is real, general infrastructure now, not a one-off: a future
  command-bar addition just tags its category `HelpSection::Commands` and
  it renders under the right header automatically, same as a future
  shortcut category tagging `Shortcuts`.
- `Enter`'s new real behavior (Decision 1) was also missing from the help
  overlay entirely (the `키` group never mentioned it, since it used to do
  nothing) — added to the "탐색" category as `Enter: 선택한 알림을 읽음
  처리`.

## Also fixed: README + "2등 시민"

- `README.md` line 5's "2등 시민 취급하지 않는" replaced with "동등하게
  지원하는" — same claim, without the loaded phrasing.
- `README.md`'s Quick Start previously claimed "C 컴파일러도 ... 필요
  없습니다" directly above a Windows section describing exactly how to fix
  a C-compiler-missing build error — a real, pre-existing internal
  contradiction, not something this phase introduced. Root cause:
  `ADR-0014` (storage engine → `redb`) genuinely removed the *original* C
  compiler requirement, but `ADR-0017` (`step14.md`, `crates/plugin-host`'s
  `wasmtime` dependency) reintroduced a real one afterward, and the README
  was never revisited after that second change even though
  `docs/06-development/platform-support.md` already documents it
  correctly. Since `crates/app` depends on `crates/plugin-host`
  unconditionally (not behind a feature flag — the plugin runtime is
  default-*disabled at runtime*, `[plugins].enabled = false`, but still
  compiled every time), `cargo run -p app` on Windows/Linux genuinely does
  need a C compiler today; only macOS is exempt, and only because Xcode
  Command Line Tools (already required there for an unrelated reason) is
  a superset of what's needed. Reworded to state this plainly instead of
  overclaiming "no C compiler" and contradicting the very next section.
- Keyboard table: added the new `Enter` row; noted the `?` overlay's new
  단축키/커맨드 split.
  로그 보기: mentioned the `step35.md` on-disk log file location, since
  that's genuinely useful for a user hitting a crash the in-app buffer
  didn't survive, and wasn't in the README at all before this pass.

## Verification

- `cargo fmt --all -- --check` / `cargo clippy --workspace --all-targets --
  -D warnings` / `cargo test --workspace` all green.
- New tests:
  - `crates/commands/src/lib.rs`:
    `mark_notification_read_removes_it_from_the_live_read_model` — seeds
    the read model with two items, marks one read, asserts only the other
    remains (the actual bug the read-model wiring fixes: without it, this
    test fails with both items still present).
  - `crates/ui/src/render.rs`:
    `help_overlay_separates_shortcuts_from_commands_as_distinct_sections`
    — asserts both section headers render and 단축키 precedes 커맨드, not
    just that the category text exists somewhere on screen (the existing
    `help_overlay_groups_shortcuts_under_category_headers` was extended
    with the two new header strings rather than duplicated).
- No new unit test for `apply_pane_action`'s `Activate` arm itself
  (`crates/ui/src/lib.rs`) — this crate has no existing harness for
  constructing a full `TuiRenderer` (terminal, dispatcher, pickers, event
  bus, scheduler, log buffer all required) the way `crates/app`'s
  integration tests do, and building one solely for this thin glue (look
  up id, dispatch, reclamp) would be disproportionate; the underlying
  read-model-removal logic it depends on is covered by the `commands`
  crate test above. Manually verified instead: ran the app, confirmed
  `Enter` on a highlighted notification removes it from the panel
  immediately (not just after the next poll), confirmed the highlight
  reclamps sensibly after removing the last visible row, confirmed the
  `?` overlay now shows 단축키 above 커맨드 with `Enter` listed.
