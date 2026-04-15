use async_trait::async_trait;
use std::path::PathBuf;

use crate::traits::*;
use crate::types::Message;

/// Phase 1 占位：不压缩，返回空字符串。
pub struct NoopCompaction;

#[async_trait]
impl CompactionProvider for NoopCompaction {
    async fn summarize(&self, _messages: &[Message]) -> anyhow::Result<String> {
        Ok(String::new())
    }
}

/// Phase 1 占位：不检索，返回空结果。
pub struct NoopRetriever;

#[async_trait]
impl MemoryRetriever for NoopRetriever {
    async fn retrieve(&self, _query: &str, _opts: RetrieveOptions) -> Vec<MemoryChunk> {
        vec![]
    }

    async fn index(&self, _files: &[PathBuf]) -> anyhow::Result<()> {
        Ok(())
    }
}

/// Phase 1 占位：不向量化，返回空结果。
pub struct NoopEmbedding;

#[async_trait]
impl EmbeddingProvider for NoopEmbedding {
    async fn embed(&self, _texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        Ok(vec![])
    }

    fn dimension(&self) -> usize {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_noop_compaction_returns_empty() {
        let noop = NoopCompaction;
        let result = noop.summarize(&[]).await.unwrap();
        assert_eq!(result, "");
    }

    #[tokio::test]
    async fn test_noop_retriever_returns_empty() {
        let noop = NoopRetriever;
        let result = noop.retrieve("any", RetrieveOptions::default()).await;
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_noop_retriever_index_ok() {
        let noop = NoopRetriever;
        let result = noop.index(&[PathBuf::from("/some/path")]).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_noop_embedding_returns_empty() {
        let noop = NoopEmbedding;
        let result = noop.embed(&["text".into()]).await.unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_noop_embedding_dimension_zero() {
        let noop = NoopEmbedding;
        assert_eq!(noop.dimension(), 0);
    }
}
