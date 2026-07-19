//! Ratatui drawing functions. See `docs/01-product/screen-spec.md` for the
//! layout this implements (Phase 5 subset — see `step5.md`).

use crate::state::{
    CalendarGridStatus, CalendarPickerStatus, CalendarSetupField, CalendarSetupStatus, FocusMode,
    GitHubPickerStatus, GitHubSetupStatus, OverlayKind, SlackPickerStatus, SlackSetupStatus,
    WorkspaceState,
};
use commands::DashboardReadModel;
use domain::{IntegrationSource, NotificationItem, PresenceStatus, PriorityLevel};
use events::IntegrationConnectionStatus;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::block::Title;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;
use registry::UiDockSlot;
use scheduler::{PomodoroMode, PomodoroSnapshot};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const MIN_WIDTH: u16 = 80;
const MIN_HEIGHT: u16 = 24;
const SIDEBAR_COLLAPSE_WIDTH: u16 = 120;

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
    left_dock_width: u16,
    right_dock_width: u16,
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
        // `left_dock_width` (`config.toml`, step26.md) is a ceiling, not a
        // fixed value (step27.md): a short roster shouldn't force a wide
        // empty-looking box, and the Notification panel is already fluid
        // (`Constraint::Min(0)`), so it automatically reclaims whatever
        // width Team doesn't need.
        let team_width = team_panel_natural_width(model).min(left_dock_width).max(10);
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(team_width),
                Constraint::Min(0),
                Constraint::Length(right_dock_width),
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
            OverlayKind::CalendarRename => render_calendar_rename_overlay(frame, area, state),
            OverlayKind::CalendarGrid => render_calendar_grid_overlay(frame, area, state),
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
                key: "↑/↓",
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
                key: "/calendar-range <시간>",
                description: "캘린더 알림이 몇 시간 앞까지 보일지 변경",
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
                description: "연결된 캘린더 관리/제거/이름 변경(e)",
            },
            HelpEntry {
                key: "Ctrl+M",
                description: "달력 그리드 뷰 ([/]: 이전/다음 달)",
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
    let mut items: Vec<ListItem> = Vec::new();
    // A fixed 60%-of-screen popup was fine when this list was short, but
    // it stopped fitting once Calendar's shortcuts (`step25.md`) pushed
    // the content past that budget -- the overlay silently clipped
    // everything after the GitHub category with no scroll to reach the
    // rest. Sizing to the real content (clamped to the terminal, same as
    // `centered_rect_fixed`) means it can never truncate again as new
    // categories are added, short of the terminal itself being too small.
    let mut content_width: u16 = UnicodeWidthStr::width("도움말") as u16;
    for (i, (title, entries)) in HELP_CATEGORIES.iter().enumerate() {
        if i > 0 {
            items.push(ListItem::new(""));
        }
        content_width = content_width.max(UnicodeWidthStr::width(*title) as u16);
        items.push(ListItem::new(Span::styled(
            *title,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )));
        for entry in *entries {
            let line = format!("  {:<20} {}", entry.key, entry.description);
            content_width = content_width.max(UnicodeWidthStr::width(line.as_str()) as u16);
            items.push(ListItem::new(line));
        }
    }

    let content_height = u16::try_from(items.len()).unwrap_or(u16::MAX);
    let popup = centered_rect_fixed(
        content_width.saturating_add(4),
        content_height.saturating_add(2),
        area,
    );
    frame.render_widget(Clear, popup);

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

    // `picker.cursor` indexes the *logical* channels-then-users list, but
    // `items` also has two bold section headers interspersed -- this
    // tracks which rendered row that logical cursor actually lands on, so
    // the `ListState` below (which operates on rendered indices) can tell
    // ratatui to keep the right row in view (`step29.md`).
    let mut items: Vec<ListItem> = Vec::new();
    let mut selected_render_index = 0usize;
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
            selected_render_index = items.len();
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
            selected_render_index = items.len();
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        items.push(ListItem::new(format!("  {checkbox} {}", row.label)).style(style));
    }
    let mut list_state = ListState::default().with_selected(Some(selected_render_index));
    frame.render_stateful_widget(List::new(items), layout[0], &mut list_state);

    let status_line = match &picker.status {
        SlackPickerStatus::Saving => "저장 중...",
        SlackPickerStatus::Saved => "저장됨!",
        _ => "↑/↓: 이동  Space: 선택/해제  Enter: 저장  Esc: 닫기",
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
    // Same scrolling fix as the GitHub/Slack pickers (`step29.md`).
    let mut list_state = ListState::default().with_selected(Some(picker.cursor));
    frame.render_stateful_widget(List::new(items), layout[0], &mut list_state);

    let status_line = match &picker.status {
        CalendarPickerStatus::Saving => "저장 중...",
        CalendarPickerStatus::Saved => "저장됨!",
        _ => "↑/↓: 이동  Space: 선택/해제  e: 이름 변경  Enter: 저장  Esc: 닫기",
    };
    frame.render_widget(
        Paragraph::new(status_line).style(Style::default().fg(Color::DarkGray)),
        layout[1],
    );
}

/// Calendar rename prompt (`e` inside `Ctrl+K`'s picker, `step25.md`) --
/// mirrors the setup overlays' single-field shape, but plain text, not
/// masked (a label isn't a secret).
fn render_calendar_rename_overlay(frame: &mut Frame, area: Rect, state: &WorkspaceState) {
    let popup = centered_rect(50, 20, area);
    frame.render_widget(Clear, popup);

    let text = format!(
        "새 이름: {}_\n\n엔터로 저장, Esc: 닫기",
        state.calendar_rename.label_input
    );
    let block = Block::default()
        .title("캘린더 이름 변경")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    frame.render_widget(
        Paragraph::new(text).block(block).wrap(Wrap { trim: true }),
        popup,
    );
}

/// The local calendar day (1-31) `timestamp_ms` (epoch millis, UTC) falls
/// on, converted to the user's local time -- same "never display raw UTC"
/// reasoning `format_occurrence_time` already established. `None` only for
/// a value so far out of range it can't be represented at all, which
/// nothing in this codebase actually produces.
fn local_day_of(timestamp_ms: u64) -> Option<u32> {
    use chrono::Datelike;
    let utc = chrono::DateTime::from_timestamp_millis(i64::try_from(timestamp_ms).ok()?)?;
    Some(utc.with_timezone(&chrono::Local).day())
}

/// Month grid view (`Ctrl+M`, `step25.md`) — a real calendar grid, not the
/// right dock's flat "upcoming reminders" list. Read-only: shows which
/// days in the displayed month have at least one event (a yellow `●`
/// marker), today's real date (bold cyan, independent of the cursor), and
/// weekends in the usual red-Sunday/blue-Saturday convention (`step28.md`)
/// — and the highlighted day's events by time and title, underneath.
fn render_calendar_grid_overlay(frame: &mut Frame, area: Rect, state: &WorkspaceState) {
    use chrono::Datelike;

    // Enlarged from the original 70x80 (`step25.md`) after live use --
    // still a floating popup over the dashboard (`step26.md` Decision 1),
    // just claiming most of the screen instead of leaving a lot of the
    // underlying dashboard visible around its edges.
    let popup = centered_rect(92, 90, area);
    frame.render_widget(Clear, popup);

    let grid = &state.calendar_grid;
    let block = Block::default()
        .title(
            Title::from(Span::styled(
                format!("{}년 {}월", grid.year, grid.month),
                Style::default().add_modifier(Modifier::BOLD),
            ))
            .alignment(Alignment::Center),
        )
        .title(Title::from("Esc: 닫기").alignment(Alignment::Right))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    match &grid.status {
        CalendarGridStatus::Loading => {
            frame.render_widget(Paragraph::new("불러오는 중...").block(block), popup);
            return;
        }
        CalendarGridStatus::Failed(reason) => {
            frame.render_widget(
                Paragraph::new(format!("불러오기 실패: {reason}\n\nEsc: 닫기"))
                    .block(block)
                    .wrap(Wrap { trim: true }),
                popup,
            );
            return;
        }
        CalendarGridStatus::Idle | CalendarGridStatus::Loaded => {}
    }

    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    // Widened from the original 4-column day cell (`step25.md`) and given
    // a blank spacer row between weeks (`step27.md`) -- the grid itself
    // used to stay a small, left-aligned block even after the popup grew
    // (`step26.md`); this is what actually makes it read as bigger, not
    // just the popup around it.
    const CELL_WIDTH: u16 = 6;
    const GRID_WIDTH: u16 = CELL_WIDTH * 7;
    // 6 week rows, each followed by a blank spacer row (the last one just
    // borders the event list below, harmless).
    const GRID_HEIGHT: u16 = 12;

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),           // weekday header
            Constraint::Length(GRID_HEIGHT), // week rows + spacers
            Constraint::Length(1),           // spacer
            Constraint::Min(1),              // highlighted day's events
            Constraint::Length(1),           // status line
        ])
        .split(inner);

    // Centers the fixed-width grid within the popup's (much wider) inner
    // area instead of leaving it flush against the left edge.
    let centered_header = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(GRID_WIDTH),
            Constraint::Min(0),
        ])
        .split(layout[0])[1];
    let centered_grid = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(GRID_WIDTH),
            Constraint::Min(0),
        ])
        .split(layout[1])[1];

    // Sunday/Saturday get the same red/blue convention most calendar apps
    // use, on both the header and the day cells beneath it -- a plain
    // grid of same-colored numbers didn't read as a *calendar* so much as
    // a generic number grid.
    let header_spans: Vec<Span> = ["일", "월", "화", "수", "목", "금", "토"]
        .into_iter()
        .enumerate()
        .map(|(i, wd)| {
            let style = match i {
                0 => Style::default().fg(Color::Red),
                6 => Style::default().fg(Color::Blue),
                _ => Style::default(),
            }
            .add_modifier(Modifier::BOLD);
            Span::styled(format!(" {wd}   "), style)
        })
        .collect();
    frame.render_widget(Paragraph::new(Line::from(header_spans)), centered_header);

    let days_with_events: std::collections::HashSet<u32> = grid
        .events
        .iter()
        .filter_map(|item| local_day_of(item.timestamp_ms))
        .collect();

    let first_weekday = chrono::NaiveDate::from_ymd_opt(grid.year, grid.month, 1)
        .map_or(0, |d| d.weekday().num_days_from_sunday());
    let last_day = crate::state::days_in_month(grid.year, grid.month);

    // Real "today," not just the cursor -- distinct from the cursor day so
    // navigating away from today doesn't lose track of it, the same way a
    // real calendar app keeps today visually marked regardless of what's
    // currently selected.
    let today = chrono::Local::now().date_naive();
    let is_current_month = grid.year == today.year() && grid.month == today.month();

    let mut week_lines: Vec<Line> = Vec::new();
    let mut day = 1_i64 - i64::from(first_weekday);
    for _week in 0..6 {
        let mut spans = Vec::new();
        for weekday in 0..7u32 {
            if day < 1 || day > i64::from(last_day) {
                spans.push(Span::raw(" ".repeat(CELL_WIDTH as usize)));
            } else {
                // Safe: bounded by `1..=last_day` (a real month never
                // exceeds 31), well within u32 range.
                let d = u32::try_from(day).unwrap_or(0);
                let has_event = days_with_events.contains(&d);
                let is_today = is_current_month && d == today.day();
                let marker = if has_event { "●" } else { " " };
                let style = if d == grid.cursor_day {
                    Style::default()
                        .add_modifier(Modifier::REVERSED)
                        .add_modifier(Modifier::BOLD)
                } else if is_today {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else if has_event {
                    Style::default().fg(Color::Yellow)
                } else if weekday == 0 {
                    Style::default().fg(Color::Red)
                } else if weekday == 6 {
                    Style::default().fg(Color::Blue)
                } else {
                    Style::default()
                };
                spans.push(Span::styled(format!(" {d:>2}{marker}  "), style));
            }
            day += 1;
        }
        week_lines.push(Line::from(spans));
        week_lines.push(Line::from(""));
    }
    frame.render_widget(Paragraph::new(week_lines), centered_grid);

    let day_events: Vec<&NotificationItem> = grid
        .events
        .iter()
        .filter(|item| local_day_of(item.timestamp_ms) == Some(grid.cursor_day))
        .collect();
    // Weekday name alongside the day number, so this heading reads the
    // same way a real date would ("7월 15일 (월)") instead of a bare
    // number -- and a styled bullet per event instead of a plain "-".
    let cursor_weekday_label =
        chrono::NaiveDate::from_ymd_opt(grid.year, grid.month, grid.cursor_day)
            .map(|d| {
                ["일", "월", "화", "수", "목", "금", "토"]
                    [d.weekday().num_days_from_sunday() as usize]
            })
            .unwrap_or("");
    let mut event_lines: Vec<Line> = vec![Line::from(Span::styled(
        format!("{}일 ({cursor_weekday_label}) 일정", grid.cursor_day),
        Style::default().add_modifier(Modifier::BOLD),
    ))];
    if day_events.is_empty() {
        event_lines.push(Line::from(Span::styled(
            "  일정 없음",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for item in &day_events {
            event_lines.push(Line::from(vec![
                Span::styled("  ● ", Style::default().fg(Color::Yellow)),
                Span::raw(format!("{} ", format_occurrence_clock(item.timestamp_ms))),
                Span::raw(item.title.clone()),
            ]));
        }
    }
    frame.render_widget(
        Paragraph::new(event_lines).wrap(Wrap { trim: true }),
        layout[3],
    );

    frame.render_widget(
        Paragraph::new("←/→: 날짜 이동  [/]: 이전/다음 달  Esc: 닫기")
            .style(Style::default().fg(Color::DarkGray)),
        layout[4],
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
    // Real regression fix: a plain `List::new(items)` + `render_widget`
    // never scrolls -- the cursor could move past the bottom of a long
    // list with no visual indication anything was still selected
    // (`step29.md`). A `ListState` with the cursor selected makes ratatui
    // shift the viewport to keep the highlighted row visible.
    let mut list_state = ListState::default().with_selected(Some(picker.cursor));
    frame.render_stateful_widget(List::new(items), layout[0], &mut list_state);

    let status_line = match &picker.status {
        GitHubPickerStatus::Saving => "저장 중...",
        GitHubPickerStatus::Saved => "저장됨!",
        _ => "↑/↓: 이동  Space: 선택/해제  Enter: 저장  Esc: 닫기",
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

/// Centers a popup sized to its actual content (`width`/`height` in
/// terminal columns/rows), clamped to `area` so it never claims more space
/// than the real terminal has. Unlike `centered_rect`'s percent-of-`area`
/// idiom, this is for overlays whose content size varies (the Help overlay
/// grew past a fixed 60% height once Calendar's shortcuts were added,
/// silently truncating the last category — this is the fix).
fn centered_rect_fixed(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    }
}

/// Hard-wraps `text` to `width` terminal columns, honoring wide (e.g.
/// Korean) characters via `unicode-width` rather than counting `char`s 1:1
/// -- the Calendar panel's dock width clips long event titles with no way
/// to see the rest otherwise. Breaks mid-word rather than at word
/// boundaries: this panel is narrow enough (32 columns by default, minus
/// borders) that word-wrapping would frequently orphan a single word onto
/// its own line for little readability gain, not worth the extra
/// complexity.
fn wrap_to_width(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![text.to_string()];
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;
    for ch in text.chars() {
        let ch_width = ch.width().unwrap_or(1);
        if current_width + ch_width > width && !current.is_empty() {
            lines.push(std::mem::take(&mut current));
            current_width = 0;
        }
        current.push(ch);
        current_width += ch_width;
    }
    if !current.is_empty() || lines.is_empty() {
        lines.push(current);
    }
    lines
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

/// How wide the Team dock actually needs to be for its real content
/// (`step27.md`) -- the longest `"{display_name} [{status_label}]"` line
/// (or the empty-state text), plus 2 for the block's borders. The caller
/// still clamps this against the configured `left_dock_width` (a ceiling,
/// not a target) and a floor, so this function itself doesn't need to
/// know either bound.
fn team_panel_natural_width(model: &DashboardReadModel) -> u16 {
    let content_width = if model.team_presence.is_empty() {
        UnicodeWidthStr::width("(아직 팀원이 없습니다)")
    } else {
        model
            .team_presence
            .iter()
            .map(|member| {
                UnicodeWidthStr::width(member.display_name.as_str())
                    + UnicodeWidthStr::width(presence_status_label(member.status))
                    + " [".len()
                    + "]".len()
            })
            .max()
            .unwrap_or(0)
    };
    u16::try_from(content_width + 2).unwrap_or(u16::MAX)
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
    let mut items = calendar_notifications(model);
    // Real gap found via live use: the occurrence's start time was already
    // in `NotificationItem.timestamp_ms` (`map_occurrence` populates it
    // correctly) but this panel never rendered it -- a list of reminders
    // with no visible date/time. Sorting soonest-first goes with that fix:
    // an unsorted list of dated items reads as broken, not just unordered
    // (`unread_notifications`' order is whatever order the Projector
    // received the underlying events in, not chronological).
    items.sort_by_key(|item| item.timestamp_ms);
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

    // Inner width available for text once the block's left/right borders
    // are subtracted -- ratatui's `List` doesn't wrap long lines on its
    // own, it clips them, which is exactly what was hiding the rest of
    // long event titles.
    let inner_width = area.width.saturating_sub(2).max(1) as usize;
    let list_items: Vec<ListItem> = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let when = format_occurrence_time(item.timestamp_ms);
            let full_line = format!("{when}  {}", item.title);
            let style = if state.focused_dock == UiDockSlot::Right && i == state.selected_index {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            // Dims the timestamp so the title (the part actually worth
            // scanning) stands out more -- only when the line fits on one
            // row; the wrapped case falls back to plain text, since
            // `wrap_to_width` operates on a flat string and re-deriving
            // span boundaries across wrapped rows isn't worth the
            // complexity for what's already a fallback path.
            let lines: Vec<Line> = if UnicodeWidthStr::width(full_line.as_str()) <= inner_width {
                vec![Line::from(vec![
                    Span::styled(when, Style::default().fg(Color::DarkGray)),
                    Span::raw(format!("  {}", item.title)),
                ])]
            } else {
                wrap_to_width(&full_line, inner_width)
                    .into_iter()
                    .map(Line::from)
                    .collect()
            };
            ListItem::new(Text::from(lines)).style(style)
        })
        .collect();
    frame.render_widget(List::new(list_items).block(block), area);
}

/// `"7/20 14:00"` in the user's local time (not UTC — `timestamp_ms` is
/// epoch milliseconds; a raw UTC display would silently be wrong for
/// anyone not in UTC). No year shown -- `lookahead_hours` bounds this
/// panel to the near future (default 24h, configurable), so a reminder
/// crossing a year boundary isn't a real scenario worth the extra width.
fn format_occurrence_time(timestamp_ms: u64) -> String {
    let Some(utc) =
        chrono::DateTime::from_timestamp_millis(i64::try_from(timestamp_ms).unwrap_or(i64::MAX))
    else {
        return "?".to_string();
    };
    let local = utc.with_timezone(&chrono::Local);
    local.format("%-m/%-d %H:%M").to_string()
}

/// `"14:00"` in local time -- the grid overlay's per-day event list groups
/// events under an already-visible day heading, so unlike
/// `format_occurrence_time` the date portion would be redundant.
fn format_occurrence_clock(timestamp_ms: u64) -> String {
    let Some(utc) =
        chrono::DateTime::from_timestamp_millis(i64::try_from(timestamp_ms).unwrap_or(i64::MAX))
    else {
        return "?".to_string();
    };
    utc.with_timezone(&chrono::Local)
        .format("%H:%M")
        .to_string()
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

    /// `step26.md`'s configurable dock widths' pre-`step26.md` defaults --
    /// what every test that doesn't care about custom widths should keep
    /// exercising, so this pair stays the single source of truth for
    /// "the ordinary, unconfigured layout" rather than each test call site
    /// hardcoding `24`/`32` directly.
    const DEFAULT_LEFT_DOCK_WIDTH: u16 = 24;
    const DEFAULT_RIGHT_DOCK_WIDTH: u16 = 32;

    fn draw(width: u16, height: u16, state: &WorkspaceState, model: &DashboardReadModel) -> String {
        draw_with_logs(width, height, state, model, &[])
    }

    /// Like [`draw`], but with custom dock widths (`step26.md`) -- for
    /// tests specifically about the configurable-layout feature.
    fn draw_with_dock_widths(
        width: u16,
        height: u16,
        state: &WorkspaceState,
        model: &DashboardReadModel,
        left_dock_width: u16,
        right_dock_width: u16,
    ) -> String {
        buffer_text(&draw_terminal(
            width,
            height,
            state,
            model,
            &[],
            &PomodoroSnapshot::default(),
            left_dock_width,
            right_dock_width,
        ))
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
            width,
            height,
            state,
            model,
            log_lines,
            pomodoro,
            DEFAULT_LEFT_DOCK_WIDTH,
            DEFAULT_RIGHT_DOCK_WIDTH,
        ))
    }

    /// Like [`draw_with_logs_and_pomodoro`], but returns the `Terminal`
    /// itself rather than its flattened text -- for tests that need to
    /// inspect cell styles (`fg_color_of`), not just which characters
    /// rendered where (`step19.md`).
    #[allow(clippy::too_many_arguments)]
    fn draw_terminal(
        width: u16,
        height: u16,
        state: &WorkspaceState,
        model: &DashboardReadModel,
        log_lines: &[String],
        pomodoro: &PomodoroSnapshot,
        left_dock_width: u16,
        right_dock_width: u16,
    ) -> Terminal<TestBackend> {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render(
                    frame,
                    state,
                    model,
                    log_lines,
                    pomodoro,
                    left_dock_width,
                    right_dock_width,
                )
            })
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
    fn wrap_to_width_keeps_every_character_across_the_split() {
        let text = "이것은 아주 긴 문장입니다 매우 매우 매우 길게 만든 문장";
        let wrapped = wrap_to_width(text, 10);
        assert!(wrapped.len() > 1, "expected the text to actually wrap");
        for line in &wrapped {
            assert!(
                UnicodeWidthStr::width(line.as_str()) <= 10,
                "line {line:?} exceeds the requested width"
            );
        }
        assert_eq!(wrapped.concat(), text, "wrapping must not drop characters");
    }

    #[test]
    fn wrap_to_width_leaves_short_text_on_one_line() {
        assert_eq!(wrap_to_width("short", 30), vec!["short".to_string()]);
    }

    #[test]
    fn format_occurrence_time_renders_local_month_day_and_time() {
        // 2025-01-01T09:00:00Z -- checked against a fixed UTC instant
        // (not "now") so the test doesn't depend on the machine's local
        // timezone offset for its *input*, only for confirming the
        // conversion actually happened (see the two assertions below).
        let ts = chrono::DateTime::parse_from_rfc3339("2025-01-01T09:00:00Z")
            .unwrap()
            .timestamp_millis();
        let formatted = format_occurrence_time(u64::try_from(ts).unwrap());
        // Real regression guard: this is the fix itself -- the panel
        // previously showed no date/time at all.
        assert!(formatted.contains(':'), "expected a time in {formatted:?}");
        assert!(formatted.contains('/'), "expected a date in {formatted:?}");
    }

    /// Real regression test: the Calendar panel previously rendered
    /// `"{title} — {body}"` with no date/time anywhere, discovered via live
    /// use with multiple calendars connected. Confirms the fix actually
    /// reaches the screen, not just the formatting helper in isolation.
    #[test]
    fn calendar_panel_shows_the_occurrence_time_not_just_the_title() {
        let ts = chrono::DateTime::parse_from_rfc3339("2025-06-15T14:30:00Z")
            .unwrap()
            .timestamp_millis();
        let model = DashboardReadModel {
            unread_notifications: vec![NotificationItem {
                timestamp_ms: u64::try_from(ts).unwrap(),
                ..sample_notification_for(IntegrationSource::Calendar, "Design Review")
            }],
            ..Default::default()
        };
        let text = draw(140, 30, &WorkspaceState::default(), &model);
        let expected_time = format_occurrence_time(u64::try_from(ts).unwrap());
        assert!(
            contains_ignoring_whitespace(&text, &expected_time),
            "expected the formatted time {expected_time:?} in:\n{text}"
        );
    }

    /// Proves the render function's own sort, not just that sorting is
    /// possible -- checks the *rendered buffer's* row order, since
    /// `calendar_notifications` itself (the filter, tested separately
    /// below) preserves insertion order and does no sorting on its own.
    #[test]
    fn calendar_panel_sorts_reminders_soonest_first() {
        let later = NotificationItem {
            timestamp_ms: 999_999,
            ..sample_notification_for(IntegrationSource::Calendar, "LaterEventXYZ")
        };
        let sooner = NotificationItem {
            timestamp_ms: 1,
            ..sample_notification_for(IntegrationSource::Calendar, "SoonerEventXYZ")
        };
        let model = DashboardReadModel {
            // Deliberately inserted out of chronological order.
            unread_notifications: vec![later, sooner],
            ..Default::default()
        };
        let text = draw(140, 30, &WorkspaceState::default(), &model);
        // Bare titles aren't unique enough: the Notification panel (center
        // column) also shows both Calendar-sourced items -- unsorted, and
        // positioned *before* the Calendar panel (right column) in the
        // buffer's row-major text, which would make a plain title search
        // find "LaterEventXYZ" first regardless of whether the Calendar
        // panel's own sort works. Only the Calendar panel prefixes a
        // formatted time, so pairing title with time scopes the search to
        // that panel specifically.
        let sooner_needle = format!("{}  SoonerEventXYZ", format_occurrence_time(1));
        let later_needle = format!("{}  LaterEventXYZ", format_occurrence_time(999_999));
        let sooner_pos = text
            .find(&sooner_needle)
            .unwrap_or_else(|| panic!("{sooner_needle:?} not rendered in:\n{text}"));
        let later_pos = text
            .find(&later_needle)
            .unwrap_or_else(|| panic!("{later_needle:?} not rendered in:\n{text}"));
        assert!(
            sooner_pos < later_pos,
            "expected the sooner event to render first:\n{text}"
        );
    }

    /// Real regression test, reported via live use: `List` doesn't wrap
    /// long lines on its own, it clips them at the panel's edge, so an
    /// event title that didn't fit in the Calendar dock's fixed 32-column
    /// width was simply invisible past the cutoff with no way to see the
    /// rest. Confirms the full title now reaches the buffer somewhere
    /// (wrapped across rows), not just the prefix that fit on one row.
    #[test]
    fn calendar_panel_wraps_long_titles_instead_of_clipping_them() {
        let long_title =
            "이것은 한 줄에 다 들어가지 않을 정도로 아주 아주 길게 지어진 회의 이름입니다";
        let model = DashboardReadModel {
            unread_notifications: vec![sample_notification_for(
                IntegrationSource::Calendar,
                long_title,
            )],
            ..Default::default()
        };
        let text = draw(140, 30, &WorkspaceState::default(), &model);
        // Checks the *tail* of the title, not the whole reassembled
        // string: the old bug clipped the line at the panel's width, so
        // the tail never reached the screen at all -- that's the real
        // regression guard. A full-string contiguous match isn't used
        // here because the flattened buffer text joins terminal *rows*,
        // and an adjoining panel's own border character lands between two
        // of the Calendar panel's wrapped rows in that flattening even
        // though the wrap itself is correct -- a `TestBackend` scraping
        // gotcha (see `buffer_text`'s doc comment), not a real bug.
        assert!(
            contains_ignoring_whitespace(&text, "회의 이름입니다"),
            "expected the tail of the long title to be visible (wrapped), not clipped:\n{text}"
        );
    }

    #[test]
    fn calendar_notifications_preserves_insertion_order_sorting_is_the_render_fns_job() {
        // Documents the boundary: the filter itself is order-preserving:
        // `render_calendar_panel` is where chronological sorting happens,
        // not here -- so this helper stays reusable by anything that wants
        // the raw filtered list in its original order (e.g. a future
        // consumer that sorts differently).
        let model = DashboardReadModel {
            unread_notifications: vec![
                sample_notification_for(IntegrationSource::Calendar, "First"),
                sample_notification_for(IntegrationSource::Calendar, "Second"),
            ],
            ..Default::default()
        };
        let items = calendar_notifications(&model);
        assert_eq!(items[0].title, "First");
        assert_eq!(items[1].title, "Second");
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
            DEFAULT_LEFT_DOCK_WIDTH,
            DEFAULT_RIGHT_DOCK_WIDTH,
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

    /// Real regression guard for `step26.md`'s configurable dock widths --
    /// proves a custom width actually reaches the rendered buffer's column
    /// boundaries, not just that `render()` accepts the parameters without
    /// panicking. Checks exact cell positions (not text search) since the
    /// thing under test *is* where a column boundary lands.
    #[test]
    fn configured_dock_widths_change_where_the_body_panels_split() {
        let (left_dock_width, right_dock_width) = (15u16, 50u16);
        let total_width = 140;
        let terminal = draw_terminal(
            total_width,
            30,
            &WorkspaceState::default(),
            &DashboardReadModel::default(),
            &[],
            &PomodoroSnapshot::default(),
            left_dock_width,
            right_dock_width,
        );
        let buffer = terminal.backend().buffer();
        // Header is `Constraint::Length(3)`, so the body's top border row
        // (shared by all three panels) is row 3.
        let body_top_row = 3;
        let notification_corner = buffer.get(left_dock_width, body_top_row).symbol();
        assert_eq!(
            notification_corner, "┌",
            "expected the Notification panel to start exactly at the configured \
             left_dock_width ({left_dock_width}), found {notification_corner:?}"
        );
        let calendar_x = total_width - right_dock_width;
        let calendar_corner = buffer.get(calendar_x, body_top_row).symbol();
        assert_eq!(
            calendar_corner, "┌",
            "expected the Calendar panel to start exactly at \
             total_width - right_dock_width ({calendar_x}), found {calendar_corner:?}"
        );
    }

    /// Real regression guard for `step27.md`'s content-sized Team dock --
    /// a short roster should render narrower than the *configured*
    /// `left_dock_width`, with the Notification panel's border shifting
    /// left to reclaim the difference (same cell-position technique as
    /// `configured_dock_widths_change_where_the_body_panels_split`, since
    /// `left_dock_width` is now a ceiling, not the exact width used).
    #[test]
    fn a_short_roster_shrinks_the_team_dock_below_the_configured_ceiling() {
        let model = DashboardReadModel {
            team_presence: vec![MemberPresence {
                user_id: UserId("u1".into()),
                display_name: "ab".into(),
                status: PresenceStatus::Active,
                custom_status_text: None,
                last_updated_ms: 0,
            }],
            unread_notifications: Vec::new(),
        };
        // "ab [활동중]" is far narrower than this configured ceiling.
        let configured_left_dock_width = 40u16;
        let total_width = 140;
        let terminal = draw_terminal(
            total_width,
            30,
            &WorkspaceState::default(),
            &model,
            &[],
            &PomodoroSnapshot::default(),
            configured_left_dock_width,
            DEFAULT_RIGHT_DOCK_WIDTH,
        );
        let buffer = terminal.backend().buffer();
        let body_top_row = 3;
        // "ab" (width 2) + " [" (2) + "활동중" (width 6) + "]" (1) + 2
        // border columns = 13 -- well short of the 40-column ceiling.
        let expected_team_width = 13;
        let notification_corner = buffer.get(expected_team_width, body_top_row).symbol();
        assert_eq!(
            notification_corner, "┌",
            "expected the Team dock to shrink to its content width ({expected_team_width}), \
             not stay at the configured ceiling ({configured_left_dock_width}); \
             found {notification_corner:?} at x={expected_team_width}"
        );
        // And *not* still sitting at the old, unshrunk ceiling.
        let stale_corner = buffer
            .get(configured_left_dock_width, body_top_row)
            .symbol();
        assert_ne!(
            stale_corner, "┌",
            "the Notification panel should have already started before \
             the configured ceiling ({configured_left_dock_width})"
        );
    }

    /// A wider configured Calendar dock should mean less wrapping
    /// (`wrap_to_width`'s regression fix from earlier this phase) -- ties
    /// the two `step26.md`-adjacent fixes together: a title that needs two
    /// lines at the default 32-column dock should fit on one at 60.
    #[test]
    fn a_wider_right_dock_width_wraps_long_titles_less() {
        let title = "이것은 서른 칸보다 길고 예순 칸보다 짧음";
        let model = DashboardReadModel {
            unread_notifications: vec![sample_notification_for(IntegrationSource::Calendar, title)],
            ..Default::default()
        };
        let narrow = draw_with_dock_widths(140, 30, &WorkspaceState::default(), &model, 24, 32);
        let wide = draw_with_dock_widths(140, 30, &WorkspaceState::default(), &model, 10, 60);
        // Scoped to the Calendar panel specifically via the time-prefixed
        // compound needle (same reasoning as
        // `calendar_panel_sorts_reminders_soonest_first`): the bare title
        // alone also renders in the wider Notification panel, which never
        // wraps at this length regardless of the Calendar dock's
        // configured width, so it isn't a useful signal on its own.
        let calendar_needle = format!("{}  {title}", format_occurrence_time(0));
        // At the default 32-column dock this title wraps, splitting the
        // tail onto a later row behind another panel's border character
        // (the same `TestBackend` flattening quirk documented on the
        // clipping-regression test above) -- so it does *not* appear
        // contiguously. At 60 columns it fits on one line and does.
        assert!(
            !contains_ignoring_whitespace(&narrow, &calendar_needle),
            "expected the title to still be wrapped (not contiguous) at the default width:\n{narrow}"
        );
        assert!(
            contains_ignoring_whitespace(&wide, &calendar_needle),
            "expected the title to fit on one line at the wider configured width:\n{wide}"
        );
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
            DEFAULT_LEFT_DOCK_WIDTH,
            DEFAULT_RIGHT_DOCK_WIDTH,
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
            DEFAULT_LEFT_DOCK_WIDTH,
            DEFAULT_RIGHT_DOCK_WIDTH,
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

    /// Real regression test, reported via live use: the overlay used to be
    /// a fixed 60%-of-terminal-height popup, which silently clipped
    /// everything after the GitHub category on any terminal shorter than
    /// ~55 rows -- ordinary terminal sizes, not an edge case. This uses a
    /// height picked to be comfortably enough for the content but well
    /// under what the old 60% formula would have needed, proving the fix
    /// sizes the popup to its actual content instead of a fraction of the
    /// screen.
    #[test]
    fn help_overlay_is_not_truncated_on_an_ordinary_sized_terminal() {
        let state = WorkspaceState {
            focus_mode: FocusMode::Overlay,
            ..Default::default()
        };
        let text = draw(140, 34, &state, &DashboardReadModel::default());
        assert!(
            contains_ignoring_whitespace(&text, "기타"),
            "last category was clipped:\n{text}"
        );
        assert!(
            contains_ignoring_whitespace(&text, "Ctrl+Q"),
            "last category's entries were clipped:\n{text}"
        );
        assert!(
            contains_ignoring_whitespace(&text, "Ctrl+M"),
            "Calendar's grid-view shortcut was clipped:\n{text}"
        );
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

    /// Real regression guard, same root cause as
    /// `github_picker_scrolls_the_viewport_to_keep_a_faroff_cursor_visible`
    /// (`step29.md`) -- but for Slack's picker specifically, since its
    /// combined channels-then-users list has two bold section headers
    /// interspersed, so the logical `cursor` index doesn't map 1:1 onto
    /// the rendered row index the way GitHub's/Calendar's flat lists do.
    /// A cursor deep in the *users* section (not just a long list) proves
    /// that index translation is correct, not just that scrolling happens
    /// at all.
    #[test]
    fn slack_picker_scrolls_to_a_faroff_cursor_in_the_users_section() {
        let channels: Vec<PickerRow> = (0..20)
            .map(|i| PickerRow {
                id: format!("C{i}"),
                label: format!("channel-{i}"),
                selected: false,
            })
            .collect();
        let users: Vec<PickerRow> = (0..10)
            .map(|i| PickerRow {
                id: format!("U{i}"),
                label: format!("user-{i}"),
                selected: false,
            })
            .collect();
        let state = WorkspaceState {
            focus_mode: FocusMode::Overlay,
            active_overlay: OverlayKind::SlackPicker,
            slack_picker: SlackPickerState {
                channels,
                users,
                cursor: 25, // logical index 25 = 5th user (20 channels + 5)
                status: SlackPickerStatus::Loaded,
            },
            ..Default::default()
        };
        let text = draw(140, 30, &state, &DashboardReadModel::default());
        assert!(
            contains_ignoring_whitespace(&text, "user-5"),
            "expected the cursor's row (5th user, far past the first screenful) \
             to have scrolled into view:\n{text}"
        );
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
    fn calendar_grid_overlay_shows_loading_state() {
        let state = WorkspaceState {
            focus_mode: FocusMode::Overlay,
            active_overlay: OverlayKind::CalendarGrid,
            calendar_grid: crate::state::CalendarGridState {
                status: CalendarGridStatus::Loading,
                ..Default::default()
            },
            ..Default::default()
        };
        let text = draw(140, 30, &state, &DashboardReadModel::default());
        assert!(contains_ignoring_whitespace(&text, "불러오는 중"));
    }

    #[test]
    fn calendar_grid_overlay_shows_the_failure_reason() {
        let state = WorkspaceState {
            focus_mode: FocusMode::Overlay,
            active_overlay: OverlayKind::CalendarGrid,
            calendar_grid: crate::state::CalendarGridState {
                status: CalendarGridStatus::Failed("network error".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        let text = draw(140, 30, &state, &DashboardReadModel::default());
        assert!(contains_ignoring_whitespace(&text, "network error"));
    }

    #[test]
    fn calendar_grid_overlay_shows_the_cursor_days_events() {
        let event = sample_notification_for(IntegrationSource::Calendar, "[회사] Design Review");
        let state = WorkspaceState {
            focus_mode: FocusMode::Overlay,
            active_overlay: OverlayKind::CalendarGrid,
            calendar_grid: crate::state::CalendarGridState {
                year: 2026,
                month: 6,
                cursor_day: 15,
                events: vec![NotificationItem {
                    timestamp_ms: u64::try_from(
                        chrono::DateTime::parse_from_rfc3339("2026-06-15T09:00:00Z")
                            .unwrap()
                            .timestamp_millis(),
                    )
                    .unwrap(),
                    ..event
                }],
                status: CalendarGridStatus::Loaded,
            },
            ..Default::default()
        };
        let text = draw(140, 30, &state, &DashboardReadModel::default());
        assert!(contains_ignoring_whitespace(&text, "Design Review"));
    }

    /// Real regression guard for `step28.md`'s calendar-app color
    /// convention -- Sunday's weekday-header label should read distinctly
    /// from the rest, not the same default color as every other day.
    #[test]
    fn calendar_grid_overlay_colors_sunday_in_the_weekday_header() {
        let state = WorkspaceState {
            focus_mode: FocusMode::Overlay,
            active_overlay: OverlayKind::CalendarGrid,
            calendar_grid: crate::state::CalendarGridState {
                year: 2026,
                month: 6,
                cursor_day: 15,
                events: Vec::new(),
                status: CalendarGridStatus::Loaded,
            },
            ..Default::default()
        };
        let terminal = draw_terminal(
            200,
            40,
            &state,
            &DashboardReadModel::default(),
            &[],
            &PomodoroSnapshot::default(),
            DEFAULT_LEFT_DOCK_WIDTH,
            DEFAULT_RIGHT_DOCK_WIDTH,
        );
        // "일" only appears as the Sunday header label in this scenario
        // (no events, so the day-events list below doesn't repeat it) --
        // and it's the first such glyph scanning top-to-bottom, since the
        // weekday header row sits above everything else in the popup.
        assert_eq!(fg_color_of(&terminal, "일"), Color::Red);
    }

    /// Real regression guard for `step28.md` -- the cursor day's heading
    /// now includes its weekday name, not just the bare day number, so it
    /// reads like an actual date. Computes the expected weekday the same
    /// way the production code does rather than hardcoding one, so this
    /// doesn't silently start asserting the wrong day if the fixture date
    /// ever changes.
    #[test]
    fn calendar_grid_overlay_shows_the_weekday_name_next_to_the_cursor_day() {
        let state = WorkspaceState {
            focus_mode: FocusMode::Overlay,
            active_overlay: OverlayKind::CalendarGrid,
            calendar_grid: crate::state::CalendarGridState {
                year: 2026,
                month: 6,
                cursor_day: 15,
                events: Vec::new(),
                status: CalendarGridStatus::Loaded,
            },
            ..Default::default()
        };
        let text = draw(140, 30, &state, &DashboardReadModel::default());
        use chrono::Datelike;
        let expected_weekday = ["일", "월", "화", "수", "목", "금", "토"]
            [chrono::NaiveDate::from_ymd_opt(2026, 6, 15)
                .unwrap()
                .weekday()
                .num_days_from_sunday() as usize];
        let needle = format!("15일 ({expected_weekday})");
        assert!(
            contains_ignoring_whitespace(&text, &needle),
            "expected {needle:?} in:\n{text}"
        );
    }

    /// Real regression guard for `step27.md`'s grid centering -- the
    /// weekday header (and the day-number grid beneath it) used to render
    /// flush against the popup's left edge even after the popup itself
    /// grew (`step26.md`), leaving a small block surrounded by empty
    /// space. Scans the actual buffer cells for the "일" (Sunday) label's
    /// column, rather than a byte-offset text search, since the popup's
    /// margins can show dashboard content with its own multi-byte glyphs
    /// that would throw off a naive string search.
    #[test]
    fn calendar_grid_overlay_centers_the_day_grid_instead_of_flush_left() {
        let state = WorkspaceState {
            focus_mode: FocusMode::Overlay,
            active_overlay: OverlayKind::CalendarGrid,
            calendar_grid: crate::state::CalendarGridState {
                year: 2026,
                month: 6,
                cursor_day: 15,
                events: Vec::new(),
                status: CalendarGridStatus::Loaded,
            },
            ..Default::default()
        };
        let total_width = 200;
        let terminal = draw_terminal(
            total_width,
            40,
            &state,
            &DashboardReadModel::default(),
            &[],
            &PomodoroSnapshot::default(),
            DEFAULT_LEFT_DOCK_WIDTH,
            DEFAULT_RIGHT_DOCK_WIDTH,
        );
        let buffer = terminal.backend().buffer();
        let mut header_x = None;
        'outer: for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                if buffer.get(x, y).symbol() == "일" {
                    header_x = Some(x);
                    break 'outer;
                }
            }
        }
        let header_x = header_x.expect("weekday header '일' not found in the rendered buffer");
        assert!(
            header_x > total_width / 4,
            "expected the weekday header to be horizontally centered, not flush against \
             the popup's left edge; found '일' at column {header_x}"
        );
    }

    #[test]
    fn calendar_grid_overlay_shows_no_events_text_for_a_day_with_nothing_on_it() {
        let state = WorkspaceState {
            focus_mode: FocusMode::Overlay,
            active_overlay: OverlayKind::CalendarGrid,
            calendar_grid: crate::state::CalendarGridState {
                year: 2026,
                month: 6,
                cursor_day: 15,
                events: Vec::new(),
                status: CalendarGridStatus::Loaded,
            },
            ..Default::default()
        };
        let text = draw(140, 30, &state, &DashboardReadModel::default());
        assert!(contains_ignoring_whitespace(&text, "일정 없음"));
    }

    #[test]
    fn local_day_of_converts_epoch_millis_to_the_correct_local_day() {
        let ts = chrono::DateTime::parse_from_rfc3339("2026-06-15T12:00:00Z")
            .unwrap()
            .timestamp_millis();
        // Whatever this machine's local timezone is, the result must be a
        // real day-of-month (1-31) -- the exact value depends on the
        // offset (a UTC instant near midnight can land on an adjacent
        // local day), which is the entire reason this conversion exists
        // rather than displaying raw UTC.
        let day = local_day_of(u64::try_from(ts).unwrap());
        assert!(day.is_some_and(|d| (1..=31).contains(&d)));
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

    /// Real regression guard, reported via live use with a large
    /// repository list: the picker used to render `List` statelessly, so
    /// the cursor could move past the bottom of the visible window with
    /// no scrolling at all -- the highlighted row just went off-screen
    /// (`step29.md`). A list long enough to overflow a normal popup, with
    /// the cursor near the end, should still show that row.
    #[test]
    fn github_picker_scrolls_the_viewport_to_keep_a_faroff_cursor_visible() {
        let repositories: Vec<PickerRow> = (0..30)
            .map(|i| PickerRow {
                id: format!("owner/repo-{i}"),
                label: format!("owner/repo-{i}"),
                selected: false,
            })
            .collect();
        let state = WorkspaceState {
            focus_mode: FocusMode::Overlay,
            active_overlay: OverlayKind::GitHubPicker,
            github_picker: crate::state::GitHubPickerState {
                repositories,
                cursor: 25,
                status: GitHubPickerStatus::Loaded,
            },
            ..Default::default()
        };
        let text = draw(140, 30, &state, &DashboardReadModel::default());
        assert!(
            contains_ignoring_whitespace(&text, "owner/repo-25"),
            "expected the cursor's row (far past the first screenful) to have scrolled \
             into view:\n{text}"
        );
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
