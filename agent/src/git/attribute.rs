use std::path::Path;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use tokio::sync::Mutex;

use crate::provenance::{attribution, interaction};
use crate::store::{self, Store};

use super::get_commit_details;

/// Attribute a commit's changes to AI interactions from a session within a time window.
/// Returns the number of attributions stored.
pub async fn attribute_commit(
    store: &Arc<Mutex<Store>>,
    repo_path: &Path,
    commit_sha: &str,
    session_id: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> anyhow::Result<usize> {
    // 1. Get events in the time window
    let events = store.lock().await.get_events_in_time_range(session_id, from, to)?;
    if events.is_empty() {
        return Ok(0);
    }

    // 2. Build interactions from those events
    let interactions = interaction::build_interactions(session_id, &events);
    if interactions.is_empty() {
        return Ok(0);
    }

    // 3. Use existing attribute_diff to match hunks to interactions
    let repo_path_str = repo_path.to_string_lossy();
    let diff_attrs = attribution::attribute_diff(&repo_path_str, commit_sha, &interactions);

    // 4. Get commit details for file list
    let details = get_commit_details(repo_path, commit_sha).ok();
    let now = Utc::now();

    // 5. Store each attribution
    let mut count = 0;
    let s = store.lock().await;
    for da in &diff_attrs {
        let attr = store::Attribution {
            id: None,
            commit_sha: commit_sha.to_string(),
            session_id: session_id.to_string(),
            file_path: da.file_path.clone(),
            hunk_start: da.hunk_start as i32,
            hunk_end: da.hunk_end as i32,
            event_id: None,
            confidence: da.confidence,
            attribution_type: format!("{}", da.attribution_type),
            method: if details.is_some() {
                "realtime_content_match".to_string()
            } else {
                "content_match".to_string()
            },
            created_at: now,
        };
        if let Err(e) = s.insert_attribution(&attr) {
            tracing::warn!("Failed to insert attribution: {}", e);
        } else {
            count += 1;
        }
    }

    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::read_repo_state;
    use std::process::Command;

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

    #[tokio::test]
    async fn test_attribute_commit_to_session() {
        let dir = init_test_repo();
        let db_dir = tempfile::TempDir::new().unwrap();
        let store = Arc::new(Mutex::new(
            Store::open(&db_dir.path().join("test.db")).unwrap(),
        ));

        let now = Utc::now();
        store
            .lock()
            .await
            .upsert_session(&store::Session {
                id: "attr-sess".to_string(),
                tool: "claude_code".to_string(),
                project_path: Some(dir.path().to_string_lossy().to_string()),
                started_at: now,
                ended_at: None,
                synced_at: None,
                metadata: None,
            })
            .unwrap();

        // Simulate a user prompt
        store
            .lock()
            .await
            .insert_event(&store::Event {
                id: None,
                session_id: "attr-sess".to_string(),
                event_type: "user_prompt".to_string(),
                content: Some("Write an auth module".to_string()),
                context_files: None,
                timestamp: now + chrono::Duration::seconds(1),
                metadata: None,
            })
            .unwrap();

        // Simulate a Write tool call event
        let file_path = dir.path().join("src/auth.rs");
        store
            .lock()
            .await
            .insert_event(&store::Event {
                id: None,
                session_id: "attr-sess".to_string(),
                event_type: "tool_use".to_string(),
                content: Some("Write".to_string()),
                context_files: None,
                timestamp: now + chrono::Duration::seconds(5),
                metadata: Some(serde_json::json!({
                    "tool_id": "toolu_01",
                    "input": {
                        "file_path": file_path.to_string_lossy().to_string(),
                        "content": "fn authenticate() {\n    validate_token();\n}\n"
                    }
                })),
            })
            .unwrap();

        // Actually write the file and commit
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            &file_path,
            "fn authenticate() {\n    validate_token();\n}\n",
        )
        .unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "add auth"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        let head = read_repo_state(dir.path()).unwrap().head_sha;

        // Run attribution
        let count = attribute_commit(
            &store,
            dir.path(),
            &head,
            "attr-sess",
            now,
            now + chrono::Duration::minutes(1),
        )
        .await
        .unwrap();

        assert!(count > 0);

        let attrs = store
            .lock()
            .await
            .get_attributions_for_commit(&head)
            .unwrap();
        assert!(!attrs.is_empty());
        assert_eq!(attrs[0].file_path, "src/auth.rs");
        assert!(attrs[0].confidence > 0.5);
        assert_eq!(attrs[0].attribution_type, "AI-generated");
    }
}
