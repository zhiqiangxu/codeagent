//! LspRuntimePrompter：通过 LSP showMessageRequest 实现权限弹窗。

use async_trait::async_trait;
use forge_core::{PermissionDecision, RuntimePrompter};
use tower_lsp::lsp_types::*;
use tower_lsp::Client;

/// 通过 LSP Client 发送 showMessageRequest 的权限交互器。
pub struct LspRuntimePrompter {
    client: Client,
}

impl LspRuntimePrompter {
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}

/// 构造 approval 请求的消息和选项。
pub fn format_approval_message(tool_name: &str, args: &str) -> String {
    format!("Allow {} tool: {}?", tool_name, args)
}

/// 构造 action items。
pub fn approval_actions() -> Vec<MessageActionItem> {
    vec![
        MessageActionItem {
            title: "Allow".into(),
            properties: Default::default(),
        },
        MessageActionItem {
            title: "Deny".into(),
            properties: Default::default(),
        },
        MessageActionItem {
            title: "Always Allow".into(),
            properties: Default::default(),
        },
    ]
}

/// 将 IDE 的选择映射为 PermissionDecision。
pub fn parse_approval_response(
    response: Option<MessageActionItem>,
) -> PermissionDecision {
    match response.as_ref().map(|r| r.title.as_str()) {
        Some("Allow") => PermissionDecision::Allow,
        Some("Always Allow") => PermissionDecision::AlwaysAllow,
        _ => PermissionDecision::Deny,
    }
}

#[async_trait]
impl RuntimePrompter for LspRuntimePrompter {
    async fn ask(&self, tool_name: &str, args: &str) -> PermissionDecision {
        let message = format_approval_message(tool_name, args);
        let actions = approval_actions();
        match self
            .client
            .show_message_request(MessageType::WARNING, message, Some(actions))
            .await
        {
            Ok(response) => parse_approval_response(response),
            Err(_) => PermissionDecision::Deny,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lsp_prompter_format_request() {
        let message = format_approval_message("bash", "git status");
        assert!(message.contains("Allow bash"));
        assert!(message.contains("git status"));

        let actions = approval_actions();
        assert_eq!(actions.len(), 3);
        assert!(actions.iter().any(|a| a.title == "Allow"));
        assert!(actions.iter().any(|a| a.title == "Deny"));
        assert!(actions.iter().any(|a| a.title == "Always Allow"));
    }

    #[test]
    fn test_lsp_prompter_parse_allow() {
        let resp = Some(MessageActionItem {
            title: "Allow".into(),
            properties: Default::default(),
        });
        assert_eq!(parse_approval_response(resp), PermissionDecision::Allow);
    }

    #[test]
    fn test_lsp_prompter_parse_deny() {
        let resp = Some(MessageActionItem {
            title: "Deny".into(),
            properties: Default::default(),
        });
        assert_eq!(parse_approval_response(resp), PermissionDecision::Deny);
    }

    #[test]
    fn test_lsp_prompter_parse_always() {
        let resp = Some(MessageActionItem {
            title: "Always Allow".into(),
            properties: Default::default(),
        });
        assert_eq!(parse_approval_response(resp), PermissionDecision::AlwaysAllow);
    }

    #[test]
    fn test_lsp_prompter_parse_none() {
        assert_eq!(parse_approval_response(None), PermissionDecision::Deny);
    }
}
