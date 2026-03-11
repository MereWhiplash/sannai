use clap::{Parser, Subcommand};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;

use sannai::{api, comment, config, daemon, provenance, service, session, store, watcher};

#[derive(Parser)]
#[command(name = "sannai")]
#[command(about = "AI coding session capture daemon")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the background daemon
    Start {
        /// Run in foreground (default; daemon mode not yet supported)
        #[arg(long, default_value = "true")]
        foreground: bool,
    },
    /// Stop the running daemon
    Stop,
    /// Show daemon status
    Status,
    /// List captured sessions
    Sessions {
        /// Maximum number of sessions to display
        #[arg(long, default_value = "20")]
        limit: u32,
    },
    /// Install sannai as a system service (launchd on macOS, systemd on Linux)
    Install,
    /// Uninstall the sannai system service
    Uninstall {
        /// Also remove all stored data (sessions, database)
        #[arg(long)]
        purge: bool,
    },
    /// Post provenance comment on a GitHub PR
    Comment {
        /// PR URL (e.g., https://github.com/owner/repo/pull/123)
        #[arg(long)]
        pr: String,

        /// Path to the git repository (defaults to current directory)
        #[arg(long)]
        repo: Option<String>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "sannai=info".into()),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Start { .. } => {
            run_daemon().await?;
        }
        Commands::Stop => {
            daemon::stop_daemon()?;
        }
        Commands::Install => {
            service::install_service()?;
        }
        Commands::Uninstall { purge } => {
            service::uninstall_service(purge)?;
        }
        Commands::Status => {
            match daemon::daemon_status() {
                Some(pid) => println!("sannai daemon: running (PID {})", pid),
                None => println!("sannai daemon: not running"),
            }
            println!(
                "Service installed: {}",
                if service::is_service_installed() { "yes" } else { "no" }
            );
            let data_dir = daemon::data_dir();
            println!("Data directory: {}", data_dir.display());
            let db_path = data_dir.join("store.db");
            if db_path.exists() {
                if let Ok(s) = store::Store::open(&db_path) {
                    if let Ok(sessions) = s.list_sessions(u32::MAX, 0) {
                        println!("Sessions captured: {}", sessions.len());
                    }
                }
            }
        }
        Commands::Sessions { limit } => {
            let db_path = daemon::data_dir().join("store.db");
            if !db_path.exists() {
                println!("No sessions captured yet. (database not found)");
                return Ok(());
            }
            let s = store::Store::open(&db_path)?;
            let sessions = s.list_sessions(limit, 0)?;
            if sessions.is_empty() {
                println!("No sessions captured yet.");
            } else {
                println!("{:<38} {:<12} {:<40} STARTED", "SESSION ID", "TOOL", "PROJECT");
                println!("{}", "-".repeat(110));
                for session in &sessions {
                    println!(
                        "{:<38} {:<12} {:<40} {}",
                        session.id,
                        session.tool,
                        session.project_path.as_deref().unwrap_or("-"),
                        session.started_at.format("%Y-%m-%d %H:%M"),
                    );
                }
                println!("\n{} session(s)", sessions.len());
            }
        }
        Commands::Comment { pr, repo } => {
            run_comment(&pr, repo.as_deref())?;
        }
    }

    Ok(())
}

fn run_comment(pr_url: &str, repo_path: Option<&str>) -> anyhow::Result<()> {
    let repo_path = repo_path.map(|s| s.to_string()).unwrap_or_else(|| {
        std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| ".".to_string())
    });

    // 1. Load config
    let cfg = config::load_config();

    // 2. Get PR commit SHAs
    println!("Fetching PR commits...");
    let commit_shas = comment::github::get_pr_commits(pr_url)?;
    if commit_shas.is_empty() {
        println!("No commits found in this PR.");
        return Ok(());
    }
    println!("Found {} commit(s)", commit_shas.len());

    // 3. Open store and find linked sessions
    let db_path = daemon::data_dir().join("store.db");
    if !db_path.exists() {
        anyhow::bail!("No Sannai database found. Has the agent been running?");
    }
    let store = store::Store::open(&db_path)?;

    let mut session_ids = HashSet::new();
    for sha in &commit_shas {
        for s in store.get_sessions_for_commit(sha)? {
            session_ids.insert(s.id);
        }
    }

    // Fallback: match sessions by repo path + time window
    if session_ids.is_empty() {
        println!("No commit-linked sessions found, trying project path matching...");
        let time_range = comment::github::get_pr_commit_time_range(pr_url)?;
        for session in store.list_sessions(1000, 0)? {
            if let Some(proj) = &session.project_path {
                let path_match =
                    proj == &repo_path || repo_path.ends_with(proj) || proj.ends_with(&repo_path);
                if !path_match {
                    continue;
                }
                // Filter by time: session must overlap with PR commit window (with 2hr buffer)
                if let Some((earliest, latest)) = &time_range {
                    let buffer = chrono::Duration::hours(2);
                    let window_start = *earliest - buffer;
                    let window_end = *latest + buffer;
                    let session_end = session.ended_at.unwrap_or(chrono::Utc::now());
                    if session.started_at <= window_end && session_end >= window_start {
                        session_ids.insert(session.id);
                    }
                } else {
                    // No time range available, include all path matches
                    session_ids.insert(session.id);
                }
            }
        }
    }

    if session_ids.is_empty() {
        println!("No Sannai sessions found for this PR's commits.");
        println!("Make sure the Sannai agent was running during development.");
        return Ok(());
    }

    println!("Found {} session(s) with provenance data", session_ids.len());

    // 4. Build interactions and lineage per session
    let mut all_interactions = Vec::new();
    let mut all_lineage = Vec::new();
    let mut session_summaries = Vec::new();

    for session_id in &session_ids {
        let events = store.get_events_for_session(session_id)?;
        let interactions = provenance::interaction::build_interactions(session_id, &events);

        let mut session_lineage = Vec::new();
        for interaction in &interactions {
            session_lineage.extend(provenance::lineage::build_lineage(interaction));
        }

        let duration =
            if let (Some(first), Some(last)) = (interactions.first(), interactions.last()) {
                format_duration(last.timestamp_end - first.timestamp_start)
            } else {
                "0m".to_string()
            };

        session_summaries.push(comment::format::SessionSummary {
            session_id: session_id.clone(),
            interactions: interactions.clone(),
            lineage: session_lineage.clone(),
            duration,
        });

        all_interactions.extend(interactions);
        all_lineage.extend(session_lineage);
    }

    // 5. Diff attribution
    println!("Computing diff attribution...");
    let pr_diff = comment::github::get_pr_diff(pr_url).unwrap_or_default();
    let all_attributions =
        provenance::attribution::attribute_diff_text(&pr_diff, &all_interactions);

    // 6. LLM summary (optional)
    let llm_summary = if cfg.summary.enabled && !cfg.summary.command.is_empty() {
        println!("Generating LLM summary...");
        let bundle = provenance::summary::ProvenanceBundle {
            interactions: all_interactions,
            lineage: all_lineage,
            attributions: all_attributions.clone(),
            diff: pr_diff,
        };
        let summary_config = provenance::summary::SummaryConfig {
            enabled: cfg.summary.enabled,
            command: cfg.summary.command,
            max_length: cfg.summary.max_length,
        };
        provenance::summary::generate_summary(&bundle, &summary_config)
    } else {
        None
    };

    // 7. Format comment
    let comment_data = comment::format::CommentData {
        sessions: session_summaries,
        attributions: all_attributions,
        llm_summary,
    };
    let comment_body = comment::format::format_comment(&comment_data);

    // 8. Post to GitHub
    println!("Posting comment to PR...");
    comment::github::post_pr_comment(pr_url, &comment_body)?;
    println!("Done! Provenance comment posted.");

    Ok(())
}

fn format_duration(dur: chrono::Duration) -> String {
    let total_seconds = dur.num_seconds();
    if total_seconds < 60 {
        format!("{}s", total_seconds)
    } else if total_seconds < 3600 {
        format!("{}m", total_seconds / 60)
    } else {
        let hours = total_seconds / 3600;
        let minutes = (total_seconds % 3600) / 60;
        format!("{}h {}m", hours, minutes)
    }
}

async fn run_daemon() -> anyhow::Result<()> {
    // 1. Acquire PID file
    daemon::acquire_pidfile()?;

    // 2. Open store
    let db_path = daemon::data_dir().join("store.db");
    let store = Arc::new(Mutex::new(store::Store::open(&db_path)?));

    // 3. Cancellation token for graceful shutdown
    let cancel = CancellationToken::new();

    // 4. Signal handlers
    {
        let cancel = cancel.clone();
        tokio::spawn(async move {
            tokio::signal::ctrl_c().await.ok();
            tracing::info!("Received SIGINT, shutting down...");
            cancel.cancel();
        });
    }

    #[cfg(unix)]
    {
        let cancel = cancel.clone();
        tokio::spawn(async move {
            let mut sigterm =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("Failed to register SIGTERM handler");
            sigterm.recv().await;
            tracing::info!("Received SIGTERM, shutting down...");
            cancel.cancel();
        });
    }

    // 5. Watcher -> Session Manager channel
    let (tx, rx) = mpsc::channel::<watcher::WatcherEvent>(100_000);

    // 6. Session Manager
    let session_mgr = Arc::new(Mutex::new(session::SessionManager::new(
        store.clone(),
        10, // idle timeout in minutes
    )));

    // 7. API state
    let api_state = api::AppState { store: store.clone(), session_manager: session_mgr.clone() };

    // 8. Spawn all tasks
    let claude_dir = daemon::claude_projects_dir();
    let state_path = daemon::data_dir().join("watcher_state.json");

    let watcher_cancel = cancel.clone();
    let watcher_handle = tokio::spawn(async move {
        let mut w = watcher::FileWatcher::new(claude_dir, state_path, tx);
        w.run(watcher_cancel).await
    });

    let session_cancel = cancel.clone();
    let session_handle =
        tokio::spawn(async move { session_mgr.lock().await.run(rx, session_cancel).await });

    let api_cancel = cancel.clone();
    let api_handle = tokio::spawn(async move { api::serve(api_state, api_cancel).await });

    tracing::info!("sannai daemon started (PID {}, db={})", std::process::id(), db_path.display(),);

    // 9. Wait for any task to complete (or cancellation)
    tokio::select! {
        r = watcher_handle => {
            if let Err(e) = r? { tracing::error!("Watcher error: {}", e); }
        }
        r = session_handle => {
            if let Err(e) = r? { tracing::error!("Session manager error: {}", e); }
        }
        r = api_handle => {
            if let Err(e) = r? { tracing::error!("API server error: {}", e); }
        }
    }

    // 10. Cleanup
    daemon::release_pidfile()?;
    tracing::info!("sannai daemon stopped");

    Ok(())
}
