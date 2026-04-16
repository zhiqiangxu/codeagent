//! LlmCompaction: 使用 LLM 对历史对话进行摘要压缩。

use std::sync::Arc;

use async_trait::async_trait;
use forge_core::{
    ChatRequest, CompactionProvider, Content, Message, ModelProvider, Role, StreamEvent,
};

/// 使用 LLM 生成对话摘要的 CompactionProvider 实现。
pub struct LlmCompaction {
    model: Arc<dyn ModelProvider>,
}

impl LlmCompaction {
    pub fn new(model: Arc<dyn ModelProvider>) -> Self {
        Self { model }
    }

    /// 构建摘要 prompt（可在测试中验证格式）。
    pub fn build_prompt(messages: &[Message]) -> String {
        let mut conversation = String::new();
        for msg in messages {
            let role = match msg.role {
                Role::User => "User",
                Role::Assistant => "Assistant",
                Role::System => "System",
            };
            let content = match &msg.content {
                Content::Text(t) => t.as_str(),
                Content::ToolResult { output, .. } => output.as_str(),
            };
            conversation.push_str(&format!("{}: {}\n", role, content));

            for tc in &msg.tool_calls {
                conversation.push_str(&format!(
                    "  [tool_call: {}({})]\n",
                    tc.name, tc.arguments
                ));
            }
        }

        format!(
            "Please summarize the following conversation, preserving key information such as file paths, tool calls, and decisions made. Be concise.\n\n{}",
            conversation
        )
    }
}

#[async_trait]
impl CompactionProvider for LlmCompaction {
    async fn summarize(&self, messages: &[Message]) -> anyhow::Result<String> {
        if messages.is_empty() {
            return Ok(String::new());
        }

        let prompt = Self::build_prompt(messages);
        let request = ChatRequest::builder()
            .model("default")
            .messages(vec![Message {
                role: Role::User,
                content: Content::Text(prompt),
                tool_calls: vec![],
            }])
            .build();

        let mut stream = self.model.chat_stream(request).await?;
        let mut summary = String::new();

        while let Some(event) = stream.next().await {
            if let StreamEvent::Delta { content } = event {
                summary.push_str(&content);
            }
        }

        Ok(summary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_llm_compaction_prompt() {
        let messages = vec![
            Message {
                role: Role::User,
                content: Content::Text("read file /a.rs".into()),
                tool_calls: vec![],
            },
            Message {
                role: Role::Assistant,
                content: Content::Text("here is the content".into()),
                tool_calls: vec![forge_core::ToolCall {
                    id: "tc1".into(),
                    name: "read".into(),
                    arguments: serde_json::json!({"path": "/a.rs"}),
                }],
            },
        ];

        let prompt = LlmCompaction::build_prompt(&messages);
        assert!(prompt.contains("summarize the following conversation"));
        assert!(prompt.contains("User: read file /a.rs"));
        assert!(prompt.contains("[tool_call: read"));
    }

    #[test]
    fn test_llm_compaction_returns_summary() {
        // 此测试在集成测试中用 MockModel 验证
        // 这里只验证 build_prompt 不 panic
        let messages = vec![Message {
            role: Role::User,
            content: Content::Text("hello".into()),
            tool_calls: vec![],
        }];
        let prompt = LlmCompaction::build_prompt(&messages);
        assert!(!prompt.is_empty());
    }
}
