use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

const HOOK_MARKER: &str = "# SANNAI-MANAGED-HOOK";

const LINK_COMMIT_SCRIPT: &str = include_str!("../../../hooks/link-commit.sh");
const POST_PUSH_SCRIPT: &str = include_str!("../../../hooks/post-push-comment.sh");

/// Where hooks live relative to a repo root.
pub struct HookPaths {
    pub repo_root: PathBuf,
    pub pre_push: PathBuf,
    pub link_commit: PathBuf,
    pub claude_settings: PathBuf,
}

impl HookPaths {
    pub fn new(repo_root: &Path) -> Self {
        Self {
            repo_root: repo_root.to_path_buf(),
            pre_push: repo_root.join(".git/hooks/pre-push"),
            link_commit: repo_root.join(".sannai/hooks/link-commit.sh"),
            claude_settings: repo_root.join(".claude/settings.json"),
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum HookState {
    Installed,
    NotInstalled,
    ExternalExists,
}

pub struct HookStatusReport {
    pub pre_push: HookState,
    pub link_commit: HookState,
    pub claude_settings: bool,
}

// --- Status ---

pub fn hook_status_at(paths: &HookPaths) -> HookStatusReport {
    HookStatusReport {
        pre_push: check_hook_state(&paths.pre_push),
        link_commit: check_hook_state(&paths.link_commit),
        claude_settings: check_claude_settings(&paths.claude_settings),
    }
}

fn check_hook_state(path: &Path) -> HookState {
    match std::fs::read_to_string(path) {
        Ok(content) => {
            if content.contains(HOOK_MARKER) {
                HookState::Installed
            } else {
                HookState::ExternalExists
            }
        }
        Err(_) => HookState::NotInstalled,
    }
}

fn check_claude_settings(path: &Path) -> bool {
    match std::fs::read_to_string(path) {
        Ok(content) => has_sannai_hook_entry(&content),
        Err(_) => false,
    }
}

fn has_sannai_hook_entry(json_str: &str) -> bool {
    let Ok(root) = serde_json::from_str::<serde_json::Value>(json_str) else {
        return false;
    };
    root.get("hooks")
        .and_then(|h| h.get("PostToolUse"))
        .and_then(|ptu| ptu.as_array())
        .map(|arr| {
            arr.iter().any(|entry| {
                entry
                    .get("hooks")
                    .and_then(|h| h.as_array())
                    .map(|hooks| {
                        hooks.iter().any(|h| {
                            h.get("command")
                                .and_then(|c| c.as_str())
                                .map(|c| c.contains("link-commit") || c.contains("sannai"))
                                .unwrap_or(false)
                        })
                    })
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

// --- Install ---

pub fn install_hooks(repo_root: &Path, force: bool) -> Result<()> {
    if !repo_root.join(".git").exists() {
        bail!("Not a git repository: {}", repo_root.display());
    }

    let bin_path = std::env::current_exe()?.to_string_lossy().to_string();
    let paths = HookPaths::new(repo_root);

    install_hooks_to(&paths, &bin_path, force)
}

pub fn install_hooks_to(paths: &HookPaths, sannai_bin: &str, force: bool) -> Result<()> {
    println!("Installing sannai hooks into {}...\n", paths.repo_root.display());

    let mut ok_count = 0;

    // 1. Git pre-push hook
    match install_pre_push(paths, sannai_bin, force) {
        Ok(true) => {
            println!("  [OK] Git pre-push hook: .git/hooks/pre-push");
            ok_count += 1;
        }
        Ok(false) => {
            println!(
                "  [SKIP] Git pre-push hook: .git/hooks/pre-push already exists (not sannai-managed)"
            );
            println!("         Use --force to overwrite");
        }
        Err(e) => println!("  [ERR] Git pre-push hook: {}", e),
    }

    // 2. Link-commit script
    match install_link_commit(paths) {
        Ok(()) => {
            println!("  [OK] Commit linker: .sannai/hooks/link-commit.sh");
            ok_count += 1;
        }
        Err(e) => println!("  [ERR] Commit linker: {}", e),
    }

    // 3. Claude Code settings
    match install_claude_settings(paths) {
        Ok(()) => {
            println!("  [OK] Claude Code settings: .claude/settings.json");
            ok_count += 1;
        }
        Err(e) => println!("  [ERR] Claude Code settings: {}", e),
    }

    println!();
    if ok_count == 3 {
        println!("All hooks installed.");
    } else {
        println!("{}/3 hooks installed.", ok_count);
    }
    println!();
    println!("Next steps:");
    println!("  - Commit .claude/settings.json and .sannai/ to share with your team");
    println!("  - The pre-push hook is local to this clone (not committed)");
    println!("  - Make sure the sannai daemon is running: sannai start");

    Ok(())
}

/// Install the pre-push git hook. Returns Ok(true) if installed, Ok(false) if skipped.
fn install_pre_push(paths: &HookPaths, sannai_bin: &str, force: bool) -> Result<bool> {
    if let Some(parent) = paths.pre_push.parent() {
        std::fs::create_dir_all(parent)?;
    }

    if paths.pre_push.exists() {
        let existing = std::fs::read_to_string(&paths.pre_push)?;
        if !existing.contains(HOOK_MARKER) && !force {
            return Ok(false);
        }
    }

    let script = generate_pre_push_script(sannai_bin);
    std::fs::write(&paths.pre_push, &script)?;
    set_executable(&paths.pre_push)?;
    Ok(true)
}

fn generate_pre_push_script(sannai_bin: &str) -> String {
    let script: String = POST_PUSH_SCRIPT
        .lines()
        .map(|line| {
            if line.starts_with("SANNAI_BIN=") {
                format!("SANNAI_BIN=\"${{SANNAI_BIN:-{}}}\"", sannai_bin)
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("{}\n{}\n", HOOK_MARKER, script)
}

fn install_link_commit(paths: &HookPaths) -> Result<()> {
    if let Some(parent) = paths.link_commit.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let script = format!("{}\n{}", HOOK_MARKER, LINK_COMMIT_SCRIPT);
    std::fs::write(&paths.link_commit, &script)?;
    set_executable(&paths.link_commit)?;
    Ok(())
}

fn install_claude_settings(paths: &HookPaths) -> Result<()> {
    let hook_command = ".sannai/hooks/link-commit.sh";

    let existing = std::fs::read_to_string(&paths.claude_settings).ok();
    let updated = merge_claude_settings(existing.as_deref(), hook_command)?;

    if let Some(parent) = paths.claude_settings.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&paths.claude_settings, &updated)?;
    Ok(())
}

fn merge_claude_settings(existing: Option<&str>, hook_command: &str) -> Result<String> {
    let mut root: serde_json::Value = match existing {
        Some(s) if !s.trim().is_empty() => {
            serde_json::from_str(s).context("Failed to parse .claude/settings.json")?
        }
        _ => serde_json::json!({}),
    };

    let hooks =
        root.as_object_mut().unwrap().entry("hooks").or_insert_with(|| serde_json::json!({}));

    let post_tool_use = hooks
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("hooks is not an object in .claude/settings.json"))?
        .entry("PostToolUse")
        .or_insert_with(|| serde_json::json!([]));

    let arr = post_tool_use
        .as_array_mut()
        .ok_or_else(|| anyhow::anyhow!("PostToolUse is not an array in .claude/settings.json"))?;

    // Check for existing sannai entry
    let already_present = has_sannai_hook_in_array(arr);

    if !already_present {
        arr.push(serde_json::json!({
            "matcher": "Bash",
            "hooks": [{
                "type": "command",
                "command": hook_command,
                "timeout": 10
            }]
        }));
    }

    let mut buf = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b"  ");
    let mut serializer = serde_json::Serializer::with_formatter(&mut buf, formatter);
    root.serialize(&mut serializer)?;
    let mut json = String::from_utf8(buf)?;
    json.push('\n');
    Ok(json)
}

fn has_sannai_hook_in_array(arr: &[serde_json::Value]) -> bool {
    arr.iter().any(|entry| {
        entry
            .get("hooks")
            .and_then(|h| h.as_array())
            .map(|hooks| {
                hooks.iter().any(|h| {
                    h.get("command")
                        .and_then(|c| c.as_str())
                        .map(|c| c.contains("link-commit") || c.contains("sannai"))
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false)
    })
}

// --- Uninstall ---

pub fn uninstall_hooks(repo_root: &Path) -> Result<()> {
    if !repo_root.join(".git").exists() {
        bail!("Not a git repository: {}", repo_root.display());
    }
    let paths = HookPaths::new(repo_root);
    uninstall_hooks_from(&paths)
}

pub fn uninstall_hooks_from(paths: &HookPaths) -> Result<()> {
    println!("Removing sannai hooks from {}...\n", paths.repo_root.display());

    // 1. Pre-push hook
    if paths.pre_push.exists() {
        let content = std::fs::read_to_string(&paths.pre_push)?;
        if content.contains(HOOK_MARKER) {
            std::fs::remove_file(&paths.pre_push)?;
            println!("  [OK] Removed git pre-push hook");
        } else {
            println!("  [SKIP] .git/hooks/pre-push exists but is not sannai-managed");
        }
    } else {
        println!("  [--] No pre-push hook found");
    }

    // 2. Link-commit script
    if paths.link_commit.exists() {
        std::fs::remove_file(&paths.link_commit)?;
        // Clean up empty dirs
        if let Some(hooks_dir) = paths.link_commit.parent() {
            let _ = std::fs::remove_dir(hooks_dir); // only removes if empty
            if let Some(sannai_dir) = hooks_dir.parent() {
                let _ = std::fs::remove_dir(sannai_dir);
            }
        }
        println!("  [OK] Removed commit linker");
    } else {
        println!("  [--] No commit linker found");
    }

    // 3. Claude settings
    match remove_from_claude_settings(paths) {
        Ok(true) => println!("  [OK] Removed sannai entry from Claude Code settings"),
        Ok(false) => println!("  [--] No sannai entry in Claude Code settings"),
        Err(e) => println!("  [ERR] Claude Code settings: {}", e),
    }

    println!("\nSannai hooks removed.");
    Ok(())
}

fn remove_from_claude_settings(paths: &HookPaths) -> Result<bool> {
    let content = match std::fs::read_to_string(&paths.claude_settings) {
        Ok(c) => c,
        Err(_) => return Ok(false),
    };

    let mut root: serde_json::Value = serde_json::from_str(&content)?;

    let removed = if let Some(hooks) = root.get_mut("hooks").and_then(|h| h.as_object_mut()) {
        if let Some(ptu) = hooks.get_mut("PostToolUse").and_then(|a| a.as_array_mut()) {
            let before = ptu.len();
            ptu.retain(|entry| {
                !entry
                    .get("hooks")
                    .and_then(|h| h.as_array())
                    .map(|hooks| {
                        hooks.iter().any(|h| {
                            h.get("command")
                                .and_then(|c| c.as_str())
                                .map(|c| c.contains("link-commit") || c.contains("sannai"))
                                .unwrap_or(false)
                        })
                    })
                    .unwrap_or(false)
            });
            let removed = ptu.len() < before;

            // Clean up empty PostToolUse array
            if ptu.is_empty() {
                hooks.remove("PostToolUse");
            }
            // Clean up empty hooks object
            if hooks.is_empty() {
                root.as_object_mut().unwrap().remove("hooks");
            }
            removed
        } else {
            false
        }
    } else {
        false
    };

    if removed {
        let mut buf = Vec::new();
        let formatter = serde_json::ser::PrettyFormatter::with_indent(b"  ");
        let mut serializer = serde_json::Serializer::with_formatter(&mut buf, formatter);
        root.serialize(&mut serializer)?;
        let mut json = String::from_utf8(buf)?;
        json.push('\n');
        std::fs::write(&paths.claude_settings, json)?;
    }

    Ok(removed)
}

// --- Print ---

pub fn print_hook_status(repo_root: &Path) -> Result<()> {
    if !repo_root.join(".git").exists() {
        bail!("Not a git repository: {}", repo_root.display());
    }
    let paths = HookPaths::new(repo_root);
    let status = hook_status_at(&paths);

    println!("Hook status for {}:\n", repo_root.display());
    println!(
        "  Git pre-push hook:     {}",
        match status.pre_push {
            HookState::Installed => "installed (sannai-managed)",
            HookState::NotInstalled => "not installed",
            HookState::ExternalExists => "external hook present (not sannai)",
        }
    );
    println!(
        "  Commit linker:         {}",
        match status.link_commit {
            HookState::Installed => "installed",
            HookState::NotInstalled => "not installed",
            HookState::ExternalExists => "external script present (not sannai)",
        }
    );
    println!(
        "  Claude Code settings:  {}",
        if status.claude_settings { "configured" } else { "not configured" }
    );

    Ok(())
}

// --- Helpers ---

use serde::Serialize;

#[cfg(unix)]
fn set_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms)?;
    Ok(())
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_fake_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".git/hooks")).unwrap();
        dir
    }

    #[test]
    fn test_generate_pre_push_injects_binary_path() {
        let script = generate_pre_push_script("/usr/local/bin/sannai");
        assert!(script.contains(HOOK_MARKER));
        assert!(script.contains("/usr/local/bin/sannai"));
        assert!(!script.contains("__SANNAI_BIN__"));
    }

    #[test]
    fn test_install_creates_pre_push_hook() {
        let dir = setup_fake_repo();
        let paths = HookPaths::new(dir.path());

        let result = install_pre_push(&paths, "/usr/bin/sannai", false).unwrap();
        assert!(result);
        assert!(paths.pre_push.exists());

        let content = std::fs::read_to_string(&paths.pre_push).unwrap();
        assert!(content.contains(HOOK_MARKER));
        assert!(content.contains("/usr/bin/sannai"));
    }

    #[test]
    fn test_install_skips_external_hook() {
        let dir = setup_fake_repo();
        let paths = HookPaths::new(dir.path());

        std::fs::write(&paths.pre_push, "#!/bin/bash\necho 'my hook'").unwrap();

        let result = install_pre_push(&paths, "/usr/bin/sannai", false).unwrap();
        assert!(!result); // skipped

        let content = std::fs::read_to_string(&paths.pre_push).unwrap();
        assert!(content.contains("my hook")); // unchanged
    }

    #[test]
    fn test_install_force_overwrites_external() {
        let dir = setup_fake_repo();
        let paths = HookPaths::new(dir.path());

        std::fs::write(&paths.pre_push, "#!/bin/bash\necho 'my hook'").unwrap();

        let result = install_pre_push(&paths, "/usr/bin/sannai", true).unwrap();
        assert!(result);

        let content = std::fs::read_to_string(&paths.pre_push).unwrap();
        assert!(content.contains(HOOK_MARKER));
    }

    #[test]
    fn test_install_updates_existing_sannai_hook() {
        let dir = setup_fake_repo();
        let paths = HookPaths::new(dir.path());

        // Install with old path
        install_pre_push(&paths, "/old/path/sannai", false).unwrap();
        // Update with new path
        let result = install_pre_push(&paths, "/new/path/sannai", false).unwrap();
        assert!(result);

        let content = std::fs::read_to_string(&paths.pre_push).unwrap();
        assert!(content.contains("/new/path/sannai"));
        assert!(!content.contains("/old/path/sannai"));
    }

    #[test]
    fn test_install_creates_link_commit() {
        let dir = setup_fake_repo();
        let paths = HookPaths::new(dir.path());

        install_link_commit(&paths).unwrap();
        assert!(paths.link_commit.exists());

        let content = std::fs::read_to_string(&paths.link_commit).unwrap();
        assert!(content.contains(HOOK_MARKER));
        assert!(content.contains("hook/commit"));
    }

    #[test]
    fn test_merge_claude_settings_empty() {
        let result = merge_claude_settings(None, ".sannai/hooks/link-commit.sh").unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();

        let hooks = &parsed["hooks"]["PostToolUse"];
        assert!(hooks.is_array());
        assert_eq!(hooks.as_array().unwrap().len(), 1);

        let cmd = hooks[0]["hooks"][0]["command"].as_str().unwrap();
        assert!(cmd.contains("link-commit"));
    }

    #[test]
    fn test_merge_claude_settings_existing_hooks() {
        let existing = r#"{
  "hooks": {
    "SessionStart": [{"matcher": "startup", "hooks": [{"type": "command", "command": "echo hi"}]}]
  }
}"#;

        let result = merge_claude_settings(Some(existing), ".sannai/hooks/link-commit.sh").unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();

        // Existing hook preserved
        assert!(parsed["hooks"]["SessionStart"].is_array());
        // New hook added
        assert!(parsed["hooks"]["PostToolUse"].is_array());
    }

    #[test]
    fn test_merge_claude_settings_existing_post_tool_use() {
        let existing = r#"{
  "hooks": {
    "PostToolUse": [{"matcher": "Write", "hooks": [{"type": "command", "command": "echo write"}]}]
  }
}"#;

        let result = merge_claude_settings(Some(existing), ".sannai/hooks/link-commit.sh").unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();

        let ptu = parsed["hooks"]["PostToolUse"].as_array().unwrap();
        assert_eq!(ptu.len(), 2); // existing + sannai
    }

    #[test]
    fn test_merge_claude_settings_idempotent() {
        let result1 = merge_claude_settings(None, ".sannai/hooks/link-commit.sh").unwrap();
        let result2 =
            merge_claude_settings(Some(&result1), ".sannai/hooks/link-commit.sh").unwrap();

        let parsed: serde_json::Value = serde_json::from_str(&result2).unwrap();
        let ptu = parsed["hooks"]["PostToolUse"].as_array().unwrap();
        assert_eq!(ptu.len(), 1); // not duplicated
    }

    #[test]
    fn test_status_all_not_installed() {
        let dir = setup_fake_repo();
        let paths = HookPaths::new(dir.path());
        let status = hook_status_at(&paths);

        assert_eq!(status.pre_push, HookState::NotInstalled);
        assert_eq!(status.link_commit, HookState::NotInstalled);
        assert!(!status.claude_settings);
    }

    #[test]
    fn test_status_all_installed() {
        let dir = setup_fake_repo();
        let paths = HookPaths::new(dir.path());

        install_hooks_to(&paths, "/usr/bin/sannai", false).unwrap();
        let status = hook_status_at(&paths);

        assert_eq!(status.pre_push, HookState::Installed);
        assert_eq!(status.link_commit, HookState::Installed);
        assert!(status.claude_settings);
    }

    #[test]
    fn test_status_external_hook() {
        let dir = setup_fake_repo();
        let paths = HookPaths::new(dir.path());

        std::fs::write(&paths.pre_push, "#!/bin/bash\necho external").unwrap();
        let status = hook_status_at(&paths);

        assert_eq!(status.pre_push, HookState::ExternalExists);
    }

    #[test]
    fn test_uninstall_removes_sannai_hooks() {
        let dir = setup_fake_repo();
        let paths = HookPaths::new(dir.path());

        install_hooks_to(&paths, "/usr/bin/sannai", false).unwrap();
        assert!(paths.pre_push.exists());
        assert!(paths.link_commit.exists());

        uninstall_hooks_from(&paths).unwrap();
        assert!(!paths.pre_push.exists());
        assert!(!paths.link_commit.exists());
    }

    #[test]
    fn test_uninstall_preserves_external_hooks() {
        let dir = setup_fake_repo();
        let paths = HookPaths::new(dir.path());

        std::fs::write(&paths.pre_push, "#!/bin/bash\necho external").unwrap();
        uninstall_hooks_from(&paths).unwrap();

        // External hook preserved
        assert!(paths.pre_push.exists());
        let content = std::fs::read_to_string(&paths.pre_push).unwrap();
        assert!(content.contains("external"));
    }

    #[test]
    fn test_uninstall_cleans_claude_settings() {
        let dir = setup_fake_repo();
        let paths = HookPaths::new(dir.path());

        install_hooks_to(&paths, "/usr/bin/sannai", false).unwrap();
        assert!(check_claude_settings(&paths.claude_settings));

        uninstall_hooks_from(&paths).unwrap();
        assert!(!check_claude_settings(&paths.claude_settings));
    }

    #[test]
    fn test_uninstall_preserves_other_settings() {
        let dir = setup_fake_repo();
        let paths = HookPaths::new(dir.path());

        // Settings with both sannai and other hooks
        let settings = r#"{
  "hooks": {
    "PostToolUse": [
      {"matcher": "Write", "hooks": [{"type": "command", "command": "echo write"}]},
      {"matcher": "Bash", "hooks": [{"type": "command", "command": ".sannai/hooks/link-commit.sh"}]}
    ],
    "SessionStart": [{"matcher": "startup", "hooks": [{"type": "command", "command": "echo hi"}]}]
  }
}"#;
        std::fs::create_dir_all(paths.claude_settings.parent().unwrap()).unwrap();
        std::fs::write(&paths.claude_settings, settings).unwrap();

        remove_from_claude_settings(&paths).unwrap();

        let content = std::fs::read_to_string(&paths.claude_settings).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();

        // Sannai entry removed, other preserved
        let ptu = parsed["hooks"]["PostToolUse"].as_array().unwrap();
        assert_eq!(ptu.len(), 1);
        assert!(ptu[0]["matcher"].as_str().unwrap() == "Write");

        // SessionStart untouched
        assert!(parsed["hooks"]["SessionStart"].is_array());
    }
}
