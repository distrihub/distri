use crate::ToolDefinition;
use crate::agent::StandardDefinition;
use crate::configuration::manifest::DistriServerConfig;
use crate::configuration::workflow::{
    CustomAgentDefinition, DagWorkflowDefinition, SequentialWorkflowDefinition,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Tool definition ready for DAP registration with runtime info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginToolDefinition {
    pub name: String,
    pub package_name: String,
    pub description: String,
    #[serde(default)]
    pub parameters: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<crate::auth::AuthRequirement>,
}

/// Workflow definition ready for DAP registration with runtime info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginWorkflowDefinition {
    pub name: String,
    pub package_name: String,
    pub description: String,
    #[serde(default)]
    pub parameters: serde_json::Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<serde_json::Value>,
}

/// Cloud-specific metadata for agents (optional, only present in cloud responses)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfigWithTools {
    #[serde(flatten)]
    pub agent: AgentConfig,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resolved_tools: Vec<ToolDefinition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub markdown: Option<String>,
    /// Cloud-specific metadata (optional, only present in cloud responses)
    #[serde(flatten, default)]
    pub cloud: AgentCloudMetadata,
}

/// Unified agent configuration enum - combines all agent and workflow types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "agent_type", rename_all = "snake_case")]
pub enum AgentConfig {
    /// Standard markdown-based agent
    StandardAgent(StandardDefinition),
    /// Sequential workflow agent with ordered steps
    SequentialWorkflowAgent(SequentialWorkflowDefinition),
    /// DAG workflow agent with dependency graph
    DagWorkflowAgent(DagWorkflowDefinition),
    /// Custom TypeScript-based agent
    CustomAgent(CustomAgentDefinition),
}

impl AgentConfig {
    /// Get the name of the agent/workflow
    pub fn get_name(&self) -> &str {
        match self {
            AgentConfig::StandardAgent(def) => &def.name,
            AgentConfig::SequentialWorkflowAgent(def) => &def.name,
            AgentConfig::DagWorkflowAgent(def) => &def.name,
            AgentConfig::CustomAgent(def) => &def.name,
        }
    }

    /// Get the description of the agent/workflow
    pub fn get_description(&self) -> &str {
        match self {
            AgentConfig::StandardAgent(def) => &def.description,
            AgentConfig::SequentialWorkflowAgent(def) => &def.description,
            AgentConfig::DagWorkflowAgent(def) => &def.description,
            AgentConfig::CustomAgent(def) => &def.description,
        }
    }

    /// Validate the configuration
    pub fn validate(&self) -> anyhow::Result<()> {
        match self {
            AgentConfig::StandardAgent(def) => def.validate(),
            AgentConfig::SequentialWorkflowAgent(def) => {
                if def.name.is_empty() {
                    return Err(anyhow::anyhow!("Workflow name cannot be empty"));
                }
                if def.steps.is_empty() {
                    return Err(anyhow::anyhow!("Workflow must have at least one step"));
                }
                Ok(())
            }
            AgentConfig::DagWorkflowAgent(def) => {
                if def.name.is_empty() {
                    return Err(anyhow::anyhow!("Workflow name cannot be empty"));
                }
                if def.nodes.is_empty() {
                    return Err(anyhow::anyhow!("DAG workflow must have at least one node"));
                }
                Ok(())
            }
            AgentConfig::CustomAgent(def) => {
                if def.name.is_empty() {
                    return Err(anyhow::anyhow!("Custom agent name cannot be empty"));
                }
                if def.script_path.is_empty() {
                    return Err(anyhow::anyhow!("Custom agent script_path cannot be empty"));
                }
                Ok(())
            }
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginAgentDefinition {
    pub name: String,
    pub package_name: String,
    pub description: String,
    pub file_path: PathBuf,
    /// The full agent configuration (supports all agent types)
    pub agent_config: AgentConfig,
}

/// Built DAP package artifact ready for registration in distri
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginArtifact {
    pub name: String,
    pub path: PathBuf,
    pub configuration: crate::configuration::manifest::DistriServerConfig,
    pub tools: Vec<PluginToolDefinition>,
    pub workflows: Vec<PluginWorkflowDefinition>,
    pub agents: Vec<PluginAgentDefinition>,
}
