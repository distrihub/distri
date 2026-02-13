use crate::ArtifactStorageConfig;
use anyhow::Result;
use distri_types::{filesystem::FileSystemOps, Part, ToolResponse};
use std::sync::Arc;

/// Artifact wrapper for processing tool responses and managing file storage.
///
/// The wrapper operates relative to a `prefix_path` which should be the task namespace
/// (e.g., `threads/{thread_id}/tasks/{task_id}`). All artifacts are stored under
/// `{prefix_path}/content/{filename}`.
///
/// The filesystem passed to the wrapper should already be scoped to the appropriate
/// root (e.g., `.distri/session_storage`), so the final storage path will be:
/// `{filesystem_root}/{prefix_path}/content/{filename}`
#[derive(Debug)]
pub struct ArtifactWrapper {
    filesystem: Arc<dyn FileSystemOps>,
    prefix_path: String,
}

impl ArtifactWrapper {
    pub async fn new(filesystem: Arc<dyn FileSystemOps>, prefix_path: String) -> Result<Self> {
        Ok(Self {
            filesystem,
            prefix_path,
        })
    }

    /// Generate namespace path for thread_id/task_id using short hex IDs.
    /// Returns: `threads/{short_thread}/tasks/{short_task}`
    pub fn task_namespace(thread_id: &str, task_id: &str) -> String {
        let short_thread = Self::short_hex(thread_id);
        let short_task = Self::short_hex(task_id);
        format!("threads/{}/tasks/{}", short_thread, short_task)
    }

    /// Convert ID to short hex (8 chars like git commits)
    fn short_hex(id: &str) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        id.hash(&mut hasher);
        format!("{:08x}", hasher.finish())
    }

    /// Extract thread and task hashes from an artifact_id path.
    /// Supports formats: `threads/{thread_hash}` or `threads/{thread_hash}/tasks/{task_hash}`
    /// Returns: (thread_hash, task_hash) where task_hash is Some if present
    pub fn parse_artifact_id(artifact_id: &str) -> (Option<String>, Option<String>) {
        if artifact_id.starts_with("threads/") {
            let parts: Vec<&str> = artifact_id.split('/').collect();
            if parts.len() >= 2 {
                let thread = parts[1].to_string();
                let task = if parts.len() >= 4 && parts[2] == "tasks" {
                    Some(parts[3].to_string())
                } else {
                    None
                };
                (Some(thread), task)
            } else {
                (None, None)
            }
        } else {
            (None, None)
        }
    }

    /// Get all paths to check for an artifact_id (thread level and task level if applicable).
    /// This is used when we need to check both thread-level and task-level paths
    /// to find artifacts that might be stored at either location.
    pub fn get_paths_to_check(artifact_id: &str) -> Vec<String> {
        let (thread_hash, task_hash) = Self::parse_artifact_id(artifact_id);

        let mut paths = Vec::new();
        if let Some(thread) = &thread_hash {
            paths.push(format!("threads/{}", thread));
            if let Some(task) = &task_hash {
                paths.push(format!("threads/{}/tasks/{}", thread, task));
            }
        } else {
            // Not a threads path, just check the original
            paths.push(artifact_id.to_string());
        }

        paths
    }

    /// Get the current prefix path (task namespace)
    pub fn prefix_path(&self) -> &str {
        &self.prefix_path
    }

    /// Generate the content directory path
    fn content_dir(&self) -> String {
        format!("{}/content", self.prefix_path)
    }

    /// Generate artifact file path: `{prefix_path}/content/{filename}`
    fn artifact_path(&self, filename: &str) -> String {
        format!("{}/content/{}", self.prefix_path, filename)
    }

    /// List all artifacts in the content directory
    pub async fn list_artifacts(&self) -> Result<Vec<distri_types::filesystem::DirectoryEntry>> {
        let content_path = self.content_dir();
        let listing = self.filesystem.list(&content_path).await?;
        Ok(listing.entries)
    }

    /// List artifacts checking both thread and task level paths.
    /// This merges results from all paths and deduplicates by filename.
    pub async fn list_artifacts_multi_path(
        filesystem: Arc<dyn FileSystemOps>,
        artifact_id: &str,
    ) -> Result<Vec<distri_types::filesystem::DirectoryEntry>> {
        let paths_to_check = Self::get_paths_to_check(artifact_id);
        let mut all_artifacts = Vec::new();
        let mut seen_filenames = std::collections::HashSet::new();

        for path_id in paths_to_check {
            if let Ok(wrapper) = Self::new(filesystem.clone(), path_id.clone()).await {
                if let Ok(entries) = wrapper.list_artifacts().await {
                    for entry in entries {
                        if !seen_filenames.contains(&entry.name) {
                            seen_filenames.insert(entry.name.clone());
                            all_artifacts.push(entry);
                        }
                    }
                }
            }
        }

        Ok(all_artifacts)
    }

    /// Read artifact checking both thread and task level paths.
    /// Returns the first match found, along with the path where it was found.
    pub async fn read_artifact_multi_path(
        filesystem: Arc<dyn FileSystemOps>,
        artifact_id: &str,
        filename: &str,
        start_line: Option<u64>,
        end_line: Option<u64>,
    ) -> Result<(distri_types::filesystem::FileReadResult, String)> {
        let paths_to_check = Self::get_paths_to_check(artifact_id);

        for path_id in paths_to_check {
            if let Ok(wrapper) = Self::new(filesystem.clone(), path_id.clone()).await {
                if let Ok(result) = wrapper.read_artifact(filename, start_line, end_line).await {
                    return Ok((result, path_id));
                }
            }
        }

        anyhow::bail!("Artifact '{}' not found in any namespace path", filename)
    }

    /// Read artifact content as raw string without line numbers
    pub async fn read_artifact_raw(&self, filename: &str) -> Result<String> {
        let path = self.artifact_path(filename);
        self.filesystem.read_raw(&path).await
    }

    /// Read artifact content as binary data (for images, PDFs, etc.)
    /// For base64-encoded content (like images), reads the raw base64 string as bytes
    pub async fn read_artifact_binary(&self, filename: &str) -> Result<Vec<u8>> {
        let path = self.artifact_path(filename);
        // Read as raw string (base64 content) and convert to bytes
        let content = self.filesystem.read_raw(&path).await?;
        Ok(content.into_bytes())
    }

    /// Read artifact content by filename with optional line range (includes line numbers).
    /// The filename should be just the file name (e.g., `myfile.json`), not a path.
    pub async fn read_artifact(
        &self,
        filename: &str,
        start_line: Option<u64>,
        end_line: Option<u64>,
    ) -> Result<distri_types::filesystem::FileReadResult> {
        let path = self.artifact_path(filename);
        let params = distri_types::filesystem::ReadParams {
            start_line,
            end_line,
        };
        self.filesystem.read_with_line_numbers(&path, params).await
    }

    /// Search artifacts for content pattern
    pub async fn search_artifacts(
        &self,
        pattern: &str,
    ) -> Result<distri_types::filesystem::SearchResult> {
        let content_path = self.content_dir();
        self.filesystem
            .search(&content_path, Some(pattern), None)
            .await
    }

    /// Save artifact with filename and content.
    /// Returns the filename that was saved.
    pub async fn save_artifact(&self, filename: &str, content: &str) -> Result<()> {
        let path = self.artifact_path(filename);
        self.filesystem.write(&path, content).await
    }

    /// Clean up the entire task namespace folder
    pub async fn cleanup_task_folder(&self) -> Result<()> {
        self.filesystem.delete(&self.prefix_path, true).await
    }

    /// Check if part content should be stored separately based on intelligent rules
    pub fn should_store(&self, part: &Part, config: &ArtifactStorageConfig) -> bool {
        config.should_store(part)
    }

    /// Process a part and store it as an artifact if needed.
    /// Returns an Artifact part with metadata pointing to the stored file.
    pub async fn process_part(&self, part: &Part) -> Result<Part> {
        let filename = match &part {
            Part::Data(_) | Part::ToolCall(_) | Part::ToolResult(_) => {
                format!("{}.json", uuid::Uuid::new_v4())
            }
            Part::Text(_) => format!("{}.txt", uuid::Uuid::new_v4()),
            Part::Image(_) => format!("{}.json", uuid::Uuid::new_v4()),
            Part::Artifact(part) => return Ok(Part::Artifact(part.clone())),
        };

        let content_str = match &part {
            Part::Data(value) => serde_json::to_string_pretty(value)?,
            Part::Text(text) => text.clone(),
            Part::ToolCall(call) => serde_json::to_string_pretty(call)?,
            Part::ToolResult(response) => serde_json::to_string_pretty(response)?,
            Part::Image(file_type) => serde_json::to_string_pretty(file_type)?,
            Part::Artifact(_) => unreachable!(),
        };

        self.save_artifact(&filename, &content_str).await?;

        // Store full path relative to the filesystem root so it can be retrieved later
        let relative_path = self.artifact_path(&filename);

        let metadata = distri_types::FileMetadata {
            file_id: filename.clone(),
            relative_path,
            size: content_str.len() as u64,
            content_type: match &part {
                Part::Data(_) => Some("application/json".to_string()),
                Part::Text(_) => Some("text/plain".to_string()),
                Part::ToolCall(_) => Some("application/json".to_string()),
                Part::ToolResult(_) => Some("application/json".to_string()),
                Part::Image(_) => Some("application/json".to_string()),
                Part::Artifact(_) => unreachable!(),
            },
            original_filename: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            checksum: None,
            stats: None,
            preview: if content_str.len() > 500 {
                let truncated = content_str.chars().take(500).collect::<String>();
                Some(format!("{}...", truncated))
            } else {
                Some(content_str.clone())
            },
        };

        Ok(Part::Artifact(metadata))
    }

    /// Process tool response and convert large parts to artifacts
    pub async fn process_tool_response(
        &self,
        response: ToolResponse,
        config: &ArtifactStorageConfig,
    ) -> Result<ToolResponse> {
        let mut processed_parts = Vec::new();

        for part in response.parts {
            if self.should_store(&part, config) {
                let part = self.process_part(&part).await?;
                processed_parts.push(part);
            } else {
                processed_parts.push(part);
            }
        }

        Ok(ToolResponse {
            tool_call_id: response.tool_call_id,
            tool_name: response.tool_name,
            parts: processed_parts,
            parts_metadata: None,
        })
    }

    /// Load artifact content if include_artifacts is true.
    /// Currently supports images (converts to Part::Image with base64 content).
    /// Future: will support PDFs and other file types.
    ///
    /// Returns:
    /// - If include_artifacts is false: returns Part::Artifact(metadata) unchanged
    /// - If include_artifacts is true and artifact is an image: returns Part::Image with loaded content
    /// - If include_artifacts is true but artifact type is not supported: returns Part::Artifact(metadata)
    /// - If loading fails: returns Part::Artifact(metadata) and logs warning
    pub async fn load_artifact(
        filesystem: Arc<dyn FileSystemOps>,
        metadata: &distri_types::FileMetadata,
        include_artifacts: bool,
    ) -> Part {
        if !include_artifacts {
            return Part::Artifact(metadata.clone());
        }

        // Check if this is an image artifact
        let is_image = metadata
            .content_type
            .as_ref()
            .map(|ct| ct.starts_with("image/"))
            .unwrap_or(false)
            || metadata.file_id.ends_with(".png")
            || metadata.file_id.ends_with(".jpg")
            || metadata.file_id.ends_with(".jpeg");

        if is_image {
            // Extract artifact namespace from relative_path
            // Format: threads/{thread_hash}/tasks/{task_hash}/content/{filename}
            if let Some((artifact_namespace, filename)) =
                metadata.relative_path.rsplit_once("/content/")
            {
                match Self::new(filesystem.clone(), artifact_namespace.to_string()).await {
                    Ok(wrapper) => {
                        // For image artifacts, read as binary (bytes) then encode to base64
                        // This handles the case where artifacts are stored as base64 strings
                        match wrapper.read_artifact_binary(&filename).await {
                            Ok(bytes) => {
                                // The bytes are the UTF-8 encoding of the base64 string
                                // Convert to string and use directly as base64
                                match String::from_utf8(bytes) {
                                    Ok(base64_content) => {
                                        // Verify it's valid base64
                                        use base64::{engine::general_purpose, Engine as _};
                                        if general_purpose::STANDARD
                                            .decode(&base64_content)
                                            .is_err()
                                        {
                                            tracing::warn!(
                                                "Image artifact {} does not contain valid base64. Keeping as artifact.",
                                                metadata.file_id
                                            );
                                        } else {
                                            use distri_types::FileType;
                                            let image_part = Part::Image(FileType::Bytes {
                                                bytes: base64_content, // Base64-encoded string
                                                mime_type: metadata
                                                    .content_type
                                                    .clone()
                                                    .unwrap_or_else(|| "image/png".to_string()),
                                                name: Some(metadata.file_id.clone()),
                                            });
                                            let image_size = match &image_part {
                                                Part::Image(FileType::Bytes { bytes, .. }) => {
                                                    bytes.len()
                                                }
                                                _ => 0,
                                            };
                                            tracing::info!(
                                                "Loaded image artifact {} as Part::Image ({} base64 chars)",
                                                metadata.file_id,
                                                image_size
                                            );
                                            return image_part;
                                        }
                                    }
                                    Err(e) => {
                                        tracing::warn!(
                                            "Failed to convert image artifact bytes to UTF-8 string: {}. Keeping as artifact.",
                                            e
                                        );
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "Failed to read image artifact {}: {}. Keeping as artifact.",
                                    metadata.file_id,
                                    e
                                );
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to create artifact wrapper for {}: {}. Keeping as artifact.",
                            artifact_namespace,
                            e
                        );
                    }
                }
            }
        } else {
            // Future: Handle other file types (PDFs, etc.) here
            // For now, keep as artifact
            tracing::debug!(
                "Artifact {} is not an image and include_artifacts is true. Keeping as artifact (future: will support PDFs, etc.)",
                metadata.file_id
            );
        }

        // Return as artifact if conversion failed, not an image, or not supported yet
        Part::Artifact(metadata.clone())
    }
}
