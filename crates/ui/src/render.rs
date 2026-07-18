//! Ratatui drawing functions. See `docs/01-product/screen-spec.md` for the
//! layout this implements (Phase 5 subset — see `step5.md`).

use crate::state::{
    FocusMode, GitHubPickerStatus, GitHubSetupStatus, OverlayKind, SlackPickerStatus,
    SlackSetupStatus, WorkspaceState,
};
use commands::DashboardReadModel;
use domain::{IntegrationSource, PresenceStatus};
use events::IntegrationConnectionStatus;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;
use registry::UiDockSlot;

const MIN_WIDTH: u16 = 80;
const MIN_HEIGHT: u16 = 24;
const SIDEBAR_COLLAPSE_WIDTH: u16 = 120;
const LEFT_DOCK_WIDTH: u16 = 24;
const RIGHT_DOCK_WIDTH: u16 = 32;

/// Entry point: draws the whole frame per `state`/`model`, or the
/// too-small placeholder if the terminal is below `docs/01-product/screen-spec.md`'s
/// minimum grid size.
pub fn render(frame: &mut Frame, state: &WorkspaceState, model: &DashboardReadModel) {
    let area = frame.size();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        render_too_small(frame, area);
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Header
            Constraint::Min(5),    // Body (Left/Center/Right docks)
            Constraint::Length(3), // Bottom dock
            Constraint::Length(1), // Command bar
            Constraint::Length(1), // Footer
        ])
        .split(area);

    render_header(frame, rows[0], state);

    let collapse_sidebars = area.width < SIDEBAR_COLLAPSE_WIDTH;
    if collapse_sidebars {
        render_notification_panel(frame, rows[1], state, model);
    } else {
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(LEFT_DOCK_WIDTH),
                Constraint::Min(0),
                Constraint::Length(RIGHT_DOCK_WIDTH),
            ])
            .split(rows[1]);
        render_team_panel(frame, body[0], state, model);
        render_notification_panel(frame, body[1], state, model);
        render_right_dock_placeholder(frame, body[2]);
    }

    render_bottom_dock_placeholder(frame, rows[2]);
    render_command_bar(frame, rows[3], state);
    render_footer(frame, rows[4]);

    if state.focus_mode == FocusMode::Overlay {
        match state.active_overlay {
            OverlayKind::Help => render_help_overlay(frame, area),
            OverlayKind::SlackSetup => render_slack_setup_overlay(frame, area, state),
            OverlayKind::SlackPicker => render_slack_picker_overlay(frame, area, state),
            OverlayKind::GitHubSetup => render_github_setup_overlay(frame, area, state),
            OverlayKind::GitHubPicker => render_github_picker_overlay(frame, area, state),
        }
    }
}

/// One key/description row within a [`HELP_CATEGORIES`] section.
struct HelpEntry {
    key: &'static str,
    description: &'static str,
}

/// A titled group of [`HelpEntry`] rows. Data-driven rather than one
/// hand-formatted string: as more integrations arrive (Calendar, Jira, ...)
/// each just appends a category here instead of hand-aligning a growing
/// wall of text — the flat-list version of this became hard to scan once
/// Slack and GitHub's shortcuts were both mixed in with navigation and
/// command-bar syntax.
const HELP_CATEGORIES: &[(&str, &[HelpEntry])] = &[
    (
        "탐색",
        &[
            HelpEntry {
                key: "Tab / Shift+Tab",
                description: "패널 포커스 순환",
            },
            HelpEntry {
                key: "Ctrl+1~4",
                description: "패널로 바로 이동 (팀/알림/캘린더/로그)",
            },
            HelpEntry {
                key: "j/k, ↑/↓",
                description: "선택한 패널 안에서 위아래 이동",
            },
        ],
    ),
    (
        "명령줄",
        &[
            HelpEntry {
                key: ":",
                description: "명령줄 입력",
            },
            HelpEntry {
                key: "/send #채널 메시지",
                description: "Slack 메시지 보내기",
            },
            HelpEntry {
                key: "/away, /active, /offline, /meeting, /lunch [메시지]",
                description: "내 상태 변경",
            },
        ],
    ),
    (
        "Slack 연동",
        &[
            HelpEntry {
                key: "Ctrl+S",
                description: "연결 설정",
            },
            HelpEntry {
                key: "Ctrl+P",
                description: "채널/사용자 선택",
            },
        ],
    ),
    (
        "GitHub 연동",
        &[
            HelpEntry {
                key: "Ctrl+G",
                description: "연결 설정",
            },
            HelpEntry {
                key: "Ctrl+R",
                description: "저장소 선택",
            },
        ],
    ),
    (
        "기타",
        &[
            HelpEntry {
                key: "Esc",
                description: "닫기 / Normal 모드로 복귀",
            },
            HelpEntry {
                key: "Ctrl+Q",
                description: "종료",
            },
        ],
    ),
];

fn render_help_overlay(frame: &mut Frame, area: Rect) {
    let popup = centered_rect(60, 60, area);
    frame.render_widget(Clear, popup);

    let mut items: Vec<ListItem> = Vec::new();
    for (i, (title, entries)) in HELP_CATEGORIES.iter().enumerate() {
        if i > 0 {
            items.push(ListItem::new(""));
        }
        items.push(ListItem::new(Span::styled(
            *title,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )));
        for entry in *entries {
            items.push(ListItem::new(format!(
                "  {:<20} {}",
                entry.key, entry.description
            )));
        }
    }

    let block = Block::default()
        .title("도움말")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    frame.render_widget(List::new(items).block(block), popup);
}

/// In-app Slack Bot Token entry (`step7.md`). The token is rendered
/// masked (`*` per character) — never the raw value — so a screenshot or
/// shoulder-surf of this overlay doesn't leak it the way the command bar
/// (plain-text history) would.
fn render_slack_setup_overlay(frame: &mut Frame, area: Rect, state: &WorkspaceState) {
    let popup = centered_rect(60, 30, area);
    frame.render_widget(Clear, popup);

    let masked: String = "*".repeat(state.slack_setup.token_input.chars().count());
    let status_line = match &state.slack_setup.status {
        SlackSetupStatus::Idle => "Bot Token을 입력하고 Enter를 누르세요.".to_string(),
        SlackSetupStatus::Connecting => "연결 중...".to_string(),
        SlackSetupStatus::Connected => "연결됨.".to_string(),
        SlackSetupStatus::Failed(reason) => format!("연결 실패: {reason}"),
    };
    let text = format!("Token: {masked}\n\n{status_line}\n\nEsc: 닫기");

    let block = Block::default()
        .title("Slack 연결 설정")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    frame.render_widget(
        Paragraph::new(text).block(block).wrap(Wrap { trim: true }),
        popup,
    );
}

/// Slack channel/watched-user picker (`step8.md`). `cursor` indexes into
/// the combined channel-then-user list; the section headers ("채널"/"사용자")
/// aren't part of that index, they're just labels.
fn render_slack_picker_overlay(frame: &mut Frame, area: Rect, state: &WorkspaceState) {
    let popup = centered_rect(70, 70, area);
    frame.render_widget(Clear, popup);

    let picker = &state.slack_picker;
    let block = Block::default()
        .title("Slack 채널/사용자 선택")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    match &picker.status {
        SlackPickerStatus::Loading => {
            frame.render_widget(Paragraph::new("불러오는 중...").block(block), popup);
            return;
        }
        SlackPickerStatus::Failed(reason) => {
            frame.render_widget(
                Paragraph::new(format!("불러오기 실패: {reason}\n\nEsc: 닫기"))
                    .block(block)
                    .wrap(Wrap { trim: true }),
                popup,
            );
            return;
        }
        SlackPickerStatus::Idle
        | SlackPickerStatus::Loaded
        | SlackPickerStatus::Saving
        | SlackPickerStatus::Saved => {}
    }

    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(inner);

    let mut items: Vec<ListItem> = Vec::new();
    items.push(ListItem::new(Span::styled(
        "채널 (봇이 초대된 채널만 표시)",
        Style::default().add_modifier(Modifier::BOLD),
    )));
    if picker.channels.is_empty() {
        items.push(ListItem::new(
            "  (없음 — 채널에 봇을 초대해야 여기 나타납니다)",
        ));
    }
    for (i, row) in picker.channels.iter().enumerate() {
        let checkbox = if row.selected { "[x]" } else { "[ ]" };
        let style = if picker.cursor == i {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        items.push(ListItem::new(format!("  {checkbox} #{}", row.label)).style(style));
    }
    items.push(ListItem::new(Span::styled(
        "사용자",
        Style::default().add_modifier(Modifier::BOLD),
    )));
    for (i, row) in picker.users.iter().enumerate() {
        let checkbox = if row.selected { "[x]" } else { "[ ]" };
        let index = picker.channels.len() + i;
        let style = if picker.cursor == index {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        items.push(ListItem::new(format!("  {checkbox} {}", row.label)).style(style));
    }
    frame.render_widget(List::new(items), layout[0]);

    let status_line = match &picker.status {
        SlackPickerStatus::Saving => "저장 중...",
        SlackPickerStatus::Saved => "저장됨!",
        _ => "j/k: 이동  Space: 선택/해제  Enter: 저장  Esc: 닫기",
    };
    frame.render_widget(
        Paragraph::new(status_line).style(Style::default().fg(Color::DarkGray)),
        layout[1],
    );
}

/// In-app GitHub PAT entry (`step10.md`). Mirrors `render_slack_setup_overlay`
/// exactly, including the masked-token rationale.
fn render_github_setup_overlay(frame: &mut Frame, area: Rect, state: &WorkspaceState) {
    let popup = centered_rect(60, 30, area);
    frame.render_widget(Clear, popup);

    let masked: String = "*".repeat(state.github_setup.token_input.chars().count());
    let status_line = match &state.github_setup.status {
        GitHubSetupStatus::Idle => "Personal Access Token을 입력하고 Enter를 누르세요.".to_string(),
        GitHubSetupStatus::Connecting => "연결 중...".to_string(),
        GitHubSetupStatus::Connected => "연결됨.".to_string(),
        GitHubSetupStatus::Failed(reason) => format!("연결 실패: {reason}"),
    };
    let text = format!("Token: {masked}\n\n{status_line}\n\nEsc: 닫기");

    let block = Block::default()
        .title("GitHub 연결 설정")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    frame.render_widget(
        Paragraph::new(text).block(block).wrap(Wrap { trim: true }),
        popup,
    );
}

/// GitHub repository picker (`step10.md`). Simpler than
/// `render_slack_picker_overlay`: one list, no channel/user section split.
fn render_github_picker_overlay(frame: &mut Frame, area: Rect, state: &WorkspaceState) {
    let popup = centered_rect(70, 70, area);
    frame.render_widget(Clear, popup);

    let picker = &state.github_picker;
    let block = Block::default()
        .title("GitHub 저장소 선택")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    match &picker.status {
        GitHubPickerStatus::Loading => {
            frame.render_widget(Paragraph::new("불러오는 중...").block(block), popup);
            return;
        }
        GitHubPickerStatus::Failed(reason) => {
            frame.render_widget(
                Paragraph::new(format!("불러오기 실패: {reason}\n\nEsc: 닫기"))
                    .block(block)
                    .wrap(Wrap { trim: true }),
                popup,
            );
            return;
        }
        GitHubPickerStatus::Idle
        | GitHubPickerStatus::Loaded
        | GitHubPickerStatus::Saving
        | GitHubPickerStatus::Saved => {}
    }

    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(inner);

    let mut items: Vec<ListItem> = Vec::new();
    if picker.repositories.is_empty() {
        items.push(ListItem::new("  (없음)"));
    }
    for (i, row) in picker.repositories.iter().enumerate() {
        let checkbox = if row.selected { "[x]" } else { "[ ]" };
        let style = if picker.cursor == i {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        items.push(ListItem::new(format!("  {checkbox} {}", row.label)).style(style));
    }
    frame.render_widget(List::new(items), layout[0]);

    let status_line = match &picker.status {
        GitHubPickerStatus::Saving => "저장 중...",
        GitHubPickerStatus::Saved => "저장됨!",
        _ => "j/k: 이동  Space: 선택/해제  Enter: 저장  Esc: 닫기",
    };
    frame.render_widget(
        Paragraph::new(status_line).style(Style::default().fg(Color::DarkGray)),
        layout[1],
    );
}

/// Standard ratatui centered-popup idiom: `percent_x`/`percent_y` of `area`,
/// centered on both axes.
fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

fn render_too_small(frame: &mut Frame, area: Rect) {
    let paragraph = Paragraph::new("터미널 크기가 너무 작습니다. 화면을 넓혀주세요.")
        .style(Style::default().fg(Color::Red))
        .wrap(Wrap { trim: true });
    frame.render_widget(paragraph, area);
}

fn presence_status_label(status: PresenceStatus) -> &'static str {
    match status {
        PresenceStatus::Active => "활동중",
        PresenceStatus::Away => "자리비움",
        PresenceStatus::Offline => "오프라인",
        PresenceStatus::Meeting => "회의중",
        PresenceStatus::Lunch => "식사중",
    }
}

fn integration_source_label(source: IntegrationSource) -> &'static str {
    match source {
        IntegrationSource::Slack => "슬랙",
        IntegrationSource::GitHub => "깃허브",
        IntegrationSource::Calendar => "캘린더",
        IntegrationSource::Gmail => "지메일",
        IntegrationSource::Jira => "지라",
    }
}

fn dock_block<'a>(title: &'a str, slot: UiDockSlot, state: &WorkspaceState) -> Block<'a> {
    let focused = state.focused_dock == slot;
    let style = if focused {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(style)
}

fn render_header(frame: &mut Frame, area: Rect, state: &WorkspaceState) {
    let (slack_text, slack_color) =
        connection_status_label("Slack", &state.slack_connection_status);
    let (github_text, github_color) =
        connection_status_label("GitHub", &state.github_connection_status);
    let header = Paragraph::new(Line::from(vec![
        Span::styled(
            "Terminal Workspace",
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw("  |  "),
        Span::styled(slack_text, Style::default().fg(slack_color)),
        Span::raw("  |  "),
        Span::styled(github_text, Style::default().fg(github_color)),
        Span::raw("  |  도움말: ?   종료: Ctrl+Q"),
    ]));
    frame.render_widget(header, area);
}

/// Kept current purely by the `EventBus` subscription in `crates/ui/src/lib.rs`'s
/// event loop (`step9.md`, ADR-0016) — not polled, genuinely live. Generic
/// across integrations since `step10.md` (was `slack_status_label`,
/// Slack-only) — `label` is the integration's display name.
fn connection_status_label(
    label: &'static str,
    status: &IntegrationConnectionStatus,
) -> (String, Color) {
    let (suffix, color) = match status {
        IntegrationConnectionStatus::Disconnected => ("연결 안 됨", Color::DarkGray),
        IntegrationConnectionStatus::Connecting => ("연결 중...", Color::Yellow),
        IntegrationConnectionStatus::Connected => ("연결됨", Color::Green),
        IntegrationConnectionStatus::Reconnecting => ("재연결 중...", Color::Yellow),
        IntegrationConnectionStatus::Failed(_) => ("연결 실패", Color::Red),
    };
    (format!("{label}: {suffix}"), color)
}

fn render_team_panel(
    frame: &mut Frame,
    area: Rect,
    state: &WorkspaceState,
    model: &DashboardReadModel,
) {
    let block = dock_block("팀", UiDockSlot::Left, state);
    if model.team_presence.is_empty() {
        let empty = Paragraph::new("(아직 팀원이 없습니다)")
            .block(block)
            .wrap(Wrap { trim: true });
        frame.render_widget(empty, area);
        return;
    }

    let items: Vec<ListItem> = model
        .team_presence
        .iter()
        .enumerate()
        .map(|(i, member)| {
            let line = format!(
                "{} [{}]",
                member.display_name,
                presence_status_label(member.status)
            );
            let style = if state.focused_dock == UiDockSlot::Left && i == state.selected_index {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            ListItem::new(line).style(style)
        })
        .collect();
    frame.render_widget(List::new(items).block(block), area);
}

fn render_notification_panel(
    frame: &mut Frame,
    area: Rect,
    state: &WorkspaceState,
    model: &DashboardReadModel,
) {
    let block = dock_block("알림", UiDockSlot::Center, state);
    if model.unread_notifications.is_empty() {
        let empty = Paragraph::new("(아직 알림이 없습니다)")
            .block(block)
            .wrap(Wrap { trim: true });
        frame.render_widget(empty, area);
        return;
    }

    let items: Vec<ListItem> = model
        .unread_notifications
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let line = format!("[{}] {}", integration_source_label(item.source), item.title);
            let style = if state.focused_dock == UiDockSlot::Center && i == state.selected_index {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            ListItem::new(line).style(style)
        })
        .collect();
    frame.render_widget(List::new(items).block(block), area);
}

fn render_right_dock_placeholder(frame: &mut Frame, area: Rect) {
    let block = Block::default().title("캘린더").borders(Borders::ALL);
    let placeholder = Paragraph::new("(캘린더 연동이 아직 구현되지 않았습니다)")
        .block(block)
        .wrap(Wrap { trim: true });
    frame.render_widget(placeholder, area);
}

fn render_bottom_dock_placeholder(frame: &mut Frame, area: Rect) {
    let block = Block::default().title("로그").borders(Borders::ALL);
    let placeholder = Paragraph::new("(로그 스트림이 아직 연결되지 않았습니다)")
        .block(block)
        .wrap(Wrap { trim: true });
    frame.render_widget(placeholder, area);
}

fn render_command_bar(frame: &mut Frame, area: Rect, state: &WorkspaceState) {
    // A command-parse/dispatch error (`step9.md`) stays visible even after
    // Esc leaves Input mode -- the user typed a deliberate command and
    // should be able to read what went wrong, not have it vanish the
    // instant the buffer closes.
    if let Some(error) = &state.cmd_buffer.last_error {
        let text = format!(":{}  {error}", state.cmd_buffer.raw_text);
        frame.render_widget(
            Paragraph::new(text).style(Style::default().fg(Color::Red)),
            area,
        );
        return;
    }
    let text = match state.focus_mode {
        FocusMode::Input => format!(":{}", state.cmd_buffer.raw_text),
        _ => String::new(),
    };
    frame.render_widget(Paragraph::new(text), area);
}

fn render_footer(frame: &mut Frame, area: Rect) {
    let footer =
        Paragraph::new("Tab:다음 패널  Ctrl+1~4:포커스 이동  ::명령줄  ?:도움말  Ctrl+Q:종료")
            .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(footer, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{CommandBufferState, PickerRow, SlackPickerState, SlackSetupState};
    use commands::DashboardReadModel;
    use domain::{MemberPresence, PresenceStatus, UserId};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    /// Per `docs/05-operations/testing.md` §4: draw into a `TestBackend`
    /// virtual buffer and assert on its contents instead of a real terminal.
    /// Checked via substring search rather than exact `assert_buffer` line
    /// matching, which is more robust to minor spacing/border changes while
    /// still proving the right content actually rendered.
    fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
        let buffer = terminal.backend().buffer();
        let width = buffer.area.width as usize;
        // Row boundaries matter for multi-byte/wide (e.g. Korean) text: a
        // flat cell concatenation with no row separators can splice a
        // phrase that wraps mid-word across two rows back together in a way
        // that reads fine to a human but silently drops the wrap point,
        // which then makes substring assertions fragile in the other
        // direction (matching text that only looks contiguous because the
        // row boundary was discarded). Joining rows with '\n' keeps the
        // buffer's actual line structure intact.
        buffer
            .content()
            .chunks(width.max(1))
            .map(|row| {
                row.iter()
                    .map(ratatui::buffer::Cell::symbol)
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Ratatui fills the trailing cell of every double-width glyph (Korean
    /// included) with a literal space, so a Korean phrase read back cell-by
    /// -cell has one extra space after every character (`"너 무 " `, not
    /// `"너무"`) — that's the buffer's actual rendered structure, not a
    /// bug. Comparing with whitespace stripped from both sides checks "are
    /// the right glyphs present, in order" without being tripped up by that
    /// per-glyph padding.
    fn contains_ignoring_whitespace(haystack: &str, needle: &str) -> bool {
        let strip = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
        strip(haystack).contains(&strip(needle))
    }

    fn draw(width: u16, height: u16, state: &WorkspaceState, model: &DashboardReadModel) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, state, model)).unwrap();
        buffer_text(&terminal)
    }

    #[test]
    fn renders_too_small_placeholder_below_minimum_grid() {
        let text = draw(
            40,
            10,
            &WorkspaceState::default(),
            &DashboardReadModel::default(),
        );
        assert!(contains_ignoring_whitespace(&text, "너무 작습니다"));
    }

    #[test]
    fn renders_empty_state_text_for_team_and_notifications() {
        let text = draw(
            140,
            30,
            &WorkspaceState::default(),
            &DashboardReadModel::default(),
        );
        assert!(text.contains("팀"));
        assert!(contains_ignoring_whitespace(&text, "아직 팀원이 없습니다"));
        assert!(contains_ignoring_whitespace(&text, "알림"));
        assert!(contains_ignoring_whitespace(&text, "아직 알림이 없습니다"));
    }

    #[test]
    fn collapses_sidebars_below_120_columns() {
        let text = draw(
            100,
            30,
            &WorkspaceState::default(),
            &DashboardReadModel::default(),
        );
        assert!(!contains_ignoring_whitespace(&text, "아직 팀원이 없습니다"));
        assert!(contains_ignoring_whitespace(&text, "알림"));
    }

    #[test]
    fn calendar_placeholder_text_is_not_truncated() {
        // The right dock (32 cols minus borders) is narrower than this
        // sentence needs, so it wraps onto a second line. Without
        // `.wrap(...)`, Paragraph fills up to the panel's width and
        // silently drops the rest -- this tail fragment is exactly what
        // would vanish if that ever regresses. (Not checked as one
        // contiguous string: with three panels side by side, a wrapped
        // panel's second line sits after its neighbors' first-line border
        // characters in raw buffer order, so a whitespace-stripped
        // contiguous match would be a false negative even when both lines
        // render correctly -- see the neighboring dock's `┌`/`│` borders in
        // between.)
        let text = draw(
            140,
            30,
            &WorkspaceState::default(),
            &DashboardReadModel::default(),
        );
        assert!(contains_ignoring_whitespace(&text, "구현되지"));
        assert!(contains_ignoring_whitespace(&text, "않았습니다"));
    }

    #[test]
    fn log_placeholder_text_is_not_truncated() {
        let text = draw(
            140,
            30,
            &WorkspaceState::default(),
            &DashboardReadModel::default(),
        );
        assert!(contains_ignoring_whitespace(
            &text,
            "로그 스트림이 아직 연결되지 않았습니다"
        ));
    }

    #[test]
    fn layout_adapts_across_a_range_of_terminal_sizes() {
        // The layout is recomputed from `frame.size()` on every draw, so
        // resizing the real terminal (not just the app's own math) is what
        // makes this actually responsive — this test sweeps sizes above
        // the documented minimum to confirm none of them panic or produce
        // an empty frame.
        for (width, height) in [(80, 24), (100, 30), (119, 30), (120, 30), (200, 50)] {
            let text = draw(
                width,
                height,
                &WorkspaceState::default(),
                &DashboardReadModel::default(),
            );
            assert!(!text.trim().is_empty(), "blank frame at {width}x{height}");
        }
    }

    #[test]
    fn renders_real_team_presence_data() {
        let model = DashboardReadModel {
            team_presence: vec![MemberPresence {
                user_id: UserId("u1".into()),
                display_name: "Alice".into(),
                status: PresenceStatus::Active,
                custom_status_text: None,
                last_updated_ms: 0,
            }],
            unread_notifications: Vec::new(),
        };
        let text = draw(140, 30, &WorkspaceState::default(), &model);
        assert!(text.contains("Alice"));
    }

    #[test]
    fn does_not_panic_at_exact_minimum_grid_size() {
        // 80x24 is the documented minimum (docs/01-product/screen-spec.md
        // §3), not "too small" — must render the real layout, not the
        // placeholder.
        let text = draw(
            80,
            24,
            &WorkspaceState::default(),
            &DashboardReadModel::default(),
        );
        assert!(!contains_ignoring_whitespace(&text, "너무 작습니다"));
    }

    // "도움말" alone isn't a safe marker — the header/footer show "도움말"
    // hints in every mode. "패널로 바로 이동" only appears inside the
    // overlay body.
    const OVERLAY_ONLY_TEXT: &str = "패널로 바로 이동";

    #[test]
    fn overlay_mode_renders_help_popup() {
        let state = WorkspaceState {
            focus_mode: FocusMode::Overlay,
            ..Default::default()
        };
        let text = draw(140, 30, &state, &DashboardReadModel::default());
        assert!(contains_ignoring_whitespace(&text, OVERLAY_ONLY_TEXT));
    }

    #[test]
    fn help_overlay_groups_shortcuts_under_category_headers() {
        let state = WorkspaceState {
            focus_mode: FocusMode::Overlay,
            ..Default::default()
        };
        let text = draw(140, 40, &state, &DashboardReadModel::default());
        for category in ["탐색", "명령줄", "Slack 연동", "GitHub 연동", "기타"] {
            assert!(
                contains_ignoring_whitespace(&text, category),
                "missing help category: {category}"
            );
        }
        // A representative entry from each category, proving rows still
        // show up under their heading, not just the headings themselves.
        assert!(contains_ignoring_whitespace(&text, "Ctrl+S"));
        assert!(contains_ignoring_whitespace(&text, "Ctrl+G"));
    }

    #[test]
    fn normal_mode_does_not_render_help_popup() {
        let text = draw(
            140,
            30,
            &WorkspaceState::default(),
            &DashboardReadModel::default(),
        );
        assert!(!contains_ignoring_whitespace(&text, OVERLAY_ONLY_TEXT));
    }

    #[test]
    fn slack_setup_overlay_renders_masked_token_not_the_raw_value() {
        let state = WorkspaceState {
            focus_mode: FocusMode::Overlay,
            active_overlay: OverlayKind::SlackSetup,
            slack_setup: SlackSetupState {
                token_input: "xoxb-super-secret".to_string(),
                status: SlackSetupStatus::Idle,
            },
            ..Default::default()
        };
        let text = draw(140, 30, &state, &DashboardReadModel::default());
        // The real regression to guard against: the raw token must never
        // appear in the rendered buffer, only asterisks standing in for it.
        assert!(!text.contains("xoxb-super-secret"));
        assert!(contains_ignoring_whitespace(&text, "Slack 연결 설정"));
        let mask = "*".repeat("xoxb-super-secret".chars().count());
        assert!(text.contains(&mask));
    }

    #[test]
    fn slack_setup_overlay_shows_the_failure_reason() {
        let state = WorkspaceState {
            focus_mode: FocusMode::Overlay,
            active_overlay: OverlayKind::SlackSetup,
            slack_setup: SlackSetupState {
                token_input: String::new(),
                status: SlackSetupStatus::Failed("invalid_auth".to_string()),
            },
            ..Default::default()
        };
        let text = draw(140, 30, &state, &DashboardReadModel::default());
        assert!(contains_ignoring_whitespace(&text, "invalid_auth"));
    }

    #[test]
    fn help_overlay_does_not_render_when_slack_setup_is_active() {
        let state = WorkspaceState {
            focus_mode: FocusMode::Overlay,
            active_overlay: OverlayKind::SlackSetup,
            ..Default::default()
        };
        let text = draw(140, 30, &state, &DashboardReadModel::default());
        assert!(!contains_ignoring_whitespace(&text, OVERLAY_ONLY_TEXT));
    }

    #[test]
    fn slack_picker_overlay_shows_loading_state() {
        let state = WorkspaceState {
            focus_mode: FocusMode::Overlay,
            active_overlay: OverlayKind::SlackPicker,
            slack_picker: SlackPickerState {
                status: SlackPickerStatus::Loading,
                ..Default::default()
            },
            ..Default::default()
        };
        let text = draw(140, 30, &state, &DashboardReadModel::default());
        assert!(contains_ignoring_whitespace(&text, "불러오는 중"));
    }

    #[test]
    fn slack_picker_overlay_renders_checkboxes_reflecting_selection() {
        let state = WorkspaceState {
            focus_mode: FocusMode::Overlay,
            active_overlay: OverlayKind::SlackPicker,
            slack_picker: SlackPickerState {
                channels: vec![
                    PickerRow {
                        id: "C1".into(),
                        label: "general".into(),
                        selected: true,
                    },
                    PickerRow {
                        id: "C2".into(),
                        label: "random".into(),
                        selected: false,
                    },
                ],
                users: vec![],
                cursor: 0,
                status: SlackPickerStatus::Loaded,
            },
            ..Default::default()
        };
        let text = draw(140, 30, &state, &DashboardReadModel::default());
        assert!(text.contains("[x] #general"));
        assert!(text.contains("[ ] #random"));
    }

    #[test]
    fn slack_picker_overlay_shows_the_failure_reason() {
        let state = WorkspaceState {
            focus_mode: FocusMode::Overlay,
            active_overlay: OverlayKind::SlackPicker,
            slack_picker: SlackPickerState {
                status: SlackPickerStatus::Failed("invalid_auth".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        let text = draw(140, 30, &state, &DashboardReadModel::default());
        assert!(contains_ignoring_whitespace(&text, "invalid_auth"));
    }

    #[test]
    fn header_shows_connected_status() {
        let state = WorkspaceState {
            slack_connection_status: IntegrationConnectionStatus::Connected,
            ..Default::default()
        };
        let text = draw(140, 30, &state, &DashboardReadModel::default());
        assert!(contains_ignoring_whitespace(&text, "Slack: 연결됨"));
    }

    #[test]
    fn header_shows_reconnecting_status() {
        let state = WorkspaceState {
            slack_connection_status: IntegrationConnectionStatus::Reconnecting,
            ..Default::default()
        };
        let text = draw(140, 30, &state, &DashboardReadModel::default());
        assert!(contains_ignoring_whitespace(&text, "재연결"));
    }

    #[test]
    fn header_shows_disconnected_status_by_default() {
        let text = draw(
            140,
            30,
            &WorkspaceState::default(),
            &DashboardReadModel::default(),
        );
        assert!(contains_ignoring_whitespace(&text, "연결 안"));
    }

    #[test]
    fn github_setup_overlay_renders_masked_token_not_the_raw_value() {
        let state = WorkspaceState {
            focus_mode: FocusMode::Overlay,
            active_overlay: OverlayKind::GitHubSetup,
            github_setup: crate::state::GitHubSetupState {
                token_input: "ghp_super_secret".to_string(),
                status: GitHubSetupStatus::Idle,
            },
            ..Default::default()
        };
        let text = draw(140, 30, &state, &DashboardReadModel::default());
        assert!(!text.contains("ghp_super_secret"));
        assert!(contains_ignoring_whitespace(&text, "GitHub 연결 설정"));
        let mask = "*".repeat("ghp_super_secret".chars().count());
        assert!(text.contains(&mask));
    }

    #[test]
    fn github_setup_overlay_shows_the_failure_reason() {
        let state = WorkspaceState {
            focus_mode: FocusMode::Overlay,
            active_overlay: OverlayKind::GitHubSetup,
            github_setup: crate::state::GitHubSetupState {
                token_input: String::new(),
                status: GitHubSetupStatus::Failed("bad_credentials".to_string()),
            },
            ..Default::default()
        };
        let text = draw(140, 30, &state, &DashboardReadModel::default());
        assert!(contains_ignoring_whitespace(&text, "bad_credentials"));
    }

    #[test]
    fn github_picker_overlay_shows_loading_state() {
        let state = WorkspaceState {
            focus_mode: FocusMode::Overlay,
            active_overlay: OverlayKind::GitHubPicker,
            github_picker: crate::state::GitHubPickerState {
                status: GitHubPickerStatus::Loading,
                ..Default::default()
            },
            ..Default::default()
        };
        let text = draw(140, 30, &state, &DashboardReadModel::default());
        assert!(contains_ignoring_whitespace(&text, "불러오는 중"));
    }

    #[test]
    fn github_picker_overlay_renders_checkboxes_reflecting_selection() {
        let state = WorkspaceState {
            focus_mode: FocusMode::Overlay,
            active_overlay: OverlayKind::GitHubPicker,
            github_picker: crate::state::GitHubPickerState {
                repositories: vec![
                    PickerRow {
                        id: "owner/repo-one".into(),
                        label: "owner/repo-one".into(),
                        selected: true,
                    },
                    PickerRow {
                        id: "owner/repo-two".into(),
                        label: "owner/repo-two".into(),
                        selected: false,
                    },
                ],
                cursor: 0,
                status: GitHubPickerStatus::Loaded,
            },
            ..Default::default()
        };
        let text = draw(140, 30, &state, &DashboardReadModel::default());
        assert!(text.contains("[x] owner/repo-one"));
        assert!(text.contains("[ ] owner/repo-two"));
    }

    #[test]
    fn header_shows_github_connected_status() {
        let state = WorkspaceState {
            github_connection_status: IntegrationConnectionStatus::Connected,
            ..Default::default()
        };
        let text = draw(140, 30, &state, &DashboardReadModel::default());
        assert!(contains_ignoring_whitespace(&text, "GitHub: 연결됨"));
    }

    #[test]
    fn header_shows_both_slack_and_github_status_independently() {
        let state = WorkspaceState {
            slack_connection_status: IntegrationConnectionStatus::Connected,
            github_connection_status: IntegrationConnectionStatus::Failed("boom".into()),
            ..Default::default()
        };
        let text = draw(140, 30, &state, &DashboardReadModel::default());
        assert!(contains_ignoring_whitespace(&text, "Slack: 연결됨"));
        assert!(contains_ignoring_whitespace(&text, "GitHub: 연결 실패"));
    }

    #[test]
    fn command_bar_shows_a_parse_error_even_after_leaving_input_mode() {
        let state = WorkspaceState {
            focus_mode: FocusMode::Normal,
            cmd_buffer: CommandBufferState {
                last_error: Some("'nope' 채널을 찾을 수 없습니다".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        let text = draw(140, 30, &state, &DashboardReadModel::default());
        assert!(contains_ignoring_whitespace(
            &text,
            "채널을 찾을 수 없습니다"
        ));
    }
}
