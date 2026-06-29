use crate::GrepSearcher;
use anyhow::Result;
use async_trait::async_trait;
use distri_types::filesystem::{
    DirectoryListing, FileMetadata, FileReadResult, FileSystemOps, ReadParams, SearchResult,
};
use std::sync::Arc;

/// File system abstraction for compatibility  
#[derive(Debug, Clone)]
pub struct FileSystem {
    file_store: Arc<crate::FileSystemStore>,
    grep_searcher: Arc<dyn GrepSearcher>,
    root_prefix: Option<String>,
}

impl FileSystem {
    pub fn new(
        file_store: Arc<crate::FileSystemStore>,
        grep_searcher: Arc<dyn GrepSearcher>,
    ) -> Self {
        let root_prefix = file_store.root_prefix();
        Self {
            file_store,
            grep_searcher,
            root_prefix,
        }
    }

    pub fn root_prefix(&self) -> Option<String> {
        self.root_prefix.clone()
    }

    pub fn create_tools(&self) -> Vec<Arc<dyn distri_types::Tool>> {
        crate::create_filesystem_tools(Arc::new(self.clone()))
    }

    pub fn scoped(&self, prefix: Option<&str>) -> anyhow::Result<Self> {
        let store = Arc::new(self.file_store.scoped(prefix)?);
        let grep_searcher =
            Arc::new(crate::FileSystemGrepSearcher::new(store.clone())) as Arc<dyn GrepSearcher>;
        Ok(Self {
            root_prefix: store.root_prefix(),
            file_store: store,
            grep_searcher,
        })
    }

    /// Create an artifact wrapper for processing tool responses
    pub async fn create_artifact_wrapper(
        &self,
        base_path: String,
    ) -> Result<crate::ArtifactWrapper, anyhow::Error> {
        crate::ArtifactWrapper::new(Arc::new(self.clone()), base_path).await
    }

    /// Write binary data to a file (for screenshots, images, etc.)
    pub async fn write_binary(&self, path: &str, content: &[u8]) -> Result<()> {
        self.file_store.write_binary(path, content).await
    }

    /// Read binary data from a file
    pub async fn read_binary(&self, path: &str) -> Result<Vec<u8>> {
        self.file_store.read_binary(path).await
    }
}

#[async_trait]
impl FileSystemOps for FileSystem {
    async fn read(&self, path: &str, params: ReadParams) -> Result<FileReadResult> {
        let read_params = crate::ReadParams {
            start_line: params.start_line,
            end_line: params.end_line,
        };
        let result = self.file_store.read(path, read_params).await?;
        Ok(FileReadResult {
            content: result.content,
            start_line: result.start_line,
            end_line: result.end_line,
            total_lines: result.total_lines,
        })
    }

    async fn read_raw(&self, path: &str) -> Result<String> {
        self.file_store.read_raw(path).await
    }

    async fn read_with_line_numbers(
        &self,
        path: &str,
        params: ReadParams,
    ) -> Result<FileReadResult> {
        self.read(path, params).await
    }

    async fn write(&self, path: &str, content: &str) -> Result<()> {
        self.file_store.write(path, content).await
    }

    async fn list(&self, path: &str) -> Result<DirectoryListing> {
        let listing = self.file_store.list(path).await?;
        Ok(DirectoryListing {
            path: listing.path,
            entries: listing
                .entries
                .into_iter()
                .map(|e| distri_types::filesystem::DirectoryEntry {
                    name: e.name,
                    is_file: e.is_file,
                    is_dir: e.is_dir,
                    size: e.size,
                })
                .collect(),
        })
    }

    async fn delete(&self, path: &str, recursive: bool) -> Result<()> {
        self.file_store.delete(path, recursive).await
    }

    async fn search(
        &self,
        path: &str,
        content_pattern: Option<&str>,
        file_pattern: Option<&str>,
    ) -> Result<SearchResult> {
        let result = self
            .grep_searcher
            .search(path, content_pattern, file_pattern)
            .await?;
        Ok(SearchResult {
            path: result.path,
            matches: result
                .matches
                .into_iter()
                .map(|m| distri_types::filesystem::SearchMatch {
                    file_path: m.file_path,
                    line_number: m.line_number,
                    line_content: m.line_content,
                    match_text: m.match_text,
                })
                .collect(),
        })
    }

    async fn copy(&self, from: &str, to: &str) -> Result<()> {
        let content = self
            .file_store
            .read(from, crate::ReadParams::default())
            .await?
            .content;
        self.file_store.write(to, &content).await
    }

    async fn move_file(&self, from: &str, to: &str) -> Result<()> {
        self.copy(from, to).await?;
        self.file_store.delete(from, false).await
    }

    async fn mkdir(&self, _path: &str) -> Result<()> {
        // Directories are created automatically when writing files
        Ok(())
    }

    async fn info(&self, path: &str) -> Result<FileMetadata> {
        self.file_store.info(path).await
    }

    async fn tree(&self, path: &str) -> Result<DirectoryListing> {
        self.list(path).await
    }
}

pub async fn create_file_system(config: crate::FileSystemConfig) -> anyhow::Result<FileSystem> {
    let file_store = Arc::new(crate::FileSystemStore::new(config).await?);
    let grep_searcher =
        Arc::new(crate::FileSystemGrepSearcher::new(file_store.clone())) as Arc<dyn GrepSearcher>;

    Ok(FileSystem::new(file_store, grep_searcher))
}
