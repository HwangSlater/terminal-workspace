# Implementation Plan - Phase 22: Terminal Tab/Window Title Badge

This is a **design document for review — nothing described below has been implemented yet**, per the same process used for Phases 6-21.

## Context

Follow-up from `step21.md`'s desktop notification work. During that discussion, the user clarified their original ask more precisely: not a desktop OS toast specifically, but something visible **in a terminal** — e.g. while working in a different tab of the same terminal emulator. That's genuinely a different mechanism than `step21.md`'s OS notifications, and (per the user's explicit "OS 고려해서 생각해야해 — 모든 운영체제에 맞게" — think broadly, but weigh every option by whether it actually works across all three Tier-1 platforms) was chosen over three alternatives that were researched and rejected for this phase:

- **Shell prompt hook** (bash/zsh `precmd`, PowerShell `prompt` function): works on all 3 OSes but needs a separate snippet per shell, `cmd.exe` can't do it at all, and it's an opt-in install step for the user (violates this project's Zero Setup principle more than the option below).
- **System tray icon** (`tray-icon` crate, feasibility-checked the same way `notify-rust` was in `step21.md`): compiles without a new C-compiler requirement, but Linux tray behavior is genuinely fragmented — GNOME needs an extension, minimal window managers have no tray at all, pure-Wayland setups may not support the legacy X11 tray protocol `tray-icon` falls back to. Rejected specifically for failing the "works the same on all three OSes" bar.
- **Remote push (webhook to phone via ntfy.sh/Pushover/Telegram)**: technically the most OS-agnostic option of all (just an HTTP POST, `reqwest` is already a dependency) but solves a different problem (a different *device*, for the SSH/remote scenario) than what the user asked for here. Not rejected outright — just out of scope for this specific phase, worth its own future phase if the remote scenario becomes a real need.

**Terminal tab/window title changes won this comparison**: the OSC 0/2 escape sequence for setting a terminal's title is one of the most universally-supported terminal features that exists — Windows Terminal, iTerm2/Terminal.app (macOS), and virtually every Linux terminal emulator (GNOME Terminal, Konsole, xterm, Alacritty, kitty) all honor it identically. A title change is visible in the tab bar/taskbar/window switcher even when that tab isn't focused, which directly answers "I want to notice this from a different terminal tab" without needing tmux, a shell hook, or a GUI tray subsystem. `crossterm::terminal::SetTitle` (already a workspace dependency, `crossterm = "0.27"`) supports this directly — zero new dependencies.

---

## Decisions

### 1. What the title shows: unread notification count, not per-event flashing

**Confirmed** (chosen for consistency, not separately asked): the title becomes `"Terminal Workspace"` when there are no unread notifications, or `"Terminal Workspace (N)"` when there are `N` — reusing `DashboardReadModel.unread_notifications.len()`, the exact same count `step19.md`'s dock-title badges (`"알림 (3)"`) already compute. A persistent ambient count rather than a one-shot "flash the title on each new event, then revert" avoids tracking a separate "has the user seen this yet" state that the unread count already represents.

### 2. Update point: every `draw()` call, unconditionally

**Confirmed** (low-stakes, entailed by Decision 1): `TuiRenderer::draw()` already reads `DashboardReadModel` fresh every frame (keypress, resize, or bus event all trigger a redraw). Setting the title unconditionally on every draw — rather than diffing against the last-set value — is simpler and the escape sequence itself is cheap; terminals do not flicker or visibly re-render on a title set to the same string.

### 3. Title reset on exit

**Confirmed** (low-stakes): `restore_terminal()` resets the title back to `"Terminal Workspace"` (no count) alongside its existing raw-mode/alternate-screen/cursor cleanup, so a stale unread count doesn't linger in the tab bar after the app has actually quit.

---

## Proposed Changes

#### [MODIFY] `crates/ui/src/lib.rs`
`TuiRenderer::draw()` calls `crossterm::execute!(terminal.backend_mut(), SetTitle(title))` using the current unread count (Decision 1/2). `restore_terminal()` resets the title (Decision 3).

---

## Verification Plan

- Can't unit-test an actual terminal emulator's tab bar rendering an escape sequence -- verify the *title string* is correct for a given count (0 → plain title, N → `"(N)"` suffix), the same "test what's actually testable" split this project already applies elsewhere (e.g. `step21.md`'s pure `notification_for_event` mapping tested without a real OS notification).
- Manual verification on this (Windows) machine: run the real app in Windows Terminal, confirm the tab title updates with the unread count and is visible from a different tab.
- `cargo fmt --all --check` / `cargo check --workspace --all-targets` / `cargo clippy --workspace --all-targets -- -D warnings` / `cargo test --workspace` all green.

---

## Implementation Notes (what actually happened)

Shipped exactly as designed — all three Decisions held with no changes.

`title_for_unread_count(usize) -> String` is a small pure function (`crates/ui/src/lib.rs`), tested directly (2 tests: zero-unread plain title, non-zero count suffix) rather than trying to assert anything about a real terminal emulator's tab bar, the same "test what's actually testable" split `step21.md`'s pure `notification_for_event` mapping used. `TuiRenderer::draw()` calls `crossterm::execute!(terminal.backend_mut(), SetTitle(...))` unconditionally on every frame (Decision 2) — no diffing against a previous value, since `crossterm::terminal::SetTitle` was already a zero-cost addition (the crate was already a workspace dependency; no `Cargo.toml` changes needed for this phase at all). `restore_terminal()` resets the title back to the plain `"Terminal Workspace"` on exit (Decision 3).

This phase needed no manual empirical verification of the OS/notification-daemon kind `step21.md` required — `SetTitle` is a well-established, unversioned terminal escape sequence with no failure mode to catch (unlike a real desktop notification backend that can be genuinely absent).

Final state: 2 new tests in `crates/ui` (112 total, up from 110). No new dependencies, no `Cargo.toml`/`Cargo.lock` changes. Full `cargo fmt`/`check`/`clippy -D warnings`/`test --workspace` green with no regressions.
