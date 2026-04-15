use crate::profile::Profile;
use crate::rule::{find_matching_rule, Action, Rule};
use forge_core::{PermissionDecision, RuntimePrompter};
use std::path::Path;

/// PermissionGateway：三层权限检查（Profile → Rule → RuntimePrompter）。
pub struct PermissionGateway<P: RuntimePrompter> {
    profile: Profile,
    rules: Vec<Rule>,
    prompter: P,
}

impl<P: RuntimePrompter> PermissionGateway<P> {
    pub fn new(profile: Profile, rules: Vec<Rule>, prompter: P) -> Self {
        Self {
            profile,
            rules,
            prompter,
        }
    }

    /// 检查是否允许执行指定工具。
    pub async fn check(&mut self, tool: &str, args: &str) -> PermissionDecision {
        // Layer 1: Profile
        if !self.profile.allows(tool) {
            return PermissionDecision::Deny;
        }

        // Layer 2: Rules
        if let Some(rule) = find_matching_rule(&self.rules, tool, args) {
            return match rule.action {
                Action::AutoAllow => PermissionDecision::Allow,
                Action::AlwaysDeny => PermissionDecision::Deny,
                Action::Ask => {
                    // fallthrough to Layer 3
                    self.ask_and_maybe_save(tool, args).await
                }
            };
        }

        // Layer 3: RuntimePrompter (no rule matched)
        self.ask_and_maybe_save(tool, args).await
    }

    async fn ask_and_maybe_save(&mut self, tool: &str, args: &str) -> PermissionDecision {
        let decision = self.prompter.ask(tool, args).await;
        if decision == PermissionDecision::AlwaysAllow {
            // 生成新的 AutoAllow 规则
            self.rules.push(Rule {
                tool: tool.to_string(),
                pattern: args.to_string(),
                action: Action::AutoAllow,
            });
        }
        decision
    }

    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    /// 将规则保存到文件。
    pub fn save_rules(&self, path: &Path) -> anyhow::Result<()> {
        let json = serde_json::to_string_pretty(&self.rules)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// 从文件加载规则。
    pub fn load_rules(path: &Path) -> anyhow::Result<Vec<Rule>> {
        let json = std::fs::read_to_string(path)?;
        let rules: Vec<Rule> = serde_json::from_str(&json)?;
        Ok(rules)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use forge_core::PermissionDecision;

    // -- Mock RuntimePrompter for unit tests --

    struct NeverCalledPrompter;

    #[async_trait]
    impl RuntimePrompter for NeverCalledPrompter {
        async fn ask(&self, _tool: &str, _args: &str) -> PermissionDecision {
            panic!("RuntimePrompter should not be called in this test");
        }
    }

    #[tokio::test]
    async fn test_gateway_auto_allow_matching_rule() {
        let rules = vec![Rule::parse("Bash(git *)").unwrap()];
        let mut gw = PermissionGateway::new(Profile::Coding, rules, NeverCalledPrompter);
        let result = gw.check("Bash", "git status").await;
        assert_eq!(result, PermissionDecision::Allow);
    }

    #[tokio::test]
    async fn test_gateway_always_deny_matching_rule() {
        let rules = vec![Rule::parse("!Bash(rm *)").unwrap()];
        let mut gw = PermissionGateway::new(Profile::Coding, rules, NeverCalledPrompter);
        let result = gw.check("Bash", "rm -rf /").await;
        assert_eq!(result, PermissionDecision::Deny);
    }

    #[tokio::test]
    async fn test_gateway_readonly_blocks_write_tool() {
        let mut gw =
            PermissionGateway::new(Profile::ReadOnly, vec![], NeverCalledPrompter);
        let result = gw.check("write", "anything").await;
        assert_eq!(result, PermissionDecision::Deny);
    }
}
