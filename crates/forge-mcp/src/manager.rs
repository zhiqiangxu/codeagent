//! MCP Server 生命周期管理：启动、停止、自动重启、多 Server 隔离。

use std::collections::HashMap;
use std::sync::Mutex;

use serde::Deserialize;

/// MCP Server 配置。
#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

/// 配置文件结构（包含多个 server）。
#[derive(Debug, Clone, Deserialize)]
pub struct McpConfig {
    #[serde(default, rename = "mcp_servers")]
    pub servers: Vec<ServerConfig>,
}

/// 重启策略。
#[derive(Debug, Clone)]
pub struct RestartPolicy {
    pub max_restarts: usize,
    pub restart_count: usize,
}

impl RestartPolicy {
    pub fn new(max_restarts: usize) -> Self {
        Self {
            max_restarts,
            restart_count: 0,
        }
    }

    pub fn should_restart(&self) -> bool {
        self.restart_count < self.max_restarts
    }

    pub fn record_restart(&mut self) {
        self.restart_count += 1;
    }
}

/// Server 运行状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerStatus {
    Running,
    Stopped,
    Failed,
}

/// 管理的 Server 实例信息。
#[derive(Debug)]
pub struct ServerInstance {
    pub config: ServerConfig,
    pub pid: Option<u32>,
    pub status: ServerStatus,
    pub restart_policy: RestartPolicy,
}

/// MCP Server 管理器。
pub struct ServerManager {
    servers: Mutex<HashMap<String, ServerInstance>>,
}

impl ServerManager {
    pub fn new() -> Self {
        Self {
            servers: Mutex::new(HashMap::new()),
        }
    }

    /// 启动一个 MCP Server 子进程。
    pub async fn start(&self, config: ServerConfig) -> anyhow::Result<u32> {
        let child = tokio::process::Command::new(&config.command)
            .args(&config.args)
            .envs(&config.env)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()?;

        let pid = child.id().unwrap_or(0);
        let name = config.name.clone();

        let instance = ServerInstance {
            config,
            pid: Some(pid),
            status: ServerStatus::Running,
            restart_policy: RestartPolicy::new(3),
        };

        self.servers.lock().unwrap().insert(name, instance);
        Ok(pid)
    }

    /// 停止指定 Server（先 SIGTERM，超时后 SIGKILL）。
    pub async fn stop(&self, name: &str) -> anyhow::Result<()> {
        let pid = {
            let servers = self.servers.lock().unwrap();
            let instance = servers
                .get(name)
                .ok_or_else(|| anyhow::anyhow!("server not found: {}", name))?;
            instance.pid
        };

        if let Some(pid) = pid {
            // SIGTERM
            #[cfg(unix)]
            {
                unsafe {
                    libc::kill(pid as i32, libc::SIGTERM);
                }
            }

            // 等待 2 秒
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
            loop {
                if tokio::time::Instant::now() >= deadline {
                    // SIGKILL
                    #[cfg(unix)]
                    {
                        unsafe {
                            libc::kill(pid as i32, libc::SIGKILL);
                        }
                    }
                    break;
                }
                // 检查进程是否已退出
                #[cfg(unix)]
                {
                    let result = unsafe { libc::kill(pid as i32, 0) };
                    if result != 0 {
                        break; // 进程已退出
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }

        let mut servers = self.servers.lock().unwrap();
        if let Some(instance) = servers.get_mut(name) {
            instance.status = ServerStatus::Stopped;
            instance.pid = None;
        }

        Ok(())
    }

    /// 获取 Server 状态。
    pub fn status(&self, name: &str) -> Option<ServerStatus> {
        self.servers
            .lock()
            .unwrap()
            .get(name)
            .map(|i| i.status.clone())
    }

    /// 获取 Server PID。
    pub fn pid(&self, name: &str) -> Option<u32> {
        self.servers.lock().unwrap().get(name).and_then(|i| i.pid)
    }

    /// 尝试重启 Server。
    pub async fn restart(&self, name: &str) -> anyhow::Result<bool> {
        let (config, restart_policy) = {
            let mut servers = self.servers.lock().unwrap();
            let instance = servers
                .get_mut(name)
                .ok_or_else(|| anyhow::anyhow!("server not found: {}", name))?;

            if !instance.restart_policy.should_restart() {
                instance.status = ServerStatus::Failed;
                return Ok(false);
            }

            instance.restart_policy.record_restart();
            (instance.config.clone(), instance.restart_policy.clone())
        };

        // Start new process
        let child = tokio::process::Command::new(&config.command)
            .args(&config.args)
            .envs(&config.env)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()?;

        let pid = child.id().unwrap_or(0);

        // Update instance preserving restart count
        let mut servers = self.servers.lock().unwrap();
        if let Some(instance) = servers.get_mut(name) {
            instance.pid = Some(pid);
            instance.status = ServerStatus::Running;
            instance.restart_policy = restart_policy;
        }

        Ok(true)
    }

    /// 列出所有 Server 名称。
    pub fn list(&self) -> Vec<String> {
        self.servers.lock().unwrap().keys().cloned().collect()
    }
}

impl Default for ServerManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_config_parse() {
        let toml_str = r#"
[[mcp_servers]]
name = "web"
command = "node"
args = ["server.js"]
"#;
        let config: McpConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.servers.len(), 1);
        assert_eq!(config.servers[0].name, "web");
        assert_eq!(config.servers[0].command, "node");
        assert_eq!(config.servers[0].args, vec!["server.js"]);
    }

    #[test]
    fn test_server_config_invalid() {
        let toml_str = r#"
[[mcp_servers]]
name = "bad"
"#;
        let result: Result<McpConfig, _> = toml::from_str(toml_str);
        assert!(result.is_err());
    }

    #[test]
    fn test_restart_policy_max_3() {
        let policy = RestartPolicy {
            max_restarts: 3,
            restart_count: 3,
        };
        assert!(!policy.should_restart());
    }

    #[test]
    fn test_restart_policy_within_limit() {
        let policy = RestartPolicy {
            max_restarts: 3,
            restart_count: 1,
        };
        assert!(policy.should_restart());
    }
}
