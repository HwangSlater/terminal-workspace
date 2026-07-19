//! `docs/03-domain/workspace-state.md`.

use domain::NotificationItem;
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
    /// In-app Calendar connection entry (`Ctrl+L`, `step12.md`, extended
    /// for multiple calendars in `step24.md`) — adds a calendar (label +
    /// secret iCal URL), doesn't replace the existing set.
    CalendarSetup,
    /// Connected-calendar picker (`Ctrl+K`, `step24.md`) — select which
    /// calendars to *keep*; unchecking one and saving removes it. A local
    /// read of already-connected calendars, not a remote "list my
    /// calendars" discovery call (the secret-URL auth model still has no
    /// such API).
    CalendarPicker,
    /// Rename prompt for the calendar highlighted in `Ctrl+K`'s picker
    /// (`e`, `step25.md`) — a single plain-text field (a label isn't a
    /// secret), pre-filled with the current name.
    CalendarRename,
    /// Month grid view (`Ctrl+M`, `step25.md`) — a real calendar grid, not
    /// the flat "upcoming reminders" list the right dock shows. Read-only:
    /// navigate months and days, see which days have something on them.
    /// Fetches fresh via `CalendarManager::events_in_range` on open and on
    /// every month change -- independent of the reminder poll loop's
    /// `lookahead_hours` window entirely (a whole month's events has
    /// nothing to do with what the near-term reminder mechanism surfaces).
    CalendarGrid,
    /// Full scrollback view of the app's own log buffer (`Ctrl+4`,
    /// `step19.md`) — opened directly, the same way `Ctrl+S`/`Ctrl+G`/
    /// `Ctrl+L` open their setup overlays, rather than a "focus a dock,
    /// then Enter" two-step. Replaced a permanently-visible 1-line-tall
    /// bottom dock row that never showed enough to be useful.
    LogViewer,
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

/// Which field the Calendar setup overlay is currently capturing
/// (`step24.md`) -- adding a calendar now collects a display label before
/// the secret URL, so unlike Slack/GitHub's single-field overlays this one
/// has a small two-step sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CalendarSetupField {
    #[default]
    Label,
    Url,
}

/// Text input + last outcome for the Calendar setup overlay (`step12.md`,
/// extended for multiple calendars in `step24.md`). The "token" here is
/// the calendar's secret iCal feed URL, not a short bearer string — masked
/// the same way regardless, since it's still a bearer credential (leaking
/// it grants read access to the calendar). `label_input` is a plain
/// display name, not a secret, and isn't masked.
#[derive(Debug, Clone, Default)]
pub struct CalendarSetupState {
    /// Display label as typed so far (`step24.md`) — shown alongside this
    /// calendar's reminders once connected, e.g. `[회사] Design Review`.
    pub label_input: String,
    /// URL as typed so far — rendered masked, never shown in the clear or
    /// pushed into the command bar's history.
    pub token_input: String,
    /// Which field `Char`/`Backspace`/`Enter` currently apply to.
    pub field: CalendarSetupField,
    /// Result of the most recent connection attempt, if any.
    pub status: CalendarSetupStatus,
}

/// State for the Calendar rename prompt (`e` inside `Ctrl+K`'s picker,
/// `step25.md`) — a single plain-text field, pre-filled with the current
/// label when opened.
#[derive(Debug, Clone, Default)]
pub struct CalendarRenameState {
    /// Which connection (`PickerItem::id`, a UUID string).
    pub id: String,
    /// New label as typed so far, pre-filled with the current one.
    pub label_input: String,
}

/// Outcome of the Calendar picker's fetch/apply flow (`step24.md`).
/// Structurally identical to [`GitHubPickerStatus`] — "fetch" here is a
/// local read of already-connected calendars, not a network call, but the
/// overlay's loading/saving/failure states still apply the same way.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum CalendarPickerStatus {
    /// Overlay not open, or open but not yet fetched.
    #[default]
    Idle,
    /// `Picker::list_items` in flight.
    Loading,
    /// Connected calendars fetched successfully; rows are ready to select.
    Loaded,
    /// `Command::ApplySelection` dispatched, awaiting the result.
    Saving,
    /// Selection applied (i.e. unselected calendars removed) successfully.
    Saved,
    /// A fetch or apply failed; the message is shown to the user.
    Failed(String),
}

/// State for the Calendar picker overlay (`Ctrl+K`, `step24.md`) — select
/// which connected calendars to *keep*; unchecking one and saving removes
/// it. Simpler than [`SlackPickerState`]: one list, not two.
#[derive(Debug, Clone, Default)]
pub struct CalendarPickerState {
    /// Currently connected calendars.
    pub calendars: Vec<PickerRow>,
    /// Index into `calendars`.
    pub cursor: usize,
    /// Current fetch/apply outcome.
    pub status: CalendarPickerStatus,
}

/// Outcome of the month grid's on-demand fetch (`step25.md`). Simpler than
/// [`CalendarPickerStatus`] -- there's no "apply a selection" step here,
/// just fetch and display.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum CalendarGridStatus {
    /// Overlay not open, or open but not yet fetched.
    #[default]
    Idle,
    /// `CalendarManager::events_in_range` in flight for the displayed month.
    Loading,
    /// Fetched successfully; `events` reflects the displayed month.
    Loaded,
    /// The fetch failed; the message is shown to the user.
    Failed(String),
}

/// State for the month grid view (`Ctrl+M`, `step25.md`).
#[derive(Debug, Clone)]
pub struct CalendarGridState {
    /// Displayed month's year (e.g. `2026`).
    pub year: i32,
    /// Displayed month, 1-12.
    pub month: u32,
    /// Day-of-month the cursor is on, 1-based. Clamped to the displayed
    /// month's real day count, not carried over verbatim across a month
    /// change (a `31` selected in a 31-day month would be invalid in the
    /// next, shorter one).
    pub cursor_day: u32,
    /// Every occurrence fetched for the displayed month, across every
    /// connected calendar, labeled (`"[label] title"`) the same way the
    /// Notification/Calendar panel's reminders are. Rendering derives
    /// "which days have something" and "the cursor day's events" from
    /// this directly rather than pre-grouping it into a day map — the
    /// list is small (one month, from a handful of calendars) and
    /// re-filtering per render is simpler than keeping a derived
    /// structure in sync.
    pub events: Vec<NotificationItem>,
    /// Current fetch outcome.
    pub status: CalendarGridStatus,
}

impl Default for CalendarGridState {
    /// Defaults to *today's* year/month/day, not `0`/`1` -- opening the
    /// grid for the first time should land on "now," matching what every
    /// real calendar app does, not January of year zero.
    fn default() -> Self {
        use chrono::Datelike;
        let today = chrono::Local::now().date_naive();
        Self {
            year: today.year(),
            month: today.month(),
            cursor_day: today.day(),
            events: Vec::new(),
            status: CalendarGridStatus::default(),
        }
    }
}

/// Number of days in `(year, month)` -- shared by `keyboard.rs` (clamping
/// cursor movement) and `render.rs` (drawing the grid), so it's defined
/// once here rather than duplicated (`step25.md`). Computed as "one day
/// before the first of next month" since `chrono` has no direct
/// days-in-month query; handles the December -> January-of-next-year
/// rollover the same way month navigation itself does.
pub(crate) fn days_in_month(year: i32, month: u32) -> u32 {
    use chrono::Datelike;
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    chrono::NaiveDate::from_ymd_opt(next_year, next_month, 1)
        .and_then(|d| d.pred_opt())
        .map_or(30, |d| d.day())
}

/// `(year, month)` shifted by `delta` months (positive = forward,
/// negative = back), wrapping the year at the December/January boundary.
/// Shared by the same two callers as [`days_in_month`].
pub(crate) fn shift_month(year: i32, month: u32, delta: i32) -> (i32, u32) {
    let zero_based = i64::from(month) - 1 + i64::from(delta);
    let year_offset = zero_based.div_euclid(12);
    let new_month = zero_based.rem_euclid(12) + 1;
    (
        year + i32::try_from(year_offset).unwrap_or(0),
        u32::try_from(new_month).unwrap_or(1),
    )
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
    /// Calendar picker overlay's fetched rows and selection (`step24.md`).
    pub calendar_picker: CalendarPickerState,
    /// Calendar rename prompt's target id and text input (`step25.md`).
    pub calendar_rename: CalendarRenameState,
    /// Month grid view's displayed month, cursor, and fetched events
    /// (`step25.md`).
    pub calendar_grid: CalendarGridState,
    /// Calendar's connection status, kept current the same way
    /// `slack_connection_status` is (`step12.md`).
    pub calendar_connection_status: IntegrationConnectionStatus,
    /// Active theme name (`docs/02-architecture/theme.md` lists valid values).
    pub active_theme: String,
    /// Animation frame counter (`step30.md`), incremented on a periodic
    /// redraw tick (`crates/ui/src/lib.rs`'s `event_loop`) — drives the
    /// loading spinner's frame (`theme::spinner_frame`). Not tied to real
    /// time itself; only its parity/modulus matters.
    pub anim_tick: u64,
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
            // `Left` (Team) is no longer a focus-navigable body dock
            // (`step32.md` -- Team moved into the header), so the default
            // focus target becomes Notification instead.
            focused_dock: UiDockSlot::Center,
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
            calendar_picker: CalendarPickerState::default(),
            calendar_rename: CalendarRenameState::default(),
            calendar_grid: CalendarGridState::default(),
            // Same placeholder reasoning as slack_connection_status above.
            calendar_connection_status: IntegrationConnectionStatus::Disconnected,
            active_theme: "default-dark".to_string(),
            anim_tick: 0,
            selected_index: 0,
            should_quit: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_in_month_handles_a_normal_month() {
        assert_eq!(days_in_month(2026, 1), 31);
        assert_eq!(days_in_month(2026, 4), 30);
    }

    #[test]
    fn days_in_month_handles_february_leap_and_non_leap_years() {
        assert_eq!(days_in_month(2024, 2), 29); // leap year
        assert_eq!(days_in_month(2026, 2), 28); // not a leap year
    }

    #[test]
    fn days_in_month_handles_the_december_year_rollover() {
        assert_eq!(days_in_month(2026, 12), 31);
    }

    #[test]
    fn shift_month_moves_forward_within_the_same_year() {
        assert_eq!(shift_month(2026, 6, 1), (2026, 7));
    }

    #[test]
    fn shift_month_moves_backward_within_the_same_year() {
        assert_eq!(shift_month(2026, 6, -1), (2026, 5));
    }

    #[test]
    fn shift_month_rolls_forward_into_the_next_year() {
        assert_eq!(shift_month(2026, 12, 1), (2027, 1));
    }

    #[test]
    fn shift_month_rolls_backward_into_the_previous_year() {
        assert_eq!(shift_month(2026, 1, -1), (2025, 12));
    }

    #[test]
    fn calendar_grid_state_defaults_to_todays_date_not_january_of_year_zero() {
        use chrono::Datelike;
        let today = chrono::Local::now().date_naive();
        let grid = CalendarGridState::default();
        assert_eq!(grid.year, today.year());
        assert_eq!(grid.month, today.month());
        assert_eq!(grid.cursor_day, today.day());
    }
}
