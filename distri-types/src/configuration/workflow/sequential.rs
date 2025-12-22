use crate::agent::StandardDefinition;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Agent reference - either by name or inline definition
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum AgentRef {
    /// Reference to existing agent by name
    Name(String),
    /// Inline agent definition
    Definition(StandardDefinition),
}

/// Workflow step for sequential workflows
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkflowStep {
    /// Tool execution step
    Tool {
        /// Name of the step (optional, auto-generated if not provided)
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        /// Tool name to execute
        tool_name: String,
        /// Input parameters for the tool
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input: Option<serde_json::Value>,
    },
    /// Agent execution step
    Agent {
        /// Name of the step (optional, auto-generated if not provided)
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        /// Agent reference (name or inline definition)
        agent: AgentRef,
        /// Task description for the agent
        task: String,
    },
}

/// Sequential workflow definition - TOML-based workflow with ordered steps
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SequentialWorkflowDefinition {
    /// The name of the workflow
    pub name: String,
    /// A brief description of the workflow's purpose
    #[serde(default)]
    pub description: String,
    /// Maximum execution time in milliseconds
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_time: Option<u64>,
    /// List of workflow steps to execute in order
    pub steps: Vec<WorkflowStep>,
    /// Working directory for this workflow agent (defaults to package working_directory, DISTRI_HOME env var, or current directory)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<std::path::PathBuf>,
}
