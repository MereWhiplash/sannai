use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;

use crate::store::{self, Store};

use super::attribute::attribute_commit;
use super::{get_commit_details, infer_head_change_cause, read_repo_state, HeadChangeCause};

pub enum GitObserverCommand {
    TrackRepo {
        repo_path: PathBuf,
        session_id: String,
    },
    UntrackSession {
        session_id: String,
    },
    ImmediatePoll,
}

struct TrackedRepo {
    repo_path: PathBuf,
    session_ids: Vec<String>,
    last_head_sha: String,
    last_poll_at: DateTime<Utc>,
}

pub struct GitObserver {
    store: Arc<Mutex<Store>>,
    cmd_rx: mpsc::Receiver<GitObserverCommand>,
    tracked: HashMap<PathBuf, TrackedRepo>,
}

impl GitObserver {
    pub fn new(store: Arc<Mutex<Store>>, cmd_rx: mpsc::Receiver<GitObserverCommand>) -> Self {
        Self {
            store,
            cmd_rx,
            tracked: HashMap::new(),
        }
    }

    pub async fn run(&mut self, cancel: CancellationToken) -> anyhow::Result<()> {
        let mut poll_interval = tokio::time::interval(std::time::Duration::from_secs(3));
        tracing::info!("Git observer started");
        loop {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    tracing::info!("Git observer shutting down");
                    break;
                }
                Some(cmd) = self.cmd_rx.recv() => {
                    self.handle_command(cmd).await;
                    self.process_commands().await;
                    self.poll_repos().await;
                }
                _ = poll_interval.tick() => {
                    self.process_commands().await;
                    self.poll_repos().await;
                }
            }
        }
        Ok(())
    }

    async fn handle_command(&mut self, cmd: GitObserverCommand) {
        match cmd {
            GitObserverCommand::TrackRepo { repo_path, session_id } => {
                if let Some(tracked) = self.tracked.get_mut(&repo_path) {
                    if !tracked.session_ids.contains(&session_id) {
                        tracked.session_ids.push(session_id);
                    }
                } else if let Ok(state) = read_repo_state(&repo_path) {
                    tracing::info!("Tracking repo {} for session {}", repo_path.display(), session_id);
                    self.tracked.insert(repo_path.clone(), TrackedRepo {
                        repo_path,
                        session_ids: vec![session_id],
                        last_head_sha: state.head_sha,
                        last_poll_at: Utc::now(),
                    });
                }
            }
            GitObserverCommand::UntrackSession { session_id } => {
                let mut to_remove = Vec::new();
                for (path, tracked) in self.tracked.iter_mut() {
                    tracked.session_ids.retain(|id| id != &session_id);
                    if tracked.session_ids.is_empty() {
                        to_remove.push(path.clone());
                    }
                }
                for path in to_remove {
                    self.tracked.remove(&path);
                }
            }
            GitObserverCommand::ImmediatePoll => {
                // Handled by the poll_repos call after this
            }
        }
    }

    pub async fn process_commands(&mut self) {
        while let Ok(cmd) = self.cmd_rx.try_recv() {
            self.handle_command(cmd).await;
        }
    }

    pub async fn poll_repos(&mut self) {
        let paths: Vec<PathBuf> = self.tracked.keys().cloned().collect();
        for path in paths {
            let tracked = self.tracked.get(&path).unwrap();
            let state = match read_repo_state(&tracked.repo_path) {
                Ok(s) => s,
                Err(e) => {
                    // If repo directory no longer exists, untrack it
                    if !tracked.repo_path.exists() {
                        tracing::warn!(
                            "Repo directory deleted, untracking: {}",
                            tracked.repo_path.display()
                        );
                        self.tracked.remove(&path);
                    } else {
                        tracing::warn!(
                            "Failed to read repo state for {}: {}",
                            tracked.repo_path.display(),
                            e
                        );
                    }
                    continue;
                }
            };

            if state.head_sha == tracked.last_head_sha {
                continue;
            }

            let old_sha = tracked.last_head_sha.clone();
            let new_sha = state.head_sha.clone();
            let repo_path = tracked.repo_path.clone();
            let session_ids = tracked.session_ids.clone();

            let cause = infer_head_change_cause(&repo_path, &old_sha, &new_sha)
                .unwrap_or(HeadChangeCause::Unknown);

            let cause_str = match &cause {
                HeadChangeCause::Commit => "commit",
                HeadChangeCause::Amend => "amend",
                HeadChangeCause::Rebase => "rebase",
                HeadChangeCause::Checkout => "checkout",
                HeadChangeCause::Reset => "reset",
                HeadChangeCause::Merge => "merge",
                HeadChangeCause::CherryPick => "cherry_pick",
                HeadChangeCause::Unknown => "unknown",
            };

            tracing::info!(
                "HEAD changed in {}: {} -> {} (cause: {})",
                repo_path.display(),
                &old_sha[..8.min(old_sha.len())],
                &new_sha[..8.min(new_sha.len())],
                cause_str
            );

            let now = Utc::now();
            let store = self.store.lock().await;

            for session_id in &session_ids {
                // Record git_event
                let git_event = store::GitEvent {
                    id: None,
                    session_id: session_id.clone(),
                    repo_path: repo_path.to_string_lossy().to_string(),
                    event_type: "head_changed".to_string(),
                    timestamp: now,
                    data: serde_json::json!({
                        "old_sha": old_sha,
                        "new_sha": new_sha,
                        "cause": cause_str,
                    }),
                };
                if let Err(e) = store.insert_git_event(&git_event) {
                    tracing::warn!("Failed to insert git event: {}", e);
                }

                // For commit-like causes, create a commit_link with details
                if matches!(
                    cause,
                    HeadChangeCause::Commit
                        | HeadChangeCause::Amend
                        | HeadChangeCause::Merge
                        | HeadChangeCause::CherryPick
                ) {
                    let details = get_commit_details(&repo_path, &new_sha).ok();
                    let link = store::CommitLink {
                        commit_sha: new_sha.clone(),
                        session_id: session_id.clone(),
                        repo_path: repo_path.to_string_lossy().to_string(),
                        linked_at: now,
                        parent_shas: details.as_ref().map(|d| d.parent_shas.clone()),
                        message: details.as_ref().map(|d| d.message.clone()),
                        files_changed: details.as_ref().map(|d| d.files_changed.clone()),
                        diff_stat: details.as_ref().map(|d| {
                            serde_json::json!({
                                "insertions": d.insertions,
                                "deletions": d.deletions,
                            })
                        }),
                        detection_method: Some("poll".to_string()),
                    };
                    if let Err(e) = store.link_commit(&link) {
                        tracing::warn!("Failed to link commit: {}", e);
                    }
                }
            }
            drop(store);

            // Run attribution for commit-like causes
            let last_poll = tracked.last_poll_at;
            if matches!(
                cause,
                HeadChangeCause::Commit
                    | HeadChangeCause::Amend
                    | HeadChangeCause::Merge
                    | HeadChangeCause::CherryPick
            ) {
                for session_id in &session_ids {
                    match attribute_commit(
                        &self.store,
                        &repo_path,
                        &new_sha,
                        session_id,
                        last_poll,
                        now,
                    )
                    .await
                    {
                        Ok(count) if count > 0 => {
                            tracing::info!(
                                "Created {} attribution(s) for commit {}",
                                count,
                                &new_sha[..8.min(new_sha.len())]
                            );
                        }
                        Err(e) => {
                            tracing::warn!("Attribution failed for {}: {}", &new_sha[..8.min(new_sha.len())], e);
                        }
                        _ => {}
                    }
                }
            }

            // Update tracked state
            if let Some(tracked) = self.tracked.get_mut(&path) {
                tracked.last_head_sha = new_sha;
                tracked.last_poll_at = now;
            }
        }
    }
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

    async fn setup_observer(
        repo_dir: &std::path::Path,
    ) -> (
        GitObserver,
        mpsc::Sender<GitObserverCommand>,
        Arc<Mutex<Store>>,
        tempfile::TempDir,
    ) {
        let db_dir = tempfile::TempDir::new().unwrap();
        let store = Arc::new(Mutex::new(
            Store::open(&db_dir.path().join("test.db")).unwrap(),
        ));

        store
            .lock()
            .await
            .upsert_session(&store::Session {
                id: "obs-session".to_string(),
                tool: "claude_code".to_string(),
                project_path: Some(repo_dir.to_string_lossy().to_string()),
                started_at: Utc::now(),
                ended_at: None,
                synced_at: None,
                metadata: None,
            })
            .unwrap();

        let (cmd_tx, cmd_rx) = mpsc::channel(10);
        let observer = GitObserver::new(store.clone(), cmd_rx);
        (observer, cmd_tx, store, db_dir)
    }

    #[tokio::test]
    async fn test_observer_creates_attributions_on_commit() {
        let dir = init_test_repo();
        let (mut observer, cmd_tx, store, _db_dir) = setup_observer(dir.path()).await;

        cmd_tx
            .send(GitObserverCommand::TrackRepo {
                repo_path: dir.path().to_path_buf(),
                session_id: "obs-session".to_string(),
            })
            .await
            .unwrap();
        observer.process_commands().await;

        // Backdate the last_poll_at so events fall within the attribution window
        let repo_path = dir.path().to_path_buf();
        if let Some(tracked) = observer.tracked.get_mut(&repo_path) {
            tracked.last_poll_at = Utc::now() - chrono::Duration::minutes(5);
        }

        let event_time = Utc::now();

        // Insert a user prompt event
        store
            .lock()
            .await
            .insert_event(&store::Event {
                id: None,
                session_id: "obs-session".to_string(),
                event_type: "user_prompt".to_string(),
                content: Some("Write a hello module".to_string()),
                context_files: None,
                timestamp: event_time,
                metadata: None,
            })
            .unwrap();

        // Insert a Write tool_use event
        let file_path = dir.path().join("hello.rs");
        store
            .lock()
            .await
            .insert_event(&store::Event {
                id: None,
                session_id: "obs-session".to_string(),
                event_type: "tool_use".to_string(),
                content: Some("Write".to_string()),
                context_files: None,
                timestamp: event_time + chrono::Duration::seconds(2),
                metadata: Some(serde_json::json!({
                    "tool_id": "toolu_attr",
                    "input": {
                        "file_path": file_path.to_string_lossy().to_string(),
                        "content": "fn hello() {\n    println!(\"hello\");\n}\n"
                    }
                })),
            })
            .unwrap();

        // Actually write the file and commit
        std::fs::write(&file_path, "fn hello() {\n    println!(\"hello\");\n}\n").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "add hello"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        // Poll the observer
        observer.poll_repos().await;

        // Verify git_event and commit_link were created
        let head = crate::git::read_repo_state(dir.path()).unwrap().head_sha;
        let events = store
            .lock()
            .await
            .get_git_events_for_session("obs-session")
            .unwrap();
        assert!(!events.is_empty(), "Expected git events to be recorded");

        let links = store
            .lock()
            .await
            .get_commit_links_for_session("obs-session")
            .unwrap();
        assert!(!links.is_empty(), "Expected commit link to be created");

        // Verify attributions were created
        let attrs = store
            .lock()
            .await
            .get_attributions_for_commit(&head)
            .unwrap();
        assert!(
            !attrs.is_empty(),
            "Expected attributions for commit {}",
            head
        );
        assert_eq!(attrs[0].file_path, "hello.rs");
    }

    #[tokio::test]
    async fn test_observer_handles_deleted_repo() {
        let dir = init_test_repo();
        let repo_path = dir.path().to_path_buf();
        let (mut observer, cmd_tx, _store, _db_dir) = setup_observer(dir.path()).await;

        cmd_tx
            .send(GitObserverCommand::TrackRepo {
                repo_path: repo_path.clone(),
                session_id: "obs-session".to_string(),
            })
            .await
            .unwrap();
        observer.process_commands().await;
        assert!(observer.tracked.contains_key(&repo_path));

        // Delete the repo directory
        drop(dir);

        // Poll should handle the error gracefully and untrack
        observer.poll_repos().await;

        // Should have been untracked
        assert!(!observer.tracked.contains_key(&repo_path));
    }

    #[tokio::test]
    async fn test_observer_multiple_sessions_same_repo() {
        let dir = init_test_repo();
        let (mut observer, cmd_tx, store, _db_dir) = setup_observer(dir.path()).await;

        // Create second session
        store
            .lock()
            .await
            .upsert_session(&store::Session {
                id: "obs-session-2".to_string(),
                tool: "claude_code".to_string(),
                project_path: Some(dir.path().to_string_lossy().to_string()),
                started_at: Utc::now(),
                ended_at: None,
                synced_at: None,
                metadata: None,
            })
            .unwrap();

        // Track same repo for two sessions
        cmd_tx
            .send(GitObserverCommand::TrackRepo {
                repo_path: dir.path().to_path_buf(),
                session_id: "obs-session".to_string(),
            })
            .await
            .unwrap();
        cmd_tx
            .send(GitObserverCommand::TrackRepo {
                repo_path: dir.path().to_path_buf(),
                session_id: "obs-session-2".to_string(),
            })
            .await
            .unwrap();
        observer.process_commands().await;

        // Should have a single tracked repo with two session IDs
        let tracked = observer.tracked.get(dir.path()).unwrap();
        assert_eq!(tracked.session_ids.len(), 2);

        // Make a commit
        std::fs::write(dir.path().join("new.txt"), "hello").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "multi-session commit"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        observer.poll_repos().await;

        // Both sessions should have git events
        let events1 = store
            .lock()
            .await
            .get_git_events_for_session("obs-session")
            .unwrap();
        let events2 = store
            .lock()
            .await
            .get_git_events_for_session("obs-session-2")
            .unwrap();
        assert_eq!(events1.len(), 1);
        assert_eq!(events2.len(), 1);

        // Untrack one session — repo should still be tracked
        cmd_tx
            .send(GitObserverCommand::UntrackSession {
                session_id: "obs-session".to_string(),
            })
            .await
            .unwrap();
        observer.process_commands().await;
        let tracked = observer.tracked.get(dir.path()).unwrap();
        assert_eq!(tracked.session_ids.len(), 1);
        assert_eq!(tracked.session_ids[0], "obs-session-2");
    }

    #[tokio::test]
    async fn test_observer_detects_amend() {
        let dir = init_test_repo();
        let (mut observer, cmd_tx, store, _db_dir) = setup_observer(dir.path()).await;

        cmd_tx
            .send(GitObserverCommand::TrackRepo {
                repo_path: dir.path().to_path_buf(),
                session_id: "obs-session".to_string(),
            })
            .await
            .unwrap();
        observer.process_commands().await;

        // Make a commit, poll
        std::fs::write(dir.path().join("file.txt"), "v1").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "original"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        observer.poll_repos().await;

        // Amend, poll again
        std::fs::write(dir.path().join("file.txt"), "v2").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "--amend", "-m", "amended"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        observer.poll_repos().await;

        let events = store
            .lock()
            .await
            .get_git_events_for_session("obs-session")
            .unwrap();
        assert_eq!(events.len(), 2);
        let amend_event = events.iter().find(|e| e.data["cause"] == "amend");
        assert!(amend_event.is_some());
    }

    #[tokio::test]
    async fn test_observer_detects_reset() {
        let dir = init_test_repo();
        let (mut observer, cmd_tx, store, _db_dir) = setup_observer(dir.path()).await;

        cmd_tx
            .send(GitObserverCommand::TrackRepo {
                repo_path: dir.path().to_path_buf(),
                session_id: "obs-session".to_string(),
            })
            .await
            .unwrap();
        observer.process_commands().await;

        let initial_sha = read_repo_state(dir.path()).unwrap().head_sha;

        // Make a commit, poll
        std::fs::write(dir.path().join("file.txt"), "v1").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "to-be-reset"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        observer.poll_repos().await;

        // Reset back, poll
        Command::new("git")
            .args(["reset", "--hard", &initial_sha])
            .current_dir(dir.path())
            .output()
            .unwrap();
        observer.poll_repos().await;

        let events = store
            .lock()
            .await
            .get_git_events_for_session("obs-session")
            .unwrap();
        assert_eq!(events.len(), 2);
        let reset_event = events.iter().find(|e| e.data["cause"] == "reset");
        assert!(reset_event.is_some());

        // Reset should NOT create a commit_link (it's destructive, not a new commit)
        let links = store
            .lock()
            .await
            .get_commit_links_for_session("obs-session")
            .unwrap();
        // Only the first commit should have a link, not the reset
        assert_eq!(links.len(), 1);
    }

    #[tokio::test]
    async fn test_observer_detects_new_commit() {
        let dir = init_test_repo();
        let (mut observer, cmd_tx, store, _db_dir) = setup_observer(dir.path()).await;

        // Tell observer to track this repo and process the command
        cmd_tx
            .send(GitObserverCommand::TrackRepo {
                repo_path: dir.path().to_path_buf(),
                session_id: "obs-session".to_string(),
            })
            .await
            .unwrap();
        observer.process_commands().await;

        // Make a commit in the test repo AFTER tracking starts
        std::fs::write(dir.path().join("new.txt"), "hello").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "detected commit"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        // Poll to detect the change
        observer.poll_repos().await;

        // Check that a git_event was recorded
        let events = store
            .lock()
            .await
            .get_git_events_for_session("obs-session")
            .unwrap();
        assert!(!events.is_empty());
        assert_eq!(events.last().unwrap().event_type, "head_changed");

        // Check that a commit_link was created
        let head_sha = read_repo_state(dir.path()).unwrap().head_sha;
        let linked = store
            .lock()
            .await
            .get_sessions_for_commit(&head_sha)
            .unwrap();
        assert_eq!(linked.len(), 1);
        assert_eq!(linked[0].id, "obs-session");
    }
}
