# API — sannai cloud (Go)

Cloud backend for syncing and serving session data. **Not yet functional** — all route handlers return 501.

## Commands

```bash
go build ./...
go test ./...
go test ./internal/handler/...    # Test single package
go vet ./...                      # Lint
go run ./cmd/server               # Run on :8080 (or PORT env)
```

## Architecture

- **cmd/server/main.go** — Entry point. Sets up Gin router with all routes.
- **internal/model/** — Go structs matching the PostgreSQL schema (`Session`, `SessionEvent`, `CommitLink`).
- **internal/handler/** — HTTP handlers (empty, to be implemented).
- **internal/store/** — Database queries (empty, to be implemented).
- **internal/middleware/** — Auth, logging (empty, to be implemented).
- **migrations/001_init.sql** — PostgreSQL schema.

## Routes (all 501 stubs)

All under `/api/v1`:

| Method | Path | Purpose |
|--------|------|---------|
| POST | `/sessions` | Sync session from agent |
| POST | `/sessions/:id/events` | Sync events from agent |
| POST | `/commits` | Sync commit links |
| GET | `/sessions` | Dashboard: list sessions |
| GET | `/sessions/:id` | Dashboard: session detail |
| GET | `/sessions/:id/timeline` | Dashboard: session timeline |
| GET | `/prs/:owner/:repo/:number` | PR integration view |
| POST | `/webhooks/github` | GitHub webhook receiver |
| GET | `/auth/sso/:provider` | SSO initiation |
| POST | `/auth/sso/callback` | SSO callback |
| POST | `/auth/token` | Token exchange |
| GET | `/admin/users` | Admin: user list |
| GET | `/admin/analytics` | Admin: analytics |
| POST | `/admin/export` | Admin: data export |

Health check: `GET /healthz`

## Database Schema

PostgreSQL tables: `organizations`, `users`, `sessions`, `session_events`, `commit_links`, `pr_sessions`. The cloud schema adds org/user/PR concepts on top of what the agent stores locally.

## Conventions

- Module path: `github.com/merewhiplash/sannai`
- Dependencies: gin (HTTP), pgx (PostgreSQL), golang-jwt (auth), go-envconfig (config)
- Config via env vars (PORT, etc.)
