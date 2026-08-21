use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

/// ORBIT in the same box-drawing font the splash has always used. Rows are
/// padded to equal width because the paragraph is centre-aligned: ragged rows
/// each centre differently and the wordmark visibly wobbles. See the tests.
const SPLASH_WORDMARK: [&str; 40] = [
    r" ██████╗ ██████╗ ██████╗ ██╗████████╗",
    r"██╔═══██╗██╔══██╗██╔══██╗██║╚══██╔══╝",
    r"██║   ██║██████╔╝██████╔╝██║   ██║   ",
    r"██║   ██║██╔══██╗██╔══██╗██║   ██║   ",
    r"╚██████╔╝██║  ██║██████╔╝██║   ██║   ",
    r" ╚═════╝ ╚═╝  ╚═╝╚═════╝ ╚═╝   ╚═╝   ",
];
// const SPLASH_WORDMARK: [&str; 6] = [
//     r" ██████╗ ██████╗ ██████╗ ██╗████████╗",
//     r"██╔═══██╗██╔══██╗██╔══██╗██║╚══██╔══╝",
//     r"██║   ██║██████╔╝██████╔╝██║   ██║   ",
//     r"██║   ██║██╔══██╗██╔══██╗██║   ██║   ",
//     r"╚██████╔╝██║  ██║██████╔╝██║   ██║   ",
//     r" ╚═════╝ ╚═╝  ╚═╝╚═════╝ ╚═╝   ╚═╝   ",
// ];

/// Rows reserved for the logo block: the wordmark, a blank line, the tagline
/// and the version.
const LOGO_BLOCK_HEIGHT: u16 = 9;

pub struct SplashState {
    pub current_step: usize,
    pub total_steps: usize,
    pub current_message: String,
    pub spinner_frame: usize,
}

impl SplashState {
    pub fn new() -> Self {
        Self {
            current_step: 0,
            total_steps: 6,
            current_message: "Initializing...".to_string(),
            spinner_frame: 0,
        }
    }

    pub fn set_message(&mut self, message: &str) {
        self.current_message = message.to_string();
        self.spinner_frame = (self.spinner_frame + 1) % 4;
    }

    pub fn complete_step(&mut self) {
        self.current_step += 1;
    }
}

pub fn render(f: &mut Frame, splash: &SplashState) {
    let area = f.area();

    // Center everything vertically
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(25),
            Constraint::Min(15),
            Constraint::Percentage(30),
        ])
        .split(area);

    let center_area = vertical[1];

    // Split center into logo and loading area
    let content = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(LOGO_BLOCK_HEIGHT), // Big logo
            Constraint::Length(2),                 // Spacer
            Constraint::Length(1),                 // Loading bar
            Constraint::Length(1),                 // Spacer
            Constraint::Length(1),                 // Status message
        ])
        .split(center_area);

    // Render big ASCII logo
    render_big_logo(f, content[0]);

    // Render loading bar
    render_loading_bar(f, splash, content[2]);

    // Render status message
    render_status(f, splash, content[4]);
}

fn render_big_logo(f: &mut Frame, area: Rect) {
    let brand = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);

    let mut logo_lines: Vec<Line> = SPLASH_WORDMARK
        .iter()
        .map(|row| Line::from(Span::styled(*row, brand)))
        .collect();

    logo_lines.push(Line::from(""));
    logo_lines.push(Line::from(Span::styled(
        "Terminal UI for AWS",
        Style::default().fg(Color::DarkGray),
    )));
    logo_lines.push(Line::from(Span::styled(
        crate::VERSION,
        Style::default().fg(Color::DarkGray),
    )));

    let paragraph = Paragraph::new(logo_lines).alignment(Alignment::Center);
    f.render_widget(paragraph, area);
}

fn render_loading_bar(f: &mut Frame, splash: &SplashState, area: Rect) {
    let progress = splash.current_step as f64 / splash.total_steps as f64;
    let bar_width = (area.width as usize).saturating_sub(20); // Leave some margin
    let filled = (bar_width as f64 * progress) as usize;
    let empty = bar_width.saturating_sub(filled);

    let bar = Line::from(vec![
        Span::styled("  [", Style::default().fg(Color::DarkGray)),
        Span::styled("█".repeat(filled), Style::default().fg(Color::Cyan)),
        Span::styled("░".repeat(empty), Style::default().fg(Color::DarkGray)),
        Span::styled("]", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!(" {}%", (progress * 100.0) as u8),
            Style::default().fg(Color::White),
        ),
    ]);

    let paragraph = Paragraph::new(bar).alignment(Alignment::Center);
    f.render_widget(paragraph, area);
}

fn render_status(f: &mut Frame, splash: &SplashState, area: Rect) {
    let spinner_chars = ["⠋", "⠙", "⠹", "⠸"];
    let spinner = spinner_chars[splash.spinner_frame % spinner_chars.len()];

    let status = Line::from(vec![
        Span::styled(format!("{} ", spinner), Style::default().fg(Color::Yellow)),
        Span::styled(&splash.current_message, Style::default().fg(Color::White)),
    ]);

    let paragraph = Paragraph::new(status).alignment(Alignment::Center);
    f.render_widget(paragraph, area);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Box-drawing glyphs are one cell wide, so char count is display width.
    fn width(row: &str) -> usize {
        row.chars().count()
    }

    /// Centre-aligned art with ragged rows shears sideways, and the trailing
    /// padding that prevents it is invisible in a diff.
    #[test]
    fn splash_wordmark_rows_are_the_same_width() {
        let widths: Vec<usize> = SPLASH_WORDMARK.iter().map(|row| width(row)).collect();
        assert!(
            widths.windows(2).all(|pair| pair[0] == pair[1]),
            "splash wordmark rows must line up, got widths {:?}",
            widths
        );
    }

    /// The logo block is a fixed-height layout slot, so an extra art row
    /// silently pushes the version line out of view instead of failing.
    #[test]
    fn splash_wordmark_leaves_room_for_tagline_and_version() {
        // Blank spacer, tagline, version.
        let trailing = 3;
        assert!(
            SPLASH_WORDMARK.len() + trailing <= LOGO_BLOCK_HEIGHT as usize,
            "wordmark is {} rows and needs {} more for the tagline and version, \
             but the logo block is only {}",
            SPLASH_WORDMARK.len(),
            trailing,
            LOGO_BLOCK_HEIGHT
        );
    }
}
