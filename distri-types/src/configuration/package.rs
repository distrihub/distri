use crate::ToolDefinition;
use crate::agent::StandardDefinition;
use crate::configuration::manifest::DistriServerConfig;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use utoipa::ToSchema;

/// Tool definition ready for DAP registration with runtime info
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PluginToolDefinition {
    pub name: String,
    pub package_name: String,
    pub description: String,
    #[serde(default)]
    pub parameters: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    pub auth: Option<crate::auth::AuthRequirement>,
}

/// Cloud-specific metadata for agents (optional, only present in cloud responses)
#[derive(Debug, Clone, Serialize, Deserialize, Default, ToSchema)]
pub struct AgentCloudMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<uuid::Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_owner: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
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
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "agent_type", rename_all = "snake_case")]
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
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
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

    /// Validate the configuration
    pub fn validate(&self) -> anyhow::Result<()> {
        match self {
            AgentConfig::StandardAgent(def) => def.validate(),
            AgentConfig::WorkflowAgent(_def) => Ok(()), // Workflow validation happens at execution
        }
    }

    /// Get the working directory with fallback chain: agent config -> package config -> DISTRI_HOME -> current_dir
    pub fn get_working_directory(
        &self,
        package_config: Option<&DistriServerConfig>,
    ) -> anyhow::Result<std::path::PathBuf> {
        // Fall back to package configuration
        if let Some(config) = package_config {
            return config.get_working_directory();
        }

        // Try DISTRI_HOME environment variable
        if let Ok(distri_home) = std::env::var("DISTRI_HOME") {
            return Ok(std::path::PathBuf::from(distri_home));
        }

        // Fallback to current directory
        std::env::current_dir()
            .map_err(|e| anyhow::anyhow!("Failed to get current directory: {}", e))
    }
}

/// Agent definition ready for DAP registration with runtime info
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PluginAgentDefinition {
    pub name: String,
    pub package_name: String,
    pub description: String,
    #[schema(value_type = String)]
    pub file_path: PathBuf,
    /// The full agent configuration (supports all agent types)
    #[schema(value_type = Object)]
    pub agent_config: AgentConfig,
}

/// Built DAP package artifact ready for registration in distri
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PluginArtifact {
    pub name: String,
    #[schema(value_type = String)]
    pub path: PathBuf,
    pub configuration: crate::configuration::manifest::DistriServerConfig,
    pub tools: Vec<PluginToolDefinition>,
    pub agents: Vec<PluginAgentDefinition>,
}
