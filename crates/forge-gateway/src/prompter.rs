//! GrpcRuntimePrompter：通过 channel 实现远程权限交互。
//!
//! 实际 gRPC 传输在集成时替换；这里定义消息格式和逻辑。

use async_trait::async_trait;
use forge_core::{PermissionDecision, RuntimePrompter};
use serde::{Deserialize, Serialize};

/// 工具审批请求（发给 Client）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolApprovalRequest {
    pub tool: String,
    pub args: String,
    pub request_id: String,
}

/// 工具审批响应（Client 回复）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolApprovalResponse {
    pub request_id: String,
    pub decision: String, // "allow" | "deny" | "always_allow"
}

/// 解析审批响应为 PermissionDecision。
pub fn parse_decision(decision: &str) -> PermissionDecision {
    match decision {
        "allow" => PermissionDecision::Allow,
        "always_allow" => PermissionDecision::AlwaysAllow,
        _ => PermissionDecision::Deny,
    }
}

/// 构造审批请求。
pub fn format_request(tool: &str, args: &str) -> ToolApprovalRequest {
    ToolApprovalRequest {
        tool: tool.to_string(),
        args: args.to_string(),
        request_id: uuid_v4(),
    }
}

fn uuid_v4() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{:x}", ts)
}

/// Channel-based GrpcRuntimePrompter。
/// 通过 mpsc channel 与 gRPC stream handler 通信。
pub struct GrpcRuntimePrompter {
    request_tx: tokio::sync::mpsc::UnboundedSender<ToolApprovalRequest>,
    response_rx: tokio::sync::Mutex<tokio::sync::mpsc::UnboundedReceiver<ToolApprovalResponse>>,
}

impl GrpcRuntimePrompter {
    pub fn new(
        request_tx: tokio::sync::mpsc::UnboundedSender<ToolApprovalRequest>,
        response_rx: tokio::sync::mpsc::UnboundedReceiver<ToolApprovalResponse>,
    ) -> Self {
        Self {
            request_tx,
            response_rx: tokio::sync::Mutex::new(response_rx),
        }
    }
}

#[async_trait]
impl RuntimePrompter for GrpcRuntimePrompter {
    async fn ask(&self, tool_name: &str, args: &str) -> PermissionDecision {
        let req = format_request(tool_name, args);
        let request_id = req.request_id.clone();

        if self.request_tx.send(req).is_err() {
            return PermissionDecision::Deny;
        }

        // Wait for matching response
        let mut rx = self.response_rx.lock().await;
        match rx.recv().await {
            Some(resp) if resp.request_id == request_id => parse_decision(&resp.decision),
            _ => PermissionDecision::Deny,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grpc_prompter_format() {
        let req = format_request("write", "path=/a");
        assert_eq!(req.tool, "write");
        assert_eq!(req.args, "path=/a");
        assert!(!req.request_id.is_empty());
    }

    #[test]
    fn test_grpc_prompter_parse_response_allow() {
        assert_eq!(parse_decision("allow"), PermissionDecision::Allow);
    }

    #[test]
    fn test_grpc_prompter_parse_response_deny() {
        assert_eq!(parse_decision("deny"), PermissionDecision::Deny);
    }

    #[test]
    fn test_grpc_prompter_parse_response_always() {
        assert_eq!(parse_decision("always_allow"), PermissionDecision::AlwaysAllow);
    }

    #[test]
    fn test_grpc_prompter_parse_unknown() {
        assert_eq!(parse_decision("unknown"), PermissionDecision::Deny);
    }
}
