# Implementation Plan - Phase 18: Pomodoro Timer (Scheduler)

This is a **design document for review — nothing described below has been implemented yet**, per the same process used for Phases 6-17.

## Context

`crates/scheduler` has been a ~25-line stub since Phase 2, never wired into `crates/app/src/main.rs`. `docs/03-domain/scheduler.md` (flagged unbuilt in this session's docs-honesty pass) sketches a broad "Scheduler Bounded Context" covering reminders, timers, local agenda logging, *and* Pomodoro trackers — `SchedulerEvent`/`TriggerPolicy`/`RecurrenceRule` for generic scheduled events, plus a separate `PomodoroState`.

This phase builds **the Pomodoro timer only**, matching the roadmap discussion's own framing ("Scheduler 패널 (Pomodoro/일정 알림)") and this project's repeated "don't build for hypothetical needs" discipline. The generic recurring-reminder machinery (`SchedulerEvent`, `RecurrenceRule`, multiple `TriggerPolicy` variants) is deliberately deferred — nothing needs it yet, and Calendar already covers "remind me about a calendar event" via `Event::CalendarReminderTriggered` (`step12.md`). `docs/03-domain/scheduler.md` keeps its existing Implementation Status note, amended to point at this phase for the Pomodoro slice specifically.

---

## Decisions

### 1. State is computed, not ticked

**Confirmed**: `PomodoroState` stores `mode` (`Work`/`ShortBreak`/`LongBreak`), `session_count`, `is_running`, `started_at: Option<SystemTime>`, and the configured work/break durations — **not** a `remaining_seconds` counter decremented once a second. Remaining time is derived at read time (`now - started_at`, clamped to the configured duration) — the same "compute fresh each frame" approach `step17.md`'s log panel just established, avoiding a per-second background write to shared state for something that's only ever displayed, never persisted mid-session.

### 2. One background task per running session, cancellable via `tokio::sync::Notify`

**Confirmed**: `AgendaScheduler::run_loop` (finally given real logic) owns a `tokio::select!` between `tokio::time::sleep(remaining_until_session_end)` and a `Notify` that `start`/`pause`/`resume`/`reset` all signal — so a stale sleep from before a pause/reset never fires a stale "session ended" trigger. When a sleep actually completes (not interrupted), the scheduler auto-transitions the mode (Work → a break, break → Work, incrementing `session_count` on every completed Work session) and fires the trigger (Decision 3).

### 3. Trigger action: terminal bell + reused `Event::SystemAlert`, not a new `Event` variant

**Confirmed**: on session end, print the ASCII bell character (`\x07`) — picked up by most terminals as an audible/visual alert without needing a TUI popup overlay — and publish `Event::SystemAlert(String)` with a human-readable message ("Work session complete — take a break!"). `Event` is frozen by Architecture Freeze v1 (`development.md` §3); reusing the existing generic `SystemAlert` variant avoids reopening it, the same choice `step14.md` made for plugin events (`PluginCustomEvent`) rather than adding new variants for everything.

### 4. Command surface: start / pause (toggle) / reset

**Confirmed**: three new `Command` variants — `StartPomodoro { work_minutes: u32, break_minutes: u32 }`, `PausePomodoro` (toggles running/paused — resuming is "pause" again while paused, not a separate command), `ResetPomodoro`. CLI syntax mirrors the existing `/` command bar convention (`step9.md`): `/pomodoro start [work_min] [break_min]` (defaults 25/5 if omitted), `/pomodoro pause`, `/pomodoro reset`.

### 5. Displayed in the header, not a new panel/overlay

**Confirmed**: a fourth segment in the existing header line (`crates/ui/src/render.rs::render_header`, already `Slack | GitHub | Calendar | ...`) showing e.g. `🍅 24:35 (Work)` while running, nothing extra when idle — matches how connection statuses already live there rather than adding a new dock/overlay for a single always-visible number. `Ctrl+1~4`/dock-focus machinery is unaffected; the Pomodoro segment isn't focusable, same as the rest of the header.

### 6. Ownership & wiring: `Arc<AgendaScheduler>` shared between `commands` (mutation) and `ui` (display)

**Confirmed**: `crates/scheduler`'s `AgendaScheduler` becomes the single owner of `PomodoroState` (an internal `RwLock`, mirroring `SharedReadModel`'s shape). `crates/commands` gains a dependency on `crates/scheduler` (consistent with its existing dependency on `crates/integration` for other command targets); `WorkspaceCommandHandler` gains a plain `Arc<AgendaScheduler>` field (not `Option` — Pomodoro isn't gated behind "configured or not" the way integrations are, it's always available). `crates/app/src/main.rs` constructs one `Arc<AgendaScheduler>`, passes it to both `WorkspaceCommandHandler::new` and `TuiRenderer::new` (a new parameter, same pattern as `step17.md`'s `log_buffer`), and spawns `run_loop()` as a background task.

---

## Proposed Changes (pending confirmation of Decisions 1-6 above)

#### [MODIFY] `crates/scheduler/src/lib.rs`
`PomodoroState`/`PomodoroMode` (Decision 1), real `AgendaScheduler` with `start`/`pause`/`reset`/`snapshot` methods and a real `run_loop` (Decisions 2-3).

#### [MODIFY] `crates/commands/Cargo.toml`, `crates/commands/src/lib.rs`
New dependency on `scheduler`; three new `Command` variants (Decision 4); `WorkspaceCommandHandler` gains the `Arc<AgendaScheduler>` field and dispatches the three variants to it (Decision 6).

#### [MODIFY] `crates/ui/Cargo.toml`, `crates/ui/src/lib.rs`, `crates/ui/src/keyboard.rs`, `crates/ui/src/render.rs`
`TuiRenderer` gains an `Arc<AgendaScheduler>` field, snapshotted each frame like `log_buffer`; `render_header` gains the Pomodoro segment (Decision 5); `keyboard.rs` gains `/pomodoro start|pause|reset` parsing alongside the existing `/send`/presence commands (Decision 4).

#### [MODIFY] `crates/app/src/main.rs`
Construct `Arc<AgendaScheduler>`, wire into both `WorkspaceCommandHandler::new` and `TuiRenderer::new`, spawn `run_loop()`.

#### [MODIFY] `docs/03-domain/scheduler.md`
Amend the existing Implementation Status note: the Pomodoro slice is real as of this phase; `SchedulerEvent`/`RecurrenceRule`/generic reminders remain unbuilt.

---

## Verification Plan

- Unit tests for remaining-time computation (Decision 1): a session started N seconds ago with duration D reports `D - N` remaining, clamped to 0.
- A real cancellation test (Decision 2): start a session, `reset` before it would naturally end, assert no session-end trigger fires (the interrupted sleep never completes) — the single most important test in this phase, mirroring `step14.md`'s fuel-trap test as "the one that actually proves the mechanism, not just the happy path."
- Command dispatch tests: `StartPomodoro`/`PausePomodoro`/`ResetPomodoro` each mutate `AgendaScheduler` state correctly through `WorkspaceCommandHandler`.
- `/pomodoro` command-bar parsing tests (valid start with/without args, pause, reset, invalid subcommand) mirroring `crates/ui/src/keyboard.rs`'s existing `/send`/presence test patterns.
- Manual verification: run the app, `/pomodoro start 1 1` (short durations for a fast manual check), confirm the header shows a live countdown and the terminal bell fires (audibly or as a visual flash, depending on terminal) when the work session ends.
- `cargo fmt --all --check` / `cargo check --workspace --all-targets` / `cargo clippy --workspace --all-targets -- -D warnings` / `cargo test --workspace` all green.

---

## Implementation Notes (what actually happened)

All six Decisions were implemented as designed, with one deliberate simplification and two real concurrency/timing bugs found and fixed along the way.

**Deviation from Decision 1**: `PomodoroMode` shipped as `Work` / `Break` only, not `Work` / `ShortBreak` / `LongBreak`. Nothing in this phase's scope (a single work/break cycle, no "every 4th session gets a long break" rule) needed the third variant, so it was dropped rather than built and left unused — consistent with this project's "don't build for hypothetical needs" discipline, same reasoning the Context section already applied to deferring `SchedulerEvent`/`RecurrenceRule`. `started_at` also ended up typed as `tokio::time::Instant`, not `SystemTime` as Decision 1's prose said — see the bug below.

**Bug 1 — lost-notification race with `Notify::notify_waiters()`**: the first `run_loop` implementation used `notify_waiters()`, which only wakes tasks *already* registered as waiting on `.notified()`. If `start()`/`pause()`/`reset()` fired before the spawned loop task reached its `.notified().await` point, the notification was silently dropped and the loop got stuck waiting on an event that had already happened. Diagnosed with temporary `eprintln!` tracing showing the loop never re-evaluating after `start()`. Fixed by switching every call site to `notify_one()`, which buffers a single permit when nobody's listening yet, making the interrupt safe regardless of task-poll ordering.

**Bug 2 — `SystemTime` vs. `tokio::time::Instant` under paused-clock tests**: `elapsed_secs()` originally read `std::time::SystemTime::now()` while the background task slept on `tokio::time::sleep`, which (under `#[tokio::test(start_paused = true)]` + `tokio::time::advance()`) only moves tokio's virtual clock, not the real wall clock. The two diverged, so `on_session_ended`'s safety re-check (`remaining_secs() > 0`) always saw stale near-zero elapsed time and silently aborted — the sleep completed but the transition never fired. This isn't just a test artifact: `SystemTime` is also wrong in production if the system clock is ever adjusted mid-session, while `Instant` is monotonic. Fixed by switching `PomodoroState::started_at` and all `.now()` call sites to `tokio::time::Instant`.

**Environment note**: mid-implementation, `cargo test --workspace` failed with a linker "No space left on device" error — the `target/` directory (grown large from the Phase 14 `wasmtime`/`cranelift` plugin runtime dependencies) had filled the disk. Resolved with `cargo clean` (safe: gitignored, fully regenerable build cache), freeing 36.9 GiB. Full `fmt`/`check`/`clippy`/`test` re-verified clean after the resulting full rebuild.

Final state: 8 new tests in `crates/scheduler` (state computation, pause/resume, the cancellation-prevents-stale-trigger test called out in the Verification Plan as the most important one, and a real session-end-under-paused-clock test), 4 new dispatch tests in `crates/commands` (25 total), 7 new `/pomodoro` parsing tests in `crates/ui/src/keyboard.rs`, 3 new header-rendering tests in `crates/ui/src/render.rs` (101 total in `ui`). Full `cargo test --workspace` green with no regressions. `WorkspaceCommandHandler::new` grew to 8 constructor arguments; `#[allow(clippy::too_many_arguments)]` added, matching the existing precedent on `TuiRenderer::new`.
