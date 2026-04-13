use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Maximum characters of bash output to include in a frame.
const MAX_BASH_OUTPUT: usize = 400;
/// Maximum characters of assistant text to include.
const MAX_ASSISTANT_TEXT: usize = 160;
/// Seconds between frames in the highlight reel.
const FRAME_PACE: f64 = 1.2;
/// Seconds to hold tool results before next event.
const RESULT_HOLD: f64 = 2.0;
/// Seconds to hold the title card.
const TITLE_HOLD: f64 = 3.0;

/// Records a single Claude Code session as a highlight reel.
pub struct SessionRecorder {
    jsonl_path: PathBuf,
    offset: u64,
    cast_file: File,
    cast_path: PathBuf,
    final_path: PathBuf,
    is_gif: bool,
    virtual_time: f64, // Synthetic clock for paced playback
    width: u16,
    height: u16,
    title_written: bool,
    session_name: String,
    // Running tally for the header
    edits: u32,
    commands: u32,
    errors: u32,
}

/// A parsed event from the JSONL stream.
enum SessionEvent {
    AssistantText(String),
    ToolUse { tool: String, summary: String },
    ToolResult { output: String, is_error: bool },
}

/// Which tool events make the highlight reel.
fn is_highlight_tool(tool: &str) -> bool {
    matches!(tool, "Edit" | "Write" | "Bash" | "Agent" | "NotebookEdit")
}

impl SessionRecorder {
    pub fn new(
        jsonl_path: &Path,
        output_path: &str,
        session_name: &str,
        width: u16,
        height: u16,
    ) -> std::io::Result<Self> {
        let is_gif = output_path.ends_with(".gif");
        let final_path = PathBuf::from(output_path);

        let cast_path = if is_gif {
            let mut tmp = std::env::temp_dir();
            tmp.push(format!("claudectl-sess-{}.cast", std::process::id()));
            tmp
        } else {
            final_path.clone()
        };

        let mut cast_file = File::create(&cast_path)?;

        // Write asciicast v2 header
        let header = serde_json::json!({
            "version": 2,
            "width": width,
            "height": height,
            "timestamp": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            "title": format!("claudectl: {session_name}"),
            "env": {
                "SHELL": std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into()),
                "TERM": "xterm-256color"
            }
        });
        writeln!(cast_file, "{}", header)?;

        Ok(Self {
            jsonl_path: jsonl_path.to_path_buf(),
            offset: 0,
            cast_file,
            cast_path,
            final_path,
            is_gif,
            virtual_time: 0.0,
            width,
            height,
            title_written: false,
            session_name: session_name.to_string(),
            edits: 0,
            commands: 0,
            errors: 0,
        })
    }

    /// Read new JSONL lines and emit highlight frames.
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

        // Write title card on first poll
        if !self.title_written {
            self.write_title_card()?;
            self.title_written = true;
        }

        let reader = BufReader::new(&file);
        let mut had_events = false;

        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => break,
            };

            for event in parse_events(&line) {
                if self.emit_highlight(&event)? {
                    had_events = true;
                }
            }
        }

        self.offset = file_len;
        Ok(had_events)
    }

    fn write_frame(&mut self, data: &str) -> std::io::Result<()> {
        let event = serde_json::json!([self.virtual_time, "o", data]);
        writeln!(self.cast_file, "{}", event)
    }

    fn write_title_card(&mut self) -> std::io::Result<()> {
        let w = self.width as usize;
        let h = self.height as usize;
        let sep = "═".repeat(w.min(60));
        let pad_top = "\r\n".repeat(h / 3);
        let name = &self.session_name;

        let card = format!(
            "\x1b[2J\x1b[H{pad_top}\
             \x1b[1;36m  {sep}\x1b[0m\r\n\
             \x1b[1;37m  {name:^width$}\x1b[0m\r\n\
             \x1b[1;36m  {sep}\x1b[0m\r\n\
             \r\n\
             \x1b[90m  Recorded with claudectl\x1b[0m\r\n",
            width = w.min(60)
        );
        self.write_frame(&card)?;
        self.virtual_time += TITLE_HOLD;
        Ok(())
    }

    fn write_stats_header(&mut self) -> std::io::Result<()> {
        let stats = format!(
            "\x1b[2J\x1b[H\x1b[1;36m {} \x1b[0m\x1b[90m│\x1b[0m \
             \x1b[32m{} edits\x1b[0m \x1b[90m│\x1b[0m \
             \x1b[33m{} commands\x1b[0m\
             {}\r\n\
             \x1b[90m{}\x1b[0m\r\n",
            self.session_name,
            self.edits,
            self.commands,
            if self.errors > 0 {
                format!(" \x1b[90m│\x1b[0m \x1b[31m{} errors\x1b[0m", self.errors)
            } else {
                String::new()
            },
            "─"
                .repeat(self.width as usize)
                .chars()
                .take(80)
                .collect::<String>()
        );
        self.write_frame(&stats)?;
        self.virtual_time += 0.3;
        Ok(())
    }

    /// Emit a frame only for highlight-worthy events. Returns true if emitted.
    fn emit_highlight(&mut self, event: &SessionEvent) -> std::io::Result<bool> {
        match event {
            SessionEvent::AssistantText(text) => {
                // Only show brief planning statements, not verbose explanations
                if text.len() < 30 || text.contains("```") {
                    return Ok(false);
                }
                let truncated = if text.len() > MAX_ASSISTANT_TEXT {
                    format!("{}...", &text[..MAX_ASSISTANT_TEXT])
                } else {
                    text.clone()
                };
                self.write_stats_header()?;
                let frame = format!(
                    "\x1b[37m  {}\x1b[0m\r\n\r\n",
                    truncated.replace('\n', "\r\n  ")
                );
                self.write_frame(&frame)?;
                self.virtual_time += FRAME_PACE;
                Ok(true)
            }
            SessionEvent::ToolUse { tool, summary } => {
                if !is_highlight_tool(tool) {
                    return Ok(false);
                }

                // Update tally
                match tool.as_str() {
                    "Edit" | "Write" | "NotebookEdit" => self.edits += 1,
                    "Bash" => self.commands += 1,
                    _ => {}
                }

                let icon = match tool.as_str() {
                    "Edit" => "✏️ ",
                    "Write" => "📝",
                    "Bash" => "⚡",
                    "Agent" => "🤖",
                    _ => "🔧",
                };

                self.write_stats_header()?;
                let frame = format!(
                    "\x1b[1;33m  {icon} {tool}\x1b[0m\r\n\
                     \x1b[37m  {summary}\x1b[0m\r\n\r\n"
                );
                self.write_frame(&frame)?;
                self.virtual_time += FRAME_PACE;
                Ok(true)
            }
            SessionEvent::ToolResult { output, is_error } => {
                if output.is_empty() {
                    return Ok(false);
                }

                if *is_error {
                    self.errors += 1;
                }

                let color = if *is_error { "1;31" } else { "32" };
                let truncated = if output.len() > MAX_BASH_OUTPUT {
                    format!("{}...", &output[..MAX_BASH_OUTPUT])
                } else {
                    output.clone()
                };
                let display = truncated.replace('\n', "\r\n  ");
                let prefix = if *is_error { "  ✗ " } else { "  ✓ " };
                let frame = format!("\x1b[{color}m{prefix}{display}\x1b[0m\r\n\r\n");
                self.write_frame(&frame)?;
                self.virtual_time += RESULT_HOLD;
                Ok(true)
            }
        }
    }

    pub fn finish(&mut self) -> std::io::Result<()> {
        // Final stats card
        let w = self.width as usize;
        let sep = "═".repeat(w.min(60));
        let summary = format!(
            "\x1b[2J\x1b[H\r\n\
             \x1b[1;36m  {sep}\x1b[0m\r\n\
             \x1b[1;37m  {} — complete\x1b[0m\r\n\
             \x1b[1;36m  {sep}\x1b[0m\r\n\r\n\
             \x1b[32m  {} edits\x1b[0m  \
             \x1b[33m{} commands\x1b[0m  \
             \x1b[31m{} errors\x1b[0m\r\n\r\n\
             \x1b[90m  claudectl — github.com/mercurialsolo/claudectl\x1b[0m\r\n",
            self.session_name, self.edits, self.commands, self.errors
        );
        self.write_frame(&summary)?;
        self.virtual_time += TITLE_HOLD;

        self.cast_file.flush()?;

        if self.is_gif {
            return self.convert_to_gif();
        }
        Ok(())
    }

    fn convert_to_gif(&self) -> std::io::Result<()> {
        let cast = self.cast_path.to_string_lossy();
        let gif = self.final_path.to_string_lossy();

        let result = std::process::Command::new("agg")
            .args([cast.as_ref(), gif.as_ref()])
            .output();

        match result {
            Ok(output) if output.status.success() => {
                let _ = std::fs::remove_file(&self.cast_path);
                Ok(())
            }
            _ => {
                let fallback = self.final_path.with_extension("cast");
                if self.cast_path != fallback {
                    std::fs::rename(&self.cast_path, &fallback)?;
                }
                Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!(
                        "agg not found — install with: cargo install agg\n\
                         Saved asciicast to {}",
                        fallback.display()
                    ),
                ))
            }
        }
    }
}

/// Parse a JSONL line into zero or more session events.
fn parse_events(line: &str) -> Vec<SessionEvent> {
    let mut events = Vec::new();

    let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) else {
        return events;
    };

    let msg = match entry.get("message") {
        Some(m) => m,
        None => return events,
    };

    // Real Claude Code JSONL uses message.role ("assistant"/"user"), not message.type
    let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
    let content = msg.get("content").and_then(|c| c.as_array());

    if let Some(blocks) = content {
        for block in blocks {
            let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");

            match block_type {
                "text" if role == "assistant" => {
                    if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                        let trimmed = text.trim();
                        if !trimmed.is_empty() && trimmed.len() > 20 {
                            events.push(SessionEvent::AssistantText(trimmed.to_string()));
                        }
                    }
                }
                "tool_use" if role == "assistant" => {
                    let tool = block
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("unknown")
                        .to_string();

                    let summary = summarize_tool_use(&tool, block.get("input"));
                    events.push(SessionEvent::ToolUse { tool, summary });
                }
                // tool_result comes in "user" role messages (Claude API convention)
                "tool_result" if role == "user" => {
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
            let short = shorten_path(file);
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
            let short = shorten_path(file);
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
            shorten_path(file)
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

fn shorten_path(path: &str) -> String {
    let parts: Vec<&str> = path.rsplit('/').take(2).collect();
    match parts.len() {
        2 => format!("{}/{}", parts[1], parts[0]),
        1 => parts[0].to_string(),
        _ => path.to_string(),
    }
}
