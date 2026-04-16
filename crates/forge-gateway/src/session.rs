//! Session 管理：创建、恢复、列出对话会话。

use std::collections::HashMap;
use std::sync::Mutex;

use forge_core::Message;

/// Session 元信息。
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub id: String,
    pub created_at: u64,
    pub message_count: usize,
}

/// Session 管理器。
pub struct SessionManager {
    sessions: Mutex<HashMap<String, Vec<Message>>>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
        }
    }

    /// 创建新 Session。
    pub fn create(&self) -> String {
        let id = format!("session-{}", uuid_short());
        self.sessions.lock().unwrap().insert(id.clone(), Vec::new());
        id
    }

    /// 获取 Session 的消息历史。
    pub fn get_messages(&self, id: &str) -> Option<Vec<Message>> {
        self.sessions.lock().unwrap().get(id).cloned()
    }

    /// 追加消息到 Session。
    pub fn append(&self, id: &str, messages: &[Message]) -> anyhow::Result<()> {
        let mut sessions = self.sessions.lock().unwrap();
        let session = sessions
            .get_mut(id)
            .ok_or_else(|| anyhow::anyhow!("session not found: {}", id))?;
        session.extend_from_slice(messages);
        Ok(())
    }

    /// 列出所有 Session。
    pub fn list(&self) -> Vec<SessionInfo> {
        self.sessions
            .lock()
            .unwrap()
            .iter()
            .map(|(id, msgs)| SessionInfo {
                id: id.clone(),
                created_at: 0,
                message_count: msgs.len(),
            })
            .collect()
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

fn uuid_short() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64;
    let count = COUNTER.fetch_add(1, Ordering::SeqCst);
    format!("{:x}-{}", ts & 0xFFFFFFFF, count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_core::{Content, Role};

    fn msg(text: &str) -> Message {
        Message {
            role: Role::User,
            content: Content::Text(text.into()),
            tool_calls: vec![],
        }
    }

    #[test]
    fn test_session_create() {
        let mgr = SessionManager::new();
        let id = mgr.create();
        assert!(id.starts_with("session-"));
    }

    #[test]
    fn test_session_append_and_get() {
        let mgr = SessionManager::new();
        let id = mgr.create();
        mgr.append(&id, &[msg("hello"), msg("world")]).unwrap();

        let messages = mgr.get_messages(&id).unwrap();
        assert_eq!(messages.len(), 2);
    }

    #[test]
    fn test_session_list() {
        let mgr = SessionManager::new();
        mgr.create();
        mgr.create();
        mgr.create();
        assert_eq!(mgr.list().len(), 3);
    }

    #[test]
    fn test_session_resume() {
        let mgr = SessionManager::new();
        let id = mgr.create();
        mgr.append(&id, &[msg("first")]).unwrap();
        mgr.append(&id, &[msg("second")]).unwrap();

        let messages = mgr.get_messages(&id).unwrap();
        assert_eq!(messages.len(), 2);
        assert!(matches!(&messages[0].content, Content::Text(t) if t == "first"));
        assert!(matches!(&messages[1].content, Content::Text(t) if t == "second"));
    }
}
