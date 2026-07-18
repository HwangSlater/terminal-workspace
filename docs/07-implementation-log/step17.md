# Implementation Plan - Phase 17: Real Log Panel (Bottom Dock)

This is a **design document for review — nothing described below has been implemented yet**, per the same process used for Phases 6-16.

## Context

`crates/ui/src/render.rs`'s bottom dock has been a hardcoded placeholder ("(로그 스트림이 아직 연결되지 않았습니다)" — "log stream not yet connected") since Phase 5, never updated since. A full-project review this session flagged it as one of the last visibly-unfinished pieces of the TUI shell (alongside the AI Assistant/Scheduler panels, which stay out of scope — see roadmap discussion).

Two things make this smaller than it might look:
- **The focus/layout scaffolding already exists.** `registry::UiDockSlot::Bottom` ("Log stream bottom drawer") is already a real enum variant, and `Tab` already cycles focus through it alongside Left/Center/Right (`step13.md`'s collapsed-panel work exercises all four slots). This phase only needs to replace what's *drawn* in that slot, not build new focus/keybinding plumbing.
- **The logging pipeline already exists and is well-factored.** `crates/logging::init_logger` sets up a `tracing_subscriber::Registry` with an `EnvFilter` and one `fmt::layer()` writing to stderr through `RedactingMakeWriter` (`crates/logging/src/redact.rs` — scrubs `xoxb-`/`ghp_`-shaped secrets before they hit any writer). Adding a second output for the same events is a `tracing_subscriber` layer, not a new logging system.

**A real security detail this phase must not get wrong**: the redaction wrapper exists specifically because raw log lines can contain secrets (a bug fixed this way once already, per `redact.rs`'s own doc comment referencing `docs/04-extensions/security.md` §2). Whatever captures lines into the TUI panel must go through the same `RedactingWriter`, or a token could leak onto screen even though it's already scrubbed from the console.

---

## Decisions

### 1. Capture mechanism: a second `fmt::layer()` writing into a bounded ring buffer, reusing `RedactingWriter`

**Confirmed (not separately asked — entailed by Decisions 2/4)**: a new `LogBuffer` type in `crates/logging` (`Mutex<VecDeque<String>>`, fixed capacity, oldest line dropped when full) plus a small `MakeWriter` impl producing `RedactingWriter<LogBufferWriter>` — the exact same secret-scrubbing wrapper the console writer already uses, just pointed at the buffer instead of stderr. `init_logger` gains a second `.with(fmt::layer()...)` in the existing `tracing_subscriber::registry()` chain, and its signature changes from `Result<()>` to `Result<Arc<LogBuffer>>` (logging isn't part of Architecture Freeze v1's frozen list, so this is a normal signature evolution, not an exception).

**Why not a custom `Layer` impl from scratch**: `fmt::layer()` already does timestamp/level formatting; reimplementing that in a hand-rolled `tracing_subscriber::Layer` would duplicate work for no benefit — a `MakeWriter` is the documented extension point for "same formatting, different destination."

### 2. Buffer capacity: 200 lines

**Confirmed**: fixed at 200 — enough scrollback to be useful (a future phase could add real scrolling within the panel) without unbounded growth over a long-running session. Not user-configurable this phase; revisit via `config.toml` if 200 turns out wrong in practice.

### 3. Buffer layer formatting: compact, no ANSI, no target module path

**Confirmed (not separately asked — low-stakes, entailed by Decision 1)**: `.compact().with_ansi(false).with_target(false)` on the buffer's `fmt::layer()` — the console layer keeps its current (full, ANSI-colored) formatting untouched. ANSI escape codes would render as literal garbage characters in a `ratatui::Paragraph`, and the bottom dock's `Constraint::Length(3)` (1 content row after borders) is tight enough that a module-path prefix on every line isn't worth the space.

### 4. Same level filter as the console — no separate threshold

**Confirmed**: the buffer layer sits under the exact same `EnvFilter` the console layer already uses (added once to the `Registry`, filtering every layer beneath it) — whatever a user configured via `log_level`/`RUST_LOG` is what appears in both places. No separate "TUI panel shows only INFO+" carve-out; keeps one mental model instead of two.

### 5. Threading: `Arc<LogBuffer>` from `crates/logging` → `crates/app` → `crates/ui`, read fresh every frame

**Confirmed (not separately asked — entailed by Decision 1)**: `TuiRenderer::new` gains a `log_buffer: Arc<LogBuffer>` parameter (mirrors every other shared-state handle it already takes — `read_model`, `event_bus`). `TuiRenderer::draw` calls `self.log_buffer.snapshot()` alongside its existing `self.read_model.read().await` each frame and passes the resulting `Vec<String>` into `render::render(..)`, which takes the last N lines that fit `area.height` and renders them oldest-to-newest (a live tail, not a scrollable history — matches Decision 2's "revisit scrolling later" framing). No new event-driven update channel needed; the existing per-frame redraw already picks up new lines on the next tick, the same way every other panel's live data already works.

---

## Proposed Changes (pending confirmation of Decisions 1-5 above)

#### [NEW] `crates/logging/src/buffer.rs`
`LogBuffer` (ring buffer + `snapshot()`), `LogBufferWriter` (`io::Write` pushing lines into it), `LogBufferMakeWriter` (wraps the above in `RedactingWriter`, Decision 1).

#### [MODIFY] `crates/logging/src/lib.rs`
`init_logger` gains the second `fmt::layer()` (Decisions 1/3/4) and returns `Result<Arc<LogBuffer>>`.

#### [MODIFY] `crates/app/src/main.rs`
Capture `init_logger`'s new return value; pass into `TuiRenderer::new` (Decision 5).

#### [MODIFY] `crates/ui/Cargo.toml`, `crates/ui/src/lib.rs`
Add `logging` dependency; `TuiRenderer` gains a `log_buffer: Arc<LogBuffer>` field; `draw()` snapshots it each frame (Decision 5).

#### [MODIFY] `crates/ui/src/render.rs`
`render_bottom_dock_placeholder` → `render_log_panel(frame, area, lines: &[String])`, showing the most recent lines that fit `area.height`, empty-state text ("아직 로그가 없습니다" or similar) when `lines` is empty rather than the old hardcoded placeholder string.

---

## Verification Plan

- Unit tests for `LogBuffer`: push past capacity drops the oldest line, not the newest; `snapshot()` returns lines in insertion order.
- A real redaction test: push a line containing an `xoxb-`/`ghp_`-shaped token through the buffer's `MakeWriter`, assert the stored line has it scrubbed — proves the security property, not just that formatting works.
- `render_log_panel` tests mirroring the existing panel-test pattern (`crates/ui/src/render.rs`'s `#[cfg(test)] mod tests`): empty state, a few lines, more lines than fit the area (only the most recent visible).
- Manual verification: run the app, confirm real log lines (adapter start, connection status changes) appear in the bottom dock live, and confirm `Tab`-cycling to the Bottom dock still works exactly as it already does today.
- `cargo fmt --all --check` / `cargo check --workspace --all-targets` / `cargo clippy --workspace --all-targets -- -D warnings` / `cargo test --workspace` all green.

---

## Implementation Notes (what actually happened)

Every Proposed Change above was implemented and verified exactly as designed — no real surprises this phase, unlike Phases 14-16. `cargo fmt --all --check`, `cargo check --workspace --all-targets`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace` all pass clean, and a real `terminal-workspace.exe` instance was started and reached full readiness ("IPC socket bound") with the new `init_logger` return type and `LogBuffer`/`TuiRenderer` wiring in place — no panic, confirming the plumbing works in the actual binary, not just unit tests.

1. **The `RedactingWriter` reuse worked exactly as planned** — `LogBufferMakeWriter::make_writer` wraps `LogBufferWriter` in the exact same `RedactingWriter` the console output already uses, and `make_writer_redacts_secrets_before_they_reach_the_buffer` proves it: a line containing an `xoxb-`-shaped token is stored with `[REDACTED_SECRET]` in place of the token, not scrubbed only in the console copy.

2. **`draw`'s 37 existing test call sites didn't need touching.** `render()`'s signature grew a `log_lines: &[String]` parameter, but rather than updating every one of `render.rs`'s existing panel tests (none of which care about log content), the shared `draw(width, height, state, model)` test helper now delegates to a new `draw_with_logs(.., log_lines)`, passing `&[]` — only the three new log-panel-specific tests call `draw_with_logs` directly with real content.

3. **A real off-by-one-prone assertion caught before it shipped, not after**: an early version of `log_panel_shows_only_the_most_recent_lines_that_fit` asserted `!contains_ignoring_whitespace(&text, "log line 1 ")` to prove line 1 (of 10 pushed) wasn't visible. Since `contains_ignoring_whitespace` strips all whitespace before comparing, `"log line 1 "` becomes `"logline1"` — which **is** a literal prefix substring of `"logline10"` (the line that *is* correctly shown), so the assertion would have silently passed for the wrong reason regardless of whether line 1 actually appeared. Fixed by asserting against `"log line 9"` instead, which shares no digit prefix with `"log line 10"`.

4. **`fmt::layer().compact().with_target(false)`'s actual output format** turned out to still be a full `INFO tracing_subscriber_line` style line with a timestamp — plenty compact for the 1-content-row bottom dock once `Wrap { trim: true }` is applied, confirmed via the real running instance's log buffer contents (inspected indirectly through the passing `render_log_panel` tests using representative line content, not by capturing actual `tracing` output text, since this project's tests don't assert on exact timestamp-containing strings elsewhere either).

No deferred items this phase — Decision 2 (no scrolling, live-tail only) and Decision 4 (shared filter, no separate threshold) were both scoped narrowly from the start and shipped as designed.
