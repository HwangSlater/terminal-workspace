//! Ratatui drawing functions. See `docs/01-product/screen-spec.md` for the
//! layout this implements (Phase 5 subset — see `step5.md`).

use crate::state::{
    CalendarPickerStatus, CalendarSetupField, CalendarSetupStatus, FocusMode, GitHubPickerStatus,
    GitHubSetupStatus, OverlayKind, SlackPickerStatus, SlackSetupStatus, WorkspaceState,
};
use commands::DashboardReadModel;
use domain::{IntegrationSource, NotificationItem, PresenceStatus, PriorityLevel};
use events::IntegrationConnectionStatus;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;
use registry::UiDockSlot;
use scheduler::{PomodoroMode, PomodoroSnapshot};

const MIN_WIDTH: u16 = 80;
const MIN_HEIGHT: u16 = 24;
const SIDEBAR_COLLAPSE_WIDTH: u16 = 120;
const LEFT_DOCK_WIDTH: u16 = 24;
const RIGHT_DOCK_WIDTH: u16 = 32;

/// Entry point: draws the whole frame per `state`/`model`, or the
/// too-small placeholder if the terminal is below `docs/01-product/screen-spec.md`'s
/// minimum grid size. `log_lines` backs the Log Viewer overlay (`Ctrl+4`,
/// `step19.md`) -- the most recent buffered lines, oldest first. There is
/// no permanently-visible log dock anymore (`step17.md`'s 1-content-row
/// bottom strip never showed enough to be useful); the body panels get
/// that space back. `pomodoro` backs the header's Pomodoro segment
/// (`step18.md`).
pub fn render(
    frame: &mut Frame,
    state: &WorkspaceState,
    model: &DashboardReadModel,
    log_lines: &[String],
    pomodoro: &PomodoroSnapshot,
) {
    let area = frame.size();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        render_too_small(frame, area);
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header (title / status / Pomodoro rows, step19.md)
            Constraint::Min(5),    // Body (Left/Center/Right docks)
            Constraint::Length(1), // Command bar
            Constraint::Length(1), // Footer
        ])
        .split(area);

    render_header(frame, rows[0], state, pomodoro);

    let collapse_sidebars = area.width < SIDEBAR_COLLAPSE_WIDTH;
    if collapse_sidebars {
        // Below the width where all three body panels fit side by side,
        // only one is shown at a time -- which one follows `focused_dock`
        // (already what `Tab`/`Shift+Tab`/`Ctrl+1~3` move) instead of
        // being hardcoded to the Notification panel regardless of what the
        // user last focused. Team/Calendar were previously unreachable at
        // all on a narrow terminal; this was a real usability gap, not a
        // deliberate simplification.
        match state.focused_dock {
            UiDockSlot::Left => render_team_panel(frame, rows[1], state, model),
            UiDockSlot::Right => render_calendar_panel(frame, rows[1], state, model),
            // Bottom is unreachable in practice since step19.md -- Log is
            // now a Ctrl+4 overlay, not a focusable dock -- but the match
            // stays exhaustive over `UiDockSlot`'s four variants, so it
            // falls back to Notification defensively like Center does.
            UiDockSlot::Center | UiDockSlot::Bottom => {
                render_notification_panel(frame, rows[1], state, model);
            }
        }
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
        render_calendar_panel(frame, body[2], state, model);
    }

    render_command_bar(frame, rows[2], state);
    render_footer(frame, rows[3]);

    if state.focus_mode == FocusMode::Overlay {
        match state.active_overlay {
            OverlayKind::Help => render_help_overlay(frame, area),
            OverlayKind::SlackSetup => render_slack_setup_overlay(frame, area, state),
            OverlayKind::SlackPicker => render_slack_picker_overlay(frame, area, state),
            OverlayKind::GitHubSetup => render_github_setup_overlay(frame, area, state),
            OverlayKind::GitHubPicker => render_github_picker_overlay(frame, area, state),
            OverlayKind::LogViewer => render_log_viewer_overlay(frame, area, log_lines),
            OverlayKind::CalendarSetup => render_calendar_setup_overlay(frame, area, state),
            OverlayKind::CalendarPicker => render_calendar_picker_overlay(frame, area, state),
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
                key: "Ctrl+1~3",
                description: "패널로 바로 이동 (팀/알림/캘린더)",
            },
            HelpEntry {
                key: "Ctrl+4",
                description: "로그 보기 (최근 기록 전체, 오버레이)",
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
            HelpEntry {
                key: "/pomodoro start|pause|reset [작업분] [휴식분]",
                description: "뽀모도로 타이머 시작/일시정지/재설정",
            },
            HelpEntry {
                key: "Tab",
                description: "명령어/채널 자동완성 (연속 Tab: 다음 후보)",
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
        "Calendar 연동",
        &[
            HelpEntry {
                key: "Ctrl+L",
                description: "캘린더 추가 (이름 + 비공개 iCal 주소)",
            },
            HelpEntry {
                key: "Ctrl+K",
                description: "연결된 캘린더 관리/제거",
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

/// In-app Calendar connection entry (`step12.md`, extended to a two-field
/// add flow in `step24.md`): a display label first (plain text -- it's
/// shown alongside reminders, not a secret), then the secret iCal URL
/// (masked, same treatment `render_github_setup_overlay`/
/// `render_slack_setup_overlay` give their tokens). Adds a calendar to the
/// existing set rather than replacing it -- see `Ctrl+K`'s picker overlay
/// (`render_calendar_picker_overlay`) for removal.
fn render_calendar_setup_overlay(frame: &mut Frame, area: Rect, state: &WorkspaceState) {
    let popup = centered_rect(60, 30, area);
    frame.render_widget(Clear, popup);

    let setup = &state.calendar_setup;
    let label_line = match setup.field {
        CalendarSetupField::Label => format!("이름: {}_", setup.label_input),
        CalendarSetupField::Url => format!("이름: {}", setup.label_input),
    };
    let masked: String = "*".repeat(setup.token_input.chars().count());
    let url_line = match setup.field {
        CalendarSetupField::Label => String::new(),
        CalendarSetupField::Url => format!("\nURL: {masked}"),
    };
    let status_line = match &setup.status {
        CalendarSetupStatus::Idle => match setup.field {
            CalendarSetupField::Label => {
                "이 캘린더를 부를 이름을 입력하고 Enter를 누르세요.".to_string()
            }
            CalendarSetupField::Url => "비공개 iCal 주소를 입력하고 Enter를 누르세요.".to_string(),
        },
        CalendarSetupStatus::Connecting => "연결 중...".to_string(),
        CalendarSetupStatus::Connected => "연결됨.".to_string(),
        CalendarSetupStatus::Failed(reason) => format!("연결 실패: {reason}"),
    };
    let text = format!("{label_line}{url_line}\n\n{status_line}\n\nEsc: 닫기");

    let block = Block::default()
        .title("캘린더 추가")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    frame.render_widget(
        Paragraph::new(text).block(block).wrap(Wrap { trim: true }),
        popup,
    );
}

/// Connected-calendar picker (`Ctrl+K`, `step24.md`). Mirrors
/// `render_github_picker_overlay` almost exactly (single list, no
/// channel/user section split like Slack's) -- the only real difference is
/// framing: rows start checked (`open_picker`'s population logic), since
/// unchecking means "remove" here rather than GitHub's "add this repo to
/// what I'm watching."
fn render_calendar_picker_overlay(frame: &mut Frame, area: Rect, state: &WorkspaceState) {
    let popup = centered_rect(70, 70, area);
    frame.render_widget(Clear, popup);

    let picker = &state.calendar_picker;
    let block = Block::default()
        .title("캘린더 관리 (선택 해제 후 저장 시 제거)")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    match &picker.status {
        CalendarPickerStatus::Loading => {
            frame.render_widget(Paragraph::new("불러오는 중...").block(block), popup);
            return;
        }
        CalendarPickerStatus::Failed(reason) => {
            frame.render_widget(
                Paragraph::new(format!("불러오기 실패: {reason}\n\nEsc: 닫기"))
                    .block(block)
                    .wrap(Wrap { trim: true }),
                popup,
            );
            return;
        }
        CalendarPickerStatus::Idle
        | CalendarPickerStatus::Loaded
        | CalendarPickerStatus::Saving
        | CalendarPickerStatus::Saved => {}
    }

    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(inner);

    let mut items: Vec<ListItem> = Vec::new();
    if picker.calendars.is_empty() {
        items.push(ListItem::new(
            "  (연결된 캘린더가 없습니다 — Ctrl+L로 추가하세요)",
        ));
    }
    for (i, row) in picker.calendars.iter().enumerate() {
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
        CalendarPickerStatus::Saving => "저장 중...",
        CalendarPickerStatus::Saved => "저장됨!",
        _ => "j/k: 이동  Space: 선택/해제  Enter: 저장  Esc: 닫기",
    };
    frame.render_widget(
        Paragraph::new(status_line).style(Style::default().fg(Color::DarkGray)),
        layout[1],
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

/// Mirrors the semantic colors `connection_status_label` already
/// established for Slack/GitHub/Calendar connection status (`step19.md`) --
/// the Team panel is the one place presence is actually listed and it
/// previously rendered every status in the same plain color, unlike the
/// header. `Away`/`Meeting`/`Lunch` all share Yellow ("stepped away, not
/// gone") -- a fifth distinct color for that nuance wasn't worth it.
fn presence_status_color(status: PresenceStatus) -> Color {
    match status {
        PresenceStatus::Active => Color::Green,
        PresenceStatus::Away | PresenceStatus::Meeting | PresenceStatus::Lunch => Color::Yellow,
        PresenceStatus::Offline => Color::DarkGray,
    }
}

/// `PriorityLevel::Medium` deliberately keeps the default (unstyled) color
/// rather than getting its own -- most notifications are Medium, and
/// coloring the common case would just be visual noise; only the two
/// extremes (`step19.md`) stand out.
fn priority_color(priority: PriorityLevel) -> Color {
    match priority {
        PriorityLevel::High => Color::Red,
        PriorityLevel::Medium => Color::Reset,
        PriorityLevel::Low => Color::DarkGray,
    }
}

/// Level substrings scanned in priority order (`step19.md`) -- "WARN"
/// doesn't contain "ERROR" but the reverse check order would still be safe;
/// listed most-to-least severe for readability. Matches
/// `tracing_subscriber::fmt::layer()`'s compact, no-ANSI output format
/// (`step17.md`), which always puts the level right after the timestamp.
fn log_line_color(line: &str) -> Color {
    if line.contains("ERROR") {
        Color::Red
    } else if line.contains("WARN") {
        Color::Yellow
    } else if line.contains("DEBUG") || line.contains("TRACE") {
        Color::DarkGray
    } else {
        Color::Reset
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

/// `count` appends `" (N)"` to the title when non-zero (`step19.md`) -- a
/// quick-glance unread/item count without having to focus the panel. `0` is
/// rendered as the plain title, not `"(0)"`, to avoid permanent visual
/// noise on an empty workspace.
fn dock_block(
    title: &str,
    count: usize,
    slot: UiDockSlot,
    state: &WorkspaceState,
) -> Block<'static> {
    let focused = state.focused_dock == slot;
    let style = if focused {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    let full_title = if count > 0 {
        format!("{title} ({count})")
    } else {
        title.to_string()
    };
    Block::default()
        .title(full_title)
        .borders(Borders::ALL)
        .border_style(style)
}

/// Three fixed rows, not one (`step19.md` follow-up) -- a single
/// `Constraint::Length(1)` row with no wrap silently truncated mid-word at
/// the documented 80-column minimum terminal width (confirmed empirically:
/// `Calendar: 연` got cut off, and the trailing help/quit hint vanished
/// entirely with no `...` or other sign anything was missing). A first
/// attempt moved only Pomodoro to its own row (2 rows total) and dropped
/// the redundant help/quit hint (already in the footer), but title +
/// three statuses alone still measured 89 cells -- 9 over budget even
/// without Pomodoro. Splitting the title onto its own row instead of
/// trimming any *content* keeps every connection status fully legible at
/// the minimum size (65 cells for statuses alone, comfortable headroom)
/// rather than solving an overflow by cutting the information the row
/// exists to show. Row 1: title. Row 2: the three connection statuses.
/// Row 3: Pomodoro when active, blank otherwise.
fn render_header(
    frame: &mut Frame,
    area: Rect,
    state: &WorkspaceState,
    pomodoro: &PomodoroSnapshot,
) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);

    frame.render_widget(
        Paragraph::new(Span::styled(
            "Terminal Workspace",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        rows[0],
    );

    let (slack_text, slack_color) =
        connection_status_label("Slack", &state.slack_connection_status);
    let (github_text, github_color) =
        connection_status_label("GitHub", &state.github_connection_status);
    let (calendar_text, calendar_color) =
        connection_status_label("Calendar", &state.calendar_connection_status);

    let status_line = Line::from(vec![
        Span::styled(slack_text, Style::default().fg(slack_color)),
        Span::raw("  |  "),
        Span::styled(github_text, Style::default().fg(github_color)),
        Span::raw("  |  "),
        Span::styled(calendar_text, Style::default().fg(calendar_color)),
    ]);
    frame.render_widget(Paragraph::new(status_line), rows[1]);

    // Nothing shown while idle (never started) -- `step18.md` Decision 5.
    if let Some((pomodoro_text, pomodoro_color)) = pomodoro_label(pomodoro) {
        let pomodoro_line = Line::from(Span::styled(
            pomodoro_text,
            Style::default().fg(pomodoro_color),
        ));
        frame.render_widget(Paragraph::new(pomodoro_line), rows[2]);
    }
}

/// `🍅 24:35 (Work)` while running, `⏸` swapped in and dimmed while paused
/// (`step18.md` Decision 5) -- `None` (nothing shown) if no session has
/// ever been started, keeping the header uncluttered until the feature is
/// actually in use.
fn pomodoro_label(pomodoro: &PomodoroSnapshot) -> Option<(String, Color)> {
    if !pomodoro.has_been_started {
        return None;
    }
    let minutes = pomodoro.remaining_secs / 60;
    let seconds = pomodoro.remaining_secs % 60;
    let mode_label = match pomodoro.mode {
        PomodoroMode::Work => "Work",
        PomodoroMode::Break => "Break",
    };
    if pomodoro.is_running {
        Some((
            format!("🍅 {minutes:02}:{seconds:02} ({mode_label})"),
            Color::Green,
        ))
    } else {
        Some((
            format!("⏸ {minutes:02}:{seconds:02} ({mode_label}, 일시정지)"),
            Color::Yellow,
        ))
    }
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
    let block = dock_block("팀", model.team_presence.len(), UiDockSlot::Left, state);
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
            let selected = state.focused_dock == UiDockSlot::Left && i == state.selected_index;
            let line = Line::from(vec![
                Span::raw(format!("{} [", member.display_name)),
                // Colored regardless of selection -- REVERSED (swapped
                // fg/bg) still reads the status color as the background,
                // keeping the same at-a-glance signal either way.
                Span::styled(
                    presence_status_label(member.status),
                    Style::default().fg(presence_status_color(member.status)),
                ),
                Span::raw("]"),
            ]);
            let style = if selected {
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
    let block = dock_block(
        "알림",
        model.unread_notifications.len(),
        UiDockSlot::Center,
        state,
    );
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
            let selected = state.focused_dock == UiDockSlot::Center && i == state.selected_index;
            let line = format!("[{}] {}", integration_source_label(item.source), item.title);
            let style = if selected {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default().fg(priority_color(item.priority))
            };
            ListItem::new(line).style(style)
        })
        .collect();
    frame.render_widget(List::new(items).block(block), area);
}

/// The Calendar-sourced subset of `unread_notifications` — shared between
/// this panel's render and `crate::lib`'s `apply_pane_action` (which needs
/// the same count to know how far `j`/`k` can move the Right dock's
/// selection), so the filter is written once instead of the two call sites
/// silently drifting out of sync with each other.
pub(crate) fn calendar_notifications(model: &DashboardReadModel) -> Vec<&NotificationItem> {
    model
        .unread_notifications
        .iter()
        .filter(|n| n.source == IntegrationSource::Calendar)
        .collect()
}

/// Upcoming Calendar reminders. Was a static "not implemented" stub that
/// never got updated once Calendar actually shipped (`step12.md`) — the
/// data was already flowing into `DashboardReadModel.unread_notifications`
/// (via the shared `Projector`, same as Slack/GitHub), this panel just
/// never read it. Filters by `IntegrationSource::Calendar` rather than
/// getting its own `DashboardReadModel` field, mirroring how the
/// Notification panel already mixes all three sources together — the
/// underlying data model didn't need to change, only this render function.
fn render_calendar_panel(
    frame: &mut Frame,
    area: Rect,
    state: &WorkspaceState,
    model: &DashboardReadModel,
) {
    let items = calendar_notifications(model);
    let block = dock_block("캘린더", items.len(), UiDockSlot::Right, state);

    if items.is_empty() {
        let text = if state.calendar_connection_status == IntegrationConnectionStatus::Disconnected
        {
            "(Calendar 연동이 연결되지 않았습니다 — Ctrl+L)"
        } else {
            "(다가오는 일정이 없습니다)"
        };
        let empty = Paragraph::new(text).block(block).wrap(Wrap { trim: true });
        frame.render_widget(empty, area);
        return;
    }

    let list_items: Vec<ListItem> = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let line = format!("{} — {}", item.title, item.body);
            let style = if state.focused_dock == UiDockSlot::Right && i == state.selected_index {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            ListItem::new(line).style(style)
        })
        .collect();
    frame.render_widget(List::new(list_items).block(block), area);
}

/// Full scrollback view of the app's own `tracing` output (`Ctrl+4`,
/// `step19.md`, superseding `step17.md`'s permanently-visible 1-line
/// strip) -- a large centered overlay, the most recent lines that fit its
/// height, oldest at the top like a scrolling `tail -f`. Not scrollable yet
/// (same deferral `step17.md` Decision 2 made); always shows whatever's
/// most recent. Lines are colored by level (`log_line_color`) so an
/// ERROR/WARN actually stands out from routine INFO noise.
fn render_log_viewer_overlay(frame: &mut Frame, area: Rect, log_lines: &[String]) {
    let popup = centered_rect(80, 70, area);
    frame.render_widget(Clear, popup);

    let block = Block::default()
        .title("로그 (Esc: 닫기)")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    if log_lines.is_empty() {
        let placeholder = Paragraph::new("(아직 로그가 없습니다)")
            .block(block)
            .wrap(Wrap { trim: true });
        frame.render_widget(placeholder, popup);
        return;
    }

    let visible_rows = popup.height.saturating_sub(2).max(1) as usize;
    let start = log_lines.len().saturating_sub(visible_rows);
    let lines: Vec<Line> = log_lines[start..]
        .iter()
        .map(|line| Line::styled(line.clone(), Style::default().fg(log_line_color(line))))
        .collect();

    let paragraph = Paragraph::new(lines).block(block).wrap(Wrap { trim: true });
    frame.render_widget(paragraph, popup);
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
    if state.focus_mode != FocusMode::Input {
        frame.render_widget(Paragraph::new(""), area);
        return;
    }

    let mut spans = vec![Span::raw(format!(":{}", state.cmd_buffer.raw_text))];
    // Tab-completion hint (`step13.md`) -- extends the command bar the same
    // way the `last_error` branch above already does, rather than a new
    // layout row or overlay (Decision 3: not enough candidates at once to
    // justify either).
    if !state.cmd_buffer.autocomplete_suggestions.is_empty() {
        spans.push(Span::styled(
            "  (Tab: ",
            Style::default().fg(Color::DarkGray),
        ));
        for (i, candidate) in state.cmd_buffer.autocomplete_suggestions.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(", ", Style::default().fg(Color::DarkGray)));
            }
            let style = if state.cmd_buffer.selected_suggestion_index == Some(i) {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            spans.push(Span::styled(candidate.clone(), style));
        }
        spans.push(Span::styled(")", Style::default().fg(Color::DarkGray)));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_footer(frame: &mut Frame, area: Rect) {
    let footer = Paragraph::new(
        "Tab:다음 패널  Ctrl+1~3:포커스 이동  Ctrl+4:로그 보기  ::명령줄  ?:도움말  Ctrl+Q:종료",
    )
    .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(footer, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{CommandBufferState, PickerRow, SlackPickerState, SlackSetupState};
    use commands::DashboardReadModel;
    use domain::{MemberPresence, NotificationId, PresenceStatus, PriorityLevel, UserId};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn sample_notification_for(source: IntegrationSource, title: &str) -> NotificationItem {
        NotificationItem {
            id: NotificationId(uuid::Uuid::new_v4()),
            source,
            title: title.to_string(),
            body: String::new(),
            timestamp_ms: 0,
            priority: PriorityLevel::Medium,
            is_read: false,
            action_link: None,
        }
    }

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

    /// Scans the rendered buffer, row by row, for `needle` (whitespace
    /// stripped, same convention as [`contains_ignoring_whitespace`] -- a
    /// wide glyph's padding cell reads back as a literal space, and this
    /// must also tolerate a multi-word needle's own spaces) and returns the
    /// foreground color of the first matching character's cell -- for tests
    /// proving a color decision (`step19.md`'s presence/priority/log-level
    /// coloring), not just that some text is present. Matches the *whole*
    /// needle rather than just its first character -- popups don't cover
    /// the full screen (`centered_rect`), so header/footer text stays
    /// visible around them and a single-letter match can collide with an
    /// unrelated earlier word (e.g. the 'u' in the header's "Workspace").
    fn fg_color_of(terminal: &Terminal<TestBackend>, needle: &str) -> Color {
        let buffer = terminal.backend().buffer();
        let width = buffer.area.width.max(1) as usize;
        let needle_stripped: String = needle.chars().filter(|c| !c.is_whitespace()).collect();
        for row in buffer.content().chunks(width) {
            let mut stripped = String::new();
            let mut index_map = Vec::new(); // stripped char index -> original cell index
            for (idx, cell) in row.iter().enumerate() {
                for ch in cell.symbol().chars().filter(|c| !c.is_whitespace()) {
                    stripped.push(ch);
                    index_map.push(idx);
                }
            }
            if let Some(byte_pos) = stripped.find(&needle_stripped) {
                let char_idx = stripped[..byte_pos].chars().count();
                return row[index_map[char_idx]].fg;
            }
        }
        panic!("'{needle}' not found in rendered buffer");
    }

    fn draw(width: u16, height: u16, state: &WorkspaceState, model: &DashboardReadModel) -> String {
        draw_with_logs(width, height, state, model, &[])
    }

    /// Like [`draw`], but with real log panel content (`step17.md`) --
    /// kept separate rather than adding a `log_lines` parameter to every
    /// one of `draw`'s 37 existing call sites, almost none of which care
    /// about log panel content.
    fn draw_with_logs(
        width: u16,
        height: u16,
        state: &WorkspaceState,
        model: &DashboardReadModel,
        log_lines: &[String],
    ) -> String {
        draw_with_logs_and_pomodoro(
            width,
            height,
            state,
            model,
            log_lines,
            &PomodoroSnapshot::default(),
        )
    }

    /// Like [`draw_with_logs`], but with a real (non-idle) Pomodoro
    /// snapshot (`step18.md`) -- same "keep the common helper's signature
    /// small" reasoning as `draw_with_logs` itself.
    fn draw_with_logs_and_pomodoro(
        width: u16,
        height: u16,
        state: &WorkspaceState,
        model: &DashboardReadModel,
        log_lines: &[String],
        pomodoro: &PomodoroSnapshot,
    ) -> String {
        buffer_text(&draw_terminal(
            width, height, state, model, log_lines, pomodoro,
        ))
    }

    /// Like [`draw_with_logs_and_pomodoro`], but returns the `Terminal`
    /// itself rather than its flattened text -- for tests that need to
    /// inspect cell styles (`fg_color_of`), not just which characters
    /// rendered where (`step19.md`).
    fn draw_terminal(
        width: u16,
        height: u16,
        state: &WorkspaceState,
        model: &DashboardReadModel,
        log_lines: &[String],
        pomodoro: &PomodoroSnapshot,
    ) -> Terminal<TestBackend> {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(frame, state, model, log_lines, pomodoro))
            .unwrap();
        terminal
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
    fn collapses_to_a_single_panel_below_120_columns() {
        // Center focus (Ctrl+2) shows Notification -- proven separately
        // from the default-focus case below since WorkspaceState::default()
        // focuses Left, not Center.
        let state = WorkspaceState {
            focused_dock: UiDockSlot::Center,
            ..Default::default()
        };
        let text = draw(100, 30, &state, &DashboardReadModel::default());
        assert!(!contains_ignoring_whitespace(&text, "아직 팀원이 없습니다"));
        assert!(contains_ignoring_whitespace(&text, "알림"));
    }

    #[test]
    fn collapsed_panel_follows_focused_dock_instead_of_always_showing_notifications() {
        // Team/Calendar were previously unreachable at all below 120
        // columns -- this is the actual bug fix, not just a rename of the
        // test above. Tab/Shift+Tab/Ctrl+1~4 already move `focused_dock`;
        // the collapsed body now honors it instead of ignoring it.
        let team_state = WorkspaceState {
            focused_dock: UiDockSlot::Left,
            ..Default::default()
        };
        let team_text = draw(100, 30, &team_state, &DashboardReadModel::default());
        assert!(contains_ignoring_whitespace(
            &team_text,
            "아직 팀원이 없습니다"
        ));

        let calendar_state = WorkspaceState {
            focused_dock: UiDockSlot::Right,
            ..Default::default()
        };
        let calendar_text = draw(100, 30, &calendar_state, &DashboardReadModel::default());
        assert!(contains_ignoring_whitespace(&calendar_text, "Ctrl+L"));
    }

    #[test]
    fn collapsed_bottom_focus_falls_back_to_notification_panel() {
        // Unreachable via real keyboard input since step19.md (Bottom
        // dropped out of DOCK_CYCLE, Ctrl+4 opens the Log Viewer overlay
        // directly) -- this proves the defensive fallback still holds if
        // `focused_dock` is ever `Bottom` regardless.
        let state = WorkspaceState {
            focused_dock: UiDockSlot::Bottom,
            ..Default::default()
        };
        let text = draw(100, 30, &state, &DashboardReadModel::default());
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
        assert!(contains_ignoring_whitespace(&text, "연결되지"));
        assert!(contains_ignoring_whitespace(&text, "않았습니다"));
    }

    #[test]
    fn calendar_panel_shows_upcoming_reminders() {
        let model = DashboardReadModel {
            unread_notifications: vec![
                sample_notification_for(IntegrationSource::Slack, "slack msg"),
                sample_notification_for(IntegrationSource::Calendar, "Design Review"),
            ],
            ..Default::default()
        };
        let state = WorkspaceState {
            focused_dock: UiDockSlot::Right,
            selected_index: 0,
            ..Default::default()
        };
        // The Notification panel legitimately shows the Slack item too (it
        // mixes every source by design) -- a whole-screen "must not
        // contain 'slack msg'" assertion would be checking the wrong
        // panel. The actual per-source isolation is proven directly
        // against the filter below, not by scraping rendered text.
        let text = draw(140, 30, &state, &model);
        assert!(contains_ignoring_whitespace(&text, "Design Review"));
    }

    #[test]
    fn calendar_notifications_filters_out_other_sources() {
        let model = DashboardReadModel {
            unread_notifications: vec![
                sample_notification_for(IntegrationSource::Slack, "slack msg"),
                sample_notification_for(IntegrationSource::GitHub, "github pr"),
                sample_notification_for(IntegrationSource::Calendar, "Design Review"),
            ],
            ..Default::default()
        };
        let filtered = calendar_notifications(&model);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].title, "Design Review");
    }

    #[test]
    fn calendar_panel_reports_disconnected_when_not_connected() {
        let state = WorkspaceState {
            calendar_connection_status: IntegrationConnectionStatus::Disconnected,
            ..Default::default()
        };
        let text = draw(140, 30, &state, &DashboardReadModel::default());
        assert!(contains_ignoring_whitespace(&text, "Ctrl+L"));
    }

    /// Real content only ever shows inside the Log Viewer overlay
    /// (`Ctrl+4`, `step19.md`) now -- there's no permanently-visible log
    /// dock anymore. This is the actual regression this redesign must
    /// guard against: the old placeholder text must NOT leak into a
    /// perfectly ordinary Normal-mode frame.
    #[test]
    fn log_viewer_overlay_does_not_render_in_normal_mode() {
        let text = draw(
            140,
            30,
            &WorkspaceState::default(),
            &DashboardReadModel::default(),
        );
        assert!(!contains_ignoring_whitespace(&text, "아직 로그가 없습니다"));
    }

    fn log_viewer_state() -> WorkspaceState {
        WorkspaceState {
            focus_mode: FocusMode::Overlay,
            active_overlay: OverlayKind::LogViewer,
            ..Default::default()
        }
    }

    #[test]
    fn log_viewer_overlay_shows_empty_state_text_when_no_lines_buffered_yet() {
        let text = draw(140, 30, &log_viewer_state(), &DashboardReadModel::default());
        assert!(contains_ignoring_whitespace(&text, "아직 로그가 없습니다"));
    }

    #[test]
    fn log_viewer_overlay_shows_real_buffered_lines() {
        let log_lines = vec!["Slack adapter started.".to_string()];
        let text = draw_with_logs(
            140,
            30,
            &log_viewer_state(),
            &DashboardReadModel::default(),
            &log_lines,
        );
        assert!(contains_ignoring_whitespace(
            &text,
            "Slack adapter started."
        ));
        assert!(!contains_ignoring_whitespace(&text, "아직 로그가 없습니다"));
    }

    #[test]
    fn log_viewer_overlay_shows_only_the_most_recent_lines_that_fit() {
        // Zero-padded so no line number is a literal prefix substring of
        // another (unlike plain "1"/"10") -- the overlay is tall enough
        // now (`step19.md`, 70% of terminal height) to show many lines at
        // once, so the old single-digit dodge from the 1-content-row era
        // no longer holds across a wide enough number range.
        let log_lines: Vec<String> = (1..=50).map(|n| format!("log line {n:04}")).collect();
        let text = draw_with_logs(
            140,
            24,
            &log_viewer_state(),
            &DashboardReadModel::default(),
            &log_lines,
        );
        assert!(contains_ignoring_whitespace(&text, "log line 0050"));
        assert!(!contains_ignoring_whitespace(&text, "log line 0001"));
    }

    #[test]
    fn log_viewer_overlay_colors_an_error_line_differently_from_an_info_line() {
        let log_lines = vec![
            "2026-01-01T00:00:00Z INFO routine startup message".to_string(),
            "2026-01-01T00:00:01Z ERROR something broke".to_string(),
        ];
        let terminal = draw_terminal(
            140,
            30,
            &log_viewer_state(),
            &DashboardReadModel::default(),
            &log_lines,
            &PomodoroSnapshot::default(),
        );
        let info_color = fg_color_of(&terminal, "routine");
        let error_color = fg_color_of(&terminal, "something");
        assert_eq!(error_color, Color::Red);
        assert_ne!(info_color, error_color);
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
    fn team_panel_colors_presence_status_like_the_header_does() {
        let model = DashboardReadModel {
            team_presence: vec![
                MemberPresence {
                    user_id: UserId("u1".into()),
                    display_name: "Alice".into(),
                    status: PresenceStatus::Active,
                    custom_status_text: None,
                    last_updated_ms: 0,
                },
                MemberPresence {
                    user_id: UserId("u2".into()),
                    display_name: "Bob".into(),
                    status: PresenceStatus::Offline,
                    custom_status_text: None,
                    last_updated_ms: 0,
                },
            ],
            unread_notifications: Vec::new(),
        };
        let terminal = draw_terminal(
            140,
            30,
            &WorkspaceState::default(),
            &model,
            &[],
            &PomodoroSnapshot::default(),
        );
        assert_eq!(fg_color_of(&terminal, "활동중"), Color::Green);
        assert_eq!(fg_color_of(&terminal, "오프라인"), Color::DarkGray);
    }

    #[test]
    fn notification_panel_colors_high_priority_differently_from_low() {
        let model = DashboardReadModel {
            unread_notifications: vec![
                NotificationItem {
                    priority: PriorityLevel::High,
                    ..sample_notification_for(IntegrationSource::GitHub, "urgent thing")
                },
                NotificationItem {
                    priority: PriorityLevel::Low,
                    ..sample_notification_for(IntegrationSource::Slack, "fyi thing")
                },
            ],
            ..Default::default()
        };
        let terminal = draw_terminal(
            140,
            30,
            &WorkspaceState::default(),
            &model,
            &[],
            &PomodoroSnapshot::default(),
        );
        let high_color = fg_color_of(&terminal, "urgent");
        let low_color = fg_color_of(&terminal, "fyi");
        assert_eq!(high_color, Color::Red);
        assert_ne!(high_color, low_color);
    }

    #[test]
    fn dock_titles_show_a_count_when_non_empty_and_omit_it_when_zero() {
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
        assert!(contains_ignoring_whitespace(&text, "팀 (1)"));
        // Notification panel is empty in this model -- must stay plain
        // "알림", not the noisy "알림 (0)".
        assert!(!contains_ignoring_whitespace(&text, "알림 (0)"));
        assert!(contains_ignoring_whitespace(&text, "알림"));
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
        // Tall enough that all 6 categories' rows fit in the popup (60% of
        // height) without List clipping the later ones -- a real risk that
        // grows every time a category is added, unlike Paragraph/Wrap
        // overflow which at least still occupies buffer rows.
        let text = draw(140, 55, &state, &DashboardReadModel::default());
        for category in [
            "탐색",
            "명령줄",
            "Slack 연동",
            "GitHub 연동",
            "Calendar 연동",
            "기타",
        ] {
            assert!(
                contains_ignoring_whitespace(&text, category),
                "missing help category: {category}"
            );
        }
        // A representative entry from each category, proving rows still
        // show up under their heading, not just the headings themselves.
        assert!(contains_ignoring_whitespace(&text, "Ctrl+S"));
        assert!(contains_ignoring_whitespace(&text, "Ctrl+G"));
        assert!(contains_ignoring_whitespace(&text, "Ctrl+L"));
    }

    /// `/pomodoro` was a real, shipped command (`step18.md`) that the help
    /// overlay never mentioned -- the actual gap `step19.md` fixes.
    #[test]
    fn help_overlay_documents_the_pomodoro_command() {
        let state = WorkspaceState {
            focus_mode: FocusMode::Overlay,
            ..Default::default()
        };
        let text = draw(140, 55, &state, &DashboardReadModel::default());
        assert!(contains_ignoring_whitespace(&text, "/pomodoro"));
        assert!(contains_ignoring_whitespace(&text, "Ctrl+4"));
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

    /// Regression test for a real bug found by manual review: at the
    /// documented minimum terminal size, the old single-row header (no
    /// wrap) silently truncated mid-word -- `Calendar`'s status got cut off
    /// and the header's own trailing help/quit hint vanished with no `...`
    /// or other sign anything was missing. Fixed by splitting the header
    /// into two fixed rows and dropping the hint (already duplicated in
    /// the footer). This proves all three connection statuses survive
    /// intact at 80x24, the smallest size the app renders its real layout
    /// (not the placeholder) at.
    #[test]
    fn header_does_not_truncate_any_connection_status_at_minimum_terminal_size() {
        let text = draw(
            80,
            24,
            &WorkspaceState::default(),
            &DashboardReadModel::default(),
        );
        assert!(contains_ignoring_whitespace(&text, "Terminal Workspace"));
        assert!(contains_ignoring_whitespace(&text, "Slack: 연결 안 됨"));
        assert!(contains_ignoring_whitespace(&text, "GitHub: 연결 안 됨"));
        assert!(contains_ignoring_whitespace(&text, "Calendar: 연결 안 됨"));
    }

    #[test]
    fn pomodoro_label_is_none_when_never_started() {
        // Not a whole-screen text scrape: "Terminal Workspace" (the header
        // title, always present) contains "Work" as a literal substring
        // once whitespace is stripped, which would make a scraped
        // assertion for "Work" a false positive regardless of this
        // function's actual behavior -- test the function directly.
        assert!(pomodoro_label(&PomodoroSnapshot::default()).is_none());
    }

    #[test]
    fn header_shows_a_running_pomodoro_countdown() {
        let pomodoro = PomodoroSnapshot {
            mode: PomodoroMode::Work,
            session_count: 0,
            is_running: true,
            has_been_started: true,
            remaining_secs: 24 * 60 + 35,
        };
        let text = draw_with_logs_and_pomodoro(
            140,
            30,
            &WorkspaceState::default(),
            &DashboardReadModel::default(),
            &[],
            &pomodoro,
        );
        assert!(contains_ignoring_whitespace(&text, "24:35"));
        assert!(contains_ignoring_whitespace(&text, "Work"));
    }

    #[test]
    fn header_shows_a_paused_pomodoro_distinctly() {
        let pomodoro = PomodoroSnapshot {
            mode: PomodoroMode::Break,
            session_count: 1,
            is_running: false,
            has_been_started: true,
            remaining_secs: 90,
        };
        let text = draw_with_logs_and_pomodoro(
            140,
            30,
            &WorkspaceState::default(),
            &DashboardReadModel::default(),
            &[],
            &pomodoro,
        );
        assert!(contains_ignoring_whitespace(&text, "01:30"));
        assert!(contains_ignoring_whitespace(&text, "일시정지"));
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
    fn calendar_setup_overlay_renders_masked_url_not_the_raw_value() {
        let state = WorkspaceState {
            focus_mode: FocusMode::Overlay,
            active_overlay: OverlayKind::CalendarSetup,
            calendar_setup: crate::state::CalendarSetupState {
                field: CalendarSetupField::Url,
                token_input: "https://secret.example/cal.ics".to_string(),
                status: CalendarSetupStatus::Idle,
                ..Default::default()
            },
            ..Default::default()
        };
        let text = draw(140, 30, &state, &DashboardReadModel::default());
        assert!(!text.contains("https://secret.example/cal.ics"));
        assert!(contains_ignoring_whitespace(&text, "캘린더 추가"));
        let mask = "*".repeat("https://secret.example/cal.ics".chars().count());
        assert!(text.contains(&mask));
    }

    #[test]
    fn calendar_setup_overlay_shows_the_failure_reason() {
        let state = WorkspaceState {
            focus_mode: FocusMode::Overlay,
            active_overlay: OverlayKind::CalendarSetup,
            calendar_setup: crate::state::CalendarSetupState {
                status: CalendarSetupStatus::Failed("feed unreachable".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        let text = draw(140, 30, &state, &DashboardReadModel::default());
        assert!(contains_ignoring_whitespace(&text, "feed unreachable"));
    }

    #[test]
    fn header_shows_calendar_connected_status() {
        let state = WorkspaceState {
            calendar_connection_status: IntegrationConnectionStatus::Connected,
            ..Default::default()
        };
        let text = draw(140, 30, &state, &DashboardReadModel::default());
        assert!(contains_ignoring_whitespace(&text, "Calendar: 연결됨"));
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

    #[test]
    fn command_bar_shows_the_autocomplete_hint_when_suggestions_exist() {
        let state = WorkspaceState {
            focus_mode: FocusMode::Input,
            cmd_buffer: CommandBufferState {
                raw_text: "/a".to_string(),
                autocomplete_suggestions: vec!["/away".to_string(), "/active".to_string()],
                selected_suggestion_index: Some(0),
                ..Default::default()
            },
            ..Default::default()
        };
        let text = draw(140, 30, &state, &DashboardReadModel::default());
        assert!(contains_ignoring_whitespace(&text, "/away"));
        assert!(contains_ignoring_whitespace(&text, "/active"));
    }

    #[test]
    fn command_bar_shows_no_hint_when_there_are_no_suggestions() {
        let state = WorkspaceState {
            focus_mode: FocusMode::Input,
            cmd_buffer: CommandBufferState {
                raw_text: "hello".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        // "Tab:" alone isn't a safe marker -- the footer legitimately
        // shows "Tab:다음 패널" regardless of the command bar's state. The
        // hint's own opening paren is what actually distinguishes it.
        let text = draw(140, 30, &state, &DashboardReadModel::default());
        assert!(!contains_ignoring_whitespace(&text, "(Tab:"));
    }
}
