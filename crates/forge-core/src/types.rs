use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Content {
    Text(String),
    ToolResult {
        tool_use_id: String,
        output: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Content,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolOutput {
    pub content: String,
    pub is_error: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TokenBudget {
    pub max_tokens: usize,
    pub reserved: usize,
}

impl TokenBudget {
    pub fn available(&self) -> usize {
        self.max_tokens.saturating_sub(self.reserved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_serialization() {
        let msg = Message {
            role: Role::User,
            content: Content::Text("hi".into()),
            tool_calls: vec![],
        };
        let json = serde_json::to_string(&msg).unwrap();
        let restored: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, restored);
    }

    #[test]
    fn test_token_budget_available() {
        let budget = TokenBudget {
            max_tokens: 4096,
            reserved: 512,
        };
        assert_eq!(budget.available(), 3584);
    }

    #[test]
    fn test_token_budget_zero_reserved() {
        let budget = TokenBudget {
            max_tokens: 4096,
            reserved: 0,
        };
        assert_eq!(budget.available(), 4096);
    }

    #[test]
    fn test_tool_output_error_flag() {
        let output = ToolOutput {
            content: "err".into(),
            is_error: true,
        };
        assert!(output.is_error);
    }

    #[test]
    fn test_tool_call_from_json() {
        let json = r#"{"id":"1","name":"read","arguments":{"path":"/a"}}"#;
        let tc: ToolCall = serde_json::from_str(json).unwrap();
        assert_eq!(tc.id, "1");
        assert_eq!(tc.name, "read");
        assert_eq!(tc.arguments["path"], "/a");
    }
}
