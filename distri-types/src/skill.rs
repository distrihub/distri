use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::agent::StandardDefinition;

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
    #[allow(clippy::too_many_arguments)]
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
