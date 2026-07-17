//! Terminal User Interface (TUI). See `docs/02-architecture/ui.md`,
//! `docs/02-architecture/keyboard.md`, `docs/03-domain/workspace-state.md`,
//! and `docs/01-product/screen-spec.md` for the full specification this
//! implements (Phase 5 scope — see `step5.md`).

mod keyboard;
mod render;
mod state;

pub use keyboard::{handle_key, KeyOutcome, PaneAction};
pub use state::{ActiveLayout, CommandBufferState, FocusMode, PanelId, WorkspaceState};

use commands::SharedReadModel;
use common::{Result, WorkspaceError};
use crossterm::event::{Event as CrosstermEvent, KeyEventKind};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use registry::UiRegistry;
use std::io::Stdout;
use std::sync::Arc;
use tokio::sync::mpsc;

type Backend = CrosstermBackend<Stdout>;

fn io_err(e: std::io::Error) -> WorkspaceError {
    WorkspaceError::Internal(e.to_string())
}

/// Dynamic TUI App engine orchestrating Crossterm events and Ratatui frames.
pub struct TuiRenderer {
    #[allow(dead_code)] // wired for future dynamic panel registration (plugins)
    ui_registry: Arc<dyn UiRegistry>,
    read_model: SharedReadModel,
}

impl TuiRenderer {
    /// Create new renderer wrapper.
    #[must_use]
    pub fn new(ui_registry: Arc<dyn UiRegistry>, read_model: SharedReadModel) -> Self {
        Self {
            ui_registry,
            read_model,
        }
    }

    /// Enter the terminal, run the render/input loop until the user quits
    /// (`Ctrl+Q`), then restore the terminal — even on panic.
    pub async fn run_loop(&self) -> Result<()> {
        install_panic_hook();
        let mut terminal = setup_terminal()?;
        let mut state = WorkspaceState::default();

        let (tx, mut rx) = mpsc::unbounded_channel();
        spawn_input_reader(tx);

        let result = self.event_loop(&mut terminal, &mut state, &mut rx).await;

        restore_terminal(&mut terminal)?;
        result
    }

    async fn event_loop(
        &self,
        terminal: &mut Terminal<Backend>,
        state: &mut WorkspaceState,
        rx: &mut mpsc::UnboundedReceiver<InputEvent>,
    ) -> Result<()> {
        self.draw(terminal, state).await?;

        while let Some(event) = rx.recv().await {
            if let InputEvent::Key(key) = event {
                let outcome = handle_key(state, key);
                if let KeyOutcome::DispatchToPane(action) = outcome {
                    self.apply_pane_action(state, action).await;
                }
                if state.should_quit {
                    break;
                }
            }
            // `InputEvent::Resize` carries no data to apply to `state` —
            // `ratatui::Terminal::draw` re-queries the backend's current
            // size on every call (`Terminal::autoresize`), so simply
            // redrawing is enough to pick up the new dimensions.
            self.draw(terminal, state).await?;
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
