use async_trait::async_trait;
use forge_core::{Tool, ToolOutput};
use regex::Regex;
use serde_json::{json, Value};
use std::path::Path;

pub struct GrepTool {
    base_dir: std::path::PathBuf,
}

impl GrepTool {
    pub fn new(base_dir: impl Into<std::path::PathBuf>) -> Self {
        Self { base_dir: base_dir.into() }
    }
}

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str { "grep" }

    fn description(&self) -> &str {
        "Search file contents using regex patterns."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Regex pattern to search" },
                "path": { "type": "string", "description": "File or directory to search" },
                "glob": { "type": "string", "description": "File glob filter (e.g. '*.rs')" },
                "context": { "type": "integer", "description": "Lines of context around matches" },
                "case_insensitive": { "type": "boolean", "description": "Case insensitive search" }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolOutput> {
        let pattern_str = args["pattern"].as_str().unwrap_or("");
        let case_insensitive = args["case_insensitive"].as_bool().unwrap_or(false);
        let context_lines = args["context"].as_u64().unwrap_or(0) as usize;
        let file_glob = args["glob"].as_str();

        let regex_pattern = if case_insensitive {
            format!("(?i){}", pattern_str)
        } else {
            pattern_str.to_string()
        };

        let re = match Regex::new(&regex_pattern) {
            Ok(r) => r,
            Err(e) => {
                return Ok(ToolOutput {
                    content: format!("Invalid regex: {}", e),
                    is_error: true,
                });
            }
        };

        let base = match args["path"].as_str() {
            Some(p) => {
                let pb = Path::new(p);
                if pb.is_absolute() { pb.to_path_buf() } else { self.base_dir.join(pb) }
            }
            None => self.base_dir.clone(),
        };

        let mut results = Vec::new();

        if base.is_file() {
            search_file(&base, &re, context_lines, &self.base_dir, &mut results);
        } else {
            let glob_pattern = match file_glob {
                Some(g) => base.join("**").join(g).to_string_lossy().to_string(),
                None => base.join("**/*").to_string_lossy().to_string(),
            };

            for entry in glob::glob(&glob_pattern).unwrap_or_else(|_| glob::glob("").unwrap()) {
                if let Ok(path) = entry {
                    if path.is_file() {
                        search_file(&path, &re, context_lines, &self.base_dir, &mut results);
                    }
                }
            }
        }

        Ok(ToolOutput {
            content: results.join("\n"),
            is_error: false,
        })
    }
}

fn search_file(
    path: &Path,
    re: &Regex,
    context: usize,
    base_dir: &Path,
    results: &mut Vec<String>,
) {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return, // skip binary/unreadable files
    };

    let lines: Vec<&str> = content.lines().collect();
    let rel_path = path.strip_prefix(base_dir).unwrap_or(path);

    for (i, line) in lines.iter().enumerate() {
        if re.is_match(line) {
            let start = i.saturating_sub(context);
            let end = (i + context + 1).min(lines.len());

            for j in start..end {
                let prefix = if j == i { ">" } else { " " };
                results.push(format!(
                    "{}{}:{}:{}",
                    prefix,
                    rel_path.display(),
                    j + 1,
                    lines[j]
                ));
            }
            if context > 0 {
                results.push("--".to_string());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn setup() -> (tempfile::TempDir, GrepTool) {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("main.rs"), "fn main() {\n    println!(\"hello\");\n}\n").unwrap();
        std::fs::write(dir.path().join("lib.rs"), "pub fn hello() {}\npub fn world() {}\n").unwrap();
        let tool = GrepTool::new(dir.path());
        (dir, tool)
    }

    #[tokio::test]
    async fn test_grep_literal() {
        let (_dir, tool) = setup();
        let result = tool.execute(json!({"pattern": "fn main"})).await.unwrap();
        assert!(result.content.contains("main.rs"));
        assert!(result.content.contains("fn main"));
    }

    #[tokio::test]
    async fn test_grep_regex() {
        let (_dir, tool) = setup();
        let result = tool.execute(json!({"pattern": "fn \\w+"})).await.unwrap();
        assert!(result.content.contains("main.rs"));
        assert!(result.content.contains("lib.rs"));
    }

    #[tokio::test]
    async fn test_grep_with_context() {
        let (_dir, tool) = setup();
        let result = tool.execute(json!({"pattern": "fn main", "context": 1})).await.unwrap();
        // Should contain the line after "fn main"
        assert!(result.content.contains("println!"));
    }

    #[tokio::test]
    async fn test_grep_file_filter() {
        let (_dir, tool) = setup();
        let result = tool.execute(json!({"pattern": "fn", "glob": "lib.rs"})).await.unwrap();
        assert!(result.content.contains("lib.rs"));
        assert!(!result.content.contains("main.rs"));
    }

    #[tokio::test]
    async fn test_grep_case_insensitive() {
        let (_dir, tool) = setup();
        let result = tool.execute(json!({"pattern": "FN MAIN", "case_insensitive": true})).await.unwrap();
        assert!(result.content.contains("fn main"));
    }

    #[tokio::test]
    async fn test_grep_no_match() {
        let (_dir, tool) = setup();
        let result = tool.execute(json!({"pattern": "zzz_not_exist"})).await.unwrap();
        assert_eq!(result.content, "");
    }
}
