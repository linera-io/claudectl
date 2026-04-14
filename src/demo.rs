use std::collections::HashMap;
use std::time::Duration;

use crate::session::{ClaudeSession, RawSession, SessionStatus, TelemetryStatus};

/// Fake project definitions for demo mode.
const PROJECTS: &[(&str, &str, &str)] = &[
    ("acme-api", "/Users/dev/projects/acme-api", "opus-4.6"),
    ("acme-api", "/Users/dev/projects/acme-api", "opus-4.6"),
    (
        "web-frontend",
        "/Users/dev/projects/web-frontend",
        "sonnet-4.6",
    ),
    ("ml-pipeline", "/Users/dev/projects/ml-pipeline", "opus-4.6"),
    (
        "ml-pipeline",
        "/Users/dev/worktrees/ml-pipeline-feat",
        "sonnet-4.6",
    ),
    (
        "infra-terraform",
        "/Users/dev/projects/infra-terraform",
        "haiku",
    ),
    ("docs-site", "/Users/dev/projects/docs-site", "sonnet-4.6"),
    ("mobile-app", "/Users/dev/projects/mobile-app", "opus-4.6"),
];

/// Deterministic status progression per session (cycles through these).
const STATUS_SEQUENCES: &[&[SessionStatus]] = &[
    &[
        SessionStatus::Processing,
        SessionStatus::Processing,
        SessionStatus::Processing,
        SessionStatus::NeedsInput,
        SessionStatus::NeedsInput,
        SessionStatus::Processing,
        SessionStatus::Processing,
        SessionStatus::WaitingInput,
    ],
    &[
        SessionStatus::Processing,
        SessionStatus::NeedsInput,
        SessionStatus::NeedsInput,
        SessionStatus::NeedsInput,
        SessionStatus::Processing,
        SessionStatus::Processing,
        SessionStatus::WaitingInput,
        SessionStatus::Processing,
    ],
    &[
        SessionStatus::Processing,
        SessionStatus::Processing,
        SessionStatus::WaitingInput,
        SessionStatus::WaitingInput,
        SessionStatus::Processing,
        SessionStatus::Processing,
        SessionStatus::NeedsInput,
        SessionStatus::Processing,
    ],
    &[
        SessionStatus::Idle,
        SessionStatus::Idle,
        SessionStatus::Processing,
        SessionStatus::Processing,
        SessionStatus::Processing,
        SessionStatus::WaitingInput,
        SessionStatus::Idle,
        SessionStatus::Idle,
    ],
    &[
        SessionStatus::Processing,
        SessionStatus::Processing,
        SessionStatus::Processing,
        SessionStatus::Processing,
        SessionStatus::WaitingInput,
        SessionStatus::Processing,
        SessionStatus::Processing,
        SessionStatus::NeedsInput,
    ],
    &[
        SessionStatus::WaitingInput,
        SessionStatus::Idle,
        SessionStatus::Idle,
        SessionStatus::Idle,
        SessionStatus::Idle,
        SessionStatus::Idle,
        SessionStatus::Idle,
        SessionStatus::Idle,
    ],
    &[
        SessionStatus::Processing,
        SessionStatus::Processing,
        SessionStatus::WaitingInput,
        SessionStatus::Processing,
        SessionStatus::Processing,
        SessionStatus::Processing,
        SessionStatus::WaitingInput,
        SessionStatus::WaitingInput,
    ],
    &[
        SessionStatus::Processing,
        SessionStatus::Processing,
        SessionStatus::NeedsInput,
        SessionStatus::Processing,
        SessionStatus::Processing,
        SessionStatus::Processing,
        SessionStatus::WaitingInput,
        SessionStatus::Processing,
    ],
];

/// Generate deterministic fake sessions for demo mode.
pub fn generate_sessions(tick: u32) -> Vec<ClaudeSession> {
    let base_pid = 10000u32;

    PROJECTS
        .iter()
        .enumerate()
        .map(|(i, (name, cwd, model))| {
            let pid = base_pid + (i as u32 * 1111);
            let raw = RawSession {
                pid,
                session_id: format!("demo-{:04x}-{:04x}-{:04x}", i, i * 7, i * 13),
                cwd: cwd.to_string(),
                started_at: 0, // Will be overridden
            };
            let mut s = ClaudeSession::from_raw(raw);
            s.project_name = name.to_string();
            s.model = model.to_string();
            s.telemetry_status = TelemetryStatus::Available;
            s.usage_metrics_available = true;

            // Deterministic status from sequence
            let seq = STATUS_SEQUENCES[i % STATUS_SEQUENCES.len()];
            s.status = seq[(tick as usize) % seq.len()];

            // Deterministic metrics that grow over time
            let base_tokens = (i as u64 + 1) * 50_000 + (tick as u64) * 2_000;
            s.total_input_tokens = base_tokens;
            s.total_output_tokens = base_tokens / 5;
            s.cache_read_tokens = base_tokens / 3;
            s.cache_write_tokens = base_tokens / 10;

            // Context grows over time, different rates per session
            let ctx_rate = 0.3 + (i as f64 * 0.08);
            let ctx_pct = ((tick as f64 * ctx_rate) % 95.0) + 5.0;
            s.context_max = crate::monitor::model_context_max(model);
            s.context_tokens = (s.context_max as f64 * ctx_pct / 100.0) as u64;

            // Cost grows over time
            s.cost_usd = (i as f64 + 1.0) * 0.15 + (tick as f64) * 0.03 * (i as f64 + 1.0);
            s.burn_rate_per_hr = if matches!(s.status, SessionStatus::Processing) {
                2.0 + (i as f64 * 0.8)
            } else {
                0.0
            };

            // Elapsed time
            let base_elapsed = (i as u64 + 1) * 300 + tick as u64 * 2;
            s.elapsed = Duration::from_secs(base_elapsed);

            // CPU/MEM
            s.cpu_percent = match s.status {
                SessionStatus::Processing => 15.0 + (i as f32 * 3.0),
                SessionStatus::NeedsInput => 0.3,
                _ => 0.8,
            };
            s.mem_mb = 200.0 + (i as f64 * 50.0);

            // Subagents for some sessions
            if i == 0 || i == 3 {
                s.subagent_count = 2 + (tick as usize % 3);
            }

            // Activity sparkline history
            for t in 0..15 {
                let past_tick = if tick > 15 { tick - 15 + t } else { t };
                let past_status = seq[(past_tick as usize) % seq.len()];
                let level = match past_status {
                    SessionStatus::Processing => 7,
                    SessionStatus::NeedsInput => 4,
                    SessionStatus::WaitingInput => 2,
                    SessionStatus::Unknown => 2,
                    SessionStatus::Idle => 1,
                    SessionStatus::Finished => 0,
                };
                s.activity_history.push(level);
            }

            // Worktree IDs — sessions 0 and 1 share same worktree (conflict!)
            // Session 4 is a worktree of project 3's repo (no conflict)
            s.worktree_id = Some(cwd.to_string());

            // Tool usage for detail panel
            if tick > 3 {
                let mut tools = HashMap::new();
                tools.insert(
                    "Bash".to_string(),
                    crate::session::ToolStats {
                        calls: 12 + (i as u32 * 3),
                    },
                );
                tools.insert(
                    "Read".to_string(),
                    crate::session::ToolStats {
                        calls: 25 + (i as u32 * 5),
                    },
                );
                tools.insert(
                    "Edit".to_string(),
                    crate::session::ToolStats {
                        calls: 8 + (i as u32 * 2),
                    },
                );
                tools.insert(
                    "Grep".to_string(),
                    crate::session::ToolStats {
                        calls: 6 + (i as u32),
                    },
                );
                s.tool_usage = tools;
            }

            // File changes for detail panel
            if tick > 5 {
                let mut files = HashMap::new();
                files.insert(format!("/Users/dev/projects/{name}/src/main.rs"), 3);
                files.insert(format!("/Users/dev/projects/{name}/src/lib.rs"), 1);
                if i % 2 == 0 {
                    files.insert(format!("/Users/dev/projects/{name}/Cargo.toml"), 1);
                }
                s.files_modified = files;
            }

            s
        })
        .collect()
}
