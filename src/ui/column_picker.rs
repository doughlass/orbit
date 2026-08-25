use crate::app::App;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

/// Column visibility picker, opened with `v` from the resource list. Mirrors
/// the AWS console "Attribute columns" preferences dialog.
pub fn render(f: &mut Frame, app: &App) {
    let area = centered_rect(50, 80, f.area());
    f.render_widget(Clear, area);

    let title = match app.current_resource() {
        Some(resource) => format!(" Column Preferences — {} ", resource.display_name),
        None => " Column Preferences ".to_string(),
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            title,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);

    let Some(resource) = app.current_resource() else {
        return;
    };

    let selected = app.column_picker_selected;
    let visible_height = chunks[0].height as usize;
    // Keep the cursor in view when the column list outgrows the popup
    let scroll = selected.saturating_sub(visible_height.saturating_sub(1));

    let lines: Vec<Line> = resource
        .columns
        .iter()
        .enumerate()
        .skip(scroll)
        .take(visible_height)
        .map(|(idx, col)| {
            let on = app.column_picker_toggles.get(idx).copied().unwrap_or(true);
            let is_cursor = idx == selected;

            let checkbox = if on { "[x]" } else { "[ ]" };
            let checkbox_style = if on {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::DarkGray)
            };

            let label_style = if is_cursor {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            let cursor = if is_cursor { "> " } else { "  " };

            Line::from(vec![
                Span::styled(format!(" {}{}", cursor, checkbox), checkbox_style),
                Span::raw(" "),
                Span::styled(col.header.clone(), label_style),
            ])
        })
        .collect();

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, chunks[0]);

    let hints = Line::from(Span::styled(
        " <j/k> move  <Space> toggle  <Enter> save  <Esc> cancel ",
        Style::default().fg(Color::DarkGray),
    ));
    let hint_para = Paragraph::new(hints).alignment(ratatui::layout::Alignment::Center);
    f.render_widget(hint_para, chunks[1]);
}

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
