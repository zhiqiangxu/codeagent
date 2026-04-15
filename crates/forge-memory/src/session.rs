use std::path::PathBuf;

use forge_core::Message;
use tokio::io::AsyncWriteExt;

/// JSONL 格式的会话持久化管理器。
///
/// 每条 Message 序列化为一行 JSON，支持增量 append。
pub struct SessionManager {
    path: PathBuf,
}

impl SessionManager {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// 全量保存消息列表到 JSONL 文件（覆盖写）。
    pub async fn save(&self, messages: &[Message]) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let mut file = tokio::fs::File::create(&self.path).await?;
        for msg in messages {
            let line = serde_json::to_string(msg)?;
            file.write_all(line.as_bytes()).await?;
            file.write_all(b"\n").await?;
        }
        file.flush().await?;
        Ok(())
    }

    /// 从 JSONL 文件加载全部消息。
    pub async fn load(&self) -> anyhow::Result<Vec<Message>> {
        let content = tokio::fs::read_to_string(&self.path).await?;
        let mut messages = Vec::new();
        for line in content.lines() {
            if !line.is_empty() {
                let msg: Message = serde_json::from_str(line)?;
                messages.push(msg);
            }
        }
        Ok(messages)
    }

    /// 增量追加一条消息到 JSONL 文件末尾。
    pub async fn append(&self, message: &Message) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await?;
        let line = serde_json::to_string(message)?;
        file.write_all(line.as_bytes()).await?;
        file.write_all(b"\n").await?;
        file.flush().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_core::{Content, Role};
    use tempfile::TempDir;

    fn make_message(role: Role, text: &str) -> Message {
        Message {
            role,
            content: Content::Text(text.to_string()),
            tool_calls: vec![],
        }
    }

    #[tokio::test]
    async fn test_session_save_jsonl() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("session.jsonl");
        let mgr = SessionManager::new(&path);

        let messages = vec![
            make_message(Role::User, "hello"),
            make_message(Role::Assistant, "hi"),
            make_message(Role::User, "bye"),
        ];
        mgr.save(&messages).await.unwrap();

        let content = tokio::fs::read_to_string(&path).await.unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 3);
    }

    #[tokio::test]
    async fn test_session_load_jsonl() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("session.jsonl");
        let mgr = SessionManager::new(&path);

        let messages = vec![
            make_message(Role::User, "hello"),
            make_message(Role::Assistant, "hi there"),
            make_message(Role::System, "you are helpful"),
        ];
        mgr.save(&messages).await.unwrap();

        let loaded = mgr.load().await.unwrap();
        assert_eq!(loaded.len(), 3);
        assert_eq!(loaded[0].role, Role::User);
        assert_eq!(loaded[1].content, Content::Text("hi there".into()));
        assert_eq!(loaded[2].role, Role::System);
    }

    #[tokio::test]
    async fn test_session_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("session.jsonl");
        let mgr = SessionManager::new(&path);

        let messages = vec![
            make_message(Role::User, "question"),
            make_message(Role::Assistant, "answer"),
            Message {
                role: Role::Assistant,
                content: Content::Text("with tools".into()),
                tool_calls: vec![forge_core::ToolCall {
                    id: "tc1".into(),
                    name: "read".into(),
                    arguments: serde_json::json!({"path": "/a"}),
                }],
            },
        ];
        mgr.save(&messages).await.unwrap();
        let loaded = mgr.load().await.unwrap();

        assert_eq!(messages, loaded);
    }

    #[tokio::test]
    async fn test_session_append() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("session.jsonl");
        let mgr = SessionManager::new(&path);

        let initial = vec![
            make_message(Role::User, "first"),
            make_message(Role::Assistant, "second"),
        ];
        mgr.save(&initial).await.unwrap();

        mgr.append(&make_message(Role::User, "third")).await.unwrap();

        let loaded = mgr.load().await.unwrap();
        assert_eq!(loaded.len(), 3);
        assert_eq!(loaded[2].content, Content::Text("third".into()));
    }
}
