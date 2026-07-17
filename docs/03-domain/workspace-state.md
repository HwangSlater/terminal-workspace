# Workspace State Specification

This document details the UI state context governing active viewports, split panels, focal nodes, and custom terminal buffers.

---

## 1. UI Focus & Layout State

The TUI uses a stateful manager to determine which panel captures user keystrokes:

```rust
pub struct WorkspaceState {
    pub active_layout: ActiveLayout,
    pub focused_dock: DockSlot,
    pub docking_registry: HashMap<DockSlot, Vec<PanelId>>,
    pub active_panel_focus: HashMap<DockSlot, PanelId>,
    pub cmd_buffer: CommandBufferState,
    pub active_theme: String,
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

## 3. State Mutation Rules
1. **Focus Shift**: Tab key increments the `focused_dock` Slot clockwise. Arrow keys steer selection within the active focused panel.
2. **Dynamic Docking**: When a plugin registers a panel, it specifies the target `DockSlot`. If a panel already occupies the target slot, it is appended to the slot's array and rendered as a Tab within that slot container.
3. **State Querying (CQRS)**: The TUI renders states based on the active `WorkspaceState` and `DashboardReadModel`.
