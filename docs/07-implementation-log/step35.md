# Implementation Plan - Phase 35: Stop background log lines from tearing through the live TUI screen

Real bug fix, root-caused via a live screenshot the user pasted directly —
skips the Decisions/AskUserQuestion cycle (there was only one reasonable
fix once the cause was clear), documented here per `development.md`'s own
rule that a change still needs a record even when it didn't need up-front
confirmation.

## Context

User pasted a real terminal capture: the Calendar panel and its borders
were visibly corrupted, with raw lines like

```
2026-07-19T13:50:58.298259Z  WARN rrule::parser::content_line::date_content_line: Parameter `DATE` is not supported...
```

interleaved mid-frame, breaking box-drawing characters and misaligning
panel content — "디자인 깨진다" (the layout is breaking). This wasn't a
one-time startup glitch; it kept recurring throughout the session,
correlated with the Calendar adapter's background poll loop hitting
non-fatal `rrule` parser warnings on real feed data.

## Root cause

`init_logger` (`crates/logging/src/lib.rs`) composed two `tracing_subscriber`
`fmt::layer()`s: one feeding the TUI's own in-app `LogBuffer` (correctly
isolated, only ever read by `Ctrl+4`'s log viewer overlay), and a second,
human-formatted one writing straight to **stderr** via `RedactingMakeWriter`
— present since Phase 2 (`docs/05-operations/logging.md` §3), predating the
TUI entirely. The TUI (`ratatui` + `crossterm`) owns the terminal through
raw mode and an alternate screen, but that control is over the terminal's
*display state*, not over what gets written to the underlying file
descriptors — stderr is the same physical terminal, unbuffered, and every
`tracing::warn!`/`info!` call anywhere in the process (including from a
background `tokio::spawn`ed poll loop, not just the main thread) wrote a
raw formatted line straight through the middle of whatever frame ratatui
had just drawn.

This was always latent — any `WARN`/`ERROR` at any point during a TUI
session would have caused it — but stayed invisible until a poller started
actually emitting warnings during real use (the `rrule` crate's
`RDATE`/`DATE` parameter warning, itself benign and unrelated to this fix).

Separately, `docs/05-operations/logging.md` §1's own destination table had
always specified file destinations (`app.log`/`error.log`), never
implemented — Phase 2 shipped stderr instead and the table was never
corrected to match. This phase finally closes that gap rather than papering
over it.

## Fix

- `crates/logging/src/redact.rs`: generalized `RedactingMakeWriter` from a
  stderr-only unit struct into `RedactingMakeWriter<M>`, wrapping any inner
  `MakeWriter` (`RedactingMakeWriter::new(inner)`). The scrubbing behavior
  itself (`RedactingWriter`, `redact_secrets`) is unchanged.
- `crates/logging/src/lib.rs`: `init_logger` now writes the formatted-text
  layer to a daily-rotating file (`tracing-appender`'s
  `rolling::daily(standard_log_dir(), "app.log")`, wrapped in
  `RedactingMakeWriter` exactly as before) instead of stderr.
  `standard_log_dir()` mirrors `storage::standard_db_path()`'s own
  `AppData/Local`/`.local/share` resolution, one level down in `logs/`.
  `init_logger`'s return type changed from `Result<Arc<LogBuffer>>` to
  `Result<(Arc<LogBuffer>, WorkerGuard)>` — `tracing-appender`'s
  non-blocking writer needs its `WorkerGuard` kept alive for the process's
  whole lifetime (dropping it early silently truncates the log file).
- `crates/app/src/main.rs`: call site updated to
  `let (log_buffer, _log_guard) = init_logger(...)?;`, bound once near the
  top of `main` and never touched again — it naturally lives until process
  exit, which is exactly the lifetime it needs.
- The in-app `LogBuffer`/`Ctrl+4` layer is untouched — it already was, and
  remains, the correct "watch logs live without leaving the TUI" surface.

Also fixed two small leftover strings from `step32.md` noticed while
reading through this same rendering code: the `?` help overlay
(`HELP_CATEGORIES` in `crates/ui/src/render.rs`) still listed `Ctrl+1~3`
with the description "패널로 바로 이동 (팀/알림/캘린더)", and the status
footer still said `Ctrl+1~3:포커스 이동` — both stale since `step32.md`
removed `Ctrl+1`/Team from the dock-focus cycle entirely (`Tab`/`Shift+Tab`
already correctly said `Ctrl+2~3` at every other call site in the same
file). Corrected to `Ctrl+2~3` and "패널로 바로 이동 (알림/캘린더)" in
both places. `docs/02-architecture/ui.md`'s Implementation Status banner
had the same stale `Ctrl+1~3` mention and is amended alongside.

## Verification

- `cargo fmt --all -- --check` / `cargo clippy --workspace --all-targets --
  -D warnings` / `cargo test --workspace` all green — no test asserted the
  exact stderr-destination behavior or the stale `Ctrl+1~3` strings, so
  nothing needed rewriting, only the production strings themselves changed.
- Manual run (`cargo run -p app`): confirmed a `logs/app.log.<date>` file
  is created under the resolved `standard_log_dir()` and receives the
  startup `INFO` lines in the same formatted-text shape stderr used to get;
  confirmed stderr itself stays silent of tracing output (the only stderr
  line seen was `main`'s own top-level `Error: ...` propagation on an
  unrelated pre-existing "database already open" condition from another
  running instance, via the ordinary `Result` return path, not `tracing`).
- Did not add a direct unit test for `init_logger`/`standard_log_dir`
  themselves — consistent with this codebase's existing pattern of not
  unit-testing process-wide `tracing_subscriber::registry().init()`
  wiring (a global, one-time-per-process side effect); the manual run
  above is the acceptance check, the same boundary already documented for
  live-network poll bodies elsewhere in this log.
