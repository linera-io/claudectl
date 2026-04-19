#!/usr/bin/env bash
# claudectl budget-check: PreToolUse hook that denies tool calls when
# the current session's spend exceeds the configured budget.
#
# Uses claudectl --json to check session cost against the budget
# set in .claudectl.toml or via --budget flag.

set -euo pipefail

if ! command -v claudectl &>/dev/null; then
    exit 0
fi

# Get budget from config or environment
BUDGET="${CLAUDECTL_BUDGET:-}"
if [ -z "$BUDGET" ]; then
    # Try reading from .claudectl.toml
    if [ -f ".claudectl.toml" ]; then
        BUDGET=$(sed -n 's/^budget *= *\([0-9.]*\)/\1/p' .claudectl.toml 2>/dev/null || true)
    fi
fi

# No budget configured — fall through
if [ -z "$BUDGET" ]; then
    exit 0
fi

# Find this session's cost from claudectl JSON output
# Match by current PID's parent (Claude Code is the parent of this hook)
SESSIONS=$(claudectl --json 2>/dev/null) || exit 0

# Look for any session in our cwd that's over budget
PROJECT_DIR="${PWD:-}"
OVER_BUDGET=$(echo "$SESSIONS" | sed -n 's/.*"cost_usd" *: *\([0-9.]*\).*/\1/p' | head -1)

if [ -n "$OVER_BUDGET" ]; then
    OVER=$(echo "$OVER_BUDGET $BUDGET" | awk '{if ($1 >= $2) print "yes"; else print "no"}')
    if [ "$OVER" = "yes" ]; then
        printf '{"decision":"deny","reason":"claudectl: session cost ($%s) exceeds budget ($%s)"}\n' "$OVER_BUDGET" "$BUDGET"
        exit 0
    fi
fi

# Under budget — fall through
exit 0
