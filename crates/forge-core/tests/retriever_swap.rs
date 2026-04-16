use std::path::PathBuf;

use async_trait::async_trait;
use forge_core::{
    Content, ContextEngine, MemoryChunk, MemoryRetriever, RetrieveOptions, Role, SimpleContextEngine, TokenBudget, Message,
};
use forge_core::noop::NoopCompaction;

// ─── Custom Retrievers ────────────────────────────────

/// 返回固定内容的自定义 Retriever。
struct FixedRetriever {
    content: String,
}

#[async_trait]
impl MemoryRetriever for FixedRetriever {
    async fn retrieve(&self, _query: &str, _opts: RetrieveOptions) -> Vec<MemoryChunk> {
        vec![MemoryChunk {
            content: self.content.clone(),
            source: Some("custom".into()),
            score: 1.0,
        }]
    }
    async fn index(&self, _files: &[PathBuf]) -> anyhow::Result<()> {
        Ok(())
    }
}

/// 纯向量检索（模拟）。
struct VecOnlyRetriever;

#[async_trait]
impl MemoryRetriever for VecOnlyRetriever {
    async fn retrieve(&self, _query: &str, _opts: RetrieveOptions) -> Vec<MemoryChunk> {
        vec![MemoryChunk {
            content: "vec-only result".into(),
            source: Some("vector".into()),
            score: 0.95,
        }]
    }
    async fn index(&self, _files: &[PathBuf]) -> anyhow::Result<()> {
        Ok(())
    }
}

/// 纯 FTS 检索（模拟）。
struct FtsOnlyRetriever;

#[async_trait]
impl MemoryRetriever for FtsOnlyRetriever {
    async fn retrieve(&self, _query: &str, _opts: RetrieveOptions) -> Vec<MemoryChunk> {
        vec![MemoryChunk {
            content: "fts-only result".into(),
            source: Some("fts".into()),
            score: 0.8,
        }]
    }
    async fn index(&self, _files: &[PathBuf]) -> anyhow::Result<()> {
        Ok(())
    }
}

// ─── Helpers ───────────────────────────────────────────

fn char_counter(messages: &[Message]) -> usize {
    messages.iter().map(|m| match &m.content {
        Content::Text(t) => t.len(),
        Content::ToolResult { output, .. } => output.len(),
    }).sum()
}

fn build_engine(retriever: Box<dyn MemoryRetriever>) -> SimpleContextEngine {
    SimpleContextEngine::new(
        retriever,
        Box::new(NoopCompaction),
        vec![],
        "System prompt.".into(),
        Box::new(char_counter),
    )
}

fn make_msg(text: &str) -> Message {
    Message {
        role: Role::User,
        content: Content::Text(text.into()),
        tool_calls: vec![],
    }
}

fn system_text(result: &[Message]) -> String {
    match &result[0].content {
        Content::Text(t) => t.clone(),
        _ => panic!("expected text"),
    }
}

// ─── Tests ─────────────────────────────────────────────

#[tokio::test]
async fn test_swap_forgemd_to_hybrid() {
    // ForgemdRetriever 已在 iter 6 测试过；这里验证换成另一个 Retriever 后 ContextEngine 仍正常
    let retriever = FixedRetriever {
        content: "hybrid memory content".into(),
    };
    let engine = build_engine(Box::new(retriever));

    let result = engine
        .assemble(&[make_msg("hi")], TokenBudget { max_tokens: 4096, reserved: 0 })
        .await;

    let sys = system_text(&result);
    assert!(sys.contains("hybrid memory content"));
}

#[tokio::test]
async fn test_swap_to_vec_only() {
    let engine = build_engine(Box::new(VecOnlyRetriever));

    let result = engine
        .assemble(&[make_msg("hi")], TokenBudget { max_tokens: 4096, reserved: 0 })
        .await;

    let sys = system_text(&result);
    assert!(sys.contains("vec-only result"));
}

#[tokio::test]
async fn test_swap_to_fts_only() {
    let engine = build_engine(Box::new(FtsOnlyRetriever));

    let result = engine
        .assemble(&[make_msg("hi")], TokenBudget { max_tokens: 4096, reserved: 0 })
        .await;

    let sys = system_text(&result);
    assert!(sys.contains("fts-only result"));
}

#[tokio::test]
async fn test_custom_retriever() {
    let retriever = FixedRetriever {
        content: "my custom rules: always use snake_case".into(),
    };
    let engine = build_engine(Box::new(retriever));

    let result = engine
        .assemble(&[make_msg("test")], TokenBudget { max_tokens: 4096, reserved: 0 })
        .await;

    let sys = system_text(&result);
    assert!(
        sys.contains("my custom rules: always use snake_case"),
        "custom retriever content should be in system prompt"
    );
}
