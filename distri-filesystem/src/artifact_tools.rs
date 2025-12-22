use crate::ArtifactWrapper;
use anyhow::Result;
use distri_types::{filesystem::FileSystemOps, Tool, ToolContext};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

// Base path metadata configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactBasePath {
    pub base_path: String,
}

impl ArtifactBasePath {
    /// Extract base_path from ToolContext metadata with fallback to computed path
    pub fn from_context(context: &ToolContext) -> Option<String> {
        let base_path: Option<&str> = context
            .metadata
            .as_ref()
            .map(|m| m.get("artifact_base_path"))
            .flatten()
            .map(|m| m.as_str())
            .flatten();
        if let Some(v) = base_path {
            tracing::info!("✅ Using artifact_base_path from metadata: {}", v);
            return Some(v.to_string());
        } else {
            // If not injected from parent, you can use your task namespace
            let artifact_base_path =
                crate::ArtifactWrapper::task_namespace(&context.thread_id, &context.task_id);
            tracing::info!(
                "No metadata provided, using computed path: {}",
                artifact_base_path
            );
            return Some(artifact_base_path);
        }
    }
}

// Artifact-specific parameter types
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ListArtifactsParams {
    pub namespace: Option<String>, // thread_id or custom namespace
    pub limit: Option<usize>,
    #[serde(default)]
    pub include_preview: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReadArtifactParams {
    pub filename: String,
    pub start_line: Option<u64>,
    pub end_line: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SearchArtifactsParams {
    pub pattern: String, // Search pattern to find within artifact content
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DeleteArtifactParams {
    pub filename: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SaveArtifactParams {
    pub filename: String,
    pub content: String,
}

// Artifact response types
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

/// List available artifacts
#[derive(Debug)]
pub struct ListArtifactsTool {
    filesystem: Arc<dyn FileSystemOps>,
}

impl ListArtifactsTool {
    pub fn new(filesystem: Arc<dyn FileSystemOps>) -> Self {
        Self { filesystem }
    }
}

#[async_trait::async_trait]
impl Tool for ListArtifactsTool {
    fn get_name(&self) -> String {
        "list_artifacts".to_string()
    }

    fn get_description(&self) -> String {
        "List all available artifacts in the current task".to_string()
    }

    fn get_parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "namespace": {
                    "type": "string",
                    "description": "Optional namespace override (thread_id or custom namespace)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Optional maximum number of artifacts to return"
                },
                "include_preview": {
                    "type": "boolean",
                    "description": "When true, include short content previews for each artifact",
                    "default": false
                }
            }
        })
    }

    async fn execute(
        &self,
        _tool_call: distri_types::ToolCall,
        context: Arc<ToolContext>,
    ) -> Result<Vec<distri_types::Part>, anyhow::Error> {
        // Use ArtifactNamespace to get thread and task paths
        let namespace = distri_types::ArtifactNamespace::new(
            context.thread_id.clone(),
            Some(context.task_id.clone()),
        );
        
        // Check thread level first, then task level
        let thread_path = namespace.thread_path();
        let paths_to_check = if let Some(task_path) = namespace.task_path() {
            vec![thread_path, task_path]
        } else {
            vec![thread_path]
        };
        
        tracing::info!(
            "🔍 ListArtifactsTool: thread_id={}, task_id={}, checking paths: {:?}",
            context.thread_id,
            context.task_id,
            paths_to_check
        );
        
        // Check all paths and merge results
        let mut all_artifacts = Vec::new();
        let mut seen_filenames = std::collections::HashSet::new();
        
        for path in paths_to_check {
            if let Ok(wrapper) = ArtifactWrapper::new(self.filesystem.clone(), path.clone()).await {
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
        
        tracing::info!("✅ ListArtifactsTool: Found {} artifacts", all_artifacts.len());
        Ok(vec![distri_types::Part::Data(serde_json::to_value(
            all_artifacts,
        )?)])
    }
}

/// Read specific artifact content
#[derive(Debug)]
pub struct ReadArtifactTool {
    filesystem: Arc<dyn FileSystemOps>,
}

impl ReadArtifactTool {
    pub fn new(filesystem: Arc<dyn FileSystemOps>) -> Self {
        Self { filesystem }
    }
}

#[async_trait::async_trait]
impl Tool for ReadArtifactTool {
    fn get_name(&self) -> String {
        "read_artifact".to_string()
    }

    fn get_description(&self) -> String {
        "Read the content of a specific artifact by filename (including extension)".to_string()
    }

    fn get_parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "filename": {
                    "type": "string",
                    "description": "Filename of the artifact to read, including extension"
                },
                "start_line": {
                    "type": "integer",
                    "description": "Optional starting line (1-based)",
                    "minimum": 1
                },
                "end_line": {
                    "type": "integer",
                    "description": "Optional ending line (inclusive, 1-based)",
                    "minimum": 1
                }
            },
            "required": ["filename"]
        })
    }

    async fn execute(
        &self,
        tool_call: distri_types::ToolCall,
        context: Arc<ToolContext>,
    ) -> Result<Vec<distri_types::Part>, anyhow::Error> {
        let params: ReadArtifactParams = serde_json::from_value(tool_call.input)?;
        
        // Use ArtifactNamespace to get thread and task paths
        let namespace = distri_types::ArtifactNamespace::new(
            context.thread_id.clone(),
            Some(context.task_id.clone()),
        );
        
        // Check thread level first, then task level
        let thread_path = namespace.thread_path();
        let paths_to_check = if let Some(task_path) = namespace.task_path() {
            vec![thread_path, task_path]
        } else {
            vec![thread_path]
        };
        
        tracing::info!(
            "🔍 ReadArtifactTool: thread_id={}, task_id={}, filename={}, checking paths: {:?}",
            context.thread_id,
            context.task_id,
            params.filename,
            paths_to_check
        );
        
        // Try each path until we find the artifact
        let mut last_error = None;
        for path in paths_to_check {
            if let Ok(wrapper) = ArtifactWrapper::new(self.filesystem.clone(), path.clone()).await {
                match wrapper.read_artifact(&params.filename, params.start_line, params.end_line).await {
                    Ok(result) => {
                        tracing::info!("✅ ReadArtifactTool: Found artifact at path: {}", path);
                        return Ok(vec![distri_types::Part::Data(serde_json::to_value(
                            result,
                        )?)]);
                    }
                    Err(e) => {
                        last_error = Some(e);
                        continue;
                    }
                }
            }
        }
        
        // If we get here, artifact wasn't found in any path
        Err(last_error.unwrap_or_else(|| anyhow::anyhow!(
            "Artifact '{}' not found in any namespace path",
            params.filename
        )))
    }
}

/// Search within artifact contents
#[derive(Debug)]
pub struct SearchArtifactsTool {
    filesystem: Arc<dyn FileSystemOps>,
}

impl SearchArtifactsTool {
    pub fn new(filesystem: Arc<dyn FileSystemOps>) -> Self {
        Self { filesystem }
    }
}

#[async_trait::async_trait]
impl Tool for SearchArtifactsTool {
    fn get_name(&self) -> String {
        "search_artifacts".to_string()
    }

    fn get_description(&self) -> String {
        "Search for text patterns within all available artifacts".to_string()
    }

    fn get_parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Search pattern to find within artifact content"
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(
        &self,
        tool_call: distri_types::ToolCall,
        context: Arc<ToolContext>,
    ) -> Result<Vec<distri_types::Part>, anyhow::Error> {
        let params: SearchArtifactsParams = serde_json::from_value(tool_call.input)?;
        let base_path = ArtifactBasePath::from_context(&context)
            .ok_or(anyhow::anyhow!("artifact_base_path is empty in metadata"))?;
        let wrapper = ArtifactWrapper::new(self.filesystem.clone(), base_path).await?;
        let result = wrapper.search_artifacts(&params.pattern).await?;
        Ok(vec![distri_types::Part::Data(serde_json::to_value(
            result,
        )?)])
    }
}

/// Delete specific artifact
#[derive(Debug)]
pub struct DeleteArtifactTool {
    filesystem: Arc<dyn FileSystemOps>,
}

impl DeleteArtifactTool {
    pub fn new(filesystem: Arc<dyn FileSystemOps>) -> Self {
        Self { filesystem }
    }
}

#[async_trait::async_trait]
impl Tool for DeleteArtifactTool {
    fn get_name(&self) -> String {
        "delete_artifact".to_string()
    }

    fn get_description(&self) -> String {
        "Delete a specific artifact by ID".to_string()
    }

    fn get_parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "filename": {
                    "type": "string",
                    "description": "Filename of the artifact to delete"
                }
            },
            "required": ["filename"]
        })
    }

    async fn execute(
        &self,
        tool_call: distri_types::ToolCall,
        context: Arc<ToolContext>,
    ) -> Result<Vec<distri_types::Part>, anyhow::Error> {
        let params: DeleteArtifactParams = serde_json::from_value(tool_call.input)?;
        let base_path = ArtifactBasePath::from_context(&context)
            .ok_or(anyhow::anyhow!("artifact_base_path is empty in metadata"))?;
        let wrapper = ArtifactWrapper::new(self.filesystem.clone(), base_path).await?;
        wrapper.cleanup_task_folder().await?;
        Ok(vec![distri_types::Part::Data(
            serde_json::json!({"success": true, "filename": params.filename}),
        )])
    }
}

/// Save content as artifact
#[derive(Debug)]
pub struct SaveArtifactTool {
    filesystem: Arc<dyn FileSystemOps>,
}

impl SaveArtifactTool {
    pub fn new(filesystem: Arc<dyn FileSystemOps>) -> Self {
        Self { filesystem }
    }
}

#[async_trait::async_trait]
impl Tool for SaveArtifactTool {
    fn get_name(&self) -> String {
        "save_artifact".to_string()
    }

    fn get_description(&self) -> String {
        "Save content as an artifact with specified filename".to_string()
    }

    fn get_parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "filename": {
                    "type": "string",
                    "description": "Filename (including extension) to save the artifact as"
                },
                "content": {
                    "type": "string",
                    "description": "Content to store in the artifact"
                }
            },
            "required": ["filename", "content"]
        })
    }

    async fn execute(
        &self,
        tool_call: distri_types::ToolCall,
        context: Arc<ToolContext>,
    ) -> Result<Vec<distri_types::Part>, anyhow::Error> {
        let params: SaveArtifactParams = serde_json::from_value(tool_call.input)?;
        let base_path = ArtifactBasePath::from_context(&context)
            .ok_or(anyhow::anyhow!("artifact_base_path is empty in metadata"))?;
        tracing::info!(
            "🔍 SaveArtifactTool: thread_id={}, task_id={}, base_path={}",
            context.thread_id,
            context.task_id,
            base_path
        );
        let wrapper = ArtifactWrapper::new(self.filesystem.clone(), base_path).await?;

        tracing::info!(
            "💾 SaveArtifactTool: Saving filename={}, content_len={}",
            params.filename,
            params.content.len()
        );

        wrapper
            .save_artifact(&params.filename, &params.content)
            .await?;
        tracing::info!("✅ SaveArtifactTool: Successfully saved artifact");
        Ok(vec![distri_types::Part::Data(serde_json::json!({
            "success": true,
            "filename": params.filename
        }))])
    }
}

/// Factory function to create all artifact tools
pub fn create_artifact_tools(filesystem: Arc<dyn FileSystemOps>) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(ListArtifactsTool::new(filesystem.clone())) as Arc<dyn Tool>,
        Arc::new(ReadArtifactTool::new(filesystem.clone())) as Arc<dyn Tool>,
        Arc::new(SearchArtifactsTool::new(filesystem.clone())) as Arc<dyn Tool>,
        Arc::new(DeleteArtifactTool::new(filesystem.clone())) as Arc<dyn Tool>,
        Arc::new(SaveArtifactTool::new(filesystem)) as Arc<dyn Tool>,
    ]
}
