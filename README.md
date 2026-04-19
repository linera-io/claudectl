# claudectl

**Auto-pilot for Claude Code.**

Fully local on-device model that learns and decides what to approve — no cloud API, no telemetry. Plus orchestration, health monitoring, spend control, and highlight-reels.

[![CI](https://github.com/mercurialsolo/claudectl/actions/workflows/ci.yml/badge.svg)](https://github.com/mercurialsolo/claudectl/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/claudectl)](https://crates.io/crates/claudectl)
[![Homebrew](https://img.shields.io/badge/homebrew-mercurialsolo%2Ftap-orange)](https://github.com/mercurialsolo/homebrew-tap)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Downloads](https://img.shields.io/crates/d/claudectl)](https://crates.io/crates/claudectl)
[![Platforms](https://img.shields.io/badge/platform-macOS%20%7C%20Linux-lightgrey)]()

<sub>~1 MB binary. Sub-50ms startup. Zero config required.</sub>

[Website](https://mercurialsolo.github.io/claudectl/) | [Demo](https://asciinema.org/a/bovJrUq2vEmC08NU) | [Blog: Why a local brain?](blog/local-brain-architecture.md) | [Releases](https://github.com/mercurialsolo/claudectl/releases)

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
claudectl --init                          # Wire up Claude Code hooks (one-time)
claudectl --demo                          # Fake sessions, no Claude needed
claudectl                                 # Live dashboard
claudectl --brain                         # Local LLM auto-pilot
claudectl --mode auto                     # Let the brain run fully unattended
claudectl --new --cwd ./myproject         # Launch a new session
claudectl --run tasks.json --parallel     # Orchestrate multiple sessions
claudectl --decompose "Add auth and update tests and docs"  # Split into parallel tasks
```

## Why claudectl

| Capability | Claude Code alone | With claudectl |
|-----------|:-:|:-:|
| One-command setup | No | **`claudectl --init`** |
| Local LLM auto-approve/deny | No | **Brain with ollama** |
| Session health monitoring | No | **Cognitive rot, cache, cost spikes, loops, stalls, context** |
| Record session highlight reels | No | **Press `R`** |
| Orchestrate multi-session workflows | No | **Dependency-ordered tasks** |
| Launch/resume sessions | Separate terminal | **Press `n` or `--new`** |
| See status of all sessions at once | No | **Yes** |
| Know which session is blocked | Tab-hunt | **At a glance** |
| Track cost per session | Manually | **Live $/hr burn rate** |
| Enforce spend budgets | No | **Auto-kill at limit** |
| File conflict detection | No | **Auto-detect + brain pre-check + auto-deny** |
| Idle mode / unattended work | No | **Run tasks while you sleep** |
| Session auto-restart | No | **Checkpoint + restart on context saturation** |
| Task decomposition | No | **`--decompose` splits prompts into parallel DAGs** |
| Auto-rule engine | No | **Match by tool/command/project/cost** |
| Approve prompts without switching | No | **Press `y`** |
| Get notified on stalls/blocks | No | **Desktop + webhook** |
| Auto-insights (friction, rules, cost trends) | No | **Self-improving brain** |
| Claude Code plugin with `/brain`, `/sessions`, `/spend` | No | **Built-in plugin** |
| Toggle brain on/off/auto mid-session | No | **`--mode off` or `/brain off`** |

## Local LLM Brain

The brain is claudectl's core intelligence layer. A local LLM continuously observes all your sessions — what they're doing, what tools they're calling, how much they're spending — and makes real-time decisions:

- **Approve** safe tool calls automatically (reads, greps, test runs)
- **Deny** dangerous operations before they execute (force pushes, destructive commands)
- **Terminate** sessions that are looping, stalled, or burning money
- **Route** summarized output between sessions so they share context
- **Spawn** new sessions when the brain detects parallelizable work
- **Delegate** tasks to external agents (Codex, Aider, custom tools)

The brain **continuously learns** from everything you do — not just brain-involved decisions, but every manual approve, reject, input, rule execution, and conflict resolution. These signals are distilled into compact conditional preferences and injected into the LLM prompt, so the brain's judgment compounds over time. All data stays on your machine — no cloud API, no telemetry.

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

**How the brain learns:**

The brain captures rich context at every decision point and distills it into compact rules that fit Gemma4's context window (~250 tokens). Four levels of learning work together:

| Level | What it does | Example |
|-------|-------------|---------|
| **Context capture** | Records 13 session state fields (cost, context%, errors, burn rate, files, conflicts) with every decision | `cost_usd: 14.50, context_pct: 82, recent_error_count: 3` |
| **Conditional preferences** | Learns context-dependent rules via decision tree splits | `approve [Bash] "git push" when cost<$5 (n=8)` |
| **Outcome tracking** | Correlates consecutive decisions to detect "approved but broke" | Downweights false-positive approvals, reinforces correct rejections |
| **Temporal patterns** | Detects behavioral sequences across decisions | `After 3+ errors: user usually denies (n=12)` |
| **Time-of-day** | Learns work-hours vs off-hours approval behavior | `More permissive during work hours (accept 90% vs 40%)` |
| **Per-project models** | Distills separate preferences per project | `[Read] always approve in frontend, usually deny in infra` |

The brain learns passively from all user actions, not just brain-involved decisions:

| Your action | What the brain learns |
|---|---|
| Press `y` (approve) | "This tool+command at this cost/context level is safe" |
| Press `B` (reject brain) | "Brain was wrong here — correction signal" (weighted 8x) |
| Press `i` (send input) | "Session needed human guidance at this point" |
| Static rule fires | "This pattern should be internalized" |
| File conflict deny | "Concurrent edits to this file = deny" |

Adaptive confidence thresholds track accuracy per tool — if the brain is 90%+ accurate on Read, it auto-executes with low confidence (0.5). If it's <50% accurate on Bash, it requires 0.95 confidence or defers to you.

**What the brain sees per session:**
- Project name, status, model, pending tool call + command
- Cost, burn rate, context window utilization
- **Git state** — branch, uncommitted changes, diff stats, recent commits (cached, 30s TTL)
- Recent transcript (last 8 messages, earlier ones compacted)
- All other active sessions (for cross-session reasoning)
- **Per-project preferences** — distilled from project-specific decision history (falls back to global with <10 decisions)
- Situational rules (error streaks, cost pressure, context pressure, **time-of-day patterns**)
- Outcome-weighted few-shot examples (corrections weighted highest)

**Measure brain effectiveness:**

```bash
claudectl --brain-stats impact           # Impact scorecard — your headline numbers
claudectl --brain-stats learning-curve   # Is correction rate declining? (= learning)
claudectl --brain-stats accuracy         # Per-tool, per-risk, per-project breakdown
claudectl --brain-stats baseline         # Brain vs. dumb rules classifier
claudectl --brain-stats false-approve    # Safety: how often does brain approve risky actions?
```

The impact scorecard shows what claudectl is doing for you:

```
Impact Scorecard
=================

  Interruptions avoided
    847/1200 tool calls handled without interruption (71%)
    353 required manual review (29%)

  Decision coverage
    Brain: 100% of tool calls (1200/1200)
    Static rules: 34% of tool calls (408/1200)
    Brain covers 2.9x more decisions than rules alone

  Safety
    12 dangerous operations blocked
      2 critical (rm -rf, force push, etc.)
      10 high-risk (git push, sudo, etc.)
    False-approve rate on risky actions: 0.0% (0/38)

  Brain accuracy
    96.2% correct (1154/1200 decisions)
    Correction rate: 8.4% -> 2.1% (+6.3pp improvement)

  Estimated time saved
    ~42.4 minutes (847 auto-handled tool calls x 3s each)
```

**Auto-insights — self-improving sessions:**

The brain automatically detects friction patterns and suggests improvements to your workflow:

```bash
claudectl --brain --insights              # View current insights
claudectl --brain --insights on           # Enable auto-generation (every 10 decisions)
claudectl --brain --insights off          # Disable auto-generation
claudectl --brain --insights status       # Show current mode
```

Insights are generated from your decision history — no LLM call needed. The system detects:
- **Friction patterns** — tools/commands you keep rejecting (suggests deny rules)
- **Error loops** — same tool failing repeatedly across sessions
- **Context blowouts** — sessions frequently hitting high context usage
- **Missing rules** — high-confidence patterns that should be AutoRules
- **Accuracy gaps** — tools where the brain needs more training data
- **Cost trends** — burn rate increases and cost outlier sessions

Only new insights are surfaced — the system tracks what you've already seen. When auto-generation is on, insights are produced alongside preference distillation in the background. Use the `/auto-insights` plugin command to access this inside Claude Code sessions.

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

**File conflict pre-check** — before auto-approving Write/Edit calls, the brain checks if another session has the target file in its edit history. Conflicts are demoted to advisory mode, requiring your confirmation.

### Brain Gate Mode

Control the brain's behavior mid-session without restarting:

```bash
claudectl --mode on                       # Brain evaluates tool calls (default)
claudectl --mode off                      # Disable brain — all calls pass through
claudectl --mode auto                     # Auto-approve above confidence threshold
claudectl --mode status                   # Show current mode
```

In **on** mode, the brain denies dangerous operations and approves safe ones. Low-confidence decisions fall through to normal permission prompts. In **auto** mode, all brain approvals execute without prompts. In **off** mode, the brain is bypassed entirely.

The mode persists across sessions (stored in `~/.claudectl/brain/gate-mode`). If you use the Claude Code plugin, the `/brain` command does the same thing inline.

## Claude Code Plugin

claudectl ships with a Claude Code plugin that integrates the brain directly into your sessions — no TUI required.

```
claude-plugin/
├── hooks/        # PreToolUse hook that queries the brain on every tool call
├── commands/     # /sessions, /spend, /brain-stats, /brain, /auto-insights
├── agents/       # Session supervisor agent for health triage
└── skills/       # Auto-activated session monitoring awareness
```

**What the plugin provides:**

| Component | What it does |
|-----------|-------------|
| **Brain gate hook** | Queries the brain before every Bash/Write/Edit call. Approves safe ops, denies dangerous ones. |
| `/brain on\|off\|auto` | Toggle brain mode mid-session |
| `/sessions` | Show all active sessions with status, cost, health |
| `/spend` | Cost breakdown by project and time window |
| `/brain-stats` | Brain learning metrics and accuracy |
| `/auto-insights` | Show or configure auto-generated workflow insights |
| **Supervisor agent** | Proactive health triage across all sessions |

**Install:** Copy or symlink the `claude-plugin/` directory to your Claude Code plugins path, or point Claude Code at the directory.

The plugin works standalone (no TUI needed) or alongside the dashboard. The brain gate hook and the TUI brain share the same decision history and learned preferences.

## Idle Mode

When you step away, claudectl detects inactivity and can run pre-configured low-risk tasks:

```toml
# .claudectl.toml
[idle]
enabled = true
after_idle_mins = 15         # Transition to idle after 15 minutes
max_concurrent = 2           # Max parallel idle tasks
max_cost_usd = 5.0           # Budget cap for idle work
```

The status bar shows idle state and elapsed time. On your first keypress back, a morning report summarizes what happened while you were away.

## Session Lifecycle

Long-running sessions degrade as their context window fills. claudectl can auto-restart them:

```toml
# .claudectl.toml
[lifecycle]
auto_restart = true          # Enable auto-restart on context saturation
restart_threshold_pct = 90.0 # Restart when context exceeds 90%
restart_only_when_idle = true # Only restart during idle mode
```

When triggered, the brain summarizes the session state, saves a checkpoint to `~/.claudectl/brain/checkpoints/`, and spawns a fresh session with the summary as context.

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

**Auto-decompose prompts** — let the brain split large prompts into parallel sub-tasks:

```bash
# Analyze a prompt and output a task DAG (pipe to --run)
claudectl --decompose "Add JWT auth, write tests for all endpoints, and update the API docs"

# Decompose and run in one pipeline
claudectl --decompose "..." > tasks.json && claudectl --run tasks.json --parallel
```

The decomposition prompt template is user-overridable via `~/.claudectl/brain/prompts/decomposition.md`.

## Session Health Monitoring

claudectl continuously checks each session for problems and surfaces them with severity-ranked icons in the dashboard:

- **Cognitive rot detection** — composite 0-100 decay score that tracks degradation over time: token efficiency decline, error acceleration, file re-read repetition, and context pressure. Icons: `◐` early, `◉` significant, `⊘` severe
- **Proactive compaction** — suggests `/compact` at 50% context (research shows degradation starts at 40-50%), before the existing 80/90% thresholds fire
- **Cache health** — detects low cache hit ratios that can silently multiply costs
- **Cost spikes** — flags when burn rate exceeds the session average
- **Loop detection** — catches tools failing repeatedly in retry loops
- **Stall detection** — sessions spending money but producing no file edits
- **Context saturation** — warns when a session approaches its context window limit

The detail panel shows a **Cognitive Health** section with decay score, efficiency vs baseline, error trend, repetition count, and actionable mitigation suggestions.

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

## Uninstall

Remove claudectl hooks from Claude Code:

```bash
claudectl --uninstall                      # Remove from user settings
claudectl --uninstall -s project           # Remove from project settings
```

This surgically removes only claudectl entries — all other settings and hooks are preserved.

## Comparison

claudectl was the first tool to combine local LLM supervision with multi-session orchestration for Claude Code (shipped April 2026). Here's how it compares:

| Feature | claudectl | Static auto-approve tools | Cloud-based supervisors |
|---------|:---------:|:-------------------------:|:-----------------------:|
| Local LLM brain that learns your preferences | Yes | No | No |
| Cross-session orchestration + context routing | Yes | No | Varies |
| Cognitive rot / health monitoring | Yes | No | No |
| File conflict detection across sessions | Yes | No | No |
| Per-tool adaptive confidence thresholds | Yes | No | No |
| Task decomposition into parallel DAGs | Yes | No | No |
| Binary size | <1 MB | Varies | N/A |
| Startup time | <50 ms | Varies | N/A |
| Data stays on your machine | 100% | Usually | No |
| Learns from every correction | Yes | No | No |
| Claude Code plugin with inline brain | Yes | No | No |
| Toggle brain on/off/auto mid-session | Yes | No | No |

## Docs

| | |
|---|---|
| [Quick Start](docs/quickstart.md) | Install, init, first dashboard, uninstall |
| [Reference](docs/reference.md) | Dashboard features, keybindings, CLI modes, status detection |
| [Configuration](docs/configuration.md) | Config files, hooks, rules, model pricing overrides |
| [Terminal Support](docs/terminal-support.md) | Compatibility matrix and setup notes |
| [Troubleshooting](docs/troubleshooting.md) | Common issues and FAQ |
| [Contributing](docs/contributing.md) | Setup, guidelines, and architecture |
| [Changelog](CHANGELOG.md) | Release history |

## Community

- Questions or ideas? [Start a Discussion](https://github.com/mercurialsolo/claudectl/discussions)
- Found a bug? [Open an issue](https://github.com/mercurialsolo/claudectl/issues/new)
- Share your setup, brain stats, or workflows in [Show & Tell](https://github.com/mercurialsolo/claudectl/discussions/categories/show-and-tell)

## License

MIT
