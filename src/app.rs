use std::collections::{HashMap, HashSet};

use ratatui::widgets::TableState;

use crate::action;
use crate::discovery;
use crate::monitor;
use crate::process::ProcessMonitor;
use crate::session::{ClaudeSession, SessionStatus};

/// Number of sortable columns and their display names.
pub const SORT_COLUMNS: &[&str] = &["Status", "Context", "Cost", "$/hr", "Elapsed"];

pub struct App {
    pub sessions: Vec<ClaudeSession>,
    pub table_state: TableState,
    pub should_quit: bool,
    pub process_monitor: ProcessMonitor,
    pub status_msg: String,
    pub pending_kill: Option<u32>,
    pub input_mode: bool,
    pub input_buffer: String,
    pub input_target_pid: Option<u32>,
    // Feature #31: Desktop notifications
    pub notify: bool,
    pub prev_statuses: HashMap<u32, SessionStatus>,
    // Feature #26: Help overlay
    pub show_help: bool,
    // Feature #28: Sort by column
    pub sort_column: usize,
    // Feature #33: Auto-approve
    pub auto_approve: HashSet<u32>,
    pub pending_auto_approve: Option<u32>,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        let mut app = Self {
            sessions: Vec::new(),
            table_state: TableState::default(),
            should_quit: false,
            process_monitor: ProcessMonitor::new(),
            status_msg: String::new(),
            pending_kill: None,
            input_mode: false,
            input_buffer: String::new(),
            input_target_pid: None,
            notify: false,
            prev_statuses: HashMap::new(),
            show_help: false,
            sort_column: 0,
            auto_approve: HashSet::new(),
            pending_auto_approve: None,
        };
        app.refresh();
        // Select first row if sessions exist
        if !app.sessions.is_empty() {
            app.table_state.select(Some(0));
        }
        app
    }

    pub fn refresh(&mut self) {
        let mut sessions = discovery::scan_sessions();

        // Enrich with process data (also filters dead PIDs)
        self.process_monitor.refresh();
        self.process_monitor.enrich(&mut sessions);
        self.process_monitor.fetch_ps_data(&mut sessions);

        // Resolve JSONL paths AFTER ps data (needs command_args for --resume UUID)
        discovery::resolve_jsonl_paths(&mut sessions);

        // Feature #29: Scan for subagent task files
        discovery::scan_subagents(&mut sessions);

        // Carry forward previous costs for burn rate calculation
        let prev_costs: HashMap<u32, f64> = self
            .sessions
            .iter()
            .map(|s| (s.pid, s.cost_usd))
            .collect();

        // Read JSONL for tokens + status
        for session in &mut sessions {
            monitor::update_tokens(session);

            // Compute burn rate: cost delta / time delta
            if let Some(&prev) = prev_costs.get(&session.pid) {
                session.prev_cost_usd = prev;
                let delta = session.cost_usd - prev;
                if delta > 0.0 {
                    // tick_rate is ~2s, extrapolate to $/hr
                    session.burn_rate_per_hr = delta * 1800.0; // delta per 2s * 1800 = per hour
                }
            }
        }

        // Feature #28: Sort by selected column
        self.apply_sort(&mut sessions);

        // Feature #31: Check for NeedsInput transitions and fire notifications
        if self.notify {
            for session in &sessions {
                let prev = self.prev_statuses.get(&session.pid).copied();
                if session.status == SessionStatus::NeedsInput
                    && prev != Some(SessionStatus::NeedsInput)
                    && prev.is_some()
                {
                    fire_notification(&session.project_name);
                }
            }
        }

        // Update prev_statuses for next comparison
        self.prev_statuses = sessions
            .iter()
            .map(|s| (s.pid, s.status))
            .collect();

        self.sessions = sessions;

        // Fix selection bounds
        let len = self.sessions.len();
        if len == 0 {
            self.table_state.select(None);
        } else if let Some(sel) = self.table_state.selected() {
            if sel >= len {
                self.table_state.select(Some(len - 1));
            }
        }
    }

    /// Apply sort based on the current sort_column selection.
    fn apply_sort(&self, sessions: &mut [ClaudeSession]) {
        match self.sort_column {
            0 => {
                // Status (default): status priority, then elapsed desc
                sessions.sort_by(|a, b| {
                    a.status
                        .sort_key()
                        .cmp(&b.status.sort_key())
                        .then(b.elapsed.cmp(&a.elapsed))
                });
            }
            1 => {
                // Context%: descending
                sessions.sort_by(|a, b| {
                    b.context_percent()
                        .partial_cmp(&a.context_percent())
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            }
            2 => {
                // Cost: descending
                sessions.sort_by(|a, b| {
                    b.cost_usd
                        .partial_cmp(&a.cost_usd)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            }
            3 => {
                // $/hr: descending
                sessions.sort_by(|a, b| {
                    b.burn_rate_per_hr
                        .partial_cmp(&a.burn_rate_per_hr)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            }
            4 => {
                // Elapsed: descending
                sessions.sort_by(|a, b| b.elapsed.cmp(&a.elapsed));
            }
            _ => {}
        }
    }

    /// Cycle sort column.
    pub fn cycle_sort(&mut self) {
        self.sort_column = (self.sort_column + 1) % SORT_COLUMNS.len();
        self.status_msg = format!("Sort: {}", SORT_COLUMNS[self.sort_column]);
        // Re-sort immediately
        let mut sessions = std::mem::take(&mut self.sessions);
        self.apply_sort(&mut sessions);
        self.sessions = sessions;
    }

    pub fn tick(&mut self) {
        // Clear status message on tick
        self.status_msg.clear();

        // Re-read elapsed times
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        for session in &mut self.sessions {
            let elapsed_ms = now_ms.saturating_sub(session.started_at);
            session.elapsed = std::time::Duration::from_millis(elapsed_ms);
        }

        self.refresh();

        // Feature #33: Auto-approve sessions that are NeedsInput
        self.run_auto_approve();
    }

    /// Feature #33: For sessions in auto_approve set that are NeedsInput, send approve.
    fn run_auto_approve(&mut self) {
        let pids_to_approve: Vec<u32> = self
            .sessions
            .iter()
            .filter(|s| {
                s.status == SessionStatus::NeedsInput && self.auto_approve.contains(&s.pid)
            })
            .map(|s| s.pid)
            .collect();

        for pid in pids_to_approve {
            if let Some(session) = self.sessions.iter().find(|s| s.pid == pid) {
                match action::approve_session(session) {
                    Ok(()) => {
                        self.status_msg =
                            format!("Auto-approved {}", session.display_name());
                    }
                    Err(e) => {
                        self.status_msg = format!("Auto-approve error: {e}");
                    }
                }
            }
        }
    }

    /// Handle `a` key press -- first press sets pending, second confirms auto-approve toggle.
    pub fn handle_auto_approve(&mut self) {
        let Some(session) = self.selected_session() else {
            return;
        };
        let pid = session.pid;
        let name = session.display_name().to_string();

        if self.pending_auto_approve == Some(pid) {
            // Second press -- toggle
            if self.auto_approve.contains(&pid) {
                self.auto_approve.remove(&pid);
                self.status_msg = format!("Auto-approve OFF for {name}");
            } else {
                self.auto_approve.insert(pid);
                self.status_msg = format!("Auto-approve ON for {name}");
            }
            self.pending_auto_approve = None;
        } else {
            // First press -- ask for confirmation
            self.pending_auto_approve = Some(pid);
            let current = if self.auto_approve.contains(&pid) {
                "disable"
            } else {
                "enable"
            };
            self.status_msg =
                format!("Press a again to {current} auto-approve for {name}");
        }
    }

    /// Cancel any pending auto-approve confirmation.
    pub fn cancel_pending_auto_approve(&mut self) {
        if self.pending_auto_approve.is_some() {
            self.pending_auto_approve = None;
            // Don't overwrite status_msg if cancel_pending_kill already set it
        }
    }

    pub fn next(&mut self) {
        if self.sessions.is_empty() {
            return;
        }
        let i = match self.table_state.selected() {
            Some(i) => {
                if i >= self.sessions.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.table_state.select(Some(i));
    }

    pub fn previous(&mut self) {
        if self.sessions.is_empty() {
            return;
        }
        let i = match self.table_state.selected() {
            Some(i) => {
                if i == 0 {
                    self.sessions.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.table_state.select(Some(i));
    }

    pub fn selected_session(&self) -> Option<&ClaudeSession> {
        self.table_state
            .selected()
            .and_then(|i| self.sessions.get(i))
    }

    /// Handle `d` key press -- first press sets pending, second confirms kill.
    pub fn handle_kill(&mut self) {
        let Some(session) = self.selected_session() else {
            return;
        };
        let pid = session.pid;
        let name = session.display_name().to_string();

        if self.pending_kill == Some(pid) {
            // Second press -- kill it
            match kill_process(pid) {
                Ok(()) => {
                    self.status_msg = format!("Killed {name} (PID {pid})");
                    // Remove the session JSON
                    let session_file = dirs_home()
                        .join(".claude")
                        .join("sessions")
                        .join(format!("{pid}.json"));
                    let _ = std::fs::remove_file(session_file);
                    // Also remove from auto_approve set
                    self.auto_approve.remove(&pid);
                    self.refresh();
                }
                Err(e) => {
                    self.status_msg = format!("Kill failed: {e}");
                }
            }
            self.pending_kill = None;
        } else {
            // First press -- ask for confirmation
            self.pending_kill = Some(pid);
            self.status_msg = format!("Kill {name} (PID {pid})? Press d again to confirm");
        }
    }

    /// Cancel any pending kill on non-d key press.
    pub fn cancel_pending_kill(&mut self) {
        if self.pending_kill.is_some() {
            self.pending_kill = None;
            self.status_msg = "Kill cancelled".into();
        }
    }
}

/// Fire a macOS desktop notification via osascript.
fn fire_notification(project: &str) {
    let _ = std::process::Command::new("osascript")
        .args([
            "-e",
            &format!(
                "display notification \"{project} needs input\" with title \"claudectl\""
            ),
        ])
        .spawn();
}

fn dirs_home() -> std::path::PathBuf {
    std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
}

fn kill_process(pid: u32) -> Result<(), String> {
    // Send SIGTERM first
    let output = std::process::Command::new("kill")
        .arg(pid.to_string())
        .output()
        .map_err(|e| format!("Failed to run kill: {e}"))?;

    if output.status.success() {
        Ok(())
    } else {
        // Try SIGKILL as fallback
        let output = std::process::Command::new("kill")
            .args(["-9", &pid.to_string()])
            .output()
            .map_err(|e| format!("Failed to run kill -9: {e}"))?;

        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(stderr.trim().to_string())
        }
    }
}
