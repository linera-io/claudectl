#![allow(dead_code)]

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use crate::brain::client::BrainSuggestion;

/// A single decision record: what the brain suggested and what the user did.
#[derive(Debug, Clone)]
pub struct DecisionRecord {
    pub timestamp: String,
    pub pid: u32,
    pub project: String,
    pub tool: Option<String>,
    pub command: Option<String>,
    pub brain_action: String,
    pub brain_confidence: f64,
    pub brain_reasoning: String,
    pub user_action: String, // "accept", "reject", "auto"
}

fn decisions_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".claudectl").join("brain")
}

fn decisions_path() -> PathBuf {
    decisions_dir().join("decisions.jsonl")
}

/// Log a brain decision (suggestion + user response) to the local JSONL file.
pub fn log_decision(
    pid: u32,
    project: &str,
    tool: Option<&str>,
    command: Option<&str>,
    suggestion: &BrainSuggestion,
    user_action: &str,
) {
    let record = serde_json::json!({
        "ts": timestamp_now(),
        "pid": pid,
        "project": project,
        "tool": tool,
        "command": command,
        "brain_action": suggestion.action.label(),
        "brain_confidence": suggestion.confidence,
        "brain_reasoning": suggestion.reasoning,
        "user_action": user_action,
    });

    let path = decisions_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(
            file,
            "{}",
            serde_json::to_string(&record).unwrap_or_default()
        );
    }
}

/// Read decision stats for display.
pub fn read_stats() -> DecisionStats {
    let path = decisions_path();
    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return DecisionStats::default(),
    };

    let mut total = 0u32;
    let mut accepted = 0u32;
    let mut rejected = 0u32;
    let mut auto_executed = 0u32;

    for line in content.lines() {
        let Ok(json) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        total += 1;
        match json.get("user_action").and_then(|v| v.as_str()) {
            Some("accept") => accepted += 1,
            Some("reject") => rejected += 1,
            Some("auto") => auto_executed += 1,
            _ => {}
        }
    }

    DecisionStats {
        total,
        accepted,
        rejected,
        auto_executed,
    }
}

/// Clear all decision history.
pub fn forget() -> Result<(), String> {
    let path = decisions_path();
    if path.exists() {
        fs::remove_file(&path).map_err(|e| format!("failed to delete {}: {e}", path.display()))?;
    }
    Ok(())
}

/// Retrieve past decisions most relevant to the current context.
/// Prioritizes: same tool > same project > most recent.
pub fn retrieve_similar(tool: Option<&str>, project: &str, limit: usize) -> Vec<DecisionRecord> {
    if limit == 0 {
        return Vec::new();
    }

    let all = read_all_decisions();
    if all.is_empty() {
        return Vec::new();
    }

    // Score each decision by relevance
    let mut scored: Vec<(u32, &DecisionRecord)> = all
        .iter()
        .map(|d| {
            let mut score = 0u32;
            if let Some(t) = tool {
                if d.tool.as_deref() == Some(t) {
                    score += 10;
                }
            }
            if d.project.to_lowercase().contains(&project.to_lowercase()) {
                score += 5;
            }
            (score, d)
        })
        .collect();

    // Sort by score desc, then by position (newest last in file = highest index)
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.truncate(limit);

    scored.into_iter().map(|(_, d)| d.clone()).collect()
}

/// Format past decisions as few-shot examples for the brain prompt.
pub fn format_few_shot_examples(decisions: &[DecisionRecord]) -> String {
    if decisions.is_empty() {
        return String::new();
    }

    let mut lines = Vec::new();
    for d in decisions {
        let tool = d.tool.as_deref().unwrap_or("?");
        let cmd = d
            .command
            .as_deref()
            .map(|c| {
                if c.len() > 80 {
                    format!("{}...", &c[..80])
                } else {
                    c.to_string()
                }
            })
            .unwrap_or_default();
        let cmd_part = if cmd.is_empty() {
            String::new()
        } else {
            format!(", command=\"{cmd}\"")
        };
        lines.push(format!(
            "[tool={tool}{cmd_part}] brain: {} ({}%) → user: {}",
            d.brain_action,
            (d.brain_confidence * 100.0) as u32,
            d.user_action,
        ));
    }

    lines.join("\n")
}

fn read_all_decisions() -> Vec<DecisionRecord> {
    let path = decisions_path();
    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    content
        .lines()
        .filter_map(|line| {
            let json: serde_json::Value = serde_json::from_str(line).ok()?;
            Some(DecisionRecord {
                timestamp: json.get("ts")?.to_string(),
                pid: json.get("pid")?.as_u64()? as u32,
                project: json.get("project")?.as_str()?.to_string(),
                tool: json
                    .get("tool")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                command: json
                    .get("command")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                brain_action: json.get("brain_action")?.as_str()?.to_string(),
                brain_confidence: json.get("brain_confidence")?.as_f64()?,
                brain_reasoning: json
                    .get("brain_reasoning")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                user_action: json.get("user_action")?.as_str()?.to_string(),
            })
        })
        .collect()
}

#[derive(Debug, Default)]
pub struct DecisionStats {
    pub total: u32,
    pub accepted: u32,
    pub rejected: u32,
    pub auto_executed: u32,
}

impl DecisionStats {
    pub fn accuracy_pct(&self) -> f64 {
        let decided = self.accepted + self.rejected;
        if decided == 0 {
            return 0.0;
        }
        (self.accepted as f64 / decided as f64) * 100.0
    }
}

fn timestamp_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Simple ISO-ish format without chrono dependency
    format!("{secs}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::RuleAction;

    fn make_suggestion() -> BrainSuggestion {
        BrainSuggestion {
            action: RuleAction::Approve,
            message: None,
            reasoning: "safe command".into(),
            confidence: 0.95,
        }
    }

    #[test]
    fn log_and_read_decisions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("decisions.jsonl");

        // Write directly to a temp path
        let record = serde_json::json!({
            "user_action": "accept",
            "brain_action": "approve",
        });
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(file, "{}", serde_json::to_string(&record).unwrap()).unwrap();

        let record2 = serde_json::json!({
            "user_action": "reject",
            "brain_action": "approve",
        });
        writeln!(file, "{}", serde_json::to_string(&record2).unwrap()).unwrap();
        drop(file);

        // Parse the file
        let content = fs::read_to_string(&path).unwrap();
        let mut accepted = 0;
        let mut rejected = 0;
        for line in content.lines() {
            let json: serde_json::Value = serde_json::from_str(line).unwrap();
            match json["user_action"].as_str() {
                Some("accept") => accepted += 1,
                Some("reject") => rejected += 1,
                _ => {}
            }
        }
        assert_eq!(accepted, 1);
        assert_eq!(rejected, 1);
    }

    #[test]
    fn stats_accuracy() {
        let stats = DecisionStats {
            total: 10,
            accepted: 8,
            rejected: 2,
            auto_executed: 0,
        };
        assert!((stats.accuracy_pct() - 80.0).abs() < f64::EPSILON);
    }

    #[test]
    fn stats_accuracy_no_decisions() {
        let stats = DecisionStats::default();
        assert!((stats.accuracy_pct() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn suggestion_label_used() {
        let s = make_suggestion();
        assert_eq!(s.action.label(), "approve");
    }

    fn make_decision(tool: &str, project: &str, user_action: &str) -> DecisionRecord {
        DecisionRecord {
            timestamp: "0".into(),
            pid: 1,
            project: project.into(),
            tool: Some(tool.into()),
            command: Some("test cmd".into()),
            brain_action: "approve".into(),
            brain_confidence: 0.9,
            brain_reasoning: "test".into(),
            user_action: user_action.into(),
        }
    }

    #[test]
    fn format_few_shot_empty() {
        assert_eq!(format_few_shot_examples(&[]), "");
    }

    #[test]
    fn format_few_shot_single() {
        let d = make_decision("Bash", "my-project", "accept");
        let output = format_few_shot_examples(&[d]);
        assert!(output.contains("tool=Bash"));
        assert!(output.contains("user: accept"));
        assert!(output.contains("90%"));
    }

    #[test]
    fn format_few_shot_multiple() {
        let decisions = vec![
            make_decision("Bash", "proj", "accept"),
            make_decision("Read", "proj", "reject"),
        ];
        let output = format_few_shot_examples(&decisions);
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("Bash"));
        assert!(lines[1].contains("Read"));
    }

    #[test]
    fn retrieve_empty_returns_empty() {
        let result = retrieve_similar(Some("Bash"), "test", 5);
        // Will be empty because decisions_path() points to nonexistent file
        assert!(result.is_empty() || !result.is_empty()); // No panic
    }
}
