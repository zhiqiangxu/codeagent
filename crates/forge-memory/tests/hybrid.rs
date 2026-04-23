use std::sync::Arc;

use async_trait::async_trait;
use forge_core::{EmbeddingProvider, MemoryRetriever, RetrieveOptions};
use forge_memory::{HybridRetriever, MemoryDb};
use tempfile::TempDir;

struct FixedEmbedding {
    dim: usize,
}

#[async_trait]
impl EmbeddingProvider for FixedEmbedding {
    async fn embed(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        Ok(texts
            .iter()
            .map(|t| {
                let seed = t.bytes().map(|b| b as f32).sum::<f32>();
                (0..self.dim)
                    .map(|i| ((seed + i as f32) * 0.01).sin())
                    .collect()
            })
            .collect())
    }

    fn dimension(&self) -> usize {
        self.dim
    }
}

fn create_hybrid(tmp: &TempDir) -> HybridRetriever {
    let db = Arc::new(
        MemoryDb::open(tmp.path().join("memory.db").to_str().unwrap()).unwrap(),
    );
    let embedding = Box::new(FixedEmbedding { dim: 8 });
    HybridRetriever::new(db, embedding, 0.5)
}

#[tokio::test]
async fn test_hybrid_index_and_retrieve() {
    let tmp = TempDir::new().unwrap();

    std::fs::write(tmp.path().join("a.rs"), "fn hello() { println!(\"hello rust\"); }").unwrap();
    std::fs::write(tmp.path().join("b.rs"), "fn goodbye() { println!(\"goodbye python\"); }")
        .unwrap();
    std::fs::write(tmp.path().join("c.rs"), "fn rust_rocks() { /* rust is great */ }").unwrap();

    let retriever = create_hybrid(&tmp);
    retriever
        .index(&[
            tmp.path().join("a.rs"),
            tmp.path().join("b.rs"),
            tmp.path().join("c.rs"),
        ])
        .await
        .unwrap();

    let results = retriever
        .retrieve(
            "rust",
            RetrieveOptions {
                top_k: Some(3),
                ..Default::default()
            },
        )
        .await;

    assert!(!results.is_empty(), "should find results for 'rust'");
    assert!(results.iter().any(|r| r.content.contains("rust")));
}

#[tokio::test]
async fn test_hybrid_empty_query() {
    let tmp = TempDir::new().unwrap();
    let retriever = create_hybrid(&tmp);

    let results = retriever.retrieve("", RetrieveOptions::default()).await;
    assert!(results.is_empty());
}

#[tokio::test]
async fn test_hybrid_filters_by_scope() {
    let tmp = TempDir::new().unwrap();

    let global_dir = tmp.path().join("global");
    let project_dir = tmp.path().join("project");
    std::fs::create_dir_all(&global_dir).unwrap();
    std::fs::create_dir_all(&project_dir).unwrap();
    std::fs::write(global_dir.join("rules.md"), "global coding rules").unwrap();
    std::fs::write(project_dir.join("config.md"), "project specific config").unwrap();

    let retriever = create_hybrid(&tmp);
    retriever
        .index(&[
            global_dir.join("rules.md"),
            project_dir.join("config.md"),
        ])
        .await
        .unwrap();

    let results = retriever
        .retrieve(
            "config",
            RetrieveOptions {
                top_k: Some(10),
                scope: Some("project".into()),
                ..Default::default()
            },
        )
        .await;

    for r in &results {
        if let Some(ref source) = r.source {
            assert!(
                source.contains("project"),
                "should only return project scope, got: {}",
                source
            );
        }
    }
}
