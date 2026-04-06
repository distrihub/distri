use distri_types::configuration::ObjectStorageConfig;
use distri_types::Part;
use serde::{Deserialize, Serialize};

/// Configuration for the FileStore
#[derive(Debug, Clone)]
pub struct FileSystemConfig {
    /// Object store backing the filesystem operations
    pub object_store: ObjectStorageConfig,
    /// Optional prefix applied to all stored paths (used for run namespacing)
    pub root_prefix: Option<String>,
}

impl Default for FileSystemConfig {
    fn default() -> Self {
        Self {
            object_store: ObjectStorageConfig::FileSystem {
                base_path: ".distri/files".to_string(),
            },
            root_prefix: Some("runs/default".to_string()),
        }
    }
}

/// Configuration for intelligent artifact storage decisions
#[derive(Debug, Clone)]
pub struct ArtifactStorageConfig {
    /// Token threshold for Part::Data (approximate token count)
    pub data_token_threshold: usize,
    /// Token threshold for Part::Text (approximate token count)
    pub text_token_threshold: usize,
    /// Always store images/files as artifacts (typically large binary data)
    pub always_store_images: bool,
    /// For testing: always store everything as artifacts to test the behavior
    pub always_store_for_testing: bool,
}

impl Default for ArtifactStorageConfig {
    fn default() -> Self {
        Self {
            data_token_threshold: 1000,
            text_token_threshold: 1000,
            always_store_images: true,
            always_store_for_testing: true,
        }
    }
}

impl ArtifactStorageConfig {
    /// Create configuration for testing - stores everything as artifacts
    pub fn for_testing() -> Self {
        Self {
            data_token_threshold: 0,
            text_token_threshold: 0,
            always_store_images: true,
            always_store_for_testing: true,
        }
    }

    /// Create configuration for production - stores based on intelligent thresholds
    pub fn for_production() -> Self {
        Self {
            data_token_threshold: 10000,
            text_token_threshold: 10000,
            always_store_images: false,
            always_store_for_testing: false,
        }
    }

    /// Check if part content should be stored separately based on this configuration
    pub fn should_store(&self, part: &Part) -> bool {
        if self.always_store_for_testing {
            return match part {
                Part::ToolCall(_) => false,
                Part::Artifact(_) => false,
                _ => true,
            };
        }

        match part {
            Part::Data(value) => {
                let estimated_tokens = estimate_json_tokens(value);
                estimated_tokens > self.data_token_threshold
            }
            Part::Text(text) => {
                let estimated_tokens = estimate_text_tokens(text);
                estimated_tokens > self.text_token_threshold
            }
            Part::ToolCall(_) => false,
            Part::ToolResult(response) => response.parts.iter().any(|p| self.should_store(p)),
            Part::Image(_) => self.always_store_images,
            Part::Artifact(_) => false,
        }
    }
}

/// Parameters for reading files
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

/// Estimate token count for JSON data (rough approximation)
fn estimate_json_tokens(value: &serde_json::Value) -> usize {
    let json_string = serde_json::to_string(value).unwrap_or_default();
    json_string.len() / 4
}

/// Estimate token count for text (rough approximation)
fn estimate_text_tokens(text: &str) -> usize {
    text.len() / 4
}
