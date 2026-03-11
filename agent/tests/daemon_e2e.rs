//! True daemon-level end-to-end test.
//!
//! Starts the `sannai` binary as a child process, writes a fake JSONL session
//! file, makes a git commit, then queries the API to verify the full pipeline:
//!   watcher → parser → session → git observer → process metrics → API

use std::io::Write;
use std::net::TcpListener;
use std::process::{Child, Command};
use std::time::Duration;

/// Find a free TCP port by binding to :0
fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

/// Create a temp git repo with an initial commit.
fn init_git_repo(dir: &std::path::Path) {
    for args in [
        vec!["init"],
        vec!["config", "user.email", "test@e2e.com"],
        vec!["config", "user.name", "E2E Test"],
    ] {
        Command::new("git")
            .args(&args)
            .current_dir(dir)
            .output()
            .unwrap();
    }
    std::fs::write(dir.join("README.md"), "# e2e test repo\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "initial commit"])
        .current_dir(dir)
        .output()
        .unwrap();
}

/// Get the HEAD SHA of a git repo.
fn head_sha(repo_dir: &std::path::Path) -> String {
    let out = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_dir)
        .output()
        .unwrap();
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

/// Build JSONL lines for a fake coding session using real timestamps.
fn build_session_jsonl(session_id: &str, repo_path: &str, file_path: &str) -> String {
    // Use real timestamps so events fall within the observer's poll window
    let now = chrono::Utc::now();
    let ts = |secs: i64| -> String {
        (now + chrono::Duration::seconds(secs))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    };

    let mut lines = Vec::new();

    // 1. queue-operation dequeue (session start)
    lines.push(format!(
        r#"{{"type":"queue-operation","operation":"dequeue","timestamp":"{}","sessionId":"{session_id}"}}"#,
        ts(0),
    ));

    // 2. User prompt
    lines.push(format!(
        r#"{{"parentUuid":null,"isSidechain":false,"userType":"external","cwd":"{repo_path}","sessionId":"{session_id}","version":"2.1.15","gitBranch":"main","type":"user","message":{{"role":"user","content":"Create a hello world function in Rust"}},"uuid":"u-1","timestamp":"{}","permissionMode":"default"}}"#,
        ts(1),
    ));

    // 3. Assistant response with text + tool_use (Write)
    lines.push(format!(
        r#"{{"parentUuid":"u-1","isSidechain":false,"userType":"external","cwd":"{repo_path}","sessionId":"{session_id}","version":"2.1.15","gitBranch":"main","message":{{"model":"claude-opus-4-5-20251101","id":"msg_01","type":"message","role":"assistant","content":[{{"type":"text","text":"I'll create a hello world function for you."}},{{"type":"tool_use","id":"toolu_e2e_01","name":"Write","input":{{"file_path":"{file_path}","content":"pub fn hello() -> &'static str {{\n    \"Hello, world!\"\n}}\n"}}}}],"stop_reason":null,"stop_sequence":null,"usage":{{"input_tokens":150,"output_tokens":80}}}},"requestId":"req_01","type":"assistant","uuid":"a-1","timestamp":"{}"}}"#,
        ts(3),
    ));

    // 4. Tool result (success)
    lines.push(format!(
        r#"{{"parentUuid":"a-1","isSidechain":false,"userType":"external","cwd":"{repo_path}","sessionId":"{session_id}","version":"2.1.15","gitBranch":"main","type":"user","message":{{"role":"user","content":[{{"tool_use_id":"toolu_e2e_01","type":"tool_result","content":"File written successfully","is_error":false}}]}},"uuid":"u-2","timestamp":"{}","toolUseResult":{{"stdout":"File written successfully","stderr":"","interrupted":false,"isImage":false}},"sourceToolAssistantUUID":"a-1"}}"#,
        ts(4),
    ));

    // 5. Assistant reads the file
    lines.push(format!(
        r#"{{"parentUuid":"u-2","isSidechain":false,"userType":"external","cwd":"{repo_path}","sessionId":"{session_id}","version":"2.1.15","gitBranch":"main","message":{{"model":"claude-opus-4-5-20251101","id":"msg_02","type":"message","role":"assistant","content":[{{"type":"text","text":"Let me verify the file."}},{{"type":"tool_use","id":"toolu_e2e_02","name":"Read","input":{{"file_path":"{file_path}"}}}}],"stop_reason":null,"stop_sequence":null,"usage":{{"input_tokens":200,"output_tokens":40}}}},"requestId":"req_02","type":"assistant","uuid":"a-2","timestamp":"{}"}}"#,
        ts(5),
    ));

    // 6. Read result
    lines.push(format!(
        r#"{{"parentUuid":"a-2","isSidechain":false,"userType":"external","cwd":"{repo_path}","sessionId":"{session_id}","version":"2.1.15","gitBranch":"main","type":"user","message":{{"role":"user","content":[{{"tool_use_id":"toolu_e2e_02","type":"tool_result","content":"pub fn hello() -> &'static str {{\n    \"Hello, world!\"\n}}\n","is_error":false}}]}},"uuid":"u-3","timestamp":"{}","toolUseResult":{{"stdout":"","stderr":"","interrupted":false,"isImage":false}},"sourceToolAssistantUUID":"a-2"}}"#,
        ts(6),
    ));

    // 7. Final assistant message
    lines.push(format!(
        r#"{{"parentUuid":"u-3","isSidechain":false,"userType":"external","cwd":"{repo_path}","sessionId":"{session_id}","version":"2.1.15","gitBranch":"main","message":{{"model":"claude-opus-4-5-20251101","id":"msg_03","type":"message","role":"assistant","content":[{{"type":"text","text":"The hello world function is ready."}}],"stop_reason":"end_turn","stop_sequence":null,"usage":{{"input_tokens":250,"output_tokens":30}}}},"requestId":"req_03","type":"assistant","uuid":"a-3","timestamp":"{}"}}"#,
        ts(7),
    ));

    lines.join("\n") + "\n"
}

struct DaemonGuard {
    child: Child,
}

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        // Capture stderr for debugging
        if let Some(mut stderr) = self.child.stderr.take() {
            let mut s = String::new();
            let _ = std::io::Read::read_to_string(&mut stderr, &mut s);
            if !s.is_empty() {
                eprintln!("\n--- Daemon stderr ---\n{}\n--- End daemon stderr ---", s);
            }
        }
        let _ = self.child.wait();
    }
}

fn start_daemon(
    binary: &str,
    data_dir: &std::path::Path,
    claude_dir: &std::path::Path,
    port: u16,
) -> DaemonGuard {
    let child = Command::new(binary)
        .args(["start"])
        .env("SANNAI_DATA_DIR", data_dir.as_os_str())
        .env("SANNAI_CLAUDE_DIR", claude_dir.as_os_str())
        .env("SANNAI_API_PORT", port.to_string())
        .env("RUST_LOG", "sannai_agent=debug")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("Failed to start sannai daemon");

    DaemonGuard { child }
}

fn wait_for_health(port: u16, timeout: Duration) -> bool {
    let client = reqwest::blocking::Client::new();
    let url = format!("http://127.0.0.1:{}/health", port);
    let deadline = std::time::Instant::now() + timeout;

    while std::time::Instant::now() < deadline {
        if let Ok(resp) = client.get(&url).send() {
            if resp.status().is_success() {
                return true;
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

fn api_get(port: u16, path: &str) -> reqwest::blocking::Response {
    let client = reqwest::blocking::Client::new();
    let url = format!("http://127.0.0.1:{}{}", port, path);
    client.get(&url).send().unwrap()
}

#[test]
fn test_daemon_e2e_full_pipeline() {
    // 0. Find the binary
    let binary = env!("CARGO_BIN_EXE_sannai");

    // 1. Set up temp directories
    let data_dir = tempfile::TempDir::new().unwrap();
    let claude_dir = tempfile::TempDir::new().unwrap();
    let repo_dir = tempfile::TempDir::new().unwrap();

    // 2. Init git repo
    init_git_repo(repo_dir.path());
    let initial_sha = head_sha(repo_dir.path());

    // 3. Pick a free port and start daemon
    let port = free_port();
    let _daemon = start_daemon(binary, data_dir.path(), claude_dir.path(), port);

    // 4. Wait for the API to be ready
    assert!(
        wait_for_health(port, Duration::from_secs(10)),
        "Daemon did not become healthy within 10s"
    );

    // 5. Create the JSONL session file
    let session_id = "e2e-daemon-test-session";
    let project_slug = "-e2e-test-project";
    let project_dir = claude_dir.path().join(project_slug);
    std::fs::create_dir_all(&project_dir).unwrap();

    let repo_path_str = repo_dir.path().to_string_lossy().to_string();
    let target_file = repo_dir.path().join("src").join("hello.rs");
    let target_file_str = target_file.to_string_lossy().to_string();

    let jsonl_content = build_session_jsonl(session_id, &repo_path_str, &target_file_str);
    let jsonl_path = project_dir.join(format!("{}.jsonl", session_id));

    // Write the JSONL file (watcher polls every 1 sec)
    let mut f = std::fs::File::create(&jsonl_path).unwrap();
    f.write_all(jsonl_content.as_bytes()).unwrap();
    f.flush().unwrap();
    drop(f);

    // 6. Wait for the session to appear in the API
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    let mut session_found = false;
    while std::time::Instant::now() < deadline {
        let resp = api_get(port, "/sessions");
        if resp.status().is_success() {
            let body: serde_json::Value = resp.json().unwrap();
            if let Some(arr) = body.as_array() {
                if arr.iter().any(|s| s["id"] == session_id) {
                    session_found = true;
                    break;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    assert!(session_found, "Session '{}' did not appear in API", session_id);

    // 7. Verify events were parsed
    let resp = api_get(port, &format!("/sessions/{}/events", session_id));
    assert_eq!(resp.status(), 200);
    let events: Vec<serde_json::Value> = resp.json().unwrap();
    // Expected: user_prompt, assistant_response, tool_use(Write), tool_result,
    //           assistant_response(text), tool_use(Read), tool_result, assistant_response(final)
    assert!(
        events.len() >= 5,
        "Expected at least 5 events, got {}",
        events.len()
    );

    // Check event types are correct
    let event_types: Vec<&str> = events
        .iter()
        .filter_map(|e| e["event_type"].as_str())
        .collect();
    assert!(
        event_types.contains(&"user_prompt"),
        "Missing user_prompt event"
    );
    assert!(
        event_types.contains(&"assistant_response"),
        "Missing assistant_response event"
    );
    assert!(
        event_types.contains(&"tool_use"),
        "Missing tool_use event"
    );
    assert!(
        event_types.contains(&"tool_result"),
        "Missing tool_result event"
    );

    // 8. Verify the session detail endpoint
    let resp = api_get(port, &format!("/sessions/{}", session_id));
    assert_eq!(resp.status(), 200);
    let session_detail: serde_json::Value = resp.json().unwrap();
    assert_eq!(session_detail["id"], session_id);
    assert_eq!(session_detail["tool"], "claude_code");
    assert_eq!(session_detail["project_path"], repo_path_str);
    assert!(
        session_detail["event_count"].as_u64().unwrap() >= 5,
        "Expected event_count >= 5"
    );

    // 8b. Check git-events to see if repo tracking was established
    // The session manager should have sent TrackRepo when it saw the cwd
    let resp = api_get(port, &format!("/sessions/{}/git-events", session_id));
    let pre_git_events: Vec<serde_json::Value> = resp.json().unwrap();
    eprintln!("  Pre-commit git events: {}", pre_git_events.len());

    // 9. Make a git commit in the repo (simulating what happened during the session)
    std::fs::create_dir_all(repo_dir.path().join("src")).unwrap();
    std::fs::write(&target_file, "pub fn hello() -> &'static str {\n    \"Hello, world!\"\n}\n")
        .unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(repo_dir.path())
        .output()
        .unwrap();
    let commit_output = Command::new("git")
        .args(["commit", "-m", "feat: add hello world function"])
        .current_dir(repo_dir.path())
        .output()
        .unwrap();
    assert!(
        commit_output.status.success(),
        "git commit failed: {}",
        String::from_utf8_lossy(&commit_output.stderr)
    );

    let commit_sha = head_sha(repo_dir.path());
    assert_ne!(commit_sha, initial_sha, "HEAD should have changed");

    // 10. Wait for the git observer to detect the commit (polls every 3 sec)
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    let mut commits_found = false;
    while std::time::Instant::now() < deadline {
        let resp = api_get(port, &format!("/sessions/{}/commits", session_id));
        if resp.status() == 200 {
            let commits: Vec<serde_json::Value> = resp.json().unwrap();
            if !commits.is_empty() {
                commits_found = true;
                break;
            }
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    assert!(commits_found, "Git observer did not detect the commit");

    // 11. Verify commit link details
    let resp = api_get(port, &format!("/sessions/{}/commits", session_id));
    let commits: Vec<serde_json::Value> = resp.json().unwrap();
    let commit = &commits[commits.len() - 1];
    assert_eq!(
        commit["commit_sha"].as_str().unwrap(),
        &commit_sha,
        "Commit SHA mismatch"
    );
    assert_eq!(
        commit["message"].as_str().unwrap(),
        "feat: add hello world function"
    );
    assert_eq!(commit["detection_method"].as_str().unwrap(), "poll");

    // 12. Verify git events
    let resp = api_get(port, &format!("/sessions/{}/git-events", session_id));
    assert_eq!(resp.status(), 200);
    let git_events: Vec<serde_json::Value> = resp.json().unwrap();
    assert!(
        !git_events.is_empty(),
        "Expected at least one git event"
    );
    assert_eq!(git_events.last().unwrap()["event_type"], "head_changed");

    // 13. Verify process metrics were generated
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut metrics_found = false;
    while std::time::Instant::now() < deadline {
        let resp = api_get(
            port,
            &format!("/sessions/{}/process-metrics", session_id),
        );
        if resp.status() == 200 {
            let metrics: Vec<serde_json::Value> = resp.json().unwrap();
            if !metrics.is_empty() {
                metrics_found = true;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    assert!(
        metrics_found,
        "Process metrics were not generated for the commit"
    );

    // Verify process metrics content
    let resp = api_get(
        port,
        &format!("/sessions/{}/process-metrics", session_id),
    );
    let metrics: Vec<serde_json::Value> = resp.json().unwrap();
    let pm = &metrics[0];
    assert_eq!(pm["session_id"], session_id);
    assert_eq!(pm["commit_sha"], commit_sha);
    assert!(
        pm["total_interactions"].as_i64().unwrap() >= 1,
        "Expected at least 1 interaction"
    );
    assert!(
        pm["files_written"].as_i64().unwrap() >= 1,
        "Expected at least 1 file written"
    );

    // Also verify the commit-based metrics endpoint
    let resp = api_get(
        port,
        &format!("/commits/{}/process-metrics", commit_sha),
    );
    assert_eq!(resp.status(), 200);
    let commit_metrics: Vec<serde_json::Value> = resp.json().unwrap();
    assert!(
        !commit_metrics.is_empty(),
        "Commit-based process metrics endpoint returned empty"
    );

    // === Print summary ===
    println!("\n=== Daemon E2E Test Passed ===");
    println!("  Port: {}", port);
    println!("  Session: {}", session_id);
    println!("  Events: {}", events.len());
    println!("  Commit: {}", &commit_sha[..8]);
    println!("  Git events: {}", git_events.len());
    println!("  Process metrics: {} entries", metrics.len());
    println!(
        "  Steering: {:.0}%, Exploration: {:.2}",
        pm["steering_ratio"].as_f64().unwrap_or(0.0) * 100.0,
        pm["exploration_score"].as_f64().unwrap_or(0.0),
    );
    println!("  Test behavior: {}", pm["test_behavior"]);
    println!("=================================\n");
}
