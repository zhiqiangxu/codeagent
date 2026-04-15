use async_trait::async_trait;
use forge_core::types::*;
use forge_core::traits::*;
use std::collections::VecDeque;
use std::sync::Mutex;
use tempfile::TempDir;

/// 创建一个带有基本文件结构的临时项目目录。
pub fn create_test_project() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(dir.path().join("lib.rs"), "pub fn hello() {}\n").unwrap();
    dir
}

/// 按剧本返回预设响应的 ModelProvider。
pub struct ScriptedModelProvider {
    responses: Mutex<VecDeque<Vec<StreamEvent>>>,
}

impl ScriptedModelProvider {
    pub fn new(responses: Vec<Vec<StreamEvent>>) -> Self {
        Self {
            responses: Mutex::new(VecDeque::from(responses)),
        }
    }
}

#[async_trait]
impl ModelProvider for ScriptedModelProvider {
    async fn chat_stream(&self, _req: ChatRequest) -> anyhow::Result<StreamResponse> {
        let events = self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| anyhow::anyhow!("ScriptedModelProvider: no more responses"))?;

        let (tx, rx) = tokio::sync::mpsc::channel(32);
        tokio::spawn(async move {
            for event in events {
                let _ = tx.send(event).await;
            }
        });

        Ok(StreamResponse::new(rx))
    }

    fn capabilities(&self) -> ModelCapabilities {
        ModelCapabilities {
            streaming: true,
            tool_use: true,
            vision: false,
            max_context_tokens: 128_000,
        }
    }

    fn token_counter(&self, messages: &[Message]) -> usize {
        messages
            .iter()
            .map(|m| match &m.content {
                Content::Text(t) => t.len() / 4,
                Content::ToolResult { output, .. } => output.len() / 4,
            })
            .sum()
    }
}

/// 自动放行所有工具请求。
pub struct AutoAllowPrompter;

#[async_trait]
impl RuntimePrompter for AutoAllowPrompter {
    async fn ask(&self, _tool_name: &str, _args: &str) -> PermissionDecision {
        PermissionDecision::Allow
    }
}

/// 自动拒绝所有工具请求。
pub struct AutoDenyPrompter;

#[async_trait]
impl RuntimePrompter for AutoDenyPrompter {
    async fn ask(&self, _tool_name: &str, _args: &str) -> PermissionDecision {
        PermissionDecision::Deny
    }
}

/// 断言 ToolOutput.content 包含指定字符串。
pub fn assert_tool_output_contains(output: &ToolOutput, expected: &str) {
    assert!(
        output.content.contains(expected),
        "Expected ToolOutput to contain '{}', got: '{}'",
        expected,
        output.content
    );
}

/// 断言消息列表中存在指定 role 和内容的消息。
pub fn assert_has_message(messages: &[Message], role: Role, contains: &str) {
    let found = messages.iter().any(|m| {
        m.role == role
            && match &m.content {
                Content::Text(t) => t.contains(contains),
                Content::ToolResult { output, .. } => output.contains(contains),
            }
    });
    assert!(
        found,
        "Expected message with role {:?} containing '{}' not found",
        role, contains
    );
}
