//! Modal input capture pipeline. See `docs/02-architecture/keyboard.md` —
//! this is a direct implementation of that document's capture pipeline
//! diagram, not a new design.

use crate::state::{
    CalendarSetupStatus, FocusMode, GitHubPickerStatus, GitHubSetupStatus, OverlayKind,
    SlackPickerState, SlackPickerStatus, SlackSetupStatus, WorkspaceState,
};
use commands::Command;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use domain::{IntegrationSource, PresenceStatus};
use registry::UiDockSlot;
use scheduler::{DEFAULT_BREAK_MINUTES, DEFAULT_WORK_MINUTES};

/// Every recognized command head, in the same order `parse_command`
/// matches them — the single source of truth `compute_suggestions`
/// (`step13.md`) filters against, so the two can't drift apart.
const COMMAND_HEADS: &[&str] = &[
    "/send",
    "/away",
    "/active",
    "/offline",
    "/meeting",
    "/lunch",
    "/pomodoro",
];

/// Fixed focus-cycle order for `Tab`/`Shift+Tab` (`keyboard.md`'s "Cycles
/// focus clockwise/counter-clockwise through visible layout panes"). Only
/// the three panels with an actual visible body panel participate --
/// `UiDockSlot::Bottom` (Log) dropped out in `step19.md`: it's no longer a
/// focusable dock at all, `Ctrl+4` opens the Log Viewer overlay directly
/// instead.
const DOCK_CYCLE: [UiDockSlot; 3] = [UiDockSlot::Left, UiDockSlot::Center, UiDockSlot::Right];

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
    /// `Enter` pressed in a setup overlay (Slack's `Ctrl+S` or GitHub's
    /// `Ctrl+G`) with a non-empty token — the caller (`crates/ui`'s async
    /// event loop) dispatches `Command::Connect{source, token}`;
    /// `handle_key` itself stays synchronous and can't perform that I/O.
    /// Generalized in `step11.md` from separate `SubmitSlackToken`/
    /// `SubmitGitHubToken` variants — the connect flow is identical shape
    /// for every integration built so far, including Slack.
    SubmitToken(IntegrationSource, String),
    /// `Ctrl+P` pressed — the caller opens the Slack picker overlay and
    /// fetches channel/user lists (`SlackPicker::list_channels`/
    /// `list_users`, both network I/O `handle_key` can't perform
    /// synchronously). Slack's two independent lists don't fit the generic
    /// `OpenPicker` below.
    OpenSlackPicker,
    /// `Enter` pressed in the Slack picker overlay — `(channel_ids,
    /// watched_user_ids)` of the currently checked rows; the caller
    /// dispatches `Command::ApplySlackSelection` with them.
    SubmitSlackSelection(Vec<String>, Vec<String>),
    /// `Ctrl+R` (GitHub) or a future single-list integration's picker
    /// shortcut — the caller opens that integration's picker overlay and
    /// fetches its item list (`Picker::list_items`, network I/O). Generalized
    /// in `step11.md` from `OpenGitHubPicker`.
    OpenPicker(IntegrationSource),
    /// `Enter` pressed in a single-list picker overlay — the checked rows'
    /// ids; the caller dispatches `Command::ApplySelection{source, items}`.
    /// Generalized in `step11.md` from `SubmitGitHubSelection`.
    SubmitSelection(IntegrationSource, Vec<String>),
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
            // Pure view, no input fields -- same as Help. Closed via Esc
            // (handled unconditionally above this match), not any key here.
            OverlayKind::Help | OverlayKind::LogViewer => KeyOutcome::Handled,
            OverlayKind::SlackSetup => capture_slack_setup_input(state, key),
            OverlayKind::SlackPicker => capture_slack_picker_input(state, key),
            OverlayKind::GitHubSetup => capture_github_setup_input(state, key),
            OverlayKind::GitHubPicker => capture_github_picker_input(state, key),
            OverlayKind::CalendarSetup => capture_calendar_setup_input(state, key),
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
            // Clears whatever was typed before an earlier Esc, not just
            // `status` -- otherwise a fresh paste appends onto leftover
            // text from a prior attempt instead of replacing it, producing
            // a garbled token/URL with no visible sign anything is wrong
            // (real bug found via a live Calendar connection failure).
            state.slack_setup.token_input.clear();
            state.slack_setup.status = SlackSetupStatus::Idle;
            Some(KeyOutcome::Handled)
        }
        (KeyCode::Char('p'), m) if m.contains(KeyModifiers::CONTROL) => {
            state.focus_mode = FocusMode::Overlay;
            state.active_overlay = OverlayKind::SlackPicker;
            state.slack_picker.status = SlackPickerStatus::Loading;
            Some(KeyOutcome::OpenSlackPicker)
        }
        (KeyCode::Char('g'), m) if m.contains(KeyModifiers::CONTROL) => {
            state.focus_mode = FocusMode::Overlay;
            state.active_overlay = OverlayKind::GitHubSetup;
            // See the matching comment on Ctrl+S above -- same bug, same fix.
            state.github_setup.token_input.clear();
            state.github_setup.status = GitHubSetupStatus::Idle;
            Some(KeyOutcome::Handled)
        }
        (KeyCode::Char('r'), m) if m.contains(KeyModifiers::CONTROL) => {
            state.focus_mode = FocusMode::Overlay;
            state.active_overlay = OverlayKind::GitHubPicker;
            state.github_picker.status = GitHubPickerStatus::Loading;
            Some(KeyOutcome::OpenPicker(IntegrationSource::GitHub))
        }
        (KeyCode::Char('l'), m) if m.contains(KeyModifiers::CONTROL) => {
            state.focus_mode = FocusMode::Overlay;
            state.active_overlay = OverlayKind::CalendarSetup;
            // See the matching comment on Ctrl+S above -- same bug, same
            // fix (this is the one a live Calendar connection failure
            // actually traced back to).
            state.calendar_setup.token_input.clear();
            state.calendar_setup.status = CalendarSetupStatus::Idle;
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
        // Opens the Log Viewer overlay directly (`step19.md`) -- unlike
        // Ctrl+1~3, this isn't a "focus a dock" shortcut anymore. The
        // Bottom dock's persistent 1-line strip showed too little to be
        // useful; a full scrollback overlay (mirroring Ctrl+S/Ctrl+G/
        // Ctrl+L's "open directly" pattern) replaced it.
        (KeyCode::Char('4' | 'c'), m) if m.contains(KeyModifiers::CONTROL) => {
            state.focus_mode = FocusMode::Overlay;
            state.active_overlay = OverlayKind::LogViewer;
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
            KeyOutcome::SubmitToken(IntegrationSource::Slack, token)
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

/// Text capture for the GitHub setup overlay's token field. Mirrors
/// `capture_slack_setup_input` exactly (`step10.md`).
fn capture_github_setup_input(state: &mut WorkspaceState, key: KeyEvent) -> KeyOutcome {
    let setup = &mut state.github_setup;
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
            setup.status = GitHubSetupStatus::Connecting;
            KeyOutcome::SubmitToken(IntegrationSource::GitHub, token)
        }
        _ => KeyOutcome::Handled,
    }
}

/// Text capture for the Calendar setup overlay's secret-URL field. Mirrors
/// `capture_github_setup_input`/`capture_slack_setup_input` exactly
/// (`step12.md`) — the field holds a URL instead of a short token, but the
/// capture semantics (append/backspace-from-the-end, submit on non-empty
/// `Enter`) don't care about that distinction.
fn capture_calendar_setup_input(state: &mut WorkspaceState, key: KeyEvent) -> KeyOutcome {
    let setup = &mut state.calendar_setup;
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
            setup.status = CalendarSetupStatus::Connecting;
            KeyOutcome::SubmitToken(IntegrationSource::Calendar, token)
        }
        _ => KeyOutcome::Handled,
    }
}

/// Navigation + selection for the GitHub repo picker overlay (`step10.md`).
/// Simpler than `capture_slack_picker_input`: one list, not a combined
/// channels-then-users index space, so there's no split-index arithmetic to
/// share between them — a forced common helper would trade ~10 straight-line
/// lines for an abstraction with only one real shape on each side.
fn capture_github_picker_input(state: &mut WorkspaceState, key: KeyEvent) -> KeyOutcome {
    let picker = &mut state.github_picker;
    let total = picker.repositories.len();
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
            if let Some(row) = picker.repositories.get_mut(picker.cursor) {
                row.selected = !row.selected;
            }
            KeyOutcome::Handled
        }
        KeyCode::Enter => {
            let repositories = picker
                .repositories
                .iter()
                .filter(|r| r.selected)
                .map(|r| r.id.clone())
                .collect();
            picker.status = GitHubPickerStatus::Saving;
            KeyOutcome::SubmitSelection(IntegrationSource::GitHub, repositories)
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
            refresh_suggestions(state);
            None
        }
        KeyCode::Backspace => {
            let buf = &mut state.cmd_buffer;
            if let Some(prev) = buf.raw_text[..buf.cursor_position].chars().next_back() {
                let new_pos = buf.cursor_position - prev.len_utf8();
                buf.raw_text.remove(new_pos);
                buf.cursor_position = new_pos;
            }
            refresh_suggestions(state);
            None
        }
        KeyCode::Tab => {
            apply_next_suggestion(state);
            None
        }
        KeyCode::Enter => {
            if state.cmd_buffer.raw_text.is_empty() {
                return None;
            }
            let text = std::mem::take(&mut state.cmd_buffer.raw_text);
            state.cmd_buffer.cursor_position = 0;
            state.cmd_buffer.history_index = None;
            state.cmd_buffer.autocomplete_suggestions = Vec::new();
            state.cmd_buffer.selected_suggestion_index = None;

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

/// Recomputes `autocomplete_suggestions` fresh from the current text/cursor
/// (`step13.md`) — called after every edit so candidates never go stale,
/// and resets `selected_suggestion_index` so a fresh `Tab` always starts
/// cycling from the first candidate rather than wherever a previous,
/// now-irrelevant cycle left off.
fn refresh_suggestions(state: &mut WorkspaceState) {
    state.cmd_buffer.autocomplete_suggestions = compute_suggestions(
        &state.cmd_buffer.raw_text,
        state.cmd_buffer.cursor_position,
        &state.slack_picker,
    );
    state.cmd_buffer.selected_suggestion_index = None;
}

/// `Tab`: advances to the next candidate (wrapping) and splices it into
/// `raw_text` at the current word's boundaries. Deliberately does **not**
/// recompute `autocomplete_suggestions` from the post-splice text — once
/// the word has been replaced with a full candidate (e.g. `/a` → `/active`),
/// re-deriving candidates from `/active` would only ever match itself,
/// silently breaking cycling to `/away`. The candidate list is frozen from
/// the last real edit; only the cursor/replacement position is
/// recalculated each press (`word_start` is safe to call against
/// already-completed text, since none of the candidates contain spaces —
/// the word boundary itself doesn't move just because the word's content
/// changed).
fn apply_next_suggestion(state: &mut WorkspaceState) {
    let buf = &mut state.cmd_buffer;
    if buf.autocomplete_suggestions.is_empty() {
        return;
    }
    let next_index = match buf.selected_suggestion_index {
        Some(i) => (i + 1) % buf.autocomplete_suggestions.len(),
        None => 0,
    };
    buf.selected_suggestion_index = Some(next_index);

    let start = word_start(&buf.raw_text, buf.cursor_position);
    let replacement = buf.autocomplete_suggestions[next_index].clone();
    buf.raw_text
        .replace_range(start..buf.cursor_position, &replacement);
    buf.cursor_position = start + replacement.len();
}

/// The byte offset where the word under/before `cursor` begins — the last
/// space before `cursor`, or `0`. Pure and cheap enough to call on every
/// `Tab` press rather than caching it (`step13.md`).
fn word_start(text: &str, cursor: usize) -> usize {
    text[..cursor].rfind(' ').map_or(0, |i| i + 1)
}

/// Completion candidates for the word ending at `cursor`, or `[]` if that
/// word isn't in a completable position. Two modes (`step13.md` Decision
/// 1): the first word (a command head, prefix-matched against
/// `COMMAND_HEADS`) or `/send`'s second word (a channel name, prefix-
/// matched case-insensitively against `picker.channels`, same case
/// sensitivity `resolve_channel_id` already uses). Anything else — the
/// free-text message body, presence custom-text, non-`/send` second
/// words — has no finite candidate set and yields nothing.
fn compute_suggestions(text: &str, cursor: usize, picker: &SlackPickerState) -> Vec<String> {
    let up_to_cursor = &text[..cursor];
    let words: Vec<&str> = up_to_cursor.split(' ').collect();
    let current_word = words.last().copied().unwrap_or("");

    match words.len() {
        1 if current_word.starts_with('/') => COMMAND_HEADS
            .iter()
            .filter(|head| head.starts_with(current_word))
            .map(|head| (*head).to_string())
            .collect(),
        2 if words[0] == "/send" && current_word.starts_with('#') => {
            let prefix = &current_word[1..];
            picker
                .channels
                .iter()
                .filter(|c| c.label.to_lowercase().starts_with(&prefix.to_lowercase()))
                .map(|c| format!("#{}", c.label))
                .collect()
        }
        _ => Vec::new(),
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
        "/pomodoro" => parse_pomodoro_command(rest),
        _ => Ok(None),
    }
}

/// `/pomodoro start [work_min] [break_min]` (defaults if omitted),
/// `/pomodoro pause` (toggles running/paused), `/pomodoro reset`
/// (`step18.md` Decision 4).
fn parse_pomodoro_command(rest: &str) -> Result<Option<Command>, String> {
    let mut args = rest.split_whitespace();
    match args.next().unwrap_or("") {
        "start" => {
            let work_minutes = args
                .next()
                .map(|s| {
                    s.parse()
                        .map_err(|_| format!("'{s}'은(는) 유효한 분(minute) 값이 아닙니다."))
                })
                .transpose()?
                .unwrap_or(DEFAULT_WORK_MINUTES);
            let break_minutes = args
                .next()
                .map(|s| {
                    s.parse()
                        .map_err(|_| format!("'{s}'은(는) 유효한 분(minute) 값이 아닙니다."))
                })
                .transpose()?
                .unwrap_or(DEFAULT_BREAK_MINUTES);
            Ok(Some(Command::StartPomodoro {
                work_minutes,
                break_minutes,
            }))
        }
        "pause" => Ok(Some(Command::PausePomodoro)),
        "reset" => Ok(Some(Command::ResetPomodoro)),
        "" => Err(
            "사용법: /pomodoro start [작업분] [휴식분] | /pomodoro pause | /pomodoro reset"
                .to_string(),
        ),
        other => Err(format!(
            "알 수 없는 pomodoro 하위 명령어: '{other}' (start|pause|reset 중 하나)"
        )),
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
    use crate::state::{
        CalendarSetupState, GitHubSetupState, PickerRow, SlackPickerState, SlackSetupState,
    };
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
    fn tab_cycles_focus_through_all_three_docks_and_wraps() {
        // Bottom (Log) dropped out of the cycle in step19.md -- it's no
        // longer a focusable dock, Ctrl+4 opens the Log Viewer overlay
        // directly instead.
        let mut state = WorkspaceState::default();
        assert_eq!(state.focused_dock, UiDockSlot::Left);
        handle_key(&mut state, key(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(state.focused_dock, UiDockSlot::Center);
        handle_key(&mut state, key(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(state.focused_dock, UiDockSlot::Right);
        handle_key(&mut state, key(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(state.focused_dock, UiDockSlot::Left);
    }

    #[test]
    fn shift_tab_cycles_backward() {
        let mut state = WorkspaceState::default();
        handle_key(&mut state, key(KeyCode::BackTab, KeyModifiers::NONE));
        assert_eq!(state.focused_dock, UiDockSlot::Right);
    }

    #[test]
    fn ctrl_4_opens_the_log_viewer_overlay_directly_without_touching_focused_dock() {
        // Not a "focus a dock" shortcut like Ctrl+1~3 -- step19.md replaced
        // the old "focus Bottom dock" behavior with opening the overlay
        // straight away, so `focused_dock` must be untouched.
        let mut state = WorkspaceState::default();
        let outcome = handle_key(&mut state, key(KeyCode::Char('4'), KeyModifiers::CONTROL));
        assert_eq!(outcome, KeyOutcome::Handled);
        assert_eq!(state.focus_mode, FocusMode::Overlay);
        assert_eq!(state.active_overlay, OverlayKind::LogViewer);
        assert_eq!(state.focused_dock, UiDockSlot::Left);
    }

    #[test]
    fn esc_closes_the_log_viewer_overlay_like_any_other_overlay() {
        let mut state = WorkspaceState {
            focus_mode: FocusMode::Overlay,
            active_overlay: OverlayKind::LogViewer,
            ..Default::default()
        };
        handle_key(&mut state, key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(state.focus_mode, FocusMode::Normal);
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
    fn pomodoro_start_with_no_args_uses_the_default_durations() {
        let mut state = WorkspaceState {
            focus_mode: FocusMode::Input,
            ..Default::default()
        };
        let outcome = type_and_submit(&mut state, "/pomodoro start");
        assert_eq!(
            outcome,
            KeyOutcome::SubmitCommand(Command::StartPomodoro {
                work_minutes: DEFAULT_WORK_MINUTES,
                break_minutes: DEFAULT_BREAK_MINUTES,
            })
        );
    }

    #[test]
    fn pomodoro_start_with_explicit_durations_uses_them() {
        let mut state = WorkspaceState {
            focus_mode: FocusMode::Input,
            ..Default::default()
        };
        let outcome = type_and_submit(&mut state, "/pomodoro start 10 2");
        assert_eq!(
            outcome,
            KeyOutcome::SubmitCommand(Command::StartPomodoro {
                work_minutes: 10,
                break_minutes: 2,
            })
        );
    }

    #[test]
    fn pomodoro_start_with_a_non_numeric_duration_is_a_real_error_not_a_fallthrough() {
        let mut state = WorkspaceState {
            focus_mode: FocusMode::Input,
            ..Default::default()
        };
        let outcome = type_and_submit(&mut state, "/pomodoro start abc");
        assert_eq!(outcome, KeyOutcome::Handled);
        assert!(state.cmd_buffer.last_error.is_some());
    }

    #[test]
    fn pomodoro_pause_parses_to_pause_pomodoro() {
        let mut state = WorkspaceState {
            focus_mode: FocusMode::Input,
            ..Default::default()
        };
        let outcome = type_and_submit(&mut state, "/pomodoro pause");
        assert_eq!(outcome, KeyOutcome::SubmitCommand(Command::PausePomodoro));
    }

    #[test]
    fn pomodoro_reset_parses_to_reset_pomodoro() {
        let mut state = WorkspaceState {
            focus_mode: FocusMode::Input,
            ..Default::default()
        };
        let outcome = type_and_submit(&mut state, "/pomodoro reset");
        assert_eq!(outcome, KeyOutcome::SubmitCommand(Command::ResetPomodoro));
    }

    #[test]
    fn pomodoro_with_no_subcommand_is_a_usage_error() {
        let mut state = WorkspaceState {
            focus_mode: FocusMode::Input,
            ..Default::default()
        };
        let outcome = type_and_submit(&mut state, "/pomodoro");
        assert_eq!(outcome, KeyOutcome::Handled);
        assert!(state.cmd_buffer.last_error.is_some());
    }

    #[test]
    fn pomodoro_with_an_unknown_subcommand_is_a_real_error() {
        let mut state = WorkspaceState {
            focus_mode: FocusMode::Input,
            ..Default::default()
        };
        let outcome = type_and_submit(&mut state, "/pomodoro bogus");
        assert_eq!(outcome, KeyOutcome::Handled);
        assert!(state.cmd_buffer.last_error.is_some());
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

    /// Real bug: typing a partial token, pressing Esc, then reopening with
    /// Ctrl+S left the old text sitting in `token_input` -- a fresh paste
    /// would append onto it instead of replacing it, silently producing a
    /// garbled token with no visible sign anything was wrong.
    #[test]
    fn ctrl_s_clears_leftover_token_input_from_a_previous_attempt() {
        let mut state = WorkspaceState {
            slack_setup: SlackSetupState {
                token_input: "leftover-from-before".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        handle_key(&mut state, key(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert_eq!(state.slack_setup.token_input, "");
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
        assert_eq!(
            outcome,
            KeyOutcome::SubmitToken(IntegrationSource::Slack, "x".to_string())
        );
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

    #[test]
    fn ctrl_g_opens_the_github_setup_overlay() {
        let mut state = WorkspaceState::default();
        let outcome = handle_key(&mut state, key(KeyCode::Char('g'), KeyModifiers::CONTROL));
        assert_eq!(outcome, KeyOutcome::Handled);
        assert_eq!(state.focus_mode, FocusMode::Overlay);
        assert_eq!(state.active_overlay, OverlayKind::GitHubSetup);
    }

    /// See `ctrl_s_clears_leftover_token_input_from_a_previous_attempt` --
    /// same bug, same fix, same regression guard for GitHub.
    #[test]
    fn ctrl_g_clears_leftover_token_input_from_a_previous_attempt() {
        let mut state = WorkspaceState {
            github_setup: GitHubSetupState {
                token_input: "leftover-from-before".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        handle_key(&mut state, key(KeyCode::Char('g'), KeyModifiers::CONTROL));
        assert_eq!(state.github_setup.token_input, "");
    }

    #[test]
    fn github_setup_overlay_captures_typed_characters() {
        let mut state = WorkspaceState {
            focus_mode: FocusMode::Overlay,
            active_overlay: OverlayKind::GitHubSetup,
            ..Default::default()
        };
        handle_key(&mut state, key(KeyCode::Char('g'), KeyModifiers::NONE));
        handle_key(&mut state, key(KeyCode::Char('h'), KeyModifiers::NONE));
        assert_eq!(state.github_setup.token_input, "gh");
    }

    #[test]
    fn github_setup_enter_with_a_token_submits_and_clears_the_input() {
        let mut state = WorkspaceState {
            focus_mode: FocusMode::Overlay,
            active_overlay: OverlayKind::GitHubSetup,
            ..Default::default()
        };
        handle_key(&mut state, key(KeyCode::Char('x'), KeyModifiers::NONE));
        let outcome = handle_key(&mut state, key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(
            outcome,
            KeyOutcome::SubmitToken(IntegrationSource::GitHub, "x".to_string())
        );
        assert_eq!(state.github_setup.token_input, "");
        assert_eq!(state.github_setup.status, GitHubSetupStatus::Connecting);
    }

    #[test]
    fn github_setup_enter_with_no_token_does_not_submit() {
        let mut state = WorkspaceState {
            focus_mode: FocusMode::Overlay,
            active_overlay: OverlayKind::GitHubSetup,
            ..Default::default()
        };
        let outcome = handle_key(&mut state, key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(outcome, KeyOutcome::Handled);
    }

    #[test]
    fn ctrl_l_opens_the_calendar_setup_overlay() {
        let mut state = WorkspaceState::default();
        let outcome = handle_key(&mut state, key(KeyCode::Char('l'), KeyModifiers::CONTROL));
        assert_eq!(outcome, KeyOutcome::Handled);
        assert_eq!(state.focus_mode, FocusMode::Overlay);
        assert_eq!(state.active_overlay, OverlayKind::CalendarSetup);
    }

    /// See `ctrl_s_clears_leftover_token_input_from_a_previous_attempt` --
    /// this is the one a live Calendar connection failure actually traced
    /// back to (a stale partial entry concatenated with a fresh paste
    /// produced a URL `reqwest` rejected as "relative URL without a base").
    #[test]
    fn ctrl_l_clears_leftover_token_input_from_a_previous_attempt() {
        let mut state = WorkspaceState {
            calendar_setup: CalendarSetupState {
                token_input: "leftover-from-before".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        handle_key(&mut state, key(KeyCode::Char('l'), KeyModifiers::CONTROL));
        assert_eq!(state.calendar_setup.token_input, "");
    }

    #[test]
    fn calendar_setup_overlay_captures_typed_characters() {
        let mut state = WorkspaceState {
            focus_mode: FocusMode::Overlay,
            active_overlay: OverlayKind::CalendarSetup,
            ..Default::default()
        };
        handle_key(&mut state, key(KeyCode::Char('u'), KeyModifiers::NONE));
        handle_key(&mut state, key(KeyCode::Char('r'), KeyModifiers::NONE));
        handle_key(&mut state, key(KeyCode::Char('l'), KeyModifiers::NONE));
        assert_eq!(state.calendar_setup.token_input, "url");
    }

    #[test]
    fn calendar_setup_enter_with_a_url_submits_and_clears_the_input() {
        let mut state = WorkspaceState {
            focus_mode: FocusMode::Overlay,
            active_overlay: OverlayKind::CalendarSetup,
            ..Default::default()
        };
        handle_key(&mut state, key(KeyCode::Char('x'), KeyModifiers::NONE));
        let outcome = handle_key(&mut state, key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(
            outcome,
            KeyOutcome::SubmitToken(IntegrationSource::Calendar, "x".to_string())
        );
        assert_eq!(state.calendar_setup.token_input, "");
        assert_eq!(
            state.calendar_setup.status,
            crate::state::CalendarSetupStatus::Connecting
        );
    }

    #[test]
    fn calendar_setup_enter_with_no_url_does_not_submit() {
        let mut state = WorkspaceState {
            focus_mode: FocusMode::Overlay,
            active_overlay: OverlayKind::CalendarSetup,
            ..Default::default()
        };
        let outcome = handle_key(&mut state, key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(outcome, KeyOutcome::Handled);
    }

    #[test]
    fn ctrl_r_opens_the_github_picker_overlay() {
        let mut state = WorkspaceState::default();
        let outcome = handle_key(&mut state, key(KeyCode::Char('r'), KeyModifiers::CONTROL));
        assert_eq!(outcome, KeyOutcome::OpenPicker(IntegrationSource::GitHub));
        assert_eq!(state.focus_mode, FocusMode::Overlay);
        assert_eq!(state.active_overlay, OverlayKind::GitHubPicker);
    }

    fn github_picker_state_with_two_repos() -> crate::state::GitHubPickerState {
        crate::state::GitHubPickerState {
            repositories: vec![
                PickerRow {
                    id: "owner/repo-one".into(),
                    label: "owner/repo-one".into(),
                    selected: false,
                },
                PickerRow {
                    id: "owner/repo-two".into(),
                    label: "owner/repo-two".into(),
                    selected: false,
                },
            ],
            cursor: 0,
            ..Default::default()
        }
    }

    #[test]
    fn github_picker_space_toggles_the_row_under_the_cursor() {
        let mut state = WorkspaceState {
            focus_mode: FocusMode::Overlay,
            active_overlay: OverlayKind::GitHubPicker,
            github_picker: github_picker_state_with_two_repos(),
            ..Default::default()
        };
        handle_key(&mut state, key(KeyCode::Char(' '), KeyModifiers::NONE));
        assert!(state.github_picker.repositories[0].selected);
        assert!(!state.github_picker.repositories[1].selected);
    }

    #[test]
    fn github_picker_j_does_not_run_past_the_last_row() {
        let mut state = WorkspaceState {
            focus_mode: FocusMode::Overlay,
            active_overlay: OverlayKind::GitHubPicker,
            github_picker: github_picker_state_with_two_repos(),
            ..Default::default()
        };
        handle_key(&mut state, key(KeyCode::Char('j'), KeyModifiers::NONE));
        assert_eq!(state.github_picker.cursor, 1);
        handle_key(&mut state, key(KeyCode::Char('j'), KeyModifiers::NONE));
        assert_eq!(state.github_picker.cursor, 1);
    }

    #[test]
    fn github_picker_enter_submits_only_the_selected_repos() {
        let mut picker = github_picker_state_with_two_repos();
        picker.repositories[1].selected = true;
        let mut state = WorkspaceState {
            focus_mode: FocusMode::Overlay,
            active_overlay: OverlayKind::GitHubPicker,
            github_picker: picker,
            ..Default::default()
        };
        let outcome = handle_key(&mut state, key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(
            outcome,
            KeyOutcome::SubmitSelection(
                IntegrationSource::GitHub,
                vec!["owner/repo-two".to_string()]
            )
        );
    }

    fn picker_with_channels(labels: &[&str]) -> SlackPickerState {
        SlackPickerState {
            channels: labels
                .iter()
                .map(|label| PickerRow {
                    id: format!("C_{label}"),
                    label: (*label).to_string(),
                    selected: false,
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn compute_suggestions_matches_command_heads_by_prefix() {
        let picker = SlackPickerState::default();
        let mut candidates = compute_suggestions("/a", 2, &picker);
        candidates.sort();
        assert_eq!(candidates, vec!["/active".to_string(), "/away".to_string()]);
    }

    #[test]
    fn compute_suggestions_is_empty_for_a_full_word_that_matches_nothing() {
        let picker = SlackPickerState::default();
        assert!(compute_suggestions("/xyz", 4, &picker).is_empty());
    }

    #[test]
    fn compute_suggestions_matches_send_channel_argument_by_prefix() {
        let picker = picker_with_channels(&["general", "general-eng", "random"]);
        let mut candidates = compute_suggestions("/send #gen", 10, &picker);
        candidates.sort();
        assert_eq!(
            candidates,
            vec!["#general".to_string(), "#general-eng".to_string()]
        );
    }

    #[test]
    fn compute_suggestions_channel_matching_is_case_insensitive() {
        let picker = picker_with_channels(&["General"]);
        let candidates = compute_suggestions("/send #gen", 10, &picker);
        assert_eq!(candidates, vec!["#General".to_string()]);
    }

    #[test]
    fn compute_suggestions_does_not_offer_channels_for_non_send_commands() {
        let picker = picker_with_channels(&["general"]);
        assert!(compute_suggestions("/away #gen", 10, &picker).is_empty());
    }

    #[test]
    fn compute_suggestions_does_not_offer_channels_past_the_argument_position() {
        // Third word (the free-text message body) is never completable.
        let picker = picker_with_channels(&["general"]);
        let text = "/send #general #gen";
        assert!(compute_suggestions(text, text.len(), &picker).is_empty());
    }

    #[test]
    fn word_start_finds_the_last_space_before_the_cursor() {
        assert_eq!(word_start("/send #general", "/send #general".len()), 6);
        assert_eq!(word_start("/active", "/active".len()), 0);
        assert_eq!(word_start("", 0), 0);
    }

    #[test]
    fn tab_with_no_candidates_is_a_noop() {
        let mut state = WorkspaceState {
            focus_mode: FocusMode::Input,
            ..Default::default()
        };
        for c in "hello".chars() {
            handle_key(&mut state, key(KeyCode::Char(c), KeyModifiers::NONE));
        }
        let outcome = handle_key(&mut state, key(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(outcome, KeyOutcome::Handled);
        assert_eq!(state.cmd_buffer.raw_text, "hello");
    }

    #[test]
    fn first_tab_completes_to_the_first_candidate() {
        let mut state = WorkspaceState {
            focus_mode: FocusMode::Input,
            ..Default::default()
        };
        for c in "/a".chars() {
            handle_key(&mut state, key(KeyCode::Char(c), KeyModifiers::NONE));
        }
        handle_key(&mut state, key(KeyCode::Tab, KeyModifiers::NONE));
        // COMMAND_HEADS declares "send", "away", "active", "offline",
        // "meeting", "lunch" in that order -- "/a" prefix-matches "away"
        // before "active" since filtering preserves array order.
        assert_eq!(state.cmd_buffer.raw_text, "/away");
        assert_eq!(state.cmd_buffer.cursor_position, "/away".len());
    }

    #[test]
    fn consecutive_tabs_cycle_through_candidates_and_wrap() {
        let mut state = WorkspaceState {
            focus_mode: FocusMode::Input,
            ..Default::default()
        };
        for c in "/a".chars() {
            handle_key(&mut state, key(KeyCode::Char(c), KeyModifiers::NONE));
        }
        handle_key(&mut state, key(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(state.cmd_buffer.raw_text, "/away");
        handle_key(&mut state, key(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(state.cmd_buffer.raw_text, "/active");
        handle_key(&mut state, key(KeyCode::Tab, KeyModifiers::NONE));
        // Wraps back to the first candidate, not stuck or advancing past
        // the end of the list.
        assert_eq!(state.cmd_buffer.raw_text, "/away");
    }

    #[test]
    fn typing_between_tabs_starts_a_fresh_cycle() {
        let mut state = WorkspaceState {
            focus_mode: FocusMode::Input,
            ..Default::default()
        };
        for c in "/a".chars() {
            handle_key(&mut state, key(KeyCode::Char(c), KeyModifiers::NONE));
        }
        handle_key(&mut state, key(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(state.cmd_buffer.raw_text, "/away");
        // Typing "y" after completion changes the word under the cursor to
        // "/awayy" -- no longer a real command, so the next Tab has
        // nothing to offer rather than continuing the stale cycle.
        handle_key(&mut state, key(KeyCode::Char('y'), KeyModifiers::NONE));
        let outcome = handle_key(&mut state, key(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(outcome, KeyOutcome::Handled);
        assert_eq!(state.cmd_buffer.raw_text, "/awayy");
    }

    #[test]
    fn tab_completes_a_channel_name_for_send() {
        let mut state = WorkspaceState {
            focus_mode: FocusMode::Input,
            slack_picker: picker_with_channels(&["general"]),
            ..Default::default()
        };
        for c in "/send #gen".chars() {
            handle_key(&mut state, key(KeyCode::Char(c), KeyModifiers::NONE));
        }
        handle_key(&mut state, key(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(state.cmd_buffer.raw_text, "/send #general");
    }

    #[test]
    fn enter_clears_autocomplete_state() {
        let mut state = WorkspaceState {
            focus_mode: FocusMode::Input,
            ..Default::default()
        };
        for c in "/away".chars() {
            handle_key(&mut state, key(KeyCode::Char(c), KeyModifiers::NONE));
        }
        assert!(!state.cmd_buffer.autocomplete_suggestions.is_empty());
        handle_key(&mut state, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(state.cmd_buffer.autocomplete_suggestions.is_empty());
        assert_eq!(state.cmd_buffer.selected_suggestion_index, None);
    }
}
