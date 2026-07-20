# Keyboard Bindings Specification

The Terminal Workspace utilizes a **Modal Input System** (inspired by Vim) to allow developers to perform rapid navigation, command dispatch, and content viewing without leaving the home row.

> **Implementation Status (Phase 5, amended Phase 7/8/13/19/32/36/37/38)**: The three input modes, the global key bindings, and the capture pipeline below are implemented in `crates/ui` exactly as specified — global shortcuts take precedence over pane-specific and plugin shortcuts per the rule at the bottom of this document. Pane-specific navigation is implemented for the Notification and Calendar panels; Detail Pane/CI panel navigation will follow when those panels do. `Ctrl+S` (Phase 7, `step7.md`) is the first real Overlay/Dialog Mode dialog with actual input fields — the Slack credential setup screen; line 12's "connection setup" example was written before any adapter existed to connect, and this is what filled it in. `Ctrl+P` (Phase 8, `step8.md`) is the second — a checkbox-list picker for `channel_ids`/`watched_user_ids`, using `j`/`k`/`Space`/`Enter` rather than Tab/arrows (a flat list has no separate "fields" to tab between). **Phase 13** (`step13.md`) gave `Tab` a second meaning specific to Input Mode: classic shell-style completion for the command head (`/a` → `/away`/`/active`, cycling on repeated presses) and, for `/send`'s first argument, the channel name — `Tab`'s Normal Mode meaning (focus-cycle) is unaffected, since the two modes never overlap. **Phase 19** (`step19.md`) repointed `Ctrl+4`/`Ctrl+c`: the row below documented it as "Focus CI/CD Build Status Panel," which was never actually true (the real implementation always mapped it to the Bottom/Log dock, and no CI/CD panel exists) — it now opens the Log Viewer overlay directly, like `Ctrl+S`/`Ctrl+G`/`Ctrl+L` open their own overlays. **Phase 32** (`step32.md`) moved Team out of the body-panel/focus-cycle set entirely — a roster that short (the product's own working assumption: realistically a handful of people) never needed a tall scrollable panel, so it's a single always-visible header line instead. `Tab`/`Shift+Tab`'s focus cycle now only visits the two real body panels (Notification/Calendar); `Ctrl+1` (previously "focus Team") was removed rather than renumbered. **Phase 36** (`step36.md`, a full shortcut/command review requested directly) gave `Enter` its first real behavior — see §1 below — and restructured the `?` help overlay to group all shortcuts under one "단축키" section and all command-bar syntax under a separate "커맨드" section, instead of interleaving a command category among the shortcut ones. **Phase 37** (`step37.md`) added `/`-to-search inside the Slack and GitHub pickers (the `Ctrl+P`/`Ctrl+R` row below) and changed the `?` help overlay from one tall vertical list to two side-by-side columns (단축키 | 커맨드) so the split `step36.md` introduced reads as two panels, not just two headers in one long scroll. **Phase 38** (`step38.md`, requested directly as a follow-up) removed `Ctrl+2`/`Ctrl+n` and `Ctrl+3`/`Ctrl+d` (direct-jump to Notification/Calendar) entirely — `Tab`/`Shift+Tab` alone already reaches either of the two remaining body docks in at most one keystroke, so the numeric shortcuts were redundant, not load-bearing — and dropped `Ctrl+4`, the numeric half of the Log Viewer's alias pair, leaving `Ctrl+c` as its only binding. See §2 below. **Phase 43** (`step43.md`, requested directly) extended `step37.md`'s `/`-to-search to the Calendar picker (`Ctrl+K`) too — it had been deliberately left out under the assumption that a connected-calendar list stays short, but the user asked for it directly once they had enough calendars connected that scrolling got old. **Phase 45** (`step45.md`) extended `step13.md`'s `Tab` autocomplete past the command head and `/send`'s channel argument to cover every `step41.md` picker command's argument(s) too — `/slack-watch`, `/repo-watch`, `/calendar-rename`, `/calendar-remove` — matched the same way, case-insensitively by prefix against whatever the corresponding picker last fetched.

## Input Modes

The system operates in one of three modes:
1. **Normal Mode**: Default mode. Keys map to navigation, pane-switching, and action shortcuts.
2. **Input Mode**: Toggled when focused on the Command Line Bar or writing a reply/issue. Every keystroke is treated as text input except for the escape character — with one further exception since Phase 13: `Tab` triggers/cycles command-and-channel-name autocomplete (`step13.md`) rather than inserting a literal tab character.
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
| `Ctrl + p` | Slack Channel/User Picker | Global | Opens the channel/watched-user picker (`step8.md`) — arrow keys move (`step29.md`: `j`/`k` no longer advertised, though still accepted), `Space` toggles, `Enter` saves and restarts polling. `/` (`step37.md`) starts typing a live, case-insensitive label search that narrows the visible rows; `Enter` while typing a search stops typing and returns to browsing (a second, separate meaning from `Enter`'s "save" role above — the two never overlap, since a picker is either being searched or being browsed at any instant, never both). The GitHub repository picker (`Ctrl+R`) works identically, one flat list instead of two sections; so does the Calendar picker (`Ctrl+K`), as of `step43.md`. |

---

## Navigation & Pane-Specific Key Bindings

When a specific panel is focused in **Normal Mode**, keys change behavior:

### 1. General Panel Navigation
- `Up Arrow`: Move selection up.
- `Down Arrow`: Move selection down.
- `Enter`: Activate item -- mark the highlighted notification/reminder read (`step36.md`; see below). To mark every unread notification read in one shot instead of one row at a time, use the `/read-all` command (`step44.md`) from the command bar.
- **`step29.md`**: `j`/`k` (and, in overlays that historically accepted `h`/`l` too) are no longer documented as the primary navigation — arrow keys are the one advertised method everywhere, consistent with the Calendar grid view's arrow-only navigation (`step26.md`/`step27.md`). Existing `j`/`k` key bindings in list pickers are left functionally in place (removing them wasn't requested, only the help text advertising them was), so muscle memory built on the old hint still works.
- **`step32.md`**: the Team roster has no panel-navigation section anymore — it's a static header line, nothing in it is selectable, and up/down movement within it was never wired to any action to begin with (confirmed before removing the panel: selecting a team member changed only its highlight, never triggered anything).
- **`step36.md`**: `Enter` was documented here since Phase 5 as "Activate item (e.g., open thread, edit event)" but the actual implementation was a literal no-op the whole time (`PaneAction::Activate`'s match arm did nothing) — `Command::MarkNotificationRead` existed fully wired end-to-end on the write-side (repository + a passing test) since Phase 3, just never dispatched from anywhere in the UI. `Enter` on the Notification or Calendar dock now dispatches it for the highlighted row, which also removes that row from the live read model directly (not through an `Event` -- no frozen variant fits "notification read," see `step36.md`'s Context for why a new one wasn't added either) so it disappears from the panel immediately rather than only after the next poll cycle happens to touch it.

### 2. Log Viewer Shortcut (Global shortcut)
- `Ctrl + c`: Open the Log Viewer overlay directly (`step19.md`) — not a "focus a dock" shortcut, the same "open an overlay directly" category as `Ctrl+S`/`Ctrl+G`/`Ctrl+L`; there is no CI/CD Build Status Panel.
- **`step32.md`**: `Ctrl + 1`/`Ctrl + t` ("Focus Team Panel") was removed, not renumbered — Team is no longer a focus-navigable body dock.
- **`step38.md`**, requested directly ("단축키가 뒤죽박죽"): this section used to be "Quick Focus Switchers" — `Ctrl+2`/`Ctrl+n` (focus Notification) and `Ctrl+3`/`Ctrl+d` (focus Calendar) sat right next to `Ctrl+4`/`Ctrl+c` (open Log) as if all three were the same kind of shortcut, when only the first two ever were. With exactly two real body docks left after `step32.md`, `Tab`/`Shift+Tab` alone already reaches either one in at most one keystroke, so the dedicated numeric jump shortcuts (and their `n`/`d` letter aliases) were removed outright rather than kept as a second way to do what Tab already does — leaving this section with exactly the one shortcut that was never actually a "focus a dock" action to begin with. `Ctrl+4`, the numeric half of the Log Viewer's own alias pair, was dropped the same way; `Ctrl+c` (`c` for log *C*onsole) is now Log's only binding.

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
- If a plugin registers a command or custom shortcut, it **cannot** override Global Hotkeys (`Ctrl+Q`, `Esc`, `Tab`, `Ctrl+C`).
- All key-capture operations are non-blocking to prevent UI thread lag.
