use crate::app::App;
use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, Borders, Cell, Row, Table, TableState},
    Frame,
};

pub fn render(f: &mut Frame, app: &App, area: Rect) {
    let visible = app.profiles_visible();

    // Create bordered box with centered title. While filtering, the count
    // reads [shown/total] so an over-narrow filter is obvious.
    let title = if app.profiles_filter_active || !app.profiles_filter_text.is_empty() {
        format!(
            " Profiles[{}/{}] ",
            visible.len(),
            app.available_profiles.len()
        )
    } else {
        format!(" Profiles[{}] ", app.available_profiles.len())
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(
            title,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ))
        .title_alignment(Alignment::Center);

    let inner_area = block.inner(area);
    f.render_widget(block, area);

    // A filter input line sits above the list while it is being typed; it
    // collapses once cleared so the list regains the row.
    let (filter_area, list_area) =
        if app.profiles_filter_active || !app.profiles_filter_text.is_empty() {
            let chunks = Layout::default()
                .direction(ratatui::layout::Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Min(1)])
                .split(inner_area);
            (Some(chunks[0]), chunks[1])
        } else {
            (None, inner_area)
        };

    if let Some(area) = filter_area {
        let filter_display = if app.profiles_filter_active {
            format!("/{}_", app.profiles_filter_text)
        } else {
            format!("/{}", app.profiles_filter_text)
        };
        f.render_widget(
            Span::styled(
                filter_display,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            area,
        );
    }

    let header_cells = [" PROFILE"].iter().map(|h| {
        Cell::from(*h).style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
    });

    let header = Row::new(header_cells).height(1);

    let rows = visible.iter().map(|profile| {
        let style = if profile == &app.profile {
            Style::default().fg(Color::Green)
        } else {
            Style::default()
        };

        let marker = if profile == &app.profile {
            " * "
        } else {
            "   "
        };

        Row::new(vec![
            Cell::from(format!("{}{}", marker, profile)).style(style)
        ])
    });

    let widths = [Constraint::Percentage(100)];

    let table = Table::new(rows, widths).header(header).row_highlight_style(
        Style::default()
            .bg(Color::DarkGray)
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    );

    let mut state = TableState::default();
    state.select(Some(app.profiles_selected));

    f.render_stateful_widget(table, list_area, &mut state);
}
