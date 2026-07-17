//! Modal input capture pipeline. See `docs/02-architecture/keyboard.md` —
//! this is a direct implementation of that document's capture pipeline
//! diagram, not a new design.

use crate::state::{FocusMode, WorkspaceState};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use registry::UiDockSlot;

/// Fixed focus-cycle order for `Tab`/`Shift+Tab` (`keyboard.md`'s "Cycles
/// focus clockwise/counter-clockwise through visible layout panes").
const DOCK_CYCLE: [UiDockSlot; 4] = [
    UiDockSlot::Left,
    UiDockSlot::Center,
    UiDockSlot::Right,
    UiDockSlot::Bottom,
];

/// A pane-specific navigation action, once a key has fallen through the
/// global-shortcut checks in Normal mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneAction {
    /// `k` / Up arrow.
    Up,
    /// `j` / Down arrow.
    Down,
    /// `h` / Left arrow.
    Left,
    /// `l` / Right arrow.
    Right,
    /// `Enter`.
    Activate,
}

/// Result of processing one key event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyOutcome {
    /// Consumed internally (mode switch, global shortcut, text input).
    Handled,
    /// Not a global shortcut in Normal mode; forward to the focused pane.
    DispatchToPane(PaneAction),
    /// Recognized as "nothing to do" in the current context.
    Ignored,
}

/// Process one key event against `state`, mutating it per
/// `docs/02-architecture/keyboard.md`'s capture pipeline:
/// `Esc` always returns to Normal mode; Input mode captures raw text;
/// Normal mode checks global shortcuts first, then falls through to the
/// focused pane.
pub fn handle_key(state: &mut WorkspaceState, key: KeyEvent) -> KeyOutcome {
    if key.code == KeyCode::Esc {
        state.focus_mode = FocusMode::Normal;
        return KeyOutcome::Handled;
    }

    match state.focus_mode {
        FocusMode::Input => {
            capture_command_text(state, key);
            KeyOutcome::Handled
        }
        FocusMode::Overlay => {
            // Tab/arrows cycle dialog fields once a dialog has fields to
            // cycle through; Phase 5 has no overlay dialogs wired up yet
            // (only `?` opens an as-yet-content-free Help overlay).
            KeyOutcome::Handled
        }
        FocusMode::Normal => {
            if let Some(outcome) = try_global_shortcut(state, key) {
                return outcome;
            }
            dispatch_to_pane(key)
        }
    }
}

fn try_global_shortcut(state: &mut WorkspaceState, key: KeyEvent) -> Option<KeyOutcome> {
    match (key.code, key.modifiers) {
        (KeyCode::Char('q'), m) if m.contains(KeyModifiers::CONTROL) => {
            state.should_quit = true;
            Some(KeyOutcome::Handled)
        }
        (KeyCode::Char(':'), _) => {
            state.focus_mode = FocusMode::Input;
            Some(KeyOutcome::Handled)
        }
        (KeyCode::Char('?'), _) => {
            state.focus_mode = FocusMode::Overlay;
            Some(KeyOutcome::Handled)
        }
        (KeyCode::Tab, _) => {
            focus_dock(state, 1);
            Some(KeyOutcome::Handled)
        }
        (KeyCode::BackTab, _) => {
            focus_dock(state, -1);
            Some(KeyOutcome::Handled)
        }
        (KeyCode::Char('1' | 't'), m) if m.contains(KeyModifiers::CONTROL) => {
            set_focused_dock(state, UiDockSlot::Left);
            Some(KeyOutcome::Handled)
        }
        (KeyCode::Char('2' | 'n'), m) if m.contains(KeyModifiers::CONTROL) => {
            set_focused_dock(state, UiDockSlot::Center);
            Some(KeyOutcome::Handled)
        }
        (KeyCode::Char('3' | 'd'), m) if m.contains(KeyModifiers::CONTROL) => {
            set_focused_dock(state, UiDockSlot::Right);
            Some(KeyOutcome::Handled)
        }
        (KeyCode::Char('4' | 'c'), m) if m.contains(KeyModifiers::CONTROL) => {
            set_focused_dock(state, UiDockSlot::Bottom);
            Some(KeyOutcome::Handled)
        }
        _ => None,
    }
}

fn set_focused_dock(state: &mut WorkspaceState, slot: UiDockSlot) {
    state.focused_dock = slot;
    state.selected_index = 0;
}

fn focus_dock(state: &mut WorkspaceState, step: i32) {
    let idx = DOCK_CYCLE
        .iter()
        .position(|d| *d == state.focused_dock)
        .unwrap_or(0) as i32;
    let len = DOCK_CYCLE.len() as i32;
    let next = ((idx + step).rem_euclid(len)) as usize;
    set_focused_dock(state, DOCK_CYCLE[next]);
}

fn dispatch_to_pane(key: KeyEvent) -> KeyOutcome {
    match key.code {
        KeyCode::Char('k') | KeyCode::Up => KeyOutcome::DispatchToPane(PaneAction::Up),
        KeyCode::Char('j') | KeyCode::Down => KeyOutcome::DispatchToPane(PaneAction::Down),
        KeyCode::Char('h') | KeyCode::Left => KeyOutcome::DispatchToPane(PaneAction::Left),
        KeyCode::Char('l') | KeyCode::Right => KeyOutcome::DispatchToPane(PaneAction::Right),
        KeyCode::Enter => KeyOutcome::DispatchToPane(PaneAction::Activate),
        _ => KeyOutcome::Ignored,
    }
}

fn capture_command_text(state: &mut WorkspaceState, key: KeyEvent) {
    let buf = &mut state.cmd_buffer;
    match key.code {
        KeyCode::Char(c) => {
            buf.raw_text.insert(buf.cursor_position, c);
            buf.cursor_position += c.len_utf8();
        }
        KeyCode::Backspace => {
            if let Some(prev) = buf.raw_text[..buf.cursor_position].chars().next_back() {
                let new_pos = buf.cursor_position - prev.len_utf8();
                buf.raw_text.remove(new_pos);
                buf.cursor_position = new_pos;
            }
        }
        KeyCode::Enter => {
            // Command parsing/dispatch (`/slack-send` etc.) is deferred —
            // no integration exists yet to act on it (step5.md scope note).
            if !buf.raw_text.is_empty() {
                buf.history.push(std::mem::take(&mut buf.raw_text));
                buf.cursor_position = 0;
                buf.history_index = None;
            }
        }
        KeyCode::Left => buf.cursor_position = buf.cursor_position.saturating_sub(1),
        KeyCode::Right => buf.cursor_position = (buf.cursor_position + 1).min(buf.raw_text.len()),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEventKind;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new_with_kind(code, modifiers, KeyEventKind::Press)
    }

    #[test]
    fn colon_enters_input_mode() {
        let mut state = WorkspaceState::default();
        let outcome = handle_key(&mut state, key(KeyCode::Char(':'), KeyModifiers::NONE));
        assert_eq!(outcome, KeyOutcome::Handled);
        assert_eq!(state.focus_mode, FocusMode::Input);
    }

    #[test]
    fn esc_always_returns_to_normal_mode() {
        let mut state = WorkspaceState {
            focus_mode: FocusMode::Input,
            ..Default::default()
        };
        let outcome = handle_key(&mut state, key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(outcome, KeyOutcome::Handled);
        assert_eq!(state.focus_mode, FocusMode::Normal);
    }

    #[test]
    fn ctrl_q_sets_should_quit() {
        let mut state = WorkspaceState::default();
        handle_key(&mut state, key(KeyCode::Char('q'), KeyModifiers::CONTROL));
        assert!(state.should_quit);
    }

    #[test]
    fn tab_cycles_focus_through_all_four_docks_and_wraps() {
        let mut state = WorkspaceState::default();
        assert_eq!(state.focused_dock, UiDockSlot::Left);
        handle_key(&mut state, key(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(state.focused_dock, UiDockSlot::Center);
        handle_key(&mut state, key(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(state.focused_dock, UiDockSlot::Right);
        handle_key(&mut state, key(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(state.focused_dock, UiDockSlot::Bottom);
        handle_key(&mut state, key(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(state.focused_dock, UiDockSlot::Left);
    }

    #[test]
    fn shift_tab_cycles_backward() {
        let mut state = WorkspaceState::default();
        handle_key(&mut state, key(KeyCode::BackTab, KeyModifiers::NONE));
        assert_eq!(state.focused_dock, UiDockSlot::Bottom);
    }

    #[test]
    fn ctrl_2_focuses_notification_dock_directly() {
        let mut state = WorkspaceState::default();
        handle_key(&mut state, key(KeyCode::Char('2'), KeyModifiers::CONTROL));
        assert_eq!(state.focused_dock, UiDockSlot::Center);
    }

    #[test]
    fn global_shortcut_takes_precedence_over_pane_dispatch() {
        // ':' is both a printable char a pane *could* theoretically care
        // about and a global shortcut — global must win (keyboard.md's
        // explicit precedence rule).
        let mut state = WorkspaceState::default();
        let outcome = handle_key(&mut state, key(KeyCode::Char(':'), KeyModifiers::NONE));
        assert_eq!(outcome, KeyOutcome::Handled);
        assert_ne!(outcome, KeyOutcome::DispatchToPane(PaneAction::Activate));
    }

    #[test]
    fn unmapped_key_in_normal_mode_falls_through_to_pane() {
        let mut state = WorkspaceState::default();
        let outcome = handle_key(&mut state, key(KeyCode::Char('j'), KeyModifiers::NONE));
        assert_eq!(outcome, KeyOutcome::DispatchToPane(PaneAction::Down));
    }

    #[test]
    fn input_mode_captures_text_and_moves_cursor() {
        let mut state = WorkspaceState {
            focus_mode: FocusMode::Input,
            ..Default::default()
        };
        handle_key(&mut state, key(KeyCode::Char('a'), KeyModifiers::NONE));
        handle_key(&mut state, key(KeyCode::Char('b'), KeyModifiers::NONE));
        assert_eq!(state.cmd_buffer.raw_text, "ab");
        assert_eq!(state.cmd_buffer.cursor_position, 2);
    }

    #[test]
    fn input_mode_enter_pushes_history_and_clears_buffer() {
        let mut state = WorkspaceState {
            focus_mode: FocusMode::Input,
            ..Default::default()
        };
        handle_key(&mut state, key(KeyCode::Char('x'), KeyModifiers::NONE));
        handle_key(&mut state, key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(state.cmd_buffer.history, vec!["x".to_string()]);
        assert_eq!(state.cmd_buffer.raw_text, "");
    }

    #[test]
    fn input_mode_backspace_removes_previous_char() {
        let mut state = WorkspaceState {
            focus_mode: FocusMode::Input,
            ..Default::default()
        };
        handle_key(&mut state, key(KeyCode::Char('a'), KeyModifiers::NONE));
        handle_key(&mut state, key(KeyCode::Char('b'), KeyModifiers::NONE));
        handle_key(&mut state, key(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(state.cmd_buffer.raw_text, "a");
    }
}
