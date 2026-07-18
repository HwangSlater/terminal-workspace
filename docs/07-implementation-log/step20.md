# Implementation Plan - Phase 20: Header Overflow Fix

This is a **design document for review — nothing described below has been implemented yet**, per the same process used for Phases 6-19.

## Context

Follow-up from `step19.md`'s UI polish pass. During a post-commit review requested explicitly to check whether the current UI design is actually the best it can be (user's own framing: "디자인 완전히 바꿔도 괜찮으니까 지금이 최선인지 확인해" — full license to redesign, verify this is genuinely good), the header (`crates/ui/src/render.rs::render_header`) was rendered at the documented minimum terminal size and inspected directly:

```
Terminal Workspace  |  Slack: 연결 안 됨  |  GitHub: 연결 안 됨  |  Calendar: 연
```

At 80×24, the line is cut off mid-word — `Calendar`'s connection status is truncated after one character, and the header's own trailing help/quit hint is gone entirely. `Paragraph::new(Line::from(spans))` has no `.wrap()`, so ratatui silently drops whatever doesn't fit in the single `Constraint::Length(1)` row, with no `...` or other visible sign that anything is missing. This is a real, confirmed bug (empirically measured, not a taste call): a user on the minimum supported terminal size cannot reliably tell whether Calendar is connected, and loses the header's help/quit hint outright. It gets worse whenever Pomodoro is running (adds ~15 more cells) and will get worse again if Gmail/Jira (already real `IntegrationSource` variants) ever get their own header segment.

## Decision

**Confirmed** (user picked directly): split the header into fixed rows rather than fighting for a single-row fit. Two iterations happened during implementation — see Implementation Notes for why the first (2 rows) wasn't enough and the shipped version is 3 rows.

The help/quit hint (`도움말: ?   종료: Ctrl+Q`) is dropped from the header entirely rather than carried into a new row — the footer already shows the same information (`?:도움말  Ctrl+Q:종료`), so it was pure duplication, not information the header actually needed to preserve.

## Verification Plan

- A regression test rendering at the exact documented minimum (80×24) and asserting all three connection statuses survive intact — the concrete failure this phase fixes.
- Full `cargo fmt`/`check`/`clippy -D warnings`/`test --workspace`.

---

## Implementation Notes (what actually happened)

**First attempt (2 rows) was insufficient — measured, not assumed.** The initial fix moved only the Pomodoro segment to its own second row and dropped the help/quit hint, keeping title + three statuses together on row 1. Re-measuring at 80×24 showed `Calendar: 연` *still* truncated: title + three statuses alone is 89 cells wide, 9 over budget even with Pomodoro and the hint both removed. Fixed by giving the title its own row too (3 rows total: title / three statuses / Pomodoro). Deliberately did not shorten any label or status text to force a single-row fit — cutting the *content* the row exists to display would trade one truncation bug for a quieter one. Three connection statuses alone measure 65 cells, comfortably under 80.

`MIN_HEIGHT` (24) was not changed — `Constraint::Length(3)` for the header plus `Constraint::Min(5)` for the body still leaves the body its full minimum at the smallest supported terminal.

Final state: 1 new regression test (`header_does_not_truncate_any_connection_status_at_minimum_terminal_size`, drawn at the real 80×24 minimum, not a synthetic wide terminal) plus all prior header tests updated implicitly (still pass unchanged, since they scan the whole rendered buffer rather than a specific row). 110 tests in `crates/ui` (up from 109). Full `cargo fmt`/`check`/`clippy -D warnings`/`test --workspace` green with no regressions.
