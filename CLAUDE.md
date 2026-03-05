# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What is Sannai

AI code provenance platform. Captures AI coding sessions on the developer's machine and links them to pull requests so reviewers can see how code was generated.

## Architecture

Three independent components, each with its own CLAUDE.md for component-specific details:

- **agent/** (Rust) — Local daemon. Watches Claude Code JSONL session files, parses events, stores in SQLite, links git commits. Local API on `:9847`. Fully implemented with 31+ tests.
- **api/** (Go) — Cloud backend. PostgreSQL schema and models defined, all route handlers are 501 stubs.
- **web/** (TypeScript) — SSR React frontend. Placeholder pages only. Dev server on `:3000`.

Data flow: agent watches `~/.claude/projects/` JSONL files -> parses -> stores in SQLite -> (future) syncs to cloud API -> web displays.

## Top-level Build Commands

```bash
make build          # Build all three components
make test           # Test all (agent + api; web has no tests yet)
make lint           # Lint all (clippy, go vet, eslint)
make setup          # Install deps (npm install + go mod download)
make manual-test    # Generate fake session data and test agent
```

## Component Commands

| Task | Agent | API | Web |
|------|-------|-----|-----|
| Build | `cd agent && cargo build` | `cd api && go build ./...` | `cd web && npm run build` |
| Test all | `cd agent && cargo test` | `cd api && go test ./...` | (none yet) |
| Test one | `cd agent && cargo test <name>` | `cd api && go test ./internal/store/...` | - |
| Lint | `cd agent && cargo clippy -- -D warnings` | `cd api && go vet ./...` | `cd web && npm run lint` |
| Format | `cd agent && cargo fmt` | - | - |
| Typecheck | - | - | `cd web && npm run typecheck` |
| Run | `cd agent && cargo run -- start` | `make run-api` (:8080) | `make run-web` (:3000) |
