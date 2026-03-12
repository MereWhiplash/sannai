# Open Source Release Implementation Plan

**Design:** [docs/designs/2026-03-11-open-source-release.md](../designs/2026-03-11-open-source-release.md)

**Goal:** Prepare sannai for open-source release with OSS boilerplate, crates.io packaging, and native daemon management.

**Architecture:** Add `sannai install` and `sannai uninstall` CLI commands that write/remove platform-native service files (launchd on macOS, systemd on Linux). New `service` module in the agent handles platform detection, template rendering, and service lifecycle. OSS boilerplate files are static additions at repo root.

**EC Context:**
- Decision: sannai/release — Crate name `sannai`, v0.1.0, gh CLI auth, keep data on uninstall
- Decision: sannai/install — Launchd plist + systemd user unit, binary path via current_exe()
- Learning: sannai/agent — 33 tests, clippy clean, camelCase envelope / snake_case inner fields

---

## Part 1: Open Source Boilerplate

### Task 1: Add LICENSE file

**Files:**
- Create: `LICENSE`

**Step 1: Write failing test** (RED)
```bash
test -f LICENSE && echo "PASS" || echo "FAIL"
```
Expected: FAIL

**Step 2: Create LICENSE**

MIT License, year 2026, "Sannai contributors".

**Step 3: Verify**
```bash
test -f LICENSE && head -1 LICENSE | grep -q "MIT" && echo "PASS" || echo "FAIL"
```
Expected: PASS

**Step 4: Commit**
```bash
git add LICENSE && git commit -m "Add MIT LICENSE file"
```

---

### Task 2: Add CONTRIBUTING.md

**Files:**
- Create: `CONTRIBUTING.md`

Content:
- Prerequisites (Rust stable)
- Build from source
- Running tests
- Code style (rustfmt.toml, clippy)
- PR process (fork, branch, test, PR)
- Issue reporting guidance

**Step 1: Create file**

**Step 2: Verify**
```bash
test -f CONTRIBUTING.md && echo "PASS" || echo "FAIL"
```

**Step 3: Commit**
```bash
git add CONTRIBUTING.md && git commit -m "Add CONTRIBUTING.md"
```

---

### Task 3: Add CODE_OF_CONDUCT.md

**Files:**
- Create: `CODE_OF_CONDUCT.md`

Standard Contributor Covenant v2.1.

**Step 1: Create file**

**Step 2: Commit**
```bash
git add CODE_OF_CONDUCT.md && git commit -m "Add Contributor Covenant code of conduct"
```

---

### Task 4: Add GitHub issue templates

**Files:**
- Create: `.github/ISSUE_TEMPLATE/bug_report.yml`
- Create: `.github/ISSUE_TEMPLATE/feature_request.yml`

Bug report fields: description, steps to reproduce, expected behavior, OS, sannai version.
Feature request fields: problem description, proposed solution, alternatives considered.

**Step 1: Create templates**

**Step 2: Verify valid YAML**
```bash
python3 -c "import yaml; yaml.safe_load(open('.github/ISSUE_TEMPLATE/bug_report.yml')); yaml.safe_load(open('.github/ISSUE_TEMPLATE/feature_request.yml')); print('PASS')"
```

**Step 3: Commit**
```bash
git add .github/ISSUE_TEMPLATE/ && git commit -m "Add GitHub issue templates for bugs and features"
```

---

### Task 5: Add CI badges to README and update Cargo.toml metadata

**Files:**
- Modify: `README.md` (add badges at top)
- Modify: `agent/Cargo.toml` (rename crate, add metadata)

**Step 1: Update Cargo.toml**

Change `name` from `sannai-agent` to `sannai`. Add fields:
```toml
repository = "https://github.com/MereWhiplash/sannai"
homepage = "https://github.com/MereWhiplash/sannai"
keywords = ["ai", "provenance", "code-review", "claude", "developer-tools"]
categories = ["command-line-utilities", "development-tools"]
readme = "../README.md"
```

**Step 2: Update internal references**

Grep for `sannai-agent` and `sannai_agent` across the codebase. The binary name stays `sannai`. The lib crate name will change from `sannai_agent` to `sannai` (underscore form of crate name). Update:
- `agent/src/main.rs`: `use sannai_agent::` → `use sannai::`
- `RUST_LOG=sannai_agent=info` → `RUST_LOG=sannai=info` (in main.rs and scripts)
- `agent/CLAUDE.md` references
- Data dir path in `daemon/mod.rs`: `ProjectDirs::from("dev", "sannai", "sannai-agent")` → `ProjectDirs::from("dev", "sannai", "sannai")`

**Step 3: Add badges to README**

Add after `# Sannai` heading:
```markdown
[![CI](https://github.com/MereWhiplash/sannai/actions/workflows/ci.yml/badge.svg)](https://github.com/MereWhiplash/sannai/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Crate](https://img.shields.io/crates/v/sannai.svg)](https://crates.io/crates/sannai)
```

**Step 4: Verify build**
```bash
cd agent && cargo build && cargo test
```

**Step 5: Commit**
```bash
git add -A && git commit -m "Rename crate to sannai, add badges and crates.io metadata"
```

---

## Part 2: Service Management

### Task 6: Add service module with platform detection @tdd

**Files:**
- Create: `agent/src/service/mod.rs`
- Modify: `agent/src/lib.rs`

**Step 1: Write failing test** (RED)
```rust
// agent/src/service/mod.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_platform() {
        let platform = detect_platform();
        #[cfg(target_os = "macos")]
        assert_eq!(platform, Platform::MacOS);
        #[cfg(target_os = "linux")]
        assert_eq!(platform, Platform::Linux);
    }

    #[test]
    fn test_service_file_path() {
        let path = service_file_path(Platform::MacOS);
        assert!(path.to_string_lossy().contains("LaunchAgents"));
        assert!(path.to_string_lossy().contains("dev.sannai.agent.plist"));

        let path = service_file_path(Platform::Linux);
        assert!(path.to_string_lossy().contains("systemd/user"));
        assert!(path.to_string_lossy().contains("sannai.service"));
    }
}
```

**Step 2: Run test, verify it fails**
```bash
cd agent && cargo test test_detect_platform
```
Expected: FAIL — module doesn't exist

**Step 3: Implement**

```rust
// agent/src/service/mod.rs

use anyhow::{bail, Result};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Platform {
    MacOS,
    Linux,
}

pub fn detect_platform() -> Platform {
    if cfg!(target_os = "macos") {
        Platform::MacOS
    } else {
        Platform::Linux
    }
}

pub fn service_file_path(platform: Platform) -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    match platform {
        Platform::MacOS => PathBuf::from(&home)
            .join("Library/LaunchAgents/dev.sannai.agent.plist"),
        Platform::Linux => PathBuf::from(&home)
            .join(".config/systemd/user/sannai.service"),
    }
}
```

Add `pub mod service;` to `lib.rs`.

**Step 4: Run test, verify it passes** @verifying
```bash
cd agent && cargo test test_detect_platform test_service_file_path
```
Expected: PASS

**Step 5: Commit**
```bash
git add -A && git commit -m "feat: add service module with platform detection"
```

---

### Task 7: Launchd plist generation @tdd

**Files:**
- Modify: `agent/src/service/mod.rs`

**Step 1: Write failing test** (RED)
```rust
#[test]
fn test_generate_launchd_plist() {
    let plist = generate_launchd_plist("/usr/local/bin/sannai");
    assert!(plist.contains("<key>Label</key>"));
    assert!(plist.contains("dev.sannai.agent"));
    assert!(plist.contains("/usr/local/bin/sannai"));
    assert!(plist.contains("start"));
    assert!(plist.contains("--foreground"));
    assert!(plist.contains("<key>RunAtLoad</key>"));
    assert!(plist.contains("<key>KeepAlive</key>"));
    assert!(plist.contains("sannai.log"));
}
```

**Step 2: Run test, verify it fails**
```bash
cd agent && cargo test test_generate_launchd_plist
```

**Step 3: Implement**

```rust
pub fn generate_launchd_plist(bin_path: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    format!(r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>dev.sannai.agent</string>
    <key>ProgramArguments</key>
    <array>
        <string>{bin_path}</string>
        <string>start</string>
        <string>--foreground</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>{home}/Library/Logs/sannai.log</string>
    <key>StandardErrorPath</key>
    <string>{home}/Library/Logs/sannai.log</string>
</dict>
</plist>
"#)
}
```

**Step 4: Run test, verify it passes** @verifying
```bash
cd agent && cargo test test_generate_launchd_plist
```

**Step 5: Commit**
```bash
git add -A && git commit -m "feat: launchd plist generation"
```

---

### Task 8: Systemd unit generation @tdd

**Files:**
- Modify: `agent/src/service/mod.rs`

**Step 1: Write failing test** (RED)
```rust
#[test]
fn test_generate_systemd_unit() {
    let unit = generate_systemd_unit("/usr/local/bin/sannai");
    assert!(unit.contains("[Unit]"));
    assert!(unit.contains("[Service]"));
    assert!(unit.contains("[Install]"));
    assert!(unit.contains("/usr/local/bin/sannai start --foreground"));
    assert!(unit.contains("Restart=on-failure"));
    assert!(unit.contains("WantedBy=default.target"));
}
```

**Step 2: Implement**

```rust
pub fn generate_systemd_unit(bin_path: &str) -> String {
    format!(r#"[Unit]
Description=Sannai AI coding session capture daemon
After=default.target

[Service]
Type=simple
ExecStart={bin_path} start --foreground
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
"#)
}
```

**Step 3: Run test, verify it passes** @verifying
```bash
cd agent && cargo test test_generate_systemd_unit
```

**Step 4: Commit**
```bash
git add -A && git commit -m "feat: systemd unit generation"
```

---

### Task 9: Install command @tdd

**Files:**
- Modify: `agent/src/service/mod.rs`
- Modify: `agent/src/main.rs`

**Step 1: Write failing test** (RED)
```rust
#[test]
fn test_install_writes_service_file() {
    let dir = tempfile::tempdir().unwrap();
    let service_path = dir.path().join("test.plist");
    let bin_path = "/usr/local/bin/sannai";

    install_service_to(Platform::MacOS, bin_path, &service_path).unwrap();

    assert!(service_path.exists());
    let content = std::fs::read_to_string(&service_path).unwrap();
    assert!(content.contains("dev.sannai.agent"));
}

#[test]
fn test_install_refuses_if_already_installed() {
    let dir = tempfile::tempdir().unwrap();
    let service_path = dir.path().join("test.plist");
    let bin_path = "/usr/local/bin/sannai";

    install_service_to(Platform::MacOS, bin_path, &service_path).unwrap();
    let result = install_service_to(Platform::MacOS, bin_path, &service_path);
    assert!(result.is_err());
}
```

**Step 2: Implement**

```rust
/// Install service file to a specific path (testable).
pub fn install_service_to(platform: Platform, bin_path: &str, path: &PathBuf) -> Result<()> {
    if path.exists() {
        bail!("Service already installed at {}. Run `sannai uninstall` first.", path.display());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = match platform {
        Platform::MacOS => generate_launchd_plist(bin_path),
        Platform::Linux => generate_systemd_unit(bin_path),
    };
    std::fs::write(path, content)?;
    Ok(())
}

/// Install service and load it.
pub fn install_service() -> Result<()> {
    let platform = detect_platform();
    let bin_path = std::env::current_exe()?
        .to_string_lossy().to_string();
    let path = service_file_path(platform);

    install_service_to(platform, &bin_path, &path)?;

    // Load the service
    match platform {
        Platform::MacOS => {
            let status = std::process::Command::new("launchctl")
                .args(["load", &path.to_string_lossy()])
                .status()?;
            if !status.success() {
                bail!("Failed to load launchd service");
            }
            println!("Installed and loaded launchd service.");
            println!("Sannai will start automatically on login.");
        }
        Platform::Linux => {
            std::process::Command::new("systemctl")
                .args(["--user", "daemon-reload"])
                .status()?;
            let status = std::process::Command::new("systemctl")
                .args(["--user", "enable", "--now", "sannai"])
                .status()?;
            if !status.success() {
                bail!("Failed to enable systemd service");
            }
            println!("Installed and enabled systemd user service.");
            println!("Sannai will start automatically on login.");
        }
    }

    println!("Service file: {}", path.display());
    Ok(())
}
```

Add `Install` variant to `Commands` enum in main.rs, wire to `service::install_service()`.

**Step 3: Run tests** @verifying
```bash
cd agent && cargo test test_install
```

**Step 4: Commit**
```bash
git add -A && git commit -m "feat: sannai install command with launchd/systemd support"
```

---

### Task 10: Uninstall command @tdd

**Files:**
- Modify: `agent/src/service/mod.rs`
- Modify: `agent/src/main.rs`

**Step 1: Write failing test** (RED)
```rust
#[test]
fn test_uninstall_removes_service_file() {
    let dir = tempfile::tempdir().unwrap();
    let service_path = dir.path().join("test.plist");
    let bin_path = "/usr/local/bin/sannai";

    install_service_to(Platform::MacOS, bin_path, &service_path).unwrap();
    assert!(service_path.exists());

    uninstall_service_from(&service_path).unwrap();
    assert!(!service_path.exists());
}

#[test]
fn test_uninstall_noop_if_not_installed() {
    let dir = tempfile::tempdir().unwrap();
    let service_path = dir.path().join("nonexistent.plist");

    let result = uninstall_service_from(&service_path);
    assert!(result.is_ok()); // no error, just prints message
}

#[test]
fn test_purge_removes_data_dir() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::write(data_dir.join("store.db"), "test").unwrap();

    purge_data_dir(&data_dir).unwrap();
    assert!(!data_dir.exists());
}
```

**Step 2: Implement**

```rust
/// Remove service file (testable).
pub fn uninstall_service_from(path: &PathBuf) -> Result<()> {
    if !path.exists() {
        println!("No service installed.");
        return Ok(());
    }
    std::fs::remove_file(path)?;
    Ok(())
}

/// Remove the data directory.
pub fn purge_data_dir(data_dir: &PathBuf) -> Result<()> {
    if data_dir.exists() {
        std::fs::remove_dir_all(data_dir)?;
        println!("Removed data directory: {}", data_dir.display());
    }
    Ok(())
}

/// Uninstall service, optionally purge data.
pub fn uninstall_service(purge: bool) -> Result<()> {
    let platform = detect_platform();
    let path = service_file_path(platform);

    // Stop the service first
    if path.exists() {
        match platform {
            Platform::MacOS => {
                std::process::Command::new("launchctl")
                    .args(["unload", &path.to_string_lossy()])
                    .status()
                    .ok();
            }
            Platform::Linux => {
                std::process::Command::new("systemctl")
                    .args(["--user", "disable", "--now", "sannai"])
                    .status()
                    .ok();
            }
        }
    }

    uninstall_service_from(&path)?;

    if purge {
        let data_dir = crate::daemon::data_dir();
        purge_data_dir(&data_dir)?;
    }

    println!("Sannai service uninstalled.");
    Ok(())
}
```

Add `Uninstall { #[arg(long)] purge: bool }` variant to `Commands` in main.rs.

**Step 3: Run tests** @verifying
```bash
cd agent && cargo test test_uninstall test_purge
```

**Step 4: Commit**
```bash
git add -A && git commit -m "feat: sannai uninstall command with --purge option"
```

---

### Task 11: Enhanced status command @tdd

**Files:**
- Modify: `agent/src/service/mod.rs`
- Modify: `agent/src/main.rs`

**Step 1: Write failing test** (RED)
```rust
#[test]
fn test_is_service_installed() {
    let dir = tempfile::tempdir().unwrap();

    let not_installed = dir.path().join("nope.plist");
    assert!(!is_service_installed_at(&not_installed));

    let installed = dir.path().join("yes.plist");
    std::fs::write(&installed, "test").unwrap();
    assert!(is_service_installed_at(&installed));
}
```

**Step 2: Implement**

```rust
pub fn is_service_installed_at(path: &PathBuf) -> bool {
    path.exists()
}

pub fn is_service_installed() -> bool {
    let platform = detect_platform();
    is_service_installed_at(&service_file_path(platform))
}
```

Update `Commands::Status` handler in main.rs to show:
- Running status (PID)
- Service installed (yes/no)
- Data dir path
- Session count (from SQLite if DB exists)

**Step 3: Run tests** @verifying
```bash
cd agent && cargo test test_is_service && cargo test
```

**Step 4: Commit**
```bash
git add -A && git commit -m "feat: enhanced status output with service and data info"
```

---

## Part 3: Final Verification

### Task 12: Full build and test sweep @verifying

**Step 1: Run full suite**
```bash
cd agent && cargo fmt --check && cargo clippy -- -D warnings && cargo test
```

**Step 2: Verify cargo package**
```bash
cd agent && cargo package --list
```
Ensure no junk files are included.

**Step 3: Commit any fixes**

---

**Patterns to Store:**
- Service file generation pattern: platform-specific templates with testable `_to()` variants that accept explicit paths
- Purge pattern: destructive operations behind explicit flags, data preserved by default
