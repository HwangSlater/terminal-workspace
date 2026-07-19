# Terminal User Interface (TUI) Specification

The presentation layer of the Workspace is designed as a dynamic, non-blocking **Terminal User Interface (TUI)** built on `Ratatui` and `crossterm`. It uses a layout system split into functional panes and leverages incremental rendering to ensure smooth operation at 60 FPS under minimal resource footprint.

> **Implementation Status (Phase 5, amended Phase 7/32/38)**: `crates/ui` implements the docking shell, Header Bar, Command Line Input Bar, Status Footer, Team Panel, and Notification Panel described below, rendering real data from `PresenceRepository`/`NotificationRepository` via `DashboardReadModel` (`crates/commands`, closing ADR-0007's deferred read path) — no longer always-empty as of Phase 6/7's Slack adapter. The Active Detail Pane's Calendar view, the CI/CD Panel, and the AI Assistant Panel are **not yet implemented** — each needs an integration or domain crate (`assistant` is still a stub); see `step5.md` for the scoping rationale. The layout is fully responsive: every draw recomputes constraints from the terminal's *current* size (sidebar collapse under 120 columns, too-small placeholder under 80x24), and the input reader thread forwards `crossterm`'s `Resize` event (not just key presses) so an actual terminal-window resize triggers a redraw at the new dimensions on its own, without requiring a keypress first. Below the collapse width, the single visible body panel follows `focused_dock` (`Tab` only as of `step38.md`; was `Tab`/`Ctrl+2~3` since `step32.md`, and `Ctrl+1~3` including Team before that) instead of being hardcoded to Notification — a real usability gap (Team/Calendar were simply unreachable on a narrow terminal) fixed after `step12.md`, once Calendar's own panel existing made the gap concrete rather than hypothetical. See `docs/01-product/screen-spec.md` §3. **Phase 7** (`step7.md`) added the first real Overlay dialog with input fields (the `Ctrl+S` Slack setup screen) and gave `TuiRenderer` a `CommandDispatcher` reference — before that, the TUI was pure CQRS *read* side with no way to dispatch anything the user typed. **Phase 8** (`step8.md`, `Ctrl+P`) added the channel/watched-user picker and, with it, `TuiRenderer`'s first *direct* read port (`SlackPicker`, held separately from `CommandDispatcher`) — listing channels/users is a query, not a mutation, so it deliberately doesn't go through `Command`/`CommandHandler` the way `Command::Connect`/`ApplySlackSelection` do. **Phase 11** (`step11.md`) generalized `Command::ConnectSlack`/`ConnectGitHub` into a single `Command::Connect{source, token}` and `Command::ApplyGitHubSelection` into `Command::ApplySelection{source, items}` once GitHub proved the shape was identical across integrations — `ApplySlackSelection` stayed its own variant since Slack's two independent lists (channels + users) don't fit the generic single-list shape.

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
