//! 增量索引器：响应文件变更和对话回合，更新索引。
//! 基于 MemoryDb 统一存储，hash 去重。

use std::sync::Arc;

use forge_core::{Content, EmbeddingProvider, Message};

use crate::memory_db::{content_hash, MemoryDb};

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
    db: Arc<MemoryDb>,
    embedding: Arc<dyn EmbeddingProvider>,
}

impl IncrementalIndexer {
    pub fn new(db: Arc<MemoryDb>, embedding: Arc<dyn EmbeddingProvider>) -> Self {
        Self { db, embedding }
    }

    /// 处理单个文件变更（hash 去重）。
    pub async fn on_file_change(&self, change: &FileChange) -> anyhow::Result<()> {
        match change.kind {
            FileChangeKind::Created | FileChangeKind::Modified => {
                let content = tokio::fs::read_to_string(&change.path).await?;
                let hash = content_hash(&content);

                // Hash 去重
                if let Some(existing) = self.db.get_hash(&change.path) {
                    if existing == hash {
                        return Ok(()); // 内容没变，跳过
                    }
                    self.db.delete_all(&change.path)?;
                }

                // FTS 索引
                self.db.fts_index(&change.path, &content)?;

                // 向量索引
                let embeddings = self.embedding.embed(&[content.clone()]).await?;
                if let Some(vec) = embeddings.first() {
                    self.db
                        .vec_insert(&change.path, &content, vec, Some(&change.path))?;
                }

                // 记录 meta
                self.db
                    .upsert_meta(&change.path, &hash, content.len() as u64)?;
            }
            FileChangeKind::Deleted => {
                self.db.delete_all(&change.path)?;
            }
        }
        Ok(())
    }

    /// 后台批量处理文件变更。
    pub fn process_batch(
        self: Arc<Self>,
        changes: Vec<FileChange>,
    ) -> tokio::task::JoinHandle<anyhow::Result<()>> {
        tokio::spawn(async move {
            for change in &changes {
                self.on_file_change(change).await?;
            }
            Ok(())
        })
    }

    /// 对话回合结束后索引消息内容。
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
            self.db.fts_index(&id, &content)?;

            let embeddings = self.embedding.embed(&[content.clone()]).await?;
            if let Some(vec) = embeddings.first() {
                self.db
                    .vec_insert(&id, &content, vec, Some("conversation"))?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
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
        let db = Arc::new(
            MemoryDb::open(tmp.path().join("memory.db").to_str().unwrap()).unwrap(),
        );
        IncrementalIndexer::new(db, Arc::new(FakeEmbedding))
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

        let fts_results = indexer.db.fts_search("hello", 10).unwrap();
        assert!(!fts_results.is_empty());

        let vec_results = indexer.db.vec_knn(&[0.1, 0.2, 0.3], 10).unwrap();
        assert!(!vec_results.is_empty());
    }

    #[tokio::test]
    async fn test_incremental_index_modified_file() {
        let tmp = TempDir::new().unwrap();
        let file_path = tmp.path().join("a.rs");
        let path_str = file_path.to_str().unwrap().to_string();

        std::fs::write(&file_path, "fn old_function() {}").unwrap();
        let indexer = make_indexer(&tmp);
        indexer
            .on_file_change(&FileChange {
                kind: FileChangeKind::Created,
                path: path_str.clone(),
            })
            .await
            .unwrap();

        std::fs::write(&file_path, "fn new_function() {}").unwrap();
        indexer
            .on_file_change(&FileChange {
                kind: FileChangeKind::Modified,
                path: path_str,
            })
            .await
            .unwrap();

        let results = indexer.db.fts_search("new_function", 10).unwrap();
        assert!(!results.is_empty());

        let old_results = indexer.db.fts_search("old_function", 10).unwrap();
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

        indexer
            .on_file_change(&FileChange {
                kind: FileChangeKind::Deleted,
                path: path_str,
            })
            .await
            .unwrap();

        let results = indexer.db.fts_search("doomed", 10).unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_incremental_index_hash_dedup() {
        let tmp = TempDir::new().unwrap();
        let file_path = tmp.path().join("same.rs");
        let path_str = file_path.to_str().unwrap().to_string();

        std::fs::write(&file_path, "fn same() {}").unwrap();
        let indexer = make_indexer(&tmp);

        // Index twice with same content — should not duplicate
        indexer
            .on_file_change(&FileChange {
                kind: FileChangeKind::Created,
                path: path_str.clone(),
            })
            .await
            .unwrap();
        indexer
            .on_file_change(&FileChange {
                kind: FileChangeKind::Modified,
                path: path_str,
            })
            .await
            .unwrap();

        let results = indexer.db.vec_knn(&[0.1, 0.2, 0.3], 10).unwrap();
        assert_eq!(results.len(), 1);
    }
}
