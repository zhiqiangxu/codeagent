//! HybridRetriever: 向量相似度 + FTS5 全文检索混合排序。
//!
//! 使用 RRF (Reciprocal Rank Fusion) 合并两个排序结果。
//! 基于 MemoryDb 统一存储，支持 hash 去重避免重复索引。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use forge_core::{EmbeddingProvider, MemoryChunk, MemoryRetriever, RetrieveOptions};

use crate::memory_db::{content_hash, MemoryDb};

/// 混合检索器：向量 + FTS5，使用 RRF 合并排序。
pub struct HybridRetriever {
    db: Arc<MemoryDb>,
    embedding: Box<dyn EmbeddingProvider>,
    vec_weight: f32,
}

impl HybridRetriever {
    pub fn new(
        db: Arc<MemoryDb>,
        embedding: Box<dyn EmbeddingProvider>,
        vec_weight: f32,
    ) -> Self {
        Self {
            db,
            embedding,
            vec_weight,
        }
    }

    /// 获取底层 MemoryDb（用于外部启动索引等）。
    pub fn db(&self) -> &Arc<MemoryDb> {
        &self.db
    }
}

#[async_trait]
impl MemoryRetriever for HybridRetriever {
    async fn retrieve(&self, query: &str, opts: RetrieveOptions) -> Vec<MemoryChunk> {
        if query.is_empty() {
            return vec![];
        }

        let top_k = opts.top_k.unwrap_or(10);
        let fetch_k = top_k * 3;

        // 向量检索
        let vec_results =
            if let Ok(embeddings) = self.embedding.embed(&[query.to_string()]).await {
                if let Some(query_vec) = embeddings.first() {
                    self.db.vec_knn(query_vec, fetch_k).unwrap_or_default()
                } else {
                    vec![]
                }
            } else {
                vec![]
            };

        // FTS5 检索
        let fts_results = self.db.fts_search(query, fetch_k).unwrap_or_default();

        // Scope 过滤 + RRF 合并
        let scope_filter = opts.scope.as_deref();
        let mut scores: HashMap<String, (f32, String, Option<String>)> = HashMap::new();
        let rrf_k = 60.0_f32;

        for (rank, result) in vec_results.iter().enumerate() {
            if let Some(scope) = scope_filter {
                if let Some(ref source) = result.source {
                    if !source.contains(scope) {
                        continue;
                    }
                }
            }
            let rrf_score = self.vec_weight / (rrf_k + rank as f32 + 1.0);
            let entry = scores
                .entry(result.id.clone())
                .or_insert((0.0, result.content.clone(), result.source.clone()));
            entry.0 += rrf_score;
        }

        let fts_weight = 1.0 - self.vec_weight;
        for (rank, result) in fts_results.iter().enumerate() {
            if let Some(scope) = scope_filter {
                if !result.id.contains(scope) {
                    continue;
                }
            }
            let rrf_score = fts_weight / (rrf_k + rank as f32 + 1.0);
            let entry = scores
                .entry(result.id.clone())
                .or_insert((0.0, result.content.clone(), None));
            entry.0 += rrf_score;
        }

        let mut merged: Vec<(String, f32, String, Option<String>)> = scores
            .into_iter()
            .map(|(id, (score, content, source))| (id, score, content, source))
            .collect();
        merged.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        merged.truncate(top_k);

        merged
            .into_iter()
            .map(|(_, score, content, source)| MemoryChunk {
                content,
                source,
                score,
            })
            .collect()
    }

    async fn index(&self, files: &[PathBuf]) -> anyhow::Result<()> {
        for path in files {
            let id = path.to_string_lossy().to_string();
            let content = tokio::fs::read_to_string(path).await?;
            let hash = content_hash(&content);

            // Hash 去重：相同则跳过
            if let Some(existing_hash) = self.db.get_hash(&id) {
                if existing_hash == hash {
                    continue;
                }
                // Hash 不同，删旧索引
                self.db.delete_all(&id)?;
            }

            // FTS 索引
            self.db.fts_index(&id, &content)?;

            // 向量索引
            let embeddings = self.embedding.embed(&[content.clone()]).await?;
            if let Some(vec) = embeddings.first() {
                self.db.vec_insert(&id, &content, vec, Some(&id))?;
            }

            // 记录 meta
            self.db
                .upsert_meta(&id, &hash, content.len() as u64)?;
        }
        Ok(())
    }
}

/// RRF 合并函数（独立出来方便单元测试）。
pub fn rrf_merge(
    vec_ranking: &[&str],
    fts_ranking: &[&str],
    vec_weight: f32,
) -> Vec<(String, f32)> {
    let rrf_k = 60.0_f32;
    let fts_weight = 1.0 - vec_weight;
    let mut scores: HashMap<String, f32> = HashMap::new();

    for (rank, id) in vec_ranking.iter().enumerate() {
        *scores.entry(id.to_string()).or_default() +=
            vec_weight / (rrf_k + rank as f32 + 1.0);
    }
    for (rank, id) in fts_ranking.iter().enumerate() {
        *scores.entry(id.to_string()).or_default() +=
            fts_weight / (rrf_k + rank as f32 + 1.0);
    }

    let mut result: Vec<(String, f32)> = scores.into_iter().collect();
    result.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hybrid_rrf_ranking() {
        let vec_rank = vec!["A", "B", "C"];
        let fts_rank = vec!["B", "A", "D"];

        let result = rrf_merge(&vec_rank, &fts_rank, 0.5);

        let top_two: Vec<&str> = result.iter().take(2).map(|(id, _)| id.as_str()).collect();
        assert!(top_two.contains(&"A"));
        assert!(top_two.contains(&"B"));
        assert_eq!(result.len(), 4);
    }

    #[test]
    fn test_hybrid_vec_weight() {
        let vec_rank = vec!["A", "B"];
        let fts_rank = vec!["B", "A"];

        let result_vec = rrf_merge(&vec_rank, &fts_rank, 1.0);
        assert_eq!(result_vec[0].0, "A");

        let result_fts = rrf_merge(&vec_rank, &fts_rank, 0.0);
        assert_eq!(result_fts[0].0, "B");
    }

    #[test]
    fn test_hybrid_top_k() {
        let vec_rank: Vec<&str> = (0..10)
            .map(|i| match i {
                0 => "a", 1 => "b", 2 => "c", 3 => "d", 4 => "e",
                5 => "f", 6 => "g", 7 => "h", 8 => "i", _ => "j",
            })
            .collect();
        let fts_rank = vec!["b", "d", "f"];

        let result = rrf_merge(&vec_rank, &fts_rank, 0.5);
        assert_eq!(result.len(), 10);
    }

    #[tokio::test]
    async fn test_hybrid_index_dedup() {
        use async_trait::async_trait;

        struct FakeEmbed;
        #[async_trait]
        impl EmbeddingProvider for FakeEmbed {
            async fn embed(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
                Ok(texts.iter().map(|_| vec![0.1, 0.2]).collect())
            }
            fn dimension(&self) -> usize { 2 }
        }

        let db = Arc::new(MemoryDb::open(":memory:").unwrap());
        let retriever = HybridRetriever::new(db.clone(), Box::new(FakeEmbed), 0.5);

        let tmp = tempfile::TempDir::new().unwrap();
        let file = tmp.path().join("test.rs");
        std::fs::write(&file, "fn hello() {}").unwrap();

        // Index twice — second should be skipped (same hash)
        retriever.index(&[file.clone()]).await.unwrap();
        retriever.index(&[file.clone()]).await.unwrap();

        let results = db.vec_knn(&[0.1, 0.2], 10).unwrap();
        assert_eq!(results.len(), 1); // Not duplicated

        // Modify file — should re-index
        std::fs::write(&file, "fn changed() {}").unwrap();
        retriever.index(&[file.clone()]).await.unwrap();

        let results = db.fts_search("changed", 10).unwrap();
        assert!(!results.is_empty());
    }
}
