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

## Security

claudectl runs entirely locally. It reads Claude Code's session files from disk and process data from `ps`. It does not:
- Send data to any server (unless you configure webhooks or the brain feature)
- Modify Claude Code's files or behavior
- Require API keys or authentication
- Run with elevated privileges

Webhook payloads contain session metadata (project name, cost, status). Review your webhook URL and event filters before enabling.

The brain feature sends session context to a **local** LLM endpoint (default `localhost:11434`). No data leaves your machine unless you point `--brain-endpoint` at a remote server.
