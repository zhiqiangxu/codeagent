//! SSE 流解析：将各厂商的 SSE 事件解析为统一的 StreamEvent。

use forge_core::{StreamEvent, TokenUsage};
use serde_json::Value;

/// 解析 Anthropic SSE 事件。
pub fn parse_anthropic_sse(event_type: &str, data: &Value) -> Option<StreamEvent> {
    match event_type {
        "content_block_delta" => {
            let text = data
                .pointer("/delta/text")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            Some(StreamEvent::Delta {
                content: text.to_string(),
            })
        }
        "content_block_start" => {
            let block_type = data.pointer("/content_block/type").and_then(|v| v.as_str());
            if block_type == Some("tool_use") {
                let id = data
                    .pointer("/content_block/id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let name = data
                    .pointer("/content_block/name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                Some(StreamEvent::ToolCall {
                    id,
                    name,
                    arguments: serde_json::Value::Null,
                })
            } else {
                None
            }
        }
        "message_stop" => {
            let input = data
                .pointer("/usage/input_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            let output = data
                .pointer("/usage/output_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            Some(StreamEvent::Done {
                usage: TokenUsage { input, output },
            })
        }
        _ => None,
    }
}

/// 解析 OpenAI SSE data 行（也适用于 Gemini 兼容模式）。
pub fn parse_openai_sse(data: &str) -> Option<StreamEvent> {
    if data == "[DONE]" {
        return Some(StreamEvent::Done {
            usage: TokenUsage::default(),
        });
    }

    let json: Value = serde_json::from_str(data).ok()?;

    // 检查 usage（如果有的话）
    let usage = json.get("usage").and_then(|u| {
        let input = u.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let output = u
            .get("completion_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        Some(TokenUsage { input, output })
    });

    let choice = json.pointer("/choices/0")?;
    let delta = choice.get("delta")?;

    // Tool call
    if let Some(tool_calls) = delta.get("tool_calls") {
        if let Some(tc) = tool_calls.get(0) {
            let id = tc
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let name = tc
                .pointer("/function/name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let args_str = tc
                .pointer("/function/arguments")
                .and_then(|v| v.as_str())
                .unwrap_or("{}");
            let arguments = serde_json::from_str(args_str).unwrap_or(Value::Null);
            return Some(StreamEvent::ToolCall {
                id,
                name,
                arguments,
            });
        }
    }

    // Text delta
    if let Some(content) = delta.get("content").and_then(|v| v.as_str()) {
        if !content.is_empty() {
            return Some(StreamEvent::Delta {
                content: content.to_string(),
            });
        }
    }

    // finish_reason == stop
    if choice.get("finish_reason").and_then(|v| v.as_str()) == Some("stop") {
        return Some(StreamEvent::Done {
            usage: usage.unwrap_or_default(),
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // -- Anthropic SSE tests --

    #[test]
    fn test_anthropic_parse_stream_delta() {
        let data = json!({"delta": {"text": "hi"}});
        let event = parse_anthropic_sse("content_block_delta", &data).unwrap();
        match event {
            StreamEvent::Delta { content } => assert_eq!(content, "hi"),
            _ => panic!("expected Delta"),
        }
    }

    #[test]
    fn test_anthropic_parse_stream_tool_use() {
        let data = json!({
            "content_block": {
                "type": "tool_use",
                "id": "tool_1",
                "name": "read",
            }
        });
        let event = parse_anthropic_sse("content_block_start", &data).unwrap();
        match event {
            StreamEvent::ToolCall { id, name, .. } => {
                assert_eq!(id, "tool_1");
                assert_eq!(name, "read");
            }
            _ => panic!("expected ToolCall"),
        }
    }

    #[test]
    fn test_anthropic_parse_stream_done() {
        let data = json!({"usage": {"input_tokens": 10, "output_tokens": 5}});
        let event = parse_anthropic_sse("message_stop", &data).unwrap();
        match event {
            StreamEvent::Done { usage } => {
                assert_eq!(usage.input, 10);
                assert_eq!(usage.output, 5);
            }
            _ => panic!("expected Done"),
        }
    }

    // -- OpenAI SSE tests --

    #[test]
    fn test_openai_parse_stream_delta() {
        let data = r#"{"choices":[{"delta":{"content":"hi"},"index":0}]}"#;
        let event = parse_openai_sse(data).unwrap();
        match event {
            StreamEvent::Delta { content } => assert_eq!(content, "hi"),
            _ => panic!("expected Delta"),
        }
    }

    #[test]
    fn test_openai_parse_stream_tool_call() {
        let data = r#"{"choices":[{"delta":{"tool_calls":[{"id":"tc_1","function":{"name":"read","arguments":"{}"}}]},"index":0}]}"#;
        let event = parse_openai_sse(data).unwrap();
        match event {
            StreamEvent::ToolCall { id, name, .. } => {
                assert_eq!(id, "tc_1");
                assert_eq!(name, "read");
            }
            _ => panic!("expected ToolCall"),
        }
    }

    #[test]
    fn test_openai_parse_stream_done() {
        let event = parse_openai_sse("[DONE]").unwrap();
        match event {
            StreamEvent::Done { .. } => {}
            _ => panic!("expected Done"),
        }
    }

    // -- Error mapping tests (just testing parse behavior) --

    #[test]
    fn test_anthropic_error_429() {
        // 429 errors would come from HTTP response, not SSE
        // Here we verify unknown event types return None
        let event = parse_anthropic_sse("error", &json!({"error": {"type": "rate_limit_error"}}));
        assert!(event.is_none());
    }
}
