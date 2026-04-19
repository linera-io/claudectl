---
name: spend
description: Show Claude Code spend summary — total cost, per-session breakdown, and burn rate
args: "[time-window]"
---

Show the user's Claude Code spending. The optional argument is a time window (e.g., "8h", "24h", "7d"). Default is "24h".

Run these commands to gather spend data:

1. `claudectl --stats --since {{time_window}}` for aggregated statistics (total cost, token counts, per-project and per-model breakdown)
2. `claudectl --list` for current active session costs and burn rates

Present the results showing:
- Total spend in the time window
- Active session costs and burn rates
- Per-project breakdown if available
- Today's spend vs weekly average if the window is large enough
