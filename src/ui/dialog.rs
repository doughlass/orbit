use crate::app::{App, ConsoleLoginState, Mode, SsoLoginState};
use crate::version_check::InstallMethod;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

pub fn render(f: &mut Frame, app: &App) {
    match app.mode {
        Mode::Confirm => render_confirm_dialog(f, app),
        Mode::Warning => render_warning_dialog(f, app),
        Mode::SsoLogin => render_sso_dialog(f, app),
        Mode::ConsoleLogin => render_console_login_dialog(f, app),
        Mode::Update => render_update_dialog(f, app),
        _ => {}
    }
}

fn render_confirm_dialog(f: &mut Frame, app: &App) {
    let Some(pending) = &app.pending_action else {
        return;
    };

    let area = centered_rect(60, 9, f.area());

    f.render_widget(Clear, area);

    // Determine title color based on destructive flag
    let title_color = if pending.destructive {
        Color::Red
    } else {
        Color::Yellow
    };

    let title = if pending.destructive {
        "Delete"
    } else {
        "Confirm"
    };

    // Build Cancel/OK buttons with selection indicator (Cancel = !selected_yes, OK = selected_yes)
    let cancel_style = if !pending.selected_yes {
        Style::default().fg(Color::Black).bg(Color::Magenta)
    } else {
        Style::default().fg(Color::White)
    };

    let ok_style = if pending.selected_yes {
        Style::default().fg(Color::Black).bg(Color::Magenta)
    } else {
        Style::default().fg(Color::White)
    };

    // Build the dialog content
    let text = vec![
        Line::from(Span::styled(
            format!("<{}>", title),
            Style::default()
                .fg(title_color)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            &pending.message,
            Style::default().fg(Color::White),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(" Cancel ", cancel_style),
            Span::raw("    "),
            Span::styled(" OK ", ok_style),
        ]),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let paragraph = Paragraph::new(text)
        .block(block)
        .alignment(Alignment::Center);

    f.render_widget(paragraph, area);
}

fn render_warning_dialog(f: &mut Frame, app: &App) {
    let Some(message) = &app.warning_message else {
        return;
    };

    // Split message into lines and estimate wrapped line count
    let message_lines: Vec<&str> = message.lines().collect();

    // Estimate height based on content (account for long URLs wrapping)
    let estimated_lines: usize = message_lines
        .iter()
        .map(|line| (line.len() / 70).max(1))
        .sum();

    // Calculate dialog height: header + blank + message lines + blank + button + borders
    let height = (estimated_lines + 6).min(20) as u16;

    let area = centered_rect(75, height, f.area());

    f.render_widget(Clear, area);

    let mut text = vec![
        Line::from(Span::styled(
            "<Warning>",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    // Add each message line
    for line in message_lines {
        text.push(Line::from(Span::styled(
            line,
            Style::default().fg(Color::White),
        )));
    }

    text.push(Line::from(""));
    text.push(Line::from(vec![Span::styled(
        " OK ",
        Style::default().fg(Color::Black).bg(Color::Magenta),
    )]));

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let paragraph = Paragraph::new(text)
        .block(block)
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: false });

    f.render_widget(paragraph, area);
}

fn render_sso_dialog(f: &mut Frame, app: &App) {
    let Some(ref sso_state) = app.sso_state else {
        return;
    };

    match sso_state {
        SsoLoginState::Prompt {
            profile,
            sso_session,
        } => {
            let area = centered_rect(70, 10, f.area());
            f.render_widget(Clear, area);

            let text = vec![
                Line::from(Span::styled(
                    "<SSO Login Required>",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    format!("Profile '{}' requires SSO authentication.", profile),
                    Style::default().fg(Color::White),
                )),
                Line::from(Span::styled(
                    format!("Session: {}", sso_session),
                    Style::default().fg(Color::DarkGray),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "Press Enter to open browser for login, Esc to cancel",
                    Style::default().fg(Color::Yellow),
                )),
            ];

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan));

            let paragraph = Paragraph::new(text)
                .block(block)
                .alignment(Alignment::Center);

            f.render_widget(paragraph, area);
        }

        SsoLoginState::WaitingForAuth {
            user_code,
            verification_uri,
            ..
        } => {
            let area = centered_rect(70, 12, f.area());
            f.render_widget(Clear, area);

            let text = vec![
                Line::from(Span::styled(
                    "<Waiting for SSO Authentication>",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "Complete authentication in your browser.",
                    Style::default().fg(Color::White),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::styled("Code: ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        user_code,
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(vec![
                    Span::styled("URL: ", Style::default().fg(Color::DarkGray)),
                    Span::styled(verification_uri, Style::default().fg(Color::Blue)),
                ]),
                Line::from(""),
                Line::from(Span::styled(
                    "Waiting... (Press Esc to cancel)",
                    Style::default().fg(Color::DarkGray),
                )),
            ];

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow));

            let paragraph = Paragraph::new(text)
                .block(block)
                .alignment(Alignment::Center);

            f.render_widget(paragraph, area);
        }

        SsoLoginState::Success { profile } => {
            let area = centered_rect(50, 7, f.area());
            f.render_widget(Clear, area);

            let text = vec![
                Line::from(Span::styled(
                    "<SSO Login Successful>",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    format!("Authentication complete for '{}'!", profile),
                    Style::default().fg(Color::White),
                )),
            ];

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Green));

            let paragraph = Paragraph::new(text)
                .block(block)
                .alignment(Alignment::Center);

            f.render_widget(paragraph, area);
        }

        SsoLoginState::Failed { error } => {
            let area = centered_rect(70, 9, f.area());
            f.render_widget(Clear, area);

            let text = vec![
                Line::from(Span::styled(
                    "<SSO Login Failed>",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    error.as_str(),
                    Style::default().fg(Color::White),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "Press Enter or Esc to close",
                    Style::default().fg(Color::DarkGray),
                )),
            ];

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Red));

            let paragraph = Paragraph::new(text)
                .block(block)
                .alignment(Alignment::Center);

            f.render_widget(paragraph, area);
        }
    }
}

fn render_console_login_dialog(f: &mut Frame, app: &App) {
    let Some(ref console_state) = app.console_login_state else {
        return;
    };

    match console_state {
        ConsoleLoginState::Prompt {
            profile,
            login_session,
        } => {
            let area = centered_rect(70, 12, f.area());
            f.render_widget(Clear, area);

            let text = vec![
                Line::from(Span::styled(
                    "<Console Login Required>",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    format!("Profile '{}' requires AWS Console login.", profile),
                    Style::default().fg(Color::White),
                )),
                Line::from(Span::styled(
                    format!("Session: {}", login_session),
                    Style::default().fg(Color::DarkGray),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "Press Enter to open browser for login",
                    Style::default().fg(Color::Yellow),
                )),
                Line::from(Span::styled(
                    "(requires AWS CLI v2.32.0+)",
                    Style::default().fg(Color::DarkGray),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "Press Esc to cancel",
                    Style::default().fg(Color::DarkGray),
                )),
            ];

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan));

            let paragraph = Paragraph::new(text)
                .block(block)
                .alignment(Alignment::Center);

            f.render_widget(paragraph, area);
        }

        ConsoleLoginState::WaitingForAuth { profile, url, .. } => {
            // Adjust height based on whether URL is shown
            let height = if url.is_some() { 14 } else { 11 };
            let area = centered_rect(70, height, f.area());
            f.render_widget(Clear, area);

            let mut text = vec![
                Line::from(Span::styled(
                    "<Waiting for Console Authentication>",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "Complete authentication in your browser.",
                    Style::default().fg(Color::White),
                )),
                Line::from(""),
            ];

            // Display URL if available (like SSO does)
            if let Some(ref login_url) = url {
                text.push(Line::from(Span::styled(
                    "If browser didn't open, visit:",
                    Style::default().fg(Color::DarkGray),
                )));
                text.push(Line::from(Span::styled(
                    login_url.as_str(),
                    Style::default().fg(Color::Blue),
                )));
                text.push(Line::from(""));
            }

            text.push(Line::from(Span::styled(
                format!("Profile: {}", profile),
                Style::default().fg(Color::DarkGray),
            )));
            text.push(Line::from(Span::styled(
                "Waiting... (Press Esc to cancel)",
                Style::default().fg(Color::DarkGray),
            )));

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow));

            let paragraph = Paragraph::new(text)
                .block(block)
                .alignment(Alignment::Center);

            f.render_widget(paragraph, area);
        }

        ConsoleLoginState::Success { profile } => {
            let area = centered_rect(50, 7, f.area());
            f.render_widget(Clear, area);

            let text = vec![
                Line::from(Span::styled(
                    "<Console Login Successful>",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    format!("Authentication complete for '{}'!", profile),
                    Style::default().fg(Color::White),
                )),
            ];

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Green));

            let paragraph = Paragraph::new(text)
                .block(block)
                .alignment(Alignment::Center);

            f.render_widget(paragraph, area);
        }

        ConsoleLoginState::Failed { error, .. } => {
            let area = centered_rect(70, 11, f.area());
            f.render_widget(Clear, area);

            let text = vec![
                Line::from(Span::styled(
                    "<Console Login Failed>",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    error.as_str(),
                    Style::default().fg(Color::White),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "Press Enter to retry, Esc to cancel",
                    Style::default().fg(Color::DarkGray),
                )),
            ];

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Red));

            let paragraph = Paragraph::new(text)
                .block(block)
                .alignment(Alignment::Center);

            f.render_widget(paragraph, area);
        }
    }
}

/// Render the "a newer version is available" prompt. The action wording and
/// the command shown depend on how orbit was installed, so the user is never
/// told to do something their package manager will fight.
fn render_update_dialog(f: &mut Frame, app: &App) {
    let Some(info) = &app.update_available else {
        return;
    };

    let area = centered_rect(75, 11, f.area());
    f.render_widget(Clear, area);

    if info.in_progress {
        let text = vec![
            Line::from(Span::styled(
                "<Updating orbit>",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                format!(
                    "Downloading v{} and replacing the current binary...",
                    info.latest
                ),
                Style::default().fg(Color::White),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "orbit will restart automatically.",
                Style::default().fg(Color::DarkGray),
            )),
        ];

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow));
        let paragraph = Paragraph::new(text)
            .block(block)
            .alignment(Alignment::Center);
        f.render_widget(paragraph, area);
        return;
    }

    let action_line = match info.method {
        InstallMethod::Cargo | InstallMethod::Brew => {
            format!("Update with package manager: {}", info.method.label())
        }
        _ => format!("Update method: {}", info.method.label()),
    };

    let text = vec![
        Line::from(Span::styled(
            "<Update available>",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            format!(
                "A new version of orbit is available: v{} (you have v{})",
                info.latest,
                crate::VERSION
            ),
            Style::default().fg(Color::White),
        )),
        Line::from(Span::styled(
            action_line,
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                " [u] update ",
                Style::default().fg(Color::Black).bg(Color::Cyan),
            ),
            Span::raw("    "),
            Span::styled(
                " [c] continue ",
                Style::default().fg(Color::Black).bg(Color::Magenta),
            ),
        ]),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let paragraph = Paragraph::new(text)
        .block(block)
        .alignment(Alignment::Center);
    f.render_widget(paragraph, area);
}

fn centered_rect(percent_x: u16, height: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(40),
            Constraint::Length(height),
            Constraint::Percentage(40),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, UpdateInfo};
    use crate::aws::client::AwsClients;
    use crate::config::Config;
    use crate::version_check::InstallMethod;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn test_app() -> App {
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
            "ec2-instances",
        )
    }

    /// A render-panic here crashes the whole TUI, and the update dialog is the
    /// first thing a user on a stale binary sees, so pin that it draws.
    #[test]
    fn update_dialog_renders_when_update_is_available() {
        let mut app = test_app();
        app.mode = Mode::Update;
        app.update_available = Some(UpdateInfo {
            latest: "9.9.9".to_string(),
            method: InstallMethod::Raw,
            in_progress: false,
            should_quit: false,
        });

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render_update_dialog(f, &app))
            .expect("update dialog must render without panicking");

        let buffer = terminal.backend().buffer();
        let rendered: String = buffer
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<_>>()
            .join("");
        assert!(
            rendered.contains("Update available"),
            "title should be shown, got: {}",
            rendered
        );
    }
}
