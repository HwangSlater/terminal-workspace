# Terminal User Interface (TUI) Specification

The presentation layer of the Workspace is designed as a dynamic, non-blocking **Terminal User Interface (TUI)** built on `Ratatui` and `crossterm`. It uses a layout system split into functional panes and leverages incremental rendering to ensure smooth operation at 60 FPS under minimal resource footprint.

## Visual Grid Layout

The terminal viewport is partitioned using a rectangular grid system. Below is the layout blueprint:

```text
+-----------------------------------------------------------------------------+
|  [Workspace App]  [Slack: Active]  [GitHub: 3 PRs]        16:07:22 | v1.0.0 | -> Top Header Bar
+---------------------+-------------------------------------------------------+
|                     | [Notifications Queue]                                 |
|  [Team Panel]       | - [Slack DM] From @bob: "Please review PR #42" (15m)  |
|                     | - [GitHub PR] Review requested for wasm-runner (1h)   |
|  - @alice (Active)  | - [Calendar] Standup Meeting in 5 mins                |
|  - @bob (Away)      +-------------------------------------------------------+
|  - @charlie (Lunch) | [Active Detail Pane / Calendar Panel]                 |
|                     |                                                       |
|                     |  July 2026                                            |
|                     |  Su Mo Tu We Th Fr Sa                                 |
|                     |            1  2  3  4                                 |
|                     |   5  6  7  8  9 10 11                                 |
|                     |  12 13 14 15 16[17]18                                 |
+---------------------+-------------------------------------------------------+
|  [CI/CD Status]     | [Logs / Events Terminal Stream]                       |
|  Main: SUCCESS      | [2026-07-17 16:05] Slack DM received from @bob.       |
|  Dev: RUNNING       | [2026-07-17 16:06] SQLite cache synchronized.         |
+---------------------+-------------------------------------------------------+
|  CommandLine: /slack-send @bob I'm on it!                                   | -> Command Input Bar
+-----------------------------------------------------------------------------+
|  F1:Help  F2:Slack  F3:GitHub  F4:Calendar  Esc:Normal Mode  Ctrl+Q:Quit    | -> Status Footer Bar
+-----------------------------------------------------------------------------+
```

---

## Widget Descriptions

1. **Top Header Bar**: Displays app branding, connection health indicator, active integration statuses (e.g., green/yellow/red dots for Slack/GitHub API connections), system time, and current release version.
2. **Team Panel**: Collapsible left sidebar showing team members, their presence statuses (`Active`, `Away`, `Meeting`, `Lunch`, `Offline`), and active branches.
3. **Notification Panel**: Centered top dashboard listing high and medium priority incoming notifications. Each row is interactive (pressing `Enter` opens the corresponding link or detail pane).
4. **Active Detail Pane**: Main interaction container. Dynamically switches between the Calendar Grid, Markdown previewer for GitHub PRs, Slack thread viewer, or system configuration editor.
5. **CI/CD Panel**: Bottom-left widget showing real-time integration status (GitHub Actions, GitLab CI, or Jenkins builds) queried from the Event Bus.
6. **Command Line Input Bar**: Toggled via `:` or `Esc`. Processes user CLI commands, supports autocomplete for integrations (e.g., `/slack-send`, `/github-approve`).
7. **Status Footer**: Shows context-aware hotkeys depending on the currently focused pane.

---

## Reactive UI State Machine

The UI does not execute polling. It reacts exclusively to `TuiState` mutation events dispatched on the main thread loop.

```rust
pub struct TuiState {
    pub active_pane: ActivePane,
    pub focus_mode: FocusMode, // Normal, Input, Dialog
    pub selected_index: usize,
    pub notifications: Vec<NotificationItem>,
    pub team_presence: Vec<TeamMember>,
    pub command_buffer: String,
}

pub enum ActivePane {
    Team,
    Notifications,
    Detail,
    CIStatus,
}

pub enum FocusMode {
    Normal, // Navigating using Arrow/Vim keys
    Input,  // CLI Typing inside Command Line bar
}
```

### Rendering Loop (Double Buffering)
1. **Event Trigger**: An asynchronous service publishes a `TuiStateChanged` event to the Event Bus.
2. **State Sync**: The main UI thread catches the event and updates its local copy of `TuiState`.
3. **Draft Render**: The UI computes the next layout frame in memory (`Ratatui Frame`).
4. **Diff Render**: Ratatui compares the draft frame with the current screen buffer and executes incremental ANSI terminal updates, preventing screen flickering.
5. **Non-blocking input**: User keyboard interrupts are processed asynchronously on a separate `tokio::task` via `crossterm::event::read` and translated to domain actions immediately.
