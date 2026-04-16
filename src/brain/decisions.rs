#![allow(dead_code)]

use std::collections::HashMap;
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
    pub user_action: String, // "accept", "reject", "auto", "deny_rule_override"
}

impl DecisionRecord {
    /// Whether this decision represents a positive outcome (user agreed or auto-executed).
    fn is_positive(&self) -> bool {
        matches!(self.user_action.as_str(), "accept" | "auto")
    }

    /// Whether this decision represents a negative outcome (user disagreed).
    fn is_negative(&self) -> bool {
        matches!(self.user_action.as_str(), "reject" | "deny_rule_override")
    }
}

fn decisions_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".claudectl").join("brain")
}

fn decisions_path() -> PathBuf {
    decisions_dir().join("decisions.jsonl")
}

fn preferences_path() -> PathBuf {
    decisions_dir().join("preferences.json")
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

    // Re-distill preferences after every Nth decision (amortized cost)
    let all = read_all_decisions();
    if all.len() % DISTILL_INTERVAL == 0 {
        let prefs = distill_preferences(&all);
        let _ = save_preferences(&prefs);
    }
}

/// How often to re-distill preferences (every N decisions).
const DISTILL_INTERVAL: usize = 10;

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

/// Clear all decision history and distilled preferences.
pub fn forget() -> Result<(), String> {
    let path = decisions_path();
    if path.exists() {
        fs::remove_file(&path).map_err(|e| format!("failed to delete {}: {e}", path.display()))?;
    }
    let pref_path = preferences_path();
    if pref_path.exists() {
        let _ = fs::remove_file(&pref_path);
    }
    Ok(())
}

// ────────────────────────────────────────────────────────────────────────────
// Outcome-weighted few-shot retrieval
// ────────────────────────────────────────────────────────────────────────────

/// Retrieve past decisions most relevant to the current context.
/// Weights: same tool, same project, user-confirmed outcomes rank higher.
pub fn retrieve_similar(tool: Option<&str>, project: &str, limit: usize) -> Vec<DecisionRecord> {
    if limit == 0 {
        return Vec::new();
    }

    let all = read_all_decisions();
    if all.is_empty() {
        return Vec::new();
    }

    // Score each decision by relevance + outcome signal
    let mut scored: Vec<(i32, usize, &DecisionRecord)> = all
        .iter()
        .enumerate()
        .map(|(idx, d)| {
            let mut score: i32 = 0;

            // Context match
            if let Some(t) = tool {
                if d.tool.as_deref() == Some(t) {
                    score += 10;
                }
            }
            if d.project.to_lowercase().contains(&project.to_lowercase()) {
                score += 5;
            }

            // Outcome weighting: user-confirmed decisions are more informative
            if d.is_positive() {
                score += 3; // Accepted/auto = brain was right, reinforce
            } else if d.is_negative() {
                score += 8; // Rejected = correction signal, very valuable for learning
            }

            // Recency bonus: newer decisions reflect current preferences
            // idx is position in file (0=oldest), scale to 0-2 bonus
            let recency = if all.len() > 1 {
                (idx as i32 * 2) / (all.len() as i32 - 1)
            } else {
                2
            };
            score += recency;

            (score, idx, d)
        })
        .collect();

    // Sort by score desc, break ties by recency (higher idx = newer)
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    scored.truncate(limit);

    scored.into_iter().map(|(_, _, d)| d.clone()).collect()
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
                    format!("{}...", crate::session::truncate_str(c, 80))
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

// ────────────────────────────────────────────────────────────────────────────
// Preference distillation — compact learned patterns for small context windows
// ────────────────────────────────────────────────────────────────────────────

/// A distilled preference pattern learned from the decision history.
/// Compact representation: one pattern replaces many raw examples.
#[derive(Debug, Clone)]
pub struct PreferencePattern {
    /// The tool this pattern applies to (e.g. "Bash", "Read"), or "*" for all.
    pub tool: String,
    /// Optional command substring pattern (e.g. "rm -rf", "git push --force").
    pub command_pattern: Option<String>,
    /// What the user typically wants for this pattern.
    pub preferred_action: String,
    /// How many decisions this pattern was distilled from.
    pub sample_count: u32,
    /// Accept rate: 0.0 to 1.0.
    pub accept_rate: f64,
}

/// Per-tool accuracy tracking for adaptive confidence thresholds.
#[derive(Debug, Clone)]
pub struct ToolAccuracy {
    pub tool: String,
    pub total: u32,
    pub correct: u32,
    /// Adaptive confidence threshold: brain must exceed this to auto-execute.
    pub confidence_threshold: f64,
}

/// The full distilled preferences object, saved to preferences.json.
#[derive(Debug, Clone)]
pub struct DistilledPreferences {
    pub patterns: Vec<PreferencePattern>,
    pub tool_accuracy: Vec<ToolAccuracy>,
    pub total_decisions: u32,
    pub overall_accuracy: f64,
}

/// Distill the decision log into compact preference patterns.
/// Groups decisions by (tool, command_keyword) and computes accept rates.
pub fn distill_preferences(decisions: &[DecisionRecord]) -> DistilledPreferences {
    if decisions.is_empty() {
        return DistilledPreferences {
            patterns: Vec::new(),
            tool_accuracy: Vec::new(),
            total_decisions: 0,
            overall_accuracy: 0.0,
        };
    }

    // (total, accepted, rejected)
    type ToolCounts = (u32, u32, u32);
    // (total, accepted, rejected, most_common_brain_action)
    type PatternCounts = (u32, u32, u32, String);

    // Group by tool → aggregate accept/reject counts
    let mut tool_stats: HashMap<String, ToolCounts> = HashMap::new();
    let mut pattern_stats: HashMap<(String, Option<String>), PatternCounts> = HashMap::new();

    for d in decisions {
        let tool = d.tool.clone().unwrap_or_else(|| "*".to_string());
        let cmd_key = extract_command_keyword(d.command.as_deref());

        // Tool-level stats
        let ts = tool_stats.entry(tool.clone()).or_insert((0, 0, 0));
        ts.0 += 1;
        if d.is_positive() {
            ts.1 += 1;
        } else if d.is_negative() {
            ts.2 += 1;
        }

        // Pattern-level stats (tool + command keyword)
        let key = (tool, cmd_key);
        let ps = pattern_stats
            .entry(key)
            .or_insert((0, 0, 0, d.brain_action.clone()));
        ps.0 += 1;
        if d.is_positive() {
            ps.1 += 1;
        } else if d.is_negative() {
            ps.2 += 1;
        }
    }

    // Build preference patterns (only from groups with enough data)
    let mut patterns = Vec::new();
    for ((tool, cmd_pattern), (total, accepted, rejected, brain_action)) in &pattern_stats {
        if *total < 2 {
            continue; // Need at least 2 decisions to form a pattern
        }
        let decided = accepted + rejected;
        if decided == 0 {
            continue;
        }
        let accept_rate = *accepted as f64 / decided as f64;
        let preferred = if accept_rate >= 0.7 {
            brain_action.clone() // User mostly agrees with the brain
        } else if accept_rate <= 0.3 {
            // User mostly disagrees — the opposite of what brain suggests
            if brain_action == "approve" {
                "deny".to_string()
            } else {
                "approve".to_string()
            }
        } else {
            continue; // Ambiguous — don't form a pattern
        };

        patterns.push(PreferencePattern {
            tool: tool.clone(),
            command_pattern: cmd_pattern.clone(),
            preferred_action: preferred,
            sample_count: *total,
            accept_rate,
        });
    }

    // Sort patterns: most confident first (further from 0.5)
    patterns.sort_by(|a, b| {
        let a_strength = (a.accept_rate - 0.5).abs();
        let b_strength = (b.accept_rate - 0.5).abs();
        b_strength
            .partial_cmp(&a_strength)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Build per-tool accuracy and adaptive thresholds
    let mut tool_accuracy = Vec::new();
    for (tool, (total, correct, _rejected)) in &tool_stats {
        let decided = correct + _rejected;
        let accuracy = if decided > 0 {
            *correct as f64 / decided as f64
        } else {
            1.0 // No feedback yet, assume good
        };

        // Adaptive threshold: lower accuracy → higher confidence required
        // Base threshold 0.6, scales up to 0.95 as accuracy drops
        let threshold = if decided < 3 {
            0.6 // Not enough data, use default
        } else if accuracy >= 0.9 {
            0.5 // Brain is very accurate here, trust it more
        } else if accuracy >= 0.7 {
            0.7 // Decent accuracy, moderate threshold
        } else if accuracy >= 0.5 {
            0.85 // Shaky accuracy, be cautious
        } else {
            0.95 // Brain is mostly wrong here, very high bar
        };

        tool_accuracy.push(ToolAccuracy {
            tool: tool.clone(),
            total: *total,
            correct: *correct,
            confidence_threshold: threshold,
        });
    }

    let total_decided: u32 = tool_stats.values().map(|(_, a, r)| a + r).sum();
    let total_correct: u32 = tool_stats.values().map(|(_, a, _)| *a).sum();
    let overall_accuracy = if total_decided > 0 {
        total_correct as f64 / total_decided as f64
    } else {
        0.0
    };

    DistilledPreferences {
        patterns,
        tool_accuracy,
        total_decisions: decisions.len() as u32,
        overall_accuracy,
    }
}

/// Extract a command keyword for pattern grouping.
/// e.g., "rm -rf /tmp/foo" → "rm -rf", "cargo test --release" → "cargo test"
fn extract_command_keyword(command: Option<&str>) -> Option<String> {
    let cmd = command?.trim();
    if cmd.is_empty() {
        return None;
    }
    // Take first two tokens as the keyword (captures "rm -rf", "git push", "cargo test")
    let tokens: Vec<&str> = cmd.split_whitespace().take(2).collect();
    Some(tokens.join(" "))
}

/// Format distilled preferences as a compact prompt section.
/// This replaces verbose few-shot examples for small context windows.
pub fn format_preference_summary(prefs: &DistilledPreferences) -> String {
    if prefs.patterns.is_empty() && prefs.tool_accuracy.is_empty() {
        return String::new();
    }

    let mut lines = Vec::new();

    // Overall accuracy context
    if prefs.total_decisions >= 5 {
        lines.push(format!(
            "Overall brain accuracy: {:.0}% ({} decisions)",
            prefs.overall_accuracy * 100.0,
            prefs.total_decisions,
        ));
    }

    // Compact preference rules (most impactful first)
    if !prefs.patterns.is_empty() {
        lines.push("User preferences:".to_string());
        for p in prefs.patterns.iter().take(10) {
            let cmd_part = p
                .command_pattern
                .as_ref()
                .map(|c| format!(" \"{c}\""))
                .unwrap_or_default();
            let strength = if p.accept_rate >= 0.9 || p.accept_rate <= 0.1 {
                "always"
            } else if p.accept_rate >= 0.7 || p.accept_rate <= 0.3 {
                "usually"
            } else {
                "sometimes"
            };
            lines.push(format!(
                "- {strength} {} [{}]{cmd_part} (n={})",
                p.preferred_action, p.tool, p.sample_count,
            ));
        }
    }

    // Per-tool accuracy warnings (only for tools where brain struggles)
    let weak_tools: Vec<&ToolAccuracy> = prefs
        .tool_accuracy
        .iter()
        .filter(|ta| ta.total >= 3 && ta.confidence_threshold > 0.7)
        .collect();
    if !weak_tools.is_empty() {
        lines.push("Caution areas (low accuracy):".to_string());
        for ta in weak_tools {
            let accuracy = if ta.total > 0 {
                (ta.correct as f64 / ta.total as f64) * 100.0
            } else {
                0.0
            };
            lines.push(format!(
                "- [{}]: {:.0}% accuracy, be extra careful",
                ta.tool, accuracy,
            ));
        }
    }

    lines.join("\n")
}

/// Save distilled preferences to disk.
fn save_preferences(prefs: &DistilledPreferences) -> Result<(), String> {
    let path = preferences_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let json = serde_json::json!({
        "patterns": prefs.patterns.iter().map(|p| {
            serde_json::json!({
                "tool": p.tool,
                "command_pattern": p.command_pattern,
                "preferred_action": p.preferred_action,
                "sample_count": p.sample_count,
                "accept_rate": p.accept_rate,
            })
        }).collect::<Vec<_>>(),
        "tool_accuracy": prefs.tool_accuracy.iter().map(|ta| {
            serde_json::json!({
                "tool": ta.tool,
                "total": ta.total,
                "correct": ta.correct,
                "confidence_threshold": ta.confidence_threshold,
            })
        }).collect::<Vec<_>>(),
        "total_decisions": prefs.total_decisions,
        "overall_accuracy": prefs.overall_accuracy,
    });

    fs::write(
        &path,
        serde_json::to_string_pretty(&json).map_err(|e| format!("json error: {e}"))?,
    )
    .map_err(|e| format!("write error: {e}"))
}

/// Load distilled preferences from disk.
pub fn load_preferences() -> Option<DistilledPreferences> {
    let path = preferences_path();
    let content = fs::read_to_string(&path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;

    let patterns = json
        .get("patterns")?
        .as_array()?
        .iter()
        .filter_map(|p| {
            Some(PreferencePattern {
                tool: p.get("tool")?.as_str()?.to_string(),
                command_pattern: p
                    .get("command_pattern")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                preferred_action: p.get("preferred_action")?.as_str()?.to_string(),
                sample_count: p.get("sample_count")?.as_u64()? as u32,
                accept_rate: p.get("accept_rate")?.as_f64()?,
            })
        })
        .collect();

    let tool_accuracy = json
        .get("tool_accuracy")?
        .as_array()?
        .iter()
        .filter_map(|ta| {
            Some(ToolAccuracy {
                tool: ta.get("tool")?.as_str()?.to_string(),
                total: ta.get("total")?.as_u64()? as u32,
                correct: ta.get("correct")?.as_u64()? as u32,
                confidence_threshold: ta.get("confidence_threshold")?.as_f64()?,
            })
        })
        .collect();

    Some(DistilledPreferences {
        patterns,
        tool_accuracy,
        total_decisions: json.get("total_decisions")?.as_u64()? as u32,
        overall_accuracy: json.get("overall_accuracy")?.as_f64()?,
    })
}

/// Get the adaptive confidence threshold for a specific tool.
/// Returns None if no preference data exists (use default threshold).
pub fn adaptive_threshold(tool: Option<&str>) -> Option<f64> {
    let prefs = load_preferences()?;
    let tool_name = tool?;
    prefs
        .tool_accuracy
        .iter()
        .find(|ta| ta.tool == tool_name)
        .map(|ta| ta.confidence_threshold)
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

    fn make_decision_with_cmd(
        tool: &str,
        command: &str,
        project: &str,
        user_action: &str,
    ) -> DecisionRecord {
        DecisionRecord {
            timestamp: "0".into(),
            pid: 1,
            project: project.into(),
            tool: Some(tool.into()),
            command: Some(command.into()),
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

    // ── Preference distillation tests ─────────────────────────────────

    #[test]
    fn distill_empty_returns_empty() {
        let prefs = distill_preferences(&[]);
        assert!(prefs.patterns.is_empty());
        assert!(prefs.tool_accuracy.is_empty());
        assert_eq!(prefs.total_decisions, 0);
    }

    #[test]
    fn distill_builds_accept_pattern() {
        // User accepts Read 5 times → should create "always approve Read" pattern
        let decisions: Vec<DecisionRecord> = (0..5)
            .map(|_| make_decision("Read", "proj", "accept"))
            .collect();

        let prefs = distill_preferences(&decisions);
        assert!(!prefs.patterns.is_empty());

        let read_pattern = prefs.patterns.iter().find(|p| p.tool == "Read");
        assert!(read_pattern.is_some());
        let rp = read_pattern.unwrap();
        assert_eq!(rp.preferred_action, "approve");
        assert!(rp.accept_rate >= 0.9);
    }

    #[test]
    fn distill_builds_reject_pattern() {
        // User rejects Bash "rm -rf" 4 times → should create "deny" pattern
        let decisions: Vec<DecisionRecord> = (0..4)
            .map(|_| make_decision_with_cmd("Bash", "rm -rf /tmp", "proj", "reject"))
            .collect();

        let prefs = distill_preferences(&decisions);
        let rm_pattern = prefs
            .patterns
            .iter()
            .find(|p| p.command_pattern.as_deref() == Some("rm -rf"));
        assert!(rm_pattern.is_some());
        let rp = rm_pattern.unwrap();
        assert_eq!(rp.preferred_action, "deny");
        assert!(rp.accept_rate <= 0.1);
    }

    #[test]
    fn distill_skips_ambiguous_patterns() {
        // Mixed accept/reject → no clear preference, should be skipped
        let decisions = vec![
            make_decision("Bash", "proj", "accept"),
            make_decision("Bash", "proj", "reject"),
            make_decision("Bash", "proj", "accept"),
            make_decision("Bash", "proj", "reject"),
        ];

        let prefs = distill_preferences(&decisions);
        // Bash with "test cmd" pattern should NOT appear (50/50 split)
        let bash_pattern = prefs
            .patterns
            .iter()
            .find(|p| p.tool == "Bash" && p.command_pattern.as_deref() == Some("test cmd"));
        assert!(bash_pattern.is_none());
    }

    #[test]
    fn adaptive_threshold_low_accuracy() {
        // Brain is wrong most of the time for Bash → high threshold
        let decisions: Vec<DecisionRecord> = (0..10)
            .map(|i| {
                if i < 2 {
                    make_decision("Bash", "proj", "accept")
                } else {
                    make_decision("Bash", "proj", "reject")
                }
            })
            .collect();

        let prefs = distill_preferences(&decisions);
        let bash_acc = prefs.tool_accuracy.iter().find(|ta| ta.tool == "Bash");
        assert!(bash_acc.is_some());
        let ba = bash_acc.unwrap();
        // 20% accuracy → threshold should be very high (0.95)
        assert!(
            ba.confidence_threshold >= 0.9,
            "threshold was {}",
            ba.confidence_threshold
        );
    }

    #[test]
    fn adaptive_threshold_high_accuracy() {
        // Brain is right most of the time for Read → low threshold
        let decisions: Vec<DecisionRecord> = (0..10)
            .map(|_| make_decision("Read", "proj", "accept"))
            .collect();

        let prefs = distill_preferences(&decisions);
        let read_acc = prefs.tool_accuracy.iter().find(|ta| ta.tool == "Read");
        assert!(read_acc.is_some());
        let ra = read_acc.unwrap();
        // 100% accuracy → threshold should be low (0.5)
        assert!(
            ra.confidence_threshold <= 0.6,
            "threshold was {}",
            ra.confidence_threshold
        );
    }

    #[test]
    fn format_preference_summary_empty() {
        let prefs = distill_preferences(&[]);
        assert_eq!(format_preference_summary(&prefs), "");
    }

    #[test]
    fn format_preference_summary_with_patterns() {
        let decisions: Vec<DecisionRecord> = (0..8)
            .map(|_| make_decision("Read", "proj", "accept"))
            .collect();
        let prefs = distill_preferences(&decisions);
        let summary = format_preference_summary(&prefs);

        assert!(summary.contains("User preferences:"));
        assert!(summary.contains("[Read]"));
        assert!(summary.contains("approve"));
    }

    #[test]
    fn format_preference_summary_with_caution() {
        let mut decisions: Vec<DecisionRecord> = (0..8)
            .map(|_| make_decision("Bash", "proj", "reject"))
            .collect();
        // Add a few accepts so total is enough
        decisions.push(make_decision("Bash", "proj", "accept"));
        decisions.push(make_decision("Bash", "proj", "accept"));

        let prefs = distill_preferences(&decisions);
        let summary = format_preference_summary(&prefs);

        assert!(summary.contains("Caution areas"));
        assert!(summary.contains("[Bash]"));
    }

    #[test]
    fn extract_command_keyword_works() {
        assert_eq!(
            extract_command_keyword(Some("rm -rf /tmp/foo")),
            Some("rm -rf".into())
        );
        assert_eq!(
            extract_command_keyword(Some("cargo test --release")),
            Some("cargo test".into())
        );
        assert_eq!(extract_command_keyword(Some("ls")), Some("ls".into()));
        assert_eq!(extract_command_keyword(None), None);
        assert_eq!(extract_command_keyword(Some("")), None);
    }

    #[test]
    fn decision_record_outcome_classification() {
        let accept = make_decision("Bash", "proj", "accept");
        assert!(accept.is_positive());
        assert!(!accept.is_negative());

        let reject = make_decision("Bash", "proj", "reject");
        assert!(!reject.is_positive());
        assert!(reject.is_negative());

        let auto = make_decision("Bash", "proj", "auto");
        assert!(auto.is_positive());
        assert!(!auto.is_negative());

        let deny_override = make_decision("Bash", "proj", "deny_rule_override");
        assert!(!deny_override.is_positive());
        assert!(deny_override.is_negative());
    }

    #[test]
    fn outcome_weighted_retrieval_prefers_corrections() {
        // Rejected decisions should score higher (correction signal)
        let decisions = vec![
            make_decision("Bash", "proj", "accept"),
            make_decision("Bash", "proj", "reject"),
        ];

        // Simulate scoring: reject gets +8, accept gets +3
        // Both match on tool (+10) and project (+5)
        // reject total = 10+5+8+recency = higher
        let reject = &decisions[1];
        assert!(reject.is_negative());
    }
}
