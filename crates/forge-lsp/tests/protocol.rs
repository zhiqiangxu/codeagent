use tower_lsp::lsp_types::*;
use tower_lsp::{LspService, LanguageServer};
use forge_lsp::{CodeForgeLsp, ServerState};

fn create_server() -> (LspService<CodeForgeLsp>, tower_lsp::ClientSocket) {
    let (service, socket) = LspService::new(|client| CodeForgeLsp {
        client,
        state: ServerState::new(),
    });
    (service, socket)
}

fn init_params(root: &str) -> InitializeParams {
    InitializeParams {
        root_uri: Some(Url::parse(root).unwrap()),
        capabilities: ClientCapabilities::default(),
        ..Default::default()
    }
}

#[tokio::test]
async fn test_lsp_initialize() {
    let (service, _socket) = create_server();
    let server = service.inner();

    let result = server
        .initialize(init_params("file:///home/user/project"))
        .await
        .unwrap();

    assert!(result.capabilities.text_document_sync.is_some());
    assert_eq!(
        result.server_info.as_ref().unwrap().name,
        "codeforge-lsp"
    );
}

#[tokio::test]
async fn test_lsp_shutdown() {
    let (service, _socket) = create_server();
    let server = service.inner();

    server
        .initialize(init_params("file:///tmp/project"))
        .await
        .unwrap();
    let result = server.shutdown().await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_lsp_text_document_open() {
    let (service, _socket) = create_server();
    let server = service.inner();

    server
        .initialize(init_params("file:///tmp/project"))
        .await
        .unwrap();

    server
        .did_open(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: Url::parse("file:///tmp/project/main.rs").unwrap(),
                language_id: "rust".into(),
                version: 1,
                text: "fn main() {}".into(),
            },
        })
        .await;

    let docs = server.state.documents.lock().unwrap();
    let doc = docs.get("file:///tmp/project/main.rs").unwrap();
    assert_eq!(doc.content, "fn main() {}");
    assert_eq!(doc.version, 1);
}

#[tokio::test]
async fn test_lsp_text_document_change() {
    let (service, _socket) = create_server();
    let server = service.inner();

    server
        .initialize(init_params("file:///tmp/project"))
        .await
        .unwrap();

    let uri = Url::parse("file:///tmp/project/main.rs").unwrap();

    server
        .did_open(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: uri.clone(),
                language_id: "rust".into(),
                version: 1,
                text: "fn main() {}".into(),
            },
        })
        .await;

    server
        .did_change(DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier {
                uri: uri.clone(),
                version: 2,
            },
            content_changes: vec![TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: "fn main() { println!(\"hello\"); }".into(),
            }],
        })
        .await;

    let docs = server.state.documents.lock().unwrap();
    let doc = docs.get("file:///tmp/project/main.rs").unwrap();
    assert!(doc.content.contains("println"));
    assert_eq!(doc.version, 2);
}

#[tokio::test]
async fn test_lsp_workspace_root_extracted() {
    let (service, _socket) = create_server();
    let server = service.inner();

    server
        .initialize(init_params("file:///home/user/myproject"))
        .await
        .unwrap();

    let root = server.state.workspace_root.lock().unwrap().clone();
    assert_eq!(root, Some("file:///home/user/myproject".to_string()));
}
