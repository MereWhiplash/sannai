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
    PRIMARY KEY (commit_sha, session_id)
);

CREATE INDEX IF NOT EXISTS idx_events_session ON events(session_id);
CREATE INDEX IF NOT EXISTS idx_events_timestamp ON events(timestamp);
CREATE INDEX IF NOT EXISTS idx_commits_sha ON commit_links(commit_sha);
CREATE INDEX IF NOT EXISTS idx_sessions_ended ON sessions(ended_at);
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
pub struct CommitLink {
    pub commit_sha: String,
    pub session_id: String,
    pub repo_path: String,
    pub linked_at: DateTime<Utc>,
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
            "INSERT OR IGNORE INTO commit_links (commit_sha, session_id, repo_path, linked_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                link.commit_sha,
                link.session_id,
                link.repo_path,
                link.linked_at.to_rfc3339(),
            ],
        )?;
        Ok(())
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
