use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::Result;
use serde::Serialize;

use super::attribution::DiffAttribution;
use super::interaction::Interaction;
use super::lineage::FileLineage;

#[derive(Debug, Clone)]
pub struct SummaryConfig {
    pub enabled: bool,
    pub command: String,
    pub max_length: usize,
}

impl Default for SummaryConfig {
    fn default() -> Self {
        Self { enabled: false, command: String::new(), max_length: 2000 }
    }
}

#[derive(Debug, Serialize)]
pub struct ProvenanceBundle {
    pub interactions: Vec<Interaction>,
    pub lineage: Vec<FileLineage>,
    pub attributions: Vec<DiffAttribution>,
    pub diff: String,
}

/// Build a structured prompt from provenance data instead of dumping raw JSON.
///
/// This produces a compact, human-readable summary of the session log that an
/// LLM can reason about without wading through tool call payloads and diffs.
fn build_prompt(bundle: &ProvenanceBundle) -> String {
    let mut prompt = String::new();

    prompt.push_str(
        "You are summarizing the AI-assisted development process behind a pull request \
         for a code reviewer. Below is structured provenance data extracted from the \
         developer's AI coding sessions.\n\n",
    );

    // --- Session log: prompt sequence with files touched ---
    prompt.push_str("## Session Log\n\n");

    if bundle.interactions.is_empty() {
        prompt.push_str("No interactions recorded.\n\n");
    } else {
        for interaction in &bundle.interactions {
            prompt.push_str(&format!(
                "### Interaction {} ({})\n",
                interaction.sequence,
                interaction.timestamp_start.format("%H:%M:%S"),
            ));
            prompt.push_str(&format!("**Prompt:** {}\n", interaction.prompt));

            // Summarize tool calls concisely
            let write_calls: Vec<_> = interaction
                .tool_calls
                .iter()
                .filter(|tc| {
                    let name = tc.tool_name.to_lowercase();
                    matches!(
                        name.as_str(),
                        "write" | "write_file" | "edit" | "str_replace" | "str_replace_editor"
                    )
                })
                .collect();

            let read_calls: Vec<_> = interaction
                .tool_calls
                .iter()
                .filter(|tc| {
                    let name = tc.tool_name.to_lowercase();
                    matches!(name.as_str(), "read" | "read_file" | "glob" | "grep")
                })
                .collect();

            let other_calls = interaction.tool_calls.len() - write_calls.len() - read_calls.len();

            if !interaction.tool_calls.is_empty() {
                let mut parts = Vec::new();
                if !write_calls.len() == 0 {
                } else {
                    let files: Vec<String> = write_calls
                        .iter()
                        .filter_map(|tc| {
                            tc.input
                                .get("file_path")
                                .or_else(|| tc.input.get("path"))
                                .and_then(|v| v.as_str())
                                .map(|p| p.rsplit('/').next().unwrap_or(p).to_string())
                        })
                        .collect();
                    let unique: Vec<String> = {
                        let mut seen = std::collections::HashSet::new();
                        files.into_iter().filter(|f| seen.insert(f.clone())).collect()
                    };
                    parts.push(format!("wrote {}", unique.join(", ")));
                }
                if !read_calls.is_empty() {
                    parts.push(format!("{} reads", read_calls.len()));
                }
                if other_calls > 0 {
                    parts.push(format!("{} other tool calls", other_calls));
                }
                if !parts.is_empty() {
                    prompt.push_str(&format!("**Actions:** {}\n", parts.join(", ")));
                }
            }

            // Duration
            let dur = interaction.timestamp_end - interaction.timestamp_start;
            if dur.num_seconds() > 0 {
                prompt.push_str(&format!("**Duration:** {}s\n", dur.num_seconds()));
            }

            prompt.push('\n');
        }
    }

    // --- Attribution summary ---
    if !bundle.attributions.is_empty() {
        prompt.push_str("## Diff Attribution\n\n");

        let mut ai_gen_lines: u32 = 0;
        let mut ai_assist_lines: u32 = 0;
        let mut unknown_lines: u32 = 0;

        for attr in &bundle.attributions {
            let lines = attr.hunk_end.saturating_sub(attr.hunk_start) + 1;
            match attr.attribution_type {
                super::attribution::AttributionType::AiGenerated => ai_gen_lines += lines,
                super::attribution::AttributionType::AiAssisted => ai_assist_lines += lines,
                _ => unknown_lines += lines,
            }
        }

        let total = ai_gen_lines + ai_assist_lines + unknown_lines;
        prompt.push_str(&format!(
            "- AI-generated (high confidence match): {} lines ({}%)\n- AI-assisted (file-level match): {} lines ({}%)\n- Unattributed (no matching session data): {} lines ({}%)\n\n",
            ai_gen_lines, if total > 0 { ai_gen_lines * 100 / total } else { 0 },
            ai_assist_lines, if total > 0 { ai_assist_lines * 100 / total } else { 0 },
            unknown_lines, if total > 0 { unknown_lines * 100 / total } else { 0 },
        ));

        // Per-file breakdown (top 10 files by changed lines)
        let mut file_lines: std::collections::HashMap<
            &str,
            (u32, &super::attribution::AttributionType),
        > = std::collections::HashMap::new();
        for attr in &bundle.attributions {
            let lines = attr.hunk_end.saturating_sub(attr.hunk_start) + 1;
            let entry = file_lines.entry(&attr.file_path).or_insert((0, &attr.attribution_type));
            entry.0 += lines;
            if attr.confidence > 0.5 {
                entry.1 = &attr.attribution_type;
            }
        }
        let mut sorted_files: Vec<_> = file_lines.into_iter().collect();
        sorted_files.sort_by(|a, b| b.1 .0.cmp(&a.1 .0));
        for (path, (lines, attr_type)) in sorted_files.iter().take(10) {
            prompt.push_str(&format!("- `{}`: {} lines ({})\n", path, lines, attr_type));
        }
        prompt.push('\n');
    }

    // --- Diff stats (just the shape, not the full diff) ---
    if !bundle.diff.is_empty() {
        let added =
            bundle.diff.lines().filter(|l| l.starts_with('+') && !l.starts_with("+++")).count();
        let removed =
            bundle.diff.lines().filter(|l| l.starts_with('-') && !l.starts_with("---")).count();
        let files_changed: std::collections::HashSet<&str> =
            bundle.diff.lines().filter_map(|l| l.strip_prefix("+++ b/")).collect();
        prompt.push_str(&format!(
            "## Diff Stats\n\n{} files changed, +{} lines, -{} lines\n\n",
            files_changed.len(),
            added,
            removed,
        ));
    }

    // --- Instructions ---
    prompt.push_str(
        "## Your Task\n\n\
         Write a concise summary (3-5 sentences) for a code reviewer. Address:\n\
         1. What the developer accomplished and their approach\n\
         2. Whether the work was straightforward or required multiple iterations/corrections\n\
         3. How much of the code was AI-generated or AI-assisted\n\
         4. Any areas a reviewer should scrutinize (e.g., complex logic, patterns suggesting \
            the AI struggled, or areas that needed multiple attempts)\n\n\
         IMPORTANT: All code produced during an AI coding session should be considered \
         AI-generated, even if attribution is \"unattributed\" \u{2014} that label means the \
         tool couldn't match the exact content, NOT that the code was written manually. \
         If sessions exist, assume the developer used AI for the work.\n\n\
         Do NOT list files or repeat raw numbers. Synthesize the data into a narrative.\n",
    );

    prompt
}

pub fn generate_summary(bundle: &ProvenanceBundle, config: &SummaryConfig) -> Option<String> {
    if !config.enabled || config.command.is_empty() {
        return None;
    }

    let prompt = build_prompt(bundle);

    match run_llm_command(&config.command, &prompt) {
        Ok(output) => {
            if config.max_length > 0 && output.len() > config.max_length {
                let end = output.floor_char_boundary(config.max_length);
                Some(format!("{}...", &output[..end]))
            } else {
                Some(output)
            }
        }
        Err(e) => {
            eprintln!("Warning: LLM summary generation failed: {}", e);
            tracing::warn!("LLM summary generation failed: {}", e);
            None
        }
    }
}

fn run_llm_command(command: &str, prompt: &str) -> Result<String> {
    if command.is_empty() {
        anyhow::bail!("Empty LLM command");
    }

    // Use shell to handle quoted arguments and pipes correctly
    let shell = if cfg!(target_os = "windows") { "cmd" } else { "sh" };
    let flag = if cfg!(target_os = "windows") { "/C" } else { "-c" };

    let mut child = Command::new(shell)
        .args([flag, command])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(prompt.as_bytes())?;
    }

    let output = child.wait_with_output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("LLM command failed: {}", stderr);
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provenance::attribution::AttributionType;
    use crate::provenance::interaction::ToolCall;
    use chrono::{Duration, Utc};

    #[test]
    fn test_build_prompt_empty() {
        let bundle = ProvenanceBundle {
            interactions: vec![],
            lineage: vec![],
            attributions: vec![],
            diff: String::new(),
        };
        let prompt = build_prompt(&bundle);
        assert!(prompt.contains("Session Log"));
        assert!(prompt.contains("No interactions recorded"));
        assert!(prompt.contains("Your Task"));
    }

    #[test]
    fn test_build_prompt_with_interactions() {
        let now = Utc::now();
        let bundle = ProvenanceBundle {
            interactions: vec![Interaction {
                id: "s-1".to_string(),
                session_id: "s".to_string(),
                sequence: 1,
                prompt: "Fix the upload handler".to_string(),
                response_texts: vec![],
                tool_calls: vec![ToolCall {
                    tool_name: "Edit".to_string(),
                    tool_id: "t1".to_string(),
                    input: serde_json::json!({"file_path": "/src/upload.rs", "old_string": "old", "new_string": "new"}),
                    output: None,
                    timestamp: now,
                    sequence: 1,
                }],
                timestamp_start: now,
                timestamp_end: now + Duration::seconds(30),
            }],
            lineage: vec![],
            attributions: vec![DiffAttribution {
                commit_sha: String::new(),
                file_path: "src/upload.rs".to_string(),
                hunk_start: 1,
                hunk_end: 10,
                interaction_id: Some("s-1".to_string()),
                confidence: 0.9,
                attribution_type: AttributionType::AiGenerated,
            }],
            diff: "+++ b/src/upload.rs\n+new line\n".to_string(),
        };
        let prompt = build_prompt(&bundle);
        assert!(prompt.contains("Fix the upload handler"));
        assert!(prompt.contains("upload.rs"));
        assert!(prompt.contains("AI-generated"));
        assert!(prompt.contains("Diff Stats"));
    }

    #[test]
    fn test_generate_summary_disabled() {
        let bundle = ProvenanceBundle {
            interactions: vec![],
            lineage: vec![],
            attributions: vec![],
            diff: String::new(),
        };
        let config =
            SummaryConfig { enabled: false, command: "echo test".to_string(), max_length: 2000 };
        assert!(generate_summary(&bundle, &config).is_none());
    }

    #[test]
    fn test_max_length_truncation() {
        let bundle = ProvenanceBundle {
            interactions: vec![],
            lineage: vec![],
            attributions: vec![],
            diff: String::new(),
        };
        let config = SummaryConfig {
            enabled: true,
            command: "echo 'This is a long summary that should be truncated at some point'"
                .to_string(),
            max_length: 20,
        };
        let result = generate_summary(&bundle, &config);
        if let Some(summary) = result {
            assert!(summary.len() <= 24); // 20 + "..."
        }
    }
}
