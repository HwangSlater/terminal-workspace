# Implementation Plan - Phase 30: Full visual redesign (Nord palette + live UI)

This is a **design document for review — nothing described below has been
implemented yet**, per the same process used for Phases 6-29. Scope
confirmed directly (four questions, all answered before this doc was
written): a completely new unified color palette (Nord), applied
everywhere at once (main dashboard and every overlay together, not
staged), including new UI patterns (an animated loading spinner and a
Pomodoro progress bar) — not just recoloring. `config.toml`'s dormant
`theme` field and `docs/02-architecture/theme.md`'s theme list stay
aspirational; this phase ships one fixed, hardcoded design, not a
selector.

## Context

Every color decision so far (Phases 19, 26, 27, 28...) picked whatever
`ratatui::style::Color` named variant seemed reasonable in isolation
(`Color::Cyan` for focus, `Color::Yellow` for warnings, `Color::Green` for
success, plain `Modifier::REVERSED` for selection). Functional, but not a
*designed* look — no shared palette, no consistent selection treatment,
and loading states are static text ("불러오는 중...") with no motion at
all. Direct request: make it genuinely pretty, "고도화" (level it up) with
real UI patterns, not just a paint job.

## Decisions

### 1. Palette: Nord, as true 24-bit RGB, applied everywhere

**Confirmed**: [Nord](https://www.nordtheme.com/)'s 16-color palette,
defined as `Color::Rgb(r, g, b)` constants — not `Color::Cyan`/`Color::Red`/etc.,
which resolve to whatever the user's own terminal theme maps them to
(inconsistent, and the whole point of "one unified palette" is to *not*
depend on that). This requires a truecolor-capable terminal; virtually
every terminal in real use today (Windows Terminal, iTerm2, GNOME
Terminal/VTE, Alacritty, kitty, Ghostty...) supports it. Documented as a
known limitation (a terminal without truecolor support will show
approximated/wrong colors, not a crash) rather than silently assumed.

New `crates/ui/src/theme.rs` module: the 16 named Nord constants
(`NORD0`-`NORD15`) plus semantic aliases actually used by call sites
(`ACCENT`, `MUTED`, `SUCCESS`, `WARNING`, `ERROR`, `INFO`, `TEXT`,
`SELECTED_BG`) so a render function reads `theme::SUCCESS`, not a raw
`NORD14` — the semantic name is what changes if a color's *role* ever
needs to move to a different swatch, without touching every call site.

Applied to every panel and overlay in one pass (not staged): header
connection dots, Team presence colors, Notification priority colors,
Calendar panel/grid (weekend/today/event colors from `step28.md`),
every setup/picker overlay's border and status colors, Help overlay
category headers, Log Viewer's WARN/ERROR coloring, command bar error
text.

### 2. Selection highlight: a real highlight color, not bare `REVERSED`

**Confirmed as part of "고도화", not asked as a separate question but
follows directly from "true palette everywhere"**: every list's selected
row currently uses `Modifier::REVERSED` (swap whatever fg/bg already are)
— terminal-safe but visually flat, and it fights a deliberately-chosen
palette by inverting it rather than using it. Replaces `REVERSED` with an
explicit `Style::default().bg(theme::SELECTED_BG).fg(theme::TEXT_BRIGHT).add_modifier(Modifier::BOLD)`
everywhere a row is "the cursor is here" (Team/Notification/Calendar
panels, all three pickers, the Calendar grid's cursor day).

### 3. New UI pattern: an animated loading spinner

**Confirmed** ("로딩 애니메이션" was explicitly named): every static
"불러오는 중..."/"연결 중..."/"재연결 중..." text gets an animated Braille
spinner (`⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏`, the same "dots" spinner most CLI tools use)
prefixed to it. This needs a real capability this app doesn't have yet —
a periodic redraw tick. Today, `crates/ui/src/lib.rs`'s `event_loop` only
redraws in response to a keypress or a domain-bus event
(`crossterm::event::read()` blocks with no timeout, and nothing else
wakes the loop) — an animation would render one static frame and never
advance. Adds a `tokio::time::interval` branch to the `event_loop`'s
`tokio::select!`, ticking every 250ms (smooth enough for a spinner,
without redrawing 10x/sec for no visible benefit — this is a lightweight
desktop tool, not a game). `WorkspaceState` gains `anim_tick: u64`,
incremented each tick; `theme::spinner_frame(tick)` indexes into the
frame list.

Real side benefit, not just the spinner: this also makes the Pomodoro
header countdown *genuinely* live for the first time — today it only
visually updates on the next keypress or integration poll event, despite
`docs/01-product`'s own description calling it real-time.

### 4. New UI pattern: a Pomodoro progress bar

**Confirmed** ("진행률 표시" was the second named example): the header's
Pomodoro segment gains a compact block-character progress bar
(`█`/`░`) showing how far through the current Work/Break phase the
session is, next to the existing `MM:SS` countdown. `PomodoroSnapshot`
gains a `total_secs: u64` field (today it only exposes `remaining_secs`,
not the phase's total duration, so a ratio can't be computed from the
existing shape) — `crates/scheduler` change, not just `crates/ui`.

---

## Proposed Changes

#### [NEW] `crates/ui/src/theme.rs`
Nord RGB constants, semantic aliases, `SPINNER_FRAMES`/`spinner_frame(tick)`,
`progress_bar(ratio, width) -> String`, and a shared `selected_style()`
helper.

#### [MODIFY] `crates/scheduler/src/lib.rs`
`PomodoroSnapshot` gains `total_secs: u64`, populated from
`PomodoroState::current_duration_secs()` in `AgendaScheduler::snapshot()`.

#### [MODIFY] `crates/ui/src/state.rs`
`WorkspaceState` gains `anim_tick: u64` (`#[derive(Default)]` covers it —
`0` is a fine starting frame).

#### [MODIFY] `crates/ui/src/lib.rs`
New `tokio::time::interval(Duration::from_millis(250))` branch in
`event_loop`'s `tokio::select!`: increments `state.anim_tick`, redraws.
`render::render(...)`'s call site passes the tick through.

#### [MODIFY] `crates/ui/src/render.rs`
Every hardcoded `Color::*` call site touched — recolored to a
`theme::*` constant, matched to the closest existing semantic role
(success/warning/error/info/muted/accent). Every bare
`Modifier::REVERSED` selection style replaced with `theme::selected_style()`.
Every "불러오는 중..."/connection-status "...중..." string gets
`theme::spinner_frame(tick)` prefixed. `render_header` gains the
Pomodoro progress bar. `render(...)` gains an `anim_tick: u64` parameter,
threaded to every function that needs a spinner frame.

---

## Verification Plan

- `theme::spinner_frame`/`theme::progress_bar` are pure functions — direct
  unit tests (frame cycling wraps correctly, a few ratio/width
  combinations render the expected block counts, 0%/100%/mid-range).
- Every existing render test asserting a specific `Color::*` (presence
  status, priority, weekend/today, connection status, log level) updated
  to assert the corresponding `theme::*` constant instead — same
  assertions, new expected values, not weaker coverage.
- A render test proving the spinner frame actually changes between two
  different `anim_tick` values passed to `draw`, for at least one loading
  overlay.
- A render test proving the Pomodoro progress bar reflects a known
  `remaining_secs`/`total_secs` ratio.
- `cargo fmt --all --check` / `cargo check --workspace --all-targets` /
  `cargo clippy --workspace --all-targets -- -D warnings` /
  `cargo test --workspace` all green.

---

## Implementation Notes (what actually happened)

All four decisions shipped as designed, no deviations.

**Palette module**: `crates/ui/src/theme.rs` — all 16 Nord swatches named (`NORD0`-`NORD15`, `#[allow(dead_code)]` on the ones with no semantic role yet — background swatches nord0/nord1 in particular, since nothing in this app overrides the terminal's own background), plus the 9 semantic aliases actually used (`ACCENT`, `ACCENT_BRIGHT`, `MUTED`, `TEXT_BRIGHT`, `SUCCESS`, `WARNING`, `ERROR`, `INFO`, `SELECTED_BG`). Bulk-migrated every `Color::Cyan`/`Red`/`Yellow`/`Green`/`Blue`/`DarkGray` call site in `render.rs` (61 occurrences) to its semantic equivalent — mechanical enough (each named `Color` variant had exactly one consistent semantic role everywhere it appeared) that a handful of `sed` passes handled nearly all of it correctly on the first try; only the Calendar grid's cursor-day style (which chained `REVERSED` + `BOLD` on one `Style`, unlike every other selection site's bare `REVERSED`) needed a manual edit.

**Selection highlight**: every bare `Style::default().add_modifier(Modifier::REVERSED)` became `theme::selected_style()` (bg nord2, fg nord6, bold) — same mechanical migration, same one exception (the grid cursor, which now calls `theme::selected_style()` directly instead of chaining two modifiers by hand).

**Animation tick**: `crates/ui/src/lib.rs`'s `event_loop` gained a `tokio::time::interval(Duration::from_millis(250))` branch in its `tokio::select!`, incrementing a new `WorkspaceState.anim_tick: u64` and redrawing. `MissedTickBehavior::Delay` (not the default `Burst`) — a spinner or a countdown catching up with a single tick after a slow draw is fine; firing a *burst* of back-to-back ticks to make up lost time would just make the spinner look like it briefly sped up for no reason. `render()` itself did **not** need a new parameter for this — `anim_tick` lives on `WorkspaceState`, which every render function already receives, so `state.anim_tick` was enough everywhere a spinner frame was needed (simpler than the original proposal's plan to thread a separate `anim_tick: u64` parameter through `render(...)`).

**Spinner**: a hand-rolled 10-frame Braille "dots" spinner (`theme::spinner_frame`), prefixed onto every "불러오는 중..." (4 sites: Slack/GitHub/Calendar pickers, the Calendar grid) and every Connecting/Reconnecting setup/header status text (3 setup overlays + the header's 3 connection-status labels, via a new `tick: u64` parameter on `connection_status_label`).

**Progress bar**: `PomodoroSnapshot` gained `total_secs: u64` (`crates/scheduler`), populated from the already-existing private `PomodoroState::current_duration_secs()`. `pomodoro_label` computes `elapsed_ratio = 1.0 - remaining_secs/total_secs` and renders a 10-cell `theme::progress_bar(...)` bracketed into the existing `MM:SS (Mode)` text.

**Real side effect confirmed working, not just theorized**: the Pomodoro header countdown is now genuinely live — before this phase, `event_loop` only ever redrew on a keypress or a domain-bus event, so the countdown only visually advanced whenever one of those happened to fire (could sit stale for the entire length of a poll interval, up to 900s for Calendar, if the user wasn't typing). The new 250ms tick fixes this as a byproduct of the spinner infrastructure, not a separately-scoped fix.

**Test migration was far cheaper than expected**: because the `sed` recoloring pass touched *every* `Color::*` literal in the file including existing test assertions (`assert_eq!(fg_color_of(&terminal, "..."); Color::Red)` etc.), those assertions came out already pointing at the correct new `theme::*` constant — self-consistent with the production code by construction, not by a second manual pass. Loading-state tests (`contains_ignoring_whitespace(&text, "불러오는 중")`) needed no changes at all, since the spinner glyph is a prefix, not a replacement, of the text they already searched for. Net new tests: 4 pure-function tests for `theme::spinner_frame`/`theme::progress_bar`, 1 for `PomodoroSnapshot.total_secs` staying constant as `remaining_secs` counts down, 1 proving the header's progress bar reflects a known ratio, 1 proving a loading overlay's spinner glyph actually differs between two `anim_tick` values through the full render pipeline (not just `theme::spinner_frame` in isolation). Final counts: `crates/ui` 170 tests (up from 163), `crates/scheduler` 9 (up from 8).

Full `cargo fmt`/`check`/`clippy -D warnings`/`test --workspace` green throughout, including the very first `cargo check` after the bulk recolor — the mechanical migration didn't introduce any type errors, which was not a given going in (some `Color::*` call sites were inside functions returning `Color` vs. `Style`, and a few `match` arms mixed `&'static str` and `String` after the spinner text change, caught and fixed as part of the same pass).
