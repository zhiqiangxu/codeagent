//! McpTool：将 MCP Server 上的工具包装为 forge-core Tool trait 实现。

use std::sync::Arc;

use async_trait::async_trait;
use forge_core::{Tool, ToolOutput};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::client::McpClient;
use crate::protocol::ToolDef;

/// 将 MCP 远程工具包装为本地 Tool trait 实现。
pub struct McpTool<R, W>
where
    R: AsyncRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send,
{
    client: Arc<McpClient<R, W>>,
    def: ToolDef,
}

impl<R, W> McpTool<R, W>
where
    R: AsyncRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send,
{
    pub fn new(client: Arc<McpClient<R, W>>, def: ToolDef) -> Self {
        Self { client, def }
    }
}

#[async_trait]
impl<R, W> Tool for McpTool<R, W>
where
    R: AsyncRead + Unpin + Send + Sync,
    W: AsyncWrite + Unpin + Send + Sync,
{
    fn name(&self) -> &str {
        &self.def.name
    }

    fn description(&self) -> &str {
        &self.def.description
    }

    fn schema(&self) -> serde_json::Value {
        self.def.input_schema.clone()
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolOutput> {
        match self.client.call_tool(&self.def.name, args).await {
            Ok(text) => Ok(ToolOutput {
                content: text,
                is_error: false,
            }),
            Err(e) => Ok(ToolOutput {
                content: e.to_string(),
                is_error: true,
            }),
        }
    }
}
