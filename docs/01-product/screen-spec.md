# Screen Specifications

This document defines the layout grids, ASCII mockups, and layout slots for the Terminal UI (TUI) windows.

---

## 1. Docking Slot Layout Rules
- **Left Dock**: Reserved for navigation/directories/presence trees. Fixed width (default: 24 columns).
- **Right Dock**: Reserved for contextual information (Calendar monthly view, PR details). Fixed width (default: 32 columns).
- **Center Dock**: Fluid width container. Multi-pane layouts (split horizontally or vertically) displaying active content.
- **Bottom Dock**: Fluid width container for logs, shells, and persistent inputs.

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
- **Sidebar Auto-Collapse**: If terminal width is $< 120$ characters, the **Left Dock (Team)** and **Right Dock (Calendar)** collapse into hidden tabs toggled via `Ctrl+1` and `Ctrl+4` respectively.
