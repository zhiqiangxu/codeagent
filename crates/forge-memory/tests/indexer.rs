use std::sync::Arc;

use async_trait::async_trait;
use forge_core::{Content, EmbeddingProvider, Message, Role};
use forge_memory::fts_store::Fts5Store;
use forge_memory::indexer::{FileChange, FileChangeKind, IncrementalIndexer};
use forge_memory::vec_store::SqliteVecStore;
use tempfile::TempDir;

struct FakeEmbedding;

#[async_trait]
impl EmbeddingProvider for FakeEmbedding {
    async fn embed(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        Ok(texts
            .iter()
            .map(|t| {
                let seed = t.bytes().map(|b| b as f32).sum::<f32>();
                vec![seed * 0.001, seed * 0.002, seed * 0.003]
            })
            .collect())
    }
    fn dimension(&self) -> usize {
        3
    }
}

fn make_indexer(tmp: &TempDir) -> Arc<IncrementalIndexer> {
    let vec_store = Arc::new(
        SqliteVecStore::open(tmp.path().join("vec.db").to_str().unwrap(), 3).unwrap(),
    );
    let fts_store =
        Arc::new(Fts5Store::open(tmp.path().join("fts.db").to_str().unwrap()).unwrap());
    Arc::new(IncrementalIndexer::new(
        vec_store,
        fts_store,
        Arc::new(FakeEmbedding),
    ))
}

#[tokio::test]
async fn test_incremental_index_async() {
    let tmp = TempDir::new().unwrap();

    // 创建 3 个文件
    for i in 0..3 {
        let path = tmp.path().join(format!("file{}.rs", i));
        std::fs::write(&path, format!("fn func{}() {{}}", i)).unwrap();
    }

    let indexer = make_indexer(&tmp);
    let changes: Vec<FileChange> = (0..3)
        .map(|i| FileChange {
            kind: FileChangeKind::Created,
            path: tmp
                .path()
                .join(format!("file{}.rs", i))
                .to_str()
                .unwrap()
                .to_string(),
        })
        .collect();

    // 后台批量索引
    let handle = indexer.clone().process_batch(changes);

    // 主线程不阻塞（这里直接 await，实际使用中可以做其他事）
    handle.await.unwrap().unwrap();

    // 验证 3 个文件都被索引了 — 用 vec_store KNN 查询
    let vec_store = Arc::new(
        SqliteVecStore::open(tmp.path().join("vec.db").to_str().unwrap(), 3).unwrap(),
    );
    let results = vec_store.knn(&[0.1, 0.2, 0.3], 10).unwrap();
    assert_eq!(results.len(), 3);
}

#[tokio::test]
async fn test_index_conversation_turn() {
    let tmp = TempDir::new().unwrap();
    let indexer = make_indexer(&tmp);

    let messages = vec![
        Message {
            role: Role::User,
            content: Content::Text("explain the authentication module".into()),
            tool_calls: vec![],
        },
        Message {
            role: Role::Assistant,
            content: Content::Text("the auth module uses JWT tokens for session management".into()),
            tool_calls: vec![],
        },
    ];

    indexer.on_turn_end(&messages).await.unwrap();

    // 验证对话内容被索引
    let fts_store =
        Fts5Store::open(tmp.path().join("fts.db").to_str().unwrap()).unwrap();
    let results = fts_store.search("authentication", 10).unwrap();
    assert!(!results.is_empty());

    let results2 = fts_store.search("JWT", 10).unwrap();
    assert!(!results2.is_empty());
}
