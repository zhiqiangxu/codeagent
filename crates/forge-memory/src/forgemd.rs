use std::path::PathBuf;

use async_trait::async_trait;
use forge_core::{MemoryChunk, MemoryRetriever, RetrieveOptions};

/// Phase 1 MemoryRetriever：全量加载 FORGE.md 文件（不做语义检索）。
///
/// 加载两个位置的 FORGE.md：
/// - `global_dir`/.codeforge/FORGE.md — 用户全局规则
/// - `project_dir`/.codeforge/FORGE.md — 项目级规则
///
/// 两者合并返回，project 追加在 global 之后（project 规则可覆盖 global 规则）。
pub struct ForgemdRetriever {
    global_dir: PathBuf,
    project_dir: PathBuf,
}

impl ForgemdRetriever {
    pub fn new(global_dir: impl Into<PathBuf>, project_dir: impl Into<PathBuf>) -> Self {
        Self {
            global_dir: global_dir.into(),
            project_dir: project_dir.into(),
        }
    }

    fn forge_md_path(base: &PathBuf) -> PathBuf {
        base.join(".codeforge").join("FORGE.md")
    }

    async fn load_file(path: &PathBuf, source_label: &str) -> Option<MemoryChunk> {
        match tokio::fs::read_to_string(path).await {
            Ok(content) if !content.is_empty() => Some(MemoryChunk {
                content,
                source: Some(source_label.to_string()),
                score: 1.0,
            }),
            _ => None,
        }
    }
}

#[async_trait]
impl MemoryRetriever for ForgemdRetriever {
    async fn retrieve(&self, _query: &str, _opts: RetrieveOptions) -> Vec<MemoryChunk> {
        let mut chunks = Vec::new();

        let global_path = Self::forge_md_path(&self.global_dir);
        if let Some(chunk) = Self::load_file(&global_path, "global/FORGE.md").await {
            chunks.push(chunk);
        }

        let project_path = Self::forge_md_path(&self.project_dir);
        if let Some(chunk) = Self::load_file(&project_path, "project/FORGE.md").await {
            chunks.push(chunk);
        }

        chunks
    }

    async fn index(&self, _files: &[PathBuf]) -> anyhow::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn write_forge_md(base: &std::path::Path, content: &str) {
        let dir = base.join(".codeforge");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(dir.join("FORGE.md"), content).await.unwrap();
    }

    #[tokio::test]
    async fn test_forgemd_load_global() {
        let global = TempDir::new().unwrap();
        let project = TempDir::new().unwrap();
        write_forge_md(global.path(), "global rules").await;

        let retriever = ForgemdRetriever::new(global.path(), project.path());
        let chunks = retriever.retrieve("", RetrieveOptions::default()).await;

        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].content.contains("global rules"));
        assert_eq!(chunks[0].source.as_deref(), Some("global/FORGE.md"));
    }

    #[tokio::test]
    async fn test_forgemd_load_project() {
        let global = TempDir::new().unwrap();
        let project = TempDir::new().unwrap();
        write_forge_md(project.path(), "project rules").await;

        let retriever = ForgemdRetriever::new(global.path(), project.path());
        let chunks = retriever.retrieve("", RetrieveOptions::default()).await;

        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].content.contains("project rules"));
        assert_eq!(chunks[0].source.as_deref(), Some("project/FORGE.md"));
    }

    #[tokio::test]
    async fn test_forgemd_merge_global_and_project() {
        let global = TempDir::new().unwrap();
        let project = TempDir::new().unwrap();
        write_forge_md(global.path(), "global rules").await;
        write_forge_md(project.path(), "project rules").await;

        let retriever = ForgemdRetriever::new(global.path(), project.path());
        let chunks = retriever.retrieve("", RetrieveOptions::default()).await;

        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].content.contains("global rules"));
        assert!(chunks[1].content.contains("project rules"));
    }

    #[tokio::test]
    async fn test_forgemd_not_found_ok() {
        let global = TempDir::new().unwrap();
        let project = TempDir::new().unwrap();
        // No FORGE.md files created

        let retriever = ForgemdRetriever::new(global.path(), project.path());
        let chunks = retriever.retrieve("", RetrieveOptions::default()).await;

        assert!(chunks.is_empty());
    }

    #[tokio::test]
    async fn test_forgemd_retrieve_ignores_query() {
        let global = TempDir::new().unwrap();
        let project = TempDir::new().unwrap();
        write_forge_md(global.path(), "some content").await;

        let retriever = ForgemdRetriever::new(global.path(), project.path());
        let result_empty = retriever.retrieve("", RetrieveOptions::default()).await;
        let result_query = retriever.retrieve("any query", RetrieveOptions::default()).await;

        assert_eq!(result_empty.len(), result_query.len());
        assert_eq!(result_empty[0].content, result_query[0].content);
    }

    #[tokio::test]
    async fn test_forgemd_index_is_noop() {
        let global = TempDir::new().unwrap();
        let project = TempDir::new().unwrap();

        let retriever = ForgemdRetriever::new(global.path(), project.path());
        let result = retriever.index(&[PathBuf::from("/any/path")]).await;

        assert!(result.is_ok());
    }
}
