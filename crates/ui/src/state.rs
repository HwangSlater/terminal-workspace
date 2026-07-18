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

/// Which dialog is showing while `focus_mode == FocusMode::Overlay`
/// (`step7.md`, `step8.md`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum OverlayKind {
    /// Static keybinding reference.
    #[default]
    Help,
    /// In-app Slack Bot Token entry (`Ctrl+S`).
    SlackSetup,
    /// Channel/watched-user picker (`Ctrl+P`, `step8.md`).
    SlackPicker,
}

/// One selectable row in the Slack channel/user picker (`step8.md`) — a
/// channel or a user, the picker doesn't care which once rendered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerRow {
    /// Slack channel or user id.
    pub id: String,
    /// Display label (channel name or person's display name).
    pub label: String,
    /// Whether this row is checked for inclusion in the selection.
    pub selected: bool,
}

/// Outcome of the picker's data fetch / apply flow.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SlackPickerStatus {
    /// Overlay not open, or open but not yet fetched.
    #[default]
    Idle,
    /// `SlackPicker::list_channels`/`list_users` in flight.
    Loading,
    /// Lists fetched successfully; rows are ready to select.
    Loaded,
    /// `Command::ApplySlackSelection` dispatched, awaiting the result.
    Saving,
    /// Selection applied successfully.
    Saved,
    /// A fetch or apply failed; the message is shown to the user.
    Failed(String),
}

/// State for the Slack channel/user picker overlay (`step8.md`).
#[derive(Debug, Clone, Default)]
pub struct SlackPickerState {
    /// Fetched channels the bot has already been invited to.
    pub channels: Vec<PickerRow>,
    /// Fetched non-bot, non-deleted workspace members.
    pub users: Vec<PickerRow>,
    /// Index into the combined `channels` then `users` list.
    pub cursor: usize,
    /// Current fetch/apply outcome.
    pub status: SlackPickerStatus,
}

/// Outcome of the last `Command::ConnectSlack` dispatch, shown inline in
/// the setup overlay.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SlackSetupStatus {
    /// No connection attempt made yet this overlay session.
    #[default]
    Idle,
    /// `Command::ConnectSlack` dispatched, awaiting the result.
    Connecting,
    /// The command returned successfully.
    Connected,
    /// The command returned an error; the message is shown to the user.
    Failed(String),
}

/// Text input + last outcome for the Slack setup overlay (`step7.md`).
#[derive(Debug, Clone, Default)]
pub struct SlackSetupState {
    /// Token as typed so far — rendered masked (`*` per character), never
    /// shown in the clear or pushed into the command bar's history.
    pub token_input: String,
    /// Result of the most recent connection attempt, if any.
    pub status: SlackSetupStatus,
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
    /// Which dialog `Overlay` mode is currently showing.
    pub active_overlay: OverlayKind,
    /// Slack setup overlay's text input and last connection outcome.
    pub slack_setup: SlackSetupState,
    /// Slack channel/user picker overlay's fetched rows and selection.
    pub slack_picker: SlackPickerState,
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
            active_overlay: OverlayKind::default(),
            slack_setup: SlackSetupState::default(),
            slack_picker: SlackPickerState::default(),
            active_theme: "default-dark".to_string(),
            selected_index: 0,
            should_quit: false,
        }
    }
}
