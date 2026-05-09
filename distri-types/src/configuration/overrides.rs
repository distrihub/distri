use crate::agent::{RuntimeMode, ToolsConfig};
use crate::dynamic_tool::DynamicToolFactory;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Overrides for agent definition - only the most commonly overridden fields
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct DefinitionOverrides {
    /// Override the model (e.g., "gpt-4o", "gpt-4.1-mini")
    pub model: Option<String>,
    /// Override the temperature
    pub temperature: Option<f32>,
    /// Override max tokens
    pub max_tokens: Option<u32>,
    /// Override max iterations
    pub max_iterations: Option<usize>,
    /// Override instructions
    pub instructions: Option<String>,

    /// Override browser usage flag
    pub use_browser: Option<bool>,

    /// Override the agent's runtime constraint. When `Some(...)`, replaces
    /// `StandardDefinition.runtime` wholesale. The `--remote` CLI flag is
    /// sugar for `Some(vec![RuntimeMode::Cloud])` — when the caller's
    /// current runtime doesn't match, the orchestrator routes via the
    /// configured `RemoteTaskRunner`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Vec<String>>)]
    pub runtime: Option<Vec<RuntimeMode>>,

    /// Override the agent's tools configuration (used for ad-hoc agents).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    pub tools: Option<ToolsConfig>,

    /// Override the agent's description (display name for ad-hoc instances).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Override the agent's sub_agents list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub_agents: Option<Vec<String>>,

    /// Override the agent's name (display label for ad-hoc instances).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Additional dynamic tool factories to inject into the agent's tool config
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic_tools: Option<Vec<DynamicToolFactory>>,
}

impl DefinitionOverrides {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_model(mut self, model: String) -> Self {
        self.model = Some(model);
        self
    }

    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = Some(temperature);
        self
    }

    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    pub fn with_max_iterations(mut self, max_iterations: usize) -> Self {
        self.max_iterations = Some(max_iterations);
        self
    }

    pub fn with_instructions(mut self, instructions: String) -> Self {
        self.instructions = Some(instructions);
        self
    }

    pub fn with_browser_enabled(mut self, enabled: bool) -> Self {
        self.use_browser = Some(enabled);
        self
    }

    pub fn with_runtime(mut self, runtime: Vec<RuntimeMode>) -> Self {
        self.runtime = Some(runtime);
        self
    }

    /// Sugar for the `--remote` CLI flag: forces runtime = [Cloud] so the
    /// orchestrator routes the invocation through a cloud-providing
    /// RemoteTaskRunner.
    pub fn with_remote(mut self, remote: bool) -> Self {
        if remote {
            self.runtime = Some(vec![RuntimeMode::Cloud]);
        } else {
            self.runtime = None;
        }
        self
    }

    pub fn with_tools(mut self, tools: ToolsConfig) -> Self {
        self.tools = Some(tools);
        self
    }

    pub fn with_description(mut self, description: String) -> Self {
        self.description = Some(description);
        self
    }

    pub fn with_sub_agents(mut self, sub_agents: Vec<String>) -> Self {
        self.sub_agents = Some(sub_agents);
        self
    }

    pub fn with_name(mut self, name: String) -> Self {
        self.name = Some(name);
        self
    }

    pub fn with_dynamic_tools(mut self, tools: Vec<DynamicToolFactory>) -> Self {
        self.dynamic_tools = Some(tools);
        self
    }
}
