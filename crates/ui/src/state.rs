//! `docs/03-domain/workspace-state.md`.

use registry::UiDockSlot;
use std::collections::HashMap;

/// Panel identifier — matches `registry::UiRegistry::register_panel`'s
/// `panel_id: &str` convention; owned here since `WorkspaceState` stores it
/// rather than just looking it up.
pub type PanelId = String;

/// Keyboard input mode (`docs/02-architecture/keyboard.md` §"Input Modes").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FocusMode {
    /// Default mode: navigation, pane-switching, action shortcuts.
    #[default]
    Normal,
    /// Focused on the Command Line Bar; keystrokes are text input except Esc.
    Input,
    /// A popup dialog is visible; Tab/arrows cycle dialog fields.
    Overlay,
}

/// `docs/03-domain/workspace-state.md`'s `ActiveLayout`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ActiveLayout {
    /// Default dashboard: all docks visible.
    #[default]
    DefaultDashboard,
    /// One panel fills the whole viewport.
    MaximizedPanel(PanelId),
    /// AI Assistant split-screen. Not yet rendered (Phase 5 scope note in
    /// `docs/02-architecture/ui.md`) — the variant exists so `WorkspaceState`
    /// matches the frozen spec; nothing currently sets it.
    AiSplitScreen,
}

/// `docs/03-domain/workspace-state.md`'s `CommandBufferState`.
#[derive(Debug, Clone, Default)]
pub struct CommandBufferState {
    /// Current text in the command line bar.
    pub raw_text: String,
    /// Byte offset of the cursor within `raw_text`.
    pub cursor_position: usize,
    /// Autocomplete candidates. Always empty in Phase 5 — see `step5.md`'s
    /// scope note (today's `Command` enum is too small to make this useful).
    pub autocomplete_suggestions: Vec<String>,
    /// Currently highlighted autocomplete candidate, if any.
    pub selected_suggestion_index: Option<usize>,
    /// Previously submitted command lines, most recent last.
    pub history: Vec<String>,
    /// Position while scrolling through `history`; `None` means "not browsing history".
    pub history_index: Option<usize>,
}

/// `docs/03-domain/workspace-state.md`'s `WorkspaceState`. `DockSlot` is not
/// a new enum here — it reuses `registry::UiDockSlot` (ADR-0012 amendment).
pub struct WorkspaceState {
    /// Which top-level layout is active.
    pub active_layout: ActiveLayout,
    /// Current keyboard input mode.
    pub focus_mode: FocusMode,
    /// Which dock currently captures navigation keys.
    pub focused_dock: UiDockSlot,
    /// Panels registered per dock slot.
    pub docking_registry: HashMap<UiDockSlot, Vec<PanelId>>,
    /// Which panel within each slot currently has focus (for tabbed docks).
    pub active_panel_focus: HashMap<UiDockSlot, PanelId>,
    /// Command line bar state.
    pub cmd_buffer: CommandBufferState,
    /// Active theme name (`docs/02-architecture/theme.md` lists valid values).
    pub active_theme: String,
    /// Selected index within the focused pane's list (Team/Notification).
    pub selected_index: usize,
    /// Set by the global quit shortcut (`Ctrl+Q`); the run loop exits once true.
    pub should_quit: bool,
}

impl Default for WorkspaceState {
    fn default() -> Self {
        Self {
            active_layout: ActiveLayout::default(),
            focus_mode: FocusMode::default(),
            focused_dock: UiDockSlot::Left,
            docking_registry: HashMap::new(),
            active_panel_focus: HashMap::new(),
            cmd_buffer: CommandBufferState::default(),
            active_theme: "default-dark".to_string(),
            selected_index: 0,
            should_quit: false,
        }
    }
}
