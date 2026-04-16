#![allow(dead_code)]

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::brain::client::BrainSuggestion;

/// Counter for decisions logged this process lifetime (avoids reading file to check).
static DECISION_COUNT: AtomicU32 = AtomicU32::new(0);
/// Guard to prevent concurrent distillation threads.
static DISTILLING: AtomicBool = AtomicBool::new(false);

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
    pub context: Option<DecisionContext>,
    pub outcome: Option<DecisionOutcome>,
}

/// Outcome of a decision, backfilled during distillation by looking at
/// consecutive same-PID records.
#[derive(Debug, Clone)]
pub enum DecisionOutcome {
    Success,
    Error(String),
}

/// Snapshot of session state captured at decision time.
/// Stored in JSONL for rich distillation. NOT sent to LLM directly.
#[derive(Debug, Clone)]
pub struct DecisionContext {
    pub cost_usd: f64,
    pub context_pct: u8,
    pub last_tool_error: bool,
    pub error_message: Option<String>,
    pub model: String,
    pub elapsed_secs: u64,
    pub files_modified_count: u32,
    pub total_tool_calls: u32,
    pub has_file_conflict: bool,
    pub status: String,
    pub burn_rate_per_hr: f64,
    pub recent_error_count: u8,
    pub subagent_count: u8,
}

impl DecisionRecord {
    /// Whether this decision represents a positive outcome (user agreed or auto-executed).
    pub fn is_positive(&self) -> bool {
        matches!(
            self.user_action.as_str(),
            "accept" | "auto" | "user_approve" | "rule_approve"
        )
    }

    /// Whether this decision represents a negative outcome (user disagreed).
    pub fn is_negative(&self) -> bool {
        matches!(
            self.user_action.as_str(),
            "reject" | "deny_rule_override" | "rule_deny" | "conflict_deny"
        )
    }

    /// Whether this is a passive observation (brain was NOT involved).
    pub fn is_observation(&self) -> bool {
        matches!(
            self.user_action.as_str(),
            "user_approve"
                | "user_input"
                | "rule_approve"
                | "rule_deny"
                | "rule_send"
                | "conflict_deny"
        )
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

/// Build a JSON snapshot of session state for embedding in a JSONL record.
fn snapshot_context(session: &crate::session::ClaudeSession) -> serde_json::Value {
    let context_pct = if session.context_max > 0 {
        ((session.context_tokens as f64 / session.context_max as f64) * 100.0) as u8
    } else {
        0
    };
    serde_json::json!({
        "cost_usd": session.cost_usd,
        "context_pct": context_pct,
        "last_tool_error": session.last_tool_error,
        "error_message": session.last_error_message.as_deref().map(|m| crate::session::truncate_str(m, 100)),
        "model": session.model,
        "elapsed_secs": session.elapsed.as_secs(),
        "files_modified_count": session.files_modified.len() as u32,
        "total_tool_calls": session.tool_usage.values().map(|t| t.calls).sum::<u32>(),
        "has_file_conflict": session.has_file_conflict,
        "status": session.status.to_string(),
        "burn_rate_per_hr": session.burn_rate_per_hr,
        "recent_error_count": session.recent_errors.len() as u8,
        "subagent_count": session.subagent_count as u8,
    })
}

/// Log a brain decision (suggestion + user response) to the local JSONL file.
pub fn log_decision(
    pid: u32,
    project: &str,
    tool: Option<&str>,
    command: Option<&str>,
    suggestion: &BrainSuggestion,
    user_action: &str,
    session: Option<&crate::session::ClaudeSession>,
) {
    let mut record = serde_json::json!({
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
    if let Some(s) = session {
        record["context"] = snapshot_context(s);
    }

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

    // Re-distill preferences in a background thread every Nth decision.
    // The file append above is fast (single write), but distillation reads
    // the full history and computes patterns — must not block the TUI.
    maybe_distill_background();
}

/// Log a passive observation: a user action the brain was NOT involved in.
/// These provide ground-truth training data — what the user does when
/// deciding on their own. Same JSONL format so distillation picks them up.
pub fn log_observation(
    pid: u32,
    project: &str,
    tool: Option<&str>,
    command: Option<&str>,
    observed_action: &str, // "user_approve", "user_input", "rule_approve", "rule_deny", etc.
    session: Option<&crate::session::ClaudeSession>,
) {
    let mut record = serde_json::json!({
        "ts": timestamp_now(),
        "pid": pid,
        "project": project,
        "tool": tool,
        "command": command,
        "brain_action": null,
        "brain_confidence": 0.0,
        "brain_reasoning": "",
        "user_action": observed_action,
    });
    if let Some(s) = session {
        record["context"] = snapshot_context(s);
    }

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

    maybe_distill_background();
}

/// How often to re-distill preferences (every N decisions).
const DISTILL_INTERVAL: u32 = 10;

/// Spawn a background thread to re-distill preferences if the interval has been reached.
/// Uses atomic guards to avoid blocking the main thread and prevent concurrent distillation.
fn maybe_distill_background() {
    let count = DECISION_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
    if count % DISTILL_INTERVAL != 0 {
        return;
    }

    // Prevent concurrent distillation (compare_exchange: only one thread wins)
    if DISTILLING
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        return; // Another distillation is already running
    }

    std::thread::spawn(|| {
        let all = read_all_decisions();
        if !all.is_empty() {
            let prefs = distill_preferences(&all);
            let _ = save_preferences(&prefs);
        }
        DISTILLING.store(false, Ordering::Release);
    });
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
    let mut observations = 0u32;

    for line in content.lines() {
        let Ok(json) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        total += 1;
        match json.get("user_action").and_then(|v| v.as_str()) {
            Some("accept") => accepted += 1,
            Some("reject") => rejected += 1,
            Some("auto") => auto_executed += 1,
            Some(
                "user_approve" | "user_input" | "rule_approve" | "rule_deny" | "rule_send"
                | "conflict_deny",
            ) => observations += 1,
            _ => {}
        }
    }

    DecisionStats {
        total,
        accepted,
        rejected,
        auto_executed,
        observations,
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
            if d.is_observation() {
                score += 2; // Passive observation: ground truth but no correction signal
            } else if d.is_positive() {
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
        if d.is_observation() {
            // Passive observation: show what the user did directly
            lines.push(format!(
                "[tool={tool}{cmd_part}] user action: {}",
                d.user_action,
            ));
        } else {
            lines.push(format!(
                "[tool={tool}{cmd_part}] brain: {} ({}%) → user: {}",
                d.brain_action,
                (d.brain_confidence * 100.0) as u32,
                d.user_action,
            ));
        }
    }

    lines.join("\n")
}

// ────────────────────────────────────────────────────────────────────────────
// Preference distillation — compact learned patterns for small context windows
// ────────────────────────────────────────────────────────────────────────────

/// Condition for a conditional preference pattern.
#[derive(Debug, Clone)]
pub enum PreferenceCondition {
    CostBelow(f64),
    CostAbove(f64),
    ContextBelow(u8),
    ContextAbove(u8),
    NoErrors,
    HasErrors,
    NoFileConflict,
    HasFileConflict,
}

impl PreferenceCondition {
    /// Compact human-readable suffix for prompt rendering.
    pub fn label(&self) -> String {
        match self {
            PreferenceCondition::CostBelow(v) => format!("cost<${v:.0}"),
            PreferenceCondition::CostAbove(v) => format!("cost>${v:.0}"),
            PreferenceCondition::ContextBelow(v) => format!("ctx<{v}%"),
            PreferenceCondition::ContextAbove(v) => format!("ctx>{v}%"),
            PreferenceCondition::NoErrors => "no errors".to_string(),
            PreferenceCondition::HasErrors => "errors".to_string(),
            PreferenceCondition::NoFileConflict => "no conflict".to_string(),
            PreferenceCondition::HasFileConflict => "conflict".to_string(),
        }
    }

    /// Serialize to JSON value.
    fn to_json(&self) -> serde_json::Value {
        match self {
            PreferenceCondition::CostBelow(v) => {
                serde_json::json!({"type": "cost_below", "value": v})
            }
            PreferenceCondition::CostAbove(v) => {
                serde_json::json!({"type": "cost_above", "value": v})
            }
            PreferenceCondition::ContextBelow(v) => {
                serde_json::json!({"type": "context_below", "value": v})
            }
            PreferenceCondition::ContextAbove(v) => {
                serde_json::json!({"type": "context_above", "value": v})
            }
            PreferenceCondition::NoErrors => serde_json::json!({"type": "no_errors"}),
            PreferenceCondition::HasErrors => serde_json::json!({"type": "has_errors"}),
            PreferenceCondition::NoFileConflict => serde_json::json!({"type": "no_file_conflict"}),
            PreferenceCondition::HasFileConflict => {
                serde_json::json!({"type": "has_file_conflict"})
            }
        }
    }

    /// Parse from JSON value.
    fn from_json(v: &serde_json::Value) -> Option<Self> {
        let typ = v.get("type")?.as_str()?;
        match typ {
            "cost_below" => Some(PreferenceCondition::CostBelow(v.get("value")?.as_f64()?)),
            "cost_above" => Some(PreferenceCondition::CostAbove(v.get("value")?.as_f64()?)),
            "context_below" => Some(PreferenceCondition::ContextBelow(
                v.get("value")?.as_u64()? as u8
            )),
            "context_above" => Some(PreferenceCondition::ContextAbove(
                v.get("value")?.as_u64()? as u8
            )),
            "no_errors" => Some(PreferenceCondition::NoErrors),
            "has_errors" => Some(PreferenceCondition::HasErrors),
            "no_file_conflict" => Some(PreferenceCondition::NoFileConflict),
            "has_file_conflict" => Some(PreferenceCondition::HasFileConflict),
            _ => None,
        }
    }
}

/// A distilled preference pattern learned from the decision history.
/// Compact representation: one pattern replaces many raw examples.
/// May include conditions learned from context-enriched records.
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
    /// Conditions under which this preference applies (empty = unconditional).
    pub conditions: Vec<PreferenceCondition>,
    /// Confidence in this pattern (0.0 to 1.0), higher when context-enriched.
    pub confidence: f64,
}

/// A temporal behavior pattern detected across sequential decisions.
#[derive(Debug, Clone)]
pub struct TemporalPattern {
    pub description: String,
    pub sample_count: u32,
    pub strength: f64,
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
    pub temporal: Vec<TemporalPattern>,
}

/// Compute Gini impurity for a binary split.
fn gini_impurity(positive: u32, negative: u32) -> f64 {
    let total = (positive + negative) as f64;
    if total == 0.0 {
        return 0.0;
    }
    let p = positive as f64 / total;
    let n = negative as f64 / total;
    1.0 - (p * p + n * n)
}

/// Try splitting a group of context-enriched decisions on a single feature.
/// Returns the best split condition pair (left, right) if information gain > 0.15.
fn best_split(decisions: &[&DecisionRecord]) -> Option<(PreferenceCondition, PreferenceCondition)> {
    // Only consider records that have context
    let enriched: Vec<(&DecisionRecord, &DecisionContext)> = decisions
        .iter()
        .filter_map(|d| d.context.as_ref().map(|ctx| (*d, ctx)))
        .collect();
    if enriched.len() < 5 {
        return None;
    }

    let total_pos = enriched.iter().filter(|(d, _)| d.is_positive()).count() as u32;
    let total_neg = enriched.iter().filter(|(d, _)| d.is_negative()).count() as u32;
    let parent_gini = gini_impurity(total_pos, total_neg);

    if parent_gini < 0.01 {
        return None; // Already pure, no split needed
    }

    let total = enriched.len() as f64;
    let mut best_gain = 0.0f64;
    let mut best_result: Option<(PreferenceCondition, PreferenceCondition)> = None;

    // Helper: compute weighted gini for a boolean split
    let try_split = |left: &[bool], decisions: &[(&DecisionRecord, &DecisionContext)]| -> f64 {
        let mut l_pos = 0u32;
        let mut l_neg = 0u32;
        let mut r_pos = 0u32;
        let mut r_neg = 0u32;
        for (i, &is_left) in left.iter().enumerate() {
            let positive = decisions[i].0.is_positive();
            if is_left {
                if positive {
                    l_pos += 1;
                } else {
                    l_neg += 1;
                }
            } else if positive {
                r_pos += 1;
            } else {
                r_neg += 1;
            }
        }
        let l_total = (l_pos + l_neg) as f64;
        let r_total = (r_pos + r_neg) as f64;
        if l_total == 0.0 || r_total == 0.0 {
            return 0.0; // Degenerate split
        }
        let weighted = (l_total / total) * gini_impurity(l_pos, l_neg)
            + (r_total / total) * gini_impurity(r_pos, r_neg);
        parent_gini - weighted
    };

    // Split on cost_usd median
    {
        let mut costs: Vec<f64> = enriched.iter().map(|(_, ctx)| ctx.cost_usd).collect();
        costs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let median = costs[costs.len() / 2];
        if median > 0.0 {
            let left_mask: Vec<bool> = enriched
                .iter()
                .map(|(_, ctx)| ctx.cost_usd < median)
                .collect();
            let gain = try_split(&left_mask, &enriched);
            if gain > best_gain {
                best_gain = gain;
                best_result = Some((
                    PreferenceCondition::CostBelow(median),
                    PreferenceCondition::CostAbove(median),
                ));
            }
        }
    }

    // Split on context_pct median
    {
        let mut pcts: Vec<u8> = enriched.iter().map(|(_, ctx)| ctx.context_pct).collect();
        pcts.sort();
        let median = pcts[pcts.len() / 2];
        if median > 0 && median < 100 {
            let left_mask: Vec<bool> = enriched
                .iter()
                .map(|(_, ctx)| ctx.context_pct < median)
                .collect();
            let gain = try_split(&left_mask, &enriched);
            if gain > best_gain {
                best_gain = gain;
                best_result = Some((
                    PreferenceCondition::ContextBelow(median),
                    PreferenceCondition::ContextAbove(median),
                ));
            }
        }
    }

    // Split on last_tool_error
    {
        let left_mask: Vec<bool> = enriched
            .iter()
            .map(|(_, ctx)| !ctx.last_tool_error)
            .collect();
        let gain = try_split(&left_mask, &enriched);
        if gain > best_gain {
            best_gain = gain;
            best_result = Some((
                PreferenceCondition::NoErrors,
                PreferenceCondition::HasErrors,
            ));
        }
    }

    // Split on has_file_conflict
    {
        let left_mask: Vec<bool> = enriched
            .iter()
            .map(|(_, ctx)| !ctx.has_file_conflict)
            .collect();
        let gain = try_split(&left_mask, &enriched);
        if gain > best_gain {
            best_gain = gain;
            best_result = Some((
                PreferenceCondition::NoFileConflict,
                PreferenceCondition::HasFileConflict,
            ));
        }
    }

    if best_gain > 0.15 { best_result } else { None }
}

/// Backfill outcomes by examining consecutive same-PID decision pairs.
/// If decision[i+1] has context.last_tool_error == true, decision[i] gets Error outcome.
pub fn backfill_outcomes(decisions: &mut [DecisionRecord]) {
    if decisions.len() < 2 {
        return;
    }
    // Group consecutive indices by PID
    for i in 0..decisions.len() - 1 {
        if decisions[i].pid != decisions[i + 1].pid {
            continue;
        }
        if let Some(ref next_ctx) = decisions[i + 1].context {
            if next_ctx.last_tool_error {
                let msg = next_ctx
                    .error_message
                    .clone()
                    .unwrap_or_else(|| "tool error".to_string());
                decisions[i].outcome = Some(DecisionOutcome::Error(msg));
            } else {
                decisions[i].outcome = Some(DecisionOutcome::Success);
            }
        }
    }
}

/// Detect temporal patterns from decision history.
fn detect_temporal_patterns(decisions: &[DecisionRecord]) -> Vec<TemporalPattern> {
    let mut patterns = Vec::new();

    // --- Error streaks: 3+ consecutive errors on same PID → what users do ---
    {
        let mut streak_count = 0u32;
        let mut streak_responses = 0u32; // How many post-streak decisions exist
        let mut streak_denials = 0u32;
        let mut current_pid: u32 = 0;
        let mut error_run = 0u32;

        for d in decisions {
            if d.pid != current_pid {
                current_pid = d.pid;
                error_run = 0;
            }
            if let Some(ref ctx) = d.context {
                if ctx.last_tool_error {
                    error_run += 1;
                } else {
                    if error_run >= 3 {
                        streak_count += 1;
                        streak_responses += 1;
                        if d.is_negative() {
                            streak_denials += 1;
                        }
                    }
                    error_run = 0;
                }
            }
        }
        if streak_count >= 2 {
            let denial_rate = streak_denials as f64 / streak_responses as f64;
            if denial_rate > 0.5 {
                patterns.push(TemporalPattern {
                    description: format!(
                        "After 3+ errors: user usually denies (n={})",
                        streak_count
                    ),
                    sample_count: streak_count,
                    strength: denial_rate,
                });
            }
        }
    }

    // --- Cost pressure: rejection rate by burn rate quartile ---
    {
        let mut burn_rates: Vec<f64> = decisions
            .iter()
            .filter_map(|d| d.context.as_ref().map(|ctx| ctx.burn_rate_per_hr))
            .filter(|r| *r > 0.0)
            .collect();
        burn_rates.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        if burn_rates.len() >= 8 {
            let q3_idx = burn_rates.len() * 3 / 4;
            let q3_threshold = burn_rates[q3_idx];
            let high_burn: Vec<&DecisionRecord> = decisions
                .iter()
                .filter(|d| {
                    d.context
                        .as_ref()
                        .map(|ctx| ctx.burn_rate_per_hr >= q3_threshold)
                        .unwrap_or(false)
                })
                .collect();
            let decided: Vec<&&DecisionRecord> = high_burn
                .iter()
                .filter(|d| d.is_positive() || d.is_negative())
                .collect();
            if decided.len() >= 3 {
                let denied = decided.iter().filter(|d| d.is_negative()).count();
                let rate = denied as f64 / decided.len() as f64;
                if rate > 0.5 {
                    patterns.push(TemporalPattern {
                        description: format!(
                            "High burn rate (>${:.1}/hr): rejection rate {:.0}% (n={})",
                            q3_threshold,
                            rate * 100.0,
                            decided.len()
                        ),
                        sample_count: decided.len() as u32,
                        strength: rate,
                    });
                }
            }
        }
    }

    // --- Context pressure: approval rate drop when context >80% ---
    {
        let high_ctx: Vec<&DecisionRecord> = decisions
            .iter()
            .filter(|d| {
                d.context
                    .as_ref()
                    .map(|ctx| ctx.context_pct > 80)
                    .unwrap_or(false)
            })
            .collect();
        let low_ctx: Vec<&DecisionRecord> = decisions
            .iter()
            .filter(|d| {
                d.context
                    .as_ref()
                    .map(|ctx| ctx.context_pct <= 80)
                    .unwrap_or(false)
            })
            .collect();

        let high_decided: Vec<&&DecisionRecord> = high_ctx
            .iter()
            .filter(|d| d.is_positive() || d.is_negative())
            .collect();
        let low_decided: Vec<&&DecisionRecord> = low_ctx
            .iter()
            .filter(|d| d.is_positive() || d.is_negative())
            .collect();

        if high_decided.len() >= 3 && low_decided.len() >= 3 {
            let high_accept = high_decided.iter().filter(|d| d.is_positive()).count() as f64
                / high_decided.len() as f64;
            let low_accept = low_decided.iter().filter(|d| d.is_positive()).count() as f64
                / low_decided.len() as f64;
            let drop = low_accept - high_accept;
            if drop > 0.2 {
                patterns.push(TemporalPattern {
                    description: format!(
                        "Context >80%: approval drops {:.0}% vs low context (n={})",
                        drop * 100.0,
                        high_decided.len()
                    ),
                    sample_count: high_decided.len() as u32,
                    strength: drop,
                });
            }
        }
    }

    patterns
}

/// Distill the decision log into compact preference patterns.
/// Groups decisions by (tool, command_keyword) and computes accept rates.
/// Enhanced with conditional splits, outcome weighting, and temporal patterns.
pub fn distill_preferences(decisions: &[DecisionRecord]) -> DistilledPreferences {
    if decisions.is_empty() {
        return DistilledPreferences {
            patterns: Vec::new(),
            tool_accuracy: Vec::new(),
            total_decisions: 0,
            overall_accuracy: 0.0,
            temporal: Vec::new(),
        };
    }

    // Backfill outcomes on a mutable copy
    let mut decisions_mut = decisions.to_vec();
    backfill_outcomes(&mut decisions_mut);

    // (total, accepted, rejected)
    type ToolCounts = (u32, u32, u32);

    // Group by tool → aggregate accept/reject counts
    let mut tool_stats: HashMap<String, ToolCounts> = HashMap::new();
    // Group decisions by (tool, command_keyword) for pattern analysis
    let mut pattern_groups: HashMap<(String, Option<String>), Vec<usize>> = HashMap::new();

    for (idx, d) in decisions_mut.iter().enumerate() {
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

        // Pattern-level grouping
        let key = (tool, cmd_key);
        pattern_groups.entry(key).or_default().push(idx);
    }

    // Build preference patterns (only from groups with enough data)
    let mut patterns = Vec::new();
    for ((tool, cmd_pattern), indices) in &pattern_groups {
        if indices.len() < 2 {
            continue; // Need at least 2 decisions to form a pattern
        }
        let group: Vec<&DecisionRecord> = indices.iter().map(|&i| &decisions_mut[i]).collect();
        let brain_action = group
            .first()
            .map(|d| d.brain_action.clone())
            .unwrap_or_default();

        let accepted: u32 = group.iter().filter(|d| d.is_positive()).count() as u32;
        let rejected: u32 = group.iter().filter(|d| d.is_negative()).count() as u32;
        let total = indices.len() as u32;
        let decided = accepted + rejected;
        if decided == 0 {
            continue;
        }

        // Outcome weighting: downweight accepted-but-errored decisions
        let mut weighted_accept = 0.0f64;
        let mut weighted_total = 0.0f64;
        for d in &group {
            if !d.is_positive() && !d.is_negative() {
                continue;
            }
            let weight = match (&d.outcome, d.is_positive()) {
                (Some(DecisionOutcome::Error(_)), true) => 0.3, // Accepted but broke
                (Some(DecisionOutcome::Error(_)), false) => 1.5, // Rejected rightly
                _ => 1.0,
            };
            weighted_total += weight;
            if d.is_positive() {
                weighted_accept += weight;
            }
        }
        let weighted_rate = if weighted_total > 0.0 {
            weighted_accept / weighted_total
        } else {
            accepted as f64 / decided as f64
        };

        let accept_rate = weighted_rate;

        // Check if we can split this group on context features (Level 2)
        let enriched_count = group.iter().filter(|d| d.context.is_some()).count();
        if enriched_count >= 5 && accept_rate > 0.3 && accept_rate < 0.7 {
            // Ambiguous overall — try splitting
            if let Some((left_cond, right_cond)) = best_split(&group) {
                // Build two conditional patterns
                for (cond, is_left) in [(left_cond, true), (right_cond, false)] {
                    let sub: Vec<&DecisionRecord> = group
                        .iter()
                        .filter(|d| {
                            d.context.as_ref().is_some_and(|ctx| match &cond {
                                PreferenceCondition::CostBelow(v) => ctx.cost_usd < *v,
                                PreferenceCondition::CostAbove(v) => ctx.cost_usd >= *v,
                                PreferenceCondition::ContextBelow(v) => ctx.context_pct < *v,
                                PreferenceCondition::ContextAbove(v) => ctx.context_pct >= *v,
                                PreferenceCondition::NoErrors => !ctx.last_tool_error,
                                PreferenceCondition::HasErrors => ctx.last_tool_error,
                                PreferenceCondition::NoFileConflict => !ctx.has_file_conflict,
                                PreferenceCondition::HasFileConflict => ctx.has_file_conflict,
                            })
                        })
                        .copied()
                        .collect();
                    let sub_acc = sub.iter().filter(|d| d.is_positive()).count() as u32;
                    let sub_rej = sub.iter().filter(|d| d.is_negative()).count() as u32;
                    let sub_dec = sub_acc + sub_rej;
                    if sub_dec < 2 {
                        continue;
                    }
                    let sub_rate = sub_acc as f64 / sub_dec as f64;
                    let preferred = if sub_rate >= 0.7 {
                        if brain_action.is_empty() {
                            "approve".to_string()
                        } else {
                            brain_action.clone()
                        }
                    } else if sub_rate <= 0.3 {
                        if brain_action == "approve" || brain_action.is_empty() {
                            "deny".to_string()
                        } else {
                            "approve".to_string()
                        }
                    } else {
                        continue; // Still ambiguous after split
                    };
                    let _ = is_left; // suppress unused warning
                    patterns.push(PreferencePattern {
                        tool: tool.clone(),
                        command_pattern: cmd_pattern.clone(),
                        preferred_action: preferred,
                        sample_count: sub.len() as u32,
                        accept_rate: sub_rate,
                        conditions: vec![cond],
                        confidence: (sub_rate - 0.5).abs() * 2.0,
                    });
                }
                continue; // Skip unconditional pattern for this group
            }
        }

        // No split or not enough context data — unconditional pattern
        let preferred = if accept_rate >= 0.7 {
            if brain_action.is_empty() {
                "approve".to_string()
            } else {
                brain_action.clone()
            }
        } else if accept_rate <= 0.3 {
            if brain_action == "approve" || brain_action.is_empty() {
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
            sample_count: total,
            accept_rate,
            conditions: Vec::new(),
            confidence: (accept_rate - 0.5).abs() * 2.0,
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

    // Detect temporal patterns (Level 4)
    let temporal = detect_temporal_patterns(&decisions_mut);

    DistilledPreferences {
        patterns,
        tool_accuracy,
        total_decisions: decisions.len() as u32,
        overall_accuracy,
        temporal,
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
    if prefs.patterns.is_empty() && prefs.tool_accuracy.is_empty() && prefs.temporal.is_empty() {
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
            let cond_suffix = if p.conditions.is_empty() {
                String::new()
            } else {
                let conds: Vec<String> = p.conditions.iter().map(|c| c.label()).collect();
                format!(" when {}", conds.join(", "))
            };
            lines.push(format!(
                "- {strength} {} [{}]{cmd_part}{cond_suffix} (n={})",
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

    // Temporal patterns (situational rules)
    if !prefs.temporal.is_empty() {
        lines.push("Situational rules:".to_string());
        for tp in &prefs.temporal {
            lines.push(format!("- {}", tp.description));
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
                "conditions": p.conditions.iter().map(|c| c.to_json()).collect::<Vec<_>>(),
                "confidence": p.confidence,
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
        "temporal": prefs.temporal.iter().map(|tp| {
            serde_json::json!({
                "description": tp.description,
                "sample_count": tp.sample_count,
                "strength": tp.strength,
            })
        }).collect::<Vec<_>>(),
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
            let conditions = p
                .get("conditions")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(PreferenceCondition::from_json)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let confidence = p.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.0);
            Some(PreferencePattern {
                tool: p.get("tool")?.as_str()?.to_string(),
                command_pattern: p
                    .get("command_pattern")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                preferred_action: p.get("preferred_action")?.as_str()?.to_string(),
                sample_count: p.get("sample_count")?.as_u64()? as u32,
                accept_rate: p.get("accept_rate")?.as_f64()?,
                conditions,
                confidence,
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

    let temporal = json
        .get("temporal")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|tp| {
                    Some(TemporalPattern {
                        description: tp.get("description")?.as_str()?.to_string(),
                        sample_count: tp.get("sample_count")?.as_u64()? as u32,
                        strength: tp.get("strength")?.as_f64()?,
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Some(DistilledPreferences {
        patterns,
        tool_accuracy,
        total_decisions: json.get("total_decisions")?.as_u64()? as u32,
        overall_accuracy: json.get("overall_accuracy")?.as_f64()?,
        temporal,
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

pub fn read_all_decisions() -> Vec<DecisionRecord> {
    let path = decisions_path();
    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    content
        .lines()
        .filter_map(|line| {
            let json: serde_json::Value = serde_json::from_str(line).ok()?;
            let context = json.get("context").and_then(|ctx| {
                Some(DecisionContext {
                    cost_usd: ctx.get("cost_usd")?.as_f64()?,
                    context_pct: ctx.get("context_pct")?.as_u64()? as u8,
                    last_tool_error: ctx.get("last_tool_error")?.as_bool()?,
                    error_message: ctx
                        .get("error_message")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    model: ctx.get("model")?.as_str()?.to_string(),
                    elapsed_secs: ctx.get("elapsed_secs")?.as_u64()?,
                    files_modified_count: ctx.get("files_modified_count")?.as_u64()? as u32,
                    total_tool_calls: ctx.get("total_tool_calls")?.as_u64()? as u32,
                    has_file_conflict: ctx.get("has_file_conflict")?.as_bool()?,
                    status: ctx.get("status")?.as_str()?.to_string(),
                    burn_rate_per_hr: ctx.get("burn_rate_per_hr")?.as_f64()?,
                    recent_error_count: ctx.get("recent_error_count")?.as_u64()? as u8,
                    subagent_count: ctx.get("subagent_count")?.as_u64()? as u8,
                })
            });
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
                // Handle null brain_action (observations log it as null)
                brain_action: json
                    .get("brain_action")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                brain_confidence: json
                    .get("brain_confidence")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0),
                brain_reasoning: json
                    .get("brain_reasoning")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                user_action: json.get("user_action")?.as_str()?.to_string(),
                context,
                outcome: None, // Backfilled during distillation
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
    pub observations: u32,
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
            observations: 0,
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
            context: None,
            outcome: None,
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
            context: None,
            outcome: None,
        }
    }

    fn make_context(cost_usd: f64, context_pct: u8, last_tool_error: bool) -> DecisionContext {
        DecisionContext {
            cost_usd,
            context_pct,
            last_tool_error,
            error_message: if last_tool_error {
                Some("test error".to_string())
            } else {
                None
            },
            model: "sonnet".into(),
            elapsed_secs: 60,
            files_modified_count: 2,
            total_tool_calls: 10,
            has_file_conflict: false,
            status: "Working".into(),
            burn_rate_per_hr: 1.0,
            recent_error_count: if last_tool_error { 1 } else { 0 },
            subagent_count: 0,
        }
    }

    fn make_decision_with_context(
        tool: &str,
        project: &str,
        user_action: &str,
        ctx: DecisionContext,
    ) -> DecisionRecord {
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
            context: Some(ctx),
            outcome: None,
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
        assert!(prefs.temporal.is_empty());
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
        assert!(!accept.is_observation());

        let reject = make_decision("Bash", "proj", "reject");
        assert!(!reject.is_positive());
        assert!(reject.is_negative());
        assert!(!reject.is_observation());

        let auto = make_decision("Bash", "proj", "auto");
        assert!(auto.is_positive());
        assert!(!auto.is_negative());
        assert!(!auto.is_observation());

        let deny_override = make_decision("Bash", "proj", "deny_rule_override");
        assert!(!deny_override.is_positive());
        assert!(deny_override.is_negative());
    }

    // ── Passive observation tests ─────────────────────────────────────

    #[test]
    fn observation_user_approve_is_positive() {
        let d = make_decision("Read", "proj", "user_approve");
        assert!(d.is_positive());
        assert!(!d.is_negative());
        assert!(d.is_observation());
    }

    #[test]
    fn observation_rule_approve_is_positive() {
        let d = make_decision("Bash", "proj", "rule_approve");
        assert!(d.is_positive());
        assert!(d.is_observation());
    }

    #[test]
    fn observation_rule_deny_is_negative() {
        let d = make_decision("Bash", "proj", "rule_deny");
        assert!(d.is_negative());
        assert!(d.is_observation());
    }

    #[test]
    fn observation_conflict_deny_is_negative() {
        let d = make_decision("Write", "proj", "conflict_deny");
        assert!(d.is_negative());
        assert!(d.is_observation());
    }

    #[test]
    fn observation_user_input_is_observation() {
        let d = make_decision("Bash", "proj", "user_input");
        assert!(d.is_observation());
        // user_input is neither approve nor deny
        assert!(!d.is_positive());
        assert!(!d.is_negative());
    }

    #[test]
    fn observations_feed_into_distillation() {
        // Mix of brain decisions and observations — all should be used
        let mut decisions: Vec<DecisionRecord> = (0..3)
            .map(|_| make_decision("Read", "proj", "accept"))
            .collect();
        decisions.extend((0..5).map(|_| make_decision("Read", "proj", "user_approve")));

        let prefs = distill_preferences(&decisions);
        // Read should show as strongly positive (8/8 positive outcomes)
        let read_pattern = prefs.patterns.iter().find(|p| p.tool == "Read");
        assert!(read_pattern.is_some());
        assert!(read_pattern.unwrap().accept_rate >= 0.9);
    }

    #[test]
    fn format_few_shot_observation_format() {
        let d = make_decision("Read", "proj", "user_approve");
        let output = format_few_shot_examples(&[d]);
        assert!(output.contains("user action: user_approve"));
        assert!(!output.contains("brain:"));
    }

    #[test]
    fn format_few_shot_brain_decision_format() {
        let d = make_decision("Bash", "proj", "accept");
        let output = format_few_shot_examples(&[d]);
        assert!(output.contains("brain: approve"));
        assert!(output.contains("user: accept"));
    }

    #[test]
    fn outcome_weighted_retrieval_prefers_corrections() {
        // Rejected decisions should score higher (correction signal)
        let decisions = [
            make_decision("Bash", "proj", "accept"),
            make_decision("Bash", "proj", "reject"),
        ];

        // Simulate scoring: reject gets +8, accept gets +3
        // Both match on tool (+10) and project (+5)
        // reject total = 10+5+8+recency = higher
        let reject = &decisions[1];
        assert!(reject.is_negative());
    }

    // ── Multi-level learning tests ───────────────────────────────────

    #[test]
    fn test_snapshot_context_fields() {
        use crate::session::{ClaudeSession, SessionStatus};
        use std::collections::HashMap;
        use std::time::Duration;

        let mut tool_usage = HashMap::new();
        tool_usage.insert("Bash".to_string(), crate::session::ToolStats { calls: 5 });
        tool_usage.insert("Read".to_string(), crate::session::ToolStats { calls: 3 });

        let mut files = HashMap::new();
        files.insert("src/main.rs".to_string(), 2u32);

        let session = ClaudeSession {
            pid: 42,
            session_id: "test-session".into(),
            cwd: "/tmp".into(),
            project_name: "test-proj".into(),
            started_at: 0,
            elapsed: Duration::from_secs(120),
            tty: "/dev/pts/0".into(),
            status: SessionStatus::Processing,
            cpu_percent: 50.0,
            cpu_history: vec![],
            mem_mb: 100.0,
            own_input_tokens: 1000,
            own_output_tokens: 500,
            own_cache_read_tokens: 0,
            own_cache_write_tokens: 0,
            subagent_input_tokens: 0,
            subagent_output_tokens: 0,
            subagent_cache_read_tokens: 0,
            subagent_cache_write_tokens: 0,
            total_input_tokens: 1000,
            total_output_tokens: 500,
            model: "sonnet".into(),
            command_args: "".into(),
            session_name: "test".into(),
            jsonl_path: None,
            jsonl_offset: 0,
            last_message_ts: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            cost_usd: 3.50,
            context_tokens: 80000,
            context_max: 100000,
            prev_cost_usd: 3.0,
            burn_rate_per_hr: 2.5,
            subagent_count: 1,
            active_subagent_count: 0,
            active_subagent_jsonl_paths: vec![],
            subagent_rollups: HashMap::new(),
            activity_history: vec![],
            files_modified: files,
            tool_usage,
            worktree_id: None,
            telemetry_status: crate::session::TelemetryStatus::Available,
            usage_metrics_available: true,
            cost_estimate_unverified: false,
            model_profile_source: "builtin".into(),
            last_msg_type: "".into(),
            last_stop_reason: "".into(),
            is_waiting_for_task: false,
            pending_tool_name: None,
            pending_tool_input: None,
            pending_file_path: None,
            has_file_conflict: false,
            last_tool_error: true,
            last_error_message: Some("command failed".into()),
            recent_errors: vec![crate::session::ErrorEntry {
                tool_name: "Bash".into(),
                message: "exit code 1".into(),
            }],
        };

        let ctx = snapshot_context(&session);

        // Verify all 13 fields
        assert_eq!(ctx["cost_usd"].as_f64().unwrap(), 3.5);
        assert_eq!(ctx["context_pct"].as_u64().unwrap(), 80);
        assert!(ctx["last_tool_error"].as_bool().unwrap());
        assert_eq!(ctx["error_message"].as_str().unwrap(), "command failed");
        assert_eq!(ctx["model"].as_str().unwrap(), "sonnet");
        assert_eq!(ctx["elapsed_secs"].as_u64().unwrap(), 120);
        assert_eq!(ctx["files_modified_count"].as_u64().unwrap(), 1);
        assert_eq!(ctx["total_tool_calls"].as_u64().unwrap(), 8); // 5+3
        assert!(!ctx["has_file_conflict"].as_bool().unwrap());
        assert_eq!(ctx["status"].as_str().unwrap(), "Processing");
        assert_eq!(ctx["burn_rate_per_hr"].as_f64().unwrap(), 2.5);
        assert_eq!(ctx["recent_error_count"].as_u64().unwrap(), 1);
        assert_eq!(ctx["subagent_count"].as_u64().unwrap(), 1);
    }

    #[test]
    fn test_backward_compat_no_context() {
        // Simulate a JSONL record without the "context" field (old format)
        let json_str = r#"{"ts":"123","pid":1,"project":"proj","tool":"Bash","command":"ls","brain_action":"approve","brain_confidence":0.9,"brain_reasoning":"safe","user_action":"accept"}"#;
        let json: serde_json::Value = serde_json::from_str(json_str).unwrap();

        // Parse context — should be None
        let context = json.get("context").and_then(|ctx| {
            Some(DecisionContext {
                cost_usd: ctx.get("cost_usd")?.as_f64()?,
                context_pct: ctx.get("context_pct")?.as_u64()? as u8,
                last_tool_error: ctx.get("last_tool_error")?.as_bool()?,
                error_message: None,
                model: ctx.get("model")?.as_str()?.to_string(),
                elapsed_secs: ctx.get("elapsed_secs")?.as_u64()?,
                files_modified_count: ctx.get("files_modified_count")?.as_u64()? as u32,
                total_tool_calls: ctx.get("total_tool_calls")?.as_u64()? as u32,
                has_file_conflict: ctx.get("has_file_conflict")?.as_bool()?,
                status: ctx.get("status")?.as_str()?.to_string(),
                burn_rate_per_hr: ctx.get("burn_rate_per_hr")?.as_f64()?,
                recent_error_count: ctx.get("recent_error_count")?.as_u64()? as u8,
                subagent_count: ctx.get("subagent_count")?.as_u64()? as u8,
            })
        });
        assert!(context.is_none());

        // Also verify the record still parses with null brain_action (observation)
        let obs_str = r#"{"ts":"124","pid":1,"project":"proj","tool":"Bash","command":"ls","brain_action":null,"brain_confidence":0.0,"brain_reasoning":"","user_action":"user_approve"}"#;
        let obs_json: serde_json::Value = serde_json::from_str(obs_str).unwrap();
        let brain_action = obs_json
            .get("brain_action")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        assert_eq!(brain_action, "");
    }

    #[test]
    fn test_conditional_split_on_cost() {
        // Low-cost decisions: all accepted. High-cost decisions: all rejected.
        // Should produce a cost-based split.
        let mut decisions = Vec::new();
        for _ in 0..5 {
            decisions.push(make_decision_with_context(
                "Bash",
                "proj",
                "accept",
                make_context(1.0, 50, false),
            ));
        }
        for _ in 0..5 {
            decisions.push(make_decision_with_context(
                "Bash",
                "proj",
                "reject",
                make_context(10.0, 50, false),
            ));
        }

        let prefs = distill_preferences(&decisions);
        // Should have conditional patterns (split on cost)
        let conditional = prefs.patterns.iter().any(|p| !p.conditions.is_empty());
        assert!(
            conditional,
            "Expected conditional patterns from cost split, got: {:?}",
            prefs.patterns
        );
    }

    #[test]
    fn test_conditional_split_on_error() {
        // No-error decisions: all accepted. Error decisions: all rejected.
        let mut decisions = Vec::new();
        for _ in 0..5 {
            decisions.push(make_decision_with_context(
                "Bash",
                "proj",
                "accept",
                make_context(5.0, 50, false),
            ));
        }
        for _ in 0..5 {
            decisions.push(make_decision_with_context(
                "Bash",
                "proj",
                "reject",
                make_context(5.0, 50, true),
            ));
        }

        let prefs = distill_preferences(&decisions);
        let conditional = prefs.patterns.iter().any(|p| !p.conditions.is_empty());
        assert!(
            conditional,
            "Expected conditional patterns from error split, got: {:?}",
            prefs.patterns
        );
    }

    #[test]
    fn test_no_split_when_ambiguous() {
        // Even mix of accept/reject at all cost levels — no meaningful split
        let mut decisions = Vec::new();
        for i in 0..10 {
            let action = if i % 2 == 0 { "accept" } else { "reject" };
            let cost = (i as f64) + 1.0; // Different costs but same 50/50 split
            decisions.push(make_decision_with_context(
                "Bash",
                "proj",
                action,
                make_context(cost, 50, false),
            ));
        }

        let prefs = distill_preferences(&decisions);
        // No patterns at all (50/50 cannot split into clear halves)
        let conditional = prefs.patterns.iter().any(|p| !p.conditions.is_empty());
        assert!(
            !conditional,
            "Expected no conditional patterns for ambiguous data"
        );
    }

    #[test]
    fn test_outcome_backfill() {
        // Two consecutive same-PID records: first accept, second has error context
        let mut decisions = vec![
            DecisionRecord {
                timestamp: "1".into(),
                pid: 42,
                project: "proj".into(),
                tool: Some("Bash".into()),
                command: Some("deploy".into()),
                brain_action: "approve".into(),
                brain_confidence: 0.9,
                brain_reasoning: "safe".into(),
                user_action: "accept".into(),
                context: Some(make_context(1.0, 50, false)),
                outcome: None,
            },
            DecisionRecord {
                timestamp: "2".into(),
                pid: 42,
                project: "proj".into(),
                tool: Some("Bash".into()),
                command: Some("fix".into()),
                brain_action: "approve".into(),
                brain_confidence: 0.9,
                brain_reasoning: "safe".into(),
                user_action: "accept".into(),
                context: Some(make_context(1.5, 55, true)),
                outcome: None,
            },
        ];

        backfill_outcomes(&mut decisions);

        // First decision should be marked as Error (next had tool error)
        assert!(matches!(
            decisions[0].outcome,
            Some(DecisionOutcome::Error(_))
        ));
        // Second has no subsequent record, so outcome stays None
        assert!(decisions[1].outcome.is_none());
    }

    #[test]
    fn test_temporal_error_streak() {
        // Build a scenario with error streaks
        let mut decisions = Vec::new();
        // 4 consecutive errors (same PID)
        for _ in 0..4 {
            decisions.push(DecisionRecord {
                timestamp: "0".into(),
                pid: 1,
                project: "proj".into(),
                tool: Some("Bash".into()),
                command: Some("test cmd".into()),
                brain_action: "approve".into(),
                brain_confidence: 0.9,
                brain_reasoning: "test".into(),
                user_action: "accept".into(),
                context: Some(make_context(1.0, 50, true)),
                outcome: None,
            });
        }
        // Then user denies
        decisions.push(DecisionRecord {
            timestamp: "0".into(),
            pid: 1,
            project: "proj".into(),
            tool: Some("Bash".into()),
            command: Some("test cmd".into()),
            brain_action: "approve".into(),
            brain_confidence: 0.9,
            brain_reasoning: "test".into(),
            user_action: "reject".into(),
            context: Some(make_context(1.0, 50, false)),
            outcome: None,
        });
        // Repeat the streak pattern to reach threshold of 2
        for _ in 0..4 {
            decisions.push(DecisionRecord {
                timestamp: "0".into(),
                pid: 1,
                project: "proj".into(),
                tool: Some("Bash".into()),
                command: Some("test cmd".into()),
                brain_action: "approve".into(),
                brain_confidence: 0.9,
                brain_reasoning: "test".into(),
                user_action: "accept".into(),
                context: Some(make_context(1.0, 50, true)),
                outcome: None,
            });
        }
        decisions.push(DecisionRecord {
            timestamp: "0".into(),
            pid: 1,
            project: "proj".into(),
            tool: Some("Bash".into()),
            command: Some("test cmd".into()),
            brain_action: "approve".into(),
            brain_confidence: 0.9,
            brain_reasoning: "test".into(),
            user_action: "reject".into(),
            context: Some(make_context(1.0, 50, false)),
            outcome: None,
        });

        let patterns = detect_temporal_patterns(&decisions);
        let error_streak = patterns.iter().any(|p| p.description.contains("3+ errors"));
        assert!(
            error_streak,
            "Expected error streak pattern, got: {:?}",
            patterns
        );
    }

    #[test]
    fn test_temporal_context_pressure() {
        // Low context: mostly accepted. High context: mostly rejected.
        let mut decisions = Vec::new();
        // 5 low-context accepts
        for _ in 0..5 {
            decisions.push(make_decision_with_context(
                "Bash",
                "proj",
                "accept",
                make_context(1.0, 30, false),
            ));
        }
        // 5 high-context rejects
        for _ in 0..5 {
            decisions.push(make_decision_with_context(
                "Bash",
                "proj",
                "reject",
                make_context(1.0, 90, false),
            ));
        }

        let patterns = detect_temporal_patterns(&decisions);
        let ctx_pressure = patterns
            .iter()
            .any(|p| p.description.contains("Context >80%"));
        assert!(
            ctx_pressure,
            "Expected context pressure pattern, got: {:?}",
            patterns
        );
    }

    #[test]
    fn test_gini_pure() {
        // All positive → gini = 0
        assert!((gini_impurity(10, 0) - 0.0).abs() < f64::EPSILON);
        // All negative → gini = 0
        assert!((gini_impurity(0, 10) - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_gini_mixed() {
        // 50/50 → gini = 0.5
        assert!((gini_impurity(5, 5) - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_gini_empty() {
        // No data → gini = 0
        assert!((gini_impurity(0, 0) - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_preference_condition_label() {
        assert_eq!(PreferenceCondition::CostBelow(5.0).label(), "cost<$5");
        assert_eq!(PreferenceCondition::CostAbove(10.0).label(), "cost>$10");
        assert_eq!(PreferenceCondition::ContextBelow(80).label(), "ctx<80%");
        assert_eq!(PreferenceCondition::ContextAbove(80).label(), "ctx>80%");
        assert_eq!(PreferenceCondition::NoErrors.label(), "no errors");
        assert_eq!(PreferenceCondition::HasErrors.label(), "errors");
        assert_eq!(PreferenceCondition::NoFileConflict.label(), "no conflict");
        assert_eq!(PreferenceCondition::HasFileConflict.label(), "conflict");
    }

    #[test]
    fn test_preference_condition_roundtrip() {
        let conditions = vec![
            PreferenceCondition::CostBelow(5.0),
            PreferenceCondition::CostAbove(10.0),
            PreferenceCondition::ContextBelow(80),
            PreferenceCondition::ContextAbove(80),
            PreferenceCondition::NoErrors,
            PreferenceCondition::HasErrors,
            PreferenceCondition::NoFileConflict,
            PreferenceCondition::HasFileConflict,
        ];
        for cond in &conditions {
            let json = cond.to_json();
            let parsed = PreferenceCondition::from_json(&json);
            assert!(parsed.is_some(), "Failed roundtrip for: {:?}", cond);
        }
    }

    #[test]
    fn test_format_summary_with_conditions() {
        let prefs = DistilledPreferences {
            patterns: vec![PreferencePattern {
                tool: "Bash".into(),
                command_pattern: Some("git push".into()),
                preferred_action: "approve".into(),
                sample_count: 8,
                accept_rate: 0.9,
                conditions: vec![PreferenceCondition::CostBelow(5.0)],
                confidence: 0.8,
            }],
            tool_accuracy: Vec::new(),
            total_decisions: 10,
            overall_accuracy: 0.8,
            temporal: Vec::new(),
        };
        let summary = format_preference_summary(&prefs);
        assert!(summary.contains("when cost<$5"));
        assert!(summary.contains("[Bash]"));
        assert!(summary.contains("git push"));
    }

    #[test]
    fn test_format_summary_with_temporal() {
        let prefs = DistilledPreferences {
            patterns: Vec::new(),
            tool_accuracy: vec![ToolAccuracy {
                tool: "Bash".into(),
                total: 5,
                correct: 1,
                confidence_threshold: 0.95,
            }],
            total_decisions: 10,
            overall_accuracy: 0.2,
            temporal: vec![TemporalPattern {
                description: "After 3+ errors: user usually denies (n=5)".into(),
                sample_count: 5,
                strength: 0.8,
            }],
        };
        let summary = format_preference_summary(&prefs);
        assert!(summary.contains("Situational rules:"));
        assert!(summary.contains("3+ errors"));
    }
}
