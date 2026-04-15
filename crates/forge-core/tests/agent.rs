use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use async_trait::async_trait;
use forge_core::{
    AgentEvent, AgentLoop, ChatRequest, Content, ContextEngine, Message, ModelCapabilities,
    ModelError, ModelProvider, Role, SessionStore, StreamEvent, StreamResponse, TokenBudget,
    TokenUsage, ToolCall, ToolExecutor, ToolOutput,
};

// ═══════════════════════════════════════════════════════
// Mock Implementations
// ═══════════════════════════════════════════════════════

// ─── MockModel ─────────────────────────────────────────

enum MockModelResponse {
    Events(Vec<StreamEvent>),
    Transient(u16, String),
    AuthError(u16, String),
}

struct MockModel {
    responses: Mutex<VecDeque<MockModelResponse>>,
    max_context_tokens: usize,
}

impl MockModel {
    fn new(responses: Vec<MockModelResponse>) -> Self {
        Self {
            responses: Mutex::new(VecDeque::from(responses)),
            max_context_tokens: 8192,
        }
    }

    fn with_context_limit(mut self, limit: usize) -> Self {
        self.max_context_tokens = limit;
        self
    }
}

#[async_trait]
impl ModelProvider for MockModel {
    async fn chat_stream(&self, _req: ChatRequest) -> anyhow::Result<StreamResponse> {
        let response = self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("MockModel: no more responses");

        match response {
            MockModelResponse::Events(events) => {
                let (tx, rx) = tokio::sync::mpsc::channel(32);
                tokio::spawn(async move {
                    for event in events {
                        let _ = tx.send(event).await;
                    }
                });
                Ok(StreamResponse::new(rx))
            }
            MockModelResponse::Transient(status, message) => {
                Err(ModelError::Transient { status, message }.into())
            }
            MockModelResponse::AuthError(status, message) => {
                Err(ModelError::Auth { status, message }.into())
            }
        }
    }

    fn capabilities(&self) -> ModelCapabilities {
        ModelCapabilities {
            streaming: true,
            tool_use: true,
            vision: false,
            max_context_tokens: self.max_context_tokens,
        }
    }

    fn token_counter(&self, messages: &[Message]) -> usize {
        // 简单的字符计数器作为 token 近似
        messages
            .iter()
            .map(|m| match &m.content {
                Content::Text(t) => t.len(),
                Content::ToolResult { output, .. } => output.len(),
            })
            .sum()
    }
}

// ─── PassthroughContext ────────────────────────────────

struct PassthroughContext;

#[async_trait]
impl ContextEngine for PassthroughContext {
    async fn assemble(&self, messages: &[Message], _budget: TokenBudget) -> Vec<Message> {
        messages.to_vec()
    }
    async fn compact(&self, messages: &[Message], _target: TokenBudget) -> Vec<Message> {
        messages.to_vec()
    }
}

// ─── OverflowContext ──────────────────────────────────

struct OverflowContext {
    assemble_count: AtomicUsize,
}

#[async_trait]
impl ContextEngine for OverflowContext {
    async fn assemble(&self, messages: &[Message], _budget: TokenBudget) -> Vec<Message> {
        let count = self.assemble_count.fetch_add(1, Ordering::SeqCst);
        if count == 0 {
            // 第一次：返回超大内容（触发 overflow 检测）
            let mut result = messages.to_vec();
            result.push(Message {
                role: Role::System,
                content: Content::Text("x".repeat(10000)),
                tool_calls: vec![],
            });
            result
        } else {
            // compact 之后：正常返回
            messages.to_vec()
        }
    }
    async fn compact(&self, _messages: &[Message], _target: TokenBudget) -> Vec<Message> {
        // 返回精简的消息
        vec![Message {
            role: Role::User,
            content: Content::Text("compacted".into()),
            tool_calls: vec![],
        }]
    }
}

// ─── MockTools ─────────────────────────────────────────

struct MockTools {
    results: Mutex<VecDeque<anyhow::Result<ToolOutput>>>,
}

impl MockTools {
    fn new(results: Vec<anyhow::Result<ToolOutput>>) -> Self {
        Self {
            results: Mutex::new(VecDeque::from(results)),
        }
    }
}

#[async_trait]
impl ToolExecutor for MockTools {
    async fn execute(&self, _call: &ToolCall) -> anyhow::Result<ToolOutput> {
        self.results
            .lock()
            .unwrap()
            .pop_front()
            .expect("MockTools: no more results")
    }

    fn tool_schemas(&self) -> Vec<serde_json::Value> {
        vec![]
    }
}

struct EmptyTools;

#[async_trait]
impl ToolExecutor for EmptyTools {
    async fn execute(&self, _call: &ToolCall) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput {
            content: "ok".into(),
            is_error: false,
        })
    }
    fn tool_schemas(&self) -> Vec<serde_json::Value> {
        vec![]
    }
}

// ─── MockSessionStore ─────────────────────────────────

struct MockSessionStore {
    saved: Mutex<Vec<Vec<Message>>>,
}

impl MockSessionStore {
    fn new() -> Self {
        Self {
            saved: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl SessionStore for MockSessionStore {
    async fn save(&self, messages: &[Message]) -> anyhow::Result<()> {
        self.saved.lock().unwrap().push(messages.to_vec());
        Ok(())
    }
}

// ─── Helpers ───────────────────────────────────────────

fn done_event() -> StreamEvent {
    StreamEvent::Done {
        usage: TokenUsage {
            input: 10,
            output: 5,
        },
    }
}

fn text_events(chunks: &[&str]) -> Vec<StreamEvent> {
    let mut events: Vec<StreamEvent> = chunks
        .iter()
        .map(|c| StreamEvent::Delta {
            content: c.to_string(),
        })
        .collect();
    events.push(done_event());
    events
}

fn tool_call_events(calls: Vec<(&str, &str, serde_json::Value)>) -> Vec<StreamEvent> {
    let mut events: Vec<StreamEvent> = calls
        .into_iter()
        .map(|(id, name, args)| StreamEvent::ToolCall {
            id: id.to_string(),
            name: name.to_string(),
            arguments: args,
        })
        .collect();
    events.push(done_event());
    events
}

fn make_channel() -> (
    tokio::sync::mpsc::UnboundedSender<AgentEvent>,
    tokio::sync::mpsc::UnboundedReceiver<AgentEvent>,
) {
    tokio::sync::mpsc::unbounded_channel()
}

async fn collect_events(
    mut rx: tokio::sync::mpsc::UnboundedReceiver<AgentEvent>,
) -> Vec<AgentEvent> {
    let mut events = Vec::new();
    while let Ok(event) = rx.try_recv() {
        events.push(event);
    }
    events
}

// ═══════════════════════════════════════════════════════
// Tests — 基本流程
// ═══════════════════════════════════════════════════════

#[tokio::test]
async fn test_agent_text_response() {
    let model = MockModel::new(vec![MockModelResponse::Events(text_events(&["hi"]))]);
    let (tx, _rx) = make_channel();

    let mut agent = AgentLoop::new(model, PassthroughContext, EmptyTools, 5);
    let result = agent.run("input", tx).await.unwrap();

    assert_eq!(result, "hi");
}

#[tokio::test]
async fn test_agent_single_tool_call() {
    let model = MockModel::new(vec![
        // 第 1 次：返回 tool call
        MockModelResponse::Events(tool_call_events(vec![(
            "tc1",
            "read",
            serde_json::json!({"path": "/a"}),
        )])),
        // 第 2 次：返回文本
        MockModelResponse::Events(text_events(&["结果是..."])),
    ]);

    let tools = MockTools::new(vec![Ok(ToolOutput {
        content: "file content".into(),
        is_error: false,
    })]);

    let (tx, _rx) = make_channel();
    let mut agent = AgentLoop::new(model, PassthroughContext, tools, 5);
    let result = agent.run("read /a", tx).await.unwrap();

    assert_eq!(result, "结果是...");
}

#[tokio::test]
async fn test_agent_multiple_tool_calls() {
    let model = MockModel::new(vec![
        // 返回 2 个 tool calls
        MockModelResponse::Events(tool_call_events(vec![
            ("tc1", "read", serde_json::json!({"path": "/a"})),
            ("tc2", "read", serde_json::json!({"path": "/b"})),
        ])),
        // 看到两个结果后返回文本
        MockModelResponse::Events(text_events(&["done"])),
    ]);

    let tools = MockTools::new(vec![
        Ok(ToolOutput {
            content: "content a".into(),
            is_error: false,
        }),
        Ok(ToolOutput {
            content: "content b".into(),
            is_error: false,
        }),
    ]);

    let (tx, _rx) = make_channel();
    let mut agent = AgentLoop::new(model, PassthroughContext, tools, 5);
    let result = agent.run("read both", tx).await.unwrap();

    assert_eq!(result, "done");
}

// ═══════════════════════════════════════════════════════
// Tests — 边界条件
// ═══════════════════════════════════════════════════════

#[tokio::test]
async fn test_agent_tool_loop_depth_limit() {
    // 模型每次都返回 tool call，max_tool_rounds=3 → 只调 3 次模型
    let model = MockModel::new(vec![
        MockModelResponse::Events(tool_call_events(vec![(
            "tc1",
            "read",
            serde_json::json!({}),
        )])),
        MockModelResponse::Events(tool_call_events(vec![(
            "tc2",
            "read",
            serde_json::json!({}),
        )])),
        MockModelResponse::Events(tool_call_events(vec![(
            "tc3",
            "read",
            serde_json::json!({}),
        )])),
        // 第 4 个不应该被消费
    ]);

    let tools = MockTools::new(vec![
        Ok(ToolOutput {
            content: "ok".into(),
            is_error: false,
        }),
        Ok(ToolOutput {
            content: "ok".into(),
            is_error: false,
        }),
        Ok(ToolOutput {
            content: "ok".into(),
            is_error: false,
        }),
    ]);

    let (tx, _rx) = make_channel();
    let mut agent = AgentLoop::new(model, PassthroughContext, tools, 3);
    let result = agent.run("loop", tx).await.unwrap();

    assert!(
        result.contains("exceeded"),
        "should report depth limit exceeded, got: {}",
        result
    );
}

#[tokio::test]
async fn test_agent_tool_error_reported() {
    let model = MockModel::new(vec![
        // 返回 tool call
        MockModelResponse::Events(tool_call_events(vec![(
            "tc1",
            "read",
            serde_json::json!({"path": "/x"}),
        )])),
        // 看到错误后生成回复
        MockModelResponse::Events(text_events(&["I see the error"])),
    ]);

    let tools = MockTools::new(vec![Err(anyhow::anyhow!("file not found: /x"))]);

    let (tx, rx) = make_channel();
    let mut agent = AgentLoop::new(model, PassthroughContext, tools, 5);
    let result = agent.run("read /x", tx).await.unwrap();

    assert_eq!(result, "I see the error");

    // 验证错误通过 ToolResult 事件回传
    let events = collect_events(rx).await;
    let tool_result_events: Vec<&AgentEvent> = events
        .iter()
        .filter(|e| matches!(e, AgentEvent::ToolResult { .. }))
        .collect();
    assert_eq!(tool_result_events.len(), 1);
    if let AgentEvent::ToolResult { output, .. } = &tool_result_events[0] {
        assert!(output.is_error);
        assert!(output.content.contains("file not found"));
    }
}

#[tokio::test]
async fn test_agent_permission_deny_reported() {
    let model = MockModel::new(vec![
        MockModelResponse::Events(tool_call_events(vec![(
            "tc1",
            "bash",
            serde_json::json!({"command": "rm -rf /"}),
        )])),
        MockModelResponse::Events(text_events(&["understood"])),
    ]);

    let tools = MockTools::new(vec![Err(anyhow::anyhow!("permission denied: bash"))]);

    let (tx, rx) = make_channel();
    let mut agent = AgentLoop::new(model, PassthroughContext, tools, 5);
    let result = agent.run("delete everything", tx).await.unwrap();

    assert_eq!(result, "understood");

    let events = collect_events(rx).await;
    let tool_result_events: Vec<&AgentEvent> = events
        .iter()
        .filter(|e| matches!(e, AgentEvent::ToolResult { .. }))
        .collect();
    assert_eq!(tool_result_events.len(), 1);
    if let AgentEvent::ToolResult { output, .. } = &tool_result_events[0] {
        assert!(output.is_error);
        assert!(output.content.contains("permission denied"));
    }
}

// ═══════════════════════════════════════════════════════
// Tests — 流式和持久化
// ═══════════════════════════════════════════════════════

#[tokio::test]
async fn test_agent_streaming_events() {
    let model = MockModel::new(vec![MockModelResponse::Events(text_events(&[
        "hel", "lo",
    ]))]);

    let (tx, rx) = make_channel();
    let mut agent = AgentLoop::new(model, PassthroughContext, EmptyTools, 5);
    agent.run("hi", tx).await.unwrap();

    let events = collect_events(rx).await;

    // 验证事件顺序：StreamStart, Delta, Delta, Done
    assert!(events.len() >= 4, "expected at least 4 events, got {}", events.len());
    assert!(matches!(events[0], AgentEvent::StreamStart));
    assert!(matches!(events[1], AgentEvent::Delta { .. }));
    assert!(matches!(events[2], AgentEvent::Delta { .. }));
    assert!(matches!(events[3], AgentEvent::Done));

    // 验证 Delta 内容
    if let AgentEvent::Delta { content } = &events[1] {
        assert_eq!(content, "hel");
    }
    if let AgentEvent::Delta { content } = &events[2] {
        assert_eq!(content, "lo");
    }
}

#[tokio::test]
async fn test_agent_saves_session() {
    let model = MockModel::new(vec![MockModelResponse::Events(text_events(&["reply"]))]);
    let session = std::sync::Arc::new(MockSessionStore::new());

    let (tx, _rx) = make_channel();
    let mut agent = AgentLoop::new(model, PassthroughContext, EmptyTools, 5)
        .with_session(Box::new(ArcSessionStore(session.clone())));
    agent.run("hello", tx).await.unwrap();

    let saved = session.saved.lock().unwrap();
    assert_eq!(saved.len(), 1, "session should be saved once");

    let messages = &saved[0];
    // 至少包含 user + assistant 消息
    assert!(messages.len() >= 2);
    assert_eq!(messages[0].role, Role::User);
    assert!(matches!(&messages[0].content, Content::Text(t) if t == "hello"));
    assert_eq!(messages[1].role, Role::Assistant);
    assert!(matches!(&messages[1].content, Content::Text(t) if t == "reply"));
}

// Arc wrapper for SessionStore to allow shared ownership in tests
struct ArcSessionStore(std::sync::Arc<MockSessionStore>);

#[async_trait]
impl SessionStore for ArcSessionStore {
    async fn save(&self, messages: &[Message]) -> anyhow::Result<()> {
        self.0.save(messages).await
    }
}

// ═══════════════════════════════════════════════════════
// Tests — 错误恢复
// ═══════════════════════════════════════════════════════

#[tokio::test]
async fn test_agent_model_error_retry() {
    let model = MockModel::new(vec![
        // 第 1 次：503 暂时性错误
        MockModelResponse::Transient(503, "service unavailable".into()),
        // 重试：成功
        MockModelResponse::Events(text_events(&["recovered"])),
    ]);

    let (tx, _rx) = make_channel();
    let mut agent = AgentLoop::new(model, PassthroughContext, EmptyTools, 5);
    let result = agent.run("test", tx).await.unwrap();

    assert_eq!(result, "recovered");
}

#[tokio::test]
async fn test_agent_model_error_fatal() {
    let model = MockModel::new(vec![
        // 401 致命错误
        MockModelResponse::AuthError(401, "invalid api key".into()),
    ]);

    let (tx, _rx) = make_channel();
    let mut agent = AgentLoop::new(model, PassthroughContext, EmptyTools, 5);
    let result = agent.run("test", tx).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("auth error") || err.to_string().contains("invalid api key"),
        "error should indicate auth failure, got: {}",
        err
    );
}

#[tokio::test]
async fn test_agent_context_overflow() {
    // MockModel：context limit = 200 chars
    let model = MockModel::new(vec![
        // compact 后重新 assemble，然后模型回复
        MockModelResponse::Events(text_events(&["after compact"])),
    ])
    .with_context_limit(200);

    let overflow_ctx = OverflowContext {
        assemble_count: AtomicUsize::new(0),
    };

    let (tx, _rx) = make_channel();
    let mut agent = AgentLoop::new(model, overflow_ctx, EmptyTools, 5);
    let result = agent.run("test", tx).await.unwrap();

    assert_eq!(result, "after compact");
}
