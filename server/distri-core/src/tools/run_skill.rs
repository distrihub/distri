use std::sync::Arc;

use distri_types::{tool::ToolContext, Part, ToolCall};
use serde_json::json;

use crate::agent::ExecutorContext;
use crate::tools::ExecutorContextTool;
use crate::AgentError;

/// `run_skill` — single-call "load a skill AND fork into it as a sub-agent".
///
/// Equivalent to claude-code's `SkillTool` (see github.com/v3g42/claude-code,
/// `src/tools/SkillTool/SkillTool.ts`): the skill body becomes the child's
/// system_prompt, the skill's preferred model overrides the child's model,
/// and the child runs as a fork of `_adhoc_base` so it inherits the seeded
/// safe defaults (`builtin = ["final"]`, `external = ["*"]`) and the parent
/// session's external tools.
///
/// Replaces the verbose two-step pattern:
///     1. `load_skill({skill_id})` — drag full content into parent context
///     2. `call_agent({mode:"fork", system_prompt:"<paste skill body>", ...})`
///
/// With a single call:
///     `run_skill({skill_id, args?, mode?, model?, prompt?})`
#[derive(Debug, Clone)]
pub struct RunSkillTool;

#[async_trait::async_trait]
impl distri_types::Tool for RunSkillTool {
    fn get_name(&self) -> String {
        "run_skill".to_string()
    }

    fn get_description(&self) -> String {
        "Run a skill in a sub-agent. The skill's body becomes the worker's system prompt; \
         the skill's preferred model (if any) overrides the worker's model; the worker \
         inherits the parent session's external tools by default. Returns the worker's \
         `final` result. Prefer this over `load_skill` + `call_agent` when you want to \
         delegate a focused task to a skill."
            .to_string()
    }

    fn get_parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["skill_id"],
            "properties": {
                "skill_id": {
                    "type": "string",
                    "description": "The id of the skill to run (e.g. \"zippy_importer\" or \"workspace_slug/skill_name\")."
                },
                "args": {
                    "type": "object",
                    "additionalProperties": true,
                    "description": "Optional named arguments. Each `{{key}}` placeholder in the skill body is replaced by the JSON-stringified value of `args[key]`."
                },
                "prompt": {
                    "type": "string",
                    "description": "Optional task directive sent as the worker's user message. If omitted, the worker just executes the skill body."
                },
                "mode": {
                    "type": "string",
                    "enum": ["in_process", "fork", "offload"],
                    "default": "fork",
                    "description": "How to invoke the worker. `fork` (default) — child runs as a parallel sub-agent in its own task; parent dispatches multiple in one turn for fan-out. `in_process` — fresh context, parent blocks on each call (serial). `offload` — fire-and-forget."
                }
            }
        })
    }

    fn needs_executor_context(&self) -> bool {
        true
    }

    async fn execute(
        &self,
        _tool_call: ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        Err(anyhow::anyhow!(
            "RunSkillTool requires ExecutorContext, not ToolContext"
        ))
    }
}

#[derive(Debug, serde::Deserialize)]
struct RunSkillInput {
    skill_id: String,
    #[serde(default)]
    args: Option<serde_json::Value>,
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    mode: Option<String>,
}

/// Replace `${key}` placeholders in the skill body with values from `args`.
///
/// Uses `${…}` rather than `{{…}}` so it doesn't collide with Handlebars
/// syntax in agent.instructions / skill bodies. The downstream prompt
/// renderer is strict-mode Handlebars; using `{{…}}` here would force every
/// skill author to either always pass every arg or escape every literal
/// `{{`, both of which are easy to get wrong.
///
/// **Security:** every substituted value has its `{{` escaped to `\{{`
/// before being pasted in. That neutralises template-injection attempts
/// from runtime arg data — a caller passing `args: { name: "{{secret}}" }`
/// can't make the rendered prompt resolve a context-bound `secret` var.
/// String values paste verbatim (post-escape); non-string JSON values are
/// stringified. Unknown keys are left as-is so the caller sees the
/// unresolved placeholder if they typo'd it.
pub(crate) fn interpolate_args(body: &str, args: &serde_json::Value) -> String {
    let map = match args.as_object() {
        Some(m) => m,
        None => return body.to_string(),
    };
    let mut out = body.to_string();
    for (key, value) in map {
        let needle = format!("${{{}}}", key);
        let raw = match value {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        let replacement = escape_handlebars_in_value(&raw);
        out = out.replace(&needle, &replacement);
    }
    out
}

/// Escape every `{{` in a runtime-supplied value to `\{{` so it can't be
/// parsed as a Handlebars variable when the surrounding template is rendered.
/// Idempotent; `\{{` runs are left alone.
pub(crate) fn escape_handlebars_in_value(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'{' && bytes[i + 1] == b'{' {
            let already_escaped = i > 0 && bytes[i - 1] == b'\\';
            if !already_escaped {
                out.push('\\');
            }
            out.push_str("{{");
            i += 2;
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

#[async_trait::async_trait]
impl ExecutorContextTool for RunSkillTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let input: RunSkillInput = serde_json::from_value(tool_call.input.clone())
            .map_err(|e| AgentError::ToolExecution(format!("Invalid run_skill input: {}", e)))?;

        let orchestrator = context.get_orchestrator()?;
        let skill_store =
            orchestrator.stores.skill_store.as_ref().ok_or_else(|| {
                AgentError::ToolExecution("Skill store not configured".to_string())
            })?;

        let skill = skill_store
            .get(&input.skill_id)
            .await
            .map_err(|e| AgentError::ToolExecution(format!("Failed to load skill: {}", e)))?
            .ok_or_else(|| {
                AgentError::ToolExecution(format!("Skill '{}' not found", input.skill_id))
            })?;

        // Build system_prompt = skill body with `{{arg}}` placeholders
        // optionally interpolated when the caller passed `args`. Unsubstituted
        // `{{...}}` is no longer our concern — `PromptRegistry::escape_handlebars`
        // runs at the template-rendering boundary (see
        // `agent/strategy/planning/unified.rs`) and neutralises any leftover
        // template syntax in agent-authored content.
        let body = match input.args.as_ref() {
            Some(a) => interpolate_args(&skill.content, a),
            None => skill.content.clone(),
        };

        // Caller's user-message directive plus a JSON dump of `args` so the
        // worker can recover them even if the LLM that called us put the
        // per-call data in `prompt` instead of `args` (or vice versa).
        let prompt_text =
            build_prompt_with_args(input.prompt.clone(), &input.skill_id, input.args.as_ref());

        // Build the call_agent input as a typed value and serialize. Lets
        // serde handle field-name + skip-if-none consistently with the rest
        // of the call_agent dispatch path.
        let call_input_struct = super::universal_agent::CallAgentInput {
            agent: None,
            prompt: prompt_text,
            system_prompt: Some(body),
            tools: None,
            external: None,
            description: None,
            name: None,
            mode: parse_mode(input.mode.as_deref()),
            reason: None,
        };
        let inner_call = ToolCall {
            tool_call_id: tool_call.tool_call_id.clone(),
            tool_name: "call_agent".to_string(),
            input: serde_json::to_value(&call_input_struct).map_err(|e| {
                AgentError::ToolExecution(format!("Failed to encode call_agent input: {}", e))
            })?,
        };
        let universal = super::universal_agent::UniversalAgentTool;
        universal
            .execute_with_executor_context(inner_call, context.clone())
            .await
    }
}

/// Map our string-typed `mode` field to `CallMode`. Defaults to `fork` —
/// parent dispatches one tool_call per work item in a single turn and the
/// children run as parallel sub-agents (the fan-out shape). Unknown/typo
/// values fall back to the same default rather than erroring — keeps the
/// caller side typo-safe.
pub(crate) fn parse_mode(mode: Option<&str>) -> super::universal_agent::CallMode {
    use super::universal_agent::CallMode;
    match mode.unwrap_or("fork") {
        "in_process" => CallMode::InProcess,
        "fork" => CallMode::Fork,
        "offload" => CallMode::Offload,
        "transfer" => CallMode::Transfer,
        _ => CallMode::Fork,
    }
}

/// Compose the worker's user message: caller's prompt directive + a JSON
/// dump of `args` (if any). Belt-and-braces so the worker has the per-call
/// data regardless of whether the LLM populated `args`, `prompt`, or both.
pub(crate) fn build_prompt_with_args(
    caller_prompt: Option<String>,
    skill_id: &str,
    args: Option<&serde_json::Value>,
) -> String {
    let directive = caller_prompt.unwrap_or_else(|| format!("Run the '{}' skill.", skill_id));
    let args_obj = args.and_then(|a| a.as_object()).filter(|m| !m.is_empty());
    match args_obj {
        Some(_) => {
            let pretty = serde_json::to_string_pretty(args.unwrap())
                .unwrap_or_else(|_| args.unwrap().to_string());
            format!("{}\n\nargs:\n```json\n{}\n```", directive, pretty)
        }
        None => directive,
    }
}
