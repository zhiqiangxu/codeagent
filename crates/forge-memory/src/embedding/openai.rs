use async_trait::async_trait;
use forge_core::EmbeddingProvider;

/// OpenAI text-embedding 模型配置。
const MODEL_DIMENSIONS: &[(&str, usize)] = &[
    ("text-embedding-3-small", 1536),
    ("text-embedding-3-large", 3072),
    ("text-embedding-ada-002", 1536),
];

/// OpenAI Embedding API 实现。
pub struct OpenAIEmbedding {
    model: String,
    dim: usize,
    api_key: String,
    client: reqwest::Client,
}

impl OpenAIEmbedding {
    pub fn new(model: &str, api_key: String) -> Self {
        let dim = MODEL_DIMENSIONS
            .iter()
            .find(|(m, _)| *m == model)
            .map(|(_, d)| *d)
            .unwrap_or(1536);

        Self {
            model: model.to_string(),
            dim,
            api_key,
            client: reqwest::Client::new(),
        }
    }

    /// 构造 API 请求 body（可在测试中验证格式）。
    pub fn format_request(&self, texts: &[String]) -> serde_json::Value {
        serde_json::json!({
            "input": texts,
            "model": self.model,
        })
    }

    /// 从 API 响应解析向量。
    pub fn parse_response(
        &self,
        response: &serde_json::Value,
    ) -> anyhow::Result<Vec<Vec<f32>>> {
        let data = response
            .get("data")
            .and_then(|d| d.as_array())
            .ok_or_else(|| anyhow::anyhow!("missing 'data' array in response"))?;

        let mut result = Vec::with_capacity(data.len());
        for item in data {
            let embedding = item
                .get("embedding")
                .and_then(|e| e.as_array())
                .ok_or_else(|| anyhow::anyhow!("missing 'embedding' in data item"))?;

            let vec: Vec<f32> = embedding
                .iter()
                .map(|v| v.as_f64().unwrap_or(0.0) as f32)
                .collect();
            result.push(vec);
        }

        Ok(result)
    }
}

#[async_trait]
impl EmbeddingProvider for OpenAIEmbedding {
    async fn embed(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        let body = self.format_request(texts);
        let resp = self
            .client
            .post("https://api.openai.com/v1/embeddings")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await?;

        let json: serde_json::Value = resp.json().await?;
        self.parse_response(&json)
    }

    fn dimension(&self) -> usize {
        self.dim
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_openai_embedding_format_request() {
        let emb = OpenAIEmbedding::new("text-embedding-3-small", "test-key".into());
        let body = emb.format_request(&["hello".to_string()]);

        assert_eq!(body["model"], "text-embedding-3-small");
        assert_eq!(body["input"][0], "hello");
    }

    #[test]
    fn test_openai_embedding_parse_response() {
        let emb = OpenAIEmbedding::new("text-embedding-3-small", "test-key".into());
        let response = serde_json::json!({
            "data": [
                {"embedding": [0.1, 0.2, 0.3], "index": 0}
            ]
        });

        let vectors = emb.parse_response(&response).unwrap();
        assert_eq!(vectors.len(), 1);
        assert_eq!(vectors[0].len(), 3);
        assert!((vectors[0][0] - 0.1).abs() < f32::EPSILON);
    }

    #[test]
    fn test_openai_embedding_dimension() {
        let emb = OpenAIEmbedding::new("text-embedding-3-small", "test-key".into());
        assert_eq!(emb.dimension(), 1536);

        let emb2 = OpenAIEmbedding::new("text-embedding-3-large", "test-key".into());
        assert_eq!(emb2.dimension(), 3072);
    }
}
