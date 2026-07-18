# Workspace State Specification

This document details the UI state context governing active viewports, split panels, focal nodes, and custom terminal buffers.

> **Implementation Status (Phase 5, amended Phase 7/8)**: Implemented in `crates/ui`. One deviation from the original sketch: `DockSlot` is not a new enum — it reuses `registry::UiDockSlot` (already `Left`/`Center`/`Right`/`Bottom` per ADR-0012) rather than duplicating the same four variants under a different name. `docking_registry`/`active_panel_focus` key off that shared type. Fields added since the original Phase 5 sketch: `selected_index`/`should_quit` (Phase 5, omitted from the first draft below but present from the start of the real implementation), and `active_overlay`/`slack_setup`/`slack_picker` (Phase 7/8 — the `Ctrl+S` token-entry and `Ctrl+P` channel/user-picker overlays; `step7.md`, `step8.md`). The sketch below is kept current, not historical.

---

## 1. UI Focus & Layout State

The TUI uses a stateful manager to determine which panel captures user keystrokes:

```rust
pub struct WorkspaceState {
    pub active_layout: ActiveLayout,
    pub focus_mode: FocusMode,
    pub focused_dock: DockSlot,
    pub docking_registry: HashMap<DockSlot, Vec<PanelId>>,
    pub active_panel_focus: HashMap<DockSlot, PanelId>,
    pub cmd_buffer: CommandBufferState,
    pub active_overlay: OverlayKind,
    pub slack_setup: SlackSetupState,
    pub slack_picker: SlackPickerState,
    pub active_theme: String,
    pub selected_index: usize,
    pub should_quit: bool,
}

pub enum ActiveLayout {
    DefaultDashboard,
    MaximizedPanel(PanelId),
    AiSplitScreen,
}

pub enum DockSlot {
    Left,
    Center,
    Right,
    Bottom,
    None,
}

/// `docs/02-architecture/keyboard.md`'s three input modes.
pub enum FocusMode {
    Normal,
    Input,
    Overlay,
}
```

---

## 2. Command Buffer State

Stores the state of the interactive command line input bar.

```rust
pub struct CommandBufferState {
    pub raw_text: String,
    pub cursor_position: usize,
    pub autocomplete_suggestions: Vec<String>,
    pub selected_suggestion_index: Option<usize>,
    pub history: Vec<String>,
    pub history_index: Option<usize>,
}
```

---

## 3. Overlay State (Phase 7/8)

Which dialog `FocusMode::Overlay` is currently showing, and each dialog's own state:

```rust
pub enum OverlayKind {
    Help,
    SlackSetup,   // Ctrl+S — step7.md
    SlackPicker,  // Ctrl+P — step8.md
}

pub struct SlackSetupState {
    pub token_input: String,       // rendered masked, never in the clear
    pub status: SlackSetupStatus,
}
pub enum SlackSetupStatus { Idle, Connecting, Connected, Failed(String) }

pub struct SlackPickerState {
    pub channels: Vec<PickerRow>,
    pub users: Vec<PickerRow>,
    pub cursor: usize,             // indexes the combined channels-then-users list
    pub status: SlackPickerStatus,
}
pub struct PickerRow { pub id: String, pub label: String, pub selected: bool }
pub enum SlackPickerStatus { Idle, Loading, Loaded, Saving, Saved, Failed(String) }
```

---

## 4. State Mutation Rules
1. **Focus Shift**: Tab key increments the `focused_dock` Slot clockwise. Arrow keys steer selection within the active focused panel.
2. **Dynamic Docking**: When a plugin registers a panel, it specifies the target `DockSlot`. If a panel already occupies the target slot, it is appended to the slot's array and rendered as a Tab within that slot container.
3. **State Querying (CQRS)**: The TUI renders states based on the active `WorkspaceState` and `DashboardReadModel`. Both `SlackSetupState`/`SlackPickerState` submit through the CQRS write path when confirmed (`Command::ConnectSlack`/`ApplySlackSelection`) — `WorkspaceState` itself is never queried or dispatched *as* a command, only used to render and to capture in-progress input.
