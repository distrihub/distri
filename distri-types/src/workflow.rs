use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Trait for storing and retrieving workflows
#[async_trait]
pub trait WorkflowStore: Send + Sync {
    /// Store a workflow definition
    async fn store_workflow(&self, workflow: Workflow) -> anyhow::Result<()>;

    /// Retrieve a workflow by ID
    async fn get_workflow(&self, id: &str) -> anyhow::Result<Option<Workflow>>;

    /// List all workflows
    async fn list_workflows(&self) -> anyhow::Result<Vec<Workflow>>;

    /// Update a workflow
    async fn update_workflow(&self, workflow: Workflow) -> anyhow::Result<()>;

    /// Delete a workflow
    async fn delete_workflow(&self, id: &str) -> anyhow::Result<()>;

    /// Store workflow execution event
    async fn store_event(&self, event: WorkflowEvent) -> anyhow::Result<()>;

    /// Get workflow execution event
    async fn get_event(&self, event_id: &str) -> anyhow::Result<Option<WorkflowEvent>>;

    /// Store workflow execution result
    async fn store_result(&self, result: WorkflowResult) -> anyhow::Result<()>;

    /// Get workflow execution result
    async fn get_result(&self, event_id: &str) -> anyhow::Result<Option<WorkflowResult>>;

    /// List workflow execution results for a workflow
    async fn list_results(&self, workflow_id: &str) -> anyhow::Result<Vec<WorkflowResult>>;
}

/// A workflow is a TypeScript file with embedded Agent definitions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    pub id: String,
    pub name: String,
    pub description: String,
    pub typescript_code: String,   // Complete TypeScript workflow file
    pub file_path: Option<String>, // Optional path to .ts file
    pub metadata: WorkflowMetadata,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    // Runtime fields populated after validation
    pub agents: Vec<String>, // Agent names used in workflow
    pub tools: Vec<String>,  // Tool names used in workflow
    pub validated: bool,     // Whether workflow has been validated
}
impl Workflow {
    pub fn new(name: String, description: String, typescript_code: String) -> Self {
        let now = chrono::Utc::now();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            description,
            typescript_code,
            file_path: None,
            metadata: WorkflowMetadata {
                version: "1.0.0".to_string(),
                author: None,
                tags: Vec::new(),
                timeout_seconds: Some(300), // 5 minutes default
                max_memory_mb: Some(512),   // 512MB default
                max_retries: Some(3),       // 3 retries default
            },
            created_at: now,
            updated_at: now,
            agents: Vec::new(),
            tools: Vec::new(),
            validated: false,
        }
    }

    pub fn from_file(
        name: String,
        description: String,
        file_path: String,
        typescript_code: String,
    ) -> Self {
        let mut workflow = Self::new(name, description, typescript_code);
        workflow.file_path = Some(file_path);
        workflow
    }
}

/// Runtime Agent definition extracted from TypeScript
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowAgent {
    pub name: String,
    pub description: String,
    pub instructions: String,
    pub tools: Vec<String>,
    pub config: Option<Value>, // Additional agent configuration
}

/// Runtime Tool definition extracted from TypeScript
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowTool {
    pub name: String,
    pub function_code: Option<String>, // TypeScript function code if custom tool
    pub tool_type: WorkflowToolType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WorkflowToolType {
    Builtin { name: String },
    Custom { code: String },
    External { reference: String },
}

/// Workflow metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowMetadata {
    pub version: String,
    pub author: Option<String>,
    pub tags: Vec<String>,
    pub timeout_seconds: Option<u64>,
    pub max_memory_mb: Option<u64>,
    pub max_retries: Option<u32>,
}

/// Input event to trigger workflow execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowEvent {
    pub id: String,
    pub workflow_id: String,
    pub input_data: Value,
    pub context: HashMap<String, Value>,
    pub streaming: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Result of workflow execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowResult {
    pub event_id: String,
    pub workflow_id: String,
    pub success: bool,
    pub output_data: Option<Value>,
    pub error: Option<String>,
    pub execution_time_ms: u64,
    pub agent_calls: Vec<AgentCallLog>,
    pub logs: Vec<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Log of agent function calls during workflow execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCallLog {
    pub function_name: String,
    pub agent_id: String,
    pub input: Value,
    pub output: Option<Value>,
    pub error: Option<String>,
    pub duration_ms: u64,
    pub retry_count: u32,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Node in the workflow DAG for visualization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowNode {
    pub id: String,
    pub node_type: WorkflowNodeType,
    pub label: String,
    pub position: Option<(f64, f64)>,
    pub metadata: HashMap<String, Value>,
}

/// Edge in the workflow DAG for visualization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowEdge {
    pub id: String,
    pub source: String,
    pub target: String,
    pub label: Option<String>,
    pub condition: Option<String>,
}

/// Types of nodes in workflow DAG
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WorkflowNodeType {
    Start,
    End,
    AgentCall { function_name: String },
    ToolCall { tool_name: String },
    Condition { expression: String },
    Loop { variable: String, iterable: String },
    Parallel { branches: Vec<String> },
}

/// DAG representation of a workflow for visualization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDAG {
    pub nodes: Vec<WorkflowNode>,
    pub edges: Vec<WorkflowEdge>,
    pub layout: Option<String>, // Layout algorithm hint (e.g., "hierarchical", "force")
}
