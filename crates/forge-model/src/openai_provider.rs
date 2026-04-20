//! OpenAI 兼容 ModelProvider：支持 OpenAI、Gemini、DeepSeek、Ollama 等。

use async_trait::async_trait;
use forge_core::{
    ChatRequest, Content, Message, ModelCapabilities, ModelProvider, StreamEvent, StreamResponse,
    TokenUsage,
};
use reqwest::Client;

use crate::format::format_openai;
use crate::sse::OpenAISseParser;

pub struct OpenAICompatProvider {
    api_key: String,
    api_url: String,
    client: Client,
}

impl OpenAICompatProvider {
    pub fn new(api_key: String, api_url: String) -> Self {
        Self {
            api_key,
            api_url,
            client: Client::new(),
        }
    }

    /// 从环境变量构造（OPENAI_API_KEY + OPENAI_API_URL）。
    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("OPENAI_API_KEY").ok().unwrap_or_default();
        let api_url = std::env::var("OPENAI_API_URL")
            .unwrap_or_else(|_| "https://api.openai.com/v1".into());
        // 允许空 key（Ollama 本地不需要 key）
        Some(Self::new(api_key, api_url))
    }
}

#[async_trait]
impl ModelProvider for OpenAICompatProvider {
    async fn chat_stream(&self, req: ChatRequest) -> anyhow::Result<StreamResponse> {
        let body = format_openai(&req);

        let mut request = self
            .client
            .post(format!("{}/chat/completions", self.api_url.trim_end_matches('/')))
            .header("content-type", "application/json");

        if !self.api_key.is_empty() {
            request = request.header("authorization", format!("Bearer {}", self.api_key));
        }

        let resp = request.body(body.to_string()).send().await?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            if status == 401 || status == 403 {
                return Err(forge_core::ModelError::Auth {
                    status,
                    message: text,
                }
                .into());
            }
            return Err(forge_core::ModelError::Transient {
                status,
                message: text,
            }
            .into());
        }

        let (tx, rx) = tokio::sync::mpsc::channel(64);

        let mut stream = resp.bytes_stream();
        tokio::spawn(async move {
            use futures::StreamExt;
            let mut buffer = String::new();
            let mut parser = OpenAISseParser::new();
            let mut got_done = false;

            while let Some(chunk) = stream.next().await {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(_) => break,
                };
                buffer.push_str(&String::from_utf8_lossy(&chunk));

                while let Some(pos) = buffer.find("\n\n") {
                    let block = buffer[..pos].to_string();
                    buffer = buffer[pos + 2..].to_string();

                    for line in block.lines() {
                        if let Some(data) = line.strip_prefix("data: ") {
                            let data = data.trim();
                            if let Some(event) = parser.parse(data) {
                                if matches!(event, StreamEvent::Done { .. }) {
                                    got_done = true;
                                }
                                if tx.send(event).await.is_err() {
                                    return;
                                }
                            }
                        }
                    }
                }
            }

            if !got_done {
                let _ = tx
                    .send(StreamEvent::Done {
                        usage: TokenUsage::default(),
                    })
                    .await;
            }
        });

        Ok(StreamResponse::new(rx))
    }

    fn capabilities(&self) -> ModelCapabilities {
        ModelCapabilities {
            streaming: true,
            tool_use: true,
            vision: true,
            max_context_tokens: 128_000,
        }
    }

    fn token_counter(&self, messages: &[Message]) -> usize {
        messages
            .iter()
            .map(|m| match &m.content {
                Content::Text(t) => t.len() / 4,
                Content::ToolResult { output, .. } => output.len() / 4,
            })
            .sum()
    }
}
