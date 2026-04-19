---
name: auto-insights
description: Show or configure auto-generated insights from the claudectl brain — friction patterns, suggested rules, and workflow improvements
args: "[on|off|status]"
---

Manage claudectl's auto-insights system, which analyzes brain decision history to detect friction patterns and suggest workflow improvements.

## What to do

If the user provides an argument (`on`, `off`, or `status`):
- Run `claudectl --brain --insights {{arg}}` to set the insights mode.

If no argument is provided:
- Run `claudectl --brain --insights` to show current insights.

## How to present results

Insights are grouped by category:
- **Friction Patterns** — tools/commands the user frequently rejects (suggest deny rules)
- **Error Loops** — repeated errors from the same tool (investigate root cause)
- **Context Blowouts** — sessions frequently hitting high context usage (suggest earlier /compact)
- **Recommended Rules** — high-confidence patterns that should become auto-rules in `.claudectl.toml`
- **Accuracy Gaps** — tools where the brain needs more training data
- **Temporal Patterns** — time-of-day or error-streak behaviors
- **Cost Patterns** — burn rate trends and expensive session outliers

For **Recommended Rules**, show the exact TOML syntax the user can copy into `.claudectl.toml`.

If the user asks to apply a suggested rule, help them edit their `.claudectl.toml` directly.
