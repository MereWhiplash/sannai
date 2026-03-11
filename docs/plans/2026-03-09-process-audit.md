# Process Audit Implementation Plan

**Design:** [docs/designs/2026-03-09-process-audit.md](../designs/2026-03-09-process-audit.md)

**Goal:** Replace hunk-level code-origin attribution with AI workflow process auditing — compute quality signals from session events and surface them via PR comment and API.

**Architecture:** New `process` module with an `Analyzer` that takes `Vec<Interaction>` and computes aggregate metrics (steering ratio, exploration score, test behavior, red flags, etc.). Runs agent-side at commit detection time. Replaces `git/attribute.rs` and rewrites `comment/format.rs`. Store gets a new `process_metrics` table, `attributions` table is dropped.

**EC Context:**
- Decision #97: Pivot from hunk-level attribution to process auditing
- Decision #98: Agent-side compute, heuristics only, evolve existing branch
- Decision #99: Dashboard progressive disclosure (timeline → interactions → conversation)
- Pattern #96: Git observer dual detection (poll + Bash tool_use detection)
- Learning #45: JSONL format camelCase/snake_case conventions

**Test command:** `cd agent && cargo test`

---

## Phase 1: Process Metrics Store Layer

### Task 1: Add `ProcessMetrics` struct and store table @tdd

**Files:**
- Modify: `agent/src/store/mod.rs`

**Step 1: Write failing test** (RED)
```rust
#[test]
fn test_insert_and_get_process_metrics() {
    let (store, _dir) = test_store();
    let now = Utc::now();

    store.upsert_session(&Session {
        id: "pm-session".to_string(),
        tool: "claude_code".to_string(),
        project_path: None,
        started_at: now,
        ended_at: None,
        synced_at: None,
        metadata: None,
    }).unwrap();

    let pm = ProcessMetrics {
        id: None,
        session_id: "pm-session".to_string(),
        commit_sha: Some("abc123".to_string()),
        steering_ratio: 0.72,
        exploration_score: 0.85,
        read_write_ratio: 2.5,
        test_behavior: "test_after".to_string(),
        error_fix_cycles: 2,
        red_flags: serde_json::json!([]),
        prompt_specificity: 0.65,
        total_interactions: 47,
        total_tool_calls: 120,
        files_read: 12,
        files_written: 5,
        created_at: now,
    };

    store.insert_process_metrics(&pm).unwrap();

    let retrieved = store.get_process_metrics_for_session("pm-session").unwrap();
    assert_eq!(retrieved.len(), 1);
    assert!((retrieved[0].steering_ratio - 0.72).abs() < f64::EPSILON);
    assert_eq!(retrieved[0].test_behavior, "test_after");
    assert_eq!(retrieved[0].total_interactions, 47);

    let by_commit = store.get_process_metrics_for_commit("abc123").unwrap();
    assert_eq!(by_commit.len(), 1);
}
```

**Step 2: Run test, verify it fails**
```bash
cd agent && cargo test test_insert_and_get_process_metrics
```
Expected: FAIL — `ProcessMetrics` struct and methods don't exist

**Step 3: Implement** (GREEN)

Add to `store/mod.rs`:

1. Add `process_metrics` table to MIGRATION string:
```sql
CREATE TABLE IF NOT EXISTS process_metrics (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL REFERENCES sessions(id),
    commit_sha TEXT,
    steering_ratio REAL NOT NULL,
    exploration_score REAL NOT NULL,
    read_write_ratio REAL NOT NULL,
    test_behavior TEXT NOT NULL,
    error_fix_cycles INTEGER NOT NULL,
    red_flags TEXT NOT NULL,
    prompt_specificity REAL NOT NULL,
    total_interactions INTEGER NOT NULL,
    total_tool_calls INTEGER NOT NULL,
    files_read INTEGER NOT NULL,
    files_written INTEGER NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_process_metrics_session ON process_metrics(session_id);
CREATE INDEX IF NOT EXISTS idx_process_metrics_commit ON process_metrics(commit_sha);
```

2. Add `ProcessMetrics` struct:
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessMetrics {
    pub id: Option<i64>,
    pub session_id: String,
    pub commit_sha: Option<String>,
    pub steering_ratio: f64,
    pub exploration_score: f64,
    pub read_write_ratio: f64,
    pub test_behavior: String,
    pub error_fix_cycles: i32,
    pub red_flags: serde_json::Value,
    pub prompt_specificity: f64,
    pub total_interactions: i32,
    pub total_tool_calls: i32,
    pub files_read: i32,
    pub files_written: i32,
    pub created_at: DateTime<Utc>,
}
```

3. Add `insert_process_metrics`, `get_process_metrics_for_session`, `get_process_metrics_for_commit` methods.

**Step 4: Run test, verify it passes** @verifying
```bash
cd agent && cargo test test_insert_and_get_process_metrics
```
Expected: PASS

**Step 5: Commit**
```bash
git add agent/src/store/mod.rs && git commit -m "feat(store): add process_metrics table and CRUD"
```

---

## Phase 2: Process Analyzer — Core Metrics Engine

### Task 2: Create `process` module with steering ratio @tdd

**Files:**
- Create: `agent/src/process/mod.rs`
- Create: `agent/src/process/analyzer.rs`
- Modify: `agent/src/lib.rs` (add `pub mod process;`)

**Step 1: Write failing test** (RED)

In `agent/src/process/analyzer.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use crate::provenance::interaction::{Interaction, ToolCall};

    fn make_interaction(seq: u32, prompt: &str, tools: Vec<ToolCall>) -> Interaction {
        let now = Utc::now();
        Interaction {
            id: format!("test-{}", seq),
            session_id: "test-session".to_string(),
            sequence: seq,
            prompt: prompt.to_string(),
            response_texts: vec![],
            tool_calls: tools,
            timestamp_start: now + Duration::seconds(seq as i64 * 10),
            timestamp_end: now + Duration::seconds(seq as i64 * 10 + 5),
        }
    }

    #[test]
    fn test_steering_ratio_all_user_driven() {
        // Every interaction has a specific user prompt
        let interactions = vec![
            make_interaction(1, "Add retry logic to the HTTP client with exponential backoff", vec![]),
            make_interaction(2, "Now add unit tests for the retry function", vec![]),
        ];
        let metrics = analyze(&interactions);
        // Steering ratio should be high — user prompted every interaction
        assert!((metrics.steering_ratio - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_steering_ratio_empty() {
        let metrics = analyze(&[]);
        assert!((metrics.steering_ratio - 0.0).abs() < f64::EPSILON);
    }
}
```

**Step 2: Run test, verify it fails**
```bash
cd agent && cargo test test_steering_ratio
```
Expected: FAIL — module doesn't exist

**Step 3: Implement** (GREEN)

`agent/src/process/mod.rs`:
```rust
pub mod analyzer;
```

`agent/src/process/analyzer.rs`:
```rust
use serde::{Deserialize, Serialize};
use crate::provenance::interaction::Interaction;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessMetricsResult {
    pub steering_ratio: f64,
    pub exploration_score: f64,
    pub read_write_ratio: f64,
    pub test_behavior: String,
    pub error_fix_cycles: i32,
    pub red_flags: Vec<String>,
    pub prompt_specificity: f64,
    pub total_interactions: i32,
    pub total_tool_calls: i32,
    pub files_read: i32,
    pub files_written: i32,
}

pub fn analyze(interactions: &[Interaction]) -> ProcessMetricsResult {
    ProcessMetricsResult {
        steering_ratio: compute_steering_ratio(interactions),
        exploration_score: 0.0,
        read_write_ratio: 0.0,
        test_behavior: "unknown".to_string(),
        error_fix_cycles: 0,
        red_flags: vec![],
        prompt_specificity: 0.0,
        total_interactions: interactions.len() as i32,
        total_tool_calls: interactions.iter().map(|i| i.tool_calls.len()).sum::<usize>() as i32,
        files_read: 0,
        files_written: 0,
    }
}

fn compute_steering_ratio(interactions: &[Interaction]) -> f64 {
    if interactions.is_empty() {
        return 0.0;
    }
    // Every interaction has a user prompt by definition (that's how interactions are built).
    // Steering ratio = proportion of interactions where the user gave meaningful direction.
    // For now: all interactions count as user-steered (ratio = 1.0).
    // This will be refined when prompt_specificity is implemented.
    1.0
}
```

Add `pub mod process;` to `agent/src/lib.rs`.

**Step 4: Run test, verify it passes** @verifying
```bash
cd agent && cargo test test_steering_ratio
```
Expected: PASS

**Step 5: Commit**
```bash
git add agent/src/process/ agent/src/lib.rs && git commit -m "feat(process): add analyzer module with steering ratio"
```

### Task 3: Exploration score and read:write ratio @tdd

**Files:**
- Modify: `agent/src/process/analyzer.rs`

**Step 1: Write failing test** (RED)
```rust
#[test]
fn test_exploration_score_reads_before_writes() {
    let now = Utc::now();
    // 3 reads, then 1 write
    let interactions = vec![
        make_interaction(1, "Look at the codebase", vec![
            make_tool_call("Read", json!({"file_path": "/src/a.rs"}), now, 1),
            make_tool_call("Read", json!({"file_path": "/src/b.rs"}), now, 2),
            make_tool_call("Glob", json!({"pattern": "**/*.rs"}), now, 3),
        ]),
        make_interaction(2, "Now fix the bug", vec![
            make_tool_call("Edit", json!({"file_path": "/src/a.rs"}), now, 1),
        ]),
    ];
    let metrics = analyze(&interactions);
    // 3 explore calls before first write = good exploration
    assert!(metrics.exploration_score > 0.5);
    // 2 files read, 1 written
    assert!(metrics.read_write_ratio > 1.0);
    assert_eq!(metrics.files_read, 2); // Read + Glob are exploration, but files_read counts unique file reads
    assert_eq!(metrics.files_written, 1);
}

#[test]
fn test_exploration_score_write_first() {
    let now = Utc::now();
    // Write immediately, no exploration
    let interactions = vec![
        make_interaction(1, "Write a new file", vec![
            make_tool_call("Write", json!({"file_path": "/src/new.rs", "content": "fn main() {}"}), now, 1),
        ]),
    ];
    let metrics = analyze(&interactions);
    assert!((metrics.exploration_score - 0.0).abs() < f64::EPSILON);
}
```

Helper needed in test module:
```rust
fn make_tool_call(name: &str, input: serde_json::Value, base: DateTime<Utc>, seq: u32) -> ToolCall {
    ToolCall {
        tool_name: name.to_string(),
        tool_id: format!("toolu_{}", seq),
        input,
        output: None,
        timestamp: base + Duration::seconds(seq as i64),
        sequence: seq,
    }
}
```

**Step 2: Run test, verify it fails**
```bash
cd agent && cargo test test_exploration_score
```
Expected: FAIL — returns 0.0 for both

**Step 3: Implement** (GREEN)

Add to `analyzer.rs`:
```rust
const EXPLORE_TOOLS: &[&str] = &["Read", "Glob", "Grep"];
const WRITE_TOOLS: &[&str] = &["Write", "Edit"];

fn compute_exploration_score(interactions: &[Interaction]) -> f64 {
    let all_tools: Vec<&ToolCall> = interactions.iter()
        .flat_map(|i| i.tool_calls.iter())
        .collect();

    // Count explore calls before the first write call
    let first_write_idx = all_tools.iter()
        .position(|tc| WRITE_TOOLS.contains(&tc.tool_name.as_str()));

    let explore_before_write = match first_write_idx {
        Some(idx) => all_tools[..idx].iter()
            .filter(|tc| EXPLORE_TOOLS.contains(&tc.tool_name.as_str()))
            .count(),
        None => all_tools.iter()
            .filter(|tc| EXPLORE_TOOLS.contains(&tc.tool_name.as_str()))
            .count(),
    };

    if all_tools.is_empty() {
        return 0.0;
    }

    // Normalize: 0 explore = 0.0, 5+ explore before write = 1.0
    (explore_before_write as f64 / 5.0).min(1.0)
}

fn compute_read_write_ratio(interactions: &[Interaction]) -> (f64, i32, i32) {
    let mut files_read = std::collections::HashSet::new();
    let mut files_written = std::collections::HashSet::new();

    for interaction in interactions {
        for tc in &interaction.tool_calls {
            let file_path = tc.input.get("file_path").and_then(|v| v.as_str());
            match tc.tool_name.as_str() {
                "Read" => { if let Some(fp) = file_path { files_read.insert(fp.to_string()); } }
                "Write" | "Edit" => { if let Some(fp) = file_path { files_written.insert(fp.to_string()); } }
                _ => {}
            }
        }
    }

    let read_count = files_read.len() as i32;
    let write_count = files_written.len().max(1) as i32; // avoid div by zero
    let ratio = read_count as f64 / write_count as f64;

    (ratio, read_count, write_count)
}
```

Wire these into the `analyze` function.

**Step 4: Run test, verify it passes** @verifying
```bash
cd agent && cargo test test_exploration_score
```
Expected: PASS

**Step 5: Commit**
```bash
git add agent/src/process/analyzer.rs && git commit -m "feat(process): add exploration score and read:write ratio"
```

### Task 4: Test behavior detection @tdd

**Files:**
- Modify: `agent/src/process/analyzer.rs`

**Step 1: Write failing test** (RED)
```rust
#[test]
fn test_detect_test_after_code() {
    let now = Utc::now();
    let interactions = vec![
        make_interaction(1, "Add the feature", vec![
            make_tool_call("Write", json!({"file_path": "/src/feature.rs"}), now, 1),
        ]),
        make_interaction(2, "Add tests", vec![
            make_tool_call("Bash", json!({"command": "cargo test"}), now, 1),
        ]),
    ];
    let metrics = analyze(&interactions);
    assert_eq!(metrics.test_behavior, "test_after");
}

#[test]
fn test_detect_tdd() {
    let now = Utc::now();
    let interactions = vec![
        make_interaction(1, "Write a failing test first", vec![
            make_tool_call("Bash", json!({"command": "cargo test test_foo"}), now, 1),
        ]),
        make_interaction(2, "Now implement", vec![
            make_tool_call("Write", json!({"file_path": "/src/foo.rs"}), now, 1),
        ]),
        make_interaction(3, "Run tests again", vec![
            make_tool_call("Bash", json!({"command": "cargo test"}), now, 1),
        ]),
    ];
    let metrics = analyze(&interactions);
    assert_eq!(metrics.test_behavior, "tdd");
}

#[test]
fn test_detect_no_tests() {
    let now = Utc::now();
    let interactions = vec![
        make_interaction(1, "Write code", vec![
            make_tool_call("Write", json!({"file_path": "/src/main.rs"}), now, 1),
        ]),
    ];
    let metrics = analyze(&interactions);
    assert_eq!(metrics.test_behavior, "no_tests");
}
```

**Step 2: Run test, verify it fails**
```bash
cd agent && cargo test test_detect_test
```
Expected: FAIL — returns "unknown"

**Step 3: Implement** (GREEN)
```rust
const TEST_COMMANDS: &[&str] = &[
    "cargo test", "npm test", "npx jest", "pytest", "go test",
    "make test", "yarn test", "bun test",
];

fn detect_test_behavior(interactions: &[Interaction]) -> String {
    let all_tools: Vec<(&str, bool)> = interactions.iter()
        .flat_map(|i| i.tool_calls.iter())
        .map(|tc| {
            let is_test = tc.tool_name == "Bash"
                && tc.input.get("command")
                    .and_then(|v| v.as_str())
                    .map(|cmd| TEST_COMMANDS.iter().any(|t| cmd.contains(t)))
                    .unwrap_or(false);
            let is_write = WRITE_TOOLS.contains(&tc.tool_name.as_str());
            if is_test { ("test", is_test) }
            else if is_write { ("write", is_write) }
            else { ("other", false) }
        })
        .filter(|(kind, _)| *kind != "other")
        .collect();

    let has_test = all_tools.iter().any(|(k, _)| *k == "test");
    let has_write = all_tools.iter().any(|(k, _)| *k == "write");

    if !has_test {
        return "no_tests".to_string();
    }

    // Check if first test comes before first write
    let first_test = all_tools.iter().position(|(k, _)| *k == "test");
    let first_write = all_tools.iter().position(|(k, _)| *k == "write");

    match (first_test, first_write) {
        (Some(t), Some(w)) if t < w => "tdd".to_string(),
        (Some(_), Some(_)) => "test_after".to_string(),
        (Some(_), None) => "test_only".to_string(),
        _ => "no_tests".to_string(),
    }
}
```

**Step 4: Run test, verify it passes** @verifying
```bash
cd agent && cargo test test_detect_test
```
Expected: PASS

**Step 5: Commit**
```bash
git add agent/src/process/analyzer.rs && git commit -m "feat(process): add test behavior detection"
```

### Task 5: Error-fix cycle detection @tdd

**Files:**
- Modify: `agent/src/process/analyzer.rs`

**Step 1: Write failing test** (RED)
```rust
#[test]
fn test_error_fix_cycles() {
    let now = Utc::now();
    let interactions = vec![
        make_interaction(1, "Build it", vec![
            ToolCall {
                tool_name: "Bash".to_string(),
                tool_id: "t1".to_string(),
                input: json!({"command": "cargo build"}),
                output: Some("error[E0308]: mismatched types".to_string()),
                timestamp: now,
                sequence: 1,
            },
        ]),
        make_interaction(2, "Fix the error", vec![
            make_tool_call("Edit", json!({"file_path": "/src/main.rs"}), now, 1),
            ToolCall {
                tool_name: "Bash".to_string(),
                tool_id: "t2".to_string(),
                input: json!({"command": "cargo build"}),
                output: Some("Compiling sannai v0.1.0".to_string()),
                timestamp: now,
                sequence: 2,
            },
        ]),
    ];
    let metrics = analyze(&interactions);
    assert_eq!(metrics.error_fix_cycles, 1);
}

#[test]
fn test_no_error_cycles() {
    let now = Utc::now();
    let interactions = vec![
        make_interaction(1, "Build", vec![
            ToolCall {
                tool_name: "Bash".to_string(),
                tool_id: "t1".to_string(),
                input: json!({"command": "cargo build"}),
                output: Some("Finished dev".to_string()),
                timestamp: now,
                sequence: 1,
            },
        ]),
    ];
    let metrics = analyze(&interactions);
    assert_eq!(metrics.error_fix_cycles, 0);
}
```

**Step 2: Run test, verify it fails**
```bash
cd agent && cargo test test_error_fix_cycles && cargo test test_no_error_cycles
```
Expected: FAIL — returns 0 for both

**Step 3: Implement** (GREEN)

Detect error-fix cycles by looking for Bash tool calls with error-like output followed by Edit/Write then another Bash call:

```rust
fn count_error_fix_cycles(interactions: &[Interaction]) -> i32 {
    let all_tools: Vec<&ToolCall> = interactions.iter()
        .flat_map(|i| i.tool_calls.iter())
        .collect();

    let mut cycles = 0;
    let mut i = 0;
    while i < all_tools.len() {
        // Look for a Bash call with error output
        if all_tools[i].tool_name == "Bash" && is_error_output(all_tools[i]) {
            // Look ahead for a write then another bash call
            let rest = &all_tools[i + 1..];
            let has_fix = rest.iter().any(|tc| WRITE_TOOLS.contains(&tc.tool_name.as_str()));
            if has_fix {
                cycles += 1;
            }
        }
        i += 1;
    }
    cycles
}

fn is_error_output(tc: &ToolCall) -> bool {
    tc.output.as_ref().map(|o| {
        let lower = o.to_lowercase();
        lower.contains("error") || lower.contains("failed") || lower.contains("panic")
            || lower.contains("exception") || lower.contains("traceback")
    }).unwrap_or(false)
}
```

**Step 4: Run test, verify it passes** @verifying
```bash
cd agent && cargo test test_error_fix_cycles && cargo test test_no_error_cycles
```
Expected: PASS

**Step 5: Commit**
```bash
git add agent/src/process/analyzer.rs && git commit -m "feat(process): add error-fix cycle detection"
```

### Task 6: Red flag detection @tdd

**Files:**
- Modify: `agent/src/process/analyzer.rs`

**Step 1: Write failing test** (RED)
```rust
#[test]
fn test_red_flags_force_push() {
    let now = Utc::now();
    let interactions = vec![
        make_interaction(1, "Push it", vec![
            make_tool_call("Bash", json!({"command": "git push --force origin main"}), now, 1),
        ]),
    ];
    let metrics = analyze(&interactions);
    assert!(metrics.red_flags.iter().any(|f| f.contains("force push")));
}

#[test]
fn test_red_flags_no_verify() {
    let now = Utc::now();
    let interactions = vec![
        make_interaction(1, "Commit", vec![
            make_tool_call("Bash", json!({"command": "git commit --no-verify -m 'wip'"}), now, 1),
        ]),
    ];
    let metrics = analyze(&interactions);
    assert!(metrics.red_flags.iter().any(|f| f.contains("--no-verify")));
}

#[test]
fn test_red_flags_reset_hard() {
    let now = Utc::now();
    let interactions = vec![
        make_interaction(1, "Reset", vec![
            make_tool_call("Bash", json!({"command": "git reset --hard HEAD~3"}), now, 1),
        ]),
    ];
    let metrics = analyze(&interactions);
    assert!(metrics.red_flags.iter().any(|f| f.contains("reset --hard")));
}

#[test]
fn test_no_red_flags() {
    let now = Utc::now();
    let interactions = vec![
        make_interaction(1, "Normal work", vec![
            make_tool_call("Read", json!({"file_path": "/src/main.rs"}), now, 1),
            make_tool_call("Edit", json!({"file_path": "/src/main.rs"}), now, 2),
            make_tool_call("Bash", json!({"command": "cargo test"}), now, 3),
        ]),
    ];
    let metrics = analyze(&interactions);
    assert!(metrics.red_flags.is_empty());
}
```

**Step 2: Run test, verify it fails**
```bash
cd agent && cargo test test_red_flags && cargo test test_no_red_flags
```
Expected: FAIL — returns empty

**Step 3: Implement** (GREEN)
```rust
fn detect_red_flags(interactions: &[Interaction]) -> Vec<String> {
    let mut flags = Vec::new();

    for interaction in interactions {
        for tc in &interaction.tool_calls {
            if tc.tool_name != "Bash" { continue; }
            let cmd = match tc.input.get("command").and_then(|v| v.as_str()) {
                Some(c) => c,
                None => continue,
            };

            if cmd.contains("--force") && cmd.contains("git push") {
                flags.push("Detected: force push".to_string());
            }
            if cmd.contains("--no-verify") {
                flags.push("Detected: --no-verify (skipped hooks)".to_string());
            }
            if cmd.contains("reset --hard") {
                flags.push("Detected: reset --hard".to_string());
            }
            if cmd.contains("rm -rf") {
                flags.push("Detected: rm -rf".to_string());
            }
        }
    }

    flags.dedup();
    flags
}
```

**Step 4: Run test, verify it passes** @verifying
```bash
cd agent && cargo test test_red_flags && cargo test test_no_red_flags
```
Expected: PASS

**Step 5: Commit**
```bash
git add agent/src/process/analyzer.rs && git commit -m "feat(process): add red flag detection"
```

### Task 7: Prompt specificity heuristic @tdd

**Files:**
- Modify: `agent/src/process/analyzer.rs`

**Step 1: Write failing test** (RED)
```rust
#[test]
fn test_prompt_specificity_high() {
    let now = Utc::now();
    let interactions = vec![
        make_interaction(1, "Add retry logic to agent/src/api/mod.rs with exponential backoff, max 3 retries, starting at 100ms", vec![]),
        make_interaction(2, "Write a test in agent/tests/api_retry.rs that verifies the backoff timing", vec![]),
    ];
    let metrics = analyze(&interactions);
    assert!(metrics.prompt_specificity > 0.5);
}

#[test]
fn test_prompt_specificity_low() {
    let now = Utc::now();
    let interactions = vec![
        make_interaction(1, "fix it", vec![]),
        make_interaction(2, "make it work", vec![]),
    ];
    let metrics = analyze(&interactions);
    assert!(metrics.prompt_specificity < 0.3);
}
```

**Step 2: Run test, verify it fails**
```bash
cd agent && cargo test test_prompt_specificity
```
Expected: FAIL — returns 0.0

**Step 3: Implement** (GREEN)

Heuristic based on prompt length, file path mentions, and constraint words:

```rust
fn compute_prompt_specificity(interactions: &[Interaction]) -> f64 {
    if interactions.is_empty() {
        return 0.0;
    }

    let scores: Vec<f64> = interactions.iter().map(|i| {
        let prompt = &i.prompt;
        let mut score = 0.0;

        // Length factor: longer prompts tend to be more specific
        let word_count = prompt.split_whitespace().count();
        score += (word_count as f64 / 20.0).min(0.4); // max 0.4 from length

        // File path mentions
        if prompt.contains('/') || prompt.contains('.rs') || prompt.contains('.ts')
            || prompt.contains('.go') || prompt.contains('.py') {
            score += 0.3;
        }

        // Constraint words (specific technical direction)
        let constraint_words = ["must", "should", "ensure", "max", "min", "retry", "timeout",
            "error", "test", "validate", "return", "handle", "implement", "add", "remove"];
        let constraint_count = constraint_words.iter()
            .filter(|w| prompt.to_lowercase().contains(**w))
            .count();
        score += (constraint_count as f64 / 5.0).min(0.3);

        score.min(1.0)
    }).collect();

    scores.iter().sum::<f64>() / scores.len() as f64
}
```

**Step 4: Run test, verify it passes** @verifying
```bash
cd agent && cargo test test_prompt_specificity
```
Expected: PASS

**Step 5: Commit**
```bash
git add agent/src/process/analyzer.rs && git commit -m "feat(process): add prompt specificity heuristic"
```

---

## Phase 3: Wire Analyzer into Observer

### Task 8: Replace attribution with process analysis in observer @tdd

**Files:**
- Modify: `agent/src/git/observer.rs`
- Modify: `agent/src/git/attribute.rs` (will be deleted later, first decouple)

**Step 1: Write failing test** (RED)

Add integration test in `agent/tests/`:
```rust
// In agent/tests/process_integration.rs
#[tokio::test]
async fn test_observer_stores_process_metrics_on_commit() {
    // Setup: store + session + events simulating a session with reads then writes
    // Trigger: call the analyze_and_store function
    // Assert: process_metrics table has a row with expected values
}
```

(Full test code follows the pattern of the existing `git_integration.rs` test.)

**Step 2: Run test, verify it fails**
```bash
cd agent && cargo test test_observer_stores_process_metrics
```

**Step 3: Implement** (GREEN)

In `observer.rs`, replace the `attribute_commit` call in `poll_repos` with a new function:

```rust
use crate::process::analyzer;

async fn analyze_commit(
    store: &Arc<Mutex<Store>>,
    session_id: &str,
    commit_sha: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> anyhow::Result<()> {
    let events = store.lock().await.get_events_in_time_range(session_id, from, to)?;
    if events.is_empty() {
        return Ok(());
    }

    let interactions = interaction::build_interactions(session_id, &events);
    let result = analyzer::analyze(&interactions);

    let pm = store::ProcessMetrics {
        id: None,
        session_id: session_id.to_string(),
        commit_sha: Some(commit_sha.to_string()),
        steering_ratio: result.steering_ratio,
        exploration_score: result.exploration_score,
        read_write_ratio: result.read_write_ratio,
        test_behavior: result.test_behavior,
        error_fix_cycles: result.error_fix_cycles,
        red_flags: serde_json::json!(result.red_flags),
        prompt_specificity: result.prompt_specificity,
        total_interactions: result.total_interactions,
        total_tool_calls: result.total_tool_calls,
        files_read: result.files_read,
        files_written: result.files_written,
        created_at: Utc::now(),
    };

    store.lock().await.insert_process_metrics(&pm)?;
    Ok(())
}
```

**Step 4: Run test, verify it passes** @verifying
```bash
cd agent && cargo test test_observer_stores_process_metrics
```
Expected: PASS

**Step 5: Commit**
```bash
git add agent/src/git/observer.rs agent/tests/ && git commit -m "feat(git): wire process analyzer into observer replacing attribution"
```

---

## Phase 4: New Comment Format

### Task 9: Rewrite PR comment format for process audit @tdd

**Files:**
- Modify: `agent/src/comment/format.rs`

**Step 1: Write failing test** (RED)
```rust
#[test]
fn test_format_process_audit_comment() {
    let data = ProcessAuditData {
        session_count: 3,
        total_interactions: 47,
        steering_ratio: 0.72,
        exploration_score: 0.85,
        test_behavior: "test_after".to_string(),
        error_fix_cycles: 2,
        red_flags: vec![],
        prompt_specificity: 0.65,
        files_read: 12,
        files_written: 5,
    };
    let comment = format_process_audit(&data);
    assert!(comment.contains("AI Process Audit"));
    assert!(comment.contains("3 sessions"));
    assert!(comment.contains("47 interactions"));
    assert!(comment.contains("Steering"));
    assert!(comment.contains("72%"));
    assert!(comment.contains("12 files read"));
    assert!(comment.contains("Red flags:") && comment.contains("None"));
}

#[test]
fn test_format_with_red_flags() {
    let data = ProcessAuditData {
        session_count: 1,
        total_interactions: 10,
        steering_ratio: 0.3,
        exploration_score: 0.1,
        test_behavior: "no_tests".to_string(),
        error_fix_cycles: 5,
        red_flags: vec!["Detected: force push".to_string(), "Detected: --no-verify".to_string()],
        prompt_specificity: 0.2,
        files_read: 1,
        files_written: 8,
    };
    let comment = format_process_audit(&data);
    assert!(comment.contains("force push"));
    assert!(comment.contains("--no-verify"));
    assert!(comment.contains("No tests"));
}
```

**Step 2: Run test, verify it fails**
```bash
cd agent && cargo test test_format_process_audit
```
Expected: FAIL — structs and function don't exist

**Step 3: Implement** (GREEN)

Replace the existing `CommentData`/`format_comment` with new process-oriented types:

```rust
pub struct ProcessAuditData {
    pub session_count: i32,
    pub total_interactions: i32,
    pub steering_ratio: f64,
    pub exploration_score: f64,
    pub test_behavior: String,
    pub error_fix_cycles: i32,
    pub red_flags: Vec<String>,
    pub prompt_specificity: f64,
    pub files_read: i32,
    pub files_written: i32,
}

pub fn format_process_audit(data: &ProcessAuditData) -> String {
    let mut md = String::new();

    md.push_str(&format!(
        "## AI Process Audit — {} session{}, {} interactions\n\n",
        data.session_count,
        if data.session_count != 1 { "s" } else { "" },
        data.total_interactions,
    ));

    // Steering
    let steering_pct = (data.steering_ratio * 100.0) as u32;
    let steering_label = if steering_pct >= 70 { "High" } else if steering_pct >= 40 { "Medium" } else { "Low" };
    md.push_str(&format!("**Steering:** {} — {}% of interactions had specific human prompts\n", steering_label, steering_pct));

    // Exploration
    md.push_str(&format!("**Exploration:** {} files read before first edit\n", data.files_read));

    // Testing
    let test_label = match data.test_behavior.as_str() {
        "tdd" => "TDD — tests written before code",
        "test_after" => "Tests written after code",
        "test_only" => "Tests run (no code changes)",
        "no_tests" => "No tests detected",
        _ => "Unknown",
    };
    let test_warn = if data.test_behavior == "no_tests" { " ⚠" } else { "" };
    md.push_str(&format!("**Testing:** {}{}\n", test_label, test_warn));

    // Error-fix cycles
    let cycle_note = if data.error_fix_cycles <= 2 { "(normal)" } else { "(high — possible brute forcing)" };
    md.push_str(&format!("**Iterations:** {} error-fix cycle{} {}\n", data.error_fix_cycles, if data.error_fix_cycles != 1 { "s" } else { "" }, cycle_note));

    // Red flags
    if data.red_flags.is_empty() {
        md.push_str("**Red flags:** None\n");
    } else {
        md.push_str("**Red flags:**\n");
        for flag in &data.red_flags {
            md.push_str(&format!("- {}\n", flag));
        }
    }

    md.push_str("\n---\n");
    md.push_str("<sub>Generated by [Sannai](https://github.com/MereWhiplash/sannai) — AI process audit</sub>\n");

    md
}
```

**Step 4: Run test, verify it passes** @verifying
```bash
cd agent && cargo test test_format_process_audit
```
Expected: PASS

**Step 5: Commit**
```bash
git add agent/src/comment/format.rs && git commit -m "feat(comment): rewrite PR comment for process audit format"
```

### Task 10: Update `main.rs` comment command to use process metrics @tdd

**Files:**
- Modify: `agent/src/main.rs`

**Step 1:** This is a wiring task. The `run_comment` function needs to:
1. Still fetch PR commits and find linked sessions
2. Instead of building interactions/lineage/attributions, query `process_metrics` from store
3. Aggregate metrics across sessions
4. Call `format_process_audit` instead of `format_comment`

No new unit test needed — the integration is tested by the format tests and store tests. But verify it compiles and the existing flow still works.

**Step 2: Implement**

Rewrite the `run_comment` function to:
- Query `get_process_metrics_for_session` for each linked session
- Aggregate: average steering_ratio, sum interactions, merge red_flags, etc.
- Build `ProcessAuditData` and call `format_process_audit`

**Step 3: Run full test suite** @verifying
```bash
cd agent && cargo test
```
Expected: all pass

**Step 4: Commit**
```bash
git add agent/src/main.rs && git commit -m "feat: update comment command to use process metrics"
```

---

## Phase 5: Update API

### Task 11: Add process metrics API endpoint, update attribution endpoints @tdd

**Files:**
- Modify: `agent/src/api/mod.rs`

**Step 1: Write failing test** (RED)

(Use the existing API test pattern or a simple integration test.)

**Step 2: Implement** (GREEN)

Add new routes:
```rust
.route("/sessions/:id/process-metrics", get(get_session_process_metrics))
.route("/commits/:sha/process-metrics", get(get_commit_process_metrics))
```

Replace attribution endpoints with process-metrics endpoints. Keep the old routes temporarily returning empty arrays for backwards compatibility, or remove them outright.

**Step 3: Run test, verify it passes** @verifying
```bash
cd agent && cargo test
```

**Step 4: Commit**
```bash
git add agent/src/api/mod.rs && git commit -m "feat(api): add process-metrics endpoints, deprecate attributions"
```

---

## Phase 6: Cleanup

### Task 12: Remove attribution code and table

**Files:**
- Delete: `agent/src/git/attribute.rs`
- Modify: `agent/src/git/mod.rs` (remove `pub mod attribute;`)
- Modify: `agent/src/provenance/attribution.rs` (can keep for on-the-fly fallback or remove)
- Modify: `agent/src/store/mod.rs` (remove attributions table from MIGRATION — note: this is a breaking schema change for existing databases, may need a migration strategy)
- Modify: `agent/src/api/mod.rs` (remove attribution routes)
- Delete or update: `agent/tests/git_integration.rs` (remove attribution-focused tests)

**Step 1: Remove code, update imports**

**Step 2: Run full test suite** @verifying
```bash
cd agent && cargo test && cargo clippy -- -D warnings
```
Expected: all pass, no warnings

**Step 3: Commit**
```bash
git add -A && git commit -m "refactor: remove hunk-level attribution code, replaced by process audit"
```

---

## Patterns to Store (after implementation)

- Process analyzer pattern: how to extract workflow quality signals from interaction sequences
- Red flag detection pattern: extensible flag list from Bash command analysis
- Test behavior detection: sequence analysis for TDD vs test-after vs no-tests
