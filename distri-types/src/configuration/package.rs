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
    /// When the agent row was last modified. Used as a cache epoch — e.g. the
    /// gateway keys its compiled `CommandRouter` on `(agent_id, updated_at)`
    /// so a workflow/command edit invalidates the router.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
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
    /// Agent-level triggers — applied to the default entry point.
    /// Empty = `Manual` only (direct API/UI invocation). Entry-point
    /// triggers (in `WorkflowDefinition.entry_points[*].triggers`)
    /// take precedence per-entry-point.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub triggers: Vec<crate::WorkflowTrigger>,
    /// Channel chrome when this workflow agent backs a bot. Optional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channels: Option<crate::channel_commands::ChannelBindings>,
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

    /// Get schedule triggers for this agent (only workflow agents
    /// can have them). Walks both the agent-level `triggers` and
    /// each entry point's `triggers` since both layers may declare
    /// `Schedule`.
    pub fn get_schedule_triggers(&self) -> Vec<&crate::WorkflowTrigger> {
        match self {
            AgentConfig::StandardAgent(_) => vec![],
            AgentConfig::WorkflowAgent(def) => def
                .triggers
                .iter()
                .filter(|t| matches!(t, crate::WorkflowTrigger::Schedule { .. }))
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

#[cfg(test)]
mod channel_binding_tests {
    use super::*;

    #[test]
    fn workflow_agent_accepts_channels_field() {
        let json = serde_json::json!({
            "name": "z", "description": "d",
            "definition": {"id":"w","steps":[]},
            "channels": {"telegram": {"web_app_base": "https://a.app"}}
        });
        let def: WorkflowAgentDefinition = serde_json::from_value(json).unwrap();
        assert!(def.channels.is_some());
    }

    #[test]
    fn workflow_agent_channels_optional() {
        let json = serde_json::json!({
            "name": "z", "description": "d", "definition": {"id":"w","steps":[]}
        });
        let def: WorkflowAgentDefinition = serde_json::from_value(json).unwrap();
        assert!(def.channels.is_none());
    }
}
