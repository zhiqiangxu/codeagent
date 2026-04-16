//! LSP Server handler：处理 IDE 发来的 LSP 请求。

use std::collections::HashMap;
use std::sync::Mutex;

use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

/// 打开的文档记录。
#[derive(Debug, Clone)]
pub struct DocumentState {
    pub uri: String,
    pub content: String,
    pub version: i32,
}

/// LSP Server 内部状态。
pub struct ServerState {
    pub workspace_root: Mutex<Option<String>>,
    pub documents: Mutex<HashMap<String, DocumentState>>,
}

impl ServerState {
    pub fn new() -> Self {
        Self {
            workspace_root: Mutex::new(None),
            documents: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for ServerState {
    fn default() -> Self {
        Self::new()
    }
}

/// CodeForge LSP Server 实现。
pub struct CodeForgeLsp {
    pub client: Client,
    pub state: ServerState,
}

/// 构建 ServerCapabilities。
pub fn server_capabilities() -> ServerCapabilities {
    ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(
            TextDocumentSyncKind::FULL,
        )),
        ..Default::default()
    }
}

/// 从 InitializeParams 提取 workspace root。
pub fn extract_workspace_root(params: &InitializeParams) -> Option<String> {
    params
        .root_uri
        .as_ref()
        .map(|uri| uri.to_string())
        .or_else(|| {
            #[allow(deprecated)]
            params.root_path.clone()
        })
}

#[tower_lsp::async_trait]
impl LanguageServer for CodeForgeLsp {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        if let Some(root) = extract_workspace_root(&params) {
            *self.state.workspace_root.lock().unwrap() = Some(root);
        }

        Ok(InitializeResult {
            capabilities: server_capabilities(),
            server_info: Some(ServerInfo {
                name: "codeforge-lsp".to_string(),
                version: Some("0.1.0".to_string()),
            }),
        })
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.to_string();
        let doc = DocumentState {
            uri: uri.clone(),
            content: params.text_document.text,
            version: params.text_document.version,
        };
        self.state.documents.lock().unwrap().insert(uri, doc);
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri.to_string();
        if let Some(change) = params.content_changes.into_iter().last() {
            let mut docs = self.state.documents.lock().unwrap();
            if let Some(doc) = docs.get_mut(&uri) {
                doc.content = change.text;
                doc.version = params.text_document.version;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lsp_capabilities() {
        let caps = server_capabilities();
        assert!(caps.text_document_sync.is_some());
    }

    #[test]
    fn test_lsp_parse_initialize_params() {
        let params = InitializeParams {
            root_uri: Some(Url::parse("file:///home/user/project").unwrap()),
            ..Default::default()
        };
        let root = extract_workspace_root(&params);
        assert_eq!(root, Some("file:///home/user/project".to_string()));
    }
}
