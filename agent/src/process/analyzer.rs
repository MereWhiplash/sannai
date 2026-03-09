use serde::{Deserialize, Serialize};

use crate::provenance::interaction::Interaction;

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

pub fn analyze(interactions: &[Interaction]) -> ProcessMetricsResult {
    ProcessMetricsResult {
        steering_ratio: compute_steering_ratio(interactions),
        exploration_score: 0.0,
        read_write_ratio: 0.0,
        test_behavior: "unknown".to_string(),
        error_fix_cycles: 0,
        red_flags: vec![],
        prompt_specificity: 0.0,
        total_interactions: interactions.len() as i32,
        total_tool_calls: interactions
            .iter()
            .map(|i| i.tool_calls.len())
            .sum::<usize>() as i32,
        files_read: 0,
        files_written: 0,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provenance::interaction::{Interaction, ToolCall};
    use chrono::{Duration, Utc};

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
