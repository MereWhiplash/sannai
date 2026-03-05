use anyhow::{Context, Result};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::parser::{self, ParsedEvent};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Event emitted by the watcher to the session manager.
#[derive(Debug)]
pub struct WatcherEvent {
    pub parsed: ParsedEvent,
    pub source_file: PathBuf,
    pub project_dir: String,
    pub is_subagent: bool,
}

/// Classification of a discovered .jsonl file.
#[derive(Debug, Clone)]
#[allow(dead_code)]
enum FileClass {
    MainSession {
        session_id: String,
        project_dir: String,
    },
    Subagent {
        parent_session_id: String,
        agent_id: String,
        project_dir: String,
    },
}

/// Tracks a file we're tailing.
struct TailedFile {
    path: PathBuf,
    offset: u64,
    class: FileClass,
}

/// Persistent state for resuming after restart.
#[derive(Debug, Serialize, Deserialize, Default)]
struct WatcherState {
    /// Map of canonical file path -> byte offset
    file_offsets: HashMap<String, u64>,
}

impl WatcherState {
    fn load(path: &Path) -> Self {
        fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        fs::write(path, json)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// FileWatcher
// ---------------------------------------------------------------------------

pub struct FileWatcher {
    claude_dir: PathBuf,
    state_path: PathBuf,
    tailed_files: HashMap<PathBuf, TailedFile>,
    tx: mpsc::Sender<WatcherEvent>,
}

impl FileWatcher {
    pub fn new(
        claude_dir: PathBuf,
        state_path: PathBuf,
        tx: mpsc::Sender<WatcherEvent>,
    ) -> Self {
        Self {
            claude_dir,
            state_path,
            tailed_files: HashMap::new(),
            tx,
        }
    }

    /// Main run loop. Scans existing files, then watches for changes.
    pub async fn run(&mut self, cancel: CancellationToken) -> Result<()> {
        // Load persisted state (offsets from previous run)
        let saved = WatcherState::load(&self.state_path);

        // Scan existing files
        if self.claude_dir.exists() {
            self.scan_existing_files(&saved)?;
        } else {
            tracing::info!(
                "Claude projects directory not found at {}, waiting for it to appear...",
                self.claude_dir.display()
            );
        }

        // Set up notify watcher
        let (notify_tx, notify_rx) = std::sync::mpsc::channel::<notify::Result<Event>>();
        let mut watcher = RecommendedWatcher::new(notify_tx, notify::Config::default())
            .context("Failed to create file watcher")?;

        // Watch the Claude projects directory (create it if needed so the watcher can attach)
        if self.claude_dir.exists() {
            watcher
                .watch(&self.claude_dir, RecursiveMode::Recursive)
                .context("Failed to watch Claude projects directory")?;
            tracing::info!("Watching {}", self.claude_dir.display());
        }

        // Bridge sync notify events to our async loop
        let cancel_clone = cancel.clone();
        loop {
            if cancel_clone.is_cancelled() {
                break;
            }

            // Non-blocking check for notify events
            match notify_rx.recv_timeout(std::time::Duration::from_millis(500)) {
                Ok(Ok(event)) => {
                    self.handle_notify_event(event).await;
                }
                Ok(Err(e)) => {
                    tracing::warn!("File watcher error: {}", e);
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    // Check if the claude dir appeared (in case it didn't exist at startup)
                    if self.claude_dir.exists() && self.tailed_files.is_empty() {
                        self.scan_existing_files(&WatcherState::default())?;
                        if let Err(e) = watcher.watch(&self.claude_dir, RecursiveMode::Recursive) {
                            tracing::warn!("Failed to start watching: {}", e);
                        }
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    tracing::error!("File watcher channel disconnected");
                    break;
                }
            }
        }

        // Persist state on shutdown
        self.save_state();
        Ok(())
    }

    fn scan_existing_files(&mut self, saved: &WatcherState) -> Result<()> {
        let projects_dir = &self.claude_dir;
        if !projects_dir.exists() {
            return Ok(());
        }

        for entry in fs::read_dir(projects_dir)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let project_dir = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();

            // Scan main session files: <project>/<uuid>.jsonl
            for file_entry in fs::read_dir(&path)? {
                let file_entry = file_entry?;
                let file_path = file_entry.path();

                if file_path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                    if let Some(session_id) = parser::extract_session_id(
                        &file_path.file_name().unwrap_or_default().to_string_lossy(),
                    ) {
                        let key = file_path.to_string_lossy().to_string();
                        let offset = saved.file_offsets.get(&key).copied().unwrap_or(0);

                        self.register_file(
                            file_path,
                            FileClass::MainSession {
                                session_id,
                                project_dir: project_dir.clone(),
                            },
                            offset,
                        );
                    }
                }

                // Scan subagent directories: <project>/<uuid>/subagents/agent-*.jsonl
                if file_entry.path().is_dir() {
                    let subagents_dir = file_entry.path().join("subagents");
                    if subagents_dir.exists() {
                        let parent_session_id = file_entry
                            .file_name()
                            .to_string_lossy()
                            .to_string();

                        for sub_entry in fs::read_dir(&subagents_dir)? {
                            let sub_entry = sub_entry?;
                            let sub_path = sub_entry.path();
                            if sub_path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                                let filename =
                                    sub_path.file_name().unwrap_or_default().to_string_lossy();
                                let agent_id = filename
                                    .strip_prefix("agent-")
                                    .and_then(|s| s.strip_suffix(".jsonl"))
                                    .unwrap_or(&filename)
                                    .to_string();

                                let key = sub_path.to_string_lossy().to_string();
                                let offset = saved.file_offsets.get(&key).copied().unwrap_or(0);

                                self.register_file(
                                    sub_path,
                                    FileClass::Subagent {
                                        parent_session_id: parent_session_id.clone(),
                                        agent_id,
                                        project_dir: project_dir.clone(),
                                    },
                                    offset,
                                );
                            }
                        }
                    }
                }
            }
        }

        // Tail all registered files from their saved offsets
        let paths: Vec<PathBuf> = self.tailed_files.keys().cloned().collect();
        for path in paths {
            if let Err(e) = self.tail_file(&path) {
                tracing::warn!("Failed to tail {}: {}", path.display(), e);
            }
        }

        tracing::info!("Scanning complete: {} files tracked", self.tailed_files.len());
        Ok(())
    }

    fn register_file(&mut self, path: PathBuf, class: FileClass, offset: u64) {
        if self.tailed_files.contains_key(&path) {
            return;
        }
        tracing::debug!("Tracking file: {} (offset={})", path.display(), offset);
        self.tailed_files.insert(
            path.clone(),
            TailedFile {
                path,
                offset,
                class,
            },
        );
    }

    async fn handle_notify_event(&mut self, event: Event) {
        match event.kind {
            EventKind::Create(_) | EventKind::Modify(_) => {
                for path in &event.paths {
                    if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                        continue;
                    }

                    // Register if new
                    if !self.tailed_files.contains_key(path) {
                        if let Some(class) = self.classify_path(path) {
                            self.register_file(path.clone(), class, 0);
                        }
                    }

                    // Tail the file
                    if let Err(e) = self.tail_file(path) {
                        tracing::warn!("Failed to tail {}: {}", path.display(), e);
                    }
                }
            }
            _ => {}
        }
    }

    fn tail_file(&mut self, path: &Path) -> Result<()> {
        let tailed = match self.tailed_files.get_mut(path) {
            Some(t) => t,
            None => return Ok(()),
        };

        let file = File::open(&tailed.path)?;
        let file_len = file.metadata()?.len();

        if file_len <= tailed.offset {
            return Ok(());
        }

        let mut reader = BufReader::new(file);
        reader.seek(SeekFrom::Start(tailed.offset))?;

        let mut bytes_read = tailed.offset;
        let mut line_buf = String::new();

        loop {
            line_buf.clear();
            let n = reader.read_line(&mut line_buf)?;
            if n == 0 {
                break; // EOF
            }
            bytes_read += n as u64;

            let line = line_buf.trim();
            if line.is_empty() {
                continue;
            }

            match parser::parse_line(line) {
                Ok(parsed_events) => {
                    for parsed in parsed_events {
                        if matches!(parsed, ParsedEvent::Ignored) {
                            continue;
                        }

                        let (project_dir, is_subagent) = match &tailed.class {
                            FileClass::MainSession { project_dir, .. } => {
                                (project_dir.clone(), false)
                            }
                            FileClass::Subagent { project_dir, .. } => {
                                (project_dir.clone(), true)
                            }
                        };

                        let event = WatcherEvent {
                            parsed,
                            source_file: tailed.path.clone(),
                            project_dir,
                            is_subagent,
                        };

                        // Use try_send to avoid blocking; if channel is full, log and skip
                        if let Err(e) = self.tx.try_send(event) {
                            tracing::warn!("Failed to send watcher event: {}", e);
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to parse line in {}: {}",
                        tailed.path.display(),
                        e
                    );
                }
            }
        }

        tailed.offset = bytes_read;
        Ok(())
    }

    fn classify_path(&self, path: &Path) -> Option<FileClass> {
        // Expected structures:
        // ~/.claude/projects/<project-dir>/<uuid>.jsonl          -> MainSession
        // ~/.claude/projects/<project-dir>/<uuid>/subagents/agent-<id>.jsonl -> Subagent

        let rel = path.strip_prefix(&self.claude_dir).ok()?;
        let components: Vec<&std::ffi::OsStr> = rel.iter().collect();

        match components.len() {
            // <project-dir>/<session>.jsonl
            2 => {
                let project_dir = components[0].to_string_lossy().to_string();
                let filename = components[1].to_string_lossy();
                let session_id = parser::extract_session_id(&filename)?;
                Some(FileClass::MainSession {
                    session_id,
                    project_dir,
                })
            }
            // <project-dir>/<session-id>/subagents/<agent>.jsonl
            4 => {
                let project_dir = components[0].to_string_lossy().to_string();
                let parent_session_id = components[1].to_string_lossy().to_string();
                let filename = components[3].to_string_lossy();
                let agent_id = filename
                    .strip_prefix("agent-")
                    .and_then(|s| s.strip_suffix(".jsonl"))
                    .unwrap_or(&filename)
                    .to_string();
                Some(FileClass::Subagent {
                    parent_session_id,
                    agent_id,
                    project_dir,
                })
            }
            _ => None,
        }
    }

    fn save_state(&self) {
        let mut state = WatcherState::default();
        for (path, tailed) in &self.tailed_files {
            state
                .file_offsets
                .insert(path.to_string_lossy().to_string(), tailed.offset);
        }
        if let Err(e) = state.save(&self.state_path) {
            tracing::warn!("Failed to save watcher state: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_watcher_scans_jsonl_files() {
        let dir = tempfile::TempDir::new().unwrap();
        let projects_dir = dir.path().join("projects");
        let project_dir = projects_dir.join("-Users-test-dev-myproject");
        fs::create_dir_all(&project_dir).unwrap();

        // Write a session file
        let session_file = project_dir.join("abc-123.jsonl");
        fs::write(
            &session_file,
            r#"{"type":"queue-operation","operation":"dequeue","timestamp":"2026-01-27T15:56:56.357Z","sessionId":"abc-123"}
{"parentUuid":null,"isSidechain":false,"userType":"external","cwd":"/Users/test/dev/myproject","sessionId":"abc-123","version":"2.1.15","gitBranch":"main","type":"user","message":{"role":"user","content":"Hello"},"uuid":"u-1","timestamp":"2026-01-27T15:56:57.000Z"}
"#,
        )
        .unwrap();

        let state_path = dir.path().join("state.json");
        let (tx, mut rx) = mpsc::channel(100);
        let mut watcher = FileWatcher::new(projects_dir, state_path, tx);

        // Just scan, don't run the full loop
        watcher
            .scan_existing_files(&WatcherState::default())
            .unwrap();

        // Drain events
        let mut events = Vec::new();
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }

        assert_eq!(events.len(), 2); // SessionStart + UserPrompt
        assert_eq!(events[0].project_dir, "-Users-test-dev-myproject");
        assert!(!events[0].is_subagent);
    }

    #[tokio::test]
    async fn test_watcher_state_persistence() {
        let dir = tempfile::TempDir::new().unwrap();
        let state_path = dir.path().join("state.json");

        let mut state = WatcherState::default();
        state
            .file_offsets
            .insert("/some/file.jsonl".to_string(), 42);
        state.save(&state_path).unwrap();

        let loaded = WatcherState::load(&state_path);
        assert_eq!(loaded.file_offsets.get("/some/file.jsonl"), Some(&42));
    }

    #[test]
    fn test_classify_main_session() {
        let claude_dir = PathBuf::from("/home/user/.claude/projects");
        let (tx, _rx) = mpsc::channel(1);
        let watcher = FileWatcher::new(
            claude_dir,
            PathBuf::from("/tmp/state.json"),
            tx,
        );

        let path = PathBuf::from(
            "/home/user/.claude/projects/-Users-test-dev/abc-123-def.jsonl",
        );
        let class = watcher.classify_path(&path).unwrap();
        match class {
            FileClass::MainSession {
                session_id,
                project_dir,
            } => {
                assert_eq!(session_id, "abc-123-def");
                assert_eq!(project_dir, "-Users-test-dev");
            }
            _ => panic!("Expected MainSession"),
        }
    }

    #[test]
    fn test_classify_subagent() {
        let claude_dir = PathBuf::from("/home/user/.claude/projects");
        let (tx, _rx) = mpsc::channel(1);
        let watcher = FileWatcher::new(
            claude_dir,
            PathBuf::from("/tmp/state.json"),
            tx,
        );

        let path = PathBuf::from(
            "/home/user/.claude/projects/-Users-test-dev/abc-123/subagents/agent-x7y8z9.jsonl",
        );
        let class = watcher.classify_path(&path).unwrap();
        match class {
            FileClass::Subagent {
                parent_session_id,
                agent_id,
                project_dir,
            } => {
                assert_eq!(parent_session_id, "abc-123");
                assert_eq!(agent_id, "x7y8z9");
                assert_eq!(project_dir, "-Users-test-dev");
            }
            _ => panic!("Expected Subagent"),
        }
    }

    #[tokio::test]
    async fn test_watcher_tails_from_offset() {
        let dir = tempfile::TempDir::new().unwrap();
        let projects_dir = dir.path().join("projects");
        let project_dir = projects_dir.join("-test");
        fs::create_dir_all(&project_dir).unwrap();

        let session_file = project_dir.join("sess-1.jsonl");
        let line1 = r#"{"type":"queue-operation","operation":"dequeue","timestamp":"2026-01-27T15:56:56.357Z","sessionId":"sess-1"}"#;
        let line2 = r#"{"parentUuid":null,"isSidechain":false,"userType":"external","cwd":"/test","sessionId":"sess-1","version":"2.1.15","gitBranch":"","type":"user","message":{"role":"user","content":"First prompt"},"uuid":"u-1","timestamp":"2026-01-27T15:57:00.000Z"}"#;
        fs::write(&session_file, format!("{}\n{}\n", line1, line2)).unwrap();

        // Save state with offset past the first line
        let first_line_len = (line1.len() + 1) as u64; // +1 for newline
        let mut saved = WatcherState::default();
        saved.file_offsets.insert(
            session_file.to_string_lossy().to_string(),
            first_line_len,
        );

        let state_path = dir.path().join("state.json");
        let (tx, mut rx) = mpsc::channel(100);
        let mut watcher = FileWatcher::new(projects_dir, state_path, tx);

        watcher.scan_existing_files(&saved).unwrap();

        // Should only get the second event (UserPrompt), not the SessionStart
        let mut events = Vec::new();
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }

        assert_eq!(events.len(), 1);
        match &events[0].parsed {
            ParsedEvent::UserPrompt { content, .. } => {
                assert_eq!(content, "First prompt");
            }
            other => panic!("Expected UserPrompt, got {:?}", other),
        }
    }
}
