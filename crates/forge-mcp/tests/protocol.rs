use forge_mcp::McpClient;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

async fn read_request(reader: &mut BufReader<tokio::io::DuplexStream>) -> (String, u64) {
    let mut line = String::new();
    reader.read_line(&mut line).await.unwrap();
    let req: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
    let method = req["method"].as_str().unwrap().to_string();
    let id = req["id"].as_u64().unwrap();
    (method, id)
}

async fn write_response(writer: &mut tokio::io::DuplexStream, id: u64, result: serde_json::Value) {
    let resp = serde_json::json!({"jsonrpc":"2.0","id":id,"result":result});
    let mut line = serde_json::to_string(&resp).unwrap();
    line.push('\n');
    writer.write_all(line.as_bytes()).await.unwrap();
    writer.flush().await.unwrap();
}

#[tokio::test]
async fn test_mcp_initialize_handshake() {
    let (client_read, mut server_write) = tokio::io::duplex(4096);
    let (server_read, client_write) = tokio::io::duplex(4096);
    let client = McpClient::new(client_read, client_write);
    let mut server_reader = BufReader::new(server_read);

    let server = tokio::spawn(async move {
        let (method, id) = read_request(&mut server_reader).await;
        assert_eq!(method, "initialize");
        write_response(
            &mut server_write,
            id,
            serde_json::json!({"capabilities": {"tools": true}}),
        )
        .await;
    });

    client.connect().await.unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn test_mcp_resources_list() {
    let (client_read, mut server_write) = tokio::io::duplex(4096);
    let (server_read, client_write) = tokio::io::duplex(4096);
    let client = McpClient::new(client_read, client_write);
    let mut server_reader = BufReader::new(server_read);

    let server = tokio::spawn(async move {
        let (method, id) = read_request(&mut server_reader).await;
        assert_eq!(method, "resources/list");
        write_response(
            &mut server_write,
            id,
            serde_json::json!({
                "resources": [
                    {"uri": "file://config.toml", "name": "config"},
                    {"uri": "file://rules.md", "name": "rules"}
                ]
            }),
        )
        .await;
    });

    let resp = client.send_request("resources/list", None).await.unwrap();
    let resources = resp.result.unwrap();
    let list = resources["resources"].as_array().unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0]["name"], "config");

    server.await.unwrap();
}

#[tokio::test]
async fn test_mcp_initialize_request_format() {
    // Verify initialize request contains proper JSON-RPC fields
    let (client_read, mut server_write) = tokio::io::duplex(4096);
    let (server_read, client_write) = tokio::io::duplex(4096);
    let client = McpClient::new(client_read, client_write);
    let mut server_reader = BufReader::new(server_read);

    let server = tokio::spawn(async move {
        let mut line = String::new();
        server_reader.read_line(&mut line).await.unwrap();
        let req: serde_json::Value = serde_json::from_str(line.trim()).unwrap();

        // Verify JSON-RPC 2.0 format
        assert_eq!(req["jsonrpc"], "2.0");
        assert_eq!(req["method"], "initialize");
        assert!(req["id"].is_number());

        let id = req["id"].as_u64().unwrap();
        write_response(&mut server_write, id, serde_json::json!({})).await;
    });

    client.connect().await.unwrap();
    server.await.unwrap();
}
