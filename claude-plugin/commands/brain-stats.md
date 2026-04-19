---
name: brain-stats
description: Show claudectl brain learning metrics — accuracy, correction rate, and effectiveness
args: "[metric]"
---

Show how well the claudectl brain is learning the user's preferences. The optional argument selects a specific metric view.

Available metrics (pass as argument, or show all if omitted):
- `learning-curve` — is the correction rate declining over time? (= brain is learning)
- `accuracy` — per-tool, per-risk, per-project accuracy breakdown
- `baseline` — brain vs. dumb static rules classifier comparison
- `false-approve` — safety metric: how often does brain approve risky actions that get rejected?

Run the appropriate command:
- All metrics: run `claudectl --brain-stats learning-curve` then `claudectl --brain-stats accuracy`
- Specific metric: run `claudectl --brain-stats {{metric}}`

Present the results highlighting:
- Whether the brain is improving over time
- Which tools the brain handles well vs. poorly
- Any safety concerns (high false-approve rates)
