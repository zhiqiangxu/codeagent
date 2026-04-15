use async_trait::async_trait;
use forge_core::{Tool, ToolOutput};
use serde_json::{json, Value};
use std::path::Path;

pub struct EditTool {
    base_dir: std::path::PathBuf,
}

impl EditTool {
    pub fn new(base_dir: impl Into<std::path::PathBuf>) -> Self {
        Self { base_dir: base_dir.into() }
    }
}

#[async_trait]
impl Tool for EditTool {
    fn name(&self) -> &str { "edit" }
    fn description(&self) -> &str { "Replace exact string in a file. Fails if old_string is not found or ambiguous." }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "old_string": { "type": "string" },
                "new_string": { "type": "string" }
            },
            "required": ["path", "old_string", "new_string"]
        })
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolOutput> {
        let path_str = args["path"].as_str().unwrap_or("");
        let old = args["old_string"].as_str().unwrap_or("");
        let new = args["new_string"].as_str().unwrap_or("");

        let path = if Path::new(path_str).is_absolute() {
            std::path::PathBuf::from(path_str)
        } else {
            self.base_dir.join(path_str)
        };

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                return Ok(ToolOutput {
                    content: format!("Error reading file: {}", e),
                    is_error: true,
                });
            }
        };

        let count = content.matches(old).count();

        if count == 0 {
            return Ok(ToolOutput {
                content: format!("old_string not found in {}", path_str),
                is_error: true,
            });
        }

        if count > 1 {
            return Ok(ToolOutput {
                content: format!("old_string is ambiguous: found {} occurrences in {}", count, path_str),
                is_error: true,
            });
        }

        let new_content = content.replacen(old, new, 1);
        std::fs::write(&path, &new_content)?;

        Ok(ToolOutput {
            content: format!("Edited {}", path_str),
            is_error: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_edit_replace_exact() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("f.rs"), "old text here").unwrap();
        let tool = EditTool::new(dir.path());
        let result = tool.execute(json!({"path": "f.rs", "old_string": "old text", "new_string": "new text"})).await.unwrap();
        assert!(!result.is_error);
        assert_eq!(std::fs::read_to_string(dir.path().join("f.rs")).unwrap(), "new text here");
    }

    #[tokio::test]
    async fn test_edit_old_string_not_found() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("f.rs"), "hello").unwrap();
        let tool = EditTool::new(dir.path());
        let result = tool.execute(json!({"path": "f.rs", "old_string": "nonexist", "new_string": "x"})).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("not found"));
    }

    #[tokio::test]
    async fn test_edit_old_string_ambiguous() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("f.rs"), "dup and dup").unwrap();
        let tool = EditTool::new(dir.path());
        let result = tool.execute(json!({"path": "f.rs", "old_string": "dup", "new_string": "x"})).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("ambiguous"));
    }

    #[tokio::test]
    async fn test_edit_preserves_other_lines() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("f.rs"), "line1\nline2\nline3").unwrap();
        let tool = EditTool::new(dir.path());
        tool.execute(json!({"path": "f.rs", "old_string": "line2", "new_string": "CHANGED"})).await.unwrap();
        let content = std::fs::read_to_string(dir.path().join("f.rs")).unwrap();
        assert!(content.contains("line1"));
        assert!(content.contains("CHANGED"));
        assert!(content.contains("line3"));
    }
}
