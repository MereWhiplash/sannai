# Agent — sannai (Rust)

Local daemon that captures Claude Code sessions and posts provenance comments on PRs. Binary name: `sannai`.

## Commands

```bash
cargo build
cargo test                        # All 98 tests
cargo test test_parse_user        # Single test by name
cargo clippy -- -D warnings       # Lint (CI treats warnings as errors)
cargo fmt                         # Format (max_width=100, see rustfmt.toml)
cargo run -- start                # Run daemon in foreground
cargo run -- status               # Check if daemon is running
cargo run -- sessions             # List captured sessions
cargo run -- comment --pr <url>   # Post provenance comment on a PR
cargo run -- install              # Register as system service
cargo run -- uninstall [--purge]  # Remove service (and optionally data)
cargo run -- hook install         # Install git + Claude Code hooks in cwd
cargo run -- hook status          # Check which hooks are installed
cargo run -- hook uninstall       # Remove sannai hooks from cwd
```

## Architecture

The daemon runs three concurrent tokio tasks connected by channels:

1. **watcher** — Uses `notify` to watch `~/.claude/projects/` for JSONL files. Tails files from saved byte offsets (persisted in `watcher_state.json`). Sends `WatcherEvent` over an mpsc channel.
2. **session** — `SessionManager` receives `WatcherEvent`s, maintains in-memory `ActiveSession` map, persists to SQLite via `Store`. Ends sessions after 10min idle timeout.
3. **api** — Axum HTTP server on `127.0.0.1:9847`. Read-only endpoints plus a `POST /hook/commit` for linking commits to active sessions.

Shared state: `Store` and `SessionManager` are wrapped in `Arc<Mutex<_>>` and passed to all tasks.

## Module Pipeline

```
watcher -> parser -> session -> store
                                  ^
                            api --+
                            comment (PR posting, uses provenance/)
```

### Core pipeline
- **parser** — `parse_line()` converts a JSONL line into `Vec<ParsedEvent>`. One line can produce multiple events (e.g., assistant message with text + tool_use). Handles: `queue-operation`, `user`, `assistant`. Ignores: `progress`, `system`.
- **watcher** — `FileWatcher` classifies paths as `MainSession` or `Subagent` based on directory depth. Persists file byte offsets for resume after restart.
- **session** — `SessionManager.ensure_session()` creates or updates sessions. `process_event()` maps `ParsedEvent` variants to store operations. Stores tool_id/input on tool_use events and content on tool_result events for provenance.
- **store** — SQLite with WAL mode. Tables: `sessions`, `events`, `commit_links`. All timestamps stored as RFC 3339 strings. Uses `upsert` (INSERT ... ON CONFLICT) for sessions.
- **daemon** — PID file management, data dir paths, signal handling. Override dirs with `SANNAI_DATA_DIR` and `SANNAI_CLAUDE_DIR` env vars.

### Provenance & commenting
- **provenance/interaction** — Groups raw events into logical interactions (prompt → response cycle). Filters noise (confirmations, slash commands, internal tags).
- **provenance/lineage** — Extracts file-level lineage from tool calls (which files were read/written per interaction).
- **provenance/attribution** — Matches PR diff hunks to interactions via content similarity, with file-level fallback for when tool call content is truncated.
- **provenance/summary** — Optional LLM summary generation. Builds structured prompt from session data, pipes to configurable command.
- **comment/format** — Renders provenance data as a GitHub markdown comment with attribution stats, per-session interaction tables, and diff attribution.
- **comment/github** — `gh` CLI wrapper for fetching PR data and posting/updating comments.
- **config** — Loads `~/.config/sannai/config.toml` for summary settings.
- **service** — Cross-platform daemon installer (launchd on macOS, systemd on Linux).
- **hook** — Manages git pre-push hooks (auto PR comments) and Claude Code PostToolUse hooks (commit linking). Embeds hook scripts via `include_str!`, injects binary path at install time. Merges into existing `.claude/settings.json` without clobbering.

## API Endpoints (local, :9847)

- `GET /health` — status + version
- `GET /sessions` — list sessions (`?limit=20&offset=0`)
- `GET /sessions/{id}` — session detail
- `GET /sessions/{id}/events` — all events for a session
- `POST /hook/commit` — link a git commit SHA to active sessions (`{"sha": "...", "repo": "..."}`)

## Conventions

- Error handling: `anyhow::Result` everywhere, `bail!` for early returns
- Logging: `tracing` crate, filter via `RUST_LOG` env var (default: `sannai=info`)
- Tests use `tempfile::TempDir` for isolated SQLite databases
- JSONL format uses camelCase field names (matches Claude Code output), Rust structs use snake_case with `#[serde(rename_all = "camelCase")]`

## File Paths at Runtime

- Data dir: `~/Library/Application Support/dev.sannai.sannai/` (macOS)
- SQLite DB: `<data_dir>/store.db`
- Watcher state: `<data_dir>/watcher_state.json`
- PID file: `<data_dir>/sannai.pid`
- Watched dir: `~/.claude/projects/`
