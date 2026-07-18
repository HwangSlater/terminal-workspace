//! `docs/03-domain/workspace-state.md`.

use events::IntegrationConnectionStatus;
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
/// (`step7.md`, `step8.md`, `step10.md`, `step12.md`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum OverlayKind {
    /// Static keybinding reference.
    #[default]
    Help,
    /// In-app Slack Bot Token entry (`Ctrl+S`).
    SlackSetup,
    /// Channel/watched-user picker (`Ctrl+P`, `step8.md`).
    SlackPicker,
    /// In-app GitHub PAT entry (`Ctrl+G`, `step10.md`).
    GitHubSetup,
    /// Repository picker (`Ctrl+R`, `step10.md`).
    GitHubPicker,
    /// In-app Calendar secret iCal URL entry (`Ctrl+L`, `step12.md`). No
    /// picker overlay exists for Calendar — there's no "list my calendars"
    /// discovery call under the secret-URL auth model (`step12.md`
    /// Decision 1's consequence).
    CalendarSetup,
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

/// Outcome of the last `Command::Connect` dispatch, shown inline in
/// the setup overlay.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SlackSetupStatus {
    /// No connection attempt made yet this overlay session.
    #[default]
    Idle,
    /// `Command::Connect` dispatched, awaiting the result.
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

/// Outcome of the last `Command::Connect` dispatch, shown inline in
/// the setup overlay. Structurally identical to [`SlackSetupStatus`] —
/// kept as a separate type (not a shared generic) since the two overlays
/// are independent UI flows that happen to look alike today, matching
/// `step10.md` Decision 1's "duplicate structure per integration" choice.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum GitHubSetupStatus {
    /// No connection attempt made yet this overlay session.
    #[default]
    Idle,
    /// `Command::Connect` dispatched, awaiting the result.
    Connecting,
    /// The command returned successfully.
    Connected,
    /// The command returned an error; the message is shown to the user.
    Failed(String),
}

/// Text input + last outcome for the GitHub setup overlay (`step10.md`).
#[derive(Debug, Clone, Default)]
pub struct GitHubSetupState {
    /// Token as typed so far — rendered masked (`*` per character), never
    /// shown in the clear or pushed into the command bar's history.
    pub token_input: String,
    /// Result of the most recent connection attempt, if any.
    pub status: GitHubSetupStatus,
}

/// Outcome of the GitHub repository picker's data fetch / apply flow. See
/// [`SlackPickerStatus`] — same shape, kept separate for the same reason
/// [`GitHubSetupStatus`] is.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum GitHubPickerStatus {
    /// Overlay not open, or open but not yet fetched.
    #[default]
    Idle,
    /// `Picker::list_items` in flight.
    Loading,
    /// Repositories fetched successfully; rows are ready to select.
    Loaded,
    /// `Command::ApplySelection` dispatched, awaiting the result.
    Saving,
    /// Selection applied successfully.
    Saved,
    /// A fetch or apply failed; the message is shown to the user.
    Failed(String),
}

/// State for the GitHub repository picker overlay (`step10.md`). Simpler
/// than [`SlackPickerState`]: one list (repositories), not two
/// (channels + users).
#[derive(Debug, Clone, Default)]
pub struct GitHubPickerState {
    /// Fetched repositories the authenticated user can access.
    pub repositories: Vec<PickerRow>,
    /// Index into `repositories`.
    pub cursor: usize,
    /// Current fetch/apply outcome.
    pub status: GitHubPickerStatus,
}

/// Outcome of the last `Command::Connect` dispatch, shown inline in the
/// Calendar setup overlay. Structurally identical to
/// [`GitHubSetupStatus`]/[`SlackSetupStatus`] — same "duplicate structure
/// per integration" choice (`step10.md` Decision 1).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum CalendarSetupStatus {
    /// No connection attempt made yet this overlay session.
    #[default]
    Idle,
    /// `Command::Connect` dispatched, awaiting the result.
    Connecting,
    /// The command returned successfully.
    Connected,
    /// The command returned an error; the message is shown to the user.
    Failed(String),
}

/// Text input + last outcome for the Calendar setup overlay (`step12.md`).
/// The "token" here is the calendar's secret iCal feed URL, not a short
/// bearer string — masked the same way regardless, since it's still a
/// bearer credential (leaking it grants read access to the calendar).
#[derive(Debug, Clone, Default)]
pub struct CalendarSetupState {
    /// URL as typed so far — rendered masked, never shown in the clear or
    /// pushed into the command bar's history.
    pub token_input: String,
    /// Result of the most recent connection attempt, if any.
    pub status: CalendarSetupStatus,
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
    /// Set when the last submitted line looked like a command attempt
    /// (leading `/`) but failed to parse or resolve (e.g. `/send #unknown`)
    /// — shown inline so a deliberate command attempt doesn't silently do
    /// nothing. Plain chat-style text (no leading `/`) never sets this.
    pub last_error: Option<String>,
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
    /// Slack's connection status, kept current by an `EventBus` subscription
    /// in the run loop (`step9.md`, ADR-0016) — not polled, genuinely live.
    pub slack_connection_status: IntegrationConnectionStatus,
    /// GitHub setup overlay's text input and last connection outcome.
    pub github_setup: GitHubSetupState,
    /// GitHub repository picker overlay's fetched rows and selection.
    pub github_picker: GitHubPickerState,
    /// GitHub's connection status, kept current the same way
    /// `slack_connection_status` is (`step10.md`).
    pub github_connection_status: IntegrationConnectionStatus,
    /// Calendar setup overlay's text input and last connection outcome.
    pub calendar_setup: CalendarSetupState,
    /// Calendar's connection status, kept current the same way
    /// `slack_connection_status` is (`step12.md`). No picker state exists
    /// for Calendar (`step12.md` Decision 1's consequence).
    pub calendar_connection_status: IntegrationConnectionStatus,
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
            // Placeholder until `TuiRenderer::run_loop` overwrites it with
            // the real value from `SlackAdapter::health_check` at boot —
            // `WorkspaceState::default()` has no way to reach that itself.
            slack_connection_status: IntegrationConnectionStatus::Disconnected,
            github_setup: GitHubSetupState::default(),
            github_picker: GitHubPickerState::default(),
            // Same placeholder reasoning as slack_connection_status above.
            github_connection_status: IntegrationConnectionStatus::Disconnected,
            calendar_setup: CalendarSetupState::default(),
            // Same placeholder reasoning as slack_connection_status above.
            calendar_connection_status: IntegrationConnectionStatus::Disconnected,
            active_theme: "default-dark".to_string(),
            selected_index: 0,
            should_quit: false,
        }
    }
}
