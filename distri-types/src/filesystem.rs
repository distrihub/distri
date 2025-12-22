use anyhow::Result;
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Context information for file storage operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileContext {
    /// Thread ID for organizing files
    pub thread_id: String,
    /// Task ID if available
    pub task_id: Option<String>,
    /// Tool call ID that generated this content
    pub tool_call_id: Option<String>,
    /// Content type/mime type hint
    pub content_type: Option<String>,
    /// Original filename if content represents a file
    pub original_filename: Option<String>,
}

/// Pure filesystem metadata about a file - no artifact context
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct FileMetadata {
    /// Unique file ID
    pub file_id: String,
    /// Relative path from filesystem base
    pub relative_path: String,
    /// File size in bytes
    pub size: u64,
    /// Content type/mime type if known
    pub content_type: Option<String>,
    /// Original filename if available
    pub original_filename: Option<String>,
    /// When the file was created
    #[schemars(with = "String")]
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// When the file was last modified
    #[schemars(with = "String")]
    pub updated_at: chrono::DateTime<chrono::Utc>,
    /// File checksum for integrity verification
    pub checksum: Option<String>,
    /// Rich statistics about the file content
    pub stats: Option<FileStats>,
    /// Short preview of the content for context
    pub preview: Option<String>,
}

/// Artifact metadata that combines filesystem metadata with context information
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct Artifact {
    /// Filesystem metadata
    pub file_metadata: FileMetadata,
    /// Associated thread ID
    pub thread_id: String,
    /// Associated task ID if available
    pub task_id: Option<String>,
    /// Tool call ID that generated this artifact
    pub tool_call_id: Option<String>,
}

impl FileMetadata {
    /// Get the full display name for this file
    pub fn display_name(&self) -> String {
        self.original_filename
            .clone()
            .unwrap_or_else(|| format!("file_{}", &self.file_id[..8]))
    }

    /// Get a human readable size string
    pub fn size_display(&self) -> String {
        let size = self.size as f64;
        if size < 1024.0 {
            format!("{}B", self.size)
        } else if size < 1024.0 * 1024.0 {
            format!("{:.1}KB", size / 1024.0)
        } else if size < 1024.0 * 1024.0 * 1024.0 {
            format!("{:.1}MB", size / (1024.0 * 1024.0))
        } else {
            format!("{:.1}GB", size / (1024.0 * 1024.0 * 1024.0))
        }
    }

    /// Check if this appears to be a text file
    pub fn is_text_file(&self) -> bool {
        self.content_type
            .as_ref()
            .map(|ct| ct.starts_with("text/") || ct.contains("json") || ct.contains("xml"))
            .unwrap_or(false)
    }

    /// Get a summary line for display
    pub fn summary(&self) -> String {
        format!(
            "{} ({}{})",
            self.display_name(),
            self.size_display(),
            if let Some(ct) = &self.content_type {
                format!(", {}", ct)
            } else {
                String::new()
            }
        )
    }
}

impl Artifact {
    /// Create a new artifact with file metadata and context
    pub fn new(
        file_metadata: FileMetadata,
        thread_id: String,
        task_id: Option<String>,
        tool_call_id: Option<String>,
    ) -> Self {
        Self {
            file_metadata,
            thread_id,
            task_id,
            tool_call_id,
        }
    }

    /// Get the artifact path in the namespaced format
    pub fn artifact_path(&self) -> String {
        if let Some(task_id) = &self.task_id {
            format!(
                "{}/artifact/{}/{}",
                self.thread_id, task_id, self.file_metadata.file_id
            )
        } else {
            format!("{}/artifact/{}", self.thread_id, self.file_metadata.file_id)
        }
    }

    /// Delegate display methods to file metadata
    pub fn display_name(&self) -> String {
        self.file_metadata.display_name()
    }

    pub fn size_display(&self) -> String {
        self.file_metadata.size_display()
    }

    pub fn summary(&self) -> String {
        self.file_metadata.summary()
    }
}

/// Artifact namespace for organizing artifacts by thread and task
/// Handles path creation logic consistently across the codebase
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Hash)]
pub struct ArtifactNamespace {
    /// Thread ID (required)
    pub thread_id: String,
    /// Task ID (optional - if None, uses thread-level namespace)
    pub task_id: Option<String>,
}

impl ArtifactNamespace {
    /// Create a new namespace from thread_id and optional task_id
    pub fn new(thread_id: String, task_id: Option<String>) -> Self {
        Self {
            thread_id,
            task_id,
        }
    }

    /// Convert ID to short hex (8 chars like git commits)
    fn short_hex(id: &str) -> String {
        let mut hasher = DefaultHasher::new();
        id.hash(&mut hasher);
        format!("{:08x}", hasher.finish())
    }

    /// Get the thread-level namespace path: `threads/{short_thread}`
    pub fn thread_path(&self) -> String {
        let short_thread = Self::short_hex(&self.thread_id);
        format!("threads/{}", short_thread)
    }

    /// Get the task-level namespace path: `threads/{short_thread}/tasks/{short_task}`
    /// Returns None if task_id is not set
    pub fn task_path(&self) -> Option<String> {
        self.task_id.as_ref().map(|task_id| {
            let short_thread = Self::short_hex(&self.thread_id);
            let short_task = Self::short_hex(task_id);
            format!("threads/{}/tasks/{}", short_thread, short_task)
        })
    }

    /// Get the primary namespace path (task-level if available, otherwise thread-level)
    /// This is the path where artifacts should be saved
    pub fn primary_path(&self) -> String {
        self.task_path().unwrap_or_else(|| self.thread_path())
    }

    /// Get all paths that should be checked when listing artifacts
    /// Returns both thread-level and task-level paths (if task_id is set)
    /// This ensures list_artifacts can find artifacts saved at either level
    pub fn all_paths(&self) -> Vec<String> {
        let mut paths = vec![self.thread_path()];
        if let Some(task_path) = self.task_path() {
            paths.push(task_path);
        }
        paths
    }

    /// Parse a namespace path back into thread_id and task_id
    /// Handles both `threads/{hash}` and `threads/{hash}/tasks/{hash}` formats
    /// Note: This cannot reverse the hash to get the original UUIDs, so it returns None
    /// In practice, you should store the mapping or use the namespace directly
    pub fn from_path(_path: &str) -> Option<Self> {
        // We can't reverse the hash, so we return None
        // In practice, we'd need to store the mapping or use the namespace directly
        None
    }
}

/// Type-specific file statistics that provide rich metadata about file content
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FileStats {
    Json(JsonStats),
    Markdown(MarkdownStats),
    Text(TextStats),
}

impl FileStats {
    /// Get the type of file stats as a string
    pub fn stats_type(&self) -> &'static str {
        match self {
            FileStats::Json(_) => "json",
            FileStats::Markdown(_) => "markdown",
            FileStats::Text(_) => "text",
        }
    }

    /// Extract a human-readable summary of the file stats
    pub fn summary(&self) -> String {
        match self {
            FileStats::Json(stats) => stats.summary(),
            FileStats::Markdown(stats) => stats.summary(),
            FileStats::Text(stats) => stats.summary(),
        }
    }

    /// Get context information suitable for agent prompts
    pub fn context_info(&self) -> String {
        match self {
            FileStats::Json(stats) => stats.context_info(),
            FileStats::Markdown(stats) => stats.context_info(),
            FileStats::Text(stats) => stats.context_info(),
        }
    }
}

/// Statistics for JSON files
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
pub struct JsonStats {
    /// Whether the root is an array
    pub is_array: bool,
    /// Length if it's an array
    pub array_length: Option<usize>,
    /// Top-level keys (for objects) or sample element keys (for arrays)
    pub top_level_keys: Vec<String>,
    /// Maximum nesting depth
    pub nested_depth: usize,
    /// Sample of unique values for interesting fields (max 5 values each)
    pub unique_values_sample: HashMap<String, Vec<String>>,
    /// Estimated cardinality for fields with many unique values
    pub cardinality_estimates: HashMap<String, usize>,
    /// Preview of first few elements/properties (as JSON string)
    pub preview: String,
}

impl JsonStats {
    pub fn summary(&self) -> String {
        if self.is_array {
            format!(
                "JSON array with {} elements, {} unique keys, depth {}",
                self.array_length.unwrap_or(0),
                self.top_level_keys.len(),
                self.nested_depth
            )
        } else {
            format!(
                "JSON object with {} keys, depth {}",
                self.top_level_keys.len(),
                self.nested_depth
            )
        }
    }

    pub fn context_info(&self) -> String {
        let mut info = self.summary();

        if !self.top_level_keys.is_empty() {
            info.push_str(&format!("\nKeys: {}", self.top_level_keys.join(", ")));
        }

        // Highlight high-cardinality fields
        let high_card_fields: Vec<_> = self
            .cardinality_estimates
            .iter()
            .filter(|&(_, &count)| count > 50)
            .map(|(field, count)| format!("{} (~{})", field, count))
            .collect();

        if !high_card_fields.is_empty() {
            info.push_str(&format!(
                "\nHigh-cardinality fields: {}",
                high_card_fields.join(", ")
            ));
        }

        // Show sample values for categorical fields
        for (field, values) in &self.unique_values_sample {
            if values.len() <= 10 {
                // Only show for low-cardinality categorical fields
                info.push_str(&format!("\n{}: {}", field, values.join(", ")));
            }
        }

        info
    }
}

/// Statistics for Markdown files
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
pub struct MarkdownStats {
    /// Word count
    pub word_count: usize,
    /// Headings with their text and level (1-6)
    pub headings: Vec<HeadingInfo>,
    /// Number of code blocks
    pub code_blocks: usize,
    /// Number of links
    pub links: usize,
    /// Number of images
    pub images: usize,
    /// Number of tables
    pub tables: usize,
    /// Number of lists
    pub lists: usize,
    /// YAML/TOML frontmatter type if present
    pub front_matter: Option<String>,
    /// Preview of first few lines
    pub preview: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
pub struct HeadingInfo {
    pub text: String,
    pub level: usize,
}

impl MarkdownStats {
    pub fn summary(&self) -> String {
        format!(
            "Markdown: {} words, {} headings, {} code blocks, {} tables",
            self.word_count,
            self.headings.len(),
            self.code_blocks,
            self.tables
        )
    }

    pub fn context_info(&self) -> String {
        let mut info = self.summary();

        if !self.headings.is_empty() {
            info.push_str("\nStructure:");
            for heading in &self.headings[..5.min(self.headings.len())] {
                let indent = "  ".repeat(heading.level.saturating_sub(1));
                info.push_str(&format!("\n{}{}", indent, heading.text));
            }
        }

        if let Some(fm_type) = &self.front_matter {
            info.push_str(&format!("\nFrontmatter: {}", fm_type));
        }

        info
    }
}

/// Statistics for plain text files
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
pub struct TextStats {
    /// Number of lines
    pub lines: usize,
    /// Number of words
    pub words: usize,
    /// Number of characters
    pub characters: usize,
    /// Detected encoding
    pub encoding: String,
    /// Detected language (if any)
    pub language: Option<String>,
    /// Hints about the text structure
    pub structure_hints: TextStructure,
    /// Preview of first few lines
    pub preview: String,
}

/// Detected structure patterns in text files
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TextStructure {
    LogFile {
        log_level_counts: HashMap<String, usize>,
    },
    ConfigFile {
        format: String,
    },
    CodeFile {
        language: String,
        function_count: usize,
    },
    PlainText,
}

impl TextStats {
    pub fn summary(&self) -> String {
        format!(
            "Text: {} lines, {} words ({} chars)",
            self.lines, self.words, self.characters
        )
    }

    pub fn context_info(&self) -> String {
        let mut info = self.summary();

        if let Some(lang) = &self.language {
            info.push_str(&format!("\nLanguage: {}", lang));
        }

        match &self.structure_hints {
            TextStructure::LogFile { log_level_counts } => {
                info.push_str("\nStructure: Log file");
                let levels: Vec<_> = log_level_counts
                    .iter()
                    .map(|(level, count)| format!("{}: {}", level, count))
                    .collect();
                if !levels.is_empty() {
                    info.push_str(&format!("\nLevels: {}", levels.join(", ")));
                }
            }
            TextStructure::ConfigFile { format } => {
                info.push_str(&format!("\nStructure: Config file ({})", format));
            }
            TextStructure::CodeFile {
                language,
                function_count,
            } => {
                info.push_str(&format!(
                    "\nStructure: Code file ({}, {} functions)",
                    language, function_count
                ));
            }
            TextStructure::PlainText => {
                info.push_str("\nStructure: Plain text");
            }
        }

        info
    }
}

/// Parameters for reading files with optional line ranges
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReadParams {
    pub start_line: Option<u64>,
    pub end_line: Option<u64>,
}

/// File read result with content and metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileReadResult {
    pub content: String,
    pub start_line: u64,
    pub end_line: u64,
    pub total_lines: u64,
}

/// Directory listing result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryListing {
    pub path: String,
    pub entries: Vec<DirectoryEntry>,
}

/// Directory entry information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryEntry {
    pub name: String,
    pub is_file: bool,
    pub is_dir: bool,
    pub size: Option<u64>,
}

/// Search result containing matches
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub path: String,
    pub matches: Vec<SearchMatch>,
}

/// Individual search match
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchMatch {
    pub file_path: String,
    pub line_number: Option<u64>,
    pub line_content: String,
    pub match_text: String,
}

/// Legacy filesystem operations interface for tool compatibility
#[async_trait]
pub trait FileSystemOps: Send + Sync + std::fmt::Debug {
    /// Read file with optional line range (includes line numbers)
    async fn read(&self, path: &str, params: ReadParams) -> Result<FileReadResult>;

    /// Read file as raw string without line numbers
    async fn read_raw(&self, path: &str) -> Result<String> {
        // Default implementation: use read() and strip line numbers
        let result = self.read(path, ReadParams::default()).await?;
        // Strip line number prefixes if present
        if result.content.contains("→") {
            Ok(result
                .content
                .lines()
                .map(|line| {
                    if let Some(pos) = line.find("→") {
                        &line[pos + 1..]
                    } else {
                        line
                    }
                })
                .collect::<Vec<_>>()
                .join("\n"))
        } else {
            Ok(result.content)
        }
    }

    /// Read file with line numbers and optional line range
    async fn read_with_line_numbers(
        &self,
        path: &str,
        params: ReadParams,
    ) -> Result<FileReadResult> {
        self.read(path, params).await
    }

    /// Write content to file
    async fn write(&self, path: &str, content: &str) -> Result<()>;

    /// List directory contents with metadata
    async fn list(&self, path: &str) -> Result<DirectoryListing>;

    /// Delete file or directory
    async fn delete(&self, path: &str, recursive: bool) -> Result<()>;

    /// Search files and content
    async fn search(
        &self,
        path: &str,
        content_pattern: Option<&str>,
        file_pattern: Option<&str>,
    ) -> Result<SearchResult>;

    /// Copy file (legacy - should use shell commands)
    async fn copy(&self, from: &str, to: &str) -> Result<()>;

    /// Move file (legacy - should use shell commands)
    async fn move_file(&self, from: &str, to: &str) -> Result<()>;

    /// Create directory (legacy - directories created automatically)
    async fn mkdir(&self, path: &str) -> Result<()>;

    /// Get file metadata
    async fn info(&self, path: &str) -> Result<FileMetadata>;

    /// List directory tree (legacy - same as list)
    async fn tree(&self, path: &str) -> Result<DirectoryListing>;
}
