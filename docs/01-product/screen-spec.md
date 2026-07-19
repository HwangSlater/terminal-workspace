# Screen Specifications

This document defines the layout grids, ASCII mockups, and layout slots for the Terminal UI (TUI) windows.

> **Implementation Status (Phase 5, amended Phase 32/38)**: The Docking Slot Layout Rules (§1) and the responsive rules (§3) are implemented in `crates/ui`. Notification Center and Calendar regions render real data; the Main Detail Pane and CI/CD Status region remain placeholder concepts, not built. Screens 2 (Command Palette) and 3 (AI Assistant) are not yet implemented — see `step5.md`. **`step32.md`**: the Left Dock (Team) described below is no longer part of the body layout at all — a roster this short (realistically a handful of people) never needed a tall scrollable panel, so it's a single always-visible header line instead (§2's Screen 1 mockup and this section both predate that change; kept below as the original three-dock design for history, with amendments noting what actually shipped).

---

## 1. Docking Slot Layout Rules
- **Left Dock (Team)**: **Amended `step32.md`**: no longer a body dock at all — Team moved into the header as a single line (`"팀  ● Alice  ● Bob"`, colored dot per presence status), truncated with an ellipsis rather than wrapped if the roster doesn't fit on one line. `config.toml`'s `[layout].left_dock_width` was removed accordingly; the width/ceiling mechanics described below now apply to the *Right* Dock only. Originally: reserved for navigation/directories/presence trees, `Constraint::Length`-sized (`step26.md`/`step27.md`).
- **Right Dock**: Reserved for contextual information (Calendar monthly view, PR details). Width configurable via `config.toml`'s `[layout].right_dock_width` (default: 60 columns, raised from 32 in `step32.md` once Calendar stopped sharing the row with Team), read once at startup — see `docs/05-operations/configuration.md` §1, `step26.md`. Must be 10-60 (`crates/config`'s `AppConfig::validate()`). This is a ceiling on the dock's width, not the exact value rendered (`step27.md`'s pattern, retargeted from Team to Calendar in `step32.md`): a light day's events render narrower than the configured value (floored at 20), and the Notification dock (already fluid) automatically reclaims the difference. A title too long even at the ceiling is truncated with an ellipsis, never wrapped (`step32.md`, explicit requirement).
- **Center Dock**: Fluid width container. Multi-pane layouts (split horizontally or vertically) displaying active content.
- **Bottom Dock**: Fluid width container for logs, shells, and persistent inputs. **Amended `step19.md`**: the real implementation does not render this as a permanently-visible screen row at all — a 1-content-row strip (`step17.md`'s original shape) never showed enough of the log buffer to be useful. `Ctrl+c` (**amended `step38.md`**: was `Ctrl+4`/`Ctrl+c`, the numeric alias dropped) instead opens a large on-demand overlay (`registry::UiDockSlot::Bottom` still exists as a type, but is no longer part of the visible dock layout or the `Tab`/`Shift+Tab` focus cycle).

---

## 2. Screen ASCII Mockups

### Screen 1: Workspace Dashboard — as currently implemented (`step32.md`)
```text
+--------------------------------------------------------------------------------+
| Terminal Workspace                                                             |
| Slack: 연결됨  |  GitHub: 연결 안 됨  |  Calendar: 연결됨                       |
| 팀  ● alice  ● bob  ● charlie                                                  |
+---------------------------------------------------+----------------------------+
| 알림 (2)                                           | 캘린더 (2)                 |
| [Slack] Approved PR                                | 7/20 15:00  [회사] Standup |
| [GitHub] Review requested in repo/wasm             | 7/20 19:30  [개인] 병원    |
|                                                     |                            |
+---------------------------------------------------+----------------------------+
| :                                                                               |
+--------------------------------------------------------------------------------+
```
No Main Detail Pane, CI/CD Status region, or bottom chat stream exist in the real
implementation — those remain unbuilt concepts from the original Phase 5 sketch
below, kept for history.

### Screen 1 (original Phase 5 sketch, superseded by `step32.md` above)
```text
+---------------------+---------------------------------------+------------------+
| Workspace [Main]    | Slack Connection: ACTIVE              | 16:07:22 | v1.0  |
+---------------------+---------------------------------------+------------------+
| TEAM STATUS (Left)  | NOTIFICATION CENTER (Center Top)                         |
| • @alice   [Active] | [1] Slack DM from @bob: "Approved PR" (2m ago)           |
| o @bob     [Away]   | [2] GitHub: Review requested in repo/wasm (1h ago)       |
| • @charlie [Meeting]|                                                          |
|                     +---------------------------------------+------------------+
|                     | MAIN DETAIL PANE (Center Bottom)      | CALENDAR (Right) |
|                     | Welcome to your Terminal Workspace.   | Su Mo Tu We Th   |
|                     | Active issue: #104 FFI binding        |        1  2  3   |
|                     | Status: IN PROGRESS                   |  4  5  6  7  8   |
|                     |                                       |  9 10 11 12 13   |
+---------------------+---------------------------------------+------------------+
| CI/CD Status        | SYSTEM CHAT STREAM (Bottom)                              |
| main: [SUCCESS]     | [16:05] Synced SQLite databases.                         |
+---------------------+----------------------------------------------------------+
| CommandLine: :                                                                 |
+--------------------------------------------------------------------------------+
```

### Screen 2: Command Palette Modal Overlay
Activated by pressing `:` in normal mode.
```text
                      +------------------------------------------+
                      | COMMAND PALETTE                          |
                      | > /slack-send _                          |
                      | ---------------------------------------- |
                      | /slack-send  [user] [message]            |
                      | /slack-away                              |
                      | /github-pr   [repo]                      |
                      | /github-approve [pr_id]                  |
                      +------------------------------------------+
```

### Screen 3: AI Assistant Interface Panel
Docked on the Center Pane or popped as a floating modal window.
```text
+--------------------------------------------------------------------------------+
| AI ASSISTANT CHAT                                                              |
+--------------------------------------------------------------------------------+
| User: How do I approve PR #42 via CLI?                                         |
|                                                                                |
| Assistant: You can use the built-in Command Palette:                           |
| 1. Press `:` to focus the Command Line.                                        |
| 2. Input `/github-approve google/terminal-workspace 42 "LGTM!"`.               |
|                                                                                |
| This translates to the `ApprovePR` Command and broadcasts to the Event Bus.    |
|                                                                                |
| > _                                                                            |
+--------------------------------------------------------------------------------+
```

---

## 3. Terminal Constraints & Responsive Rules
- **Minimum Grid Size**: $80 \times 24$ character matrix. If terminal drops below these bounds, the UI renders a full-screen placeholder: `Terminal size too small. Please enlarge to continue.`
- **Sidebar Auto-Collapse**: If terminal width is $< 120$ characters, the body area shows only one panel at a time instead of Notification/Calendar side by side. Which one follows `focused_dock` — `Tab`/`Shift+Tab` switch it, exactly the same shortcut used to move focus on a wide terminal. Corrected from an earlier draft of this doc that named `Ctrl+1`/`Ctrl+4` as the Team/Calendar toggles specifically (`docs/02-architecture/keyboard.md`), and prior to that fix Calendar was simply unreachable below 120 columns regardless of which key was pressed. **Amended `step19.md`**: `Ctrl+4` is no longer part of this family at all — Bottom/Log dropped out of the dock-focus cycle entirely once Log became an on-demand overlay instead of a body panel; it opens that overlay directly regardless of terminal width, the same way `Ctrl+S`/`Ctrl+G`/`Ctrl+L` open their setup overlays. **Amended `step32.md`**: Team (`Ctrl+1`) dropped out of this family too, for a different reason than Bottom — it isn't a body panel at all anymore (a header line instead), so there's nothing left for a narrow terminal to collapse to or expand from. **Amended `step38.md`**: `Ctrl+2`/`Ctrl+3` (direct-jump to Notification/Calendar) were removed outright, requested directly — `Tab`/`Shift+Tab` alone already reaches either of the two remaining body docks in at most one keystroke, collapsed or not, so the direct-jump shortcuts were redundant rather than load-bearing.
