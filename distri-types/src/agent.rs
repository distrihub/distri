use crate::AgentError;
use crate::a2a::AgentSkill;
use crate::browser::{BrowserAgentConfig, BrowsrClientConfig};
use crate::configuration::DefinitionOverrides;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::default::Default;

/// Default timeout for external tool execution in seconds
pub const DEFAULT_EXTERNAL_TOOL_TIMEOUT_SECS: u64 = 120;

/// A reference to a stored skill that an agent can load on demand
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct AvailableSkill {
    /// The skill ID (UUID)
    pub id: String,
    /// Human-readable skill name (for display in the partial)
    pub name: String,
    /// Brief description of what this skill does (shown to the agent)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Unified Agent Strategy Configuration
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
#[derive(Default)]
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
        match self {
            CodeLanguage::Typescript => write!(f, "typescript"),
        }
    }
}

/// Reflection configuration
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct ReflectionConfig {
    /// Whether to enable reflection
    #[serde(default)]
    pub enabled: bool,
    /// Name of the agent definition to use for reflection.
    /// Must be an agent that has the "reflect" tool configured.
    /// If not set, uses the built-in reflection_agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reflection_agent: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_settings: Option<ModelSettings>,
    /// The maximum number of iterations allowed during planning
    #[serde(default = "default_plan_max_iterations")]
    pub max_iterations: usize,
}

impl Default for PlanConfig {
    fn default() -> Self {
        Self {
            model_settings: None,
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
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    #[default]
    None,
    ShortTerm,
    LongTerm,
}

/// How tools are delivered to the LLM in the prompt.
///
/// Controls the tradeoff between prompt size and tool discoverability:
/// - `Full`: All tools get full schemas (classic behavior, largest prompt)
/// - `Deferred`: Core tools get full schemas; others are name+description only
/// - `NamesOnly`: Maximum savings — only core tools have schemas
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ToolDeliveryMode {
    /// All tools get full schemas in the prompt.
    #[serde(alias = "all_tools")]
    Full,
    /// Core tools get full schemas; others get name+description only (default).
    #[default]
    #[serde(alias = "tool_search")]
    Deferred,
    /// Only tool names are listed. Model must use `tool_search` for everything
    /// except core tools. Maximum context savings.
    NamesOnly,
}

/// Which OpenAI-family API format to use when talking to the LLM.
///
/// - `Auto` (default): Auto-detects from the model name. Codex models use Responses API,
///   everything else uses Chat Completions.
/// - `Completions`: Forces the Chat Completions API (`/v1/chat/completions`)
/// - `Responses`: Forces the Responses API (`/v1/responses`)
///
/// Most OpenAI models (GPT-4o, GPT-4.1, GPT-5, o1, etc.) support both APIs.
/// Codex models (`codex-*`, `*-codex`) are Responses API only.
/// OpenAI recommends the Responses API for new projects (better caching, reasoning).
///
/// Can be set at the model_settings level in agent definitions:
/// ```toml
/// [model_settings]
/// model = "codex-mini-latest"
/// api_format = "responses"  # or "completions" or "auto"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum OpenAiApiFormat {
    /// Auto-detect based on model name (codex models → Responses, everything else → Completions)
    #[default]
    Auto,
    /// Chat Completions API (`/v1/chat/completions`)
    Completions,
    /// Responses API (`/v1/responses`) — required for Codex models, recommended for new projects
    Responses,
}

impl OpenAiApiFormat {
    /// Resolve the effective format given a model name.
    /// When `Auto`, inspects the model name to decide.
    pub fn resolve(&self, model: &str) -> ResolvedOpenAiApiFormat {
        match self {
            OpenAiApiFormat::Completions => ResolvedOpenAiApiFormat::Completions,
            OpenAiApiFormat::Responses => ResolvedOpenAiApiFormat::Responses,
            OpenAiApiFormat::Auto => {
                if Self::model_requires_responses_api(model) {
                    ResolvedOpenAiApiFormat::Responses
                } else {
                    ResolvedOpenAiApiFormat::Completions
                }
            }
        }
    }

    /// Heuristic: models that require the Responses API.
    ///
    /// These models return errors on /v1/chat/completions and MUST use /v1/responses:
    /// - Codex models: codex-mini-latest, gpt-5.1-codex, gpt-5.3-codex, etc.
    /// - Pro models: gpt-5-pro, gpt-5.2-pro, gpt-5.4-pro, o3-pro
    /// - Deep research models: o3-deep-research, o4-mini-deep-research
    fn model_requires_responses_api(model: &str) -> bool {
        let m = model.to_lowercase();
        // Codex models (codex-*, *-codex, */codex*)
        m.starts_with("codex")
            || m.ends_with("-codex")
            || m.contains("/codex")
            // Pro models (*-pro) — require multi-turn interactions only Responses supports
            || m.ends_with("-pro")
            // Deep research models (*-deep-research)
            || m.ends_with("-deep-research")
    }
}

/// Resolved (non-Auto) API format
#[derive(Debug, Clone, PartialEq)]
pub enum ResolvedOpenAiApiFormat {
    Completions,
    Responses,
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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LlmDefinition {
    /// The name of the agent.
    pub name: String,
    /// Settings related to the model used by the agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_settings: Option<ModelSettings>,
    /// Tool calling configuration
    #[serde(default)]
    pub tool_format: ToolCallFormat,
    /// How tools are delivered to the LLM (all upfront vs on-demand search)
    #[serde(default)]
    pub tool_delivery_mode: ToolDeliveryMode,
}

impl LlmDefinition {
    /// Get a reference to model_settings.
    /// Returns an error if model_settings is None.
    pub fn ms(&self) -> Result<&ModelSettings, String> {
        self.model_settings.as_ref().ok_or_else(|| {
            "No model configured. Please set a default model in Agent Settings → Default Model."
                .to_string()
        })
    }

    /// Get a mutable reference to model_settings.
    /// Returns an error if model_settings is None.
    pub fn ms_mut(&mut self) -> Result<&mut ModelSettings, String> {
        self.model_settings.as_mut().ok_or_else(|| {
            "No model configured. Please set a default model in Agent Settings → Default Model."
                .to_string()
        })
    }
}

/// Agent definition - complete configuration for an agent
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
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
    /// When `None`, the agent inherits model settings from the orchestrator context defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_settings: Option<ModelSettings>,
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

    /// A2A agent card skills metadata (describes capabilities for agent-to-agent protocol)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills_description: Vec<AgentSkill>,

    /// Skills available for on-demand loading by this agent
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub available_skills: Vec<AvailableSkill>,

    /// List of sub-agents that this agent can transfer control to
    #[serde(default)]
    pub sub_agents: Vec<String>,

    /// Tool calling configuration
    #[serde(default)]
    pub tool_format: ToolCallFormat,

    /// How tools are delivered to the LLM (all upfront vs on-demand search)
    #[serde(default)]
    pub tool_delivery_mode: ToolDeliveryMode,

    /// Tools configuration for this agent
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<ToolsConfig>,

    /// Custom handlebars partials (name -> template path) for use in custom prompts
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub partials: std::collections::HashMap<String, String>,

    /// Whether to write large tool responses to filesystem as artifacts (default: false)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write_large_tool_responses_to_fs: Option<bool>,

    /// Reflection configuration for post-execution analysis using a subagent
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reflection: Option<ReflectionConfig>,
    /// Whether to enable TODO management functionality
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enable_todos: Option<bool>,

    /// Browser configuration for this agent (enables shared Chromium automation)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser_config: Option<BrowserAgentConfig>,

    /// Whether to include shell/code execution tools (start_shell, execute_shell, stop_shell)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_shell: Option<bool>,

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

    /// Whether context compaction is enabled for this agent (default: true)
    #[serde(
        default = "default_compaction_enabled",
        skip_serializing_if = "is_true"
    )]
    pub compaction_enabled: bool,

    /// When true and a sandbox runner is available, this agent runs in an isolated
    /// browsr container with full local tool access (Read, Write, Grep, Bash, etc.)
    /// instead of running in-process. Used for sub-agents that need filesystem access.
    /// Also settable via `--remote` CLI flag or `--overrides '{"remote":true}'`.
    #[serde(default, alias = "deepagent")]
    pub remote: bool,
}
fn default_append_default_instructions() -> Option<bool> {
    Some(true)
}
fn default_include_scratchpad() -> Option<bool> {
    Some(true)
}
fn default_compaction_enabled() -> bool {
    true
}
fn is_true(v: &bool) -> bool {
    *v
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
    pub fn browser_runtime_config(&self) -> Option<BrowsrClientConfig> {
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
        self.reflection.as_ref().map(|r| r.enabled).unwrap_or(false)
    }

    /// Get the reflection configuration, if any
    pub fn reflection_config(&self) -> Option<&ReflectionConfig> {
        self.reflection.as_ref().filter(|r| r.enabled)
    }
    /// Check if TODO management functionality is enabled (default: false)
    pub fn is_todos_enabled(&self) -> bool {
        self.enable_todos.unwrap_or(false)
    }

    /// Check if shell/code execution tools should be included (default: false)
    pub fn should_include_shell(&self) -> bool {
        self.include_shell.unwrap_or(false)
    }

    /// Get model settings if configured.
    pub fn model_settings(&self) -> Option<&ModelSettings> {
        self.model_settings.as_ref()
    }

    /// Get a mutable reference to model settings, if present.
    pub fn model_settings_mut(&mut self) -> Option<&mut ModelSettings> {
        self.model_settings.as_mut()
    }

    /// Get the effective context size (agent-level override or model settings)
    pub fn get_effective_context_size(&self) -> u32 {
        self.context_size
            .filter(|&s| s > 0)
            .or_else(|| {
                self.model_settings()
                    .map(|ms| ms.inner.context_size)
                    .filter(|&s| s > 0)
            })
            .unwrap_or_else(default_context_size)
    }

    /// Model settings to use for lightweight browser analysis helpers (e.g., observe_summary commands)
    pub fn analysis_model_settings_config(&self) -> Option<&ModelSettings> {
        self.analysis_model_settings
            .as_ref()
            .or_else(|| self.model_settings())
    }

    /// Whether to include the persistent scratchpad/history in prompts
    pub fn include_scratchpad(&self) -> bool {
        self.include_scratchpad.unwrap_or(true)
    }

    /// Apply definition overrides to this agent definition
    pub fn apply_overrides(&mut self, overrides: DefinitionOverrides) {
        // Override model settings (only if model_settings already exists)
        if let Some(ref mut ms) = self.model_settings {
            if let Some(model) = overrides.model {
                // Strip provider prefix if present (e.g. "custom_microsoft_foundry/gpt-5.4" → "gpt-5.4")
                ms.model = model
                    .split_once('/')
                    .map(|(_, m)| m.to_string())
                    .unwrap_or(model);
            }
            if let Some(temperature) = overrides.temperature {
                ms.inner.temperature = Some(temperature);
            }
            if let Some(max_tokens) = overrides.max_tokens {
                ms.inner.max_tokens = Some(max_tokens);
            }
        }

        // Override max_iterations
        if let Some(max_iterations) = overrides.max_iterations {
            self.max_iterations = Some(max_iterations);
        }

        // Override instructions
        if let Some(instructions) = overrides.instructions {
            self.instructions = instructions;
        }

        if let Some(remote) = overrides.remote {
            self.remote = remote;
        }

        if let Some(use_browser) = overrides.use_browser {
            let mut config = self.browser_config.clone().unwrap_or_default();
            config.enabled = use_browser;
            self.browser_config = Some(config);
        }

        // Append dynamic tool factories
        if let Some(dynamic_tools) = overrides.dynamic_tools {
            let tools = self.tools.get_or_insert_with(ToolsConfig::default);
            tools.dynamic.extend(dynamic_tools);
        }
    }
}

/// Tools configuration for agents
/// Canonical list of valid builtin tool names.
///
/// Includes both server-executed tools (search, start_shell, etc.) and
/// client-executed tools (http_request). Agent definitions reference these
/// by name in `tools.builtin = [...]`.
pub const VALID_BUILTIN_TOOLS: &[&str] = &[
    // Agent control
    "final",
    "reflect",
    "transfer_to_agent",
    // Browser & scraping
    "browsr_scrape",
    "browsr_browser",
    "browsr_crawl",
    "browser_step",
    "search",
    // Shell
    "start_shell",
    "execute_shell",
    "stop_shell",
    // Code execution
    "distri_execute_code",
    // Tool discovery
    "tool_search",
    "load_skill",
    // Connection & secrets
    "inject_connection_env",
    // Logging
    "console_log",
    // Artifacts & filesystem
    "artifact_tool",
    // Todos
    "todos",
];

/// Tools that always get full schemas, never deferred.
/// These are the most commonly used tools that agents need immediately.
pub const CORE_TOOLS: &[&str] = &[
    "final",
    "transfer_to_agent",
    "tool_search",
    "write_todos",
    "execute_shell",
    "start_shell",
    "load_skill",
];

/// Default threshold: defer tools when total count exceeds this.
pub const DEFAULT_DEFERRED_THRESHOLD: usize = 15;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct ToolsConfig {
    /// Built-in tools to include (e.g., ["final", "transfer_to_agent"])
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub builtin: Vec<String>,

    /// Dynamic tool factories — each creates a named tool at runtime.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dynamic: Vec<crate::dynamic_tool::DynamicToolFactory>,

    /// MCP server tool configurations
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcp: Vec<McpToolConfig>,

    /// External tools to include from client
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external: Option<Vec<String>>,

    /// How tools are delivered to the model. Defaults to `Full`.
    /// When set to `Deferred`, only core tools get full schemas;
    /// others appear as name+description and must be fetched via `tool_search`.
    #[serde(default, skip_serializing_if = "is_default_delivery_mode")]
    pub delivery_mode: ToolDeliveryMode,

    /// Tool count threshold for automatic deferral.
    /// When `delivery_mode` is `Deferred` and total tools exceed this,
    /// non-core tools are deferred. Default: 15.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deferred_threshold: Option<usize>,

    /// Additional tool names to always include with full schemas (beyond CORE_TOOLS).
    /// Useful for agent-specific tools that should never be deferred.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub always_full_schema: Vec<String>,
}

fn is_default_delivery_mode(mode: &ToolDeliveryMode) -> bool {
    *mode == ToolDeliveryMode::Deferred
}

impl ToolsConfig {
    /// Validate that all builtin tool names are recognized.
    /// Returns a list of invalid tool names, or empty if all are valid.
    pub fn invalid_builtin_tools(&self) -> Vec<String> {
        self.builtin
            .iter()
            .filter(|name| !VALID_BUILTIN_TOOLS.contains(&name.as_str()))
            .cloned()
            .collect()
    }

    /// Whether a tool should always get a full schema (never deferred).
    pub fn is_core_tool(&self, name: &str) -> bool {
        CORE_TOOLS.contains(&name)
            || self.always_full_schema.iter().any(|n| n == name)
            // call_* agent tools are always core (the model needs to know how to call sub-agents)
            || name.starts_with("call_")
    }

    /// Effective threshold for automatic tool deferral.
    pub fn effective_threshold(&self) -> usize {
        self.deferred_threshold
            .unwrap_or(DEFAULT_DEFERRED_THRESHOLD)
    }

    /// Determine the effective delivery mode given the total tool count.
    /// If mode is `Full` but tool count exceeds threshold, stays `Full`
    /// Deferred always stays Deferred — context efficiency is the default.
    pub fn effective_delivery_mode(&self, _total_tools: usize) -> ToolDeliveryMode {
        self.delivery_mode.clone()
    }
}

/// Where filesystem and artifact tools should execute.
/// Deprecated: filesystem tools are no longer included as server builtins.

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
    #[serde(rename = "azure_openai")]
    AzureOpenAI {
        base_url: String,
        api_key: Option<String>,
        deployment: String,
        #[serde(default = "ModelProvider::azure_api_version")]
        api_version: String,
    },
    #[serde(rename = "anthropic")]
    Anthropic {
        #[serde(default = "ModelProvider::anthropic_base_url")]
        base_url: Option<String>,
        api_key: Option<String>,
    },
    #[serde(rename = "gemini")]
    Gemini {
        #[serde(default = "ModelProvider::gemini_base_url")]
        base_url: String,
        api_key: Option<String>,
    },
    #[serde(rename = "azure_ai_foundry")]
    AzureAiFoundry {
        base_url: String,
        api_key: Option<String>,
    },
    #[serde(rename = "aws_bedrock")]
    AwsBedrock {
        base_url: String,
        api_key: Option<String>,
    },
    #[serde(rename = "google_vertex")]
    GoogleVertex {
        base_url: String,
        api_key: Option<String>,
        project_id: Option<String>,
    },
    #[serde(rename = "alibaba_cloud")]
    AlibabaCloud {
        #[serde(default = "ModelProvider::alibaba_cloud_base_url")]
        base_url: String,
        api_key: Option<String>,
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
    /// Whether this field contains sensitive data (masked in UI, stored encrypted).
    /// Defaults to true. Set to false for non-sensitive config like URLs, project IDs.
    #[serde(default = "default_sensitive")]
    pub sensitive: bool,
}

fn default_required() -> bool {
    true
}

fn default_sensitive() -> bool {
    true
}

/// A model entry within a provider
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    /// Model identifier (e.g., "gpt-4o", "claude-sonnet-4")
    pub id: String,
    /// Human-readable name
    pub name: String,
}

/// Combined provider definition used in default_models.json.
/// Merges secret key definitions and well-known models into one entry per provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DefaultProviderEntry {
    id: String,
    label: String,
    keys: Vec<SecretKeyDefinition>,
    models: Vec<crate::models::Model>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DefaultModelsFile {
    providers: Vec<DefaultProviderEntry>,
}

fn load_default_providers() -> &'static [DefaultProviderEntry] {
    use std::sync::OnceLock;
    static PROVIDERS: OnceLock<Vec<DefaultProviderEntry>> = OnceLock::new();
    PROVIDERS.get_or_init(|| {
        let json = include_str!("default_models.json");
        let file: DefaultModelsFile =
            serde_json::from_str(json).expect("Failed to parse default_models.json");
        file.providers
    })
}

/// Models grouped by provider, with configuration status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderModels {
    /// Provider identifier
    pub provider_id: String,
    /// Human-readable provider name
    pub provider_label: String,
    /// Available models for this provider
    pub models: Vec<crate::models::Model>,
}

/// Provider models with configuration status (returned by API)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderModelsStatus {
    /// Provider identifier
    pub provider_id: String,
    /// Human-readable provider name
    pub provider_label: String,
    /// Whether the provider's API key is configured
    pub configured: bool,
    /// Available models for this provider
    pub models: Vec<crate::models::Model>,
}

impl Default for ModelProvider {
    fn default() -> Self {
        ModelProvider::OpenAI {}
    }
}

impl ModelProvider {
    pub fn openai_base_url() -> String {
        "https://api.openai.com/v1".to_string()
    }

    pub fn anthropic_base_url() -> Option<String> {
        None
    }

    pub fn gemini_base_url() -> String {
        "https://generativelanguage.googleapis.com/v1beta/openai".to_string()
    }

    pub fn azure_api_version() -> String {
        "2024-06-01".to_string()
    }

    pub fn alibaba_cloud_base_url() -> String {
        "https://dashscope-intl.aliyuncs.com/compatible-mode/v1".to_string()
    }

    /// Returns the provider type enum for this provider.
    pub fn provider_type(&self) -> crate::models::ProviderType {
        match self {
            ModelProvider::OpenAI {} => crate::models::ProviderType::OpenAI,
            ModelProvider::OpenAICompatible { .. } => {
                crate::models::ProviderType::Custom("openai_compat".to_string())
            }
            ModelProvider::AzureOpenAI { .. } => crate::models::ProviderType::Azure,
            ModelProvider::Anthropic { .. } => crate::models::ProviderType::Anthropic,
            ModelProvider::Gemini { .. } => crate::models::ProviderType::Gemini,
            ModelProvider::AzureAiFoundry { .. } => crate::models::ProviderType::AzureAiFoundry,
            ModelProvider::AwsBedrock { .. } => crate::models::ProviderType::AwsBedrock,
            ModelProvider::GoogleVertex { .. } => crate::models::ProviderType::GoogleVertex,
            ModelProvider::AlibabaCloud { .. } => crate::models::ProviderType::AlibabaCloud,
        }
    }

    /// Returns the provider ID string for secret lookup and "provider/model" format.
    pub fn provider_id(&self) -> &str {
        match self {
            ModelProvider::OpenAI {} => "openai",
            ModelProvider::OpenAICompatible { .. } => "openai_compat",
            ModelProvider::AzureOpenAI { .. } => "azure_openai",
            ModelProvider::Anthropic { .. } => "anthropic",
            ModelProvider::Gemini { .. } => "gemini",
            ModelProvider::AzureAiFoundry { .. } => "azure_ai_foundry",
            ModelProvider::AwsBedrock { .. } => "aws_bedrock",
            ModelProvider::GoogleVertex { .. } => "google_vertex",
            ModelProvider::AlibabaCloud { .. } => "alibaba_cloud",
        }
    }

    /// Returns the required secret keys for this provider.
    pub fn required_secret_keys(&self) -> Vec<&'static str> {
        match self {
            ModelProvider::OpenAI {} => vec!["OPENAI_API_KEY"],
            ModelProvider::OpenAICompatible { api_key, .. } => {
                if api_key.is_some() {
                    vec![]
                } else {
                    vec!["OPENAI_API_KEY"]
                }
            }
            ModelProvider::AzureOpenAI { api_key, .. } => {
                if api_key.is_some() {
                    vec![]
                } else {
                    vec!["AZURE_OPENAI_API_KEY"]
                }
            }
            ModelProvider::Anthropic { api_key, .. } => {
                if api_key.is_some() {
                    vec![]
                } else {
                    vec!["ANTHROPIC_API_KEY"]
                }
            }
            ModelProvider::Gemini { api_key, .. } => {
                if api_key.is_some() {
                    vec![]
                } else {
                    vec!["GEMINI_API_KEY"]
                }
            }
            ModelProvider::AzureAiFoundry { api_key, .. } => {
                if api_key.is_some() {
                    vec![]
                } else {
                    vec!["AZURE_AI_FOUNDRY_API_KEY"]
                }
            }
            ModelProvider::AwsBedrock { api_key, .. } => {
                if api_key.is_some() {
                    vec![]
                } else {
                    vec!["AWS_ACCESS_KEY_ID"]
                }
            }
            ModelProvider::GoogleVertex { api_key, .. } => {
                if api_key.is_some() {
                    vec![]
                } else {
                    vec!["GOOGLE_VERTEX_API_KEY"]
                }
            }
            ModelProvider::AlibabaCloud { api_key, .. } => {
                if api_key.is_some() {
                    vec![]
                } else {
                    vec!["DASHSCOPE_API_KEY"]
                }
            }
        }
    }

    /// Returns all provider secret definitions, loaded from default_models.json.
    pub fn all_provider_definitions() -> Vec<ProviderSecretDefinition> {
        load_default_providers()
            .iter()
            .map(|p| ProviderSecretDefinition {
                id: p.id.clone(),
                label: p.label.clone(),
                keys: p.keys.clone(),
            })
            .collect()
    }

    /// Returns the well-known models grouped by provider, loaded from default_models.json.
    pub fn well_known_models() -> Vec<ProviderModels> {
        load_default_providers()
            .iter()
            .filter(|p| !p.models.is_empty())
            .map(|p| ProviderModels {
                provider_id: p.id.clone(),
                provider_label: p.label.clone(),
                models: p.models.clone(),
            })
            .collect()
    }

    /// Get the human-readable name for a provider
    pub fn display_name(&self) -> &'static str {
        match self {
            ModelProvider::OpenAI {} => "OpenAI",
            ModelProvider::OpenAICompatible { .. } => "OpenAI Compatible",
            ModelProvider::AzureOpenAI { .. } => "Azure",
            ModelProvider::Anthropic { .. } => "Anthropic",
            ModelProvider::Gemini { .. } => "Google Gemini",
            ModelProvider::AzureAiFoundry { .. } => "Azure AI Foundry",
            ModelProvider::AwsBedrock { .. } => "AWS Bedrock",
            ModelProvider::GoogleVertex { .. } => "Google Vertex AI",
            ModelProvider::AlibabaCloud { .. } => "Alibaba Cloud",
        }
    }

    /// OTel `gen_ai.provider.name` attribute value for this provider.
    /// Uses the semantic convention identifiers from the 2025 GenAI spec.
    pub fn otel_provider_name(&self) -> &'static str {
        match self {
            ModelProvider::OpenAI { .. } => "openai",
            ModelProvider::OpenAICompatible { .. } => "openai",
            ModelProvider::AzureOpenAI { .. } => "azure.ai.openai",
            ModelProvider::Anthropic { .. } => "anthropic",
            ModelProvider::Gemini { .. } => "google.gemini",
            ModelProvider::AzureAiFoundry { .. } => "azure.ai.inference",
            ModelProvider::AwsBedrock { .. } => "aws.bedrock",
            ModelProvider::GoogleVertex { .. } => "gcp.vertex_ai",
            ModelProvider::AlibabaCloud { .. } => "alibaba_cloud",
        }
    }
}

/// Model settings configuration.
/// A `ModelSettings` always has a valid model string.
/// Use `Option<ModelSettings>` when no model is configured yet.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ModelSettings {
    pub model: String,
    #[serde(flatten)]
    pub inner: ModelSettingsInner,
}

/// Optional/defaultable model parameters. Split from `ModelSettings` so callers
/// can construct `ModelSettings { model: "...", ..Default::default() }` easily
/// via the `inner` field having `Default`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct ModelSettingsInner {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(default = "default_context_size")]
    pub context_size: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<f32>,
    #[serde(default = "default_model_provider")]
    pub provider: ModelProvider,
    /// Additional parameters for the agent, if any.
    #[serde(default)]
    pub parameters: Option<serde_json::Value>,
    /// The format of the response, if specified.
    #[serde(default)]
    pub response_format: Option<serde_json::Value>,
    /// Which OpenAI-family API format to use (auto-detected by default).
    /// Only relevant for OpenAI, OpenAI-compatible, and Azure OpenAI providers.
    #[serde(default, skip_serializing_if = "is_default_api_format")]
    pub api_format: OpenAiApiFormat,
}

impl ModelSettings {
    /// Create a new `ModelSettings` with the given model and default inner settings.
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            inner: ModelSettingsInner::default(),
        }
    }

    /// Parse a "provider/model" string (e.g. "anthropic/claude-sonnet-4") into ModelSettings.
    /// Returns None if the format is invalid.
    /// For custom providers (prefixed with "custom_"), returns an OpenAICompatible provider
    /// with empty base_url/api_key — the caller must fill these from secrets/config.
    /// Parse "provider/model" string into ModelSettings.
    /// Returns Err with a descriptive message if the provider is not recognized.
    /// Returns Ok(None) if the input is empty or has no slash.
    pub fn from_provider_model_str(s: &str) -> Result<Option<Self>, String> {
        let Some((provider_str, model_id)) = s.split_once('/') else {
            return Ok(None);
        };
        if model_id.is_empty() {
            return Ok(None);
        }
        let provider = match provider_str {
            "openai" => ModelProvider::OpenAI {},
            "anthropic" => ModelProvider::Anthropic {
                base_url: None,
                api_key: None,
            },
            "azure_openai" | "azure" => ModelProvider::AzureOpenAI {
                base_url: String::new(),
                api_key: None,
                deployment: model_id.to_string(),
                api_version: ModelProvider::azure_api_version(),
            },
            "gemini" => ModelProvider::Gemini {
                base_url: ModelProvider::gemini_base_url(),
                api_key: None,
            },
            "azure_ai_foundry" => ModelProvider::AzureAiFoundry {
                base_url: String::new(),
                api_key: None,
            },
            "aws_bedrock" => ModelProvider::AwsBedrock {
                base_url: String::new(),
                api_key: None,
            },
            "google_vertex" => ModelProvider::GoogleVertex {
                base_url: String::new(),
                api_key: None,
                project_id: None,
            },
            "alibaba_cloud" => ModelProvider::AlibabaCloud {
                base_url: ModelProvider::alibaba_cloud_base_url(),
                api_key: None,
            },
            _ if provider_str.starts_with("custom_") => ModelProvider::OpenAICompatible {
                base_url: String::new(),
                api_key: None,
                project_id: None,
            },
            // Unknown providers — treat as OpenAI-compatible
            _ => ModelProvider::OpenAICompatible {
                base_url: String::new(),
                api_key: None,
                project_id: None,
            },
        };
        Ok(Some(Self {
            model: model_id.to_string(),
            inner: ModelSettingsInner {
                provider,
                ..Default::default()
            },
        }))
    }
}

// Default functions
pub fn default_agent_version() -> Option<String> {
    Some("0.2.2".to_string())
}

fn default_model_provider() -> ModelProvider {
    ModelProvider::OpenAI {}
}

fn default_context_size() -> u32 {
    20000 // Default limit for general use - agents can override with higher values as needed
}

fn is_default_api_format(f: &OpenAiApiFormat) -> bool {
    *f == OpenAiApiFormat::Auto
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

        // Validate reflection configuration
        if let Some(ref reflection) = self.reflection
            && reflection.enabled
        {
            // If a custom reflection_agent is specified, validate the name
            if let Some(ref agent_name) = reflection.reflection_agent
                && agent_name.is_empty()
            {
                return Err(anyhow::anyhow!(
                    "Reflection agent name cannot be empty when specified"
                ));
            }
        }

        Ok(())
    }

    /// Validate that a reflection agent definition has the "reflect" tool configured.
    /// This is called at registration time when we have access to the full agent config.
    pub fn validate_reflection_agent(agent_def: &StandardDefinition) -> anyhow::Result<()> {
        let has_reflect_tool = agent_def
            .tools
            .as_ref()
            .map(|t| t.builtin.iter().any(|name| name == "reflect"))
            .unwrap_or(false);

        if !has_reflect_tool {
            // The built-in reflection_agent gets the reflect tool automatically,
            // but custom reflection agents must explicitly list it
            anyhow::bail!(
                "Reflection agent '{}' must have the 'reflect' tool in its tools.builtin configuration",
                agent_def.name
            );
        }

        Ok(())
    }
}

impl From<StandardDefinition> for LlmDefinition {
    fn from(definition: StandardDefinition) -> Self {
        let model_settings = match (definition.model_settings, definition.context_size) {
            (Some(mut ms), Some(ctx)) => {
                ms.inner.context_size = ctx;
                Some(ms)
            }
            (ms, _) => ms,
        };

        Self {
            name: definition.name,
            model_settings,
            tool_format: definition.tool_format,
            tool_delivery_mode: definition.tool_delivery_mode,
        }
    }
}

impl ToolsConfig {
    /// Create a simple configuration with just built-in tools
    pub fn builtin_only(tools: Vec<&str>) -> Self {
        Self {
            builtin: tools.into_iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    /// Create a configuration that includes all tools from an MCP server
    pub fn mcp_all(server: &str) -> Self {
        Self {
            mcp: vec![McpToolConfig {
                server: server.to_string(),
                include: vec!["*".to_string()],
                exclude: vec![],
            }],
            ..Default::default()
        }
    }

    /// Create a configuration with specific MCP tool patterns
    pub fn mcp_filtered(server: &str, include: Vec<&str>, exclude: Vec<&str>) -> Self {
        Self {
            mcp: vec![McpToolConfig {
                server: server.to_string(),
                include: include.into_iter().map(|s| s.to_string()).collect(),
                exclude: exclude.into_iter().map(|s| s.to_string()).collect(),
            }],
            ..Default::default()
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
            .is_some_and(|c| c.is_numeric())
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
    if let Some(first_char) = name.chars().next()
        && !first_char.is_ascii_alphabetic()
        && first_char != '_'
    {
        return Err(format!(
            "Plugin name '{}' must start with a letter or underscore",
            name
        ));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compaction_enabled_defaults_to_true_via_serde() {
        // serde default uses default_compaction_enabled() -> true
        let json = r#"{"name": "test"}"#;
        let def: StandardDefinition = serde_json::from_str(json).unwrap();
        assert!(def.compaction_enabled);
    }

    #[test]
    fn test_compaction_enabled_deserializes_true_when_absent() {
        let json = r#"{"name": "test", "description": "test agent"}"#;
        let def: StandardDefinition = serde_json::from_str(json).unwrap();
        assert!(def.compaction_enabled);
    }

    #[test]
    fn test_compaction_enabled_deserializes_false() {
        let json = r#"{"name": "test", "description": "test agent", "compaction_enabled": false}"#;
        let def: StandardDefinition = serde_json::from_str(json).unwrap();
        assert!(!def.compaction_enabled);
    }

    #[test]
    fn test_compaction_enabled_true_skipped_in_serialization() {
        let def = StandardDefinition {
            name: "test".to_string(),
            compaction_enabled: true,
            ..Default::default()
        };
        let json = serde_json::to_string(&def).unwrap();
        assert!(!json.contains("compaction_enabled"));
    }

    #[test]
    fn test_compaction_enabled_false_serialized() {
        let def = StandardDefinition {
            name: "test".to_string(),
            compaction_enabled: false,
            ..Default::default()
        };
        let json = serde_json::to_string(&def).unwrap();
        assert!(json.contains("\"compaction_enabled\":false"));
    }

    #[test]
    fn test_max_tokens_optional_defaults_to_none() {
        let def = StandardDefinition::default();
        assert!(def.model_settings().is_none());
    }

    #[test]
    fn test_max_tokens_deserializes_when_present() {
        let json =
            r#"{"name": "test", "model_settings": {"model": "gpt-4.1", "max_tokens": 4096}}"#;
        let def: StandardDefinition = serde_json::from_str(json).unwrap();
        assert_eq!(def.model_settings().unwrap().inner.max_tokens, Some(4096));
    }

    #[test]
    fn test_max_tokens_none_when_absent() {
        let json = r#"{"name": "test", "model_settings": {"model": "gpt-4.1"}}"#;
        let def: StandardDefinition = serde_json::from_str(json).unwrap();
        assert!(def.model_settings().unwrap().inner.max_tokens.is_none());
    }

    #[test]
    fn test_max_tokens_none_skipped_in_serialization() {
        let settings = ModelSettings {
            model: "test-model".to_string(),
            inner: ModelSettingsInner {
                max_tokens: None,
                provider: ModelProvider::OpenAI {},
                ..Default::default()
            },
        };
        let json = serde_json::to_string(&settings).unwrap();
        assert!(!json.contains("max_tokens"));
    }

    #[test]
    fn test_max_tokens_some_serialized() {
        let settings = ModelSettings {
            model: "test-model".to_string(),
            inner: ModelSettingsInner {
                max_tokens: Some(2048),
                provider: ModelProvider::OpenAI {},
                ..Default::default()
            },
        };
        let json = serde_json::to_string(&settings).unwrap();
        assert!(json.contains("\"max_tokens\":2048"));
    }

    #[test]
    fn test_api_format_auto_detect_codex_prefix() {
        let fmt = OpenAiApiFormat::Auto;
        assert_eq!(
            fmt.resolve("codex-mini-latest"),
            ResolvedOpenAiApiFormat::Responses
        );
        assert_eq!(
            fmt.resolve("codex-mini-2025-01-24"),
            ResolvedOpenAiApiFormat::Responses
        );
    }

    #[test]
    fn test_api_format_auto_detect_codex_suffix() {
        let fmt = OpenAiApiFormat::Auto;
        assert_eq!(
            fmt.resolve("gpt-5.1-codex"),
            ResolvedOpenAiApiFormat::Responses
        );
        assert_eq!(
            fmt.resolve("gpt-5.3-codex"),
            ResolvedOpenAiApiFormat::Responses
        );
    }

    #[test]
    fn test_api_format_auto_detect_pro_models() {
        let fmt = OpenAiApiFormat::Auto;
        assert_eq!(fmt.resolve("gpt-5-pro"), ResolvedOpenAiApiFormat::Responses);
        assert_eq!(
            fmt.resolve("gpt-5.2-pro"),
            ResolvedOpenAiApiFormat::Responses
        );
        assert_eq!(
            fmt.resolve("gpt-5.4-pro"),
            ResolvedOpenAiApiFormat::Responses
        );
        assert_eq!(fmt.resolve("o3-pro"), ResolvedOpenAiApiFormat::Responses);
    }

    #[test]
    fn test_api_format_auto_detect_deep_research_models() {
        let fmt = OpenAiApiFormat::Auto;
        assert_eq!(
            fmt.resolve("o3-deep-research"),
            ResolvedOpenAiApiFormat::Responses
        );
        assert_eq!(
            fmt.resolve("o4-mini-deep-research"),
            ResolvedOpenAiApiFormat::Responses
        );
    }

    #[test]
    fn test_api_format_auto_detect_non_codex() {
        let fmt = OpenAiApiFormat::Auto;
        assert_eq!(fmt.resolve("gpt-4o"), ResolvedOpenAiApiFormat::Completions);
        assert_eq!(fmt.resolve("gpt-4.1"), ResolvedOpenAiApiFormat::Completions);
        assert_eq!(fmt.resolve("gpt-5"), ResolvedOpenAiApiFormat::Completions);
        assert_eq!(fmt.resolve("o1"), ResolvedOpenAiApiFormat::Completions);
        assert_eq!(
            fmt.resolve("gpt-5.4-mini"),
            ResolvedOpenAiApiFormat::Completions
        );
        assert_eq!(fmt.resolve("o3-mini"), ResolvedOpenAiApiFormat::Completions);
    }

    #[test]
    fn test_api_format_explicit_override() {
        // Explicit Responses overrides auto-detect even for non-codex models
        assert_eq!(
            OpenAiApiFormat::Responses.resolve("gpt-4o"),
            ResolvedOpenAiApiFormat::Responses
        );
        // Explicit Completions overrides auto-detect even for codex models
        assert_eq!(
            OpenAiApiFormat::Completions.resolve("codex-mini-latest"),
            ResolvedOpenAiApiFormat::Completions
        );
    }

    #[test]
    fn test_api_format_defaults_to_auto() {
        let inner = ModelSettingsInner::default();
        assert_eq!(inner.api_format, OpenAiApiFormat::Auto);
    }

    #[test]
    fn test_api_format_auto_skipped_in_serialization() {
        let settings = ModelSettings {
            model: "test-model".to_string(),
            inner: ModelSettingsInner {
                provider: ModelProvider::OpenAI {},
                ..Default::default()
            },
        };
        let json = serde_json::to_string(&settings).unwrap();
        assert!(!json.contains("api_format"));
    }

    #[test]
    fn test_api_format_responses_serialized() {
        let settings = ModelSettings {
            model: "test-model".to_string(),
            inner: ModelSettingsInner {
                api_format: OpenAiApiFormat::Responses,
                provider: ModelProvider::OpenAI {},
                ..Default::default()
            },
        };
        let json = serde_json::to_string(&settings).unwrap();
        assert!(json.contains("\"api_format\":\"responses\""));
    }

    #[test]
    fn test_api_format_deserializes_from_toml() {
        let toml_str = r#"
            model = "codex-mini-latest"
            api_format = "responses"
            [provider]
            name = "openai"
        "#;
        let settings: ModelSettings = toml::from_str(toml_str).unwrap();
        assert_eq!(settings.inner.api_format, OpenAiApiFormat::Responses);
    }

    // ── ToolDeliveryMode tests ────────────────────────────────────

    #[test]
    fn test_tool_delivery_mode_defaults_to_deferred() {
        let mode: ToolDeliveryMode = Default::default();
        assert_eq!(mode, ToolDeliveryMode::Deferred);
    }

    #[test]
    fn test_tool_delivery_mode_backwards_compat_all_tools() {
        // Old configs that used "all_tools" should deserialize to Full
        let json = r#""all_tools""#;
        let mode: ToolDeliveryMode = serde_json::from_str(json).unwrap();
        assert_eq!(mode, ToolDeliveryMode::Full);
    }

    #[test]
    fn test_tool_delivery_mode_backwards_compat_tool_search() {
        // Old configs that used "tool_search" should deserialize to Deferred
        let json = r#""tool_search""#;
        let mode: ToolDeliveryMode = serde_json::from_str(json).unwrap();
        assert_eq!(mode, ToolDeliveryMode::Deferred);
    }

    #[test]
    fn test_tools_config_is_core_tool() {
        let config = ToolsConfig::default();
        assert!(config.is_core_tool("final"));
        assert!(config.is_core_tool("tool_search"));
        assert!(config.is_core_tool("execute_shell"));
        assert!(config.is_core_tool("call_coder"));
        assert!(!config.is_core_tool("browsr_scrape"));
    }

    #[test]
    fn test_tools_config_always_full_schema() {
        let config = ToolsConfig {
            always_full_schema: vec!["browsr_scrape".to_string()],
            ..Default::default()
        };
        assert!(config.is_core_tool("browsr_scrape"));
        assert!(!config.is_core_tool("browsr_browser"));
    }

    #[test]
    fn test_effective_delivery_mode_full_stays_full() {
        let config = ToolsConfig {
            delivery_mode: ToolDeliveryMode::Full,
            ..Default::default()
        };
        // Even with many tools, Full stays Full
        assert_eq!(config.effective_delivery_mode(100), ToolDeliveryMode::Full);
    }

    #[test]
    fn test_effective_delivery_mode_deferred_stays_deferred() {
        let config = ToolsConfig {
            delivery_mode: ToolDeliveryMode::Deferred,
            deferred_threshold: Some(20),
            ..Default::default()
        };
        // Deferred always stays Deferred regardless of count
        assert_eq!(
            config.effective_delivery_mode(10),
            ToolDeliveryMode::Deferred
        );
    }

    #[test]
    fn test_effective_delivery_mode_deferred_over_threshold() {
        let config = ToolsConfig {
            delivery_mode: ToolDeliveryMode::Deferred,
            deferred_threshold: Some(10),
            ..Default::default()
        };
        // Over threshold: stays Deferred
        assert_eq!(
            config.effective_delivery_mode(15),
            ToolDeliveryMode::Deferred
        );
    }
}
