//! Daemon e2e test using a real captured JSONL session file.
//!
//! Takes a real Claude Code session, rewrites sessionId and cwd to point at
//! a temp git repo, then feeds it through the running daemon and verifies
//! the full pipeline via the API.

use std::io::Write;
use std::net::TcpListener;
use std::process::{Child, Command};
use std::time::Duration;

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

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
    std::fs::write(dir.join("README.md"), "# test\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "initial"])
        .current_dir(dir)
        .output()
        .unwrap();
}

struct DaemonGuard {
    child: Child,
}

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        if let Some(mut stderr) = self.child.stderr.take() {
            let mut s = String::new();
            let _ = std::io::Read::read_to_string(&mut stderr, &mut s);
            if !s.is_empty() {
                eprintln!(
                    "\n--- Daemon stderr (last 2000 chars) ---\n{}\n--- End ---",
                    &s[s.len().saturating_sub(2000)..]
                );
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
        .expect("Failed to start daemon");
    DaemonGuard { child }
}

fn wait_for_health(port: u16) -> bool {
    let client = reqwest::blocking::Client::new();
    let url = format!("http://127.0.0.1:{}/health", port);
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        if let Ok(r) = client.get(&url).send() {
            if r.status().is_success() {
                return true;
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

fn api_get(port: u16, path: &str) -> reqwest::blocking::Response {
    reqwest::blocking::Client::new()
        .get(format!("http://127.0.0.1:{}{}", port, path))
        .send()
        .unwrap()
}

/// Rewrite a real JSONL file: replace sessionId and cwd fields.
fn rewrite_session(content: &str, new_session_id: &str, new_cwd: &str) -> String {
    let mut out = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<serde_json::Value>(line) {
            Ok(mut obj) => {
                // Replace sessionId
                if obj.get("sessionId").is_some() {
                    obj["sessionId"] = serde_json::json!(new_session_id);
                }
                // Replace cwd
                if obj.get("cwd").is_some() {
                    obj["cwd"] = serde_json::json!(new_cwd);
                }
                out.push(serde_json::to_string(&obj).unwrap());
            }
            Err(_) => {
                // Keep line as-is if it can't be parsed
                out.push(line.to_string());
            }
        }
    }
    out.join("\n") + "\n"
}

#[test]
fn test_daemon_with_real_session() {
    let binary = env!("CARGO_BIN_EXE_sannai");

    // Read the real fixture
    let raw = std::fs::read_to_string("tests/fixtures/real_session.jsonl")
        .expect("fixture file missing");

    // Set up temp dirs
    let data_dir = tempfile::TempDir::new().unwrap();
    let claude_dir = tempfile::TempDir::new().unwrap();
    let repo_dir = tempfile::TempDir::new().unwrap();
    init_git_repo(repo_dir.path());

    // Rewrite session file
    let session_id = "real-e2e-session";
    let repo_path = repo_dir.path().to_string_lossy().to_string();
    let rewritten = rewrite_session(&raw, session_id, &repo_path);

    // Start daemon
    let port = free_port();
    let _daemon = start_daemon(binary, data_dir.path(), claude_dir.path(), port);
    assert!(wait_for_health(port), "Daemon did not start");

    // Write the JSONL file
    let project_dir = claude_dir.path().join("-e2e-real-session");
    std::fs::create_dir_all(&project_dir).unwrap();
    let jsonl_path = project_dir.join(format!("{}.jsonl", session_id));
    let mut f = std::fs::File::create(&jsonl_path).unwrap();
    f.write_all(rewritten.as_bytes()).unwrap();
    f.flush().unwrap();
    drop(f);

    // Wait for session to appear
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    let mut found = false;
    while std::time::Instant::now() < deadline {
        let resp = api_get(port, "/sessions");
        if resp.status().is_success() {
            let body: serde_json::Value = resp.json().unwrap();
            if let Some(arr) = body.as_array() {
                if arr.iter().any(|s| s["id"] == session_id) {
                    found = true;
                    break;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    assert!(found, "Session did not appear in API");

    // Wait a bit more for all events to be processed
    std::thread::sleep(Duration::from_secs(2));

    // Verify session detail
    let resp = api_get(port, &format!("/sessions/{}", session_id));
    assert_eq!(resp.status(), 200, "Session detail should return 200");
    let detail: serde_json::Value = resp.json().unwrap();
    assert_eq!(detail["id"], session_id);
    assert_eq!(detail["tool"], "claude_code");
    let event_count = detail["event_count"].as_u64().unwrap();
    println!("  Session event_count: {}", event_count);

    // Verify events
    let resp = api_get(port, &format!("/sessions/{}/events", session_id));
    assert_eq!(resp.status(), 200);
    let events: Vec<serde_json::Value> = resp.json().unwrap();
    println!("  Events returned by API: {}", events.len());

    // Count by type
    let mut type_counts = std::collections::HashMap::new();
    for e in &events {
        let t = e["event_type"].as_str().unwrap_or("unknown");
        *type_counts.entry(t.to_string()).or_insert(0u32) += 1;
    }

    println!("  Event type breakdown:");
    let mut sorted: Vec<_> = type_counts.iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(a.1));
    for (t, c) in &sorted {
        println!("    {:<25} {}", t, c);
    }

    // The real session had 9 user prompts, 25 assistant texts, 49 tool uses, 49 tool results
    // Some may be lost to "Ignored" (file-history-snapshot, progress, system, queue enqueue/remove)
    assert!(
        events.len() >= 100,
        "Expected at least 100 stored events from 201-line session, got {}",
        events.len()
    );
    assert!(
        type_counts.get("user_prompt").copied().unwrap_or(0) >= 5,
        "Expected at least 5 user prompts"
    );
    assert!(
        type_counts.get("assistant_response").copied().unwrap_or(0) >= 10,
        "Expected at least 10 assistant responses"
    );
    assert!(
        type_counts.get("tool_use").copied().unwrap_or(0) >= 20,
        "Expected at least 20 tool uses"
    );
    assert!(
        type_counts.get("tool_result").copied().unwrap_or(0) >= 20,
        "Expected at least 20 tool results"
    );

    // Make a git commit to test the full provenance chain
    std::fs::write(repo_dir.path().join("output.txt"), "generated content\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(repo_dir.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "feat: real session output"])
        .current_dir(repo_dir.path())
        .output()
        .unwrap();

    // Wait for observer to detect commit
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    let mut commits_found = false;
    while std::time::Instant::now() < deadline {
        let resp = api_get(port, &format!("/sessions/{}/commits", session_id));
        if resp.status() == 200 {
            let commits: Vec<serde_json::Value> = resp.json().unwrap();
            if !commits.is_empty() {
                commits_found = true;
                println!("  Commit detected: {}", commits[0]["commit_sha"]);
                break;
            }
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    assert!(commits_found, "Git observer did not detect the commit");

    // Verify process metrics
    let resp = api_get(
        port,
        &format!("/sessions/{}/process-metrics", session_id),
    );
    let metrics: Vec<serde_json::Value> = resp.json().unwrap();
    println!("  Process metrics: {} entries", metrics.len());
    if !metrics.is_empty() {
        let pm = &metrics[0];
        println!(
            "  Steering: {:.0}%, Exploration: {:.2}, Test: {}",
            pm["steering_ratio"].as_f64().unwrap_or(0.0) * 100.0,
            pm["exploration_score"].as_f64().unwrap_or(0.0),
            pm["test_behavior"],
        );
        println!(
            "  Interactions: {}, Tool calls: {}, Files R/W: {}/{}",
            pm["total_interactions"],
            pm["total_tool_calls"],
            pm["files_read"],
            pm["files_written"],
        );
    }

    println!("\n=== Real Session Daemon E2E Passed ===\n");
}
