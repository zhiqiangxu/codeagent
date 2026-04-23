//! CombinedRetriever: merges results from ForgemdRetriever + HybridRetriever.
//!
//! FORGE.md rules always loaded. HybridRetriever adds RAG results for project code.

use std::path::PathBuf;

use async_trait::async_trait;
use forge_core::{MemoryChunk, MemoryRetriever, RetrieveOptions};

/// Combines multiple retrievers, merging results in order.
pub struct CombinedRetriever {
    retrievers: Vec<Box<dyn MemoryRetriever>>,
}

impl CombinedRetriever {
    pub fn new(retrievers: Vec<Box<dyn MemoryRetriever>>) -> Self {
        Self { retrievers }
    }
}

#[async_trait]
impl MemoryRetriever for CombinedRetriever {
    async fn retrieve(&self, query: &str, opts: RetrieveOptions) -> Vec<MemoryChunk> {
        let mut all = Vec::new();
        for r in &self.retrievers {
            let chunks = r.retrieve(query, opts.clone()).await;
            all.extend(chunks);
        }
        // Deduplicate by content (keep first occurrence)
        let mut seen = std::collections::HashSet::new();
        all.retain(|chunk| seen.insert(chunk.content.clone()));
        all
    }

    async fn index(&self, files: &[PathBuf]) -> anyhow::Result<()> {
        for r in &self.retrievers {
            r.index(files).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forgemd::ForgemdRetriever;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_combined_merges_results() {
        let tmp1 = TempDir::new().unwrap();
        let tmp2 = TempDir::new().unwrap();

        // Write FORGE.md for first retriever
        let forge_dir = tmp1.path().join(".codeforge");
        std::fs::create_dir_all(&forge_dir).unwrap();
        std::fs::write(forge_dir.join("FORGE.md"), "global rules").unwrap();

        let forge_dir2 = tmp2.path().join(".codeforge");
        std::fs::create_dir_all(&forge_dir2).unwrap();
        std::fs::write(forge_dir2.join("FORGE.md"), "project rules").unwrap();

        let r1 = ForgemdRetriever::new(tmp1.path(), tmp1.path());
        let r2 = ForgemdRetriever::new(tmp2.path(), tmp2.path());

        let combined = CombinedRetriever::new(vec![Box::new(r1), Box::new(r2)]);
        let results = combined.retrieve("", RetrieveOptions::default()).await;

        assert!(results.len() >= 2);
        assert!(results.iter().any(|c| c.content.contains("global")));
        assert!(results.iter().any(|c| c.content.contains("project")));
    }

    #[tokio::test]
    async fn test_combined_deduplicates() {
        let tmp = TempDir::new().unwrap();
        let forge_dir = tmp.path().join(".codeforge");
        std::fs::create_dir_all(&forge_dir).unwrap();
        std::fs::write(forge_dir.join("FORGE.md"), "same content").unwrap();

        // Two retrievers pointing at same FORGE.md
        let r1 = ForgemdRetriever::new(tmp.path(), tmp.path());
        let r2 = ForgemdRetriever::new(tmp.path(), tmp.path());

        let combined = CombinedRetriever::new(vec![Box::new(r1), Box::new(r2)]);
        let results = combined.retrieve("", RetrieveOptions::default()).await;

        // Should deduplicate identical content
        let count = results
            .iter()
            .filter(|c| c.content == "same content")
            .count();
        assert_eq!(count, 1);
    }
}
