//! 增量索引器：响应文件变更和对话回合，更新向量/FTS 索引。

use forge_core::{Content, EmbeddingProvider, Message};

use crate::fts_store::Fts5Store;
use crate::vec_store::SqliteVecStore;

/// 文件变更类型。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileChangeKind {
    Created,
    Modified,
    Deleted,
}

/// 文件变更事件。
#[derive(Debug, Clone)]
pub struct FileChange {
    pub kind: FileChangeKind,
    pub path: String,
}

/// 增量索引器。
pub struct IncrementalIndexer {
    vec_store: std::sync::Arc<SqliteVecStore>,
    fts_store: std::sync::Arc<Fts5Store>,
    embedding: std::sync::Arc<dyn EmbeddingProvider>,
}

impl IncrementalIndexer {
    pub fn new(
        vec_store: std::sync::Arc<SqliteVecStore>,
        fts_store: std::sync::Arc<Fts5Store>,
        embedding: std::sync::Arc<dyn EmbeddingProvider>,
    ) -> Self {
        Self {
            vec_store,
            fts_store,
            embedding,
        }
    }

    /// 处理单个文件变更。
    pub async fn on_file_change(&self, change: &FileChange) -> anyhow::Result<()> {
        match change.kind {
            FileChangeKind::Created | FileChangeKind::Modified => {
                let content = tokio::fs::read_to_string(&change.path).await?;

                // 先删旧记录（Modified 场景）
                self.vec_store.delete(&change.path)?;
                self.fts_store.delete(&change.path)?;

                // FTS 索引
                self.fts_store.index(&change.path, &content)?;

                // 向量索引
                let embeddings = self.embedding.embed(&[content.clone()]).await?;
                if let Some(vec) = embeddings.first() {
                    self.vec_store
                        .insert(&change.path, &content, vec, Some(&change.path))?;
                }
            }
            FileChangeKind::Deleted => {
                self.vec_store.delete(&change.path)?;
                self.fts_store.delete(&change.path)?;
            }
        }
        Ok(())
    }

    /// 处理批量文件变更（异步，返回 JoinHandle）。
    pub fn process_batch(
        self: std::sync::Arc<Self>,
        changes: Vec<FileChange>,
    ) -> tokio::task::JoinHandle<anyhow::Result<()>> {
        tokio::spawn(async move {
            for change in &changes {
                self.on_file_change(change).await?;
            }
            Ok(())
        })
    }

    /// 对话回合结束后索引新消息。
    pub async fn on_turn_end(&self, messages: &[Message]) -> anyhow::Result<()> {
        for (i, msg) in messages.iter().enumerate() {
            let content = match &msg.content {
                Content::Text(t) => t.clone(),
                Content::ToolResult { output, .. } => output.clone(),
            };
            if content.is_empty() {
                continue;
            }

            let id = format!("msg:{}", i);

            // FTS 索引
            self.fts_store.index(&id, &content)?;

            // 向量索引
            let embeddings = self.embedding.embed(&[content.clone()]).await?;
            if let Some(vec) = embeddings.first() {
                self.vec_store
                    .insert(&id, &content, vec, Some("conversation"))?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Arc;
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

    fn make_indexer(tmp: &TempDir) -> IncrementalIndexer {
        let vec_store = Arc::new(
            SqliteVecStore::open(tmp.path().join("vec.db").to_str().unwrap(), 3).unwrap(),
        );
        let fts_store =
            Arc::new(Fts5Store::open(tmp.path().join("fts.db").to_str().unwrap()).unwrap());
        IncrementalIndexer::new(vec_store, fts_store, Arc::new(FakeEmbedding))
    }

    #[tokio::test]
    async fn test_incremental_index_new_file() {
        let tmp = TempDir::new().unwrap();
        let file_path = tmp.path().join("new.rs");
        std::fs::write(&file_path, "fn hello() { println!(\"hello\"); }").unwrap();

        let indexer = make_indexer(&tmp);
        indexer
            .on_file_change(&FileChange {
                kind: FileChangeKind::Created,
                path: file_path.to_str().unwrap().to_string(),
            })
            .await
            .unwrap();

        // 验证 FTS 能搜到
        let fts_results = indexer.fts_store.search("hello", 10).unwrap();
        assert!(!fts_results.is_empty());

        // 验证 vec store 有记录
        let vec_results = indexer
            .vec_store
            .knn(&[0.1, 0.2, 0.3], 10)
            .unwrap();
        assert!(!vec_results.is_empty());
    }

    #[tokio::test]
    async fn test_incremental_index_modified_file() {
        let tmp = TempDir::new().unwrap();
        let file_path = tmp.path().join("a.rs");
        let path_str = file_path.to_str().unwrap().to_string();

        // 初始内容
        std::fs::write(&file_path, "fn old_function() {}").unwrap();
        let indexer = make_indexer(&tmp);
        indexer
            .on_file_change(&FileChange {
                kind: FileChangeKind::Created,
                path: path_str.clone(),
            })
            .await
            .unwrap();

        // 修改内容
        std::fs::write(&file_path, "fn new_function() {}").unwrap();
        indexer
            .on_file_change(&FileChange {
                kind: FileChangeKind::Modified,
                path: path_str,
            })
            .await
            .unwrap();

        // FTS 应该找到新内容
        let results = indexer.fts_store.search("new_function", 10).unwrap();
        assert!(!results.is_empty());

        // 旧内容不应该找到
        let old_results = indexer.fts_store.search("old_function", 10).unwrap();
        assert!(old_results.is_empty());
    }

    #[tokio::test]
    async fn test_incremental_index_deleted_file() {
        let tmp = TempDir::new().unwrap();
        let file_path = tmp.path().join("a.rs");
        let path_str = file_path.to_str().unwrap().to_string();

        std::fs::write(&file_path, "fn doomed() {}").unwrap();
        let indexer = make_indexer(&tmp);
        indexer
            .on_file_change(&FileChange {
                kind: FileChangeKind::Created,
                path: path_str.clone(),
            })
            .await
            .unwrap();

        // 删除
        indexer
            .on_file_change(&FileChange {
                kind: FileChangeKind::Deleted,
                path: path_str,
            })
            .await
            .unwrap();

        let results = indexer.fts_store.search("doomed", 10).unwrap();
        assert!(results.is_empty());
    }
}
