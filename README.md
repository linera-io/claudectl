# claudectl

**Auto-pilot for Claude Code.**

Fully local on-device model that learns and decide what to approve - no cloud API, no telemetry. +orchestration, health monitoring, spend control, and highlight-reels.

[![CI](https://github.com/mercurialsolo/claudectl/actions/workflows/ci.yml/badge.svg)](https://github.com/mercurialsolo/claudectl/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/claudectl)](https://crates.io/crates/claudectl)
[![Homebrew](https://img.shields.io/badge/homebrew-mercurialsolo%2Ftap-orange)](https://github.com/mercurialsolo/homebrew-tap)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Platforms](https://img.shields.io/badge/platform-macOS%20%7C%20Linux-lightgrey)]()

<sub>~1 MB binary. Sub-50ms startup. Zero config required.</sub>

[Website](https://mercurialsolo.github.io/claudectl/) | [Demo](https://asciinema.org/a/bovJrUq2vEmC08NU) | [Releases](https://github.com/mercurialsolo/claudectl/releases)

<a href="https://asciinema.org/a/bovJrUq2vEmC08NU?autoplay=1"><img src="https://asciinema.org/a/bovJrUq2vEmC08NU.svg" alt="claudectl demo" width="100%" /></a>

## Install

```bash
brew install mercurialsolo/tap/claudectl     # Homebrew (macOS / Linux)
cargo install claudectl                       # Cargo (any platform)
```

<details>
<summary>Other methods</summary>

```bash
curl -fsSL https://raw.githubusercontent.com/mercurialsolo/claudectl/main/install.sh | sh
nix run github:mercurialsolo/claudectl
git clone https://github.com/mercurialsolo/claudectl.git && cd claudectl && cargo install --path .
```

</details>

## Try it now

```bash
claudectl --demo                          # Fake sessions, no Claude needed
claudectl                                 # Live dashboard
claudectl --brain                         # Local LLM auto-pilot
claudectl --new --cwd ./myproject         # Launch a new session
claudectl --run tasks.json --parallel     # Orchestrate multiple sessions
```

## Why claudectl

| Capability | Claude Code alone | With claudectl |
|-----------|:-:|:-:|
| Local LLM auto-approve/deny | No | **Brain with ollama** |
| Session health monitoring | No | **Cache, cost spikes, loops, stalls, context** |
| Record session highlight reels | No | **Press `R`** |
| Orchestrate multi-session workflows | No | **Dependency-ordered tasks** |
| Launch/resume sessions | Separate terminal | **Press `n` or `--new`** |
| See status of all sessions at once | No | **Yes** |
| Know which session is blocked | Tab-hunt | **At a glance** |
| Track cost per session | Manually | **Live $/hr burn rate** |
| Enforce spend budgets | No | **Auto-kill at limit** |
| File conflict detection | No | **Auto-detect + auto-deny** |
| Auto-rule engine | No | **Match by tool/command/project/cost** |
| Approve prompts without switching | No | **Press `y`** |
| Get notified on stalls/blocks | No | **Desktop + webhook** |

## Local LLM Brain

The brain is claudectl's core intelligence layer. A local LLM continuously observes all your sessions — what they're doing, what tools they're calling, how much they're spending — and makes real-time decisions:

- **Approve** safe tool calls automatically (reads, greps, test runs)
- **Deny** dangerous operations before they execute (force pushes, destructive commands)
- **Terminate** sessions that are looping, stalled, or burning money
- **Route** summarized output between sessions so they share context
- **Spawn** new sessions when the brain detects parallelizable work
- **Delegate** tasks to external agents (Codex, Aider, custom tools)

The brain **continuously learns** from your corrections. Every accept/reject is logged, distilled into compact preference patterns, and used to adapt future decisions. Accuracy is tracked per tool — if the brain keeps getting Bash wrong, it raises the confidence bar before auto-executing. All data stays on your machine — no cloud API, no telemetry.

```bash
# Start with one command (requires ollama)
ollama pull gemma4:e4b && ollama serve
claudectl --brain

# Advisory mode (default): brain suggests, you press b/B to accept/reject
claudectl --brain

# Auto mode: brain executes decisions without asking
claudectl --brain --auto-run
```

**Supported backends:**

| Backend | Setup | Default endpoint |
|---------|-------|-----------------|
| [ollama](https://ollama.com) | `ollama pull gemma4:e4b && ollama serve` | `localhost:11434` |
| [llama.cpp](https://github.com/ggerganov/llama.cpp) | `llama-server -m model.gguf` | `localhost:8080` |
| [vLLM](https://github.com/vllm-project/vllm) | `vllm serve gemma4` | `localhost:8000` |
| [LM Studio](https://lmstudio.ai) | Start server in UI | `localhost:1234` |

Any endpoint that accepts a JSON POST and returns generated text will work.

**What the brain sees per session:**
- Project name, status, model, pending tool call + command
- Cost, burn rate, context window utilization
- Recent transcript (last 8 messages, earlier ones compacted)
- All other active sessions (for cross-session reasoning)
- Distilled preference patterns (compact rules learned from your history)
- Outcome-weighted few-shot examples (corrections weighted highest)
- Per-tool adaptive confidence thresholds

**Diagnostics and customization:**

```bash
claudectl --doctor          # Check if backend is reachable
claudectl --brain-eval      # Test decision quality against built-in scenarios
claudectl --brain-prompts   # List prompt templates and their source
```

```toml
# .claudectl.toml
[brain]
enabled = true
endpoint = "http://localhost:11434/api/generate"
model = "gemma4:e4b"
auto = false                # true = auto-execute suggestions
few_shot_count = 5          # Past decisions to include as examples
max_sessions = 10           # Max sessions brain can spawn
orchestrate = false         # Enable cross-session orchestration
orchestrate_interval = 30   # Seconds between orchestration passes
```

Override any prompt template by placing files in `~/.claudectl/brain/prompts/`.

## Record and Share

**Highlight reels** — Press `R` on any session. claudectl extracts file edits, bash commands, errors, and successes. Idle time and noise are stripped. Output is a shareable GIF.

**Dashboard recording** — Capture the full TUI as a GIF or asciicast:

```bash
claudectl --record session.gif             # GIF (requires agg)
claudectl --demo --record demo.gif         # One-command demo GIF for your README
```

## Orchestrate Sessions

Run coordinated tasks with dependency ordering, retries, cross-session data routing, and resumable sessions:

```json
{
  "tasks": [
    { "name": "auth", "cwd": "./backend", "prompt": "Add JWT auth middleware" },
    { "name": "tests", "cwd": "./backend", "prompt": "Update API tests for auth. Previous output: {{auth.stdout}}", "depends_on": ["auth"] },
    { "name": "docs", "cwd": "./docs", "prompt": "Document the new auth flow", "depends_on": ["auth"] }
  ]
}
```

```bash
claudectl --run tasks.json --parallel
```

## Session Health Monitoring

claudectl continuously checks each session for problems and surfaces them with severity-ranked icons in the dashboard:

- **Cache health** — detects low cache hit ratios that can silently multiply costs
- **Cost spikes** — flags when burn rate exceeds the session average
- **Loop detection** — catches tools failing repeatedly in retry loops
- **Stall detection** — sessions spending money but producing no file edits
- **Context saturation** — warns when a session approaches its context window limit

Health issues appear as icons in the session table and as a summary in the status bar. No configuration needed.

## File Conflict Detection

When multiple sessions edit the same file, claudectl detects the conflict and flags it:

- **`!F` prefix** in the session table for sessions with file-level conflicts
- **File Conflicts section** in the detail panel showing which files conflict and with which sessions
- **Predictive detection** — flags pending Edit/Write calls targeting files another session has already modified
- **Auto-deny** — optionally deny writes to conflicting files with an actionable message

```toml
# .claudectl.toml
[orchestrate]
file_conflicts = true              # Detect file-level conflicts (default: on)
auto_deny_file_conflicts = true    # Auto-deny conflicting writes (default: off)
```

File conflicts can also be matched in auto-rules:

```toml
[rules.deny_conflicts]
match_file_conflict = true
action = "deny"
message = "Another session is editing this file"
```

## Launch and Resume Sessions

Start new Claude Code sessions without leaving the dashboard:

```bash
claudectl --new --cwd ./backend                       # Launch in a directory
claudectl --new --cwd ./api --prompt "Add rate limiting"  # Launch with a prompt
claudectl --new --resume abc123                       # Resume a previous session
```

From the dashboard, press `n` to open the launch wizard (directory, prompt, resume fields).

## Auto-Rules

Define rules in `.claudectl.toml` to automatically approve, deny, terminate, or route sessions based on conditions:

```toml
[[rules]]
name = "approve-cargo"
match_tool = ["Bash"]
match_command = ["cargo"]
action = "approve"

[[rules]]
name = "deny-rm-rf"
match_command = ["rm -rf"]
action = "deny"

[[rules]]
name = "kill-runaway"
match_cost_above = 20.0
action = "terminate"
```

Rules support matching by status, tool name, command substring, project name, cost threshold, and error state. Deny rules always take precedence. Rules can also route output between sessions, spawn new sessions, or delegate to agents.

## Supervise and Control Spend

```bash
claudectl --budget 5 --kill-on-budget      # Auto-kill at $5
claudectl --notify                         # Desktop notifications on blocks/stalls
claudectl --webhook https://hooks.slack.com/... --webhook-on NeedsInput,Finished
claudectl --history --since 24h            # Review past session costs
claudectl --stats --since 24h             # Aggregated session statistics
claudectl --summary --since 8h            # Activity summary
```

From the dashboard: `y` approve, `i` input, `Tab` switch terminal, `d` kill, `n` new session, `R` record, `?` all keys.

## Filter and Search

```bash
claudectl --filter-status NeedsInput       # Only show sessions needing input
claudectl --focus attention                # High-signal triage view
claudectl --focus over-budget              # Sessions exceeding budget
claudectl --search "my-project"            # Filter by project name
claudectl --watch                          # Stream status changes (no TUI)
claudectl --watch --json                   # Stream as JSON
```

In the dashboard: `f` cycle status filters, `v` cycle focus filters, `/` search, `z` clear all filters, `g` group by project, `s` cycle sort order.

## Clean Up

```bash
claudectl --clean                          # Remove old session data
claudectl --clean --older-than 7d          # Only sessions older than 7 days
claudectl --clean --finished --dry-run     # Preview what would be removed
```

## Docs

| | |
|---|---|
| [Reference](docs/reference.md) | Dashboard features, keybindings, CLI modes, status detection |
| [Configuration](docs/configuration.md) | Config files, hooks, rules, model pricing overrides |
| [Terminal Support](docs/terminal-support.md) | Compatibility matrix and setup notes |
| [Troubleshooting](docs/troubleshooting.md) | Common issues and FAQ |
| [Contributing](docs/contributing.md) | Setup, guidelines, and architecture |
| [Changelog](CHANGELOG.md) | Release history |

## Community

Questions or ideas? [Start a Discussion](https://github.com/mercurialsolo/claudectl/discussions). Found a bug? [Open an issue](https://github.com/mercurialsolo/claudectl/issues/new).

## License

MIT
