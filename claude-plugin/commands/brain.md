---
name: brain
description: Toggle the claudectl brain gate on/off/auto for this session
args: "[on|off|auto|status]"
---

Control the claudectl brain's auto-approve/deny behavior for tool calls in this session.

## Modes

- **`on`** (default) — brain evaluates every tool call, auto-approves safe ones, denies dangerous ones. Low-confidence decisions fall through to normal permission prompts.
- **`off`** — brain is disabled. All tool calls go through Claude Code's normal permission flow. Use this when you want full manual control.
- **`auto`** — brain auto-approves all tool calls it considers safe, even with lower confidence. Use this when you trust the brain's judgment and want maximum speed.
- **`status`** — show the current mode without changing it.

## What to do

Run `claudectl --mode {{mode}}` where `{{mode}}` is the argument the user provided (on, off, auto, or status). If no argument was provided, run `claudectl --mode status`.

After running the command, confirm the mode change to the user and briefly explain what it means:
- `off`: "Brain disabled. Tool calls will go through normal permission prompts."
- `on`: "Brain active. Safe tool calls will be auto-approved, dangerous ones auto-denied."
- `auto`: "Brain in full auto mode. All decisions above the confidence threshold will execute without prompts."
