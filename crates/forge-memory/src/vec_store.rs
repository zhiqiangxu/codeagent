//! SQLite 向量存储：使用 JSON 编码的向量 + 余弦相似度实现 KNN 检索。
//!
//! 注：真实生产环境可替换为 sqlite-vec 扩展，此处用纯 SQL 实现 Phase 2 MVP。

use std::sync::Mutex;

use rusqlite::{params, Connection};

/// SQLite 向量存储。
pub struct SqliteVecStore {
    conn: Mutex<Connection>,
    dimension: usize,
}

impl SqliteVecStore {
    /// 创建新的向量存储（在给定路径或 :memory:）。
    pub fn open(path: &str, dimension: usize) -> anyhow::Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS vec_store (
                id TEXT PRIMARY KEY,
                content TEXT NOT NULL,
                embedding TEXT NOT NULL,
                source TEXT
            )",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
            dimension,
        })
    }

    /// 插入一条向量记录。
    pub fn insert(
        &self,
        id: &str,
        content: &str,
        embedding: &[f32],
        source: Option<&str>,
    ) -> anyhow::Result<()> {
        let embedding_json = serde_json::to_string(embedding)?;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO vec_store (id, content, embedding, source) VALUES (?1, ?2, ?3, ?4)",
            params![id, content, embedding_json, source],
        )?;
        Ok(())
    }

    /// 删除一条记录。
    pub fn delete(&self, id: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM vec_store WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// KNN 查询：返回与 query_vec 最相似的 k 条记录，按余弦相似度降序。
    pub fn knn(
        &self,
        query_vec: &[f32],
        k: usize,
    ) -> anyhow::Result<Vec<VecSearchResult>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT id, content, embedding, source FROM vec_store")?;

        let rows = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let content: String = row.get(1)?;
            let embedding_json: String = row.get(2)?;
            let source: Option<String> = row.get(3)?;
            Ok((id, content, embedding_json, source))
        })?;

        let mut results: Vec<VecSearchResult> = Vec::new();
        for row in rows {
            let (id, content, embedding_json, source) = row?;
            let embedding: Vec<f32> = serde_json::from_str(&embedding_json)?;
            let score = cosine_similarity(query_vec, &embedding);
            results.push(VecSearchResult {
                id,
                content,
                source,
                score,
            });
        }

        // 按相似度降序排列
        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(k);
        Ok(results)
    }
}

/// 向量搜索结果。
#[derive(Debug, Clone)]
pub struct VecSearchResult {
    pub id: String,
    pub content: String,
    pub source: Option<String>,
    pub score: f32,
}

/// 余弦相似度计算。
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_store(dim: usize) -> SqliteVecStore {
        SqliteVecStore::open(":memory:", dim).unwrap()
    }

    #[test]
    fn test_sqlite_vec_insert() {
        let store = memory_store(3);
        store
            .insert("doc1", "hello world", &[0.1, 0.2, 0.3], None)
            .unwrap();

        let results = store.knn(&[0.1, 0.2, 0.3], 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "doc1");
        assert_eq!(results[0].content, "hello world");
    }

    #[test]
    fn test_sqlite_vec_knn_query() {
        let store = memory_store(3);
        store.insert("a", "alpha", &[1.0, 0.0, 0.0], None).unwrap();
        store.insert("b", "beta", &[0.9, 0.1, 0.0], None).unwrap();
        store
            .insert("c", "gamma", &[0.0, 0.0, 1.0], None)
            .unwrap();

        let results = store.knn(&[1.0, 0.0, 0.0], 2).unwrap();
        assert_eq!(results.len(), 2);
        // a 最近，b 次近
        assert_eq!(results[0].id, "a");
        assert_eq!(results[1].id, "b");
        // 相似度降序
        assert!(results[0].score >= results[1].score);
    }

    #[test]
    fn test_sqlite_vec_empty_table() {
        let store = memory_store(3);
        let results = store.knn(&[1.0, 0.0, 0.0], 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_sqlite_vec_delete() {
        let store = memory_store(3);
        store
            .insert("doc1", "content", &[0.1, 0.2, 0.3], None)
            .unwrap();
        store.delete("doc1").unwrap();

        let results = store.knn(&[0.1, 0.2, 0.3], 10).unwrap();
        assert!(results.is_empty());
    }
}
