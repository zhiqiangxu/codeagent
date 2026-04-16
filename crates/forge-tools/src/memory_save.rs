//! memory_save 工具：让 LLM 可以主动保存信息到 FORGE.md。

use std::path::PathBuf;

use async_trait::async_trait;
use forge_core::{Tool, ToolOutput};

pub struct MemorySaveTool {
    forge_md_path: PathBuf,
}

impl MemorySaveTool {
    pub fn new(forge_md_path: impl Into<PathBuf>) -> Self {
        Self {
            forge_md_path: forge_md_path.into(),
        }
    }
}

#[async_trait]
impl Tool for MemorySaveTool {
    fn name(&self) -> &str {
        "memory_save"
    }

    fn description(&self) -> &str {
        "Save a piece of information to project memory (FORGE.md) for future reference"
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "The information to save"
                }
            },
            "required": ["content"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolOutput> {
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'content' argument"))?;

        // 确保目录存在
        if let Some(parent) = self.forge_md_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        // 追加到 FORGE.md
        use tokio::io::AsyncWriteExt;
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.forge_md_path)
            .await?;
        file.write_all(b"\n").await?;
        file.write_all(content.as_bytes()).await?;
        file.flush().await?;

        Ok(ToolOutput {
            content: format!("Saved to {}", self.forge_md_path.display()),
            is_error: false,
        })
    }
}
