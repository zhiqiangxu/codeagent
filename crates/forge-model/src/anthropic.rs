//! Anthropic ModelProvider 实现：调用 Claude API。

use async_trait::async_trait;
use forge_core::{
    ChatRequest, Content, Message, ModelCapabilities, ModelProvider, StreamEvent, StreamResponse,
    TokenUsage,
};
use reqwest::Client;

use crate::format::format_anthropic;
use crate::sse::AnthropicSseParser;

pub struct AnthropicProvider {
    api_key: String,
    client: Client,
    api_url: String,
}

impl AnthropicProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            client: Client::new(),
            api_url: std::env::var("ANTHROPIC_API_URL")
                .unwrap_or_else(|_| "https://api.anthropic.com".into()),
        }
    }
}

#[async_trait]
impl ModelProvider for AnthropicProvider {
    async fn chat_stream(&self, req: ChatRequest) -> anyhow::Result<StreamResponse> {
        let mut body = format_anthropic(&req);
        body["stream"] = serde_json::json!(true);
        if body.get("max_tokens").is_none() {
            body["max_tokens"] = serde_json::json!(8192);
        }

        let resp = self
            .client
            .post(format!("{}/v1/messages", self.api_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .body(body.to_string())
            .send()
            .await?;

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

        // 解析 SSE 流（使用有状态解析器处理 tool_use input 分块）
        let mut stream = resp.bytes_stream();
        tokio::spawn(async move {
            use futures::StreamExt;
            let mut buffer = String::new();
            let mut parser = AnthropicSseParser::new();
            let mut got_done = false;

            while let Some(chunk) = stream.next().await {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(_) => break,
                };
                buffer.push_str(&String::from_utf8_lossy(&chunk));

                // 按 SSE 协议解析：event: type\ndata: json\n\n
                while let Some(pos) = buffer.find("\n\n") {
                    let block = buffer[..pos].to_string();
                    buffer = buffer[pos + 2..].to_string();

                    let mut event_type = String::new();
                    let mut data_str = String::new();

                    for line in block.lines() {
                        if let Some(et) = line.strip_prefix("event: ") {
                            event_type = et.trim().to_string();
                        } else if let Some(d) = line.strip_prefix("data: ") {
                            data_str = d.trim().to_string();
                        }
                    }

                    if event_type.is_empty() {
                        continue;
                    }

                    let data = if data_str.is_empty() {
                        serde_json::Value::Object(serde_json::Map::new())
                    } else {
                        serde_json::from_str(&data_str).unwrap_or(serde_json::Value::Null)
                    };

                    if let Some(event) = parser.parse(&event_type, &data) {
                        if matches!(event, StreamEvent::Done { .. }) {
                            got_done = true;
                        }
                        if tx.send(event).await.is_err() {
                            return;
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
            max_context_tokens: 200_000,
        }
    }

    fn token_counter(&self, messages: &[Message]) -> usize {
        // 近似：每 4 字符 ≈ 1 token
        messages
            .iter()
            .map(|m| match &m.content {
                Content::Text(t) => t.len() / 4,
                Content::ToolResult { output, .. } => output.len() / 4,
            })
            .sum()
    }
}
