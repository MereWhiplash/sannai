use clap::{Parser, Subcommand};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;

use sannai_agent::{api, daemon, session, store, watcher};

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
    }

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

    // 6. Session Manager
    let session_mgr = Arc::new(Mutex::new(session::SessionManager::new(
        store.clone(),
        10, // idle timeout in minutes
    )));

    // 7. API state
    let api_state = api::AppState {
        store: store.clone(),
        session_manager: session_mgr.clone(),
    };

    // 8. Spawn all tasks
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

    tracing::info!(
        "sannai daemon started (PID {}, db={})",
        std::process::id(),
        db_path.display(),
    );

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
