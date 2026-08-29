mod app;
mod aws;
mod completion;
mod config;
mod demo;
mod event;
mod resource;
mod ui;
mod version_check;

/// Version from Cargo.toml, embedded at compile time.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

use anyhow::Result;
use app::{App, Mode, SsoLoginState};
use aws::client::ClientResult;
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::{generate, Shell};
use config::Config;
use crossterm::{
    event::{poll, read, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;
use std::io;
use std::path::PathBuf;
use std::time::Duration;
use tracing::Level;
use tracing_subscriber::fmt::writer::MakeWriterExt;
use ui::splash::{render as render_splash, SplashState};

/// Terminal UI for AWS
#[derive(Parser, Debug)]
#[command(name = "orbit", version, about, long_about = None)]
struct Args {
    /// AWS profile to use
    #[arg(short, long)]
    profile: Option<String>,

    /// AWS region to use
    #[arg(short, long)]
    region: Option<String>,

    /// Log level for debugging (logs to platform config dir: Linux ~/.config/orbit/orbit.log, macOS ~/Library/Application Support/orbit/orbit.log, Windows %APPDATA%/orbit/orbit.log)
    #[arg(long, value_enum, default_value = "off")]
    log_level: LogLevel,

    /// Run in read-only mode (block all write operations). This is the default.
    #[arg(long, default_value = "true")]
    readonly: bool,

    /// Run in write mode (allow all write operations). Overrides --readonly.
    #[arg(long)]
    write: bool,

    /// Custom AWS endpoint URL (for LocalStack, etc.). Also reads from AWS_ENDPOINT_URL env var.
    #[arg(long)]
    endpoint_url: Option<String>,

    /// Force a version check on startup and prompt to update if a newer
    /// version exists, ignoring the once-a-day auto-check cache.
    #[arg(long)]
    update: bool,

    /// Run with synthetic demo data (no AWS connection required).
    /// Bare `--demo` shows EC2 instances. `--demo all` shows everything.
    /// Or choose specific resources: `--demo ec2-instances,route53-hosted-zones`.
    #[arg(long, num_args = 0..=1, default_missing_value = "ec2-instances")]
    demo: Option<String>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Generate shell completion scripts
    Completion {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: Shell,
    },
    /// List available AWS profiles (for shell completion)
    #[command(hide = true)]
    ListProfiles,
    /// List available AWS regions (for shell completion)
    #[command(hide = true)]
    ListRegions,
    /// Internal: apply a staged self-update then relaunch (hidden helper)
    #[command(hide = true)]
    UpdateApply {
        /// Path to the staged new binary
        staged_path: String,
        /// Path of the currently-running binary to replace
        current_path: String,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum LogLevel {
    Off,
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    fn to_tracing_level(self) -> Option<Level> {
        match self {
            LogLevel::Off => None,
            LogLevel::Error => Some(Level::ERROR),
            LogLevel::Warn => Some(Level::WARN),
            LogLevel::Info => Some(Level::INFO),
            LogLevel::Debug => Some(Level::DEBUG),
            LogLevel::Trace => Some(Level::TRACE),
        }
    }
}

fn setup_logging(level: LogLevel) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let tracing_level = level.to_tracing_level()?;

    // Get log file path
    let log_path = get_log_path();

    // Ensure parent directory exists
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // Create file appender
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .expect("Failed to open log file");

    let (non_blocking, guard) = tracing_appender::non_blocking(file);

    tracing_subscriber::fmt()
        .with_max_level(tracing_level)
        .with_writer(non_blocking.with_max_level(tracing_level))
        .with_ansi(false)
        .with_target(true)
        .with_thread_ids(false)
        .with_file(true)
        .with_line_number(true)
        .init();

    tracing::info!("orbit started with log level: {:?}", level);
    tracing::info!("Log file: {:?}", log_path);

    Some(guard)
}

fn get_log_path() -> PathBuf {
    if let Some(config_dir) = dirs::config_dir() {
        return config_dir.join("orbit").join("orbit.log");
    }
    if let Some(home) = dirs::home_dir() {
        return home.join(".orbit").join("orbit.log");
    }
    PathBuf::from("orbit.log")
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse CLI arguments
    let args = Args::parse();

    // Handle subcommands that don't need TUI
    match &args.command {
        Some(Command::Completion { shell }) => {
            match shell {
                Shell::Bash => print!("{}", completion::generate_bash()),
                Shell::Zsh => print!("{}", completion::generate_zsh()),
                Shell::Fish => print!("{}", completion::generate_fish()),
                Shell::PowerShell => print!("{}", completion::generate_powershell()),
                _ => {
                    // Fall back to clap's default for other shells (e.g., Elvish)
                    let mut cmd = Args::command();
                    generate(*shell, &mut cmd, "orbit", &mut std::io::stdout());
                }
            }
            return Ok(());
        }
        Some(Command::ListProfiles) => {
            // Output profiles for shell completion
            if let Ok(profiles) = aws::profiles::list_profiles() {
                for profile in profiles {
                    println!("{}", profile);
                }
            }
            return Ok(());
        }
        Some(Command::ListRegions) => {
            // Output regions for shell completion
            for region in aws::profiles::list_regions() {
                println!("{}", region);
            }
            return Ok(());
        }
        Some(Command::UpdateApply {
            staged_path,
            current_path,
        }) => {
            version_check::apply_update(staged_path, current_path);
            return Ok(());
        }
        None => {}
    }

    // Setup logging (keep guard alive for the duration of the program)
    let _log_guard = setup_logging(args.log_level);

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Show splash screen and initialize
    let result = initialize_with_splash(&mut terminal, &args).await;

    match result {
        Ok(Some(mut app)) => {
            // Run the main app
            let run_result = run_app(&mut terminal, &mut app, args.update).await;

            // Restore terminal
            cleanup_terminal(&mut terminal)?;

            if let Err(err) = run_result {
                eprintln!("Error: {err:?}");
            }
        }
        Ok(None) => {
            // User aborted during initialization
            cleanup_terminal(&mut terminal)?;
        }
        Err(err) => {
            // Restore terminal before showing error
            cleanup_terminal(&mut terminal)?;
            eprintln!("Initialization error: {err:?}");
        }
    }

    Ok(())
}

fn cleanup_terminal<B: Backend + std::io::Write>(terminal: &mut Terminal<B>) -> Result<()>
where
    B::Error: Send + Sync + 'static,
{
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

/// Result of initialization - either an App or SSO login is required
#[allow(clippy::large_enum_variant)]
enum InitResult {
    App(App),
    /// SSO login required (IAM Identity Center) - user needs `aws sso login`
    SsoRequired {
        profile: String,
        sso_session: String,
        region: String,
        endpoint_url: Option<String>,
        config: Config,
        available_profiles: Vec<String>,
        available_regions: Vec<String>,
        readonly: bool,
    },
    /// Console login required - user needs `aws login`
    ConsoleLoginRequired {
        profile: String,
        login_session: String,
        region: String,
        endpoint_url: Option<String>,
        config: Config,
        available_profiles: Vec<String>,
        available_regions: Vec<String>,
        readonly: bool,
    },
}

async fn initialize_with_splash<B: Backend>(
    terminal: &mut Terminal<B>,
    args: &Args,
) -> Result<Option<App>>
where
    B::Error: Send + Sync + 'static,
{
    match initialize_inner(terminal, args).await? {
        None => Ok(None), // User aborted
        Some(InitResult::App(app)) => Ok(Some(app)),
        Some(InitResult::SsoRequired {
            profile,
            sso_session,
            region,
            endpoint_url,
            config,
            available_profiles,
            available_regions,
            readonly,
        }) => {
            // Handle SSO login flow (aws sso login)
            handle_sso_login_flow(
                terminal,
                profile,
                sso_session,
                region,
                endpoint_url,
                config,
                available_profiles,
                available_regions,
                readonly,
            )
            .await
        }
        Some(InitResult::ConsoleLoginRequired {
            profile,
            login_session,
            region,
            endpoint_url,
            config,
            available_profiles,
            available_regions,
            readonly,
        }) => {
            // Handle console login flow (aws login)
            handle_console_login_flow(
                terminal,
                profile,
                login_session,
                region,
                endpoint_url,
                config,
                available_profiles,
                available_regions,
                readonly,
            )
            .await
        }
    }
}

async fn initialize_inner<B: Backend>(
    terminal: &mut Terminal<B>,
    args: &Args,
) -> Result<Option<InitResult>>
where
    B::Error: Send + Sync + 'static,
{
    let readonly = !args.write && args.readonly;

    let mut splash = SplashState::new(readonly);

    // Render initial splash
    terminal.draw(|f| render_splash(f, &splash))?;

    // Check for abort
    if check_abort()? {
        return Ok(None);
    }

    // Step 1: Load configuration (CLI args > env vars > saved config)
    let config = Config::load();
    let profile = args
        .profile
        .clone()
        .unwrap_or_else(|| config.effective_profile());
    let region = args
        .region
        .clone()
        .unwrap_or_else(|| config.effective_region());

    // Get endpoint URL from CLI arg or environment variable
    let endpoint_url = args
        .endpoint_url
        .clone()
        .or_else(|| std::env::var("AWS_ENDPOINT_URL").ok());

    tracing::info!(
        "Using profile: {}, region: {}, endpoint_url: {:?}",
        profile,
        region,
        endpoint_url
    );

    splash.set_message(&format!("Loading AWS config [profile: {}]", profile));
    terminal.draw(|f| render_splash(f, &splash))?;
    splash.complete_step();

    if check_abort()? {
        return Ok(None);
    }

    // Step 2: Load profiles early (needed for SSO flow too)
    splash.set_message("Reading ~/.aws/config");
    terminal.draw(|f| render_splash(f, &splash))?;

    let available_profiles =
        aws::profiles::list_profiles().unwrap_or_else(|_| vec!["default".to_string()]);
    let available_regions = aws::profiles::list_regions();
    splash.complete_step();

    if check_abort()? {
        return Ok(None);
    }

    // Step 3: Initialize AWS clients (or use dummy in demo mode)
    let (clients, actual_region) = if args.demo.is_some() {
        splash.set_message("Demo mode — no AWS connection");
        terminal.draw(|f| render_splash(f, &splash))?;
        (aws::client::AwsClients::dummy(), "eu-west-1".to_string())
    } else {
        splash.set_message(&format!("Connecting to AWS services [{}]", region));
        terminal.draw(|f| render_splash(f, &splash))?;

        let client_result =
            aws::client::AwsClients::new_with_sso_check(&profile, &region, endpoint_url.clone())
                .await?;

        match client_result {
            ClientResult::Ok(clients, actual_region) => (clients, actual_region),
            ClientResult::SsoLoginRequired {
                profile,
                sso_session,
                region,
                endpoint_url,
            } => {
                tracing::debug!(
                    "SSO login required for profile '{}', session '{}' - showing login dialog",
                    profile,
                    sso_session
                );
                return Ok(Some(InitResult::SsoRequired {
                    profile,
                    sso_session,
                    region,
                    endpoint_url,
                    config,
                    available_profiles,
                    available_regions,
                    readonly,
                }));
            }
            ClientResult::ConsoleLoginRequired {
                profile,
                login_session,
                region,
                endpoint_url,
            } => {
                tracing::debug!(
                    "Console login required for profile '{}', session '{}' - showing login dialog",
                    profile,
                    login_session
                );
                return Ok(Some(InitResult::ConsoleLoginRequired {
                    profile,
                    login_session,
                    region,
                    endpoint_url,
                    config,
                    available_profiles,
                    available_regions,
                    readonly,
                }));
            }
        }
    };

    splash.complete_step();

    if check_abort()? {
        return Ok(None);
    }

    // Step 4: Fetch EC2 instances (or load demo data)
    let (instances, initial_error, demo, first_resource) = if let Some(ref selection) = args.demo {
        splash.set_message("Loading demo data");
        terminal.draw(|f| render_splash(f, &splash))?;
        let keys: Vec<&str> = selection.split(',').map(|s| s.trim()).collect();
        let (demo_data, initial_key) = demo::load(&keys);
        let instances = demo_data.get(&*initial_key).cloned().unwrap_or_default();
        (instances, None, true, initial_key)
    } else {
        splash.set_message(&format!("Fetching instances from {}", actual_region));
        terminal.draw(|f| render_splash(f, &splash))?;

        match resource::fetch_resources_paginated("ec2-instances", &clients, &[], None).await {
            Ok(result) => (result.items, None, false, "ec2-instances".to_string()),
            Err(e) => {
                let error_msg = aws::client::format_aws_error(&e);
                (
                    Vec::new(),
                    Some(error_msg),
                    false,
                    "ec2-instances".to_string(),
                )
            }
        }
    };

    splash.complete_step();
    splash.set_message("Ready!");
    terminal.draw(|f| render_splash(f, &splash))?;

    // Small delay to show completion
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Create the app with config
    let mut app = App::from_initialized(
        clients,
        profile,
        actual_region,
        available_profiles,
        available_regions,
        instances,
        config,
        readonly,
        endpoint_url,
        demo,
        &first_resource,
    );

    // Set initial error if any
    if let Some(err) = initial_error {
        app.error_message = Some(err);
    }

    Ok(Some(InitResult::App(app)))
}

/// Handle SSO login flow interactively
#[allow(clippy::too_many_arguments)]
async fn handle_sso_login_flow<B: Backend>(
    terminal: &mut Terminal<B>,
    profile: String,
    sso_session: String,
    region: String,
    endpoint_url: Option<String>,
    config: Config,
    available_profiles: Vec<String>,
    available_regions: Vec<String>,
    readonly: bool,
) -> Result<Option<App>>
where
    B::Error: Send + Sync + 'static,
{
    use aws::sso;

    tracing::info!(
        "Entering SSO login flow for profile '{}', session '{}'",
        profile,
        sso_session
    );

    // Create a minimal app state for the SSO dialog
    let mut sso_state = SsoLoginState::Prompt {
        profile: profile.clone(),
        sso_session: sso_session.clone(),
    };

    loop {
        // Render SSO dialog
        terminal.draw(|f| {
            render_sso_standalone(f, &sso_state);
        })?;

        // Handle input
        if poll(Duration::from_millis(100))? {
            if let Event::Key(key) = read()? {
                match &sso_state {
                    SsoLoginState::Prompt { profile, .. } => {
                        match key.code {
                            KeyCode::Enter => {
                                // First check if we already have a valid cached token (e.g., from aws sso login)
                                let profile_clone = profile.clone();

                                enum SsoStartResult {
                                    ExistingToken(String),
                                    NeedAuth {
                                        profile: String,
                                        device_auth: sso::DeviceAuthInfo,
                                        sso_region: String,
                                    },
                                    Error(String),
                                }

                                let result = tokio::task::spawn_blocking(move || {
                                    let sso_config = match sso::get_sso_config(&profile_clone) {
                                        Some(c) => c,
                                        None => {
                                            return SsoStartResult::Error(format!(
                                                "SSO config not found for profile '{}'",
                                                profile_clone
                                            ))
                                        }
                                    };

                                    // Check for existing valid token first
                                    if let Some(_token) = sso::check_existing_token(&sso_config) {
                                        return SsoStartResult::ExistingToken(profile_clone);
                                    }

                                    // No valid token, start device authorization
                                    match sso::start_device_authorization(&sso_config) {
                                        Ok(device_auth) => {
                                            // Open browser
                                            let _ = sso::open_sso_browser(
                                                &device_auth.verification_uri_complete,
                                            );
                                            SsoStartResult::NeedAuth {
                                                profile: profile_clone,
                                                device_auth,
                                                sso_region: sso_config.sso_region,
                                            }
                                        }
                                        Err(e) => SsoStartResult::Error(format!(
                                            "Failed to start SSO: {}",
                                            e
                                        )),
                                    }
                                })
                                .await?;

                                match result {
                                    SsoStartResult::ExistingToken(prof) => {
                                        // Already have valid token, skip straight to success
                                        sso_state = SsoLoginState::Success { profile: prof };
                                    }
                                    SsoStartResult::NeedAuth {
                                        profile: prof,
                                        device_auth,
                                        sso_region,
                                    } => {
                                        sso_state = SsoLoginState::WaitingForAuth {
                                            profile: prof,
                                            user_code: device_auth.user_code,
                                            verification_uri: device_auth.verification_uri,
                                            device_code: device_auth.device_code,
                                            interval: device_auth.interval as u64,
                                            sso_region,
                                        };
                                    }
                                    SsoStartResult::Error(e) => {
                                        sso_state = SsoLoginState::Failed { error: e };
                                    }
                                }
                            }
                            KeyCode::Esc | KeyCode::Char('q') => {
                                return Ok(None); // User cancelled
                            }
                            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                return Ok(None);
                            }
                            _ => {}
                        }
                    }
                    SsoLoginState::WaitingForAuth { profile, .. } => {
                        match key.code {
                            KeyCode::Esc => {
                                return Ok(None); // User cancelled
                            }
                            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                return Ok(None);
                            }
                            _ => {
                                // Any other key - continue polling
                            }
                        }

                        // Poll for token - run blocking code on separate thread
                        let profile_clone = profile.clone();
                        let result = tokio::task::spawn_blocking(move || {
                            if let Some(sso_config) = sso::get_sso_config(&profile_clone) {
                                match sso::poll_for_token(&sso_config) {
                                    Ok(Some(_token)) => Ok(Some(profile_clone)),
                                    Ok(None) => Ok(None),
                                    Err(e) => Err(e.to_string()),
                                }
                            } else {
                                Ok(None)
                            }
                        })
                        .await?;

                        match result {
                            Ok(Some(prof)) => {
                                sso_state = SsoLoginState::Success { profile: prof };
                            }
                            Ok(None) => {
                                // Still pending
                            }
                            Err(e) => {
                                sso_state = SsoLoginState::Failed { error: e };
                            }
                        }
                    }
                    SsoLoginState::Success {
                        profile: _sso_profile,
                    } => {
                        // Note: _sso_profile should match the outer `profile` variable for initial SSO
                        match key.code {
                            KeyCode::Enter | KeyCode::Esc => {
                                // SSO successful - now create the client and continue initialization
                                // AwsClients::new handles blocking internally via spawn_blocking
                                let (clients, actual_region) = aws::client::AwsClients::new(
                                    &profile,
                                    &region,
                                    endpoint_url.clone(),
                                )
                                .await?;

                                // Fetch initial resources
                                let (instances, initial_error) = {
                                    match resource::fetch_resources_paginated(
                                        "ec2-instances",
                                        &clients,
                                        &[],
                                        None,
                                    )
                                    .await
                                    {
                                        Ok(result) => (result.items, None),
                                        Err(e) => {
                                            let error_msg = aws::client::format_aws_error(&e);
                                            (Vec::new(), Some(error_msg))
                                        }
                                    }
                                };

                                let mut app = App::from_initialized(
                                    clients,
                                    profile,
                                    actual_region,
                                    available_profiles,
                                    available_regions,
                                    instances,
                                    config,
                                    readonly,
                                    endpoint_url,
                                    false,
                                    "ec2-instances",
                                );

                                if let Some(err) = initial_error {
                                    app.error_message = Some(err);
                                }

                                return Ok(Some(app));
                            }
                            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                return Ok(None);
                            }
                            _ => {}
                        }
                    }
                    SsoLoginState::Failed { .. } => {
                        match key.code {
                            KeyCode::Enter | KeyCode::Esc => {
                                return Ok(None); // Exit on failure
                            }
                            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                return Ok(None);
                            }
                            _ => {}
                        }
                    }
                }
            }
        } else {
            // No key event - poll for SSO if waiting
            if let SsoLoginState::WaitingForAuth {
                profile: waiting_profile,
                ..
            } = &sso_state
            {
                let waiting_profile = waiting_profile.clone();
                let result = tokio::task::spawn_blocking(move || {
                    if let Some(sso_config) = sso::get_sso_config(&waiting_profile) {
                        match sso::poll_for_token(&sso_config) {
                            Ok(Some(_token)) => Ok(Some(waiting_profile)),
                            Ok(None) => Ok(None),
                            Err(e) => Err(e.to_string()),
                        }
                    } else {
                        Ok(None)
                    }
                })
                .await?;

                match result {
                    Ok(Some(prof)) => {
                        sso_state = SsoLoginState::Success { profile: prof };
                    }
                    Ok(None) => {
                        // Still pending
                    }
                    Err(e) => {
                        sso_state = SsoLoginState::Failed { error: e };
                    }
                }
            }
        }
    }
}

/// Render SSO dialog standalone (during initialization, before app is created)
fn render_sso_standalone(f: &mut ratatui::Frame, sso_state: &SsoLoginState) {
    use ratatui::{
        layout::{Alignment, Constraint, Direction, Layout, Rect},
        style::{Color, Modifier, Style},
        text::{Line, Span},
        widgets::{Block, Borders, Clear, Paragraph},
    };

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

    // Clear the screen with a dark background
    let area = f.area();
    f.render_widget(Clear, area);
    let bg_block = Block::default().style(Style::default().bg(Color::Black));
    f.render_widget(bg_block, area);

    match sso_state {
        SsoLoginState::Prompt {
            profile,
            sso_session,
        } => {
            let dialog_area = centered_rect(70, 10, area);
            f.render_widget(Clear, dialog_area);

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
            f.render_widget(paragraph, dialog_area);
        }

        SsoLoginState::WaitingForAuth {
            user_code,
            verification_uri,
            ..
        } => {
            let dialog_area = centered_rect(70, 12, area);
            f.render_widget(Clear, dialog_area);

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
            f.render_widget(paragraph, dialog_area);
        }

        SsoLoginState::Success { profile } => {
            let dialog_area = centered_rect(50, 7, area);
            f.render_widget(Clear, dialog_area);

            let text = vec![
                Line::from(Span::styled(
                    "<SSO Login Successful>",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    format!("Authenticated '{}'. Press Enter to continue.", profile),
                    Style::default().fg(Color::White),
                )),
            ];

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Green));

            let paragraph = Paragraph::new(text)
                .block(block)
                .alignment(Alignment::Center);
            f.render_widget(paragraph, dialog_area);
        }

        SsoLoginState::Failed { error } => {
            let dialog_area = centered_rect(70, 9, area);
            f.render_widget(Clear, dialog_area);

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
                    "Press Enter or Esc to exit",
                    Style::default().fg(Color::DarkGray),
                )),
            ];

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Red));

            let paragraph = Paragraph::new(text)
                .block(block)
                .alignment(Alignment::Center);
            f.render_widget(paragraph, dialog_area);
        }
    }
}

/// Handle console login flow (aws login command via subprocess)
#[allow(clippy::too_many_arguments)]
async fn handle_console_login_flow<B: Backend>(
    terminal: &mut Terminal<B>,
    profile: String,
    login_session: String,
    region: String,
    endpoint_url: Option<String>,
    config: Config,
    available_profiles: Vec<String>,
    available_regions: Vec<String>,
    readonly: bool,
) -> Result<Option<App>>
where
    B::Error: Send + Sync + 'static,
{
    use app::ConsoleLoginState;
    use aws::console_login;

    tracing::info!(
        "Entering console login flow for profile '{}', session '{}'",
        profile,
        login_session
    );

    let mut console_state = ConsoleLoginState::Prompt {
        profile: profile.clone(),
        login_session: login_session.clone(),
    };
    let mut child_process: Option<std::process::Child> = None;
    let mut login_rx: Option<std::sync::mpsc::Receiver<console_login::LoginInfo>> = None;

    loop {
        // Render console login dialog
        terminal.draw(|f| {
            render_console_login_standalone(f, &console_state);
        })?;

        // Poll child process status if waiting
        if let ConsoleLoginState::WaitingForAuth {
            profile: waiting_profile,
            login_session: waiting_login_session,
            url: current_url,
        } = &console_state
        {
            // Check for URL updates from the receiver
            if let Some(ref rx) = login_rx {
                if let Ok(info) = rx.try_recv() {
                    if info.url.is_some() {
                        console_state = ConsoleLoginState::WaitingForAuth {
                            profile: waiting_profile.clone(),
                            login_session: waiting_login_session.clone(),
                            url: info.url,
                        };
                        continue;
                    }
                }
            }

            if let Some(ref mut child) = child_process {
                match console_login::check_login_status(child) {
                    Ok(Some(true)) => {
                        // Success!
                        child_process = None;
                        login_rx = None;
                        console_state = ConsoleLoginState::Success {
                            profile: waiting_profile.clone(),
                        };
                        continue;
                    }
                    Ok(Some(false)) => {
                        // Failed - get error message from stderr
                        let error = console_login::read_child_stderr(child)
                            .unwrap_or_else(|| "aws login command failed".to_string());
                        child_process = None;
                        login_rx = None;
                        console_state = ConsoleLoginState::Failed {
                            profile: waiting_profile.clone(),
                            error,
                        };
                        continue;
                    }
                    Ok(None) => {
                        // Still running - preserve current URL state
                        let _ = current_url; // Suppress unused warning
                    }
                    Err(e) => {
                        child_process = None;
                        login_rx = None;
                        console_state = ConsoleLoginState::Failed {
                            profile: waiting_profile.clone(),
                            error: format!("Error checking login status: {}", e),
                        };
                        continue;
                    }
                }
            }
        }

        // Handle input
        if poll(Duration::from_millis(100))? {
            if let Event::Key(key) = read()? {
                match &console_state {
                    ConsoleLoginState::Prompt {
                        profile: prompt_profile,
                        login_session: prompt_login_session,
                    } => {
                        match key.code {
                            KeyCode::Enter => {
                                // Check if AWS CLI supports `aws login`
                                if !console_login::is_aws_login_available() {
                                    console_state = ConsoleLoginState::Failed {
                                        profile: prompt_profile.clone(),
                                        error: "AWS CLI v2.32.0+ required for 'aws login' command. Please upgrade your AWS CLI.".to_string(),
                                    };
                                    continue;
                                }

                                // Spawn `aws login` subprocess
                                match console_login::spawn_aws_login(prompt_profile, &region) {
                                    Ok((child, rx)) => {
                                        child_process = Some(child);
                                        login_rx = Some(rx);
                                        console_state = ConsoleLoginState::WaitingForAuth {
                                            profile: prompt_profile.clone(),
                                            login_session: prompt_login_session.clone(),
                                            url: None,
                                        };
                                    }
                                    Err(e) => {
                                        console_state = ConsoleLoginState::Failed {
                                            profile: prompt_profile.clone(),
                                            error: format!("Failed to spawn aws login: {}", e),
                                        };
                                    }
                                }
                            }
                            KeyCode::Esc | KeyCode::Char('q') => {
                                return Ok(None); // User cancelled
                            }
                            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                return Ok(None);
                            }
                            _ => {}
                        }
                    }
                    ConsoleLoginState::WaitingForAuth { .. } => {
                        match key.code {
                            KeyCode::Esc => {
                                // Kill the subprocess and cancel
                                if let Some(mut child) = child_process.take() {
                                    let _ = child.kill();
                                }
                                return Ok(None);
                            }
                            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                if let Some(mut child) = child_process.take() {
                                    let _ = child.kill();
                                }
                                return Ok(None);
                            }
                            _ => {
                                // Continue waiting
                            }
                        }
                    }
                    ConsoleLoginState::Success { .. } => {
                        match key.code {
                            KeyCode::Enter | KeyCode::Esc => {
                                // Console login successful - create the client and continue
                                let (clients, actual_region) = aws::client::AwsClients::new(
                                    &profile,
                                    &region,
                                    endpoint_url.clone(),
                                )
                                .await?;

                                // Fetch initial resources
                                let (instances, initial_error) = {
                                    match resource::fetch_resources_paginated(
                                        "ec2-instances",
                                        &clients,
                                        &[],
                                        None,
                                    )
                                    .await
                                    {
                                        Ok(result) => (result.items, None),
                                        Err(e) => {
                                            let error_msg = aws::client::format_aws_error(&e);
                                            (Vec::new(), Some(error_msg))
                                        }
                                    }
                                };

                                let mut app = App::from_initialized(
                                    clients,
                                    profile,
                                    actual_region,
                                    available_profiles,
                                    available_regions,
                                    instances,
                                    config,
                                    readonly,
                                    endpoint_url,
                                    false,
                                    "ec2-instances",
                                );

                                if let Some(err) = initial_error {
                                    app.error_message = Some(err);
                                }

                                return Ok(Some(app));
                            }
                            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                return Ok(None);
                            }
                            _ => {}
                        }
                    }
                    ConsoleLoginState::Failed { .. } => {
                        match key.code {
                            KeyCode::Enter => {
                                // Retry - go back to prompt state
                                console_state = ConsoleLoginState::Prompt {
                                    profile: profile.clone(),
                                    login_session: login_session.clone(),
                                };
                            }
                            KeyCode::Esc => {
                                return Ok(None); // Exit on failure
                            }
                            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                return Ok(None);
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }
}

/// Render console login dialog standalone (during initialization, before app is created)
fn render_console_login_standalone(f: &mut ratatui::Frame, console_state: &app::ConsoleLoginState) {
    use app::ConsoleLoginState;
    use ratatui::{
        layout::{Alignment, Constraint, Direction, Layout, Rect},
        style::{Color, Modifier, Style},
        text::{Line, Span},
        widgets::{Block, Borders, Clear, Paragraph},
    };

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

    // Clear the screen with a dark background
    let area = f.area();
    f.render_widget(Clear, area);
    let bg_block = Block::default().style(Style::default().bg(Color::Black));
    f.render_widget(bg_block, area);

    match console_state {
        ConsoleLoginState::Prompt {
            profile,
            login_session,
        } => {
            let dialog_area = centered_rect(70, 12, area);
            f.render_widget(Clear, dialog_area);

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
            f.render_widget(paragraph, dialog_area);
        }

        ConsoleLoginState::WaitingForAuth { profile, url, .. } => {
            // Adjust height based on whether URL is shown
            let height = if url.is_some() { 14 } else { 11 };
            let dialog_area = centered_rect(70, height, area);
            f.render_widget(Clear, dialog_area);

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
            f.render_widget(paragraph, dialog_area);
        }

        ConsoleLoginState::Success { profile } => {
            let dialog_area = centered_rect(50, 7, area);
            f.render_widget(Clear, dialog_area);

            let text = vec![
                Line::from(Span::styled(
                    "<Console Login Successful>",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    format!("Authenticated '{}'. Press Enter to continue.", profile),
                    Style::default().fg(Color::White),
                )),
            ];

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Green));

            let paragraph = Paragraph::new(text)
                .block(block)
                .alignment(Alignment::Center);
            f.render_widget(paragraph, dialog_area);
        }

        ConsoleLoginState::Failed { error, .. } => {
            let dialog_area = centered_rect(70, 9, area);
            f.render_widget(Clear, dialog_area);

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
                    "Press Enter to retry, Esc to exit",
                    Style::default().fg(Color::DarkGray),
                )),
            ];

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Red));

            let paragraph = Paragraph::new(text)
                .block(block)
                .alignment(Alignment::Center);
            f.render_widget(paragraph, dialog_area);
        }
    }
}

fn check_abort() -> Result<bool> {
    if poll(Duration::from_millis(50))? {
        if let Event::Key(key) = read()? {
            if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Full-screen dialog shown while an S3 object downloads. Repainted live from
/// `app.download_progress`, which the main loop updates between chunks.
fn render_download_standalone(f: &mut ratatui::Frame, req: &app::PendingDownload, app: &app::App) {
    use ratatui::{
        layout::{Alignment, Constraint, Direction, Layout},
        style::{Color, Modifier, Style},
        text::{Line, Span},
        widgets::{Block, Borders, Clear, Gauge, Paragraph},
    };

    let area = f.area();
    f.render_widget(Clear, area);
    f.render_widget(
        Block::default().style(Style::default().bg(Color::Black)),
        area,
    );

    // Centered dialog, sized to the progress bar.
    let dialog_area = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(42),
            Constraint::Length(9),
            Constraint::Percentage(42),
        ])
        .split(area)[1];
    let dialog_area = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(25),
            Constraint::Percentage(50),
            Constraint::Percentage(25),
        ])
        .split(dialog_area)[1];
    f.render_widget(Clear, dialog_area);

    let (downloaded, total) = app.download_progress.unwrap_or((0, None));

    let pct = match total {
        Some(total) if total > 0 => ((downloaded.min(total) as f64 / total as f64) * 100.0) as u16,
        _ => 0,
    };

    let text = vec![
        Line::from(Span::styled(
            "<Downloading>",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            format!("{} → {}", req.bucket, req.key),
            Style::default().fg(Color::White),
        )),
        Line::from(""),
        Line::from(Span::styled(
            match total {
                Some(total) => format!("{} / {} bytes ({}%)", downloaded, total, pct),
                None => format!("{} bytes", downloaded),
            },
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    // Stack the text paragraph and the gauge in the dialog's inner area.
    let inner = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(6), Constraint::Length(1)])
        .split(block.inner(dialog_area));

    f.render_widget(
        Paragraph::new(text)
            .block(block)
            .alignment(Alignment::Center),
        dialog_area,
    );

    let gauge = Gauge::default()
        .block(Block::default())
        .gauge_style(Style::default().fg(Color::Cyan))
        .ratio(pct as f64 / 100.0);
    f.render_widget(gauge, inner[1]);
}

/// Drive a staged S3 download to completion with a live progress bar.
///
/// The event loop cannot repaint while it awaits, so the fetch is spawned on a
/// task that streams progress over a channel; this loop redraws the download
/// dialog on every chunk and returns once the channel closes. The write-back to
/// `app` (status/error) happens only after the loop, so there is no aliasing of
/// the app while it is being read for rendering.
async fn run_download<B: Backend>(terminal: &mut Terminal<B>, app: &mut App) -> Result<()>
where
    B::Error: Send + Sync + 'static,
{
    let Some(req) = app.take_pending_download() else {
        return Ok(());
    };

    let (progress_tx, mut progress_rx) =
        tokio::sync::mpsc::unbounded_channel::<(u64, Option<u64>)>();

    // Clients and the request are copied out so the fetch task owns them and
    // never borrows `app` (the loop below holds `app` immutably for rendering).
    let clients = app.clients.clone();
    let bucket = req.bucket.clone();
    let key = req.key.clone();
    let path = req.path.clone();
    let display_path = req.path.clone();

    let task = tokio::spawn(async move {
        let region = clients.http.get_bucket_region(&bucket).await?;
        clients
            .http
            .get_s3_object_to_file(&bucket, &key, &region, &path, move |downloaded, total| {
                let _ = progress_tx.send((downloaded, total));
            })
            .await
    });

    // Repaint on every progress chunk; the sender closing (None) signals done.
    loop {
        terminal.draw(|f| render_download_standalone(f, &req, app))?;

        tokio::select! {
            msg = progress_rx.recv() => {
                match msg {
                    Some((downloaded, total)) => {
                        app.download_progress = Some((downloaded, total));
                    }
                    None => break,
                }
            }
        }
    }

    app.download_progress = None;

    match task.await {
        Ok(Ok(bytes)) => {
            app.status_message = Some(format!(
                "Saved {} bytes to {}",
                bytes,
                display_path.display()
            ));
        }
        Ok(Err(e)) => {
            app.status_message = None;
            app.error_message = Some(format!("Download failed: {}", e));
        }
        Err(e) => {
            app.status_message = None;
            app.error_message = Some(format!("Download task failed: {}", e));
        }
    }

    Ok(())
}

async fn run_app<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    force_update_check: bool,
) -> Result<()>
where
    B::Error: Send + Sync + 'static,
{
    // Kick off the up-to-once-a-day version check in the background so startup
    // is never blocked on the network. The result is polled each loop; when it
    // arrives the Update prompt appears. `--update` forces the check to ignore
    // the daily cache.
    let mut update_rx = {
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let _ = tx.send(
                tokio::task::spawn_blocking(move || {
                    version_check::check_with_cache(force_update_check)
                })
                .await
                .unwrap_or(None),
            );
        });
        Some(rx)
    };

    loop {
        terminal.draw(|f| ui::render(f, app))?;

        // Surface a completed version check as the Update prompt. Polls the
        // channel so the loop stays responsive (never blocks on the network);
        // `None` — up to date or a silent network failure — leaves
        // `update_available` unset, so no prompt appears.
        if app.update_available.is_none() {
            if let Some(rx) = &mut update_rx {
                match rx.try_recv() {
                    Ok(Some(latest)) => {
                        let method = version_check::InstallMethod::detect(
                            std::env::current_exe().unwrap_or_default().as_path(),
                        );
                        app.update_available = Some(app::UpdateInfo {
                            latest,
                            method,
                            in_progress: false,
                            should_quit: false,
                        });
                        app.mode = Mode::Update;
                        update_rx = None;
                    }
                    Ok(None) | Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                        update_rx = None;
                    }
                    Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
                }
            }
        }

        // Handle user input
        if event::handle_events(app).await? {
            return Ok(());
        }

        // A raw-binary self-update handed off a relaunch: quit the current run.
        if app
            .update_available
            .as_ref()
            .map(|u| u.should_quit)
            .unwrap_or(false)
        {
            return Ok(());
        }

        // Execute a staged S3 download with a live progress bar.
        if app.pending_download.is_some() {
            run_download(terminal, app).await?;
        }

        // Handle SSM connect request (requires suspending TUI)
        if let Some(request) = app.take_ssm_connect_request() {
            execute_ssm_connect(terminal, &request)?;
        }

        // Poll SSO if in waiting state
        if app.mode == Mode::SsoLogin {
            event::poll_sso_if_waiting(app).await;
        }

        // Poll console login subprocess if waiting
        if app.mode == Mode::ConsoleLogin {
            event::poll_console_login_if_waiting(app).await;
        }

        // Poll for new log events if in log tail mode
        if app.mode == Mode::LogTail {
            event::poll_logs_if_tailing(app).await;
        }

        // Auto-refresh every 5 seconds (only in Normal mode)
        if app.needs_refresh() {
            let _ = app.refresh_current().await;
        }
    }
}

/// Execute SSM connect by suspending TUI and running aws ssm start-session
fn execute_ssm_connect<B: Backend>(
    terminal: &mut Terminal<B>,
    request: &app::SsmConnectRequest,
) -> Result<()>
where
    B::Error: Send + Sync + 'static,
{
    use std::io::Write;

    // Suspend TUI - restore terminal to normal mode
    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(
        std::io::stdout(),
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::cursor::Show
    )?;

    // Print connection info
    println!(
        "\n\x1b[1;36m>>> Connecting to {} via SSM...\x1b[0m\n",
        request.instance_id
    );
    std::io::stdout().flush()?;

    // Run aws ssm start-session
    let status = std::process::Command::new("aws")
        .args([
            "ssm",
            "start-session",
            "--target",
            &request.instance_id,
            "--region",
            &request.region,
            "--profile",
            &request.profile,
        ])
        .status();

    match status {
        Ok(exit_status) => {
            if !exit_status.success() {
                let code = exit_status.code().unwrap_or(-1);
                println!("\n\x1b[1;33mSSM session exited with code: {}\x1b[0m", code);
                if code == 254 {
                    println!("\x1b[0;33mThis usually means the instance is not connected to SSM.");
                    println!(
                        "Check that SSM Agent is installed and running on the instance.\x1b[0m"
                    );
                }
            }
        }
        Err(e) => {
            println!("\n\x1b[1;31mFailed to start SSM session: {}\x1b[0m", e);
        }
    }

    println!("\n\x1b[1;36m>>> Returning to orbit... Press any key.\x1b[0m");
    std::io::stdout().flush()?;

    // Wait for a key press before restoring TUI
    crossterm::terminal::enable_raw_mode()?;
    let _ = crossterm::event::read(); // Wait for any key
    crossterm::terminal::disable_raw_mode()?;

    // Restore TUI
    crossterm::terminal::enable_raw_mode()?;
    crossterm::execute!(
        std::io::stdout(),
        crossterm::terminal::EnterAlternateScreen,
        crossterm::cursor::Hide
    )?;
    terminal.clear()?;

    Ok(())
}
