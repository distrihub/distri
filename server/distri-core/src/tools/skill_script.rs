use std::sync::Arc;

use distri_types::{Part, ToolCall, tool::ToolContext};
use serde_json::json;

use crate::agent::ExecutorContext;
use crate::tools::ExecutorContextTool;
use crate::AgentError;

/// Tool that loads a skill's content on demand.
/// The agent calls this tool when it needs a specific skill.
/// The skill's markdown content is returned as-is, including
/// any script definitions and usage instructions embedded within.
#[derive(Debug, Clone)]
pub struct LoadSkillTool;

#[async_trait::async_trait]
impl distri_types::Tool for LoadSkillTool {
    fn get_name(&self) -> String {
        "load_skill".to_string()
    }

    fn get_description(&self) -> String {
        "Load a skill by its ID. Returns the skill's full content including instructions and scripts.".to_string()
    }

    fn get_parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "skill_id": {
                    "type": "string",
                    "description": "The ID of the skill to load"
                }
            },
            "required": ["skill_id"]
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
            "LoadSkillTool requires ExecutorContext, not ToolContext"
        ))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for LoadSkillTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let skill_id = tool_call
            .input
            .get("skill_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                AgentError::ToolExecution("Missing required parameter: skill_id".to_string())
            })?;

        let orchestrator = context.get_orchestrator()?;
        let skill_store = orchestrator
            .stores
            .skill_store
            .as_ref()
            .ok_or_else(|| {
                AgentError::ToolExecution("Skill store not configured".to_string())
            })?;

        let skill = skill_store
            .get_skill(skill_id)
            .await
            .map_err(|e| AgentError::ToolExecution(format!("Failed to load skill: {}", e)))?
            .ok_or_else(|| {
                AgentError::ToolExecution(format!("Skill '{}' not found", skill_id))
            })?;

        // Return the skill's content as-is - the content includes
        // all instructions and script usage details as authored
        Ok(vec![Part::Text(skill.content)])
    }
}

/// Tool that runs a script from a loaded skill.
/// The agent calls this after loading a skill to execute one of its scripts.
/// Scripts are executed directly using the JS sandbox runtime.
#[derive(Debug, Clone)]
pub struct RunSkillScriptTool;

#[async_trait::async_trait]
impl distri_types::Tool for RunSkillScriptTool {
    fn get_name(&self) -> String {
        "run_skill_script".to_string()
    }

    fn get_description(&self) -> String {
        "Run a script from a skill. Provide the skill ID and script name to execute the script with the given input.".to_string()
    }

    fn get_parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "skill_id": {
                    "type": "string",
                    "description": "The ID of the skill containing the script"
                },
                "script_name": {
                    "type": "string",
                    "description": "The name of the script to run"
                },
                "input": {
                    "type": "object",
                    "description": "Input parameters to pass to the script"
                }
            },
            "required": ["skill_id", "script_name"]
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
            "RunSkillScriptTool requires ExecutorContext, not ToolContext"
        ))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for RunSkillScriptTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let skill_id = tool_call
            .input
            .get("skill_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                AgentError::ToolExecution("Missing required parameter: skill_id".to_string())
            })?;

        let script_name = tool_call
            .input
            .get("script_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                AgentError::ToolExecution("Missing required parameter: script_name".to_string())
            })?;

        let input = tool_call
            .input
            .get("input")
            .cloned()
            .unwrap_or(serde_json::Value::Null);

        // Load the skill and find the script
        let orchestrator = context.get_orchestrator()?;
        let skill_store = orchestrator
            .stores
            .skill_store
            .as_ref()
            .ok_or_else(|| {
                AgentError::ToolExecution("Skill store not configured".to_string())
            })?;

        let skill = skill_store
            .get_skill(skill_id)
            .await
            .map_err(|e| AgentError::ToolExecution(format!("Failed to load skill: {}", e)))?
            .ok_or_else(|| {
                AgentError::ToolExecution(format!("Skill '{}' not found", skill_id))
            })?;

        let script = skill
            .scripts
            .iter()
            .find(|s| s.name == script_name)
            .ok_or_else(|| {
                let available: Vec<_> = skill.scripts.iter().map(|s| s.name.as_str()).collect();
                AgentError::ToolExecution(format!(
                    "Script '{}' not found in skill '{}'. Available scripts: {:?}",
                    script_name, skill.name, available
                ))
            })?;

        // Execute the script using the JS sandbox if available
        run_script(&script.code, &script.language, &input, &script.name, &context.env_vars).await
    }
}

/// Execute a script using the available runtime.
/// When the `code` feature is enabled, uses the JS sandbox for direct execution.
/// Otherwise, returns the script code with the input for the agent to interpret.
#[allow(unused_variables)]
async fn run_script(
    code: &str,
    language: &str,
    input: &serde_json::Value,
    script_name: &str,
    env_vars: &Option<std::collections::HashMap<String, String>>,
) -> Result<Vec<Part>, AgentError> {
    #[cfg(feature = "code")]
    {
        return run_script_with_sandbox(code, input, script_name, env_vars);
    }

    #[cfg(not(feature = "code"))]
    {
        // No JS sandbox available - return the script code with input for the agent to interpret
        let result = format!(
            "## Script: {} ({})\n\n### Code\n```{}\n{}\n```\n\n### Input\n```json\n{}\n```\n\n> Direct script execution is not available. Interpret and apply this script based on the code and input above.",
            script_name,
            language,
            language,
            code,
            serde_json::to_string_pretty(input).unwrap_or_else(|_| "null".to_string())
        );
        Ok(vec![Part::Text(result)])
    }
}

/// Execute script code using the JS sandbox runtime (requires `code` feature).
#[cfg(feature = "code")]
fn run_script_with_sandbox(
    code: &str,
    input: &serde_json::Value,
    script_name: &str,
    env_vars: &Option<std::collections::HashMap<String, String>>,
) -> Result<Vec<Part>, AgentError> {
    use distri_js_sandbox::{JsWorker, JsWorkerOptions, JsExecutor, JsWorkerError};

    // Minimal executor that doesn't provide external functions
    struct NoOpExecutor;

    #[async_trait::async_trait]
    impl JsExecutor for NoOpExecutor {
        async fn execute(&self, name: &str, _args: Vec<serde_json::Value>) -> Result<serde_json::Value, JsWorkerError> {
            Err(JsWorkerError::Other(format!("Function '{}' is not available in skill scripts", name)))
        }
        fn execute_sync(&self, name: &str, _args: Vec<serde_json::Value>) -> Result<serde_json::Value, JsWorkerError> {
            Err(JsWorkerError::Other(format!("Function '{}' is not available in skill scripts", name)))
        }
    }

    // Serialize env_vars to inject alongside input
    let env_vars_json = match env_vars {
        Some(vars) => serde_json::to_string(vars).unwrap_or_else(|_| "{}".to_string()),
        None => "{}".to_string(),
    };

    // Wrap the script code to inject input and env_vars as global constants
    let wrapped_code = format!(
        "const input = {};\nconst env = {};\n{}",
        serde_json::to_string(input).unwrap_or_else(|_| "null".to_string()),
        env_vars_json,
        code,
    );

    let options = JsWorkerOptions {
        timeout: std::time::Duration::from_secs(30),
        functions: vec![],
        executor: std::sync::Arc::new(NoOpExecutor),
    };

    let worker = JsWorker::new(options)
        .map_err(|e| AgentError::ToolExecution(format!("Failed to create JS runtime: {}", e)))?;

    let result: serde_json::Value = worker
        .execute(&wrapped_code)
        .map_err(|e| AgentError::ToolExecution(format!("Script '{}' execution failed: {}", script_name, e)))?;

    match result {
        serde_json::Value::Null => Ok(vec![Part::Text(format!("Script '{}' completed successfully (no return value).", script_name))]),
        value => Ok(vec![Part::Data(value)]),
    }
}
