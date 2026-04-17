mod model_id;
mod router;
pub mod format;
pub mod sse;
pub mod anthropic;
pub mod openai_provider;

// Re-export traits and types from forge-core
pub use forge_core::{
    ChatRequest, ChatRequestBuilder, ModelCapabilities, ModelProvider, StreamEvent, StreamResponse,
    TokenUsage,
};

pub use model_id::{ModelId, Provider};
pub use router::{ModelRouter, RouterError};
pub use anthropic::AnthropicProvider;
pub use openai_provider::OpenAICompatProvider;
