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
use events::{Event as DomainEvent, EventBus, IntegrationConnectionStatus};
use integration::SlackPicker;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use registry::UiRegistry;
use state::{PickerRow, SlackPickerStatus, SlackSetupStatus};
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
    /// (`Command::ConnectSlack` from the setup overlay, `step7.md`) — before
    /// this phase, `TuiRenderer` was pure CQRS *read* side with no way to
    /// dispatch anything.
    command_dispatcher: Arc<dyn CommandDispatcher>,
    /// Direct read-only port for the picker overlay (`Ctrl+P`, `step8.md`)
    /// to list channels/users. Held separately from `command_dispatcher`
    /// deliberately — listing is a query, not a mutation, so it doesn't go
    /// through `Command`/`CommandHandler` (see `SlackSelectionApplier`'s
    /// doc comment in `crates/commands` for the full reasoning).
    slack_picker: Arc<dyn SlackPicker>,
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
}

impl TuiRenderer {
    /// Create new renderer wrapper.
    #[must_use]
    pub fn new(
        ui_registry: Arc<dyn UiRegistry>,
        read_model: SharedReadModel,
        command_dispatcher: Arc<dyn CommandDispatcher>,
        slack_picker: Arc<dyn SlackPicker>,
        event_bus: Arc<dyn EventBus>,
        initial_slack_status: IntegrationConnectionStatus,
    ) -> Self {
        Self {
            ui_registry,
            read_model,
            command_dispatcher,
            slack_picker,
            event_bus,
            initial_slack_status,
        }
    }

    /// Enter the terminal, run the render/input loop until the user quits
    /// (`Ctrl+Q`), then restore the terminal — even on panic.
    pub async fn run_loop(&self) -> Result<()> {
        install_panic_hook();
        let mut terminal = setup_terminal()?;
        let mut state = WorkspaceState {
            slack_connection_status: self.initial_slack_status.clone(),
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
                            KeyOutcome::SubmitSlackToken(token) => {
                                self.submit_slack_token(terminal, state, token).await?;
                            }
                            KeyOutcome::OpenSlackPicker => {
                                self.open_slack_picker(terminal, state).await?;
                            }
                            KeyOutcome::SubmitSlackSelection(channel_ids, watched_user_ids) => {
                                self.submit_slack_selection(terminal, state, channel_ids, watched_user_ids)
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
                        Ok(DomainEvent::IntegrationStatusChanged { status, .. }) => {
                            state.slack_connection_status = status;
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
            registry::UiDockSlot::Right | registry::UiDockSlot::Bottom => 0,
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

    /// Dispatches `Command::ConnectSlack` for a token submitted through the
    /// setup overlay (`step7.md`), redrawing before the network call so
    /// "연결 중..." is visible immediately rather than the UI appearing to
    /// freeze until the request completes.
    async fn submit_slack_token(
        &self,
        terminal: &mut Terminal<Backend>,
        state: &mut WorkspaceState,
        token: String,
    ) -> Result<()> {
        self.draw(terminal, state).await?;
        let result = self
            .command_dispatcher
            .dispatch(Command::ConnectSlack { token })
            .await;
        state.slack_setup.status = match result {
            Ok(()) => SlackSetupStatus::Connected,
            Err(e) => SlackSetupStatus::Failed(e.to_string()),
        };
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
        terminal
            .draw(|frame| render::render(frame, state, &model))
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
