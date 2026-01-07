use crate::AgentError;
use crate::a2a::AgentSkill;
use crate::browser::{BrowserAgentConfig, DistriBrowserConfig};
use crate::configuration::DefinitionOverrides;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::default::Default;

/// Default timeout for external tool execution in seconds
pub const DEFAULT_EXTERNAL_TOOL_TIMEOUT_SECS: u64 = 120;

/// Unified Agent Strategy Configuration
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct AgentStrategy {
    /// Depth of reasoning (shallow, standard, deep)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_depth: Option<ReasoningDepth>,

    /// Execution mode - tools vs code
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_mode: Option<ExecutionMode>,
    /// When to replan
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replanning: Option<ReplanningConfig>,

    /// Timeout in seconds for external tool execution (default: 120)
    /// External tools are tools that delegate execution to the frontend/client.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_tool_timeout_secs: Option<u64>,
}

impl Default for AgentStrategy {
    fn default() -> Self {
        Self {
            reasoning_depth: None,
            execution_mode: None,
            replanning: None,
            external_tool_timeout_secs: None,
        }
    }
}

impl AgentStrategy {
    /// Get reasoning depth with default fallback
    pub fn get_reasoning_depth(&self) -> ReasoningDepth {
        self.reasoning_depth.clone().unwrap_or_default()
    }

    /// Get execution mode with default fallback
    pub fn get_execution_mode(&self) -> ExecutionMode {
        self.execution_mode.clone().unwrap_or_default()
    }

    /// Get replanning config with default fallback
    pub fn get_replanning(&self) -> ReplanningConfig {
        self.replanning.clone().unwrap_or_default()
    }

    /// Get external tool timeout with default fallback
    pub fn get_external_tool_timeout_secs(&self) -> u64 {
        self.external_tool_timeout_secs
            .unwrap_or(DEFAULT_EXTERNAL_TOOL_TIMEOUT_SECS)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CodeLanguage {
    #[default]
    Typescript,
}

impl std::fmt::Display for CodeLanguage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_string())
    }
}

impl CodeLanguage {
    pub fn to_string(&self) -> String {
        match self {
            CodeLanguage::Typescript => "typescript".to_string(),
        }
    }
}

/// Reflection configuration
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct ReflectionConfig {
    /// Whether to enable reflection
    #[serde(default)]
    pub enabled: bool,
    /// When to trigger reflection
    #[serde(default)]
    pub trigger: ReflectionTrigger,
    /// Depth of reflection
    #[serde(default)]
    pub depth: ReflectionDepth,
}

/// When to trigger reflection
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReflectionTrigger {
    /// At the end of execution
    #[default]
    EndOfExecution,
    /// After each step
    AfterEachStep,
    /// After failures only
    AfterFailures,
    /// After N steps
    AfterNSteps(usize),
}

/// Depth of reflection
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReflectionDepth {
    /// Light reflection
    #[default]
    Light,
    /// Standard reflection
    Standard,
    /// Deep reflection
    Deep,
}

/// Configuration for planning operations
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PlanConfig {
    /// The model settings for the planning agent
    #[serde(default)]
    pub model_settings: ModelSettings,
    /// The maximum number of iterations allowed during planning
    #[serde(default = "default_plan_max_iterations")]
    pub max_iterations: usize,
}

impl Default for PlanConfig {
    fn default() -> Self {
        Self {
            model_settings: ModelSettings::default(),
            max_iterations: default_plan_max_iterations(),
        }
    }
}

fn default_plan_max_iterations() -> usize {
    10
}

/// Depth of reasoning for planning
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningDepth {
    /// Shallow reasoning - direct action with minimal thought, skip reasoning sections
    Shallow,
    /// Standard reasoning - moderate planning and thought
    #[default]
    Standard,
    /// Deep reasoning - extensive planning, multi-step analysis, and comprehensive thinking
    Deep,
}

/// Execution mode - tools vs code
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default, PartialEq)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ExecutionMode {
    /// Use tools for execution
    #[default]
    Tools,
    /// Use code execution
    Code { language: CodeLanguage },
}

/// Replanning configuration
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub struct ReplanningConfig {
    /// When to trigger replanning
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<ReplanningTrigger>,
    /// Whether to replan at all
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

/// When to trigger replanning
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReplanningTrigger {
    /// Never replan (default)
    #[default]
    Never,
    /// Replan after execution reflection
    AfterReflection,
    /// Replan after N iterations
    AfterNIterations(usize),
    /// Replan after failures
    AfterFailures,
}

impl ReplanningConfig {
    /// Get trigger with default fallback
    pub fn get_trigger(&self) -> ReplanningTrigger {
        self.trigger.clone().unwrap_or_default()
    }

    /// Get enabled with default fallback
    pub fn is_enabled(&self) -> bool {
        self.enabled.unwrap_or(false)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionKind {
    #[default]
    Retriable,
    Interleaved,
    Sequential,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    #[default]
    None,
    ShortTerm,
    LongTerm,
}

/// Supported tool call formats
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallFormat {
    /// New XML format: Streaming-capable XML tool calls
    /// Example: <search><query>test</query></search>
    #[default]
    Xml,
    /// New JSON format: JSONL with tool_calls blocks
    /// Example: ```tool_calls\n{"name":"search","arguments":{"query":"test"}}```
    JsonL,

    /// Code execution format: TypeScript/JavaScript code blocks
    /// Example: ```typescript ... ```
    Code,
    #[serde(rename = "provider")]
    Provider,
    None,
}

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema, Default)]
pub struct UserMessageOverrides {
    /// The parts to include in the user message
    pub parts: Vec<PartDefinition>,
    /// If true, artifacts will be expanded to their actual content (e.g., image artifacts become Part::Image)
    #[serde(default)]
    pub include_artifacts: bool,
    /// If true (default), step count information will be included at the end of the user message
    #[serde(default = "default_include_step_count")]
    pub include_step_count: Option<bool>,
}

fn default_include_step_count() -> Option<bool> {
    Some(true)
}

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
#[serde(tag = "type", content = "source", rename_all = "snake_case")]
pub enum PartDefinition {
    Template(String),   // Prompt Template Key
    SessionKey(String), // Session key reference
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct LlmDefinition {
    /// The name of the agent.
    pub name: String,
    /// Settings related to the model used by the agent.
    #[serde(default)]
    pub model_settings: ModelSettings,
    /// Tool calling configuration
    #[serde(default)]
    pub tool_format: ToolCallFormat,
}

/// Hooks configuration to control how browser hooks interact with Browsr.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum BrowserHooksConfig {
    /// Disable browser hooks entirely.
    Disabled,
    /// Send hook events to Browsr via HTTP/stdout.
    Webhook {
        /// Optional override base URL for the Browsr HTTP API.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        api_base_url: Option<String>,
    },
    /// Await an inline hook completion (e.g., via POST /event/hooks) before continuing.
    Inline {
        /// Optional timeout in milliseconds to await the inline hook.
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
}

impl Default for BrowserHooksConfig {
    fn default() -> Self {
        BrowserHooksConfig::Disabled
    }
}

/// Agent definition - complete configuration for an agent
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct StandardDefinition {
    /// The name of the agent.
    pub name: String,
    /// Optional package name that registered this agent (workspace/plugin metadata)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_name: Option<String>,
    /// A brief description of the agent's purpose.
    #[serde(default)]
    pub description: String,

    /// The version of the agent.
    #[serde(default = "default_agent_version")]
    pub version: Option<String>,

    /// Instructions for the agent - serves as an introduction defining what the agent is and does.
    #[serde(default)]
    pub instructions: String,

    /// A list of MCP server definitions associated with the agent.
    #[serde(default)]
    pub mcp_servers: Option<Vec<McpDefinition>>,
    /// Settings related to the model used by the agent.
    #[serde(default)]
    pub model_settings: ModelSettings,
    /// Optional lower-level model settings for lightweight analysis helpers
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analysis_model_settings: Option<ModelSettings>,

    /// The size of the history to maintain for the agent.
    #[serde(default = "default_history_size")]
    pub history_size: Option<usize>,
    /// The new strategy configuration for the agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy: Option<AgentStrategy>,
    /// A2A-specific fields
    #[serde(default)]
    pub icon_url: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_iterations: Option<usize>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<AgentSkill>,

    /// List of sub-agents that this agent can transfer control to
    #[serde(default)]
    pub sub_agents: Vec<String>,

    /// Tool calling configuration
    #[serde(default)]
    pub tool_format: ToolCallFormat,

    /// Tools configuration for this agent
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<ToolsConfig>,

    /// Where filesystem and artifact tools should run (server or local)
    #[serde(default)]
    pub file_system: FileSystemMode,

    /// Custom handlebars partials (name -> template path) for use in custom prompts
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub partials: std::collections::HashMap<String, String>,

    /// Whether to write large tool responses to filesystem as artifacts (default: false)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write_large_tool_responses_to_fs: Option<bool>,

    /// Whether to enable reflection using a subagent (default: false)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enable_reflection: Option<bool>,
    /// Whether to enable TODO management functionality
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enable_todos: Option<bool>,

    /// Browser configuration for this agent (enables shared Chromium automation)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser_config: Option<BrowserAgentConfig>,
    /// Browser hook configuration (API vs local)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser_hooks: Option<BrowserHooksConfig>,

    /// Context size override for this agent (overrides model_settings.context_size)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_size: Option<u32>,

    /// Strategy for prompt construction (append default template vs fully custom)
    #[serde(
        skip_serializing_if = "Option::is_none",
        default = "default_append_default_instructions"
    )]
    pub append_default_instructions: Option<bool>,
    /// Whether to include the built-in scratchpad/history in prompts (default: true)
    #[serde(
        skip_serializing_if = "Option::is_none",
        default = "default_include_scratchpad"
    )]
    pub include_scratchpad: Option<bool>,

    /// Optional hook names to attach to this agent
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hooks: Vec<String>,
 
    /// Custom user message construction (dynamic prompting)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_message_overrides: Option<UserMessageOverrides>,
}
fn default_append_default_instructions() -> Option<bool> {
    Some(true)
}
fn default_include_scratchpad() -> Option<bool> {
    Some(true)
}
impl StandardDefinition {
    /// Check if large tool responses should be written to filesystem (default: false)
    pub fn should_write_large_tool_responses_to_fs(&self) -> bool {
        self.write_large_tool_responses_to_fs.unwrap_or(false)
    }

    /// Check if browser should be initialized automatically in orchestrator (default: false)
    pub fn should_use_browser(&self) -> bool {
        self.browser_config
            .as_ref()
            .map(|cfg| cfg.is_enabled())
            .unwrap_or(false)
    }

    /// Returns browser config if defined
    pub fn browser_settings(&self) -> Option<&BrowserAgentConfig> {
        self.browser_config.as_ref()
    }

    /// Returns the runtime Chromium driver configuration if enabled
    pub fn browser_runtime_config(&self) -> Option<DistriBrowserConfig> {
        self.browser_config.as_ref().map(|cfg| cfg.runtime_config())
    }

    /// Should browser session state be serialized after tool runs
    pub fn should_persist_browser_session(&self) -> bool {
        self.browser_config
            .as_ref()
            .map(|cfg| cfg.should_persist_session())
            .unwrap_or(false)
    }

    /// Check if reflection is enabled (default: false)
    pub fn is_reflection_enabled(&self) -> bool {
        self.enable_reflection.unwrap_or(false)
    }
    /// Check if TODO management functionality is enabled (default: false)
    pub fn is_todos_enabled(&self) -> bool {
        self.enable_todos.unwrap_or(false)
    }

    /// Get the effective context size (agent-level override or model settings)
    pub fn get_effective_context_size(&self) -> u32 {
        self.context_size
            .unwrap_or(self.model_settings.context_size)
    }

    /// Model settings to use for lightweight browser analysis helpers (e.g., observe_summary commands)
    pub fn analysis_model_settings_config(&self) -> ModelSettings {
        self.analysis_model_settings
            .clone()
            .unwrap_or_else(|| self.model_settings.clone())
    }

    /// Whether to include the persistent scratchpad/history in prompts
    pub fn include_scratchpad(&self) -> bool {
        self.include_scratchpad.unwrap_or(true)
    }

    /// Apply definition overrides to this agent definition
    pub fn apply_overrides(&mut self, overrides: DefinitionOverrides) {
        // Override model settings
        if let Some(model) = overrides.model {
            self.model_settings.model = model;
        }

        if let Some(temperature) = overrides.temperature {
            self.model_settings.temperature = temperature;
        }

        if let Some(max_tokens) = overrides.max_tokens {
            self.model_settings.max_tokens = max_tokens;
        }

        // Override max_iterations
        if let Some(max_iterations) = overrides.max_iterations {
            self.max_iterations = Some(max_iterations);
        }

        // Override instructions
        if let Some(instructions) = overrides.instructions {
            self.instructions = instructions;
        }

        if let Some(use_browser) = overrides.use_browser {
            let mut config = self.browser_config.clone().unwrap_or_default();
            config.enabled = use_browser;
            self.browser_config = Some(config);
        }
    }
}

/// Tools configuration for agents
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct ToolsConfig {
    /// Built-in tools to include (e.g., ["final", "transfer_to_agent"])
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub builtin: Vec<String>,

    /// DAP package tools: package_name -> list of tool names
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub packages: std::collections::HashMap<String, Vec<String>>,

    /// MCP server tool configurations
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcp: Vec<McpToolConfig>,

    /// External tools to include from client  
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external: Option<Vec<String>>,
}

/// Where filesystem and artifact tools should execute
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileSystemMode {
    /// Run filesystem/artifact tools on the server (default)
    #[default]
    Remote,
    /// Handle filesystem/artifact tools locally via external tool callbacks
    Local,
}

impl FileSystemMode {
    pub fn include_server_tools(&self) -> bool {
        !matches!(self, FileSystemMode::Local)
    }
}

/// Configuration for tools from an MCP server
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpToolConfig {
    /// Name of the MCP server
    pub server: String,

    /// Include patterns (glob-style, e.g., ["fetch_*", "extract_text"])
    /// Use ["*"] to include all tools from the server
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include: Vec<String>,

    /// Exclude patterns (glob-style, e.g., ["delete_*", "rm_*"])
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpDefinition {
    /// The filter applied to the tools in this MCP definition.
    #[serde(default)]
    pub filter: Option<Vec<String>>,
    /// The name of the MCP server.
    pub name: String,
    /// The type of the MCP server (Tool or Agent).
    #[serde(default)]
    pub r#type: McpServerType,
    /// Authentication configuration for this MCP server.
    #[serde(default)]
    pub auth_config: Option<crate::a2a::SecurityScheme>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum McpServerType {
    #[default]
    Tool,
    Agent,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "lowercase", tag = "name")]
pub enum ModelProvider {
    #[serde(rename = "openai")]
    OpenAI {},
    #[serde(rename = "openai_compat")]
    OpenAICompatible {
        base_url: String,
        api_key: Option<String>,
        project_id: Option<String>,
    },
    #[serde(rename = "vllora")]
    Vllora {
        #[serde(default = "ModelProvider::vllora_url")]
        base_url: String,
    },
}
/// Defines the secret requirements for a provider
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSecretDefinition {
    /// Provider identifier (e.g., "openai", "anthropic")
    pub id: String,
    /// Human-readable label
    pub label: String,
    /// List of required secret keys with metadata
    pub keys: Vec<SecretKeyDefinition>,
}

/// Defines a single secret key requirement
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretKeyDefinition {
    /// The environment variable / secret store key (e.g., "OPENAI_API_KEY")
    pub key: String,
    /// Human-readable label
    pub label: String,
    /// Placeholder for UI input
    pub placeholder: String,
    /// Whether this secret is required (vs optional)
    #[serde(default = "default_required")]
    pub required: bool,
}

fn default_required() -> bool {
    true
}

impl ModelProvider {
    pub fn openai_base_url() -> String {
        "https://api.openai.com/v1".to_string()
    }

    pub fn vllora_url() -> String {
        "http://localhost:9090/v1".to_string()
    }

    /// Returns the provider ID for secret lookup
    pub fn provider_id(&self) -> &'static str {
        match self {
            ModelProvider::OpenAI {} => "openai",
            ModelProvider::OpenAICompatible { .. } => "openai_compat",
            ModelProvider::Vllora { .. } => "vllora",
        }
    }

    /// Returns the required secret keys for this provider
    pub fn required_secret_keys(&self) -> Vec<&'static str> {
        match self {
            ModelProvider::OpenAI {} => vec!["OPENAI_API_KEY"],
            ModelProvider::OpenAICompatible { api_key, .. } => {
                // If api_key is already provided in config, no secret needed
                if api_key.is_some() {
                    vec![]
                } else {
                    vec![] // OpenAI compatible doesn't require secrets if base_url handles auth
                }
            }
            ModelProvider::Vllora { .. } => vec![], // Local server, no API key needed
        }
    }

    /// Returns all provider secret definitions (static registry)
    pub fn all_provider_definitions() -> Vec<ProviderSecretDefinition> {
        vec![
            ProviderSecretDefinition {
                id: "openai".to_string(),
                label: "OpenAI".to_string(),
                keys: vec![SecretKeyDefinition {
                    key: "OPENAI_API_KEY".to_string(),
                    label: "API key".to_string(),
                    placeholder: "sk-...".to_string(),
                    required: true,
                }],
            },
            ProviderSecretDefinition {
                id: "anthropic".to_string(),
                label: "Anthropic".to_string(),
                keys: vec![SecretKeyDefinition {
                    key: "ANTHROPIC_API_KEY".to_string(),
                    label: "API key".to_string(),
                    placeholder: "sk-ant-...".to_string(),
                    required: true,
                }],
            },
            ProviderSecretDefinition {
                id: "gemini".to_string(),
                label: "Google Gemini".to_string(),
                keys: vec![SecretKeyDefinition {
                    key: "GEMINI_API_KEY".to_string(),
                    label: "API key".to_string(),
                    placeholder: "AIza...".to_string(),
                    required: true,
                }],
            },
            ProviderSecretDefinition {
                id: "custom".to_string(),
                label: "Custom".to_string(),
                keys: vec![],
            },
        ]
    }

    /// Get the human-readable name for a provider
    pub fn display_name(&self) -> &'static str {
        match self {
            ModelProvider::OpenAI {} => "OpenAI",
            ModelProvider::OpenAICompatible { .. } => "OpenAI Compatible",
            ModelProvider::Vllora { .. } => "vLLORA",
        }
    }
}

/// Model settings configuration
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelSettings {
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    #[serde(default = "default_context_size")]
    pub context_size: u32,
    #[serde(default = "default_top_p")]
    pub top_p: f32,
    #[serde(default = "default_frequency_penalty")]
    pub frequency_penalty: f32,
    #[serde(default = "default_presence_penalty")]
    pub presence_penalty: f32,
    #[serde(default = "default_model_provider")]
    pub provider: ModelProvider,
    /// Additional parameters for the agent, if any.
    #[serde(default)]
    pub parameters: Option<serde_json::Value>,
    /// The format of the response, if specified.
    #[serde(default)]
    pub response_format: Option<serde_json::Value>,
}

impl Default for ModelSettings {
    fn default() -> Self {
        Self {
            model: "gpt-4.1-mini".to_string(),
            temperature: 0.7,
            max_tokens: 1000,
            context_size: 20000,
            top_p: 1.0,
            frequency_penalty: 0.0,
            presence_penalty: 0.0,
            provider: default_model_provider(),
            parameters: None,
            response_format: None,
        }
    }
}

// Default functions
pub fn default_agent_version() -> Option<String> {
    Some("0.2.2".to_string())
}

fn default_model_provider() -> ModelProvider {
    ModelProvider::OpenAI {}
}

fn default_model() -> String {
    "gpt-4.1-mini".to_string()
}

fn default_temperature() -> f32 {
    0.7
}

fn default_max_tokens() -> u32 {
    1000
}

fn default_context_size() -> u32 {
    20000 // Default limit for general use - agents can override with higher values as needed
}

fn default_top_p() -> f32 {
    1.0
}

fn default_frequency_penalty() -> f32 {
    0.0
}

fn default_presence_penalty() -> f32 {
    0.0
}

fn default_history_size() -> Option<usize> {
    Some(5)
}

impl StandardDefinition {
    pub fn validate(&self) -> anyhow::Result<()> {
        // Basic validation - can be expanded
        if self.name.is_empty() {
            return Err(anyhow::anyhow!("Agent name cannot be empty"));
        }
        Ok(())
    }
}

impl From<StandardDefinition> for LlmDefinition {
    fn from(definition: StandardDefinition) -> Self {
        let mut model_settings = definition.model_settings.clone();
        // Use agent-level context_size override if provided
        if let Some(context_size) = definition.context_size {
            model_settings.context_size = context_size;
        }

        Self {
            name: definition.name,
            model_settings,
            tool_format: definition.tool_format,
        }
    }
}

impl ToolsConfig {
    /// Create a simple configuration with just built-in tools
    pub fn builtin_only(tools: Vec<&str>) -> Self {
        Self {
            builtin: tools.into_iter().map(|s| s.to_string()).collect(),
            packages: std::collections::HashMap::new(),
            mcp: vec![],
            external: None,
        }
    }

    /// Create a configuration that includes all tools from an MCP server
    pub fn mcp_all(server: &str) -> Self {
        Self {
            builtin: vec![],
            packages: std::collections::HashMap::new(),
            mcp: vec![McpToolConfig {
                server: server.to_string(),
                include: vec!["*".to_string()],
                exclude: vec![],
            }],
            external: None,
        }
    }

    /// Create a configuration with specific MCP tool patterns
    pub fn mcp_filtered(server: &str, include: Vec<&str>, exclude: Vec<&str>) -> Self {
        Self {
            builtin: vec![],
            packages: std::collections::HashMap::new(),
            mcp: vec![McpToolConfig {
                server: server.to_string(),
                include: include.into_iter().map(|s| s.to_string()).collect(),
                exclude: exclude.into_iter().map(|s| s.to_string()).collect(),
            }],
            external: None,
        }
    }
}

pub async fn parse_agent_markdown_content(content: &str) -> Result<StandardDefinition, AgentError> {
    // Split by --- to separate TOML frontmatter from markdown content
    let parts: Vec<&str> = content.split("---").collect();

    if parts.len() < 3 {
        return Err(AgentError::Validation(
            "Invalid agent markdown format. Expected TOML frontmatter between --- markers"
                .to_string(),
        ));
    }

    // Parse TOML frontmatter (parts[1] is between the first two --- markers)
    let toml_content = parts[1].trim();
    let mut agent_def: crate::StandardDefinition =
        toml::from_str(toml_content).map_err(|e| AgentError::Validation(e.to_string()))?;

    // Validate agent name format using centralized validation
    if let Err(validation_error) = validate_plugin_name(&agent_def.name) {
        return Err(AgentError::Validation(format!(
            "Invalid agent name '{}': {}",
            agent_def.name, validation_error
        )));
    }

    // Validate that agent name is a valid JavaScript identifier
    if !agent_def
        .name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_')
        || agent_def
            .name
            .chars()
            .next()
            .map_or(false, |c| c.is_numeric())
    {
        return Err(AgentError::Validation(format!(
            "Invalid agent name '{}': Agent names must be valid JavaScript identifiers (alphanumeric + underscores, cannot start with number). \
                Reason: Agent names become function names in TypeScript runtime.",
            agent_def.name
        )));
    }

    // Extract markdown instructions (everything after the second ---)
    let instructions = parts[2..].join("---").trim().to_string();

    // Set the instructions in the agent definition
    agent_def.instructions = instructions;

    Ok(agent_def)
}

/// Validate plugin name follows naming conventions
/// Plugin names must be valid JavaScript identifiers (no hyphens)
pub fn validate_plugin_name(name: &str) -> Result<(), String> {
    if name.contains('-') {
        return Err(format!(
            "Plugin name '{}' cannot contain hyphens. Use underscores instead.",
            name
        ));
    }

    if name.is_empty() {
        return Err("Plugin name cannot be empty".to_string());
    }

    // Check if first character is valid for JavaScript identifier
    if let Some(first_char) = name.chars().next() {
        if !first_char.is_ascii_alphabetic() && first_char != '_' {
            return Err(format!(
                "Plugin name '{}' must start with a letter or underscore",
                name
            ));
        }
    }

    // Check if all characters are valid for JavaScript identifier
    for ch in name.chars() {
        if !ch.is_ascii_alphanumeric() && ch != '_' {
            return Err(format!(
                "Plugin name '{}' can only contain letters, numbers, and underscores",
                name
            ));
        }
    }

    Ok(())
}
