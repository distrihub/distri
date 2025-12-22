use std::path::PathBuf;

use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::agent::StandardDefinition;
use crate::configuration::{
    AgentConfig, DistriConfiguration, EntryPoints, PluginAgentDefinition, PluginArtifact,
    PluginToolDefinition, PluginWorkflowDefinition,
};
use crate::stores::PluginMetadataRecord;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub agent_definition: StandardDefinition,
    pub files: Vec<SkillFile>,
    pub metadata: SkillMetadata,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Skill {
    pub fn new(
        id: String,
        name: String,
        description: String,
        agent_definition: StandardDefinition,
        files: Vec<SkillFile>,
        metadata: SkillMetadata,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    ) -> Self {
        Self {
            id,
            name,
            description,
            agent_definition,
            files,
            metadata,
            created_at,
            updated_at,
        }
    }

    pub fn to_plugin_record(
        &self,
        object_prefix: String,
        entrypoint: Option<String>,
    ) -> PluginMetadataRecord {
        let script_path = self
            .files
            .iter()
            .find(|file| matches!(file.kind, SkillFileKind::Script))
            .map(|file| file.path.clone())
            .unwrap_or_else(|| "scripts/main.ts".to_string());

        let mut configuration = DistriConfiguration::new_minimal(self.id.clone());
        configuration.description = if self.description.is_empty() {
            None
        } else {
            Some(self.description.clone())
        };
        if !self.metadata.tags.is_empty() {
            configuration.keywords = Some(self.metadata.tags.clone());
        }
        configuration.entrypoints = Some(EntryPoints {
            path: script_path.clone(),
        });
        configuration.agents = Some(vec![self.agent_definition.name.clone()]);

        let agent_definition = PluginAgentDefinition {
            name: self.agent_definition.name.clone(),
            package_name: self.id.clone(),
            description: self.agent_definition.description.clone(),
            file_path: PathBuf::from(script_path.clone()),
            agent_config: AgentConfig::StandardAgent(self.agent_definition.clone()),
        };

        let mut tools = Vec::new();
        let mut workflows = Vec::new();
        if let Some(export) = self
            .files
            .iter()
            .find(|file| matches!(file.kind, SkillFileKind::Script))
            .and_then(|file| file.export.clone())
        {
            match export {
                SkillExport::Tool { name, description } => {
                    tools.push(PluginToolDefinition {
                        name,
                        package_name: self.id.clone(),
                        description: description.unwrap_or_else(|| self.description.clone()),
                        parameters: serde_json::Value::Null,
                        auth: None,
                    });
                }
                SkillExport::Workflow { agent_name } => {
                    workflows.push(PluginWorkflowDefinition {
                        name: agent_name,
                        package_name: self.id.clone(),
                        description: self.description.clone(),
                        parameters: serde_json::Value::Null,
                        examples: Vec::new(),
                    });
                }
            }
        }

        let artifact = PluginArtifact {
            name: self.name.clone(),
            path: PathBuf::from(&object_prefix),
            configuration,
            tools,
            workflows,
            agents: vec![agent_definition],
        };

        PluginMetadataRecord {
            package_name: self.id.clone(),
            version: Some(self.metadata.version.clone()),
            object_prefix,
            entrypoint: entrypoint.or_else(|| Some(script_path)),
            artifact,
            updated_at: self.updated_at,
        }
    }

    pub fn from_plugin_record(
        record: &PluginMetadataRecord,
        mut files: Vec<SkillFile>,
    ) -> Result<Self> {
        let configuration = &record.artifact.configuration;
        let description = configuration.description.clone().unwrap_or_default();
        let version = record
            .version
            .clone()
            .unwrap_or_else(|| configuration.version.clone());
        let tags = configuration.keywords.clone().unwrap_or_default();

        let agent_definition = record
            .artifact
            .agents
            .iter()
            .find_map(|agent| match &agent.agent_config {
                AgentConfig::StandardAgent(def) => Some(def.clone()),
                _ => None,
            })
            .ok_or_else(|| anyhow!("Skill plugin does not contain a standard agent definition"))?;

        let export = if let Some(tool) = record.artifact.tools.first() {
            Some(SkillExport::Tool {
                name: tool.name.clone(),
                description: if tool.description.is_empty() {
                    None
                } else {
                    Some(tool.description.clone())
                },
            })
        } else if let Some(workflow) = record.artifact.workflows.first() {
            Some(SkillExport::Workflow {
                agent_name: workflow.name.clone(),
            })
        } else {
            None
        };

        if let Some(export_value) = export {
            if let Some(script_file) = files
                .iter_mut()
                .find(|file| matches!(file.kind, SkillFileKind::Script))
            {
                script_file.export = Some(export_value);
            }
        }

        let skill = Skill {
            id: record.package_name.clone(),
            name: record.artifact.name.clone(),
            description,
            agent_definition,
            files,
            metadata: SkillMetadata { version, tags },
            created_at: record.updated_at,
            updated_at: record.updated_at,
        };

        Ok(skill)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMetadata {
    #[serde(default = "default_skill_version")]
    pub version: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

impl Default for SkillMetadata {
    fn default() -> Self {
        Self {
            version: default_skill_version(),
            tags: Vec::new(),
        }
    }
}

fn default_skill_version() -> String {
    "0.1.0".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillFile {
    pub path: String,
    pub content: String,
    pub kind: SkillFileKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub export: Option<SkillExport>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillFileKind {
    Script,
    Markdown,
    Asset,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SkillExport {
    Tool {
        name: String,
        description: Option<String>,
    },
    Workflow {
        agent_name: String,
    },
}

pub fn slugify_id(name: &str) -> String {
    let mut slug = name
        .to_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>();
    while slug.contains("__") {
        slug = slug.replace("__", "_");
    }
    slug.trim_matches('_').to_string()
}
