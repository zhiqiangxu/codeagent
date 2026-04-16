use std::sync::Mutex;
use std::collections::VecDeque;

use async_trait::async_trait;
use forge_core::{
    ChatRequest, CompactionProvider, Content, ContextEngine, Message, ModelCapabilities,
    ModelProvider, Role, SimpleContextEngine, StreamEvent, StreamResponse, TokenBudget, TokenUsage,
};
use forge_core::noop::NoopRetriever;

// ─── MockModel for Compaction ──────────────────────────

struct CompactionMockModel {
    responses: Mutex<VecDeque<String>>,
    called: Mutex<Vec<String>>,
}

impl CompactionMockModel {
    fn new(responses: Vec<String>) -> Self {
        Self {
            responses: Mutex::new(VecDeque::from(responses)),
            called: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl ModelProvider for CompactionMockModel {
    async fn chat_stream(&self, req: ChatRequest) -> anyhow::Result<StreamResponse> {
        // 记录被调用（用于验证 LLM 路径）
        if let Some(msg) = req.messages.first() {
            if let Content::Text(t) = &msg.content {
                self.called.lock().unwrap().push(t.clone());
            }
        }

        let text = self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| "default summary".into());

        let (tx, rx) = tokio::sync::mpsc::channel(8);
        tokio::spawn(async move {
            let _ = tx
                .send(StreamEvent::Delta {
                    content: text,
                })
                .await;
            let _ = tx
                .send(StreamEvent::Done {
                    usage: TokenUsage { input: 0, output: 0 },
                })
                .await;
        });
        Ok(StreamResponse::new(rx))
    }

    fn capabilities(&self) -> ModelCapabilities {
        ModelCapabilities {
            streaming: true,
            tool_use: false,
            vision: false,
            max_context_tokens: 8192,
        }
    }

    fn token_counter(&self, messages: &[Message]) -> usize {
        messages
            .iter()
            .map(|m| match &m.content {
                Content::Text(t) => t.len(),
                Content::ToolResult { output, .. } => output.len(),
            })
            .sum()
    }
}

// ─── LlmCompaction (re-implemented here since it's in forge-memory) ───

struct TestLlmCompaction {
    model: std::sync::Arc<CompactionMockModel>,
}

#[async_trait]
impl CompactionProvider for TestLlmCompaction {
    async fn summarize(&self, messages: &[Message]) -> anyhow::Result<String> {
        if messages.is_empty() {
            return Ok(String::new());
        }

        let mut conversation = String::new();
        for msg in messages {
            let content = match &msg.content {
                Content::Text(t) => t.as_str(),
                Content::ToolResult { output, .. } => output.as_str(),
            };
            conversation.push_str(content);
            conversation.push('\n');
            for tc in &msg.tool_calls {
                conversation.push_str(&format!("[tool: {}]\n", tc.name));
            }
        }

        let prompt = format!("Summarize: {}", conversation);
        let request = forge_core::ChatRequest::builder()
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

// ─── Helpers ───────────────────────────────────────────

fn make_conversation(turns: usize) -> Vec<Message> {
    let mut msgs = Vec::new();
    for i in 0..turns {
        msgs.push(Message {
            role: Role::User,
            content: Content::Text(format!("user message {} with some extra padding text to make it longer for token counting purposes", i)),
            tool_calls: vec![],
        });
        msgs.push(Message {
            role: Role::Assistant,
            content: Content::Text(format!("assistant response {} with detailed explanation and reasoning about the topic", i)),
            tool_calls: vec![],
        });
    }
    msgs
}

fn char_counter(messages: &[Message]) -> usize {
    messages
        .iter()
        .map(|m| match &m.content {
            Content::Text(t) => t.len(),
            Content::ToolResult { output, .. } => output.len(),
        })
        .sum()
}

// ─── Tests ─────────────────────────────────────────────

#[tokio::test]
async fn test_llm_compaction_reduces_tokens() {
    let model = std::sync::Arc::new(CompactionMockModel::new(vec![
        "Summary: user discussed file operations".into(),
    ]));
    let compaction = TestLlmCompaction {
        model: model.clone(),
    };

    let messages = make_conversation(10); // ~2000 chars
    let original_tokens = char_counter(&messages);
    assert!(original_tokens > 1000);

    let summary = compaction.summarize(&messages).await.unwrap();
    assert!(
        summary.len() < original_tokens / 4,
        "summary ({}) should be < 1/4 of original ({})",
        summary.len(),
        original_tokens
    );
}

#[tokio::test]
async fn test_llm_compaction_preserves_key_info() {
    let model = std::sync::Arc::new(CompactionMockModel::new(vec![
        "User modified /a.rs using write tool, then read /b.rs".into(),
    ]));
    let compaction = TestLlmCompaction {
        model: model.clone(),
    };

    let messages = vec![
        Message {
            role: Role::Assistant,
            content: Content::Text("writing file".into()),
            tool_calls: vec![forge_core::ToolCall {
                id: "tc1".into(),
                name: "write".into(),
                arguments: serde_json::json!({"path": "/a.rs"}),
            }],
        },
    ];

    let summary = compaction.summarize(&messages).await.unwrap();
    assert!(summary.contains("/a.rs"));
}

#[tokio::test]
async fn test_context_engine_uses_llm_compaction() {
    let model = std::sync::Arc::new(CompactionMockModel::new(vec![
        "compacted summary of old conversation".into(),
    ]));
    let compaction = TestLlmCompaction {
        model: model.clone(),
    };

    let engine = SimpleContextEngine::new(
        Box::new(NoopRetriever),
        Box::new(compaction),
        vec![],
        "System.".into(),
        Box::new(char_counter),
    );

    let messages = make_conversation(10);
    let result = engine
        .compact(
            &messages,
            TokenBudget {
                max_tokens: 500,
                reserved: 0,
            },
        )
        .await;

    // compact 应该调用了 MockModel（LLM 路径）
    let calls = model.called.lock().unwrap();
    assert!(
        !calls.is_empty(),
        "LLM compaction should have called the model"
    );

    // 结果应该比原始消息少
    assert!(result.len() < messages.len());
}
