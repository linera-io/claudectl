#![allow(dead_code)]

use crate::session::ClaudeSession;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Info,
    Warning,
    Critical,
}

#[derive(Debug, Clone)]
pub struct HealthCheck {
    pub icon: &'static str,
    pub name: &'static str,
    pub severity: Severity,
    pub message: String,
}

/// Run all health checks against a session. Returns warnings sorted by severity.
pub fn check_session(session: &ClaudeSession) -> Vec<HealthCheck> {
    let mut checks = Vec::new();

    if let Some(c) = check_cache_health(session) {
        checks.push(c);
    }
    if let Some(c) = check_cost_spike(session) {
        checks.push(c);
    }
    if let Some(c) = check_loop_detection(session) {
        checks.push(c);
    }
    if let Some(c) = check_stalled(session) {
        checks.push(c);
    }
    if let Some(c) = check_context_saturation(session) {
        checks.push(c);
    }

    // Sort: Critical first, then Warning, then Info
    checks.sort_by_key(|c| match c.severity {
        Severity::Critical => 0,
        Severity::Warning => 1,
        Severity::Info => 2,
    });

    checks
}

/// Return the most severe health icon for display in the table, or empty string if healthy.
pub fn status_icon(session: &ClaudeSession) -> &'static str {
    let checks = check_session(session);
    match checks.first() {
        Some(c) if c.severity == Severity::Critical => c.icon,
        Some(c) if c.severity == Severity::Warning => c.icon,
        _ => "",
    }
}

/// Format a compact health summary for the status bar.
pub fn format_health_summary(sessions: &[ClaudeSession]) -> Option<String> {
    let mut warnings = 0;
    let mut criticals = 0;
    let mut worst_msg = String::new();

    for session in sessions {
        for check in check_session(session) {
            match check.severity {
                Severity::Critical => {
                    criticals += 1;
                    if worst_msg.is_empty() {
                        worst_msg =
                            format!("{} {}: {}", check.icon, session.display_name(), check.name);
                    }
                }
                Severity::Warning => warnings += 1,
                Severity::Info => {}
            }
        }
    }

    if criticals == 0 && warnings == 0 {
        return None;
    }

    let count = criticals + warnings;
    Some(format!(
        "{} health issue{} | {}",
        count,
        if count == 1 { "" } else { "s" },
        worst_msg,
    ))
}

// ────────────────────────────────────────────────────────────────────────────
// Individual health checks
// ────────────────────────────────────────────────────────────────────────────

/// Detect low cache hit ratio (e.g., cache TTL bug causing 12x cost).
fn check_cache_health(session: &ClaudeSession) -> Option<HealthCheck> {
    let total_input = session.total_input_tokens;
    let cache_read = session.cache_read_tokens;

    // Need enough data to be meaningful (at least 10K tokens processed)
    if total_input < 10_000 {
        return None;
    }

    let hit_ratio = cache_read as f64 / total_input as f64;

    if hit_ratio < 0.10 {
        Some(HealthCheck {
            icon: "🔥",
            name: "low cache",
            severity: Severity::Critical,
            message: format!(
                "Cache hit ratio is {:.0}% — expected >50% for long sessions. \
                 Possible cache TTL issue (check telemetry settings).",
                hit_ratio * 100.0
            ),
        })
    } else if hit_ratio < 0.30 {
        Some(HealthCheck {
            icon: "⚠",
            name: "low cache",
            severity: Severity::Warning,
            message: format!(
                "Cache hit ratio is {:.0}% — below typical range. \
                 May indicate cache TTL or model configuration issue.",
                hit_ratio * 100.0
            ),
        })
    } else {
        None
    }
}

/// Detect burn rate spikes — paying more for less output.
fn check_cost_spike(session: &ClaudeSession) -> Option<HealthCheck> {
    if session.cost_usd < 1.0 || session.burn_rate_per_hr <= 0.0 {
        return None;
    }

    // Compare current burn rate to average burn rate over session lifetime
    let elapsed_hrs = session.elapsed.as_secs_f64() / 3600.0;
    if elapsed_hrs < 0.01 {
        return None;
    }
    let avg_rate = session.cost_usd / elapsed_hrs;

    if avg_rate <= 0.0 {
        return None;
    }

    let spike_factor = session.burn_rate_per_hr / avg_rate;

    if spike_factor > 5.0 {
        Some(HealthCheck {
            icon: "💸",
            name: "cost spike",
            severity: Severity::Critical,
            message: format!(
                "Burn rate ${:.1}/hr is {:.0}x the session average ${:.1}/hr.",
                session.burn_rate_per_hr, spike_factor, avg_rate,
            ),
        })
    } else if spike_factor > 2.5 {
        Some(HealthCheck {
            icon: "💰",
            name: "cost spike",
            severity: Severity::Warning,
            message: format!(
                "Burn rate ${:.1}/hr is {:.1}x the session average.",
                session.burn_rate_per_hr, spike_factor,
            ),
        })
    } else {
        None
    }
}

/// Detect tool error loops — same tool failing repeatedly.
fn check_loop_detection(session: &ClaudeSession) -> Option<HealthCheck> {
    if !session.last_tool_error {
        return None;
    }

    // Check if any tool has been called many times (could indicate looping)
    let max_calls = session
        .tool_usage
        .values()
        .map(|t| t.calls)
        .max()
        .unwrap_or(0);

    if max_calls >= 10 && session.last_tool_error {
        let tool_name = session
            .tool_usage
            .iter()
            .max_by_key(|(_, t)| t.calls)
            .map(|(name, _)| name.as_str())
            .unwrap_or("?");

        Some(HealthCheck {
            icon: "🔄",
            name: "looping",
            severity: Severity::Warning,
            message: format!(
                "{tool_name} called {max_calls} times with recent errors — may be stuck in a retry loop.",
            ),
        })
    } else {
        None
    }
}

/// Detect stalled sessions — high cost but no file output.
fn check_stalled(session: &ClaudeSession) -> Option<HealthCheck> {
    if session.cost_usd < 5.0 {
        return None;
    }

    let files_edited: u32 = session.files_modified.values().sum();
    let elapsed_mins = session.elapsed.as_secs() / 60;

    // Spent >$5 and running >10 min with no file edits
    if files_edited == 0 && elapsed_mins > 10 {
        Some(HealthCheck {
            icon: "🐌",
            name: "stalled",
            severity: Severity::Warning,
            message: format!(
                "Spent ${:.1} over {} min with no file edits.",
                session.cost_usd, elapsed_mins,
            ),
        })
    } else {
        None
    }
}

/// Detect context window saturation.
fn check_context_saturation(session: &ClaudeSession) -> Option<HealthCheck> {
    if session.context_max == 0 {
        return None;
    }

    let pct = (session.context_tokens as f64 / session.context_max as f64) * 100.0;

    if pct > 90.0 {
        Some(HealthCheck {
            icon: "🧠",
            name: "context full",
            severity: Severity::Critical,
            message: format!(
                "Context at {:.0}% — session may degrade or auto-compact. \
                 Consider spawning a fresh session.",
                pct,
            ),
        })
    } else if pct > 80.0 {
        Some(HealthCheck {
            icon: "🧠",
            name: "context high",
            severity: Severity::Warning,
            message: format!("Context at {:.0}% — approaching limit.", pct),
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{RawSession, SessionStatus, TelemetryStatus};

    fn make_session() -> ClaudeSession {
        let raw = RawSession {
            pid: 1,
            session_id: "test".into(),
            cwd: "/tmp/test".into(),
            started_at: 0,
        };
        let mut s = ClaudeSession::from_raw(raw);
        s.status = SessionStatus::Processing;
        s.telemetry_status = TelemetryStatus::Available;
        s.model = "opus".into();
        s
    }

    #[test]
    fn healthy_session_no_warnings() {
        let s = make_session();
        assert!(check_session(&s).is_empty());
    }

    #[test]
    fn low_cache_critical() {
        let mut s = make_session();
        s.total_input_tokens = 100_000;
        s.cache_read_tokens = 5_000; // 5% hit ratio
        let checks = check_session(&s);
        assert!(
            checks
                .iter()
                .any(|c| c.name == "low cache" && c.severity == Severity::Critical)
        );
    }

    #[test]
    fn low_cache_warning() {
        let mut s = make_session();
        s.total_input_tokens = 100_000;
        s.cache_read_tokens = 20_000; // 20% hit ratio
        let checks = check_session(&s);
        assert!(
            checks
                .iter()
                .any(|c| c.name == "low cache" && c.severity == Severity::Warning)
        );
    }

    #[test]
    fn healthy_cache_no_warning() {
        let mut s = make_session();
        s.total_input_tokens = 100_000;
        s.cache_read_tokens = 60_000; // 60% hit ratio
        assert!(check_cache_health(&s).is_none());
    }

    #[test]
    fn context_saturation_critical() {
        let mut s = make_session();
        s.context_tokens = 190_000;
        s.context_max = 200_000;
        let checks = check_session(&s);
        assert!(
            checks
                .iter()
                .any(|c| c.name == "context full" && c.severity == Severity::Critical)
        );
    }

    #[test]
    fn context_saturation_warning() {
        let mut s = make_session();
        s.context_tokens = 170_000;
        s.context_max = 200_000;
        let checks = check_session(&s);
        assert!(
            checks
                .iter()
                .any(|c| c.name == "context high" && c.severity == Severity::Warning)
        );
    }

    #[test]
    fn stalled_detection() {
        let mut s = make_session();
        s.cost_usd = 10.0;
        s.elapsed = std::time::Duration::from_secs(15 * 60);
        // No files modified
        let checks = check_session(&s);
        assert!(checks.iter().any(|c| c.name == "stalled"));
    }

    #[test]
    fn status_icon_returns_worst() {
        let mut s = make_session();
        s.context_tokens = 190_000;
        s.context_max = 200_000;
        assert_eq!(status_icon(&s), "🧠");
    }

    #[test]
    fn status_icon_empty_when_healthy() {
        let s = make_session();
        assert_eq!(status_icon(&s), "");
    }

    #[test]
    fn sorted_by_severity() {
        let mut s = make_session();
        s.total_input_tokens = 100_000;
        s.cache_read_tokens = 5_000; // Critical cache
        s.context_tokens = 170_000;
        s.context_max = 200_000; // Warning context
        let checks = check_session(&s);
        assert!(checks.len() >= 2);
        assert_eq!(checks[0].severity, Severity::Critical);
    }
}
