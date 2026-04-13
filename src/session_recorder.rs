use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::recorder::Recorder;

/// Maximum characters of bash output to include in a frame.
const MAX_BASH_OUTPUT: usize = 500;
/// Maximum characters of assistant text to include.
const MAX_ASSISTANT_TEXT: usize = 200;
/// Idle time compression: any gap > this becomes this duration in the recording.
const MAX_IDLE_SECS: f64 = 1.5;

/// Records a single Claude Code session by tailing its JSONL file.
pub struct SessionRecorder {
    jsonl_path: PathBuf,
    offset: u64,
    recorder: Recorder,
    last_event_time: Instant,
    frame_count: u32,
    width: u16,
}

/// A parsed event from the JSONL stream.
enum SessionEvent {
    /// Claude said something (brief text)
    AssistantText(String),
    /// Claude used a tool
    ToolUse { tool: String, summary: String },
    /// Tool returned a result
    ToolResult { output: String, is_error: bool },
    /// Status transition
    StatusChange(String),
}

impl SessionRecorder {
    pub fn new(
        jsonl_path: &Path,
        output_path: &str,
        width: u16,
        height: u16,
    ) -> std::io::Result<Self> {
        let recorder = Recorder::new(output_path, width, height)?;
        Ok(Self {
            jsonl_path: jsonl_path.to_path_buf(),
            offset: 0,
            recorder,
            last_event_time: Instant::now(),
            frame_count: 0,
            width,
        })
    }

    /// Read new JSONL lines and emit recording frames for interesting events.
    /// Returns true if new events were processed.
    pub fn poll(&mut self) -> std::io::Result<bool> {
        let mut file = match File::open(&self.jsonl_path) {
            Ok(f) => f,
            Err(_) => return Ok(false),
        };

        let file_len = file.metadata().map(|m| m.len()).unwrap_or(0);
        if self.offset >= file_len {
            return Ok(false);
        }

        if self.offset > 0 {
            file.seek(SeekFrom::Start(self.offset))?;
        }

        let reader = BufReader::new(&file);
        let mut had_events = false;

        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => break,
            };

            let events = parse_events(&line);
            for event in events {
                self.emit_frame(&event)?;
                had_events = true;
            }
        }

        self.offset = file_len;
        Ok(had_events)
    }

    fn emit_frame(&mut self, event: &SessionEvent) -> std::io::Result<()> {
        let frame = self.render_event(event);
        if frame.is_empty() {
            return Ok(());
        }

        // Time compression: if there was a long gap, compress it
        let elapsed = self.last_event_time.elapsed().as_secs_f64();
        if elapsed > MAX_IDLE_SECS && self.frame_count > 0 {
            // The recorder uses real time; we compress by just writing quickly
            // The tee writer captures real time, but for session recording
            // we write directly to the recorder
        }

        self.recorder.capture(frame.as_bytes());
        self.recorder.flush_frame()?;
        self.last_event_time = Instant::now();
        self.frame_count += 1;
        Ok(())
    }

    fn render_event(&self, event: &SessionEvent) -> String {
        let w = self.width as usize;
        let sep: String = "─".repeat(w.min(80));

        match event {
            SessionEvent::AssistantText(text) => {
                let truncated = if text.len() > MAX_ASSISTANT_TEXT {
                    format!("{}...", &text[..MAX_ASSISTANT_TEXT])
                } else {
                    text.clone()
                };
                format!(
                    "\x1b[2J\x1b[H\x1b[1;36m Claude \x1b[0m\r\n{sep}\r\n\x1b[37m{truncated}\x1b[0m\r\n{sep}\r\n"
                )
            }
            SessionEvent::ToolUse { tool, summary } => {
                let icon = match tool.as_str() {
                    "Edit" => "✏️ ",
                    "Write" => "📝",
                    "Bash" => "⚡",
                    "Read" => "📖",
                    "Grep" => "🔍",
                    "Glob" => "📂",
                    "Agent" => "🤖",
                    _ => "🔧",
                };
                format!(
                    "\x1b[1;33m {icon} {tool}\x1b[0m\r\n\x1b[90m{sep}\x1b[0m\r\n\x1b[37m{summary}\x1b[0m\r\n\r\n"
                )
            }
            SessionEvent::ToolResult { output, is_error } => {
                let color = if *is_error { "1;31" } else { "32" };
                let truncated = if output.len() > MAX_BASH_OUTPUT {
                    format!("{}...", &output[..MAX_BASH_OUTPUT])
                } else {
                    output.clone()
                };
                // Replace newlines with \r\n for terminal
                let display = truncated.replace('\n', "\r\n");
                format!("\x1b[{color}m{display}\x1b[0m\r\n\r\n")
            }
            SessionEvent::StatusChange(status) => {
                format!("\x1b[1;35m → {status}\x1b[0m\r\n")
            }
        }
    }

    pub fn finish(&mut self) -> std::io::Result<()> {
        // Write a final "recording complete" frame
        self.recorder
            .capture(b"\x1b[2J\x1b[H\x1b[1;32m Recording complete \x1b[0m\r\n");
        self.recorder.flush_frame()?;
        self.recorder.finish()
    }
}

/// Parse a JSONL line into zero or more session events.
fn parse_events(line: &str) -> Vec<SessionEvent> {
    let mut events = Vec::new();

    let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) else {
        return events;
    };

    // Check for assistant message with content blocks
    let msg = match entry.get("message") {
        Some(m) => m,
        None => return events,
    };

    let msg_type = msg.get("type").and_then(|t| t.as_str()).unwrap_or("");
    let content = msg.get("content").and_then(|c| c.as_array());

    if msg_type == "assistant" {
        if let Some(blocks) = content {
            for block in blocks {
                let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");

                match block_type {
                    "text" => {
                        if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                            let trimmed = text.trim();
                            if !trimmed.is_empty() && trimmed.len() > 10 {
                                events.push(SessionEvent::AssistantText(trimmed.to_string()));
                            }
                        }
                    }
                    "tool_use" => {
                        let tool = block
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("unknown")
                            .to_string();

                        let summary = summarize_tool_use(&tool, block.get("input"));
                        events.push(SessionEvent::ToolUse { tool, summary });
                    }
                    "tool_result" => {
                        let is_error = block
                            .get("is_error")
                            .and_then(|e| e.as_bool())
                            .unwrap_or(false);
                        let output = block
                            .get("content")
                            .and_then(|c| {
                                if let Some(s) = c.as_str() {
                                    Some(s.to_string())
                                } else if let Some(arr) = c.as_array() {
                                    arr.first()
                                        .and_then(|b| b.get("text"))
                                        .and_then(|t| t.as_str())
                                        .map(|s| s.to_string())
                                } else {
                                    None
                                }
                            })
                            .unwrap_or_default();

                        if !output.is_empty() {
                            events.push(SessionEvent::ToolResult { output, is_error });
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // Detect status changes via stop_reason
    if let Some(reason) = msg.get("stop_reason").and_then(|r| r.as_str()) {
        match reason {
            "end_turn" => events.push(SessionEvent::StatusChange("Done".to_string())),
            "tool_use" => {} // Normal, don't clutter
            _ => events.push(SessionEvent::StatusChange(reason.to_string())),
        }
    }

    events
}

/// Produce a human-readable summary of a tool use invocation.
fn summarize_tool_use(tool: &str, input: Option<&serde_json::Value>) -> String {
    let input = match input {
        Some(v) => v,
        None => return String::new(),
    };

    match tool {
        "Edit" => {
            let file = input
                .get("file_path")
                .and_then(|p| p.as_str())
                .unwrap_or("?");
            let short = file.rsplit('/').next().unwrap_or(file);
            let old_len = input
                .get("old_string")
                .and_then(|s| s.as_str())
                .map(|s| s.len())
                .unwrap_or(0);
            let new_len = input
                .get("new_string")
                .and_then(|s| s.as_str())
                .map(|s| s.len())
                .unwrap_or(0);
            format!("{short}  ({old_len} → {new_len} chars)")
        }
        "Write" => {
            let file = input
                .get("file_path")
                .and_then(|p| p.as_str())
                .unwrap_or("?");
            let short = file.rsplit('/').next().unwrap_or(file);
            let content_len = input
                .get("content")
                .and_then(|s| s.as_str())
                .map(|s| s.len())
                .unwrap_or(0);
            format!("{short}  ({content_len} chars)")
        }
        "Bash" => {
            let cmd = input.get("command").and_then(|c| c.as_str()).unwrap_or("?");
            if cmd.len() > 80 {
                format!("{}...", &cmd[..77])
            } else {
                cmd.to_string()
            }
        }
        "Read" => {
            let file = input
                .get("file_path")
                .and_then(|p| p.as_str())
                .unwrap_or("?");
            let short = file.rsplit('/').next().unwrap_or(file);
            short.to_string()
        }
        "Grep" => {
            let pattern = input.get("pattern").and_then(|p| p.as_str()).unwrap_or("?");
            format!("/{pattern}/")
        }
        "Glob" => {
            let pattern = input.get("pattern").and_then(|p| p.as_str()).unwrap_or("?");
            pattern.to_string()
        }
        _ => String::new(),
    }
}
