mod command_box;
mod dialog;
mod header;
mod help;
mod highlight;
mod profiles;
mod regions;
pub mod splash;

use crate::app::{App, Mode};
use crate::resource::{extract_json_value, get_color_for_value, ColumnDef};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Cell, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState,
        Table, TableState, Wrap,
    },
    Frame,
};
use std::cmp::Reverse;

pub fn render(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6), // Header (multi-line)
            Constraint::Min(1),    // Main content (table or describe)
            Constraint::Length(1), // Footer/crumb
        ])
        .split(f.area());

    // Header - multi-line with context info
    header::render(f, app, chunks[0]);

    // Main content - depends on mode and view
    match app.mode {
        Mode::Profiles => {
            profiles::render(f, app, chunks[1]);
        }
        Mode::Regions => {
            regions::render(f, app, chunks[1]);
        }
        Mode::Describe => {
            render_describe_view(f, app, chunks[1]);
        }
        Mode::LogTail => {
            render_log_tail_view(f, app, chunks[1]);
        }
        _ => {
            render_main_content(f, app, chunks[1]);
        }
    }

    // Footer/crumb
    render_crumb(f, app, chunks[2]);

    // Overlays
    match app.mode {
        Mode::Help => {
            help::render(f, app);
        }
        Mode::Confirm | Mode::Warning | Mode::SsoLogin | Mode::ConsoleLogin => {
            dialog::render(f, app);
        }
        Mode::Command => {
            command_box::render(f, app);
        }
        _ => {}
    }
}

fn render_main_content(f: &mut Frame, app: &App, area: Rect) {
    // If filter is active, has text, or has active AWS filters, show filter bar
    let show_filter = app.filter_active || !app.filter_text.is_empty() || app.aws_filters.is_some();

    if show_filter {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1)])
            .split(area);

        render_filter_bar(f, app, chunks[0]);
        render_dynamic_table(f, app, chunks[1]);
    } else {
        render_dynamic_table(f, app, area);
    }
}

fn render_filter_bar(f: &mut Frame, app: &App, area: Rect) {
    let mut spans: Vec<Span> = Vec::new();

    // Show active AWS filters if present (server-side filter)
    if let Some(filters_display) = app.aws_filters_display() {
        spans.push(Span::styled(
            format!("[{}] ", filters_display),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            "(Esc to clear)",
            Style::default().fg(Color::DarkGray),
        ));
    }

    // Show filter input if active or has text
    if app.filter_active || !app.filter_text.is_empty() {
        let cursor_style = if app.filter_active {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let filter_display = if app.filter_active {
            format!("/{}_", app.filter_text)
        } else {
            format!("/{}", app.filter_text)
        };

        spans.push(Span::styled(filter_display, cursor_style));

        // Show autocomplete hint for Filters:
        if app.filters_autocomplete_shown {
            let remaining = &"Filters: "[app.filter_text.len()..];
            spans.push(Span::styled(
                remaining.to_string(),
                Style::default().fg(Color::DarkGray),
            ));
            spans.push(Span::styled(
                " (Tab to complete)",
                Style::default().fg(Color::Cyan),
            ));
        }

        // Show hint for filters format when typing Filters:
        if app.filter_text.to_lowercase().starts_with("filters:") {
            // Show resource-specific filter hint if available
            if let Some(hint) = app.current_resource_filters_hint() {
                spans.push(Span::styled(
                    format!(" {}", hint),
                    Style::default().fg(Color::DarkGray),
                ));
            } else {
                spans.push(Span::styled(
                    " key=value, key2=value2",
                    Style::default().fg(Color::DarkGray),
                ));
            }
        }
    }

    let paragraph = Paragraph::new(Line::from(spans));
    f.render_widget(paragraph, area);
}

/// Render dynamic table based on current resource definition
fn render_dynamic_table(f: &mut Frame, app: &App, area: Rect) {
    let Some(resource) = app.current_resource() else {
        let msg = Paragraph::new("Unknown resource").style(Style::default().fg(Color::Red));
        f.render_widget(msg, area);
        return;
    };

    let query = app.filter_text.trim();
    let highlight_filter_matches = !query.is_empty();

    // Build title with count, region info, and pagination
    let title = {
        let count = app.filtered_items.len();
        let total = app.items.len();
        let is_global = resource.is_global;

        // Build pagination indicator
        let page_info = if app.pagination.has_more || app.pagination.current_page > 1 {
            format!(
                " pg.{}{}",
                app.pagination.current_page,
                if app.pagination.has_more { "+" } else { "" }
            )
        } else {
            String::new()
        };

        if is_global {
            if query.is_empty() {
                format!(" {}[{}]{} ", resource.display_name, count, page_info)
            } else {
                format!(
                    " {}[{}/{}]{} ",
                    resource.display_name, count, total, page_info
                )
            }
        } else if query.is_empty() {
            format!(
                " {}({})[{}]{} ",
                resource.display_name, app.region, count, page_info
            )
        } else {
            format!(
                " {}({})[{}/{}]{} ",
                resource.display_name, app.region, count, total, page_info
            )
        }
    };

    // Create the bordered box with centered title
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

    // Apportion the JSON column weights across the real area; see column_layout
    // for why the weights cannot go to ratatui as percentages.
    let weights: Vec<u16> = resource.columns.iter().map(|col| col.width).collect();
    let (widths, column_widths) = column_layout(inner_area.width, &weights);

    // Build header from column definitions with left padding.
    // Sorted column gets a direction arrow in cyan; the cursor column (what Tab
    // would sort) is underlined, so the two states stay tellable apart.
    let header_cells = resource.columns.iter().enumerate().map(|(col_idx, col)| {
        let is_sorted = app.sort.column == Some(col_idx);
        let (label, color) = if is_sorted {
            (
                format!(" {} {}", col.header, app.sort.indicator()),
                Color::Cyan,
            )
        } else {
            (format!(" {}", col.header), Color::Yellow)
        };
        let mut style = Style::default().fg(color).add_modifier(Modifier::BOLD);
        if app.sort.cursor == col_idx {
            style = style.add_modifier(Modifier::UNDERLINED);
        }
        Cell::from(label).style(style)
    });
    let header = Row::new(header_cells).height(1);

    // Build rows from filtered items with left padding
    let selected_row = app.selected;
    let column_widths_clone = column_widths.clone();
    let rows = app
        .filtered_items
        .iter()
        .enumerate()
        .map(|(row_index, item)| {
            let is_selected = row_index == selected_row;
            let cells = resource.columns.iter().enumerate().map(|(col_idx, col)| {
                let value = extract_json_value(item, &col.json_path);
                let mut style = get_cell_style(&value, col);
                if is_selected {
                    style = style.fg(Color::White);
                }
                let display_value = format_cell_value(&value, col);
                let col_width = column_widths_clone.get(col_idx).copied().unwrap_or(40);
                let display_value = truncate_cell(&display_value, col_width);

                if highlight_filter_matches
                    && (col.json_path == resource.name_field || col.json_path == resource.id_field)
                {
                    let match_style = Style::default()
                        .fg(Color::LightGreen)
                        .add_modifier(Modifier::BOLD);
                    highlight::fuzzy_cell(
                        &display_value,
                        query,
                        &app.fuzzy_matcher,
                        style,
                        match_style,
                    )
                } else {
                    Cell::from(format!(" {}", display_value)).style(style)
                }
            });
            Row::new(cells)
        });

    let table = Table::new(rows, widths)
        .column_spacing(TABLE_COLUMN_SPACING)
        .header(header)
        .row_highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        );

    let mut state = TableState::default();
    state.select(Some(app.selected));

    f.render_stateful_widget(table, inner_area, &mut state);
}

/// Gap ratatui inserts between table columns. Matches the `.column_spacing()`
/// set on the table in `render_dynamic_table`; both must move together or
/// `cell_text_widths` starts lying.
const TABLE_COLUMN_SPACING: u16 = 1;

/// Cells render as `format!(" {}", value)`, so one cell of every column goes to
/// the leading pad rather than the text.
const CELL_PAD: usize = 1;

/// Table column constraints and the matching room for text in each, both derived
/// from the `width` values in the resource JSON.
///
/// The JSON calls those values percentages, but only 9 of 61 resources have them
/// summing to 100 — they run from 50 to 148 — so in practice they are weights.
/// Handing them to ratatui as percentages went wrong both ways: a resource
/// summing to 50 left half the table blank, and one summing over 100
/// over-constrained the solver, which then flattened the ratios. EC2's
/// 20-weight NAME and 12-weight STATE both came out 12 cells wide at 100
/// columns, and its 16-weight PUBLIC IP came out wider than NAME.
///
/// So apportion the cells here and hand ratatui exact lengths. Fills the area
/// exactly, largest remainder first, so no cell goes unused.
///
/// Returned together because they must agree: the text widths are what
/// `truncate_cell` trims to, and a mismatch clips silently. Assumes no
/// `highlight_symbol` on the table, which is what would reserve ratatui's
/// selection column.
fn column_layout(area_width: u16, weights: &[u16]) -> (Vec<Constraint>, Vec<usize>) {
    if weights.is_empty() {
        return (Vec::new(), Vec::new());
    }

    let gaps = (weights.len() as u16 - 1) * TABLE_COLUMN_SPACING;
    let available = area_width.saturating_sub(gaps) as usize;
    let total: usize = weights.iter().map(|w| *w as usize).sum();

    let mut cells: Vec<usize> = if total == 0 {
        // Nothing to apportion by. Split evenly rather than collapsing to zero.
        vec![available / weights.len(); weights.len()]
    } else {
        weights
            .iter()
            .map(|w| available * *w as usize / total)
            .collect()
    };

    // Hand out the cells integer division dropped, biggest fractional part
    // first. Without this a 7-column table can sit several cells short of the
    // area for no visible reason.
    let mut by_remainder: Vec<usize> = (0..weights.len()).collect();
    if total > 0 {
        by_remainder.sort_by_key(|&i| Reverse(available * weights[i] as usize % total));
    }
    let leftover = available.saturating_sub(cells.iter().sum::<usize>());
    for &i in by_remainder.iter().take(leftover) {
        cells[i] += 1;
    }

    let constraints = cells
        .iter()
        .map(|c| Constraint::Length(*c as u16))
        .collect();
    let text_widths = cells.iter().map(|c| c.saturating_sub(CELL_PAD)).collect();
    (constraints, text_widths)
}

/// Cut `value` down to `width` cells, keeping the start and marking the cut with
/// a trailing "...".
///
/// Names share their prefix far less often than their suffix, so a visible head
/// tells resources apart better than a visible tail: `finance_suite_web...`
/// beats `...ance_suite_webserv`.
fn truncate_cell(value: &str, width: usize) -> String {
    if value.chars().count() <= width {
        return value.to_string();
    }
    // Below 4 cells there is no room for the marker plus a character of content,
    // so hard-cut instead. Emitting "..." anyway would overrun the column.
    if width < 4 {
        return value.chars().take(width).collect();
    }
    let kept: String = value.chars().take(width - 3).collect();
    format!("{}...", kept)
}

/// Get cell style based on value and column definition
fn get_cell_style(value: &str, col: &ColumnDef) -> Style {
    if let Some(ref color_map_name) = col.color_map {
        if let Some([r, g, b]) = get_color_for_value(color_map_name, value) {
            return Style::default().fg(Color::Rgb(r, g, b));
        }
    }
    Style::default()
}

/// Format cell value, adding indicators for transitional states
fn format_cell_value(value: &str, col: &ColumnDef) -> String {
    // Check if this is a state/status column with transitional states
    if col.color_map.is_some() {
        let lower = value.to_lowercase();
        // Transitional states get an arrow indicator
        if lower.contains("pending")
            || lower.contains("starting")
            || lower.contains("stopping")
            || lower.contains("creating")
            || lower.contains("deleting")
            || lower.contains("updating")
            || lower.contains("modifying")
            || lower.contains("provisioning")
            || lower.contains("shutting-down")
            || lower.contains("terminating")
            || lower.contains("in-progress")
            || lower.contains("initializing")
        {
            return format!("{} ↻", value);
        }
    }
    value.to_string()
}

fn describe_title(resource_display_name: &str, action_display_name: Option<&str>) -> String {
    if let Some(action) = action_display_name {
        format!(" {} ", action)
    } else {
        format!(" {} Details ", resource_display_name)
    }
}

fn render_describe_view(f: &mut Frame, app: &App, area: Rect) {
    let json = app
        .selected_item_json()
        .unwrap_or_else(|| "No item selected".to_string());

    let title = if let Some(resource) = app.current_resource() {
        describe_title(
            &resource.display_name,
            app.last_action_display_name.as_deref(),
        )
    } else {
        " Details ".to_string()
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

    let inner_area = block.inner(area);
    f.render_widget(block, area);

    // Split inner area for search bar if search is active or has text
    let show_search = app.describe_search_active || !app.describe_search_text.is_empty();
    let (content_area, search_area) = if show_search {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(inner_area);
        (chunks[0], Some(chunks[1]))
    } else {
        (inner_area, None)
    };

    // Apply JSON syntax highlighting with search match highlighting
    let search_text = &app.describe_search_text;
    let lines: Vec<Line> = json
        .lines()
        .enumerate()
        .map(|(line_num, line)| {
            let is_current_match = app
                .describe_match_lines
                .get(app.describe_current_match)
                .map(|&m| m == line_num)
                .unwrap_or(false);
            highlight_json_line_with_search(line, search_text, is_current_match)
        })
        .collect();
    let total_lines = lines.len();

    // Calculate max scroll based on content area
    let visible_lines = content_area.height as usize;
    let max_scroll = total_lines.saturating_sub(visible_lines);
    let scroll = app.describe_scroll.min(max_scroll);

    let paragraph = Paragraph::new(lines.clone())
        .wrap(Wrap { trim: false })
        .scroll((scroll as u16, 0));
    f.render_widget(paragraph, content_area);

    // Render search bar if active
    if let Some(search_area) = search_area {
        render_describe_search_bar(f, app, search_area);
    }
}

fn render_describe_search_bar(f: &mut Frame, app: &App, area: Rect) {
    let match_info = if app.describe_match_lines.is_empty() {
        if app.describe_search_text.is_empty() {
            String::new()
        } else {
            " [no matches]".to_string()
        }
    } else {
        format!(
            " [{}/{}]",
            app.describe_current_match + 1,
            app.describe_match_lines.len()
        )
    };

    let cursor = if app.describe_search_active { "_" } else { "" };
    let search_display = format!("/{}{}{}", app.describe_search_text, cursor, match_info);

    let style = if app.describe_search_active {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let paragraph = Paragraph::new(Line::from(vec![Span::styled(search_display, style)]));
    f.render_widget(paragraph, area);
}

/// Apply JSON syntax highlighting with search term highlighting
fn highlight_json_line_with_search(
    line: &str,
    search_text: &str,
    is_current_match: bool,
) -> Line<'static> {
    if search_text.is_empty() {
        return highlight_json_line(line);
    }

    let line_lower = line.to_lowercase();
    let search_lower = search_text.to_lowercase();

    // If no match in this line, just use regular highlighting
    if !line_lower.contains(&search_lower) {
        return highlight_json_line(line);
    }

    // Build line with search highlights
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut last_end = 0;

    // Find all occurrences (case-insensitive)
    let mut search_start = 0;
    while let Some(pos) = line_lower[search_start..].find(&search_lower) {
        let match_start = search_start + pos;
        let match_end = match_start + search_text.len();

        // Add text before match with JSON highlighting (simplified - just use default color)
        if match_start > last_end {
            let before = &line[last_end..match_start];
            // Apply simple JSON coloring to the before part
            for span in highlight_json_line(before).spans {
                spans.push(span);
            }
        }

        // Add matched text with highlight
        let matched = &line[match_start..match_end];
        let highlight_style = if is_current_match {
            Style::default()
                .bg(Color::Yellow)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .bg(Color::DarkGray)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        };
        spans.push(Span::styled(matched.to_string(), highlight_style));

        last_end = match_end;
        search_start = match_end;
    }

    // Add remaining text after last match
    if last_end < line.len() {
        let after = &line[last_end..];
        for span in highlight_json_line(after).spans {
            spans.push(span);
        }
    }

    Line::from(spans)
}

fn render_log_tail_view(f: &mut Frame, app: &App, area: Rect) {
    let Some(ref state) = app.log_tail_state else {
        let msg = Paragraph::new("No log tail state").style(Style::default().fg(Color::Red));
        f.render_widget(msg, area);
        return;
    };

    // Build title with stream info and status
    let status = if state.paused { "PAUSED" } else { "LIVE" };
    let status_color = if state.paused {
        Color::Yellow
    } else {
        Color::Green
    };
    let title = format!(" {} | {} ", state.log_stream, status);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            title,
            Style::default()
                .fg(status_color)
                .add_modifier(Modifier::BOLD),
        ));

    let inner_area = block.inner(area);
    f.render_widget(block, area);

    if state.events.is_empty() {
        let msg = if let Some(ref err) = state.error {
            Paragraph::new(format!("Error: {}", err)).style(Style::default().fg(Color::Red))
        } else {
            Paragraph::new("Waiting for log events...").style(Style::default().fg(Color::DarkGray))
        };
        f.render_widget(msg, inner_area);
        return;
    }

    // Build lines from log events with syntax highlighting
    let lines: Vec<Line> = state
        .events
        .iter()
        .map(|event| {
            let timestamp = crate::resource::format_log_timestamp(event.timestamp);
            let message = &event.message;

            // Determine color based on log level keywords
            let msg_style = if message.contains("ERROR")
                || message.contains("error")
                || message.contains("Error")
            {
                Style::default().fg(Color::Red)
            } else if message.contains("WARN")
                || message.contains("warn")
                || message.contains("Warning")
            {
                Style::default().fg(Color::Yellow)
            } else if message.contains("INFO") || message.contains("info") {
                Style::default().fg(Color::Green)
            } else if message.contains("DEBUG") || message.contains("debug") {
                Style::default().fg(Color::Blue)
            } else {
                Style::default().fg(Color::White)
            };

            Line::from(vec![
                Span::styled(
                    format!("[{}] ", timestamp),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(message.trim_end().to_string(), msg_style),
            ])
        })
        .collect();

    let total_lines = lines.len();
    let visible_lines = inner_area.height as usize;
    let max_scroll = total_lines.saturating_sub(visible_lines);
    let scroll = state.scroll.min(max_scroll);

    let paragraph = Paragraph::new(lines.clone()).scroll((scroll as u16, 0));
    f.render_widget(paragraph, inner_area);

    // Render scrollbar if content exceeds visible area
    if total_lines > visible_lines {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("↑"))
            .end_symbol(Some("↓"));
        // content_length = total_lines, position = scroll, viewport = visible_lines
        let mut scrollbar_state = ScrollbarState::new(total_lines)
            .position(scroll)
            .viewport_content_length(visible_lines);
        f.render_stateful_widget(scrollbar, inner_area, &mut scrollbar_state);
    }
}

/// Apply JSON syntax highlighting to a single line
fn highlight_json_line(line: &str) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut chars = line.chars().peekable();
    let mut current = String::new();
    let mut is_key = true; // Track if we're parsing a key or value

    while let Some(c) = chars.next() {
        match c {
            '"' => {
                if !current.is_empty() {
                    spans.push(Span::raw(current.clone()));
                    current.clear();
                }

                // Collect the entire string
                let mut string_content = String::from("\"");
                while let Some(&next_c) = chars.peek() {
                    chars.next();
                    string_content.push(next_c);
                    if next_c == '"' {
                        break;
                    }
                    if next_c == '\\' {
                        if let Some(&escaped) = chars.peek() {
                            chars.next();
                            string_content.push(escaped);
                        }
                    }
                }

                // Color based on whether it's a key or value
                let style = if is_key {
                    Style::default().fg(Color::Cyan) // Keys in cyan
                } else {
                    Style::default().fg(Color::Green) // String values in green
                };
                spans.push(Span::styled(string_content, style));
            }
            ':' => {
                current.push(c);
                spans.push(Span::styled(
                    current.clone(),
                    Style::default().fg(Color::White),
                ));
                current.clear();
                is_key = false; // After colon, we're parsing a value
            }
            ',' => {
                if !current.is_empty() {
                    // Check if it's a number or keyword
                    let style = get_json_value_style(&current);
                    spans.push(Span::styled(current.clone(), style));
                    current.clear();
                }
                spans.push(Span::styled(
                    ",".to_string(),
                    Style::default().fg(Color::White),
                ));
                is_key = true; // After comma, next string is a key
            }
            '{' | '}' | '[' | ']' => {
                if !current.is_empty() {
                    let style = get_json_value_style(&current);
                    spans.push(Span::styled(current.clone(), style));
                    current.clear();
                }
                spans.push(Span::styled(
                    c.to_string(),
                    Style::default().fg(Color::Yellow),
                ));
                if c == '{' || c == '[' {
                    is_key = c == '{'; // After {, next is key; after [, next is value
                }
            }
            ' ' | '\t' => {
                if !current.is_empty() {
                    let style = get_json_value_style(&current);
                    spans.push(Span::styled(current.clone(), style));
                    current.clear();
                }
                spans.push(Span::raw(c.to_string()));
            }
            _ => {
                current.push(c);
            }
        }
    }

    if !current.is_empty() {
        let style = get_json_value_style(&current);
        spans.push(Span::styled(current, style));
    }

    Line::from(spans)
}

/// Get style for JSON values (numbers, booleans, null)
fn get_json_value_style(value: &str) -> Style {
    let trimmed = value.trim();
    if trimmed == "null" {
        Style::default().fg(Color::DarkGray)
    } else if trimmed == "true" || trimmed == "false" {
        Style::default().fg(Color::Magenta)
    } else if trimmed.parse::<f64>().is_ok() {
        Style::default().fg(Color::LightBlue)
    } else {
        Style::default().fg(Color::White)
    }
}

fn render_crumb(f: &mut Frame, app: &App, area: Rect) {
    // Build breadcrumb from navigation
    let breadcrumb = app.get_breadcrumb();
    let crumb_display = breadcrumb.join(" > ");

    // Build sub-resource shortcuts hint
    let shortcuts_hint = if let Some(resource) = app.current_resource() {
        if !resource.sub_resources.is_empty() && app.mode == Mode::Normal {
            let hints: Vec<String> = resource
                .sub_resources
                .iter()
                .map(|s| format!("{}:{}", s.shortcut, s.display_name))
                .collect();
            format!(" | {}", hints.join(" "))
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    // Build pagination hint
    let pagination_hint = if app.pagination.has_more || app.pagination.current_page > 1 {
        let mut hints = Vec::new();
        if app.pagination.current_page > 1 {
            hints.push("[:prev");
        }
        if app.pagination.has_more {
            hints.push("]:next");
        }
        format!(" | {}", hints.join(" "))
    } else {
        String::new()
    };

    // Build sort hint. Lives here rather than the top header, which is a fixed
    // 6 rows that the keybinding columns already fill.
    // Names the cursor column so it's obvious what Tab will act on
    let sort_hint = {
        let target = app.sort_cursor_header().unwrap_or_default();
        match app.sort_display() {
            Some(label) => format!(" | sort:{} | ←→ tab:sort {}", label, target),
            None => format!(" | ←→ tab:sort {}", target),
        }
    };

    let status_text = if let Some(err) = &app.error_message {
        format!("Error: {}", err)
    } else if let Some(msg) = &app.status_message {
        msg.clone()
    } else if app.loading {
        "Loading...".to_string()
    } else if app.mode == Mode::Describe {
        if app.describe_search_active {
            "Type to search | Enter: confirm | Esc: cancel".to_string()
        } else if !app.describe_search_text.is_empty() {
            "n/N: next/prev match | /: new search | Esc: clear".to_string()
        } else if app.last_action_display_name.is_some() {
            "j/k: scroll | /: search | c: copy value | q/d/Esc: back".to_string()
        } else {
            "j/k: scroll | /: search | q/d/Esc: back".to_string()
        }
    } else if app.mode == Mode::LogTail {
        "j/k: scroll | G: bottom (live) | g: top | SPACE: pause | q: exit".to_string()
    } else if app.filter_active {
        if app.filter_text.to_lowercase().starts_with("filters:") {
            // Show resource-specific hint if available
            if let Some(hint) = app.current_resource_filters_hint() {
                format!("Filters: {} | Enter: apply | Esc: clear", hint)
            } else {
                "Filters: key=value, key2=value2 | Enter: apply | Esc: clear".to_string()
            }
        } else if app.filters_autocomplete_shown {
            "Tab: complete 'Filters:' | Type to filter locally | Esc: clear".to_string()
        } else if app.current_resource_supports_filters() {
            "Type 'F' for Filters | Type to filter locally | Esc: clear".to_string()
        } else {
            "Type to filter | Enter: apply | Esc: clear".to_string()
        }
    } else {
        format!("{}{}{}", shortcuts_hint, pagination_hint, sort_hint)
    };

    let style = if app.error_message.is_some() {
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
    } else if app.status_message.is_some() {
        Style::default().fg(Color::Green)
    } else if app.loading {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let crumb = Line::from(vec![
        Span::styled(
            format!("<{}>", crumb_display),
            Style::default().fg(Color::Black).bg(Color::Cyan),
        ),
        Span::raw(" "),
        Span::styled(status_text, style),
    ]);

    let paragraph = Paragraph::new(crumb);
    f.render_widget(paragraph, area);
}

#[cfg(test)]
mod tests {
    use super::{column_layout, describe_title, truncate_cell, TABLE_COLUMN_SPACING};
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;
    use ratatui::widgets::{Cell, Row, Table};
    use ratatui::Terminal;

    /// The real EC2 Instances weights: NAME, INSTANCE ID, STATE, TYPE, AZ,
    /// PUBLIC IP, PRIVATE IP. They sum to 111, which is the whole problem.
    const EC2: [u16; 7] = [20, 21, 12, 12, 14, 16, 16];

    /// Renders one row the way `render_dynamic_table` does, filling each cell
    /// with a distinct marker character repeated `fill(col)` times, and returns
    /// how many times each marker survived into the buffer.
    fn rendered_marker_counts(
        area: Rect,
        weights: &[u16],
        fill: impl Fn(usize) -> usize,
    ) -> Vec<usize> {
        let (constraints, _) = column_layout(area.width, weights);
        let markers: Vec<char> = (0..weights.len())
            .map(|i| (b'a' + i as u8) as char)
            .collect();
        let cells: Vec<Cell> = markers
            .iter()
            .enumerate()
            .map(|(i, m)| Cell::from(format!(" {}", m.to_string().repeat(fill(i)))))
            .collect();
        let table = Table::new([Row::new(cells)], constraints).column_spacing(TABLE_COLUMN_SPACING);

        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
        terminal
            .draw(|f| f.render_widget(table, area))
            .expect("draw");

        let buffer = terminal.backend().buffer().clone();
        let row: String = (0..area.width)
            .filter_map(|x| buffer.cell((x, 0)).map(|c| c.symbol().to_string()))
            .collect();
        markers
            .iter()
            .map(|m| row.chars().filter(|c| c == m).count())
            .collect()
    }

    /// The widths we hand `truncate_cell` have to be the widths ratatui really
    /// gives each column, or text clips despite passing the length check. Renders
    /// values sized to the claimed width and checks every character survived.
    #[test]
    fn column_layout_text_widths_are_not_clipped_when_rendered() {
        let area = Rect::new(0, 0, 120, 3);
        let (_, text) = column_layout(area.width, &EC2);

        assert_eq!(
            rendered_marker_counts(area, &EC2, |i| text[i]),
            text,
            "a value sized to the claimed width lost characters when rendered"
        );
    }

    /// Other half of the same contract: the claim is the ceiling, not a guess
    /// under it. One character more must clip, otherwise we are wasting room.
    #[test]
    fn column_layout_text_widths_are_the_most_that_fits() {
        let area = Rect::new(0, 0, 120, 3);
        let (_, text) = column_layout(area.width, &EC2);

        assert_eq!(
            rendered_marker_counts(area, &EC2, |i| text[i] + 1),
            text,
            "an over-long value rendered more than the claimed width, so columns \
             have room we are not using"
        );
    }

    /// The bug this replaced: handing ratatui percentages that sum to 111 made it
    /// give EC2's 20-weight NAME and 12-weight STATE the same 12 cells, and made
    /// the 16-weight PUBLIC IP wider than NAME. A heavier column must always get
    /// at least as many cells as a lighter one.
    #[test]
    fn column_layout_never_lets_a_lighter_column_outgrow_a_heavier_one() {
        for width in 40..=240 {
            let (_, text) = column_layout(width, &EC2);

            for (i, wi) in EC2.iter().enumerate() {
                for (j, wj) in EC2.iter().enumerate() {
                    if wi > wj {
                        assert!(
                            text[i] >= text[j],
                            "at width {} column {} (weight {}) got {} cells but \
                             lighter column {} (weight {}) got {}",
                            width,
                            i,
                            wi,
                            text[i],
                            j,
                            wj,
                            text[j]
                        );
                    }
                }
            }
        }
    }

    /// 52 of the 61 resource definitions have weights that do not sum to 100,
    /// from 50 up to 148. Every one of them must still use the full table: the
    /// old percentage handling left a 50-sum resource with half the table blank.
    #[test]
    fn column_layout_fills_the_width_whatever_the_weights_sum_to() {
        let weight_sets: [&[u16]; 4] = [&EC2, &[25, 15, 10], &[50, 50], &[40, 40, 40, 28]];

        for weights in weight_sets {
            for width in 40..=240u16 {
                let (_, text) = column_layout(width, weights);
                let gaps = (weights.len() as u16 - 1) * TABLE_COLUMN_SPACING;
                let pads = weights.len() as u16;
                let used: u16 = text.iter().map(|t| *t as u16).sum::<u16>() + pads + gaps;

                assert_eq!(
                    used, width,
                    "weights {:?} at width {} used {} cells",
                    weights, width, used
                );
            }
        }
    }

    /// Narrow terminals must not panic or hand out phantom cells. Below roughly
    /// two cells per column there is nothing useful to show, but it still has to
    /// stay inside the area.
    #[test]
    fn column_layout_survives_areas_too_small_for_its_columns() {
        for width in 0..=20u16 {
            let (constraints, text) = column_layout(width, &EC2);
            assert_eq!(constraints.len(), EC2.len());
            assert!(text.iter().map(|t| *t as u16).sum::<u16>() <= width);
        }
    }

    #[test]
    fn column_layout_handles_a_resource_with_no_columns() {
        let (constraints, text) = column_layout(80, &[]);
        assert!(constraints.is_empty());
        assert!(text.is_empty());
    }

    #[test]
    fn truncate_cell_keeps_the_start_and_marks_the_cut_at_the_end() {
        assert_eq!(
            truncate_cell("finance_suite_webserver", 20),
            "finance_suite_web..."
        );
    }

    #[test]
    fn truncate_cell_leaves_values_that_fit_alone() {
        assert_eq!(truncate_cell("Reporting", 20), "Reporting");
        assert_eq!(truncate_cell("exactly-ten", 11), "exactly-ten");
    }

    /// The ellipsis itself is 3 cells, so a very narrow column has no room for
    /// both it and any content. Overflowing here would bleed into the next
    /// column, so drop the marker rather than the width limit.
    #[test]
    fn truncate_cell_never_exceeds_the_width_it_is_given() {
        for width in 0..=24 {
            let out = truncate_cell("finance_suite_webserver", width);
            assert!(
                out.chars().count() <= width,
                "width {} produced {:?} ({} cells)",
                width,
                out,
                out.chars().count()
            );
        }
    }

    /// Widths are cell counts, not byte counts; slicing bytes would panic here.
    #[test]
    fn truncate_cell_counts_characters_not_bytes() {
        assert_eq!(truncate_cell("ααααααααα", 6), "ααα...");
    }

    #[test]
    fn describe_title_uses_action_display_name_when_present() {
        let title = describe_title("Secrets Manager Secrets", Some("Secret Value"));
        assert_eq!(title, " Secret Value ");
    }

    #[test]
    fn describe_title_falls_back_to_resource_details() {
        let title = describe_title("EC2 Instances", None);
        assert_eq!(title, " EC2 Instances Details ");
    }
}
