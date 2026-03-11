# Deep Git Provenance — Implementation Plan

**Design:** [docs/designs/2026-03-08-deep-git-provenance.md](../designs/2026-03-08-deep-git-provenance.md)

**Goal:** Automatically detect git changes during AI sessions, record them as events, and attribute code to the prompts that generated it — with zero developer configuration.

**Architecture:** New `GitObserver` tokio task polls repos via `git2`, dual-detection with tool-parse in SessionManager, git events stored alongside JSONL events, real-time attribution at commit time.

**EC Context:**
- Learning: sannai/agent — Rust agent has 6 modules, 33 tests. JSONL parsing uses camelCase outer / snake_case inner.

**Test command:** `cd agent && cargo test`

---

## Phase 1: Schema & Foundation

### Task 1: Schema migration — git_events table @tdd

**Files:**
- Modify: `agent/src/store/mod.rs`

**Step 1: Write failing test** (RED)
```rust
#[test]
fn test_insert_and_get_git_events() {
    let (store, _dir) = test_store();
    let now = Utc::now();

    // Create a session first
    store.upsert_session(&Session {
        id: "git-test-session".to_string(),
        tool: "claude_code".to_string(),
        project_path: Some("/test/repo".to_string()),
        started_at: now,
        ended_at: None,
        synced_at: None,
        metadata: None,
    }).unwrap();

    let event = GitEvent {
        id: None,
        session_id: "git-test-session".to_string(),
        repo_path: "/test/repo".to_string(),
        event_type: "head_changed".to_string(),
        timestamp: now,
        data: serde_json::json!({
            "old_sha": "aaa111",
            "new_sha": "bbb222",
            "branch": "main",
            "cause": "commit"
        }),
    };

    store.insert_git_event(&event).unwrap();

    let events = store.get_git_events_for_session("git-test-session").unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "head_changed");
    assert_eq!(events[0].data["new_sha"], "bbb222");
}
```

**Step 2: Run test, verify it fails**
```bash
cd agent && cargo test test_insert_and_get_git_events
```
Expected: FAIL — `GitEvent` struct and methods don't exist

**Step 3: Implement minimal code** (GREEN)

Add to MIGRATION const:
```sql
CREATE TABLE IF NOT EXISTS git_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL REFERENCES sessions(id),
    repo_path TEXT NOT NULL,
    event_type TEXT NOT NULL,
    timestamp TEXT NOT NULL,
    data TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_git_events_session ON git_events(session_id);
CREATE INDEX IF NOT EXISTS idx_git_events_timestamp ON git_events(timestamp);
```

Add `GitEvent` struct and CRUD methods to `Store`.

**Step 4: Run test, verify it passes** @verifying
```bash
cd agent && cargo test test_insert_and_get_git_events
```
Expected: PASS

**Step 5: Commit**
```bash
git add -A && git commit -m "feat(store): add git_events table and CRUD"
```

---

### Task 2: Schema migration — attributions table @tdd

**Files:**
- Modify: `agent/src/store/mod.rs`

**Step 1: Write failing test** (RED)
```rust
#[test]
fn test_insert_and_get_attributions() {
    let (store, _dir) = test_store();
    let now = Utc::now();

    store.upsert_session(&Session {
        id: "attr-session".to_string(),
        tool: "claude_code".to_string(),
        project_path: Some("/test/repo".to_string()),
        started_at: now,
        ended_at: None,
        synced_at: None,
        metadata: None,
    }).unwrap();

    let attr = Attribution {
        id: None,
        commit_sha: "abc123".to_string(),
        session_id: "attr-session".to_string(),
        file_path: "src/main.rs".to_string(),
        hunk_start: 10,
        hunk_end: 15,
        event_id: None,
        confidence: 0.95,
        attribution_type: "ai_generated".to_string(),
        method: "content_match".to_string(),
        created_at: now,
    };

    store.insert_attribution(&attr).unwrap();

    let attrs = store.get_attributions_for_commit("abc123").unwrap();
    assert_eq!(attrs.len(), 1);
    assert_eq!(attrs[0].file_path, "src/main.rs");
    assert!((attrs[0].confidence - 0.95).abs() < f32::EPSILON);
}

#[test]
fn test_attribution_uniqueness() {
    let (store, _dir) = test_store();
    let now = Utc::now();

    store.upsert_session(&Session {
        id: "unique-session".to_string(),
        tool: "claude_code".to_string(),
        project_path: None,
        started_at: now,
        ended_at: None,
        synced_at: None,
        metadata: None,
    }).unwrap();

    let attr = Attribution {
        id: None,
        commit_sha: "abc123".to_string(),
        session_id: "unique-session".to_string(),
        file_path: "src/main.rs".to_string(),
        hunk_start: 10,
        hunk_end: 15,
        event_id: None,
        confidence: 0.8,
        attribution_type: "ai_generated".to_string(),
        method: "content_match".to_string(),
        created_at: now,
    };

    store.insert_attribution(&attr).unwrap();
    // Second insert with same unique key should upsert (update confidence)
    let attr2 = Attribution { confidence: 0.95, ..attr };
    store.insert_attribution(&attr2).unwrap();

    let attrs = store.get_attributions_for_commit("abc123").unwrap();
    assert_eq!(attrs.len(), 1);
    assert!((attrs[0].confidence - 0.95).abs() < f32::EPSILON);
}
```

**Step 2: Run test, verify it fails**
```bash
cd agent && cargo test test_insert_and_get_attributions test_attribution_uniqueness
```
Expected: FAIL — `Attribution` struct doesn't exist

**Step 3: Implement minimal code** (GREEN)

Add to MIGRATION:
```sql
CREATE TABLE IF NOT EXISTS attributions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    commit_sha TEXT NOT NULL,
    session_id TEXT NOT NULL,
    file_path TEXT NOT NULL,
    hunk_start INTEGER NOT NULL,
    hunk_end INTEGER NOT NULL,
    event_id INTEGER REFERENCES events(id),
    confidence REAL NOT NULL,
    attribution_type TEXT NOT NULL,
    method TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE(commit_sha, file_path, hunk_start, hunk_end)
);
CREATE INDEX IF NOT EXISTS idx_attributions_commit ON attributions(commit_sha);
CREATE INDEX IF NOT EXISTS idx_attributions_session ON attributions(session_id);
```

Add `Attribution` struct and CRUD methods. Use `INSERT OR REPLACE` for upsert on unique constraint.

**Step 4: Run test, verify it passes** @verifying
```bash
cd agent && cargo test test_insert_and_get_attributions test_attribution_uniqueness
```
Expected: PASS

**Step 5: Commit**
```bash
git add -A && git commit -m "feat(store): add attributions table and CRUD"
```

---

### Task 3: Enrich commit_links table @tdd

**Files:**
- Modify: `agent/src/store/mod.rs`

**Step 1: Write failing test** (RED)
```rust
#[test]
fn test_enriched_commit_link() {
    let (store, _dir) = test_store();
    let now = Utc::now();

    store.upsert_session(&Session {
        id: "enrich-session".to_string(),
        tool: "claude_code".to_string(),
        project_path: Some("/test/repo".to_string()),
        started_at: now,
        ended_at: None,
        synced_at: None,
        metadata: None,
    }).unwrap();

    let link = CommitLink {
        commit_sha: "abc123".to_string(),
        session_id: "enrich-session".to_string(),
        repo_path: "/test/repo".to_string(),
        linked_at: now,
        parent_shas: Some(vec!["parent1".to_string()]),
        message: Some("feat: add auth".to_string()),
        files_changed: Some(vec!["src/auth.rs".to_string(), "Cargo.toml".to_string()]),
        diff_stat: Some(serde_json::json!({"insertions": 45, "deletions": 3})),
        detection_method: Some("poll".to_string()),
    };

    store.link_commit(&link).unwrap();

    let sessions = store.get_sessions_for_commit("abc123").unwrap();
    assert_eq!(sessions.len(), 1);

    let links = store.get_commit_links_for_session("enrich-session").unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].message.as_deref(), Some("feat: add auth"));
    assert_eq!(links[0].detection_method.as_deref(), Some("poll"));
}
```

**Step 2: Run test, verify it fails**
```bash
cd agent && cargo test test_enriched_commit_link
```
Expected: FAIL — `CommitLink` doesn't have the new fields

**Step 3: Implement minimal code** (GREEN)

Add new fields to `CommitLink` struct (all `Option` for backwards compat). Add new columns to migration. Add `get_commit_links_for_session` method. Update `link_commit` INSERT to include new columns.

**Step 4: Run test, verify it passes** @verifying
```bash
cd agent && cargo test test_enriched_commit_link
```
Expected: PASS

**Step 5: Also verify existing commit_links tests still pass**
```bash
cd agent && cargo test test_commit_links
```
Expected: PASS (existing test uses default None for new fields)

**Step 6: Commit**
```bash
git add -A && git commit -m "feat(store): enrich commit_links with parent_shas, message, files, diff_stat"
```

---

## Phase 2: GitObserver Core

### Task 4: Git helpers — repo discovery and state reading @tdd

**Files:**
- Create: `agent/src/git/mod.rs`
- Modify: `agent/src/lib.rs` (add `pub mod git;`)

**Step 1: Write failing test** (RED)
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn init_test_repo() -> tempfile::TempDir {
        let dir = tempfile::TempDir::new().unwrap();
        Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output().unwrap();
        Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(dir.path())
            .output().unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(dir.path())
            .output().unwrap();
        // Initial commit
        std::fs::write(dir.path().join("README.md"), "# test").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(dir.path())
            .output().unwrap();
        Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(dir.path())
            .output().unwrap();
        dir
    }

    #[test]
    fn test_discover_repo() {
        let dir = init_test_repo();
        let subdir = dir.path().join("src");
        std::fs::create_dir(&subdir).unwrap();

        // Should find repo from subdirectory
        let repo_path = discover_repo(&subdir).unwrap();
        assert_eq!(repo_path, dir.path().canonicalize().unwrap());

        // Should fail for non-repo
        let tmp = tempfile::TempDir::new().unwrap();
        assert!(discover_repo(tmp.path()).is_none());
    }

    #[test]
    fn test_read_repo_state() {
        let dir = init_test_repo();
        let state = read_repo_state(dir.path()).unwrap();

        assert!(!state.head_sha.is_empty());
        assert!(state.branch.is_some());
        assert!(state.dirty_files.is_empty());
    }

    #[test]
    fn test_read_repo_state_dirty() {
        let dir = init_test_repo();
        std::fs::write(dir.path().join("new.txt"), "dirty").unwrap();

        let state = read_repo_state(dir.path()).unwrap();
        assert_eq!(state.dirty_files.len(), 1);
        assert_eq!(state.dirty_files[0].path, "new.txt");
    }
}
```

**Step 2: Run test, verify it fails**
```bash
cd agent && cargo test git::tests
```
Expected: FAIL — module doesn't exist

**Step 3: Implement minimal code** (GREEN)

```rust
use git2::Repository;
use std::path::{Path, PathBuf};

pub struct RepoState {
    pub head_sha: String,
    pub branch: Option<String>,
    pub dirty_files: Vec<DirtyFile>,
}

pub struct DirtyFile {
    pub path: String,
    pub status: FileStatus,
}

pub enum FileStatus {
    Modified,
    Added,
    Deleted,
    Renamed,
}

pub fn discover_repo(path: &Path) -> Option<PathBuf> {
    Repository::discover(path)
        .ok()
        .and_then(|repo| repo.workdir().map(|p| p.to_path_buf()))
}

pub fn read_repo_state(repo_path: &Path) -> anyhow::Result<RepoState> {
    let repo = Repository::open(repo_path)?;
    let head = repo.head()?;
    let head_sha = head.peel_to_commit()?.id().to_string();
    let branch = head.shorthand().map(|s| s.to_string());

    let mut dirty_files = Vec::new();
    let statuses = repo.statuses(None)?;
    for entry in statuses.iter() {
        if let Some(path) = entry.path() {
            let status = entry.status();
            let file_status = if status.intersects(git2::Status::WT_NEW | git2::Status::INDEX_NEW) {
                FileStatus::Added
            } else if status.intersects(git2::Status::WT_DELETED | git2::Status::INDEX_DELETED) {
                FileStatus::Deleted
            } else if status.intersects(git2::Status::WT_RENAMED | git2::Status::INDEX_RENAMED) {
                FileStatus::Renamed
            } else {
                FileStatus::Modified
            };
            dirty_files.push(DirtyFile {
                path: path.to_string(),
                status: file_status,
            });
        }
    }

    Ok(RepoState { head_sha, branch, dirty_files })
}
```

**Step 4: Run test, verify it passes** @verifying
```bash
cd agent && cargo test git::tests
```
Expected: PASS

**Step 5: Commit**
```bash
git add -A && git commit -m "feat(git): add repo discovery and state reading via git2"
```

---

### Task 5: HeadChangeCause inference @tdd

**Files:**
- Modify: `agent/src/git/mod.rs`

**Step 1: Write failing test** (RED)
```rust
#[derive(Debug, Clone, PartialEq)]
pub enum HeadChangeCause {
    Commit,
    Amend,
    Rebase,
    Checkout,
    Reset,
    Merge,
    CherryPick,
    Unknown,
}

#[test]
fn test_infer_cause_commit() {
    let dir = init_test_repo();
    let state_before = read_repo_state(dir.path()).unwrap();

    // Make a new commit
    std::fs::write(dir.path().join("file.txt"), "content").unwrap();
    Command::new("git").args(["add", "."]).current_dir(dir.path()).output().unwrap();
    Command::new("git").args(["commit", "-m", "second"]).current_dir(dir.path()).output().unwrap();

    let state_after = read_repo_state(dir.path()).unwrap();
    let cause = infer_head_change_cause(dir.path(), &state_before.head_sha, &state_after.head_sha).unwrap();
    assert_eq!(cause, HeadChangeCause::Commit);
}

#[test]
fn test_infer_cause_amend() {
    let dir = init_test_repo();

    // Make a commit then amend
    std::fs::write(dir.path().join("file.txt"), "v1").unwrap();
    Command::new("git").args(["add", "."]).current_dir(dir.path()).output().unwrap();
    Command::new("git").args(["commit", "-m", "original"]).current_dir(dir.path()).output().unwrap();
    let state_before = read_repo_state(dir.path()).unwrap();

    std::fs::write(dir.path().join("file.txt"), "v2").unwrap();
    Command::new("git").args(["add", "."]).current_dir(dir.path()).output().unwrap();
    Command::new("git").args(["commit", "--amend", "-m", "amended"]).current_dir(dir.path()).output().unwrap();

    let state_after = read_repo_state(dir.path()).unwrap();
    let cause = infer_head_change_cause(dir.path(), &state_before.head_sha, &state_after.head_sha).unwrap();
    assert_eq!(cause, HeadChangeCause::Amend);
}

#[test]
fn test_infer_cause_checkout() {
    let dir = init_test_repo();
    let state_before = read_repo_state(dir.path()).unwrap();

    Command::new("git").args(["checkout", "-b", "other"]).current_dir(dir.path()).output().unwrap();
    std::fs::write(dir.path().join("file.txt"), "on other").unwrap();
    Command::new("git").args(["add", "."]).current_dir(dir.path()).output().unwrap();
    Command::new("git").args(["commit", "-m", "on other"]).current_dir(dir.path()).output().unwrap();

    // Switch back
    Command::new("git").args(["checkout", state_before.branch.as_deref().unwrap()]).current_dir(dir.path()).output().unwrap();

    let state_after = read_repo_state(dir.path()).unwrap();
    // HEAD went back to original SHA — but branch changed during the journey
    // For this test, we check checkout to "other" branch
    Command::new("git").args(["checkout", "other"]).current_dir(dir.path()).output().unwrap();
    let state_other = read_repo_state(dir.path()).unwrap();
    let cause = infer_head_change_cause(dir.path(), &state_before.head_sha, &state_other.head_sha).unwrap();
    // old_sha is not parent of new_sha, and new_sha is not ancestor of old — could be checkout or cherry-pick
    // With branch change context, this is a checkout. Without branch context, infer from ancestry.
    assert!(matches!(cause, HeadChangeCause::Checkout | HeadChangeCause::CherryPick | HeadChangeCause::Unknown));
}

#[test]
fn test_infer_cause_reset() {
    let dir = init_test_repo();
    let initial_sha = read_repo_state(dir.path()).unwrap().head_sha;

    // Make two commits
    std::fs::write(dir.path().join("file.txt"), "v1").unwrap();
    Command::new("git").args(["add", "."]).current_dir(dir.path()).output().unwrap();
    Command::new("git").args(["commit", "-m", "c1"]).current_dir(dir.path()).output().unwrap();

    let state_before = read_repo_state(dir.path()).unwrap();

    // Reset back
    Command::new("git").args(["reset", "--hard", &initial_sha]).current_dir(dir.path()).output().unwrap();

    let state_after = read_repo_state(dir.path()).unwrap();
    let cause = infer_head_change_cause(dir.path(), &state_before.head_sha, &state_after.head_sha).unwrap();
    assert_eq!(cause, HeadChangeCause::Reset);
}
```

**Step 2: Run test, verify it fails**
```bash
cd agent && cargo test test_infer_cause
```
Expected: FAIL — `infer_head_change_cause` doesn't exist

**Step 3: Implement minimal code** (GREEN)

```rust
pub fn infer_head_change_cause(
    repo_path: &Path,
    old_sha: &str,
    new_sha: &str,
) -> anyhow::Result<HeadChangeCause> {
    if old_sha == new_sha {
        return Ok(HeadChangeCause::Unknown);
    }

    let repo = Repository::open(repo_path)?;
    let old_oid = git2::Oid::from_str(old_sha)?;
    let new_oid = git2::Oid::from_str(new_sha)?;
    let new_commit = repo.find_commit(new_oid)?;

    // Check if merge (multiple parents)
    if new_commit.parent_count() > 1 {
        return Ok(HeadChangeCause::Merge);
    }

    // Check if old_sha is parent of new_sha (normal commit)
    if new_commit.parent_count() == 1 {
        let parent = new_commit.parent(0)?;
        if parent.id() == old_oid {
            return Ok(HeadChangeCause::Commit);
        }

        // Same parent as old commit = amend
        if let Ok(old_commit) = repo.find_commit(old_oid) {
            if old_commit.parent_count() == 1 && old_commit.parent(0)?.id() == parent.id() {
                return Ok(HeadChangeCause::Amend);
            }
        }
    }

    // Check if new_sha is ancestor of old_sha (reset)
    if repo.graph_descendant_of(old_oid, new_oid)? {
        return Ok(HeadChangeCause::Reset);
    }

    // Check if old_sha is ancestor of new_sha but not direct parent (rebase or fast-forward)
    if repo.graph_descendant_of(new_oid, old_oid)? {
        return Ok(HeadChangeCause::Rebase);
    }

    // Diverged — likely checkout or cherry-pick
    Ok(HeadChangeCause::Unknown)
}
```

**Step 4: Run test, verify it passes** @verifying
```bash
cd agent && cargo test test_infer_cause
```
Expected: PASS

**Step 5: Commit**
```bash
git add -A && git commit -m "feat(git): add HeadChangeCause inference from SHA comparison"
```

---

### Task 6: Commit detail extraction @tdd

**Files:**
- Modify: `agent/src/git/mod.rs`

**Step 1: Write failing test** (RED)
```rust
#[test]
fn test_get_commit_details() {
    let dir = init_test_repo();

    std::fs::write(dir.path().join("src/main.rs"), "fn main() {}").unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/main.rs"), "fn main() {}").unwrap();
    Command::new("git").args(["add", "."]).current_dir(dir.path()).output().unwrap();
    Command::new("git").args(["commit", "-m", "add main"]).current_dir(dir.path()).output().unwrap();

    let state = read_repo_state(dir.path()).unwrap();
    let details = get_commit_details(dir.path(), &state.head_sha).unwrap();

    assert_eq!(details.message, "add main");
    assert!(!details.parent_shas.is_empty());
    assert!(details.files_changed.contains(&"src/main.rs".to_string()));
    assert!(details.insertions > 0);
}
```

**Step 2: Run test, verify fails**
```bash
cd agent && cargo test test_get_commit_details
```

**Step 3: Implement** (GREEN)

```rust
pub struct CommitDetails {
    pub sha: String,
    pub parent_shas: Vec<String>,
    pub message: String,
    pub author: String,
    pub files_changed: Vec<String>,
    pub insertions: u32,
    pub deletions: u32,
}

pub fn get_commit_details(repo_path: &Path, sha: &str) -> anyhow::Result<CommitDetails> {
    let repo = Repository::open(repo_path)?;
    let oid = git2::Oid::from_str(sha)?;
    let commit = repo.find_commit(oid)?;

    let message = commit.message().unwrap_or("").trim().to_string();
    let author = commit.author().name().unwrap_or("unknown").to_string();
    let parent_shas: Vec<String> = commit.parent_ids().map(|id| id.to_string()).collect();

    let tree = commit.tree()?;
    let parent_tree = if commit.parent_count() > 0 {
        Some(commit.parent(0)?.tree()?)
    } else {
        None
    };

    let diff = repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)?;
    let stats = diff.stats()?;

    let mut files_changed = Vec::new();
    diff.foreach(
        &mut |delta, _| {
            if let Some(path) = delta.new_file().path().and_then(|p| p.to_str()) {
                files_changed.push(path.to_string());
            }
            true
        },
        None, None, None,
    )?;

    Ok(CommitDetails {
        sha: sha.to_string(),
        parent_shas,
        message,
        author,
        files_changed,
        insertions: stats.insertions() as u32,
        deletions: stats.deletions() as u32,
    })
}
```

**Step 4: Run test, verify passes** @verifying
```bash
cd agent && cargo test test_get_commit_details
```

**Step 5: Commit**
```bash
git add -A && git commit -m "feat(git): add commit detail extraction via git2"
```

---

## Phase 3: GitObserver Task

### Task 7: GitObserver struct and poll loop @tdd

**Files:**
- Create: `agent/src/git/observer.rs`
- Modify: `agent/src/git/mod.rs` (add `pub mod observer;`)

**Step 1: Write failing test** (RED)
```rust
#[tokio::test]
async fn test_observer_detects_new_commit() {
    let dir = init_test_repo();
    let db_dir = tempfile::TempDir::new().unwrap();
    let store = Arc::new(Mutex::new(Store::open(&db_dir.path().join("test.db")).unwrap()));

    // Create a session pointing at our test repo
    store.lock().await.upsert_session(&Session {
        id: "obs-session".to_string(),
        tool: "claude_code".to_string(),
        project_path: Some(dir.path().to_string_lossy().to_string()),
        started_at: Utc::now(),
        ended_at: None,
        synced_at: None,
        metadata: None,
    }).unwrap();

    let (cmd_tx, cmd_rx) = mpsc::channel(10);
    let cancel = CancellationToken::new();

    let mut observer = GitObserver::new(store.clone(), cmd_rx);

    // Tell observer to track this repo
    cmd_tx.send(GitObserverCommand::TrackRepo {
        repo_path: dir.path().to_path_buf(),
        session_id: "obs-session".to_string(),
    }).await.unwrap();

    // Make a commit in the test repo
    std::fs::write(dir.path().join("new.txt"), "hello").unwrap();
    Command::new("git").args(["add", "."]).current_dir(dir.path()).output().unwrap();
    Command::new("git").args(["commit", "-m", "detected commit"]).current_dir(dir.path()).output().unwrap();

    // Run one poll cycle
    observer.process_commands().await;
    observer.poll_repos().await;

    // Check that a git_event was recorded
    let events = store.lock().await.get_git_events_for_session("obs-session").unwrap();
    assert!(events.len() >= 1);
    assert_eq!(events.last().unwrap().event_type, "head_changed");

    // Check that a commit_link was created
    let head_sha = read_repo_state(dir.path()).unwrap().head_sha;
    let linked = store.lock().await.get_sessions_for_commit(&head_sha).unwrap();
    assert_eq!(linked.len(), 1);
    assert_eq!(linked[0].id, "obs-session");
}
```

**Step 2: Run test, verify fails**
```bash
cd agent && cargo test test_observer_detects_new_commit
```

**Step 3: Implement** (GREEN)

```rust
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::sync::{mpsc, Mutex};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub enum GitObserverCommand {
    TrackRepo { repo_path: PathBuf, session_id: String },
    UntrackSession { session_id: String },
}

struct TrackedRepo {
    repo_path: PathBuf,
    session_ids: Vec<String>,
    last_head_sha: String,
    last_branch: Option<String>,
}

pub struct GitObserver {
    store: Arc<Mutex<Store>>,
    cmd_rx: mpsc::Receiver<GitObserverCommand>,
    tracked: HashMap<PathBuf, TrackedRepo>,
}

impl GitObserver {
    pub fn new(store: Arc<Mutex<Store>>, cmd_rx: mpsc::Receiver<GitObserverCommand>) -> Self { ... }

    pub async fn run(&mut self, cancel: CancellationToken) -> anyhow::Result<()> {
        let mut poll_interval = tokio::time::interval(std::time::Duration::from_secs(3));
        loop {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => break,
                _ = poll_interval.tick() => {
                    self.process_commands().await;
                    self.poll_repos().await;
                }
            }
        }
        Ok(())
    }

    pub async fn process_commands(&mut self) { /* drain cmd_rx, add/remove tracked repos */ }

    pub async fn poll_repos(&mut self) {
        for tracked in self.tracked.values_mut() {
            if let Ok(state) = read_repo_state(&tracked.repo_path) {
                if state.head_sha != tracked.last_head_sha {
                    let cause = infer_head_change_cause(
                        &tracked.repo_path,
                        &tracked.last_head_sha,
                        &state.head_sha,
                    ).unwrap_or(HeadChangeCause::Unknown);

                    // Record git_event
                    // If cause is Commit/Amend/Merge/CherryPick, create commit_link + get details
                    // Update tracked state
                }
            }
        }
    }
}
```

**Step 4: Run test, verify passes** @verifying
```bash
cd agent && cargo test test_observer_detects_new_commit
```

**Step 5: Commit**
```bash
git add -A && git commit -m "feat(git): add GitObserver with poll-based commit detection"
```

---

### Task 8: Observer handles amend and reset @tdd

**Files:**
- Modify: `agent/src/git/observer.rs`

**Step 1: Write failing test** (RED)
```rust
#[tokio::test]
async fn test_observer_detects_amend() {
    let dir = init_test_repo();
    // ... setup observer tracking repo ...

    // Make commit, poll, then amend, poll again
    std::fs::write(dir.path().join("file.txt"), "v1").unwrap();
    Command::new("git").args(["add", "."]).current_dir(dir.path()).output().unwrap();
    Command::new("git").args(["commit", "-m", "original"]).current_dir(dir.path()).output().unwrap();
    observer.poll_repos().await;

    std::fs::write(dir.path().join("file.txt"), "v2").unwrap();
    Command::new("git").args(["add", "."]).current_dir(dir.path()).output().unwrap();
    Command::new("git").args(["commit", "--amend", "-m", "amended"]).current_dir(dir.path()).output().unwrap();
    observer.poll_repos().await;

    let events = store.lock().await.get_git_events_for_session("obs-session").unwrap();
    let amend_event = events.iter().find(|e| e.data["cause"] == "amend");
    assert!(amend_event.is_some());

    // Both SHAs should have commit_links
    assert_eq!(
        store.lock().await.get_git_events_for_session("obs-session").unwrap()
            .iter().filter(|e| e.event_type == "head_changed").count(),
        2
    );
}

#[tokio::test]
async fn test_observer_detects_reset() {
    // ... similar: commit, poll, reset --hard, poll, verify reset cause recorded ...
}
```

**Step 2-5:** Implement, verify, commit.
```bash
git add -A && git commit -m "feat(git): observer handles amend and reset detection"
```

---

### Task 9: Wire GitObserver into daemon @tdd

**Files:**
- Modify: `agent/src/main.rs`
- Modify: `agent/src/session/mod.rs`

**Step 1: Write failing test** (RED)

Test that SessionManager sends TrackRepo when a new session with a git repo is created:
```rust
#[tokio::test]
async fn test_session_manager_sends_track_command() {
    let (mut mgr, _store, mut git_cmd_rx) = test_session_mgr_with_git().await;
    let now = Utc::now();

    // Process an event with a cwd that is a git repo
    // (use a real temp git repo)
    let dir = init_test_repo();
    let event = make_watcher_event(ParsedEvent::UserPrompt {
        session_id: "git-sess".to_string(),
        uuid: "u-1".to_string(),
        timestamp: now,
        content: "Hello".to_string(),
        cwd: Some(dir.path().to_string_lossy().to_string()),
        git_branch: Some("main".to_string()),
    });

    mgr.process_event(event).await.unwrap();

    // Should have sent a TrackRepo command
    let cmd = git_cmd_rx.try_recv().unwrap();
    match cmd {
        GitObserverCommand::TrackRepo { session_id, .. } => {
            assert_eq!(session_id, "git-sess");
        }
        _ => panic!("Expected TrackRepo"),
    }
}
```

**Step 2: Run test, verify fails**

**Step 3: Implement** (GREEN)

- Add `git_cmd_tx: Option<mpsc::Sender<GitObserverCommand>>` to `SessionManager`
- In `ensure_session`, when creating a new session, call `discover_repo(project_path)` and if found, send `TrackRepo`
- In `end_session`, send `UntrackSession`
- In `run_daemon()` in main.rs, create the channel and spawn GitObserver as a 4th task

**Step 4: Run test, verify passes** @verifying
```bash
cd agent && cargo test
```

**Step 5: Commit**
```bash
git add -A && git commit -m "feat: wire GitObserver into daemon as 4th concurrent task"
```

---

## Phase 4: Tool-Parse Detection

### Task 10: Detect git commands in Bash tool calls @tdd

**Files:**
- Create: `agent/src/git/tool_detect.rs`

**Step 1: Write failing test** (RED)
```rust
#[test]
fn test_detect_git_commit_in_bash() {
    assert_eq!(
        detect_git_command("git commit -m 'feat: add auth'"),
        Some(DetectedGitOp::Commit { amend: false, message: Some("feat: add auth".into()) })
    );
    assert_eq!(
        detect_git_command("git commit --amend -m 'fix'"),
        Some(DetectedGitOp::Commit { amend: true, message: Some("fix".into()) })
    );
    assert_eq!(
        detect_git_command("git push origin main"),
        Some(DetectedGitOp::Push { force: false })
    );
    assert_eq!(
        detect_git_command("git push --force"),
        Some(DetectedGitOp::Push { force: true })
    );
    assert_eq!(
        detect_git_command("git checkout -b feat/new"),
        Some(DetectedGitOp::Checkout { branch: Some("feat/new".into()) })
    );
    assert_eq!(
        detect_git_command("git rebase main"),
        Some(DetectedGitOp::Rebase)
    );
    assert_eq!(
        detect_git_command("git stash"),
        Some(DetectedGitOp::Stash { pop: false })
    );
    assert_eq!(
        detect_git_command("git stash pop"),
        Some(DetectedGitOp::Stash { pop: true })
    );
    assert_eq!(
        detect_git_command("git reset --hard HEAD~1"),
        Some(DetectedGitOp::Reset { hard: true })
    );
    // Not a git command
    assert_eq!(detect_git_command("cargo build"), None);
    // Git read-only commands should be ignored
    assert_eq!(detect_git_command("git status"), None);
    assert_eq!(detect_git_command("git log"), None);
    assert_eq!(detect_git_command("git diff"), None);
}
```

**Step 2: Run test, verify fails**

**Step 3: Implement** (GREEN)

Parse the bash input string for git subcommands. Only detect mutating operations. Use simple string matching — no need for a full shell parser.

**Step 4: Run test, verify passes** @verifying

**Step 5: Commit**
```bash
git add -A && git commit -m "feat(git): detect git commands in Bash tool call inputs"
```

---

### Task 11: SessionManager triggers immediate poll on git tool detection @tdd

**Files:**
- Modify: `agent/src/session/mod.rs`

**Step 1: Write failing test** (RED)
```rust
#[tokio::test]
async fn test_git_bash_triggers_poll() {
    // Setup session manager with git observer channel
    // Process a ToolUse event with tool_name "Bash" and input containing "git commit -m 'test'"
    // Then process the successful ToolResult
    // Verify an ImmediatePoll command was sent to the git observer
}
```

**Step 2-5:** Implement: Add `ImmediatePoll { repo_path }` variant to `GitObserverCommand`. In `process_event` for `ToolResult`, check if the preceding `ToolUse` was a detected git command and the result is not an error. If so, send `ImmediatePoll`.

```bash
git add -A && git commit -m "feat: trigger immediate git poll on Bash git commands"
```

---

## Phase 5: Real-Time Attribution

### Task 12: Time-windowed interaction retrieval @tdd

**Files:**
- Modify: `agent/src/store/mod.rs`

**Step 1: Write failing test** (RED)
```rust
#[test]
fn test_get_events_in_time_range() {
    let (store, _dir) = test_store();
    let now = Utc::now();

    store.upsert_session(&Session {
        id: "range-session".to_string(),
        tool: "claude_code".to_string(),
        project_path: None,
        started_at: now,
        ended_at: None,
        synced_at: None,
        metadata: None,
    }).unwrap();

    // Insert events at different times
    for i in 0..5 {
        store.insert_event(&Event {
            id: None,
            session_id: "range-session".to_string(),
            event_type: "tool_use".to_string(),
            content: Some(format!("event-{}", i)),
            context_files: None,
            timestamp: now + chrono::Duration::minutes(i),
            metadata: None,
        }).unwrap();
    }

    // Get events between minute 1 and minute 3
    let from = now + chrono::Duration::minutes(1);
    let to = now + chrono::Duration::minutes(3);
    let events = store.get_events_in_time_range("range-session", from, to).unwrap();
    assert_eq!(events.len(), 3); // minutes 1, 2, 3
}
```

**Step 2-5:** Implement `get_events_in_time_range` with SQL `BETWEEN` on timestamp.

```bash
git add -A && git commit -m "feat(store): add time-range event queries for attribution windowing"
```

---

### Task 13: Attribution at commit time @tdd

**Files:**
- Create: `agent/src/git/attribute.rs`

**Step 1: Write failing test** (RED)
```rust
#[tokio::test]
async fn test_attribute_commit_to_session() {
    let dir = init_test_repo();
    let db_dir = tempfile::TempDir::new().unwrap();
    let store = Arc::new(Mutex::new(Store::open(&db_dir.path().join("test.db")).unwrap()));

    let now = Utc::now();
    store.lock().await.upsert_session(&Session {
        id: "attr-sess".to_string(),
        tool: "claude_code".to_string(),
        project_path: Some(dir.path().to_string_lossy().to_string()),
        started_at: now,
        ended_at: None,
        synced_at: None,
        metadata: None,
    }).unwrap();

    // Simulate a Write tool call event
    store.lock().await.insert_event(&Event {
        id: None,
        session_id: "attr-sess".to_string(),
        event_type: "tool_use".to_string(),
        content: Some("Write".to_string()),
        context_files: None,
        timestamp: now + chrono::Duration::seconds(5),
        metadata: Some(serde_json::json!({
            "tool_id": "toolu_01",
            "input": {
                "file_path": dir.path().join("src/auth.rs").to_string_lossy().to_string(),
                "content": "fn authenticate() {\n    validate_token();\n}\n"
            }
        })),
    }).unwrap();

    // Actually write the file and commit
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/auth.rs"), "fn authenticate() {\n    validate_token();\n}\n").unwrap();
    Command::new("git").args(["add", "."]).current_dir(dir.path()).output().unwrap();
    Command::new("git").args(["commit", "-m", "add auth"]).current_dir(dir.path()).output().unwrap();

    let head = read_repo_state(dir.path()).unwrap().head_sha;

    // Run attribution
    let count = attribute_commit(
        &store,
        dir.path(),
        &head,
        "attr-sess",
        now, // from
        now + chrono::Duration::minutes(1), // to
    ).await.unwrap();

    assert!(count > 0);

    let attrs = store.lock().await.get_attributions_for_commit(&head).unwrap();
    assert!(!attrs.is_empty());
    assert_eq!(attrs[0].file_path, "src/auth.rs");
    assert!(attrs[0].confidence > 0.5);
    assert_eq!(attrs[0].attribution_type, "ai_generated");
}
```

**Step 2: Run test, verify fails**

**Step 3: Implement** (GREEN)

`attribute_commit` function:
1. Get events in time range from store
2. Build interactions from those events
3. Use existing `attribute_diff` from `provenance/attribution.rs` (but with `git2` instead of CLI)
4. Store each `DiffAttribution` as an `Attribution` row

**Step 4: Run test, verify passes** @verifying

**Step 5: Commit**
```bash
git add -A && git commit -m "feat(git): real-time attribution at commit detection time"
```

---

### Task 14: Wire attribution into GitObserver poll loop @tdd

**Files:**
- Modify: `agent/src/git/observer.rs`

**Step 1: Write failing test** (RED)
```rust
#[tokio::test]
async fn test_observer_creates_attributions_on_commit() {
    // Full integration test:
    // 1. Setup observer + store with session
    // 2. Insert a Write tool_use event into store
    // 3. Actually write the file in the test repo
    // 4. Commit in the test repo
    // 5. Poll the observer
    // 6. Verify attributions table has entries
}
```

**Step 2-5:** Wire `attribute_commit` call into `poll_repos` after commit detection. Run attribution in a spawned task to avoid blocking the poll loop.

```bash
git add -A && git commit -m "feat(git): observer triggers attribution on commit detection"
```

---

## Phase 6: Integration & Robustness

### Task 15: Git observer handles repo errors gracefully @tdd

**Files:**
- Modify: `agent/src/git/observer.rs`

Test cases:
- Repo directory deleted while being tracked → log warning, untrack
- Repo in detached HEAD state → still works
- Repo with no commits (bare init) → skip until first commit
- Multiple sessions tracking same repo → single TrackedRepo entry, multiple session_ids

```bash
git add -A && git commit -m "feat(git): robust error handling for edge cases"
```

---

### Task 16: Update PR comment to use stored attributions @tdd

**Files:**
- Modify: `agent/src/main.rs` (run_comment function)

Instead of computing attribution at PR-comment time, query the `attributions` table for the PR's commit SHAs. Fall back to on-the-fly attribution if no stored attributions exist (for commits made before the observer was running).

```bash
git add -A && git commit -m "feat: PR comments use stored attributions with on-the-fly fallback"
```

---

### Task 17: API endpoints for git data @tdd

**Files:**
- Modify: `agent/src/api/mod.rs`

New endpoints:
- `GET /sessions/{id}/git-events` — git timeline for a session
- `GET /sessions/{id}/commits` — enriched commit_links for a session
- `GET /commits/{sha}/attributions` — attributions for a commit
- `GET /sessions/{id}/attributions` — all attributions for a session

```bash
git add -A && git commit -m "feat(api): add git events, commits, and attributions endpoints"
```

---

### Task 18: Full integration test @tdd

**Files:**
- Create: `agent/tests/git_integration.rs`

End-to-end test:
1. Create temp git repo with initial commit
2. Start store, session manager, git observer
3. Simulate session events (user prompt → tool_use Write → tool_result)
4. Actually write the file in the repo and commit
5. Poll observer
6. Verify: session exists, commit_link created, git_events recorded, attributions stored
7. Query via store methods and verify the full provenance chain

```bash
git add -A && git commit -m "test: full git provenance integration test"
```

---

## Summary

| Phase | Tasks | What it delivers |
|-------|-------|-----------------|
| 1: Schema | 1-3 | git_events, attributions, enriched commit_links tables |
| 2: Git Core | 4-6 | git2-based repo state reading, HEAD change inference, commit details |
| 3: Observer | 7-9 | GitObserver task, wired into daemon lifecycle |
| 4: Tool Parse | 10-11 | Instant detection of git commands from Bash tool calls |
| 5: Attribution | 12-14 | Real-time attribution at commit time, stored permanently |
| 6: Integration | 15-18 | Error handling, API endpoints, PR comment update, e2e test |

**Patterns to Store:**
- git2 repo state polling pattern (open once, cache handle, read HEAD + statuses)
- HeadChangeCause inference from SHA ancestry
- Time-windowed attribution (only match interactions between commits)
- Dual detection pattern (tool-parse for instant + poll for fallback)
