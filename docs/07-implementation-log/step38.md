# Implementation Plan - Phase 38: Keybinding cleanup — drop the numeric focus shortcuts

Requested directly, as two follow-up messages to `step37.md`'s deferred
"단축키 재정리" item:

1. "Ctrl+2부터 시작하고 뒤죽박죽인거 같아" — reorganize the `Ctrl+` scheme.
2. "알림/캘린더는 tab키로도 충분하니까 놔두고 로그만 따로 두자" — leave
   Notification/Calendar alone (Tab already covers them), just pull Log
   out on its own.

Skips the Decisions/`AskUserQuestion` cycle `step37.md` deferred this to
— the user's second message already resolved the ambiguity `step37.md`
flagged (renumber vs. re-letter vs. something else) with a concrete,
narrow instruction, so there was one reasonable implementation left, not
several to weigh.

## Root cause of the "뒤죽박죽" feeling

`Ctrl+2`/`Ctrl+3` (direct-jump to Notification/Calendar, plus `n`/`d`
letter aliases) and `Ctrl+4`/`Ctrl+c` (open the Log Viewer overlay) sat
in the same numeric family in the docs and the footer legend, as if all
three were the same kind of shortcut. Only the first two ever were —
`Ctrl+4` was always documented as *not* a "focus a dock" shortcut (`Log
is a Ctrl+4 overlay, not a focusable dock`, per `step19.md`), just never
moved out of the numeric sequence it happened to inherit. Every other
"open an overlay directly" shortcut in this app (`Ctrl+S`, `Ctrl+G`,
`Ctrl+L`) uses a mnemonic letter, not a number — Log was the one
exception, and that inconsistency is what read as messy.

## What changed

- `Ctrl+2`/`Ctrl+n` (focus Notification) and `Ctrl+3`/`Ctrl+d` (focus
  Calendar) are **removed outright**, not renumbered. Per the user's own
  reasoning: with exactly two real body docks left since `step32.md`
  moved Team into the header, `Tab`/`Shift+Tab` alone already reaches
  either one in at most one keystroke (cycling between exactly two things
  is never worse than a direct jump) — a dedicated shortcut per dock
  stopped earning its keep once cycling and jumping became the same cost.
- `Ctrl+4` (the numeric alias for the Log Viewer overlay) is also
  dropped. `Ctrl+c` — already a documented, working alias since
  `step19.md`, just never promoted to primary — is now Log's only
  binding. Nothing new was invented: this reuses an alias that already
  existed rather than picking a fresh letter, and reads as "log
  *C*onsole," consistent with every other overlay shortcut's mnemonic.
- `Ctrl+c` opening the Log Viewer (rather than sending SIGINT) works
  correctly because this app runs the terminal in raw mode the whole
  time it's in the foreground — `crossterm` captures `Ctrl+c` as an
  ordinary keystroke, not a signal. Not new behavior (the alias already
  worked this way since `step19.md`), but promoting it to the *primary*
  binding means users will actually reach for it now, so `README.md`
  gained a one-line note explaining why it doesn't quit (quitting is
  always `Ctrl+Q`).

## Implementation

- `crates/ui/src/keyboard.rs`: removed the `('2' | 'n', CONTROL)` and
  `('3' | 'd', CONTROL)` match arms from `try_global_shortcut` entirely.
  `(KeyCode::Char('4' | 'c'), ...)` narrowed to `(KeyCode::Char('c'), ...)`
  — Ctrl+4 now falls through to `dispatch_to_pane`, which doesn't
  recognize `'4'` either and returns `KeyOutcome::Ignored`, the same
  "no-op, not a crash" outcome `Ctrl+1` already got in `step32.md`.
- `crates/ui/src/lib.rs`, `crates/ui/src/state.rs`,
  `crates/integration/src/calendar.rs`: doc-comment references to
  `Ctrl+4` updated to `Ctrl+c`.
- `crates/ui/src/render.rs`: `HELP_CATEGORIES`'s "탐색" category lost its
  "Ctrl+2~3: 패널로 바로 이동" row entirely (nothing replaced it — `Tab /
  Shift+Tab`'s existing row already covers the same ground, its
  description extended to say so); the "Ctrl+4: 로그 보기" row moved out
  of "탐색" into "기타" (Esc/Ctrl+Q's category — Log was never a
  navigation action, it belongs with the other general/utility
  shortcuts) and its key updated to `Ctrl+C`. The status footer's legend
  string dropped the `Ctrl+2~3:포커스 이동` segment and updated
  `Ctrl+4` to `Ctrl+C`.
- Every doc referencing the old scheme as current/live (not historical
  implementation-log entries, which stay as accurate-for-their-time
  records) updated: `README.md`, `docs/02-architecture/keyboard.md`
  (§2 renamed from "Quick Focus Switchers" to "Log Viewer Shortcut," now
  documenting exactly the one shortcut it always should have),
  `docs/01-product/screen-spec.md`, `docs/02-architecture/ui.md`,
  `docs/03-domain/workspace-state.md`, `docs/05-operations/logging.md`,
  `docs/06-development/decisions/0012-docking-system.md`,
  `docs/04-extensions/integrations/calendar.md`, `CHANGELOG.md` (also
  fixed two unrelated stale claims noticed in the same pass: the "no C
  compiler" overclaim `step37.md` already corrected in `README.md` but
  not here, and a "Team/Notification/Calendar panels" description that
  predates `step32.md` moving Team into the header).

## Verification

- `cargo fmt --all -- --check` / `cargo clippy --workspace --all-targets --
  -D warnings` / `cargo test --workspace` all green.
- New/renamed tests, `crates/ui/src/keyboard.rs`:
  `ctrl_c_opens_the_log_viewer_overlay_directly_without_touching_focused_dock`
  (renamed from the `ctrl_4_...` version), `ctrl_4_no_longer_opens_the_log_viewer`,
  `ctrl_2_and_ctrl_3_no_longer_focus_a_dock_directly` (covers all four
  removed bindings — `2`, `3`, `n`, `d` — in one test, asserting
  `focused_dock` never moves for any of them).
- `crates/ui/src/render.rs`'s `overlay_mode_renders_help_popup` test used
  "패널로 바로 이동" (the now-deleted Ctrl+2~3 row's description) as its
  "this text only exists inside the help overlay" marker string — updated
  to "읽음 처리" (Enter's `step36.md` description), which still satisfies
  the same "only appears in the overlay body, not the header/footer"
  property the original comment documented.
- Manually ran the app: confirmed `Tab`/`Shift+Tab` still moves between
  Notification and Calendar exactly as before; confirmed `Ctrl+2`/`Ctrl+3`/
  `Ctrl+4` are now silently ignored; confirmed `Ctrl+c` opens the Log
  Viewer and does not terminate the process.
