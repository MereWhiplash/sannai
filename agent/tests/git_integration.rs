use chrono::Utc;
use sannai_agent::git;
use sannai_agent::git::observer::{GitObserver, GitObserverCommand};
use sannai_agent::store::{self, Store};
use std::process::Command;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

fn init_test_repo() -> tempfile::TempDir {
    let dir = tempfile::TempDir::new().unwrap();
    Command::new("git")
        .args(["init"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    std::fs::write(dir.path().join("README.md"), "# test").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(dir.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "initial"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    dir
}

/// Full end-to-end test of the git provenance pipeline:
/// 1. Create temp git repo with initial commit
/// 2. Start store, git observer
/// 3. Simulate session events (user prompt → tool_use Write → tool_result)
/// 4. Actually write the file in the repo and commit
/// 5. Poll observer
/// 6. Verify: session exists, commit_link created, git_events recorded, attributions stored
#[tokio::test]
async fn test_full_git_provenance_pipeline() {
    // 1. Create temp git repo
    let repo_dir = init_test_repo();
    let db_dir = tempfile::TempDir::new().unwrap();
    let store = Arc::new(Mutex::new(
        Store::open(&db_dir.path().join("test.db")).unwrap(),
    ));

    // Leak db_dir so it doesn't get cleaned up
    let _ = Box::leak(Box::new(db_dir));

    // 2. Create session
    let session_id = "e2e-test-session";
    let now = Utc::now();
    store
        .lock()
        .await
        .upsert_session(&store::Session {
            id: session_id.to_string(),
            tool: "claude_code".to_string(),
            project_path: Some(repo_dir.path().to_string_lossy().to_string()),
            started_at: now,
            ended_at: None,
            synced_at: None,
            metadata: None,
        })
        .unwrap();

    // 3. Set up GitObserver
    let (cmd_tx, cmd_rx) = mpsc::channel::<GitObserverCommand>(10);
    let mut observer = GitObserver::new(store.clone(), cmd_rx);

    cmd_tx
        .send(GitObserverCommand::TrackRepo {
            repo_path: repo_dir.path().to_path_buf(),
            session_id: session_id.to_string(),
        })
        .await
        .unwrap();
    observer.process_commands().await;

    // Small delay so event timestamps are after the observer's last_poll_at
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // 4. Simulate session events
    let event_time = Utc::now();

    // User prompt
    store
        .lock()
        .await
        .insert_event(&store::Event {
            id: None,
            session_id: session_id.to_string(),
            event_type: "user_prompt".to_string(),
            content: Some("Create a fibonacci function".to_string()),
            context_files: None,
            timestamp: event_time,
            metadata: None,
        })
        .unwrap();

    // Assistant response
    store
        .lock()
        .await
        .insert_event(&store::Event {
            id: None,
            session_id: session_id.to_string(),
            event_type: "assistant_response".to_string(),
            content: Some("I'll create a fibonacci function for you.".to_string()),
            context_files: None,
            timestamp: event_time + chrono::Duration::seconds(1),
            metadata: Some(serde_json::json!({
                "model": "claude-opus",
                "input_tokens": 50,
                "output_tokens": 100,
            })),
        })
        .unwrap();

    // Tool use: Write file
    let fib_content = "pub fn fibonacci(n: u32) -> u64 {\n    match n {\n        0 => 0,\n        1 => 1,\n        _ => fibonacci(n - 1) + fibonacci(n - 2),\n    }\n}\n";
    let file_path = repo_dir.path().join("src/fib.rs");
    store
        .lock()
        .await
        .insert_event(&store::Event {
            id: None,
            session_id: session_id.to_string(),
            event_type: "tool_use".to_string(),
            content: Some("Write".to_string()),
            context_files: None,
            timestamp: event_time + chrono::Duration::seconds(2),
            metadata: Some(serde_json::json!({
                "tool_id": "toolu_fib",
                "input": {
                    "file_path": file_path.to_string_lossy().to_string(),
                    "content": fib_content,
                }
            })),
        })
        .unwrap();

    // Tool result
    store
        .lock()
        .await
        .insert_event(&store::Event {
            id: None,
            session_id: session_id.to_string(),
            event_type: "tool_result".to_string(),
            content: Some("File written successfully".to_string()),
            context_files: None,
            timestamp: event_time + chrono::Duration::seconds(3),
            metadata: Some(serde_json::json!({
                "tool_use_id": "toolu_fib",
                "is_error": false,
            })),
        })
        .unwrap();

    // 5. Actually write the file and commit
    std::fs::create_dir_all(repo_dir.path().join("src")).unwrap();
    std::fs::write(&file_path, fib_content).unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(repo_dir.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "feat: add fibonacci function"])
        .current_dir(repo_dir.path())
        .output()
        .unwrap();

    // 6. Poll observer
    observer.poll_repos().await;

    // 7. Verify the full provenance chain
    let head_sha = git::read_repo_state(repo_dir.path()).unwrap().head_sha;
    let s = store.lock().await;

    // Session exists
    let session = s.get_session(session_id).unwrap().unwrap();
    assert_eq!(session.tool, "claude_code");

    // Events recorded
    let events = s.get_events_for_session(session_id).unwrap();
    assert_eq!(events.len(), 4); // prompt, response, tool_use, tool_result

    // Git event recorded
    let git_events = s.get_git_events_for_session(session_id).unwrap();
    assert!(!git_events.is_empty(), "Expected git events");
    assert_eq!(git_events.last().unwrap().event_type, "head_changed");
    assert_eq!(git_events.last().unwrap().data["cause"], "commit");

    // Commit link created with enriched details
    let links = s.get_commit_links_for_session(session_id).unwrap();
    assert!(!links.is_empty(), "Expected commit links");
    let link = &links[links.len() - 1];
    assert_eq!(link.commit_sha, head_sha);
    assert_eq!(
        link.message.as_deref(),
        Some("feat: add fibonacci function")
    );
    assert!(link.files_changed.is_some());
    assert_eq!(link.detection_method.as_deref(), Some("poll"));

    // Commit linked to session via get_sessions_for_commit
    let linked_sessions = s.get_sessions_for_commit(&head_sha).unwrap();
    assert_eq!(linked_sessions.len(), 1);
    assert_eq!(linked_sessions[0].id, session_id);

    // Attributions stored
    let attrs = s.get_attributions_for_commit(&head_sha).unwrap();
    assert!(!attrs.is_empty(), "Expected attributions for commit");
    assert_eq!(attrs[0].file_path, "src/fib.rs");
    assert_eq!(attrs[0].session_id, session_id);

    // Debug: print attribution details
    for attr in &attrs {
        println!(
            "  Attribution: {} hunk {}..{} confidence={:.3} type={} method={}",
            attr.file_path, attr.hunk_start, attr.hunk_end, attr.confidence,
            attr.attribution_type, attr.method
        );
    }

    println!("=== Full Git Provenance Pipeline Test Passed ===");
    println!("  Session: {}", session_id);
    println!("  Events: {}", events.len());
    println!("  Git events: {}", git_events.len());
    println!("  Commit links: {}", links.len());
    println!("  Commit SHA: {}", &head_sha[..8]);
    println!("  Attributions: {}", attrs.len());
    println!(
        "  Attribution: {} -> {} (confidence: {:.2})",
        attrs[0].file_path, attrs[0].attribution_type, attrs[0].confidence
    );
}
