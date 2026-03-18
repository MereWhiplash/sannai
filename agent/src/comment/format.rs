use std::collections::HashMap;

use crate::provenance::attribution::{AttributionType, DiffAttribution};
use crate::provenance::interaction::Interaction;
use crate::provenance::lineage::{FileLineage, FileOpType};

pub struct CommentData {
    pub sessions: Vec<SessionSummary>,
    pub attributions: Vec<DiffAttribution>,
    pub llm_summary: Option<String>,
}

pub struct SessionSummary {
    pub session_id: String,
    pub interactions: Vec<Interaction>,
    pub lineage: Vec<FileLineage>,
    pub duration: String,
    /// Only shown when wall time > 2x active time and > 10min.
    pub wall_time: Option<String>,
}

pub fn format_comment(data: &CommentData) -> String {
    let mut md = String::new();

    // Header
    let total_interactions: usize = data.sessions.iter().map(|s| s.interactions.len()).sum();
    let total_sessions = data.sessions.len();

    md.push_str("## Sannai Code Provenance\n\n");
    md.push_str(&format!(
        "**{} AI session{}** across **{} interaction{}**.\n\n",
        total_sessions,
        if total_sessions != 1 { "s" } else { "" },
        total_interactions,
        if total_interactions != 1 { "s" } else { "" },
    ));

    // Summary stats table
    if !data.attributions.is_empty() {
        let stats = compute_attribution_stats(&data.attributions);
        let total = stats.ai_generated_lines + stats.ai_assisted_lines + stats.unlinked_lines;
        if total > 0 {
            let pct = |n: u32| -> u32 {
                if total == 0 {
                    0
                } else {
                    ((n as f64 / total as f64) * 100.0).round() as u32
                }
            };
            md.push_str("| AI-generated | AI-assisted | Unlinked |\n");
            md.push_str("|:---:|:---:|:---:|\n");
            md.push_str(&format!(
                "| {}% ({} lines) | {}% ({} lines) | {}% ({} lines) |\n\n",
                pct(stats.ai_generated_lines),
                stats.ai_generated_lines,
                pct(stats.ai_assisted_lines),
                stats.ai_assisted_lines,
                pct(stats.unlinked_lines),
                stats.unlinked_lines,
            ));
        }
    }

    // LLM Summary
    if let Some(summary) = &data.llm_summary {
        md.push_str("### Summary\n\n");
        md.push_str(summary);
        md.push_str("\n\n");
    }

    // Per-session interaction tables: key interactions shown, others collapsed
    for session in &data.sessions {
        let short_id = &session.session_id[..std::cmp::min(8, session.session_id.len())];
        let n = session.interactions.len();

        let wall_note = match &session.wall_time {
            Some(wt) => format!(", {} wall", wt),
            None => String::new(),
        };

        // Partition into key (wrote files or has attribution) and supporting
        let (key, supporting): (Vec<_>, Vec<_>) =
            session.interactions.iter().partition(|interaction| {
                let has_writes = session.lineage.iter().any(|l| {
                    l.interaction_id == interaction.id
                        && l.operations.iter().any(|op| {
                            matches!(op.op_type, FileOpType::Write | FileOpType::ReadWrite)
                        })
                });
                let has_attr = data
                    .attributions
                    .iter()
                    .any(|a| a.interaction_id.as_deref() == Some(&interaction.id));
                has_writes || has_attr
            });

        md.push_str(&format!(
            "### Session `{}` \u{2014} {} interaction{}, {} active{}\n\n",
            short_id,
            n,
            if n != 1 { "s" } else { "" },
            session.duration,
            wall_note,
        ));

        // Key interactions table (always visible)
        if !key.is_empty() {
            md.push_str(&format!(
                "**{} key interaction{}** (wrote files or attributed to diff):\n\n",
                key.len(),
                if key.len() != 1 { "s" } else { "" },
            ));
            md.push_str("| # | Prompt | Files | Attribution |\n");
            md.push_str("|---|--------|-------|-------------|\n");

            for interaction in &key {
                let prompt_preview = truncate_for_table(&interaction.prompt, 60);
                let files = format_files_touched(&session.lineage, &interaction.id);
                let attr = format_interaction_attribution(&data.attributions, &interaction.id);

                md.push_str(&format!(
                    "| {} | {} | {} | {} |\n",
                    interaction.sequence, prompt_preview, files, attr,
                ));
            }
            md.push('\n');
        }

        // Supporting interactions collapsed
        if !supporting.is_empty() {
            md.push_str("<details>\n");
            md.push_str(&format!(
                "<summary>{} supporting interaction{} (investigation, discussion, \
                 read-only)</summary>\n\n",
                supporting.len(),
                if supporting.len() != 1 { "s" } else { "" },
            ));
            md.push_str("| # | Prompt | Files |\n");
            md.push_str("|---|--------|-------|\n");

            for interaction in &supporting {
                let prompt_preview = truncate_for_table(&interaction.prompt, 60);
                let files = format_files_touched(&session.lineage, &interaction.id);

                md.push_str(&format!(
                    "| {} | {} | {} |\n",
                    interaction.sequence, prompt_preview, files,
                ));
            }
            md.push_str("\n</details>\n\n");
        }
    }

    // Diff attribution in collapsible section, aggregated per-file
    if !data.attributions.is_empty() {
        let file_attrs = aggregate_file_attributions(&data.attributions);
        md.push_str("<details>\n");
        md.push_str(&format!(
            "<summary>Diff attribution ({} file{})</summary>\n\n",
            file_attrs.len(),
            if file_attrs.len() != 1 { "s" } else { "" },
        ));

        md.push_str("| File | Lines | Source |\n");
        md.push_str("|------|-------|--------|\n");

        for fa in &file_attrs {
            let label = format_attribution_label(&fa.attribution_type, fa.confidence);
            md.push_str(&format!("| {} | {} | {} |\n", fa.file_path, fa.total_lines, label,));
        }
        md.push_str("\n</details>\n\n");
    }

    // Footer
    md.push_str("---\n");
    md.push_str("<sub>Generated by [Sannai](https://github.com/MereWhiplash/sannai) \u{2014} code provenance for AI-assisted development</sub>\n");

    md
}

struct AttributionStats {
    ai_generated_lines: u32,
    ai_assisted_lines: u32,
    unlinked_lines: u32,
}

fn compute_attribution_stats(attributions: &[DiffAttribution]) -> AttributionStats {
    let mut stats =
        AttributionStats { ai_generated_lines: 0, ai_assisted_lines: 0, unlinked_lines: 0 };

    for attr in attributions {
        let lines = attr.hunk_end.saturating_sub(attr.hunk_start) + 1;
        match attr.attribution_type {
            AttributionType::AiGenerated => stats.ai_generated_lines += lines,
            AttributionType::AiAssisted => stats.ai_assisted_lines += lines,
            AttributionType::Unknown | AttributionType::Manual => stats.unlinked_lines += lines,
        }
    }

    stats
}

struct FileAttribution {
    file_path: String,
    total_lines: u32,
    confidence: f32,
    attribution_type: AttributionType,
}

fn aggregate_file_attributions(attributions: &[DiffAttribution]) -> Vec<FileAttribution> {
    let mut map: HashMap<String, (u32, f32, AttributionType)> = HashMap::new();

    for attr in attributions {
        let lines = attr.hunk_end.saturating_sub(attr.hunk_start) + 1;
        let entry = map.entry(attr.file_path.clone()).or_insert((0, 0.0, AttributionType::Unknown));
        entry.0 += lines;
        // Keep highest-confidence attribution
        if attr.confidence > entry.1 {
            entry.1 = attr.confidence;
            entry.2 = attr.attribution_type.clone();
        }
    }

    let mut result: Vec<FileAttribution> = map
        .into_iter()
        .map(|(file_path, (total_lines, confidence, attribution_type))| FileAttribution {
            file_path,
            total_lines,
            confidence,
            attribution_type,
        })
        .collect();

    result.sort_by(|a, b| b.total_lines.cmp(&a.total_lines));
    result
}

fn format_attribution_label(attr_type: &AttributionType, confidence: f32) -> String {
    match attr_type {
        AttributionType::AiGenerated => {
            format!("\u{1f916} AI-generated ({:.0}%)", confidence * 100.0)
        }
        AttributionType::AiAssisted => {
            format!("\u{1f916} AI-assisted ({:.0}%)", confidence * 100.0)
        }
        AttributionType::Manual => "Manual".to_string(),
        AttributionType::Unknown => "Unlinked".to_string(),
    }
}

fn truncate_for_table(text: &str, max_len: usize) -> String {
    let first_line = text.lines().next().unwrap_or("");
    // Escape pipes and newlines for markdown table cells
    let clean = first_line.replace('|', "\\|");
    if clean.len() <= max_len {
        clean
    } else {
        let end = clean.floor_char_boundary(max_len);
        format!("{}...", &clean[..end])
    }
}

fn format_files_touched(lineage: &[FileLineage], interaction_id: &str) -> String {
    // Deduplicate: one entry per file with combined R/W label
    let mut file_ops: HashMap<String, (bool, bool)> = HashMap::new();

    for l in lineage.iter().filter(|l| l.interaction_id == interaction_id) {
        let filename = l.file_path.rsplit('/').next().unwrap_or(&l.file_path).to_string();
        let entry = file_ops.entry(filename).or_insert((false, false));
        for op in &l.operations {
            match op.op_type {
                FileOpType::Read => entry.0 = true,
                FileOpType::Write => entry.1 = true,
                FileOpType::ReadWrite => {
                    entry.0 = true;
                    entry.1 = true;
                }
            }
        }
    }

    if file_ops.is_empty() {
        return "\u{2014}".to_string();
    }

    let mut files: Vec<(String, String)> = file_ops
        .into_iter()
        .map(|(name, (r, w))| {
            let label = match (r, w) {
                (true, true) => "R/W",
                (true, false) => "R",
                (false, true) => "W",
                _ => "",
            };
            (name, label.to_string())
        })
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));

    let max_shown = 3;
    if files.len() <= max_shown {
        files.iter().map(|(n, l)| format!("{} ({})", n, l)).collect::<Vec<_>>().join(", ")
    } else {
        let shown: Vec<String> =
            files[..max_shown].iter().map(|(n, l)| format!("{} ({})", n, l)).collect();
        format!("{}, +{} more", shown.join(", "), files.len() - max_shown)
    }
}

fn format_interaction_attribution(
    attributions: &[DiffAttribution],
    interaction_id: &str,
) -> String {
    let attrs: Vec<&DiffAttribution> = attributions
        .iter()
        .filter(|a| a.interaction_id.as_deref() == Some(interaction_id))
        .collect();

    if attrs.is_empty() {
        return "\u{2014}".to_string();
    }

    let best = attrs
        .iter()
        .max_by(|a, b| a.confidence.partial_cmp(&b.confidence).unwrap_or(std::cmp::Ordering::Equal))
        .unwrap();

    format_attribution_label(&best.attribution_type, best.confidence)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn test_format_empty_comment() {
        let data = CommentData { sessions: vec![], attributions: vec![], llm_summary: None };
        let comment = format_comment(&data);
        assert!(comment.contains("Sannai Code Provenance"));
        assert!(comment.contains("0 AI sessions"));
    }

    #[test]
    fn test_format_with_session() {
        let now = Utc::now();
        let data = CommentData {
            sessions: vec![SessionSummary {
                session_id: "abcdef12-3456-7890".to_string(),
                interactions: vec![Interaction {
                    id: "abcdef12-1".to_string(),
                    session_id: "abcdef12".to_string(),
                    sequence: 1,
                    prompt: "Fix the upload bug".to_string(),
                    response_texts: vec!["I'll fix that.".to_string()],
                    tool_calls: vec![],
                    timestamp_start: now,
                    timestamp_end: now,
                }],
                lineage: vec![],
                duration: "5m".to_string(),
                wall_time: None,
            }],
            attributions: vec![],
            llm_summary: None,
        };

        let comment = format_comment(&data);
        assert!(comment.contains("1 AI session"));
        assert!(comment.contains("abcdef12"));
        assert!(comment.contains("Fix the upload bug"));
        // No-write interaction goes to supporting section
        assert!(comment.contains("supporting interaction"));
    }

    #[test]
    fn test_format_with_summary() {
        let data = CommentData {
            sessions: vec![],
            attributions: vec![],
            llm_summary: Some("The developer built a feature.".to_string()),
        };
        let comment = format_comment(&data);
        assert!(comment.contains("### Summary"));
        assert!(comment.contains("The developer built a feature."));
    }

    #[test]
    fn test_truncate_for_table() {
        assert_eq!(truncate_for_table("short", 60), "short");
        let long = "a".repeat(100);
        let truncated = truncate_for_table(&long, 60);
        assert!(truncated.ends_with("..."));
        assert!(truncated.len() <= 64); // 60 + "..."
    }

    #[test]
    fn test_truncate_for_table_multibyte_utf8() {
        // Ensure truncation doesn't panic on multi-byte chars
        let text = "Fix the \u{1f41b} bug in upload \u{2014} handle edge cases properly and more text here";
        let truncated = truncate_for_table(text, 20);
        assert!(truncated.ends_with("..."));
        // Should not panic and should be valid UTF-8
        assert!(truncated.is_char_boundary(truncated.len() - 3));
    }

    #[test]
    fn test_format_with_wall_time() {
        let now = Utc::now();
        let data = CommentData {
            sessions: vec![SessionSummary {
                session_id: "abcdef12-3456-7890".to_string(),
                interactions: vec![Interaction {
                    id: "abcdef12-1".to_string(),
                    session_id: "abcdef12".to_string(),
                    sequence: 1,
                    prompt: "Fix bug".to_string(),
                    response_texts: vec![],
                    tool_calls: vec![],
                    timestamp_start: now,
                    timestamp_end: now,
                }],
                lineage: vec![],
                duration: "6m".to_string(),
                wall_time: Some("3h 6m".to_string()),
            }],
            attributions: vec![],
            llm_summary: None,
        };

        let comment = format_comment(&data);
        assert!(comment.contains("6m active"));
        assert!(comment.contains("3h 6m wall"), "comment was: {}", comment);
    }

    #[test]
    fn test_format_attribution_stats() {
        let data = CommentData {
            sessions: vec![],
            attributions: vec![
                DiffAttribution {
                    commit_sha: String::new(),
                    file_path: "src/main.rs".to_string(),
                    hunk_start: 1,
                    hunk_end: 10,
                    interaction_id: Some("s-1".to_string()),
                    confidence: 0.9,
                    attribution_type: AttributionType::AiGenerated,
                },
                DiffAttribution {
                    commit_sha: String::new(),
                    file_path: "src/lib.rs".to_string(),
                    hunk_start: 1,
                    hunk_end: 5,
                    interaction_id: None,
                    confidence: 0.0,
                    attribution_type: AttributionType::Unknown,
                },
            ],
            llm_summary: None,
        };

        let comment = format_comment(&data);
        assert!(comment.contains("AI-generated"));
        assert!(comment.contains("Unlinked"));
        assert!(comment.contains("<details>"));
        assert!(comment.contains("Diff attribution"));
    }

    #[test]
    fn test_files_touched_dedup() {
        let lineage = vec![FileLineage {
            interaction_id: "test-1".to_string(),
            file_path: "/src/main.rs".to_string(),
            operations: vec![
                crate::provenance::lineage::FileOp {
                    op_type: FileOpType::Read,
                    tool_call_sequence: 1,
                    content_snippet: String::new(),
                },
                crate::provenance::lineage::FileOp {
                    op_type: FileOpType::Write,
                    tool_call_sequence: 2,
                    content_snippet: String::new(),
                },
            ],
        }];

        let result = format_files_touched(&lineage, "test-1");
        assert!(result.contains("main.rs (R/W)"));
        // Should NOT have duplicate entries
        assert_eq!(result.matches("main.rs").count(), 1);
    }

    #[test]
    fn test_files_touched_truncation() {
        let lineage: Vec<FileLineage> = (1..=5)
            .map(|i| FileLineage {
                interaction_id: "test-1".to_string(),
                file_path: format!("/src/file{}.rs", i),
                operations: vec![crate::provenance::lineage::FileOp {
                    op_type: FileOpType::Read,
                    tool_call_sequence: 1,
                    content_snippet: String::new(),
                }],
            })
            .collect();

        let result = format_files_touched(&lineage, "test-1");
        assert!(result.contains("+2 more"));
    }
}
