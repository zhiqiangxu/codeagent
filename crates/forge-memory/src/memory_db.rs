//! 统一内存数据库：vec_store + fts_docs + index_meta 共享一个 SQLite 连接。

use std::sync::Mutex;

use rusqlite::{params, Connection};

/// 向量搜索结果。
#[derive(Debug, Clone)]
pub struct VecSearchResult {
    pub id: String,
    pub content: String,
    pub source: Option<String>,
    pub score: f32,
}

/// FTS 搜索结果。
#[derive(Debug, Clone)]
pub struct FtsSearchResult {
    pub id: String,
    pub content: String,
    pub rank: f64,
}

/// 统一内存数据库。
pub struct MemoryDb {
    conn: Mutex<Connection>,
}

impl MemoryDb {
    /// 打开（或创建）数据库，初始化三张表。
    pub fn open(path: &str) -> anyhow::Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS vec_store (
                id TEXT PRIMARY KEY,
                content TEXT NOT NULL,
                embedding TEXT NOT NULL,
                source TEXT
            );

            CREATE TABLE IF NOT EXISTS index_meta (
                path TEXT PRIMARY KEY,
                content_hash TEXT NOT NULL,
                size INTEGER NOT NULL DEFAULT 0,
                indexed_at INTEGER NOT NULL DEFAULT 0
            );

            CREATE VIRTUAL TABLE IF NOT EXISTS fts_docs USING fts5(
                id UNINDEXED,
                content,
                tokenize = 'unicode61 remove_diacritics 2'
            );",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    // ─── index_meta ────────────────────────────────────

    /// 查询文件的 hash，None 表示未索引。
    pub fn get_hash(&self, path: &str) -> Option<String> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT content_hash FROM index_meta WHERE path = ?1",
            params![path],
            |row| row.get(0),
        )
        .ok()
    }

    /// 更新或插入文件的索引元数据。
    pub fn upsert_meta(&self, path: &str, hash: &str, size: u64) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        conn.execute(
            "INSERT OR REPLACE INTO index_meta (path, content_hash, size, indexed_at) VALUES (?1, ?2, ?3, ?4)",
            params![path, hash, size as i64, now],
        )?;
        Ok(())
    }

    /// 删除文件的索引元数据。
    pub fn delete_meta(&self, path: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM index_meta WHERE path = ?1", params![path])?;
        Ok(())
    }

    // ─── vec_store ─────────────────────────────────────

    pub fn vec_insert(
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

    pub fn vec_delete(&self, id: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM vec_store WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn vec_knn(&self, query_vec: &[f32], k: usize) -> anyhow::Result<Vec<VecSearchResult>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT id, content, embedding, source FROM vec_store")?;

        let rows = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let content: String = row.get(1)?;
            let embedding_json: String = row.get(2)?;
            let source: Option<String> = row.get(3)?;
            Ok((id, content, embedding_json, source))
        })?;

        let mut results = Vec::new();
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

        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(k);
        Ok(results)
    }

    // ─── fts_docs ──────────────────────────────────────

    pub fn fts_index(&self, id: &str, content: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM fts_docs WHERE id = ?1", params![id])?;
        conn.execute(
            "INSERT INTO fts_docs (id, content) VALUES (?1, ?2)",
            params![id, content],
        )?;
        Ok(())
    }

    pub fn fts_delete(&self, id: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM fts_docs WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn fts_search(&self, query: &str, limit: usize) -> anyhow::Result<Vec<FtsSearchResult>> {
        if query.is_empty() {
            return Ok(vec![]);
        }
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, content, rank FROM fts_docs WHERE fts_docs MATCH ?1 ORDER BY rank LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![query, limit as i64], |row| {
            Ok(FtsSearchResult {
                id: row.get(0)?,
                content: row.get(1)?,
                rank: row.get(2)?,
            })
        })?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    // ─── atomic operations ─────────────────────────────

    /// 原子性地删除一个文件的全部索引（meta + vec + fts）。
    pub fn delete_all(&self, path: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM index_meta WHERE path = ?1", params![path])?;
        conn.execute("DELETE FROM vec_store WHERE id = ?1", params![path])?;
        conn.execute("DELETE FROM fts_docs WHERE id = ?1", params![path])?;
        Ok(())
    }
}

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

/// 计算内容的 SHA256 hash。
pub fn content_hash(content: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem_db() -> MemoryDb {
        MemoryDb::open(":memory:").unwrap()
    }

    #[test]
    fn test_index_meta_roundtrip() {
        let db = mem_db();
        assert!(db.get_hash("/a.rs").is_none());

        db.upsert_meta("/a.rs", "abc123", 100).unwrap();
        assert_eq!(db.get_hash("/a.rs"), Some("abc123".into()));

        db.upsert_meta("/a.rs", "def456", 200).unwrap();
        assert_eq!(db.get_hash("/a.rs"), Some("def456".into()));

        db.delete_meta("/a.rs").unwrap();
        assert!(db.get_hash("/a.rs").is_none());
    }

    #[test]
    fn test_hash_dedup() {
        let h1 = content_hash("hello world");
        let h2 = content_hash("hello world");
        let h3 = content_hash("hello world!");
        assert_eq!(h1, h2);
        assert_ne!(h1, h3);
    }

    #[test]
    fn test_vec_and_fts_in_same_db() {
        let db = mem_db();
        db.vec_insert("doc1", "hello", &[0.1, 0.2], None).unwrap();
        db.fts_index("doc1", "hello world").unwrap();

        let vec_results = db.vec_knn(&[0.1, 0.2], 10).unwrap();
        assert_eq!(vec_results.len(), 1);

        let fts_results = db.fts_search("hello", 10).unwrap();
        assert_eq!(fts_results.len(), 1);
    }

    #[test]
    fn test_delete_all_atomic() {
        let db = mem_db();
        db.upsert_meta("/a.rs", "hash1", 50).unwrap();
        db.vec_insert("/a.rs", "content", &[0.1], None).unwrap();
        db.fts_index("/a.rs", "content").unwrap();

        db.delete_all("/a.rs").unwrap();

        assert!(db.get_hash("/a.rs").is_none());
        assert!(db.vec_knn(&[0.1], 10).unwrap().is_empty());
        assert!(db.fts_search("content", 10).unwrap().is_empty());
    }
}
