use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;

use crate::parser::ParsedEvent;
use crate::store::{self, Store};
use crate::watcher::WatcherEvent;

// ---------------------------------------------------------------------------
// In-memory session state
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct ActiveSession {
    id: String,
    project_path: Option<String>,
    cwd: Option<String>,
    git_branch: Option<String>,
    started_at: DateTime<Utc>,
    last_event_at: DateTime<Utc>,
    prompt_count: u64,
    event_count: u64,
}

// ---------------------------------------------------------------------------
// Session Manager
// ---------------------------------------------------------------------------

pub struct SessionManager {
    store: Arc<Mutex<Store>>,
    active_sessions: HashMap<String, ActiveSession>,
    idle_timeout: Duration,
}

impl SessionManager {
    pub fn new(store: Arc<Mutex<Store>>, idle_timeout_minutes: i64) -> Self {
        Self {
            store,
            active_sessions: HashMap::new(),
            idle_timeout: Duration::minutes(idle_timeout_minutes),
        }
    }

    /// Main run loop. Consumes events from the watcher and periodically checks for idle sessions.
    pub async fn run(
        &mut self,
        mut rx: mpsc::Receiver<WatcherEvent>,
        cancel: CancellationToken,
    ) -> Result<()> {
        tracing::info!("Session manager started");
        let mut poll_interval = tokio::time::interval(std::time::Duration::from_millis(200));
        let mut idle_interval = tokio::time::interval(std::time::Duration::from_secs(60));

        loop {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    tracing::info!("Session manager shutting down");
                    self.end_all_sessions().await;
                    break;
                }
                _ = poll_interval.tick() => {
                    // Drain all available events from the channel.
                    // We poll instead of using rx.recv().await because the watcher
                    // sends events from a blocking thread, which may not wake the
                    // async receiver.
                    loop {
                        match rx.try_recv() {
                            Ok(event) => {
                                if let Err(e) = self.process_event(event).await {
                                    tracing::warn!("Failed to process event: {}", e);
                                }
                            }
                            Err(mpsc::error::TryRecvError::Empty) => break,
                            Err(mpsc::error::TryRecvError::Disconnected) => {
                                tracing::info!("Watcher channel disconnected");
                                self.end_all_sessions().await;
                                return Ok(());
                            }
                        }
                    }
                }
                _ = idle_interval.tick() => {
                    if let Err(e) = self.check_idle_sessions().await {
                        tracing::warn!("Failed to check idle sessions: {}", e);
                    }
                }
            }
        }

        Ok(())
    }

    async fn process_event(&mut self, watcher_event: WatcherEvent) -> Result<()> {
        let WatcherEvent { parsed, project_dir, .. } = watcher_event;

        match &parsed {
            ParsedEvent::SessionStart { session_id, timestamp } => {
                self.ensure_session(session_id, *timestamp, &project_dir, None, None).await?;
            }
            ParsedEvent::UserPrompt { session_id, timestamp, content, cwd, git_branch, .. } => {
                self.ensure_session(
                    session_id,
                    *timestamp,
                    &project_dir,
                    cwd.as_deref(),
                    git_branch.as_deref(),
                )
                .await?;

                self.persist_event(session_id, "user_prompt", Some(content), *timestamp, None)
                    .await?;

                if let Some(active) = self.active_sessions.get_mut(session_id) {
                    active.prompt_count += 1;
                    active.event_count += 1;
                    active.last_event_at = *timestamp;
                }
            }
            ParsedEvent::AssistantText {
                session_id,
                timestamp,
                text,
                model,
                input_tokens,
                output_tokens,
                ..
            } => {
                self.ensure_session(session_id, *timestamp, &project_dir, None, None).await?;

                let metadata = serde_json::json!({
                    "model": model,
                    "input_tokens": input_tokens,
                    "output_tokens": output_tokens,
                });

                self.persist_event(
                    session_id,
                    "assistant_response",
                    Some(text),
                    *timestamp,
                    Some(metadata),
                )
                .await?;

                if let Some(active) = self.active_sessions.get_mut(session_id) {
                    active.event_count += 1;
                    active.last_event_at = *timestamp;
                }
            }
            ParsedEvent::ToolUse { session_id, timestamp, tool_name, tool_id, input, .. } => {
                self.ensure_session(session_id, *timestamp, &project_dir, None, None).await?;

                let metadata = serde_json::json!({
                    "tool_id": tool_id,
                    "input": input,
                });

                self.persist_event(
                    session_id,
                    "tool_use",
                    Some(tool_name),
                    *timestamp,
                    Some(metadata),
                )
                .await?;

                if let Some(active) = self.active_sessions.get_mut(session_id) {
                    active.event_count += 1;
                    active.last_event_at = *timestamp;
                }
            }
            ParsedEvent::ToolResult {
                session_id,
                timestamp,
                tool_use_id,
                is_error,
                content,
                ..
            } => {
                self.ensure_session(session_id, *timestamp, &project_dir, None, None).await?;

                let metadata = serde_json::json!({
                    "tool_use_id": tool_use_id,
                    "is_error": is_error,
                });

                self.persist_event(
                    session_id,
                    "tool_result",
                    content.as_deref(),
                    *timestamp,
                    Some(metadata),
                )
                .await?;

                if let Some(active) = self.active_sessions.get_mut(session_id) {
                    active.event_count += 1;
                    active.last_event_at = *timestamp;
                }
            }
            ParsedEvent::Ignored => {}
        }

        Ok(())
    }

    async fn ensure_session(
        &mut self,
        session_id: &str,
        timestamp: DateTime<Utc>,
        project_dir: &str,
        cwd: Option<&str>,
        git_branch: Option<&str>,
    ) -> Result<()> {
        if self.active_sessions.contains_key(session_id) {
            let needs_store_update;
            if let Some(active) = self.active_sessions.get_mut(session_id) {
                let mut changed = false;
                if let Some(cwd) = cwd {
                    if active.cwd.is_none() {
                        active.cwd = Some(cwd.to_string());
                        active.project_path = Some(cwd.to_string());
                        changed = true;
                    }
                }
                if let Some(branch) = git_branch {
                    if active.git_branch.is_none() {
                        active.git_branch = Some(branch.to_string());
                        changed = true;
                    }
                }
                needs_store_update = changed;
                if changed {
                    let session = store::Session {
                        id: session_id.to_string(),
                        tool: "claude_code".to_string(),
                        project_path: active.project_path.clone(),
                        git_branch: active.git_branch.clone(),
                        started_at: active.started_at,
                        ended_at: None,
                        synced_at: None,
                        metadata: None,
                    };
                    self.store.lock().await.upsert_session(&session)?;
                }
            } else {
                needs_store_update = false;
            }
            let _ = needs_store_update;
            return Ok(());
        }

        // New session
        let project_path =
            cwd.map(|s| s.to_string()).unwrap_or_else(|| project_dir.replace('-', "/"));

        let session = store::Session {
            id: session_id.to_string(),
            tool: "claude_code".to_string(),
            project_path: Some(project_path.clone()),
            git_branch: git_branch.map(|s| s.to_string()),
            started_at: timestamp,
            ended_at: None,
            synced_at: None,
            metadata: None,
        };

        self.store.lock().await.upsert_session(&session)?;

        tracing::info!("New session: {} (project: {})", session_id, project_path);

        self.active_sessions.insert(
            session_id.to_string(),
            ActiveSession {
                id: session_id.to_string(),
                project_path: Some(project_path),
                cwd: cwd.map(|s| s.to_string()),
                git_branch: git_branch.map(|s| s.to_string()),
                started_at: timestamp,
                last_event_at: timestamp,
                prompt_count: 0,
                event_count: 0,
            },
        );

        Ok(())
    }

    async fn persist_event(
        &self,
        session_id: &str,
        event_type: &str,
        content: Option<&str>,
        timestamp: DateTime<Utc>,
        metadata: Option<serde_json::Value>,
    ) -> Result<()> {
        let event = store::Event {
            id: None,
            session_id: session_id.to_string(),
            event_type: event_type.to_string(),
            content: content.map(|s| s.to_string()),
            context_files: None,
            timestamp,
            metadata,
        };
        self.store.lock().await.insert_event(&event)?;
        Ok(())
    }

    async fn check_idle_sessions(&mut self) -> Result<()> {
        let now = Utc::now();
        let mut to_end = Vec::new();

        for (id, active) in &self.active_sessions {
            if now - active.last_event_at > self.idle_timeout {
                to_end.push(id.clone());
            }
        }

        for id in to_end {
            self.end_session(&id).await?;
        }

        Ok(())
    }

    async fn end_session(&mut self, session_id: &str) -> Result<()> {
        if let Some(active) = self.active_sessions.remove(session_id) {
            self.store.lock().await.end_session(session_id, active.last_event_at)?;
            tracing::info!(
                "Session ended: {} ({} events, {} prompts)",
                session_id,
                active.event_count,
                active.prompt_count,
            );
        }
        Ok(())
    }

    async fn end_all_sessions(&mut self) {
        let ids: Vec<String> = self.active_sessions.keys().cloned().collect();
        for id in ids {
            if let Err(e) = self.end_session(&id).await {
                tracing::warn!("Failed to end session {}: {}", id, e);
            }
        }
    }

    /// Returns session IDs of active sessions whose project path matches the given repo path.
    pub fn active_sessions_for_repo(&self, repo_path: &str) -> Vec<String> {
        self.active_sessions
            .values()
            .filter(|s| {
                s.project_path.as_deref().map(|p| p == repo_path).unwrap_or(false)
                    || s.cwd.as_deref().map(|c| c == repo_path).unwrap_or(false)
            })
            .map(|s| s.id.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::ParsedEvent;
    use crate::watcher::WatcherEvent;
    use std::path::PathBuf;

    async fn test_session_mgr() -> (SessionManager, Arc<Mutex<Store>>) {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let store = Arc::new(Mutex::new(Store::open(&db_path).unwrap()));
        let mgr = SessionManager::new(store.clone(), 10);
        (mgr, store)
    }

    fn make_watcher_event(parsed: ParsedEvent) -> WatcherEvent {
        WatcherEvent {
            parsed,
            source_file: PathBuf::from("/test/file.jsonl"),
            project_dir: "-Users-test-dev".to_string(),
            is_subagent: false,
        }
    }

    #[tokio::test]
    async fn test_session_created_on_first_event() {
        let (mut mgr, store) = test_session_mgr().await;
        let now = Utc::now();

        let event = make_watcher_event(ParsedEvent::UserPrompt {
            session_id: "sess-1".to_string(),
            uuid: "u-1".to_string(),
            timestamp: now,
            content: "Hello".to_string(),
            cwd: Some("/Users/test/dev".to_string()),
            git_branch: Some("main".to_string()),
        });

        mgr.process_event(event).await.unwrap();

        // Check store
        let session = store.lock().await.get_session("sess-1").unwrap().unwrap();
        assert_eq!(session.tool, "claude_code");
        assert_eq!(session.project_path.as_deref(), Some("/Users/test/dev"));
        assert!(session.ended_at.is_none());

        // Check in-memory state
        assert!(mgr.active_sessions.contains_key("sess-1"));
        assert_eq!(mgr.active_sessions["sess-1"].prompt_count, 1);
    }

    #[tokio::test]
    async fn test_events_persisted() {
        let (mut mgr, store) = test_session_mgr().await;
        let now = Utc::now();

        // User prompt
        mgr.process_event(make_watcher_event(ParsedEvent::UserPrompt {
            session_id: "sess-2".to_string(),
            uuid: "u-1".to_string(),
            timestamp: now,
            content: "Fix the bug".to_string(),
            cwd: Some("/project".to_string()),
            git_branch: None,
        }))
        .await
        .unwrap();

        // Assistant response
        mgr.process_event(make_watcher_event(ParsedEvent::AssistantText {
            session_id: "sess-2".to_string(),
            uuid: "a-1".to_string(),
            timestamp: now + Duration::seconds(5),
            text: "I'll fix that.".to_string(),
            model: Some("claude-opus".to_string()),
            input_tokens: Some(100),
            output_tokens: Some(50),
        }))
        .await
        .unwrap();

        // Tool use
        mgr.process_event(make_watcher_event(ParsedEvent::ToolUse {
            session_id: "sess-2".to_string(),
            uuid: "a-2".to_string(),
            timestamp: now + Duration::seconds(6),
            tool_name: "Edit".to_string(),
            tool_id: "toolu_01".to_string(),
            input: serde_json::json!({"file_path": "/src/main.rs"}),
        }))
        .await
        .unwrap();

        let events = store.lock().await.get_events_for_session("sess-2").unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].event_type, "user_prompt");
        assert_eq!(events[1].event_type, "assistant_response");
        assert_eq!(events[2].event_type, "tool_use");
    }

    #[tokio::test]
    async fn test_idle_session_detection() {
        let (mut mgr, store) = test_session_mgr().await;
        // Use a very short timeout for testing
        mgr.idle_timeout = Duration::seconds(0);

        let past = Utc::now() - Duration::minutes(20);

        mgr.process_event(make_watcher_event(ParsedEvent::UserPrompt {
            session_id: "old-sess".to_string(),
            uuid: "u-1".to_string(),
            timestamp: past,
            content: "Hello".to_string(),
            cwd: Some("/test".to_string()),
            git_branch: None,
        }))
        .await
        .unwrap();

        assert!(mgr.active_sessions.contains_key("old-sess"));

        mgr.check_idle_sessions().await.unwrap();

        assert!(!mgr.active_sessions.contains_key("old-sess"));

        let session = store.lock().await.get_session("old-sess").unwrap().unwrap();
        assert!(session.ended_at.is_some());
    }

    #[tokio::test]
    async fn test_active_sessions_for_repo() {
        let (mut mgr, _store) = test_session_mgr().await;
        let now = Utc::now();

        // Session in /project-a
        mgr.process_event(make_watcher_event(ParsedEvent::UserPrompt {
            session_id: "sess-a".to_string(),
            uuid: "u-1".to_string(),
            timestamp: now,
            content: "Hello".to_string(),
            cwd: Some("/project-a".to_string()),
            git_branch: None,
        }))
        .await
        .unwrap();

        // Session in /project-b
        mgr.process_event(make_watcher_event(ParsedEvent::UserPrompt {
            session_id: "sess-b".to_string(),
            uuid: "u-2".to_string(),
            timestamp: now,
            content: "Hi".to_string(),
            cwd: Some("/project-b".to_string()),
            git_branch: None,
        }))
        .await
        .unwrap();

        let matches = mgr.active_sessions_for_repo("/project-a");
        assert_eq!(matches, vec!["sess-a".to_string()]);

        let no_matches = mgr.active_sessions_for_repo("/project-c");
        assert!(no_matches.is_empty());
    }
}
