// forge-model: ModelProvider 的具体实现（Anthropic/OpenAI/Gemini）在迭代 2-3 实现。
// trait 定义在 forge-core::traits 中。
pub use forge_core::{
    ChatRequest, ChatRequestBuilder, ModelCapabilities, ModelProvider, StreamEvent, StreamResponse,
    TokenUsage,
};
