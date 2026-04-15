mod model_id;
mod router;

// Re-export traits and types from forge-core
pub use forge_core::{
    ChatRequest, ChatRequestBuilder, ModelCapabilities, ModelProvider, StreamEvent, StreamResponse,
    TokenUsage,
};

pub use model_id::{ModelId, Provider};
pub use router::{ModelRouter, RouterError};
