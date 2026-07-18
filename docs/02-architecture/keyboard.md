# Keyboard Bindings Specification

The Terminal Workspace utilizes a **Modal Input System** (inspired by Vim) to allow developers to perform rapid navigation, command dispatch, and content viewing without leaving the home row.

> **Implementation Status (Phase 5, amended Phase 7/8)**: The three input modes, the global key bindings, and the capture pipeline below are implemented in `crates/ui` exactly as specified — global shortcuts take precedence over pane-specific and plugin shortcuts per the rule at the bottom of this document. Pane-specific navigation is implemented for the Team Panel and Notification Panel (the two panels that exist so far); Detail Pane/CI panel navigation will follow when those panels do. `Ctrl+S` (Phase 7, `step7.md`) is the first real Overlay/Dialog Mode dialog with actual input fields — the Slack credential setup screen; line 12's "connection setup" example was written before any adapter existed to connect, and this is what filled it in. `Ctrl+P` (Phase 8, `step8.md`) is the second — a checkbox-list picker for `channel_ids`/`watched_user_ids`, using `j`/`k`/`Space`/`Enter` rather than Tab/arrows (a flat list has no separate "fields" to tab between).

## Input Modes

The system operates in one of three modes:
1. **Normal Mode**: Default mode. Keys map to navigation, pane-switching, and action shortcuts.
2. **Input Mode**: Toggled when focused on the Command Line Bar or writing a reply/issue. Every keystroke is treated as text input except for the escape character.
3. **Overlay/Dialog Mode**: Active when a popup dialog (e.g., connection setup, calendar event creation) is visible. Tab and arrow keys cycle through dialog fields.

---

## Global Key Bindings (Normal Mode)

| Key | Action | Scope | Description |
| :--- | :--- | :--- | :--- |
| `Ctrl + q` | Quit Application | Global | Gracefully terminates connections, writes cache to `redb` (ADR-0014), and exits. |
| `Esc` | Enter Normal Mode | Global | Cancels active operations, closes popups, unfocuses input bar. |
| `:` | Enter Input Mode | Global | Focuses the Command Line Input Bar for command entry. |
| `Tab` | Focus Next Pane | Global | Cycles focus clockwise through visible layout panes. |
| `Shift + Tab`| Focus Prev Pane | Global | Cycles focus counter-clockwise through visible layout panes. |
| `?` | Show Help Dialog | Global | Renders an overlay listing all context-aware shortcuts. |
| `Ctrl + s` | Slack Setup | Global | Opens the Slack Bot Token entry overlay (`step7.md`) — masked input, connects immediately on submit. |
| `Ctrl + p` | Slack Channel/User Picker | Global | Opens the channel/watched-user picker (`step8.md`) — `j`/`k` move, `Space` toggles, `Enter` saves and restarts polling. |

---

## Navigation & Pane-Specific Key Bindings

When a specific panel is focused in **Normal Mode**, keys change behavior:

### 1. General Panel Navigation (Vim Keys Supported)
- `k` or `Up Arrow`: Move selection up.
- `j` or `Down Arrow`: Move selection down.
- `h` or `Left Arrow`: Collapse node / scroll left.
- `l` or `Right Arrow`: Expand node / scroll right / view details.
- `Enter`: Activate item (e.g., open thread, edit event).

### 2. Quick Focus Switchers (Global shortcuts)
- `Ctrl + 1` or `Ctrl + t`: Focus Team Panel.
- `Ctrl + 2` or `Ctrl + n`: Focus Notification Queue.
- `Ctrl + 3` or `Ctrl + d`: Focus Detail/Calendar Panel.
- `Ctrl + 4` or `Ctrl + c`: Focus CI/CD Build Status Panel.

---

## Conflict Resolution & Key Capture Pipeline

Because TUI terminals interpret keystrokes differently based on emulator capabilities, keyboard handling follows this strict capture pipeline:

```text
  Keyboard Interrupt (Crossterm)
                |
                v
       [Is Key ESC?] --Yes--> Exit Input Mode / Close Dialogs -> Handled
                |
                No
                v
       [Active Focus Mode?]
         /            \
    Input Mode     Normal Mode
       /                \
  Capture text      [Is Global Shortcut?] --Yes--> Execute Action -> Handled
  (except ESC)           |
                         No
                         v
                    [Dispatch to Focused Pane] -> Execute Pane Action
```
- If a plugin registers a command or custom shortcut, it **cannot** override Global Hotkeys (`Ctrl+Q`, `Esc`, `Tab`, `Ctrl+1..4`).
- All key-capture operations are non-blocking to prevent UI thread lag.
