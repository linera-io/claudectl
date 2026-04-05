use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::session::ClaudeSession;

pub fn render_detail_panel(frame: &mut Frame, area: Rect, session: &ClaudeSession) {
    let pid = session.pid.to_string();
    let status = session.status.to_string();
    let elapsed = session.format_elapsed();
    let model = if session.model.is_empty() {
        "-".to_string()
    } else {
        session.model.clone()
    };
    let tty = if session.tty.is_empty() {
        "-".to_string()
    } else {
        session.tty.clone()
    };
    let input_tok = format_tokens(session.total_input_tokens);
    let output_tok = format_tokens(session.total_output_tokens);
    let cache_read = format_tokens(session.cache_read_tokens);
    let cache_write = format_tokens(session.cache_write_tokens);
    let context_str = format!(
        "{} / {} ({}%)",
        format_tokens(session.context_tokens),
        format_tokens(session.context_max),
        session.context_percent() as u32
    );
    let cost = session.format_cost();
    let burn_rate = session.format_burn_rate();
    let command = if session.command_args.is_empty() {
        "claude".to_string()
    } else {
        session.command_args.clone()
    };
    let jsonl = session
        .jsonl_path
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "-".into());
    let subagents = session.subagent_count.to_string();

    let lines = vec![
        detail_line("PID", &pid),
        detail_line("Session ID", &session.session_id),
        detail_line("CWD", &session.cwd),
        detail_line("Project", &session.project_name),
        detail_line("Model", &model),
        detail_line("Status", &status),
        detail_line("TTY", &tty),
        detail_line("Elapsed", &elapsed),
        Line::from(""),
        Line::from(Span::styled(
            " Tokens",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        detail_line("  Input", &input_tok),
        detail_line("  Output", &output_tok),
        detail_line("  Cache Read", &cache_read),
        detail_line("  Cache Write", &cache_write),
        detail_line("  Context", &context_str),
        Line::from(""),
        Line::from(Span::styled(
            " Cost",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        detail_line("  Total", &cost),
        detail_line("  Burn Rate", &burn_rate),
        Line::from(""),
        detail_line("Command", &command),
        detail_line("JSONL", &jsonl),
        detail_line("Subagents", &subagents),
    ];

    let block = Block::default()
        .title(" Session Detail ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, area);
}

fn detail_line(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!(" {label:<15}"),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(value.to_string(), Style::default().fg(Color::White)),
    ])
}

fn format_tokens(n: u64) -> String {
    if n == 0 {
        return "-".to_string();
    }
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}
