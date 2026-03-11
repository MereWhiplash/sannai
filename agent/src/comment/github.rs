use std::process::Command;

use anyhow::{Context, Result};

/// Get commit SHAs for a PR using the `gh` CLI.
pub fn get_pr_commits(pr_url: &str) -> Result<Vec<String>> {
    let (owner_repo, pr_number) = parse_pr_url(pr_url)?;

    let output = Command::new("gh")
        .args([
            "pr",
            "view",
            &pr_number,
            "--repo",
            &owner_repo,
            "--json",
            "commits",
            "--jq",
            ".commits[].oid",
        ])
        .output()
        .context("Failed to run `gh` CLI. Is it installed?")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("gh pr view failed: {}", stderr);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let shas: Vec<String> = stdout
        .lines()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    Ok(shas)
}

/// Get the diff for a PR.
pub fn get_pr_diff(pr_url: &str) -> Result<String> {
    let (owner_repo, pr_number) = parse_pr_url(pr_url)?;

    let output = Command::new("gh")
        .args(["pr", "diff", &pr_number, "--repo", &owner_repo])
        .output()
        .context("Failed to run `gh` CLI")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("gh pr diff failed: {}", stderr);
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Post or update a Sannai comment on a PR.
pub fn post_pr_comment(pr_url: &str, body: &str) -> Result<()> {
    let (owner_repo, pr_number) = parse_pr_url(pr_url)?;

    // Check for existing Sannai comment to update
    let existing_id = find_existing_comment(&owner_repo, &pr_number)?;

    if let Some(comment_id) = existing_id {
        let output = Command::new("gh")
            .args([
                "api",
                &format!("repos/{}/issues/comments/{}", owner_repo, comment_id),
                "--method",
                "PATCH",
                "--field",
                &format!("body={}", body),
            ])
            .output()
            .context("Failed to update PR comment")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Failed to update comment: {}", stderr);
        }

        tracing::info!("Updated existing Sannai comment on PR #{}", pr_number);
    } else {
        let output = Command::new("gh")
            .args([
                "pr",
                "comment",
                &pr_number,
                "--repo",
                &owner_repo,
                "--body",
                body,
            ])
            .output()
            .context("Failed to post PR comment")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Failed to post comment: {}", stderr);
        }

        tracing::info!("Posted Sannai comment on PR #{}", pr_number);
    }

    Ok(())
}

fn find_existing_comment(owner_repo: &str, pr_number: &str) -> Result<Option<String>> {
    let output = Command::new("gh")
        .args([
            "api",
            &format!("repos/{}/issues/{}/comments", owner_repo, pr_number),
            "--jq",
            r#".[] | select(.body | contains("AI Process Audit")) | .id"#,
        ])
        .output()?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let id = stdout
            .lines()
            .next()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        Ok(id)
    } else {
        Ok(None)
    }
}

fn parse_pr_url(url: &str) -> Result<(String, String)> {
    let url = url.trim();

    // https://github.com/owner/repo/pull/123
    if url.contains("github.com") {
        let parts: Vec<&str> = url.split('/').collect();
        let len = parts.len();
        if len >= 5 {
            let owner = parts[len - 4];
            let repo = parts[len - 3];
            let pr_number = parts[len - 1];
            return Ok((format!("{}/{}", owner, repo), pr_number.to_string()));
        }
    }

    // owner/repo#123
    if let Some((repo, number)) = url.split_once('#') {
        return Ok((repo.to_string(), number.to_string()));
    }

    anyhow::bail!(
        "Could not parse PR URL: {}. Expected https://github.com/owner/repo/pull/N or owner/repo#N",
        url
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_pr_url_full() {
        let (repo, number) =
            parse_pr_url("https://github.com/AaronFR/sannai/pull/42").unwrap();
        assert_eq!(repo, "AaronFR/sannai");
        assert_eq!(number, "42");
    }

    #[test]
    fn test_parse_pr_url_short() {
        let (repo, number) = parse_pr_url("AaronFR/sannai#42").unwrap();
        assert_eq!(repo, "AaronFR/sannai");
        assert_eq!(number, "42");
    }

    #[test]
    fn test_parse_pr_url_invalid() {
        assert!(parse_pr_url("not-a-url").is_err());
    }
}
