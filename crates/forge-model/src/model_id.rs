use serde::{Deserialize, Serialize};

/// LLM 提供商。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Provider {
    Anthropic,
    OpenAI,
    Gemini,
}

/// 解析后的模型 ID。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelId {
    pub provider: Provider,
    pub name: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ModelIdError {
    #[error("unknown provider for model '{0}'")]
    UnknownProvider(String),
}

impl ModelId {
    /// 从模型名推断 provider。
    pub fn parse(name: &str) -> Result<Self, ModelIdError> {
        let provider = if name.starts_with("claude") {
            Provider::Anthropic
        } else if name.starts_with("gpt") || name.starts_with("o1") || name.starts_with("o3") {
            Provider::OpenAI
        } else if name.starts_with("gemini") {
            Provider::Gemini
        } else {
            return Err(ModelIdError::UnknownProvider(name.to_string()));
        };

        Ok(ModelId {
            provider,
            name: name.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_id_parse_anthropic() {
        let id = ModelId::parse("claude-sonnet-4-20250514").unwrap();
        assert_eq!(id.provider, Provider::Anthropic);
        assert_eq!(id.name, "claude-sonnet-4-20250514");
    }

    #[test]
    fn test_model_id_parse_openai() {
        let id = ModelId::parse("gpt-4o").unwrap();
        assert_eq!(id.provider, Provider::OpenAI);
    }

    #[test]
    fn test_model_id_parse_gemini() {
        let id = ModelId::parse("gemini-2.0-flash").unwrap();
        assert_eq!(id.provider, Provider::Gemini);
    }

    #[test]
    fn test_model_id_parse_unknown() {
        let result = ModelId::parse("unknown-model");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("unknown-model"));
    }
}
