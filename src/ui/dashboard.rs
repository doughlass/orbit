//! Dashboard page renderer: a bordered 2-column grid of panels, each drawn
//! from computed `PanelData`. The AWS console's billing home is the shape:
//! stat cards, a monitor card, a monthly breakdown bar chart and a trends
//! list, all in text form.

use crate::app::App;
use crate::resource::{BreakdownData, MonitorData, PanelData, StatItem, TrendRow};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

/// Service colours for breakdown segments. Index matches the legend position;
/// anything past the top-5 folds into "Others" (the last colour).
const SEGMENT_COLORS: [Color; 6] = [
    Color::Blue,
    Color::Magenta,
    Color::Green,
    Color::Yellow,
    Color::Cyan,
    Color::DarkGray,
];

pub fn render(f: &mut Frame, app: &App, area: Rect) {
    let Some(state) = &app.dashboard_state else {
        return;
    };

    if state.panels.is_empty() {
        let loading = Paragraph::new(format!("Fetching {} data...", state.def.display_name))
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(loading, area);
        return;
    }

    // Panels flow in a 2-column grid, top-to-bottom in definition order. The
    // scroll offset drops whole rows from the top (only reachable on small
    // terminals where the grid overflows).
    let visible: Vec<(usize, &PanelData)> = state
        .panels
        .iter()
        .enumerate()
        .skip(state.scroll * 2)
        .collect();
    if visible.is_empty() {
        return;
    }
    let rows: Vec<Constraint> =
        std::iter::repeat_n(Constraint::Min(6), visible.len().div_ceil(2)).collect();
    let row_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(rows)
        .split(area);

    for (idx, (panel_idx, panel_data)) in visible.iter().enumerate() {
        let row = row_chunks[idx / 2];
        let title = state
            .def
            .panels
            .get(*panel_idx)
            .map(|p| p.title.as_str())
            .unwrap_or("");
        let cell = if idx % 2 == 0 {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(row)[0]
        } else {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(row)[1]
        };
        render_panel(f, title, panel_data, cell);
    }
}

fn render_panel(f: &mut Frame, title: &str, data: &PanelData, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(
            format!(" {} ", title),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);

    match data {
        PanelData::Stats(items) => render_stats(f, items, inner),
        PanelData::Monitor(m) => render_monitor(f, m, inner),
        PanelData::Breakdown(b) => render_breakdown(f, b, inner),
        PanelData::Trends(rows) => render_trends(f, rows, inner),
        PanelData::Error(msg) => {
            let p = Paragraph::new(vec![
                Line::from(Span::styled(
                    "failed to fetch",
                    Style::default().fg(Color::Red),
                )),
                Line::from(Span::styled(
                    msg.clone(),
                    Style::default().fg(Color::DarkGray),
                )),
            ]);
            f.render_widget(p, inner);
        }
    }
}

fn render_stats(f: &mut Frame, items: &[StatItem], area: Rect) {
    // Two stats per row: label column, then value (+ note) beside it.
    let mut lines: Vec<Line> = Vec::new();
    for pair in items.chunks(2) {
        let width = area.width as usize;
        let col = width / 2;
        let mut spans: Vec<Span> = Vec::new();
        for (i, item) in pair.iter().enumerate() {
            if i == 1 {
                spans.push(Span::raw(" ".repeat(spans_width(&spans, col))));
            }
            spans.push(Span::styled(
                pad(&item.label, 22),
                Style::default().fg(Color::DarkGray),
            ));
            spans.push(Span::styled(
                item.value.clone(),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ));
            if let Some(note) = &item.note {
                spans.push(Span::styled(
                    format!("  {note}"),
                    Style::default().fg(Color::DarkGray),
                ));
            }
        }
        lines.push(Line::from(spans));
        lines.push(Line::from(""));
    }
    let p = Paragraph::new(lines);
    f.render_widget(p, area);
}

fn render_monitor(f: &mut Frame, m: &MonitorData, area: Rect) {
    let style = if m.alert {
        Style::default().fg(Color::Red)
    } else {
        Style::default().fg(Color::Green)
    };
    let lines = vec![
        Line::from(vec![Span::styled(
            "Budgets",
            Style::default().fg(Color::DarkGray),
        )]),
        Line::from(Span::styled(m.budgets_line.clone(), style)),
        Line::from(""),
        Line::from(Span::styled(
            "Cost anomalies (MTD)",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(m.anomalies_line.clone(), style)),
    ];
    f.render_widget(Paragraph::new(lines), area);
}

fn render_breakdown(f: &mut Frame, b: &BreakdownData, area: Rect) {
    let width = area.width as usize;
    let mut lines: Vec<Line> = Vec::new();

    // Bar: label + segments + total. Segment char counts are proportional to
    // the month total so months are comparable across rows.
    let bar_width = width.saturating_sub(9 + 12).max(10);
    let max_total = b
        .months
        .iter()
        .map(|m| m.total)
        .fold(0.0_f64, f64::max)
        .max(0.01);
    for month in &b.months {
        let mut spans = vec![Span::styled(
            format!("{:>8} ", month.label),
            Style::default().fg(Color::DarkGray),
        )];
        let mut used = 0usize;
        for (idx, cost) in &month.segments {
            let fill = ((cost / max_total) * bar_width as f64).round() as usize;
            let fill = fill.min(bar_width - used);
            if fill == 0 {
                continue;
            }
            used += fill;
            spans.push(Span::styled(
                "\u{2588}".repeat(fill),
                Style::default().fg(SEGMENT_COLORS[(*idx).min(SEGMENT_COLORS.len() - 1)]),
            ));
        }
        if bar_width > used {
            spans.push(Span::raw(" ".repeat(bar_width - used)));
        }
        spans.push(Span::styled(
            format!(
                " {}",
                crate::resource::field_mapper::transform_format_money(&serde_json::json!(
                    month.total
                ))
            ),
            Style::default().fg(Color::Green),
        ));
        lines.push(Line::from(spans));
    }
    lines.push(Line::from(""));

    // Legend: coloured bullet + service name, wrapped naturally by Paragraph.
    let mut legend: Vec<Span> = Vec::new();
    for (idx, name) in b.legend.iter().enumerate() {
        if !legend.is_empty() {
            legend.push(Span::raw("  "));
        }
        legend.push(Span::styled(
            "\u{25A0} ",
            Style::default().fg(SEGMENT_COLORS[idx.min(SEGMENT_COLORS.len() - 1)]),
        ));
        legend.push(Span::styled(
            shorten(name, 28),
            Style::default().fg(Color::DarkGray),
        ));
    }
    lines.push(Line::from(legend));

    f.render_widget(Paragraph::new(lines), area);
}

fn render_trends(f: &mut Frame, rows: &[TrendRow], area: Rect) {
    let mut lines: Vec<Line> = Vec::new();
    if rows.is_empty() {
        lines.push(Line::from(Span::styled(
            "No month-over-month changes",
            Style::default().fg(Color::DarkGray),
        )));
    }
    for row in rows {
        let up = row.delta > 0.0;
        let arrow = if up { "\u{25B2}" } else { "\u{25BC}" };
        let color = if up { Color::Red } else { Color::Green };
        let mut spans = vec![
            Span::styled(format!("{arrow} "), Style::default().fg(color)),
            Span::styled(
                crate::resource::field_mapper::transform_format_money(&serde_json::json!(row
                    .delta
                    .abs()))
                .as_str()
                .unwrap_or("-")
                .to_string(),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
        ];
        if let Some(pct) = row.pct {
            spans.push(Span::styled(
                format!(" ({pct:+.1}%)"),
                Style::default().fg(Color::DarkGray),
            ));
        }
        spans.push(Span::styled(
            format!("  {}", shorten(&row.service, 30)),
            Style::default().fg(Color::DarkGray),
        ));
        lines.push(Line::from(spans));
    }
    f.render_widget(Paragraph::new(lines), area);
}

fn pad(s: &str, w: usize) -> String {
    let len = s.chars().count();
    if len >= w {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(w - len))
    }
}

fn shorten(s: &str, w: usize) -> String {
    if s.chars().count() <= w {
        s.to_string()
    } else {
        format!(
            "{}...",
            s.chars().take(w.saturating_sub(3)).collect::<String>()
        )
    }
}

fn spans_width(spans: &[Span], upto: usize) -> usize {
    let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    upto.saturating_sub(used)
}
