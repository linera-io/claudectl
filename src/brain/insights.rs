#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;

use super::decisions::{DecisionRecord, DistilledPreferences};

// ────────────────────────────────────────────────────────────────────────────
// Data structures
// ────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InsightCategory {
    FrictionPattern,
    ErrorLoop,
    ContextBlowout,
    MissingRule,
    AccuracyGap,
    TemporalFriction,
    CostPattern,
}

impl InsightCategory {
    fn label(&self) -> &'static str {
        match self {
            InsightCategory::FrictionPattern => "friction_pattern",
            InsightCategory::ErrorLoop => "error_loop",
            InsightCategory::ContextBlowout => "context_blowout",
            InsightCategory::MissingRule => "missing_rule",
            InsightCategory::AccuracyGap => "accuracy_gap",
            InsightCategory::TemporalFriction => "temporal_friction",
            InsightCategory::CostPattern => "cost_pattern",
        }
    }

    fn from_label(s: &str) -> Option<Self> {
        match s {
            "friction_pattern" => Some(InsightCategory::FrictionPattern),
            "error_loop" => Some(InsightCategory::ErrorLoop),
            "context_blowout" => Some(InsightCategory::ContextBlowout),
            "missing_rule" => Some(InsightCategory::MissingRule),
            "accuracy_gap" => Some(InsightCategory::AccuracyGap),
            "temporal_friction" => Some(InsightCategory::TemporalFriction),
            "cost_pattern" => Some(InsightCategory::CostPattern),
            _ => None,
        }
    }

    fn display_name(&self) -> &'static str {
        match self {
            InsightCategory::FrictionPattern => "Friction Patterns",
            InsightCategory::ErrorLoop => "Error Loops",
            InsightCategory::ContextBlowout => "Context Blowouts",
            InsightCategory::MissingRule => "Recommended Rules",
            InsightCategory::AccuracyGap => "Accuracy Gaps",
            InsightCategory::TemporalFriction => "Temporal Patterns",
            InsightCategory::CostPattern => "Cost Patterns",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum InsightSeverity {
    Info,
    Suggestion,
    Warning,
    Critical,
}

impl InsightSeverity {
    fn label(&self) -> &'static str {
        match self {
            InsightSeverity::Info => "info",
            InsightSeverity::Suggestion => "suggestion",
            InsightSeverity::Warning => "warning",
            InsightSeverity::Critical => "critical",
        }
    }

    fn from_label(s: &str) -> Self {
        match s {
            "critical" => InsightSeverity::Critical,
            "warning" => InsightSeverity::Warning,
            "suggestion" => InsightSeverity::Suggestion,
            _ => InsightSeverity::Info,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Insight {
    pub fingerprint: String,
    pub generated_at: u64,
    pub category: InsightCategory,
    pub severity: InsightSeverity,
    pub summary: String,
    pub suggestion: Option<String>,
    pub evidence_count: u32,
}

pub struct InsightState {
    pub seen_fingerprints: HashSet<String>,
    pub last_generated: u64,
    pub current_insights: Vec<Insight>,
}

// ────────────────────────────────────────────────────────────────────────────
// Mode toggle (on/off)
// ────────────────────────────────────────────────────────────────────────────

fn insights_mode_path() -> PathBuf {
    super::decisions::decisions_dir().join("insights-mode")
}

/// Read the current insights mode. Returns "off" if no file exists (opt-in).
pub fn read_insights_mode() -> String {
    let path = insights_mode_path();
    fs::read_to_string(&path)
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "off".into())
}

/// Write the insights mode to disk.
pub fn write_insights_mode(mode: &str) -> Result<(), String> {
    let path = insights_mode_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if mode == "off" {
        let _ = fs::remove_file(&path);
        Ok(())
    } else {
        fs::write(&path, mode).map_err(|e| format!("write error: {e}"))
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Persistence
// ────────────────────────────────────────────────────────────────────────────

fn insights_path() -> PathBuf {
    super::decisions::decisions_dir().join("insights.json")
}

fn epoch_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn load_state() -> InsightState {
    let path = insights_path();
    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => {
            return InsightState {
                seen_fingerprints: HashSet::new(),
                last_generated: 0,
                current_insights: Vec::new(),
            };
        }
    };

    let json: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => {
            return InsightState {
                seen_fingerprints: HashSet::new(),
                last_generated: 0,
                current_insights: Vec::new(),
            };
        }
    };

    let seen = json
        .get("seen_fingerprints")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default();

    let last_generated = json
        .get("last_generated")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let current = json
        .get("current_insights")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(parse_insight_json).collect())
        .unwrap_or_default();

    InsightState {
        seen_fingerprints: seen,
        last_generated,
        current_insights: current,
    }
}

pub fn save_state(state: &InsightState) -> Result<(), String> {
    let path = insights_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let json = serde_json::json!({
        "seen_fingerprints": state.seen_fingerprints.iter().collect::<Vec<_>>(),
        "last_generated": state.last_generated,
        "current_insights": state.current_insights.iter().map(insight_to_json).collect::<Vec<_>>(),
    });

    fs::write(
        &path,
        serde_json::to_string_pretty(&json).map_err(|e| format!("json error: {e}"))?,
    )
    .map_err(|e| format!("write error: {e}"))
}

fn insight_to_json(i: &Insight) -> serde_json::Value {
    serde_json::json!({
        "fingerprint": i.fingerprint,
        "generated_at": i.generated_at,
        "category": i.category.label(),
        "severity": i.severity.label(),
        "summary": i.summary,
        "suggestion": i.suggestion,
        "evidence_count": i.evidence_count,
    })
}

fn parse_insight_json(v: &serde_json::Value) -> Option<Insight> {
    Some(Insight {
        fingerprint: v.get("fingerprint")?.as_str()?.to_string(),
        generated_at: v.get("generated_at")?.as_u64()?,
        category: InsightCategory::from_label(v.get("category")?.as_str()?)?,
        severity: InsightSeverity::from_label(
            v.get("severity").and_then(|s| s.as_str()).unwrap_or("info"),
        ),
        summary: v.get("summary")?.as_str()?.to_string(),
        suggestion: v
            .get("suggestion")
            .and_then(|s| s.as_str())
            .map(|s| s.to_string()),
        evidence_count: v.get("evidence_count")?.as_u64()? as u32,
    })
}

// ────────────────────────────────────────────────────────────────────────────
// Differential merging
// ────────────────────────────────────────────────────────────────────────────

/// Merge newly generated insights with existing state.
/// Returns only the insights that are NEW (unseen fingerprints).
/// Updates `state.seen_fingerprints` and `state.current_insights`.
pub fn merge_insights(generated: Vec<Insight>, state: &mut InsightState) -> Vec<Insight> {
    // Prune seen fingerprints that are no longer in the current set
    let current_fps: HashSet<String> = generated.iter().map(|i| i.fingerprint.clone()).collect();
    state
        .seen_fingerprints
        .retain(|fp| current_fps.contains(fp));

    // Filter to only unseen
    let new: Vec<Insight> = generated
        .iter()
        .filter(|i| !state.seen_fingerprints.contains(&i.fingerprint))
        .cloned()
        .collect();

    // Mark new as seen
    for i in &new {
        state.seen_fingerprints.insert(i.fingerprint.clone());
    }

    state.current_insights = generated;
    state.last_generated = epoch_now();
    new
}

// ────────────────────────────────────────────────────────────────────────────
// Detection algorithms
// ────────────────────────────────────────────────────────────────────────────

/// Extract a command keyword for grouping (first two tokens).
/// Duplicated from decisions.rs because that function is private.
fn extract_command_keyword(command: Option<&str>) -> Option<String> {
    let cmd = command?.trim();
    if cmd.is_empty() {
        return None;
    }
    let tokens: Vec<&str> = cmd.split_whitespace().take(2).collect();
    Some(tokens.join(" "))
}

/// Detect tools/commands that are repeatedly rejected by the user.
fn detect_friction_patterns(decisions: &[DecisionRecord]) -> Vec<Insight> {
    let mut groups: HashMap<(String, Option<String>), (u32, u32)> = HashMap::new();

    for d in decisions {
        let tool = d.tool.clone().unwrap_or_else(|| "*".to_string());
        let cmd = extract_command_keyword(d.command.as_deref());
        let key = (tool, cmd);
        let entry = groups.entry(key).or_insert((0, 0));
        entry.0 += 1; // total
        if d.is_negative() {
            entry.1 += 1; // rejected
        }
    }

    let now = epoch_now();
    let mut insights = Vec::new();

    for ((tool, cmd), (total, rejected)) in &groups {
        if *rejected < 3 || *total < 3 {
            continue;
        }
        let rejection_rate = *rejected as f64 / *total as f64;
        if rejection_rate < 0.6 {
            continue;
        }

        let cmd_part = cmd
            .as_ref()
            .map(|c| format!(" \"{c}\""))
            .unwrap_or_default();

        let severity = if rejection_rate >= 0.9 {
            InsightSeverity::Warning
        } else {
            InsightSeverity::Suggestion
        };

        insights.push(Insight {
            fingerprint: format!("friction:{}:{}", tool, cmd.as_deref().unwrap_or("*")),
            generated_at: now,
            category: InsightCategory::FrictionPattern,
            severity,
            summary: format!(
                "[{tool}]{cmd_part} rejected {rejected}/{total} times ({:.0}%)",
                rejection_rate * 100.0
            ),
            suggestion: Some(format!("consider adding deny rule for [{tool}]{cmd_part}")),
            evidence_count: *total,
        });
    }

    insights
}

/// Detect repeated errors from the same tool across sessions.
fn detect_error_loops(decisions: &[DecisionRecord]) -> Vec<Insight> {
    // Group by PID, then find consecutive errors for same tool
    let mut pid_groups: HashMap<u32, Vec<&DecisionRecord>> = HashMap::new();
    for d in decisions {
        pid_groups.entry(d.pid).or_default().push(d);
    }

    // Count how many sessions had error loops for each (tool, cmd) combo
    let mut loop_counts: HashMap<(String, Option<String>), u32> = HashMap::new();

    for session_decisions in pid_groups.values() {
        let mut streak_tool: Option<String> = None;
        let mut streak_cmd: Option<String> = None;
        let mut streak_count: u32 = 0;

        for d in session_decisions {
            let has_error = d
                .context
                .as_ref()
                .map(|c| c.last_tool_error)
                .unwrap_or(false);
            let tool = d.tool.clone().unwrap_or_default();
            let cmd = extract_command_keyword(d.command.as_deref());

            if has_error && Some(&tool) == streak_tool.as_ref() {
                streak_count += 1;
            } else if has_error {
                // New error streak
                streak_tool = Some(tool.clone());
                streak_cmd = cmd.clone();
                streak_count = 1;
            } else {
                // No error — check if previous streak was long enough
                if streak_count >= 3 {
                    if let Some(ref t) = streak_tool {
                        *loop_counts
                            .entry((t.clone(), streak_cmd.clone()))
                            .or_insert(0) += 1;
                    }
                }
                streak_tool = None;
                streak_cmd = None;
                streak_count = 0;
            }
        }
        // Check trailing streak
        if streak_count >= 3 {
            if let Some(ref t) = streak_tool {
                *loop_counts
                    .entry((t.clone(), streak_cmd.clone()))
                    .or_insert(0) += 1;
            }
        }
    }

    let now = epoch_now();
    loop_counts
        .into_iter()
        .filter(|(_, count)| *count >= 1)
        .map(|((tool, cmd), count)| {
            let cmd_part = cmd
                .as_ref()
                .map(|c| format!(" \"{c}\""))
                .unwrap_or_default();
            Insight {
                fingerprint: format!("error_loop:{}:{}", tool, cmd.as_deref().unwrap_or("*")),
                generated_at: now,
                category: InsightCategory::ErrorLoop,
                severity: if count >= 3 {
                    InsightSeverity::Warning
                } else {
                    InsightSeverity::Suggestion
                },
                summary: format!(
                    "[{tool}]{cmd_part} hit 3+ consecutive errors in {count} session(s)"
                ),
                suggestion: Some(format!("investigate why [{tool}]{cmd_part} keeps failing")),
                evidence_count: count,
            }
        })
        .collect()
}

/// Detect sessions frequently hitting high context usage.
fn detect_context_blowouts(decisions: &[DecisionRecord]) -> Vec<Insight> {
    // Group by PID, check if any decision in session had context > 80%
    let mut pid_max_context: HashMap<u32, u8> = HashMap::new();
    for d in decisions {
        if let Some(ref ctx) = d.context {
            let entry = pid_max_context.entry(d.pid).or_insert(0);
            if ctx.context_pct > *entry {
                *entry = ctx.context_pct;
            }
        }
    }

    if pid_max_context.is_empty() {
        return Vec::new();
    }

    // Only look at recent sessions (last 20 PIDs by insertion order)
    let recent: Vec<u8> = pid_max_context
        .values()
        .copied()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .take(20)
        .collect();

    let blowout_count = recent.iter().filter(|&&pct| pct > 80).count();
    let total = recent.len();
    let blowout_rate = blowout_count as f64 / total as f64;

    if blowout_rate < 0.4 || blowout_count < 2 {
        return Vec::new();
    }

    vec![Insight {
        fingerprint: "context_blowout:global".to_string(),
        generated_at: epoch_now(),
        category: InsightCategory::ContextBlowout,
        severity: if blowout_rate >= 0.7 {
            InsightSeverity::Warning
        } else {
            InsightSeverity::Suggestion
        },
        summary: format!(
            "Context >80% in {blowout_count}/{total} recent sessions ({:.0}%)",
            blowout_rate * 100.0
        ),
        suggestion: Some("consider earlier /compact when context approaches 70%".to_string()),
        evidence_count: blowout_count as u32,
    }]
}

/// Detect high-confidence patterns that could become AutoRules.
fn detect_missing_rules(
    _decisions: &[DecisionRecord],
    prefs: &DistilledPreferences,
) -> Vec<Insight> {
    let now = epoch_now();
    let mut insights = Vec::new();

    for p in &prefs.patterns {
        if p.sample_count < 5 || p.confidence < 0.8 {
            continue;
        }

        // High-confidence approve patterns
        if p.accept_rate >= 0.9 {
            let cmd_part = p
                .command_pattern
                .as_ref()
                .map(|c| format!(" \"{c}\""))
                .unwrap_or_default();

            insights.push(Insight {
                fingerprint: format!(
                    "missing_rule:approve:{}:{}",
                    p.tool,
                    p.command_pattern.as_deref().unwrap_or("*")
                ),
                generated_at: now,
                category: InsightCategory::MissingRule,
                severity: InsightSeverity::Suggestion,
                summary: format!(
                    "approve [{}]{cmd_part} (accepted {:.0}%, n={})",
                    p.tool,
                    p.accept_rate * 100.0,
                    p.sample_count,
                ),
                suggestion: Some(format!(
                    "add to .claudectl.toml: [[rules]] match_tool=\"{}\" match_command=\"{}\" action=\"approve\"",
                    p.tool,
                    p.command_pattern.as_deref().unwrap_or("*"),
                )),
                evidence_count: p.sample_count,
            });
        }

        // High-confidence deny patterns
        if p.accept_rate <= 0.1 {
            let cmd_part = p
                .command_pattern
                .as_ref()
                .map(|c| format!(" \"{c}\""))
                .unwrap_or_default();

            insights.push(Insight {
                fingerprint: format!(
                    "missing_rule:deny:{}:{}",
                    p.tool,
                    p.command_pattern.as_deref().unwrap_or("*")
                ),
                generated_at: now,
                category: InsightCategory::MissingRule,
                severity: InsightSeverity::Suggestion,
                summary: format!(
                    "deny [{}]{cmd_part} (rejected {:.0}%, n={})",
                    p.tool,
                    (1.0 - p.accept_rate) * 100.0,
                    p.sample_count,
                ),
                suggestion: Some(format!(
                    "add to .claudectl.toml: [[rules]] match_tool=\"{}\" match_command=\"{}\" action=\"deny\"",
                    p.tool,
                    p.command_pattern.as_deref().unwrap_or("*"),
                )),
                evidence_count: p.sample_count,
            });
        }
    }

    insights
}

/// Detect tools where brain accuracy is low.
fn detect_accuracy_gaps(prefs: &DistilledPreferences) -> Vec<Insight> {
    let now = epoch_now();
    prefs
        .tool_accuracy
        .iter()
        .filter(|ta| ta.total >= 5 && ta.confidence_threshold > 0.7)
        .map(|ta| {
            let accuracy = if ta.total > 0 {
                (ta.correct as f64 / ta.total as f64) * 100.0
            } else {
                0.0
            };
            Insight {
                fingerprint: format!("accuracy_gap:{}", ta.tool),
                generated_at: now,
                category: InsightCategory::AccuracyGap,
                severity: if accuracy < 50.0 {
                    InsightSeverity::Warning
                } else {
                    InsightSeverity::Suggestion
                },
                summary: format!(
                    "Brain accuracy for [{}] is {:.0}% (threshold raised to {:.2})",
                    ta.tool, accuracy, ta.confidence_threshold,
                ),
                suggestion: Some(format!(
                    "more training data needed for [{}] — brain defers these to manual review",
                    ta.tool,
                )),
                evidence_count: ta.total,
            }
        })
        .collect()
}

/// Convert temporal patterns from distillation into insights.
fn detect_temporal_friction(prefs: &DistilledPreferences) -> Vec<Insight> {
    let now = epoch_now();
    prefs
        .temporal
        .iter()
        .filter(|tp| tp.strength > 0.3 && tp.sample_count >= 3)
        .map(|tp| {
            // Use first 40 chars of description as fingerprint suffix
            let fp_suffix: String = tp
                .description
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == ' ')
                .take(40)
                .collect::<String>()
                .replace(' ', "_");
            Insight {
                fingerprint: format!("temporal:{fp_suffix}"),
                generated_at: now,
                category: InsightCategory::TemporalFriction,
                severity: InsightSeverity::Info,
                summary: tp.description.clone(),
                suggestion: None,
                evidence_count: tp.sample_count,
            }
        })
        .collect()
}

/// Detect increasing burn rate trends and cost outliers.
fn detect_cost_patterns(decisions: &[DecisionRecord]) -> Vec<Insight> {
    let burn_rates: Vec<f64> = decisions
        .iter()
        .filter_map(|d| d.context.as_ref().map(|c| c.burn_rate_per_hr))
        .filter(|r| *r > 0.0)
        .collect();

    if burn_rates.len() < 10 {
        return Vec::new();
    }

    let mut insights = Vec::new();
    let now = epoch_now();

    // Compare first half vs second half burn rates
    let mid = burn_rates.len() / 2;
    let first_avg: f64 = burn_rates[..mid].iter().sum::<f64>() / mid as f64;
    let second_avg: f64 = burn_rates[mid..].iter().sum::<f64>() / (burn_rates.len() - mid) as f64;

    if first_avg > 0.0 {
        let increase = (second_avg - first_avg) / first_avg;
        if increase > 0.5 {
            insights.push(Insight {
                fingerprint: "cost_trend:increasing".to_string(),
                generated_at: now,
                category: InsightCategory::CostPattern,
                severity: if increase > 1.0 {
                    InsightSeverity::Warning
                } else {
                    InsightSeverity::Suggestion
                },
                summary: format!(
                    "Burn rate trending up: ${:.2}/hr -> ${:.2}/hr ({:+.0}%)",
                    first_avg,
                    second_avg,
                    increase * 100.0,
                ),
                suggestion: Some(
                    "consider setting a budget with --budget or reviewing costly operations"
                        .to_string(),
                ),
                evidence_count: burn_rates.len() as u32,
            });
        }
    }

    // Detect cost outlier sessions
    let mut per_session_cost: HashMap<u32, f64> = HashMap::new();
    for d in decisions {
        if let Some(ref ctx) = d.context {
            let entry = per_session_cost.entry(d.pid).or_insert(0.0);
            if ctx.cost_usd > *entry {
                *entry = ctx.cost_usd;
            }
        }
    }

    if per_session_cost.len() >= 3 {
        let costs: Vec<f64> = per_session_cost.values().copied().collect();
        let avg: f64 = costs.iter().sum::<f64>() / costs.len() as f64;
        let outlier_count = costs.iter().filter(|&&c| c > avg * 2.0 && c > 1.0).count();

        if outlier_count >= 2 {
            insights.push(Insight {
                fingerprint: "cost_trend:outliers".to_string(),
                generated_at: now,
                category: InsightCategory::CostPattern,
                severity: InsightSeverity::Info,
                summary: format!(
                    "{outlier_count} sessions cost >2x average (avg ${avg:.2})"
                ),
                suggestion: Some(
                    "review high-cost sessions — consider budget limits or earlier session restarts"
                        .to_string(),
                ),
                evidence_count: outlier_count as u32,
            });
        }
    }

    insights
}

// ────────────────────────────────────────────────────────────────────────────
// Main generation entry point
// ────────────────────────────────────────────────────────────────────────────

/// Generate all insights from the decision history and distilled preferences.
/// Runs all detectors, sorts results by severity (critical first).
pub fn generate_insights(
    decisions: &[DecisionRecord],
    prefs: &DistilledPreferences,
) -> Vec<Insight> {
    let mut insights = Vec::new();
    insights.extend(detect_friction_patterns(decisions));
    insights.extend(detect_error_loops(decisions));
    insights.extend(detect_context_blowouts(decisions));
    insights.extend(detect_missing_rules(decisions, prefs));
    insights.extend(detect_accuracy_gaps(prefs));
    insights.extend(detect_temporal_friction(prefs));
    insights.extend(detect_cost_patterns(decisions));

    // Sort by severity descending, then by evidence count descending
    insights.sort_by(|a, b| {
        b.severity
            .cmp(&a.severity)
            .then_with(|| b.evidence_count.cmp(&a.evidence_count))
    });

    insights
}

// ────────────────────────────────────────────────────────────────────────────
// Formatting
// ────────────────────────────────────────────────────────────────────────────

/// Format a list of insights grouped by category.
fn format_insights(insights: &[Insight], header: &str) -> String {
    if insights.is_empty() {
        return String::new();
    }

    let mut lines = Vec::new();
    lines.push(header.to_string());
    lines.push("\u{2500}".repeat(header.len()));
    lines.push(String::new());

    // Group by category (preserving order of first appearance)
    let mut categories: Vec<InsightCategory> = Vec::new();
    let mut by_category: HashMap<InsightCategory, Vec<&Insight>> = HashMap::new();

    // Determine category order from enum variants for consistent display
    let category_order = [
        InsightCategory::FrictionPattern,
        InsightCategory::ErrorLoop,
        InsightCategory::ContextBlowout,
        InsightCategory::MissingRule,
        InsightCategory::AccuracyGap,
        InsightCategory::TemporalFriction,
        InsightCategory::CostPattern,
    ];

    for i in insights {
        by_category.entry(i.category).or_default().push(i);
    }

    for cat in &category_order {
        if let Some(group) = by_category.get(cat) {
            if !categories.contains(cat) {
                categories.push(*cat);
            }
            lines.push(format!("  {}", cat.display_name()));
            for insight in group {
                lines.push(format!("  - {}", insight.summary));
                if let Some(ref suggestion) = insight.suggestion {
                    lines.push(format!("    \u{2192} {suggestion}"));
                }
            }
            lines.push(String::new());
        }
    }

    lines.join("\n")
}

// ────────────────────────────────────────────────────────────────────────────
// CLI handler
// ────────────────────────────────────────────────────────────────────────────

/// Print insights to stdout. Called from main.rs --insights handler.
pub fn print_insights() {
    let decisions = super::decisions::read_all_decisions();
    if decisions.is_empty() {
        println!("No decision history yet. Use claudectl with --brain to build history.");
        return;
    }

    let prefs = super::decisions::load_preferences()
        .unwrap_or_else(|| super::decisions::distill_preferences(&decisions));

    let insights = generate_insights(&decisions, &prefs);
    let mut state = load_state();
    let new_insights = merge_insights(insights, &mut state);
    let _ = save_state(&state);

    if state.current_insights.is_empty() {
        println!("No insights detected. Keep using claudectl to build more history.");
        return;
    }

    let mode = read_insights_mode();
    println!(
        "Insights mode: {mode}{}",
        if mode == "off" {
            " (run claudectl --brain --insights on to enable auto-generation)"
        } else {
            ""
        }
    );
    println!();

    if !new_insights.is_empty() {
        print!(
            "{}",
            format_insights(
                &new_insights,
                &format!("New Insights ({} new)", new_insights.len()),
            )
        );
    }

    print!(
        "{}",
        format_insights(
            &state.current_insights,
            &format!("All Insights ({} total)", state.current_insights.len()),
        )
    );
}

// ────────────────────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::decisions::{
        DecisionContext, DecisionType, DistilledPreferences, PreferencePattern, TemporalPattern,
        ToolAccuracy,
    };

    fn make_decision(tool: &str, command: &str, user_action: &str, pid: u32) -> DecisionRecord {
        DecisionRecord {
            timestamp: "0".to_string(),
            pid,
            project: "test".to_string(),
            tool: Some(tool.to_string()),
            command: Some(command.to_string()),
            brain_action: "approve".to_string(),
            brain_confidence: 0.8,
            brain_reasoning: String::new(),
            user_action: user_action.to_string(),
            context: None,
            outcome: None,
            decision_type: DecisionType::Session,
        }
    }

    fn make_decision_with_context(
        tool: &str,
        command: &str,
        user_action: &str,
        pid: u32,
        context_pct: u8,
        last_error: bool,
        burn_rate: f64,
        cost: f64,
    ) -> DecisionRecord {
        let mut d = make_decision(tool, command, user_action, pid);
        d.context = Some(DecisionContext {
            cost_usd: cost,
            context_pct,
            last_tool_error: last_error,
            error_message: None,
            model: "test".to_string(),
            elapsed_secs: 100,
            files_modified_count: 0,
            total_tool_calls: 10,
            has_file_conflict: false,
            status: "Processing".to_string(),
            burn_rate_per_hr: burn_rate,
            recent_error_count: 0,
            subagent_count: 0,
            hour: Some(10),
        });
        d
    }

    fn empty_prefs() -> DistilledPreferences {
        DistilledPreferences {
            patterns: Vec::new(),
            tool_accuracy: Vec::new(),
            total_decisions: 0,
            overall_accuracy: 0.0,
            temporal: Vec::new(),
        }
    }

    #[test]
    fn test_friction_patterns_detected() {
        let decisions: Vec<DecisionRecord> = (0..10)
            .map(|i| make_decision("Bash", "npm install", "reject", i))
            .collect();

        let insights = detect_friction_patterns(&decisions);
        assert_eq!(insights.len(), 1);
        assert_eq!(insights[0].category, InsightCategory::FrictionPattern);
        assert!(insights[0].summary.contains("npm install"));
        assert!(insights[0].summary.contains("10/10"));
    }

    #[test]
    fn test_friction_below_threshold_not_detected() {
        // 2 rejections out of 5 = 40%, below 60% threshold
        let mut decisions = Vec::new();
        for i in 0..3 {
            decisions.push(make_decision("Bash", "cargo test", "accept", i));
        }
        for i in 3..5 {
            decisions.push(make_decision("Bash", "cargo test", "reject", i));
        }

        let insights = detect_friction_patterns(&decisions);
        assert!(insights.is_empty());
    }

    #[test]
    fn test_error_loops_detected() {
        // 4 consecutive errors for same tool in one session
        let decisions: Vec<DecisionRecord> = (0..4)
            .map(|_| {
                make_decision_with_context(
                    "Write",
                    "src/main.rs",
                    "accept",
                    100,
                    50,
                    true,
                    1.0,
                    0.5,
                )
            })
            .collect();

        let insights = detect_error_loops(&decisions);
        assert_eq!(insights.len(), 1);
        assert_eq!(insights[0].category, InsightCategory::ErrorLoop);
    }

    #[test]
    fn test_context_blowouts_detected() {
        // 5 sessions all hitting >80% context
        let decisions: Vec<DecisionRecord> = (0..5)
            .map(|pid| {
                make_decision_with_context("Read", "file.rs", "accept", pid, 85, false, 1.0, 0.5)
            })
            .collect();

        let insights = detect_context_blowouts(&decisions);
        assert_eq!(insights.len(), 1);
        assert_eq!(insights[0].category, InsightCategory::ContextBlowout);
    }

    #[test]
    fn test_missing_rules_detected() {
        let prefs = DistilledPreferences {
            patterns: vec![
                PreferencePattern {
                    tool: "Bash".to_string(),
                    command_pattern: Some("cargo test".to_string()),
                    preferred_action: "approve".to_string(),
                    sample_count: 15,
                    accept_rate: 1.0,
                    conditions: Vec::new(),
                    confidence: 1.0,
                },
                PreferencePattern {
                    tool: "Bash".to_string(),
                    command_pattern: Some("rm -rf".to_string()),
                    preferred_action: "deny".to_string(),
                    sample_count: 6,
                    accept_rate: 0.0,
                    conditions: Vec::new(),
                    confidence: 1.0,
                },
            ],
            tool_accuracy: Vec::new(),
            total_decisions: 21,
            overall_accuracy: 0.8,
            temporal: Vec::new(),
        };

        let insights = detect_missing_rules(&[], &prefs);
        assert_eq!(insights.len(), 2);
        assert!(insights.iter().any(|i| i.summary.contains("cargo test")));
        assert!(insights.iter().any(|i| i.summary.contains("rm -rf")));
    }

    #[test]
    fn test_accuracy_gaps_detected() {
        let prefs = DistilledPreferences {
            patterns: Vec::new(),
            tool_accuracy: vec![ToolAccuracy {
                tool: "Edit".to_string(),
                total: 10,
                correct: 4,
                confidence_threshold: 0.85,
            }],
            total_decisions: 10,
            overall_accuracy: 0.4,
            temporal: Vec::new(),
        };

        let insights = detect_accuracy_gaps(&prefs);
        assert_eq!(insights.len(), 1);
        assert!(insights[0].summary.contains("Edit"));
        assert!(insights[0].summary.contains("40%"));
    }

    #[test]
    fn test_cost_trend_detected() {
        let mut decisions = Vec::new();
        // First 10: low burn rate
        for i in 0..10 {
            decisions.push(make_decision_with_context(
                "Bash", "cmd", "accept", i, 50, false, 1.0, 0.5,
            ));
        }
        // Next 10: high burn rate (3x increase)
        for i in 10..20 {
            decisions.push(make_decision_with_context(
                "Bash", "cmd", "accept", i, 50, false, 3.0, 1.5,
            ));
        }

        let insights = detect_cost_patterns(&decisions);
        assert!(
            insights
                .iter()
                .any(|i| i.fingerprint == "cost_trend:increasing")
        );
    }

    #[test]
    fn test_differential_merging() {
        let insights = vec![
            Insight {
                fingerprint: "friction:Bash:npm install".to_string(),
                generated_at: 100,
                category: InsightCategory::FrictionPattern,
                severity: InsightSeverity::Warning,
                summary: "test".to_string(),
                suggestion: None,
                evidence_count: 5,
            },
            Insight {
                fingerprint: "accuracy_gap:Edit".to_string(),
                generated_at: 100,
                category: InsightCategory::AccuracyGap,
                severity: InsightSeverity::Suggestion,
                summary: "test2".to_string(),
                suggestion: None,
                evidence_count: 3,
            },
        ];

        let mut state = InsightState {
            seen_fingerprints: HashSet::new(),
            last_generated: 0,
            current_insights: Vec::new(),
        };

        // First merge: both are new
        let new = merge_insights(insights.clone(), &mut state);
        assert_eq!(new.len(), 2);
        assert_eq!(state.seen_fingerprints.len(), 2);

        // Second merge: none are new (already seen)
        let new2 = merge_insights(insights.clone(), &mut state);
        assert_eq!(new2.len(), 0);
        assert_eq!(state.current_insights.len(), 2);
    }

    #[test]
    fn test_stale_fingerprints_pruned() {
        let mut state = InsightState {
            seen_fingerprints: {
                let mut s = HashSet::new();
                s.insert("old_fingerprint".to_string());
                s.insert("accuracy_gap:Edit".to_string());
                s
            },
            last_generated: 0,
            current_insights: Vec::new(),
        };

        // Only one insight — the old_fingerprint should be pruned
        let insights = vec![Insight {
            fingerprint: "accuracy_gap:Edit".to_string(),
            generated_at: 100,
            category: InsightCategory::AccuracyGap,
            severity: InsightSeverity::Suggestion,
            summary: "test".to_string(),
            suggestion: None,
            evidence_count: 3,
        }];

        let _new = merge_insights(insights, &mut state);
        assert!(!state.seen_fingerprints.contains("old_fingerprint"));
        assert!(state.seen_fingerprints.contains("accuracy_gap:Edit"));
    }

    #[test]
    fn test_generate_insights_sorts_by_severity() {
        let prefs = DistilledPreferences {
            patterns: vec![PreferencePattern {
                tool: "Bash".to_string(),
                command_pattern: Some("cargo test".to_string()),
                preferred_action: "approve".to_string(),
                sample_count: 15,
                accept_rate: 1.0,
                conditions: Vec::new(),
                confidence: 1.0,
            }],
            tool_accuracy: vec![ToolAccuracy {
                tool: "Edit".to_string(),
                total: 10,
                correct: 3,
                confidence_threshold: 0.9,
            }],
            total_decisions: 25,
            overall_accuracy: 0.5,
            temporal: Vec::new(),
        };

        let mut decisions = Vec::new();
        // Add friction pattern (Warning severity)
        for i in 0..10 {
            decisions.push(make_decision("Bash", "npm install", "reject", i));
        }

        let insights = generate_insights(&decisions, &prefs);
        assert!(!insights.is_empty());

        // Verify sorted: warnings before suggestions
        for window in insights.windows(2) {
            assert!(window[0].severity >= window[1].severity);
        }
    }

    #[test]
    fn test_temporal_friction_detected() {
        let prefs = DistilledPreferences {
            patterns: Vec::new(),
            tool_accuracy: Vec::new(),
            total_decisions: 50,
            overall_accuracy: 0.8,
            temporal: vec![TemporalPattern {
                description: "After 3+ errors: user usually denies (n=5)".to_string(),
                sample_count: 5,
                strength: 0.6,
            }],
        };

        let insights = detect_temporal_friction(&prefs);
        assert_eq!(insights.len(), 1);
        assert_eq!(insights[0].category, InsightCategory::TemporalFriction);
    }

    #[test]
    fn test_empty_decisions_no_insights() {
        let insights = generate_insights(&[], &empty_prefs());
        assert!(insights.is_empty());
    }

    #[test]
    fn test_format_insights_output() {
        let insights = vec![Insight {
            fingerprint: "friction:Bash:npm install".to_string(),
            generated_at: 100,
            category: InsightCategory::FrictionPattern,
            severity: InsightSeverity::Warning,
            summary: "[Bash] \"npm install\" rejected 8/10 times".to_string(),
            suggestion: Some("consider adding deny rule".to_string()),
            evidence_count: 10,
        }];

        let output = format_insights(&insights, "Test Header");
        assert!(output.contains("Friction Patterns"));
        assert!(output.contains("npm install"));
        assert!(output.contains("consider adding deny rule"));
    }
}
