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

## Install

### From source

```bash
git clone https://github.com/MereWhiplash/sannai.git
cd sannai/agent
cargo build --release
# Binary at target/release/sannai
```

### Cargo

```bash
cargo install sannai
```

## Usage

```bash
# Start the daemon (foreground)
sannai start

# Check status
sannai status

# List captured sessions
sannai sessions

# Post a provenance comment on a PR
sannai comment --pr <owner/repo#number>
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
