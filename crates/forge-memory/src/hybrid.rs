//! HybridRetriever: 向量相似度 + FTS5 全文检索混合排序。
//!
//! 使用 RRF (Reciprocal Rank Fusion) 合并两个排序结果。

use std::collections::HashMap;
use std::path::PathBuf;

use async_trait::async_trait;
use forge_core::{EmbeddingProvider, MemoryChunk, MemoryRetriever, RetrieveOptions};

use crate::fts_store::Fts5Store;
use crate::vec_store::SqliteVecStore;

/// 混合检索器：向量 + FTS5，使用 RRF 合并排序。
pub struct HybridRetriever {
    vec_store: SqliteVecStore,
    fts_store: Fts5Store,
    embedding: Box<dyn EmbeddingProvider>,
    vec_weight: f32,
}

impl HybridRetriever {
    pub fn new(
        vec_store: SqliteVecStore,
        fts_store: Fts5Store,
        embedding: Box<dyn EmbeddingProvider>,
        vec_weight: f32,
    ) -> Self {
        Self {
            vec_store,
            fts_store,
            embedding,
            vec_weight,
        }
    }
}

#[async_trait]
impl MemoryRetriever for HybridRetriever {
    async fn retrieve(&self, query: &str, opts: RetrieveOptions) -> Vec<MemoryChunk> {
        if query.is_empty() {
            return vec![];
        }

        let top_k = opts.top_k.unwrap_or(10);
        let fetch_k = top_k * 3; // 多取一些用于合并

        // 向量检索
        let vec_results = if let Ok(embeddings) = self.embedding.embed(&[query.to_string()]).await {
            if let Some(query_vec) = embeddings.first() {
                self.vec_store.knn(query_vec, fetch_k).unwrap_or_default()
            } else {
                vec![]
            }
        } else {
            vec![]
        };

        // FTS5 检索
        let fts_results = self.fts_store.search(query, fetch_k).unwrap_or_default();

        // Scope 过滤
        let scope_filter = opts.scope.as_deref();

        // RRF 合并
        let mut scores: HashMap<String, (f32, String, Option<String>)> = HashMap::new();
        let rrf_k = 60.0_f32; // RRF 常数

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

        // 按 RRF score 降序排列
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
            let content = tokio::fs::read_to_string(path).await?;
            let id = path.to_string_lossy().to_string();

            // FTS 索引
            self.fts_store.index(&id, &content)?;

            // 向量索引
            let embeddings = self.embedding.embed(&[content.clone()]).await?;
            if let Some(vec) = embeddings.first() {
                self.vec_store
                    .insert(&id, &content, vec, Some(&id))?;
            }
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

        // A 和 B 都出现在两端，分数应该最高
        let top_two: Vec<&str> = result.iter().take(2).map(|(id, _)| id.as_str()).collect();
        assert!(top_two.contains(&"A"));
        assert!(top_two.contains(&"B"));
        // C 和 D 各只出现一端
        assert_eq!(result.len(), 4);
    }

    #[test]
    fn test_hybrid_vec_weight() {
        let vec_rank = vec!["A", "B"];
        let fts_rank = vec!["B", "A"];

        // vec_weight=1.0 → 纯向量排序
        let result_vec = rrf_merge(&vec_rank, &fts_rank, 1.0);
        assert_eq!(result_vec[0].0, "A"); // A 在 vec 排第一

        // vec_weight=0.0 → 纯 FTS 排序
        let result_fts = rrf_merge(&vec_rank, &fts_rank, 0.0);
        assert_eq!(result_fts[0].0, "B"); // B 在 fts 排第一
    }

    #[test]
    fn test_hybrid_top_k() {
        // top_k 在 retrieve 中测试，这里验证 RRF 返回所有唯一结果
        let vec_rank: Vec<&str> = (0..10).map(|i| match i {
            0 => "a", 1 => "b", 2 => "c", 3 => "d", 4 => "e",
            5 => "f", 6 => "g", 7 => "h", 8 => "i", _ => "j",
        }).collect();
        let fts_rank = vec!["b", "d", "f"];

        let result = rrf_merge(&vec_rank, &fts_rank, 0.5);
        assert_eq!(result.len(), 10); // 10 unique IDs
    }
}
