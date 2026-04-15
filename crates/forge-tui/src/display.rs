use forge_core::{Content, Message, Role, ToolCall};

/// 折叠长内容的最大字符数。
const COLLAPSE_THRESHOLD: usize = 200;

/// 用于 TUI 渲染的消息展示结构，与 forge-core 的 Message 解耦。
#[derive(Debug, Clone)]
pub struct DisplayMessage {
    pub role: String,
    pub content: String,
    pub is_tool_call: bool,
    pub is_collapsed: bool,
}

impl DisplayMessage {
    /// 从 forge-core Message 创建展示消息。
    pub fn from_message(msg: &Message) -> Self {
        let role = match msg.role {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::System => "system",
        };

        let content = match &msg.content {
            Content::Text(t) => t.clone(),
            Content::ToolResult { output, .. } => {
                if output.len() > COLLAPSE_THRESHOLD {
                    format!("{}... [truncated]", &output[..COLLAPSE_THRESHOLD])
                } else {
                    output.clone()
                }
            }
        };

        Self {
            role: role.to_string(),
            content,
            is_tool_call: false,
            is_collapsed: false,
        }
    }

    /// 从 ToolCall 创建展示消息，格式：[tool_name] key=value
    pub fn from_tool_call(call: &ToolCall) -> Self {
        let args_display = if let Some(obj) = call.arguments.as_object() {
            obj.iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect::<Vec<_>>()
                .join(" ")
        } else {
            call.arguments.to_string()
        };

        Self {
            role: "tool".to_string(),
            content: format!("[{}] {}", call.name, args_display),
            is_tool_call: true,
            is_collapsed: false,
        }
    }

    /// 从工具执行结果创建展示消息，长内容自动折叠。
    pub fn from_tool_result(tool_use_id: &str, output: &str) -> Self {
        let (content, collapsed) = if output.len() > COLLAPSE_THRESHOLD {
            (
                format!("{}... [truncated]", &output[..COLLAPSE_THRESHOLD]),
                true,
            )
        } else {
            (output.to_string(), false)
        };

        Self {
            role: format!("result:{}", tool_use_id),
            content,
            is_tool_call: false,
            is_collapsed: collapsed,
        }
    }

    /// 追加流式 Delta 内容。
    pub fn append_delta(&mut self, delta: &str) {
        self.content.push_str(delta);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_text_message() {
        let msg = Message {
            role: Role::Assistant,
            content: Content::Text("hi".into()),
            tool_calls: vec![],
        };
        let display = DisplayMessage::from_message(&msg);
        assert_eq!(display.role, "assistant");
        assert_eq!(display.content, "hi");
        assert!(!display.is_tool_call);
    }

    #[test]
    fn test_display_tool_call() {
        let call = ToolCall {
            id: "tc1".into(),
            name: "read".into(),
            arguments: serde_json::json!({"path": "/a"}),
        };
        let display = DisplayMessage::from_tool_call(&call);
        assert!(display.content.contains("[read]"));
        assert!(display.content.contains("path="));
        assert!(display.is_tool_call);
    }

    #[test]
    fn test_display_tool_result() {
        let long_content = "x".repeat(500);
        let display = DisplayMessage::from_tool_result("tc1", &long_content);
        assert!(display.is_collapsed);
        assert!(display.content.contains("[truncated]"));
        assert!(display.content.len() < 500);
    }

    #[test]
    fn test_display_streaming_delta() {
        let mut display = DisplayMessage {
            role: "assistant".into(),
            content: String::new(),
            is_tool_call: false,
            is_collapsed: false,
        };
        display.append_delta("hel");
        display.append_delta("lo");
        assert_eq!(display.content, "hello");
    }
}
