# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What is Sannai

AI code provenance tool. Captures AI coding sessions on the developer's machine and links them to pull requests so reviewers can see how code was generated.

## Architecture

Single component: **agent/** (Rust) — Local daemon. Watches Claude Code JSONL session files, parses events, stores in SQLite, links git commits, computes diff attribution, posts PR comments with provenance summaries. Local API on `:9847`. 97 tests.

Data flow: agent watches `~/.claude/projects/` JSONL files -> parses -> stores in SQLite -> links to git commits -> attributes diffs to AI interactions -> posts PR comments.

## Build Commands

```bash
make build          # Build agent
make test           # Run all tests
make lint           # Clippy with -D warnings
make fmt            # cargo fmt
make clean          # cargo clean
make manual-test    # Generate fake session data and test agent
```

## Direct Commands

```bash
cd agent && cargo build
cd agent && cargo test                    # All tests
cd agent && cargo test <name>             # Single test
cd agent && cargo clippy -- -D warnings   # Lint
cd agent && cargo fmt                     # Format
cd agent && cargo run -- start            # Run daemon
cd agent && cargo run -- status           # Check daemon status
cd agent && cargo run -- sessions         # List sessions
```

## Notes

- `_archive/` contains out-of-scope components (cloud API, web frontend) preserved for future use
- See `agent/CLAUDE.md` for detailed agent architecture and conventions
