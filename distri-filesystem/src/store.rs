use crate::{DirectoryEntry, DirectoryListing, FileReadResult, FileSystemConfig, ReadParams};
use anyhow::{anyhow, Context, Result};
use bytes::Bytes;
use futures::TryStreamExt;
use object_store::path::Path;
use object_store::ObjectStore;
use std::sync::Arc;

/// Object-store backed file storage implementation
#[derive(Debug)]
pub struct FileSystemStore {
    store: Arc<dyn ObjectStore>,
    root: Option<Path>,
}

impl FileSystemStore {
    /// Create a new FileSystemStore using the provided configuration
    pub async fn new(config: FileSystemConfig) -> Result<Self> {
        let store = crate::object_store::build_object_store(&config.object_store)?;

        let root = config.root_prefix.as_ref().and_then(|prefix| {
            let trimmed = prefix.trim_matches('/');
            if trimmed.is_empty() {
                None
            } else {
                Some(Path::from(trimmed))
            }
        });

        Ok(Self { store, root })
    }

    pub fn root_prefix(&self) -> Option<String> {
        self.root.as_ref().map(|p| p.to_string())
    }

    pub fn scoped(&self, prefix: Option<&str>) -> Result<Self> {
        let new_root = match prefix {
            Some(extra) => Self::combine_prefixes(&self.root, extra)?,
            None => self.root.clone(),
        };
        Ok(Self {
            store: self.store.clone(),
            root: new_root,
        })
    }

    fn ensure_safe_path(path: &str) -> Result<&str> {
        if path.split('/').any(|segment| segment == "..") {
            return Err(anyhow!("path segments must not contain '..'"));
        }
        Ok(path)
    }

    fn combine_prefixes(base: &Option<Path>, extra: &str) -> Result<Option<Path>> {
        let trimmed = extra.trim_matches('/');
        if trimmed.is_empty() {
            return Ok(base.clone());
        }
        Self::ensure_safe_path(trimmed)?;
        let combined = match base {
            Some(existing) => {
                let mut prefix = existing.to_string();
                if !prefix.ends_with('/') {
                    prefix.push('/');
                }
                prefix.push_str(trimmed);
                Path::from(prefix)
            }
            None => Path::from(trimmed),
        };
        Ok(Some(combined))
    }

    fn sanitize_object_path(&self, path: &str) -> Result<Path> {
        let trimmed = path.trim_matches('/');
        if trimmed.is_empty() {
            return Err(anyhow!("path cannot be empty"));
        }
        Self::ensure_safe_path(trimmed)?;
        let normalized = match &self.root {
            Some(root) if !root.as_ref().is_empty() => {
                // Join the root prefix with the relative path while preserving separators
                Path::from(format!("{}/{}", root, trimmed))
            }
            _ => Path::from(trimmed),
        };
        Ok(normalized)
    }

    fn sanitize_prefix(&self, path: &str) -> Result<Option<Path>> {
        let trimmed = path.trim_matches('/');
        if trimmed.is_empty() {
            return Ok(self.root.clone());
        }
        Self::ensure_safe_path(trimmed)?;
        let prefix = match &self.root {
            Some(root) if !root.as_ref().is_empty() => {
                // Combine the root prefix with the requested path using path separators
                Path::from(format!("{}/{}", root, trimmed))
            }
            _ => Path::from(trimmed),
        };
        Ok(Some(prefix))
    }

    fn prefix_depth(prefix: &Option<Path>) -> usize {
        prefix.as_ref().map(|p| p.parts().count()).unwrap_or(0)
    }

    fn entry_name(prefix: &Option<Path>, entry: &Path) -> Option<String> {
        let depth = Self::prefix_depth(prefix);
        entry
            .parts()
            .nth(depth)
            .map(|component| component.as_ref().to_string())
    }

    fn build_listing_entry(
        prefix: &Option<Path>,
        entry: &Path,
        is_dir: bool,
        size: Option<u64>,
    ) -> Option<DirectoryEntry> {
        Self::entry_name(prefix, entry).map(|name| DirectoryEntry {
            name,
            is_file: !is_dir,
            is_dir,
            size,
        })
    }

    fn read_to_string(bytes: Bytes, path: &str) -> Result<String> {
        String::from_utf8(bytes.to_vec())
            .with_context(|| format!("failed to decode file {} as utf-8", path))
    }

    /// Read file content as raw string without line numbers
    pub async fn read_raw(&self, path: &str) -> Result<String> {
        let object_path = self
            .sanitize_object_path(path)
            .with_context(|| format!("invalid file path: {}", path))?;
        tracing::debug!("object_path: {}", object_path.to_string());
        let get_result = self
            .store
            .get(&object_path)
            .await
            .map_err(|e| {
                tracing::error!("#{e}");
                e
            })
            .with_context(|| format!("failed to fetch object for {path}"))?;
        let bytes = get_result
            .bytes()
            .await
            .with_context(|| format!("failed to read bytes for {path}"))?;
        Self::read_to_string(bytes, path)
    }

    /// Read file content with line numbers and optional line range
    pub async fn read_with_line_numbers(
        &self,
        path: &str,
        params: ReadParams,
    ) -> Result<FileReadResult> {
        let content = self.read_raw(path).await?;

        let lines: Vec<&str> = content.lines().collect();
        let total_lines = lines.len() as u64;

        if total_lines == 0 {
            return Ok(FileReadResult {
                content: String::new(),
                start_line: 0,
                end_line: 0,
                total_lines: 0,
            });
        }

        let start = params.start_line.unwrap_or(1);
        let end = params.end_line.unwrap_or(total_lines);

        if start > total_lines || end > total_lines || start > end || start == 0 {
            return Err(anyhow!(
                "invalid line range: {}-{} for file {} with {} total lines",
                start,
                end,
                path,
                total_lines
            ));
        }

        let start_idx = (start - 1) as usize;
        let end_idx = end as usize;
        let selected_lines = &lines[start_idx..end_idx];
        let content_with_lines = selected_lines
            .iter()
            .enumerate()
            .map(|(i, line)| format!("{:4}→{}", start + i as u64, line))
            .collect::<Vec<_>>()
            .join("\n");

        Ok(FileReadResult {
            content: content_with_lines,
            start_line: start,
            end_line: end,
            total_lines,
        })
    }

    /// Read file with optional line range (backward compatibility - uses read_with_line_numbers)
    pub async fn read(&self, path: &str, params: ReadParams) -> Result<FileReadResult> {
        self.read_with_line_numbers(path, params).await
    }

    pub async fn write(&self, path: &str, content: &str) -> Result<()> {
        let object_path = self
            .sanitize_object_path(path)
            .with_context(|| format!("invalid file path: {}", path))?;
        tracing::debug!("object_path: {}", object_path.to_string());
        self.store
            .put(&object_path, Bytes::from(content.to_owned()))
            .await
            .map_err(|e| {
                tracing::error!("#{e}");
                e
            })
            .with_context(|| format!("failed to write file {}", path))?;
        Ok(())
    }

    pub async fn write_binary(&self, path: &str, content: &[u8]) -> Result<()> {
        let object_path = self
            .sanitize_object_path(path)
            .with_context(|| format!("invalid binary path: {}", path))?;
        tracing::debug!("object_path: {}", object_path.to_string());
        self.store
            .put(&object_path, Bytes::copy_from_slice(content))
            .await
            .with_context(|| format!("failed to write binary file {}", path))?;
        Ok(())
    }

    pub async fn read_binary(&self, path: &str) -> Result<Vec<u8>> {
        let object_path = self
            .sanitize_object_path(path)
            .with_context(|| format!("invalid binary path: {}", path))?;
        let get_result = self
            .store
            .get(&object_path)
            .await
            .map_err(|e| {
                tracing::error!("#{e}");
                e
            })
            .with_context(|| format!("failed to fetch binary object for {}", path))?;
        let bytes = get_result
            .bytes()
            .await
            .map_err(|e| {
                tracing::error!("#{e}");
                e
            })
            .with_context(|| format!("failed to read binary bytes for {}", path))?;
        Ok(bytes.to_vec())
    }

    pub async fn list(&self, path: &str) -> Result<DirectoryListing> {
        let prefix = self.sanitize_prefix(path)?;
        let list_result = match &prefix {
            Some(prefix) => self
                .store
                .list_with_delimiter(Some(prefix))
                .await
                .map_err(|e| {
                    tracing::error!("#{e}");
                    e
                })
                .with_context(|| format!("failed to list directory {}", path))?,
            None => self
                .store
                .list_with_delimiter(None)
                .await
                .map_err(|e| {
                    tracing::error!("#{e}");
                    e
                })
                .context("failed to list root directory")?,
        };

        let mut entries: Vec<DirectoryEntry> = Vec::new();

        for common_prefix in list_result.common_prefixes {
            if let Some(entry) = Self::build_listing_entry(&prefix, &common_prefix, true, None) {
                entries.push(entry);
            }
        }

        for object in list_result.objects {
            if let Some(entry) = Self::build_listing_entry(
                &prefix,
                &object.location,
                false,
                Some(object.size as u64),
            ) {
                entries.push(entry);
            }
        }

        entries.sort_by(|a, b| a.name.cmp(&b.name));
        entries.dedup_by(|a, b| a.name == b.name && a.is_dir == b.is_dir);

        Ok(DirectoryListing {
            path: path.to_string(),
            entries,
        })
    }

    pub async fn delete(&self, path: &str, recursive: bool) -> Result<()> {
        let object_path = self
            .sanitize_object_path(path)
            .with_context(|| format!("invalid delete path: {}", path))?;

        if recursive {
            let prefix = self
                .sanitize_prefix(path)?
                .ok_or_else(|| anyhow!("cannot delete root prefix recursively"))?;

            let mut stream = self.store.list(Some(&prefix));
            while let Some(meta) = stream.try_next().await? {
                self.store.delete(&meta.location).await.with_context(|| {
                    format!(
                        "failed to delete object {} while deleting directory {}",
                        meta.location, path
                    )
                })?;
            }
            return Ok(());
        }

        match self.store.delete(&object_path).await {
            Ok(()) => Ok(()),
            Err(object_store::Error::NotFound { .. }) => {
                let prefix = self.sanitize_prefix(path)?;
                if let Some(prefix) = prefix {
                    let mut stream = self.store.list(Some(&prefix));
                    if stream.try_next().await?.is_none() {
                        return Ok(());
                    }
                }
                Err(anyhow!("path {} not found", path))
            }
            Err(err) => Err(err).with_context(|| format!("failed to delete path {}", path)),
        }
    }

    pub async fn copy(&self, source: &str, destination: &str) -> Result<()> {
        let content = self
            .read(source, ReadParams::default())
            .await
            .with_context(|| format!("failed to read source file {}", source))?;
        self.write(destination, &content.content).await
    }

    pub async fn move_file(&self, source: &str, destination: &str) -> Result<()> {
        self.copy(source, destination).await?;
        self.delete(source, false).await
    }

    pub async fn info(&self, path: &str) -> Result<distri_types::filesystem::FileMetadata> {
        let object_path = self
            .sanitize_object_path(path)
            .with_context(|| format!("invalid info path: {}", path))?;
        let metadata = self
            .store
            .head(&object_path)
            .await
            .with_context(|| format!("failed to fetch metadata for {}", path))?;

        let preview_content = self
            .read(path, ReadParams::default())
            .await
            .ok()
            .map(|result| result.content);

        Ok(distri_types::filesystem::FileMetadata {
            file_id: uuid::Uuid::new_v4().to_string(),
            relative_path: path.trim_start_matches('/').to_string(),
            size: metadata.size as u64,
            content_type: None,
            original_filename: path.split('/').last().map(|s| s.to_string()),
            created_at: metadata.last_modified,
            updated_at: metadata.last_modified,
            checksum: metadata.e_tag,
            stats: None,
            preview: preview_content,
        })
    }

    pub async fn tree(&self, path: &str) -> Result<DirectoryListing> {
        self.list(path).await
    }
}
