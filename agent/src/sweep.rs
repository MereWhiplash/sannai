//! Periodic sweep for pending PR comments and data retention.
//!
//! - Checks recorded pushes for open PRs and posts provenance comments.
//! - Prunes old sessions/events, shrinks oversized content, and VACUUMs to keep the DB lean.

use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::store::Store;

const SWEEP_INTERVAL_SECS: u64 = 60;
const MAX_PENDING_AGE_HOURS: i64 = 24;
const RETENTION_DAYS: i64 = 14;
const MAX_CONTENT_BYTES: usize = 2048;

/// Run the sweep loop until cancelled.
pub async fn run(store: Arc<Mutex<Store>>, cancel: CancellationToken) -> anyhow::Result<()> {
    // Initial delay to let the daemon fully start
    tokio::select! {
        _ = tokio::time::sleep(std::time::Duration::from_secs(10)) => {}
        _ = cancel.cancelled() => return Ok(()),
    }

    // Run retention once on startup
    run_retention(&store, false).await;

    let mut tick_count: u64 = 0;
    loop {
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(SWEEP_INTERVAL_SECS)) => {}
            _ = cancel.cancelled() => return Ok(()),
        }

        tick_count += 1;

        // PR sweep every tick (60s)
        if let Err(e) = sweep_pending_prs(&store).await {
            tracing::warn!("PR sweep error: {}", e);
        }

        // Retention every ~6 hours (360 ticks)
        if tick_count.is_multiple_of(360) {
            // VACUUM once a day (every 4th retention run = 1440 ticks = 24h)
            let vacuum = tick_count.is_multiple_of(1440);
            run_retention(&store, vacuum).await;
        }
    }
}

async fn run_retention(store: &Arc<Mutex<Store>>, vacuum: bool) {
    let s = store.lock().await;

    match s.prune_old_sessions(RETENTION_DAYS) {
        Ok(0) => {}
        Ok(n) => {
            tracing::info!("Retention: pruned {} session(s) older than {}d", n, RETENTION_DAYS)
        }
        Err(e) => tracing::warn!("Retention prune error: {}", e),
    }

    match s.shrink_large_events(MAX_CONTENT_BYTES) {
        Ok(0) => {}
        Ok(n) => tracing::info!("Retention: shrunk {} oversized event(s)", n),
        Err(e) => tracing::warn!("Retention shrink error: {}", e),
    }

    match s.slim_tool_use_metadata(8192) {
        Ok(0) => {}
        Ok(n) => tracing::info!("Retention: slimmed {} tool_use event(s)", n),
        Err(e) => tracing::warn!("Retention slim error: {}", e),
    }

    if vacuum {
        let size_before = s.db_size_bytes().unwrap_or(0);
        match s.vacuum() {
            Ok(()) => {
                let size_after = s.db_size_bytes().unwrap_or(0);
                if size_before > size_after {
                    tracing::info!(
                        "VACUUM: {:.1}MB → {:.1}MB",
                        size_before as f64 / 1_048_576.0,
                        size_after as f64 / 1_048_576.0,
                    );
                }
            }
            Err(e) => tracing::warn!("VACUUM error: {}", e),
        }
    }
}

async fn sweep_pending_prs(store: &Arc<Mutex<Store>>) -> anyhow::Result<()> {
    let pending = {
        let s = store.lock().await;
        let cleaned = s.cleanup_old_pending_pushes(MAX_PENDING_AGE_HOURS)?;
        if cleaned > 0 {
            tracing::debug!("Cleaned up {} stale pending push records", cleaned);
        }
        s.get_pending_pushes(MAX_PENDING_AGE_HOURS)?
    };

    if pending.is_empty() {
        return Ok(());
    }

    tracing::debug!("Sweeping {} pending push(es) for open PRs", pending.len());

    for entry in &pending {
        match check_and_comment(entry).await {
            Ok(true) => {
                let s = store.lock().await;
                s.remove_pending_push(&entry.branch, &entry.owner_repo)?;
            }
            Ok(false) => {
                // No PR yet, keep for next sweep
            }
            Err(e) => {
                tracing::debug!(
                    "Sweep check failed for {}/{}: {}",
                    entry.owner_repo,
                    entry.branch,
                    e
                );
            }
        }
    }

    Ok(())
}

/// Check if an open PR exists for this branch and post a comment if so.
/// Returns Ok(true) if a comment was posted, Ok(false) if no PR found.
async fn check_and_comment(entry: &crate::store::PendingComment) -> anyhow::Result<bool> {
    let pr_output = tokio::process::Command::new("gh")
        .args([
            "pr",
            "view",
            &entry.branch,
            "--repo",
            &entry.owner_repo,
            "--json",
            "number",
            "--jq",
            ".number",
        ])
        .output()
        .await?;

    if !pr_output.status.success() {
        return Ok(false);
    }

    let pr_number = String::from_utf8_lossy(&pr_output.stdout).trim().to_string();
    if pr_number.is_empty() {
        return Ok(false);
    }

    let pr_url = format!("https://github.com/{}/pull/{}", entry.owner_repo, pr_number);

    tracing::info!("Sweep found PR for {}/{}: {}", entry.owner_repo, entry.branch, pr_url);

    let sannai_bin = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("sannai"));

    let result = tokio::process::Command::new(&sannai_bin)
        .args(["comment", "--pr", &pr_url, "--repo", &entry.repo_path])
        .output()
        .await?;

    if result.status.success() {
        tracing::info!("Sweep: comment posted on {}", pr_url);
    } else {
        let stderr = String::from_utf8_lossy(&result.stderr);
        tracing::warn!("Sweep: sannai comment failed for {}: {}", pr_url, stderr);
    }

    // Remove pending record either way to avoid infinite retries
    Ok(true)
}
