use std::path::PathBuf;

use async_trait::async_trait;
use forge_core::{
    CompactionProvider, Content, ContextEngine, MemoryChunk, MemoryRetriever, Message,
    RetrieveOptions, Role, SimpleContextEngine, TokenBudget,
};

// ─── Mock Implementations ─────────────────────────────

struct MockRetriever {
    chunks: Vec<MemoryChunk>,
}

#[async_trait]
impl MemoryRetriever for MockRetriever {
    async fn retrieve(&self, _query: &str, _opts: RetrieveOptions) -> Vec<MemoryChunk> {
        self.chunks.clone()
    }
    async fn index(&self, _files: &[PathBuf]) -> anyhow::Result<()> {
        Ok(())
    }
}

struct MockCompaction {
    summary: String,
}

#[async_trait]
impl CompactionProvider for MockCompaction {
    async fn summarize(&self, _messages: &[Message]) -> anyhow::Result<String> {
        Ok(self.summary.clone())
    }
}

// ─── Helpers ───────────────────────────────────────────

fn char_counter(messages: &[Message]) -> usize {
    messages
        .iter()
        .map(|m| match &m.content {
            Content::Text(t) => t.len(),
            Content::ToolResult { output, .. } => output.len(),
        })
        .sum()
}

fn make_msg(role: Role, text: &str) -> Message {
    Message {
        role,
        content: Content::Text(text.to_string()),
        tool_calls: vec![],
    }
}

fn make_conversation(turns: usize) -> Vec<Message> {
    let mut msgs = Vec::new();
    for i in 0..turns {
        msgs.push(make_msg(Role::User, &format!("user message {}", i)));
        msgs.push(make_msg(Role::Assistant, &format!("assistant message {}", i)));
    }
    msgs
}

fn engine_with_retriever(retriever: MockRetriever) -> SimpleContextEngine {
    SimpleContextEngine::new(
        Box::new(retriever),
        Box::new(MockCompaction {
            summary: String::new(),
        }),
        vec![],
        "System prompt.".into(),
        Box::new(char_counter),
    )
}

fn engine_with_compaction(summary: &str) -> SimpleContextEngine {
    SimpleContextEngine::new(
        Box::new(MockRetriever { chunks: vec![] }),
        Box::new(MockCompaction {
            summary: summary.to_string(),
        }),
        vec![],
        "System prompt.".into(),
        Box::new(char_counter),
    )
}

// ─── Integration Tests ────────────────────────────────

#[tokio::test]
async fn test_assemble_within_budget() {
    let engine = engine_with_retriever(MockRetriever { chunks: vec![] });
    let conversation = make_conversation(10); // 20 messages

    let budget = TokenBudget {
        max_tokens: 1000,
        reserved: 0,
    };
    let result = engine.assemble(&conversation, budget).await;

    let total_tokens = char_counter(&result);
    assert!(
        total_tokens <= 1000,
        "total tokens {} exceeded budget 1000",
        total_tokens
    );
}

#[tokio::test]
async fn test_assemble_forge_md_in_system() {
    let retriever = MockRetriever {
        chunks: vec![MemoryChunk {
            content: "FORGE content here".to_string(),
            source: Some("global/FORGE.md".into()),
            score: 1.0,
        }],
    };
    let engine = engine_with_retriever(retriever);

    let result = engine
        .assemble(
            &[make_msg(Role::User, "hi")],
            TokenBudget {
                max_tokens: 4096,
                reserved: 0,
            },
        )
        .await;

    let system_text = match &result[0].content {
        Content::Text(t) => t.clone(),
        _ => panic!("expected text"),
    };
    assert!(
        system_text.contains("FORGE content here"),
        "system prompt should contain FORGE.md content"
    );
}

#[tokio::test]
async fn test_assemble_empty_retriever() {
    let engine = engine_with_retriever(MockRetriever { chunks: vec![] });

    let result = engine
        .assemble(
            &[make_msg(Role::User, "hi")],
            TokenBudget {
                max_tokens: 4096,
                reserved: 0,
            },
        )
        .await;

    assert_eq!(result[0].role, Role::System);
    let system_text = match &result[0].content {
        Content::Text(t) => t.clone(),
        _ => panic!("expected text"),
    };
    // System prompt present but no memory content injected
    assert!(system_text.contains("System prompt."));
    assert!(!system_text.contains("---")); // No memory separator
}

#[tokio::test]
async fn test_assemble_recent_3_turns_kept() {
    let engine = engine_with_retriever(MockRetriever { chunks: vec![] });
    let conversation = make_conversation(10); // 20 messages

    // Very tight budget: only enough for system + ~6 messages
    let budget = TokenBudget {
        max_tokens: 250,
        reserved: 0,
    };
    let result = engine.assemble(&conversation, budget).await;

    // Last 3 turns = messages 7,8,9 (user+assistant each)
    let texts: Vec<String> = result
        .iter()
        .filter(|m| m.role != Role::System)
        .map(|m| match &m.content {
            Content::Text(t) => t.clone(),
            _ => String::new(),
        })
        .collect();

    // Recent 3 turns (indices 7, 8, 9) must be present
    assert!(
        texts.iter().any(|t| t.contains("user message 9")),
        "most recent turn must be kept"
    );
    assert!(
        texts.iter().any(|t| t.contains("user message 8")),
        "second recent turn must be kept"
    );
    assert!(
        texts.iter().any(|t| t.contains("user message 7")),
        "third recent turn must be kept"
    );
}

#[tokio::test]
async fn test_assemble_old_turns_truncated() {
    let engine = engine_with_retriever(MockRetriever { chunks: vec![] });
    let conversation = make_conversation(10);

    let budget = TokenBudget {
        max_tokens: 250,
        reserved: 0,
    };
    let result = engine.assemble(&conversation, budget).await;

    let texts: Vec<String> = result
        .iter()
        .filter(|m| m.role != Role::System)
        .map(|m| match &m.content {
            Content::Text(t) => t.clone(),
            _ => String::new(),
        })
        .collect();

    // Old turns (0, 1, 2...) should be truncated when budget is tight
    // With budget=250, system prompt ~14 chars, recent 6 msgs ~120 chars,
    // there's little room for old messages
    let total_non_system = texts.len();
    assert!(
        total_non_system < 20,
        "old turns should be truncated, got {} messages",
        total_non_system
    );
}

#[tokio::test]
async fn test_compact_with_noop_compaction() {
    // Noop compaction returns empty string → old messages dropped
    let engine = engine_with_compaction("");
    let conversation = make_conversation(10);

    let target = TokenBudget {
        max_tokens: 500,
        reserved: 0,
    };
    let result = engine.compact(&conversation, target).await;

    // Only recent messages should remain (Noop drops old)
    assert!(
        result.len() <= 6,
        "noop compaction should drop old, got {} messages",
        result.len()
    );
}

#[tokio::test]
async fn test_compact_preserves_recent() {
    let engine = engine_with_compaction("");
    let conversation = make_conversation(10);

    let target = TokenBudget {
        max_tokens: 500,
        reserved: 0,
    };
    let result = engine.compact(&conversation, target).await;

    let texts: Vec<String> = result
        .iter()
        .map(|m| match &m.content {
            Content::Text(t) => t.clone(),
            _ => String::new(),
        })
        .collect();

    assert!(
        texts.iter().any(|t| t.contains("user message 9")),
        "most recent turn must survive compact"
    );
    assert!(
        texts.iter().any(|t| t.contains("user message 8")),
        "second recent turn must survive compact"
    );
    assert!(
        texts.iter().any(|t| t.contains("user message 7")),
        "third recent turn must survive compact"
    );
}
