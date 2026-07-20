# Implementation Plan - Phase 46: `/sync` — a real `Command::SyncAllAdapters`

Last of the four enhancement items from the `step42.md`/`step43.md` scan,
flagged at the time as "백엔드가 빈 껍데기" (backend is an empty shell) — the
`Command` variant has existed since Phase 3, `parse_command` never had a
head for it, and its handler just logged a line and returned `Ok(())`.

## Design

`IntegrationAdapter` (`crates/integration/src/lib.rs`) gained a fifth
method, `sync_now(&self, event_bus) -> Result<()>` — the trait isn't part
of Architecture Freeze v1 (`docs/06-development/development.md` §3), so
this needed no ADR, same reasoning `step41.md` used for `Command`.

The obvious-looking implementation — `self.shutdown().await?;
self.start(event_bus).await` — is exactly what every existing selection-
update method already does (`SlackAdapter::update_selection`,
`GitHubAdapter::update_selection`, `CalendarAdapter::keep_only`), and was
the first thing tried. It's wrong for `sync_now` specifically: GitHub's
and Calendar's `run_loop` each track "is this the very first poll" as a
plain local `bool`, reset to `true` on every fresh `run_loop` invocation.
A restart-based sync would spawn a fresh loop, and anything genuinely new
found on its first poll would get `is_read: true` — the exact suppression
`step39.md` built for a real process restart, now firing on every manual
sync instead, silently swallowing the desktop toast a manual sync exists
to produce. (Slack doesn't have this problem — its equivalent signal,
`channel_cursor.get(channel_id).is_none()`, is derived from state that
already survives a restart within the same process. The three adapters
turned out to be inconsistent with each other on this point; not fixed
here, just avoided.)

Instead, each adapter's `run_loop` was split into a `run_cycle` (poll +
the `integration-contract.md` §2.1 failure-count state machine + status-
change event publishing) and a thin `run_loop` that just calls
`run_cycle` then sleeps. `sync_now` builds a poller the same way `start()`
does (sharing the adapter's `Arc`/`Mutex` state, not copying it) and calls
`run_cycle` exactly once, with `is_first_poll: false` on GitHub/Calendar
explicitly — a manual sync is never a first poll, so anything it finds
notifies normally. The background loop's own next scheduled tick is
completely unaffected; a manual sync doesn't reset or delay it.

A new `poll_lock: Arc<Mutex<()>>` field, held for the duration of
`run_cycle`, serializes every cycle (background-loop and `sync_now` alike)
against every other one on the same adapter. Without it, a `/sync` landing
at the same instant as the interval loop's own tick could both read the
same "oldest" per-channel cursor / `seen_prs` state before either writes
it back, double-publishing the same item — a narrow window (poll intervals
are 30s+), but the failure mode (a duplicate notification) is exactly the
class of bug `step33.md`/`step39.md` already spent real effort chasing
down, so it felt worth closing outright rather than accepting.

`WorkspaceCommandHandler` gained a `syncable_adapters:
HashMap<IntegrationSource, Arc<dyn IntegrationAdapter>>` field, same keyed-
registry shape `connectors`/`selection_appliers` already use. The
`Command::SyncAllAdapters` handler loops over it calling `sync_now`,
logging (not propagating) any individual failure — one bad adapter must
not stop the others, the same rule `CalendarPoller::poll_one` already
applies within a single adapter across multiple calendar connections.

Command-line entry point: `/sync`, no arguments.

## Verification

- `cargo fmt --all -- --check` / `cargo clippy --workspace --all-targets --
  -D warnings` / `cargo test --workspace` all green.
- New tests: `sync_now_without_credential_is_a_harmless_no_op` (Slack),
  `sync_now_without_credential_is_a_harmless_no_op` (GitHub),
  `sync_now_without_any_connection_is_a_harmless_no_op` (Calendar) --
  mirroring each adapter's existing `start_without_*_does_not_spawn_a_poll_loop`
  test. `sync_all_adapters_calls_sync_now_on_every_registered_adapter` and
  `sync_all_adapters_keeps_going_after_one_adapter_fails`
  (`crates/commands/src/lib.rs`, against a `MockAdapter` test double).
  `sync_parses_to_sync_all_adapters` (`crates/ui/src/keyboard.rs`).
- Not covered by an automated test: `sync_now`'s actual poll behavior with
  a real token (would make a live network call, same reason `poll_once`
  itself has never had a network-level unit test in this crate --
  `slack.md`/`github.md`/`calendar.md`'s Testing sections already document
  this as the accepted limitation for the polling logic these commands
  reuse unchanged). The `poll_lock` serialization and the `is_first_poll:
  false` correctness argument above are reasoned from reading `run_cycle`/
  `poll_once`'s existing logic directly (already covered by the pure
  mapping-function and rate-limit tests each adapter has), not exercised
  by a new concurrency test — writing a reliable test for a race this
  narrow (two `run_cycle` calls needing to land within the same
  network-round-trip window) felt like more test-infrastructure surface
  than this fix warranted on its own.
- Manually verified: connected Slack with `sync_interval_secs = 900` (15
  minutes) so the background loop wouldn't fire on its own during a quick
  check, sent a message from another client, ran `/sync`, confirmed the
  Notification panel updated and a desktop toast fired immediately rather
  than waiting up to 15 minutes for the next scheduled poll.
