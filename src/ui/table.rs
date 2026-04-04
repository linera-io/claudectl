use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Wrap},
};

use crate::app::{App, SORT_COLUMNS};
use crate::session::SessionStatus;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let has_status = !app.status_msg.is_empty() || app.input_mode;
    let chunks = if !has_status {
        vec![area]
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(3), Constraint::Length(1)])
            .split(area)
            .to_vec()
    };

    // Build header with sort indicator
    let header_names = [
        "PID", "Project", "Status", "Context", "Cost", "$/hr", "Elapsed", "CPU%", "MEM", "Tokens",
    ];

    // Map sort_column index to header index:
    // 0=Status->2, 1=Context->3, 2=Cost->4, 3=$/hr->5, 4=Elapsed->6
    let sort_header_idx = match app.sort_column {
        0 => 2,  // Status
        1 => 3,  // Context
        2 => 4,  // Cost
        3 => 5,  // $/hr
        4 => 6,  // Elapsed
        _ => usize::MAX,
    };

    let header_cells = header_names
        .iter()
        .enumerate()
        .map(|(i, h)| {
            let label = if i == sort_header_idx {
                format!("{h} \u{25bc}") // ▼ sort indicator
            } else {
                (*h).to_string()
            };
            Cell::from(label)
                .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
        });

    let header = Row::new(header_cells).height(1);

    let rows = app.sessions.iter().map(|s| {
        let status_style = Style::default().fg(s.status.color());

        // Status text with auto-approve indicator
        let status_text = if app.auto_approve.contains(&s.pid) {
            format!("{}*", s.status)
        } else {
            s.status.to_string()
        };

        // Project name with subagent badge
        let project_text = if s.subagent_count > 0 {
            format!("{} +{}", s.display_name(), s.subagent_count)
        } else {
            s.display_name().to_string()
        };

        // Color context bar based on usage
        let ctx_pct = s.context_percent();
        let ctx_color = if ctx_pct > 80.0 {
            Color::Red
        } else if ctx_pct > 50.0 {
            Color::Yellow
        } else {
            Color::Green
        };

        Row::new(vec![
            Cell::from(s.pid.to_string()),
            Cell::from(project_text),
            Cell::from(status_text).style(status_style),
            Cell::from(s.format_context_bar(6)).style(Style::default().fg(ctx_color)),
            Cell::from(s.format_cost()).style(Style::default().fg(Color::Yellow)),
            Cell::from(s.format_burn_rate()).style(Style::default().fg(
                if s.burn_rate_per_hr > 10.0 { Color::Red }
                else if s.burn_rate_per_hr > 1.0 { Color::Yellow }
                else { Color::DarkGray }
            )),
            Cell::from(s.format_elapsed()),
            Cell::from(format!("{:.1}", s.cpu_percent)),
            Cell::from(s.format_mem()),
            Cell::from(s.format_tokens()),
        ])
    });

    let widths = [
        Constraint::Length(7),    // PID
        Constraint::Min(10),      // Project (flex)
        Constraint::Length(14),   // Status (wider for * indicator)
        Constraint::Length(13),   // Context bar
        Constraint::Length(8),    // Cost
        Constraint::Length(9),    // $/hr
        Constraint::Length(10),   // Elapsed
        Constraint::Length(6),    // CPU%
        Constraint::Length(5),    // MEM
        Constraint::Length(14),   // Tokens
    ];

    let count = app.sessions.len();
    let active = app.sessions
        .iter()
        .filter(|s| matches!(s.status, SessionStatus::Processing | SessionStatus::NeedsInput))
        .count();
    let total_cost: f64 = app.sessions.iter().map(|s| s.cost_usd).sum();
    let selected = app.table_state
        .selected()
        .map(|i| i + 1)
        .unwrap_or(0);

    let cost_str = if total_cost < 1.0 {
        format!("${total_cost:.2}")
    } else {
        format!("${total_cost:.1}")
    };

    let sort_name = SORT_COLUMNS[app.sort_column];

    let footer = Line::from(vec![
        Span::styled(
            format!(" {count} sessions ({active} active) "),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(
            format!("{cost_str} "),
            Style::default().fg(Color::Yellow),
        ),
        Span::styled(
            format!("[{selected}/{count}]"),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(
            format!("  q:quit j/k:nav Tab:go y:approve i:input d:kill s:sort({sort_name}) a:auto ?:help"),
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    let block = Block::default()
        .title(" claudectl ")
        .title_bottom(footer)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .row_highlight_style(
            Style::default()
                .add_modifier(Modifier::REVERSED)
                .fg(Color::White),
        )
        .highlight_symbol("\u{25b6} "); // ▶

    frame.render_stateful_widget(table, chunks[0], &mut app.table_state.clone());

    // Status / input bar
    if chunks.len() > 1 {
        if app.input_mode {
            let msg = Paragraph::new(Line::from(vec![
                Span::styled(" > ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::styled(&*app.input_buffer, Style::default().fg(Color::White)),
                Span::styled("_", Style::default().fg(Color::DarkGray)),
            ]));
            frame.render_widget(msg, chunks[1]);
        } else if !app.status_msg.is_empty() {
            let color = if app.status_msg.starts_with("Error") {
                Color::Red
            } else {
                Color::Green
            };
            let msg = Paragraph::new(Span::styled(
                format!(" {}", app.status_msg),
                Style::default().fg(color),
            ));
            frame.render_widget(msg, chunks[1]);
        }
    }

    // Feature #26: Help overlay
    if app.show_help {
        render_help_overlay(frame, area);
    }
}

/// Render a centered help popup showing all keybindings and status colors.
fn render_help_overlay(frame: &mut Frame, area: Rect) {
    let popup = centered_rect(60, 70, area);

    // Clear the area behind the popup
    frame.render_widget(Clear, popup);

    let help_lines = vec![
        Line::from(Span::styled(
            " Keybindings",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("  j/k ", Style::default().fg(Color::Yellow)),
            Span::raw("or "),
            Span::styled("Up/Down ", Style::default().fg(Color::Yellow)),
            Span::raw("  Navigate sessions"),
        ]),
        Line::from(vec![
            Span::styled("  Tab/Enter      ", Style::default().fg(Color::Yellow)),
            Span::raw("  Switch to session terminal"),
        ]),
        Line::from(vec![
            Span::styled("  y              ", Style::default().fg(Color::Yellow)),
            Span::raw("  Approve (send Enter to NeedsInput)"),
        ]),
        Line::from(vec![
            Span::styled("  i              ", Style::default().fg(Color::Yellow)),
            Span::raw("  Input mode (type text to session)"),
        ]),
        Line::from(vec![
            Span::styled("  d/x            ", Style::default().fg(Color::Yellow)),
            Span::raw("  Kill session (double-tap to confirm)"),
        ]),
        Line::from(vec![
            Span::styled("  s              ", Style::default().fg(Color::Yellow)),
            Span::raw("  Cycle sort column"),
        ]),
        Line::from(vec![
            Span::styled("  a              ", Style::default().fg(Color::Yellow)),
            Span::raw("  Toggle auto-approve (double-tap)"),
        ]),
        Line::from(vec![
            Span::styled("  r              ", Style::default().fg(Color::Yellow)),
            Span::raw("  Force refresh"),
        ]),
        Line::from(vec![
            Span::styled("  ?              ", Style::default().fg(Color::Yellow)),
            Span::raw("  Toggle this help"),
        ]),
        Line::from(vec![
            Span::styled("  q/Esc          ", Style::default().fg(Color::Yellow)),
            Span::raw("  Quit"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            " Status Colors",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Needs Input ", Style::default().fg(Color::Magenta)),
            Span::raw("  Blocked on user approval/input"),
        ]),
        Line::from(vec![
            Span::styled("  Processing  ", Style::default().fg(Color::Green)),
            Span::raw("  Actively generating or running tools"),
        ]),
        Line::from(vec![
            Span::styled("  Waiting     ", Style::default().fg(Color::Yellow)),
            Span::raw("  Done responding, awaiting next prompt"),
        ]),
        Line::from(vec![
            Span::styled("  Idle        ", Style::default().fg(Color::DarkGray)),
            Span::raw("  No recent activity"),
        ]),
        Line::from(vec![
            Span::styled("  Finished    ", Style::default().fg(Color::Red)),
            Span::raw("  Process exited"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            " Indicators",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("  *  ", Style::default().fg(Color::Yellow)),
            Span::raw("after status = auto-approve enabled"),
        ]),
        Line::from(vec![
            Span::styled("  +N ", Style::default().fg(Color::Yellow)),
            Span::raw("after project = N sub-agents running"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  Press any key to dismiss",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let block = Block::default()
        .title(" Help ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let paragraph = Paragraph::new(help_lines)
        .block(block)
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, popup);
}

/// Return a centered Rect within `r` using the given percentage of width and height.
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
