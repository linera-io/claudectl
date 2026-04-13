use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::app::App;

pub fn render_status_bar(frame: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    if app.launch_mode {
        let msg = Paragraph::new(Line::from(vec![
            Span::styled(
                " new> ",
                Style::default().fg(t.success).add_modifier(Modifier::BOLD),
            ),
            Span::styled(&*app.launch_buffer, Style::default().fg(t.text_primary)),
            Span::styled("_", Style::default().fg(t.text_muted)),
        ]));
        frame.render_widget(msg, area);
    } else if app.input_mode {
        let msg = Paragraph::new(Line::from(vec![
            Span::styled(
                " > ",
                Style::default()
                    .fg(t.input_accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(&*app.input_buffer, Style::default().fg(t.text_primary)),
            Span::styled("_", Style::default().fg(t.text_muted)),
        ]));
        frame.render_widget(msg, area);
    } else if !app.status_msg.is_empty() {
        let color = if app.status_msg.starts_with("Error") {
            t.error
        } else {
            t.success
        };
        let msg = Paragraph::new(Span::styled(
            format!(" {}", app.status_msg),
            Style::default().fg(color),
        ));
        frame.render_widget(msg, area);
    } else if !app.session_recordings.is_empty() {
        let count = app.session_recordings.len();
        let names: Vec<&str> = app
            .session_recordings
            .keys()
            .filter_map(|pid| {
                app.sessions
                    .iter()
                    .find(|s| s.pid == *pid)
                    .map(|s| s.display_name())
            })
            .collect();
        let label = names.join(", ");
        let text = if count == 1 {
            format!(" REC {label}  (R to stop)")
        } else {
            format!(" REC {count} sessions: {label}  (R to stop)")
        };
        let msg = Paragraph::new(Span::styled(
            text,
            Style::default().fg(t.error).add_modifier(Modifier::BOLD),
        ));
        frame.render_widget(msg, area);
    }
}
