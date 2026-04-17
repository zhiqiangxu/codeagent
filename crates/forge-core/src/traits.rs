use async_trait::async_trait;
use std::path::PathBuf;

use crate::types::*;

// ──────────────────────────────────────────────────────
// 1. ModelProvider — 谁来回答（模型调用）
// ──────────────────────────────────────────────────────

/// 流式响应中的单个事件。
#[derive(Debug, Clone)]
pub enum StreamEvent {
    Delta { content: String },
    ToolCall {
        id: String,
        name: String,
        arguments: serde_json::Value,
    },
    Done { usage: TokenUsage },
}

/// Token 使用量统计。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TokenUsage {
    pub input: usize,
    pub output: usize,
}

impl std::ops::Add for TokenUsage {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        TokenUsage {
            input: self.input + rhs.input,
            output: self.output + rhs.output,
        }
    }
}

/// 模型能力描述。
#[derive(Debug, Clone)]
pub struct ModelCapabilities {
    pub streaming: bool,
    pub tool_use: bool,
    pub vision: bool,
    pub max_context_tokens: usize,
}

/// 流式响应，通过 `next()` 逐块读取事件。
pub struct StreamResponse {
    receiver: tokio::sync::mpsc::Receiver<StreamEvent>,
}

impl StreamResponse {
    pub fn new(receiver: tokio::sync::mpsc::Receiver<StreamEvent>) -> Self {
        Self { receiver }
    }

    pub async fn next(&mut self) -> Option<StreamEvent> {
        self.receiver.recv().await
    }
}

/// 发给 LLM 的完整请求体。
#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub tools: Vec<serde_json::Value>,
    pub temperature: Option<f64>,
    pub max_tokens: Option<usize>,
}

impl ChatRequest {
    pub fn builder() -> ChatRequestBuilder {
        ChatRequestBuilder::default()
    }
}

#[derive(Default)]
pub struct ChatRequestBuilder {
    model: String,
    messages: Vec<Message>,
    tools: Vec<serde_json::Value>,
    temperature: Option<f64>,
    max_tokens: Option<usize>,
}

impl ChatRequestBuilder {
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }
    pub fn messages(mut self, messages: Vec<Message>) -> Self {
        self.messages = messages;
        self
    }
    pub fn tools(mut self, tools: Vec<serde_json::Value>) -> Self {
        self.tools = tools;
        self
    }
    pub fn temperature(mut self, t: f64) -> Self {
        self.temperature = Some(t);
        self
    }
    pub fn max_tokens(mut self, n: usize) -> Self {
        self.max_tokens = Some(n);
        self
    }
    pub fn build(self) -> ChatRequest {
        ChatRequest {
            model: self.model,
            messages: self.messages,
            tools: self.tools,
            temperature: self.temperature,
            max_tokens: self.max_tokens,
        }
    }
}

#[async_trait]
pub trait ModelProvider: Send + Sync {
    async fn chat_stream(&self, req: ChatRequest) -> anyhow::Result<StreamResponse>;
    fn capabilities(&self) -> ModelCapabilities;
    fn token_counter(&self, messages: &[Message]) -> usize;
}

#[async_trait]
impl ModelProvider for Box<dyn ModelProvider> {
    async fn chat_stream(&self, req: ChatRequest) -> anyhow::Result<StreamResponse> {
        (**self).chat_stream(req).await
    }
    fn capabilities(&self) -> ModelCapabilities {
        (**self).capabilities()
    }
    fn token_counter(&self, messages: &[Message]) -> usize {
        (**self).token_counter(messages)
    }
}

// ──────────────────────────────────────────────────────
// 2. ContextEngine — 看到什么（上下文编排）
// ──────────────────────────────────────────────────────

#[async_trait]
pub trait ContextEngine: Send + Sync {
    async fn assemble(&self, messages: &[Message], budget: TokenBudget) -> Vec<Message>;
    async fn compact(&self, messages: &[Message], target: TokenBudget) -> Vec<Message>;
}

// ──────────────────────────────────────────────────────
// 3. CompactionProvider — 怎么压缩（历史压缩策略）
// ──────────────────────────────────────────────────────

#[async_trait]
pub trait CompactionProvider: Send + Sync {
    async fn summarize(&self, messages: &[Message]) -> anyhow::Result<String>;
}

// ──────────────────────────────────────────────────────
// 4. MemoryRetriever — 记住什么（记忆检索）
// ──────────────────────────────────────────────────────

/// 检索到的记忆片段。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MemoryChunk {
    pub content: String,
    pub source: Option<String>,
    pub score: f32,
}

/// 检索选项。
#[derive(Debug, Clone, Default)]
pub struct RetrieveOptions {
    pub top_k: Option<usize>,
    pub min_score: Option<f32>,
    pub scope: Option<String>,
}

#[async_trait]
pub trait MemoryRetriever: Send + Sync {
    async fn retrieve(&self, query: &str, opts: RetrieveOptions) -> Vec<MemoryChunk>;
    async fn index(&self, files: &[PathBuf]) -> anyhow::Result<()>;
}

// ──────────────────────────────────────────────────────
// 5. EmbeddingProvider — 怎么向量化（文本嵌入）
// ──────────────────────────────────────────────────────

#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>>;
    fn dimension(&self) -> usize;
}

// ──────────────────────────────────────────────────────
// 6. RuntimePrompter — 问不问用户（权限交互）
// ──────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionDecision {
    Allow,
    AlwaysAllow,
    Deny,
}

#[async_trait]
pub trait RuntimePrompter: Send + Sync {
    async fn ask(&self, tool_name: &str, args: &str) -> PermissionDecision;
}

// ──────────────────────────────────────────────────────
// 7. Tool — 能做什么（工具扩展）
// ──────────────────────────────────────────────────────

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> serde_json::Value;
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolOutput>;
}

// ──────────────────────────────────────────────────────
// 8. ToolExecutor — 工具执行器（封装 Tool + 权限检查）
// ──────────────────────────────────────────────────────

#[async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn execute(&self, call: &ToolCall) -> anyhow::Result<ToolOutput>;
    fn tool_schemas(&self) -> Vec<serde_json::Value>;
}

// ──────────────────────────────────────────────────────
// 9. SessionStore — 会话持久化
// ──────────────────────────────────────────────────────

#[async_trait]
pub trait SessionStore: Send + Sync {
    async fn save(&self, messages: &[Message]) -> anyhow::Result<()>;
}

// ──────────────────────────────────────────────────────
// 10. ModelError — 模型调用错误分类
// ──────────────────────────────────────────────────────

/// 区分暂时性错误（可重试）和致命错误（不可重试）。
#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    #[error("transient error ({status}): {message}")]
    Transient { status: u16, message: String },
    #[error("auth error ({status}): {message}")]
    Auth { status: u16, message: String },
}
