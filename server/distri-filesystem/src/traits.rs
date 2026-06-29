use crate::SearchResult;
use anyhow::Result;
use async_trait::async_trait;

/// Search operations that work on top of FileSystemOps
#[async_trait]
pub trait GrepSearcher: Send + Sync + std::fmt::Debug {
    /// Search files and content using grep
    async fn search(
        &self,
        path: &str,
        content_pattern: Option<&str>,
        file_pattern: Option<&str>,
    ) -> Result<SearchResult>;
}
