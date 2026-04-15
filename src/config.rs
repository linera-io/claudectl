use std::fs;
use std::path::PathBuf;

use crate::models::{ModelOverride, ModelProfile};
use crate::rules::{AutoRule, RuleAction};

/// Configuration loaded from TOML files, merged with CLI flags.
/// Priority: CLI flags > project config > global config > defaults.
#[derive(Debug, Clone)]
pub struct Config {
    pub interval: u64,
    pub notify: bool,
    pub debug: bool,
    pub grouped: bool,
    pub sort: Option<String>,
    pub budget: Option<f64>,
    pub kill_on_budget: bool,
    pub webhook: Option<String>,
    pub webhook_on: Option<Vec<String>>,
    pub daily_limit: Option<f64>,
    pub weekly_limit: Option<f64>,
    pub context_warn_threshold: u8, // 0-100, fires on_context_high when context % crosses this
    pub model_overrides: Vec<ModelOverride>,
    pub rules: Vec<AutoRule>,
    pub brain: Option<BrainConfig>,
}

/// Configuration for the optional local LLM brain.
/// When `None`, brain is completely disabled with zero overhead.
#[derive(Debug, Clone)]
pub struct BrainConfig {
    pub enabled: bool,
    pub endpoint: String,
    pub model: String,
    pub auto_mode: bool,
    pub timeout_ms: u64,
    pub max_context_tokens: u32,
    pub few_shot_count: usize,
}

impl Default for BrainConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            endpoint: "http://localhost:11434/api/generate".into(),
            model: "gemma4:12b".into(),
            auto_mode: false,
            timeout_ms: 5000,
            max_context_tokens: 4000,
            few_shot_count: 5,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            interval: 2000,
            notify: false,
            debug: false,
            grouped: false,
            sort: None,
            budget: None,
            kill_on_budget: false,
            webhook: None,
            webhook_on: None,
            daily_limit: None,
            weekly_limit: None,
            context_warn_threshold: 75,
            model_overrides: Vec::new(),
            rules: Vec::new(),
            brain: None,
        }
    }
}

/// Raw TOML representation — all fields optional for partial overrides.
#[derive(Debug, Default)]
struct RawConfig {
    interval: Option<u64>,
    notify: Option<bool>,
    debug: Option<bool>,
    grouped: Option<bool>,
    sort: Option<String>,
    budget: Option<f64>,
    kill_on_budget: Option<bool>,
    webhook_url: Option<String>,
    webhook_events: Option<Vec<String>>,
    daily_limit: Option<f64>,
    weekly_limit: Option<f64>,
    context_warn_threshold: Option<u8>,
    model_overrides: Vec<ModelOverride>,
    rules: Vec<AutoRule>,
    brain: Option<BrainConfig>,
}

impl Config {
    /// Load configuration from global and project config files.
    pub fn load() -> Self {
        let mut config = Config::default();

        // Layer 1: Global config
        if let Some(global) = global_config_path() {
            if let Some(raw) = parse_config_file(&global) {
                config.apply(raw);
            }
        }

        // Layer 2: Project config (.claudectl.toml in cwd)
        if let Some(raw) = parse_config_file(&PathBuf::from(".claudectl.toml")) {
            config.apply(raw);
        }

        config
    }

    /// Apply a raw config layer on top, overriding only set fields.
    fn apply(&mut self, raw: RawConfig) {
        if let Some(v) = raw.interval {
            self.interval = v;
        }
        if let Some(v) = raw.notify {
            self.notify = v;
        }
        if let Some(v) = raw.debug {
            self.debug = v;
        }
        if let Some(v) = raw.grouped {
            self.grouped = v;
        }
        if let Some(v) = raw.sort {
            self.sort = Some(v);
        }
        if let Some(v) = raw.budget {
            self.budget = Some(v);
        }
        if let Some(v) = raw.kill_on_budget {
            self.kill_on_budget = v;
        }
        if let Some(v) = raw.webhook_url {
            self.webhook = Some(v);
        }
        if let Some(v) = raw.webhook_events {
            self.webhook_on = Some(v);
        }
        if let Some(v) = raw.daily_limit {
            self.daily_limit = Some(v);
        }
        if let Some(v) = raw.weekly_limit {
            self.weekly_limit = Some(v);
        }
        if let Some(v) = raw.context_warn_threshold {
            self.context_warn_threshold = v.min(100);
        }
        for override_ in raw.model_overrides {
            upsert_model_override(&mut self.model_overrides, override_);
        }
        for rule in raw.rules {
            // Replace rule with same name, or append
            if let Some(pos) = self.rules.iter().position(|r| r.name == rule.name) {
                self.rules[pos] = rule;
            } else {
                self.rules.push(rule);
            }
        }
        if let Some(brain) = raw.brain {
            self.brain = Some(brain);
        }
    }

    /// Show resolved config and file locations (for `claudectl config`).
    pub fn print_resolved(&self) {
        println!("Resolved configuration:");
        println!();

        if let Some(p) = global_config_path() {
            if p.exists() {
                println!("  Global config: {}", p.display());
            } else {
                println!("  Global config: {} (not found)", p.display());
            }
        }

        let project_path = PathBuf::from(".claudectl.toml");
        if project_path.exists() {
            println!("  Project config: {}", project_path.display());
        } else {
            println!("  Project config: .claudectl.toml (not found)");
        }

        println!();
        println!("  interval:       {}ms", self.interval);
        println!("  notify:         {}", self.notify);
        println!("  debug:          {}", self.debug);
        println!("  grouped:        {}", self.grouped);
        println!(
            "  sort:           {}",
            self.sort.as_deref().unwrap_or("default")
        );
        println!(
            "  budget:         {}",
            self.budget
                .map(|b| format!("${b:.2}"))
                .unwrap_or_else(|| "none".into())
        );
        println!("  kill_on_budget: {}", self.kill_on_budget);
        println!(
            "  webhook:        {}",
            self.webhook.as_deref().unwrap_or("none")
        );
        println!(
            "  webhook_on:     {}",
            self.webhook_on
                .as_ref()
                .map(|v| v.join(", "))
                .unwrap_or_else(|| "all".into())
        );
        println!(
            "  daily_limit:    {}",
            self.daily_limit
                .map(|b| format!("${b:.2}"))
                .unwrap_or_else(|| "none".into())
        );
        println!(
            "  weekly_limit:   {}",
            self.weekly_limit
                .map(|b| format!("${b:.2}"))
                .unwrap_or_else(|| "none".into())
        );
        println!("  context_warn: {}%", self.context_warn_threshold);
        if self.model_overrides.is_empty() {
            println!("  model_overrides: none");
        } else {
            println!("  model_overrides:");
            for override_ in &self.model_overrides {
                println!(
                    "    {} => in ${:.2}/M, out ${:.2}/M, ctx {}",
                    override_.name,
                    override_.profile.input_per_m,
                    override_.profile.output_per_m,
                    override_.profile.context_max
                );
            }
        }
    }
}

fn global_config_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| {
        PathBuf::from(home)
            .join(".config")
            .join("claudectl")
            .join("config.toml")
    })
}

/// Minimal TOML parser — avoids adding a toml crate dependency.
/// Supports: key = value pairs, [sections], # comments, strings, numbers, booleans, arrays.
fn parse_config_file(path: &PathBuf) -> Option<RawConfig> {
    let content = fs::read_to_string(path).ok()?;
    let mut raw = RawConfig::default();
    let mut section = String::new();

    for line in content.lines() {
        let line = line.trim();

        // Skip empty lines and comments
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Section headers
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].trim().to_string();
            continue;
        }

        // Key = value
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();

        // Strip inline comments
        let value = value.split('#').next().unwrap_or(value).trim();

        match (section.as_str(), key) {
            ("" | "defaults", "interval") => {
                raw.interval = value.parse().ok();
            }
            ("" | "defaults", "notify") => {
                raw.notify = parse_bool(value);
            }
            ("" | "defaults", "debug") => {
                raw.debug = parse_bool(value);
            }
            ("" | "defaults", "grouped") => {
                raw.grouped = parse_bool(value);
            }
            ("" | "defaults", "sort") => {
                raw.sort = Some(unquote(value));
            }
            ("" | "defaults", "budget") => {
                raw.budget = value.parse().ok();
            }
            ("" | "defaults", "kill_on_budget") => {
                raw.kill_on_budget = parse_bool(value);
            }
            ("webhook", "url") => {
                raw.webhook_url = Some(unquote(value));
            }
            ("webhook", "events") => {
                raw.webhook_events = Some(parse_string_array(value));
            }
            ("budget", "daily_limit") => {
                raw.daily_limit = value.parse().ok();
            }
            ("budget", "weekly_limit") => {
                raw.weekly_limit = value.parse().ok();
            }
            ("context", "warn_threshold") => {
                raw.context_warn_threshold = value.parse().ok();
            }
            _ if parse_model_section(&section).is_some() => {
                let Some(model_name) = parse_model_section(&section) else {
                    continue;
                };
                let profile = ensure_model_override(&mut raw.model_overrides, &model_name);
                match key {
                    "input_per_m" => {
                        profile.input_per_m = value.parse().unwrap_or(profile.input_per_m);
                    }
                    "output_per_m" => {
                        profile.output_per_m = value.parse().unwrap_or(profile.output_per_m);
                    }
                    "cache_read_per_m" => {
                        profile.cache_read_per_m =
                            value.parse().unwrap_or(profile.cache_read_per_m);
                    }
                    "cache_write_per_m" => {
                        profile.cache_write_per_m =
                            value.parse().unwrap_or(profile.cache_write_per_m);
                    }
                    "context_max" => {
                        profile.context_max = value.parse().unwrap_or(profile.context_max);
                    }
                    _ => {}
                }
            }
            _ if parse_rule_section(&section).is_some() => {
                let Some(rule_name) = parse_rule_section(&section) else {
                    continue;
                };
                let rule = ensure_rule(&mut raw.rules, &rule_name);
                match key {
                    "match_status" => rule.match_status = parse_string_array(value),
                    "match_tool" => rule.match_tool = parse_string_array(value),
                    "match_command" => rule.match_command = parse_string_array(value),
                    "match_project" => rule.match_project = parse_string_array(value),
                    "match_cost_above" => rule.match_cost_above = value.parse().ok(),
                    "match_last_error" => rule.match_last_error = parse_bool(value),
                    "action" => {
                        if let Some(a) = RuleAction::parse(&unquote(value)) {
                            rule.action = a;
                        }
                    }
                    "message" => rule.message = Some(unquote(value)),
                    _ => {}
                }
            }
            ("brain", _) => {
                let brain = raw.brain.get_or_insert_with(BrainConfig::default);
                match key {
                    "enabled" => {
                        if let Some(v) = parse_bool(value) {
                            brain.enabled = v;
                        }
                    }
                    "endpoint" => brain.endpoint = unquote(value),
                    "model" => brain.model = unquote(value),
                    "auto" => {
                        if let Some(v) = parse_bool(value) {
                            brain.auto_mode = v;
                        }
                    }
                    "timeout_ms" => {
                        if let Ok(v) = value.parse() {
                            brain.timeout_ms = v;
                        }
                    }
                    "max_context_tokens" => {
                        if let Ok(v) = value.parse() {
                            brain.max_context_tokens = v;
                        }
                    }
                    "few_shot_count" => {
                        if let Ok(v) = value.parse() {
                            brain.few_shot_count = v;
                        }
                    }
                    _ => {}
                }
            }
            _ => {} // Ignore unknown keys
        }
    }

    Some(raw)
}

/// Load hooks from global and project config files.
pub fn load_hooks() -> crate::hooks::HookRegistry {
    let mut registry = crate::hooks::HookRegistry::new();

    if let Some(global) = global_config_path() {
        parse_hooks_from_file(&global, &mut registry);
    }
    parse_hooks_from_file(&PathBuf::from(".claudectl.toml"), &mut registry);

    registry
}

fn parse_hooks_from_file(path: &PathBuf, registry: &mut crate::hooks::HookRegistry) {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return,
    };

    let mut section = String::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].trim().to_string();
            continue;
        }

        // Only process hooks sections
        if !section.starts_with("hooks.") {
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        let value = value.split('#').next().unwrap_or(value).trim();

        if key == "run" {
            if let Some(event) = crate::hooks::HookEvent::from_section(&section) {
                registry.add(event, unquote(value));
            }
        }
    }
}

fn parse_bool(s: &str) -> Option<bool> {
    match s {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn unquote(s: &str) -> String {
    s.trim_matches('"').trim_matches('\'').to_string()
}

fn parse_string_array(s: &str) -> Vec<String> {
    let s = s.trim_start_matches('[').trim_end_matches(']');
    s.split(',')
        .map(|item| unquote(item.trim()))
        .filter(|item| !item.is_empty())
        .collect()
}

fn parse_model_section(section: &str) -> Option<String> {
    section.strip_prefix("models.").map(unquote)
}

fn ensure_model_override<'a>(
    overrides: &'a mut Vec<ModelOverride>,
    model_name: &str,
) -> &'a mut ModelProfile {
    if let Some(index) = overrides.iter().position(|item| item.name == model_name) {
        return &mut overrides[index].profile;
    }

    overrides.push(ModelOverride {
        name: model_name.to_string(),
        profile: ModelProfile {
            input_per_m: 0.0,
            output_per_m: 0.0,
            cache_read_per_m: 0.0,
            cache_write_per_m: 0.0,
            context_max: 0,
        },
    });

    &mut overrides
        .last_mut()
        .expect("override was just pushed")
        .profile
}

fn upsert_model_override(overrides: &mut Vec<ModelOverride>, incoming: ModelOverride) {
    if let Some(existing) = overrides.iter_mut().find(|item| item.name == incoming.name) {
        *existing = incoming;
    } else {
        overrides.push(incoming);
    }
}

fn parse_rule_section(section: &str) -> Option<String> {
    section.strip_prefix("rules.").map(unquote)
}

fn ensure_rule<'a>(rules: &'a mut Vec<AutoRule>, name: &str) -> &'a mut AutoRule {
    if let Some(index) = rules.iter().position(|r| r.name == name) {
        return &mut rules[index];
    }
    rules.push(AutoRule::new(name.to_string(), RuleAction::Approve));
    rules.last_mut().expect("rule was just pushed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_bool() {
        assert_eq!(parse_bool("true"), Some(true));
        assert_eq!(parse_bool("false"), Some(false));
        assert_eq!(parse_bool("yes"), None);
    }

    #[test]
    fn test_unquote() {
        assert_eq!(unquote("\"hello\""), "hello");
        assert_eq!(unquote("'hello'"), "hello");
        assert_eq!(unquote("hello"), "hello");
    }

    #[test]
    fn test_parse_string_array() {
        let result = parse_string_array("[\"NeedsInput\", \"Finished\"]");
        assert_eq!(result, vec!["NeedsInput", "Finished"]);
    }

    #[test]
    fn test_parse_config_file() {
        use std::io::Write;
        let mut file = tempfile::NamedTempFile::new().unwrap();
        writeln!(
            file,
            r#"
# Global claudectl config
[defaults]
interval = 1000
notify = true
grouped = true
sort = "cost"
budget = 5.00
kill_on_budget = false

[webhook]
url = "https://hooks.slack.com/test"
events = ["NeedsInput", "Finished"]

[models."gpt-4o"]
input_per_m = 1.25
output_per_m = 5.0
cache_read_per_m = 0.15
cache_write_per_m = 0.9
context_max = 128000
"#
        )
        .unwrap();
        file.flush().unwrap();

        let raw = parse_config_file(&file.path().to_path_buf()).unwrap();
        assert_eq!(raw.interval, Some(1000));
        assert_eq!(raw.notify, Some(true));
        assert_eq!(raw.grouped, Some(true));
        assert_eq!(raw.sort, Some("cost".into()));
        assert_eq!(raw.budget, Some(5.0));
        assert_eq!(raw.kill_on_budget, Some(false));
        assert_eq!(raw.webhook_url, Some("https://hooks.slack.com/test".into()));
        assert_eq!(
            raw.webhook_events,
            Some(vec!["NeedsInput".into(), "Finished".into()])
        );
        assert_eq!(raw.model_overrides.len(), 1);
        assert_eq!(raw.model_overrides[0].name, "gpt-4o");
        assert_eq!(raw.model_overrides[0].profile.context_max, 128_000);
    }

    #[test]
    fn test_config_layering() {
        let mut config = Config::default();
        assert_eq!(config.interval, 2000);
        assert!(!config.notify);

        // Apply global config
        config.apply(RawConfig {
            interval: Some(1000),
            notify: Some(true),
            budget: Some(5.0),
            ..RawConfig::default()
        });
        assert_eq!(config.interval, 1000);
        assert!(config.notify);
        assert_eq!(config.budget, Some(5.0));

        // Apply project config — overrides some fields
        config.apply(RawConfig {
            budget: Some(10.0),
            grouped: Some(true),
            ..RawConfig::default()
        });
        assert_eq!(config.interval, 1000); // Unchanged
        assert!(config.notify); // Unchanged
        assert_eq!(config.budget, Some(10.0)); // Overridden
        assert!(config.grouped); // New
    }

    #[test]
    fn test_parse_rules_from_config() {
        use std::io::Write;
        let mut file = tempfile::NamedTempFile::new().unwrap();
        writeln!(
            file,
            r#"
[rules.approve_reads]
match_status = ["Needs Input"]
match_tool = ["Read", "Glob", "Grep"]
action = "approve"

[rules.deny_destructive]
match_status = ["Needs Input"]
match_tool = ["Bash"]
match_command = ["rm -rf", "git push --force"]
action = "deny"

[rules.auto_continue]
match_status = ["Waiting"]
action = "send"
message = "continue"

[rules.kill_expensive]
match_cost_above = 10.0
action = "terminate"
"#
        )
        .unwrap();
        file.flush().unwrap();

        let raw = parse_config_file(&file.path().to_path_buf()).unwrap();
        assert_eq!(raw.rules.len(), 4);

        let r0 = &raw.rules[0];
        assert_eq!(r0.name, "approve_reads");
        assert_eq!(r0.match_tool, vec!["Read", "Glob", "Grep"]);
        assert_eq!(r0.action, RuleAction::Approve);

        let r1 = &raw.rules[1];
        assert_eq!(r1.name, "deny_destructive");
        assert_eq!(r1.match_command, vec!["rm -rf", "git push --force"]);
        assert_eq!(r1.action, RuleAction::Deny);

        let r2 = &raw.rules[2];
        assert_eq!(r2.name, "auto_continue");
        assert_eq!(r2.action, RuleAction::Send);
        assert_eq!(r2.message, Some("continue".into()));

        let r3 = &raw.rules[3];
        assert_eq!(r3.name, "kill_expensive");
        assert_eq!(r3.match_cost_above, Some(10.0));
        assert_eq!(r3.action, RuleAction::Terminate);
    }

    #[test]
    fn test_parse_brain_config() {
        use std::io::Write;
        let mut file = tempfile::NamedTempFile::new().unwrap();
        writeln!(
            file,
            r#"
[brain]
enabled = true
endpoint = "http://localhost:8080/v1/chat"
model = "llama3:8b"
auto = true
timeout_ms = 3000
max_context_tokens = 8000
"#
        )
        .unwrap();
        file.flush().unwrap();

        let raw = parse_config_file(&file.path().to_path_buf()).unwrap();
        let brain = raw.brain.expect("brain config should be parsed");
        assert!(brain.enabled);
        assert_eq!(brain.endpoint, "http://localhost:8080/v1/chat");
        assert_eq!(brain.model, "llama3:8b");
        assert!(brain.auto_mode);
        assert_eq!(brain.timeout_ms, 3000);
        assert_eq!(brain.max_context_tokens, 8000);
    }

    #[test]
    fn test_no_brain_config_returns_none() {
        use std::io::Write;
        let mut file = tempfile::NamedTempFile::new().unwrap();
        writeln!(file, "[defaults]\ninterval = 1000").unwrap();
        file.flush().unwrap();

        let raw = parse_config_file(&file.path().to_path_buf()).unwrap();
        assert!(raw.brain.is_none());
    }
}
