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

    // Filter noise interactions and renumber
    let mut filtered: Vec<Interaction> =
        interactions.into_iter().filter(|i| !is_noise_interaction(&i.prompt)).collect();
    for (i, interaction) in filtered.iter_mut().enumerate() {
        interaction.sequence = (i + 1) as u32;
        interaction.id = format!("{}-{}", session_id, interaction.sequence);
    }

    filtered
}

/// Returns true if this prompt is noise (not a real coding interaction).
fn is_noise_interaction(prompt: &str) -> bool {
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        return true;
    }

    // Short confirmations: y, ye, yes, ok, no, n, k
    if trimmed.len() <= 3 {
        let lower = trimmed.to_lowercase();
        if matches!(lower.as_str(), "y" | "ye" | "yes" | "ok" | "no" | "n" | "k") {
            return true;
        }
    }

    // Non-coding slash commands
    if trimmed.starts_with('/') {
        let cmd = trimmed.split_whitespace().next().unwrap_or("");
        let noise_commands = [
            "/clear", "/exit", "/help", "/compact", "/quit", "/logout", "/status", "/version",
            "/config", "/model", "/fast",
        ];
        if noise_commands.iter().any(|nc| cmd.eq_ignore_ascii_case(nc)) {
            return true;
        }
    }

    // Single-word non-coding prompts
    let lower = trimmed.to_lowercase();
    let noise_words = [
        "commit", "done", "thanks", "continue", "go", "stop", "lgtm", "ship", "ok", "okay", "sure",
        "yep", "nope", "thanks!", "ty",
    ];
    if !trimmed.contains(' ') && noise_words.iter().any(|w| lower == *w) {
        return true;
    }

    // Internal Claude Code tags
    if trimmed.starts_with("<command-name>")
        || trimmed.starts_with("<local-command-caveat>")
        || trimmed.starts_with("<task-notification>")
    {
        return true;
    }

    false
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
                    if let Some(tc) = tool_calls.iter_mut().find(|tc| tc.tool_id == tool_use_id) {
                        tc.output = event.content.clone();
                    }
                }
            }
            _ => {}
        }
    }

    let timestamp_end =
        response_events.last().map(|e| e.timestamp).unwrap_or(prompt_event.timestamp);

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
            make_event("user_prompt", Some("Second prompt"), now + Duration::seconds(5), None),
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
            make_event("assistant_response", Some("Stray response"), now, None),
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

    #[test]
    fn test_noise_interactions_filtered() {
        let now = Utc::now();
        let events = vec![
            make_event("user_prompt", Some("y"), now, None),
            make_event("assistant_response", Some("ok"), now + Duration::seconds(1), None),
            make_event("user_prompt", Some("Fix the real bug"), now + Duration::seconds(5), None),
            make_event("assistant_response", Some("Fixed"), now + Duration::seconds(6), None),
            make_event("user_prompt", Some("thanks"), now + Duration::seconds(10), None),
            make_event(
                "assistant_response",
                Some("You're welcome"),
                now + Duration::seconds(11),
                None,
            ),
        ];

        let interactions = build_interactions("test-session", &events);
        assert_eq!(interactions.len(), 1);
        assert_eq!(interactions[0].prompt, "Fix the real bug");
        assert_eq!(interactions[0].sequence, 1); // renumbered
    }

    #[test]
    fn test_noise_slash_commands_filtered() {
        let now = Utc::now();
        let events = vec![
            make_event("user_prompt", Some("/compact"), now, None),
            make_event("assistant_response", Some("Compacted"), now + Duration::seconds(1), None),
            make_event("user_prompt", Some("/clear"), now + Duration::seconds(5), None),
            make_event("assistant_response", Some("Cleared"), now + Duration::seconds(6), None),
            make_event(
                "user_prompt",
                Some("Add error handling to the parser"),
                now + Duration::seconds(10),
                None,
            ),
            make_event("assistant_response", Some("Done"), now + Duration::seconds(11), None),
        ];

        let interactions = build_interactions("test-session", &events);
        assert_eq!(interactions.len(), 1);
        assert_eq!(interactions[0].prompt, "Add error handling to the parser");
    }

    #[test]
    fn test_noise_internal_tags_filtered() {
        let now = Utc::now();
        let events = vec![
            make_event(
                "user_prompt",
                Some("<command-name>some internal thing</command-name>"),
                now,
                None,
            ),
            make_event("assistant_response", Some("handled"), now + Duration::seconds(1), None),
            make_event("user_prompt", Some("Refactor the tests"), now + Duration::seconds(5), None),
            make_event("assistant_response", Some("Refactored"), now + Duration::seconds(6), None),
        ];

        let interactions = build_interactions("test-session", &events);
        assert_eq!(interactions.len(), 1);
        assert_eq!(interactions[0].prompt, "Refactor the tests");
    }

    #[test]
    fn test_is_noise_interaction_fn() {
        // Noise
        assert!(is_noise_interaction("y"));
        assert!(is_noise_interaction("yes"));
        assert!(is_noise_interaction("ok"));
        assert!(is_noise_interaction("n"));
        assert!(is_noise_interaction("/clear"));
        assert!(is_noise_interaction("/help"));
        assert!(is_noise_interaction("/compact"));
        assert!(is_noise_interaction("done"));
        assert!(is_noise_interaction("thanks"));
        assert!(is_noise_interaction("continue"));
        assert!(is_noise_interaction("<command-name>foo</command-name>"));
        assert!(is_noise_interaction("<local-command-caveat>bar"));
        assert!(is_noise_interaction("<task-notification>something"));
        assert!(is_noise_interaction(""));
        assert!(is_noise_interaction("  "));

        // Not noise
        assert!(!is_noise_interaction("Fix the bug in upload.ts"));
        assert!(!is_noise_interaction("Add error handling"));
        assert!(!is_noise_interaction("/review the code")); // not a noise command
        assert!(!is_noise_interaction("yes please fix that bug"));
    }
}
