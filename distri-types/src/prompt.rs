use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use handlebars::Handlebars;
use handlebars::handlebars_helper;
use serde::Serialize;
use tokio::sync::RwLock;

use crate::{AgentError, Message, Part};

/// A registry for prompt templates that can be used across the system.
#[derive(Debug, Clone)]
pub struct PromptRegistry {
    templates: Arc<RwLock<HashMap<String, PromptTemplate>>>,
    partials: Arc<RwLock<HashMap<String, String>>>,
}

/// A prompt template with metadata.
#[derive(Debug, Clone)]
pub struct PromptTemplate {
    pub name: String,
    pub content: String,
    pub description: Option<String>,
    pub version: Option<String>,
    pub source: TemplateSource,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct TemplateData<'a> {
    pub description: String,
    pub instructions: String,
    pub available_tools: String,
    pub task: String,
    pub scratchpad: String,
    pub dynamic_sections: Vec<PromptSection>,
    pub dynamic_values: std::collections::HashMap<String, serde_json::Value>,
    /// Session values fetched from the session store - available in templates as {{session.key}}
    pub session_values: std::collections::HashMap<String, serde_json::Value>,
    pub reasoning_depth: &'a str,
    pub execution_mode: &'a str,
    pub tool_format: &'a str,
    pub show_examples: bool,
    pub max_steps: usize,
    pub current_steps: usize,
    pub remaining_steps: usize,
    pub todos: Option<String>,
    pub json_tools: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct PromptSection {
    pub key: String,
    pub content: String,
}

/// The source of a prompt template.
#[derive(Debug, Clone, PartialEq)]
pub enum TemplateSource {
    Static,
    File(String),
    Dynamic,
}

impl PromptRegistry {
    pub fn new() -> Self {
        Self {
            templates: Arc::new(RwLock::new(HashMap::new())),
            partials: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a registry preloaded with the built-in templates/partials.
    pub async fn with_defaults() -> Result<Self, AgentError> {
        let registry = Self::new();
        registry.register_static_templates().await?;
        registry.register_static_partials().await?;
        Ok(registry)
    }

    async fn register_static_templates(&self) -> Result<(), AgentError> {
        let templates = vec![
            PromptTemplate {
                name: "planning".to_string(),
                content: include_str!("../../prompt_templates/planning.hbs").to_string(),
                description: Some("Default system message template".to_string()),
                version: Some("1.0.0".to_string()),
                source: TemplateSource::Static,
            },
            PromptTemplate {
                name: "user".to_string(),
                content: include_str!("../../prompt_templates/user.hbs").to_string(),
                description: Some("Default user message template".to_string()),
                version: Some("1.0.0".to_string()),
                source: TemplateSource::Static,
            },
            PromptTemplate {
                name: "code".to_string(),
                content: include_str!("../../prompt_templates/code.hbs").to_string(),
                description: Some("Code generation template".to_string()),
                version: Some("1.0.0".to_string()),
                source: TemplateSource::Static,
            },
            PromptTemplate {
                name: "reflection".to_string(),
                content: include_str!("../../prompt_templates/reflection.hbs").to_string(),
                description: Some("Reflection and improvement template".to_string()),
                version: Some("1.0.0".to_string()),
                source: TemplateSource::Static,
            },
        ];

        let mut templates_lock = self.templates.write().await;
        for template in templates {
            templates_lock.insert(template.name.clone(), template);
        }

        Ok(())
    }

    async fn register_static_partials(&self) -> Result<(), AgentError> {
        let partials = vec![
            (
                "core_instructions",
                include_str!("../../prompt_templates/partials/core_instructions.hbs"),
            ),
            (
                "communication",
                include_str!("../../prompt_templates/partials/communication.hbs"),
            ),
            (
                "todo_instructions",
                include_str!("../../prompt_templates/partials/todo_instructions.hbs"),
            ),
            (
                "tools_xml",
                include_str!("../../prompt_templates/partials/tools_xml.hbs"),
            ),
            (
                "tools_json",
                include_str!("../../prompt_templates/partials/tools_json.hbs"),
            ),
            (
                "reasoning",
                include_str!("../../prompt_templates/partials/reasoning.hbs"),
            ),
        ];

        let mut partials_lock = self.partials.write().await;
        for (name, content) in partials {
            partials_lock.insert(name.to_string(), content.to_string());
        }

        Ok(())
    }

    pub async fn register_template(&self, template: PromptTemplate) -> Result<(), AgentError> {
        let mut templates = self.templates.write().await;
        templates.insert(template.name.clone(), template);
        Ok(())
    }

    pub async fn register_template_string(
        &self,
        name: String,
        content: String,
        description: Option<String>,
        version: Option<String>,
    ) -> Result<(), AgentError> {
        let template = PromptTemplate {
            name: name.clone(),
            content,
            description,
            version,
            source: TemplateSource::Dynamic,
        };
        self.register_template(template).await
    }

    pub async fn register_template_file<P: AsRef<Path>>(
        &self,
        name: String,
        file_path: P,
        description: Option<String>,
        version: Option<String>,
    ) -> Result<(), AgentError> {
        let path = file_path.as_ref();
        let content = tokio::fs::read_to_string(path).await.map_err(|e| {
            AgentError::Planning(format!(
                "Failed to read template file '{}': {}",
                path.display(),
                e
            ))
        })?;

        let template = PromptTemplate {
            name: name.clone(),
            content,
            description,
            version,
            source: TemplateSource::File(path.to_string_lossy().to_string()),
        };
        self.register_template(template).await
    }

    pub async fn register_partial(&self, name: String, content: String) -> Result<(), AgentError> {
        let mut partials = self.partials.write().await;
        partials.insert(name, content);
        Ok(())
    }

    pub async fn register_partial_file<P: AsRef<Path>>(
        &self,
        name: String,
        file_path: P,
    ) -> Result<(), AgentError> {
        let path = file_path.as_ref();
        let content = tokio::fs::read_to_string(path).await.map_err(|e| {
            AgentError::Planning(format!(
                "Failed to read partial file '{}': {}",
                path.display(),
                e
            ))
        })?;
        self.register_partial(name, content).await
    }

    pub async fn register_templates_from_directory<P: AsRef<Path>>(
        &self,
        dir_path: P,
    ) -> Result<(), AgentError> {
        let path = dir_path.as_ref();
        if !path.exists() {
            return Ok(());
        }

        let mut entries = tokio::fs::read_dir(path).await.map_err(|e| {
            AgentError::Planning(format!(
                "Failed to read directory '{}': {}",
                path.display(),
                e
            ))
        })?;

        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| AgentError::Planning(format!("Failed to read directory entry: {}", e)))?
        {
            let entry_path = entry.path();
            if entry_path.is_file() {
                if let Some(extension) = entry_path.extension() {
                    if extension == "hbs" || extension == "handlebars" {
                        if let Some(stem) = entry_path.file_stem() {
                            let name = stem.to_string_lossy().to_string();
                            tracing::debug!(
                                "Registering template '{}' from '{}'",
                                name,
                                entry_path.display()
                            );
                            self.register_template_file(name, &entry_path, None, None)
                                .await?;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    pub async fn register_partials_from_directory<P: AsRef<Path>>(
        &self,
        dir_path: P,
    ) -> Result<(), AgentError> {
        let path = dir_path.as_ref();
        if !path.exists() {
            return Ok(());
        }

        let mut entries = tokio::fs::read_dir(path).await.map_err(|e| {
            AgentError::Planning(format!(
                "Failed to read directory '{}': {}",
                path.display(),
                e
            ))
        })?;

        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| AgentError::Planning(format!("Failed to read directory entry: {}", e)))?
        {
            let entry_path = entry.path();
            if entry_path.is_file() {
                if let Some(extension) = entry_path.extension() {
                    if extension == "hbs" || extension == "handlebars" {
                        if let Some(stem) = entry_path.file_stem() {
                            let name = stem.to_string_lossy().to_string();
                            tracing::debug!(
                                "Registering partial '{}' from '{}'",
                                name,
                                entry_path.display()
                            );
                            self.register_partial_file(name, &entry_path).await?;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    pub async fn get_template(&self, name: &str) -> Option<PromptTemplate> {
        let templates = self.templates.read().await;
        templates.get(name).cloned()
    }

    pub async fn get_partial(&self, name: &str) -> Option<String> {
        let partials = self.partials.read().await;
        partials.get(name).cloned()
    }

    pub async fn list_templates(&self) -> Vec<String> {
        let templates = self.templates.read().await;
        templates.keys().cloned().collect()
    }

    pub async fn list_partials(&self) -> Vec<String> {
        let partials = self.partials.read().await;
        partials.keys().cloned().collect()
    }

    pub async fn get_all_templates(&self) -> HashMap<String, PromptTemplate> {
        let templates = self.templates.read().await;
        templates.clone()
    }

    pub async fn get_all_partials(&self) -> HashMap<String, String> {
        let partials = self.partials.read().await;
        partials.clone()
    }

    pub async fn clear(&self) {
        let mut templates = self.templates.write().await;
        let mut partials = self.partials.write().await;
        templates.clear();
        partials.clear();
    }

    pub async fn remove_template(&self, name: &str) -> Option<PromptTemplate> {
        let mut templates = self.templates.write().await;
        templates.remove(name)
    }

    pub async fn remove_partial(&self, name: &str) -> Option<String> {
        let mut partials = self.partials.write().await;
        partials.remove(name)
    }

    pub async fn configure_handlebars(
        &self,
        handlebars: &mut handlebars::Handlebars<'_>,
    ) -> Result<(), AgentError> {
        handlebars_helper!(eq: |x: str, y: str| x == y);
        handlebars.register_helper("eq", Box::new(eq));
        let partials = self.partials.read().await;
        for (name, content) in partials.iter() {
            handlebars.register_partial(name, content).map_err(|e| {
                AgentError::Planning(format!("Failed to register partial '{}': {}", name, e))
            })?;
        }
        Ok(())
    }

    pub async fn render_template<'a>(
        &self,
        template: &str,
        template_data: &TemplateData<'a>,
    ) -> Result<String, AgentError> {
        let mut handlebars = Handlebars::new();
        handlebars.set_strict_mode(true);

        self.configure_handlebars(&mut handlebars).await?;
        let rendered = handlebars
            .render_template(template, &template_data)
            .map_err(|e| AgentError::Planning(format!("Failed to render template: {}", e)))?;
        Ok(rendered)
    }

    pub async fn validate_template(&self, template: &str) -> Result<(), AgentError> {
        let mut handlebars = Handlebars::new();
        handlebars.set_strict_mode(true);
        self.configure_handlebars(&mut handlebars).await?;
        let sample_template_data = TemplateData::default();
        handlebars
            .render_template(template, &sample_template_data)
            .map(|_| ())
            .map_err(|e| AgentError::Planning(format!("Failed to render template: {}", e)))
    }
}

impl Default for PromptRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Render a system/user prompt pair into model-ready messages.
pub async fn build_prompt_messages<'a>(
    registry: &PromptRegistry,
    system_template: &str,
    user_template: &str,
    template_data: &TemplateData<'a>,
    user_message: &Message,
) -> Result<Vec<Message>, AgentError> {
    let rendered_system = registry
        .render_template(system_template, template_data)
        .await?;
    let rendered_user = registry
        .render_template(user_template, template_data)
        .await?;

    let system_msg = Message::system(rendered_system, None);

    let mut user_msg = user_message.clone();
    if user_msg.parts.is_empty() {
        if let Some(text) = user_message.as_text() {
            user_msg.parts.push(Part::Text(text));
        }
    }
    if !rendered_user.is_empty() {
        user_msg.parts.push(Part::Text(rendered_user));
    }

    Ok(vec![system_msg, user_msg])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn renders_templates_and_messages() {
        let registry = PromptRegistry::with_defaults().await.unwrap();
        let data = TemplateData {
            description: "desc".into(),
            instructions: "be nice".into(),
            available_tools: "none".into(),
            task: "task".into(),
            scratchpad: String::new(),
            dynamic_sections: vec![],
            dynamic_values: HashMap::new(),
            session_values: HashMap::new(),
            reasoning_depth: "standard",
            execution_mode: "tools",
            tool_format: "json",
            show_examples: false,
            max_steps: 5,
            current_steps: 0,
            remaining_steps: 5,
            todos: None,
            json_tools: true,
        };
        let msgs = build_prompt_messages(
            &registry,
            "{{instructions}}",
            "task: {{task}}",
            &data,
            &Message::user("hello".into(), None),
        )
        .await
        .unwrap();
        assert_eq!(msgs.len(), 2);
        assert!(msgs[0].as_text().unwrap().contains("be nice"));
        assert!(msgs[1].as_text().unwrap().contains("task"));
    }
}
