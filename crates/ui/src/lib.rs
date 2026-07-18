//! Terminal User Interface (TUI). See `docs/02-architecture/ui.md`,
//! `docs/02-architecture/keyboard.md`, `docs/03-domain/workspace-state.md`,
//! and `docs/01-product/screen-spec.md` for the full specification this
//! implements (Phase 5 scope — see `step5.md`).

mod keyboard;
mod render;
mod state;

pub use keyboard::{handle_key, KeyOutcome, PaneAction};
pub use state::{ActiveLayout, CommandBufferState, FocusMode, PanelId, WorkspaceState};

use commands::{Command, CommandDispatcher, SharedReadModel};
use common::{Result, WorkspaceError};
use crossterm::event::{Event as CrosstermEvent, KeyEventKind};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use domain::IntegrationSource;
use events::{Event as DomainEvent, EventBus, IntegrationConnectionStatus};
use integration::{Picker, SlackPicker};
use logging::LogBuffer;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use registry::UiRegistry;
use scheduler::AgendaScheduler;
use state::{
    CalendarSetupStatus, GitHubPickerStatus, GitHubSetupStatus, PickerRow, SlackPickerStatus,
    SlackSetupStatus,
};
use std::collections::HashMap;
use std::io::Stdout;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};

type Backend = CrosstermBackend<Stdout>;

fn io_err(e: std::io::Error) -> WorkspaceError {
    WorkspaceError::Internal(e.to_string())
}

/// Dynamic TUI App engine orchestrating Crossterm events and Ratatui frames.
pub struct TuiRenderer {
    #[allow(dead_code)] // wired for future dynamic panel registration (plugins)
    ui_registry: Arc<dyn UiRegistry>,
    read_model: SharedReadModel,
    /// Lets the run loop actually mutate state through the CQRS write path
    /// (`Command::Connect` from the setup overlay, `step7.md`) — before
    /// this phase, `TuiRenderer` was pure CQRS *read* side with no way to
    /// dispatch anything.
    command_dispatcher: Arc<dyn CommandDispatcher>,
    /// Direct read-only port for the picker overlay (`Ctrl+P`, `step8.md`)
    /// to list channels/users. Held separately from `command_dispatcher`
    /// deliberately — listing is a query, not a mutation, so it doesn't go
    /// through `Command`/`CommandHandler` (see `SlackSelectionApplier`'s
    /// doc comment in `crates/commands` for the full reasoning).
    slack_picker: Arc<dyn SlackPicker>,
    /// Every single-selectable-list integration's picker port, keyed by
    /// source. Holds GitHub today; a future Calendar just adds a key here
    /// instead of `TuiRenderer` growing another named field
    /// (`step11.md` — replaces the earlier `github_picker: Arc<dyn
    /// GitHubPicker>` field). Slack's two-list `slack_picker` above stays
    /// separate, same reasoning as `commands::WorkspaceCommandHandler`.
    pickers: HashMap<IntegrationSource, Arc<dyn Picker>>,
    /// Subscribed to in `run_loop` so the render loop redraws on background
    /// changes (a new message, a status change) instead of only ever on a
    /// keypress/resize (`step9.md`, ADR-0016) — before this phase
    /// `TuiRenderer` never read from the bus at all, only `Projector` did.
    event_bus: Arc<dyn EventBus>,
    /// Slack's status as of construction (`SlackAdapter::health_check`,
    /// read once in `crates/app/src/main.rs` before the bus has published
    /// anything) — seeds `WorkspaceState.slack_connection_status`; kept
    /// current after that purely by the `event_bus` subscription.
    initial_slack_status: IntegrationConnectionStatus,
    /// GitHub's equivalent of `initial_slack_status` (`step10.md`).
    initial_github_status: IntegrationConnectionStatus,
    /// Calendar's equivalent of `initial_slack_status` (`step12.md`).
    initial_calendar_status: IntegrationConnectionStatus,
    /// Backs the bottom "로그" dock (`step17.md`) — snapshotted fresh each
    /// frame in `draw()`, the same way `read_model` already is.
    log_buffer: Arc<LogBuffer>,
    /// Backs the header's Pomodoro segment (`step18.md`) — snapshotted
    /// fresh each frame, same pattern as `log_buffer`.
    scheduler: Arc<AgendaScheduler>,
}

impl TuiRenderer {
    /// Create new renderer wrapper.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        ui_registry: Arc<dyn UiRegistry>,
        read_model: SharedReadModel,
        command_dispatcher: Arc<dyn CommandDispatcher>,
        slack_picker: Arc<dyn SlackPicker>,
        pickers: HashMap<IntegrationSource, Arc<dyn Picker>>,
        event_bus: Arc<dyn EventBus>,
        initial_slack_status: IntegrationConnectionStatus,
        initial_github_status: IntegrationConnectionStatus,
        initial_calendar_status: IntegrationConnectionStatus,
        log_buffer: Arc<LogBuffer>,
        scheduler: Arc<AgendaScheduler>,
    ) -> Self {
        Self {
            ui_registry,
            read_model,
            command_dispatcher,
            slack_picker,
            pickers,
            event_bus,
            initial_slack_status,
            initial_github_status,
            initial_calendar_status,
            log_buffer,
            scheduler,
        }
    }

    /// Enter the terminal, run the render/input loop until the user quits
    /// (`Ctrl+Q`), then restore the terminal — even on panic.
    pub async fn run_loop(&self) -> Result<()> {
        install_panic_hook();
        let mut terminal = setup_terminal()?;
        let mut state = WorkspaceState {
            slack_connection_status: self.initial_slack_status.clone(),
            github_connection_status: self.initial_github_status.clone(),
            calendar_connection_status: self.initial_calendar_status.clone(),
            ..WorkspaceState::default()
        };

        let (tx, mut rx) = mpsc::unbounded_channel();
        spawn_input_reader(tx);
        let mut event_rx = self.event_bus.subscribe();

        let result = self
            .event_loop(&mut terminal, &mut state, &mut rx, &mut event_rx)
            .await;

        restore_terminal(&mut terminal)?;
        result
    }

    async fn event_loop(
        &self,
        terminal: &mut Terminal<Backend>,
        state: &mut WorkspaceState,
        rx: &mut mpsc::UnboundedReceiver<InputEvent>,
        event_rx: &mut broadcast::Receiver<DomainEvent>,
    ) -> Result<()> {
        self.draw(terminal, state).await?;

        loop {
            tokio::select! {
                input = rx.recv() => {
                    let Some(input) = input else { break; };
                    if let InputEvent::Key(key) = input {
                        match handle_key(state, key) {
                            KeyOutcome::DispatchToPane(action) => {
                                self.apply_pane_action(state, action).await;
                            }
                            KeyOutcome::SubmitToken(source, token) => {
                                self.submit_token(terminal, state, source, token).await?;
                            }
                            KeyOutcome::OpenSlackPicker => {
                                self.open_slack_picker(terminal, state).await?;
                            }
                            KeyOutcome::SubmitSlackSelection(channel_ids, watched_user_ids) => {
                                self.submit_slack_selection(terminal, state, channel_ids, watched_user_ids)
                                    .await?;
                            }
                            KeyOutcome::OpenPicker(source) => {
                                self.open_picker(terminal, state, source).await?;
                            }
                            KeyOutcome::SubmitSelection(source, items) => {
                                self.submit_selection(terminal, state, source, items)
                                    .await?;
                            }
                            KeyOutcome::SubmitCommand(command) => {
                                self.submit_command(terminal, state, command).await?;
                            }
                            KeyOutcome::Handled | KeyOutcome::Ignored => {}
                        }
                        if state.should_quit {
                            break;
                        }
                    }
                    // `InputEvent::Resize` carries no data to apply to
                    // `state` — `ratatui::Terminal::draw` re-queries the
                    // backend's current size on every call
                    // (`Terminal::autoresize`), so simply redrawing is
                    // enough to pick up the new dimensions.
                    self.draw(terminal, state).await?;
                }
                received = event_rx.recv() => {
                    match received {
                        Ok(DomainEvent::IntegrationStatusChanged { source, status }) => {
                            // Route by source -- before GitHub existed this
                            // event only ever carried Slack, so writing to
                            // slack_connection_status unconditionally was
                            // correct by coincidence, not by design; now
                            // that a second source exists it must be
                            // checked, or a GitHub status update would
                            // clobber the Slack header line.
                            match source {
                                IntegrationSource::Slack => state.slack_connection_status = status,
                                IntegrationSource::GitHub => state.github_connection_status = status,
                                IntegrationSource::Calendar => {
                                    state.calendar_connection_status = status;
                                }
                                IntegrationSource::Gmail | IntegrationSource::Jira => {}
                            }
                            self.draw(terminal, state).await?;
                        }
                        Ok(_) => {
                            // Some other event (new message, presence
                            // change) may have just updated
                            // DashboardReadModel via Projector -- redraw so
                            // it's visible now, not only on the user's next
                            // keypress.
                            self.draw(terminal, state).await?;
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            // Missed some events under load; whatever state
                            // they'd have produced is still reachable from
                            // the next event or keypress -- nothing to
                            // reconcile specifically here.
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            // Bus is gone; the input loop (Ctrl+Q) is still
                            // the way this function returns, not this arm.
                        }
                    }
                }
            }
        }
        Ok(())
    }

    async fn apply_pane_action(&self, state: &mut WorkspaceState, action: PaneAction) {
        let model = self.read_model.read().await;
        let len = match state.focused_dock {
            registry::UiDockSlot::Left => model.team_presence.len(),
            registry::UiDockSlot::Center => model.unread_notifications.len(),
            // Right dock (Calendar) shows only the Calendar-sourced subset
            // of unread_notifications (render::render_calendar_panel) --
            // was always 0 while that panel was a static placeholder with
            // nothing to navigate; now it has real rows. Shares the same
            // filter render.rs uses so the two can't drift apart.
            registry::UiDockSlot::Right => render::calendar_notifications(&model).len(),
            // Unreachable in practice since step19.md -- Bottom never
            // enters `focused_dock` anymore (dropped from `DOCK_CYCLE`,
            // `Ctrl+4` opens the Log Viewer overlay directly instead of
            // focusing a dock). Kept for exhaustiveness over
            // `UiDockSlot`'s four variants.
            registry::UiDockSlot::Bottom => 0,
        };
        drop(model);
        if len == 0 {
            return;
        }
        match action {
            PaneAction::Up => state.selected_index = state.selected_index.saturating_sub(1),
            PaneAction::Down => state.selected_index = (state.selected_index + 1).min(len - 1),
            PaneAction::Left | PaneAction::Right | PaneAction::Activate => {
                // No expandable tree nodes or activatable detail view yet
                // (Phase 5 scope: shell only) — nothing to do.
            }
        }
    }

    /// Dispatches `Command::Connect` for a token submitted through a setup
    /// overlay (Slack `step7.md`, GitHub `step10.md`), redrawing before the
    /// network call so "연결 중..." is visible immediately rather than the UI
    /// appearing to freeze until the request completes. Generalized in
    /// `step11.md` from separate `submit_slack_token`/`submit_github_token`
    /// methods — the dispatch itself is identical, only which
    /// `WorkspaceState` field records the outcome differs per source.
    async fn submit_token(
        &self,
        terminal: &mut Terminal<Backend>,
        state: &mut WorkspaceState,
        source: IntegrationSource,
        token: String,
    ) -> Result<()> {
        self.draw(terminal, state).await?;
        let result = self
            .command_dispatcher
            .dispatch(Command::Connect { source, token })
            .await;
        match source {
            IntegrationSource::Slack => {
                state.slack_setup.status = match result {
                    Ok(()) => SlackSetupStatus::Connected,
                    Err(e) => SlackSetupStatus::Failed(e.to_string()),
                };
            }
            IntegrationSource::GitHub => {
                state.github_setup.status = match result {
                    Ok(()) => GitHubSetupStatus::Connected,
                    Err(e) => GitHubSetupStatus::Failed(e.to_string()),
                };
            }
            IntegrationSource::Calendar => {
                state.calendar_setup.status = match result {
                    Ok(()) => CalendarSetupStatus::Connected,
                    Err(e) => CalendarSetupStatus::Failed(e.to_string()),
                };
            }
            IntegrationSource::Gmail | IntegrationSource::Jira => {}
        }
        Ok(())
    }

    /// Fetches channel/user lists for the picker overlay (`step8.md`),
    /// redrawing first so "불러오는 중..." shows immediately rather than the
    /// UI appearing frozen during the network calls.
    async fn open_slack_picker(
        &self,
        terminal: &mut Terminal<Backend>,
        state: &mut WorkspaceState,
    ) -> Result<()> {
        self.draw(terminal, state).await?;

        let channels = self.slack_picker.list_channels().await;
        let users = match &channels {
            Ok(_) => self.slack_picker.list_users().await,
            Err(_) => Ok(Vec::new()), // don't bother with a second call if the first already failed
        };

        match (channels, users) {
            (Ok(channels), Ok(users)) => {
                state.slack_picker.channels = channels
                    .into_iter()
                    .map(|c| PickerRow {
                        id: c.id,
                        label: c.name,
                        selected: false,
                    })
                    .collect();
                state.slack_picker.users = users
                    .into_iter()
                    .map(|u| PickerRow {
                        id: u.id,
                        label: u.display_name,
                        selected: false,
                    })
                    .collect();
                state.slack_picker.cursor = 0;
                state.slack_picker.status = SlackPickerStatus::Loaded;
            }
            (Err(e), _) | (_, Err(e)) => {
                state.slack_picker.status = SlackPickerStatus::Failed(e.to_string());
            }
        }
        Ok(())
    }

    /// Dispatches `Command::ApplySlackSelection` for a selection confirmed
    /// in the picker overlay (`step8.md`).
    async fn submit_slack_selection(
        &self,
        terminal: &mut Terminal<Backend>,
        state: &mut WorkspaceState,
        channel_ids: Vec<String>,
        watched_user_ids: Vec<String>,
    ) -> Result<()> {
        self.draw(terminal, state).await?;
        let result = self
            .command_dispatcher
            .dispatch(Command::ApplySlackSelection {
                channel_ids,
                watched_user_ids,
            })
            .await;
        state.slack_picker.status = match result {
            Ok(()) => SlackPickerStatus::Saved,
            Err(e) => SlackPickerStatus::Failed(e.to_string()),
        };
        Ok(())
    }

    /// Fetches the item list for a single-list picker overlay (GitHub's
    /// `step10.md`; a future Calendar would land here too). Simpler than
    /// `open_slack_picker`: one list, one call. Generalized in `step11.md`
    /// from `open_github_picker` — looks up the right `Picker` by `source`
    /// instead of holding a named `github_picker` field.
    async fn open_picker(
        &self,
        terminal: &mut Terminal<Backend>,
        state: &mut WorkspaceState,
        source: IntegrationSource,
    ) -> Result<()> {
        self.draw(terminal, state).await?;

        let Some(picker) = self.pickers.get(&source) else {
            // Only a bound keybinding (Ctrl+R today) can produce
            // KeyOutcome::OpenPicker, and every such binding only exists
            // for a source that was actually registered at construction
            // time -- an unregistered source here would be a wiring bug,
            // not a reachable user-facing state. Stay honest rather than
            // silently no-op if that invariant is ever violated.
            return Err(WorkspaceError::Internal(format!(
                "no picker registered for {source:?}"
            )));
        };

        let result = picker.list_items().await;
        match (source, result) {
            (IntegrationSource::GitHub, Ok(items)) => {
                state.github_picker.repositories = items
                    .into_iter()
                    .map(|i| PickerRow {
                        id: i.id,
                        label: i.label,
                        selected: false,
                    })
                    .collect();
                state.github_picker.cursor = 0;
                state.github_picker.status = GitHubPickerStatus::Loaded;
            }
            (IntegrationSource::GitHub, Err(e)) => {
                state.github_picker.status = GitHubPickerStatus::Failed(e.to_string());
            }
            _ => {}
        }
        Ok(())
    }

    /// Dispatches `Command::ApplySelection` for a selection confirmed in a
    /// single-list picker overlay (`step10.md`). Generalized in
    /// `step11.md` from `submit_github_selection`.
    async fn submit_selection(
        &self,
        terminal: &mut Terminal<Backend>,
        state: &mut WorkspaceState,
        source: IntegrationSource,
        items: Vec<String>,
    ) -> Result<()> {
        self.draw(terminal, state).await?;
        let result = self
            .command_dispatcher
            .dispatch(Command::ApplySelection { source, items })
            .await;
        match source {
            IntegrationSource::GitHub => {
                state.github_picker.status = match result {
                    Ok(()) => GitHubPickerStatus::Saved,
                    Err(e) => GitHubPickerStatus::Failed(e.to_string()),
                };
            }
            IntegrationSource::Slack
            | IntegrationSource::Calendar
            | IntegrationSource::Gmail
            | IntegrationSource::Jira => {}
        }
        Ok(())
    }

    /// Dispatches a command bar line that parsed successfully (`/send`,
    /// `/away`, ... — `step9.md`). A failure is surfaced the same way an
    /// unresolved `/send` target is (`state.cmd_buffer.last_error`), not a
    /// silent drop — the user typed a deliberate command, they should know
    /// if it didn't work.
    async fn submit_command(
        &self,
        terminal: &mut Terminal<Backend>,
        state: &mut WorkspaceState,
        command: Command,
    ) -> Result<()> {
        self.draw(terminal, state).await?;
        let result = self.command_dispatcher.dispatch(command).await;
        state.cmd_buffer.last_error = result.err().map(|e| e.to_string());
        Ok(())
    }

    async fn draw(&self, terminal: &mut Terminal<Backend>, state: &WorkspaceState) -> Result<()> {
        let model = self.read_model.read().await;
        let log_lines = self.log_buffer.snapshot();
        let pomodoro = self.scheduler.snapshot().await;
        terminal
            .draw(|frame| render::render(frame, state, &model, &log_lines, &pomodoro))
            .map_err(io_err)?;
        Ok(())
    }
}

fn setup_terminal() -> Result<Terminal<Backend>> {
    enable_raw_mode().map_err(io_err)?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(stdout, EnterAlternateScreen).map_err(io_err)?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend).map_err(io_err)
}

fn restore_terminal(terminal: &mut Terminal<Backend>) -> Result<()> {
    disable_raw_mode().map_err(io_err)?;
    crossterm::execute!(terminal.backend_mut(), LeaveAlternateScreen).map_err(io_err)?;
    terminal.show_cursor().map_err(io_err)?;
    Ok(())
}

/// A panic inside the render loop must not leave the user's terminal stuck
/// in raw mode / the alternate screen.
fn install_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = disable_raw_mode();
        let _ = crossterm::execute!(std::io::stdout(), LeaveAlternateScreen);
        original(panic_info);
    }));
}

/// Everything the input reader thread hands to the render loop: either a
/// key press to interpret, or a terminal resize to redraw against (no
/// payload needed — see `event_loop`'s handling of this variant).
enum InputEvent {
    Key(crossterm::event::KeyEvent),
    Resize,
}

/// `crossterm::event::read()` blocks indefinitely until the next key press
/// — there is no way to cancel or time out a call already in progress. That
/// makes it unsafe to run on `tokio::task::spawn_blocking`: dropping the
/// `tokio::Runtime` at process shutdown waits for outstanding blocking
/// tasks to finish, so if the user's last input was `Ctrl+Q` and they then
/// stop typing, the read call in progress never returns and the process
/// hangs forever instead of exiting. A plain OS thread has no such
/// join-on-shutdown behavior — it's simply abandoned when the process
/// exits, which is exactly what we want here. `mpsc::UnboundedSender::send`
/// is synchronous and works from any thread, tokio or not, so nothing else
/// about the non-blocking-input design (`docs/02-architecture/ui.md`)
/// changes.
fn spawn_input_reader(tx: mpsc::UnboundedSender<InputEvent>) {
    std::thread::spawn(move || loop {
        match crossterm::event::read() {
            Ok(CrosstermEvent::Key(key)) if key.kind == KeyEventKind::Press => {
                if tx.send(InputEvent::Key(key)).is_err() {
                    break;
                }
            }
            Ok(CrosstermEvent::Resize(_, _)) => {
                if tx.send(InputEvent::Resize).is_err() {
                    break;
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    });
}
