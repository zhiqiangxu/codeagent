use std::sync::Arc;

use async_trait::async_trait;
use forge_core::{
    AgentLoop, Content, Role, SimpleContextEngine, StreamEvent, TokenUsage,
};
use forge_memory::{ForgemdRetriever, SessionManager};
use forge_core::noop::{NoopCompaction, NoopRetriever};
use forge_test_utils::{create_test_project, ScriptedModelProvider};
use forge_tools::read::ReadTool;
use forge_tools::write::WriteTool;
use forge_tools::edit::EditTool;
use forge_tools::ToolRegistry;
use tempfile::TempDir;

// ─── Helpers ───────────────────────────────────────────

fn done_event() -> StreamEvent {
    StreamEvent::Done {
        usage: TokenUsage {
            input: 10,
            output: 5,
        },
    }
}

fn text_response(chunks: &[&str]) -> Vec<StreamEvent> {
    let mut events: Vec<StreamEvent> = chunks
        .iter()
        .map(|c| StreamEvent::Delta {
            content: c.to_string(),
        })
        .collect();
    events.push(done_event());
    events
}

fn tool_call_response(calls: Vec<(&str, &str, serde_json::Value)>) -> Vec<StreamEvent> {
    let mut events: Vec<StreamEvent> = calls
        .into_iter()
        .map(|(id, name, args)| StreamEvent::ToolCall {
            id: id.to_string(),
            name: name.to_string(),
            arguments: args,
        })
        .collect();
    events.push(done_event());
    events
}

fn build_registry(base_dir: &std::path::Path) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry
        .register(Arc::new(ReadTool::new(base_dir)))
        .unwrap();
    registry
        .register(Arc::new(WriteTool::new(base_dir)))
        .unwrap();
    registry
        .register(Arc::new(EditTool::new(base_dir)))
        .unwrap();
    registry
}

fn char_counter(messages: &[forge_core::Message]) -> usize {
    messages
        .iter()
        .map(|m| match &m.content {
            Content::Text(t) => t.len(),
            Content::ToolResult { output, .. } => output.len(),
        })
        .sum()
}

fn build_context(
    retriever: Box<dyn forge_core::MemoryRetriever>,
) -> SimpleContextEngine {
    SimpleContextEngine::new(
        retriever,
        Box::new(NoopCompaction),
        vec![],
        "You are a helpful coding assistant.".into(),
        Box::new(char_counter),
    )
}

fn make_channel() -> (
    tokio::sync::mpsc::UnboundedSender<forge_core::AgentEvent>,
    tokio::sync::mpsc::UnboundedReceiver<forge_core::AgentEvent>,
) {
    tokio::sync::mpsc::unbounded_channel()
}

// ─── E2E Tests ─────────────────────────────────────────

#[tokio::test]
async fn test_e2e_simple_chat() {
    let model = ScriptedModelProvider::new(vec![text_response(&["hello"])]);
    let context = build_context(Box::new(NoopRetriever));
    let tools = ToolRegistry::new();

    let (tx, _rx) = make_channel();
    let mut agent = AgentLoop::new(model, context, tools, 5);
    let result = agent.run("hi", tx).await.unwrap();

    assert!(result.contains("hello"));
}

#[tokio::test]
async fn test_e2e_read_file() {
    let project = create_test_project();
    std::fs::write(project.path().join("test.txt"), "secret content 42").unwrap();

    let file_path = project.path().join("test.txt");
    let model = ScriptedModelProvider::new(vec![
        tool_call_response(vec![(
            "tc1",
            "read",
            serde_json::json!({"path": file_path.to_str().unwrap()}),
        )]),
        text_response(&["I found the content"]),
    ]);

    let context = build_context(Box::new(NoopRetriever));
    let tools = build_registry(project.path());

    let (tx, _rx) = make_channel();
    let mut agent = AgentLoop::new(model, context, tools, 5);
    let result = agent.run("read the file", tx).await.unwrap();

    assert_eq!(result, "I found the content");
}

#[tokio::test]
async fn test_e2e_write_file() {
    let project = TempDir::new().unwrap();

    let file_path = project.path().join("output.txt");
    let model = ScriptedModelProvider::new(vec![
        tool_call_response(vec![(
            "tc1",
            "write",
            serde_json::json!({
                "path": file_path.to_str().unwrap(),
                "content": "written by agent"
            }),
        )]),
        text_response(&["file written"]),
    ]);

    let context = build_context(Box::new(NoopRetriever));
    let tools = build_registry(project.path());

    let (tx, _rx) = make_channel();
    let mut agent = AgentLoop::new(model, context, tools, 5);
    agent.run("write a file", tx).await.unwrap();

    // 验证文件确实被创建
    let content = std::fs::read_to_string(&file_path).unwrap();
    assert_eq!(content, "written by agent");
}

#[tokio::test]
async fn test_e2e_tool_chain() {
    let project = create_test_project();
    let file_path = project.path().join("chain.txt");
    std::fs::write(&file_path, "original line\nsecond line\n").unwrap();

    let model = ScriptedModelProvider::new(vec![
        // 先读
        tool_call_response(vec![(
            "tc1",
            "read",
            serde_json::json!({"path": file_path.to_str().unwrap()}),
        )]),
        // 再改
        tool_call_response(vec![(
            "tc2",
            "edit",
            serde_json::json!({
                "path": file_path.to_str().unwrap(),
                "old_string": "original line",
                "new_string": "modified line"
            }),
        )]),
        text_response(&["done"]),
    ]);

    let context = build_context(Box::new(NoopRetriever));
    let tools = build_registry(project.path());

    let (tx, _rx) = make_channel();
    let mut agent = AgentLoop::new(model, context, tools, 5);
    agent.run("read then edit", tx).await.unwrap();

    let content = std::fs::read_to_string(&file_path).unwrap();
    assert!(content.contains("modified line"));
    assert!(!content.contains("original line"));
}

#[tokio::test]
async fn test_e2e_session_resume() {
    let project = TempDir::new().unwrap();
    let session_path = project.path().join("session.jsonl");

    // 第 1 轮
    {
        let model = ScriptedModelProvider::new(vec![text_response(&["first reply"])]);
        let context = build_context(Box::new(NoopRetriever));
        let tools = ToolRegistry::new();
        let session = SessionManager::new(&session_path);

        let (tx, _rx) = make_channel();

        // 手动用 SessionStore 包装
        let mut agent = AgentLoop::new(model, context, tools, 5)
            .with_session(Box::new(SessionStoreAdapter(session)));
        agent.run("first input", tx).await.unwrap();
    }

    // 验证 session 已保存
    let session = SessionManager::new(&session_path);
    let messages = session.load().await.unwrap();
    assert!(messages.len() >= 2); // user + assistant
    assert_eq!(messages[0].role, Role::User);
    assert!(matches!(&messages[0].content, Content::Text(t) if t == "first input"));
    assert_eq!(messages[1].role, Role::Assistant);
    assert!(matches!(&messages[1].content, Content::Text(t) if t == "first reply"));
}

/// SessionManager → SessionStore adapter
struct SessionStoreAdapter(SessionManager);

#[async_trait]
impl forge_core::SessionStore for SessionStoreAdapter {
    async fn save(&self, messages: &[forge_core::Message]) -> anyhow::Result<()> {
        self.0.save(messages).await
    }
}

#[tokio::test]
async fn test_e2e_forge_md_loaded() {
    let project = TempDir::new().unwrap();

    // 创建 FORGE.md
    let forge_dir = project.path().join(".codeforge");
    std::fs::create_dir_all(&forge_dir).unwrap();
    std::fs::write(forge_dir.join("FORGE.md"), "custom rule: always use snake_case").unwrap();

    // ScriptedModel：我们通过检查 assemble 后注入 system prompt 来验证
    // 模型收到的消息中应包含 FORGE.md 内容
    let model = ScriptedModelProvider::new(vec![text_response(&["got it"])]);

    let retriever = ForgemdRetriever::new(
        project.path(), // global dir（这里和 project 共用简化测试）
        project.path(),
    );
    let context = build_context(Box::new(retriever));
    let tools = ToolRegistry::new();

    let (tx, _rx) = make_channel();
    let mut agent = AgentLoop::new(model, context, tools, 5);
    let result = agent.run("test", tx).await.unwrap();

    // 如果 FORGE.md 加载失败，SimpleContextEngine 不会注入内容
    // 这里只验证 agent 正常运行（FORGE.md 内容注入在 context tests 中已验证）
    assert_eq!(result, "got it");
}
