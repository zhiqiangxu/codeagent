use async_trait::async_trait;
use forge_core::{Tool, ToolOutput};
use serde_json::{json, Value};
use std::path::Path;

pub struct WriteTool {
    base_dir: std::path::PathBuf,
}

impl WriteTool {
    pub fn new(base_dir: impl Into<std::path::PathBuf>) -> Self {
        Self { base_dir: base_dir.into() }
    }
}

#[async_trait]
impl Tool for WriteTool {
    fn name(&self) -> &str { "write" }
    fn description(&self) -> &str { "Write content to a file, creating parent directories as needed." }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "content": { "type": "string" }
            },
            "required": ["path", "content"]
        })
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolOutput> {
        let path_str = args["path"].as_str().unwrap_or("");
        let content = args["content"].as_str().unwrap_or("");

        let path = if Path::new(path_str).is_absolute() {
            std::path::PathBuf::from(path_str)
        } else {
            self.base_dir.join(path_str)
        };

        // Create parent directories
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return Ok(ToolOutput {
                    content: format!("Error creating directories: {}", e),
                    is_error: true,
                });
            }
        }

        match std::fs::write(&path, content) {
            Ok(()) => Ok(ToolOutput {
                content: format!("Wrote {} bytes to {}", content.len(), path_str),
                is_error: false,
            }),
            Err(e) => Ok(ToolOutput {
                content: format!("Error writing file: {}", e),
                is_error: true,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_write_new_file() {
        let dir = tempdir().unwrap();
        let tool = WriteTool::new(dir.path());
        let result = tool.execute(json!({"path": "new.rs", "content": "hi"})).await.unwrap();
        assert!(!result.is_error);
        assert_eq!(std::fs::read_to_string(dir.path().join("new.rs")).unwrap(), "hi");
    }

    #[tokio::test]
    async fn test_write_overwrite() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("f.rs"), "old").unwrap();
        let tool = WriteTool::new(dir.path());
        tool.execute(json!({"path": "f.rs", "content": "new"})).await.unwrap();
        assert_eq!(std::fs::read_to_string(dir.path().join("f.rs")).unwrap(), "new");
    }

    #[tokio::test]
    async fn test_write_creates_parent() {
        let dir = tempdir().unwrap();
        let tool = WriteTool::new(dir.path());
        let result = tool.execute(json!({"path": "a/b/c.rs", "content": "deep"})).await.unwrap();
        assert!(!result.is_error);
        assert!(dir.path().join("a/b/c.rs").exists());
    }
}
