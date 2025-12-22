use anyhow::{Context, Result};
use distri_types::filesystem::FileSystemOps;
use distri_types::{Tool, ToolContext};
use schemars::{schema_for, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

// File Operations

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
pub struct WriteFileParams {
    /// File path relative to the CODE_HOME namespace.
    #[schemars(description = "File path relative to the CODE_HOME namespace")]
    pub path: String,
    /// Full file contents to write.
    #[schemars(description = "Full file contents to write")]
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
pub struct ReadFileParams {
    /// File path relative to the CODE_HOME namespace.
    #[schemars(description = "File path relative to the CODE_HOME namespace")]
    pub path: String,
    /// Optional starting line (1-based).
    #[schemars(range(min = 1))]
    #[schemars(description = "Optional starting line (1-based)")]
    pub start_line: Option<u64>,
    /// Optional ending line (inclusive, 1-based).
    #[schemars(range(min = 1))]
    #[schemars(description = "Optional ending line (inclusive, 1-based)")]
    pub end_line: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
pub struct CopyFileParams {
    /// Original file path to copy from.
    #[schemars(description = "Original file path to copy from")]
    pub source: String,
    /// Destination file path to copy into.
    #[schemars(description = "Destination file path to copy into")]
    pub destination: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
pub struct MoveFileParams {
    /// Original file path to move from.
    #[schemars(description = "Original file path to move from")]
    pub source: String,
    /// Destination file path to move into.
    #[schemars(description = "Destination file path to move into")]
    pub destination: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
pub struct DeleteFileParams {
    /// File or directory path to delete.
    #[schemars(description = "File or directory path to delete")]
    pub path: String,
    /// When true, delete directories recursively.
    #[serde(default)]
    #[schemars(default)]
    #[schemars(description = "When true, delete directories recursively")]
    pub recursive: bool,
}

// Directory Operations
#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
pub struct ListDirectoryParams {
    /// Directory path relative to the CODE_HOME namespace.
    #[schemars(description = "Directory path relative to the CODE_HOME namespace")]
    pub path: String,
    /// If true, include recursive tree listing.
    #[serde(default)]
    #[schemars(default)]
    #[schemars(description = "If true, include recursive tree listing")]
    pub recursive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
pub struct CreateDirectoryParams {
    /// Directory path to create relative to CODE_HOME.
    #[schemars(description = "Directory path to create relative to CODE_HOME")]
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
pub struct TreeParams {
    /// Directory path to inspect relative to CODE_HOME.
    #[schemars(description = "Directory path to inspect relative to CODE_HOME")]
    pub path: String,
    /// Optional maximum depth to traverse.
    #[schemars(description = "Optional maximum depth to traverse")]
    pub depth: Option<usize>,
    /// Follow symbolic links when true.
    #[serde(default)]
    #[schemars(default)]
    #[schemars(description = "Follow symbolic links when true")]
    pub follow_symlinks: bool,
}

// Search Operations
#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
pub struct SearchFilesParams {
    /// Root directory to search under relative to CODE_HOME.
    #[schemars(description = "Root directory to search under relative to CODE_HOME")]
    pub path: String,
    /// Regex pattern applied to file paths.
    #[schemars(description = "Regex pattern applied to file paths")]
    pub pattern: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
pub struct SearchWithinFilesParams {
    /// Root directory to search under relative to CODE_HOME.
    #[schemars(description = "Root directory to search under relative to CODE_HOME")]
    pub path: String,
    /// Regex pattern applied to file contents (grep-compatible syntax).
    #[schemars(description = "Regex pattern applied to file contents (grep-compatible syntax)")]
    pub pattern: String,
    /// Optional maximum directory depth to traverse during search.
    #[schemars(description = "Optional maximum directory depth to traverse during search")]
    pub depth: Option<usize>,
    /// Optional cap on the number of matches to return.
    #[schemars(description = "Optional cap on the number of matches to return")]
    pub max_results: Option<usize>,
}

// Info Operations
#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
pub struct GetFileInfoParams {
    /// File or directory path to inspect relative to CODE_HOME.
    #[schemars(description = "File or directory path to inspect relative to CODE_HOME")]
    pub path: String,
}

// Artifact-specific convenience parameters
#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
pub struct GetArtifactParams {
    /// Full URL or path pointing to the artifact.
    #[schemars(description = "Full URL or path pointing to the artifact")]
    pub url: String, // Full URL/path to the artifact
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
pub struct ListArtifactsParams {
    /// Optional namespace override (thread_id or custom namespace).
    #[schemars(description = "Optional namespace override (thread_id or custom namespace)")]
    pub namespace: Option<String>, // thread_id or custom namespace
    /// Optional maximum number of artifacts to return.
    #[schemars(description = "Optional maximum number of artifacts to return")]
    pub limit: Option<usize>,
    /// When true, include short content previews for each artifact.
    #[serde(default)]
    #[schemars(default)]
    #[schemars(description = "When true, include short content previews for each artifact")]
    pub include_preview: bool,
}

// Artifact tool with actions
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ArtifactParams {
    /// Action to perform: "list" or "read".
    #[schemars(description = "Action to perform: 'list' or 'read'")]
    pub action: String, // "list" or "read"
    /// Artifact identifier required for the "read" action.
    #[schemars(description = "Artifact identifier required for the 'read' action")]
    pub artifact_id: Option<String>, // Required for "read" action
    /// Optional starting line for partial reads (1-based).
    #[schemars(description = "Optional starting line for partial reads (1-based)")]
    pub start_line: Option<u64>, // Optional for "read" action
    /// Optional ending line for partial reads (inclusive, 1-based).
    #[schemars(description = "Optional ending line for partial reads (inclusive, 1-based)")]
    pub end_line: Option<u64>, // Optional for "read" action
}

impl Default for ArtifactParams {
    fn default() -> Self {
        Self {
            action: "list".to_string(),
            artifact_id: None,
            start_line: None,
            end_line: None,
        }
    }
}

// Response structs for proper serde serialization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInfoResponse {
    pub path: String,
    pub size: u64,
    pub is_file: bool,
    pub is_dir: bool,
    pub modified: Option<u64>,
    pub created: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileOperationResponse {
    pub success: bool,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopyMoveResponse {
    pub success: bool,
    pub source: String,
    pub destination: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadFileResponse {
    pub content: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileContentPair {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadMultipleFilesResponse {
    pub files: Vec<FileContentPair>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListDirectoryResponse {
    pub contents: Vec<String>,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchFilesResponse {
    pub results: Vec<String>,
    pub path: String,
    pub pattern: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchMatch {
    pub line: usize,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchFileResult {
    pub path: String,
    pub matches: Vec<SearchMatch>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchWithinFilesResponse {
    pub matches: Vec<SearchFileResult>,
    pub total: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetArtifactResponse {
    pub content: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
pub struct ApplyDiffParams {
    /// File path to apply the diff against relative to CODE_HOME.
    #[schemars(description = "File path to apply the diff against relative to CODE_HOME")]
    pub path: String,
    /// Diff content expressed using SEARCH/REPLACE blocks.
    #[schemars(description = "Diff content expressed using SEARCH/REPLACE blocks")]
    pub diff: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactListEntry {
    pub artifact_id: String,
    pub path: String,
    pub size: u64,
    pub preview: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactListResponse {
    pub artifacts: Vec<ArtifactListEntry>,
    pub total: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactReadResponse {
    pub artifact_id: String,
    pub content: String,
    pub start_line: u64,
    pub end_line: u64,
    pub total_lines: u64,
}

// Tool implementations

/// Read file content tool
#[derive(Debug)]
pub struct ReadFileTool {
    filesystem: Arc<dyn FileSystemOps>,
}

impl ReadFileTool {
    pub fn new(filesystem: Arc<dyn FileSystemOps>) -> Self {
        Self { filesystem }
    }
}

#[async_trait::async_trait]
impl Tool for ReadFileTool {
    fn get_name(&self) -> String {
        "fs_read_file".to_string()
    }

    fn get_description(&self) -> String {
        "Read the contents of a file".to_string()
    }

    fn get_parameters(&self) -> Value {
        let schema = schema_for!(ReadFileParams);
        serde_json::to_value(schema).unwrap_or_else(|_| json!({}))
    }

    async fn execute(
        &self,
        tool_call: distri_types::ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<distri_types::Part>, anyhow::Error> {
        let params: ReadFileParams = serde_json::from_value(tool_call.input)?;

        let read_params = distri_types::filesystem::ReadParams {
            start_line: params.start_line,
            end_line: params.end_line,
        };
        let result = self.filesystem.read(&params.path, read_params).await?;
        Ok(vec![distri_types::Part::Data(serde_json::to_value(
            result,
        )?)])
    }
}

/// Write file content tool
#[derive(Debug)]
pub struct WriteFileTool {
    filesystem: Arc<dyn FileSystemOps>,
}

impl WriteFileTool {
    pub fn new(filesystem: Arc<dyn FileSystemOps>) -> Self {
        Self { filesystem }
    }
}

#[async_trait::async_trait]
impl Tool for WriteFileTool {
    fn get_name(&self) -> String {
        "fs_write_file".to_string()
    }

    fn get_description(&self) -> String {
        "Write content to a file".to_string()
    }

    fn get_parameters(&self) -> Value {
        let schema = schema_for!(WriteFileParams);
        serde_json::to_value(schema).unwrap_or_else(|_| json!({}))
    }

    async fn execute(
        &self,
        tool_call: distri_types::ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<distri_types::Part>, anyhow::Error> {
        let params: WriteFileParams = serde_json::from_value(tool_call.input)?;

        self.filesystem.write(&params.path, &params.content).await?;
        let response = FileOperationResponse {
            success: true,
            path: params.path,
        };
        Ok(vec![distri_types::Part::Data(serde_json::to_value(
            response,
        )?)])
    }
}

/// Apply diff tool for precise modifications
#[derive(Debug)]
pub struct ApplyDiffTool {
    filesystem: Arc<dyn FileSystemOps>,
}

impl ApplyDiffTool {
    pub fn new(filesystem: Arc<dyn FileSystemOps>) -> Self {
        Self { filesystem }
    }

    fn parse_diff(&self, diff: &str) -> Result<Vec<DiffBlock>> {
        let mut blocks = Vec::new();
        let mut lines = diff.lines().peekable();

        while let Some(line) = lines.next() {
            if line.trim().is_empty() {
                continue;
            }

            if line.trim() != "<<<<<<< SEARCH" {
                return Err(anyhow::anyhow!(
                    "Expected '<<<<<<< SEARCH' marker but found '{}'",
                    line
                ));
            }

            let start_line_line = lines
                .next()
                .ok_or_else(|| anyhow::anyhow!("Missing :start_line directive in diff block"))?;

            let start_line = start_line_line
                .strip_prefix(":start_line:")
                .ok_or_else(|| anyhow::anyhow!("Invalid :start_line directive format"))?
                .trim()
                .parse::<usize>()
                .map_err(|_| anyhow::anyhow!("Invalid start line number in diff block"))?;

            let separator = lines
                .next()
                .ok_or_else(|| anyhow::anyhow!("Missing separator after start_line directive"))?;

            if separator.trim() != "-------" {
                return Err(anyhow::anyhow!(
                    "Expected '-------' separator but found '{}'",
                    separator
                ));
            }

            let mut search_lines = Vec::new();
            while let Some(next) = lines.next() {
                if next.trim() == "=======" {
                    break;
                }
                search_lines.push(next.to_string());
            }

            let mut replace_lines = Vec::new();
            while let Some(next) = lines.next() {
                if next.trim() == ">>>>>>> REPLACE" {
                    break;
                }
                replace_lines.push(next.to_string());
            }

            if replace_lines.is_empty() && search_lines.is_empty() {
                return Err(anyhow::anyhow!(
                    "Diff block must contain search or replace content"
                ));
            }

            blocks.push(DiffBlock {
                start_line,
                search: search_lines,
                replace: replace_lines,
            });
        }

        if blocks.is_empty() {
            return Err(anyhow::anyhow!("No diff blocks found"));
        }

        Ok(blocks)
    }

    fn decode_lines(raw: &str) -> Vec<String> {
        raw.lines()
            .map(|line| {
                if let Some((_, content)) = line.split_once('→') {
                    content.to_string()
                } else {
                    line.to_string()
                }
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
struct DiffBlock {
    start_line: usize,
    search: Vec<String>,
    replace: Vec<String>,
}

#[async_trait::async_trait]
impl Tool for ApplyDiffTool {
    fn get_name(&self) -> String {
        "apply_diff".to_string()
    }

    fn get_description(&self) -> String {
        "Apply targeted diffs to a file using SEARCH/REPLACE blocks".to_string()
    }

    fn get_parameters(&self) -> Value {
        let schema = schema_for!(ApplyDiffParams);
        serde_json::to_value(schema).unwrap_or_else(|_| json!({}))
    }

    async fn execute(
        &self,
        tool_call: distri_types::ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<distri_types::Part>, anyhow::Error> {
        let params: ApplyDiffParams = serde_json::from_value(tool_call.input)?;

        let blocks = self.parse_diff(&params.diff)?;

        let read_result = self
            .filesystem
            .read(
                &params.path,
                distri_types::filesystem::ReadParams::default(),
            )
            .await;

        let (mut current_lines, had_trailing_newline) = match read_result {
            Ok(result) => {
                let content = result.content;
                let lines = if result.total_lines == 0 {
                    Vec::new()
                } else {
                    Self::decode_lines(&content)
                };
                let has_newline = content.ends_with('\n');
                (lines, has_newline)
            }
            Err(err) => {
                let message = err.to_string();
                if message.contains("Failed to read file") {
                    (Vec::new(), false)
                } else {
                    return Err(err);
                }
            }
        };

        let mut offset: isize = 0;

        for block in blocks {
            if block.start_line == 0 {
                return Err(anyhow::anyhow!(
                    "start_line must be 1-based and greater than zero"
                ));
            }

            let mut target_index = block.start_line as isize - 1 + offset;
            if target_index < 0 {
                target_index = 0;
            }
            let target_index = target_index as usize;

            if target_index > current_lines.len() {
                return Err(anyhow::anyhow!(
                    "start_line {} is beyond end of file",
                    block.start_line
                ));
            }

            let search_len = block.search.len();
            let end_index = target_index + search_len;

            if search_len > 0 {
                if end_index > current_lines.len()
                    || current_lines[target_index..end_index] != block.search
                {
                    return Err(anyhow::anyhow!(
                        "Search block did not match file content at start_line {}",
                        block.start_line
                    ));
                }
                current_lines.splice(target_index..end_index, block.replace.clone());
            } else {
                current_lines.splice(target_index..target_index, block.replace.clone());
            }

            offset += block.replace.len() as isize - search_len as isize;
        }

        let mut new_content = current_lines.join("\n");
        if !current_lines.is_empty() || had_trailing_newline {
            new_content.push('\n');
        }

        self.filesystem
            .write(&params.path, &new_content)
            .await
            .with_context(|| format!("failed to write updated content to {}", params.path))?;

        Ok(vec![distri_types::Part::Data(json!({
            "status": "success",
            "path": params.path
        }))])
    }
}

/// List directory contents tool
#[derive(Debug)]
pub struct ListDirectoryTool {
    filesystem: Arc<dyn FileSystemOps>,
}

impl ListDirectoryTool {
    pub fn new(filesystem: Arc<dyn FileSystemOps>) -> Self {
        Self { filesystem }
    }
}

#[async_trait::async_trait]
impl Tool for ListDirectoryTool {
    fn get_name(&self) -> String {
        "fs_list_directory".to_string()
    }

    fn get_description(&self) -> String {
        "List the contents of a directory".to_string()
    }

    fn get_parameters(&self) -> Value {
        let schema = schema_for!(ListDirectoryParams);
        serde_json::to_value(schema).unwrap_or_else(|_| json!({}))
    }

    async fn execute(
        &self,
        tool_call: distri_types::ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<distri_types::Part>, anyhow::Error> {
        let params: ListDirectoryParams = serde_json::from_value(tool_call.input)?;

        let listing = if params.recursive {
            self.filesystem.tree(&params.path).await?
        } else {
            self.filesystem.list(&params.path).await?
        };
        let response = ListDirectoryResponse {
            contents: listing.entries.into_iter().map(|e| e.name).collect(),
            path: params.path,
        };
        Ok(vec![distri_types::Part::Data(serde_json::to_value(
            response,
        )?)])
    }
}

/// Get file info tool
#[derive(Debug)]
pub struct GetFileInfoTool {
    filesystem: Arc<dyn FileSystemOps>,
}

impl GetFileInfoTool {
    pub fn new(filesystem: Arc<dyn FileSystemOps>) -> Self {
        Self { filesystem }
    }
}

#[async_trait::async_trait]
impl Tool for GetFileInfoTool {
    fn get_name(&self) -> String {
        "fs_get_file_info".to_string()
    }

    fn get_description(&self) -> String {
        "Get information about a file or directory".to_string()
    }

    fn get_parameters(&self) -> Value {
        let schema = schema_for!(GetFileInfoParams);
        serde_json::to_value(schema).unwrap_or_else(|_| json!({}))
    }

    async fn execute(
        &self,
        tool_call: distri_types::ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<distri_types::Part>, anyhow::Error> {
        let params: GetFileInfoParams = serde_json::from_value(tool_call.input)?;

        let info = self.filesystem.info(&params.path).await?;
        Ok(vec![distri_types::Part::Data(serde_json::to_value(info)?)])
    }
}

/// Search files tool
#[derive(Debug)]
pub struct SearchFilesTool {
    filesystem: Arc<dyn FileSystemOps>,
}

impl SearchFilesTool {
    pub fn new(filesystem: Arc<dyn FileSystemOps>) -> Self {
        Self { filesystem }
    }
}

#[async_trait::async_trait]
impl Tool for SearchFilesTool {
    fn get_name(&self) -> String {
        "fs_search_files".to_string()
    }

    fn get_description(&self) -> String {
        "Search for files matching a grep-compatible regex pattern".to_string()
    }

    fn get_parameters(&self) -> Value {
        let schema = schema_for!(SearchFilesParams);
        serde_json::to_value(schema).unwrap_or_else(|_| json!({}))
    }

    async fn execute(
        &self,
        tool_call: distri_types::ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<distri_types::Part>, anyhow::Error> {
        let params: SearchFilesParams = serde_json::from_value(tool_call.input)?;

        let search_result = self
            .filesystem
            .search(&params.path, Some(&params.pattern), None)
            .await?;
        let results: Vec<String> = search_result
            .matches
            .into_iter()
            .map(|m| m.file_path)
            .collect();
        let response = SearchFilesResponse {
            results,
            path: params.path,
            pattern: params.pattern,
        };
        Ok(vec![distri_types::Part::Data(serde_json::to_value(
            response,
        )?)])
    }
}

/// Search within file contents tool
#[derive(Debug)]
pub struct SearchWithinFilesTool {
    filesystem: Arc<dyn FileSystemOps>,
}

impl SearchWithinFilesTool {
    pub fn new(filesystem: Arc<dyn FileSystemOps>) -> Self {
        Self { filesystem }
    }
}

#[async_trait::async_trait]
impl Tool for SearchWithinFilesTool {
    fn get_name(&self) -> String {
        "fs_search_within_files".to_string()
    }

    fn get_description(&self) -> String {
        "Search for text within file contents using grep-compatible regex patterns".to_string()
    }

    fn get_parameters(&self) -> Value {
        let schema = schema_for!(SearchWithinFilesParams);
        serde_json::to_value(schema).unwrap_or_else(|_| json!({}))
    }

    async fn execute(
        &self,
        tool_call: distri_types::ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<distri_types::Part>, anyhow::Error> {
        let params: SearchWithinFilesParams = serde_json::from_value(tool_call.input)?;

        let search_result = self
            .filesystem
            .search(&params.path, Some(&params.pattern), None)
            .await?;

        let matches: Vec<serde_json::Value> = search_result
            .matches
            .into_iter()
            .map(|m| {
                let mut match_obj = serde_json::Map::new();
                match_obj.insert("path".to_string(), serde_json::Value::String(m.file_path));
                if let Some(line_num) = m.line_number {
                    match_obj.insert(
                        "line_number".to_string(),
                        serde_json::Value::Number(serde_json::Number::from(line_num)),
                    );
                }
                if !m.line_content.is_empty() {
                    match_obj.insert(
                        "line_content".to_string(),
                        serde_json::Value::String(m.line_content.clone()),
                    );
                }
                serde_json::Value::Object(match_obj)
            })
            .collect();

        let mut result_obj = serde_json::Map::new();
        result_obj.insert("matches".to_string(), serde_json::Value::Array(matches));
        let results = serde_json::Value::Object(result_obj);
        Ok(vec![distri_types::Part::Data(results)])
    }
}

/// Copy file tool
#[derive(Debug)]
pub struct CopyFileTool {
    filesystem: Arc<dyn FileSystemOps>,
}

impl CopyFileTool {
    pub fn new(filesystem: Arc<dyn FileSystemOps>) -> Self {
        Self { filesystem }
    }
}

#[async_trait::async_trait]
impl Tool for CopyFileTool {
    fn get_name(&self) -> String {
        "fs_copy_file".to_string()
    }

    fn get_description(&self) -> String {
        "Copy a file from source to destination".to_string()
    }

    fn get_parameters(&self) -> Value {
        let schema = schema_for!(CopyFileParams);
        serde_json::to_value(schema).unwrap_or_else(|_| json!({}))
    }

    async fn execute(
        &self,
        tool_call: distri_types::ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<distri_types::Part>, anyhow::Error> {
        let params: CopyFileParams = serde_json::from_value(tool_call.input)?;

        self.filesystem
            .copy(&params.source, &params.destination)
            .await?;
        let response = CopyMoveResponse {
            success: true,
            source: params.source,
            destination: params.destination,
        };
        Ok(vec![distri_types::Part::Data(serde_json::to_value(
            response,
        )?)])
    }
}

/// Move file tool
#[derive(Debug)]
pub struct MoveFileTool {
    filesystem: Arc<dyn FileSystemOps>,
}

impl MoveFileTool {
    pub fn new(filesystem: Arc<dyn FileSystemOps>) -> Self {
        Self { filesystem }
    }
}

#[async_trait::async_trait]
impl Tool for MoveFileTool {
    fn get_name(&self) -> String {
        "fs_move_file".to_string()
    }

    fn get_description(&self) -> String {
        "Move a file from source to destination".to_string()
    }

    fn get_parameters(&self) -> Value {
        let schema = schema_for!(MoveFileParams);
        serde_json::to_value(schema).unwrap_or_else(|_| json!({}))
    }

    async fn execute(
        &self,
        tool_call: distri_types::ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<distri_types::Part>, anyhow::Error> {
        let params: MoveFileParams = serde_json::from_value(tool_call.input)?;

        self.filesystem
            .move_file(&params.source, &params.destination)
            .await?;
        let response = CopyMoveResponse {
            success: true,
            source: params.source,
            destination: params.destination,
        };
        Ok(vec![distri_types::Part::Data(serde_json::to_value(
            response,
        )?)])
    }
}

/// Delete file tool
#[derive(Debug)]
pub struct DeleteFileTool {
    filesystem: Arc<dyn FileSystemOps>,
}

impl DeleteFileTool {
    pub fn new(filesystem: Arc<dyn FileSystemOps>) -> Self {
        Self { filesystem }
    }
}

#[async_trait::async_trait]
impl Tool for DeleteFileTool {
    fn get_name(&self) -> String {
        "fs_delete_file".to_string()
    }

    fn get_description(&self) -> String {
        "Delete a file or directory".to_string()
    }

    fn get_parameters(&self) -> Value {
        let schema = schema_for!(DeleteFileParams);
        serde_json::to_value(schema).unwrap_or_else(|_| json!({}))
    }

    async fn execute(
        &self,
        tool_call: distri_types::ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<distri_types::Part>, anyhow::Error> {
        let params: DeleteFileParams = serde_json::from_value(tool_call.input)?;
        self.filesystem
            .delete(&params.path, params.recursive)
            .await?;
        let response = FileOperationResponse {
            success: true,
            path: params.path,
        };
        Ok(vec![distri_types::Part::Data(serde_json::to_value(
            response,
        )?)])
    }
}

/// Create directory tool
#[derive(Debug)]
pub struct CreateDirectoryTool {
    filesystem: Arc<dyn FileSystemOps>,
}

impl CreateDirectoryTool {
    pub fn new(filesystem: Arc<dyn FileSystemOps>) -> Self {
        Self { filesystem }
    }
}

#[async_trait::async_trait]
impl Tool for CreateDirectoryTool {
    fn get_name(&self) -> String {
        "fs_create_directory".to_string()
    }

    fn get_description(&self) -> String {
        "Create a directory".to_string()
    }

    fn get_parameters(&self) -> Value {
        let schema = schema_for!(CreateDirectoryParams);
        serde_json::to_value(schema).unwrap_or_else(|_| json!({}))
    }

    async fn execute(
        &self,
        tool_call: distri_types::ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<distri_types::Part>, anyhow::Error> {
        let params: CreateDirectoryParams = serde_json::from_value(tool_call.input)?;
        self.filesystem.mkdir(&params.path).await?;
        let response = FileOperationResponse {
            success: true,
            path: params.path,
        };
        Ok(vec![distri_types::Part::Data(serde_json::to_value(
            response,
        )?)])
    }
}

/// Get directory tree tool
#[derive(Debug)]
pub struct TreeTool {
    filesystem: Arc<dyn FileSystemOps>,
}

impl TreeTool {
    pub fn new(filesystem: Arc<dyn FileSystemOps>) -> Self {
        Self { filesystem }
    }
}

#[async_trait::async_trait]
impl Tool for TreeTool {
    fn get_name(&self) -> String {
        "fs_tree".to_string()
    }

    fn get_description(&self) -> String {
        "Get directory tree structure".to_string()
    }

    fn get_parameters(&self) -> Value {
        let schema = schema_for!(TreeParams);
        serde_json::to_value(schema).unwrap_or_else(|_| json!({}))
    }

    async fn execute(
        &self,
        tool_call: distri_types::ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<distri_types::Part>, anyhow::Error> {
        let params: TreeParams = serde_json::from_value(tool_call.input)?;
        let tree = self.filesystem.tree(&params.path).await?;
        Ok(vec![distri_types::Part::Data(serde_json::to_value(tree)?)])
    }
}

/// Core filesystem tools (excluding artifact helpers)
pub fn create_core_filesystem_tools(filesystem: Arc<dyn FileSystemOps>) -> Vec<Arc<dyn Tool>> {
    let tools = vec![
        Arc::new(ReadFileTool::new(filesystem.clone())) as Arc<dyn Tool>,
        Arc::new(WriteFileTool::new(filesystem.clone())) as Arc<dyn Tool>,
        Arc::new(ApplyDiffTool::new(filesystem.clone())) as Arc<dyn Tool>,
        Arc::new(ListDirectoryTool::new(filesystem.clone())) as Arc<dyn Tool>,
        Arc::new(GetFileInfoTool::new(filesystem.clone())) as Arc<dyn Tool>,
        Arc::new(SearchFilesTool::new(filesystem.clone())) as Arc<dyn Tool>,
        Arc::new(CopyFileTool::new(filesystem.clone())) as Arc<dyn Tool>,
        Arc::new(MoveFileTool::new(filesystem.clone())) as Arc<dyn Tool>,
        Arc::new(DeleteFileTool::new(filesystem.clone())) as Arc<dyn Tool>,
        Arc::new(CreateDirectoryTool::new(filesystem.clone())) as Arc<dyn Tool>,
        Arc::new(TreeTool::new(filesystem.clone())) as Arc<dyn Tool>,
        Arc::new(SearchWithinFilesTool::new(filesystem.clone())) as Arc<dyn Tool>,
    ];

    tools
}

/// Factory function to create all filesystem tools (including artifact helpers)
pub fn create_filesystem_tools(filesystem: Arc<dyn FileSystemOps>) -> Vec<Arc<dyn Tool>> {
    let mut tools = create_core_filesystem_tools(filesystem.clone());
    tools.extend(crate::create_artifact_tools(filesystem));

    tools
}
