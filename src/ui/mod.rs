mod column_picker;
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
        Block, Borders, Cell, Clear, Paragraph, Row, Scrollbar, ScrollbarOrientation,
        ScrollbarState, Table, TableState, Wrap,
    },
    Frame,
};
use serde_json::Value;
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
        Mode::ColumnPicker => {
            column_picker::render(f, app);
        }
        Mode::Confirm | Mode::Warning | Mode::SsoLogin | Mode::ConsoleLogin | Mode::Update => {
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
        let has_more = app.pagination.has_more;

        // Route53 records and similar: parent carries the authoritative total
        let parent_total = app
            .parent_context
            .as_ref()
            .and_then(|p| p.item.get("ResourceRecordSetCount"))
            .and_then(|v| {
                v.as_str()
                    .and_then(|s| s.parse::<usize>().ok())
                    .or_else(|| v.as_u64().map(|n| n as usize))
            });

        let count_str = if let Some(pt) = parent_total {
            format!("{}/{}", count, pt)
        } else if has_more {
            format!("{}+", count)
        } else {
            count.to_string()
        };
        let total_str = if let Some(pt) = parent_total {
            format!("{}/{}", total, pt)
        } else if has_more {
            format!("{}+", total)
        } else {
            total.to_string()
        };

        // Pagination indicator (page number only when past page 1)
        let page_info = if app.pagination.current_page > 1 {
            format!(" pg.{}", app.pagination.current_page)
        } else {
            String::new()
        };

        if is_global {
            if query.is_empty() {
                format!(" {}[{}]{} ", resource.display_name, count_str, page_info)
            } else {
                format!(
                    " {}[{}/{}]{} ",
                    resource.display_name, count_str, total_str, page_info
                )
            }
        } else if query.is_empty() {
            format!(
                " {}({})[{}]{} ",
                resource.display_name, app.region, count_str, page_info
            )
        } else {
            format!(
                " {}({})[{}/{}]{} ",
                resource.display_name, app.region, count_str, total_str, page_info
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

    // Column sizing has two modes. Fit mode apportions the JSON weights across
    // the real area (see column_layout for why they cannot go to ratatui as
    // percentages). Overflow mode kicks in when even minimum-width columns
    // cannot fit: each column takes max(MIN_COLUMN_WIDTH, weight) cells and the
    // table becomes a window onto a wider canvas scrolled with Shift+arrows.
    // Sort indices stay pinned to positions in the full column list, so each
    // rendered column carries its original index.
    const MIN_COLUMN_WIDTH: u16 = 12;
    let visible_columns: Vec<(usize, &crate::resource::ColumnDef)> = app.effective_columns();
    let min_widths: Vec<u16> = visible_columns
        .iter()
        .map(|(_, col)| col.width.max(MIN_COLUMN_WIDTH))
        .collect();
    let gaps_total = visible_columns.len().saturating_sub(1) as u16 * TABLE_COLUMN_SPACING;
    let needed: u16 = min_widths.iter().sum::<u16>().saturating_add(gaps_total);
    let overflow = !min_widths.is_empty() && needed > inner_area.width;

    // (original index, column, cell width) for every column that renders
    let mut render_columns: Vec<(usize, &crate::resource::ColumnDef, u16)> = Vec::new();
    let mut h_scroll_info: Option<(usize, usize)> = None;

    if overflow {
        // Clamp the offset so scrolling right eventually pins the last columns
        // rather than scrolling everything off-screen.
        let max_offset = max_h_scroll_offset(&min_widths, inner_area.width);
        let offset = app.h_scroll.min(max_offset);
        let mut used: u16 = 0;
        for (i, &w) in min_widths.iter().enumerate().skip(offset) {
            let cost = if render_columns.is_empty() {
                w
            } else {
                w + TABLE_COLUMN_SPACING
            };
            if used + cost > inner_area.width {
                break;
            }
            used += cost;
            let (idx, col) = visible_columns[i];
            render_columns.push((idx, col, w));
        }
        h_scroll_info = Some((offset, min_widths.len()));
    } else {
        let weights: Vec<u16> = visible_columns.iter().map(|(_, col)| col.width).collect();
        let (_, text_widths) = column_layout(inner_area.width, &weights);
        for ((idx, col), w) in visible_columns.iter().zip(text_widths.iter()) {
            render_columns.push((*idx, *col, *w as u16));
        }
    }

    let widths: Vec<Constraint> = render_columns
        .iter()
        .map(|(_, _, w)| Constraint::Length(*w))
        .collect();
    let column_widths: Vec<usize> = render_columns
        .iter()
        .map(|(_, _, w)| (*w as usize).saturating_sub(CELL_PAD))
        .collect();

    // Build header from column definitions with left padding.
    // Sorted column gets a direction arrow in cyan; the cursor column (what Tab
    // would sort) is underlined, so the two states stay tellable apart.
    let header_cells = render_columns
        .iter()
        .enumerate()
        .map(|(col_idx, (_, col, _))| {
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
            let cells = render_columns
                .iter()
                .enumerate()
                .map(|(col_idx, (_, col, _))| {
                    let value = extract_json_value(item, &col.json_path);
                    let mut style = get_cell_style(&value, col);
                    if is_selected {
                        style = style.fg(Color::White);
                    }
                    let display_value = format_cell_value(&value, col);
                    let col_width = column_widths_clone.get(col_idx).copied().unwrap_or(40);
                    let display_value = truncate_cell(&display_value, col_width);

                    if highlight_filter_matches
                        && (col.json_path == resource.name_field
                            || col.json_path == resource.id_field)
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

    // Horizontal scrollbar on the table's bottom border when columns overflow.
    // Overwrites the border line — the usual scrollbar-on-border aesthetic.
    if let Some((offset, total)) = h_scroll_info {
        let scrollbar_area = Rect {
            x: area.x + 1,
            y: area.y + area.height.saturating_sub(1),
            width: area.width.saturating_sub(2),
            height: 1,
        };
        let scrollbar = Scrollbar::new(ScrollbarOrientation::HorizontalBottom)
            .begin_symbol(None)
            .end_symbol(None);
        let mut sb_state = ScrollbarState::new(total).position(offset);
        f.render_stateful_widget(scrollbar, scrollbar_area, &mut sb_state);
    }
}

/// Furthest first-visible-column offset that still shows content: the point
/// where the trailing columns fill the area exactly. Scrolling past it would
/// just push every column off the right edge.
fn max_h_scroll_offset(min_widths: &[u16], area_width: u16) -> usize {
    let mut used: usize = 0;
    let mut count: usize = 0;
    for &w in min_widths.iter().rev() {
        let cost = w as usize
            + if count == 0 {
                0
            } else {
                TABLE_COLUMN_SPACING as usize
            };
        if used + cost > area_width as usize {
            break;
        }
        used += cost;
        count += 1;
    }
    min_widths.len().saturating_sub(count)
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

/// Render the fetched policy-document drill view: a fixed header naming the
/// policy, then the document as a scrollable, wrap-enabled paragraph. The
/// header stays anchored while the body scrolls.
fn render_drill_document(f: &mut Frame, app: &App, area: Rect) {
    let Some(drill) = app.drill_data.as_ref() else {
        return;
    };

    let header_lines = if drill.label.is_empty() { 2 } else { 3 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(header_lines as u16), Constraint::Min(1)])
        .split(area);
    let header_area = chunks[0];
    let content_area = chunks[1];

    let mut header: Vec<Line> = vec![Line::from(Span::styled(
        " Policy Document ",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ))];
    if !drill.label.is_empty() {
        header.push(Line::from(Span::styled(
            format!("  {}", drill.label),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )));
    }
    header.push(Line::from(""));
    let header_para = Paragraph::new(header);
    f.render_widget(header_para, header_area);

    let lines: Vec<Line> = drill
        .document
        .lines()
        .map(|l| {
            Line::from(Span::styled(
                l.to_string(),
                Style::default().fg(Color::White),
            ))
        })
        .collect();
    let total = lines.len();
    let max_scroll = total.saturating_sub(content_area.height as usize);
    let scroll = app.drill_scroll.min(max_scroll);

    let paragraph = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((scroll as u16, 0));
    f.render_widget(paragraph, content_area);
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

    // A fetched policy document replaces the whole describe body. It is drawn
    // inside the same outer block so exit is visually "above" the describe.
    if app.drill_data.is_some() {
        render_drill_document(f, app, inner_area);
        return;
    }

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

    // A resource renders a formatted panel when it declares describe_fields or
    // an overview banner; otherwise it falls back to the raw JSON dump. An
    // action-result view (last_action_display_name set, e.g. GetSecretValue via
    // x) has a different shape, so it must fall through to the raw JSON dump and
    // never run the describe fields against the wrong data.
    let describe_config = app
        .current_resource()
        .and_then(|r| r.describe_config.as_ref())
        .filter(|dc| !dc.describe_fields.is_empty() || dc.overview.is_some())
        .filter(|_| app.last_action_display_name.is_none());

    if let Some(config) = describe_config {
        if let Some(ref data) = app.describe_data {
            let mut lines = render_formatted_describe(config, data);
            let total_lines = lines.len();
            let visible_lines = content_area.height as usize;
            let max_scroll = total_lines.saturating_sub(visible_lines);
            let scroll = app.describe_scroll.min(max_scroll);

            // Highlight the drillable list item under the describe cursor so
            // the user can see which document Enter will open.
            if let Some(target) = app
                .describe_drill_targets()
                .get(app.describe_cursor)
                .filter(|t| t.line < lines.len())
            {
                if let Some(Line { spans, .. }) = lines.get_mut(target.line) {
                    for span in spans {
                        span.style = span.style.add_modifier(Modifier::REVERSED).fg(Color::Black);
                    }
                }
            }

            let paragraph = Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .scroll((scroll as u16, 0));
            f.render_widget(paragraph, content_area);
        }
    } else {
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

    // A revealed secret value sits in an overlay above the describe body so
    // the secret is unmistakably foregrounded while visible, and disappears
    // entirely when the countdown expires.
    if app.reveal_secret.is_some() {
        render_secret_reveal_overlay(f, app, inner_area);
    }
}

/// Center a bordered panel over the describe body showing the revealed secret
/// value, with a live countdown of the remaining seconds before it auto-hides.
fn render_secret_reveal_overlay(f: &mut Frame, app: &App, area: Rect) {
    let Some(reveal) = app.reveal_secret.as_ref() else {
        return;
    };
    let seconds = app.reveal_seconds_left().unwrap_or(0);

    let mode_label = match reveal.mode {
        crate::app::RevealMode::Plaintext => "plaintext",
        crate::app::RevealMode::KeyValue => "key/value",
    };
    // A secret string is already its own text; key/value wraps it as one line
    // and adds a countdown+mode header, so the total is one row per key.
    let display: String = match reveal.mode {
        crate::app::RevealMode::KeyValue => match secret_kv_lines(&reveal.value) {
            Some(pairs) => pairs
                .iter()
                .map(|(k, v)| format!("{}: {}", k, v))
                .collect::<Vec<_>>()
                .join("\n"),
            None => {
                // Not a JSON object -- fall back to plaintext per the console.
                reveal.value.clone()
            }
        },
        crate::app::RevealMode::Plaintext => reveal.value.clone(),
    };

    let line_count = display.lines().count().clamp(1, 30);
    let popup_width = (area.width / 2).clamp(40, 80);
    let max_height = area.height.saturating_sub(4).max(8);
    let popup_height = (line_count as u16 + 5).clamp(8, max_height);

    let popup_area = Rect {
        x: area.x + (area.width.saturating_sub(popup_width)) / 2,
        y: area.y + (area.height.saturating_sub(popup_height)) / 2,
        width: popup_width,
        height: popup_height,
    };

    let title = Span::styled(
        format!(" Secret value [{mode_label}] - hidden in {}s ", seconds),
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .title(title);

    let inner = block.inner(popup_area);
    f.render_widget(Clear, popup_area);
    f.render_widget(block, popup_area);

    let paragraph = Paragraph::new(display)
        .wrap(Wrap { trim: false })
        .scroll((0, 0));
    f.render_widget(paragraph, inner);

    let hint = Paragraph::new(Span::styled(
        "  [t key/value | s / Esc to hide now]",
        Style::default().fg(Color::DarkGray),
    ))
    .alignment(Alignment::Center);
    if popup_height >= 8 {
        let hint_area = Rect {
            x: popup_area.x,
            y: popup_area.y + popup_height - 2,
            width: popup_area.width,
            height: 1,
        };
        f.render_widget(hint, hint_area);
    }
}

/// Parse a secret value as a top-level JSON object and return its entries,
/// sorted by key for a stable render. None when the value is not a JSON object
/// (including scalars and arrays), which is the "not key/value" fallback.
fn secret_kv_lines(value: &str) -> Option<Vec<(String, String)>> {
    let parsed: serde_json::Value = serde_json::from_str(value).ok()?;
    let obj = parsed.as_object()?;
    let mut pairs: Vec<(String, String)> = obj
        .iter()
        .map(|(k, v)| (k.clone(), v.to_string()))
        .collect();
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    Some(pairs)
}

fn render_formatted_describe(
    config: &crate::resource::protocol::DescribeConfig,
    data: &Value,
) -> Vec<Line<'static>> {
    describe_layout(config, data).0
}

/// Collect the drillable items of a formatted describe, in the order they
/// render, so the describe cursor and the highlight in the renderer walk the
/// same list. Splitting this out of `describe_layout` would let the two drift;
/// they share the one pass below.
pub fn collect_describe_drill_targets(
    config: &crate::resource::protocol::DescribeConfig,
    data: &Value,
) -> Vec<crate::resource::protocol::DescribeDrillTarget> {
    describe_layout(config, data).1
}

/// Build the formatted describe panel in a single pass, recording both the
/// rendered lines and, for each drillable list item, the line it renders on
/// and the item value the drill should fetch.
fn describe_layout(
    config: &crate::resource::protocol::DescribeConfig,
    data: &Value,
) -> (
    Vec<Line<'static>>,
    Vec<crate::resource::protocol::DescribeDrillTarget>,
) {
    let mut lines: Vec<Line> = vec![];
    let mut targets: Vec<crate::resource::protocol::DescribeDrillTarget> = vec![];

    if let Some(overview) = &config.overview {
        lines.extend(render_overview_banner(overview, data));
        lines.push(Line::from(""));
    }

    let mut current_section: Option<&str> = None;

    for field in &config.describe_fields {
        if let Some(section) = field.section.as_deref() {
            if current_section != Some(section) {
                if current_section.is_some() || !lines.is_empty() {
                    lines.push(Line::from(""));
                }
                lines.push(Line::from(Span::styled(
                    format!("  {}", section),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )));
                current_section = Some(section);
            }
        }

        let value = crate::resource::path_extractor::extract_by_path(data, &field.source);

        let value = if let Some(ref transform) = field.transform {
            crate::resource::field_mapper::apply_transform(&value, transform)
        } else {
            value
        };

        if field.list {
            // A drillable list item each carry the wire to fetch their
            // document. Inline the item loop so we can record the line each
            // one renders on; `describe_list_lines` cannot report that.
            let drill = field
                .drill
                .as_ref()
                .map(|d| (d, field.item_template.as_deref()));
            match drill {
                Some((drill, item_template)) => {
                    let items = value_to_items(&value);
                    if items.is_empty() {
                        lines.push(Line::from(vec![
                            Span::styled(
                                format!("  {:<20}", field.label),
                                Style::default().fg(Color::DarkGray),
                            ),
                            Span::styled("-", Style::default().fg(Color::White)),
                        ]));
                        continue;
                    }
                    let label = describe_list_label(field);
                    lines.push(label);
                    for item in items {
                        lines.push(describe_list_item_line(item_template, item));
                        targets.push(crate::resource::protocol::DescribeDrillTarget {
                            field_label: field.label.clone(),
                            item: item.clone(),
                            config: drill.clone(),
                            line: lines.len() - 1,
                            hint: describe_list_item_text(item_template, item),
                        });
                    }
                    continue;
                }
                None => {
                    lines.extend(describe_list_lines(field, &value));
                    continue;
                }
            }
        }

        let display = value_to_describe_string(&value);

        lines.push(Line::from(vec![
            Span::styled(
                format!("  {:<20}", field.label),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(display, Style::default().fg(Color::White)),
        ]));
    }

    lines.push(Line::from(""));
    (lines, targets)
}

/// A list field: its label, then one line per item. An empty list still prints
/// the label and a dash, so "there are none" cannot be mistaken for "orbit does
/// not show this".
fn describe_list_lines(
    field: &crate::resource::protocol::DescribeField,
    value: &Value,
) -> Vec<Line<'static>> {
    let items = value_to_items(value);

    if items.is_empty() {
        return vec![Line::from(vec![
            Span::styled(
                format!("  {:<20}", field.label),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled("-", Style::default().fg(Color::White)),
        ])];
    }

    let mut lines = vec![describe_list_label(field)];
    for item in items {
        lines.push(describe_list_item_line(
            field.item_template.as_deref(),
            item,
        ));
    }
    lines
}

/// A bare list value as its items, keeping the XML→JSON collapse rule: a
/// single-element list arrives as a bare object rather than an array.
fn value_to_items(value: &Value) -> Vec<&Value> {
    match value {
        Value::Array(items) => items.iter().collect(),
        Value::Null => vec![],
        other => vec![other],
    }
}

/// The header line a list field prints above its items (or "-" when empty).
fn describe_list_label(field: &crate::resource::protocol::DescribeField) -> Line<'static> {
    Line::from(Span::styled(
        format!("  {:<20}", field.label),
        Style::default().fg(Color::DarkGray),
    ))
}

/// The rendered text of one list item.
fn describe_list_item_text(item_template: Option<&str>, item: &Value) -> String {
    match item_template {
        Some(template) => render_describe_item(template, item),
        None => value_to_describe_string(item),
    }
}

/// One item's rendered line, indented under its list header.
fn describe_list_item_line(item_template: Option<&str>, item: &Value) -> Line<'static> {
    let text = describe_list_item_text(item_template, item);
    Line::from(Span::styled(
        format!("    {}", text.trim()),
        Style::default().fg(Color::White),
    ))
}

/// Fill `{key}` / `{nested/key}` from one list item. A key the item lacks
/// renders empty: EC2 rules carry `cidrIpv4` or `referencedGroupInfo`, never
/// both, and a "-" in the gap would read as a real value.
fn render_describe_item(template: &str, item: &Value) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;

    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('}') else {
            out.push('{');
            rest = after;
            continue;
        };
        let key = &after[..end];
        let value = crate::resource::path_extractor::extract_by_path(item, key);
        if !value.is_null() {
            out.push_str(&value_to_describe_string(&value));
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);

    out
}

fn value_to_describe_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => {
            if *b {
                "Yes".to_string()
            } else {
                "No".to_string()
            }
        }
        Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(value_to_describe_string).collect();
            items.join(", ")
        }
        Value::Object(obj) => {
            let pairs: Vec<String> = obj
                .iter()
                .map(|(k, v)| format!("{}: {}", k, value_to_describe_string(v)))
                .collect();
            pairs.join(", ")
        }
        Value::Null => "-".to_string(),
    }
}

/// An ASCII boxed banner heading the formatted describe panel, standing in for
/// the console's Function Overview card. The identity (title + subtitle +
/// chips) is drawn as a boxed card, then each connected-resource group is
/// rendered as a column of boxed nodes linked up by a `▲` connector line.
fn render_overview_banner(
    overview: &crate::resource::protocol::OverviewConfig,
    data: &Value,
) -> Vec<Line<'static>> {
    let mut lines: Vec<Line> = vec![];

    let title = crate::resource::path_extractor::extract_by_path(data, &overview.title_source);
    let title_text = value_to_describe_string(&title);

    let mut subtitle_text = String::new();
    if let Some(sub_source) = &overview.subtitle_source {
        let sub = crate::resource::path_extractor::extract_by_path(data, sub_source);
        subtitle_text = value_to_describe_string(&sub);
    }

    lines.push(Line::from(Span::styled(
        format!("  ╭─── {}", title_text),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )));

    if !subtitle_text.is_empty() && subtitle_text != "-" {
        lines.push(Line::from(Span::styled(
            format!("  │    {}", subtitle_text),
            Style::default().fg(Color::DarkGray),
        )));
    }

    if !overview.chips.is_empty() {
        let chip_texts: Vec<String> = overview
            .chips
            .iter()
            .map(|chip| {
                let mut v = crate::resource::path_extractor::extract_by_path(data, &chip.source);
                if let Some(ref transform) = chip.transform {
                    v = crate::resource::field_mapper::apply_transform(&v, transform);
                }
                format!("{}: {}", chip.label, value_to_describe_string(&v))
            })
            .collect();
        lines.push(Line::from(Span::styled(
            format!("  │    {}", chip_texts.join("   ·   ")),
            Style::default().fg(Color::White),
        )));
    }

    lines.push(Line::from(Span::styled(
        "  ╰──────────────",
        Style::default().fg(Color::Cyan),
    )));

    if overview.resources.is_empty() {
        return lines;
    }

    // The partial diagram: a single centre connector column rising from the
    // card, then each group's boxed nodes.
    lines.push(Line::from(Span::styled(
        "        ▲",
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(Span::styled(
        "        │",
        Style::default().fg(Color::DarkGray),
    )));

    for (i, group) in overview.resources.iter().enumerate() {
        if i > 0 {
            lines.push(Line::from(Span::styled(
                "        │",
                Style::default().fg(Color::DarkGray),
            )));
        }
        lines.push(Line::from(Span::styled(
            format!("    {}:", group.label),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )));

        let values = crate::resource::path_extractor::extract_by_path(data, &group.source);
        let items: Vec<&Value> = match &values {
            Value::Array(arr) => arr.iter().collect(),
            Value::Null => vec![],
            other => vec![other],
        };

        if items.is_empty() {
            lines.push(Line::from(Span::styled(
                "        └─ (none)",
                Style::default().fg(Color::DarkGray),
            )));
            continue;
        }

        for (j, item) in items.iter().enumerate() {
            let text = match group.item_template.as_deref() {
                Some(template) => render_describe_item(template, item),
                None => value_to_describe_string(item),
            };
            let prefix = if j == items.len() - 1 {
                "└─"
            } else {
                "├─"
            };
            let styled = if text.trim() == "-" {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default().fg(Color::Yellow)
            };
            lines.push(Line::from(Span::styled(
                format!("        {} {}", prefix, text.trim()),
                styled,
            )));
        }
    }

    lines
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
    use super::{
        column_layout, describe_title, max_h_scroll_offset, render_overview_banner, truncate_cell,
        TABLE_COLUMN_SPACING,
    };
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;
    use ratatui::widgets::{Cell, Row, Table};
    use ratatui::Terminal;

    /// Flattened text of a rendered describe line, for asserting on content
    /// without caring how it was split into spans.
    fn describe_lines(
        fields: &[crate::resource::protocol::DescribeField],
        data: &serde_json::Value,
    ) -> Vec<String> {
        let config = crate::resource::protocol::DescribeConfig {
            describe_fields: fields.to_vec(),
            ..Default::default()
        };
        super::render_formatted_describe(&config, data)
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    fn describe_field(
        label: &str,
        source: &str,
        section: Option<&str>,
    ) -> crate::resource::protocol::DescribeField {
        crate::resource::protocol::DescribeField {
            label: label.to_string(),
            source: source.to_string(),
            transform: None,
            section: section.map(|s| s.to_string()),
            list: false,
            item_template: None,
            drill: None,
        }
    }

    /// The console groups an instance's fields under headings, and a flat
    /// 30-field list is unreadable without them. A heading is emitted once, when
    /// the section changes, not per field.
    #[test]
    fn describe_sections_are_emitted_once_above_their_fields() {
        use serde_json::json;

        let fields = vec![
            describe_field("Status", "/DBInstanceStatus", Some("Summary")),
            describe_field("Class", "/DBInstanceClass", Some("Summary")),
            describe_field("VPC", "/DBSubnetGroup/VpcId", Some("Connectivity")),
        ];
        let data = json!({
            "DBInstanceStatus": "available",
            "DBInstanceClass": "db.r6g.large",
            "DBSubnetGroup": { "VpcId": "vpc-0a1b2c3d" },
        });

        let rendered = describe_lines(&fields, &data);
        let headings: Vec<&String> = rendered
            .iter()
            .filter(|l| l.contains("Summary") || l.contains("Connectivity"))
            .collect();

        assert_eq!(
            headings.len(),
            2,
            "expected one heading per section, got {:?}",
            rendered
        );
        let summary = rendered.iter().position(|l| l.contains("Summary")).unwrap();
        let status = rendered
            .iter()
            .position(|l| l.contains("available"))
            .unwrap();
        let connectivity = rendered
            .iter()
            .position(|l| l.contains("Connectivity"))
            .unwrap();
        assert!(
            summary < status && status < connectivity,
            "heading should precede its own fields: {:?}",
            rendered
        );
    }

    /// Security group rules, cluster members and subnets are tables in the
    /// console. Joined onto one line they wrap into mush, so a list field gets
    /// one line per item, shaped by its template.
    #[test]
    fn describe_list_fields_render_one_templated_line_per_item() {
        use serde_json::json;

        let mut field = describe_field("Security Group Rules", "/SecurityGroupRules/item", None);
        field.list = true;
        field.item_template = Some(
            "{isEgress} {ipProtocol} {fromPort}-{toPort} {cidrIpv4}{referencedGroupInfo/groupId}"
                .to_string(),
        );

        let data = json!({
            "SecurityGroupRules": { "item": [
                { "isEgress": "false", "ipProtocol": "tcp", "fromPort": "27017",
                  "toPort": "27017", "cidrIpv4": "10.64.80.0/21" },
                { "isEgress": "false", "ipProtocol": "tcp", "fromPort": "27017",
                  "toPort": "27017", "referencedGroupInfo": { "groupId": "sg-0fe51082fe28f1fe4" } },
            ]}
        });

        let rendered = describe_lines(&field_list(field), &data);

        assert!(
            rendered
                .iter()
                .any(|l| l.contains("tcp 27017-27017 10.64.80.0/21")),
            "expected a CIDR rule line in {:?}",
            rendered
        );
        assert!(
            rendered
                .iter()
                .any(|l| l.contains("tcp 27017-27017 sg-0fe51082fe28f1fe4")),
            "expected a referenced-group rule line in {:?}",
            rendered
        );
    }

    /// XML→JSON collapses a one-element list to a bare object. A list field must
    /// still show that single item rather than nothing at all.
    #[test]
    fn describe_list_fields_survive_a_single_element_xml_list_collapsing() {
        use serde_json::json;

        let mut field = describe_field("Members", "/DBClusterMembers/DBClusterMember", None);
        field.list = true;
        field.item_template = Some("{DBInstanceIdentifier} writer={IsClusterWriter}".to_string());

        let data = json!({
            "DBClusterMembers": { "DBClusterMember": {
                "DBInstanceIdentifier": "prod-docdb-1", "IsClusterWriter": "true"
            }}
        });

        let rendered = describe_lines(&field_list(field), &data);

        assert!(
            rendered
                .iter()
                .any(|l| l.contains("prod-docdb-1 writer=true")),
            "a collapsed single-item list vanished: {:?}",
            rendered
        );
    }

    /// A list with nothing in it must still print its label and a dash. Dropping
    /// the whole field reads as "orbit does not show this", not "there are none".
    #[test]
    fn describe_list_fields_show_a_dash_when_empty() {
        use serde_json::json;

        let mut field = describe_field("Members", "/DBClusterMembers/DBClusterMember", None);
        field.list = true;
        field.item_template = Some("{DBInstanceIdentifier}".to_string());

        let rendered = describe_lines(&field_list(field), &json!({}));

        assert!(
            rendered
                .iter()
                .any(|l| l.contains("Members") && l.contains('-')),
            "expected a labelled dash for an empty list: {:?}",
            rendered
        );
    }

    fn field_list(
        field: crate::resource::protocol::DescribeField,
    ) -> Vec<crate::resource::protocol::DescribeField> {
        vec![field]
    }

    /// A drillable list field surfaces each item as a target whose line points
    /// at that item's rendered row, so the describe cursor can highlight it and
    /// Enter can fetch its document. A plain (non-drillable) list contributes
    /// no targets.
    #[test]
    fn drillable_list_fields_produce_targets_at_their_item_lines() {
        use crate::resource::protocol::{DrillConfig, DrillKind};
        use serde_json::json;

        let mut attached = describe_field("Attached", "/AttachedPolicies", Some("Policies"));
        attached.list = true;
        attached.item_template = Some("{PolicyName}".to_string());
        attached.drill = Some(DrillConfig {
            kind: DrillKind::ManagedPolicyDocument,
            item_field: "PolicyArn".to_string(),
        });

        let mut plain = describe_field("Plain", "/PlainList", Some("Policies"));
        plain.list = true;

        let config = crate::resource::protocol::DescribeConfig {
            describe_fields: vec![attached, plain],
            ..Default::default()
        };
        let data = json!({
            "AttachedPolicies": [
                { "PolicyName": "a", "PolicyArn": "arn:1" },
                { "PolicyName": "b", "PolicyArn": "arn:2" },
            ],
            "PlainList": [1, 2],
        });

        let targets = super::collect_describe_drill_targets(&config, &data);
        assert_eq!(targets.len(), 2, "only the drillable field yields targets");
        assert_eq!(targets[0].hint, "a");
        assert_eq!(
            targets[0].item,
            json!({ "PolicyName": "a", "PolicyArn": "arn:1" })
        );
        assert_eq!(targets[1].hint, "b");
        // The two items render on consecutive lines: header line then item 0.
        assert_eq!(targets[1].line, targets[0].line + 1);
    }

    /// The clamp must leave the trailing columns reachable: scrolling right
    /// pins the last columns at the right edge instead of scrolling them all
    /// off-screen. Three 12-cell columns in a 30-cell area fit two at a time
    /// (12 + 1 spacing + 12 = 25, three would need 38), so the furthest
    /// useful offset shows columns 1..=2, i.e. offset 1.
    #[test]
    fn max_h_scroll_pins_trailing_columns_in_view() {
        assert_eq!(max_h_scroll_offset(&[12, 12, 12], 30), 1);
        // Everything fits — no scrolling at all
        assert_eq!(max_h_scroll_offset(&[12, 12, 12], 100), 0);
        // A column wider than the area can never render; offset pins to the end
        assert_eq!(max_h_scroll_offset(&[50], 30), 1);
    }

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

    #[test]
    fn overview_banner_renders_title_chips_and_trigger_nodes() {
        use crate::resource::protocol::{OverviewChip, OverviewConfig, OverviewResource};

        let overview = OverviewConfig {
            title_source: "/FunctionName".to_string(),
            subtitle_source: Some("/Description".to_string()),
            chips: vec![
                OverviewChip {
                    label: "Runtime".to_string(),
                    source: "/Runtime".to_string(),
                    transform: None,
                },
                OverviewChip {
                    label: "Size".to_string(),
                    source: "/CodeSize".to_string(),
                    transform: Some("format_bytes".to_string()),
                },
            ],
            resources: vec![OverviewResource {
                label: "TRIGGERS".to_string(),
                source: "/EventSourceMappings".to_string(),
                item_template: Some("{EventSourceArn}".to_string()),
            }],
        };

        let data = serde_json::json!({
            "FunctionName": "hello-world",
            "Description": "a test function",
            "Runtime": "nodejs18.x",
            "CodeSize": 1048576,
            "EventSourceMappings": [
                { "EventSourceArn": "arn:aws:sqs:eu-west-1:123:orders" }
            ]
        });

        let lines = render_overview_banner(&overview, &data);
        let text: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
        let joined = text.join("\n");

        assert!(joined.contains("hello-world"), "banner names the function");
        assert!(
            joined.contains("a test function"),
            "banner carries subtitle"
        );
        assert!(joined.contains("Runtime: nodejs18.x"), "chip renders");
        assert!(joined.contains("Size: 1.0 MB"), "chip transform applies");
        assert!(joined.contains("TRIGGERS"), "resource group heading");
        assert!(
            joined.contains("arn:aws:sqs:eu-west-1:123:orders"),
            "trigger node renders via item_template"
        );
    }

    #[test]
    fn overview_banner_shows_none_for_empty_resource_groups() {
        use crate::resource::protocol::{OverviewConfig, OverviewResource};

        let overview = OverviewConfig {
            title_source: "/FunctionName".to_string(),
            subtitle_source: None,
            chips: vec![],
            resources: vec![OverviewResource {
                label: "TRIGGERS".to_string(),
                source: "/EventSourceMappings".to_string(),
                item_template: None,
            }],
        };

        let data = serde_json::json!({ "FunctionName": "quiet", "EventSourceMappings": [] });

        let lines = render_overview_banner(&overview, &data);
        let joined: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
        assert!(
            joined.iter().any(|l| l.contains("(none)")),
            "an empty trigger list should render (none)"
        );
    }
}
