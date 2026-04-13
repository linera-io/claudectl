use std::io::Write;
use std::time::Duration;

use claudectl::monitor;
use claudectl::session::{ClaudeSession, RawSession, SessionStatus};

/// Helper: create a minimal session for testing status inference.
fn make_session(cpu: f32, last_message_age_secs: u64) -> ClaudeSession {
    let raw = RawSession {
        pid: 1,
        session_id: "test-session".into(),
        cwd: "/tmp/test-project".into(),
        started_at: 0,
    };
    let mut s = ClaudeSession::from_raw(raw);
    s.cpu_percent = cpu;

    // Set last_message_ts relative to now
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    s.last_message_ts = now_ms.saturating_sub(last_message_age_secs * 1000);
    s
}

// ────────────────────────────────────────────────────────────────────────────
// Status Inference Tests
// ────────────────────────────────────────────────────────────────────────────

#[test]
fn status_high_cpu_always_processing() {
    let mut s = make_session(50.0, 0);
    monitor::infer_status(&mut s, "", "", false);
    assert_eq!(s.status, SessionStatus::Processing);
}

#[test]
fn status_high_cpu_overrides_waiting_for_task() {
    let mut s = make_session(10.0, 0);
    monitor::infer_status(&mut s, "assistant", "end_turn", true);
    assert_eq!(s.status, SessionStatus::Processing);
}

#[test]
fn status_high_cpu_overrides_end_turn() {
    let mut s = make_session(20.0, 60);
    monitor::infer_status(&mut s, "assistant", "end_turn", false);
    assert_eq!(s.status, SessionStatus::Processing);
}

#[test]
fn status_waiting_for_task_needs_input() {
    let mut s = make_session(0.5, 10);
    monitor::infer_status(&mut s, "", "", true);
    assert_eq!(s.status, SessionStatus::NeedsInput);
}

#[test]
fn status_end_turn_recent_waiting_input() {
    // Assistant said end_turn, 2 minutes ago, low CPU
    let mut s = make_session(0.5, 120);
    monitor::infer_status(&mut s, "assistant", "end_turn", false);
    assert_eq!(s.status, SessionStatus::WaitingInput);
}

#[test]
fn status_end_turn_old_idle() {
    // Assistant said end_turn, 15 minutes ago → Idle
    let mut s = make_session(0.5, 15 * 60);
    monitor::infer_status(&mut s, "assistant", "end_turn", false);
    assert_eq!(s.status, SessionStatus::Idle);
}

#[test]
fn status_end_turn_exactly_10min_still_waiting() {
    // 10 minutes = boundary, should still be WaitingInput (>10 is Idle)
    let mut s = make_session(0.5, 10 * 60);
    monitor::infer_status(&mut s, "assistant", "end_turn", false);
    assert_eq!(s.status, SessionStatus::WaitingInput);
}

#[test]
fn status_end_turn_11min_idle() {
    let mut s = make_session(0.5, 11 * 60);
    monitor::infer_status(&mut s, "assistant", "end_turn", false);
    assert_eq!(s.status, SessionStatus::Idle);
}

#[test]
fn status_tool_use_low_cpu_old_needs_input() {
    // tool_use + low CPU + >5s ago = permission prompt
    let mut s = make_session(0.5, 30);
    monitor::infer_status(&mut s, "assistant", "tool_use", false);
    assert_eq!(s.status, SessionStatus::NeedsInput);
}

#[test]
fn status_tool_use_low_cpu_recent_processing() {
    // tool_use + low CPU + <5s ago = still processing (tool just fired)
    let mut s = make_session(0.5, 2);
    monitor::infer_status(&mut s, "assistant", "tool_use", false);
    assert_eq!(s.status, SessionStatus::Processing);
}

#[test]
fn status_tool_use_high_cpu_processing() {
    // tool_use + high CPU = still crunching
    let mut s = make_session(15.0, 30);
    monitor::infer_status(&mut s, "assistant", "tool_use", false);
    assert_eq!(s.status, SessionStatus::Processing);
}

#[test]
fn status_user_message_pending_processing() {
    let mut s = make_session(3.0, 5);
    monitor::infer_status(&mut s, "user", "", false);
    assert_eq!(s.status, SessionStatus::Processing);
}

#[test]
fn status_user_message_low_cpu_still_processing() {
    // User sent message, CPU low — could be waiting for API
    let mut s = make_session(0.5, 5);
    monitor::infer_status(&mut s, "user", "", false);
    assert_eq!(s.status, SessionStatus::Processing);
}

#[test]
fn status_no_signals_idle() {
    // No JSONL signals at all → Idle
    let mut s = make_session(0.0, 0);
    monitor::infer_status(&mut s, "", "", false);
    assert_eq!(s.status, SessionStatus::Idle);
}

#[test]
fn status_cpu_threshold_boundary() {
    // CPU exactly 5.0 — should NOT trigger Processing (threshold is >5.0)
    let mut s = make_session(5.0, 0);
    monitor::infer_status(&mut s, "", "", false);
    assert_eq!(s.status, SessionStatus::Idle);

    // CPU 5.1 — should trigger Processing
    let mut s2 = make_session(5.1, 0);
    monitor::infer_status(&mut s2, "", "", false);
    assert_eq!(s2.status, SessionStatus::Processing);
}

// ────────────────────────────────────────────────────────────────────────────
// Cost Estimation Tests
// ────────────────────────────────────────────────────────────────────────────

#[test]
fn cost_opus_tokens() {
    let mut s = make_session(0.0, 0);
    s.model = "opus-4.6".into();
    s.total_input_tokens = 1_000_000;
    s.total_output_tokens = 100_000;
    s.cache_read_tokens = 500_000;
    s.cache_write_tokens = 200_000;

    let cost = monitor::estimate_cost(&s);
    // plain_input = 1M - 500k - 200k = 300k
    // cost = 300k/1M * 15 + 100k/1M * 75 + 500k/1M * 1.875 + 200k/1M * 18.75
    //      = 0.3 * 15 + 0.1 * 75 + 0.5 * 1.875 + 0.2 * 18.75
    //      = 4.5 + 7.5 + 0.9375 + 3.75 = 16.6875
    let expected = 16.6875;
    assert!(
        (cost - expected).abs() < 0.001,
        "opus cost={cost}, expected={expected}"
    );
}

#[test]
fn cost_sonnet_tokens() {
    let mut s = make_session(0.0, 0);
    s.model = "sonnet-4.6".into();
    s.total_input_tokens = 100_000;
    s.total_output_tokens = 50_000;
    s.cache_read_tokens = 0;
    s.cache_write_tokens = 0;

    let cost = monitor::estimate_cost(&s);
    // plain_input = 100k
    // cost = 100k/1M * 3 + 50k/1M * 15 = 0.3 + 0.75 = 1.05
    let expected = 1.05;
    assert!(
        (cost - expected).abs() < 0.001,
        "sonnet cost={cost}, expected={expected}"
    );
}

#[test]
fn cost_haiku_tokens() {
    let mut s = make_session(0.0, 0);
    s.model = "haiku".into();
    s.total_input_tokens = 100_000;
    s.total_output_tokens = 50_000;
    s.cache_read_tokens = 0;
    s.cache_write_tokens = 0;

    let cost = monitor::estimate_cost(&s);
    // plain_input = 100k
    // cost = 100k/1M * 0.80 + 50k/1M * 4.0 = 0.08 + 0.2 = 0.28
    let expected = 0.28;
    assert!(
        (cost - expected).abs() < 0.001,
        "haiku cost={cost}, expected={expected}"
    );
}

#[test]
fn cost_unknown_model_defaults_to_opus() {
    let mut s = make_session(0.0, 0);
    s.model = "some-future-model".into();
    s.total_input_tokens = 1_000_000;
    s.total_output_tokens = 0;
    s.cache_read_tokens = 0;
    s.cache_write_tokens = 0;

    let cost = monitor::estimate_cost(&s);
    // Should use opus pricing: 1M/1M * 15 = 15.0
    let expected = 15.0;
    assert!(
        (cost - expected).abs() < 0.001,
        "unknown model cost={cost}, expected={expected}"
    );
}

#[test]
fn cost_zero_tokens() {
    let s = make_session(0.0, 0);
    let cost = monitor::estimate_cost(&s);
    assert_eq!(cost, 0.0);
}

// ────────────────────────────────────────────────────────────────────────────
// Model Context Max Tests
// ────────────────────────────────────────────────────────────────────────────

#[test]
fn context_max_opus() {
    assert_eq!(monitor::model_context_max("opus-4.6"), 1_000_000);
    assert_eq!(monitor::model_context_max("opus"), 1_000_000);
}

#[test]
fn context_max_sonnet() {
    assert_eq!(monitor::model_context_max("sonnet-4.6"), 200_000);
    assert_eq!(monitor::model_context_max("sonnet"), 200_000);
}

#[test]
fn context_max_haiku() {
    assert_eq!(monitor::model_context_max("haiku"), 200_000);
}

#[test]
fn context_max_unknown() {
    assert_eq!(monitor::model_context_max("unknown-model"), 200_000);
}

// ────────────────────────────────────────────────────────────────────────────
// Model Shortening Tests
// ────────────────────────────────────────────────────────────────────────────

#[test]
fn shorten_model_opus_46() {
    assert_eq!(
        monitor::shorten_model("claude-opus-4-6-20260401"),
        "opus-4.6"
    );
}

#[test]
fn shorten_model_opus_generic() {
    assert_eq!(monitor::shorten_model("claude-opus-20260101"), "opus");
}

#[test]
fn shorten_model_sonnet_46() {
    assert_eq!(
        monitor::shorten_model("claude-sonnet-4-6-20260401"),
        "sonnet-4.6"
    );
}

#[test]
fn shorten_model_sonnet_generic() {
    assert_eq!(monitor::shorten_model("claude-sonnet-20260101"), "sonnet");
}

#[test]
fn shorten_model_haiku() {
    assert_eq!(monitor::shorten_model("claude-haiku-4-5-20251001"), "haiku");
}

#[test]
fn shorten_model_unknown() {
    assert_eq!(monitor::shorten_model("gpt-4o"), "gpt-4o");
}

// ────────────────────────────────────────────────────────────────────────────
// JSONL Parsing Integration Tests (using temp files)
// ────────────────────────────────────────────────────────────────────────────

fn make_session_with_jsonl(content: &str) -> (ClaudeSession, tempfile::NamedTempFile) {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(content.as_bytes()).unwrap();
    file.flush().unwrap();

    let raw = RawSession {
        pid: 1,
        session_id: "test".into(),
        cwd: "/tmp/test".into(),
        started_at: 0,
    };
    let mut s = ClaudeSession::from_raw(raw);
    s.jsonl_path = Some(file.path().to_path_buf());
    (s, file)
}

#[test]
fn jsonl_parse_token_usage() {
    let jsonl = r#"{"type":"assistant","message":{"model":"claude-opus-4-6-20260401","stop_reason":"end_turn","usage":{"input_tokens":50000,"output_tokens":10000,"cache_read_input_tokens":20000,"cache_creation_input_tokens":5000}}}"#;

    let (mut s, _file) = make_session_with_jsonl(jsonl);
    monitor::update_tokens(&mut s);

    assert_eq!(s.total_input_tokens, 75000); // 50000 + 20000 + 5000
    assert_eq!(s.total_output_tokens, 10000);
    assert_eq!(s.cache_read_tokens, 20000);
    assert_eq!(s.cache_write_tokens, 5000);
    assert_eq!(s.model, "opus-4.6");
    assert_eq!(s.context_max, 1_000_000);
}

#[test]
fn jsonl_parse_multiple_entries() {
    let jsonl = concat!(
        r#"{"type":"user","message":{"type":"user"}}"#,
        "\n",
        r#"{"type":"assistant","message":{"model":"claude-sonnet-4-6-20260401","stop_reason":"tool_use","usage":{"input_tokens":1000,"output_tokens":500,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}"#,
        "\n",
        r#"{"type":"assistant","message":{"model":"claude-sonnet-4-6-20260401","stop_reason":"end_turn","usage":{"input_tokens":2000,"output_tokens":1000,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}"#,
    );

    let (mut s, _file) = make_session_with_jsonl(jsonl);
    monitor::update_tokens(&mut s);

    assert_eq!(s.total_input_tokens, 3000); // 1000 + 2000
    assert_eq!(s.total_output_tokens, 1500); // 500 + 1000
    assert_eq!(s.model, "sonnet-4.6");
}

#[test]
fn jsonl_incremental_reads() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    let line1 = r#"{"type":"assistant","message":{"model":"claude-opus-4-6-20260401","stop_reason":"end_turn","usage":{"input_tokens":1000,"output_tokens":500,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}"#;
    writeln!(file, "{line1}").unwrap();
    file.flush().unwrap();

    let raw = RawSession {
        pid: 1,
        session_id: "test".into(),
        cwd: "/tmp/test".into(),
        started_at: 0,
    };
    let mut s = ClaudeSession::from_raw(raw);
    s.jsonl_path = Some(file.path().to_path_buf());

    // First read
    monitor::update_tokens(&mut s);
    assert_eq!(s.total_input_tokens, 1000);
    assert_eq!(s.total_output_tokens, 500);

    // Second read with no new data — should not double-count
    monitor::update_tokens(&mut s);
    assert_eq!(s.total_input_tokens, 1000);
    assert_eq!(s.total_output_tokens, 500);

    // Append more data
    let line2 = r#"{"type":"assistant","message":{"model":"claude-opus-4-6-20260401","stop_reason":"end_turn","usage":{"input_tokens":2000,"output_tokens":800,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}"#;
    writeln!(file, "{line2}").unwrap();
    file.flush().unwrap();

    // Third read — should pick up new data only
    monitor::update_tokens(&mut s);
    assert_eq!(s.total_input_tokens, 3000);
    assert_eq!(s.total_output_tokens, 1300);
}

#[test]
fn jsonl_empty_file() {
    let (mut s, _file) = make_session_with_jsonl("");
    monitor::update_tokens(&mut s);
    assert_eq!(s.total_input_tokens, 0);
    assert_eq!(s.total_output_tokens, 0);
}

#[test]
fn jsonl_corrupted_lines_skipped() {
    let jsonl = concat!(
        "not valid json at all\n",
        "{\"type\":\"something but no usage\"}\n",
        r#"{"type":"assistant","message":{"model":"claude-opus-4-6-20260401","stop_reason":"end_turn","usage":{"input_tokens":5000,"output_tokens":1000,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}"#,
    );

    let (mut s, _file) = make_session_with_jsonl(jsonl);
    monitor::update_tokens(&mut s);

    // Should still parse the valid line
    assert_eq!(s.total_input_tokens, 5000);
    assert_eq!(s.total_output_tokens, 1000);
}

#[test]
fn jsonl_waiting_for_task_detection() {
    let jsonl = concat!(
        r#"{"type":"assistant","message":{"model":"claude-opus-4-6-20260401","stop_reason":"end_turn","usage":{"input_tokens":1000,"output_tokens":500,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}"#,
        "\n",
        r#"{"type":"progress","data":"waiting_for_task"}"#,
    );

    let (mut s, _file) = make_session_with_jsonl(jsonl);
    s.cpu_percent = 0.5; // Low CPU
    monitor::update_tokens(&mut s);

    // Status should be NeedsInput (waiting_for_task + low CPU)
    assert_eq!(s.status, SessionStatus::NeedsInput);
}

#[test]
fn jsonl_missing_file() {
    let raw = RawSession {
        pid: 1,
        session_id: "test".into(),
        cwd: "/tmp/test".into(),
        started_at: 0,
    };
    let mut s = ClaudeSession::from_raw(raw);
    s.jsonl_path = Some(std::path::PathBuf::from("/nonexistent/path.jsonl"));

    // Should not panic
    monitor::update_tokens(&mut s);
    assert_eq!(s.total_input_tokens, 0);
}

#[test]
fn jsonl_no_path() {
    let raw = RawSession {
        pid: 1,
        session_id: "test".into(),
        cwd: "/tmp/test".into(),
        started_at: 0,
    };
    let mut s = ClaudeSession::from_raw(raw);
    // jsonl_path is None

    monitor::update_tokens(&mut s);
    assert_eq!(s.total_input_tokens, 0);
}

// ────────────────────────────────────────────────────────────────────────────
// Session Formatting Edge Cases
// ────────────────────────────────────────────────────────────────────────────

#[test]
fn context_percent_zero_max() {
    let mut s = make_session(0.0, 0);
    s.context_max = 0;
    s.context_tokens = 1000;
    assert_eq!(s.context_percent(), 0.0);
}

#[test]
fn context_percent_zero_tokens() {
    let mut s = make_session(0.0, 0);
    s.context_max = 200_000;
    s.context_tokens = 0;
    assert_eq!(s.context_percent(), 0.0);
}

#[test]
fn context_percent_calculation() {
    let mut s = make_session(0.0, 0);
    s.context_max = 200_000;
    s.context_tokens = 100_000;
    assert!((s.context_percent() - 50.0).abs() < 0.01);
}

#[test]
fn sparkline_empty() {
    let s = make_session(0.0, 0);
    assert_eq!(s.format_sparkline(), "-");
}

#[test]
fn sparkline_records_and_renders() {
    let mut s = make_session(0.0, 0);
    s.status = SessionStatus::Processing;
    s.record_activity();
    s.status = SessionStatus::Idle;
    s.record_activity();

    let sparkline = s.format_sparkline();
    assert_eq!(sparkline.chars().count(), 2);
}

#[test]
fn sparkline_ring_buffer_limit() {
    let mut s = make_session(0.0, 0);
    for _ in 0..20 {
        s.status = SessionStatus::Processing;
        s.record_activity();
    }
    // Should be capped at 15
    assert_eq!(s.activity_history.len(), 15);
}

#[test]
fn json_export_format() {
    let mut s = make_session(0.0, 0);
    s.model = "opus-4.6".into();
    s.cost_usd = 1.234;
    s.total_input_tokens = 50000;
    s.total_output_tokens = 10000;
    s.elapsed = Duration::from_secs(300);

    let json = s.to_json_value();
    assert_eq!(json["pid"], 1);
    assert_eq!(json["status"], "Idle");
    assert_eq!(json["elapsed_secs"], 300);
    assert_eq!(json["tokens_in"], 50000);
    assert_eq!(json["tokens_out"], 10000);
}

#[test]
fn burn_rate_formatting() {
    let mut s = make_session(0.0, 0);
    assert_eq!(s.format_burn_rate(), "-");

    s.burn_rate_per_hr = 0.50;
    assert_eq!(s.format_burn_rate(), "$0.50/h");

    s.burn_rate_per_hr = 3.5;
    assert_eq!(s.format_burn_rate(), "$3.5/h");
}

#[test]
fn mem_formatting() {
    let mut s = make_session(0.0, 0);
    assert_eq!(s.format_mem(), "-");

    s.mem_mb = 256.7;
    assert_eq!(s.format_mem(), "257M");
}

#[test]
fn context_bar_formatting() {
    let mut s = make_session(0.0, 0);
    assert_eq!(s.format_context_bar(10), "-");

    s.context_max = 200_000;
    s.context_tokens = 100_000; // 50%
    let bar = s.format_context_bar(10);
    assert!(bar.contains("50%"));
    assert!(bar.contains("█████"));
    assert!(bar.contains("░░░░░"));
}

// ────────────────────────────────────────────────────────────────────────────
// Session Recorder Tests
// ────────────────────────────────────────────────────────────────────────────

#[test]
fn session_recorder_produces_highlight_reel() {
    use claudectl::session_recorder::SessionRecorder;

    // Create a JSONL matching real Claude Code format (message.role, not message.type)
    let mut jsonl_file = tempfile::NamedTempFile::new().unwrap();
    writeln!(jsonl_file, r#"{{"message":{{"role":"assistant","type":"message","content":[{{"type":"text","text":"I'll fix the authentication bug by updating the middleware."}}],"stop_reason":"tool_use"}}}}"#).unwrap();
    writeln!(jsonl_file, r#"{{"message":{{"role":"assistant","type":"message","content":[{{"type":"tool_use","name":"Edit","input":{{"file_path":"/src/auth.rs","old_string":"fn check()","new_string":"fn check_auth(token: &str)"}}}}],"stop_reason":"tool_use"}}}}"#).unwrap();
    writeln!(jsonl_file, r#"{{"message":{{"role":"assistant","type":"message","content":[{{"type":"tool_use","name":"Bash","input":{{"command":"cargo test"}}}}],"stop_reason":"tool_use"}}}}"#).unwrap();
    writeln!(jsonl_file, r#"{{"message":{{"role":"user","type":"message","content":[{{"type":"tool_result","content":"test result: ok. 12 passed","is_error":false}}]}}}}"#).unwrap();
    writeln!(jsonl_file, r#"{{"message":{{"role":"assistant","type":"message","content":[{{"type":"tool_use","name":"Read","input":{{"file_path":"/src/main.rs"}}}}],"stop_reason":"tool_use"}}}}"#).unwrap();
    jsonl_file.flush().unwrap();

    let output_file = tempfile::NamedTempFile::new().unwrap();
    let output_path = output_file.path().to_str().unwrap().to_string() + ".cast";

    let mut rec = SessionRecorder::new(jsonl_file.path(), &output_path, "test-project", 120, 40)
        .expect("Failed to create session recorder");

    let had_events = rec.poll().expect("Failed to poll");
    assert!(had_events, "Should have found events in the JSONL");

    rec.finish().expect("Failed to finish recording");

    let content = std::fs::read_to_string(&output_path).unwrap();
    let lines: Vec<&str> = content.lines().collect();

    // First line is the asciicast header
    assert!(
        lines[0].contains("\"version\":2"),
        "Should have asciicast v2 header"
    );
    assert!(
        lines[0].contains("test-project"),
        "Header should contain session name"
    );

    // Should have multiple frames (header + title card + events + finish)
    assert!(
        lines.len() >= 4,
        "Should have at least 4 lines (header + title + events + finish), got {}",
        lines.len()
    );

    // Should contain the Edit tool rendered as Claude Code style "Update(file)"
    let full = content.to_string();
    assert!(
        full.contains("Update"),
        "Should contain Update event for Edit tool"
    );
    assert!(full.contains("auth.rs"), "Should contain edited file name");

    // Should contain the Bash command rendered Claude Code style
    assert!(
        full.contains("bash command"),
        "Should contain bash command indicator"
    );
    assert!(full.contains("cargo test"), "Should contain bash command");

    // Read events should appear as brief gray context lines (not full highlight frames)
    assert!(
        full.contains("Read"),
        "Read tool should appear as context line"
    );

    // Should contain final summary
    assert!(
        full.contains("complete"),
        "Should contain completion message"
    );

    // Clean up
    let _ = std::fs::remove_file(&output_path);
}

#[test]
fn session_recorder_empty_jsonl() {
    use claudectl::session_recorder::SessionRecorder;

    let jsonl_file = tempfile::NamedTempFile::new().unwrap();
    let output_file = tempfile::NamedTempFile::new().unwrap();
    let output_path = output_file.path().to_str().unwrap().to_string() + ".cast";

    let mut rec = SessionRecorder::new(jsonl_file.path(), &output_path, "empty-session", 80, 24)
        .expect("Failed to create recorder");

    let had_events = rec.poll().expect("Failed to poll");
    assert!(!had_events, "Empty JSONL should produce no events");

    rec.finish().expect("Failed to finish");

    let content = std::fs::read_to_string(&output_path).unwrap();
    assert!(
        content.contains("\"version\":2"),
        "Should still have header"
    );

    let _ = std::fs::remove_file(&output_path);
}

#[test]
fn recorder_cast_file_creation() {
    use claudectl::recorder::Recorder;

    let output_file = tempfile::NamedTempFile::new().unwrap();
    let output_path = output_file.path().to_str().unwrap().to_string() + ".cast";

    let mut rec = Recorder::new(&output_path, 120, 40).expect("Failed to create recorder");
    rec.capture(b"hello world");
    rec.flush_frame().expect("Failed to flush");
    rec.capture(b"second frame");
    rec.flush_frame().expect("Failed to flush");
    rec.finish().expect("Failed to finish");

    let content = std::fs::read_to_string(&output_path).unwrap();
    let lines: Vec<&str> = content.lines().collect();

    assert!(lines[0].contains("\"version\":2"));
    assert!(lines[0].contains("\"width\":120"));
    assert!(lines[0].contains("\"height\":40"));
    assert!(
        lines.len() == 3,
        "Should have header + 2 frames, got {}",
        lines.len()
    );
    assert!(lines[1].contains("hello world"));
    assert!(lines[2].contains("second frame"));

    let _ = std::fs::remove_file(&output_path);
}
