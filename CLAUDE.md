# claudectl

TUI dashboard for monitoring and managing multiple Claude Code CLI sessions.

## Build & Test

```bash
cargo build                  # Debug build
cargo build --release        # Release build (optimized, <1MB binary)
cargo test                   # Run all tests
cargo clippy -- -D warnings  # Lint (warnings are errors in CI)
cargo fmt --check            # Check formatting
```

## Architecture

**Core modules** (`src/`):
- `main.rs` — CLI entry point, mode dispatch (TUI, watch, JSON, list, history, stats, orchestrator)
- `app.rs` — TUI app state, refresh loop, keyboard event handling
- `session.rs` — Session data structures and formatting
- `discovery.rs` — Scans `~/.claude/sessions/*.json` and resolves JSONL paths
- `monitor.rs` — Parses JSONL conversation logs for tokens, cost, status events
- `process.rs` — Process introspection via native `ps` (not sysinfo crate)
- `config.rs` — Layered TOML config: CLI flags > `.claudectl.toml` > `~/.config/claudectl/config.toml` > defaults
- `history.rs` — Session history persistence and cost analytics
- `hooks.rs` — Event hook system (shell commands fired on session events)
- `orchestrator.rs` — Multi-session task runner with dependency ordering
- `theme.rs` — Color theming (dark/light/monochrome, respects NO_COLOR)
- `logger.rs` — Structured diagnostic logging

**TUI** (`src/ui/`): `table.rs` (session list), `detail.rs` (expanded panel), `help.rs` (overlay), `status_bar.rs` (footer)

**Terminal backends** (`src/terminals/`): Ghostty, Kitty, tmux, WezTerm, Warp, iTerm2, Terminal.app — auto-detected, used for tab switching and input sending.

## Key Design Decisions

- **Minimal dependencies** — 6 runtime crates. Binary must stay under 1MB, startup under 50ms.
- **Native `ps`** over `sysinfo` crate to keep binary small.
- **Multi-signal status inference** — combines CPU usage, JSONL events, and timestamps (not just one signal).
- **Incremental JSONL parsing** — tracks file offsets, never rereads full files.
- **No async runtime** — synchronous with polling. Keeps complexity low.

## Conventions

- Run `cargo fmt` and `cargo clippy -- -D warnings` before committing.
- Tests live in `tests/integration_tests.rs` and `tests/unit_tests.rs`.
- Status inference logic has extensive test coverage — do not change status detection without updating tests.
- Terminal backends implement the pattern in `src/terminals/mod.rs` — add new terminals there.
- Config fields must be added to all three layers (CLI args in `main.rs`, TOML struct in `config.rs`, merge logic in `config.rs`).
