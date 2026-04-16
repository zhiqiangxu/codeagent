//! Skill 执行器：将 skill 脚本包装为 Tool trait 实现。

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use forge_core::{Tool, ToolOutput};

use super::scanner::SkillMeta;

/// Skill 作为 Tool 的执行器。
pub struct SkillTool {
    meta: SkillMeta,
    project_root: PathBuf,
    timeout: Duration,
}

impl SkillTool {
    pub fn new(meta: SkillMeta, project_root: impl Into<PathBuf>) -> Self {
        Self {
            meta,
            project_root: project_root.into(),
            timeout: Duration::from_secs(30),
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// 构建要执行的命令（可在测试中验证）。
    pub fn build_command(&self, args: &str) -> tokio::process::Command {
        let mut cmd = tokio::process::Command::new(&self.meta.path);
        if !args.is_empty() {
            cmd.arg(args);
        }
        cmd.current_dir(&self.project_root);
        cmd.env("PROJECT_DIR", &self.project_root);
        cmd.env("SKILL_NAME", &self.meta.name);
        cmd
    }
}

#[async_trait]
impl Tool for SkillTool {
    fn name(&self) -> &str {
        &self.meta.name
    }

    fn description(&self) -> &str {
        &self.meta.description
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "args": {
                    "type": "string",
                    "description": "Arguments to pass to the skill"
                }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolOutput> {
        let args_str = args
            .get("args")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let mut cmd = self.build_command(args_str);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let result = tokio::time::timeout(self.timeout, cmd.output()).await;

        match result {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();

                if output.status.success() {
                    Ok(ToolOutput {
                        content: stdout,
                        is_error: false,
                    })
                } else {
                    Ok(ToolOutput {
                        content: format!("{}\n{}", stdout, stderr).trim().to_string(),
                        is_error: true,
                    })
                }
            }
            Ok(Err(e)) => Ok(ToolOutput {
                content: format!("failed to execute skill: {}", e),
                is_error: true,
            }),
            Err(_) => Ok(ToolOutput {
                content: "skill execution timeout".to_string(),
                is_error: true,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_meta(name: &str, path: PathBuf) -> SkillMeta {
        SkillMeta {
            name: name.to_string(),
            description: format!("{} skill", name),
            usage: format!("/{}", name),
            path,
        }
    }

    #[test]
    fn test_skill_build_command() {
        let tmp = TempDir::new().unwrap();
        let script = tmp.path().join("commit.sh");
        std::fs::write(&script, "#!/bin/bash\necho done").unwrap();

        let tool = SkillTool::new(make_meta("commit", script.clone()), tmp.path());
        let cmd = tool.build_command("fix typo");

        let cmd_debug = format!("{:?}", cmd);
        assert!(cmd_debug.contains("commit.sh"));
        assert!(cmd_debug.contains("fix typo"));
    }

    #[test]
    fn test_skill_env_vars() {
        let tmp = TempDir::new().unwrap();
        let script = tmp.path().join("test.sh");
        std::fs::write(&script, "#!/bin/bash\necho $PROJECT_DIR $SKILL_NAME").unwrap();

        let tool = SkillTool::new(make_meta("test", script), tmp.path());
        let cmd = tool.build_command("");
        let envs: Vec<_> = cmd.as_std().get_envs().collect();

        assert!(envs.iter().any(|(k, _)| *k == "PROJECT_DIR"));
        assert!(envs.iter().any(|(k, _)| *k == "SKILL_NAME"));
    }
}
