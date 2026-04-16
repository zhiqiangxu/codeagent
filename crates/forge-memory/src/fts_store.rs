//! FTS5 全文检索存储。

use rusqlite::{params, Connection};

/// FTS5 全文检索结果。
#[derive(Debug, Clone)]
pub struct FtsSearchResult {
    pub id: String,
    pub content: String,
    pub rank: f64,
}

/// SQLite FTS5 全文检索存储。
pub struct Fts5Store {
    conn: Connection,
}

impl Fts5Store {
    /// 创建新的 FTS5 存储。
    pub fn open(path: &str) -> anyhow::Result<Self> {
        let conn = Connection::open(path)?;

        // 创建 FTS5 虚拟表（使用 unicode61 tokenizer 支持 CJK 分词）
        conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS fts_docs USING fts5(
                id UNINDEXED,
                content,
                tokenize = 'unicode61 remove_diacritics 2'
            )",
        )?;

        Ok(Self { conn })
    }

    /// 索引一篇文档。
    pub fn index(&self, id: &str, content: &str) -> anyhow::Result<()> {
        // 先删除旧记录（如果存在），再插入新记录
        self.conn
            .execute("DELETE FROM fts_docs WHERE id = ?1", params![id])?;
        self.conn.execute(
            "INSERT INTO fts_docs (id, content) VALUES (?1, ?2)",
            params![id, content],
        )?;
        Ok(())
    }

    /// 删除一篇文档。
    pub fn delete(&self, id: &str) -> anyhow::Result<()> {
        self.conn
            .execute("DELETE FROM fts_docs WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// 全文检索，按 BM25 排序。
    pub fn search(&self, query: &str, limit: usize) -> anyhow::Result<Vec<FtsSearchResult>> {
        if query.is_empty() {
            return Ok(vec![]);
        }

        let mut stmt = self.conn.prepare(
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_fts() -> Fts5Store {
        Fts5Store::open(":memory:").unwrap()
    }

    #[test]
    fn test_fts5_index_document() {
        let store = memory_fts();
        let result = store.index("doc1", "rust programming language");
        assert!(result.is_ok());
    }

    #[test]
    fn test_fts5_search_keyword() {
        let store = memory_fts();
        store.index("doc1", "rust programming").unwrap();
        store.index("doc2", "python scripting").unwrap();
        store.index("doc3", "rust and go comparison").unwrap();

        let results = store.search("rust", 10).unwrap();
        assert!(results.len() >= 2);
        assert!(results.iter().all(|r| r.content.contains("rust")));
    }

    #[test]
    fn test_fts5_search_phrase() {
        let store = memory_fts();
        store.index("doc1", "rust programming language").unwrap();
        store.index("doc2", "rust is a systems language for programming").unwrap();

        // FTS5 短语搜索用引号
        let results = store.search("\"rust programming\"", 10).unwrap();
        assert!(!results.is_empty());
        // doc1 精确包含 "rust programming"
        assert!(results.iter().any(|r| r.id == "doc1"));
    }

    #[test]
    fn test_fts5_search_boolean() {
        let store = memory_fts();
        store
            .index("doc1", "rust systems programming")
            .unwrap();
        store
            .index("doc2", "python and rust comparison")
            .unwrap();
        store.index("doc3", "python scripting").unwrap();

        let results = store.search("rust NOT python", 10).unwrap();
        assert!(!results.is_empty());
        // doc1 含 rust 不含 python
        assert!(results.iter().any(|r| r.id == "doc1"));
        // doc2 含 python，应被排除
        assert!(!results.iter().any(|r| r.id == "doc2"));
    }

    #[test]
    fn test_fts5_rank_bm25() {
        let store = memory_fts();
        // doc1: rust 出现多次
        store
            .index("doc1", "rust rust rust rust rust programming")
            .unwrap();
        // doc2: rust 出现一次
        store
            .index("doc2", "rust is a systems language")
            .unwrap();

        let results = store.search("rust", 10).unwrap();
        assert!(results.len() >= 2);
        // BM25 rank 越小越相关（FTS5 的 rank 是负数，越小越好）
        // doc1 应该排在前面（rank 更小）
        let doc1_idx = results.iter().position(|r| r.id == "doc1").unwrap();
        let doc2_idx = results.iter().position(|r| r.id == "doc2").unwrap();
        assert!(
            doc1_idx < doc2_idx,
            "doc1 (more 'rust') should rank higher"
        );
    }

    #[test]
    fn test_fts5_cjk_tokenizer() {
        let store = memory_fts();
        store.index("doc1", "Rust 编程语言入门").unwrap();
        store.index("doc2", "编程 is fun").unwrap();

        // unicode61 将 CJK 字符序列作为单个 token，
        // 搜索完整的 CJK token 或包含该 token 的文档
        let results = store.search("编程", 10).unwrap();
        // doc2 有独立的 "编程" token
        assert!(
            !results.is_empty(),
            "CJK search should find results"
        );
        assert!(results.iter().any(|r| r.id == "doc2"));
    }
}
