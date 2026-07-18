//! Modal input capture pipeline. See `docs/02-architecture/keyboard.md` —
//! this is a direct implementation of that document's capture pipeline
//! diagram, not a new design.

use crate::state::{
    FocusMode, OverlayKind, SlackPickerState, SlackPickerStatus, SlackSetupStatus, WorkspaceState,
};
use commands::Command;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use domain::PresenceStatus;
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyOutcome {
    /// Consumed internally (mode switch, global shortcut, text input).
    Handled,
    /// Not a global shortcut in Normal mode; forward to the focused pane.
    DispatchToPane(PaneAction),
    /// `Enter` pressed in the Slack setup overlay with a non-empty token —
    /// the caller (`crates/ui`'s async event loop) dispatches
    /// `Command::ConnectSlack` with this value; `handle_key` itself stays
    /// synchronous and can't perform that I/O.
    SubmitSlackToken(String),
    /// `Ctrl+P` pressed — the caller opens the picker overlay and fetches
    /// channel/user lists (`SlackPicker::list_channels`/`list_users`, both
    /// network I/O `handle_key` can't perform synchronously).
    OpenSlackPicker,
    /// `Enter` pressed in the picker overlay — `(channel_ids, watched_user_ids)`
    /// of the currently checked rows; the caller dispatches
    /// `Command::ApplySlackSelection` with them.
    SubmitSlackSelection(Vec<String>, Vec<String>),
    /// `Enter` pressed in the command bar and the line parsed to a real
    /// command (`/send`, `/away`, ...) — the caller dispatches it through
    /// `CommandDispatcher` (`step9.md`).
    SubmitCommand(Command),
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
        FocusMode::Input => match capture_command_text(state, key) {
            Some(command) => KeyOutcome::SubmitCommand(command),
            None => KeyOutcome::Handled,
        },
        FocusMode::Overlay => match state.active_overlay {
            OverlayKind::Help => KeyOutcome::Handled,
            OverlayKind::SlackSetup => capture_slack_setup_input(state, key),
            OverlayKind::SlackPicker => capture_slack_picker_input(state, key),
        },
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
            state.active_overlay = OverlayKind::Help;
            Some(KeyOutcome::Handled)
        }
        (KeyCode::Char('s'), m) if m.contains(KeyModifiers::CONTROL) => {
            state.focus_mode = FocusMode::Overlay;
            state.active_overlay = OverlayKind::SlackSetup;
            state.slack_setup.status = SlackSetupStatus::Idle;
            Some(KeyOutcome::Handled)
        }
        (KeyCode::Char('p'), m) if m.contains(KeyModifiers::CONTROL) => {
            state.focus_mode = FocusMode::Overlay;
            state.active_overlay = OverlayKind::SlackPicker;
            state.slack_picker.status = SlackPickerStatus::Loading;
            Some(KeyOutcome::OpenSlackPicker)
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

/// Text capture for the Slack setup overlay's token field. Deliberately
/// simpler than `capture_command_text`'s cursor-aware editing — a Bot
/// Token is typically pasted or typed once in order, not edited mid-string,
/// so append/backspace-from-the-end is enough and avoids duplicating the
/// cursor-position bookkeeping for a field that doesn't need it.
fn capture_slack_setup_input(state: &mut WorkspaceState, key: KeyEvent) -> KeyOutcome {
    let setup = &mut state.slack_setup;
    match key.code {
        KeyCode::Char(c) => {
            setup.token_input.push(c);
            KeyOutcome::Handled
        }
        KeyCode::Backspace => {
            setup.token_input.pop();
            KeyOutcome::Handled
        }
        KeyCode::Enter if !setup.token_input.is_empty() => {
            let token = std::mem::take(&mut setup.token_input);
            setup.status = SlackSetupStatus::Connecting;
            KeyOutcome::SubmitSlackToken(token)
        }
        _ => KeyOutcome::Handled,
    }
}

/// Navigation + selection for the picker overlay (`step8.md`). `cursor`
/// indexes into the combined `channels` then `users` list — `j`/`k` move
/// it, `Space` toggles the row it's on, `Enter` confirms.
fn capture_slack_picker_input(state: &mut WorkspaceState, key: KeyEvent) -> KeyOutcome {
    let picker = &mut state.slack_picker;
    let total = picker.channels.len() + picker.users.len();
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            if total > 0 {
                picker.cursor = (picker.cursor + 1).min(total - 1);
            }
            KeyOutcome::Handled
        }
        KeyCode::Char('k') | KeyCode::Up => {
            picker.cursor = picker.cursor.saturating_sub(1);
            KeyOutcome::Handled
        }
        KeyCode::Char(' ') => {
            let cursor = picker.cursor;
            if cursor < picker.channels.len() {
                if let Some(row) = picker.channels.get_mut(cursor) {
                    row.selected = !row.selected;
                }
            } else if let Some(row) = picker.users.get_mut(cursor - picker.channels.len()) {
                row.selected = !row.selected;
            }
            KeyOutcome::Handled
        }
        KeyCode::Enter => {
            let channel_ids = picker
                .channels
                .iter()
                .filter(|r| r.selected)
                .map(|r| r.id.clone())
                .collect();
            let watched_user_ids = picker
                .users
                .iter()
                .filter(|r| r.selected)
                .map(|r| r.id.clone())
                .collect();
            picker.status = SlackPickerStatus::Saving;
            KeyOutcome::SubmitSlackSelection(channel_ids, watched_user_ids)
        }
        _ => KeyOutcome::Handled,
    }
}

/// Returns `Some(command)` when `Enter` submitted a line that parsed to a
/// real command (`step9.md`) — the caller (async event loop) dispatches
/// it. Every other key, and lines that don't parse to a command, return
/// `None`: plain chat-style typing still just accumulates in the buffer
/// and lands in `history` on `Enter`, exactly as before this phase.
fn capture_command_text(state: &mut WorkspaceState, key: KeyEvent) -> Option<Command> {
    match key.code {
        KeyCode::Char(c) => {
            let buf = &mut state.cmd_buffer;
            buf.raw_text.insert(buf.cursor_position, c);
            buf.cursor_position += c.len_utf8();
            None
        }
        KeyCode::Backspace => {
            let buf = &mut state.cmd_buffer;
            if let Some(prev) = buf.raw_text[..buf.cursor_position].chars().next_back() {
                let new_pos = buf.cursor_position - prev.len_utf8();
                buf.raw_text.remove(new_pos);
                buf.cursor_position = new_pos;
            }
            None
        }
        KeyCode::Enter => {
            if state.cmd_buffer.raw_text.is_empty() {
                return None;
            }
            let text = std::mem::take(&mut state.cmd_buffer.raw_text);
            state.cmd_buffer.cursor_position = 0;
            state.cmd_buffer.history_index = None;

            let result = parse_command(&text, &state.slack_picker);
            state.cmd_buffer.history.push(text);
            match result {
                Ok(command) => {
                    state.cmd_buffer.last_error = None;
                    command
                }
                Err(message) => {
                    state.cmd_buffer.last_error = Some(message);
                    None
                }
            }
        }
        KeyCode::Left => {
            state.cmd_buffer.cursor_position = state.cmd_buffer.cursor_position.saturating_sub(1);
            None
        }
        KeyCode::Right => {
            state.cmd_buffer.cursor_position =
                (state.cmd_buffer.cursor_position + 1).min(state.cmd_buffer.raw_text.len());
            None
        }
        _ => None,
    }
}

/// `Ok(None)`: plain text, not a recognized command prefix — no error, not
/// dispatched. `Ok(Some(_))`: parsed successfully. `Err(_)`: looked like a
/// deliberate command attempt (leading `/`) but failed to parse or resolve
/// — surfaced to the user (`state.cmd_buffer.last_error`) rather than
/// silently doing nothing, unlike plain chat-style text.
fn parse_command(text: &str, picker: &SlackPickerState) -> Result<Option<Command>, String> {
    let mut top = text.splitn(2, ' ');
    let head = top.next().unwrap_or("");
    let rest = top.next().unwrap_or("").trim();

    match head {
        "/send" => {
            let mut args = rest.splitn(2, ' ');
            let target = args.next().unwrap_or("").trim();
            let message = args.next().unwrap_or("").trim();
            if target.is_empty() || message.is_empty() {
                return Err("사용법: /send #채널이름 메시지".to_string());
            }
            let channel_id = resolve_channel_id(target, picker).ok_or_else(|| {
                format!(
                    "'{target}' 채널을 찾을 수 없습니다 — 먼저 Ctrl+P로 채널 목록을 불러와주세요."
                )
            })?;
            Ok(Some(Command::SendSlackMessage {
                channel_id,
                text: message.to_string(),
            }))
        }
        "/away" => Ok(Some(presence_command(PresenceStatus::Away, rest))),
        "/active" => Ok(Some(presence_command(PresenceStatus::Active, rest))),
        "/offline" => Ok(Some(presence_command(PresenceStatus::Offline, rest))),
        "/meeting" => Ok(Some(presence_command(PresenceStatus::Meeting, rest))),
        "/lunch" => Ok(Some(presence_command(PresenceStatus::Lunch, rest))),
        _ => Ok(None),
    }
}

fn presence_command(status: PresenceStatus, custom_text: &str) -> Command {
    Command::SetPresence {
        status,
        custom_text: if custom_text.is_empty() {
            None
        } else {
            Some(custom_text.to_string())
        },
    }
}

/// Resolves `#name` (or bare `name`) against the channels the `Ctrl+P`
/// picker last fetched into `WorkspaceState.slack_picker.channels` — see
/// `step9.md` Decision 2 for why this reuses that cache instead of a
/// second lookup path.
fn resolve_channel_id(target: &str, picker: &SlackPickerState) -> Option<String> {
    let name = target.strip_prefix('#').unwrap_or(target);
    picker
        .channels
        .iter()
        .find(|c| c.label.eq_ignore_ascii_case(name))
        .map(|c| c.id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{PickerRow, SlackPickerState};
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

    fn type_and_submit(state: &mut WorkspaceState, text: &str) -> KeyOutcome {
        for c in text.chars() {
            handle_key(state, key(KeyCode::Char(c), KeyModifiers::NONE));
        }
        handle_key(state, key(KeyCode::Enter, KeyModifiers::NONE))
    }

    fn state_with_general_channel() -> WorkspaceState {
        WorkspaceState {
            focus_mode: FocusMode::Input,
            slack_picker: SlackPickerState {
                channels: vec![PickerRow {
                    id: "C0GENERAL".into(),
                    label: "general".into(),
                    selected: false,
                }],
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn send_with_a_known_channel_name_dispatches_send_slack_message() {
        let mut state = state_with_general_channel();
        let outcome = type_and_submit(&mut state, "/send #general hi there");
        assert_eq!(
            outcome,
            KeyOutcome::SubmitCommand(Command::SendSlackMessage {
                channel_id: "C0GENERAL".to_string(),
                text: "hi there".to_string(),
            })
        );
        assert!(state.cmd_buffer.last_error.is_none());
    }

    #[test]
    fn send_resolves_channel_name_case_insensitively_and_without_the_hash() {
        let mut state = state_with_general_channel();
        let outcome = type_and_submit(&mut state, "/send General hi");
        assert_eq!(
            outcome,
            KeyOutcome::SubmitCommand(Command::SendSlackMessage {
                channel_id: "C0GENERAL".to_string(),
                text: "hi".to_string(),
            })
        );
    }

    #[test]
    fn send_with_an_unknown_channel_name_is_not_dispatched_and_sets_an_error() {
        let mut state = state_with_general_channel();
        let outcome = type_and_submit(&mut state, "/send #nope hi");
        assert_eq!(outcome, KeyOutcome::Handled);
        assert!(state.cmd_buffer.last_error.is_some());
        // Still recorded in history like any other submitted line.
        assert_eq!(state.cmd_buffer.history, vec!["/send #nope hi".to_string()]);
    }

    #[test]
    fn away_with_no_text_sets_presence_with_no_custom_text() {
        let mut state = WorkspaceState {
            focus_mode: FocusMode::Input,
            ..Default::default()
        };
        let outcome = type_and_submit(&mut state, "/away");
        assert_eq!(
            outcome,
            KeyOutcome::SubmitCommand(Command::SetPresence {
                status: PresenceStatus::Away,
                custom_text: None,
            })
        );
    }

    #[test]
    fn away_with_trailing_text_sets_it_as_custom_status() {
        let mut state = WorkspaceState {
            focus_mode: FocusMode::Input,
            ..Default::default()
        };
        let outcome = type_and_submit(&mut state, "/away brb 10 min");
        assert_eq!(
            outcome,
            KeyOutcome::SubmitCommand(Command::SetPresence {
                status: PresenceStatus::Away,
                custom_text: Some("brb 10 min".to_string()),
            })
        );
    }

    #[test]
    fn plain_text_with_no_leading_slash_is_unchanged_from_before_this_phase() {
        let mut state = WorkspaceState {
            focus_mode: FocusMode::Input,
            ..Default::default()
        };
        let outcome = type_and_submit(&mut state, "hey team, on it");
        assert_eq!(outcome, KeyOutcome::Handled);
        assert!(state.cmd_buffer.last_error.is_none());
        assert_eq!(
            state.cmd_buffer.history,
            vec!["hey team, on it".to_string()]
        );
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

    #[test]
    fn ctrl_s_opens_the_slack_setup_overlay() {
        let mut state = WorkspaceState::default();
        let outcome = handle_key(&mut state, key(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert_eq!(outcome, KeyOutcome::Handled);
        assert_eq!(state.focus_mode, FocusMode::Overlay);
        assert_eq!(state.active_overlay, OverlayKind::SlackSetup);
    }

    #[test]
    fn slack_setup_overlay_captures_typed_characters_masked_in_render() {
        let mut state = WorkspaceState {
            focus_mode: FocusMode::Overlay,
            active_overlay: OverlayKind::SlackSetup,
            ..Default::default()
        };
        handle_key(&mut state, key(KeyCode::Char('x'), KeyModifiers::NONE));
        handle_key(&mut state, key(KeyCode::Char('o'), KeyModifiers::NONE));
        assert_eq!(state.slack_setup.token_input, "xo");
    }

    #[test]
    fn slack_setup_overlay_backspace_removes_last_char() {
        let mut state = WorkspaceState {
            focus_mode: FocusMode::Overlay,
            active_overlay: OverlayKind::SlackSetup,
            ..Default::default()
        };
        handle_key(&mut state, key(KeyCode::Char('x'), KeyModifiers::NONE));
        handle_key(&mut state, key(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(state.slack_setup.token_input, "");
    }

    #[test]
    fn enter_with_a_token_submits_and_clears_the_input() {
        let mut state = WorkspaceState {
            focus_mode: FocusMode::Overlay,
            active_overlay: OverlayKind::SlackSetup,
            ..Default::default()
        };
        handle_key(&mut state, key(KeyCode::Char('x'), KeyModifiers::NONE));
        let outcome = handle_key(&mut state, key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(outcome, KeyOutcome::SubmitSlackToken("x".to_string()));
        assert_eq!(state.slack_setup.token_input, "");
        assert_eq!(state.slack_setup.status, SlackSetupStatus::Connecting);
    }

    #[test]
    fn enter_with_no_token_does_not_submit() {
        let mut state = WorkspaceState {
            focus_mode: FocusMode::Overlay,
            active_overlay: OverlayKind::SlackSetup,
            ..Default::default()
        };
        let outcome = handle_key(&mut state, key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(outcome, KeyOutcome::Handled);
    }

    #[test]
    fn esc_closes_the_slack_setup_overlay_like_any_other_overlay() {
        let mut state = WorkspaceState {
            focus_mode: FocusMode::Overlay,
            active_overlay: OverlayKind::SlackSetup,
            ..Default::default()
        };
        handle_key(&mut state, key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(state.focus_mode, FocusMode::Normal);
    }

    #[test]
    fn ctrl_p_opens_the_slack_picker_overlay() {
        let mut state = WorkspaceState::default();
        let outcome = handle_key(&mut state, key(KeyCode::Char('p'), KeyModifiers::CONTROL));
        assert_eq!(outcome, KeyOutcome::OpenSlackPicker);
        assert_eq!(state.focus_mode, FocusMode::Overlay);
        assert_eq!(state.active_overlay, OverlayKind::SlackPicker);
    }

    fn picker_state_with_two_channels_one_user() -> SlackPickerState {
        SlackPickerState {
            channels: vec![
                PickerRow {
                    id: "C1".into(),
                    label: "general".into(),
                    selected: false,
                },
                PickerRow {
                    id: "C2".into(),
                    label: "random".into(),
                    selected: false,
                },
            ],
            users: vec![PickerRow {
                id: "U1".into(),
                label: "Alice".into(),
                selected: false,
            }],
            cursor: 0,
            ..Default::default()
        }
    }

    #[test]
    fn slack_picker_space_toggles_the_row_under_the_cursor() {
        let mut state = WorkspaceState {
            focus_mode: FocusMode::Overlay,
            active_overlay: OverlayKind::SlackPicker,
            slack_picker: picker_state_with_two_channels_one_user(),
            ..Default::default()
        };
        handle_key(&mut state, key(KeyCode::Char(' '), KeyModifiers::NONE));
        assert!(state.slack_picker.channels[0].selected);
        assert!(!state.slack_picker.channels[1].selected);
    }

    #[test]
    fn slack_picker_j_moves_the_cursor_into_the_user_section() {
        let mut state = WorkspaceState {
            focus_mode: FocusMode::Overlay,
            active_overlay: OverlayKind::SlackPicker,
            slack_picker: picker_state_with_two_channels_one_user(),
            ..Default::default()
        };
        // 3 rows total (2 channels + 1 user), cursor starts at 0.
        handle_key(&mut state, key(KeyCode::Char('j'), KeyModifiers::NONE));
        handle_key(&mut state, key(KeyCode::Char('j'), KeyModifiers::NONE));
        assert_eq!(state.slack_picker.cursor, 2);
        // One more 'j' must not run past the last row.
        handle_key(&mut state, key(KeyCode::Char('j'), KeyModifiers::NONE));
        assert_eq!(state.slack_picker.cursor, 2);

        handle_key(&mut state, key(KeyCode::Char(' '), KeyModifiers::NONE));
        assert!(state.slack_picker.users[0].selected);
    }

    #[test]
    fn slack_picker_enter_submits_only_the_selected_ids() {
        let mut picker = picker_state_with_two_channels_one_user();
        picker.channels[1].selected = true; // "random", not "general"
        picker.users[0].selected = true; // "Alice"
        let mut state = WorkspaceState {
            focus_mode: FocusMode::Overlay,
            active_overlay: OverlayKind::SlackPicker,
            slack_picker: picker,
            ..Default::default()
        };
        let outcome = handle_key(&mut state, key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(
            outcome,
            KeyOutcome::SubmitSlackSelection(vec!["C2".to_string()], vec!["U1".to_string()])
        );
    }
}
