use std::sync::Arc;

use forge_core::Tool;
use forge_mcp::{McpClient, McpTool, ToolDef};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

// ─── Mock Server Helper ────────────────────────────────

/// 从 duplex 读一行 JSON-RPC 请求，返回解析后的 method 和 id。
async fn read_request(
    reader: &mut BufReader<tokio::io::DuplexStream>,
) -> (String, u64) {
    let mut line = String::new();
    reader.read_line(&mut line).await.unwrap();
    let req: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
    let method = req["method"].as_str().unwrap().to_string();
    let id = req["id"].as_u64().unwrap();
    (method, id)
}

/// 向 duplex 写一行 JSON-RPC 响应。
async fn write_response(
    writer: &mut tokio::io::DuplexStream,
    id: u64,
    result: serde_json::Value,
) {
    let resp = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    });
    let mut line = serde_json::to_string(&resp).unwrap();
    line.push('\n');
    writer.write_all(line.as_bytes()).await.unwrap();
    writer.flush().await.unwrap();
}

// ─── Integration Tests ────────────────────────────────

#[tokio::test]
async fn test_mcp_client_connect() {
    let (client_read, mut server_write) = tokio::io::duplex(4096);
    let (server_read, client_write) = tokio::io::duplex(4096);

    let client = McpClient::new(client_read, client_write);
    let mut server_reader = BufReader::new(server_read);

    // 在后台处理 server 端
    let server = tokio::spawn(async move {
        let (method, id) = read_request(&mut server_reader).await;
        assert_eq!(method, "initialize");
        write_response(&mut server_write, id, serde_json::json!({})).await;
    });

    client.connect().await.unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn test_mcp_client_list_tools() {
    let (client_read, mut server_write) = tokio::io::duplex(4096);
    let (server_read, client_write) = tokio::io::duplex(4096);

    let client = McpClient::new(client_read, client_write);
    let mut server_reader = BufReader::new(server_read);

    let server = tokio::spawn(async move {
        let (method, id) = read_request(&mut server_reader).await;
        assert_eq!(method, "tools/list");
        write_response(
            &mut server_write,
            id,
            serde_json::json!({
                "tools": [
                    {"name": "search", "description": "Search", "input_schema": {"type": "object"}},
                    {"name": "fetch", "description": "Fetch URL", "input_schema": {}}
                ]
            }),
        )
        .await;
    });

    let tools = client.list_tools().await.unwrap();
    assert_eq!(tools.len(), 2);
    assert_eq!(tools[0].name, "search");
    assert_eq!(tools[1].name, "fetch");

    server.await.unwrap();
}

#[tokio::test]
async fn test_mcp_client_call_tool() {
    let (client_read, mut server_write) = tokio::io::duplex(4096);
    let (server_read, client_write) = tokio::io::duplex(4096);

    let client = McpClient::new(client_read, client_write);
    let mut server_reader = BufReader::new(server_read);

    let server = tokio::spawn(async move {
        let (method, id) = read_request(&mut server_reader).await;
        assert_eq!(method, "tools/call");
        write_response(
            &mut server_write,
            id,
            serde_json::json!({
                "content": [{"type": "text", "text": "search results here"}]
            }),
        )
        .await;
    });

    let result = client
        .call_tool("search", serde_json::json!({"query": "rust"}))
        .await
        .unwrap();
    assert_eq!(result, "search results here");

    server.await.unwrap();
}

#[tokio::test]
async fn test_mcp_client_timeout() {
    let (client_read, _server_write) = tokio::io::duplex(4096);
    let (_server_read, client_write) = tokio::io::duplex(4096);

    // 极短超时
    let client = McpClient::new(client_read, client_write)
        .with_timeout(std::time::Duration::from_millis(50));

    // Server 不响应 → 超时
    let result = client.list_tools().await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(err, forge_mcp::McpError::Timeout),
        "expected Timeout, got: {:?}",
        err
    );
}

#[tokio::test]
async fn test_mcp_client_server_crash() {
    let (client_read, server_write) = tokio::io::duplex(4096);
    let (server_read, client_write) = tokio::io::duplex(4096);

    // 立即 drop server 端
    drop(server_write);
    drop(server_read);

    let client = McpClient::new(client_read, client_write)
        .with_timeout(std::time::Duration::from_millis(500));

    let result = client.list_tools().await;
    assert!(result.is_err());
    // 可能是 ConnectionClosed 或 Timeout（取决于 OS 和 buffer 行为）
}

#[tokio::test]
async fn test_mcp_tool_as_trait() {
    let (client_read, mut server_write) = tokio::io::duplex(4096);
    let (server_read, client_write) = tokio::io::duplex(4096);

    let client = Arc::new(McpClient::new(client_read, client_write));
    let mut server_reader = BufReader::new(server_read);

    let def = ToolDef {
        name: "search".into(),
        description: "Web search".into(),
        input_schema: serde_json::json!({"type": "object", "properties": {"query": {"type": "string"}}}),
    };

    let tool = McpTool::new(client, def);

    // 验证 Tool trait 方法
    assert_eq!(tool.name(), "search");
    assert_eq!(tool.description(), "Web search");
    assert!(tool.schema().get("properties").is_some());

    // 执行工具
    let server = tokio::spawn(async move {
        let (method, id) = read_request(&mut server_reader).await;
        assert_eq!(method, "tools/call");
        write_response(
            &mut server_write,
            id,
            serde_json::json!({
                "content": [{"type": "text", "text": "result from mcp"}]
            }),
        )
        .await;
    });

    let output = tool
        .execute(serde_json::json!({"query": "test"}))
        .await
        .unwrap();
    assert_eq!(output.content, "result from mcp");
    assert!(!output.is_error);

    server.await.unwrap();
}
