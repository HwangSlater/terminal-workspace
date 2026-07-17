# Implementation Plan - Phase 5: Interactive TUI Shell

This is a **design document for review — nothing described below has been implemented yet**, per the same process used for Phase 4.

## Context

Unlike Phases 2-4, the *target* design for this phase already exists in full and predates all of it: `docs/02-architecture/ui.md` (layout + reactive state machine), `docs/02-architecture/keyboard.md` (modal input + capture pipeline), `docs/03-domain/workspace-state.md` (`WorkspaceState`/`DockSlot`/`CommandBufferState`), `docs/01-product/screen-spec.md` (ASCII mockups, responsive rules), and ADR-0012 (docking system decision). Phase 5 is **implementation of an already-frozen spec**, not a new design — the open questions below are about *how to build it*, not *what to build*, and about *what to leave out* given what doesn't exist yet.

Two things this phase closes that earlier phases explicitly deferred:
- `crates/ui::TuiRenderer` is currently a stub that logs one line and returns (`crates/ui/src/lib.rs`).
- `crates/app/src/main.rs` currently boots infrastructure, dispatches one proof-of-concept command, and **exits**. Phase 5 is what turns this into a program a person actually runs and stays inside.
- ADR-0007 deferred the CQRS *read* path ("`DashboardReadModel` + Projector... belongs with whichever future phase adds a UI consumer") — that consumer is this phase.

---

## Decisions (confirmed)

1. **Scope**: shell mechanics only — docking, modal keyboard, command bar, header/footer, Team Panel + Notification Panel on real (currently empty) data. Calendar/CI/AI Assistant/autocomplete/plugin panels deferred until their data sources exist.
2. **Crate placement**: `DashboardReadModel` + `Projector` → `crates/commands` (paired with the existing CQRS write side). `WorkspaceState` → `crates/ui`.
3. **Input threading**: `tokio::task::spawn_blocking` + `mpsc` channel, per `ui.md`'s existing spec — not a new decision, already dictated. (Revised during implementation — see Implementation Notes: a plain OS thread is used instead, for a correctness reason `ui.md`'s original spec didn't anticipate.)

---

### 1. Scope: shell mechanics now, real panel data later

No Slack/GitHub/Calendar/CI adapter exists yet (`crates/integration` is a stub; per `docs/01-product/roadmap.md`, Slack lands at v0.1, GitHub at v0.3, Calendar at v0.4 — all *after* this internal phase). Building `screen-spec.md`'s full mockup (live PR reviews, CI status, AI chat) now would mean faking data. Proposed split, mirroring how Phase 3 handled `SendSlackMessage` (honest "not implemented" error instead of a fake success):

**In scope (Phase 5):**
- Docking shell (Left/Center/Right/Bottom per ADR-0012), Header bar, Command bar, Status footer, responsive collapse rule (`screen-spec.md` §3).
- Modal keyboard system exactly per `keyboard.md` (Normal/Input/Overlay modes, global shortcuts, capture pipeline, vim navigation).
- `DashboardReadModel` + Projector (the ADR-0007 read path), fed by real `PresenceRepository`/`NotificationRepository` data via `redb`.
- **Team Panel** and **Notification Panel**, rendering that real (currently empty) data honestly — an empty team list is correct, not a bug, until an integration exists to populate it.
- Replacing `main.rs`'s boot-and-exit with an actual run loop.

**Explicitly out of scope, deferred until their data source exists:**
- Detail Pane's Calendar grid (needs Calendar integration).
- CI/CD Status panel (no CI integration scoped anywhere yet).
- AI Assistant panel (`crates/assistant` is still a stub).
- Command palette autocomplete (today's `Command` enum only has 4 variants — not enough to make autocomplete meaningful).
- Plugin-registered panels actually rendering (`plugin-host`/`plugin-sdk` are stubs).

### 2. Crate placement for the new pieces

- **`DashboardReadModel` + `Projector`** → `crates/commands`. This is the CQRS *read* half; `crates/commands` already holds the *write* half (`Command`, `CommandHandler`, `CommandDispatcher`). `Projector` implements the existing `events::EventHandler` trait and updates the read model — natural pairing, avoids a new crate for two structs.
- **`WorkspaceState` + `DockSlot`/`ActiveLayout`/`CommandBufferState`** → `crates/ui`. Pure presentation/focus concern, no domain data of its own — stays where the existing (stub) `TuiRenderer` already lives.

### 3. Threading model for input

`ui.md` already specifies non-blocking input via a separate task. Concretely: `crossterm::event::read()` is blocking, so it runs off the render loop, forwarding parsed key events over an `mpsc` channel. Redraws are event-triggered (state-change-driven, per `ui.md`'s "reactive, not polling"), not a fixed-interval tick. (The original spec assumed `tokio::task::spawn_blocking`, same idiom as `redb` calls in `crates/storage` — during implementation this turned out to be wrong for this specific case; see Implementation Notes.)

---

## Proposed Changes

#### [MODIFY] `crates/ui/src/lib.rs`
- Real `TuiRenderer`: terminal setup/teardown (raw mode, alternate screen — with a panic hook that restores the terminal, so a crash doesn't leave the user's shell broken), the event-triggered render loop, `WorkspaceState`, `DockSlot`/`ActiveLayout`/`CommandBufferState`, keyboard capture pipeline, and render functions for Header/Team Panel/Notification Panel/Command Bar/Footer.

#### [MODIFY] `crates/commands/src/lib.rs`
- `DashboardReadModel` (in-memory, per `command-model.md` §4) and `Projector` (`impl EventHandler`), populated from `PresenceRepository`/`NotificationRepository` on startup and kept current via dispatched events.

#### [MODIFY] `crates/app/src/main.rs`
- Register `Projector` on the `EventDispatcher` alongside existing handlers; replace the current "dispatch one command, log success, exit" ending with `TuiRenderer::run_loop()`, which runs until `Ctrl+Q`.

#### [MODIFY] Docs (add "Implementation Status" notes, matching the Phase 2/3 pattern — no redesign, just marking built-vs-deferred)
- `docs/02-architecture/ui.md`, `docs/02-architecture/keyboard.md`, `docs/03-domain/workspace-state.md`, `docs/01-product/screen-spec.md`: note what Phase 5 actually builds vs. defers.
- `docs/06-development/decisions/0007-cqrs.md`: amendment — read path now implemented, closing the Phase 3 deferral.
- `docs/06-development/decisions/0012-docking-system.md`: amendment — implementation crate location.

---

## Verification Plan

- **Ratatui `TestBackend` snapshot tests** (`docs/05-operations/testing.md` §4): default-size layout, `<120`-column sidebar collapse, `<80x24` "too small" placeholder.
- **Keyboard pipeline unit tests**: mode transitions (Normal/Input/Overlay), global shortcuts taking precedence over pane-specific and plugin shortcuts (`keyboard.md`'s explicit rule).
- **Manual run**: `cargo run -p app` and actually use it — this is the first phase where that sentence means something; Phases 2-4 only ever booted and exited.

---

## Implementation Notes (what actually happened)

- `crates/commands`: `DashboardReadModel` + `Projector` implemented, with unit tests covering initial population from storage and event-driven upsert (presence and notifications, including re-delivery not duplicating entries).
- `crates/ui`: split into `state.rs` (`WorkspaceState`/`ActiveLayout`/`FocusMode`/`CommandBufferState`), `keyboard.rs` (capture pipeline + 11 unit tests covering mode transitions, global-shortcut precedence, and text editing), and `render.rs` (ratatui drawing + `TestBackend` tests, plus Korean localization — see below). `DockSlot` reuses `registry::UiDockSlot` as decided.
- `crates/app/src/main.rs`: `Projector` registered on `EventDispatcher`; the run now ends with `TuiRenderer::run_loop()` instead of exiting.
- One scope note not called out earlier: the Command Bar's `Enter` key pushes to history but does **not** yet parse/dispatch the typed text as a `Command` — there's no text-to-`Command` parser, and with only 4 `Command` variants and no live integrations, building one now would have nothing meaningful to parse into. Left as a clearly-commented gap rather than a silent one.
- **Input threading correction**: the original plan called for `tokio::task::spawn_blocking`, matching the `redb` idiom. This turned out to be a real bug, not a style choice — `crossterm::event::read()` blocks indefinitely with no cancellation, and `tokio::Runtime::drop` joins outstanding `spawn_blocking` tasks at shutdown. If the user's last keypress was `Ctrl+Q` and they then stopped typing, the in-flight blocking read never returned and the process hung forever instead of exiting. Fixed by moving the reader onto a plain `std::thread::spawn` OS thread, which is simply abandoned on process exit rather than joined (`crates/ui/src/lib.rs::spawn_input_reader`, reasoning documented inline).
- **`redb` missing-table bug found via testing**: a fresh `workspace.redb` (nothing written yet) raised `redb::TableError::TableDoesNotExist` on the first read, which `crates/storage`'s `read_entry`/`scan_entries` was propagating as an error instead of treating as "no data yet." Would have crashed the app on a brand-new install. Fixed in `crates/storage/src/lib.rs` (`is_missing_table` check).
- **Korean localization**: all user-facing `render.rs` strings (panel titles, empty states, footer, header, help overlay, presence/source labels) translated to Korean, plus a new `?`-triggered help overlay (the key worked internally but rendered nothing before this). Root cause of a related test-assertion failure during this work: ratatui pads the second cell of every wide (CJK) glyph with a literal space, which broke naive substring checks — fixed via a whitespace-stripping comparison helper in the test module, not a rendering bug.
- **Verification reality**: this phase is the first one where verification is not partial. A working GCC-based MinGW-w64 toolchain (WinLibs, not LLVM-MinGW — see README's Windows troubleshooting section) was installed during this session, resolving the `dlltool.exe` gap noted since Phase 2. With it on `PATH`, `cargo check --workspace --all-targets`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`, and **`cargo test --workspace` all ran and passed, 17/17 crates, zero failures** — including the two real bugs above, both of which `cargo check`/`clippy` alone could never have caught.

