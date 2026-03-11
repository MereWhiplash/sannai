use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::store::Event;

#[derive(Debug, Clone, Serialize)]
pub struct Interaction {
    pub id: String,
    pub session_id: String,
    pub sequence: u32,
    pub prompt: String,
    pub response_texts: Vec<String>,
    pub tool_calls: Vec<ToolCall>,
    pub timestamp_start: DateTime<Utc>,
    pub timestamp_end: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolCall {
    pub tool_name: String,
    pub tool_id: String,
    pub input: serde_json::Value,
    pub output: Option<String>,
    pub timestamp: DateTime<Utc>,
    pub sequence: u32,
}

/// Group raw events into logical interactions.
///
/// An interaction starts with a `user_prompt` and includes everything
/// until the next `user_prompt` (assistant responses, tool uses, tool results).
pub fn build_interactions(session_id: &str, events: &[Event]) -> Vec<Interaction> {
    let mut interactions = Vec::new();
    let mut current_prompt: Option<&Event> = None;
    let mut current_events: Vec<&Event> = Vec::new();
    let mut sequence: u32 = 0;

    for event in events {
        if event.event_type == "user_prompt" {
            // Finish previous interaction
            if let Some(prompt_event) = current_prompt.take() {
                sequence += 1;
                interactions.push(build_single_interaction(
                    session_id,
                    sequence,
                    prompt_event,
                    &current_events,
                ));
                current_events.clear();
            }
            current_prompt = Some(event);
        } else if current_prompt.is_some() {
            current_events.push(event);
        }
    }

    // Don't forget the last interaction
    if let Some(prompt_event) = current_prompt {
        sequence += 1;
        interactions.push(build_single_interaction(
            session_id,
            sequence,
            prompt_event,
            &current_events,
        ));
    }

    interactions
}

fn build_single_interaction(
    session_id: &str,
    sequence: u32,
    prompt_event: &Event,
    response_events: &[&Event],
) -> Interaction {
    let prompt = prompt_event.content.clone().unwrap_or_default();
    let mut response_texts = Vec::new();
    let mut tool_calls = Vec::new();
    let mut tool_call_seq: u32 = 0;

    for event in response_events {
        match event.event_type.as_str() {
            "assistant_response" => {
                if let Some(text) = &event.content {
                    response_texts.push(text.clone());
                }
            }
            "tool_use" => {
                let tool_name = event.content.clone().unwrap_or_default();
                let metadata = event.metadata.as_ref();
                let tool_id = metadata
                    .and_then(|m| m.get("tool_id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let input = metadata
                    .and_then(|m| m.get("input"))
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);

                tool_call_seq += 1;
                tool_calls.push(ToolCall {
                    tool_name,
                    tool_id: tool_id.clone(),
                    input,
                    output: None,
                    timestamp: event.timestamp,
                    sequence: tool_call_seq,
                });
            }
            "tool_result" => {
                let metadata = event.metadata.as_ref();
                let tool_use_id = metadata
                    .and_then(|m| m.get("tool_use_id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                // Match result to the corresponding tool call
                if !tool_use_id.is_empty() {
                    if let Some(tc) =
                        tool_calls.iter_mut().find(|tc| tc.tool_id == tool_use_id)
                    {
                        tc.output = event.content.clone();
                    }
                }
            }
            _ => {}
        }
    }

    let timestamp_end = response_events
        .last()
        .map(|e| e.timestamp)
        .unwrap_or(prompt_event.timestamp);

    Interaction {
        id: format!("{}-{}", session_id, sequence),
        session_id: session_id.to_string(),
        sequence,
        prompt,
        response_texts,
        tool_calls,
        timestamp_start: prompt_event.timestamp,
        timestamp_end,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn make_event(
        event_type: &str,
        content: Option<&str>,
        timestamp: DateTime<Utc>,
        metadata: Option<serde_json::Value>,
    ) -> Event {
        Event {
            id: None,
            session_id: "test-session".to_string(),
            event_type: event_type.to_string(),
            content: content.map(|s| s.to_string()),
            context_files: None,
            timestamp,
            metadata,
        }
    }

    #[test]
    fn test_single_interaction() {
        let now = Utc::now();
        let events = vec![
            make_event("user_prompt", Some("Fix the bug"), now, None),
            make_event(
                "assistant_response",
                Some("I'll fix that."),
                now + Duration::seconds(2),
                None,
            ),
        ];

        let interactions = build_interactions("test-session", &events);
        assert_eq!(interactions.len(), 1);
        assert_eq!(interactions[0].sequence, 1);
        assert_eq!(interactions[0].prompt, "Fix the bug");
        assert_eq!(interactions[0].response_texts, vec!["I'll fix that."]);
        assert!(interactions[0].tool_calls.is_empty());
    }

    #[test]
    fn test_multiple_interactions() {
        let now = Utc::now();
        let events = vec![
            make_event("user_prompt", Some("First prompt"), now, None),
            make_event(
                "assistant_response",
                Some("First response"),
                now + Duration::seconds(1),
                None,
            ),
            make_event(
                "user_prompt",
                Some("Second prompt"),
                now + Duration::seconds(5),
                None,
            ),
            make_event(
                "assistant_response",
                Some("Second response"),
                now + Duration::seconds(6),
                None,
            ),
        ];

        let interactions = build_interactions("test-session", &events);
        assert_eq!(interactions.len(), 2);
        assert_eq!(interactions[0].prompt, "First prompt");
        assert_eq!(interactions[1].prompt, "Second prompt");
        assert_eq!(interactions[0].sequence, 1);
        assert_eq!(interactions[1].sequence, 2);
    }

    #[test]
    fn test_interaction_with_tool_calls() {
        let now = Utc::now();
        let events = vec![
            make_event("user_prompt", Some("Read the file"), now, None),
            make_event(
                "tool_use",
                Some("Read"),
                now + Duration::seconds(1),
                Some(serde_json::json!({
                    "tool_id": "toolu_01",
                    "input": {"file_path": "/src/main.rs"}
                })),
            ),
            make_event(
                "tool_result",
                Some("fn main() {}"),
                now + Duration::seconds(2),
                Some(serde_json::json!({
                    "tool_use_id": "toolu_01",
                    "is_error": false
                })),
            ),
        ];

        let interactions = build_interactions("test-session", &events);
        assert_eq!(interactions.len(), 1);
        assert_eq!(interactions[0].tool_calls.len(), 1);

        let tc = &interactions[0].tool_calls[0];
        assert_eq!(tc.tool_name, "Read");
        assert_eq!(tc.tool_id, "toolu_01");
        assert_eq!(tc.input["file_path"], "/src/main.rs");
        assert_eq!(tc.output.as_deref(), Some("fn main() {}"));
    }

    #[test]
    fn test_events_before_first_prompt_ignored() {
        let now = Utc::now();
        let events = vec![
            // Events before any prompt should be ignored
            make_event(
                "assistant_response",
                Some("Stray response"),
                now,
                None,
            ),
            make_event("user_prompt", Some("Actual prompt"), now + Duration::seconds(5), None),
            make_event(
                "assistant_response",
                Some("Real response"),
                now + Duration::seconds(6),
                None,
            ),
        ];

        let interactions = build_interactions("test-session", &events);
        assert_eq!(interactions.len(), 1);
        assert_eq!(interactions[0].prompt, "Actual prompt");
    }

    #[test]
    fn test_empty_events() {
        let interactions = build_interactions("test-session", &[]);
        assert!(interactions.is_empty());
    }
}
