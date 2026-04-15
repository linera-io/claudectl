# Configuration

claudectl loads settings from three layers (highest priority first):

1. **CLI flags** — override everything
2. **`.claudectl.toml`** — per-project config in the working directory
3. **`~/.config/claudectl/config.toml`** — global config

Show resolved config: `claudectl --config`

## Full Example

```toml
[defaults]
interval = 2000
notify = true
grouped = true
sort = "cost"
budget = 5.00
kill_on_budget = false

[budget]
daily_limit = 25.00
weekly_limit = 100.00

[webhook]
url = "https://hooks.slack.com/..."
events = ["NeedsInput", "Finished"]

[context]
warn_threshold = 75

[brain]
enabled = true
endpoint = "http://localhost:11434/api/generate"
model = "gemma4:e4b"
auto = false
timeout_ms = 5000
max_context_tokens = 4000
few_shot_count = 5

[models."gpt-4o"]
input_per_m = 1.25
output_per_m = 5.0
cache_read_per_m = 0.15
cache_write_per_m = 0.9
context_max = 128000
```

## Rule-Based Auto-Actions

Configure `[rules.*]` sections to automatically approve, deny, send messages, or terminate sessions based on tool name, command pattern, project, cost threshold, and error state.

Deny rules always override approve rules regardless of config order.

## Event Hooks

Run shell commands automatically when session events occur:

```toml
[hooks.on_needs_input]
run = "say 'Claude needs your attention'"

[hooks.on_finished]
run = "terminal-notifier -title 'claudectl' -message '{project} finished (${cost})'"

[hooks.on_budget_warning]
run = "curl -X POST $SLACK_WEBHOOK -d '{\"text\": \"{project} hit 80% budget (${cost})\"}'"

[hooks.on_status_change]
run = "echo '[{project}] {old_status} -> {new_status}' >> ~/claude-activity.log"
```

### Events

| Event | Trigger |
|-------|---------|
| `on_session_start` | New session discovered |
| `on_status_change` | Any status transition |
| `on_needs_input` | Session needs user approval/input |
| `on_finished` | Session process exited |
| `on_budget_warning` | Session hit 80% of budget |
| `on_budget_exceeded` | Session hit 100% of budget |
| `on_idle` | Session went idle (>10 min) |
| `on_context_high` | Context window usage crossed threshold (default 75%) |
| `on_conflict_detected` | 2+ sessions share the same working directory |

### Template Variables

`{pid}`, `{project}`, `{status}`, `{cost}`, `{model}`, `{cwd}`, `{tokens_in}`, `{tokens_out}`, `{elapsed}`, `{session_id}`, `{old_status}`, `{new_status}`, `{context_pct}`

Use `claudectl --hooks` to verify your configured hooks.

### Verified Hooks

We maintain a curated set at [mercurialsolo/claudectl-hooks](https://github.com/mercurialsolo/claudectl-hooks). To submit a hook, [open an issue](https://github.com/mercurialsolo/claudectl-hooks/issues) with the config snippet, what it solves, and any dependencies.
