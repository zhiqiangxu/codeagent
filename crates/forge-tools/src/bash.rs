use async_trait::async_trait;
use forge_core::{Tool, ToolOutput};
use serde_json::{json, Value};
use std::process::Stdio;
use tokio::process::Command;

pub struct BashTool {
    base_dir: std::path::PathBuf,
}

impl BashTool {
    pub fn new(base_dir: impl Into<std::path::PathBuf>) -> Self {
        Self { base_dir: base_dir.into() }
    }
}

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str { "bash" }
    fn description(&self) -> &str { "Execute a bash command." }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": { "type": "string" },
                "timeout": { "type": "integer", "description": "Timeout in milliseconds" }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolOutput> {
        let command = args["command"].as_str().unwrap_or("");
        let timeout_ms = args["timeout"].as_u64().unwrap_or(120_000);
        let cwd = args["cwd"]
            .as_str()
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| self.base_dir.clone());

        let child = Command::new("bash")
            .arg("-c")
            .arg(command)
            .current_dir(&cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();

        let child = match child {
            Ok(c) => c,
            Err(e) => {
                return Ok(ToolOutput {
                    content: format!("Failed to spawn bash: {}", e),
                    is_error: true,
                });
            }
        };

        let result = tokio::time::timeout(
            std::time::Duration::from_millis(timeout_ms),
            child.wait_with_output(),
        )
        .await;

        match result {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let mut content = stdout.to_string();
                if !stderr.is_empty() {
                    if !content.is_empty() {
                        content.push('\n');
                    }
                    content.push_str(&format!("STDERR:\n{}", stderr));
                }

                let is_error = !output.status.success();
                if is_error {
                    let code = output.status.code().unwrap_or(-1);
                    content.push_str(&format!("\nExit code: {}", code));
                }

                Ok(ToolOutput { content, is_error })
            }
            Ok(Err(e)) => Ok(ToolOutput {
                content: format!("Command failed: {}", e),
                is_error: true,
            }),
            Err(_) => {
                // timeout — process already consumed by wait_with_output future
                Ok(ToolOutput {
                    content: "timeout: command exceeded time limit".to_string(),
                    is_error: true,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_bash_stdout() {
        let dir = tempdir().unwrap();
        let tool = BashTool::new(dir.path());
        let result = tool.execute(json!({"command": "echo hello"})).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("hello"));
    }

    #[tokio::test]
    async fn test_bash_stderr() {
        let dir = tempdir().unwrap();
        let tool = BashTool::new(dir.path());
        let result = tool.execute(json!({"command": "echo err >&2"})).await.unwrap();
        assert!(result.content.contains("err"));
    }

    #[tokio::test]
    async fn test_bash_exit_code() {
        let dir = tempdir().unwrap();
        let tool = BashTool::new(dir.path());
        let result = tool.execute(json!({"command": "exit 1"})).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Exit code: 1"));
    }

    #[tokio::test]
    async fn test_bash_timeout() {
        let dir = tempdir().unwrap();
        let tool = BashTool::new(dir.path());
        let result = tool.execute(json!({"command": "sleep 10", "timeout": 100})).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("timeout"));
    }

    #[tokio::test]
    async fn test_bash_working_dir() {
        let dir = tempdir().unwrap();
        let tool = BashTool::new(dir.path());
        let result = tool.execute(json!({"command": "pwd"})).await.unwrap();
        assert!(!result.is_error);
        // The output should contain the tempdir path
        let canonical = dir.path().canonicalize().unwrap();
        assert!(result.content.contains(canonical.to_str().unwrap()));
    }
}
