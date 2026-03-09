use clap::{Parser, Subcommand};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;

use sannai_agent::{api, comment, daemon, git, session, store, watcher};

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
                .unwrap_or_else(|_| "sannai_agent=info".into()),
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
        Commands::Status => match daemon::daemon_status() {
            Some(pid) => println!("sannai daemon: running (PID {})", pid),
            None => println!("sannai daemon: not running"),
        },
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
                println!(
                    "{:<38} {:<12} {:<40} STARTED",
                    "SESSION ID", "TOOL", "PROJECT"
                );
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
    let repo_path = repo_path
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            std::env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| ".".to_string())
        });

    // 1. Get PR commit SHAs
    println!("Fetching PR commits...");
    let commit_shas = comment::github::get_pr_commits(pr_url)?;
    if commit_shas.is_empty() {
        println!("No commits found in this PR.");
        return Ok(());
    }
    println!("Found {} commit(s)", commit_shas.len());

    // 2. Open store and find linked sessions
    let db_path = daemon::data_dir().join("store.db");
    if !db_path.exists() {
        anyhow::bail!("No Sannai database found. Has the agent been running?");
    }
    let s = store::Store::open(&db_path)?;

    let mut session_ids = HashSet::new();
    for sha in &commit_shas {
        for sess in s.get_sessions_for_commit(sha)? {
            session_ids.insert(sess.id);
        }
    }

    // Fallback: match sessions by repo path
    if session_ids.is_empty() {
        println!("No commit-linked sessions found, trying project path matching...");
        for sess in s.list_sessions(100, 0)? {
            if let Some(proj) = &sess.project_path {
                if proj == &repo_path
                    || repo_path.ends_with(proj)
                    || proj.ends_with(&repo_path)
                {
                    session_ids.insert(sess.id);
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

    // 3. Aggregate process metrics across sessions and commits
    let mut total_interactions: i32 = 0;
    let mut total_steering: f64 = 0.0;
    let mut total_exploration: f64 = 0.0;
    let mut total_specificity: f64 = 0.0;
    let mut total_error_cycles: i32 = 0;
    let mut total_files_read: i32 = 0;
    let mut total_files_written: i32 = 0;
    let mut all_red_flags: Vec<String> = Vec::new();
    let mut test_behaviors: Vec<String> = Vec::new();
    let mut metrics_count: usize = 0;

    for session_id in &session_ids {
        let metrics = s.get_process_metrics_for_session(session_id)?;
        for pm in &metrics {
            metrics_count += 1;
            total_interactions += pm.total_interactions;
            total_steering += pm.steering_ratio;
            total_exploration += pm.exploration_score;
            total_specificity += pm.prompt_specificity;
            total_error_cycles += pm.error_fix_cycles;
            total_files_read += pm.files_read;
            total_files_written += pm.files_written;
            test_behaviors.push(pm.test_behavior.clone());
            if let Some(flags) = pm.red_flags.as_array() {
                for flag in flags {
                    if let Some(f) = flag.as_str() {
                        if !all_red_flags.contains(&f.to_string()) {
                            all_red_flags.push(f.to_string());
                        }
                    }
                }
            }
        }
    }

    if metrics_count == 0 {
        println!("No process metrics found. The agent may not have captured commit-time analysis.");
        return Ok(());
    }

    // Determine dominant test behavior
    let test_behavior = if test_behaviors.iter().any(|t| t == "tdd") {
        "tdd".to_string()
    } else if test_behaviors.iter().any(|t| t == "test_after") {
        "test_after".to_string()
    } else if test_behaviors.iter().any(|t| t == "test_only") {
        "test_only".to_string()
    } else {
        "no_tests".to_string()
    };

    // 4. Format process audit comment
    let audit_data = comment::format::ProcessAuditData {
        session_count: session_ids.len() as i32,
        total_interactions,
        steering_ratio: total_steering / metrics_count as f64,
        exploration_score: total_exploration / metrics_count as f64,
        test_behavior,
        error_fix_cycles: total_error_cycles,
        red_flags: all_red_flags,
        prompt_specificity: total_specificity / metrics_count as f64,
        files_read: total_files_read,
        files_written: total_files_written,
    };
    let comment_body = comment::format::format_process_audit(&audit_data);

    // 5. Post to GitHub
    println!("Posting process audit comment to PR...");
    comment::github::post_pr_comment(pr_url, &comment_body)?;
    println!("Done! Process audit comment posted.");

    Ok(())
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

    // 6. Git Observer channel
    let (git_cmd_tx, git_cmd_rx) = mpsc::channel::<git::observer::GitObserverCommand>(100);

    // 7. Session Manager (with git observer channel)
    let mut session_mgr_inner = session::SessionManager::new(
        store.clone(),
        10, // idle timeout in minutes
    );
    session_mgr_inner.set_git_cmd_tx(git_cmd_tx);
    let session_mgr = Arc::new(Mutex::new(session_mgr_inner));

    // 8. API state
    let api_state = api::AppState {
        store: store.clone(),
        session_manager: session_mgr.clone(),
    };

    // 9. Spawn all tasks
    let claude_dir = daemon::claude_projects_dir();
    let state_path = daemon::data_dir().join("watcher_state.json");

    let watcher_cancel = cancel.clone();
    let watcher_handle = tokio::spawn(async move {
        let mut w = watcher::FileWatcher::new(claude_dir, state_path, tx);
        w.run(watcher_cancel).await
    });

    let session_cancel = cancel.clone();
    let session_handle = tokio::spawn(async move {
        session_mgr.lock().await.run(rx, session_cancel).await
    });

    let api_cancel = cancel.clone();
    let api_handle = tokio::spawn(async move {
        api::serve(api_state, api_cancel).await
    });

    let git_cancel = cancel.clone();
    let git_store = store.clone();
    let git_handle = tokio::spawn(async move {
        let mut observer = git::observer::GitObserver::new(git_store, git_cmd_rx);
        observer.run(git_cancel).await
    });

    tracing::info!(
        "sannai daemon started (PID {}, db={})",
        std::process::id(),
        db_path.display(),
    );

    // 10. Wait for any task to complete (or cancellation)
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
        r = git_handle => {
            if let Err(e) = r? { tracing::error!("Git observer error: {}", e); }
        }
    }

    // 11. Cleanup
    daemon::release_pidfile()?;
    tracing::info!("sannai daemon stopped");

    Ok(())
}
