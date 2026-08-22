use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

/// The orbit mark, compressed 2:1 vertically with half-blocks so it fits a
/// terminal that is only 30 rows tall. Rows are padded to equal width; the
/// splash is centre-aligned and ragged rows shear sideways.
const SPLASH_ART: [&str; 17] = [
    r"                      ▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄                ██████",
    r"               ▄▄▄█████████████████████████████▄▄▄          ▀▀▀▀ ",
    r"          ▄▄███████████████████████████████████████▀▀            ",
    r"       ▄▄██████████████████▀▀▀▀▀▀▀▀▀▀▀█████████▀▀     ▄██▄       ",
    r"     ▄██████████████▀▀▀                           ▄▄███████▄     ",
    r"   ▄█████████████▀                               ████████████    ",
    r"  ▄████████████▀                                 ▀████████████   ",
    r"  ████████████                                     ████████████  ",
    r" ████████████                                      ████████████  ",
    r" ▀████████████                                     ████████████  ",
    r"  ████████████▄                                   ████████████▀  ",
    r"   ████████████▄                                ▄████████████▀   ",
    r"    ▀█████████▀                             ▄▄██████████████     ",
    r"      ▀████▀▀    ▄▄▄▄▄▄▄▄▄▄▄         ▄▄▄▄▄███████████████▀       ",
    r"        ▀    ▄▄███████████████████████████████████████▀▀         ",
    r"            ▀▀▀███████████████████████████████████▀▀             ",
    r"                  ▀▀▀▀▀███████████████████▀▀▀▀                   ",
];

/// ORBIT in the same box-drawing font the splash has always used. Rows are
/// padded to equal width because the paragraph is centre-aligned: ragged rows
/// each centre differently and the wordmark visibly wobbles. See the tests.
const SPLASH_WORDMARK: [&str; 6] = [
    r" ██████╗ ██████╗ ██████╗ ██╗████████╗",
    r"██╔═══██╗██╔══██╗██╔══██╗██║╚══██╔══╝",
    r"██║   ██║██████╔╝██████╔╝██║   ██║   ",
    r"██║   ██║██╔══██╗██╔══██╗██║   ██║   ",
    r"╚██████╔╝██║  ██║██████╔╝██║   ██║   ",
    r" ╚═════╝ ╚═╝  ╚═╝╚═════╝ ╚═╝   ╚═╝   ",
];

/// Rows the logo block adds beyond the art itself: a blank line, the tagline
/// and the version.
const LOGO_TRAILER_ROWS: u16 = 3;

/// Rows below the logo block: two spacers, the loading bar and the status line.
const BELOW_LOGO_ROWS: u16 = 5;

/// The big art only goes up when the terminal can show all of it plus the
/// loading bar; otherwise it would clip and hide progress. Falls back to the
/// block-letter wordmark.
fn choose_logo(area: Rect) -> &'static [&'static str] {
    let art_width = SPLASH_ART[0].chars().count() as u16;
    let art_height = SPLASH_ART.len() as u16 + LOGO_TRAILER_ROWS + BELOW_LOGO_ROWS;

    if area.width >= art_width && area.height >= art_height {
        &SPLASH_ART
    } else {
        &SPLASH_WORDMARK
    }
}

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
    let logo = choose_logo(area);
    let logo_height = logo.len() as u16 + LOGO_TRAILER_ROWS;

    // Centre the whole block vertically rather than padding by percentage, so
    // the tall art and the short wordmark both sit in the middle.
    let block_height = logo_height + BELOW_LOGO_ROWS;
    let top_pad = area.height.saturating_sub(block_height) / 2;

    let vertical = Layout::vertical([
        Constraint::Length(top_pad),
        Constraint::Length(block_height),
        Constraint::Min(0),
    ])
    .split(area);

    let content = Layout::vertical([
        Constraint::Length(logo_height), // Logo
        Constraint::Length(2),           // Spacer
        Constraint::Length(1),           // Loading bar
        Constraint::Length(1),           // Spacer
        Constraint::Length(1),           // Status message
    ])
    .split(vertical[1]);

    render_big_logo(f, logo, content[0]);
    render_loading_bar(f, splash, content[2]);
    render_status(f, splash, content[4]);
}

fn render_big_logo(f: &mut Frame, logo: &[&str], area: Rect) {
    let brand = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);

    let mut logo_lines: Vec<Line> = logo
        .iter()
        .map(|row| Line::from(Span::styled(*row, brand)))
        .collect();

    logo_lines.push(Line::from(""));
    logo_lines.push(Line::from(Span::styled(
        "Terminal UI explorer for AWS",
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

    /// The whole splash must fit on screen or the loading bar drops off the
    /// bottom, so each logo carries its own minimum terminal height.
    #[test]
    fn each_logo_fits_the_terminal_it_is_chosen_for() {
        let art_rows = SPLASH_ART.len() as u16 + LOGO_TRAILER_ROWS + BELOW_LOGO_ROWS;
        let art_cols = SPLASH_ART[0].chars().count() as u16;

        assert_eq!(
            choose_logo(Rect::new(0, 0, art_cols, art_rows)).len(),
            SPLASH_ART.len()
        );

        // One row or one column short and the art must give way.
        for too_small in [
            Rect::new(0, 0, art_cols, art_rows - 1),
            Rect::new(0, 0, art_cols - 1, art_rows),
        ] {
            let logo = choose_logo(too_small);
            assert_eq!(
                logo.len(),
                SPLASH_WORDMARK.len(),
                "{:?} should fall back",
                too_small
            );
            assert!(
                logo.len() as u16 + LOGO_TRAILER_ROWS + BELOW_LOGO_ROWS <= too_small.height,
                "the fallback wordmark must fit where the art does not"
            );
        }
    }

    /// Centre-aligned art with ragged rows shears sideways, and the trailing
    /// padding that prevents it is invisible in a diff.
    #[test]
    fn splash_art_rows_are_the_same_width() {
        let widths: Vec<usize> = SPLASH_ART.iter().map(|row| width(row)).collect();
        assert!(
            widths.windows(2).all(|pair| pair[0] == pair[1]),
            "splash art rows must line up, got widths {:?}",
            widths
        );
    }
}
