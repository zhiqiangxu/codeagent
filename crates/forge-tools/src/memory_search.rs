//! memory_search 工具：让 LLM 可以主动检索记忆。

use std::sync::Arc;

use async_trait::async_trait;
use forge_core::{MemoryRetriever, RetrieveOptions, Tool, ToolOutput};

pub struct MemorySearchTool {
    retriever: Arc<dyn MemoryRetriever>,
}

impl MemorySearchTool {
    pub fn new(retriever: Arc<dyn MemoryRetriever>) -> Self {
        Self { retriever }
    }
}

#[async_trait]
impl Tool for MemorySearchTool {
    fn name(&self) -> &str {
        "memory_search"
    }

    fn description(&self) -> &str {
        "Search project memory for relevant code, documentation, and conversation history"
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query"
                },
                "scope": {
                    "type": "string",
                    "description": "Optional scope filter (e.g. 'project', 'global')"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolOutput> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let scope = args.get("scope").and_then(|v| v.as_str()).map(String::from);

        let opts = RetrieveOptions {
            top_k: Some(5),
            scope,
            ..Default::default()
        };

        let chunks = self.retriever.retrieve(query, opts).await;

        if chunks.is_empty() {
            return Ok(ToolOutput {
                content: "No results found.".into(),
                is_error: false,
            });
        }

        let mut output = String::new();
        for (i, chunk) in chunks.iter().enumerate() {
            output.push_str(&format!(
                "--- Result {} (score: {:.2}) ---\n{}\n\n",
                i + 1,
                chunk.score,
                chunk.content
            ));
        }

        Ok(ToolOutput {
            content: output,
            is_error: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tool() -> MemorySearchTool {
        MemorySearchTool::new(Arc::new(forge_core::noop::NoopRetriever))
    }

    #[test]
    fn test_memory_search_name() {
        let tool = make_tool();
        assert_eq!(tool.name(), "memory_search");
    }

    #[test]
    fn test_memory_search_schema() {
        let tool = make_tool();
        let schema = tool.schema();
        let props = schema.get("properties").unwrap();
        assert!(props.get("query").is_some());
        assert!(props.get("scope").is_some());

        let required = schema.get("required").unwrap().as_array().unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("query")));
    }
}
