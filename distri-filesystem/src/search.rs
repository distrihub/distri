use crate::{FileSystemStore, SearchMatch, SearchResult};
use anyhow::{Context, Result};
use async_trait::async_trait;
use grep::regex::RegexMatcher;
use grep::searcher::{sinks::UTF8, Searcher};
use std::sync::Arc;

/// Grep-based searcher that works on top of FileSystemStore
#[derive(Debug)]
pub struct FileSystemGrepSearcher {
    file_store: Arc<FileSystemStore>,
}

impl FileSystemGrepSearcher {
    pub fn new(file_store: Arc<FileSystemStore>) -> Self {
        Self { file_store }
    }

    /// Search files using grep library with FileStore interface
    async fn search_with_grep(
        &self,
        base_path: &str,
        content_pattern: &str,
        file_pattern: Option<&str>,
    ) -> Result<Vec<SearchMatch>> {
        let mut matches = Vec::new();
        let matcher = RegexMatcher::new(content_pattern)
            .with_context(|| format!("Invalid regex pattern: {}", content_pattern))?;

        // Recursively search through directories
        self.search_directory(
            base_path,
            &matcher,
            content_pattern,
            file_pattern,
            &mut matches,
        )
        .await?;

        Ok(matches)
    }

    fn search_directory<'a>(
        &'a self,
        dir_path: &'a str,
        matcher: &'a RegexMatcher,
        pattern: &'a str,
        file_pattern: Option<&'a str>,
        matches: &'a mut Vec<SearchMatch>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let listing = self.file_store.list(dir_path).await?;

            for entry in &listing.entries {
                let entry_path = if dir_path.is_empty() {
                    entry.name.clone()
                } else {
                    format!("{}/{}", dir_path, entry.name)
                };

                if entry.is_dir {
                    // Recursive search in subdirectory
                    self.search_directory(&entry_path, matcher, pattern, file_pattern, matches)
                        .await?;
                } else {
                    // Apply file pattern filtering
                    if let Some(fp) = file_pattern {
                        if fp.starts_with("*.") {
                            let extension = &fp[2..];
                            if !entry.name.ends_with(&format!(".{}", extension)) {
                                continue;
                            }
                        } else if !entry.name.contains(fp) {
                            continue;
                        }
                    }

                    // Read file and search content using search_reader
                    if let Ok(read_result) = self
                        .file_store
                        .read(&entry_path, crate::ReadParams::default())
                        .await
                    {
                        let mut searcher = Searcher::new();
                        let content_reader = std::io::Cursor::new(read_result.content.as_bytes());

                        let path_clone = entry_path.clone();
                        let _result = searcher.search_reader(
                            matcher,
                            content_reader,
                            UTF8(|line_num, line_content| {
                                matches.push(SearchMatch {
                                    file_path: path_clone.clone(),
                                    line_number: Some(line_num),
                                    line_content: line_content.trim_end().to_string(),
                                    match_text: pattern.to_string(),
                                });
                                Ok(true)
                            }),
                        );
                    }
                }
            }

            Ok(())
        })
    }
}

#[async_trait]
impl crate::GrepSearcher for FileSystemGrepSearcher {
    async fn search(
        &self,
        path: &str,
        content_pattern: Option<&str>,
        file_pattern: Option<&str>,
    ) -> Result<SearchResult> {
        let matches = if let Some(pattern) = content_pattern {
            // Use grep library for content search
            self.search_with_grep(path, pattern, file_pattern).await?
        } else {
            // Just file name search
            let listing = self.file_store.list(path).await?;
            let mut matches = Vec::new();

            for entry in &listing.entries {
                if let Some(pattern) = file_pattern {
                    if entry.name.contains(pattern) {
                        let file_path = if path.is_empty() {
                            entry.name.clone()
                        } else {
                            format!("{}/{}", path, entry.name)
                        };

                        matches.push(SearchMatch {
                            file_path,
                            line_number: None,
                            line_content: String::new(),
                            match_text: pattern.to_string(),
                        });
                    }
                }
            }

            matches
        };

        Ok(SearchResult {
            path: path.to_string(),
            matches,
        })
    }
}
