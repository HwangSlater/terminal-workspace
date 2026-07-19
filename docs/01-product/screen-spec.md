# Screen Specifications

This document defines the layout grids, ASCII mockups, and layout slots for the Terminal UI (TUI) windows.

> **Implementation Status (Phase 5)**: The Docking Slot Layout Rules (§1) and the responsive rules (§3) are implemented in `crates/ui`. Screen 1's Team Status and Notification Center regions render real data; the Main Detail Pane, Calendar region, and CI/CD Status region are placeholder text pending their respective integrations. Screens 2 (Command Palette) and 3 (AI Assistant) are not yet implemented — see `step5.md`.

---

## 1. Docking Slot Layout Rules
- **Left Dock**: Reserved for navigation/directories/presence trees. Width configurable via `config.toml`'s `[layout].left_dock_width` (default: 24 columns), read once at startup — see `docs/05-operations/configuration.md` §1, `step26.md`. Must be 10-60, and `left_dock_width + right_dock_width` must not exceed 60 (`crates/config`'s `AppConfig::validate()`). **Amended `step27.md`**: this is a ceiling on the Team dock's width, not the exact value rendered — a short team roster renders narrower than the configured value (floored at 10), and the Notification dock (already fluid) automatically reclaims the difference.
- **Right Dock**: Reserved for contextual information (Calendar monthly view, PR details). Width configurable via `[layout].right_dock_width` (default: 32 columns), same validation and startup-only scope as the Left Dock.
- **Center Dock**: Fluid width container. Multi-pane layouts (split horizontally or vertically) displaying active content.
- **Bottom Dock**: Fluid width container for logs, shells, and persistent inputs. **Amended `step19.md`**: the real implementation does not render this as a permanently-visible screen row at all — a 1-content-row strip (`step17.md`'s original shape) never showed enough of the log buffer to be useful. `Ctrl+4` instead opens a large on-demand overlay (`registry::UiDockSlot::Bottom` still exists as a type, but is no longer part of the visible dock layout or the `Tab`/`Shift+Tab` focus cycle).

---

## 2. Screen ASCII Mockups

### Screen 1: Workspace Dashboard (Default view)
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
- **Sidebar Auto-Collapse**: If terminal width is $< 120$ characters, the body area shows only one panel at a time instead of Team/Notification/Calendar side by side. Which one follows `focused_dock` — `Tab`/`Shift+Tab` (cycle) or `Ctrl+1`/`Ctrl+2`/`Ctrl+3` (Team/Notification/Calendar directly) switch it, exactly the same shortcuts used to move focus on a wide terminal. Corrected from an earlier draft of this doc that named `Ctrl+1`/`Ctrl+4` as the Team/Calendar toggles specifically; the real binding scheme is `Ctrl+1`=Team, `Ctrl+2`=Notification, `Ctrl+3`=Calendar (`docs/02-architecture/keyboard.md`), and prior to this fix Team/Calendar were simply unreachable below 120 columns regardless of which key was pressed. **Amended `step19.md`**: `Ctrl+4` is no longer part of this family at all — Bottom/Log dropped out of the dock-focus cycle entirely once Log became an on-demand overlay instead of a body panel; `Ctrl+4` opens that overlay directly regardless of terminal width, the same way `Ctrl+S`/`Ctrl+G`/`Ctrl+L` open their setup overlays.
