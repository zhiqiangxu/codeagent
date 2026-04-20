//! PermissionToolExecutor: wraps ToolExecutor with Profile-based permission checks.

use async_trait::async_trait;
use forge_core::{ToolCall, ToolExecutor, ToolOutput};
use forge_permissions::Profile;

/// ToolExecutor wrapper that checks Profile permissions before executing tools.
pub struct PermissionToolExecutor<T: ToolExecutor> {
    inner: T,
    profile: Profile,
}

impl<T: ToolExecutor> PermissionToolExecutor<T> {
    pub fn new(inner: T, profile: Profile) -> Self {
        Self { inner, profile }
    }
}

#[async_trait]
impl<T: ToolExecutor> ToolExecutor for PermissionToolExecutor<T> {
    async fn execute(&self, call: &ToolCall) -> anyhow::Result<ToolOutput> {
        if !self.profile.allows(&call.name) {
            return Ok(ToolOutput {
                content: format!(
                    "permission denied: '{}' tool is blocked by {:?} profile",
                    call.name, self.profile
                ),
                is_error: true,
            });
        }
        self.inner.execute(call).await
    }

    fn tool_schemas(&self) -> Vec<serde_json::Value> {
        self.inner.tool_schemas()
    }
}
