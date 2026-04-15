//! 请求格式化：将统一的 ChatRequest 转换为各厂商的 API 格式。

use forge_core::{ChatRequest, Content, Message, Role};
use serde_json::{json, Value};

/// 将 ChatRequest 格式化为 Anthropic API 请求 body。
/// Anthropic 的 system 消息放在独立 `system` 字段，不在 messages 数组中。
pub fn format_anthropic(req: &ChatRequest) -> Value {
    let mut system_text = String::new();
    let mut messages = Vec::new();

    for msg in &req.messages {
        match msg.role {
            Role::System => {
                if let Content::Text(t) = &msg.content {
                    if !system_text.is_empty() {
                        system_text.push('\n');
                    }
                    system_text.push_str(t);
                }
            }
            Role::User | Role::Assistant => {
                let role = match msg.role {
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    _ => unreachable!(),
                };
                let content = match &msg.content {
                    Content::Text(t) => json!(t),
                    Content::ToolResult { tool_use_id, output } => {
                        json!([{"type": "tool_result", "tool_use_id": tool_use_id, "content": output}])
                    }
                };
                messages.push(json!({"role": role, "content": content}));
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
/// OpenAI 的 system 消息放在 messages 数组中。
pub fn format_openai(req: &ChatRequest) -> Value {
    let messages: Vec<Value> = req
        .messages
        .iter()
        .map(|msg| {
            let role = match msg.role {
                Role::System => "system",
                Role::User => "user",
                Role::Assistant => "assistant",
            };
            let content = match &msg.content {
                Content::Text(t) => json!(t),
                Content::ToolResult { output, .. } => json!(output),
            };
            json!({"role": role, "content": content})
        })
        .collect();

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
    use forge_core::ChatRequest;

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
        // system 在独立字段
        assert_eq!(body["system"], "you are helpful");
        // messages 中不包含 system
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["role"], "user");
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
        // system 在 messages 数组中
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[0]["content"], "sys");
        assert_eq!(msgs[1]["role"], "user");
    }

    #[test]
    fn test_openai_format_tool_definition() {
        let schema = json!({"type": "object"});
        let tool = format_openai_tool("read", "Read a file", &schema);
        assert!(tool.get("parameters").is_some());
        assert!(tool.get("input_schema").is_none());
    }
}
