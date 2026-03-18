use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tower_http::cors::CorsLayer;

use crate::session::SessionManager;
use crate::store::{self, Store};

// ---------------------------------------------------------------------------
// Shared state
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<Mutex<Store>>,
    pub session_manager: Arc<Mutex<SessionManager>>,
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/hook/commit", post(hook_commit))
        .route("/hook/push", post(hook_push))
        .route("/sessions", get(list_sessions))
        .route("/sessions/:id", get(get_session))
        .route("/sessions/:id/events", get(get_session_events))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

/// Start the API server, runs until cancellation.
pub async fn serve(state: AppState, cancel: CancellationToken) -> anyhow::Result<()> {
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:9847").await?;
    tracing::info!("Local API server listening on 127.0.0.1:9847");
    axum::serve(listener, app).with_graceful_shutdown(cancel.cancelled_owned()).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

// --- POST /hook/commit ---

#[derive(Deserialize)]
struct CommitHookRequest {
    sha: String,
    repo: String,
    /// If provided, link directly to this session instead of searching active sessions.
    session_id: Option<String>,
}

#[derive(Serialize)]
struct CommitHookResponse {
    linked_sessions: Vec<String>,
}

async fn hook_commit(
    State(state): State<AppState>,
    Json(req): Json<CommitHookRequest>,
) -> Result<Json<CommitHookResponse>, StatusCode> {
    // If session_id is provided (e.g. from Claude Code hook), link directly.
    // Otherwise fall back to searching active sessions by repo path.
    let session_ids = if let Some(ref sid) = req.session_id {
        vec![sid.clone()]
    } else {
        state.session_manager.lock().await.active_sessions_for_repo(&req.repo)
    };

    let store = state.store.lock().await;
    let mut linked = Vec::new();

    for session_id in &session_ids {
        // Ensure the session exists in the store before linking
        if req.session_id.is_some() {
            if let Ok(None) = store.get_session(session_id) {
                // Session not yet tracked by sannai — create a stub so the link succeeds
                let session = store::Session {
                    id: session_id.clone(),
                    tool: "claude_code".to_string(),
                    project_path: Some(req.repo.clone()),
                    git_branch: None,
                    started_at: chrono::Utc::now(),
                    ended_at: None,
                    synced_at: None,
                    metadata: None,
                };
                let _ = store.upsert_session(&session);
            }
        }

        let link = store::CommitLink {
            commit_sha: req.sha.clone(),
            session_id: session_id.clone(),
            repo_path: req.repo.clone(),
            linked_at: chrono::Utc::now(),
        };
        if let Err(e) = store.link_commit(&link) {
            tracing::warn!("Failed to link commit {} to session {}: {}", req.sha, session_id, e);
        } else {
            linked.push(session_id.clone());
        }
    }

    tracing::info!(
        "Commit {} linked to {} session(s) in {}",
        &req.sha[..std::cmp::min(8, req.sha.len())],
        linked.len(),
        req.repo,
    );

    Ok(Json(CommitHookResponse { linked_sessions: linked }))
}

// --- POST /hook/push ---

#[derive(Deserialize)]
struct PushHookRequest {
    branch: String,
    owner_repo: String,
    repo_path: String,
}

async fn hook_push(
    State(state): State<AppState>,
    Json(req): Json<PushHookRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let store = state.store.lock().await;
    store.record_push(&req.branch, &req.owner_repo, &req.repo_path).map_err(|e| {
        tracing::warn!("Failed to record push: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    tracing::info!("Recorded push: branch={} repo={}", req.branch, req.owner_repo,);

    Ok(Json(serde_json::json!({"ok": true})))
}

// --- GET /sessions ---

#[derive(Deserialize)]
struct ListSessionsQuery {
    #[serde(default = "default_limit")]
    limit: u32,
    #[serde(default)]
    offset: u32,
}

fn default_limit() -> u32 {
    20
}

#[derive(Serialize)]
struct SessionResponse {
    id: String,
    tool: String,
    project_path: Option<String>,
    started_at: String,
    ended_at: Option<String>,
    event_count: u64,
}

async fn list_sessions(
    State(state): State<AppState>,
    Query(query): Query<ListSessionsQuery>,
) -> Result<Json<Vec<SessionResponse>>, StatusCode> {
    let store = state.store.lock().await;
    let sessions = store
        .list_sessions(query.limit, query.offset)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut response = Vec::new();
    for s in sessions {
        let count = store.count_events_for_session(&s.id).unwrap_or(0);
        response.push(SessionResponse {
            id: s.id,
            tool: s.tool,
            project_path: s.project_path,
            started_at: s.started_at.to_rfc3339(),
            ended_at: s.ended_at.map(|t| t.to_rfc3339()),
            event_count: count,
        });
    }

    Ok(Json(response))
}

// --- GET /sessions/:id ---

async fn get_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<SessionResponse>, StatusCode> {
    let store = state.store.lock().await;
    let session = store
        .get_session(&id)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let count = store.count_events_for_session(&session.id).unwrap_or(0);

    Ok(Json(SessionResponse {
        id: session.id,
        tool: session.tool,
        project_path: session.project_path,
        started_at: session.started_at.to_rfc3339(),
        ended_at: session.ended_at.map(|t| t.to_rfc3339()),
        event_count: count,
    }))
}

// --- GET /sessions/:id/events ---

async fn get_session_events(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<store::Event>>, StatusCode> {
    let store = state.store.lock().await;

    // Verify session exists
    store
        .get_session(&id)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let events =
        store.get_events_for_session(&id).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(events))
}
