use async_trait::async_trait;
use forge_core::{Tool, ToolOutput};
use serde_json::{json, Value};
use std::path::Path;

pub struct ReadTool {
    /// 工作目录（安全基线）。
    base_dir: std::path::PathBuf,
}

impl ReadTool {
    pub fn new(base_dir: impl Into<std::path::PathBuf>) -> Self {
        Self { base_dir: base_dir.into() }
    }

    fn resolve_path(&self, path: &str) -> std::path::PathBuf {
        let p = Path::new(path);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.base_dir.join(p)
        }
    }
}

#[async_trait]
impl Tool for ReadTool {
    fn name(&self) -> &str { "read" }

    fn description(&self) -> &str {
        "Read a file and return its contents with line numbers."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path to read" },
                "offset": { "type": "integer", "description": "Start line (1-based)" },
                "limit": { "type": "integer", "description": "Number of lines to read" }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolOutput> {
        let path_str = args["path"].as_str().unwrap_or("");
        let path = self.resolve_path(path_str);

        if !path.exists() {
            return Ok(ToolOutput {
                content: format!("Error: file not found: {}", path_str),
                is_error: true,
            });
        }

        let bytes = std::fs::read(&path)?;

        // Binary detection: check for null bytes in first 8KB
        let check_len = bytes.len().min(8192);
        if bytes[..check_len].contains(&0) {
            return Ok(ToolOutput {
                content: format!("binary file detected: {}", path_str),
                is_error: false,
            });
        }

        let content = String::from_utf8_lossy(&bytes);
        let lines: Vec<&str> = content.lines().collect();

        let offset = args["offset"].as_u64().unwrap_or(1).max(1) as usize;
        let limit = args["limit"].as_u64().map(|n| n as usize);

        let start = (offset - 1).min(lines.len());
        let end = match limit {
            Some(n) => (start + n).min(lines.len()),
            None => lines.len(),
        };

        let mut output = String::new();
        for (i, line) in lines[start..end].iter().enumerate() {
            let line_num = start + i + 1;
            output.push_str(&format!("{:>6}\t{}\n", line_num, line));
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
    use tempfile::tempdir;

    fn setup() -> (tempfile::TempDir, ReadTool) {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("main.rs"), "fn main() {\n    println!(\"hi\");\n}\n").unwrap();
        std::fs::write(dir.path().join("empty.rs"), "").unwrap();
        std::fs::write(dir.path().join("binary.bin"), &[0u8, 1, 2, 0, 3]).unwrap();
        let tool = ReadTool::new(dir.path());
        (dir, tool)
    }

    #[tokio::test]
    async fn test_read_full_file() {
        let (_dir, tool) = setup();
        let result = tool.execute(json!({"path": "main.rs"})).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("fn main()"));
        assert!(result.content.contains("     1\t"));
    }

    #[tokio::test]
    async fn test_read_with_range() {
        let (_dir, tool) = setup();
        let result = tool.execute(json!({"path": "main.rs", "offset": 2, "limit": 1})).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("println!"));
        // Should only have 1 line
        assert_eq!(result.content.lines().count(), 1);
    }

    #[tokio::test]
    async fn test_read_file_not_found() {
        let (_dir, tool) = setup();
        let result = tool.execute(json!({"path": "nonexist.rs"})).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("not found"));
    }

    #[tokio::test]
    async fn test_read_binary_detection() {
        let (_dir, tool) = setup();
        let result = tool.execute(json!({"path": "binary.bin"})).await.unwrap();
        assert!(result.content.contains("binary file"));
    }

    #[tokio::test]
    async fn test_read_empty_file() {
        let (_dir, tool) = setup();
        let result = tool.execute(json!({"path": "empty.rs"})).await.unwrap();
        assert!(!result.is_error);
        assert_eq!(result.content, "");
    }
}
