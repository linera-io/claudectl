# Changelog

All notable changes to claudectl are documented here.

## [0.10.1] - 2026-04-13

### Fixed
- **Worktree-aware conflict detection** — sessions in different git worktrees of the same repo no longer false-positive as conflicts. Uses `git rev-parse --show-toplevel` to resolve each session's worktree identity, cached per unique cwd.

## [0.10.0] - 2026-04-13

### Added
- **Remote compaction trigger** — press `c` to send `/compact` to a running Claude Code session. Only works when session is idle/waiting. Prevents context window from filling up before auto-compaction kicks in (#64)
- **Rate limit exhaustion ETA** — title bar shows `$spent/$budget (ETA: Xh Ym)` based on aggregate burn rate. Color-coded: green (>2h), yellow (<2h), red (<30m) (#57)
- **Conflict detection** — warns when 2+ sessions share the same working directory with `!!` prefix on project name. Desktop notification and `on_conflict_detected` hook (#58)
- **Context threshold hooks** — new `on_context_high` event fires when context window % crosses configurable threshold (default 75%). Resets after `/compact`. New `{context_pct}` template variable (#59)
- **Per-tool token attribution** — detail panel shows tool call counts sorted by frequency (Bash, Read, Edit, etc.). Exposed in `--json` export (#60)
- **Session cleanup command** — `claudectl --clean` with `--older-than`, `--finished`, `--dry-run` flags. Removes dead session JSON + JSONL transcripts, reports freed disk space (#61)
- **File change tracking** — detail panel shows which files each session modified (extracted from Edit/Write tool_use events in JSONL). Exposed in `--json` export (#62)
- **Permission wait time** — status column shows `Needs Input (2m 34s)` with escalating colors (yellow >1m, red >5m). NeedsInput sessions sorted by longest-waiting first (#63)
- `[context] warn_threshold` config option for context alert threshold

## [0.9.1] - 2025-04-12

### Added
- Daily and weekly aggregate cost budget alerts
- `[budget] daily_limit` and `[budget] weekly_limit` config options
- Aggregate budget hooks fire `on_budget_warning` and `on_budget_exceeded` with synthetic sessions

## [0.9.0] - 2025-04-11

### Added
- **Event hooks system** — run shell commands on session events
- 7 hook events: `on_session_start`, `on_status_change`, `on_needs_input`, `on_finished`, `on_budget_warning`, `on_budget_exceeded`, `on_idle`
- Template variables: `{pid}`, `{project}`, `{status}`, `{cost}`, `{model}`, `{cwd}`, `{tokens_in}`, `{tokens_out}`, `{elapsed}`, `{session_id}`, `{old_status}`, `{new_status}`
- Hooks configured in `[hooks.on_*]` sections of config.toml
- `claudectl --hooks` to list configured hooks
- Verified hooks repository at mercurialsolo/claudectl-hooks

## [0.8.3] - 2025-04-10

### Added
- Weekly and daily cost/token summary in TUI title bar

## [0.8.0] - 2025-04-09

### Added
- **Multi-session orchestration** — `claudectl --run tasks.json` with dependency ordering and `--parallel` flag
- **Session history** — persist completed sessions with `--history` and `--stats` commands
- **Configuration files** — `~/.config/claudectl/config.toml` (global) and `.claudectl.toml` (per-project) with layered overrides
- **Theme system** — dark, light, and monochrome themes with `NO_COLOR` support
- **Diagnostic logging** — `--log` flag for structured debug output
- **Install script and Nix flake** for easier distribution
- First-run experience with empty state hints

### Fixed
- Approve/input for Warp terminal using AppleScript with focus management

## [0.7.0] - 2025-04-07

### Added
- **Watch mode** — `claudectl --watch` streams status changes without TUI
- **Debug mode** — timing instrumentation in the footer
- **Activity sparklines** — 30-second history ring buffer per session
- **Grouped view** — press `g` to group sessions by project with aggregate stats
- **Detail panel** — press `Enter` for expanded session info (tokens, cost, model, paths)
- **Session summary** — `claudectl --summary` for what happened while you were away
- **Webhooks** — POST JSON to Slack/Discord/URL on status changes with event filtering
- **Session launcher** — press `n` or `claudectl --new` to start sessions from the TUI
- **Budget enforcement** — `--budget` with 80% warning and optional `--kill-on-budget`
- Custom output format for watch mode
- Linux support (monitoring without terminal switching)
- Stale session cleanup for dead PIDs >24h old

## [0.6.0] - 2025-04-05

### Added
- Context window % column with visual bar
- Burn rate ($/hr) column with cost decay
- Desktop notifications when sessions enter NeedsInput (`--notify`)
- Help overlay (press `?`)
- Sort and filter by status, context, cost, $/hr, elapsed (press `s`)
- JSON export (`--json`) for scripting
- Subagent tracking with +N indicator
- Auto-approve mode (press `a` twice)

### Changed
- Renamed Tokens column to In/Out for clarity

### Fixed
- 5 critical issues: performance, burn rate calc, CPU smoothing, dropped sysinfo dependency, timestamp handling

## [0.5.0] - 2025-04-03

### Added
- Quick approve — press `y` to send Enter to NeedsInput sessions
- Input mode — press `i` to type arbitrary text to sessions
- Kill sessions — press `d`/`x` (double-tap to confirm)
- NeedsInput status detection for permission prompts
- Terminal switching — press `Tab` to jump to a session's terminal

### Fixed
- JSONL session ID mapping (use sessionId before falling back to latest)
- Input sending via terminal emulator instead of raw TTY device
- Status inference: CPU priority over JSONL flags

## [0.4.0] - 2025-04-02

### Added
- Terminal support for **Ghostty**, **Kitty**, **WezTerm**, **tmux**, **Warp**, **iTerm2**, and **Terminal.app**
- Process table enrichment (CPU, MEM, TTY, elapsed) via `ps`
- Session file scanner for `~/.claude/sessions/*.json`
- JSONL tail reader for incremental token accumulation
- Status inference engine (Processing / NeedsInput / WaitingInput / Idle / Finished)
- Cost estimation with model-aware pricing (Opus, Sonnet, Haiku)
- Diff-based UI updates (only re-render changed rows)
- Configurable poll interval

## [0.1.0] - 2025-04-01

### Added
- Initial release
- Basic TUI table showing running Claude Code sessions
- Process discovery via `~/.claude/sessions/` directory
- ratatui-based terminal UI

---

## Feature Overview

### Dashboard & Monitoring
- Live TUI dashboard with PID, project, status, context %, cost, $/hr, elapsed, CPU%, MEM, tokens, sparklines
- Smart status detection: Processing, Needs Input (with wait time), Waiting, Idle, Finished
- Context window % with configurable threshold alerts
- Cost tracking with per-session and aggregate USD estimates
- Burn rate ($/hr) with budget exhaustion ETA projection
- Activity sparklines (30-second history per session)
- Weekly/daily cost summary in title bar

### Session Actions
- `y` — Approve permission prompts (send Enter)
- `i` — Send custom text input to sessions
- `c` — Trigger `/compact` on idle sessions
- `a` — Toggle auto-approve (double-tap)
- `d`/`x` — Kill sessions (double-tap to confirm)
- `n` — Launch new Claude Code sessions
- `Tab` — Switch to session's terminal

### Observability
- Per-tool token attribution (Bash, Read, Edit call counts)
- File change tracking (which files each session modified)
- Conflict detection (2+ sessions sharing same directory)
- Permission wait time tracking with color escalation
- Detail panel with full session breakdown

### Budget & Limits
- Per-session budget with 80% warning and 100% auto-kill
- Daily and weekly aggregate spend limits
- Rate limit exhaustion ETA projection
- Context threshold alerts with `on_context_high` hook

### Event Hooks
- 9 hook events: `on_session_start`, `on_status_change`, `on_needs_input`, `on_finished`, `on_budget_warning`, `on_budget_exceeded`, `on_idle`, `on_context_high`, `on_conflict_detected`
- Template variables for shell command interpolation
- Webhook integration (POST JSON to Slack/Discord/URLs)
- Desktop notifications

### Output Modes
- Interactive TUI (default)
- `--list` — print formatted table and exit
- `--json` — export session data for scripting
- `--watch` — stream status changes without TUI
- `--summary` — session activity summary
- `--history` / `--stats` — historical analytics
- `--clean` — remove old session data

### Configuration
- Global config: `~/.config/claudectl/config.toml`
- Per-project config: `.claudectl.toml`
- CLI flags override config values
- Theme system: dark, light, monochrome, NO_COLOR

### Terminal Support
- Ghostty (native AppleScript)
- Kitty (remote control API)
- tmux (send-keys)
- WezTerm (CLI JSON API)
- Warp (System Events)
- iTerm2 (AppleScript)
- Terminal.app (AppleScript)

### Task Orchestration
- `--run tasks.json` with dependency ordering
- `--parallel` for independent tasks
- Per-task budget and cwd settings
