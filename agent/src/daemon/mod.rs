use anyhow::{bail, Context, Result};
use directories::ProjectDirs;
use std::fs;
use std::path::PathBuf;

/// Data directory for the store DB and watcher state.
/// macOS: ~/Library/Application Support/dev.sannai.sannai/
/// Linux: ~/.local/share/sannai/
/// Override with SANNAI_DATA_DIR for testing.
pub fn data_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("SANNAI_DATA_DIR") {
        return PathBuf::from(dir);
    }
    ProjectDirs::from("dev", "sannai", "sannai")
        .map(|dirs| dirs.data_dir().to_path_buf())
        .unwrap_or_else(|| {
            let home = std::env::var("HOME")
                .map(PathBuf::from)
                .expect("Could not determine home directory");
            home.join(".sannai")
        })
}

/// Claude Code projects directory.
/// Override with SANNAI_CLAUDE_DIR for testing.
pub fn claude_projects_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("SANNAI_CLAUDE_DIR") {
        return PathBuf::from(dir);
    }
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .expect("Could not determine home directory");
    home.join(".claude").join("projects")
}

/// PID file path.
fn pid_file_path() -> PathBuf {
    data_dir().join("sannai.pid")
}

/// Write current PID to pidfile. Returns error if daemon is already running.
pub fn acquire_pidfile() -> Result<()> {
    let path = pid_file_path();

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    if path.exists() {
        let pid_str = fs::read_to_string(&path).unwrap_or_default();
        if let Ok(pid) = pid_str.trim().parse::<u32>() {
            if is_process_running(pid) {
                bail!("Daemon already running (PID {})", pid);
            }
            tracing::warn!("Removing stale PID file for PID {}", pid);
        }
        fs::remove_file(&path)?;
    }

    fs::write(&path, std::process::id().to_string())
        .context("Failed to write PID file")?;

    Ok(())
}

/// Remove PID file on shutdown.
pub fn release_pidfile() -> Result<()> {
    let path = pid_file_path();
    if path.exists() {
        fs::remove_file(&path)?;
    }
    Ok(())
}

/// Check if the daemon is running. Returns PID if so.
pub fn daemon_status() -> Option<u32> {
    read_running_pid()
}

/// Read PID from pidfile, only if that process is actually running.
fn read_running_pid() -> Option<u32> {
    let path = pid_file_path();
    let pid_str = fs::read_to_string(path).ok()?;
    let pid: u32 = pid_str.trim().parse().ok()?;
    if is_process_running(pid) {
        Some(pid)
    } else {
        None
    }
}

/// Send stop signal to running daemon.
pub fn stop_daemon() -> Result<()> {
    match read_running_pid() {
        Some(pid) => {
            send_term_signal(pid)?;
            println!("Sent stop signal to daemon (PID {})", pid);
            Ok(())
        }
        None => {
            println!("No running daemon found.");
            Ok(())
        }
    }
}

/// Check if a process is running.
#[cfg(unix)]
fn is_process_running(pid: u32) -> bool {
    // kill(pid, 0) checks if process exists without sending a signal
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

#[cfg(not(unix))]
fn is_process_running(_pid: u32) -> bool {
    // Fallback: assume not running on non-Unix
    false
}

/// Send SIGTERM to a process.
#[cfg(unix)]
fn send_term_signal(pid: u32) -> Result<()> {
    let result = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
    if result != 0 {
        bail!("Failed to send SIGTERM to PID {}", pid);
    }
    Ok(())
}

#[cfg(not(unix))]
fn send_term_signal(_pid: u32) -> Result<()> {
    bail!("Stop command not supported on this platform");
}
