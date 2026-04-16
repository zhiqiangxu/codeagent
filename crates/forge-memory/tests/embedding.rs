use async_trait::async_trait;
use forge_core::EmbeddingProvider;

/// 可控维度的 Mock Embedding Provider。
struct MockEmbedding {
    dim: usize,
    /// 如果 Some，返回该维度的向量（用于测试 dimension mismatch）。
    actual_dim: Option<usize>,
}

impl MockEmbedding {
    fn new(dim: usize) -> Self {
        Self {
            dim,
            actual_dim: None,
        }
    }

    fn with_actual_dim(mut self, actual: usize) -> Self {
        self.actual_dim = Some(actual);
        self
    }
}

#[async_trait]
impl EmbeddingProvider for MockEmbedding {
    async fn embed(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        let d = self.actual_dim.unwrap_or(self.dim);
        Ok(texts
            .iter()
            .enumerate()
            .map(|(i, _)| vec![i as f32 * 0.1; d])
            .collect())
    }

    fn dimension(&self) -> usize {
        self.dim
    }
}

/// 验证 embedding 维度是否与声明一致的辅助函数。
fn validate_embeddings(
    provider: &dyn EmbeddingProvider,
    vectors: &[Vec<f32>],
) -> Result<(), String> {
    for (i, vec) in vectors.iter().enumerate() {
        if vec.len() != provider.dimension() {
            return Err(format!(
                "dimension mismatch at index {}: expected {}, got {}",
                i,
                provider.dimension(),
                vec.len()
            ));
        }
    }
    Ok(())
}

#[tokio::test]
async fn test_embedding_batch() {
    let emb = MockEmbedding::new(3);
    let result = emb
        .embed(&["hello".into(), "world".into()])
        .await
        .unwrap();

    assert_eq!(result.len(), 2);
    assert_eq!(result[0].len(), 3);
    assert_eq!(result[1].len(), 3);
}

#[tokio::test]
async fn test_embedding_dimension_mismatch() {
    // 声明 dim=3 但实际返回 dim=5
    let emb = MockEmbedding::new(3).with_actual_dim(5);
    let result = emb.embed(&["test".into()]).await.unwrap();

    let validation = validate_embeddings(&emb, &result);
    assert!(
        validation.is_err(),
        "should detect dimension mismatch"
    );
    assert!(validation.unwrap_err().contains("dimension mismatch"));
}

#[tokio::test]
#[ignore] // 需要真实 API key
async fn test_openai_real_embedding() {
    let api_key = std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY not set");
    let emb = forge_memory::OpenAIEmbedding::new("text-embedding-3-small", api_key);
    let result = emb.embed(&["hello world".into()]).await.unwrap();

    assert_eq!(result.len(), 1);
    assert!(!result[0].is_empty());
    assert_eq!(result[0].len(), emb.dimension());
}
