use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use forge_core::{MemoryChunk, MemoryRetriever, RetrieveOptions, Tool};
use forge_tools::{MemorySaveTool, MemorySearchTool};
use tempfile::TempDir;

// ─── Mock Retriever ────────────────────────────────────

struct MockRetriever {
    chunks: Vec<MemoryChunk>,
    last_scope: std::sync::Mutex<Option<String>>,
}

impl MockRetriever {
    fn new(chunks: Vec<MemoryChunk>) -> Self {
        Self {
            chunks,
            last_scope: std::sync::Mutex::new(None),
        }
    }
}

#[async_trait]
impl MemoryRetriever for MockRetriever {
    async fn retrieve(&self, _query: &str, opts: RetrieveOptions) -> Vec<MemoryChunk> {
        *self.last_scope.lock().unwrap() = opts.scope.clone();
        self.chunks.clone()
    }
    async fn index(&self, _files: &[PathBuf]) -> anyhow::Result<()> {
        Ok(())
    }
}

// ─── Tests ─────────────────────────────────────────────

#[tokio::test]
async fn test_memory_search_returns_chunks() {
    let retriever = Arc::new(MockRetriever::new(vec![
        MemoryChunk {
            content: "first result".into(),
            source: Some("a.rs".into()),
            score: 0.9,
        },
        MemoryChunk {
            content: "second result".into(),
            source: Some("b.rs".into()),
            score: 0.8,
        },
    ]));

    let tool = MemorySearchTool::new(retriever);
    let output = tool
        .execute(serde_json::json!({"query": "test"}))
        .await
        .unwrap();

    assert!(!output.is_error);
    assert!(output.content.contains("first result"));
    assert!(output.content.contains("second result"));
    assert!(output.content.contains("Result 1"));
    assert!(output.content.contains("Result 2"));
}

#[tokio::test]
async fn test_memory_search_with_scope() {
    let retriever = Arc::new(MockRetriever::new(vec![]));
    let tool = MemorySearchTool::new(retriever.clone());

    tool.execute(serde_json::json!({"query": "x", "scope": "project"}))
        .await
        .unwrap();

    let scope = retriever.last_scope.lock().unwrap().clone();
    assert_eq!(scope, Some("project".to_string()));
}

#[tokio::test]
async fn test_memory_save_writes() {
    let tmp = TempDir::new().unwrap();
    let forge_path = tmp.path().join(".codeforge").join("FORGE.md");

    let tool = MemorySaveTool::new(&forge_path);
    tool.execute(serde_json::json!({"content": "new rule"}))
        .await
        .unwrap();

    let content = std::fs::read_to_string(&forge_path).unwrap();
    assert!(content.contains("new rule"));
}

#[tokio::test]
async fn test_memory_save_append() {
    let tmp = TempDir::new().unwrap();
    let forge_dir = tmp.path().join(".codeforge");
    std::fs::create_dir_all(&forge_dir).unwrap();
    let forge_path = forge_dir.join("FORGE.md");
    std::fs::write(&forge_path, "old").unwrap();

    let tool = MemorySaveTool::new(&forge_path);
    tool.execute(serde_json::json!({"content": "new"}))
        .await
        .unwrap();

    let content = std::fs::read_to_string(&forge_path).unwrap();
    assert!(content.contains("old"));
    assert!(content.contains("new"));
}
