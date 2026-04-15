use async_trait::async_trait;
use forge_core::{Tool, ToolOutput};
use serde_json::{json, Value};
use std::path::Path;

pub struct GlobTool {
    base_dir: std::path::PathBuf,
}

impl GlobTool {
    pub fn new(base_dir: impl Into<std::path::PathBuf>) -> Self {
        Self { base_dir: base_dir.into() }
    }
}

/// 常见的应忽略的目录。
const IGNORE_DIRS: &[&str] = &["target", "node_modules", ".git", "__pycache__", ".next", "dist", "build"];

fn should_ignore(path: &Path) -> bool {
    path.components().any(|c| {
        if let std::path::Component::Normal(s) = c {
            IGNORE_DIRS.contains(&s.to_str().unwrap_or(""))
        } else {
            false
        }
    })
}

#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str { "glob" }

    fn description(&self) -> &str {
        "Find files matching a glob pattern, sorted by modification time (newest first)."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Glob pattern (e.g. '**/*.rs')" },
                "path": { "type": "string", "description": "Base directory (default: working dir)" }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolOutput> {
        let pattern = args["pattern"].as_str().unwrap_or("*");
        let base = match args["path"].as_str() {
            Some(p) => std::path::PathBuf::from(p),
            None => self.base_dir.clone(),
        };

        let full_pattern = base.join(pattern).to_string_lossy().to_string();

        let mut entries: Vec<(std::path::PathBuf, std::time::SystemTime)> = Vec::new();

        for entry in glob::glob(&full_pattern).unwrap_or_else(|_| glob::glob("").unwrap()) {
            if let Ok(path) = entry {
                if path.is_file() && !should_ignore(&path) {
                    let mtime = path
                        .metadata()
                        .and_then(|m| m.modified())
                        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                    entries.push((path, mtime));
                }
            }
        }

        // Sort by mtime descending (newest first)
        entries.sort_by(|a, b| b.1.cmp(&a.1));

        let output: Vec<String> = entries
            .iter()
            .map(|(p, _)| {
                p.strip_prefix(&base)
                    .unwrap_or(p)
                    .to_string_lossy()
                    .to_string()
            })
            .collect();

        Ok(ToolOutput {
            content: output.join("\n"),
            is_error: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use std::thread::sleep;
    use std::time::Duration;

    fn setup() -> (tempfile::TempDir, GlobTool) {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(dir.path().join("lib.rs"), "pub fn hello() {}\n").unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/utils.rs"), "// utils\n").unwrap();
        std::fs::create_dir_all(dir.path().join("target/debug")).unwrap();
        std::fs::write(dir.path().join("target/debug/out"), "binary").unwrap();
        let tool = GlobTool::new(dir.path());
        (dir, tool)
    }

    #[tokio::test]
    async fn test_glob_simple() {
        let (_dir, tool) = setup();
        let result = tool.execute(json!({"pattern": "*.rs"})).await.unwrap();
        let files: Vec<&str> = result.content.lines().collect();
        assert!(files.contains(&"main.rs"));
        assert!(files.contains(&"lib.rs"));
    }

    #[tokio::test]
    async fn test_glob_recursive() {
        let (_dir, tool) = setup();
        let result = tool.execute(json!({"pattern": "**/*.rs"})).await.unwrap();
        let files: Vec<&str> = result.content.lines().collect();
        assert!(files.iter().any(|f| f.contains("utils.rs")));
        assert!(files.len() >= 3);
    }

    #[tokio::test]
    async fn test_glob_no_match() {
        let (_dir, tool) = setup();
        let result = tool.execute(json!({"pattern": "*.py"})).await.unwrap();
        assert_eq!(result.content, "");
    }

    #[tokio::test]
    async fn test_glob_respects_gitignore() {
        let (_dir, tool) = setup();
        let result = tool.execute(json!({"pattern": "**/*"})).await.unwrap();
        // target/ should be ignored
        assert!(!result.content.contains("target/"));
    }

    #[tokio::test]
    async fn test_glob_sorted_by_mtime() {
        let (dir, tool) = setup();
        // Touch main.rs to make it newer
        sleep(Duration::from_millis(50));
        std::fs::write(dir.path().join("main.rs"), "fn main() { /* updated */ }\n").unwrap();

        let result = tool.execute(json!({"pattern": "*.rs"})).await.unwrap();
        let files: Vec<&str> = result.content.lines().collect();
        // main.rs should be first (newest)
        assert_eq!(files[0], "main.rs");
    }
}
