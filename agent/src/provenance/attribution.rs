use serde::Serialize;
use std::process::Command;

use super::interaction::Interaction;

#[derive(Debug, Clone, Serialize)]
pub struct DiffAttribution {
    pub commit_sha: String,
    pub file_path: String,
    pub hunk_start: u32,
    pub hunk_end: u32,
    pub interaction_id: Option<String>,
    pub confidence: f32,
    pub attribution_type: AttributionType,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub enum AttributionType {
    AiGenerated,
    AiAssisted,
    Manual,
    Unknown,
}

impl std::fmt::Display for AttributionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AiGenerated => write!(f, "AI-generated"),
            Self::AiAssisted => write!(f, "AI-assisted"),
            Self::Manual => write!(f, "Manual"),
            Self::Unknown => write!(f, "Unknown"),
        }
    }
}

#[derive(Debug)]
struct DiffHunk {
    file_path: String,
    new_start: u32,
    new_count: u32,
    added_lines: Vec<String>,
}

/// Attribute diff hunks to interactions for a single commit.
pub fn attribute_diff(
    repo_path: &str,
    commit_sha: &str,
    interactions: &[Interaction],
) -> Vec<DiffAttribution> {
    let hunks = match parse_commit_diff(repo_path, commit_sha) {
        Ok(h) => h,
        Err(e) => {
            tracing::warn!("Failed to get diff for {}: {}", commit_sha, e);
            return Vec::new();
        }
    };

    hunks
        .iter()
        .map(|hunk| {
            let (interaction_id, confidence, attribution_type) =
                match_hunk_to_interaction(hunk, interactions);
            DiffAttribution {
                commit_sha: commit_sha.to_string(),
                file_path: hunk.file_path.clone(),
                hunk_start: hunk.new_start,
                hunk_end: hunk.new_start + hunk.new_count.saturating_sub(1),
                interaction_id,
                confidence,
                attribution_type,
            }
        })
        .collect()
}

/// Attribute hunks from a pre-computed diff string (e.g., the full PR diff).
pub fn attribute_diff_text(diff_text: &str, interactions: &[Interaction]) -> Vec<DiffAttribution> {
    let hunks = match parse_unified_diff(diff_text) {
        Ok(h) => h,
        Err(e) => {
            tracing::warn!("Failed to parse diff: {}", e);
            return Vec::new();
        }
    };

    hunks
        .iter()
        .map(|hunk| {
            let (interaction_id, confidence, attribution_type) =
                match_hunk_to_interaction(hunk, interactions);
            DiffAttribution {
                commit_sha: String::new(),
                file_path: hunk.file_path.clone(),
                hunk_start: hunk.new_start,
                hunk_end: hunk.new_start + hunk.new_count.saturating_sub(1),
                interaction_id,
                confidence,
                attribution_type,
            }
        })
        .collect()
}

fn parse_commit_diff(repo_path: &str, commit_sha: &str) -> anyhow::Result<Vec<DiffHunk>> {
    let output = Command::new("git")
        .args(["diff", &format!("{}~1", commit_sha), commit_sha, "--unified=0"])
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        // Might be the first commit — try diff against empty tree
        let output = Command::new("git")
            .args(["diff", "4b825dc642cb6eb9a060e54bf899d69f82534100", commit_sha, "--unified=0"])
            .current_dir(repo_path)
            .output()?;

        if !output.status.success() {
            anyhow::bail!("git diff failed: {}", String::from_utf8_lossy(&output.stderr));
        }
        return parse_unified_diff(&String::from_utf8_lossy(&output.stdout));
    }

    parse_unified_diff(&String::from_utf8_lossy(&output.stdout))
}

fn parse_unified_diff(diff_text: &str) -> anyhow::Result<Vec<DiffHunk>> {
    let mut hunks = Vec::new();
    let mut current_file: Option<String> = None;
    let mut current_hunk: Option<DiffHunk> = None;

    for line in diff_text.lines() {
        if let Some(path) = line.strip_prefix("+++ b/") {
            // Save previous hunk
            if let Some(hunk) = current_hunk.take() {
                if !hunk.added_lines.is_empty() {
                    hunks.push(hunk);
                }
            }
            current_file = Some(path.to_string());
        } else if line.starts_with("+++ /dev/null") {
            current_file = None;
        } else if line.starts_with("@@ ") {
            // Save previous hunk
            if let Some(hunk) = current_hunk.take() {
                if !hunk.added_lines.is_empty() {
                    hunks.push(hunk);
                }
            }

            if let Some(file) = &current_file {
                if let Some((start, count)) = parse_hunk_header(line) {
                    current_hunk = Some(DiffHunk {
                        file_path: file.clone(),
                        new_start: start,
                        new_count: count,
                        added_lines: Vec::new(),
                    });
                }
            }
        } else if line.starts_with('+') && !line.starts_with("+++") {
            if let Some(hunk) = &mut current_hunk {
                hunk.added_lines.push(line[1..].to_string());
            }
        }
    }

    // Don't forget the last hunk
    if let Some(hunk) = current_hunk {
        if !hunk.added_lines.is_empty() {
            hunks.push(hunk);
        }
    }

    Ok(hunks)
}

fn parse_hunk_header(line: &str) -> Option<(u32, u32)> {
    // @@ -old_start,old_count +new_start,new_count @@
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 3 {
        return None;
    }

    let new_range = parts[2].trim_start_matches('+');
    let (start, count) = if let Some((s, c)) = new_range.split_once(',') {
        (s.parse().ok()?, c.parse().ok()?)
    } else {
        (new_range.parse().ok()?, 1u32)
    };

    Some((start, count))
}

fn match_hunk_to_interaction(
    hunk: &DiffHunk,
    interactions: &[Interaction],
) -> (Option<String>, f32, AttributionType) {
    if hunk.added_lines.is_empty() {
        return (None, 0.0, AttributionType::Unknown);
    }

    let added_text = hunk.added_lines.join("\n");
    let mut best_match: Option<(String, f32)> = None;

    for interaction in interactions {
        for tc in &interaction.tool_calls {
            let name = tc.tool_name.to_lowercase();
            let is_write = matches!(
                name.as_str(),
                "write" | "write_file" | "edit" | "str_replace" | "str_replace_editor"
            );
            if !is_write {
                continue;
            }

            // Check if this tool call targets the same file
            let tc_path = tc
                .input
                .get("file_path")
                .or_else(|| tc.input.get("path"))
                .and_then(|v| v.as_str())
                .unwrap_or("");

            if !paths_match(tc_path, &hunk.file_path) {
                continue;
            }

            let written = get_written_content(tc);
            if written.is_empty() {
                continue;
            }

            let similarity = compute_similarity(&added_text, &written);
            if let Some((_, best_sim)) = &best_match {
                if similarity > *best_sim {
                    best_match = Some((interaction.id.clone(), similarity));
                }
            } else if similarity > 0.1 {
                best_match = Some((interaction.id.clone(), similarity));
            }
        }
    }

    match best_match {
        Some((id, conf)) if conf >= 0.7 => (Some(id), conf, AttributionType::AiGenerated),
        Some((id, conf)) if conf >= 0.3 => (Some(id), conf, AttributionType::AiAssisted),
        Some((id, conf)) => (Some(id), conf, AttributionType::Manual),
        None => (None, 0.0, AttributionType::Manual),
    }
}

fn paths_match(tc_path: &str, diff_path: &str) -> bool {
    if tc_path.is_empty() || diff_path.is_empty() {
        return false;
    }
    tc_path.ends_with(diff_path)
        || diff_path.ends_with(tc_path)
        || tc_path.ends_with(&format!("/{}", diff_path))
}

fn get_written_content(tc: &super::interaction::ToolCall) -> String {
    let name = tc.tool_name.to_lowercase();
    match name.as_str() {
        "write" | "write_file" => {
            tc.input.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string()
        }
        "edit" | "str_replace" | "str_replace_editor" => tc
            .input
            .get("new_string")
            .or_else(|| tc.input.get("new_str"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        _ => String::new(),
    }
}

/// Simple line-based similarity: fraction of diff lines found in written content.
fn compute_similarity(diff_text: &str, written_text: &str) -> f32 {
    let diff_lines: Vec<&str> =
        diff_text.lines().map(|l| l.trim()).filter(|l| !l.is_empty()).collect();

    if diff_lines.is_empty() {
        return 0.0;
    }

    let matched = diff_lines.iter().filter(|line| written_text.contains(*line)).count();

    matched as f32 / diff_lines.len() as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hunk_header() {
        assert_eq!(parse_hunk_header("@@ -0,0 +1,25 @@"), Some((1, 25)));
        assert_eq!(parse_hunk_header("@@ -10,5 +12 @@"), Some((12, 1)));
        assert_eq!(parse_hunk_header("@@ -1,3 +1,4 @@ fn main()"), Some((1, 4)));
    }

    #[test]
    fn test_parse_unified_diff() {
        let diff = "\
diff --git a/src/main.rs b/src/main.rs
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,3 +1,5 @@
 fn main() {
+    println!(\"hello\");
+    println!(\"world\");
 }
";
        let hunks = parse_unified_diff(diff).unwrap();
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].file_path, "src/main.rs");
        assert_eq!(hunks[0].new_start, 1);
        assert_eq!(hunks[0].new_count, 5);
        assert_eq!(hunks[0].added_lines.len(), 2);
    }

    #[test]
    fn test_compute_similarity_exact() {
        let diff = "line one\nline two\nline three";
        let written = "line one\nline two\nline three\nline four";
        assert!((compute_similarity(diff, written) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_compute_similarity_partial() {
        let diff = "line one\nline two\nline three\nline four";
        let written = "line one\nline three";
        let sim = compute_similarity(diff, written);
        assert!((sim - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn test_compute_similarity_none() {
        let diff = "completely different";
        let written = "nothing matches here";
        let sim = compute_similarity(diff, written);
        assert!(sim < 0.01);
    }

    #[test]
    fn test_paths_match() {
        assert!(paths_match("/Users/dev/project/src/main.rs", "src/main.rs"));
        assert!(paths_match("src/main.rs", "src/main.rs"));
        assert!(!paths_match("src/lib.rs", "src/main.rs"));
        assert!(!paths_match("", "src/main.rs"));
    }
}
