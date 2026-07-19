# Implementation Plan - Phase 42: Stop the picker/command config save from reverting unrelated fields

Real bug fix, found while looking for further hardening work directly
requested ("고도화 할 작업 있는지 찾아볼래?"). Skips the Decisions/
`AskUserQuestion` cycle — one clear root cause, one reasonable fix.

## Context

Not a new report — this is closing a gap flagged twice already and never
fixed:

- `step33.md`'s Verification section: "A real local `config.toml` issue
  was found and fixed by hand in the same session... flagged here as a
  known quirk worth a future look, not fixed as part of this change."
- Directly after that, the user was asked "이 부분도 고칠까요?" about the
  underlying architecture and never answered — left open rather than
  guessed at, per this log's own discipline. Revisited now that it was
  asked to look for outstanding hardening work.

## Root cause

`ConfigFileSlackSelectionApplier`/`ConfigFileGitHubSelectionApplier`
(`crates/app/src/main.rs`, backing `Command::ApplySlackSelection`/
`ApplySelection` — dispatched by `Ctrl+P`/`Ctrl+R` and, as of `step41.md`,
`/slack-watch`/`/repo-watch` too) each held a `base_config: Mutex<AppConfig>`
snapshotted once, at process construction (`config.clone()`). Every save
mutated a couple of fields on *that* stale copy, then called
`AppConfig::save_to`, which always serializes and writes the **whole**
struct — silently reverting any other field that had changed on disk
since the process started, back to whatever it was at startup. This is
exactly what happened to the reporting user: `step32.md` raised
`right_dock_width`'s default from 32 to 60, but a Slack picker save
re-wrote the stale `32` right back on every subsequent save, because the
in-memory snapshot never advanced past whatever was on disk when the
process launched.

## Fix

`apply()` now calls `AppConfig::load_or_create_default()` fresh,
immediately before mutating and saving, instead of locking a
process-lifetime snapshot. This is the exact same function the real boot
path already uses, so behavior is otherwise identical (same
file→env→CLI merge order) — the only change is *when* the file gets
read: right before each write, not once at the very start of the
process. `base_config` (and the now-unused `tokio::sync::Mutex` import)
were removed entirely rather than left dead.

Calendar's selection applier (`CalendarSelectionApplierBridge`) was
never affected by this — every field of a calendar connection is a
secret, so it has no `config.toml` bridging at all (`CalendarAdapter::
keep_only` handles its own persistence via `SecretWriter`).

## Verification

- `cargo fmt --all -- --check` / `cargo clippy --workspace --all-targets --
  -D warnings` / `cargo test --workspace` all green — no test exercised
  `ConfigFileSlackSelectionApplier`/`ConfigFileGitHubSelectionApplier`
  directly before this change either (private, composition-root-only
  glue in `crates/app/src/main.rs`, no `crates/app/tests/` coverage of
  it), so none needed updating; the underlying `AppConfig::load_or_create_default`/
  `save_to`/`parse` round-trip these call are already covered by
  `crates/config`'s existing tests, unchanged by this fix.
- Not covered by an automated test: the actual regression (a stale field
  reverting on save) would need a test that constructs the applier,
  writes a config file, mutates a field the applier doesn't touch,
  triggers `apply()`, and asserts the untouched field survived — doable,
  but would require exposing these currently-private structs as a
  testable seam, which felt like more surface change than this fix
  warranted on its own. Flagged as a reasonable follow-up if this class
  of bug recurs.
- Manually verified: edited a real `config.toml`'s `[layout]` value by
  hand, triggered a Slack channel selection save (`Ctrl+P`), confirmed
  `right_dock_width` was preserved instead of reverting.
