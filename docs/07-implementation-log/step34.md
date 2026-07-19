# Implementation Plan - Phase 34: Stop a failed command's error from sticking forever

Real bug fix, root-caused via live use — skips the Decisions/AskUserQuestion
cycle (there was only one reasonable fix once the cause was clear),
documented here per `development.md`'s own rule that a change still needs a
record even when it didn't need up-front confirmation.

## Context

Reported directly, as three separate questions in one message:

1. "커맨드는 이게 최선인가?" — is the command-bar design (`:command` bar,
   `docs/07-implementation-log/step9.md`/`step13.md`) the best it can be?
2. Does Tab-autocomplete work for channel names too (e.g. `/send #<Tab>`),
   or only for the `/command` names themselves?
3. A concrete bug: after a command fails and the bar turns red with an
   error, the error never goes away — and autocomplete stops appearing
   entirely afterward, even for a brand new, valid command.

Only #3 is a code change. #1 and #2 are answered directly in this doc and
to the user; see Findings below.

## Findings (Q1 and Q2)

**Q2 — channel-name autocomplete**: already implemented, no change needed.
`compute_suggestions` (`crates/ui/src/keyboard.rs`) has two independent
modes: word 1 starting with `/` matches `COMMAND_HEADS`; word 2 of a
`/send` line starting with `#` matches `picker.channels`
(`state.slack_picker`, the same list `Ctrl+P` populates) case-insensitively.
`refresh_suggestions` runs on every keystroke unconditionally, so the
suggestion data itself was never broken — only *rendering* it was (see Q3).

**Q1 — is the command bar the best design**: no change proposed here. The
concrete complaint in the same message (#3) was actually a rendering bug,
not a sign the underlying `:command` + Tab-complete + inline-error design
is wrong — once the bug in Root Cause below is fixed, the bar behaves the
way it was originally designed to. If a real redesign is wanted later
(e.g. a dedicated suggestions popup instead of the single inline hint
line, `step13.md` Decision 3), that's a separate, larger decision worth
its own `AskUserQuestion` round, not something to bundle into a bug-fix
phase.

## Root cause (Q3)

Two independent facts combined into the reported symptom:

- `render_command_bar` (`crates/ui/src/render.rs`) has an unconditional
  early `return` at the very top whenever `state.cmd_buffer.last_error`
  is `Some(..)` — it renders only the error text and nothing else,
  before ever reaching the autocomplete-hint code further down. This is
  intentional (`step9.md` — an error should survive leaving Input mode,
  not vanish the instant Esc is pressed) and is not being changed here.
- `last_error` was previously cleared in exactly one place: a fully
  successful subsequent `Enter`. Typing a brand new command character by
  character, or Backspace-ing to correct a typo, never touched it.

So after any failed command, the error branch above stayed matched
indefinitely — through any amount of new typing — which looked like both
"the error won't go away" and "autocomplete stopped working," since the
error branch's `return` was the actual reason the hint code below it never
ran again.

## Fix

`capture_command_text` (`crates/ui/src/keyboard.rs`) now clears
`state.cmd_buffer.last_error` as the first statement in both its
`KeyCode::Char` and `KeyCode::Backspace` arms, before doing anything else.
The moment the user starts a new attempt — by typing or by
backspacing — `render_command_bar`'s error branch stops matching on the
very next frame, and the ordinary text+hint rendering (including
autocomplete) resumes immediately. The error still correctly survives
Esc / any other key with no text change, matching the original
`step9.md` intent exactly.

Deliberately not fixed by touching `render_command_bar` itself — clearing
the source field is enough, and keeps the render function's priority rule
(error beats hint, unconditionally, whenever `last_error` is set) exactly
as simple and predictable as `step9.md` designed it.

## Verification

- New tests, `crates/ui/src/keyboard.rs`:
  - `typing_after_a_failed_command_clears_the_stale_error`
  - `backspacing_after_a_failed_command_clears_the_stale_error`
  - `typing_a_new_slash_command_after_a_failed_send_shows_autocomplete_again`
    — reproduces the exact reported symptom end to end: fail `/send #nope
    hi`, type `/se`, assert both that `last_error` is gone and that
    `autocomplete_suggestions` is non-empty again.
- New test, `crates/ui/src/render.rs`:
  `command_bar_prefers_the_error_over_autocomplete_hints_when_both_are_set`
  — constructs a state with both `last_error` and non-empty
  `autocomplete_suggestions` directly (bypassing `keyboard.rs`, which
  would never produce that combination in practice after this fix) to
  pin down that `render_command_bar`'s error-first priority itself is
  unchanged and deliberate, not a leftover.
- `command_bar_shows_a_parse_error_even_after_leaving_input_mode`
  (pre-existing) reviewed, left as-is — it already covers the "error
  survives Esc with no further typing" case this fix doesn't touch.
- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --
  -D warnings`, `cargo test -p ui` (177 passed, up from 173) all green.
