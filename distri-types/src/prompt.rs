use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use handlebars::Handlebars;
use handlebars::handlebars_helper;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::{AgentError, ContextBudget, Message, Part};

/// A registry for prompt templates that can be used across the system.
///
/// Supports two-zone prompt construction with section-level caching:
/// - **Static zone**: Cached across turns/sessions, hashable for API-side caching
/// - **Dynamic zone**: Recomputed each turn (env, scratchpad, tools, skills)
///
/// Section-level cache: individual rendered sections can be memoized per-session
/// to avoid re-rendering unchanged content.
#[derive(Debug, Clone)]
pub struct PromptRegistry {
    templates: Arc<RwLock<HashMap<String, PromptTemplate>>>,
    partials: Arc<RwLock<HashMap<String, String>>>,
    /// Section-level render cache: maps section_key → (rendered_content, token_count)
    section_cache: Arc<RwLock<HashMap<String, (String, usize)>>>,
    /// Cached hash of the static prefix for prompt cache optimization
    static_prefix_hash: Arc<RwLock<Option<String>>>,
}

/// A prompt template with metadata.
#[derive(Debug, Clone)]
pub struct PromptTemplate {
    pub name: String,
    pub content: String,
    pub description: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct TemplateData<'a> {
    pub description: String,
    pub instructions: String,
    pub available_tools: String,
    pub task: String,
    pub scratchpad: String,
    pub dynamic_sections: Vec<PromptSection>,
    #[serde(flatten)]
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
    /// Formatted list of available skills the agent can load on demand
    #[serde(default)]
    pub available_skills: Option<String>,
    /// Concatenated tool prompts/instructions (all tools).
    /// Available in templates as `{{{tool_prompts}}}`.
    #[serde(default)]
    pub tool_prompts: String,
    /// Per-tool prompt list for fine-grained control in templates.
    /// Use `{{#each tool_prompt_list}}` to iterate, each has `.name` and `.prompt`.
    #[serde(default)]
    pub tool_prompt_list: Vec<ToolPromptEntry>,
    /// Formatted list of deferred tools (name + description only, no schemas).
    /// When present, rendered in the dynamic_suffix partial to tell the model
    /// it can use `tool_search` to fetch full schemas on demand.
    #[serde(default)]
    pub deferred_tools_listing: Option<String>,
}

/// A single tool's prompt entry for template iteration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolPromptEntry {
    pub name: String,
    pub prompt: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PromptSection {
    pub key: String,
    pub content: String,
}

impl PromptRegistry {
    pub fn new() -> Self {
        Self {
            templates: Arc::new(RwLock::new(HashMap::new())),
            partials: Arc::new(RwLock::new(HashMap::new())),
            section_cache: Arc::new(RwLock::new(HashMap::new())),
            static_prefix_hash: Arc::new(RwLock::new(None)),
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
                content: include_str!("../prompt_templates/planning.hbs").to_string(),
                description: Some("Default system message template".to_string()),
                version: Some("1.0.0".to_string()),
            },
            PromptTemplate {
                name: "user".to_string(),
                content: include_str!("../prompt_templates/user.hbs").to_string(),
                description: Some("Default user message template".to_string()),
                version: Some("1.0.0".to_string()),
            },
            PromptTemplate {
                name: "code".to_string(),
                content: include_str!("../prompt_templates/code.hbs").to_string(),
                description: Some("Code generation template".to_string()),
                version: Some("1.0.0".to_string()),
            },
            PromptTemplate {
                name: "reflection".to_string(),
                content: include_str!("../prompt_templates/reflection.hbs").to_string(),
                description: Some("Reflection and improvement template".to_string()),
                version: Some("1.0.0".to_string()),
            },
            PromptTemplate {
                name: "standard_user_message".to_string(),
                content: include_str!("../prompt_templates/user.hbs").to_string(),
                description: Some("Standard user message template".to_string()),
                version: Some("1.0.0".to_string()),
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
                include_str!("../prompt_templates/partials/core_instructions.hbs"),
            ),
            (
                "communication",
                include_str!("../prompt_templates/partials/communication.hbs"),
            ),
            (
                "todo_instructions",
                include_str!("../prompt_templates/partials/todo_instructions.hbs"),
            ),
            (
                "tools_xml",
                include_str!("../prompt_templates/partials/tools_xml.hbs"),
            ),
            (
                "tools_json",
                include_str!("../prompt_templates/partials/tools_json.hbs"),
            ),
            (
                "reasoning",
                include_str!("../prompt_templates/partials/reasoning.hbs"),
            ),
            (
                "skills",
                include_str!("../prompt_templates/partials/skills.hbs"),
            ),
            (
                "connections",
                include_str!("../prompt_templates/partials/connections.hbs"),
            ),
            (
                "sub_agents",
                include_str!("../prompt_templates/partials/sub_agents.hbs"),
            ),
            (
                "static_prefix",
                include_str!("../prompt_templates/partials/static_prefix.hbs"),
            ),
            (
                "dynamic_suffix",
                include_str!("../prompt_templates/partials/dynamic_suffix.hbs"),
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
        };
        self.register_template(template).await
    }

    pub fn get_default_templates() -> Vec<crate::stores::NewPromptTemplate> {
        vec![
            crate::stores::NewPromptTemplate {
                name: "planning".to_string(),
                template: include_str!("../prompt_templates/planning.hbs").to_string(),
                description: Some("Default system message template".to_string()),
                version: Some("1.0.0".to_string()),
                is_system: true,
            },
            crate::stores::NewPromptTemplate {
                name: "user".to_string(),
                template: include_str!("../prompt_templates/user.hbs").to_string(),
                description: Some("Default user message template".to_string()),
                version: Some("1.0.0".to_string()),
                is_system: true,
            },
            crate::stores::NewPromptTemplate {
                name: "code".to_string(),
                template: include_str!("../prompt_templates/code.hbs").to_string(),
                description: Some("Code generation template".to_string()),
                version: Some("1.0.0".to_string()),
                is_system: true,
            },
            crate::stores::NewPromptTemplate {
                name: "reflection".to_string(),
                template: include_str!("../prompt_templates/reflection.hbs").to_string(),
                description: Some("Reflection and improvement template".to_string()),
                version: Some("1.0.0".to_string()),
                is_system: true,
            },
            crate::stores::NewPromptTemplate {
                name: "standard_user_message".to_string(),
                template: include_str!("../prompt_templates/user.hbs").to_string(),
                description: Some("Standard user message template".to_string()),
                version: Some("1.0.0".to_string()),
                is_system: true,
            },
        ]
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
        };
        self.register_template(template).await
    }

    pub async fn register_partial(&self, name: String, content: String) -> Result<(), AgentError> {
        let mut partials = self.partials.write().await;
        partials.insert(name, content);
        Ok(())
    }

    /// Return the set of currently registered partial names.
    pub async fn partial_names(&self) -> std::collections::HashSet<String> {
        let partials = self.partials.read().await;
        partials.keys().cloned().collect()
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
            if entry_path.is_file()
                && let Some(extension) = entry_path.extension()
                && (extension == "hbs" || extension == "handlebars")
                && let Some(stem) = entry_path.file_stem()
            {
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
            if entry_path.is_file()
                && let Some(extension) = entry_path.extension()
                && (extension == "hbs" || extension == "handlebars")
                && let Some(stem) = entry_path.file_stem()
            {
                let name = stem.to_string_lossy().to_string();
                tracing::debug!(
                    "Registering partial '{}' from '{}'",
                    name,
                    entry_path.display()
                );
                self.register_partial_file(name, &entry_path).await?;
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
        {
            let mut templates = self.templates.write().await;
            templates.clear();
        }
        {
            let mut partials = self.partials.write().await;
            partials.clear();
        }
        // Also clear section caches
        self.clear_section_cache().await;
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

    /// Render a template and return the result with token budget information.
    /// This is the budget-aware version that tracks per-component token usage.
    pub async fn render_template_with_budget<'a>(
        &self,
        template: &str,
        template_data: &TemplateData<'a>,
    ) -> Result<RenderResult, AgentError> {
        let rendered = self.render_template(template, template_data).await?;
        let estimated_tokens = rough_token_count(&rendered);

        Ok(RenderResult {
            content: rendered,
            estimated_tokens,
        })
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

impl PromptRegistry {
    /// Render the static prefix zone and cache its hash.
    /// Returns (rendered_content, hash, estimated_tokens).
    /// Subsequent calls with the same template data return the cached result.
    pub async fn render_static_prefix<'a>(
        &self,
        template_data: &TemplateData<'a>,
    ) -> Result<(String, String, usize), AgentError> {
        let cache_key = "static_prefix".to_string();

        // Check section cache first
        {
            let cache = self.section_cache.read().await;
            if let Some((content, tokens)) = cache.get(&cache_key) {
                let hash = self.static_prefix_hash.read().await;
                if let Some(h) = hash.as_ref() {
                    return Ok((content.clone(), h.clone(), *tokens));
                }
            }
        }

        // Render the static prefix partial
        let static_template = "{{> static_prefix}}";
        let rendered = self.render_template(static_template, template_data).await?;
        let tokens = rough_token_count(&rendered);

        // Compute hash for API-side caching
        let hash = compute_hash(&rendered);

        // Cache the result
        {
            let mut cache = self.section_cache.write().await;
            cache.insert(cache_key, (rendered.clone(), tokens));
        }
        {
            let mut hash_lock = self.static_prefix_hash.write().await;
            *hash_lock = Some(hash.clone());
        }

        Ok((rendered, hash, tokens))
    }

    /// Render the dynamic suffix zone. This is NOT cached between turns.
    pub async fn render_dynamic_suffix<'a>(
        &self,
        template_data: &TemplateData<'a>,
    ) -> Result<(String, usize), AgentError> {
        let dynamic_template = "{{> dynamic_suffix}}";
        let rendered = self
            .render_template(dynamic_template, template_data)
            .await?;
        let tokens = rough_token_count(&rendered);
        Ok((rendered, tokens))
    }

    /// Render a section and cache the result. Returns (content, token_count).
    /// If the section was previously rendered with the same key, returns cached.
    pub async fn render_section_cached<'a>(
        &self,
        section_key: &str,
        template: &str,
        template_data: &TemplateData<'a>,
    ) -> Result<(String, usize), AgentError> {
        // Check cache
        {
            let cache = self.section_cache.read().await;
            if let Some((content, tokens)) = cache.get(section_key) {
                return Ok((content.clone(), *tokens));
            }
        }

        let rendered = self.render_template(template, template_data).await?;
        let tokens = rough_token_count(&rendered);

        // Store in cache
        {
            let mut cache = self.section_cache.write().await;
            cache.insert(section_key.to_string(), (rendered.clone(), tokens));
        }

        Ok((rendered, tokens))
    }

    /// Invalidate a specific section cache entry.
    pub async fn invalidate_section(&self, section_key: &str) {
        let mut cache = self.section_cache.write().await;
        cache.remove(section_key);
    }

    /// Clear all section caches. Call this on /clear or /compact.
    pub async fn clear_section_cache(&self) {
        let mut cache = self.section_cache.write().await;
        cache.clear();
        let mut hash = self.static_prefix_hash.write().await;
        *hash = None;
    }

    /// Get the cached static prefix hash, if available.
    pub async fn get_static_prefix_hash(&self) -> Option<String> {
        self.static_prefix_hash.read().await.clone()
    }
}

/// Compute a simple hash of content for cache tracking.
/// Uses a fast non-cryptographic hash (FNV-1a style).
fn compute_hash(content: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325; // FNV offset basis
    for byte in content.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3); // FNV prime
    }
    format!("{:016x}", hash)
}

/// Fast rough token count: ~4 chars per token.
/// Standalone function for use outside of distri-core's TokenEstimator.
#[inline]
pub fn rough_token_count(text: &str) -> usize {
    text.len().div_ceil(4)
}

/// Result of rendering a template with budget tracking.
#[derive(Debug, Clone)]
pub struct RenderResult {
    pub content: String,
    pub estimated_tokens: usize,
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
    if user_msg.parts.is_empty()
        && let Some(text) = user_message.as_text()
    {
        user_msg.parts.push(Part::Text(text));
    }
    if !rendered_user.is_empty() {
        user_msg.parts.push(Part::Text(rendered_user));
    }

    Ok(vec![system_msg, user_msg])
}

/// Result of building prompt messages with budget tracking.
#[derive(Debug, Clone)]
pub struct PromptBuildResult {
    pub messages: Vec<Message>,
    pub budget: ContextBudget,
}

/// Build prompt messages with per-component token budget tracking.
///
/// Returns both the messages and a `ContextBudget` snapshot showing
/// how many tokens each component consumes. This enables:
/// - Monitoring context utilization per turn
/// - Triggering compaction/deferral when thresholds are exceeded
/// - Optimizing which components to include
pub async fn build_prompt_messages_with_budget<'a>(
    registry: &PromptRegistry,
    system_template: &str,
    user_template: &str,
    template_data: &TemplateData<'a>,
    user_message: &Message,
    context_window_size: usize,
) -> Result<PromptBuildResult, AgentError> {
    let system_result = registry
        .render_template_with_budget(system_template, template_data)
        .await?;
    let user_result = registry
        .render_template_with_budget(user_template, template_data)
        .await?;

    let system_msg = Message::system(system_result.content, None);

    let mut user_msg = user_message.clone();
    if user_msg.parts.is_empty()
        && let Some(text) = user_message.as_text()
    {
        user_msg.parts.push(Part::Text(text));
    }
    if !user_result.content.is_empty() {
        user_msg.parts.push(Part::Text(user_result.content));
    }

    // Estimate per-component token usage from template data
    let tool_schema_tokens = rough_token_count(&template_data.available_tools);
    let skill_listing_tokens = template_data
        .available_skills
        .as_ref()
        .map(|s| rough_token_count(s))
        .unwrap_or(0);

    // The system prompt total minus tools and skills gives us the prompt itself
    let prompt_only_tokens = system_result
        .estimated_tokens
        .saturating_sub(tool_schema_tokens)
        .saturating_sub(skill_listing_tokens);

    // Split prompt tokens: static portions (core_instructions, communication, etc.)
    // vs dynamic (dynamic_sections, scratchpad, todos, step limits)
    // Heuristic: dynamic_sections + scratchpad + todos are dynamic
    let dynamic_content_tokens = {
        let mut dynamic_chars = 0;
        for section in &template_data.dynamic_sections {
            dynamic_chars += section.content.len();
        }
        dynamic_chars += template_data.scratchpad.len();
        if let Some(todos) = &template_data.todos {
            dynamic_chars += todos.len();
        }
        dynamic_chars.div_ceil(4)
    };
    let static_tokens = prompt_only_tokens.saturating_sub(dynamic_content_tokens);

    let budget = ContextBudget {
        system_prompt_static_tokens: static_tokens,
        system_prompt_dynamic_tokens: dynamic_content_tokens,
        tool_schema_tokens,
        deferred_tool_tokens: 0, // Set by tool resolution layer
        skill_listing_tokens,
        conversation_tokens: 0, // Set by caller (LLM executor)
        tool_result_tokens: 0,  // Set by caller
        context_window_size,
        static_prefix_cache_hit: false,
        static_prefix_hash: None,
    };

    if budget.is_warning() {
        tracing::warn!(
            "Context budget warning: {:.1}% utilization ({}/{} tokens). \
             system_static={}, system_dynamic={}, tools={}, skills={}",
            budget.utilization() * 100.0,
            budget.total_tokens(),
            context_window_size,
            budget.system_prompt_static_tokens,
            budget.system_prompt_dynamic_tokens,
            budget.tool_schema_tokens,
            budget.skill_listing_tokens,
        );
    }

    Ok(PromptBuildResult {
        messages: vec![system_msg, user_msg],
        budget,
    })
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
            available_skills: None,
            tool_prompts: String::new(),
            tool_prompt_list: vec![],
            deferred_tools_listing: None,
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

    #[test]
    fn test_rough_token_count() {
        assert_eq!(rough_token_count(""), 0);
        assert_eq!(rough_token_count("abcd"), 1); // 4 chars = 1 token
        assert_eq!(rough_token_count("Hello world"), 3); // 11 chars ≈ 3 tokens
        assert_eq!(rough_token_count("a"), 1); // 1 char rounds up to 1 token
    }

    #[tokio::test]
    async fn test_render_template_with_budget() {
        let registry = PromptRegistry::with_defaults().await.unwrap();
        let data = TemplateData {
            instructions: "Test instructions here".into(),
            ..Default::default()
        };
        let result = registry
            .render_template_with_budget("{{instructions}}", &data)
            .await
            .unwrap();

        assert_eq!(result.content, "Test instructions here");
        assert!(result.estimated_tokens > 0);
        // ~22 chars / 4 ≈ 6 tokens
        assert_eq!(result.estimated_tokens, 6);
    }

    #[tokio::test]
    async fn test_section_cache_returns_cached_value() {
        let registry = PromptRegistry::with_defaults().await.unwrap();
        let data = TemplateData {
            instructions: "cached content".into(),
            ..Default::default()
        };

        // First render
        let (content1, tokens1) = registry
            .render_section_cached("test_section", "{{instructions}}", &data)
            .await
            .unwrap();

        // Second render should return cached value
        let (content2, tokens2) = registry
            .render_section_cached("test_section", "{{instructions}}", &data)
            .await
            .unwrap();

        assert_eq!(content1, content2);
        assert_eq!(tokens1, tokens2);
        assert_eq!(content1, "cached content");
    }

    #[tokio::test]
    async fn test_section_cache_invalidation() {
        let registry = PromptRegistry::with_defaults().await.unwrap();
        let data = TemplateData {
            instructions: "original".into(),
            ..Default::default()
        };

        let (content1, _) = registry
            .render_section_cached("test_section", "{{instructions}}", &data)
            .await
            .unwrap();
        assert_eq!(content1, "original");

        // Invalidate
        registry.invalidate_section("test_section").await;

        // Re-render with new data
        let data2 = TemplateData {
            instructions: "updated".into(),
            ..Default::default()
        };
        let (content2, _) = registry
            .render_section_cached("test_section", "{{instructions}}", &data2)
            .await
            .unwrap();
        assert_eq!(content2, "updated");
    }

    #[tokio::test]
    async fn test_build_prompt_messages_with_budget() {
        let registry = PromptRegistry::with_defaults().await.unwrap();
        let data = TemplateData {
            instructions: "be helpful".into(),
            available_tools: "tool1, tool2".into(),
            task: "do something".into(),
            ..Default::default()
        };

        let result = build_prompt_messages_with_budget(
            &registry,
            "{{instructions}}\n{{available_tools}}",
            "{{task}}",
            &data,
            &Message::user("hello".into(), None),
            200_000,
        )
        .await
        .unwrap();

        assert_eq!(result.messages.len(), 2);
        assert_eq!(result.budget.context_window_size, 200_000);
        assert!(result.budget.tool_schema_tokens > 0);
        assert!(!result.budget.is_warning());
    }

    #[test]
    fn test_compute_hash_deterministic() {
        let hash1 = compute_hash("test content");
        let hash2 = compute_hash("test content");
        assert_eq!(hash1, hash2);

        let hash3 = compute_hash("different content");
        assert_ne!(hash1, hash3);
    }
}
