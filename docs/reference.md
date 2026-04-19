# Reference

## Dashboard Features

- Live table: PID, project, status, context %, cost, $/hr burn rate, elapsed, CPU%, memory, tokens, sparkline
- Parent sessions expand into subagent rows (completed totals + active subagents)
- Detail panel (`Enter`) with full session metadata
- Grouped view (`g`) by project with aggregate stats
- Sort by status, context, cost, burn rate, or elapsed (`s`)
- Live triage filters: status cycle (`f`), focus cycle (`v`), text search (`/`), clear (`z`)
- Conflict detection when 2+ sessions share the same git worktree (`!!`)
- Permission wait time — shows how long sessions have been waiting, longest first

## Status Detection

Multi-signal inference from CPU usage, JSONL events, and timestamps:

| Status | Color | Meaning |
|--------|-------|---------|
| **Needs Input** | Magenta | Waiting for user to approve/confirm a tool use |
| **Processing** | Green | Actively generating or executing tools |
| **Waiting** | Yellow | Done responding, waiting for user's next prompt |
| **Unknown** | Blue | Session is alive, but transcript telemetry is missing or unsupported |
| **Idle** | Gray | No recent activity (>10 min since last message) |
| **Finished** | Red | Process exited |

## Interactive Controls

| Key | Action |
|-----|--------|
| `j`/`k` or `Up`/`Down` | Navigate sessions |
| `Tab` | Switch to session's terminal tab |
| `Enter` | Toggle detail panel |
| `y` | Approve (send Enter to NeedsInput session) |
| `i` | Input mode (type text to session) |
| `d`/`x` | Kill session (double-tap to confirm) |
| `a` | Toggle auto-approve (double-tap to confirm) |
| `n` | Launch wizard for cwd, prompt, and resume |
| `g` | Toggle grouped view by project |
| `s` | Cycle sort column |
| `f` | Cycle status filter |
| `v` | Cycle focus filter (`attention`, budget, context, telemetry, conflicts) |
| `/` | Search project/model/session text |
| `z` | Clear all active filters |
| `c` | Send /compact to session (when idle) |
| `R` | Record session highlight reel (toggle) |
| `b` | Accept brain suggestion for selected session |
| `B` | Reject brain suggestion |
| `r` | Force refresh |
| `?` | Toggle help overlay |
| `q`/`Esc` | Quit |

## CLI Modes

```bash
claudectl                                    # Interactive TUI dashboard
claudectl --watch                            # Stream status changes (no TUI)
claudectl --list                             # Print session table and exit
claudectl --json                             # Machine-readable output
claudectl --history --since 24h              # Past sessions with cost
claudectl --stats --since 7d                 # Aggregated statistics
claudectl --doctor                           # Diagnose terminal support
claudectl --config                           # Show resolved configuration
claudectl --hooks                            # List configured hooks
claudectl --clean --older-than 7d --dry-run  # Preview cleanup
```

### Setup

```bash
claudectl --init                             # Wire up Claude Code hooks (global)
claudectl --init --project-local             # Wire up hooks for this project only
claudectl --uninstall                        # Remove claudectl hooks (global)
claudectl --uninstall --project-local        # Remove hooks from project-local settings
```

`--init` writes three hooks into Claude Code's `~/.claude/settings.json`:

| Hook | Matcher | Purpose |
|------|---------|---------|
| `PreToolUse` | `Bash` | Lets claudectl observe commands before execution |
| `PostToolUse` | `*` | Notifies claudectl after every tool completion |
| `Stop` | (all) | Notifies claudectl when a session ends |

The hooks call `claudectl --json` on each event. They are safe to run alongside any existing hooks — `--init` merges without overwriting.

Use `--project-local` to write to `.claude/settings.local.json` (gitignored) instead of the global file. This is useful when you want claudectl hooks only in specific projects.

`--uninstall` removes only claudectl hook entries, preserving all other settings and hooks. If the file becomes empty after removal, it is deleted.

## Cost Tracking

- Per-session USD estimates (Opus, Sonnet, Haiku model pricing)
- Live $/hr burn rate
- Per-session budget alerts at 80%, auto-kill at 100%
- Daily/weekly aggregate cost tracking in title bar
- Unknown models marked as fallback estimates until overridden in config

## Themes

Dark, light, and none (`--theme`). Respects `NO_COLOR` environment variable.

## How It Works

claudectl reads Claude Code's local data — no API keys, no network access, no modifications to Claude Code:

- **`~/.claude/sessions/*.json`** — session metadata (PID, session ID, working directory, start time)
- **`~/.claude/projects/{slug}/*.jsonl`** — conversation logs with token usage and events
- **`ps`** — CPU%, memory, TTY for each process
- **`/tmp/claude-{uid}/{slug}/{sessionId}/tasks/`** — subagent task files

Status inference combines multiple signals: `waiting_for_task` events, CPU usage thresholds, `stop_reason` fields, and message recency.

### Brain Query

Query the brain for a single tool-call decision without the TUI. Used by the Claude Code plugin hook, but also useful for scripting and testing:

```bash
claudectl --brain --brain-query --tool Bash --tool-input "rm -rf /tmp"
claudectl --brain --brain-query --tool Write --tool-input "src/main.rs" --project myapp
```

Output is JSON:

```json
{"action":"deny","reasoning":"Destructive command","confidence":0.95,"source":"brain","below_threshold":false,"threshold":0.6}
```

The decision flow is: deny rules (instant) -> approve rules (instant) -> LLM query -> adaptive threshold check.

If the brain is unreachable, returns `{"action":"abstain","source":"error"}` so callers are never blocked.

### Brain Gate Mode

Control whether the brain hook evaluates tool calls:

```bash
claudectl --mode on                    # Brain evaluates tool calls (default)
claudectl --mode off                   # Disable brain — all calls pass through
claudectl --mode auto                  # Brain auto-approves above threshold
claudectl --mode status                # Show current mode
```

| Mode | Approves safe calls | Denies dangerous calls | Low-confidence calls |
|------|:---:|:---:|:---:|
| `on` | Yes | Yes | Fall through to user |
| `auto` | Yes | Yes | Auto-approve |
| `off` | No | No | Fall through to user |

Mode is stored in `~/.claudectl/brain/gate-mode`. File absent = `on` (default).

## Claude Code Plugin

claudectl includes a Claude Code plugin in `claude-plugin/` that integrates the brain directly into sessions.

### Plugin Components

| Component | Type | What it does |
|-----------|------|-------------|
| `brain-gate.sh` | PreToolUse hook | Queries the brain before Bash/Write/Edit/NotebookEdit calls |
| `budget-check.sh` | PreToolUse hook | Denies tool calls when session exceeds budget |
| `/brain` | Command | Toggle brain mode: `/brain on`, `/brain off`, `/brain auto` |
| `/sessions` | Command | Show all active sessions with status, cost, and health |
| `/spend` | Command | Cost breakdown by project and time window |
| `/brain-stats` | Command | Brain learning metrics and accuracy |
| Supervisor | Agent | Proactive session health triage |
| Session Monitoring | Skill | Auto-activated awareness of claudectl capabilities |

### How the brain gate hook works

1. Claude Code fires a PreToolUse event with the tool name and input
2. The hook checks `~/.claudectl/brain/gate-mode` — if `off`, exits immediately
3. Calls `claudectl --brain --brain-query --tool <name> --tool-input <input>`
4. claudectl checks static deny/approve rules first (instant, no LLM)
5. If no rule matches, queries the local LLM brain
6. Returns `{"decision":"approve"}` or `{"decision":"deny","reason":"..."}` to Claude Code

In `on` mode, low-confidence brain approvals fall through to normal permission prompts. In `auto` mode, all brain approvals execute.

## Security

claudectl runs entirely locally. It reads Claude Code's session files from disk and process data from `ps`. It does not:
- Send data to any server (unless you configure webhooks or the brain feature)
- Modify Claude Code's files or behavior
- Require API keys or authentication
- Run with elevated privileges

Webhook payloads contain session metadata (project name, cost, status). Review your webhook URL and event filters before enabling.

The brain feature sends session context to a **local** LLM endpoint (default `localhost:11434`). No data leaves your machine unless you point `--brain-endpoint` at a remote server.
