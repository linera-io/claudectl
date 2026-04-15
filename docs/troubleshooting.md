# Troubleshooting

## No sessions found

- Ensure Claude Code is running (`claude` in another terminal)
- Check that `~/.claude/sessions/` contains `.json` files
- Run `claudectl --log /tmp/claudectl.log` and check the log

## Tab switching doesn't work

Run `claudectl --doctor` first to see the detected terminal, missing prerequisites, and supported actions.

- **GNOME Terminal**: Launch support is available; use tmux or Kitty if you need remote switching or input automation
- **Windows Terminal on WSL**: Launch support is available when `cmd.exe /c wt.exe` works; use tmux or Kitty inside WSL for switching and input automation
- **Ghostty**: Should work out of the box
- **Kitty**: Add `allow_remote_control yes` to `~/.config/kitty/kitty.conf`
- **Warp/iTerm2/Terminal.app**: Grant Automation/Accessibility permission in System Settings > Privacy & Security
- **tmux**: Must be running inside a tmux session

## Cost shows $0.00

claudectl reads token usage from JSONL logs. If the session just started, wait for the first response to complete. Check that `~/.claude/projects/` contains `.jsonl` files.

## High CPU usage from claudectl itself

Increase the poll interval: `claudectl --interval 3000` (default is 2000ms).

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
Native Windows is not supported yet. WSL plus Windows Terminal can now launch new Claude tabs through `claudectl --new` or `n`, and WSL plus `tmux` remains the recommended setup when you also want switch/input/approve automation.

For other issues, run with `--log` and [open an issue](https://github.com/mercurialsolo/claudectl/issues/new) with the log attached.
