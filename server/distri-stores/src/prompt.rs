use handlebars::Handlebars;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use tokio::fs;

#[async_trait::async_trait]
pub trait PromptStore {
    /// Load a template by ID
    async fn load_template(&self, template_id: &str) -> Result<String, PromptStoreError>;

    /// Render a template with variables
    async fn render_template(
        &self,
        template_id: &str,
        variables: &HashMap<String, Value>,
    ) -> Result<String, PromptStoreError> {
        let template = self.load_template(template_id).await?;

        let handlebars = Handlebars::new();
        let data = serde_json::to_value(variables)?;

        let rendered = handlebars.render_template(&template, &data)?;
        Ok(rendered)
    }

    /// Check if a template exists
    async fn template_exists(&self, template_id: &str) -> bool;
}

#[derive(Debug, thiserror::Error)]
pub enum PromptStoreError {
    #[error("Template not found: {0}")]
    TemplateNotFound(String),
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Template rendering error: {0}")]
    RenderingError(#[from] handlebars::RenderError),
    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),
}

pub struct HashMapPromptStore {
    prompts: HashMap<String, String>,
}

impl HashMapPromptStore {
    pub fn new(prompts: HashMap<String, String>) -> Self {
        Self { prompts }
    }
}

#[async_trait::async_trait]
impl PromptStore for HashMapPromptStore {
    async fn load_template(&self, template_id: &str) -> Result<String, PromptStoreError> {
        Ok(self.prompts.get(template_id).unwrap().to_owned())
    }

    async fn template_exists(&self, template_id: &str) -> bool {
        self.prompts.contains_key(template_id)
    }
}

pub struct FileBasedPromptStore {
    base_path: String,
    cache: HashMap<String, String>,
}

impl FileBasedPromptStore {
    pub fn new(base_path: String) -> Self {
        Self {
            base_path,
            cache: HashMap::new(),
        }
    }

    pub fn new_default() -> Self {
        Self::new("prompt_templates".to_string())
    }

    fn get_template_path(&self, template_id: &str) -> String {
        // Template ID format: "type/name" (e.g., "plan/cot_initial", "scratchpad/cot_scratchpad")
        // The path should be relative to the current working directory
        format!("{}/{}.hbs", self.base_path, template_id)
    }
}

#[async_trait::async_trait]
impl PromptStore for FileBasedPromptStore {
    async fn load_template(&self, template_id: &str) -> Result<String, PromptStoreError> {
        // Check cache first
        if let Some(cached) = self.cache.get(template_id) {
            return Ok(cached.clone());
        }

        let template_path = self.get_template_path(template_id);
        let template = fs::read_to_string(&template_path).await?;

        // Cache the template (in a real implementation, you'd want thread-safe caching)
        // For now, we'll just return the template
        Ok(template)
    }

    async fn template_exists(&self, template_id: &str) -> bool {
        let template_path = self.get_template_path(template_id);
        Path::new(&template_path).exists()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_template_loading() {
        let store = FileBasedPromptStore::new_default();

        // Test that we can load a template
        let result = store.load_template("plan/cot_initial").await;
        assert!(result.is_ok());

        let template = result.unwrap();
        assert!(template.contains("IMPORTANT: Create a COMPLETE plan"));
        assert!(template.contains("{{tools}}"));
    }

    #[tokio::test]
    async fn test_template_rendering() {
        let store = FileBasedPromptStore::new_default();

        let mut variables = HashMap::new();
        variables.insert("tools".to_string(), json!("search, calculator"));
        variables.insert("examples".to_string(), json!("EXAMPLE 1: Test example"));

        let result = store.render_template("plan/cot_initial", &variables).await;
        assert!(result.is_ok());

        let rendered = result.unwrap();
        assert!(rendered.contains("search, calculator"));
        assert!(rendered.contains("EXAMPLE 1: Test example"));
        assert!(!rendered.contains("{{tools}}"));
    }

    #[tokio::test]
    async fn test_template_exists() {
        let store = FileBasedPromptStore::new_default();

        assert!(store.template_exists("plan/cot_initial").await);
        assert!(store.template_exists("scratchpad/cot_scratchpad").await);
        assert!(!store.template_exists("nonexistent/template").await);
    }
}
