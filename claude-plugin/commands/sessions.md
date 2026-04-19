---
name: sessions
description: Show status of all active Claude Code sessions (cost, health, context usage)
---

Run `claudectl --list` to get the current status of all active Claude Code sessions. If no sessions are found, let the user know.

If the user wants more detail, you can also run `claudectl --json` for machine-readable output with full session data including token counts, cost, burn rate, and context window usage.

For session health issues (loops, stalls, cost spikes, context saturation), run `claudectl --json` and look at the `decay_score`, `has_file_conflict`, and `last_tool_error` fields.

Present the results in a concise table showing: project name, status, cost, context %, and any health warnings.
