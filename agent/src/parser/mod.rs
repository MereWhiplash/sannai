use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;

// ---------------------------------------------------------------------------
// Parsed output types (what the rest of the app consumes)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum ParsedEvent {
    UserPrompt {
        session_id: String,
        uuid: String,
        timestamp: DateTime<Utc>,
        content: String,
        cwd: Option<String>,
        git_branch: Option<String>,
    },
    ToolResult {
        session_id: String,
        uuid: String,
        timestamp: DateTime<Utc>,
        tool_use_id: String,
        is_error: bool,
        content: Option<String>,
    },
    AssistantText {
        session_id: String,
        uuid: String,
        timestamp: DateTime<Utc>,
        text: String,
        model: Option<String>,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
    },
    ToolUse {
        session_id: String,
        uuid: String,
        timestamp: DateTime<Utc>,
        tool_name: String,
        tool_id: String,
        input: Value,
    },
    /// Emitted for queue-operation dequeue — signals a session is starting.
    SessionStart {
        session_id: String,
        timestamp: DateTime<Utc>,
    },
    /// Events we acknowledge but don't need to store.
    Ignored,
}

// ---------------------------------------------------------------------------
// Raw deserialization types (mirrors the actual JSONL format)
// ---------------------------------------------------------------------------

/// We deserialize into a generic Value first, then branch on `type`.
/// This avoids serde_json tag-dispatch issues with the varied shapes.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct RawCommon {
    #[serde(rename = "type")]
    event_type: String,
    #[serde(default)]
    uuid: Option<String>,
    timestamp: DateTime<Utc>,
    session_id: String,
    #[serde(default)]
    parent_uuid: Option<String>,
    #[serde(default)]
    is_sidechain: bool,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    git_branch: Option<String>,
    #[serde(default)]
    version: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct RawMessage {
    role: String,
    #[serde(default)]
    content: Value,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    usage: Option<RawUsage>,
}

#[derive(Debug, Deserialize)]
struct RawUsage {
    #[serde(default)]
    input_tokens: Option<u64>,
    #[serde(default)]
    output_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct RawToolResultBlock {
    #[serde(rename = "type")]
    block_type: String,
    tool_use_id: String,
    #[serde(default)]
    content: Option<Value>,
    #[serde(default)]
    is_error: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct RawContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    // Text block
    #[serde(default)]
    text: Option<String>,
    // Tool use block
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    input: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawQueueOp {
    operation: String,
    timestamp: DateTime<Utc>,
    session_id: String,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Parse a single JSONL line into zero or more ParsedEvents.
///
/// One line can produce multiple events (e.g. an assistant message containing
/// both a text block and a tool_use block). Returns an empty vec for events
/// we intentionally ignore (progress, system, queue enqueue, etc.).
pub fn parse_line(line: &str) -> Result<Vec<ParsedEvent>> {
    let line = line.trim();
    if line.is_empty() {
        return Ok(vec![]);
    }

    // First, peek at the type field to decide how to parse.
    let peek: Value = serde_json::from_str(line).context("Invalid JSON")?;
    let event_type = peek
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    match event_type {
        "queue-operation" => parse_queue_op(line),
        "user" => parse_user_event(line, &peek),
        "assistant" => parse_assistant_event(line, &peek),
        "progress" | "system" => Ok(vec![ParsedEvent::Ignored]),
        _ => {
            tracing::debug!("Unknown event type: {}", event_type);
            Ok(vec![ParsedEvent::Ignored])
        }
    }
}

/// Extract session ID from a JSONL filename.
/// "364c6e60-10ed-41cd-895d-d1322f76d091.jsonl" -> Some("364c6e60-...")
pub fn extract_session_id(filename: &str) -> Option<String> {
    filename.strip_suffix(".jsonl").map(|s| s.to_string())
}

// ---------------------------------------------------------------------------
// Internal parsing
// ---------------------------------------------------------------------------

fn parse_queue_op(line: &str) -> Result<Vec<ParsedEvent>> {
    let op: RawQueueOp = serde_json::from_str(line)?;
    if op.operation == "dequeue" {
        Ok(vec![ParsedEvent::SessionStart {
            session_id: op.session_id,
            timestamp: op.timestamp,
        }])
    } else {
        Ok(vec![ParsedEvent::Ignored])
    }
}

fn parse_user_event(line: &str, peek: &Value) -> Result<Vec<ParsedEvent>> {
    let common: RawCommon = serde_json::from_str(line)?;
    let message_val = peek.get("message").context("User event missing message")?;
    let message: RawMessage = serde_json::from_value(message_val.clone())?;

    let uuid = common.uuid.unwrap_or_default();

    match &message.content {
        // String content = user prompt
        Value::String(text) => Ok(vec![ParsedEvent::UserPrompt {
            session_id: common.session_id,
            uuid,
            timestamp: common.timestamp,
            content: text.clone(),
            cwd: common.cwd,
            git_branch: common.git_branch,
        }]),
        // Array content = tool result(s)
        Value::Array(blocks) => {
            let mut events = Vec::new();
            for block_val in blocks {
                if let Ok(block) =
                    serde_json::from_value::<RawToolResultBlock>(block_val.clone())
                {
                    if block.block_type == "tool_result" {
                        let content_str = block.content.as_ref().map(|v| match v {
                            Value::String(s) => s.clone(),
                            other => other.to_string(),
                        });
                        events.push(ParsedEvent::ToolResult {
                            session_id: common.session_id.clone(),
                            uuid: uuid.clone(),
                            timestamp: common.timestamp,
                            tool_use_id: block.tool_use_id,
                            is_error: block.is_error.unwrap_or(false),
                            content: content_str,
                        });
                    }
                }
            }
            if events.is_empty() {
                Ok(vec![ParsedEvent::Ignored])
            } else {
                Ok(events)
            }
        }
        _ => Ok(vec![ParsedEvent::Ignored]),
    }
}

fn parse_assistant_event(_line: &str, peek: &Value) -> Result<Vec<ParsedEvent>> {
    let common: RawCommon = serde_json::from_value(peek.clone())?;
    let message_val = peek
        .get("message")
        .context("Assistant event missing message")?;
    let message: RawMessage = serde_json::from_value(message_val.clone())?;

    let uuid = common.uuid.unwrap_or_default();
    let model = message.model.clone();
    let input_tokens = message.usage.as_ref().and_then(|u| u.input_tokens);
    let output_tokens = message.usage.as_ref().and_then(|u| u.output_tokens);

    let content_blocks = match &message.content {
        Value::Array(arr) => arr.clone(),
        _ => return Ok(vec![ParsedEvent::Ignored]),
    };

    let mut events = Vec::new();
    for block_val in &content_blocks {
        if let Ok(block) = serde_json::from_value::<RawContentBlock>(block_val.clone()) {
            match block.block_type.as_str() {
                "text" => {
                    if let Some(text) = block.text {
                        if !text.is_empty() {
                            events.push(ParsedEvent::AssistantText {
                                session_id: common.session_id.clone(),
                                uuid: uuid.clone(),
                                timestamp: common.timestamp,
                                text,
                                model: model.clone(),
                                input_tokens,
                                output_tokens,
                            });
                        }
                    }
                }
                "tool_use" => {
                    if let (Some(id), Some(name)) = (block.id, block.name) {
                        events.push(ParsedEvent::ToolUse {
                            session_id: common.session_id.clone(),
                            uuid: uuid.clone(),
                            timestamp: common.timestamp,
                            tool_name: name,
                            tool_id: id,
                            input: block.input.unwrap_or(Value::Null),
                        });
                    }
                }
                _ => {}
            }
        }
    }

    if events.is_empty() {
        Ok(vec![ParsedEvent::Ignored])
    } else {
        Ok(events)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_queue_dequeue() {
        let line = r#"{"type":"queue-operation","operation":"dequeue","timestamp":"2026-01-27T15:56:56.357Z","sessionId":"364c6e60-10ed-41cd-895d-d1322f76d091"}"#;
        let events = parse_line(line).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            ParsedEvent::SessionStart { session_id, .. } => {
                assert_eq!(session_id, "364c6e60-10ed-41cd-895d-d1322f76d091");
            }
            other => panic!("Expected SessionStart, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_queue_enqueue_ignored() {
        let line = r#"{"type":"queue-operation","operation":"enqueue","timestamp":"2026-01-27T15:56:56.357Z","sessionId":"abc"}"#;
        let events = parse_line(line).unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], ParsedEvent::Ignored));
    }

    #[test]
    fn test_parse_user_prompt() {
        let line = r#"{"parentUuid":null,"isSidechain":false,"userType":"external","cwd":"/Users/test/project","sessionId":"sess-1","version":"2.1.15","gitBranch":"main","type":"user","message":{"role":"user","content":"Add error handling"},"uuid":"u-1","timestamp":"2026-01-27T15:56:56.364Z","permissionMode":"default"}"#;
        let events = parse_line(line).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            ParsedEvent::UserPrompt {
                session_id,
                content,
                cwd,
                git_branch,
                ..
            } => {
                assert_eq!(session_id, "sess-1");
                assert_eq!(content, "Add error handling");
                assert_eq!(cwd.as_deref(), Some("/Users/test/project"));
                assert_eq!(git_branch.as_deref(), Some("main"));
            }
            other => panic!("Expected UserPrompt, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_tool_result() {
        let line = r#"{"parentUuid":"p-1","isSidechain":false,"userType":"external","cwd":"/Users/test","sessionId":"sess-1","version":"2.1.15","gitBranch":"","type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu_abc","type":"tool_result","content":"output text","is_error":false}]},"uuid":"u-2","timestamp":"2026-01-27T15:57:12.654Z","toolUseResult":{"stdout":"output text","stderr":"","interrupted":false,"isImage":false},"sourceToolAssistantUUID":"a-1"}"#;
        let events = parse_line(line).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            ParsedEvent::ToolResult {
                tool_use_id,
                is_error,
                content,
                ..
            } => {
                assert_eq!(tool_use_id, "toolu_abc");
                assert!(!is_error);
                assert_eq!(content.as_deref(), Some("output text"));
            }
            other => panic!("Expected ToolResult, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_assistant_text() {
        let line = r#"{"parentUuid":"p-1","isSidechain":false,"userType":"external","cwd":"/Users/test","sessionId":"sess-1","version":"2.1.15","gitBranch":"","message":{"model":"claude-opus-4-5-20251101","id":"msg_01","type":"message","role":"assistant","content":[{"type":"text","text":"Let me help you with that."}],"stop_reason":"end_turn","stop_sequence":null,"usage":{"input_tokens":100,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":50,"service_tier":"standard"}},"requestId":"req_01","type":"assistant","uuid":"a-1","timestamp":"2026-01-27T15:57:09.628Z"}"#;
        let events = parse_line(line).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            ParsedEvent::AssistantText {
                text,
                model,
                input_tokens,
                output_tokens,
                ..
            } => {
                assert_eq!(text, "Let me help you with that.");
                assert_eq!(model.as_deref(), Some("claude-opus-4-5-20251101"));
                assert_eq!(*input_tokens, Some(100));
                assert_eq!(*output_tokens, Some(50));
            }
            other => panic!("Expected AssistantText, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_tool_use() {
        let line = r#"{"parentUuid":"p-1","isSidechain":false,"userType":"external","cwd":"/Users/test","sessionId":"sess-1","version":"2.1.15","gitBranch":"","message":{"model":"claude-opus-4-5-20251101","id":"msg_01","type":"message","role":"assistant","content":[{"type":"tool_use","id":"toolu_01C7","name":"Bash","input":{"command":"ls -la","description":"List files"}}],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":2,"output_tokens":2}},"requestId":"req_01","type":"assistant","uuid":"a-2","timestamp":"2026-01-27T15:57:11.808Z"}"#;
        let events = parse_line(line).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            ParsedEvent::ToolUse {
                tool_name,
                tool_id,
                input,
                ..
            } => {
                assert_eq!(tool_name, "Bash");
                assert_eq!(tool_id, "toolu_01C7");
                assert_eq!(input["command"], "ls -la");
            }
            other => panic!("Expected ToolUse, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_assistant_mixed_content() {
        // Assistant message with both text and tool_use blocks
        let line = r#"{"parentUuid":"p-1","isSidechain":false,"userType":"external","cwd":"/Users/test","sessionId":"sess-1","version":"2.1.15","gitBranch":"","message":{"model":"claude-opus-4-5-20251101","id":"msg_01","type":"message","role":"assistant","content":[{"type":"text","text":"Let me check."},{"type":"tool_use","id":"toolu_02","name":"Read","input":{"file_path":"/tmp/test.rs"}}],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":10,"output_tokens":20}},"requestId":"req_02","type":"assistant","uuid":"a-3","timestamp":"2026-01-27T16:00:00.000Z"}"#;
        let events = parse_line(line).unwrap();
        assert_eq!(events.len(), 2);
        assert!(matches!(&events[0], ParsedEvent::AssistantText { .. }));
        assert!(matches!(&events[1], ParsedEvent::ToolUse { .. }));
    }

    #[test]
    fn test_parse_progress_ignored() {
        let line = r#"{"parentUuid":"p-1","isSidechain":false,"userType":"external","cwd":"/Users/test","sessionId":"sess-1","version":"2.1.15","gitBranch":"","slug":"test-slug","type":"progress","data":{"type":"mcp_progress","status":"started","serverName":"ec","toolName":"ec_search"},"toolUseID":"toolu_01","parentToolUseID":"toolu_01","uuid":"u-5","timestamp":"2026-01-27T15:57:12.691Z"}"#;
        let events = parse_line(line).unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], ParsedEvent::Ignored));
    }

    #[test]
    fn test_parse_system_ignored() {
        let line = r#"{"parentUuid":"p-1","isSidechain":false,"userType":"external","cwd":"/Users/test","sessionId":"sess-1","version":"2.1.15","gitBranch":"","slug":"test-slug","type":"system","subtype":"stop_hook_summary","hookCount":1,"hookInfos":[{"command":"callback"}],"hookErrors":[],"preventedContinuation":false,"stopReason":"","hasOutput":false,"level":"suggestion","timestamp":"2026-01-27T16:01:36.752Z","uuid":"u-6","toolUseID":"tid-1"}"#;
        let events = parse_line(line).unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], ParsedEvent::Ignored));
    }

    #[test]
    fn test_parse_empty_line() {
        let events = parse_line("").unwrap();
        assert!(events.is_empty());

        let events = parse_line("   ").unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn test_parse_invalid_json() {
        let result = parse_line("not json at all");
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_session_id() {
        assert_eq!(
            extract_session_id("364c6e60-10ed-41cd-895d-d1322f76d091.jsonl"),
            Some("364c6e60-10ed-41cd-895d-d1322f76d091".to_string())
        );
        assert_eq!(extract_session_id("no-extension"), None);
    }

    #[test]
    fn test_parse_multiple_tool_results() {
        let line = r#"{"parentUuid":"p-1","isSidechain":false,"userType":"external","cwd":"/Users/test","sessionId":"sess-1","version":"2.1.15","gitBranch":"","type":"user","message":{"role":"user","content":[{"tool_use_id":"t1","type":"tool_result","content":"ok1","is_error":false},{"tool_use_id":"t2","type":"tool_result","content":"ok2","is_error":true}]},"uuid":"u-multi","timestamp":"2026-01-27T16:00:00.000Z"}"#;
        let events = parse_line(line).unwrap();
        assert_eq!(events.len(), 2);
        match &events[0] {
            ParsedEvent::ToolResult {
                tool_use_id,
                is_error,
                ..
            } => {
                assert_eq!(tool_use_id, "t1");
                assert!(!is_error);
            }
            other => panic!("Expected ToolResult, got {:?}", other),
        }
        match &events[1] {
            ParsedEvent::ToolResult {
                tool_use_id,
                is_error,
                ..
            } => {
                assert_eq!(tool_use_id, "t2");
                assert!(is_error);
            }
            other => panic!("Expected ToolResult, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_tool_result_with_json_content() {
        // Some tool results have JSON object content, not string
        let line = r#"{"parentUuid":"p-1","isSidechain":false,"userType":"external","cwd":"/Users/test","sessionId":"sess-1","version":"2.1.15","gitBranch":"","type":"user","message":{"role":"user","content":[{"tool_use_id":"t3","type":"tool_result","content":"{\"memories\":[]}"}]},"uuid":"u-json","timestamp":"2026-01-27T16:00:00.000Z","toolUseResult":"{\"memories\":[]}","sourceToolAssistantUUID":"a-1"}"#;
        let events = parse_line(line).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            ParsedEvent::ToolResult {
                content, is_error, ..
            } => {
                assert_eq!(content.as_deref(), Some("{\"memories\":[]}"));
                assert!(!is_error);
            }
            other => panic!("Expected ToolResult, got {:?}", other),
        }
    }
}
