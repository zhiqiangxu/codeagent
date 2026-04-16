use forge_core::{Content, PermissionDecision, Role, RuntimePrompter};
use forge_gateway::{
    GrpcRuntimePrompter, SessionManager, ToolApprovalResponse,
};

#[tokio::test]
async fn test_grpc_prompter_approval() {
    let (req_tx, mut req_rx) = tokio::sync::mpsc::unbounded_channel();
    let (resp_tx, resp_rx) = tokio::sync::mpsc::unbounded_channel();

    let prompter = GrpcRuntimePrompter::new(req_tx, resp_rx);

    // Simulate client responding "allow"
    let handle = tokio::spawn(async move {
        let req = req_rx.recv().await.unwrap();
        resp_tx
            .send(ToolApprovalResponse {
                request_id: req.request_id,
                decision: "allow".into(),
            })
            .unwrap();
    });

    let decision = prompter.ask("bash", "ls").await;
    assert_eq!(decision, PermissionDecision::Allow);
    handle.await.unwrap();
}

#[tokio::test]
async fn test_grpc_prompter_deny() {
    let (req_tx, mut req_rx) = tokio::sync::mpsc::unbounded_channel();
    let (resp_tx, resp_rx) = tokio::sync::mpsc::unbounded_channel();

    let prompter = GrpcRuntimePrompter::new(req_tx, resp_rx);

    tokio::spawn(async move {
        let req = req_rx.recv().await.unwrap();
        resp_tx
            .send(ToolApprovalResponse {
                request_id: req.request_id,
                decision: "deny".into(),
            })
            .unwrap();
    });

    let decision = prompter.ask("write", "/etc/passwd").await;
    assert_eq!(decision, PermissionDecision::Deny);
}

#[tokio::test]
async fn test_grpc_session_create_and_resume() {
    let mgr = SessionManager::new();

    let id = mgr.create();
    mgr.append(
        &id,
        &[forge_core::Message {
            role: Role::User,
            content: Content::Text("hello from CLI".into()),
            tool_calls: vec![],
        }],
    )
    .unwrap();

    // "Resume" from another client
    let messages = mgr.get_messages(&id).unwrap();
    assert_eq!(messages.len(), 1);
    assert!(matches!(&messages[0].content, Content::Text(t) if t.contains("CLI")));
}

#[tokio::test]
async fn test_grpc_session_list() {
    let mgr = SessionManager::new();
    mgr.create();
    mgr.create();
    mgr.create();

    let sessions = mgr.list();
    assert_eq!(sessions.len(), 3);
}

#[tokio::test]
async fn test_grpc_multi_client_isolation() {
    let mgr = SessionManager::new();

    let id1 = mgr.create();
    let id2 = mgr.create();

    mgr.append(
        &id1,
        &[forge_core::Message {
            role: Role::User,
            content: Content::Text("client 1 message".into()),
            tool_calls: vec![],
        }],
    )
    .unwrap();

    mgr.append(
        &id2,
        &[forge_core::Message {
            role: Role::User,
            content: Content::Text("client 2 message".into()),
            tool_calls: vec![],
        }],
    )
    .unwrap();

    let msgs1 = mgr.get_messages(&id1).unwrap();
    let msgs2 = mgr.get_messages(&id2).unwrap();

    assert_eq!(msgs1.len(), 1);
    assert_eq!(msgs2.len(), 1);
    assert!(matches!(&msgs1[0].content, Content::Text(t) if t.contains("client 1")));
    assert!(matches!(&msgs2[0].content, Content::Text(t) if t.contains("client 2")));
}
