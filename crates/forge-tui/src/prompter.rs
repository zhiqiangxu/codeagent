use async_trait::async_trait;
use forge_core::{PermissionDecision, RuntimePrompter};
use forge_permissions::{Action, Rule};

/// TUI 权限交互器：通过按键通道接收用户授权决策。
///
/// 实际 TUI 渲染层将用户按键（y/n/a）发送到通道中，
/// 此结构负责将按键转换为 PermissionDecision。
///
/// - 'y' → Allow（本次允许）
/// - 'n' → Deny（拒绝）
/// - 'a' → AlwaysAllow（永久允许，生成新 Rule）
pub struct TuiRuntimePrompter {
    key_rx: tokio::sync::Mutex<tokio::sync::mpsc::UnboundedReceiver<char>>,
    generated_rules: std::sync::Mutex<Vec<Rule>>,
}

impl TuiRuntimePrompter {
    pub fn new(key_rx: tokio::sync::mpsc::UnboundedReceiver<char>) -> Self {
        Self {
            key_rx: tokio::sync::Mutex::new(key_rx),
            generated_rules: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// 获取通过 "Always Allow" 生成的规则列表。
    pub fn generated_rules(&self) -> Vec<Rule> {
        self.generated_rules.lock().unwrap().clone()
    }
}

#[async_trait]
impl RuntimePrompter for TuiRuntimePrompter {
    async fn ask(&self, tool_name: &str, _args: &str) -> PermissionDecision {
        let mut rx = self.key_rx.lock().await;
        match rx.recv().await {
            Some('y') | Some('Y') => PermissionDecision::Allow,
            Some('a') | Some('A') => {
                // 生成 AutoAllow 规则
                let rule = Rule {
                    tool: tool_name.to_string(),
                    pattern: "*".to_string(),
                    action: Action::AutoAllow,
                };
                self.generated_rules.lock().unwrap().push(rule);
                PermissionDecision::AlwaysAllow
            }
            _ => PermissionDecision::Deny,
        }
    }
}
