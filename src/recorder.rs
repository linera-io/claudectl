use std::fs::File;
use std::io::{self, Write};
use std::time::Instant;

/// Records terminal output in asciicast v2 format.
/// See: https://docs.asciinema.org/manual/asciicast/v2/
pub struct Recorder {
    file: File,
    start: Instant,
}

impl Recorder {
    /// Create a new recorder writing to the given path.
    pub fn new(path: &str, width: u16, height: u16) -> io::Result<Self> {
        let mut file = File::create(path)?;

        // Write asciicast v2 header
        let header = serde_json::json!({
            "version": 2,
            "width": width,
            "height": height,
            "timestamp": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            "env": {
                "SHELL": "/bin/zsh",
                "TERM": "xterm-256color"
            },
            "title": "claudectl demo"
        });
        writeln!(file, "{}", header)?;

        Ok(Self {
            file,
            start: Instant::now(),
        })
    }

    /// Record a frame of terminal output.
    pub fn record_frame(&mut self, data: &[u8]) -> io::Result<()> {
        let elapsed = self.start.elapsed().as_secs_f64();
        // Escape the data as a JSON string
        let escaped = String::from_utf8_lossy(data);
        let event = serde_json::json!([elapsed, "o", escaped]);
        writeln!(self.file, "{}", event)?;
        Ok(())
    }

    /// Finish recording.
    pub fn finish(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}
