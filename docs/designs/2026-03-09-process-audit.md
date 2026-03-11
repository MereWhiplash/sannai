# Process Audit: AI Workflow Quality Signals

**Date:** 2026-03-09
**Status:** Accepted
**Evolves:** `feat/deep-git-provenance` branch

## Problem

When most code in a PR is AI-generated, labeling code origin ("AI-generated" / "manual") tells reviewers nothing — everything is AI anyway. The useful signal isn't *whether* AI was involved, but *how* it was involved: was the AI's engineering process sound?

## Core Insight

The JSONL session data already captures the full AI workflow — every prompt, tool call, error, and retry. We can audit the AI's engineering process the same way a senior engineer reviews a junior developer's work: did they understand the code before changing it? Did they write tests? Did they brute-force through errors?

## What Changes

**Pivot from:** Hunk-level code-origin attribution (which line came from which interaction)
**Pivot to:** Workflow quality signals (how good was the AI's engineering process)

Same underlying data capture (JSONL parsing, git detection, session tracking). Different extraction and presentation.

## Audience

All three, served from the same underlying data:
- **PR reviewers** — glanceable single comment with process summary
- **Team leads** — per-PR deep dive in dashboard
- **Compliance** — audit trail linking sessions to PRs with process metrics

## Process Signals

| Signal | Detection Method | What It Tells You |
|--------|-----------------|-------------------|
| Steering ratio | user_prompt count / total interactions | High = human directing, Low = AI autonomous |
| Exploration score | Read/Glob/Grep calls before first Write/Edit | Did AI understand the code before changing it? |
| Read:Write ratio | unique files read / unique files written | Low = writing blind |
| Test behavior | Detect test commands + position in sequence | TDD, test-after, or no tests? |
| Error-fix cycles | Non-zero exit Bash results → retry patterns | Normal iteration vs brute forcing |
| Red flags | --no-verify, --force, rm -rf, reset --hard | Process shortcuts and dangerous operations |
| Prompt specificity | Length, file paths, constraint keywords | Vague delegation vs precise direction |
| Session complexity | Total interactions, tool calls, files touched | Scale of the change |

## Architecture

### Agent (Rust)

```
KEEP:    parser/ → watcher/ → session/     (event capture, untouched)
KEEP:    git/observer.rs, mod.rs, tool_detect.rs  (detection layer)
REPLACE: git/attribute.rs → process/analyzer.rs   (metrics engine)
REPLACE: comment/format.rs                        (new summary format)
EVOLVE:  store/ (drop attributions, add process_metrics)
EVOLVE:  api/ (new endpoints)
```

Process analysis runs agent-side at commit detection time. Heuristics only (no LLM calls). LLM-enhanced analysis is a future upgrade path.

### New Store Table

```sql
CREATE TABLE process_metrics (
    id INTEGER PRIMARY KEY,
    session_id TEXT NOT NULL,
    commit_sha TEXT,
    steering_ratio REAL,
    exploration_score REAL,
    read_write_ratio REAL,
    test_behavior TEXT,        -- 'tdd' | 'test_after' | 'no_tests'
    error_fix_cycles INTEGER,
    red_flags TEXT,            -- JSON array
    prompt_specificity REAL,
    total_interactions INTEGER,
    total_tool_calls INTEGER,
    files_read INTEGER,
    files_written INTEGER,
    created_at TEXT NOT NULL
);
```

### PR Comment Format

Single comment, concise:

```markdown
## AI Process Audit — 3 sessions, 47 interactions

**Steering:** High — 72% of interactions had specific human prompts
**Exploration:** 12 files read before first edit
**Testing:** Warning: Tests written after code, 1 test failure fixed
**Iterations:** 2 error-fix cycles (normal)
**Red flags:** None

Link: View full session
```

### Dashboard (Web) — Per-PR Deep Dive

Progressive disclosure, three layers:

1. **Timeline view** — horizontal swimlane of the session. Color-coded: blue=explore, green=write, yellow=test, red=error, gray=commit. Red flag moments marked.
2. **Interaction list** — expand a timeline region to see prompts, tool calls, outcomes, flags.
3. **Conversation replay** — full exchange for a single interaction, annotated with process signals.

### API (Go) — New Endpoints

```
GET /sessions/{id}/process-metrics
GET /sessions/{id}/timeline
GET /sessions/{id}/interactions/{n}
GET /prs/{number}/audit
```

## Key Design Decisions

1. **Agent-side compute** — Metrics calculated in Rust at commit time. Works offline, fast, no API dependency.
2. **Heuristics first, LLM later** — Ship with deterministic heuristics. LLM-enhanced analysis (prompt quality classification, natural-language summaries) is a future premium feature.
3. **Drop hunk-level attribution** — The "which line came from which interaction" mapping is removed. Workflow audit provides more trustworthiness signal than origin labels.
4. **Single PR comment** — Not a GitHub Check. One concise comment with link to dashboard for depth.
5. **Multi-session aggregation** — PR comment aggregates metrics across all sessions via commit_links.

## Edge Cases

- **Multi-session PRs:** Walk commits → sessions → metrics via commit_links.
- **Mixed work in one session:** Time-windowing between commits scopes metrics to the relevant window.
- **Red flag false positives:** Flag but don't alarm. Neutral tone: "Detected: --force push" not "WARNING."
- **Privacy:** Conversation replay needs access controls for team visibility (future).

## What We Lose

Hunk-level attribution ("line 42-58 came from interaction #3 with 0.85 confidence"). This is the right trade — when everything is AI-generated, the process quality matters more than the origin label.

## Implementation Plan

See: docs/plans/2026-03-09-process-audit.md
