use serde::Serialize;
use std::collections::HashMap;

use super::interaction::{Interaction, ToolCall};

#[derive(Debug, Clone, Serialize)]
pub struct FileLineage {
    pub interaction_id: String,
    pub file_path: String,
    pub operations: Vec<FileOp>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileOp {
    pub op_type: FileOpType,
    pub tool_call_sequence: u32,
    pub content_snippet: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub enum FileOpType {
    Read,
    Write,
    ReadWrite,
}

const SNIPPET_MAX_LEN: usize = 200;

/// Extract file lineage from an interaction's tool calls.
pub fn build_lineage(interaction: &Interaction) -> Vec<FileLineage> {
    let mut file_ops: HashMap<String, Vec<FileOp>> = HashMap::new();

    for tc in &interaction.tool_calls {
        if let Some((path, op)) = classify_tool_call(tc) {
            file_ops.entry(path).or_default().push(op);
        }
    }

    let mut lineage: Vec<FileLineage> = file_ops
        .into_iter()
        .map(|(file_path, operations)| FileLineage {
            interaction_id: interaction.id.clone(),
            file_path,
            operations,
        })
        .collect();

    // Sort by file path for deterministic output
    lineage.sort_by(|a, b| a.file_path.cmp(&b.file_path));
    lineage
}

fn classify_tool_call(tc: &ToolCall) -> Option<(String, FileOp)> {
    let name = tc.tool_name.to_lowercase();

    match name.as_str() {
        "read" | "read_file" => {
            let path = extract_file_path(&tc.input)?;
            let snippet =
                tc.output.as_deref().map(|s| truncate(s, SNIPPET_MAX_LEN)).unwrap_or_default();
            Some((
                path,
                FileOp {
                    op_type: FileOpType::Read,
                    tool_call_sequence: tc.sequence,
                    content_snippet: snippet,
                },
            ))
        }
        "write" | "write_file" => {
            let path = extract_file_path(&tc.input)?;
            let content = tc.input.get("content").and_then(|v| v.as_str()).unwrap_or("");
            Some((
                path,
                FileOp {
                    op_type: FileOpType::Write,
                    tool_call_sequence: tc.sequence,
                    content_snippet: truncate(content, SNIPPET_MAX_LEN),
                },
            ))
        }
        "edit" | "str_replace" | "str_replace_editor" => {
            let path = extract_file_path(&tc.input)?;
            let old = tc
                .input
                .get("old_string")
                .or_else(|| tc.input.get("old_str"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let new = tc
                .input
                .get("new_string")
                .or_else(|| tc.input.get("new_str"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let snippet = if !old.is_empty() && !new.is_empty() {
                format!(
                    "-{}\n+{}",
                    truncate(old, SNIPPET_MAX_LEN / 2),
                    truncate(new, SNIPPET_MAX_LEN / 2)
                )
            } else {
                truncate(new, SNIPPET_MAX_LEN)
            };
            Some((
                path,
                FileOp {
                    op_type: FileOpType::ReadWrite,
                    tool_call_sequence: tc.sequence,
                    content_snippet: snippet,
                },
            ))
        }
        _ => None,
    }
}

fn extract_file_path(input: &serde_json::Value) -> Option<String> {
    input
        .get("file_path")
        .or_else(|| input.get("path"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        let end = s.floor_char_boundary(max_len);
        format!("{}...", &s[..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provenance::interaction::{Interaction, ToolCall};
    use chrono::Utc;

    fn make_interaction(tool_calls: Vec<ToolCall>) -> Interaction {
        let now = Utc::now();
        Interaction {
            id: "test-1".to_string(),
            session_id: "session-1".to_string(),
            sequence: 1,
            prompt: "test".to_string(),
            response_texts: vec![],
            tool_calls,
            timestamp_start: now,
            timestamp_end: now,
        }
    }

    fn make_tool_call(
        name: &str,
        input: serde_json::Value,
        output: Option<&str>,
        seq: u32,
    ) -> ToolCall {
        ToolCall {
            tool_name: name.to_string(),
            tool_id: format!("toolu_{}", seq),
            input,
            output: output.map(|s| s.to_string()),
            timestamp: Utc::now(),
            sequence: seq,
        }
    }

    #[test]
    fn test_read_lineage() {
        let tc = make_tool_call(
            "Read",
            serde_json::json!({"file_path": "/src/main.rs"}),
            Some("fn main() {}"),
            1,
        );
        let interaction = make_interaction(vec![tc]);
        let lineage = build_lineage(&interaction);

        assert_eq!(lineage.len(), 1);
        assert_eq!(lineage[0].file_path, "/src/main.rs");
        assert_eq!(lineage[0].operations.len(), 1);
        assert_eq!(lineage[0].operations[0].op_type, FileOpType::Read);
    }

    #[test]
    fn test_write_lineage() {
        let tc = make_tool_call(
            "Write",
            serde_json::json!({"file_path": "/src/new.rs", "content": "fn hello() {}"}),
            None,
            1,
        );
        let interaction = make_interaction(vec![tc]);
        let lineage = build_lineage(&interaction);

        assert_eq!(lineage.len(), 1);
        assert_eq!(lineage[0].file_path, "/src/new.rs");
        assert_eq!(lineage[0].operations[0].op_type, FileOpType::Write);
        assert_eq!(lineage[0].operations[0].content_snippet, "fn hello() {}");
    }

    #[test]
    fn test_edit_lineage() {
        let tc = make_tool_call(
            "Edit",
            serde_json::json!({
                "file_path": "/src/lib.rs",
                "old_string": "old code",
                "new_string": "new code"
            }),
            None,
            1,
        );
        let interaction = make_interaction(vec![tc]);
        let lineage = build_lineage(&interaction);

        assert_eq!(lineage.len(), 1);
        assert_eq!(lineage[0].operations[0].op_type, FileOpType::ReadWrite);
        assert!(lineage[0].operations[0].content_snippet.contains("-old code"));
        assert!(lineage[0].operations[0].content_snippet.contains("+new code"));
    }

    #[test]
    fn test_read_then_write_same_file() {
        let tcs = vec![
            make_tool_call(
                "Read",
                serde_json::json!({"file_path": "/src/main.rs"}),
                Some("original content"),
                1,
            ),
            make_tool_call(
                "Write",
                serde_json::json!({"file_path": "/src/main.rs", "content": "modified content"}),
                None,
                2,
            ),
        ];
        let interaction = make_interaction(tcs);
        let lineage = build_lineage(&interaction);

        assert_eq!(lineage.len(), 1);
        assert_eq!(lineage[0].operations.len(), 2);
        assert_eq!(lineage[0].operations[0].op_type, FileOpType::Read);
        assert_eq!(lineage[0].operations[1].op_type, FileOpType::Write);
    }

    #[test]
    fn test_bash_ignored() {
        let tc =
            make_tool_call("Bash", serde_json::json!({"command": "ls -la"}), Some("output"), 1);
        let interaction = make_interaction(vec![tc]);
        let lineage = build_lineage(&interaction);
        assert!(lineage.is_empty());
    }
}
