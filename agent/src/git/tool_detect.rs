#[derive(Debug, Clone, PartialEq)]
pub enum DetectedGitOp {
    Commit { amend: bool, message: Option<String> },
    Push { force: bool },
    Checkout { branch: Option<String> },
    Merge,
    Rebase,
    Stash { pop: bool },
    Reset { hard: bool },
}

/// Detect mutating git commands in a bash input string.
/// Returns None for read-only commands (status, log, diff, etc.) and non-git commands.
pub fn detect_git_command(input: &str) -> Option<DetectedGitOp> {
    // Find "git" followed by a subcommand in the input
    // Handle cases like "cd foo && git commit ..." by scanning for "git "
    let git_pos = input.find("git ")?;
    let after_git = &input[git_pos + 4..];
    let parts: Vec<&str> = after_git.split_whitespace().collect();
    let subcommand = parts.first()?;

    match *subcommand {
        "commit" => {
            let amend = parts.contains(&"--amend");
            let message = extract_message_flag(&parts);
            Some(DetectedGitOp::Commit { amend, message })
        }
        "push" => {
            let force = parts.iter().any(|&p| p == "--force" || p == "-f" || p == "--force-with-lease");
            Some(DetectedGitOp::Push { force })
        }
        "checkout" => {
            let branch = if let Some(pos) = parts.iter().position(|&p| p == "-b") {
                parts.get(pos + 1).map(|s| s.to_string())
            } else {
                parts.get(1).and_then(|s| {
                    if s.starts_with('-') { None } else { Some(s.to_string()) }
                })
            };
            Some(DetectedGitOp::Checkout { branch })
        }
        "rebase" => Some(DetectedGitOp::Rebase),
        "stash" => {
            let pop = parts.get(1).map(|&s| s == "pop" || s == "apply").unwrap_or(false);
            Some(DetectedGitOp::Stash { pop })
        }
        "reset" => {
            let hard = parts.contains(&"--hard");
            Some(DetectedGitOp::Reset { hard })
        }
        "merge" => Some(DetectedGitOp::Merge),
        // Read-only commands — ignore
        "status" | "log" | "diff" | "show" | "branch" | "remote" | "fetch" | "ls-files"
        | "rev-parse" | "describe" | "tag" | "blame" | "shortlog" | "reflog" => None,
        _ => None,
    }
}

fn extract_message_flag(parts: &[&str]) -> Option<String> {
    for (i, &part) in parts.iter().enumerate() {
        if part == "-m" {
            // Message is the next argument(s), possibly quoted
            if let Some(&msg) = parts.get(i + 1) {
                // Reconstruct message from remaining parts until we hit another flag
                let mut message_parts = vec![msg];
                for &p in parts.iter().skip(i + 2) {
                    if p.starts_with('-') {
                        break;
                    }
                    message_parts.push(p);
                }
                let full = message_parts.join(" ");
                // Strip surrounding quotes
                let trimmed = full.trim_matches('\'').trim_matches('"');
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_git_commit_in_bash() {
        assert_eq!(
            detect_git_command("git commit -m 'feat: add auth'"),
            Some(DetectedGitOp::Commit {
                amend: false,
                message: Some("feat: add auth".into())
            })
        );
        assert_eq!(
            detect_git_command("git commit --amend -m 'fix'"),
            Some(DetectedGitOp::Commit {
                amend: true,
                message: Some("fix".into())
            })
        );
        assert_eq!(
            detect_git_command("git push origin main"),
            Some(DetectedGitOp::Push { force: false })
        );
        assert_eq!(
            detect_git_command("git push --force"),
            Some(DetectedGitOp::Push { force: true })
        );
        assert_eq!(
            detect_git_command("git checkout -b feat/new"),
            Some(DetectedGitOp::Checkout {
                branch: Some("feat/new".into())
            })
        );
        assert_eq!(
            detect_git_command("git rebase main"),
            Some(DetectedGitOp::Rebase)
        );
        assert_eq!(
            detect_git_command("git stash"),
            Some(DetectedGitOp::Stash { pop: false })
        );
        assert_eq!(
            detect_git_command("git stash pop"),
            Some(DetectedGitOp::Stash { pop: true })
        );
        assert_eq!(
            detect_git_command("git reset --hard HEAD~1"),
            Some(DetectedGitOp::Reset { hard: true })
        );
        assert_eq!(
            detect_git_command("git merge feature-branch"),
            Some(DetectedGitOp::Merge)
        );
        // Not a git command
        assert_eq!(detect_git_command("cargo build"), None);
        // Git read-only commands should be ignored
        assert_eq!(detect_git_command("git status"), None);
        assert_eq!(detect_git_command("git log"), None);
        assert_eq!(detect_git_command("git diff"), None);
    }

    #[test]
    fn test_extract_message_stops_at_flag() {
        // Message extraction should stop at subsequent flags
        assert_eq!(
            detect_git_command("git commit -m 'initial' --no-verify"),
            Some(DetectedGitOp::Commit {
                amend: false,
                message: Some("initial".into())
            })
        );
    }
}
