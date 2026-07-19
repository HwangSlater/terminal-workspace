# Implementation Plan - Phase 39: Stop dropping genuinely new Slack messages and Calendar reminders

Real bug fix, reported via live use — skips the Decisions/`AskUserQuestion`
cycle (there was one reasonable fix once the cause was clear), documented
here per `development.md`'s own rule.

## Context

Two reports in one message, both traced to the same root cause:

1. "슬랙 채널에 채팅 보내도 알림 안뜨는데?" — a real Slack message, sent and
   confirmed delivered (screenshotted from the actual Slack client), never
   appeared in the Notification panel at all.
2. "캘린더 일정 갑자기 안 떠" — the Calendar panel showed "다가오는 일정이
   없습니다" (no upcoming events) despite a working connection.

## Root cause

`step33.md`'s fix for "a notification fires on every single launch"
suppressed `event_bus.publish(...)` entirely on each adapter's first poll
since process start. That was too broad: `event_bus.publish` is the
*only* path that updates `DashboardReadModel.unread_notifications`
(`Projector::handle`, `crates/commands/src/lib.rs`) — the same Vec both
the Notification panel *and* the Calendar panel render from. Suppressing
the publish didn't just silence the desktop toast (the actual original
complaint); it also meant the panels never learned the item existed.

- **Slack**: `channel_cursor` is in-memory only and resets every restart.
  The very first poll for a channel treats every message currently in
  scrollback as "not new" (`is_first_poll_for_channel = oldest.is_none()`)
  and both skips the publish *and* still advances the cursor past it —
  so a message that happened to already exist at the moment of that first
  poll was not just delayed, it was **permanently lost**: no later poll
  would ever see it as new again, since the cursor already points past it.
- **Calendar**: same shape, but the effect was different since Calendar
  reminders are forward-looking (an occurrence stays inside the lookahead
  window for a while). The panel wasn't losing data forever, but it *was*
  guaranteed empty for up to a full `sync_interval_secs` (900s by default)
  after every single restart — exactly what "갑자기 안 떠" describes: open
  the app, see nothing, and no idea a second poll cycle would eventually
  fix it 15 minutes later.

`step33.md`'s own Verification section already flagged the alternative
(persist seen-state to `redb`) as "more machinery... for a difference
that doesn't matter in practice" — that judgment turned out to be wrong
specifically for these two symptoms, though the underlying "don't spam a
desktop toast for backlog on every launch" goal was still worth keeping.

## Fix

Decouples "populate the panel accurately" from "fire a desktop toast,"
which `step33.md` had conflated by gating both on the same `publish`
call:

- **`crates/integration/src/slack.rs`, `github.rs`, `calendar.rs`**: each
  adapter's poll function now **always** publishes for every item it
  finds that isn't already in its seen-set/cursor, first poll or not —
  the panels are always accurate, immediately, even on the very first
  poll after a restart. Instead of skipping the publish, each item's
  `NotificationItem.is_read` field is set to whatever `is_first_poll`
  (or, for Slack, `is_first_poll_for_channel`) was at discovery time.
  `NotificationId` is a deterministic `Uuid::new_v5` hash of stable
  identity (channel+ts / repo+PR-number / calendar+event+occurrence), so
  this doesn't need any new persisted state — the exact same in-memory
  cursor/seen-set `step33.md` already threads through was reused, just
  applied to a field on the item instead of a branch around the publish.
- **`crates/notifications/src/lib.rs`**: `DesktopNotifier`'s
  `notification_for_event` now also checks `!item.is_read` for the three
  integration-sourced `Event` variants (`SlackMessageReceived`,
  `GitHubPRCreated`, `CalendarReminderTriggered`) before producing a
  toast — an item marked already-read at discovery (first-poll backlog)
  still updates the panel via `Projector` exactly like any other
  notification, it just doesn't interrupt with an OS popup for something
  that already existed before this session started. `SystemAlert` is
  unaffected (no `is_read` concept, was never part of this suppression).

This keeps `step33.md`'s actual goal intact (no toast burst on every
launch for pre-existing backlog) while fixing the two real bugs it
introduced as a side effect. `is_read` was already a `NotificationItem`
field with no other production writer (nothing currently persists
notifications to `notification_repo` outside `MarkNotificationRead` —
confirmed via a full grep before choosing this fix) — reusing it as "was
this item backlog at discovery" is a self-contained signal that only
`DesktopNotifier` reads, with no interaction with `step36.md`'s
mark-as-read flow (a different code path, `Command::MarkNotificationRead`,
that never runs at discovery time).

## Implementation Notes

While diagnosing this, `cargo test -p notifications -p integration`
started failing with `error calling dlltool 'dlltool.exe': program not
found` — unrelated to this fix, surfaced by a `cargo clean` run earlier
in the same session, which wiped previously-cached link artifacts. The
machine's active `rustup` toolchain was `stable-x86_64-pc-windows-gnu`
(needs MinGW's `dlltool` to link, not installed/on `PATH` here), while a
fully-installed MSVC Build Tools toolchain (`stable-x86_64-pc-windows-msvc`,
confirmed via `VC\Tools\MSVC` on disk) was sitting unused — and MSVC is
this project's own documented Tier-1-preferred Windows target
(`docs/06-development/platform-support.md` §1). Fixed for this project
specifically via `rustup override set stable-x86_64-pc-windows-msvc` (a
per-directory override, not a machine-wide default change) rather than
asking for a MinGW install this project doesn't actually need.

## Verification

- `cargo fmt --all -- --check` / `cargo clippy --workspace --all-targets --
  -D warnings` / `cargo test --workspace` all green (189 `ui`, 8
  `notifications`, 71 `integration`, unchanged/passing elsewhere).
- New test, `crates/notifications/src/lib.rs`:
  `already_read_integration_items_produce_no_notification` — the other
  half of this fix (the desktop-toast suppression) verified directly,
  covering all three integration-sourced `Event` variants in one test.
- `poll_once`/`poll_one` themselves remain untested directly (live-network
  bodies, the same boundary `step33.md`/every other integration phase in
  this log already documents — only the pure mapping/filtering functions
  around them are unit tested).
- Manual acceptance check (the actual regressions this phase fixes): sent
  a real Slack message immediately after a fresh restart, confirmed it
  appears in the Notification panel; confirmed the Calendar panel shows
  upcoming reminders immediately on launch instead of staying empty for
  up to 15 minutes; confirmed neither produces a desktop toast on that
  first poll, only on a genuinely later one.
