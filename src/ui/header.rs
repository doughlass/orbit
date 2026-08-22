use crate::app::App;
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

/// ORBIT in half-block glyphs. Rows must stay equal width; see the tests below.
const WORDMARK: [&str; 2] = ["█▀█ █▀█ █▄▄ █ ▀█▀", "█▄█ █▀▄ █▄█ █  █ "];

// Budget is tight at 100 columns, where a percentage point is one cell. Widest
// fixed content per column: shortcuts "<5> ap-southeast-1" (18), keybindings
// col 1 "<c>      Connect (SSM)" (22), col 2 "<]>      Next Page" (18), and the
// wordmark (17). Only col 2 has slack, so that is where the logo's growth came
// from — do not shave the others without re-checking for truncation.
const HEADER_CONSTRAINTS: [Constraint; 5] = [
    Constraint::Percentage(22), // Left: Context info
    Constraint::Percentage(18), // Region/Sub-resource shortcuts
    Constraint::Percentage(22), // Keybindings col 1
    Constraint::Percentage(20), // Keybindings col 2
    Constraint::Percentage(18), // Logo
];

fn header_columns(area: Rect) -> [Rect; 5] {
    Layout::horizontal(HEADER_CONSTRAINTS).areas(area)
}

pub fn render(f: &mut Frame, app: &App, area: Rect) {
    // Split header into 5 columns like k9s
    let columns = header_columns(area);

    render_context_column(f, app, columns[0]);
    render_shortcuts_column(f, app, columns[1]);
    render_keybindings_col1(f, app, columns[2]);
    render_keybindings_col2(f, app, columns[3]);
    render_logo(f, columns[4]);
}

fn render_context_column(f: &mut Frame, app: &App, area: Rect) {
    let resource_name = app
        .current_resource()
        .map(|r| r.display_name.as_str())
        .unwrap_or(&app.current_resource_key);

    let mut lines = vec![
        Line::from(vec![
            Span::styled("Profile:", Style::default().fg(Color::DarkGray)),
            Span::raw(" "),
            Span::styled(
                &app.profile,
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Region: ", Style::default().fg(Color::DarkGray)),
            Span::raw(" "),
            Span::styled(
                &app.region,
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Resource:", Style::default().fg(Color::DarkGray)),
            Span::raw(" "),
            Span::styled(
                resource_name.to_string(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
    ];

    // Show parent context if navigating
    if let Some(parent) = &app.parent_context {
        lines.push(Line::from(vec![
            Span::styled("Context:", Style::default().fg(Color::DarkGray)),
            Span::raw(" "),
            Span::styled(&parent.display_name, Style::default().fg(Color::Yellow)),
        ]));
    }

    // Show read-only mode indicator
    if app.readonly {
        lines.push(Line::from(vec![
            Span::styled("Mode:    ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "READONLY",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
    }

    // Show custom endpoint indicator
    if app.endpoint_url.is_some() {
        lines.push(Line::from(vec![
            Span::styled("Endpoint:", Style::default().fg(Color::DarkGray)),
            Span::styled(
                " CUSTOM",
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
    }

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, area);
}

fn render_shortcuts_column(f: &mut Frame, app: &App, area: Rect) {
    // If current resource has sub-resources, show those as shortcuts
    // Otherwise show region shortcuts
    if let Some(resource) = app.current_resource() {
        if !resource.sub_resources.is_empty() {
            render_subresource_shortcuts(f, app, resource, area);
            return;
        }
    }

    render_region_shortcuts(f, app, area);
}

fn render_region_shortcuts(f: &mut Frame, app: &App, area: Rect) {
    // Default regions to fill slots when recent history is incomplete
    const DEFAULT_REGIONS: &[&str] = &[
        "us-east-1",
        "us-west-2",
        "eu-west-1",
        "eu-central-1",
        "ap-northeast-1",
        "ap-southeast-1",
    ];

    // Build region list: recent first, then defaults to fill 6 slots
    let recent = app.config.get_recent_regions();
    let mut regions: Vec<String> = recent.clone();

    // Fill remaining slots with defaults (excluding any already in the list)
    for default in DEFAULT_REGIONS {
        if regions.len() >= 6 {
            break;
        }
        if !regions.iter().any(|r| r == *default) {
            regions.push(default.to_string());
        }
    }

    let lines: Vec<Line> = regions
        .iter()
        .enumerate()
        .take(6)
        .map(|(idx, region)| {
            let is_current = region == &app.region;
            let style = if is_current {
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            Line::from(vec![
                Span::styled(format!("<{}>", idx), Style::default().fg(Color::Yellow)),
                Span::raw(" "),
                Span::styled(region.as_str(), style),
            ])
        })
        .collect();

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, area);
}

fn render_subresource_shortcuts(
    f: &mut Frame,
    _app: &App,
    resource: &crate::resource::ResourceDef,
    area: Rect,
) {
    let mut lines: Vec<Line> = vec![Line::from(Span::styled(
        "Sub-resources:",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    ))];

    for sub in resource.sub_resources.iter().take(5) {
        lines.push(Line::from(vec![
            Span::styled(
                format!("<{}>", sub.shortcut),
                Style::default().fg(Color::Yellow),
            ),
            Span::raw(" "),
            Span::styled(sub.display_name.clone(), Style::default().fg(Color::White)),
        ]));
    }

    // Show if there are more
    if resource.sub_resources.len() > 5 {
        lines.push(Line::from(Span::styled(
            format!("  +{} more", resource.sub_resources.len() - 5),
            Style::default().fg(Color::DarkGray),
        )));
    }

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, area);
}

fn render_keybindings_col1(f: &mut Frame, app: &App, area: Rect) {
    // Show resource-specific actions or generic bindings
    let bindings: Vec<(String, String)> = if let Some(resource) = app.current_resource() {
        let mut b: Vec<(String, String)> = vec![("<d>".to_string(), "Describe".to_string())];

        // Add resource-specific actions
        for action in resource.actions.iter().take(4) {
            if let Some(ref shortcut) = action.shortcut {
                b.push((format!("<{}>", shortcut), action.display_name.clone()));
            }
        }

        b.push(("<?>".to_string(), "Help".to_string()));
        b
    } else {
        vec![
            ("<d>".to_string(), "Describe".to_string()),
            ("<?>".to_string(), "Help".to_string()),
        ]
    };

    let lines: Vec<Line> = bindings
        .iter()
        .map(|(key, desc)| {
            Line::from(vec![
                Span::styled(format!("{:<9}", key), Style::default().fg(Color::Yellow)),
                Span::styled(desc.clone(), Style::default().fg(Color::DarkGray)),
            ])
        })
        .collect();

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, area);
}

fn render_keybindings_col2(f: &mut Frame, app: &App, area: Rect) {
    let mut bindings = vec![("</>", "Filter"), ("<:>", "Resources"), ("<R>", "Refresh")];

    // Add pagination shortcuts if available
    if app.pagination.has_more {
        bindings.push(("<]>", "Next Page"));
    }
    if app.pagination.current_page > 1 {
        bindings.push(("<[>", "Prev Page"));
    }

    bindings.push(("<esc>", "Back"));
    bindings.push(("<ctrl-c>", "Quit"));

    let lines: Vec<Line> = bindings
        .iter()
        .map(|(key, desc)| {
            if key.is_empty() {
                Line::from("")
            } else {
                Line::from(vec![
                    Span::styled(format!("{:<9}", key), Style::default().fg(Color::Yellow)),
                    Span::styled(*desc, Style::default().fg(Color::DarkGray)),
                ])
            }
        })
        .collect();

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, area);
}

fn render_logo(f: &mut Frame, area: Rect) {
    let brand = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);

    let mut logo: Vec<Line> = WORDMARK
        .iter()
        .map(|row| Line::from(Span::styled(*row, brand)))
        .collect();

    logo.push(Line::from(""));
    logo.push(Line::from(Span::styled(
        "AWS Terminal UI explorer",
        Style::default().fg(Color::DarkGray),
    )));
    logo.push(Line::from(Span::styled(
        crate::VERSION,
        Style::default().fg(Color::DarkGray),
    )));

    let paragraph = Paragraph::new(logo);
    f.render_widget(paragraph, area);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every block glyph is one cell wide, so char count is display width.
    fn width(row: &str) -> usize {
        row.chars().count()
    }

    /// The wordmark is drawn with no wrapping, so a logo column narrower than
    /// the art silently clips the last letter instead of failing loudly.
    #[test]
    fn logo_column_fits_the_wordmark() {
        let logo = header_columns(Rect::new(0, 0, 100, 6))[4];

        for row in WORDMARK {
            assert!(
                width(row) <= logo.width as usize,
                "wordmark row {:?} is {} cells but the logo column is only {}",
                row,
                width(row),
                logo.width
            );
        }
    }

    /// Misaligned rows are the usual symptom of a hand-edited glyph.
    #[test]
    fn wordmark_rows_are_the_same_width() {
        let widths: Vec<usize> = WORDMARK.iter().map(|row| width(row)).collect();
        assert!(
            widths.windows(2).all(|pair| pair[0] == pair[1]),
            "wordmark rows must line up, got widths {:?}",
            widths
        );
    }
}
