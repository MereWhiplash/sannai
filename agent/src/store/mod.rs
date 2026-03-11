use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;

const MIGRATION: &str = r#"
CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    tool TEXT NOT NULL,
    project_path TEXT,
    started_at TEXT NOT NULL,
    ended_at TEXT,
    synced_at TEXT,
    metadata TEXT
);

CREATE TABLE IF NOT EXISTS events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL REFERENCES sessions(id),
    event_type TEXT NOT NULL,
    content TEXT,
    context_files TEXT,
    timestamp TEXT NOT NULL,
    metadata TEXT
);

CREATE TABLE IF NOT EXISTS commit_links (
    commit_sha TEXT NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions(id),
    repo_path TEXT NOT NULL,
    linked_at TEXT NOT NULL,
    parent_shas TEXT,
    message TEXT,
    files_changed TEXT,
    diff_stat TEXT,
    detection_method TEXT,
    PRIMARY KEY (commit_sha, session_id)
);

CREATE TABLE IF NOT EXISTS git_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL REFERENCES sessions(id),
    repo_path TEXT NOT NULL,
    event_type TEXT NOT NULL,
    timestamp TEXT NOT NULL,
    data TEXT NOT NULL
);

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

CREATE INDEX IF NOT EXISTS idx_attributions_commit ON attributions(commit_sha);
CREATE INDEX IF NOT EXISTS idx_attributions_session ON attributions(session_id);
CREATE INDEX IF NOT EXISTS idx_events_session ON events(session_id);
CREATE INDEX IF NOT EXISTS idx_events_timestamp ON events(timestamp);
CREATE INDEX IF NOT EXISTS idx_commits_sha ON commit_links(commit_sha);
CREATE INDEX IF NOT EXISTS idx_sessions_ended ON sessions(ended_at);
CREATE INDEX IF NOT EXISTS idx_git_events_session ON git_events(session_id);
CREATE INDEX IF NOT EXISTS idx_git_events_timestamp ON git_events(timestamp);
"#;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub tool: String,
    pub project_path: Option<String>,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub synced_at: Option<DateTime<Utc>>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: Option<i64>,
    pub session_id: String,
    pub event_type: String,
    pub content: Option<String>,
    pub context_files: Option<serde_json::Value>,
    pub timestamp: DateTime<Utc>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attribution {
    pub id: Option<i64>,
    pub commit_sha: String,
    pub session_id: String,
    pub file_path: String,
    pub hunk_start: i32,
    pub hunk_end: i32,
    pub event_id: Option<i64>,
    pub confidence: f32,
    pub attribution_type: String,
    pub method: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitEvent {
    pub id: Option<i64>,
    pub session_id: String,
    pub repo_path: String,
    pub event_type: String,
    pub timestamp: DateTime<Utc>,
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitLink {
    pub commit_sha: String,
    pub session_id: String,
    pub repo_path: String,
    pub linked_at: DateTime<Utc>,
    pub parent_shas: Option<Vec<String>>,
    pub message: Option<String>,
    pub files_changed: Option<Vec<String>>,
    pub diff_stat: Option<serde_json::Value>,
    pub detection_method: Option<String>,
}

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

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create data directory: {}", parent.display()))?;
        }

        let conn = Connection::open(db_path)
            .with_context(|| format!("Failed to open database at {}", db_path.display()))?;

        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .context("Failed to set PRAGMA options")?;

        conn.execute_batch(MIGRATION)
            .context("Failed to run migrations")?;

        Ok(Self { conn })
    }

    // --- Session CRUD ---

    pub fn upsert_session(&self, session: &Session) -> Result<()> {
        self.conn.execute(
            "INSERT INTO sessions (id, tool, project_path, started_at, ended_at, synced_at, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET
                project_path = COALESCE(excluded.project_path, sessions.project_path),
                ended_at = COALESCE(excluded.ended_at, sessions.ended_at),
                synced_at = COALESCE(excluded.synced_at, sessions.synced_at),
                metadata = COALESCE(excluded.metadata, sessions.metadata)",
            params![
                session.id,
                session.tool,
                session.project_path,
                session.started_at.to_rfc3339(),
                session.ended_at.map(|t| t.to_rfc3339()),
                session.synced_at.map(|t| t.to_rfc3339()),
                session.metadata.as_ref().map(|v| v.to_string()),
            ],
        )?;
        Ok(())
    }

    pub fn get_session(&self, id: &str) -> Result<Option<Session>> {
        self.conn
            .query_row(
                "SELECT id, tool, project_path, started_at, ended_at, synced_at, metadata
                 FROM sessions WHERE id = ?1",
                params![id],
                |row| {
                    Ok(Session {
                        id: row.get(0)?,
                        tool: row.get(1)?,
                        project_path: row.get(2)?,
                        started_at: parse_datetime(row.get::<_, String>(3)?),
                        ended_at: row.get::<_, Option<String>>(4)?.map(parse_datetime),
                        synced_at: row.get::<_, Option<String>>(5)?.map(parse_datetime),
                        metadata: row
                            .get::<_, Option<String>>(6)?
                            .and_then(|s| serde_json::from_str(&s).ok()),
                    })
                },
            )
            .optional()
            .context("Failed to query session")
    }

    pub fn list_sessions(&self, limit: u32, offset: u32) -> Result<Vec<Session>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, tool, project_path, started_at, ended_at, synced_at, metadata
             FROM sessions ORDER BY started_at DESC LIMIT ?1 OFFSET ?2",
        )?;

        let rows = stmt.query_map(params![limit, offset], |row| {
            Ok(Session {
                id: row.get(0)?,
                tool: row.get(1)?,
                project_path: row.get(2)?,
                started_at: parse_datetime(row.get::<_, String>(3)?),
                ended_at: row.get::<_, Option<String>>(4)?.map(parse_datetime),
                synced_at: row.get::<_, Option<String>>(5)?.map(parse_datetime),
                metadata: row
                    .get::<_, Option<String>>(6)?
                    .and_then(|s| serde_json::from_str(&s).ok()),
            })
        })?;

        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(row?);
        }
        Ok(sessions)
    }

    pub fn end_session(&self, id: &str, ended_at: DateTime<Utc>) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET ended_at = ?1 WHERE id = ?2 AND ended_at IS NULL",
            params![ended_at.to_rfc3339(), id],
        )?;
        Ok(())
    }

    // --- Event CRUD ---

    pub fn insert_event(&self, event: &Event) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO events (session_id, event_type, content, context_files, timestamp, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                event.session_id,
                event.event_type,
                event.content,
                event.context_files.as_ref().map(|v| v.to_string()),
                event.timestamp.to_rfc3339(),
                event.metadata.as_ref().map(|v| v.to_string()),
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_events_for_session(&self, session_id: &str) -> Result<Vec<Event>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, event_type, content, context_files, timestamp, metadata
             FROM events WHERE session_id = ?1 ORDER BY timestamp ASC",
        )?;

        let rows = stmt.query_map(params![session_id], |row| {
            Ok(Event {
                id: Some(row.get(0)?),
                session_id: row.get(1)?,
                event_type: row.get(2)?,
                content: row.get(3)?,
                context_files: row
                    .get::<_, Option<String>>(4)?
                    .and_then(|s| serde_json::from_str(&s).ok()),
                timestamp: parse_datetime(row.get::<_, String>(5)?),
                metadata: row
                    .get::<_, Option<String>>(6)?
                    .and_then(|s| serde_json::from_str(&s).ok()),
            })
        })?;

        let mut events = Vec::new();
        for row in rows {
            events.push(row?);
        }
        Ok(events)
    }

    pub fn count_events_for_session(&self, session_id: &str) -> Result<u64> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM events WHERE session_id = ?1",
            params![session_id],
            |row| row.get(0),
        )?;
        Ok(count as u64)
    }

    // --- Commit links ---

    pub fn link_commit(&self, link: &CommitLink) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO commit_links (commit_sha, session_id, repo_path, linked_at, parent_shas, message, files_changed, diff_stat, detection_method)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                link.commit_sha,
                link.session_id,
                link.repo_path,
                link.linked_at.to_rfc3339(),
                link.parent_shas.as_ref().map(|v| serde_json::to_string(v).unwrap()),
                link.message,
                link.files_changed.as_ref().map(|v| serde_json::to_string(v).unwrap()),
                link.diff_stat.as_ref().map(|v| v.to_string()),
                link.detection_method,
            ],
        )?;
        Ok(())
    }

    pub fn get_commit_links_for_session(&self, session_id: &str) -> Result<Vec<CommitLink>> {
        let mut stmt = self.conn.prepare(
            "SELECT commit_sha, session_id, repo_path, linked_at, parent_shas, message, files_changed, diff_stat, detection_method
             FROM commit_links WHERE session_id = ?1 ORDER BY linked_at ASC",
        )?;

        let rows = stmt.query_map(params![session_id], |row| {
            Ok(CommitLink {
                commit_sha: row.get(0)?,
                session_id: row.get(1)?,
                repo_path: row.get(2)?,
                linked_at: parse_datetime(row.get::<_, String>(3)?),
                parent_shas: row
                    .get::<_, Option<String>>(4)?
                    .and_then(|s| serde_json::from_str(&s).ok()),
                message: row.get(5)?,
                files_changed: row
                    .get::<_, Option<String>>(6)?
                    .and_then(|s| serde_json::from_str(&s).ok()),
                diff_stat: row
                    .get::<_, Option<String>>(7)?
                    .and_then(|s| serde_json::from_str(&s).ok()),
                detection_method: row.get(8)?,
            })
        })?;

        let mut links = Vec::new();
        for row in rows {
            links.push(row?);
        }
        Ok(links)
    }

    pub fn get_sessions_for_commit(&self, sha: &str) -> Result<Vec<Session>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.id, s.tool, s.project_path, s.started_at, s.ended_at, s.synced_at, s.metadata
             FROM sessions s
             INNER JOIN commit_links cl ON s.id = cl.session_id
             WHERE cl.commit_sha = ?1",
        )?;

        let rows = stmt.query_map(params![sha], |row| {
            Ok(Session {
                id: row.get(0)?,
                tool: row.get(1)?,
                project_path: row.get(2)?,
                started_at: parse_datetime(row.get::<_, String>(3)?),
                ended_at: row.get::<_, Option<String>>(4)?.map(parse_datetime),
                synced_at: row.get::<_, Option<String>>(5)?.map(parse_datetime),
                metadata: row
                    .get::<_, Option<String>>(6)?
                    .and_then(|s| serde_json::from_str(&s).ok()),
            })
        })?;

        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(row?);
        }
        Ok(sessions)
    }

    pub fn get_events_in_time_range(
        &self,
        session_id: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<Event>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, event_type, content, context_files, timestamp, metadata
             FROM events WHERE session_id = ?1 AND timestamp >= ?2 AND timestamp <= ?3
             ORDER BY timestamp ASC",
        )?;

        let rows = stmt.query_map(
            params![session_id, from.to_rfc3339(), to.to_rfc3339()],
            |row| {
                Ok(Event {
                    id: Some(row.get(0)?),
                    session_id: row.get(1)?,
                    event_type: row.get(2)?,
                    content: row.get(3)?,
                    context_files: row
                        .get::<_, Option<String>>(4)?
                        .and_then(|s| serde_json::from_str(&s).ok()),
                    timestamp: parse_datetime(row.get::<_, String>(5)?),
                    metadata: row
                        .get::<_, Option<String>>(6)?
                        .and_then(|s| serde_json::from_str(&s).ok()),
                })
            },
        )?;

        let mut events = Vec::new();
        for row in rows {
            events.push(row?);
        }
        Ok(events)
    }

    // --- Git events ---

    pub fn insert_git_event(&self, event: &GitEvent) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO git_events (session_id, repo_path, event_type, timestamp, data)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                event.session_id,
                event.repo_path,
                event.event_type,
                event.timestamp.to_rfc3339(),
                event.data.to_string(),
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_git_events_for_session(&self, session_id: &str) -> Result<Vec<GitEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, repo_path, event_type, timestamp, data
             FROM git_events WHERE session_id = ?1 ORDER BY timestamp ASC",
        )?;

        let rows = stmt.query_map(params![session_id], |row| {
            Ok(GitEvent {
                id: Some(row.get(0)?),
                session_id: row.get(1)?,
                repo_path: row.get(2)?,
                event_type: row.get(3)?,
                timestamp: parse_datetime(row.get::<_, String>(4)?),
                data: row
                    .get::<_, String>(5)
                    .map(|s| serde_json::from_str(&s).unwrap_or_default())?,
            })
        })?;

        let mut events = Vec::new();
        for row in rows {
            events.push(row?);
        }
        Ok(events)
    }

    // --- Attributions ---

    pub fn insert_attribution(&self, attr: &Attribution) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO attributions (commit_sha, session_id, file_path, hunk_start, hunk_end, event_id, confidence, attribution_type, method, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(commit_sha, file_path, hunk_start, hunk_end) DO UPDATE SET
                confidence = excluded.confidence,
                method = excluded.method,
                event_id = excluded.event_id",
            params![
                attr.commit_sha,
                attr.session_id,
                attr.file_path,
                attr.hunk_start,
                attr.hunk_end,
                attr.event_id,
                attr.confidence,
                attr.attribution_type,
                attr.method,
                attr.created_at.to_rfc3339(),
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_attributions_for_commit(&self, sha: &str) -> Result<Vec<Attribution>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, commit_sha, session_id, file_path, hunk_start, hunk_end, event_id, confidence, attribution_type, method, created_at
             FROM attributions WHERE commit_sha = ?1 ORDER BY file_path, hunk_start",
        )?;

        let rows = stmt.query_map(params![sha], |row| {
            Ok(Attribution {
                id: Some(row.get(0)?),
                commit_sha: row.get(1)?,
                session_id: row.get(2)?,
                file_path: row.get(3)?,
                hunk_start: row.get(4)?,
                hunk_end: row.get(5)?,
                event_id: row.get(6)?,
                confidence: row.get(7)?,
                attribution_type: row.get(8)?,
                method: row.get(9)?,
                created_at: parse_datetime(row.get::<_, String>(10)?),
            })
        })?;

        let mut attrs = Vec::new();
        for row in rows {
            attrs.push(row?);
        }
        Ok(attrs)
    }

    pub fn get_attributions_for_session(&self, session_id: &str) -> Result<Vec<Attribution>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, commit_sha, session_id, file_path, hunk_start, hunk_end, event_id, confidence, attribution_type, method, created_at
             FROM attributions WHERE session_id = ?1 ORDER BY created_at ASC",
        )?;

        let rows = stmt.query_map(params![session_id], |row| {
            Ok(Attribution {
                id: Some(row.get(0)?),
                commit_sha: row.get(1)?,
                session_id: row.get(2)?,
                file_path: row.get(3)?,
                hunk_start: row.get(4)?,
                hunk_end: row.get(5)?,
                event_id: row.get(6)?,
                confidence: row.get(7)?,
                attribution_type: row.get(8)?,
                method: row.get(9)?,
                created_at: parse_datetime(row.get::<_, String>(10)?),
            })
        })?;

        let mut attrs = Vec::new();
        for row in rows {
            attrs.push(row?);
        }
        Ok(attrs)
    }

    // --- Process metrics ---

    pub fn insert_process_metrics(&self, pm: &ProcessMetrics) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO process_metrics (session_id, commit_sha, steering_ratio, exploration_score, read_write_ratio, test_behavior, error_fix_cycles, red_flags, prompt_specificity, total_interactions, total_tool_calls, files_read, files_written, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                pm.session_id,
                pm.commit_sha,
                pm.steering_ratio,
                pm.exploration_score,
                pm.read_write_ratio,
                pm.test_behavior,
                pm.error_fix_cycles,
                pm.red_flags.to_string(),
                pm.prompt_specificity,
                pm.total_interactions,
                pm.total_tool_calls,
                pm.files_read,
                pm.files_written,
                pm.created_at.to_rfc3339(),
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_process_metrics_for_session(&self, session_id: &str) -> Result<Vec<ProcessMetrics>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, commit_sha, steering_ratio, exploration_score, read_write_ratio, test_behavior, error_fix_cycles, red_flags, prompt_specificity, total_interactions, total_tool_calls, files_read, files_written, created_at
             FROM process_metrics WHERE session_id = ?1 ORDER BY created_at ASC",
        )?;

        let rows = stmt.query_map(params![session_id], |row| {
            Ok(ProcessMetrics {
                id: Some(row.get(0)?),
                session_id: row.get(1)?,
                commit_sha: row.get(2)?,
                steering_ratio: row.get(3)?,
                exploration_score: row.get(4)?,
                read_write_ratio: row.get(5)?,
                test_behavior: row.get(6)?,
                error_fix_cycles: row.get(7)?,
                red_flags: row
                    .get::<_, String>(8)
                    .map(|s| serde_json::from_str(&s).unwrap_or_default())?,
                prompt_specificity: row.get(9)?,
                total_interactions: row.get(10)?,
                total_tool_calls: row.get(11)?,
                files_read: row.get(12)?,
                files_written: row.get(13)?,
                created_at: parse_datetime(row.get::<_, String>(14)?),
            })
        })?;

        let mut metrics = Vec::new();
        for row in rows {
            metrics.push(row?);
        }
        Ok(metrics)
    }

    pub fn get_process_metrics_for_commit(&self, sha: &str) -> Result<Vec<ProcessMetrics>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, commit_sha, steering_ratio, exploration_score, read_write_ratio, test_behavior, error_fix_cycles, red_flags, prompt_specificity, total_interactions, total_tool_calls, files_read, files_written, created_at
             FROM process_metrics WHERE commit_sha = ?1 ORDER BY created_at ASC",
        )?;

        let rows = stmt.query_map(params![sha], |row| {
            Ok(ProcessMetrics {
                id: Some(row.get(0)?),
                session_id: row.get(1)?,
                commit_sha: row.get(2)?,
                steering_ratio: row.get(3)?,
                exploration_score: row.get(4)?,
                read_write_ratio: row.get(5)?,
                test_behavior: row.get(6)?,
                error_fix_cycles: row.get(7)?,
                red_flags: row
                    .get::<_, String>(8)
                    .map(|s| serde_json::from_str(&s).unwrap_or_default())?,
                prompt_specificity: row.get(9)?,
                total_interactions: row.get(10)?,
                total_tool_calls: row.get(11)?,
                files_read: row.get(12)?,
                files_written: row.get(13)?,
                created_at: parse_datetime(row.get::<_, String>(14)?),
            })
        })?;

        let mut metrics = Vec::new();
        for row in rows {
            metrics.push(row?);
        }
        Ok(metrics)
    }

    pub fn get_active_sessions(&self) -> Result<Vec<Session>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, tool, project_path, started_at, ended_at, synced_at, metadata
             FROM sessions WHERE ended_at IS NULL ORDER BY started_at DESC",
        )?;

        let rows = stmt.query_map([], |row| {
            Ok(Session {
                id: row.get(0)?,
                tool: row.get(1)?,
                project_path: row.get(2)?,
                started_at: parse_datetime(row.get::<_, String>(3)?),
                ended_at: row.get::<_, Option<String>>(4)?.map(parse_datetime),
                synced_at: row.get::<_, Option<String>>(5)?.map(parse_datetime),
                metadata: row
                    .get::<_, Option<String>>(6)?
                    .and_then(|s| serde_json::from_str(&s).ok()),
            })
        })?;

        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(row?);
        }
        Ok(sessions)
    }
}

fn parse_datetime(s: String) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(&s)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_store() -> (Store, TempDir) {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let store = Store::open(&db_path).unwrap();
        (store, dir)
    }

    #[test]
    fn test_open_creates_tables() {
        let (store, _dir) = test_store();
        // Should be able to query sessions without error
        let sessions = store.list_sessions(10, 0).unwrap();
        assert!(sessions.is_empty());
    }

    #[test]
    fn test_upsert_and_get_session() {
        let (store, _dir) = test_store();
        let now = Utc::now();

        let session = Session {
            id: "test-session-1".to_string(),
            tool: "claude_code".to_string(),
            project_path: Some("/Users/test/project".to_string()),
            started_at: now,
            ended_at: None,
            synced_at: None,
            metadata: None,
        };

        store.upsert_session(&session).unwrap();

        let retrieved = store.get_session("test-session-1").unwrap().unwrap();
        assert_eq!(retrieved.id, "test-session-1");
        assert_eq!(retrieved.tool, "claude_code");
        assert_eq!(
            retrieved.project_path.as_deref(),
            Some("/Users/test/project")
        );
        assert!(retrieved.ended_at.is_none());
    }

    #[test]
    fn test_upsert_updates_existing() {
        let (store, _dir) = test_store();
        let now = Utc::now();

        let session = Session {
            id: "test-session-2".to_string(),
            tool: "claude_code".to_string(),
            project_path: None,
            started_at: now,
            ended_at: None,
            synced_at: None,
            metadata: None,
        };
        store.upsert_session(&session).unwrap();

        // Upsert with project_path set
        let updated = Session {
            project_path: Some("/Users/test/updated".to_string()),
            ..session
        };
        store.upsert_session(&updated).unwrap();

        let retrieved = store.get_session("test-session-2").unwrap().unwrap();
        assert_eq!(
            retrieved.project_path.as_deref(),
            Some("/Users/test/updated")
        );
    }

    #[test]
    fn test_end_session() {
        let (store, _dir) = test_store();
        let now = Utc::now();

        let session = Session {
            id: "test-session-3".to_string(),
            tool: "claude_code".to_string(),
            project_path: None,
            started_at: now,
            ended_at: None,
            synced_at: None,
            metadata: None,
        };
        store.upsert_session(&session).unwrap();

        store.end_session("test-session-3", now).unwrap();

        let retrieved = store.get_session("test-session-3").unwrap().unwrap();
        assert!(retrieved.ended_at.is_some());
    }

    #[test]
    fn test_list_sessions_ordering() {
        let (store, _dir) = test_store();
        let now = Utc::now();

        for i in 0..5 {
            let session = Session {
                id: format!("session-{}", i),
                tool: "claude_code".to_string(),
                project_path: None,
                started_at: now + chrono::Duration::minutes(i as i64),
                ended_at: None,
                synced_at: None,
                metadata: None,
            };
            store.upsert_session(&session).unwrap();
        }

        let sessions = store.list_sessions(3, 0).unwrap();
        assert_eq!(sessions.len(), 3);
        // Most recent first
        assert_eq!(sessions[0].id, "session-4");
        assert_eq!(sessions[1].id, "session-3");
        assert_eq!(sessions[2].id, "session-2");

        // Offset
        let page2 = store.list_sessions(3, 3).unwrap();
        assert_eq!(page2.len(), 2);
        assert_eq!(page2[0].id, "session-1");
    }

    #[test]
    fn test_insert_and_get_events() {
        let (store, _dir) = test_store();
        let now = Utc::now();

        let session = Session {
            id: "event-test-session".to_string(),
            tool: "claude_code".to_string(),
            project_path: None,
            started_at: now,
            ended_at: None,
            synced_at: None,
            metadata: None,
        };
        store.upsert_session(&session).unwrap();

        let event1 = Event {
            id: None,
            session_id: "event-test-session".to_string(),
            event_type: "user_prompt".to_string(),
            content: Some("Add error handling".to_string()),
            context_files: None,
            timestamp: now,
            metadata: None,
        };
        let id1 = store.insert_event(&event1).unwrap();
        assert!(id1 > 0);

        let event2 = Event {
            id: None,
            session_id: "event-test-session".to_string(),
            event_type: "assistant_response".to_string(),
            content: Some("I'll add try-catch blocks...".to_string()),
            context_files: None,
            timestamp: now + chrono::Duration::seconds(5),
            metadata: None,
        };
        store.insert_event(&event2).unwrap();

        let events = store.get_events_for_session("event-test-session").unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_type, "user_prompt");
        assert_eq!(events[1].event_type, "assistant_response");

        let count = store
            .count_events_for_session("event-test-session")
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn test_commit_links() {
        let (store, _dir) = test_store();
        let now = Utc::now();

        let session = Session {
            id: "commit-test-session".to_string(),
            tool: "claude_code".to_string(),
            project_path: Some("/Users/test/repo".to_string()),
            started_at: now,
            ended_at: None,
            synced_at: None,
            metadata: None,
        };
        store.upsert_session(&session).unwrap();

        let link = CommitLink {
            commit_sha: "abc123def456".to_string(),
            session_id: "commit-test-session".to_string(),
            repo_path: "/Users/test/repo".to_string(),
            linked_at: now,
            parent_shas: None,
            message: None,
            files_changed: None,
            diff_stat: None,
            detection_method: None,
        };
        store.link_commit(&link).unwrap();

        // Duplicate insert should not error (INSERT OR IGNORE)
        store.link_commit(&link).unwrap();

        let sessions = store.get_sessions_for_commit("abc123def456").unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "commit-test-session");
    }

    #[test]
    fn test_get_active_sessions() {
        let (store, _dir) = test_store();
        let now = Utc::now();

        // Active session
        store
            .upsert_session(&Session {
                id: "active-1".to_string(),
                tool: "claude_code".to_string(),
                project_path: None,
                started_at: now,
                ended_at: None,
                synced_at: None,
                metadata: None,
            })
            .unwrap();

        // Ended session
        store
            .upsert_session(&Session {
                id: "ended-1".to_string(),
                tool: "claude_code".to_string(),
                project_path: None,
                started_at: now,
                ended_at: Some(now),
                synced_at: None,
                metadata: None,
            })
            .unwrap();

        let active = store.get_active_sessions().unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, "active-1");
    }

    #[test]
    fn test_insert_and_get_git_events() {
        let (store, _dir) = test_store();
        let now = Utc::now();

        store
            .upsert_session(&Session {
                id: "git-test-session".to_string(),
                tool: "claude_code".to_string(),
                project_path: Some("/test/repo".to_string()),
                started_at: now,
                ended_at: None,
                synced_at: None,
                metadata: None,
            })
            .unwrap();

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

        let events = store
            .get_git_events_for_session("git-test-session")
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "head_changed");
        assert_eq!(events[0].data["new_sha"], "bbb222");
    }

    #[test]
    fn test_insert_and_get_attributions() {
        let (store, _dir) = test_store();
        let now = Utc::now();

        store
            .upsert_session(&Session {
                id: "attr-session".to_string(),
                tool: "claude_code".to_string(),
                project_path: Some("/test/repo".to_string()),
                started_at: now,
                ended_at: None,
                synced_at: None,
                metadata: None,
            })
            .unwrap();

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

        store
            .upsert_session(&Session {
                id: "unique-session".to_string(),
                tool: "claude_code".to_string(),
                project_path: None,
                started_at: now,
                ended_at: None,
                synced_at: None,
                metadata: None,
            })
            .unwrap();

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
        let attr2 = Attribution {
            confidence: 0.95,
            ..attr
        };
        store.insert_attribution(&attr2).unwrap();

        let attrs = store.get_attributions_for_commit("abc123").unwrap();
        assert_eq!(attrs.len(), 1);
        assert!((attrs[0].confidence - 0.95).abs() < f32::EPSILON);
    }

    #[test]
    fn test_get_events_in_time_range() {
        let (store, _dir) = test_store();
        let now = Utc::now();

        store
            .upsert_session(&Session {
                id: "range-session".to_string(),
                tool: "claude_code".to_string(),
                project_path: None,
                started_at: now,
                ended_at: None,
                synced_at: None,
                metadata: None,
            })
            .unwrap();

        // Insert events at different times
        for i in 0..5 {
            store
                .insert_event(&Event {
                    id: None,
                    session_id: "range-session".to_string(),
                    event_type: "tool_use".to_string(),
                    content: Some(format!("event-{}", i)),
                    context_files: None,
                    timestamp: now + chrono::Duration::minutes(i),
                    metadata: None,
                })
                .unwrap();
        }

        // Get events between minute 1 and minute 3
        let from = now + chrono::Duration::minutes(1);
        let to = now + chrono::Duration::minutes(3);
        let events = store
            .get_events_in_time_range("range-session", from, to)
            .unwrap();
        assert_eq!(events.len(), 3); // minutes 1, 2, 3
    }

    #[test]
    fn test_enriched_commit_link() {
        let (store, _dir) = test_store();
        let now = Utc::now();

        store
            .upsert_session(&Session {
                id: "enrich-session".to_string(),
                tool: "claude_code".to_string(),
                project_path: Some("/test/repo".to_string()),
                started_at: now,
                ended_at: None,
                synced_at: None,
                metadata: None,
            })
            .unwrap();

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

        let links = store
            .get_commit_links_for_session("enrich-session")
            .unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].message.as_deref(), Some("feat: add auth"));
        assert_eq!(links[0].detection_method.as_deref(), Some("poll"));
    }

    #[test]
    fn test_insert_and_get_process_metrics() {
        let (store, _dir) = test_store();
        let now = Utc::now();

        store
            .upsert_session(&Session {
                id: "pm-session".to_string(),
                tool: "claude_code".to_string(),
                project_path: None,
                started_at: now,
                ended_at: None,
                synced_at: None,
                metadata: None,
            })
            .unwrap();

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

        let retrieved = store
            .get_process_metrics_for_session("pm-session")
            .unwrap();
        assert_eq!(retrieved.len(), 1);
        assert!((retrieved[0].steering_ratio - 0.72).abs() < f64::EPSILON);
        assert_eq!(retrieved[0].test_behavior, "test_after");
        assert_eq!(retrieved[0].total_interactions, 47);

        let by_commit = store.get_process_metrics_for_commit("abc123").unwrap();
        assert_eq!(by_commit.len(), 1);
    }

    #[test]
    fn test_session_not_found() {
        let (store, _dir) = test_store();
        let result = store.get_session("nonexistent").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_metadata_json_roundtrip() {
        let (store, _dir) = test_store();
        let now = Utc::now();
        let meta = serde_json::json!({"version": "2.1.15", "model": "claude-sonnet"});

        let session = Session {
            id: "meta-test".to_string(),
            tool: "claude_code".to_string(),
            project_path: None,
            started_at: now,
            ended_at: None,
            synced_at: None,
            metadata: Some(meta.clone()),
        };
        store.upsert_session(&session).unwrap();

        let retrieved = store.get_session("meta-test").unwrap().unwrap();
        assert_eq!(retrieved.metadata.unwrap(), meta);
    }
}
