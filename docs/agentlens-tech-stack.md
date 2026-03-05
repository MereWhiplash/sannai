# AgentLens Technical Architecture

## Overview

AgentLens is a code provenance platform that captures AI coding sessions and links them to pull requests. This document outlines the technical architecture, component design, and implementation approach.

---

## System Architecture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              Developer Machine                               │
│  ┌───────────────────────────────────────────────────────────────────────┐  │
│  │                         AgentLens Daemon (Rust)                       │  │
│  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  │  │
│  │  │   Claude    │  │   Cursor    │  │     Git     │  │    Sync     │  │  │
│  │  │   Watcher   │  │   Watcher   │  │    Hooks    │  │   Module    │  │  │
│  │  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘  │  │
│  │         │                │                │                │         │  │
│  │         └────────────────┼────────────────┼────────────────┘         │  │
│  │                          ▼                ▼                          │  │
│  │                   ┌─────────────────────────────┐                    │  │
│  │                   │     Session Manager         │                    │  │
│  │                   └──────────────┬──────────────┘                    │  │
│  │                                  ▼                                   │  │
│  │                   ┌─────────────────────────────┐                    │  │
│  │                   │     SQLite (Local Store)    │                    │  │
│  │                   └─────────────────────────────┘                    │  │
│  └───────────────────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────────────────┘
                                       │
                                       │ HTTPS (metadata only)
                                       ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                              AgentLens Cloud                                 │
│  ┌───────────────────────────────────────────────────────────────────────┐  │
│  │                           API Layer (Go)                              │  │
│  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  │  │
│  │  │   Session   │  │   GitHub    │  │     Auth    │  │  Dashboard  │  │  │
│  │  │    Sync     │  │  Webhooks   │  │  SSO/OIDC   │  │     API     │  │  │
│  │  └─────────────┘  └─────────────┘  └─────────────┘  └─────────────┘  │  │
│  └───────────────────────────────────────────────────────────────────────┘  │
│                                      │                                      │
│                                      ▼                                      │
│                   ┌─────────────────────────────────┐                       │
│                   │          PostgreSQL             │                       │
│                   └─────────────────────────────────┘                       │
└─────────────────────────────────────────────────────────────────────────────┘
                                       │
                                       ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                           Web Dashboard (React)                              │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐        │
│  │   Session   │  │   Timeline  │  │     PR      │  │    Team     │        │
│  │    List     │  │   Replay    │  │   Linking   │  │  Analytics  │        │
│  └─────────────┘  └─────────────┘  └─────────────┘  └─────────────┘        │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## Component 1: Local Agent (Rust)

### Why Rust

| Factor | Rust | Go | Node |
|--------|------|-----|------|
| Binary size | ~5MB | ~10MB | ~50MB+ |
| Memory footprint | ~10MB | ~20MB | ~50MB+ |
| Startup time | <100ms | <200ms | ~500ms |
| GC pauses | None | Yes | Yes |
| Cross-compile | Excellent | Good | Poor |

The agent runs 24/7 in the background. Low resource usage and no GC pauses are critical.

### Core Dependencies

```toml
[dependencies]
tokio = { version = "1", features = ["full"] }      # Async runtime
notify = "6"                                         # File system watching
rusqlite = { version = "0.31", features = ["bundled"] }  # SQLite
serde = { version = "1", features = ["derive"] }    # JSON parsing
serde_json = "1"
git2 = "0.18"                                        # Git integration
reqwest = { version = "0.11", features = ["json"] } # HTTP client for sync
directories = "5"                                    # XDG paths
tracing = "0.1"                                      # Logging
```

### Data Sources

#### Claude Code

Location:
```
~/.claude/projects/<project-id>/conversations/<conversation-id>.jsonl
```

Format: Newline-delimited JSON events

```json
{"type": "user_message", "content": "Add error handling to upload.ts", "timestamp": "..."}
{"type": "assistant_message", "content": "I'll add try-catch...", "timestamp": "..."}
{"type": "tool_use", "tool": "edit_file", "path": "src/upload.ts", "timestamp": "..."}
```

Parsing approach:
- Watch `~/.claude/projects/` with `notify`
- Tail new `.jsonl` files as they're written
- Parse each line as a session event
- Maintain active session state in memory

#### Cursor (Phase 2)

Location:
```
# macOS
~/Library/Application Support/Cursor/User/globalStorage/state.vscdb

# Linux
~/.config/Cursor/User/globalStorage/state.vscdb
```

Format: SQLite database (schema undocumented, requires reverse engineering)

Challenges:
- Schema changes between Cursor versions
- Database may be locked while Cursor is running
- Chat history structure is nested JSON in blob columns

Approach:
- Poll database every 5 seconds (not watch—SQLite doesn't trigger FS events reliably)
- Use WAL mode read to avoid lock conflicts
- Version detection to handle schema changes

#### Git Integration

Install a post-commit hook:
```bash
#!/bin/sh
# .git/hooks/post-commit
curl -s http://localhost:9847/hook/commit \
  --data-urlencode "sha=$(git rev-parse HEAD)" \
  --data-urlencode "repo=$(pwd)"
```

The daemon:
- Receives commit notification
- Looks up active session(s) at commit time
- Records link: `commit_sha <-> session_id`

### Local Storage Schema

```sql
-- Sessions table
CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    tool TEXT NOT NULL,           -- 'claude_code' | 'cursor'
    project_path TEXT,
    started_at DATETIME NOT NULL,
    ended_at DATETIME,
    synced_at DATETIME,
    metadata JSON
);

-- Events within sessions
CREATE TABLE events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL REFERENCES sessions(id),
    event_type TEXT NOT NULL,     -- 'user_prompt' | 'assistant_response' | 'tool_use' | 'file_edit'
    content TEXT,
    context_files JSON,           -- Files in context window
    timestamp DATETIME NOT NULL,
    metadata JSON
);

-- Commit correlations
CREATE TABLE commit_links (
    commit_sha TEXT NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions(id),
    repo_path TEXT NOT NULL,
    linked_at DATETIME NOT NULL,
    PRIMARY KEY (commit_sha, session_id)
);

-- Indexes
CREATE INDEX idx_events_session ON events(session_id);
CREATE INDEX idx_events_timestamp ON events(timestamp);
CREATE INDEX idx_commits_sha ON commit_links(commit_sha);
```

### Daemon Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                     AgentLens Daemon                        │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  ┌─────────────┐     ┌─────────────┐     ┌─────────────┐   │
│  │   Watcher   │────▶│   Session   │────▶│    Store    │   │
│  │   Manager   │     │   Manager   │     │   (SQLite)  │   │
│  └─────────────┘     └─────────────┘     └─────────────┘   │
│         │                   │                   │           │
│         │                   │                   │           │
│  ┌──────┴──────┐           │            ┌──────┴──────┐   │
│  │ File System │           │            │    Sync     │   │
│  │   Events    │           │            │   Queue     │   │
│  └─────────────┘           │            └─────────────┘   │
│                            │                   │           │
│                     ┌──────┴──────┐           │           │
│                     │  Local API  │───────────┘           │
│                     │  :9847      │                       │
│                     └─────────────┘                       │
│                            │                               │
│                     ┌──────┴──────┐                       │
│                     │  Git Hook   │                       │
│                     │  Receiver   │                       │
│                     └─────────────┘                       │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

Port 9847: Local HTTP API for CLI commands, git hooks, and local web UI.

---

## Component 2: Backend API (Go)

### Why Go

- Faster iteration than Rust for web services
- Excellent standard library for HTTP, JSON, SQL
- Strong ecosystem for auth (SSO, OAuth)
- Easier to hire for backend roles

### Core Dependencies

```go
// go.mod
require (
    github.com/gin-gonic/gin v1.9           // HTTP framework
    github.com/jackc/pgx/v5 v5.5            // PostgreSQL driver
    github.com/golang-jwt/jwt/v5 v5.2       // JWT handling
    github.com/coreos/go-oidc/v3 v3.9       // OIDC/SSO
    github.com/google/go-github/v58 v58     // GitHub API
    github.com/sethvargo/go-envconfig v0.9  // Config
)
```

### API Endpoints

```
# Session sync (from local agents)
POST   /api/v1/sessions              # Create/update session
POST   /api/v1/sessions/:id/events   # Append events
POST   /api/v1/commits               # Register commit links

# Dashboard API
GET    /api/v1/sessions              # List sessions (paginated)
GET    /api/v1/sessions/:id          # Get session with events
GET    /api/v1/sessions/:id/timeline # Get timeline for replay UI

# PR integration
GET    /api/v1/prs/:owner/:repo/:number          # Get PR with linked sessions
POST   /api/v1/webhooks/github                   # GitHub webhook receiver

# Auth
GET    /api/v1/auth/sso/:provider    # SSO initiation
POST   /api/v1/auth/sso/callback     # SSO callback
POST   /api/v1/auth/token            # Token refresh

# Admin (enterprise)
GET    /api/v1/admin/users           # List users
GET    /api/v1/admin/analytics       # Usage analytics
POST   /api/v1/admin/export          # Audit export
```

### Database Schema

```sql
-- Organizations
CREATE TABLE organizations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL,
    slug TEXT UNIQUE NOT NULL,
    sso_provider TEXT,
    sso_config JSONB,
    settings JSONB DEFAULT '{}',
    created_at TIMESTAMPTZ DEFAULT now()
);

-- Users
CREATE TABLE users (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id UUID REFERENCES organizations(id),
    email TEXT NOT NULL,
    name TEXT,
    role TEXT DEFAULT 'member',  -- 'admin' | 'member' | 'viewer'
    created_at TIMESTAMPTZ DEFAULT now(),
    last_active_at TIMESTAMPTZ,
    UNIQUE(org_id, email)
);

-- Sessions (synced from local agents)
CREATE TABLE sessions (
    id TEXT PRIMARY KEY,          -- Matches local session ID
    user_id UUID REFERENCES users(id),
    tool TEXT NOT NULL,
    project_name TEXT,
    prompt_count INTEGER DEFAULT 0,
    duration_seconds INTEGER,
    started_at TIMESTAMPTZ NOT NULL,
    ended_at TIMESTAMPTZ,
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMPTZ DEFAULT now()
);

-- Session events (synced from local agents)
CREATE TABLE session_events (
    id BIGSERIAL PRIMARY KEY,
    session_id TEXT REFERENCES sessions(id),
    event_type TEXT NOT NULL,
    content TEXT,
    context_files JSONB,
    timestamp TIMESTAMPTZ NOT NULL,
    metadata JSONB DEFAULT '{}'
);

-- Commit links
CREATE TABLE commit_links (
    id BIGSERIAL PRIMARY KEY,
    session_id TEXT REFERENCES sessions(id),
    commit_sha TEXT NOT NULL,
    repo_full_name TEXT NOT NULL,  -- 'owner/repo'
    created_at TIMESTAMPTZ DEFAULT now(),
    UNIQUE(session_id, commit_sha)
);

-- PR links (populated via GitHub webhooks)
CREATE TABLE pr_sessions (
    id BIGSERIAL PRIMARY KEY,
    pr_number INTEGER NOT NULL,
    repo_full_name TEXT NOT NULL,
    session_id TEXT REFERENCES sessions(id),
    created_at TIMESTAMPTZ DEFAULT now(),
    UNIQUE(pr_number, repo_full_name, session_id)
);

-- Indexes
CREATE INDEX idx_sessions_user ON sessions(user_id);
CREATE INDEX idx_sessions_started ON sessions(started_at);
CREATE INDEX idx_events_session ON session_events(session_id);
CREATE INDEX idx_commits_sha ON commit_links(commit_sha);
CREATE INDEX idx_commits_repo ON commit_links(repo_full_name);
CREATE INDEX idx_pr_repo ON pr_sessions(repo_full_name, pr_number);
```

---

## Component 3: Web Dashboard (React)

### Stack

```json
{
  "dependencies": {
    "react": "^18",
    "typescript": "^5",
    "tailwindcss": "^3",
    "@tanstack/react-query": "^5",    // Data fetching
    "react-router-dom": "^6",          // Routing
    "zustand": "^4",                   // State management
    "date-fns": "^3",                  // Date handling
    "recharts": "^2"                   // Analytics charts
  }
}
```

### Key UI Components

#### Session Timeline (core replay feature)

```
┌────────────────────────────────────────────────────────────────┐
│  Session: abc123  │  Claude Code  │  2h 15m  │  Jan 27, 10:00  │
├────────────────────────────────────────────────────────────────┤
│                                                                │
│  Timeline                                                      │
│  ══●═══════●═══════●═══════●═══════●═══════●═══════●══════    │
│   10:02   10:15   10:28   10:45   11:02   11:34   12:15       │
│                      ▲                                         │
│                   Current                                      │
│                                                                │
├────────────────────────────────────────────────────────────────┤
│  Prompt 3 of 7                                    ⚠️ 2 retries │
│  ┌──────────────────────────────────────────────────────────┐ │
│  │  "Add error handling to the upload function. It should   │ │
│  │   catch network errors and show a user-friendly message" │ │
│  └──────────────────────────────────────────────────────────┘ │
│                                                                │
│  Context files:                                                │
│  ├── src/upload.ts (primary)                                   │
│  ├── src/types.ts                                              │
│  └── src/utils/errors.ts                                       │
│                                                                │
├────────────────────────────────────────────────────────────────┤
│  Response                                          [Expand ▼]  │
│  ┌──────────────────────────────────────────────────────────┐ │
│  │  I'll add comprehensive error handling to the upload     │ │
│  │  function. Here's my approach:                           │ │
│  │  ...                                                     │ │
│  └──────────────────────────────────────────────────────────┘ │
│                                                                │
│  Files modified:                                               │
│  └── src/upload.ts  [+42 -3]                    [View diff ▶] │
│                                                                │
└────────────────────────────────────────────────────────────────┘
```

#### PR Integration View

```
┌────────────────────────────────────────────────────────────────┐
│  PR #482: Add file upload feature                              │
│  repo: acme/webapp  │  author: @alice  │  opened: Jan 27       │
├────────────────────────────────────────────────────────────────┤
│                                                                │
│  Linked Sessions (2)                                           │
│                                                                │
│  ┌──────────────────────────────────────────────────────────┐ │
│  │  ● Session abc123                                         │ │
│  │    Claude Code  │  7 prompts  │  2h 15m  │  Jan 27        │ │
│  │    Commits: a]b3f4d, c5e6f7g                               │ │
│  │    ⚠️ Struggle detected: 2 failed attempts on error       │ │
│  │       handling                                            │ │
│  │                                          [View Replay →]  │ │
│  └──────────────────────────────────────────────────────────┘ │
│                                                                │
│  ┌──────────────────────────────────────────────────────────┐ │
│  │  ● Session def456                                         │ │
│  │    Claude Code  │  3 prompts  │  45m  │  Jan 27           │ │
│  │    Commits: h8i9j0k                                       │ │
│  │    ✓ Clean session                                        │ │
│  │                                          [View Replay →]  │ │
│  └──────────────────────────────────────────────────────────┘ │
│                                                                │
└────────────────────────────────────────────────────────────────┘
```

---

## Component 4: GitHub Integration

### GitHub App

Create a GitHub App (not OAuth App) with these permissions:

| Permission | Access | Purpose |
|------------|--------|---------|
| Pull requests | Read & Write | Post comments, read PR data |
| Contents | Read | Read commit SHAs in PRs |
| Metadata | Read | Repository information |

Webhook events:
- `pull_request` (opened, synchronize)
- `push` (for commit tracking)

### PR Comment Flow

```
1. PR opened/updated
        │
        ▼
2. GitHub webhook → AgentLens API
        │
        ▼
3. Query commits in PR
        │
        ▼
4. Lookup sessions linked to those commits
        │
        ▼
5. Post/update comment on PR with session links
```

Comment format:

```markdown
## 🔍 AgentLens

This PR includes AI-assisted code from **2 sessions**:

| Session | Tool | Prompts | Duration | Signals |
|---------|------|---------|----------|---------|
| [abc123](https://app.agentlens.dev/s/abc123) | Claude Code | 7 | 2h 15m | ⚠️ Struggle detected |
| [def456](https://app.agentlens.dev/s/def456) | Claude Code | 3 | 45m | ✓ Clean |

<sub>Powered by [AgentLens](https://agentlens.dev) • Code provenance for AI-assisted development</sub>
```

---

## Deployment

### Local Agent Distribution

**macOS (Homebrew):**
```bash
brew tap agentlens/tap
brew install agentlens
```

**Linux (install script):**
```bash
curl -sSL https://agentlens.dev/install.sh | sh
```

**Enterprise (MDM-compatible):**
- `.pkg` for macOS (signed, notarized)
- `.deb` / `.rpm` for Linux
- Silent install with config file at `/etc/agentlens/config.toml`

### Cloud Infrastructure

```
┌─────────────────────────────────────────────────────────────┐
│                        Cloudflare                           │
│                      (CDN, DDoS, WAF)                       │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                      Load Balancer                          │
└─────────────────────────────────────────────────────────────┘
                              │
              ┌───────────────┼───────────────┐
              ▼               ▼               ▼
        ┌──────────┐   ┌──────────┐   ┌──────────┐
        │  API     │   │  API     │   │  API     │
        │  Node 1  │   │  Node 2  │   │  Node 3  │
        └──────────┘   └──────────┘   └──────────┘
              │               │               │
              └───────────────┼───────────────┘
                              ▼
                    ┌──────────────────┐
                    │   PostgreSQL     │
                    │   (RDS / Cloud   │
                    │    SQL)          │
                    └──────────────────┘
```

For MVP: Single API instance + managed Postgres is sufficient.

---

## Build Phases

### Phase 1: MVP (Weeks 1-10)

| Week | Focus | Deliverable |
|------|-------|-------------|
| 1-2 | Claude Code parser | Rust lib that parses .jsonl files |
| 3-4 | Local daemon | Background service with SQLite |
| 5-6 | Git integration | Post-commit hook, commit linking |
| 7-8 | Cloud API + DB | Go API, PostgreSQL schema, basic sync |
| 9-10 | Web dashboard | Session list, timeline replay |

### Phase 2: Team Features (Weeks 11-16)

- GitHub App integration
- PR comments with session links
- Team dashboard
- Basic auth (email/password or GitHub OAuth)

### Phase 3: Enterprise (Weeks 17-24)

- SSO/SAML integration
- RBAC
- Audit export
- Struggle detection signals
- Cursor integration

---

## Security Considerations

### Data Privacy

- **Local-first by default**: Full session content stays on device
- **Metadata-only sync**: Cloud receives session IDs, timestamps, prompt counts—not content
- **Opt-in content sync**: Enterprise can enable full content sync with encryption at rest
- **No source code in cloud**: File paths only, not file contents

### Agent Security

- Daemon binds to `127.0.0.1:9847` only (not exposed to network)
- No root/admin privileges required
- Read-only access to Claude Code logs and Cursor DB
- Fail-open: If daemon crashes, dev tools continue working

### Cloud Security

- TLS everywhere
- JWT tokens with short expiry (15 min) + refresh tokens
- SOC 2 Type II compliance (enterprise tier)
- Data residency options (US, EU)
