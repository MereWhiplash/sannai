# Sannai

[![CI](https://github.com/MereWhiplash/sannai/actions/workflows/ci.yml/badge.svg)](https://github.com/MereWhiplash/sannai/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Crate](https://img.shields.io/crates/v/sannai.svg)](https://crates.io/crates/sannai)

AI code provenance for your team. Sannai captures AI coding sessions and links them to pull requests so reviewers can see how code was generated.

## What it does

Sannai runs as a local daemon that:

- **Watches** Claude Code session files (`~/.claude/projects/`)
- **Parses** JSONL conversation events (prompts, responses, tool use)
- **Stores** sessions and events in a local SQLite database
- **Links** git commits to the sessions that produced them
- **Comments** on pull requests with session provenance summaries

## Quick start

```bash
cargo install sannai          # 1. Install the binary
sannai install                # 2. Register as a login daemon
```

That's it. Sannai is now running in the background and will start automatically on login.

### From source

```bash
git clone https://github.com/MereWhiplash/sannai.git
cd sannai/agent
cargo install --path .        # Installs to ~/.cargo/bin/sannai
sannai install                # Register the daemon
```

## Usage

```bash
sannai status                 # Check daemon and service status
sannai sessions               # List captured sessions
sannai comment --pr <url>     # Post provenance comment on a PR
sannai start                  # Start daemon in foreground (manual)
sannai uninstall              # Remove the daemon service
sannai uninstall --purge      # Remove service and all stored data
```

The daemon runs a local API on `127.0.0.1:9847` with endpoints:

- `GET /health` — status + version
- `GET /sessions` — list sessions
- `GET /sessions/{id}` — session detail
- `GET /sessions/{id}/events` — session events
- `POST /hook/commit` — link commits to active sessions

## Prerequisites

- Rust (stable)
- Claude Code (generates the session files Sannai watches)
- `gh` CLI (authenticated, for posting PR comments)

## Development

```bash
make build          # cargo build
make test           # cargo test (31+ tests)
make lint           # cargo clippy -- -D warnings
make fmt            # cargo fmt
make manual-test    # Generate fake session data and test the agent
```

## License

MIT
