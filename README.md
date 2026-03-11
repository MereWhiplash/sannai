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
| `agent/` | Rust | Full pipeline: watch → parse → session → git observer → process audit → API |
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
    parser/     # Parses Claude Code JSONL events
    session/    # Manages active session lifecycle, triggers git tracking
    store/      # SQLite persistence (sessions, events, commits, git_events, process_metrics)
    git/        # Git observer (poll for HEAD changes), commit detail extraction, tool detection
    process/    # Process analyzer — computes all heuristic metrics
    provenance/ # Interaction model builder (groups events into prompt→response cycles)
    comment/    # GitHub PR comment formatting and posting
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

## Process Audit

When the agent detects a git commit during an active session, it analyzes the session's interactions and produces a **process audit** — a set of heuristic metrics that describe *how* the AI was used, not just *what* it produced.

These metrics are stored per-commit and can be posted as a GitHub PR comment via `sannai comment --pr <url>`.

### Example PR Comment

```
## AI Process Audit — 3 sessions, 47 interactions

Steering: High — 72% of interactions had specific human prompts
Exploration: 12 files read before first edit
Testing: TDD — tests written before code
Iterations: 2 error-fix cycles (normal)
Red flags: None
```

### Metrics Reference

#### Steering Ratio

**What it measures:** How much the human directed the AI vs. letting it run autonomously.

**How it's calculated:** Ratio of interactions that were initiated by a specific user prompt. Currently 1.0 for all interactions (every interaction starts with a user prompt by definition). Will be refined with prompt specificity weighting — a vague "fix it" counts less than "add retry logic to api/mod.rs with exponential backoff."

**Thresholds:**
| Range | Label |
|-------|-------|
| 70–100% | High |
| 40–69% | Medium |
| 0–39% | Low |

#### Exploration Score

**What it measures:** Whether the AI read and understood existing code before making changes.

**How it's calculated:** Counts Read, Glob, and Grep tool calls that occur *before the first Write/Edit*. Normalized: 0 reads = 0.0, 5+ reads = 1.0.

**Why it matters:** An AI that writes code without reading the codebase first is more likely to produce code that doesn't fit the project's patterns.

#### Read/Write Ratio

**What it measures:** Balance between reading existing code and writing new code.

**How it's calculated:** `(unique files read via Read) / (unique files written via Write or Edit)`. Higher ratios suggest the AI explored more context before making changes.

#### Test Behavior

**What it measures:** Whether tests were written, and in what order relative to code changes.

**How it's calculated:** Scans Bash tool calls for test commands (`cargo test`, `npm test`, `pytest`, `go test`, `make test`, `yarn test`, `bun test`, `npx jest`), then compares the position of the first test call to the first Write/Edit call.

| Value | Meaning |
|-------|---------|
| `tdd` | First test command appeared before first Write/Edit |
| `test_after` | Tests run after code was written |
| `test_only` | Tests run but no code was written |
| `no_tests` | No test commands detected |

#### Error-Fix Cycles

**What it measures:** How many times the AI hit an error and then attempted a fix.

**How it's calculated:** Scans Bash tool call outputs for error indicators (`error`, `failed`, `panic`, `exception`, `traceback`). Each error followed by a subsequent Write/Edit counts as one cycle.

**Thresholds:**
| Range | Interpretation |
|-------|---------------|
| 0–2 | Normal iterative development |
| 3+ | High — may indicate brute-force debugging |

#### Prompt Specificity

**What it measures:** How detailed and targeted the human's prompts were.

**How it's calculated:** Per-prompt score (0.0–1.0) based on three signals:

| Signal | Weight | Logic |
|--------|--------|-------|
| Length | up to 0.4 | `min(word_count / 20, 0.4)` — longer prompts tend to be more specific |
| File paths | 0.3 | Contains `/`, `.rs`, `.ts`, `.go`, or `.py` |
| Constraint words | up to 0.3 | Count of: must, should, ensure, max, min, retry, timeout, error, test, validate, return, handle, implement, add, remove — `min(count / 5, 0.3)` |

The session-level score is the average across all prompts.

#### Red Flags

**What it measures:** Specific dangerous operations the AI performed.

**What's detected:**

| Flag | Trigger |
|------|---------|
| Force push | Bash command containing `git push` + `--force` |
| Skipped hooks | Bash command containing `--no-verify` |
| Hard reset | Bash command containing `reset --hard` |
| Recursive delete | Bash command containing `rm -rf` |

### Data Pipeline

```
~/.claude/projects/**/*.jsonl
        │
        ▼
   ┌─────────┐     ┌──────────────┐     ┌───────────┐
   │ Watcher  │────>│ Session Mgr  │────>│   Store   │
   │ (notify) │     │ (parse+upsert)│    │ (SQLite)  │
   └─────────┘     └──────┬───────┘     └─────┬─────┘
                          │ TrackRepo         │
                          ▼                   │
                   ┌──────────────┐           │
                   │ Git Observer │───────────┘
                   │ (poll 3s)    │  commit_link + git_event + process_metrics
                   └──────────────┘
```

1. **Watcher** tails JSONL files using `notify` + 1s polling fallback. Persists byte offsets across restarts.
2. **Session Manager** groups events into sessions. Sends `TrackRepo` to the git observer when a session's `cwd` resolves to a git repository.
3. **Git Observer** polls tracked repos every 3 seconds. On HEAD change:
   - Infers the cause (commit, amend, rebase, reset, checkout, merge, cherry-pick)
   - Creates a `commit_link` with full commit details (message, files changed, diff stats)
   - Runs process analysis on events in the time window since the last poll
   - Stores `process_metrics` linked to the commit SHA and session

### Interaction Model

Events are grouped into **interactions** — each interaction starts with a `user_prompt` and includes all subsequent events until the next prompt:

```
Interaction 1:
  user_prompt     → "Add retry logic to the HTTP client"
  assistant_text  → "I'll add retry with exponential backoff..."
  tool_use        → Read { file_path: "src/http.rs" }
  tool_result     → [file contents]
  tool_use        → Edit { file_path: "src/http.rs" }
  tool_result     → "File edited successfully"

Interaction 2:
  user_prompt     → "Now add tests"
  ...
```

The process analyzer operates on this interaction-level view, not raw events.

## Tech Stack

- **Agent**: Tokio, notify, rusqlite, axum, serde, git2, clap
- **API**: Gin, pgx, golang-jwt
- **Web**: TanStack Start, TanStack Router, React 19, Vite 7, Nitro, Tailwind v4, Zustand, Recharts, Zod
