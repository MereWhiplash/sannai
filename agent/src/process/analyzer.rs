use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::provenance::interaction::{Interaction, ToolCall};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessMetricsResult {
    pub steering_ratio: f64,
    pub exploration_score: f64,
    pub read_write_ratio: f64,
    pub test_behavior: String,
    pub error_fix_cycles: i32,
    pub red_flags: Vec<String>,
    pub prompt_specificity: f64,
    pub total_interactions: i32,
    pub total_tool_calls: i32,
    pub files_read: i32,
    pub files_written: i32,
}

const EXPLORE_TOOLS: &[&str] = &["Read", "Glob", "Grep"];
const WRITE_TOOLS: &[&str] = &["Write", "Edit"];

pub fn analyze(interactions: &[Interaction]) -> ProcessMetricsResult {
    let (read_write_ratio, files_read, files_written) =
        compute_read_write_ratio(interactions);

    ProcessMetricsResult {
        steering_ratio: compute_steering_ratio(interactions),
        exploration_score: compute_exploration_score(interactions),
        read_write_ratio,
        test_behavior: detect_test_behavior(interactions),
        error_fix_cycles: 0,
        red_flags: vec![],
        prompt_specificity: 0.0,
        total_interactions: interactions.len() as i32,
        total_tool_calls: interactions
            .iter()
            .map(|i| i.tool_calls.len())
            .sum::<usize>() as i32,
        files_read,
        files_written,
    }
}

fn compute_steering_ratio(interactions: &[Interaction]) -> f64 {
    if interactions.is_empty() {
        return 0.0;
    }
    // Every interaction has a user prompt by definition.
    // Steering ratio = 1.0 when all interactions are user-driven.
    // Will be refined when prompt_specificity weighs in.
    1.0
}

fn compute_exploration_score(interactions: &[Interaction]) -> f64 {
    let all_tools: Vec<&ToolCall> = interactions
        .iter()
        .flat_map(|i| i.tool_calls.iter())
        .collect();

    if all_tools.is_empty() {
        return 0.0;
    }

    let first_write_idx = all_tools
        .iter()
        .position(|tc| WRITE_TOOLS.contains(&tc.tool_name.as_str()));

    let explore_before_write = match first_write_idx {
        Some(idx) => all_tools[..idx]
            .iter()
            .filter(|tc| EXPLORE_TOOLS.contains(&tc.tool_name.as_str()))
            .count(),
        None => all_tools
            .iter()
            .filter(|tc| EXPLORE_TOOLS.contains(&tc.tool_name.as_str()))
            .count(),
    };

    // Normalize: 0 explore = 0.0, 5+ explore before write = 1.0
    (explore_before_write as f64 / 5.0).min(1.0)
}

const TEST_COMMANDS: &[&str] = &[
    "cargo test",
    "npm test",
    "npx jest",
    "pytest",
    "go test",
    "make test",
    "yarn test",
    "bun test",
];

fn detect_test_behavior(interactions: &[Interaction]) -> String {
    let all_tools: Vec<&str> = interactions
        .iter()
        .flat_map(|i| i.tool_calls.iter())
        .filter_map(|tc| {
            let is_test = tc.tool_name == "Bash"
                && tc
                    .input
                    .get("command")
                    .and_then(|v| v.as_str())
                    .map(|cmd| TEST_COMMANDS.iter().any(|t| cmd.contains(t)))
                    .unwrap_or(false);
            let is_write = WRITE_TOOLS.contains(&tc.tool_name.as_str());
            if is_test {
                Some("test")
            } else if is_write {
                Some("write")
            } else {
                None
            }
        })
        .collect();

    let has_test = all_tools.iter().any(|k| *k == "test");
    if !has_test {
        return "no_tests".to_string();
    }

    let first_test = all_tools.iter().position(|k| *k == "test");
    let first_write = all_tools.iter().position(|k| *k == "write");

    match (first_test, first_write) {
        (Some(t), Some(w)) if t < w => "tdd".to_string(),
        (Some(_), Some(_)) => "test_after".to_string(),
        (Some(_), None) => "test_only".to_string(),
        _ => "no_tests".to_string(),
    }
}

fn compute_read_write_ratio(interactions: &[Interaction]) -> (f64, i32, i32) {
    let mut files_read = HashSet::new();
    let mut files_written = HashSet::new();

    for interaction in interactions {
        for tc in &interaction.tool_calls {
            let file_path = tc.input.get("file_path").and_then(|v| v.as_str());
            match tc.tool_name.as_str() {
                "Read" => {
                    if let Some(fp) = file_path {
                        files_read.insert(fp.to_string());
                    }
                }
                "Write" | "Edit" => {
                    if let Some(fp) = file_path {
                        files_written.insert(fp.to_string());
                    }
                }
                _ => {}
            }
        }
    }

    let read_count = files_read.len() as i32;
    let write_count = files_written.len().max(1) as i32;
    let ratio = read_count as f64 / write_count as f64;

    (ratio, read_count, write_count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provenance::interaction::{Interaction, ToolCall};
    use chrono::{DateTime, Duration, Utc};

    fn make_interaction(seq: u32, prompt: &str, tools: Vec<ToolCall>) -> Interaction {
        let now = Utc::now();
        Interaction {
            id: format!("test-{}", seq),
            session_id: "test-session".to_string(),
            sequence: seq,
            prompt: prompt.to_string(),
            response_texts: vec![],
            tool_calls: tools,
            timestamp_start: now + Duration::seconds(seq as i64 * 10),
            timestamp_end: now + Duration::seconds(seq as i64 * 10 + 5),
        }
    }

    fn make_tool_call(
        name: &str,
        input: serde_json::Value,
        base: DateTime<Utc>,
        seq: u32,
    ) -> ToolCall {
        ToolCall {
            tool_name: name.to_string(),
            tool_id: format!("toolu_{}", seq),
            input,
            output: None,
            timestamp: base + Duration::seconds(seq as i64),
            sequence: seq,
        }
    }

    #[test]
    fn test_exploration_score_reads_before_writes() {
        let now = Utc::now();
        let interactions = vec![
            make_interaction(
                1,
                "Look at the codebase",
                vec![
                    make_tool_call(
                        "Read",
                        serde_json::json!({"file_path": "/src/a.rs"}),
                        now,
                        1,
                    ),
                    make_tool_call(
                        "Read",
                        serde_json::json!({"file_path": "/src/b.rs"}),
                        now,
                        2,
                    ),
                    make_tool_call(
                        "Glob",
                        serde_json::json!({"pattern": "**/*.rs"}),
                        now,
                        3,
                    ),
                ],
            ),
            make_interaction(
                2,
                "Now fix the bug",
                vec![make_tool_call(
                    "Edit",
                    serde_json::json!({"file_path": "/src/a.rs"}),
                    now,
                    1,
                )],
            ),
        ];
        let metrics = analyze(&interactions);
        assert!(metrics.exploration_score > 0.5);
        assert!(metrics.read_write_ratio > 1.0);
        assert_eq!(metrics.files_read, 2);
        assert_eq!(metrics.files_written, 1);
    }

    #[test]
    fn test_exploration_score_write_first() {
        let now = Utc::now();
        let interactions = vec![make_interaction(
            1,
            "Write a new file",
            vec![make_tool_call(
                "Write",
                serde_json::json!({"file_path": "/src/new.rs", "content": "fn main() {}"}),
                now,
                1,
            )],
        )];
        let metrics = analyze(&interactions);
        assert!((metrics.exploration_score - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_detect_test_after_code() {
        let now = Utc::now();
        let interactions = vec![
            make_interaction(
                1,
                "Add the feature",
                vec![make_tool_call(
                    "Write",
                    serde_json::json!({"file_path": "/src/feature.rs"}),
                    now,
                    1,
                )],
            ),
            make_interaction(
                2,
                "Add tests",
                vec![make_tool_call(
                    "Bash",
                    serde_json::json!({"command": "cargo test"}),
                    now,
                    1,
                )],
            ),
        ];
        let metrics = analyze(&interactions);
        assert_eq!(metrics.test_behavior, "test_after");
    }

    #[test]
    fn test_detect_tdd() {
        let now = Utc::now();
        let interactions = vec![
            make_interaction(
                1,
                "Write a failing test first",
                vec![make_tool_call(
                    "Bash",
                    serde_json::json!({"command": "cargo test test_foo"}),
                    now,
                    1,
                )],
            ),
            make_interaction(
                2,
                "Now implement",
                vec![make_tool_call(
                    "Write",
                    serde_json::json!({"file_path": "/src/foo.rs"}),
                    now,
                    1,
                )],
            ),
            make_interaction(
                3,
                "Run tests again",
                vec![make_tool_call(
                    "Bash",
                    serde_json::json!({"command": "cargo test"}),
                    now,
                    1,
                )],
            ),
        ];
        let metrics = analyze(&interactions);
        assert_eq!(metrics.test_behavior, "tdd");
    }

    #[test]
    fn test_detect_no_tests() {
        let now = Utc::now();
        let interactions = vec![make_interaction(
            1,
            "Write code",
            vec![make_tool_call(
                "Write",
                serde_json::json!({"file_path": "/src/main.rs"}),
                now,
                1,
            )],
        )];
        let metrics = analyze(&interactions);
        assert_eq!(metrics.test_behavior, "no_tests");
    }

    #[test]
    fn test_steering_ratio_all_user_driven() {
        let interactions = vec![
            make_interaction(
                1,
                "Add retry logic to the HTTP client with exponential backoff",
                vec![],
            ),
            make_interaction(2, "Now add unit tests for the retry function", vec![]),
        ];
        let metrics = analyze(&interactions);
        assert!((metrics.steering_ratio - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_steering_ratio_empty() {
        let metrics = analyze(&[]);
        assert!((metrics.steering_ratio - 0.0).abs() < f64::EPSILON);
    }
}
