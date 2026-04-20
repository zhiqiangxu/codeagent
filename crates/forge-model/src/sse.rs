//! SSE 流解析：将各厂商的 SSE 事件解析为统一的 StreamEvent。

use forge_core::{StreamEvent, TokenUsage};
use serde_json::Value;

/// Anthropic SSE 流的有状态解析器。
/// 需要跨事件积累 tool_use 的 input_json_delta。
pub struct AnthropicSseParser {
    /// 当前正在构建的 tool call（如果有）
    current_tool_id: Option<String>,
    current_tool_name: Option<String>,
    current_tool_input: String,
}

impl AnthropicSseParser {
    pub fn new() -> Self {
        Self {
            current_tool_id: None,
            current_tool_name: None,
            current_tool_input: String::new(),
        }
    }

    /// 解析一个 SSE 事件，可能返回 0 或 1 个 StreamEvent。
    pub fn parse(&mut self, event_type: &str, data: &Value) -> Option<StreamEvent> {
        match event_type {
            "content_block_start" => {
                let block_type = data.pointer("/content_block/type").and_then(|v| v.as_str());
                if block_type == Some("tool_use") {
                    self.current_tool_id = data
                        .pointer("/content_block/id")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    self.current_tool_name = data
                        .pointer("/content_block/name")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    self.current_tool_input.clear();
                }
                None
            }
            "content_block_delta" => {
                // Text delta
                if let Some(text) = data.pointer("/delta/text").and_then(|v| v.as_str()) {
                    return Some(StreamEvent::Delta {
                        content: text.to_string(),
                    });
                }
                // Tool input JSON delta
                if let Some(partial) = data
                    .pointer("/delta/partial_json")
                    .and_then(|v| v.as_str())
                {
                    self.current_tool_input.push_str(partial);
                }
                None
            }
            "content_block_stop" => {
                // 如果我们正在积累 tool call，现在 emit 它
                if let (Some(id), Some(name)) =
                    (self.current_tool_id.take(), self.current_tool_name.take())
                {
                    let arguments = if self.current_tool_input.is_empty() {
                        Value::Object(serde_json::Map::new())
                    } else {
                        serde_json::from_str(&self.current_tool_input)
                            .unwrap_or(Value::Object(serde_json::Map::new()))
                    };
                    self.current_tool_input.clear();
                    return Some(StreamEvent::ToolCall {
                        id,
                        name,
                        arguments,
                    });
                }
                None
            }
            "message_delta" => {
                // usage info comes in message_delta
                let input = data
                    .pointer("/usage/input_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize;
                let output = data
                    .pointer("/usage/output_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize;
                if input > 0 || output > 0 {
                    // Don't emit Done here; wait for message_stop
                }
                None
            }
            "message_stop" => Some(StreamEvent::Done {
                usage: TokenUsage::default(),
            }),
            _ => None,
        }
    }
}

impl Default for AnthropicSseParser {
    fn default() -> Self {
        Self::new()
    }
}

/// 解析 Anthropic SSE 事件（无状态版本，保持向后兼容）。
pub fn parse_anthropic_sse(event_type: &str, data: &Value) -> Option<StreamEvent> {
    match event_type {
        "content_block_delta" => {
            let text = data
                .pointer("/delta/text")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if text.is_empty() {
                return None;
            }
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
        "message_stop" => Some(StreamEvent::Done {
            usage: TokenUsage::default(),
        }),
        _ => None,
    }
}

/// 解析 OpenAI SSE data 行（也适用于 Gemini 兼容模式）。
/// OpenAI tool calls 也是分块到达的：第一个 chunk 有 id+name，后续只有 arguments 片段。
pub fn parse_openai_sse(data: &str) -> Option<StreamEvent> {
    if data == "[DONE]" {
        return Some(StreamEvent::Done {
            usage: TokenUsage::default(),
        });
    }

    let json: Value = serde_json::from_str(data).ok()?;

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
            usage: TokenUsage::default(),
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
        let event = parse_anthropic_sse("message_stop", &json!({})).unwrap();
        assert!(matches!(event, StreamEvent::Done { .. }));
    }

    #[test]
    fn test_anthropic_stateful_tool_input() {
        let mut parser = AnthropicSseParser::new();

        // content_block_start: tool_use
        let r = parser.parse(
            "content_block_start",
            &json!({"content_block": {"type": "tool_use", "id": "tc1", "name": "read"}}),
        );
        assert!(r.is_none()); // not emitted yet

        // input_json_delta chunks
        parser.parse(
            "content_block_delta",
            &json!({"delta": {"type": "input_json_delta", "partial_json": "{\"path\":"}}),
        );
        parser.parse(
            "content_block_delta",
            &json!({"delta": {"type": "input_json_delta", "partial_json": "\"/a.rs\"}"}}),
        );

        // content_block_stop: emit complete tool call
        let event = parser.parse("content_block_stop", &json!({})).unwrap();
        match event {
            StreamEvent::ToolCall { id, name, arguments } => {
                assert_eq!(id, "tc1");
                assert_eq!(name, "read");
                assert_eq!(arguments["path"], "/a.rs");
            }
            _ => panic!("expected ToolCall"),
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
        assert!(matches!(event, StreamEvent::Done { .. }));
    }

    #[test]
    fn test_anthropic_error_429() {
        let event = parse_anthropic_sse("error", &json!({"error": {"type": "rate_limit_error"}}));
        assert!(event.is_none());
    }
}
