use forge_core::{Tool, ToolOutput};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("duplicate tool: '{0}'")]
    DuplicateTool(String),

    #[error("tool not found: '{0}'")]
    ToolNotFound(String),
}

/// 工具定义（名称 + 描述 + schema），用于生成 system prompt。
#[derive(Debug, Clone)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub schema: serde_json::Value,
}

/// 工具注册表：注册、查找、列出所有工具。
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) -> Result<(), RegistryError> {
        let name = tool.name().to_string();
        if self.tools.contains_key(&name) {
            return Err(RegistryError::DuplicateTool(name));
        }
        self.tools.insert(name, tool);
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    pub fn list(&self) -> Vec<ToolDefinition> {
        self.tools
            .values()
            .map(|t| ToolDefinition {
                name: t.name().to_string(),
                description: t.description().to_string(),
                schema: t.schema(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    struct FakeTool {
        tool_name: &'static str,
    }

    #[async_trait]
    impl Tool for FakeTool {
        fn name(&self) -> &str { self.tool_name }
        fn description(&self) -> &str { "fake" }
        fn schema(&self) -> serde_json::Value { serde_json::json!({"type": "object"}) }
        async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolOutput> {
            Ok(ToolOutput { content: "ok".into(), is_error: false })
        }
    }

    #[test]
    fn test_tool_registry_register() {
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(FakeTool { tool_name: "read" })).unwrap();
        assert!(reg.get("read").is_some());
    }

    #[test]
    fn test_tool_registry_duplicate_error() {
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(FakeTool { tool_name: "read" })).unwrap();
        let result = reg.register(Arc::new(FakeTool { tool_name: "read" }));
        assert!(matches!(result, Err(RegistryError::DuplicateTool(n)) if n == "read"));
    }

    #[test]
    fn test_tool_registry_list_all() {
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(FakeTool { tool_name: "read" })).unwrap();
        reg.register(Arc::new(FakeTool { tool_name: "glob" })).unwrap();
        reg.register(Arc::new(FakeTool { tool_name: "grep" })).unwrap();
        assert_eq!(reg.list().len(), 3);
    }
}
