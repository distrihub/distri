use anyhow::Context;
use chrono;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{self, Value, json};
use std::default::Default;
use std::{collections::HashMap, time::SystemTime};
use uuid;

use crate::filesystem::FileMetadata;

use crate::events::AgentEventType;

/// External tool that delegates execution to the frontend
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ExternalTool {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    #[serde(default)]
    pub is_final: bool,
}

#[async_trait::async_trait]
impl crate::Tool for ExternalTool {
    fn get_name(&self) -> String {
        self.name.clone()
    }

    fn get_description(&self) -> String {
        self.description.clone()
    }

    fn get_parameters(&self) -> serde_json::Value {
        self.parameters.clone()
    }

    fn is_external(&self) -> bool {
        true // ExternalTool is always external by definition
    }

    fn is_final(&self) -> bool {
        self.is_final
    }

    async fn execute(
        &self,
        _tool_call: crate::ToolCall,
        _context: std::sync::Arc<crate::ToolContext>,
    ) -> Result<Vec<crate::Part>, anyhow::Error> {
        // External tools are handled by the frontend, not executed in Rust
        Err(anyhow::anyhow!(
            "External tools cannot be executed directly in Rust"
        ))
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    pub examples: Option<String>,
    pub output_schema: Option<serde_json::Value>,
}

impl From<ToolDefinition> for async_openai::types::chat::ChatCompletionTools {
    fn from(definition: ToolDefinition) -> Self {
        async_openai::types::chat::ChatCompletionTools::Function(
            async_openai::types::chat::ChatCompletionTool {
                function: async_openai::types::chat::FunctionObject {
                    name: definition.name,
                    description: Some(definition.description),
                    parameters: Some(definition.parameters),
                    strict: None,
                },
            },
        )
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    /// Represents a system message.
    System,
    /// Represents a message from the assistant.
    Assistant,
    /// Represents a message from the user.
    User,
    /// Represents a message from a tool.
    Tool,
    /// Represents a developer message for adding context.
    /// Maps to "developer" for OpenAI, "user" for other providers.
    /// Hidden in UI by default, shown only in debug mode.
    Developer,
}

impl From<async_openai::types::chat::Role> for MessageRole {
    fn from(role: async_openai::types::chat::Role) -> Self {
        match role {
            async_openai::types::chat::Role::User => MessageRole::User,
            async_openai::types::chat::Role::Assistant => MessageRole::Assistant,
            async_openai::types::chat::Role::System => MessageRole::System,
            async_openai::types::chat::Role::Tool => MessageRole::Tool,
            // Note: Developer role is handled via catch-all since async_openai
            // may not have the Developer variant yet
            _ => MessageRole::Assistant,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case", tag = "part_type", content = "data")]
pub enum Part {
    Text(String),
    ToolCall(ToolCall),
    ToolResult(ToolResponse),
    Image(FileType),
    Data(Value),
    /// Artifact stored in filesystem - reference + metadata for large content
    Artifact(FileMetadata),
}

impl Part {
    pub fn type_name(&self) -> String {
        match self {
            Part::Text(_) => "text".to_string(),
            Part::ToolCall(_) => "tool_call".to_string(),
            Part::ToolResult(_) => "tool_result".to_string(),
            Part::Image(_) => "image".to_string(),
            Part::Data(_) => "data".to_string(),
            Part::Artifact(_) => "artifact".to_string(),
        }
    }
}

/// Instruction for how to handle additional parts
#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AdditionalPartsInstruction {
    /// Replace existing additional parts with new ones
    Replace,
    /// Append new parts to existing ones
    Append,
}

impl Default for AdditionalPartsInstruction {
    fn default() -> Self {
        AdditionalPartsInstruction::Replace
    }
}

/// Structure for managing additional user message parts
/// This allows control over how parts are added and whether artifacts should be expanded
#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema, Default)]
pub struct AdditionalParts {
    /// The parts to include in the user message
    pub parts: Vec<Part>,
    /// Whether to replace or append to existing parts
    #[serde(default)]
    pub instruction: AdditionalPartsInstruction,
    /// If true, artifacts will be expanded to their actual content (e.g., image artifacts become Part::Image)
    #[serde(default)]
    pub include_artifacts: bool,
}

/// Metadata for individual message parts.
/// Used to control part behavior such as persistence.
#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema, Default)]
pub struct PartMetadata {
    /// If false, this part will be filtered out before saving to the database.
    /// Useful for ephemeral/dynamic content that should only be sent in the current turn.
    /// Defaults to true.
    #[serde(default = "default_save")]
    pub save: bool,
}

fn default_save() -> bool {
    true
}

/// Mapping of part indices to their metadata.
/// Used in message metadata to specify per-part behavior.
pub type PartsMetadata = std::collections::HashMap<usize, PartMetadata>;

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
pub struct Message {
    pub id: String,
    pub name: Option<String>,
    pub role: MessageRole,
    pub parts: Vec<Part>,
    pub created_at: i64,
    /// The ID of the agent that generated this message (for Assistant messages)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Per-part metadata used to control part behavior during save.
    /// Parts with `save: false` will be filtered out before saving to the database.
    /// This field is used for processing only and is not persisted.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub parts_metadata: Option<PartsMetadata>,
}

impl Default for Message {
    fn default() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            role: MessageRole::User,
            name: None,
            parts: vec![],
            created_at: chrono::Utc::now().timestamp_millis(),
            agent_id: None,
            parts_metadata: None,
        }
    }
}
impl Message {
    pub fn user(task: String, name: Option<String>) -> Self {
        Self {
            role: MessageRole::User,
            name,
            parts: vec![Part::Text(task)],
            ..Default::default()
        }
    }
    pub fn system(task: String, name: Option<String>) -> Self {
        Self {
            role: MessageRole::System,
            name,
            parts: vec![Part::Text(task)],
            ..Default::default()
        }
    }

    pub fn assistant(task: String, name: Option<String>) -> Self {
        Self {
            role: MessageRole::Assistant,
            name,
            parts: vec![Part::Text(task)],
            ..Default::default()
        }
    }

    pub fn developer(task: String, name: Option<String>) -> Self {
        Self {
            role: MessageRole::Developer,
            name,
            parts: vec![Part::Text(task)],
            ..Default::default()
        }
    }
    pub fn tool_response(
        tool_call_id: String,
        tool_name: String,
        result: &serde_json::Value,
    ) -> Self {
        Self {
            parts: vec![Part::ToolResult(ToolResponse::direct(
                tool_call_id,
                tool_name,
                result.clone(),
            ))],
            ..Default::default()
        }
    }

    pub fn as_text(&self) -> Option<String> {
        let parts = self
            .parts
            .iter()
            .filter_map(|part| match part {
                Part::Text(text) => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        if !parts.is_empty() {
            Some(parts.join("\n\n"))
        } else {
            None
        }
    }

    pub fn tool_calls(&self) -> Vec<ToolCall> {
        let mut tool_calls = vec![];
        for part in self.parts.iter() {
            if let Part::ToolCall(tool_call) = part {
                tool_calls.push(tool_call.clone());
            }
        }
        tool_calls
    }

    pub fn tool_responses(&self) -> Vec<ToolResponse> {
        let mut tool_responses = vec![];
        for part in self.parts.iter() {
            if let Part::ToolResult(tool_response) = part {
                tool_responses.push(tool_response.clone());
            }
        }
        tool_responses
    }

    pub fn has_tool_response(&self) -> bool {
        self.parts
            .iter()
            .any(|part| matches!(part, Part::ToolResult(_)))
    }

    /// Filter parts based on metadata, returning a new Message with only saveable parts.
    /// Parts with `save: false` in the parts_metadata will be filtered out.
    /// If parts_metadata is None, all parts are included.
    pub fn filter_for_save(&self, parts_metadata: Option<&PartsMetadata>) -> Self {
        let parts_metadata = match parts_metadata {
            Some(meta) => meta,
            None => return self.clone(),
        };

        let filtered_parts: Vec<Part> = self
            .parts
            .iter()
            .enumerate()
            .filter(|(index, _)| {
                parts_metadata
                    .get(index)
                    .map(|meta| meta.save)
                    .unwrap_or(true) // Default to save=true if not specified
            })
            .map(|(_, part)| part.clone())
            .collect();

        Self {
            id: self.id.clone(),
            name: self.name.clone(),
            role: self.role.clone(),
            parts: filtered_parts,
            created_at: self.created_at,
            agent_id: self.agent_id.clone(),
            parts_metadata: None, // Don't persist parts_metadata
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum TaskMessage {
    Message(Message),
    Event(TaskEvent),
}

impl TaskMessage {
    pub fn created_at(&self) -> i64 {
        match self {
            TaskMessage::Message(message) => message.created_at,
            TaskMessage::Event(event) => event.created_at.clone(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema, Default)]
#[serde(rename_all = "lowercase")]
pub enum ToolCallStatus {
    #[default]
    Pending,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TaskEvent {
    pub event: AgentEventType,
    pub created_at: i64,
    pub is_final: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentPlan {
    pub steps: Vec<PlanStep>,
    pub reasoning: Option<String>,
}
impl AgentPlan {
    pub fn new(steps: Vec<PlanStep>) -> Self {
        Self {
            steps,
            reasoning: None,
        }
    }
}

/// Plan step for execution
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct PlanStep {
    pub id: String,
    pub thought: Option<String>,
    pub action: Action,
}

/// Action can be either a tool call or an LLM call
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Action {
    ToolCalls { tool_calls: Vec<ToolCall> },
    Code { code: String, language: String },
}

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema, PartialEq)]
pub struct ToolResponse {
    pub tool_call_id: String,
    pub tool_name: String,
    /// Content as parts - automatically converts large content to Part::Artifact
    pub parts: Vec<Part>,
}

impl ToolResponse {
    /// Create a ToolResponse with direct content (simple parts, no conversion)
    pub fn direct(tool_call_id: String, tool_name: String, result: serde_json::Value) -> Self {
        Self {
            tool_call_id,
            tool_name,
            parts: vec![Part::Data(result)],
        }
    }

    /// Create a ToolResponse from parts (allows for pre-converted artifacts)
    pub fn from_parts(tool_call_id: String, tool_name: String, parts: Vec<Part>) -> Self {
        Self {
            tool_call_id,
            tool_name,
            parts,
        }
    }

    /// Get all artifacts from this response
    pub fn get_artifacts(&self) -> Vec<&FileMetadata> {
        self.parts
            .iter()
            .filter_map(|part| {
                if let Part::Artifact(artifact) = part {
                    Some(artifact)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Legacy method for backward compatibility - will be deprecated
    pub fn result(&self) -> serde_json::Value {
        // If there's only one part, return it directly for backward compatibility
        if self.parts.len() == 1 {
            match &self.parts[0] {
                Part::Text(text) => serde_json::Value::String(text.clone()),
                Part::Data(data) => data.clone(),
                Part::Artifact(artifact) => {
                    // Return artifact metadata for backward compatibility
                    serde_json::json!({
                        "artifact_reference": {
                            "file_id": artifact.file_id,
                            "size": artifact.size,
                            "preview": artifact.preview,
                            "summary": artifact.summary()
                        }
                    })
                }
                _ => serde_json::json!({"unsupported_part_type": "legacy_method_deprecated"}),
            }
        } else {
            // Multiple parts - return as array
            serde_json::json!(self.parts)
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema, Default)]
pub struct Task {
    pub id: String,
    pub thread_id: String,
    pub parent_task_id: Option<String>,
    pub status: TaskStatus,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema, PartialEq, Default)]
pub enum TaskStatus {
    #[default]
    Pending,
    Running,
    InputRequired,
    Completed,
    Failed,
    Canceled,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpSession {
    /// The token for the MCP session.
    pub token: String,
    /// The expiry time of the session, if specified.
    pub expiry: Option<SystemTime>,
}

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ToolCall {
    pub tool_call_id: String,
    pub tool_name: String,
    pub input: Value,
}

pub fn validate_parameters(
    schema: &mut serde_json::Value,
    params: Option<serde_json::Value>,
) -> anyhow::Result<()> {
    if schema.is_null() {
        return Ok(());
    }

    let params = params.unwrap_or(serde_json::Value::Null);
    let obj = schema
        .as_object_mut()
        .context("parameters must be an object")?;

    // Add type: "object" if not present
    if !obj.contains_key("type") {
        obj.insert("type".to_string(), json!("object"));
    } else if obj["type"].as_str().unwrap_or_default() != "object" {
        return Err(anyhow::anyhow!("type must be an object",));
    }

    // Add required: [] if not present
    if !obj.contains_key("required") {
        obj.insert("required".to_string(), json!([]));
    }

    let validator = jsonschema::validator_for(schema)?;

    validator
        .validate(&params)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thread {
    pub id: String,
    pub title: String,
    pub agent_id: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub message_count: u32,
    pub last_message: Option<String>,
    pub metadata: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub attributes: serde_json::Value,
    pub user_id: Option<String>,
    pub external_id: Option<String>,
}

impl Thread {
    pub fn new(
        agent_id: String,
        title: Option<String>,
        thread_id: Option<String>,
        user_id: Option<String>,
        external_id: Option<String>,
    ) -> Self {
        let now = chrono::Utc::now();
        Self {
            id: thread_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            title: title.unwrap_or_else(|| "New conversation".to_string()),
            agent_id,
            created_at: now,
            updated_at: now,
            message_count: 0,
            last_message: None,
            metadata: HashMap::new(),
            attributes: serde_json::Value::Null,
            user_id,
            external_id,
        }
    }

    pub fn update_with_message(&mut self, message: &str) {
        self.updated_at = chrono::Utc::now();
        self.message_count += 1;
        self.last_message = Some(message.chars().take(100).collect());

        // Auto-generate title from first message if it's still default
        if self.title == "New conversation" && self.message_count == 1 {
            self.title = message
                .chars()
                .take(50)
                .collect::<String>()
                .trim()
                .to_string();
            if self.title.is_empty() {
                self.title = "Untitled conversation".to_string();
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadSummary {
    pub id: String,
    pub title: String,
    pub agent_id: String,
    pub agent_name: String,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub message_count: u32,
    pub last_message: Option<String>,
    pub user_id: Option<String>,
    pub external_id: Option<String>,
    /// Tags extracted from thread attributes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

// CreateThreadRequest removed - threads are now auto-created from first messages
// Thread creation is handled internally when a message is sent with a context_id
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateThreadRequest {
    pub agent_id: String,
    pub title: Option<String>,
    pub thread_id: Option<String>,
    #[serde(default)]
    pub attributes: Option<serde_json::Value>,
    #[serde(default)]
    pub user_id: Option<String>,
    pub external_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateThreadRequest {
    pub title: Option<String>,
    pub metadata: Option<HashMap<String, serde_json::Value>>,
    pub attributes: Option<serde_json::Value>,
    pub user_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum FileType {
    Bytes {
        bytes: String,
        mime_type: String,
        name: Option<String>,
    },
    Url {
        url: String,
        mime_type: String,
        name: Option<String>,
    },
}

impl FileType {
    pub fn as_image_url(&self) -> Option<String> {
        match self {
            FileType::Url { url, .. } => Some(url.clone()),
            FileType::Bytes {
                bytes, mime_type, ..
            } => Some(format!("data:{};base64,{}", mime_type, bytes)),
        }
    }
}
