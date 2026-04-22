use std::io;
use std::path::{Path, PathBuf};

/// The hooks we install into Claude Code's settings.json.
///
/// Every entry maps to a deterministic state transition for the matching
/// session — see `hook_state.rs` for the receiver. The hook command is the
/// same `claudectl` binary; main detects the JSON payload on stdin and routes
/// to the state-update path before any other dispatch.
///
/// `Notification` matcher `permission_prompt` is the load-bearing one for the
/// "Needs Input" status. The rest cover compaction, tool runs, prompt submits,
/// session lifecycle, and subagent activity.
pub struct HookSpec {
    pub event: &'static str,
    pub matcher: &'static str,
    pub command: &'static str,
    pub timeout: u32,
}

impl HookSpec {
    pub fn label(&self) -> String {
        if self.matcher.is_empty() {
            self.event.to_string()
        } else {
            format!("{} ({})", self.event, self.matcher)
        }
    }
}

const HOOK_CMD: &str = "claudectl 2>/dev/null || true";

pub const HOOKS: &[HookSpec] = &[
    // Permission prompts — the deterministic "Needs Input" signal.
    HookSpec {
        event: "Notification",
        matcher: "permission_prompt",
        command: HOOK_CMD,
        timeout: 5,
    },
    // Idle prompts (Claude Code's own "are you still there?") — recorded but
    // not surfaced as a status change today.
    HookSpec {
        event: "Notification",
        matcher: "idle_prompt",
        command: HOOK_CMD,
        timeout: 5,
    },
    // Tool lifecycle.
    HookSpec {
        event: "PreToolUse",
        matcher: "*",
        command: HOOK_CMD,
        timeout: 5,
    },
    HookSpec {
        event: "PostToolUse",
        matcher: "*",
        command: HOOK_CMD,
        timeout: 5,
    },
    // Turn lifecycle.
    HookSpec {
        event: "Stop",
        matcher: "",
        command: HOOK_CMD,
        timeout: 5,
    },
    HookSpec {
        event: "UserPromptSubmit",
        matcher: "",
        command: HOOK_CMD,
        timeout: 5,
    },
    // Session lifecycle.
    HookSpec {
        event: "SessionStart",
        matcher: "",
        command: HOOK_CMD,
        timeout: 5,
    },
    HookSpec {
        event: "SessionEnd",
        matcher: "",
        command: HOOK_CMD,
        timeout: 5,
    },
    HookSpec {
        event: "SubagentStop",
        matcher: "",
        command: HOOK_CMD,
        timeout: 5,
    },
    // Auto-compact — drives the new Compacting status.
    HookSpec {
        event: "PreCompact",
        matcher: "",
        command: HOOK_CMD,
        timeout: 5,
    },
];

fn settings_path(project: bool) -> PathBuf {
    if project {
        PathBuf::from(".claude/settings.local.json")
    } else {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        home.join(".claude/settings.json")
    }
}

#[cfg(test)]
fn build_hooks_value() -> serde_json::Value {
    let mut hooks_map = serde_json::Map::new();

    for spec in HOOKS {
        let hook_entry = serde_json::json!({
            "type": "command",
            "command": spec.command,
            "timeout": spec.timeout,
        });

        let matcher_entry = serde_json::json!({
            "matcher": spec.matcher,
            "hooks": [hook_entry],
        });

        let array = hooks_map
            .entry(spec.event)
            .or_insert_with(|| serde_json::Value::Array(Vec::new()));
        if let serde_json::Value::Array(arr) = array {
            arr.push(matcher_entry);
        }
    }

    serde_json::Value::Object(hooks_map)
}

/// Check if at least one claudectl hook is registered (used to decide
/// whether `--init` reports "already configured"). Drift detection lives in
/// `find_missing_hooks`.
fn has_claudectl_hooks(existing: &serde_json::Value) -> bool {
    let Some(hooks) = existing.get("hooks").and_then(|v| v.as_object()) else {
        return false;
    };
    for matchers in hooks.values() {
        let Some(arr) = matchers.as_array() else {
            continue;
        };
        for matcher_entry in arr {
            let Some(inner) = matcher_entry.get("hooks").and_then(|v| v.as_array()) else {
                continue;
            };
            for hook in inner {
                if hook
                    .get("command")
                    .and_then(|c| c.as_str())
                    .is_some_and(|s| s.contains("claudectl"))
                {
                    return true;
                }
            }
        }
    }
    false
}

/// Returns the spec entries that are not yet installed in `existing`. A spec
/// counts as installed when there's a matcher entry with the same `matcher`
/// string and at least one inner hook whose command mentions `claudectl`.
pub fn find_missing_hooks(existing: &serde_json::Value) -> Vec<&'static HookSpec> {
    let hooks = existing.get("hooks").and_then(|v| v.as_object());
    HOOKS
        .iter()
        .filter(|spec| !is_spec_installed(hooks, spec))
        .collect()
}

fn is_spec_installed(
    hooks: Option<&serde_json::Map<String, serde_json::Value>>,
    spec: &HookSpec,
) -> bool {
    let Some(hooks) = hooks else { return false };
    let Some(matchers) = hooks.get(spec.event).and_then(|v| v.as_array()) else {
        return false;
    };
    matchers.iter().any(|entry| {
        let matcher_matches =
            entry.get("matcher").and_then(|v| v.as_str()).unwrap_or("") == spec.matcher;
        if !matcher_matches {
            return false;
        }
        entry
            .get("hooks")
            .and_then(|v| v.as_array())
            .is_some_and(|inner| {
                inner.iter().any(|hook| {
                    hook.get("command")
                        .and_then(|c| c.as_str())
                        .is_some_and(|s| s.contains("claudectl"))
                })
            })
    })
}

/// Merge the given specs into existing settings, preserving every other key
/// and every non-claudectl hook already defined.
fn merge_specs(existing: &mut serde_json::Value, specs: &[&HookSpec]) {
    let hooks_obj = existing
        .as_object_mut()
        .expect("settings must be an object")
        .entry("hooks")
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    let Some(hooks_obj) = hooks_obj.as_object_mut() else {
        return;
    };

    for spec in specs {
        let event_arr = hooks_obj
            .entry(spec.event)
            .or_insert_with(|| serde_json::Value::Array(Vec::new()));
        if let Some(arr) = event_arr.as_array_mut() {
            arr.push(serde_json::json!({
                "matcher": spec.matcher,
                "hooks": [{
                    "type": "command",
                    "command": spec.command,
                    "timeout": spec.timeout,
                }],
            }));
        }
    }
}

#[cfg(test)]
fn merge_hooks(existing: &mut serde_json::Value) {
    let all: Vec<&HookSpec> = HOOKS.iter().collect();
    merge_specs(existing, &all);
}

/// Remove claudectl hooks from a matcher entry's inner hooks array.
/// Returns true if any hooks remain after filtering.
fn filter_claudectl_hooks(matcher_entry: &mut serde_json::Value) -> bool {
    if let Some(inner_hooks) = matcher_entry.get_mut("hooks") {
        if let Some(arr) = inner_hooks.as_array_mut() {
            arr.retain(|hook| {
                hook.get("command")
                    .and_then(|c| c.as_str())
                    .is_none_or(|s| !s.contains("claudectl"))
            });
            return !arr.is_empty();
        }
    }
    true
}

/// Remove all claudectl hook entries from settings, preserving everything else.
/// Returns the number of hook entries removed.
fn remove_claudectl_hooks(settings: &mut serde_json::Value) -> usize {
    let mut removed = 0;

    let Some(hooks) = settings.get_mut("hooks") else {
        return 0;
    };
    let Some(hooks_obj) = hooks.as_object_mut() else {
        return 0;
    };

    // For each event, filter out matcher entries that contain claudectl commands
    let mut empty_events = Vec::new();
    for (event, matchers) in hooks_obj.iter_mut() {
        if let Some(arr) = matchers.as_array_mut() {
            let before = arr.len();
            arr.retain_mut(filter_claudectl_hooks);
            removed += before - arr.len();
            if arr.is_empty() {
                empty_events.push(event.clone());
            }
        }
    }

    // Remove event keys that are now empty
    for event in empty_events {
        hooks_obj.remove(&event);
    }

    // Remove the hooks key entirely if it's now empty
    if hooks_obj.is_empty() {
        if let Some(obj) = settings.as_object_mut() {
            obj.remove("hooks");
        }
    }

    removed
}

/// Run the uninit command: remove claudectl hooks from settings.json.
pub fn run_uninit(project: bool) -> io::Result<()> {
    let path = settings_path(project);

    if !path.exists() {
        println!(
            "No settings file at {} — nothing to remove.",
            path.display()
        );
        return Ok(());
    }

    let content = std::fs::read_to_string(&path)?;
    let mut settings = match serde_json::from_str::<serde_json::Value>(&content) {
        Ok(v) if v.is_object() => v,
        _ => {
            eprintln!(
                "Error: {} is not valid JSON — refusing to modify.",
                path.display()
            );
            std::process::exit(1);
        }
    };

    if !has_claudectl_hooks(&settings) {
        println!(
            "No claudectl hooks found in {} — nothing to remove.",
            path.display()
        );
        return Ok(());
    }

    let removed = remove_claudectl_hooks(&mut settings);

    // If the settings object is now empty (only had hooks), remove the file
    let is_empty = settings.as_object().is_some_and(|obj| obj.is_empty());

    if is_empty {
        std::fs::remove_file(&path)?;
        println!(
            "Removed {removed} claudectl hook(s) — {} was empty and has been deleted.",
            path.display()
        );
    } else {
        let json = serde_json::to_string_pretty(&settings)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        std::fs::write(&path, format!("{json}\n"))?;
        println!(
            "Removed {removed} claudectl hook(s) from {}",
            path.display()
        );
    }

    Ok(())
}

/// Read settings.json (returning `{}` if missing). Bails the process on
/// invalid JSON to avoid silently overwriting user content.
fn load_settings(path: &Path) -> io::Result<serde_json::Value> {
    if !path.exists() {
        return Ok(serde_json::json!({}));
    }
    let content = std::fs::read_to_string(path)?;
    match serde_json::from_str::<serde_json::Value>(&content) {
        Ok(v) if v.is_object() => Ok(v),
        Ok(_) => {
            eprintln!(
                "Error: {} exists but is not a JSON object — refusing to overwrite.",
                path.display()
            );
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!(
                "Error: {} contains invalid JSON: {} — refusing to overwrite.",
                path.display(),
                e
            );
            std::process::exit(1);
        }
    }
}

/// Install any missing claudectl hooks. Returns the specs that were added
/// (empty when settings were already up to date).
pub fn ensure_hooks_installed(project: bool) -> io::Result<Vec<&'static HookSpec>> {
    let path = settings_path(project);
    let mut settings = load_settings(&path)?;

    let missing = find_missing_hooks(&settings);
    if missing.is_empty() {
        return Ok(Vec::new());
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    merge_specs(&mut settings, &missing);
    let json = serde_json::to_string_pretty(&settings)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    std::fs::write(&path, format!("{json}\n"))?;
    Ok(missing)
}

/// `--init`: explicit, loud install. Always reports the destination file plus
/// each hook added; reports "already up to date" when there's no work.
pub fn run_init(project: bool) -> io::Result<()> {
    let path = settings_path(project);
    let installed = ensure_hooks_installed(project)?;

    if installed.is_empty() {
        println!("claudectl hooks already up to date in {}", path.display());
        return Ok(());
    }

    println!("claudectl: hooks updated in {}", path.display());
    for spec in &installed {
        println!("  + installed {}", spec.label());
    }
    let total = HOOKS.len();
    let already = total - installed.len();
    if already > 0 {
        println!("  ({already} already up to date)");
    }
    println!();
    println!("Claude Code will now notify claudectl on these events.");
    println!("Run `claudectl` to start the dashboard.");
    Ok(())
}

/// Called from interactive entry points (TUI / watch / list / doctor) before
/// dispatch. Prints the same loud summary as `--init` whenever it adds
/// anything; silent when settings were already complete. Also opportunistically
/// prunes stale per-session state files (older than 14 days).
pub fn auto_init_loud(project: bool) {
    crate::hook_state::cleanup_stale(14 * 24 * 60 * 60);

    let path = settings_path(project);
    match ensure_hooks_installed(project) {
        Ok(installed) if installed.is_empty() => {}
        Ok(installed) => {
            println!("claudectl: hooks updated in {}", path.display());
            for spec in &installed {
                println!("  + installed {}", spec.label());
            }
            let already = HOOKS.len() - installed.len();
            if already > 0 {
                println!("  ({already} already up to date)");
            }
        }
        Err(e) => {
            eprintln!(
                "claudectl: warning — could not update hooks in {}: {e}",
                path.display()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_hooks_value() {
        let hooks = build_hooks_value();
        let obj = hooks.as_object().unwrap();

        // Should have entries for PreToolUse, PostToolUse, and Stop
        assert!(obj.contains_key("PreToolUse"));
        assert!(obj.contains_key("PostToolUse"));
        assert!(obj.contains_key("Stop"));

        // Each event should have an array of matcher entries
        for (_event, matchers) in obj {
            let arr = matchers.as_array().unwrap();
            assert!(!arr.is_empty());
            for entry in arr {
                assert!(entry.get("matcher").is_some());
                assert!(entry.get("hooks").is_some());
                let inner = entry["hooks"].as_array().unwrap();
                assert_eq!(inner[0]["type"], "command");
                assert!(inner[0]["command"].as_str().unwrap().contains("claudectl"));
            }
        }
    }

    #[test]
    fn test_has_claudectl_hooks_empty() {
        let settings = serde_json::json!({});
        assert!(!has_claudectl_hooks(&settings));
    }

    #[test]
    fn test_has_claudectl_hooks_present() {
        let settings = serde_json::json!({
            "hooks": {
                "PostToolUse": [{
                    "matcher": "*",
                    "hooks": [{
                        "type": "command",
                        "command": "claudectl --json 2>/dev/null || true",
                        "timeout": 5
                    }]
                }]
            }
        });
        assert!(has_claudectl_hooks(&settings));
    }

    #[test]
    fn test_has_claudectl_hooks_other_hooks_only() {
        let settings = serde_json::json!({
            "hooks": {
                "PreToolUse": [{
                    "matcher": "Bash",
                    "hooks": [{
                        "type": "command",
                        "command": "echo hello",
                        "timeout": 5
                    }]
                }]
            }
        });
        assert!(!has_claudectl_hooks(&settings));
    }

    #[test]
    fn test_merge_hooks_empty() {
        let mut settings = serde_json::json!({});
        merge_hooks(&mut settings);

        assert!(settings.get("hooks").is_some());
        let hooks = settings["hooks"].as_object().unwrap();
        for spec in HOOKS {
            assert!(
                hooks.contains_key(spec.event),
                "expected event {} after merge",
                spec.event
            );
        }
    }

    #[test]
    fn find_missing_hooks_on_empty_returns_all_specs() {
        let settings = serde_json::json!({});
        let missing = find_missing_hooks(&settings);
        assert_eq!(missing.len(), HOOKS.len());
    }

    #[test]
    fn find_missing_hooks_returns_only_drift() {
        let mut settings = serde_json::json!({});
        merge_hooks(&mut settings);
        // Manually drop one event to simulate drift.
        settings["hooks"]
            .as_object_mut()
            .unwrap()
            .remove("Notification");
        let missing = find_missing_hooks(&settings);
        // Two Notification specs (permission_prompt, idle_prompt) are now missing.
        assert_eq!(missing.len(), 2);
        assert!(missing.iter().all(|s| s.event == "Notification"));
    }

    #[test]
    fn find_missing_hooks_returns_nothing_when_complete() {
        let mut settings = serde_json::json!({});
        merge_hooks(&mut settings);
        let missing = find_missing_hooks(&settings);
        assert!(missing.is_empty());
    }

    #[test]
    fn test_merge_hooks_preserves_existing() {
        let mut settings = serde_json::json!({
            "allowedTools": ["Bash", "Read"],
            "hooks": {
                "PreToolUse": [{
                    "matcher": "Write",
                    "hooks": [{
                        "type": "command",
                        "command": "echo validate-write",
                        "timeout": 10
                    }]
                }]
            }
        });

        merge_hooks(&mut settings);

        // Existing allowedTools preserved
        assert_eq!(
            settings["allowedTools"],
            serde_json::json!(["Bash", "Read"])
        );

        // Existing PreToolUse Write hook preserved alongside the new wildcard one
        let pre = settings["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre.len(), 2); // original Write + new "*"
        assert_eq!(pre[0]["matcher"], "Write");
        assert_eq!(pre[1]["matcher"], "*");

        // Every spec from HOOKS shows up
        for spec in HOOKS {
            assert!(
                settings["hooks"][spec.event].is_array(),
                "expected event {} to be installed",
                spec.event
            );
        }
    }

    #[test]
    fn test_run_init_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let settings_file = dir.path().join(".claude/settings.local.json");

        // Temporarily override HOME so settings_path uses our temp dir
        // We test the file-writing logic directly instead
        let parent = settings_file.parent().unwrap();
        std::fs::create_dir_all(parent).unwrap();

        let mut settings = serde_json::json!({});
        merge_hooks(&mut settings);

        let json = serde_json::to_string_pretty(&settings).unwrap();
        std::fs::write(&settings_file, format!("{json}\n")).unwrap();

        // Verify the file was created and is valid JSON
        let content = std::fs::read_to_string(&settings_file).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(parsed.get("hooks").is_some());
        assert!(has_claudectl_hooks(&parsed));
    }

    #[test]
    fn test_settings_path_global() {
        let path = settings_path(false);
        let path_str = path.to_string_lossy();
        assert!(path_str.ends_with(".claude/settings.json"));
    }

    #[test]
    fn test_settings_path_project() {
        let path = settings_path(true);
        assert_eq!(path, PathBuf::from(".claude/settings.local.json"));
    }

    #[test]
    fn test_remove_claudectl_hooks_all() {
        let mut settings = serde_json::json!({});
        merge_hooks(&mut settings);
        assert!(has_claudectl_hooks(&settings));

        let removed = remove_claudectl_hooks(&mut settings);
        assert_eq!(removed, HOOKS.len());
        assert!(!has_claudectl_hooks(&settings));
        // hooks key removed entirely when empty
        assert!(settings.get("hooks").is_none());
    }

    #[test]
    fn test_remove_claudectl_hooks_preserves_others() {
        let mut settings = serde_json::json!({
            "allowedTools": ["Bash"],
            "hooks": {
                "PreToolUse": [
                    {
                        "matcher": "Write",
                        "hooks": [{
                            "type": "command",
                            "command": "echo validate-write",
                            "timeout": 10
                        }]
                    },
                    {
                        "matcher": "Bash",
                        "hooks": [{
                            "type": "command",
                            "command": "claudectl --json 2>/dev/null || true",
                            "timeout": 5
                        }]
                    }
                ],
                "PostToolUse": [{
                    "matcher": "*",
                    "hooks": [{
                        "type": "command",
                        "command": "claudectl --json 2>/dev/null || true",
                        "timeout": 5
                    }]
                }]
            }
        });

        let removed = remove_claudectl_hooks(&mut settings);
        assert_eq!(removed, 2); // Bash from PreToolUse + PostToolUse entry

        // Write hook in PreToolUse preserved
        let pre = settings["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre.len(), 1);
        assert_eq!(pre[0]["matcher"], "Write");

        // PostToolUse event removed entirely (was only claudectl)
        assert!(settings["hooks"].get("PostToolUse").is_none());

        // allowedTools untouched
        assert_eq!(settings["allowedTools"], serde_json::json!(["Bash"]));
    }

    #[test]
    fn test_remove_claudectl_hooks_noop_when_absent() {
        let mut settings = serde_json::json!({
            "hooks": {
                "PreToolUse": [{
                    "matcher": "Bash",
                    "hooks": [{
                        "type": "command",
                        "command": "echo hello",
                        "timeout": 5
                    }]
                }]
            }
        });

        let removed = remove_claudectl_hooks(&mut settings);
        assert_eq!(removed, 0);
        // Original hook still present
        assert!(settings["hooks"]["PreToolUse"].as_array().unwrap().len() == 1);
    }

    #[test]
    fn test_remove_then_no_hooks_key() {
        // Settings that only had claudectl hooks — hooks key should be removed entirely
        let mut settings = serde_json::json!({ "permissions": {} });
        merge_hooks(&mut settings);
        remove_claudectl_hooks(&mut settings);

        assert!(settings.get("hooks").is_none());
        // Other keys preserved
        assert!(settings.get("permissions").is_some());
    }

    #[test]
    fn test_init_uninit_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let settings_file = dir.path().join("settings.json");

        // Start with existing settings
        let original = serde_json::json!({
            "allowedTools": ["Read", "Glob"],
            "hooks": {
                "SessionStart": [{
                    "matcher": "*",
                    "hooks": [{
                        "type": "command",
                        "command": "echo started",
                        "timeout": 5
                    }]
                }]
            }
        });
        let json = serde_json::to_string_pretty(&original).unwrap();
        std::fs::write(&settings_file, &json).unwrap();

        // Init: merge claudectl hooks in
        let content = std::fs::read_to_string(&settings_file).unwrap();
        let mut settings: serde_json::Value = serde_json::from_str(&content).unwrap();
        merge_hooks(&mut settings);
        let json = serde_json::to_string_pretty(&settings).unwrap();
        std::fs::write(&settings_file, &json).unwrap();
        assert!(has_claudectl_hooks(&settings));

        // Uninit: remove claudectl hooks
        let content = std::fs::read_to_string(&settings_file).unwrap();
        let mut settings: serde_json::Value = serde_json::from_str(&content).unwrap();
        remove_claudectl_hooks(&mut settings);

        // Back to original state
        assert!(!has_claudectl_hooks(&settings));
        assert_eq!(
            settings["allowedTools"],
            serde_json::json!(["Read", "Glob"])
        );
        let session_start = settings["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(session_start.len(), 1);
        assert_eq!(session_start[0]["hooks"][0]["command"], "echo started");
    }
}
