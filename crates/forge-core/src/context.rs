use async_trait::async_trait;

use crate::traits::{CompactionProvider, ContextEngine, MemoryRetriever};
use crate::types::{Content, Message, Role, TokenBudget};

/// 最近保留的轮次数（user+assistant 为 1 轮，这里按消息数计：保留最近 6 条 = 3 轮）。
const RECENT_MESSAGES_TO_KEEP: usize = 6;

/// Phase 1 ContextEngine：简单截断策略。
///
/// - system prompt 始终存在（注入 FORGE.md 内容 + 工具定义）
/// - 最近 3 轮对话（6 条消息）始终保留
/// - 旧消息从头截断以适配 token 预算
pub struct SimpleContextEngine {
    retriever: Box<dyn MemoryRetriever>,
    compaction: Box<dyn CompactionProvider>,
    tool_schemas: Vec<serde_json::Value>,
    system_prompt: String,
    token_counter: Box<dyn Fn(&[Message]) -> usize + Send + Sync>,
}

impl SimpleContextEngine {
    pub fn new(
        retriever: Box<dyn MemoryRetriever>,
        compaction: Box<dyn CompactionProvider>,
        tool_schemas: Vec<serde_json::Value>,
        system_prompt: String,
        token_counter: Box<dyn Fn(&[Message]) -> usize + Send + Sync>,
    ) -> Self {
        Self {
            retriever,
            compaction,
            tool_schemas,
            system_prompt,
            token_counter,
        }
    }

    /// 构建 system prompt 消息，注入 FORGE.md 内容和工具定义。
    async fn build_system_message(&self) -> Message {
        let mut parts = vec![self.system_prompt.clone()];

        // 注入 FORGE.md 内容
        let chunks = self
            .retriever
            .retrieve("", crate::RetrieveOptions::default())
            .await;
        for chunk in &chunks {
            parts.push(format!("\n---\n{}", chunk.content));
        }

        // 注入工具定义
        if !self.tool_schemas.is_empty() {
            parts.push("\n## Available Tools\n".to_string());
            for schema in &self.tool_schemas {
                parts.push(serde_json::to_string_pretty(schema).unwrap_or_default());
            }
        }

        Message {
            role: Role::System,
            content: Content::Text(parts.join("\n")),
            tool_calls: vec![],
        }
    }
}

#[async_trait]
impl ContextEngine for SimpleContextEngine {
    async fn assemble(&self, messages: &[Message], budget: TokenBudget) -> Vec<Message> {
        let system_msg = self.build_system_message().await;

        // 过滤掉输入中的 System 消息（我们自己生成 system prompt）
        let non_system: Vec<&Message> = messages
            .iter()
            .filter(|m| m.role != Role::System)
            .collect();

        let available = budget.available();

        // 从最近的消息开始，尽量多保留
        let mut result = vec![system_msg];
        let system_tokens = (self.token_counter)(&result);

        if system_tokens >= available {
            // system prompt 已经超出预算，只返回 system
            return result;
        }

        let remaining_budget = available - system_tokens;

        // 保证最近 RECENT_MESSAGES_TO_KEEP 条消息，然后从旧到新尝试添加更多
        let total = non_system.len();
        let recent_start = total.saturating_sub(RECENT_MESSAGES_TO_KEEP);

        // 先确定最近消息
        let recent_msgs: Vec<Message> = non_system[recent_start..].iter().map(|m| (*m).clone()).collect();
        let recent_tokens = (self.token_counter)(&recent_msgs);

        if recent_tokens >= remaining_budget {
            // 即使只保留最近消息也超预算，但最近 3 轮是底线，必须保留
            result.extend(recent_msgs);
            return result;
        }

        // 还有预算空间，尝试从旧消息中尽量多保留
        let old_msgs = &non_system[..recent_start];
        let mut budget_left = remaining_budget - recent_tokens;
        let mut kept_old = Vec::new();

        // 从最近的旧消息开始向前添加（越近的旧消息越有价值）
        for msg in old_msgs.iter().rev() {
            let msg_tokens = (self.token_counter)(&[(*msg).clone()]);
            if msg_tokens <= budget_left {
                kept_old.push((*msg).clone());
                budget_left -= msg_tokens;
            } else {
                break;
            }
        }
        kept_old.reverse();

        result.extend(kept_old);
        result.extend(recent_msgs);
        result
    }

    async fn compact(&self, messages: &[Message], target: TokenBudget) -> Vec<Message> {
        let non_system: Vec<&Message> = messages
            .iter()
            .filter(|m| m.role != Role::System)
            .collect();

        let total = non_system.len();
        let recent_start = total.saturating_sub(RECENT_MESSAGES_TO_KEEP);

        // 最近消息始终保留
        let recent: Vec<Message> = non_system[recent_start..].iter().map(|m| (*m).clone()).collect();

        // 尝试用 CompactionProvider 压缩旧消息
        let old: Vec<Message> = non_system[..recent_start].iter().map(|m| (*m).clone()).collect();

        let mut result = Vec::new();

        if !old.is_empty() {
            let summary = self.compaction.summarize(&old).await.unwrap_or_default();
            if !summary.is_empty() {
                // 将摘要作为一条 System 消息插入
                result.push(Message {
                    role: Role::System,
                    content: Content::Text(format!("[对话历史摘要] {}", summary)),
                    tool_calls: vec![],
                });
            }
            // NoopCompaction 返回空字符串 → 旧消息直接丢弃
        }

        // 检查是否在预算内，如果不在，依然保留最近消息（底线）
        let candidate: Vec<Message> = result.iter().chain(recent.iter()).cloned().collect();
        let tokens = (self.token_counter)(&candidate);

        if tokens <= target.available() {
            return candidate;
        }

        // 超预算：丢弃摘要，只保留最近消息
        recent
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::noop::{NoopCompaction, NoopRetriever};

    fn char_counter(messages: &[Message]) -> usize {
        messages.iter().map(|m| match &m.content {
            Content::Text(t) => t.len(),
            Content::ToolResult { output, .. } => output.len(),
        }).sum()
    }

    #[tokio::test]
    async fn test_assemble_system_prompt_always() {
        let engine = SimpleContextEngine::new(
            Box::new(NoopRetriever),
            Box::new(NoopCompaction),
            vec![],
            "You are a helpful assistant.".into(),
            Box::new(char_counter),
        );

        let result = engine
            .assemble(&[], TokenBudget { max_tokens: 4096, reserved: 0 })
            .await;

        assert!(!result.is_empty());
        assert_eq!(result[0].role, Role::System);
    }

    #[tokio::test]
    async fn test_assemble_tool_definitions() {
        let tools = vec![
            serde_json::json!({"name": "read", "description": "Read a file", "parameters": {}}),
            serde_json::json!({"name": "write", "description": "Write a file", "parameters": {}}),
        ];

        let engine = SimpleContextEngine::new(
            Box::new(NoopRetriever),
            Box::new(NoopCompaction),
            tools,
            "You are a helpful assistant.".into(),
            Box::new(char_counter),
        );

        let result = engine
            .assemble(&[], TokenBudget { max_tokens: 4096, reserved: 0 })
            .await;

        let system_text = match &result[0].content {
            Content::Text(t) => t.clone(),
            _ => panic!("expected text"),
        };
        assert!(system_text.contains("read"));
        assert!(system_text.contains("write"));
        assert!(system_text.contains("Available Tools"));
    }
}
