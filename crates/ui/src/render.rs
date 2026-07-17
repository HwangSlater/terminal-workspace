//! Ratatui drawing functions. See `docs/01-product/screen-spec.md` for the
//! layout this implements (Phase 5 subset — see `step5.md`).

use crate::state::{FocusMode, WorkspaceState};
use commands::DashboardReadModel;
use domain::{IntegrationSource, PresenceStatus};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};
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
        render_help_overlay(frame, area);
    }
}

fn render_help_overlay(frame: &mut Frame, area: Rect) {
    let popup = centered_rect(60, 50, area);
    frame.render_widget(Clear, popup);
    let text = "Tab / Shift+Tab      패널 포커스 순환\n\
                Ctrl+1~4             패널로 바로 이동 (팀/알림/캘린더/로그)\n\
                j/k, ↑/↓             선택한 패널 안에서 위아래 이동\n\
                :                    명령줄 입력\n\
                Esc                  닫기 / Normal 모드로 복귀\n\
                Ctrl+Q               종료";
    let block = Block::default()
        .title("도움말")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    frame.render_widget(Paragraph::new(text).block(block), popup);
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
        .style(Style::default().fg(Color::Red));
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

fn render_header(frame: &mut Frame, area: Rect, _state: &WorkspaceState) {
    let header = Paragraph::new(Line::from(vec![
        Span::styled(
            "Terminal Workspace",
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw("  |  도움말: ?   종료: Ctrl+Q"),
    ]));
    frame.render_widget(header, area);
}

fn render_team_panel(
    frame: &mut Frame,
    area: Rect,
    state: &WorkspaceState,
    model: &DashboardReadModel,
) {
    let block = dock_block("팀", UiDockSlot::Left, state);
    if model.team_presence.is_empty() {
        let empty = Paragraph::new("(아직 팀원이 없습니다)").block(block);
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
        let empty = Paragraph::new("(아직 알림이 없습니다)").block(block);
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
    let placeholder = Paragraph::new("(캘린더 연동이 아직 구현되지 않았습니다)").block(block);
    frame.render_widget(placeholder, area);
}

fn render_bottom_dock_placeholder(frame: &mut Frame, area: Rect) {
    let block = Block::default().title("로그").borders(Borders::ALL);
    let placeholder = Paragraph::new("(로그 스트림이 아직 연결되지 않았습니다)").block(block);
    frame.render_widget(placeholder, area);
}

fn render_command_bar(frame: &mut Frame, area: Rect, state: &WorkspaceState) {
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
    fn normal_mode_does_not_render_help_popup() {
        let text = draw(
            140,
            30,
            &WorkspaceState::default(),
            &DashboardReadModel::default(),
        );
        assert!(!contains_ignoring_whitespace(&text, OVERLAY_ONLY_TEXT));
    }
}
