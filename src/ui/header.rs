use crate::app::{App, Mode};
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
// wordmark (18). Keybindings col 2 is down to its exact content width, so there
// is no slack left anywhere — every column is pinned by
// header_columns_fit_their_widest_fixed_content. Do not shave any of them.
const HEADER_CONSTRAINTS: [Constraint; 5] = [
    Constraint::Percentage(22), // Left: Context info
    Constraint::Percentage(18), // Region shortcuts (always; sub-resources live in keybindings col 1)
    Constraint::Percentage(22), // Keybindings col 1
    Constraint::Percentage(18), // Keybindings col 2
    Constraint::Percentage(20), // Logo
];

/// Logo taglines, longest first.
///
/// The logo column does not wrap, so a string wider than it is silently clipped
/// mid-word. Pick the widest variant that fits rather than trusting the
/// terminal to be roomy: the full tagline needs a 120-column terminal.
const TAGLINES: [&str; 3] = ["AWS Terminal UI explorer", "AWS Terminal UI", "AWS TUI"];

/// Widest tagline fitting `width` cells, or nothing at all if none do. A blank
/// line reads better than half a word.
fn tagline(width: u16) -> &'static str {
    TAGLINES
        .into_iter()
        .find(|text| text.chars().count() <= width as usize)
        .unwrap_or("")
}

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
        let mut context_spans = vec![
            Span::styled("Context:", Style::default().fg(Color::DarkGray)),
            Span::raw(" "),
            Span::styled(&parent.display_name, Style::default().fg(Color::Yellow)),
        ];

        if let Some(count) = parent.item.get("ResourceRecordSetCount").and_then(|v| {
            v.as_str()
                .map(|s| s.to_string())
                .or_else(|| v.as_u64().map(|n| n.to_string()))
        }) {
            context_spans.push(Span::raw(" "));
            context_spans.push(Span::styled(
                format!("({} records)", count),
                Style::default().fg(Color::DarkGray),
            ));
        }

        lines.push(Line::from(context_spans));
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
    // Always regions: the digits 0-5 switch regions on every view, so the
    // column must keep advertising them. Sub-resource shortcuts live in the
    // keybindings column instead; the 6-row header is not tall enough for
    // both lists.
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

/// Rows the header layout gives each column (`Constraint::Length(6)` in
/// ui/mod.rs). A keybindings list longer than this clips silently, so the
/// builders below treat it as a hard cap.
const HEADER_ROWS: usize = 6;

/// Build the first keybindings column for the current resource.
///
/// Sub-resource navigation lives here now: the shortcuts column always shows
/// the region list, and one header row can only hold so much, so sub-resources
/// and actions share the column and are capped to leave a row for Help.
fn resource_bindings(
    resource: &crate::resource::ResourceDef,
    readonly: bool,
    reveal_describe: bool,
) -> Vec<(String, String)> {
    let mut b: Vec<(String, String)> = vec![("<d>".to_string(), "Describe".to_string())];

    // Sub-resource shortcuts in declaration order, before actions: they are
    // navigation and every sub-resource view is reachable from here.
    for sub in resource.sub_resources.iter() {
        if b.len() >= HEADER_ROWS - 1 {
            break;
        }
        b.push((format!("<{}>", sub.shortcut), sub.display_name.clone()));
    }

    // Add resource-specific actions
    for action in resource.actions.iter() {
        // Readonly hides everything the mode blocks: confirmed actions and
        // SSM connect (an interactive shell), so no shortcut advertises
        // something pressing it would refuse.
        if readonly && (action.requires_confirm() || action.sdk_method == "ssm_connect") {
            continue;
        }
        if b.len() >= HEADER_ROWS - 1 {
            break;
        }
        if let Some(ref shortcut) = action.shortcut {
            b.push((format!("<{}>", shortcut), action.display_name.clone()));
        }
    }

    // The secret value is only revealable from its single-page (describe)
    // view, so advertise the s binding exactly there -- the list never
    // offers it, keeping the value behind an explicit enter.
    if reveal_describe {
        b.insert(0, ("<s>".to_string(), "Reveal Value".to_string()));
    }

    // Keep the list within the column, always reserving the last row for
    // Help; the reveal insert above can push a full list one row past the cap.
    while b.len() > HEADER_ROWS - 1 {
        b.pop();
    }
    b.push(("<?>".to_string(), "Help".to_string()));

    b
}

fn render_keybindings_col1(f: &mut Frame, app: &App, area: Rect) {
    // Show resource-specific actions or generic bindings
    let bindings: Vec<(String, String)> = if let Some(resource) = app.current_resource() {
        let reveal =
            app.mode == Mode::Describe && app.current_resource_key == "secretsmanager-secrets";
        resource_bindings(resource, app.readonly, reveal)
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
        tagline(area.width),
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
    use crate::app::App;
    use crate::aws::client::AwsClients;
    use crate::config::Config;
    use ratatui::backend::TestBackend;
    use ratatui::{layout::Rect, Terminal};

    fn test_app_on(resource_key: &str) -> App {
        App::from_initialized(
            AwsClients::dummy(),
            "test".to_string(),
            "eu-west-1".to_string(),
            vec!["test".to_string()],
            vec!["eu-west-1".to_string()],
            vec![],
            Config::default(),
            true,
            None,
            true,
            resource_key,
        )
    }

    /// The header is drawn with no wrapping, so clipped rows just vanish.
    /// Renders exactly the header area so buffer text is unambiguous.
    fn render_header(app: &App) -> String {
        let backend = TestBackend::new(100, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render(f, app, Rect::new(0, 0, 100, 6)))
            .expect("header must render without panicking");
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect::<String>()
    }

    /// lambda-functions has sub-resources, which used to replace the region
    /// shortcut list entirely — the digits 0-5 still switched regions there, so
    /// the header silently stopped advertising a live key. The region list must
    /// always be visible. "us-east-1" is a shortcut slot, not the current
    /// region (eu-west-1, shown by the context column).
    #[test]
    fn region_shortcuts_render_for_resources_with_sub_resources() {
        let app = test_app_on("lambda-functions");
        let rendered = render_header(&app);

        assert!(
            rendered.contains("us-east-1"),
            "region shortcuts should be visible on a sub-resource view, got: {}",
            rendered
        );
    }

    /// Moving sub-resources out of the shortcuts column must not bury them:
    /// they now live in the keybindings column, next to <d> Describe.
    #[test]
    fn sub_resource_shortcuts_render_in_the_keybindings_column() {
        let app = test_app_on("lambda-functions");
        let rendered = render_header(&app);
        let resource = crate::resource::get_resource("lambda-functions").unwrap();

        for sub in &resource.sub_resources {
            let want = format!("<{}>", sub.shortcut);
            assert!(
                rendered.contains(&want) && rendered.contains(&sub.display_name),
                "expected keybinding column to contain {:?} {}",
                want,
                sub.display_name
            );
        }
    }

    /// The keybindings column is only HEADER_ROWS tall and renders without
    /// wrapping, so an over-long list clips its last rows (today that would
    /// silently bury <?> Help). Pinned across every real resource definition,
    /// with and without the describe-only Reveal insert.
    #[test]
    fn keybinding_rows_never_exceed_the_header_height() {
        for key in crate::resource::get_all_resource_keys() {
            let resource = crate::resource::get_resource(key).unwrap();
            for reveal in [false, true] {
                let bindings = resource_bindings(resource, false, reveal);
                assert!(
                    bindings.len() <= HEADER_ROWS,
                    "{} with reveal={}: {} rows, but the header has only {}",
                    key,
                    reveal,
                    bindings.len(),
                    HEADER_ROWS
                );
            }
        }
    }

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

    /// Same clipping trap as the wordmark, but the tagline is long enough to hit
    /// it on an ordinary terminal: the full string needs 120 columns.
    #[test]
    fn tagline_always_fits_the_logo_column() {
        for terminal_width in 40..=240 {
            let logo = header_columns(Rect::new(0, 0, terminal_width, 6))[4];
            let chosen = tagline(logo.width);

            assert!(
                width(chosen) <= logo.width as usize,
                "tagline {:?} is {} cells at terminal width {}, but the logo \
                 column is only {}",
                chosen,
                width(chosen),
                terminal_width,
                logo.width
            );
        }
    }

    #[test]
    fn tagline_prefers_the_longest_that_fits() {
        assert_eq!(tagline(24), "AWS Terminal UI explorer");
        assert_eq!(tagline(23), "AWS Terminal UI");
        assert_eq!(tagline(15), "AWS Terminal UI");
        assert_eq!(tagline(14), "AWS TUI");
        assert_eq!(tagline(6), "", "nothing fits, so draw nothing");
    }

    /// The percentages are a fixed budget, so widening one column starves
    /// another and the loser clips its text without complaint. Pins the widths
    /// claimed in the comment above HEADER_CONSTRAINTS at the 100-column point
    /// where the budget is tightest.
    #[test]
    fn header_columns_fit_their_widest_fixed_content() {
        let columns = header_columns(Rect::new(0, 0, 100, 6));

        for (index, needed, content) in [
            (1, 18, "<5> ap-southeast-1"),
            (2, 22, "<c>      Connect (SSM)"),
            (3, 18, "<]>      Next Page"),
            (4, 18, "wordmark"),
        ] {
            assert!(
                columns[index].width as usize >= needed,
                "column {} is {} cells, but {:?} needs {}",
                index,
                columns[index].width,
                content,
                needed
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
