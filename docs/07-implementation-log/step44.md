# Implementation Plan - Phase 44: `/read-all` — bulk mark-all-read

Second of four items from the `step42.md`/`step43.md` enhancement scan.

## Context

`step36.md` wired `Enter` to `Command::MarkNotificationRead`, but only for
one highlighted row at a time. There was no way to clear a Notification
panel with a dozen unread items except pressing `Enter` a dozen times —
tedious enough that the user asked for a bulk version directly.

## Change

New unit variant `Command::MarkAllNotificationsRead` (`crates/commands/src/lib.rs`)
— no payload, unlike `MarkNotificationRead { id }`. `Command` is not
frozen (unlike `Event`, per Architecture Freeze v1), so this needed no ADR.

`WorkspaceCommandHandler::handle`'s new match arm: snapshots the currently
unread ids from the live read model, calls
`self.notification_repo.mark_read(&id)` for each, then removes exactly
those ids from `read_model.unread_notifications`. The snapshot-then-write
ordering (read the list, release the lock, write each id, re-take the
lock to remove) means a notification that arrives *during* the loop
(after the snapshot) is left alone rather than raced — it just wasn't in
`ids`, so nothing marks it and nothing removes it; a later `/read-all` or
`Enter` will catch it.

Command-line entry point: `/read-all` (`crates/ui/src/keyboard.rs`) —
added to `COMMAND_HEADS` for `Tab` completion and to `parse_command` as a
bare head with no arguments, `Ok(Some(Command::MarkAllNotificationsRead))`.

## Verification

- `cargo fmt --all -- --check` / `cargo clippy --workspace --all-targets --
  -D warnings` / `cargo test --workspace` all green.
- New tests: `mark_all_notifications_read_clears_every_unread_row`,
  `mark_all_notifications_read_is_a_no_op_when_nothing_is_unread`
  (`crates/commands/src/lib.rs`), `read_all_parses_to_mark_all_notifications_read`
  (`crates/ui/src/keyboard.rs`).
- Not covered by an automated test: the mid-loop-arrival race description
  above (a notification arriving between the snapshot and the write) —
  would need a fake repository that can inject a state change from inside
  a mid-test await point; the existing fakes in `crates/commands`'s test
  module don't have that hook, and building one felt like more surface
  than this fix warranted on its own. Manually verified the ordinary path
  instead: connected Slack with several unread messages, ran `/read-all`,
  confirmed the panel cleared and a fresh message right afterward still
  showed up normally.
