use crate::ToolDefinition;
use crate::agent::StandardDefinition;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Cloud-specific metadata for agents (optional, only present in cloud responses).
/// The marketplace surface (`published`, `published_at`, `is_system`, the
/// "agent from another workspace" cross-publish concept) was removed.
/// Agents are workspace-scoped only.
#[derive(Debug, Clone, Serialize, Deserialize, Default, ToSchema, JsonSchema)]
pub struct AgentCloudMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_owner: Option<bool>,
    /// True when the agent belongs to the current workspace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_workspace: Option<bool>,
    /// Workspace slug the agent belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_slug: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct AgentConfigWithTools {
    #[serde(flatten)]
    #[schema(value_type = Object)]
    pub agent: AgentConfig,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resolved_tools: Vec<ToolDefinition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub markdown: Option<String>,
    /// Cloud-specific metadata (optional, only present in cloud responses)
    #[serde(flatten, default)]
    pub cloud: AgentCloudMetadata,
}

/// Unified agent configuration enum
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(tag = "agent_type", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum AgentConfig {
    /// Standard markdown-based agent
    #[schema(value_type = Object)]
    StandardAgent(StandardDefinition),
    /// Workflow-based agent — executes a workflow DAG instead of an LLM loop
    WorkflowAgent(WorkflowAgentDefinition),
}

/// How a workflow agent is triggered.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Trigger {
    /// Manual invocation (implicit default when triggers is empty)
    OnCall {},
    /// Cron-based scheduled execution
    Schedule {
        /// Cron expression, e.g. "0 * * * *" (every hour)
        cron: String,
        /// IANA timezone, e.g. "America/Los_Angeles". Defaults to UTC.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timezone: Option<String>,
        /// Whether this schedule is active. Defaults to true.
        #[serde(default = "default_true")]
        enabled: bool,
        /// Default input passed to the workflow on each scheduled run.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input: Option<serde_json::Value>,
    },
}

fn default_true() -> bool {
    true
}

/// Definition for a workflow-based agent.
/// The workflow definition is stored as JSON to avoid crate dependency on distri-workflow.
/// Deserialize to `distri_workflow::WorkflowDefinition` at execution time.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct WorkflowAgentDefinition {
    pub name: String,
    pub description: String,
    #[serde(default = "default_version")]
    pub version: String,
    /// The workflow definition as JSON.
    pub definition: serde_json::Value,
    /// JSON Schema for required inputs (validated before execution).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<serde_json::Value>,
    /// How this workflow is triggered. Defaults to on_call if empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub triggers: Vec<Trigger>,
}

fn default_version() -> String {
    "0.1.0".to_string()
}

impl AgentConfig {
    /// Get the name of the agent
    pub fn get_name(&self) -> &str {
        match self {
            AgentConfig::StandardAgent(def) => &def.name,
            AgentConfig::WorkflowAgent(def) => &def.name,
        }
    }

    pub fn get_definition(&self) -> &StandardDefinition {
        match self {
            AgentConfig::StandardAgent(def) => def,
            AgentConfig::WorkflowAgent(_) => {
                panic!("WorkflowAgent does not have a StandardDefinition")
            }
        }
    }

    /// Get the description of the agent
    pub fn get_description(&self) -> &str {
        match self {
            AgentConfig::StandardAgent(def) => &def.description,
            AgentConfig::WorkflowAgent(def) => &def.description,
        }
    }

    /// Get the tools configuration, if this is a standard agent.
    pub fn get_tools_config(&self) -> Option<&crate::ToolsConfig> {
        match self {
            AgentConfig::StandardAgent(def) => def.tools.as_ref(),
            AgentConfig::WorkflowAgent(_) => None,
        }
    }

    /// Get schedule triggers for this agent (only workflow agents can have them).
    pub fn get_schedule_triggers(&self) -> Vec<&Trigger> {
        match self {
            AgentConfig::StandardAgent(_) => vec![],
            AgentConfig::WorkflowAgent(def) => def
                .triggers
                .iter()
                .filter(|t| matches!(t, Trigger::Schedule { .. }))
                .collect(),
        }
    }

    /// Validate the configuration
    pub fn validate(&self) -> anyhow::Result<()> {
        match self {
            AgentConfig::StandardAgent(def) => def.validate(),
            AgentConfig::WorkflowAgent(_def) => Ok(()), // Workflow validation happens at execution
        }
    }
}
