use async_trait::async_trait;
use forge_core::EmbeddingProvider;

/// Gemini embedding 模型配置。
const MODEL_DIMENSIONS: &[(&str, usize)] = &[
    ("text-embedding-004", 768),
    ("embedding-001", 768),
];

/// Gemini Embedding API 实现。
pub struct GeminiEmbedding {
    model: String,
    dim: usize,
    api_key: String,
    client: reqwest::Client,
}

impl GeminiEmbedding {
    pub fn new(model: &str, api_key: String) -> Self {
        let dim = MODEL_DIMENSIONS
            .iter()
            .find(|(m, _)| *m == model)
            .map(|(_, d)| *d)
            .unwrap_or(768);

        Self {
            model: model.to_string(),
            dim,
            api_key,
            client: reqwest::Client::new(),
        }
    }

    /// 构造 Gemini embedding API 请求 body。
    pub fn format_request(&self, texts: &[String]) -> serde_json::Value {
        let requests: Vec<serde_json::Value> = texts
            .iter()
            .map(|t| {
                serde_json::json!({
                    "model": format!("models/{}", self.model),
                    "content": {"parts": [{"text": t}]}
                })
            })
            .collect();

        serde_json::json!({ "requests": requests })
    }

    /// 从 Gemini API 响应解析向量。
    pub fn parse_response(
        &self,
        response: &serde_json::Value,
    ) -> anyhow::Result<Vec<Vec<f32>>> {
        let embeddings = response
            .get("embeddings")
            .and_then(|e| e.as_array())
            .ok_or_else(|| anyhow::anyhow!("missing 'embeddings' array"))?;

        let mut result = Vec::with_capacity(embeddings.len());
        for item in embeddings {
            let values = item
                .get("values")
                .and_then(|v| v.as_array())
                .ok_or_else(|| anyhow::anyhow!("missing 'values' in embedding"))?;

            let vec: Vec<f32> = values
                .iter()
                .map(|v| v.as_f64().unwrap_or(0.0) as f32)
                .collect();
            result.push(vec);
        }

        Ok(result)
    }
}

#[async_trait]
impl EmbeddingProvider for GeminiEmbedding {
    async fn embed(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        let body = self.format_request(texts);
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:batchEmbedContents?key={}",
            self.model, self.api_key
        );
        let resp = self.client.post(&url).json(&body).send().await?;
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
    fn test_gemini_embedding_format() {
        let emb = GeminiEmbedding::new("text-embedding-004", "test-key".into());
        let body = emb.format_request(&["hello".to_string()]);

        let requests = body["requests"].as_array().unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0]["model"]
            .as_str()
            .unwrap()
            .contains("text-embedding-004"));
        assert_eq!(requests[0]["content"]["parts"][0]["text"], "hello");
    }

    #[test]
    fn test_gemini_embedding_dimension() {
        let emb = GeminiEmbedding::new("text-embedding-004", "test-key".into());
        assert_eq!(emb.dimension(), 768);
    }
}
