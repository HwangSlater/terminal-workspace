# Keyboard Bindings Specification

The Terminal Workspace utilizes a **Modal Input System** (inspired by Vim) to allow developers to perform rapid navigation, command dispatch, and content viewing without leaving the home row.

## Input Modes

The system operates in one of three modes:
1. **Normal Mode**: Default mode. Keys map to navigation, pane-switching, and action shortcuts.
2. **Input Mode**: Toggled when focused on the Command Line Bar or writing a reply/issue. Every keystroke is treated as text input except for the escape character.
3. **Overlay/Dialog Mode**: Active when a popup dialog (e.g., connection setup, calendar event creation) is visible. Tab and arrow keys cycle through dialog fields.

---

## Global Key Bindings (Normal Mode)

| Key | Action | Scope | Description |
| :--- | :--- | :--- | :--- |
| `Ctrl + q` | Quit Application | Global | Gracefully terminates connections, writes cache to SQLite, and exits. |
| `Esc` | Enter Normal Mode | Global | Cancels active operations, closes popups, unfocuses input bar. |
| `:` | Enter Input Mode | Global | Focuses the Command Line Input Bar for command entry. |
| `Tab` | Focus Next Pane | Global | Cycles focus clockwise through visible layout panes. |
| `Shift + Tab`| Focus Prev Pane | Global | Cycles focus counter-clockwise through visible layout panes. |
| `?` | Show Help Dialog | Global | Renders an overlay listing all context-aware shortcuts. |

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
