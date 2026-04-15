use crate::model_id::{ModelId, Provider};
use forge_core::{ChatRequest, ModelProvider, StreamResponse};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum RouterError {
    #[error("unknown model: '{0}'")]
    UnknownModel(String),

    #[error("auth error from provider: {0}")]
    AuthError(String),

    #[error("all providers failed, last error: {0}")]
    AllProvidersFailed(String),
}

/// ModelRouter：根据模型名选择 provider，支持 failover。
pub struct ModelRouter {
    providers: HashMap<String, Vec<Arc<dyn ModelProvider>>>,
}

impl ModelRouter {
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
        }
    }

    /// 注册一个 provider，关联到指定的 provider 名称。
    pub fn register(&mut self, provider_name: &str, provider: Arc<dyn ModelProvider>) {
        self.providers
            .entry(provider_name.to_string())
            .or_default()
            .push(provider);
    }

    /// 根据模型名路由到对应 provider 并调用 chat_stream。
    /// 503 类错误自动 failover，401 类错误不重试。
    pub async fn route(&self, req: ChatRequest) -> Result<StreamResponse, RouterError> {
        let model_id =
            ModelId::parse(&req.model).map_err(|_| RouterError::UnknownModel(req.model.clone()))?;

        let provider_key = match model_id.provider {
            Provider::Anthropic => "anthropic",
            Provider::OpenAI => "openai",
            Provider::Gemini => "gemini",
        };

        let providers = self
            .providers
            .get(provider_key)
            .ok_or_else(|| RouterError::UnknownModel(req.model.clone()))?;

        let mut last_error = String::new();

        for provider in providers {
            match provider.chat_stream(req.clone()).await {
                Ok(stream) => return Ok(stream),
                Err(e) => {
                    let err_str = e.to_string();
                    // 401/403 类认证错误不 failover
                    if err_str.contains("401") || err_str.contains("auth") {
                        return Err(RouterError::AuthError(err_str));
                    }
                    last_error = err_str;
                    // 继续尝试下一个 provider
                }
            }
        }

        Err(RouterError::AllProvidersFailed(last_error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use forge_core::*;

    struct MockProvider {
        name: String,
        result: Result<(), String>,
    }

    #[async_trait]
    impl ModelProvider for MockProvider {
        async fn chat_stream(&self, _req: ChatRequest) -> anyhow::Result<StreamResponse> {
            match &self.result {
                Ok(()) => {
                    let (tx, rx) = tokio::sync::mpsc::channel(1);
                    tokio::spawn(async move {
                        let _ = tx
                            .send(StreamEvent::Done {
                                usage: TokenUsage::default(),
                            })
                            .await;
                    });
                    Ok(StreamResponse::new(rx))
                }
                Err(e) => Err(anyhow::anyhow!("{}", e)),
            }
        }

        fn capabilities(&self) -> ModelCapabilities {
            ModelCapabilities {
                streaming: true,
                tool_use: true,
                vision: false,
                max_context_tokens: 128_000,
            }
        }

        fn token_counter(&self, _messages: &[Message]) -> usize {
            0
        }
    }

    fn make_req(model: &str) -> ChatRequest {
        ChatRequest::builder().model(model).messages(vec![]).build()
    }

    #[tokio::test]
    async fn test_router_selects_correct_provider() {
        let mut router = ModelRouter::new();
        router.register(
            "anthropic",
            Arc::new(MockProvider {
                name: "anthropic".into(),
                result: Ok(()),
            }),
        );
        router.register(
            "openai",
            Arc::new(MockProvider {
                name: "openai".into(),
                result: Ok(()),
            }),
        );

        let result = router.route(make_req("claude-sonnet-4-20250514")).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_router_unknown_model_error() {
        let router = ModelRouter::new();
        let result = router.route(make_req("not-exist")).await;
        assert!(matches!(result, Err(RouterError::UnknownModel(m)) if m == "not-exist"));
    }

    #[tokio::test]
    async fn test_router_failover_on_error() {
        let mut router = ModelRouter::new();
        router.register(
            "anthropic",
            Arc::new(MockProvider {
                name: "a1".into(),
                result: Err("503 service unavailable".into()),
            }),
        );
        router.register(
            "anthropic",
            Arc::new(MockProvider {
                name: "a2".into(),
                result: Ok(()),
            }),
        );

        let result = router.route(make_req("claude-sonnet-4-20250514")).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_router_failover_exhausted() {
        let mut router = ModelRouter::new();
        router.register(
            "anthropic",
            Arc::new(MockProvider {
                name: "a1".into(),
                result: Err("503 service unavailable".into()),
            }),
        );
        router.register(
            "anthropic",
            Arc::new(MockProvider {
                name: "a2".into(),
                result: Err("503 again".into()),
            }),
        );

        let result = router.route(make_req("claude-sonnet-4-20250514")).await;
        assert!(matches!(result, Err(RouterError::AllProvidersFailed(_))));
    }

    #[tokio::test]
    async fn test_router_no_retry_on_auth_error() {
        let mut router = ModelRouter::new();
        router.register(
            "anthropic",
            Arc::new(MockProvider {
                name: "a1".into(),
                result: Err("401 unauthorized".into()),
            }),
        );
        router.register(
            "anthropic",
            Arc::new(MockProvider {
                name: "a2".into(),
                result: Ok(()),
            }),
        );

        let result = router.route(make_req("claude-sonnet-4-20250514")).await;
        assert!(matches!(result, Err(RouterError::AuthError(_))));
    }
}
