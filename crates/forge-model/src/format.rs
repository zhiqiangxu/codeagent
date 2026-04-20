//! 请求格式化：将统一的 ChatRequest 转换为各厂商的 API 格式。

use forge_core::{ChatRequest, Content, Role};
use serde_json::{json, Value};

/// 将 ChatRequest 格式化为 Anthropic API 请求 body。
///
/// Anthropic 格式要求：
/// - system 消息在独立 `system` 字段
/// - assistant 含 tool_use 时 content 必须是数组: [{"type":"text",...}, {"type":"tool_use",...}]
/// - 多个 tool_result 合并到一个 user 消息的 content 数组中
/// - tool_result 紧跟对应的 assistant 消息
pub fn format_anthropic(req: &ChatRequest) -> Value {
    let mut system_text = String::new();
    let mut messages: Vec<Value> = Vec::new();

    let mut i = 0;
    while i < req.messages.len() {
        let msg = &req.messages[i];
        match msg.role {
            Role::System => {
                if let Content::Text(t) = &msg.content {
                    if !system_text.is_empty() {
                        system_text.push('\n');
                    }
                    system_text.push_str(t);
                }
                i += 1;
            }
            Role::Assistant => {
                // Build content blocks array
                let mut content_blocks = Vec::new();

                // Text content (if any)
                if let Content::Text(t) = &msg.content {
                    if !t.is_empty() {
                        content_blocks.push(json!({"type": "text", "text": t}));
                    }
                }

                // Tool use blocks
                for tc in &msg.tool_calls {
                    content_blocks.push(json!({
                        "type": "tool_use",
                        "id": tc.id,
                        "name": tc.name,
                        "input": tc.arguments,
                    }));
                }

                if content_blocks.is_empty() {
                    content_blocks.push(json!({"type": "text", "text": ""}));
                }

                messages.push(json!({"role": "assistant", "content": content_blocks}));
                i += 1;

                // Collect consecutive tool_results into one user message
                let mut tool_results = Vec::new();
                while i < req.messages.len() {
                    if let Content::ToolResult { tool_use_id, output } = &req.messages[i].content {
                        tool_results.push(json!({
                            "type": "tool_result",
                            "tool_use_id": tool_use_id,
                            "content": output,
                        }));
                        i += 1;
                    } else {
                        break;
                    }
                }
                if !tool_results.is_empty() {
                    messages.push(json!({"role": "user", "content": tool_results}));
                }
            }
            Role::User => {
                match &msg.content {
                    Content::Text(t) => {
                        messages.push(json!({"role": "user", "content": t}));
                    }
                    Content::ToolResult { tool_use_id, output } => {
                        // Standalone tool result (shouldn't happen normally, but handle gracefully)
                        messages.push(json!({
                            "role": "user",
                            "content": [{"type": "tool_result", "tool_use_id": tool_use_id, "content": output}]
                        }));
                    }
                }
                i += 1;
            }
        }
    }

    let mut body = json!({
        "model": req.model,
        "messages": messages,
    });

    if !system_text.is_empty() {
        body["system"] = json!(system_text);
    }
    if !req.tools.is_empty() {
        body["tools"] = json!(req.tools);
    }
    if let Some(t) = req.temperature {
        body["temperature"] = json!(t);
    }
    if let Some(n) = req.max_tokens {
        body["max_tokens"] = json!(n);
    }

    body
}

/// 将 ChatRequest 格式化为 OpenAI API 请求 body。
pub fn format_openai(req: &ChatRequest) -> Value {
    let mut messages: Vec<Value> = Vec::new();

    for msg in &req.messages {
        let role = match msg.role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
        };

        match &msg.content {
            Content::Text(t) => {
                let mut m = json!({"role": role, "content": t});
                // OpenAI tool calls in assistant message
                if !msg.tool_calls.is_empty() {
                    let tool_calls: Vec<Value> = msg
                        .tool_calls
                        .iter()
                        .map(|tc| {
                            json!({
                                "id": tc.id,
                                "type": "function",
                                "function": {
                                    "name": tc.name,
                                    "arguments": tc.arguments.to_string(),
                                }
                            })
                        })
                        .collect();
                    m["tool_calls"] = json!(tool_calls);
                }
                messages.push(m);
            }
            Content::ToolResult { tool_use_id, output } => {
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": tool_use_id,
                    "content": output,
                }));
            }
        }
    }

    let mut body = json!({
        "model": req.model,
        "messages": messages,
        "stream": true,
    });

    if !req.tools.is_empty() {
        let tools: Vec<Value> = req
            .tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": t,
                })
            })
            .collect();
        body["tools"] = json!(tools);
    }
    if let Some(t) = req.temperature {
        body["temperature"] = json!(t);
    }
    if let Some(n) = req.max_tokens {
        body["max_tokens"] = json!(n);
    }

    body
}

/// Anthropic 工具定义格式：使用 `input_schema` 字段。
pub fn format_anthropic_tool(name: &str, description: &str, schema: &Value) -> Value {
    json!({
        "name": name,
        "description": description,
        "input_schema": schema,
    })
}

/// OpenAI 工具定义格式：使用 `parameters` 字段。
pub fn format_openai_tool(name: &str, description: &str, schema: &Value) -> Value {
    json!({
        "name": name,
        "description": description,
        "parameters": schema,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_core::{ChatRequest, Content, Message, ToolCall};

    fn user_msg(text: &str) -> Message {
        Message {
            role: Role::User,
            content: Content::Text(text.into()),
            tool_calls: vec![],
        }
    }

    fn system_msg(text: &str) -> Message {
        Message {
            role: Role::System,
            content: Content::Text(text.into()),
            tool_calls: vec![],
        }
    }

    #[test]
    fn test_anthropic_format_request() {
        let req = ChatRequest::builder()
            .model("claude-sonnet-4-20250514")
            .messages(vec![user_msg("hi")])
            .build();
        let body = format_anthropic(&req);
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[0]["content"], "hi");
    }

    #[test]
    fn test_anthropic_format_system() {
        let req = ChatRequest::builder()
            .model("claude-sonnet-4-20250514")
            .messages(vec![system_msg("you are helpful"), user_msg("hi")])
            .build();
        let body = format_anthropic(&req);
        assert_eq!(body["system"], "you are helpful");
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["role"], "user");
    }

    #[test]
    fn test_anthropic_format_tool_use_and_result() {
        // assistant with tool call → user with tool result
        let messages = vec![
            user_msg("read /a.rs"),
            Message {
                role: Role::Assistant,
                content: Content::Text("".into()),
                tool_calls: vec![ToolCall {
                    id: "tc1".into(),
                    name: "read".into(),
                    arguments: json!({"path": "/a.rs"}),
                }],
            },
            Message {
                role: Role::User,
                content: Content::ToolResult {
                    tool_use_id: "tc1".into(),
                    output: "file contents".into(),
                },
                tool_calls: vec![],
            },
        ];

        let req = ChatRequest::builder()
            .model("claude-sonnet-4-20250514")
            .messages(messages)
            .build();
        let body = format_anthropic(&req);
        let msgs = body["messages"].as_array().unwrap();

        // msg 0: user text
        assert_eq!(msgs[0]["role"], "user");
        // msg 1: assistant with tool_use content block
        assert_eq!(msgs[1]["role"], "assistant");
        let content = msgs[1]["content"].as_array().unwrap();
        assert!(content.iter().any(|c| c["type"] == "tool_use"));
        assert_eq!(content.iter().find(|c| c["type"] == "tool_use").unwrap()["name"], "read");
        // msg 2: user with tool_result
        assert_eq!(msgs[2]["role"], "user");
        let results = msgs[2]["content"].as_array().unwrap();
        assert_eq!(results[0]["type"], "tool_result");
        assert_eq!(results[0]["tool_use_id"], "tc1");
    }

    #[test]
    fn test_anthropic_format_tool_definition() {
        let schema = json!({"type": "object", "properties": {"path": {"type": "string"}}});
        let tool = format_anthropic_tool("read", "Read a file", &schema);
        assert!(tool.get("input_schema").is_some());
        assert_eq!(tool["name"], "read");
    }

    #[test]
    fn test_openai_format_request() {
        let req = ChatRequest::builder()
            .model("gpt-4o")
            .messages(vec![system_msg("sys"), user_msg("hi")])
            .build();
        let body = format_openai(&req);
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[0]["content"], "sys");
        assert_eq!(msgs[1]["role"], "user");
    }

    #[test]
    fn test_openai_format_tool_result() {
        let messages = vec![
            Message {
                role: Role::User,
                content: Content::ToolResult {
                    tool_use_id: "call_1".into(),
                    output: "result".into(),
                },
                tool_calls: vec![],
            },
        ];
        let req = ChatRequest::builder()
            .model("gpt-4o")
            .messages(messages)
            .build();
        let body = format_openai(&req);
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs[0]["role"], "tool");
        assert_eq!(msgs[0]["tool_call_id"], "call_1");
    }

    #[test]
    fn test_openai_format_tool_definition() {
        let schema = json!({"type": "object"});
        let tool = format_openai_tool("read", "Read a file", &schema);
        assert!(tool.get("parameters").is_some());
        assert!(tool.get("input_schema").is_none());
    }
}
