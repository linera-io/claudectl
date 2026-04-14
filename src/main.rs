#![allow(unknown_lints)]
#![allow(
    clippy::collapsible_if,
    clippy::manual_is_multiple_of,
    clippy::io_other_error
)]

mod app;
mod config;
mod demo;
mod discovery;
mod history;
mod hooks;
mod logger;
mod models;
mod monitor;
mod orchestrator;
mod process;
mod recorder;
mod session;
mod session_recorder;
mod terminals;
mod theme;
mod transcript;
mod ui;

use std::io;
use std::time::{Duration, Instant};

use clap::Parser;
use crossterm::{
    event::{self, Event},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

use app::{App, FocusFilter, StatusFilter};

#[derive(Clone)]
struct ViewFilters {
    status_filter: StatusFilter,
    focus_filter: FocusFilter,
    search: String,
}

#[derive(Parser)]
#[command(
    name = "claudectl",
    version,
    about = "Monitor and manage Claude Code CLI agents"
)]
struct Cli {
    /// Refresh interval in milliseconds
    #[arg(short, long, default_value_t = 2000)]
    interval: u64,

    /// Print session list to stdout and exit (no TUI)
    #[arg(short, long)]
    list: bool,

    /// Enable desktop notifications on NeedsInput transitions
    #[arg(long)]
    notify: bool,

    /// Print JSON array of sessions and exit
    #[arg(long)]
    json: bool,

    /// Stream status changes to stdout (no TUI). Only prints when status changes.
    #[arg(short, long)]
    watch: bool,

    /// Output format for watch mode. Placeholders: {pid}, {project}, {status}, {cost}, {context}
    #[arg(
        long,
        default_value = "{pid} {project}: {status} (${cost}, ctx {context}%)"
    )]
    format: String,

    /// Filter sessions by status for TUI and non-TUI views
    #[arg(long)]
    filter_status: Option<String>,

    /// Focus on a high-signal subset (`attention`, `over-budget`, `high-context`, `unknown-telemetry`, `conflict`)
    #[arg(long)]
    focus: Option<String>,

    /// Search project/model/session text for TUI and non-TUI views
    #[arg(long)]
    search: Option<String>,

    /// Enable debug mode: show timing metrics in the footer
    #[arg(long)]
    debug: bool,

    /// Show summary of session activity and exit
    #[arg(long)]
    summary: bool,

    /// Time window for summary (e.g., "8h", "24h", "30m"). Default: 24h.
    #[arg(long, default_value = "24h")]
    since: String,

    /// Webhook URL to POST JSON on status changes
    #[arg(long)]
    webhook: Option<String>,

    /// Only fire webhook on these status transitions (comma-separated, e.g. "NeedsInput,Finished")
    #[arg(long)]
    webhook_on: Option<String>,

    /// Launch a new Claude Code session in the given directory
    #[arg(long = "new")]
    new_session: bool,

    /// Working directory for the new session (used with --new)
    #[arg(long, default_value = ".")]
    cwd: String,

    /// Prompt to send to the new session (used with --new)
    #[arg(long)]
    prompt: Option<String>,

    /// Resume a session by ID (used with --new)
    #[arg(long)]
    resume: Option<String>,

    /// Per-session budget in USD. Alert at 80%, optionally kill at 100%.
    #[arg(long)]
    budget: Option<f64>,

    /// Auto-kill sessions that exceed the budget (requires --budget)
    #[arg(long)]
    kill_on_budget: bool,

    /// Show resolved configuration and exit
    #[arg(long)]
    config: bool,

    /// Color theme: dark, light, or none (respects NO_COLOR env var)
    #[arg(long)]
    theme: Option<String>,

    /// Write diagnostic logs to a file (for debugging/bug reports)
    #[arg(long)]
    log: Option<String>,

    /// List configured event hooks and exit
    #[arg(long)]
    hooks: bool,

    /// Show history of completed sessions and exit
    #[arg(long)]
    history: bool,

    /// Show aggregated session statistics and exit
    #[arg(long)]
    stats: bool,

    /// Run tasks from a JSON file (e.g., claudectl --run tasks.json)
    #[arg(long)]
    run: Option<String>,

    /// Run independent tasks in parallel (used with --run)
    #[arg(long)]
    parallel: bool,

    /// Clean up old session data (JSONL transcripts, session JSON files)
    #[arg(long)]
    clean: bool,

    /// Only clean sessions older than this duration (e.g., "7d", "24h"). Used with --clean.
    #[arg(long)]
    older_than: Option<String>,

    /// Only clean sessions that have finished. Used with --clean.
    #[arg(long)]
    finished: bool,

    /// Show what would be removed without deleting. Used with --clean.
    #[arg(long)]
    dry_run: bool,

    /// Run with deterministic fake sessions for screenshots and recordings
    #[arg(long)]
    demo: bool,

    /// Record the TUI session as an asciicast v2 file (e.g., --record demo.cast)
    #[arg(long)]
    record: Option<String>,
}

fn main() -> io::Result<()> {
    let cli = Cli::parse();

    // Initialize diagnostic logger if --log is set
    if let Some(ref log_path) = cli.log {
        if let Err(e) = logger::init(log_path) {
            eprintln!("Warning: could not open log file {log_path}: {e}");
        }
    }

    // Load config from files, then let CLI flags override
    let mut cfg = config::Config::load();

    // CLI flags override config file values (only override if explicitly set)
    if cli.interval != 2000 {
        cfg.interval = cli.interval;
    }
    if cli.notify {
        cfg.notify = true;
    }
    if cli.debug {
        cfg.debug = true;
    }
    if cli.budget.is_some() {
        cfg.budget = cli.budget;
    }
    if cli.kill_on_budget {
        cfg.kill_on_budget = true;
    }
    if cli.webhook.is_some() {
        cfg.webhook = cli.webhook.clone();
    }
    if cli.webhook_on.is_some() {
        cfg.webhook_on = cli.webhook_on.as_deref().map(|s| {
            s.split(',')
                .map(|t| t.trim().to_string())
                .collect::<Vec<_>>()
        });
    }

    models::set_overrides(cfg.model_overrides.clone());
    let filters = ViewFilters {
        status_filter: parse_status_filter(cli.filter_status.as_deref())?,
        focus_filter: parse_focus_filter(cli.focus.as_deref())?,
        search: cli.search.clone().unwrap_or_default(),
    };

    // Load event hooks from config
    let hook_registry = config::load_hooks();

    if cli.config {
        cfg.print_resolved();
        return Ok(());
    }

    if cli.hooks {
        hook_registry.print_list();
        return Ok(());
    }

    if let Some(ref run_file) = cli.run {
        let task_file = orchestrator::load_tasks(run_file)?;
        return orchestrator::run_tasks(task_file, cli.parallel);
    }

    if cli.clean {
        return run_clean(cli.older_than.as_deref(), cli.finished, cli.dry_run);
    }

    if cli.history {
        history::print_history(&cli.since);
        return Ok(());
    }

    if cli.stats {
        history::print_stats(&cli.since);
        return Ok(());
    }

    if cli.new_session {
        return launch_session(&cli.cwd, cli.prompt.as_deref(), cli.resume.as_deref());
    }

    if cli.summary {
        return print_summary(&cli.since);
    }

    if cli.json && !cli.watch {
        return print_json(cli.demo, &filters);
    }

    if cli.list {
        return print_list(cli.demo, &filters);
    }

    if cli.watch {
        return run_watch(
            Duration::from_millis(cfg.interval),
            cli.json,
            &cli.format,
            &filters,
        );
    }

    let tick_rate = Duration::from_millis(cfg.interval);
    let theme_mode = theme::ThemeMode::detect(cli.theme.as_deref());
    let app_theme = theme::Theme::from_mode(theme_mode);

    if let Some(ref record_path) = cli.record {
        // Recording mode: use TeeWriter to capture exact ANSI output
        let term_size = crossterm::terminal::size().unwrap_or((120, 40));
        let mut rec = recorder::Recorder::new(record_path, term_size.0, term_size.1)?;
        let rec_ptr: *mut recorder::Recorder = &mut rec;

        enable_raw_mode()?;
        // SAFETY: rec outlives tee_writer and terminal (both dropped before rec)
        let tee_writer = unsafe { recorder::TeeWriter::new(rec_ptr) };
        execute!(io::stdout(), EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(tee_writer);
        let mut terminal = Terminal::new(backend)?;

        let result = run_tui(
            &mut terminal,
            tick_rate,
            &cfg,
            app_theme,
            hook_registry,
            cli.demo,
            &filters,
        );

        disable_raw_mode()?;
        execute!(io::stdout(), LeaveAlternateScreen)?;
        terminal.show_cursor()?;

        match rec.finish() {
            Ok(()) => {
                eprintln!("Saved to {record_path}");
            }
            Err(e) => {
                // For GIF conversion failures, the error message contains instructions
                eprintln!("{e}");
            }
        }

        result
    } else {
        // Normal mode: plain stdout
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let result = run_tui(
            &mut terminal,
            tick_rate,
            &cfg,
            app_theme,
            hook_registry,
            cli.demo,
            &filters,
        );

        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
        terminal.show_cursor()?;

        result
    }
}

fn launch_session(cwd: &str, prompt: Option<&str>, resume: Option<&str>) -> io::Result<()> {
    let cwd_path = std::path::Path::new(cwd)
        .canonicalize()
        .unwrap_or_else(|_| std::path::PathBuf::from(cwd));

    match terminals::launch_session(cwd_path.to_string_lossy().as_ref(), prompt, resume) {
        Ok(target) => {
            println!(
                "Launched Claude session in {} at {}",
                target,
                cwd_path.display()
            );
            Ok(())
        }
        Err(e) => Err(io::Error::other(e)),
    }
}

fn parse_duration_str(s: &str) -> Duration {
    let s = s.trim();
    if let Some(hours) = s.strip_suffix('h') {
        if let Ok(h) = hours.parse::<u64>() {
            return Duration::from_secs(h * 3600);
        }
    }
    if let Some(mins) = s.strip_suffix('m') {
        if let Ok(m) = mins.parse::<u64>() {
            return Duration::from_secs(m * 60);
        }
    }
    if let Some(days) = s.strip_suffix('d') {
        if let Ok(d) = days.parse::<u64>() {
            return Duration::from_secs(d * 86400);
        }
    }
    Duration::from_secs(24 * 3600) // default 24h
}

fn parse_status_filter(value: Option<&str>) -> io::Result<StatusFilter> {
    match value {
        Some(raw) => StatusFilter::parse(raw).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "Invalid --filter-status value: {raw}. Expected one of: all, needs-input, processing, waiting, unknown, idle, finished"
                ),
            )
        }),
        None => Ok(StatusFilter::All),
    }
}

fn parse_focus_filter(value: Option<&str>) -> io::Result<FocusFilter> {
    match value {
        Some(raw) => FocusFilter::parse(raw).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "Invalid --focus value: {raw}. Expected one of: all, attention, over-budget, high-context, unknown-telemetry, conflict"
                ),
            )
        }),
        None => Ok(FocusFilter::All),
    }
}

fn apply_filters(app: &mut App, filters: &ViewFilters) {
    app.status_filter = filters.status_filter;
    app.focus_filter = filters.focus_filter;
    app.search_query = filters.search.trim().to_string();
    app.search_buffer.clear();
    app.search_mode = false;
    let len = app.visible_session_count();
    if len == 0 {
        app.table_state.select(None);
    } else if app.table_state.selected().is_none() {
        app.table_state.select(Some(0));
    } else if let Some(sel) = app.table_state.selected() {
        if sel >= len {
            app.table_state.select(Some(len - 1));
        }
    }
}

fn run_clean(older_than: Option<&str>, finished_only: bool, dry_run: bool) -> io::Result<()> {
    let min_age = older_than.map(parse_duration_str);
    let now = std::time::SystemTime::now();

    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"));

    // Collect active PIDs to avoid deleting live sessions
    let active_pids: std::collections::HashSet<u32> = {
        let app = App::new();
        app.sessions.iter().map(|s| s.pid).collect()
    };

    let mut removed_sessions = 0u64;
    let mut removed_jsonl = 0u64;
    let mut freed_bytes = 0u64;

    // Phase 1: Clean session JSON files in ~/.claude/sessions/
    let sessions_dir = home.join(".claude/sessions");
    if let Ok(entries) = std::fs::read_dir(&sessions_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            let pid: u32 = match stem.parse() {
                Ok(p) => p,
                Err(_) => continue,
            };

            // Never delete active sessions
            if active_pids.contains(&pid) {
                continue;
            }

            // Check age if --older-than is set
            if let Some(min_age) = min_age {
                let modified = entry.metadata().ok().and_then(|m| m.modified().ok());
                if let Some(modified) = modified {
                    let age = now.duration_since(modified).unwrap_or_default();
                    if age < min_age {
                        continue;
                    }
                }
            }

            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            if dry_run {
                println!("  would remove: {} ({} bytes)", path.display(), size);
            } else {
                let _ = std::fs::remove_file(&path);
            }
            removed_sessions += 1;
            freed_bytes += size;
        }
    }

    // Phase 2: Clean JSONL transcript files in ~/.claude/projects/*/
    let projects_dir = home.join(".claude/projects");
    if let Ok(project_entries) = std::fs::read_dir(&projects_dir) {
        for project_entry in project_entries.flatten() {
            let project_path = project_entry.path();
            if !project_path.is_dir() {
                continue;
            }
            let Ok(files) = std::fs::read_dir(&project_path) else {
                continue;
            };
            for file_entry in files.flatten() {
                let file_path = file_entry.path();
                if file_path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                    continue;
                }

                let metadata = match file_entry.metadata() {
                    Ok(m) => m,
                    Err(_) => continue,
                };

                // Check age if --older-than is set
                if let Some(min_age) = min_age {
                    let modified = metadata.modified().ok();
                    if let Some(modified) = modified {
                        let age = now.duration_since(modified).unwrap_or_default();
                        if age < min_age {
                            continue;
                        }
                    }
                }

                // If --finished only, skip JSONL files whose corresponding session is still active
                if finished_only {
                    // Check if any active session is using this JSONL
                    let app = App::new();
                    let is_active = app.sessions.iter().any(|s| {
                        s.jsonl_path
                            .as_ref()
                            .map(|p| p == &file_path)
                            .unwrap_or(false)
                    });
                    if is_active {
                        continue;
                    }
                }

                let size = metadata.len();
                if dry_run {
                    println!("  would remove: {} ({} bytes)", file_path.display(), size);
                } else {
                    let _ = std::fs::remove_file(&file_path);
                }
                removed_jsonl += 1;
                freed_bytes += size;
            }
        }
    }

    let freed_str = if freed_bytes >= 1_073_741_824 {
        format!("{:.1} GB", freed_bytes as f64 / 1_073_741_824.0)
    } else if freed_bytes >= 1_048_576 {
        format!("{:.1} MB", freed_bytes as f64 / 1_048_576.0)
    } else if freed_bytes >= 1024 {
        format!("{:.1} KB", freed_bytes as f64 / 1024.0)
    } else {
        format!("{freed_bytes} bytes")
    };

    if dry_run {
        println!();
        println!(
            "Dry run: would remove {} sessions + {} transcripts, freeing {}",
            removed_sessions, removed_jsonl, freed_str
        );
    } else if removed_sessions + removed_jsonl == 0 {
        println!("Nothing to clean up.");
    } else {
        println!(
            "Removed {} sessions + {} transcripts, freed {}",
            removed_sessions, removed_jsonl, freed_str
        );
    }

    Ok(())
}

fn print_summary(since: &str) -> io::Result<()> {
    let since_duration = parse_duration_str(since);
    let app = App::new();

    if app.sessions.is_empty() {
        println!("No active Claude sessions.");
        return Ok(());
    }

    for s in &app.sessions {
        let status_color = match s.status {
            session::SessionStatus::Processing => "\x1b[32m",
            session::SessionStatus::NeedsInput => "\x1b[35m",
            session::SessionStatus::WaitingInput => "\x1b[33m",
            session::SessionStatus::Unknown => "\x1b[34m",
            session::SessionStatus::Idle => "\x1b[90m",
            session::SessionStatus::Finished => "\x1b[31m",
        };
        let reset = "\x1b[0m";
        let status_text = if s.status == session::SessionStatus::Unknown {
            format!("Unknown: {}", s.telemetry_label())
        } else {
            s.status.to_string()
        };

        println!(
            "=== {} ({}, {}, {status_color}{}{reset}) ===",
            s.display_name(),
            s.format_elapsed(),
            s.format_cost(),
            status_text,
        );

        // Git stats from session's cwd
        let since_secs = since_duration.as_secs();
        let git_since = format!("{since_secs} seconds ago");

        let git_log = std::process::Command::new("git")
            .args(["log", "--oneline", &format!("--since={git_since}")])
            .current_dir(&s.cwd)
            .output();

        if let Ok(output) = git_log {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let commits: Vec<&str> = stdout.lines().collect();
            if !commits.is_empty() {
                println!("  Commits: {}", commits.len());
                for c in commits.iter().take(5) {
                    println!("    {c}");
                }
                if commits.len() > 5 {
                    println!("    ... and {} more", commits.len() - 5);
                }
            }
        }

        let git_diff = std::process::Command::new("git")
            .args(["diff", "--stat", "HEAD"])
            .current_dir(&s.cwd)
            .output();

        if let Ok(output) = git_diff {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let lines: Vec<&str> = stdout.lines().collect();
            if !lines.is_empty() {
                let file_count = lines.len().saturating_sub(1); // last line is summary
                if file_count > 0 {
                    println!("  Files changed: {file_count}");
                }
            }
        }

        // Token summary
        let total_tokens = s.total_input_tokens + s.total_output_tokens;
        if total_tokens > 0 {
            println!(
                "  Tokens: {} in / {} out",
                format_count(s.total_input_tokens),
                format_count(s.total_output_tokens)
            );
        }

        // Model and context
        if !s.model.is_empty() {
            let context_text = if s.has_usage_metrics() {
                format!("{}%", s.context_percent() as u32)
            } else {
                "n/a".to_string()
            };
            let estimate_note = if s.cost_estimate_unverified {
                " [fallback estimate]"
            } else if s.model_profile_source == "override" {
                " [config override]"
            } else {
                ""
            };
            println!(
                "  Model: {}{} (context: {})",
                s.model, estimate_note, context_text
            );
        }
        if s.status == session::SessionStatus::Unknown || !s.has_usage_metrics() {
            println!("  Telemetry: {}", s.telemetry_label());
        }

        if s.subagent_count > 0 {
            println!("  Subagents: {}", s.subagent_count);
        }

        println!();
    }

    let total_cost: f64 = app.sessions.iter().map(|s| s.cost_usd).sum();
    println!("Total cost: ${total_cost:.2}");

    Ok(())
}

fn format_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

fn make_app(demo: bool, filters: &ViewFilters) -> App {
    let mut app = if demo {
        let mut app = App::new();
        app.demo_mode = true;
        app.sessions = demo::generate_sessions(10);
        app
    } else {
        App::new()
    };
    apply_filters(&mut app, filters);
    app
}

fn print_json(demo: bool, filters: &ViewFilters) -> io::Result<()> {
    let app = make_app(demo, filters);
    let values: Vec<serde_json::Value> = app
        .visible_sessions()
        .iter()
        .map(|s| s.to_json_value())
        .collect();
    let json = serde_json::to_string_pretty(&values).unwrap_or_else(|_| "[]".to_string());
    println!("{json}");
    Ok(())
}

fn print_list(demo: bool, filters: &ViewFilters) -> io::Result<()> {
    let app = make_app(demo, filters);
    let visible_sessions = app.visible_sessions();

    if visible_sessions.is_empty() {
        if app.has_active_filters() {
            println!("No sessions match the current filters.");
        } else {
            println!("No active Claude sessions.");
        }
        if app.has_active_filters() {
            println!("  ({})", app.filter_summary());
        }
        return Ok(());
    }

    println!(
        "{:<7} {:<16} {:<12} {:<8} {:<8} {:<9} {:<10} {:<6} {:<6} TOKENS",
        "PID", "PROJECT", "STATUS", "CTX%", "COST", "$/HR", "ELAPSED", "CPU%", "MEM"
    );
    println!("{}", "-".repeat(105));

    for s in visible_sessions {
        let status_text = if s.status == session::SessionStatus::Unknown {
            s.telemetry_status.short_label().to_string()
        } else {
            s.status.to_string()
        };
        println!(
            "{:<7} {:<16} {:<12} {:<8} {:<8} {:<9} {:<10} {:<6.1} {:<6} {}",
            s.pid,
            s.display_name(),
            status_text,
            s.format_context(),
            s.format_cost(),
            s.format_burn_rate(),
            s.format_elapsed(),
            s.cpu_percent,
            s.format_mem(),
            s.format_tokens(),
        );
    }

    let total_cost: f64 = app.visible_sessions().iter().map(|s| s.cost_usd).sum();
    println!("{}", "-".repeat(105));
    println!("Total cost: ${total_cost:.2}");
    if app.has_active_filters() {
        println!("{}", app.filter_summary());
    }

    Ok(())
}

fn run_watch(
    tick_rate: Duration,
    json_mode: bool,
    format_str: &str,
    filters: &ViewFilters,
) -> io::Result<()> {
    use crate::session::SessionStatus;
    use std::collections::HashMap;

    let mut app = App::new();
    apply_filters(&mut app, filters);
    let mut prev_statuses: HashMap<u32, SessionStatus> =
        app.sessions.iter().map(|s| (s.pid, s.status)).collect();

    // Print initial state for all sessions
    for s in app.visible_sessions() {
        if json_mode {
            let obj = serde_json::json!({
                "event": "initial",
                "pid": s.pid,
                "project": s.display_name(),
                "status": s.status.to_string(),
                "telemetry": s.telemetry_label(),
                "cost_usd": if s.has_usage_metrics() { serde_json::json!((s.cost_usd * 100.0).round() / 100.0) } else { serde_json::Value::Null },
                "context_pct": if s.has_usage_metrics() { serde_json::json!((s.context_percent() * 100.0).round() / 100.0) } else { serde_json::Value::Null },
                "elapsed_secs": s.elapsed.as_secs(),
            });
            println!("{}", serde_json::to_string(&obj).unwrap_or_default());
        } else {
            println!("{}", format_session(format_str, s));
        }
    }

    loop {
        std::thread::sleep(tick_rate);
        app.tick();
        let visible_pids: std::collections::HashSet<u32> =
            app.visible_sessions().iter().map(|s| s.pid).collect();

        for s in &app.sessions {
            let prev = prev_statuses.get(&s.pid).copied();
            let changed = prev.is_none_or(|p| p != s.status);

            if !changed || !visible_pids.contains(&s.pid) {
                continue;
            }

            if json_mode {
                let obj = serde_json::json!({
                    "event": "status_change",
                    "pid": s.pid,
                    "project": s.display_name(),
                    "old_status": prev.map(|p| p.to_string()).unwrap_or_default(),
                    "new_status": s.status.to_string(),
                    "telemetry": s.telemetry_label(),
                    "cost_usd": if s.has_usage_metrics() { serde_json::json!((s.cost_usd * 100.0).round() / 100.0) } else { serde_json::Value::Null },
                    "context_pct": if s.has_usage_metrics() { serde_json::json!((s.context_percent() * 100.0).round() / 100.0) } else { serde_json::Value::Null },
                    "elapsed_secs": s.elapsed.as_secs(),
                });
                println!("{}", serde_json::to_string(&obj).unwrap_or_default());
            } else {
                println!("{}", format_session(format_str, s));
            }
        }

        prev_statuses = app.sessions.iter().map(|s| (s.pid, s.status)).collect();
    }
}

fn format_session(fmt: &str, s: &session::ClaudeSession) -> String {
    let cost = if s.has_usage_metrics() {
        format!("{:.2}", s.cost_usd)
    } else {
        "n/a".to_string()
    };
    let context = if s.has_usage_metrics() {
        format!("{}", s.context_percent() as u32)
    } else {
        "n/a".to_string()
    };
    fmt.replace("{pid}", &s.pid.to_string())
        .replace("{project}", s.display_name())
        .replace("{status}", &s.status.to_string())
        .replace("{cost}", &cost)
        .replace("{context}", &context)
}

fn run_tui<W: io::Write>(
    terminal: &mut Terminal<CrosstermBackend<W>>,
    tick_rate: Duration,
    cfg: &config::Config,
    app_theme: theme::Theme,
    hook_registry: hooks::HookRegistry,
    demo_mode: bool,
    filters: &ViewFilters,
) -> io::Result<()> {
    let mut app = App::new();
    app.notify = cfg.notify;
    app.debug = cfg.debug;
    app.webhook_url = cfg.webhook.clone();
    app.webhook_filter = cfg.webhook_on.clone();
    app.budget_usd = cfg.budget;
    app.kill_on_budget = cfg.kill_on_budget;
    app.grouped_view = cfg.grouped;
    app.theme = app_theme;
    app.hooks = hook_registry;
    app.daily_limit = cfg.daily_limit;
    app.weekly_limit = cfg.weekly_limit;
    app.context_warn_threshold = cfg.context_warn_threshold;
    app.demo_mode = demo_mode;
    apply_filters(&mut app, filters);

    if demo_mode {
        app.daily_limit = Some(50.0);
        app.budget_usd = Some(10.0);
    }

    let mut last_tick = Instant::now();
    let mut sess_recs: std::collections::HashMap<u32, session_recorder::SessionRecorder> =
        std::collections::HashMap::new();
    let term_size = crossterm::terminal::size().unwrap_or((120, 40));

    loop {
        terminal.draw(|frame| {
            ui::table::render(frame, frame.area(), &app);
        })?;

        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_secs(0));

        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if !app.handle_key(key) {
                    // Finish all session recordings on quit
                    for (_, rec) in sess_recs.iter_mut() {
                        let _ = rec.finish();
                    }
                    return Ok(());
                }
            }
        }

        if last_tick.elapsed() >= tick_rate {
            app.tick();
            last_tick = Instant::now();

            // Start recorders for newly added recordings
            for (pid, path) in &app.session_recordings {
                if sess_recs.contains_key(pid) {
                    continue;
                }
                if let Some(session) = app.sessions.iter().find(|s| s.pid == *pid) {
                    if let Some(ref jsonl) = session.jsonl_path {
                        let name = session.display_name();
                        match session_recorder::SessionRecorder::new(
                            jsonl,
                            path,
                            name,
                            term_size.0,
                            term_size.1,
                        ) {
                            Ok(r) => {
                                sess_recs.insert(*pid, r);
                            }
                            Err(e) => {
                                app.status_msg = format!("Record error: {e}");
                            }
                        }
                    }
                }
            }

            // Poll all active recorders
            for (_, rec) in sess_recs.iter_mut() {
                let _ = rec.poll();
            }

            // Finish recorders that were removed from app.session_recordings
            let stopped: Vec<u32> = sess_recs
                .keys()
                .filter(|pid| !app.session_recordings.contains_key(pid))
                .copied()
                .collect();
            for pid in stopped {
                if let Some(mut rec) = sess_recs.remove(&pid) {
                    match rec.finish() {
                        Ok(()) => {}
                        Err(e) => {
                            app.status_msg = format!("{e}");
                        }
                    }
                }
            }
        }
    }
}
