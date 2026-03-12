# Deep Git Provenance — Design

**Goal:** Deeply couple AI coding sessions to git/code changes so every line in a PR can be traced back to the prompt that produced it, with zero developer configuration.

## Problem

The agent currently captures rich session data (prompts, tool calls, responses) but git coupling is passive — commits only link to sessions via an external `POST /hook/commit` call. The `git2` crate is in Cargo.toml but unused. Attribution only runs at PR-comment time, not during capture.

Developers also do chaotic things with git during sessions: amend, rebase, stash, reset, checkout branches, cherry-pick, force push. A provenance system that assumes linear commit history breaks constantly.

## Design Principle

**Record the git timeline as events, not as state.** Don't snapshot start/end and hope for the best. Treat git changes as an append-only stream of observations, same as JSONL events.

## Architecture

### New Component: GitObserver

A fourth concurrent tokio task in the daemon alongside watcher, session_manager, and api.

```
watcher -> parser -> session_manager -> store
                          ^                ^
                    api --+                |
                                           |
                     git_observer ---------+
```

**Responsibilities:**
- Maintain `HashMap<PathBuf, RepoState>` — one entry per repo with active sessions
- Poll HEAD every 2-3 seconds using `git2` (~1ms per repo)
- Compare current state against last-known state
- Infer what happened (commit, amend, rebase, checkout, reset, merge, cherry-pick)
- Emit git events to the store
- On new commit: create commit_link and trigger attribution

**Lifecycle:** SessionManager notifies GitObserver when sessions start/end. GitObserver starts/stops tracking repos accordingly.

### Dual Detection: Tool Parsing + Polling

**Tool parsing** (instant, for AI-made commits):
- SessionManager watches for `ToolUse` with `tool_name: "Bash"` + input containing `git commit/push/rebase/reset/checkout/stash`
- On successful ToolResult, immediately check repo HEAD and trigger attribution
- Gives you the *cause* for free (you parsed the command)

**Polling** (fallback, for manual operations):
- GitObserver polls HEAD every 2-3 seconds
- Catches commits from separate terminals, GUI clients, etc.
- If tool-parse already detected the change, poll is a no-op

### HeadChangeCause Inference

Determine what happened from before/after state (no need to hook into git):
- `old_sha` is parent of `new_sha` → **Commit**
- Same parent, different SHA → **Amend**
- Same branch, `old_sha` not in ancestry of `new_sha` → **Rebase**
- Different branch → **Checkout**
- `new_sha` is ancestor of `old_sha` → **Reset**
- `new_sha` has 2+ parents → **Merge**

### Real-Time Attribution

When a commit is detected:
1. Get interactions between last commit and this commit (time-windowed)
2. Diff the commit using `git2`
3. Match each hunk to tool calls (Write/Edit) that targeted the same file with similar content
4. Store attributions immediately — don't wait for PR time

The existing `attribution.rs` logic is the foundation. Enhancements:
- Time-windowed matching (only interactions between commits, not entire session)
- Temporal sole-edit detection (only one Write to `foo.rs` between commits → confidence 1.0)
- Bash write detection (parse `echo/cat > file` patterns)
- Deletion attribution (track removed lines, not just additions)

## Schema

### New: git_events table
```sql
CREATE TABLE git_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL REFERENCES sessions(id),
    repo_path TEXT NOT NULL,
    event_type TEXT NOT NULL,  -- head_changed, branch_changed, commit_created
    timestamp TEXT NOT NULL,
    data TEXT NOT NULL          -- JSON with event-specific fields
);
```

### New: attributions table
```sql
CREATE TABLE attributions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    commit_sha TEXT NOT NULL,
    session_id TEXT NOT NULL,
    file_path TEXT NOT NULL,
    hunk_start INTEGER NOT NULL,
    hunk_end INTEGER NOT NULL,
    event_id INTEGER REFERENCES events(id),
    confidence REAL NOT NULL,
    attribution_type TEXT NOT NULL,  -- ai_generated, ai_assisted, manual, unknown
    method TEXT NOT NULL,            -- content_match, temporal, sole_edit, bash_parse
    created_at TEXT NOT NULL,
    UNIQUE(commit_sha, file_path, hunk_start, hunk_end)
);
```

### Enriched: commit_links table (add columns)
- `parent_shas TEXT` — JSON array
- `message TEXT`
- `files_changed TEXT` — JSON array
- `diff_stat TEXT` — JSON {insertions, deletions}
- `detection_method TEXT` — poll, tool_parse, hook

## Git Chaos Handling

| Scenario | How detected | How handled |
|----------|-------------|-------------|
| Amend | Same branch, same parent, new SHA | Record both SHAs, attribute the new one |
| Rebase | Same branch, old SHA not ancestor of new | Multiple HEAD changes recorded as events |
| Stash | HEAD unchanged, dirty files change | WorkingTreeDirty event (optional) |
| Reset --hard | new SHA is ancestor of old | Record as head_changed with Reset cause |
| Checkout | Branch ref changes | Record as branch_changed |
| Force push | Not visible locally | PR-time attribution uses PR's actual SHAs |
| Squash merge | Single PR commit != session commits | attribute_diff_text matches full PR diff |

## Communication: SessionManager ↔ GitObserver

GitObserver needs to know which repos to watch. Two options:
- **Channel**: SessionManager sends repo start/stop messages
- **Shared state**: GitObserver reads SessionManager's active sessions

Channel is cleaner (unidirectional). A lightweight enum:
```rust
enum GitObserverCommand {
    TrackRepo { repo_path: PathBuf, session_id: String },
    UntrackSession { session_id: String },
}
```
