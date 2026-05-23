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

/// Runtime environment in which the agent is executing.
/// Determines which built-in agent variants and tools are available.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeMode {
    /// Running from distri-cli with local filesystem access
    Cli,
    /// Running on distri-cloud server (browsr sandbox for code execution)
    #[default]
    Cloud,
    /// Running in browser with IndexedDB filesystem
    Browser,
}

impl RuntimeMode {
    /// Canonical wire/template name (matches `#[serde(rename_all =
    /// "snake_case")]`). Single source of truth for template
    /// substitution (`{{runtime_mode}}` / `{{#if (eq runtime_mode
    /// "cli")}}`), span attributes, and any string-keyed runtime
    /// dispatch table. Both
    /// `agent::strategy::planning::formatter` and
    /// `tools::skill_script::LoadSkillTool` use this so a future
    /// rename can't desync the system prompt from the skill body.
    pub fn as_template_name(&self) -> &'static str {
        match self {
            Self::Cli => "cli",
            Self::Cloud => "cloud",
            Self::Browser => "browser",
        }
    }
}

/// Agent definition - complete configuration for an agent
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct StandardDefinition {
    /// The name of the agent.
    pub name: String,
    /// A brief description of the agent's purpose.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,

    /// The version of the agent. Runtime falls back to `default_agent_version()`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,

    /// Instructions for the agent - serves as an introduction defining what the agent is and does.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub instructions: String,

    /// Settings related to the model used by the agent.
    /// When `None`, the agent inherits model settings from the orchestrator context defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_settings: Option<ModelSettings>,
    /// Optional lower-level model settings for lightweight analysis helpers
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analysis_model_settings: Option<ModelSettings>,

    /// The size of the history to maintain for the agent. Runtime falls back to `default_history_size()`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_size: Option<usize>,
    /// The new strategy configuration for the agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy: Option<AgentStrategy>,
    /// A2A-specific fields
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,

    /// Channel slash commands this agent exposes. Each command is a preset
    /// prompt — invoking it on a bot sends `prompt` to the agent. Compiled
    /// into the gateway's `CommandRouter` alongside a `WorkflowAgent`'s
    /// entry-point commands.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<crate::channel_commands::SlashCommand>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_iterations: Option<usize>,

    /// A2A agent card skills metadata (describes capabilities for agent-to-agent protocol)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills_description: Vec<AgentSkill>,

    /// Skills available for on-demand loading by this agent
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub available_skills: Vec<AvailableSkill>,

    /// Connections this agent needs to function. Resolved at run start from the
    /// workspace's connections: OAuth tokens / custom secrets / distri-native
    /// sessions are injected into `ExecutorContext.env_vars` and surfaced to
    /// the model via the `{{> connections}}` partial. Agents without declared
    /// connections get neither env vars nor the partial.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connections: Vec<crate::connections::ConnectionRequirement>,

    /// List of sub-agents that this agent can transfer control to
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sub_agents: Vec<String>,

    /// Tool calling configuration
    #[serde(default, skip_serializing_if = "is_default_tool_format")]
    pub tool_format: ToolCallFormat,

    /// How tools are delivered to the LLM (all upfront vs on-demand search)
    #[serde(default, skip_serializing_if = "is_default_tool_delivery_mode")]
    pub tool_delivery_mode: ToolDeliveryMode,

    /// Tools configuration for this agent
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<ToolsConfig>,

    /// Custom handlebars partials (name -> template path) for use in custom prompts
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub partials: std::collections::HashMap<String, String>,

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

    /// Runtime constraint for this agent. Like Docker's `platforms` field:
    ///
    /// - empty / omitted → runs in any runtime (default).
    /// - `["cli"]` → only runs when `ExecutorContext.runtime_mode == Cli`,
    ///   OR via a `RemoteTaskRunner` providing `Cli` (e.g. `SandboxLauncher`
    ///   spawning `distri-cli` inside a browsr container).
    /// - `["cli", "cloud"]` → runs in either Cli or Cloud, but not Browser.
    ///
    /// When the current runtime doesn't match any allowed value and no
    /// compatible runner exists, the orchestrator fails fast at request entry.
    ///
    /// Accepts both scalar (`runtime = "cli"`) and array (`runtime = ["cli"]`)
    /// syntax in TOML/JSON for ergonomics.
    #[serde(
        default,
        deserialize_with = "deserialize_runtime_modes",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub runtime: Vec<RuntimeMode>,
}

/// Accept either a single `RuntimeMode` string or an array of them.
fn deserialize_runtime_modes<'de, D>(deserializer: D) -> Result<Vec<RuntimeMode>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{self, Deserialize};

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(RuntimeMode),
        Many(Vec<RuntimeMode>),
    }

    match Option::<OneOrMany>::deserialize(deserializer)? {
        None => Ok(Vec::new()),
        Some(OneOrMany::One(rt)) => Ok(vec![rt]),
        Some(OneOrMany::Many(v)) => {
            // Reject duplicates so authors notice typos like ["cli", "cli"].
            let mut seen = std::collections::HashSet::new();
            for rt in &v {
                let key = format!("{:?}", rt);
                if !seen.insert(key) {
                    return Err(de::Error::custom(format!(
                        "duplicate runtime entry: {:?}",
                        rt
                    )));
                }
            }
            Ok(v)
        }
    }
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
fn is_default_tool_format(v: &ToolCallFormat) -> bool {
    *v == ToolCallFormat::default()
}
fn is_default_tool_delivery_mode(v: &ToolDeliveryMode) -> bool {
    *v == ToolDeliveryMode::default()
}
impl StandardDefinition {
    /// The set of runtimes this agent is allowed to run in.
    ///
    /// Empty result = no constraint = runs anywhere.
    pub fn allowed_runtimes(&self) -> Vec<RuntimeMode> {
        self.runtime.clone()
    }

    /// Whether this agent can execute given the caller's `current` runtime,
    /// optionally with a `RemoteTaskRunner` providing an alternative runtime
    /// via remote dispatch.
    ///
    /// Returns true when:
    /// - the agent has no runtime constraint, OR
    /// - the current runtime matches one of the allowed runtimes, OR
    /// - a runner is available whose `provided_runtime` matches one of the
    ///   allowed runtimes.
    pub fn is_runnable_in(
        &self,
        current: &RuntimeMode,
        runner_provides: Option<&RuntimeMode>,
    ) -> bool {
        let allowed = self.allowed_runtimes();
        if allowed.is_empty() {
            return true;
        }
        if allowed.iter().any(|rt| rt == current) {
            return true;
        }
        match runner_provides {
            Some(p) => allowed.iter().any(|rt| rt == p),
            None => false,
        }
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

        // Append to instructions (for `invoke_agent({system: ...})` —
        // keeps the agent's base scaffolding and adds the caller's text
        // below it, separated by a blank line).
        if let Some(suffix) = overrides.instructions_append {
            if self.instructions.is_empty() {
                self.instructions = suffix;
            } else {
                self.instructions.push_str("\n\n");
                self.instructions.push_str(&suffix);
            }
        }

        if let Some(runtime) = overrides.runtime {
            self.runtime = runtime;
        }

        if let Some(description) = overrides.description {
            self.description = description;
        }

        if let Some(name) = overrides.name {
            self.name = name;
        }

        if let Some(sub_agents) = overrides.sub_agents {
            self.sub_agents = sub_agents;
        }

        if let Some(tools_override) = overrides.tools {
            self.tools = Some(tools_override);
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
    // Sub-agent dispatch (typed Invocation; replaces call_agent + run_skill).
    "invoke_agent",
    // Supervisor tools — query / wait / cancel / list children.
    "get_task",
    "wait_task",
    "cancel_task",
    "list_my_tasks",
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
    // Skills (load body into current agent context; sub-agents call this
    // themselves after being dispatched via invoke_agent).
    "load_skill",
    // Connection & secrets
    "inject_connection_env",
    // Logging
    "console_log",
    // Artifacts & filesystem
    "artifact_tool",
    // Todos — the tool's `Tool::get_name()` returns `write_todos`, so
    // any agent definition that declared `builtin = ["todos"]` was
    // silently broken (`resolve_tools_config` filters
    // `get_builtin_tools()` by name and finds nothing). Single
    // canonical name across validation / dispatch / skill bodies.
    "write_todos",
];

/// Tools that always get full schemas, never deferred.
/// These are the most commonly used tools that agents need immediately.
pub const CORE_TOOLS: &[&str] = &[
    "final",
    "invoke_agent",
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
    /// Built-in tools to include (e.g., ["final", "call_agent"])
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
        CORE_TOOLS.contains(&name) || self.always_full_schema.iter().any(|n| n == name)
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

// Where filesystem and artifact tools should execute.
// Deprecated: filesystem tools are no longer included as server builtins.

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
        /// Azure resource name (e.g. `distri-tts-resource`), **not** a URL.
        /// Every endpoint URL is derived from this — see
        /// [`ModelProvider::completion_url`] / [`ModelProvider::tts_url`].
        resource: String,
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
    /// fal.ai — image-generation provider. The model id is the fal endpoint
    /// path (e.g. `fal-ai/flux/dev`); the gateway POSTs to
    /// `https://fal.run/<model_id>` with `Authorization: Key <api_key>`.
    #[serde(rename = "fal_ai")]
    FalAi { api_key: Option<String> },
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
    #[serde(default)]
    pub placeholder: String,
    /// Whether this secret is required (vs optional)
    #[serde(default = "default_required")]
    pub required: bool,
    /// Whether this field contains sensitive data (masked in UI, stored encrypted).
    /// Defaults to true. Set to false for non-sensitive config like URLs, project IDs.
    #[serde(default = "default_sensitive")]
    pub sensitive: bool,
    /// When set, the UI renders this field as a resource segment embedded in
    /// the URL template (`{}` marks the editable segment), showing the full
    /// endpoint read-only around it. Azure AI Foundry uses this: the user
    /// edits only the resource name and that is all we store.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url_template: Option<String>,
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
    /// Optional per-provider override of `/v1/providers/test`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    test: Option<crate::models::ProviderTestConfig>,
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

impl From<crate::models::ProviderKeyDefinition> for SecretKeyDefinition {
    fn from(k: crate::models::ProviderKeyDefinition) -> Self {
        SecretKeyDefinition {
            key: k.key,
            label: k.label,
            placeholder: k.placeholder,
            required: k.required,
            sensitive: k.sensitive,
            url_template: k.url_template,
        }
    }
}

impl From<crate::models::ModelProviderDefinition> for DefaultProviderEntry {
    fn from(d: crate::models::ModelProviderDefinition) -> Self {
        let models = d
            .models
            .into_iter()
            .map(|mut m| {
                // Config sources may omit `name`; backfill it from `id` so
                // the catalog never surfaces a blank label.
                if m.name.trim().is_empty() {
                    m.name = m.id.clone();
                }
                m
            })
            .collect();
        DefaultProviderEntry {
            id: d.id,
            label: d.label,
            keys: d.keys.into_iter().map(SecretKeyDefinition::from).collect(),
            models,
            test: d.test,
        }
    }
}

/// Look up the provider-test override for a registered provider. The
/// `/v1/providers/test` handler calls this and falls back to its default
/// `GET /models` probe when `None` is returned. Reads from the merged
/// layered registry (built-in basics + deployment extensions).
pub fn lookup_provider_test_config(provider_id: &str) -> Option<crate::models::ProviderTestConfig> {
    merged_providers()
        .into_iter()
        .find(|p| p.id == provider_id)
        .and_then(|p| p.test)
}

/// Provider/model definitions contributed by a deployment, layered on top of
/// the built-in basics in `default_models.json`. Populated once at startup —
/// the OSS server folds in `distri.yaml`, the cloud folds in its own config
/// file. See [`register_provider_extensions`].
static PROVIDER_EXTENSIONS: std::sync::OnceLock<Vec<DefaultProviderEntry>> =
    std::sync::OnceLock::new();

/// Register deployment-owned provider/model definitions — layer 2 of the
/// provider registry. Call once, at process startup, before any
/// provider/model catalog is served.
///
/// An extension whose `id` matches a built-in provider overrides it; a new
/// `id` is appended. Calling more than once logs a warning and keeps the
/// first registration.
pub fn register_provider_extensions(extensions: Vec<crate::models::ModelProviderDefinition>) {
    let entries: Vec<DefaultProviderEntry> = extensions
        .into_iter()
        .map(DefaultProviderEntry::from)
        .collect();
    let count = entries.len();
    if PROVIDER_EXTENSIONS.set(entries).is_err() {
        tracing::warn!("provider extensions already registered; ignoring {count} new entries");
    } else {
        tracing::info!("registered {count} provider extension(s)");
    }
}

/// Merge built-in providers with extension providers — extensions override
/// built-ins by `id`, new ids are appended. Pure: no global state.
fn merge_provider_layers(
    builtin: &[DefaultProviderEntry],
    extensions: &[DefaultProviderEntry],
) -> Vec<DefaultProviderEntry> {
    let mut merged: Vec<DefaultProviderEntry> = builtin.to_vec();
    for ext in extensions {
        match merged.iter_mut().find(|p| p.id == ext.id) {
            Some(slot) => *slot = ext.clone(),
            None => merged.push(ext.clone()),
        }
    }
    merged
}

/// Built-in providers plus any registered extensions — the full layered
/// registry, lowest-to-highest precedence.
fn merged_providers() -> Vec<DefaultProviderEntry> {
    let extensions = PROVIDER_EXTENSIONS.get().map(Vec::as_slice).unwrap_or(&[]);
    merge_provider_layers(load_default_providers(), extensions)
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

    /// fal.ai sync invocation root. The full URL is
    /// `https://fal.run/<model_id>`; auth is `Authorization: Key <key>`.
    pub fn fal_ai_base_url() -> &'static str {
        "https://fal.run"
    }

    /// Mutable reference to this provider's `api_key` slot, if any.
    /// Plain OpenAI returns `None` because it uses an env var directly.
    pub fn api_key_slot_mut(&mut self) -> Option<&mut Option<String>> {
        match self {
            Self::OpenAI {} => None,
            Self::OpenAICompatible { api_key, .. }
            | Self::AzureOpenAI { api_key, .. }
            | Self::Anthropic { api_key, .. }
            | Self::Gemini { api_key, .. }
            | Self::AzureAiFoundry { api_key, .. }
            | Self::AwsBedrock { api_key, .. }
            | Self::GoogleVertex { api_key, .. }
            | Self::AlibabaCloud { api_key, .. }
            | Self::FalAi { api_key } => Some(api_key),
        }
    }

    /// Mutable reference to this provider's `base_url` slot when it
    /// participates in endpoint-secret hydration. Anthropic's
    /// `Option<String>` base_url is excluded — it has a default and no
    /// endpoint secret. Plain OpenAI has no base_url field.
    pub fn base_url_slot_mut(&mut self) -> Option<&mut String> {
        match self {
            // Azure AI Foundry hydrates a *resource name* (not a URL) into
            // this slot from the `AZURE_AI_FOUNDRY_RESOURCE` secret; the URL
            // is derived later via `completion_url()` / `tts_url()`.
            Self::AzureAiFoundry { resource, .. } => Some(resource),
            Self::AzureOpenAI { base_url, .. }
            | Self::AwsBedrock { base_url, .. }
            | Self::GoogleVertex { base_url, .. }
            | Self::Gemini { base_url, .. }
            | Self::OpenAICompatible { base_url, .. }
            | Self::AlibabaCloud { base_url, .. } => Some(base_url),
            Self::OpenAI {} | Self::Anthropic { .. } | Self::FalAi { .. } => None,
        }
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
            ModelProvider::FalAi { .. } => crate::models::ProviderType::FalAi,
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
            ModelProvider::FalAi { .. } => "fal_ai",
        }
    }

    /// The canonical secret-store key under which this provider's API key
    /// lives.
    ///
    /// **Single source of truth.** Every layer that needs to look up or
    /// validate the API key MUST go through this method — the gateway
    /// (`ProviderClientConfig`), workspace-level resolution
    /// (`WorkspaceStore::resolve_model_settings`), and the validator
    /// (`required_secret_keys`) all rely on it. The UI's user-facing key list
    /// in `default_models.json` is kept in sync with this via a unit test.
    pub fn api_key_secret(&self) -> &'static str {
        match self {
            ModelProvider::OpenAI {} => "OPENAI_API_KEY",
            // Custom OpenAI-compatible providers all share the OPENAI_API_KEY
            // fallback; per-provider keys are passed inline on the provider
            // config, not stored in the secret store under a different name.
            ModelProvider::OpenAICompatible { .. } => "OPENAI_API_KEY",
            ModelProvider::AzureOpenAI { .. } => "AZURE_OPENAI_API_KEY",
            ModelProvider::Anthropic { .. } => "ANTHROPIC_API_KEY",
            ModelProvider::Gemini { .. } => "GEMINI_API_KEY",
            ModelProvider::AzureAiFoundry { .. } => "AZURE_AI_FOUNDRY_API_KEY",
            // AWS Bedrock authenticates with sigv4 — AWS_ACCESS_KEY_ID is the
            // primary key the gateway looks up; AWS_SECRET_ACCESS_KEY and
            // AWS_REGION are looked up alongside but are not "the" api_key.
            ModelProvider::AwsBedrock { .. } => "AWS_ACCESS_KEY_ID",
            ModelProvider::GoogleVertex { .. } => "GOOGLE_VERTEX_API_KEY",
            ModelProvider::AlibabaCloud { .. } => "DASHSCOPE_API_KEY",
            ModelProvider::FalAi { .. } => "FAL_KEY",
        }
    }

    /// The canonical secret-store key for this provider's endpoint URL, or
    /// `None` if the provider has a fixed endpoint baked into the variant.
    ///
    /// Only providers that require a tenant-specific endpoint (Azure, Bedrock,
    /// Vertex) return `Some`; everything else uses a default base URL.
    pub fn endpoint_secret(&self) -> Option<&'static str> {
        match self {
            ModelProvider::AzureOpenAI { .. } => Some("AZURE_OPENAI_ENDPOINT"),
            // Holds the Azure resource name, not a URL — see the variant doc.
            ModelProvider::AzureAiFoundry { .. } => Some("AZURE_AI_FOUNDRY_RESOURCE"),
            ModelProvider::AwsBedrock { .. } => Some("AWS_BEDROCK_ENDPOINT"),
            ModelProvider::GoogleVertex { .. } => Some("GOOGLE_VERTEX_ENDPOINT"),
            ModelProvider::OpenAI {}
            | ModelProvider::OpenAICompatible { .. }
            | ModelProvider::Anthropic { .. }
            | ModelProvider::Gemini { .. }
            | ModelProvider::AlibabaCloud { .. }
            | ModelProvider::FalAi { .. } => None,
        }
    }

    /// OpenAI-compatible API base URL for an Azure AI Foundry resource name.
    ///
    /// Azure AI Foundry exposes the OpenAI v1 API at
    /// `https://<resource>.openai.azure.com/openai/v1` — one endpoint serving
    /// chat completions and OpenAI-style audio (TTS/STT). The Foundry
    /// *project* endpoint (`*.services.ai.azure.com/api/projects/...`) is the
    /// Agents SDK surface and is deliberately not used for model calls.
    pub fn azure_ai_foundry_base_url(resource: &str) -> String {
        let r = resource.trim().trim_matches('/');
        format!("https://{r}.openai.azure.com/openai/v1")
    }

    /// Azure AI Foundry resource name, if this is that provider.
    pub fn azure_ai_foundry_resource(&self) -> Option<&str> {
        match self {
            ModelProvider::AzureAiFoundry { resource, .. } => Some(resource.as_str()),
            _ => None,
        }
    }

    /// Resolved chat-completions base URL for providers whose endpoint is
    /// *derived* from config (Azure AI Foundry) rather than user-supplied.
    /// `None` means the caller should fall back to the provider's own
    /// `base_url`/default.
    pub fn completion_url(&self) -> Option<String> {
        match self {
            ModelProvider::AzureAiFoundry { resource, .. } if !resource.trim().is_empty() => {
                Some(Self::azure_ai_foundry_base_url(resource))
            }
            _ => None,
        }
    }

    /// Resolved TTS base URL. Azure AI Foundry serves TTS on the same
    /// OpenAI-compatible endpoint as chat today; kept as a separate method so
    /// a future non-OpenAI surface (e.g. Azure Speech) changes only here.
    pub fn tts_url(&self) -> Option<String> {
        self.completion_url()
    }

    /// Resolved STT base URL — see [`ModelProvider::tts_url`].
    pub fn stt_url(&self) -> Option<String> {
        self.completion_url()
    }

    /// Resolved image-generation base URL.
    ///
    /// Azure AI Foundry exposes image generation on
    /// `https://<resource>.services.ai.azure.com/openai/v1` — a different
    /// subdomain from chat/TTS (which use `*.openai.azure.com`). The image
    /// dispatcher consults this method so a Foundry resource routes to the
    /// right host without changing the chat path.
    pub fn image_url(&self) -> Option<String> {
        match self {
            ModelProvider::AzureAiFoundry { resource, .. } if !resource.trim().is_empty() => {
                let r = resource.trim().trim_matches('/');
                Some(format!("https://{r}.services.ai.azure.com/openai/v1"))
            }
            _ => self.completion_url(),
        }
    }

    /// `(base_url, api_key)` for this provider — call after `hydrate_creds`.
    /// `base_url` is the OpenAI-compatible endpoint to probe; used by the
    /// `/providers/test` validation flow.
    pub fn resolved_endpoint(&self) -> (Option<String>, Option<String>) {
        match self {
            ModelProvider::OpenAI {} => (Some(Self::openai_base_url()), None),
            ModelProvider::AzureAiFoundry { api_key, .. } => {
                (self.completion_url(), api_key.clone())
            }
            ModelProvider::Anthropic { base_url, api_key } => (
                Some(
                    base_url
                        .clone()
                        .unwrap_or_else(|| "https://api.anthropic.com".to_string()),
                ),
                api_key.clone(),
            ),
            ModelProvider::Gemini { base_url, api_key } => {
                (Some(base_url.clone()), api_key.clone())
            }
            ModelProvider::OpenAICompatible {
                base_url, api_key, ..
            } => (Some(base_url.clone()), api_key.clone()),
            ModelProvider::AlibabaCloud { base_url, api_key } => {
                (Some(base_url.clone()), api_key.clone())
            }
            ModelProvider::AzureOpenAI {
                base_url, api_key, ..
            } => (Some(base_url.clone()), api_key.clone()),
            ModelProvider::AwsBedrock { base_url, api_key } => {
                (Some(base_url.clone()), api_key.clone())
            }
            ModelProvider::GoogleVertex {
                base_url, api_key, ..
            } => (Some(base_url.clone()), api_key.clone()),
            ModelProvider::FalAi { api_key } => {
                (Some(Self::fal_ai_base_url().to_string()), api_key.clone())
            }
        }
    }

    /// Returns the required secret keys for this provider — i.e. keys that
    /// must resolve via the secret store or environment for the LLM call to
    /// succeed. If the provider has an inline `api_key` already configured on
    /// the provider variant, no secret lookup is required.
    pub fn required_secret_keys(&self) -> Vec<&'static str> {
        let api_key_present = match self {
            ModelProvider::OpenAI {} => false,
            ModelProvider::OpenAICompatible { api_key, .. }
            | ModelProvider::AzureOpenAI { api_key, .. }
            | ModelProvider::Gemini { api_key, .. }
            | ModelProvider::AzureAiFoundry { api_key, .. }
            | ModelProvider::AwsBedrock { api_key, .. }
            | ModelProvider::GoogleVertex { api_key, .. }
            | ModelProvider::AlibabaCloud { api_key, .. }
            | ModelProvider::FalAi { api_key } => api_key.is_some(),
            ModelProvider::Anthropic { api_key, .. } => api_key.is_some(),
        };
        if api_key_present {
            vec![]
        } else {
            vec![self.api_key_secret()]
        }
    }

    /// Returns all provider secret definitions — the built-in basics from
    /// `default_models.json` merged with any registered extensions.
    pub fn all_provider_definitions() -> Vec<ProviderSecretDefinition> {
        merged_providers()
            .into_iter()
            .map(|p| ProviderSecretDefinition {
                id: p.id,
                label: p.label,
                keys: p.keys,
            })
            .collect()
    }

    /// Returns the well-known models grouped by provider — the built-in
    /// basics from `default_models.json` merged with registered extensions.
    pub fn well_known_models() -> Vec<ProviderModels> {
        merged_providers()
            .into_iter()
            .filter(|p| !p.models.is_empty())
            .map(|p| ProviderModels {
                provider_id: p.id,
                provider_label: p.label,
                models: p.models,
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
            ModelProvider::FalAi { .. } => "fal.ai",
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
            ModelProvider::FalAi { .. } => "fal.ai",
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

    /// Fill empty `api_key` and `base_url` fields on this provider by
    /// looking up the canonical secret keys
    /// ([`ModelProvider::api_key_secret`] /
    /// [`ModelProvider::endpoint_secret`]) in the workspace secret
    /// store. **Single source of truth** for provider credential
    /// resolution — both the workspace's default-model resolution and
    /// the orchestrator's per-task agent resolution call this so any
    /// provider-pinned ModelSettings (workspace default OR agent
    /// override) gets the same hydration treatment.
    ///
    /// Errors if a required endpoint secret is missing
    /// (`AzureOpenAI` / `AzureAiFoundry` / `AwsBedrock` /
    /// `GoogleVertex` all need a tenant-specific endpoint that has no
    /// safe default — silently dropping it produces an unparseable
    /// base URL and a downstream client panic). Missing api_keys
    /// downgrade to a warning since some providers can fall back to
    /// other auth (env vars, IAM roles, etc.).
    pub async fn hydrate_creds(
        &mut self,
        secret_store: &dyn crate::stores::SecretStore,
    ) -> Result<(), String> {
        let provider_label = self.inner.provider.provider_id().to_string();
        let api_key_secret = self.inner.provider.api_key_secret();
        let endpoint_secret = self.inner.provider.endpoint_secret();

        // api_key — fill if the slot is currently None.
        if let Some(slot) = self.inner.provider.api_key_slot_mut() {
            if slot.is_none() {
                match secret_store.get(api_key_secret).await {
                    Ok(Some(secret)) => *slot = Some(secret.value),
                    Ok(None) => tracing::warn!(
                        "{} secret not found for provider '{}'",
                        api_key_secret,
                        provider_label
                    ),
                    Err(e) => tracing::error!(
                        "failed to fetch {} for provider '{}': {}",
                        api_key_secret,
                        provider_label,
                        e
                    ),
                }
            }
        }

        // base_url — fill if the slot is currently empty AND the
        // provider exposes an endpoint_secret. Hard-error for
        // endpoint-required providers when the secret is missing.
        if let Some(endpoint_key) = endpoint_secret {
            if let Some(slot) = self.inner.provider.base_url_slot_mut() {
                if slot.is_empty() {
                    match secret_store.get(endpoint_key).await {
                        Ok(Some(secret)) => *slot = secret.value,
                        Ok(None) => {
                            return Err(format!(
                                "{} secret not set for provider '{}'. \
                                 Configure the workspace's '{}' provider \
                                 (POST /v1/providers) before pinning a model \
                                 with this provider prefix.",
                                endpoint_key, provider_label, provider_label
                            ));
                        }
                        Err(e) => {
                            return Err(format!(
                                "failed to fetch {} for provider '{}': {e}",
                                endpoint_key, provider_label
                            ));
                        }
                    }
                }
            }
        }

        Ok(())
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
                resource: String::new(),
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
            "fal_ai" => ModelProvider::FalAi { api_key: None },
            _ if provider_str.starts_with("custom_") => ModelProvider::OpenAICompatible {
                base_url: String::new(),
                api_key: None,
                project_id: None,
            },
            // Unknown provider — error out instead of silently falling
            // through to OpenAI-compatible. The previous behaviour
            // (silent fallback) caused agent definitions like
            // `model = "azure_foundry/gpt-5.4"` to typo'd → resolve to
            // a generic OpenAI-compatible client and silently load the
            // workspace's default provider's credentials, with no
            // signal to the caller that their agent's model pin was
            // ignored.
            _ => {
                return Err(format!(
                    "unknown model provider prefix '{provider_str}' in '{s}'. \
                     Recognised prefixes: openai, anthropic, azure_openai, \
                     azure (alias for azure_openai), gemini, azure_ai_foundry, \
                     aws_bedrock, google_vertex, alibaba_cloud, fal_ai, custom_*. \
                     Pass just the model name with no slash to use the \
                     workspace's default provider."
                ));
            }
        };
        Ok(Some(Self {
            model: model_id.to_string(),
            inner: ModelSettingsInner {
                provider,
                ..Default::default()
            },
        }))
    }

    /// Merge base (workspace) model settings with agent/request-level overrides.
    ///
    /// Provider resolution:
    /// - If the override explicitly sets a provider (not the default OpenAI),
    ///   the override's provider and model are used.
    /// - If only the base has a non-default provider and the override uses default
    ///   OpenAI, the base's provider AND model win — the override's bare model name
    ///   is ignored because it may not exist on the base provider.
    /// - Otherwise, the override's model wins if non-empty.
    ///
    /// Scalar fields (temperature, max_tokens, etc.) use override if present, else base.
    ///
    /// Returns `None` if the final model string is empty.
    pub fn merge(&self, override_settings: &ModelSettings) -> Option<ModelSettings> {
        let default_provider = ModelProvider::OpenAI {};
        let override_has_explicit_provider =
            std::mem::discriminant(&override_settings.inner.provider)
                != std::mem::discriminant(&default_provider);
        let base_has_explicit_provider = std::mem::discriminant(&self.inner.provider)
            != std::mem::discriminant(&default_provider);

        let (provider, model) = if override_has_explicit_provider {
            // Override explicitly set a provider — use override's provider and model.
            let model = if !override_settings.model.is_empty() {
                override_settings.model.clone()
            } else {
                self.model.clone()
            };
            (override_settings.inner.provider.clone(), model)
        } else if base_has_explicit_provider {
            // Base uses a non-default provider and override didn't specify one — use
            // base's provider AND model to avoid mismatching model names.
            let model = if !self.model.is_empty() {
                self.model.clone()
            } else if !override_settings.model.is_empty() {
                override_settings.model.clone()
            } else {
                String::new()
            };
            (self.inner.provider.clone(), model)
        } else {
            // Both use default OpenAI — override model can win.
            let model = if !override_settings.model.is_empty() {
                override_settings.model.clone()
            } else {
                self.model.clone()
            };
            (self.inner.provider.clone(), model)
        };

        if model.is_empty() {
            return None;
        }

        let default_context_size = 20000u32;
        Some(ModelSettings {
            model,
            inner: ModelSettingsInner {
                temperature: override_settings
                    .inner
                    .temperature
                    .or(self.inner.temperature),
                max_tokens: override_settings.inner.max_tokens.or(self.inner.max_tokens),
                context_size: if override_settings.inner.context_size != default_context_size {
                    override_settings.inner.context_size
                } else {
                    self.inner.context_size
                },
                top_p: override_settings.inner.top_p.or(self.inner.top_p),
                frequency_penalty: override_settings
                    .inner
                    .frequency_penalty
                    .or(self.inner.frequency_penalty),
                presence_penalty: override_settings
                    .inner
                    .presence_penalty
                    .or(self.inner.presence_penalty),
                provider,
                parameters: if override_settings.inner.parameters.is_some() {
                    override_settings.inner.parameters.clone()
                } else {
                    self.inner.parameters.clone()
                },
                response_format: if override_settings.inner.response_format.is_some() {
                    override_settings.inner.response_format.clone()
                } else {
                    self.inner.response_format.clone()
                },
                api_format: if override_settings.inner.api_format != OpenAiApiFormat::Auto {
                    override_settings.inner.api_format.clone()
                } else {
                    self.inner.api_format.clone()
                },
            },
        })
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

    // Validate that agent name characters are valid (alphanumeric, underscore, or single '/' for namespacing)
    if !agent_def
        .name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '/')
        || agent_def
            .name
            .chars()
            .next()
            .is_some_and(|c| c.is_numeric())
        || agent_def.name.chars().filter(|&c| c == '/').count() > 1
    {
        return Err(AgentError::Validation(format!(
            "Invalid agent name '{}': Agent names must be alphanumeric with underscores, at most one '/' for namespacing (e.g. '_system/plan'), cannot start with number.",
            agent_def.name
        )));
    }

    // Extract markdown instructions (everything after the second ---)
    let instructions = parts[2..].join("---").trim().to_string();

    // Set the instructions in the agent definition
    agent_def.instructions = instructions;

    // Resolve `provider/model` prefix on `model_settings.model`. When the
    // agent author writes `model = "azure_ai_foundry/gpt-5.4"` we need
    // to (a) split the prefix, (b) set `provider` explicitly so
    // ModelSettings::merge() doesn't fall back to the workspace default
    // provider, and (c) rewrite `model` to just the bare model name.
    // Unknown prefixes error out — the caller is making a clearly
    // invalid claim ("dispatch to provider X") that must not silently
    // fall back to "use whatever the workspace default is".
    if let Some(ref mut ms) = agent_def.model_settings {
        if ms.model.contains('/') {
            let resolved = ModelSettings::from_provider_model_str(&ms.model)
                .map_err(AgentError::Validation)?
                .ok_or_else(|| {
                    AgentError::Validation(format!(
                        "agent '{}': invalid model_settings.model '{}' — empty model name after the provider prefix",
                        agent_def.name, ms.model
                    ))
                })?;
            ms.model = resolved.model;
            ms.inner.provider = resolved.inner.provider;
        }
    }

    Ok(agent_def)
}

/// Validate plugin name follows naming conventions
/// Plugin names must be valid identifiers. At most one '/' is allowed for workspace namespacing (e.g. 'workspace/agent').
pub fn validate_plugin_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Plugin name cannot be empty".to_string());
    }

    if name.contains('-') {
        return Err(format!(
            "Plugin name '{}' cannot contain hyphens. Use underscores instead.",
            name
        ));
    }

    let slash_count = name.chars().filter(|&c| c == '/').count();
    if slash_count > 1 {
        return Err(format!(
            "Plugin name '{}' can contain at most one '/' for workspace namespacing (e.g. 'workspace/agent')",
            name
        ));
    }

    // Validate each segment (split by optional slash)
    let segments: Vec<&str> = name.split('/').collect();
    for segment in &segments {
        if segment.is_empty() {
            return Err(format!(
                "Plugin name '{}' has an empty segment around '/'",
                name
            ));
        }

        if let Some(first_char) = segment.chars().next()
            && !first_char.is_ascii_alphabetic()
            && first_char != '_'
        {
            return Err(format!(
                "Each segment in '{}' must start with a letter or underscore",
                name
            ));
        }

        for ch in segment.chars() {
            if !ch.is_ascii_alphanumeric() && ch != '_' {
                return Err(format!(
                    "Plugin name '{}' can only contain letters, numbers, underscores, and at most one '/' for namespacing",
                    name
                ));
            }
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
    fn sparse_definition_round_trip_does_not_inject_defaults() {
        // A user authors a minimal agent with only `name`. Parsing then
        // re-serializing must not bake in `version`, `history_size`,
        // `description`, `tool_format`, `tool_delivery_mode`, `sub_agents`,
        // or `icon_url` — those are runtime-resolved defaults, not
        // user-provided values.
        let toml_in = r#"name = "minimal""#;
        let def: StandardDefinition = toml::from_str(toml_in).unwrap();
        let toml_out = toml::to_string(&def).unwrap();

        for field in [
            "version",
            "history_size",
            "description",
            "tool_format",
            "tool_delivery_mode",
            "sub_agents",
            "icon_url",
        ] {
            assert!(
                !toml_out.contains(field),
                "round-trip injected `{field}` into sparse definition:\n{toml_out}"
            );
        }
    }

    #[test]
    fn explicit_values_survive_round_trip() {
        // Conversely, fields the user *does* set must round-trip intact.
        let toml_in = r#"
name = "explicit"
description = "a real description"
version = "1.2.3"
history_size = 7
sub_agents = ["helper"]
tool_format = "json_l"
"#;
        let def: StandardDefinition = toml::from_str(toml_in).unwrap();
        let toml_out = toml::to_string(&def).unwrap();
        assert!(toml_out.contains("description = \"a real description\""));
        assert!(toml_out.contains("version = \"1.2.3\""));
        assert!(toml_out.contains("history_size = 7"));
        assert!(toml_out.contains("sub_agents = [\"helper\"]"));
        assert!(toml_out.contains("tool_format = \"json_l\""));
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
        assert!(config.is_core_tool("call_agent"));
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

    #[test]
    fn test_runtime_mode_serde() {
        let mode: RuntimeMode = serde_json::from_str("\"cloud\"").unwrap();
        assert_eq!(mode, RuntimeMode::Cloud);
        let mode: RuntimeMode = serde_json::from_str("\"cli\"").unwrap();
        assert_eq!(mode, RuntimeMode::Cli);
        let mode: RuntimeMode = serde_json::from_str("\"browser\"").unwrap();
        assert_eq!(mode, RuntimeMode::Browser);
        assert_eq!(RuntimeMode::default(), RuntimeMode::Cloud);
        let json = serde_json::to_string(&RuntimeMode::Cli).unwrap();
        assert_eq!(json, "\"cli\"");
    }

    // ── ModelSettings::merge tests ──────────────────────────────────────────

    #[test]
    fn merge_both_default_openai_agent_model_wins() {
        let base = ModelSettings::new("gpt-5.1");
        let agent = ModelSettings::new("gpt-4.1-mini");

        let result = base.merge(&agent).unwrap();
        assert_eq!(result.model, "gpt-4.1-mini");
        assert!(matches!(result.inner.provider, ModelProvider::OpenAI {}));
    }

    #[test]
    fn merge_both_default_openai_base_model_used_when_agent_empty() {
        let base = ModelSettings::new("gpt-5.1");
        let agent = ModelSettings::new("");

        let result = base.merge(&agent).unwrap();
        assert_eq!(result.model, "gpt-5.1");
    }

    #[test]
    fn merge_agent_explicit_provider_wins() {
        let base = ModelSettings {
            model: "gpt-5.1".into(),
            inner: ModelSettingsInner {
                provider: ModelProvider::OpenAICompatible {
                    base_url: "https://custom.com/v1".into(),
                    api_key: Some("key".into()),
                    project_id: None,
                },
                ..Default::default()
            },
        };
        let agent = ModelSettings {
            model: "claude-sonnet-4".into(),
            inner: ModelSettingsInner {
                provider: ModelProvider::Anthropic {
                    base_url: None,
                    api_key: None,
                },
                ..Default::default()
            },
        };

        let result = base.merge(&agent).unwrap();
        assert_eq!(result.model, "claude-sonnet-4");
        assert!(matches!(
            result.inner.provider,
            ModelProvider::Anthropic { .. }
        ));
    }

    #[test]
    fn merge_agent_explicit_provider_no_model_uses_base() {
        let base = ModelSettings::new("gpt-5.1");
        let agent = ModelSettings {
            model: "".into(),
            inner: ModelSettingsInner {
                provider: ModelProvider::Anthropic {
                    base_url: None,
                    api_key: None,
                },
                ..Default::default()
            },
        };

        let result = base.merge(&agent).unwrap();
        assert_eq!(result.model, "gpt-5.1");
        assert!(matches!(
            result.inner.provider,
            ModelProvider::Anthropic { .. }
        ));
    }

    #[test]
    fn merge_workspace_custom_provider_overrides_agent_model() {
        let base = ModelSettings {
            model: "gpt-5.4".into(),
            inner: ModelSettingsInner {
                provider: ModelProvider::OpenAICompatible {
                    base_url: "https://custom.azure.com/openai/v1".into(),
                    api_key: Some("test-key".into()),
                    project_id: None,
                },
                ..Default::default()
            },
        };
        // Agent has no explicit provider (default OpenAI) but different model
        let agent = ModelSettings::new("gpt-5.1");

        let result = base.merge(&agent).unwrap();
        assert_eq!(result.model, "gpt-5.4");
        assert!(matches!(
            result.inner.provider,
            ModelProvider::OpenAICompatible { .. }
        ));
    }

    #[test]
    fn merge_workspace_custom_provider_agent_empty_model() {
        let base = ModelSettings {
            model: "gpt-5.4".into(),
            inner: ModelSettingsInner {
                provider: ModelProvider::OpenAICompatible {
                    base_url: "https://custom.azure.com/openai/v1".into(),
                    api_key: Some("test-key".into()),
                    project_id: None,
                },
                ..Default::default()
            },
        };
        let agent = ModelSettings::new("");

        let result = base.merge(&agent).unwrap();
        assert_eq!(result.model, "gpt-5.4");
    }

    #[test]
    fn merge_both_empty_returns_none() {
        let base = ModelSettings::new("");
        let agent = ModelSettings::new("");

        assert!(base.merge(&agent).is_none());
    }

    #[test]
    fn merge_workspace_empty_agent_empty_returns_none() {
        let base = ModelSettings {
            model: "".into(),
            inner: ModelSettingsInner {
                provider: ModelProvider::OpenAICompatible {
                    base_url: "https://custom.com".into(),
                    api_key: None,
                    project_id: None,
                },
                ..Default::default()
            },
        };
        let agent = ModelSettings::new("");

        assert!(base.merge(&agent).is_none());
    }

    #[test]
    fn merge_temperature_max_tokens_override() {
        let base = ModelSettings {
            model: "gpt-5.1".into(),
            inner: ModelSettingsInner {
                temperature: Some(0.5),
                max_tokens: Some(1000),
                top_p: Some(0.9),
                ..Default::default()
            },
        };
        let agent = ModelSettings {
            model: "gpt-4.1-mini".into(),
            inner: ModelSettingsInner {
                temperature: Some(0.9),
                max_tokens: None, // no override
                ..Default::default()
            },
        };

        let result = base.merge(&agent).unwrap();
        assert_eq!(result.model, "gpt-4.1-mini");
        assert_eq!(result.inner.temperature, Some(0.9));
        assert_eq!(result.inner.max_tokens, Some(1000)); // base value preserved
        assert_eq!(result.inner.top_p, Some(0.9)); // base value preserved
    }

    #[test]
    fn merge_context_size_non_default_wins() {
        let base = ModelSettings {
            model: "gpt-5.1".into(),
            inner: ModelSettingsInner {
                context_size: 20000, // default
                ..Default::default()
            },
        };
        let agent = ModelSettings {
            model: "gpt-4.1-mini".into(),
            inner: ModelSettingsInner {
                context_size: 100000, // explicitly set
                ..Default::default()
            },
        };

        let result = base.merge(&agent).unwrap();
        assert_eq!(result.inner.context_size, 100000);
    }

    #[test]
    fn merge_context_size_default_falls_back() {
        let base = ModelSettings {
            model: "gpt-5.1".into(),
            inner: ModelSettingsInner {
                context_size: 128000,
                ..Default::default()
            },
        };
        let agent = ModelSettings {
            model: "gpt-4.1-mini".into(),
            inner: ModelSettingsInner {
                context_size: 20000, // default — should use base
                ..Default::default()
            },
        };

        let result = base.merge(&agent).unwrap();
        assert_eq!(result.inner.context_size, 128000);
    }

    #[test]
    fn merge_azure_ai_foundry_resource_preserved() {
        let base = ModelSettings {
            model: "gpt-5.4".into(),
            inner: ModelSettingsInner {
                provider: ModelProvider::AzureAiFoundry {
                    resource: "myresource".into(),
                    api_key: Some("test-key".into()),
                },
                ..Default::default()
            },
        };
        let agent = ModelSettings::new("gpt-5.1");

        let result = base.merge(&agent).unwrap();
        assert_eq!(result.model, "gpt-5.4"); // workspace model wins
        assert!(matches!(
            result.inner.provider,
            ModelProvider::AzureAiFoundry { .. }
        ));
        if let ModelProvider::AzureAiFoundry { resource, .. } = &result.inner.provider {
            assert_eq!(resource, "myresource");
        }
        assert_eq!(
            result.inner.provider.completion_url().as_deref(),
            Some("https://myresource.openai.azure.com/openai/v1"),
        );
    }

    #[test]
    fn azure_ai_foundry_resource_resolves_openai_url() {
        let p = ModelProvider::AzureAiFoundry {
            resource: "distri-tts-resource".into(),
            api_key: None,
        };
        assert_eq!(
            p.completion_url().as_deref(),
            Some("https://distri-tts-resource.openai.azure.com/openai/v1"),
        );
        // TTS rides the same OpenAI-compatible endpoint today.
        assert_eq!(p.tts_url(), p.completion_url());
        // An empty resource yields no URL — the caller must hydrate it first.
        let empty = ModelProvider::AzureAiFoundry {
            resource: String::new(),
            api_key: None,
        };
        assert_eq!(empty.completion_url(), None);
    }

    #[test]
    fn merge_anthropic_provider_preserves_base_url() {
        let base = ModelSettings {
            model: "claude-sonnet-4".into(),
            inner: ModelSettingsInner {
                provider: ModelProvider::Anthropic {
                    base_url: Some("https://custom.anthropic.com".into()),
                    api_key: Some("key".into()),
                },
                temperature: Some(0.7),
                ..Default::default()
            },
        };
        let agent = ModelSettings::new("");

        let result = base.merge(&agent).unwrap();
        assert_eq!(result.model, "claude-sonnet-4");
        assert_eq!(result.inner.temperature, Some(0.7));
        if let ModelProvider::Anthropic { base_url, api_key } = result.inner.provider {
            assert_eq!(base_url, Some("https://custom.anthropic.com".into()));
            assert_eq!(api_key, Some("key".into()));
        }
    }

    #[test]
    fn merge_response_format_agent_wins() {
        let base = ModelSettings {
            model: "gpt-5.1".into(),
            inner: ModelSettingsInner {
                response_format: Some(serde_json::json!({"type": "text"})),
                ..Default::default()
            },
        };
        let agent = ModelSettings {
            model: "gpt-4.1-mini".into(),
            inner: ModelSettingsInner {
                response_format: Some(serde_json::json!({"type": "json_object"})),
                ..Default::default()
            },
        };

        let result = base.merge(&agent).unwrap();
        assert_eq!(
            result.inner.response_format,
            Some(serde_json::json!({"type": "json_object"}))
        );
    }

    #[test]
    fn merge_response_format_base_fallback() {
        let base = ModelSettings {
            model: "gpt-5.1".into(),
            inner: ModelSettingsInner {
                response_format: Some(serde_json::json!({"type": "text"})),
                ..Default::default()
            },
        };
        let agent = ModelSettings::new("gpt-4.1-mini");

        let result = base.merge(&agent).unwrap();
        assert_eq!(
            result.inner.response_format,
            Some(serde_json::json!({"type": "text"}))
        );
    }

    #[test]
    fn merge_parameters_agent_wins() {
        let base = ModelSettings {
            model: "gpt-5.1".into(),
            inner: ModelSettingsInner {
                parameters: Some(serde_json::json!({"key": "base"})),
                ..Default::default()
            },
        };
        let agent = ModelSettings {
            model: "gpt-4.1-mini".into(),
            inner: ModelSettingsInner {
                parameters: Some(serde_json::json!({"key": "agent"})),
                ..Default::default()
            },
        };

        let result = base.merge(&agent).unwrap();
        assert_eq!(
            result.inner.parameters,
            Some(serde_json::json!({"key": "agent"}))
        );
    }

    /// Lock the canonical API-key secret name for every provider variant.
    /// Three layers depend on this: gateway (`provider_config.rs`),
    /// validator (`required_secret_keys`), and workspace resolution
    /// (`cloud::WorkspaceStore::resolve_model_settings`). They all flow
    /// through `ModelProvider::api_key_secret()` — keep this list in sync
    /// with `default_models.json` (asserted in
    /// `test_api_key_secret_matches_default_models_json`).
    #[test]
    fn test_api_key_secret_canonical_names() {
        assert_eq!(ModelProvider::OpenAI {}.api_key_secret(), "OPENAI_API_KEY");
        assert_eq!(
            ModelProvider::Anthropic {
                base_url: None,
                api_key: None,
            }
            .api_key_secret(),
            "ANTHROPIC_API_KEY"
        );
        assert_eq!(
            ModelProvider::Gemini {
                base_url: ModelProvider::gemini_base_url(),
                api_key: None,
            }
            .api_key_secret(),
            "GEMINI_API_KEY"
        );
        assert_eq!(
            ModelProvider::AzureOpenAI {
                base_url: String::new(),
                api_key: None,
                deployment: "x".into(),
                api_version: ModelProvider::azure_api_version(),
            }
            .api_key_secret(),
            "AZURE_OPENAI_API_KEY"
        );
        assert_eq!(
            ModelProvider::AzureAiFoundry {
                resource: String::new(),
                api_key: None,
            }
            .api_key_secret(),
            "AZURE_AI_FOUNDRY_API_KEY"
        );
        assert_eq!(
            ModelProvider::AwsBedrock {
                base_url: String::new(),
                api_key: None,
            }
            .api_key_secret(),
            "AWS_ACCESS_KEY_ID"
        );
        assert_eq!(
            ModelProvider::GoogleVertex {
                base_url: String::new(),
                api_key: None,
                project_id: None,
            }
            .api_key_secret(),
            "GOOGLE_VERTEX_API_KEY"
        );
        assert_eq!(
            ModelProvider::AlibabaCloud {
                base_url: ModelProvider::alibaba_cloud_base_url(),
                api_key: None,
            }
            .api_key_secret(),
            "DASHSCOPE_API_KEY"
        );
        assert_eq!(
            ModelProvider::OpenAICompatible {
                base_url: String::new(),
                api_key: None,
                project_id: None,
            }
            .api_key_secret(),
            "OPENAI_API_KEY"
        );
        assert_eq!(
            ModelProvider::FalAi { api_key: None }.api_key_secret(),
            "FAL_KEY"
        );
    }

    /// `default_models.json` drives the UI's secret editor (via
    /// `/v1/providers`). For every built-in provider listed there, the first
    /// `*_API_KEY` entry MUST equal what `ModelProvider::api_key_secret()`
    /// returns — otherwise the UI will tell users to enter a secret name
    /// that the backend won't look up. This test catches drift.
    ///
    /// Only the OSS basics live in `default_models.json`; bespoke providers
    /// (Azure AI Foundry, Bedrock, …) ship as deployment extensions and are
    /// covered by `test_api_key_secret` above plus the cloud's config test.
    #[test]
    fn test_api_key_secret_matches_default_models_json() {
        let providers = ModelProvider::all_provider_definitions();
        let cases: &[(&str, ModelProvider)] = &[
            ("openai", ModelProvider::OpenAI {}),
            (
                "anthropic",
                ModelProvider::Anthropic {
                    base_url: None,
                    api_key: None,
                },
            ),
            (
                "gemini",
                ModelProvider::Gemini {
                    base_url: ModelProvider::gemini_base_url(),
                    api_key: None,
                },
            ),
        ];

        for (id, variant) in cases {
            let def = providers
                .iter()
                .find(|p| p.id == *id)
                .unwrap_or_else(|| panic!("provider '{}' missing from default_models.json", id));
            let first_api_key_in_json = def
                .keys
                .iter()
                .map(|k| k.key.as_str())
                .find(|k| k.ends_with("_API_KEY") || *k == "AWS_ACCESS_KEY_ID")
                .unwrap_or_else(|| {
                    panic!(
                        "provider '{}' has no API key entry in default_models.json",
                        id
                    )
                });
            assert_eq!(
                first_api_key_in_json,
                variant.api_key_secret(),
                "provider '{}': default_models.json key {:?} != api_key_secret() {:?}",
                id,
                first_api_key_in_json,
                variant.api_key_secret(),
            );
        }
    }

    fn entry(id: &str, label: &str) -> DefaultProviderEntry {
        DefaultProviderEntry {
            id: id.to_string(),
            label: label.to_string(),
            keys: vec![],
            models: vec![],
            test: None,
        }
    }

    /// Layer 2 (extensions) overrides built-ins by `id` and appends new ones.
    #[test]
    fn test_merge_provider_layers_overrides_and_appends() {
        let builtin = vec![entry("openai", "OpenAI"), entry("anthropic", "Anthropic")];
        let extensions = vec![
            entry("anthropic", "Anthropic (override)"),
            entry("azure_ai_foundry", "Azure AI Foundry"),
        ];
        let merged = merge_provider_layers(&builtin, &extensions);

        assert_eq!(merged.len(), 3);
        assert_eq!(
            merged.iter().find(|p| p.id == "openai").unwrap().label,
            "OpenAI",
            "untouched built-in is preserved"
        );
        assert_eq!(
            merged.iter().find(|p| p.id == "anthropic").unwrap().label,
            "Anthropic (override)",
            "extension overrides the built-in with the same id"
        );
        assert!(
            merged.iter().any(|p| p.id == "azure_ai_foundry"),
            "extension with a new id is appended"
        );
    }

    /// `register_provider_extensions` accepts `ModelProviderDefinition` and
    /// backfills an empty model `name` from `id`.
    #[test]
    fn test_model_provider_definition_conversion_backfills_name() {
        use crate::models::{Model, ModelCapability, ModelProviderDefinition};
        let def = ModelProviderDefinition {
            id: "acme".to_string(),
            label: "Acme".to_string(),
            keys: vec![],
            models: vec![Model {
                id: "acme-large".to_string(),
                name: String::new(),
                capability: ModelCapability::Completion,
                context_window: None,
                pricing: None,
                voices: vec![],
                formats: vec![],
            }],
            is_custom: false,
            test: None,
        };
        let converted = DefaultProviderEntry::from(def);
        assert_eq!(converted.models[0].name, "acme-large");
    }
}
