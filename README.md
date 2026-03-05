# Sannai

AI code provenance platform. Captures AI coding sessions and links them to pull requests so reviewers can see how code was generated.

## Architecture

```
Developer Machine                          Cloud
┌──────────────────────┐        ┌─────────────────────────┐
│  agent/ (Rust)       │        │  api/ (Go)              │
│  - Watches Claude    │ sync   │  - Session storage      │
│    Code sessions     │───────>│  - GitHub webhooks      │
│  - Parses JSONL      │        │  - Auth (SSO/OIDC)      │
│  - SQLite storage    │        │  - PostgreSQL            │
│  - Git commit links  │        └──────────┬──────────────┘
│  - Local API :9847   │                   │
└──────────────────────┘        ┌──────────┴──────────────┐
                                │  web/ (TanStack Start)  │
                                │  - Session timeline     │
                                │  - PR integration view  │
                                │  - Team analytics       │
                                └─────────────────────────┘
```

## Components

| Directory | Language | Status |
|-----------|----------|--------|
| `agent/` | Rust | Core pipeline implemented (watch → parse → session → store → API) |
| `api/` | Go | Schema and models defined, route handlers stubbed |
| `web/` | TypeScript | TanStack Start scaffolding with placeholder pages |

## Prerequisites

- Rust (stable)
- Go 1.22+
- Node.js 20+

## Setup

```bash
make setup    # install web deps + download Go modules
make build    # build all components
```

## Development

```bash
# Run the local agent (foreground)
cd agent && cargo run -- start

# Run the Go API server
make run-api  # :8080

# Run the web dev server
make run-web  # :3000 (proxies /api → :8080)
```

## Testing

```bash
make test     # all components
make test-agent
make test-api
```

## Project Structure

```
agent/
  src/
    watcher/    # Watches ~/.claude/projects/ for JSONL files
    parser/     # Parses Claude Code conversation events
    session/    # Manages active session lifecycle
    store/      # SQLite persistence layer
    daemon/     # PID file, signal handling, paths
    api/        # Local HTTP API (axum, :9847)

api/
  cmd/server/   # Entry point
  internal/
    handler/    # HTTP handlers (not yet implemented)
    middleware/ # Auth, logging (not yet implemented)
    model/      # Data types
    store/      # Database queries (not yet implemented)
  migrations/   # PostgreSQL schema

web/
  src/
    routes/     # File-based routing (TanStack Router)
    components/ # Shared UI components
    utils/      # Helpers
    styles/     # Tailwind CSS
```

## Tech Stack

- **Agent**: Tokio, notify, rusqlite, axum, serde, git2, clap
- **API**: Gin, pgx, golang-jwt
- **Web**: TanStack Start, TanStack Router, React 19, Vite 7, Nitro, Tailwind v4, Zustand, Recharts, Zod
