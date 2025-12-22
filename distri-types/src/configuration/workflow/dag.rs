use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// DAG workflow node - specific types for clear structure
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DagWorkflowNode {
    /// Tool execution node
    Tool {
        /// Unique ID of the node
        id: String,
        /// Name of the node
        name: String,
        /// Tool name to execute
        tool_name: String,
        /// Input parameters for the tool
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input: Option<serde_json::Value>,
        /// Node IDs this node depends on (must complete before this node runs)
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        depends_on: Vec<String>,
    },
    /// Agent execution node
    Agent {
        /// Unique ID of the node
        id: String,
        /// Name of the node
        name: String,
        /// Agent name to execute
        agent_name: String,
        /// Task description for the agent
        task: String,
        /// Node IDs this node depends on (must complete before this node runs)
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        depends_on: Vec<String>,
    },
}

/// DAG workflow definition - TOML-based workflow with dependency graph
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DagWorkflowDefinition {
    /// The name of the workflow
    pub name: String,
    /// A brief description of the workflow's purpose
    #[serde(default)]
    pub description: String,
    /// Maximum execution time in milliseconds
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_time: Option<u64>,
    /// List of workflow nodes with dependencies
    pub nodes: Vec<DagWorkflowNode>,
    /// Working directory for this workflow agent (defaults to package working_directory, DISTRI_HOME env var, or current directory)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<std::path::PathBuf>,
}
