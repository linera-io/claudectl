# claudectl

**Mission control for Claude Code.**

Monitor multiple Claude Code sessions in one terminal dashboard. Catch blocked agents, control token burn, approve actions, and orchestrate work across tmux, iTerm2, Ghostty, Warp, and more.

[![CI](https://github.com/mercurialsolo/claudectl/actions/workflows/ci.yml/badge.svg)](https://github.com/mercurialsolo/claudectl/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/claudectl)](https://crates.io/crates/claudectl)
[![Homebrew](https://img.shields.io/badge/homebrew-mercurialsolo%2Ftap-orange)](https://github.com/mercurialsolo/homebrew-tap)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Platforms](https://img.shields.io/badge/platform-macOS%20%7C%20Linux-lightgrey)]()

<sub>~1 MB binary. Sub-50ms startup. Zero config required.</sub>

[![claudectl demo](https://asciinema.org/a/bovJrUq2vEmC08NU.svg)](https://asciinema.org/a/bovJrUq2vEmC08NU)

## Install

```bash
brew install mercurialsolo/tap/claudectl     # Homebrew (macOS)
cargo install claudectl                       # Cargo (any platform)
```

<details>
<summary>Other methods</summary>

**Quick install (macOS / Linux)**

```bash
curl -fsSL https://raw.githubusercontent.com/mercurialsolo/claudectl/main/install.sh | sh
```

**Nix**

```bash
nix run github:mercurialsolo/claudectl
```

**From source**

```bash
git clone https://github.com/mercurialsolo/claudectl.git
cd claudectl && cargo install --path .
```

</details>

## Quickstart

```bash
# Make sure you have at least one Claude Code session running, then:
claudectl
```

That's it. claudectl auto-discovers all running Claude Code sessions and shows a live dashboard. No configuration needed.

From the dashboard you can:
- Press `y` to approve a blocked permission prompt
- Press `i` to type input to a session
- Press `Tab` to jump to a session's terminal tab
- Press `d` to kill a runaway session
- Press `?` for all keybindings

## Why This Exists

Claude Code is excellent at execution. It is not built to supervise many concurrent sessions.

When you run 3, 5, or 8 sessions across terminals, real problems appear:

- **Which session is blocked?** One is waiting for approval and you don't know which tab.
- **Which session is burning money?** You can't see token spend without switching to each one.
- **Which session needs input?** A permission prompt has been sitting for 10 minutes.
- **Which session is stalled?** It looks busy but CPU is zero.

claudectl is the operator layer that answers these questions from one pane.

## Claude Code vs claudectl

| Capability | Claude Code alone | With claudectl |
|-----------|:-:|:-:|
| Run a single session | Yes | Yes |
| See status of all sessions at once | No | **Yes** |
| Know which session is blocked | Tab-hunt | **At a glance** |
| Track cost per session | Manually | **Live $/hr burn rate** |
| Enforce spend budgets | No | **Auto-kill at limit** |
| Approve prompts without switching | No | **Press `y`** |
| Get notified on stalls/blocks | No | **Desktop + webhook** |
| Orchestrate multi-session workflows | No | **Dependency-ordered tasks** |
| Record session highlight reels | No | **Press `R`** |

## Core Workflows

### Supervise multiple sessions

Launch `claudectl` and see every running session: status, cost, burn rate, CPU, context window usage, token counts, and activity sparkline — all updating live.

```bash
claudectl                   # Interactive TUI dashboard
claudectl --watch           # Stream status changes (no TUI)
claudectl --list            # Print session table and exit
claudectl --json            # Machine-readable output for scripting
claudectl --doctor          # Diagnose terminal control support and setup
claudectl --filter-status needs-input --search api
claudectl --watch --focus attention
```

### Control spend

Set per-session budgets. Get warned at 80%. Optionally auto-kill at 100%.

```bash
claudectl --budget 5 --kill-on-budget
```

Weekly and daily cost aggregation shows in the title bar. Use `--history` and `--stats` to review past spend:

```bash
claudectl --history --since 24h
claudectl --stats --since 7d
```

### Catch blockers instantly

Sessions needing approval show as **Needs Input** in magenta. Desktop notifications and webhooks alert you even when claudectl isn't focused:

```bash
claudectl --notify
claudectl --webhook https://hooks.slack.com/... --webhook-on NeedsInput,Finished
```

### Orchestrate multi-session work

Run coordinated tasks with dependency ordering, retries, and resumable sessions:

```json
{
  "retries": 1,
  "tasks": [
    {
      "name": "Add auth middleware",
      "cwd": "./backend",
      "prompt": "Add JWT auth middleware to all API routes",
      "retries": 2
    },
    {
      "name": "Update tests",
      "cwd": "./backend",
      "prompt": "Update API tests for the new auth middleware",
      "depends_on": ["Add auth middleware"]
    },
    {
      "name": "Update docs",
      "cwd": "./docs",
      "prompt": "Document the new auth flow",
      "resume": "session-123"
    }
  ]
}
```

```bash
claudectl --run tasks.json --parallel
```

Each run writes live progress to `.claudectl-runs/.../status.json`, final results to `.claudectl-runs/.../summary.json`, and per-attempt stdout/stderr logs for every task. Press `Ctrl-C` to abort a run cleanly.

### Record and share

**Highlight reels** — Press `R` on any session to record a supercut: file edits, bash commands, errors, and successes. Idle time and noise are stripped. Output is a shareable GIF.

**Dashboard recording** — Capture the full TUI:

```bash
claudectl --record session.gif      # Direct GIF (requires agg)
claudectl --record session.cast     # Raw asciicast v2
```

**Demo mode** — Deterministic fake sessions for screenshots and content:

```bash
claudectl --demo                    # Animated TUI with 8 fake sessions
claudectl --demo --record demo.gif  # One-command GIF for your README
```

## Features

### Dashboard
- Live table: PID, project, status, context %, cost, $/hr burn rate, elapsed, CPU%, memory, tokens, sparkline
- Detail panel (`Enter`) with full session metadata
- Grouped view (`g`) by project with aggregate stats
- Sort by status, context, cost, burn rate, or elapsed (`s`)
- Live triage filters: status cycle (`f`), focus cycle (`v`), text search (`/`), clear (`z`)
- Conflict detection when 2+ sessions share the same git worktree (`!!`)
- Permission wait time — shows how long sessions have been waiting, longest first

### Status Detection

Multi-signal inference from CPU usage, JSONL events, and timestamps:

| Status | Color | Meaning |
|--------|-------|---------|
| **Needs Input** | Magenta | Waiting for user to approve/confirm a tool use |
| **Processing** | Green | Actively generating or executing tools |
| **Waiting** | Yellow | Done responding, waiting for user's next prompt |
| **Unknown** | Blue | Session is alive, but transcript telemetry is missing or unsupported |
| **Idle** | Gray | No recent activity (>10 min since last message) |
| **Finished** | Red | Process exited |

### Cost Tracking & Budgets
- Per-session USD estimates (Opus, Sonnet, Haiku model pricing)
- Live $/hr burn rate
- Per-session budget alerts at 80%, auto-kill at 100%
- Daily/weekly aggregate cost tracking
- Session history with cost analytics

### Interactive Controls

| Key | Action |
|-----|--------|
| `j`/`k` or `Up`/`Down` | Navigate sessions |
| `Tab` | Switch to session's terminal tab |
| `Enter` | Toggle detail panel |
| `y` | Approve (send Enter to NeedsInput session) |
| `i` | Input mode (type text to session) |
| `d`/`x` | Kill session (double-tap to confirm) |
| `a` | Toggle auto-approve (double-tap to confirm) |
| `n` | Launch wizard for cwd, prompt, and resume (`tmux`, Kitty, WezTerm) |
| `g` | Toggle grouped view by project |
| `s` | Cycle sort column |
| `f` | Cycle status filter |
| `v` | Cycle focus filter (`attention`, budget, context, telemetry, conflicts) |
| `/` | Search project/model/session text |
| `z` | Clear all active filters |
| `c` | Send /compact to session (when idle) |
| `R` | Record session highlight reel (toggle) |
| `r` | Force refresh |
| `?` | Toggle help overlay |
| `q`/`Esc` | Quit |

Use `claudectl --doctor` to check the current terminal's launch/switch/input support, CLI dependencies, and setup requirements.

### Terminal Support

| Terminal | Tab Switch | Approve/Input | Method |
|----------|-----------|---------------|--------|
| **Ghostty** | Background | Background | Native AppleScript API |
| **Kitty** | Background | Background | `kitty @` remote control |
| **tmux** | Background | Background | `tmux send-keys` |
| **WezTerm** | Background | - | CLI JSON API |
| **Warp** | Focus switch | Focus switch | Command Palette + System Events |
| **iTerm2** | Focus switch | Focus switch | AppleScript + System Events |
| **Terminal.app** | Focus switch | Focus switch | AppleScript + System Events |

**Notes:** Ghostty has the best support — no config needed. Kitty requires `allow_remote_control yes` in config. Warp, iTerm2, and Terminal.app require macOS Automation/Accessibility permission. tmux is auto-detected. Run `claudectl --doctor` from the same terminal you use for Claude to verify the current setup.

### Themes
- Dark, light, and monochrome (`--theme`)
- Respects `NO_COLOR` environment variable

## Event Hooks

Run shell commands automatically when session events occur. Add to your config file:

```toml
# ~/.config/claudectl/config.toml

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

We maintain a curated set of verified hooks at [mercurialsolo/claudectl-hooks](https://github.com/mercurialsolo/claudectl-hooks). Submitted hooks are reviewed for security, reliability, and usefulness before being added.

To submit a hook, [open an issue](https://github.com/mercurialsolo/claudectl-hooks/issues) with the config snippet, what it solves, and any dependencies.

## Configuration

claudectl loads settings from `~/.config/claudectl/config.toml` (global) and `.claudectl.toml` (per-project). CLI flags override both.

```toml
[defaults]
interval = 1000
notify = true
grouped = true
sort = "cost"
budget = 5.00
kill_on_budget = false

[webhook]
url = "https://hooks.slack.com/..."
events = ["NeedsInput", "Finished"]

[context]
warn_threshold = 75

[models."gpt-4o"]
input_per_m = 1.25
output_per_m = 5.0
cache_read_per_m = 0.15
cache_write_per_m = 0.9
context_max = 128000
```

Show resolved config: `claudectl --config`

## Maintenance

```bash
claudectl --clean --older-than 7d --dry-run   # Preview cleanup
claudectl --clean --finished                    # Remove finished session data
```

## How It Works

claudectl reads Claude Code's local data — no API keys, no network access, no modifications to Claude Code:

- **`~/.claude/sessions/*.json`** — session metadata (PID, session ID, working directory, start time)
- **`~/.claude/projects/{slug}/*.jsonl`** — conversation logs with token usage and events
- **`ps`** — CPU%, memory, TTY for each process
- **`/tmp/claude-{uid}/{slug}/{sessionId}/tasks/`** — subagent task files

Status inference combines multiple signals: `waiting_for_task` events, CPU usage thresholds, `stop_reason` fields, and message recency.

## Troubleshooting

**No sessions found**
- Ensure Claude Code is running (`claude` in another terminal)
- Check that `~/.claude/sessions/` contains `.json` files
- Run `claudectl --log /tmp/claudectl.log` and check the log

**Tab switching doesn't work**
- Run `claudectl --doctor` first to see the detected terminal, missing prerequisites, and supported actions
- Ghostty: should work out of the box
- Kitty: add `allow_remote_control yes` to `~/.config/kitty/kitty.conf`
- Warp/iTerm2/Terminal.app: grant Automation/Accessibility permission in System Settings > Privacy & Security
- tmux: must be running inside a tmux session

**Cost shows $0.00**
- claudectl reads token usage from JSONL logs. If the session just started, wait for the first response to complete
- Check that `~/.claude/projects/` contains `.jsonl` files

**High CPU usage from claudectl itself**
- Increase the poll interval: `claudectl --interval 3000` (default is 1000ms)

For other issues, run with `--log` and [open an issue](https://github.com/mercurialsolo/claudectl/issues/new) with the log attached.

## FAQ

**Does claudectl modify Claude Code or its files?**
No. It is read-only. The only writes are to its own history file and log file.

**Does it need an API key?**
No. It reads local files on disk. No network access required (unless you configure webhooks).

**Does it work with Claude Code in VS Code / JetBrains?**
It monitors any Claude Code process, regardless of how it was launched. Terminal-specific features (tab switching, input) require a supported terminal.

**Can I use it with a single session?**
Yes, but the value increases with concurrency. If you run one session, you already know where it is.

**What about Windows?**
Not yet. macOS and Linux only. WSL support is planned.

## Security

claudectl runs entirely locally. It reads Claude Code's session files from disk and process data from `ps`. It does not:
- Send data to any server (unless you configure webhooks)
- Modify Claude Code's files or behavior
- Require API keys or authentication
- Run with elevated privileges

Webhook payloads contain session metadata (project name, cost, status). Review your webhook URL and event filters before enabling.

## Contributing

Contributions are welcome.

### Setup

```bash
git clone https://github.com/mercurialsolo/claudectl.git
cd claudectl
cargo build
cargo test --all-targets
```

### Before submitting

```bash
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

### Guidelines

- **No new dependencies** without strong justification — the project stays lightweight
- **Test behavior, not implementation** — focus on what the code does
- **Match existing patterns** — look at similar code before writing new code
- **Keep commits atomic** — one logical change per commit

Not all contributions are code. Hooks, docs, config presets, terminal compatibility fixes, and packaging help are all valuable.

### Architecture

| Module | Purpose |
|--------|---------|
| `session.rs` | Session data structures and formatting |
| `discovery.rs` | Session file scanning and JSONL path resolution |
| `monitor.rs` | JSONL parsing, token counting, status inference |
| `process.rs` | Process introspection via `ps` |
| `app.rs` | Core app state, refresh loop, event handling |
| `config.rs` | TOML config file loading and layering |
| `theme.rs` | Color palette and theme modes |
| `history.rs` | Session history persistence and analytics |
| `orchestrator.rs` | Multi-session task runner |
| `hooks.rs` | Event hooks system and execution |
| `logger.rs` | Diagnostic file logging |
| `demo.rs` | Deterministic fake sessions for demo mode |
| `recorder.rs` | Asciicast recording with tee writer |
| `session_recorder.rs` | Per-session highlight reel generator |
| `terminals/` | Terminal-specific switching and input injection |
| `ui/` | TUI rendering (table, detail, help, status bar) |

## Community

Questions, ideas, or workflows to share? [Start a Discussion](https://github.com/mercurialsolo/claudectl/discussions).

Found a bug? [Open an issue](https://github.com/mercurialsolo/claudectl/issues/new) with `claudectl --version`, your terminal (`echo $TERM_PROGRAM`), and steps to reproduce.

## License

MIT
