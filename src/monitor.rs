use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};

use serde_json::Value;

use crate::models;
use crate::session::{ClaudeSession, SessionStatus, TelemetryStatus};
use crate::transcript::{TranscriptBlock, TranscriptEvent, TranscriptRole, parse_line};

/// Read new JSONL entries since last offset, accumulate token stats.
pub fn update_tokens(session: &mut ClaudeSession) {
    let Some(ref path) = session.jsonl_path else {
        session.telemetry_status = TelemetryStatus::MissingTranscript;
        return;
    };

    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(_) => {
            session.telemetry_status = TelemetryStatus::UnreadableTranscript;
            return;
        }
    };

    let file_len = file.metadata().map(|m| m.len()).unwrap_or(0);

    if file_len == 0 {
        session.telemetry_status = TelemetryStatus::Pending;
        return;
    }

    if session.jsonl_offset > file_len {
        session.jsonl_offset = 0;
    }

    if session.jsonl_offset > 0 && session.jsonl_offset >= file_len {
        return;
    }

    if session.jsonl_offset > 0 && file.seek(SeekFrom::Start(session.jsonl_offset)).is_err() {
        return;
    }

    let reader = BufReader::new(&file);
    let mut last_type = String::new();
    let mut last_stop_reason = String::new();
    let mut is_waiting_for_task = false;
    let mut saw_non_empty_line = false;
    let mut recognized_events = 0usize;

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };

        if line.trim().is_empty() {
            continue;
        }
        saw_non_empty_line = true;

        let Some(event) = parse_line(&line) else {
            continue;
        };
        recognized_events += 1;

        match event {
            TranscriptEvent::WaitingForTask => {
                is_waiting_for_task = true;
            }
            TranscriptEvent::Message(message) => {
                is_waiting_for_task = false;
                last_type = match message.role {
                    TranscriptRole::Assistant => "assistant".to_string(),
                    TranscriptRole::User => "user".to_string(),
                };

                if let Some(reason) = message.stop_reason {
                    last_stop_reason = reason;
                } else {
                    last_stop_reason.clear();
                }

                if let Some(usage) = message.usage {
                    let input = usage.input_tokens;
                    let cache_read = usage.cache_read_input_tokens;
                    let cache_create = usage.cache_creation_input_tokens;
                    let output = usage.output_tokens;

                    session.total_input_tokens += input + cache_read + cache_create;
                    session.total_output_tokens += output;
                    session.cache_read_tokens += cache_read;
                    session.cache_write_tokens += cache_create;
                    session.usage_metrics_available = true;

                    // Track context window: the input_tokens of the LAST API call
                    // represents the current prompt/context size
                    let context_size = input + cache_read + cache_create;
                    if context_size > 0 {
                        session.context_tokens = context_size;
                    }
                }

                if let Some(model) = message.model {
                    session.model = shorten_model(&model);
                }

                for block in message.content {
                    if let TranscriptBlock::ToolUse { name, input } = block {
                        record_tool_usage(&name, &input, session);
                    }
                }
            }
        }
    }

    if recognized_events > 0 || session.telemetry_status.is_available() {
        session.telemetry_status = TelemetryStatus::Available;
    } else if saw_non_empty_line {
        session.telemetry_status = TelemetryStatus::UnsupportedTranscript;
    } else {
        session.telemetry_status = TelemetryStatus::Pending;
    }

    session.jsonl_offset = file_len;

    // Use the JSONL file's mtime as "last activity" — reliable, no timestamp parsing needed
    if let Some(ref path) = session.jsonl_path {
        if let Ok(meta) = std::fs::metadata(path) {
            if let Ok(modified) = meta.modified() {
                let mtime_ms = modified
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                session.last_message_ts = mtime_ms;
            }
        }
    }

    let resolved_profile = models::resolve(&session.model);
    session.context_max = resolved_profile.profile.context_max;
    session.cost_estimate_unverified =
        resolved_profile.source == models::ModelProfileSource::Fallback;
    session.model_profile_source = resolved_profile.source.label().to_string();

    // Compute cost estimate based on model pricing
    session.cost_usd = if session.usage_metrics_available {
        estimate_cost(session)
    } else {
        0.0
    };

    infer_status(session, &last_type, &last_stop_reason, is_waiting_for_task);
}

pub fn infer_status(
    session: &mut ClaudeSession,
    last_msg_type: &str,
    last_stop_reason: &str,
    is_waiting_for_task: bool,
) {
    // CPU is the strongest real-time signal — if the process is burning CPU,
    // it's processing regardless of what the JSONL says (JSONL can lag).
    if session.cpu_percent > 5.0 {
        session.status = SessionStatus::Processing;
        return;
    }

    // NeedsInput: JSONL says waiting_for_task and CPU is low (confirmed idle)
    if is_waiting_for_task {
        session.status = SessionStatus::NeedsInput;
        return;
    }

    if !session.telemetry_status.is_available() && last_msg_type.is_empty() {
        session.status = SessionStatus::Unknown;
        return;
    }

    if last_msg_type == "assistant" && last_stop_reason == "end_turn" {
        // Claude finished its turn — waiting for user input
        // But if it's been a long time (>10 min), mark as Idle
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let age_mins = (now_ms.saturating_sub(session.last_message_ts)) / 60_000;

        if age_mins > 10 {
            session.status = SessionStatus::Idle;
        } else {
            session.status = SessionStatus::WaitingInput;
        }
        return;
    }

    if last_msg_type == "assistant" && last_stop_reason == "tool_use" {
        // Claude called a tool. If CPU is low and some time has passed,
        // it's likely waiting for user to approve/deny the tool (permission prompt).
        // The permission prompt doesn't emit waiting_for_task — detect via CPU + age.
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let age_secs = (now_ms.saturating_sub(session.last_message_ts)) / 1000;

        if session.cpu_percent < 2.0 && age_secs > 5 {
            // Low CPU + tool_use was >5s ago = stuck on permission prompt
            session.status = SessionStatus::NeedsInput;
        } else {
            session.status = SessionStatus::Processing;
        }
        return;
    }

    if last_msg_type == "user" {
        // User sent a message, Claude hasn't finished responding
        if session.cpu_percent > 1.0 {
            session.status = SessionStatus::Processing;
        } else {
            // Low CPU + user message pending — might be waiting for API or stalled
            session.status = SessionStatus::Processing;
        }
        return;
    }

    session.status = SessionStatus::Idle;
}

/// Estimate USD cost based on token usage and model.
pub fn estimate_cost(session: &ClaudeSession) -> f64 {
    // Plain input tokens = total_input - cache_read - cache_write
    let plain_input = session
        .total_input_tokens
        .saturating_sub(session.cache_read_tokens)
        .saturating_sub(session.cache_write_tokens);

    let profile = models::resolve(&session.model).profile;

    (plain_input as f64 / 1_000_000.0) * profile.input_per_m
        + (session.total_output_tokens as f64 / 1_000_000.0) * profile.output_per_m
        + (session.cache_read_tokens as f64 / 1_000_000.0) * profile.cache_read_per_m
        + (session.cache_write_tokens as f64 / 1_000_000.0) * profile.cache_write_per_m
}

/// Max context window tokens by model.
pub fn model_context_max(model: &str) -> u64 {
    models::resolve(model).profile.context_max
}

/// Extract tool usage stats and file paths from tool_use content blocks.
fn record_tool_usage(tool_name: &str, input: &Value, session: &mut ClaudeSession) {
    if tool_name.is_empty() {
        return;
    }

    session
        .tool_usage
        .entry(tool_name.to_string())
        .or_default()
        .calls += 1;

    if matches!(tool_name, "Edit" | "Write" | "NotebookEdit") {
        if let Some(path) = input.get("file_path").and_then(|p| p.as_str()) {
            *session.files_modified.entry(path.to_string()).or_insert(0) += 1;
        }
    }
}

pub fn shorten_model(model: &str) -> String {
    models::shorten_model(model)
}
