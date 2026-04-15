//! MCP Client：通过 stdio 与 MCP Server 通信。

use std::sync::atomic::{AtomicU64, Ordering};

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

use crate::protocol::*;

/// MCP Client 错误类型。
#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error("connection closed")]
    ConnectionClosed,
    #[error("timeout waiting for response")]
    Timeout,
    #[error("rpc error ({code}): {message}")]
    Rpc { code: i32, message: String },
    #[error("{0}")]
    Other(#[from] anyhow::Error),
}

/// 最小 MCP Client，基于 stdio 传输（newline-delimited JSON）。
pub struct McpClient<R, W>
where
    R: AsyncRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send,
{
    reader: Mutex<BufReader<R>>,
    writer: Mutex<W>,
    next_id: AtomicU64,
    timeout: std::time::Duration,
}

impl<R, W> McpClient<R, W>
where
    R: AsyncRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send,
{
    /// 创建新的 MCP Client（不建立连接，只包装 IO）。
    pub fn new(reader: R, writer: W) -> Self {
        Self {
            reader: Mutex::new(BufReader::new(reader)),
            writer: Mutex::new(writer),
            next_id: AtomicU64::new(1),
            timeout: std::time::Duration::from_secs(30),
        }
    }

    pub fn with_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// 建立连接（发送 initialize 请求）。
    pub async fn connect(&self) -> Result<(), McpError> {
        let _resp = self
            .send_request("initialize", Some(serde_json::json!({})))
            .await?;
        Ok(())
    }

    /// 获取 Server 提供的工具列表。
    pub async fn list_tools(&self) -> Result<Vec<ToolDef>, McpError> {
        let resp = self.send_request("tools/list", None).await?;
        let result = resp
            .result
            .ok_or_else(|| McpError::Other(anyhow::anyhow!("missing result")))?;

        let tools_value = result
            .get("tools")
            .ok_or_else(|| McpError::Other(anyhow::anyhow!("missing tools field")))?;

        let tools: Vec<ToolDef> = serde_json::from_value(tools_value.clone())
            .map_err(|e| McpError::Other(e.into()))?;
        Ok(tools)
    }

    /// 调用 Server 上的工具。
    pub async fn call_tool(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<String, McpError> {
        let resp = self
            .send_request(
                "tools/call",
                Some(serde_json::json!({
                    "name": name,
                    "arguments": arguments,
                })),
            )
            .await?;

        let result = resp
            .result
            .ok_or_else(|| McpError::Other(anyhow::anyhow!("missing result")))?;

        let call_result: ToolCallResult = serde_json::from_value(result)
            .map_err(|e| McpError::Other(e.into()))?;

        let text = call_result
            .content
            .iter()
            .filter(|c| c.content_type == "text")
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        Ok(text)
    }

    /// 发送 JSON-RPC 请求并等待响应。
    async fn send_request(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<JsonRpcResponse, McpError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let mut request = JsonRpcRequest::new(id, method);
        if let Some(p) = params {
            request = request.with_params(p);
        }

        // 序列化并发送
        let mut line = serde_json::to_string(&request)
            .map_err(|e| McpError::Other(e.into()))?;
        line.push('\n');

        {
            let mut writer = self.writer.lock().await;
            writer
                .write_all(line.as_bytes())
                .await
                .map_err(|_| McpError::ConnectionClosed)?;
            writer.flush().await.map_err(|_| McpError::ConnectionClosed)?;
        }

        // 读取响应（带超时）
        let response = tokio::time::timeout(self.timeout, self.read_response())
            .await
            .map_err(|_| McpError::Timeout)?
            ?;

        // 检查 RPC 错误
        if let Some(err) = response.error {
            return Err(McpError::Rpc {
                code: err.code,
                message: err.message,
            });
        }

        Ok(response)
    }

    /// 从 reader 读取一行 JSON 并解析为 JsonRpcResponse。
    async fn read_response(&self) -> Result<JsonRpcResponse, McpError> {
        let mut line = String::new();
        let mut reader = self.reader.lock().await;
        let n = reader
            .read_line(&mut line)
            .await
            .map_err(|_| McpError::ConnectionClosed)?;
        if n == 0 {
            return Err(McpError::ConnectionClosed);
        }
        let response: JsonRpcResponse =
            serde_json::from_str(line.trim()).map_err(|e| McpError::Other(e.into()))?;
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_jsonrpc_serialize() {
        let req = JsonRpcRequest::new(1, "tools/list");
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"jsonrpc\":\"2.0\""));
        assert!(json.contains("\"method\":\"tools/list\""));
        assert!(json.contains("\"id\":1"));
    }

    #[test]
    fn test_mcp_jsonrpc_parse_response() {
        let json = r#"{
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "tools": [
                    {"name": "search", "description": "Search the web", "input_schema": {}},
                    {"name": "fetch", "description": "Fetch URL", "input_schema": {}}
                ]
            }
        }"#;

        let resp: JsonRpcResponse = serde_json::from_str(json).unwrap();
        let tools_value = resp.result.unwrap()["tools"].clone();
        let tools: Vec<ToolDef> = serde_json::from_value(tools_value).unwrap();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].name, "search");
        assert_eq!(tools[1].name, "fetch");
    }
}
