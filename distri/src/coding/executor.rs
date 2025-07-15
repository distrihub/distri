use crate::coding::js_tools::JsToolRegistry;
use crate::error::AgentError;
use rustyscript::{js_fn, JsValue, RuntimeOptions};
use rustyscript::{Context, JsValue, Module, Runtime};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Output from JavaScript code execution
#[derive(Debug, Clone)]
pub struct JsCodeOutput {
    pub output: String,
    pub logs: String,
    pub is_final_answer: bool,
    pub variables: HashMap<String, Value>,
}

/// JavaScript executor for sandboxed code execution
pub struct JsExecutor {
    runtime: Runtime,

    tool_registry: Arc<JsToolRegistry>,
    variables: Arc<RwLock<HashMap<String, Value>>>,
}

impl JsExecutor {
    pub fn new(tool_registry: Arc<JsToolRegistry>) -> Result<Self, AgentError> {
        let runtime = Runtime::new(RuntimeOptions {
            extensions: vec![],
            extension_options: ext::ExtensionOptions::default(),
            default_entrypoint: None,
            timeout: Duration::from_secs(10),
            max_heap_size: None,
        })
        .map_err(|e| AgentError::Execution(e.to_string()))?;

        Ok(Self {
            runtime,

            tool_registry,
            variables: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Execute JavaScript code with access to registered tools
    pub async fn execute(&self, code: &str) -> Result<JsCodeOutput, AgentError> {
        // Create a module with the code and registered tools
        let module_code = self.create_module_code(code);

        let module = Module::new("main.js", &module_code)
            .map_err(|e| AgentError::Execution(format!("Module creation failed: {}", e)))?;

        // Add standard JavaScript functions and objects
        self.add_standard_js_functions()?;

        // Add tool functions to the context
        self.add_tool_functions()?;

        // Execute the module
        let result = module
            .eval()
            .map_err(|e| AgentError::Execution(format!("Code execution failed: {}", e)))?;

        // Extract output and check for final answer
        let output = self.extract_output(&result)?;
        let is_final_answer = self.check_final_answer(&result)?;
        let logs = self.extract_logs(&result)?;

        // Update variables
        self.update_variables(&result).await?;

        Ok(JsCodeOutput {
            output,
            logs,
            is_final_answer,
            variables: self.variables.read().await.clone(),
        })
    }

    /// Create module code with proper setup
    fn create_module_code(&self, code: &str) -> String {
        format!(
            r#"
// Standard JavaScript utilities
const console = {{
    log: (...args) => {{
        if (typeof global_logs === 'undefined') {{
            global_logs = [];
        }}
        global_logs.push(args.map(arg => String(arg)).join(' '));
    }},
    error: (...args) => {{
        if (typeof global_logs === 'undefined') {{
            global_logs = [];
        }}
        global_logs.push('ERROR: ' + args.map(arg => String(arg)).join(' '));
    }}
}};

// Global variables for state management
let global_output = '';
let global_logs = [];
let global_variables = {{}};

// Helper function to set final answer
function finalAnswer(value) {{
    global_output = String(value);
    global_is_final_answer = true;
    return value;
}}

// Helper function to set output
function setOutput(value) {{
    global_output = String(value);
    return value;
}}

// Helper function to set variable
function setVariable(name, value) {{
    global_variables[name] = value;
    return value;
}}

// Main code execution
let global_is_final_answer = false;

try {{
    {}
}} catch (error) {{
    console.error('Code execution error:', error.message);
    global_output = 'Error: ' + error.message;
}}

// Return result
{{
    output: global_output,
    logs: global_logs.join('\n'),
    is_final_answer: global_is_final_answer,
    variables: global_variables
}}
"#,
            code
        )
    }

    /// Add standard JavaScript functions to the context
    fn add_standard_js_functions(&self) -> Result<(), AgentError> {
        let global = self.context.global();

        // Add JSON functions
        global
            .set(
                "JSON",
                json!({
                    "stringify": js_fn!(|value: JsValue| -> Result<JsValue, rustyscript::Error> {
                        Ok(JsValue::String(serde_json::to_string(&value).unwrap_or_default()))
                    }),
                    "parse": js_fn!(|text: JsValue| -> Result<JsValue, rustyscript::Error> {
                        let text_str = text.as_string().unwrap_or_default();
                        let parsed: Value = serde_json::from_str(&text_str).unwrap_or(json!(null));
                        Ok(JsValue::from(parsed))
                    })
                }),
            )
            .map_err(|e| AgentError::Execution(format!("Failed to add JSON: {}", e)))?;

        // Add Math functions
        global
            .set(
                "Math",
                json!({
                    "floor": js_fn!(|value: JsValue| -> Result<JsValue, rustyscript::Error> {
                        let num = value.as_f64().unwrap_or(0.0);
                        Ok(JsValue::Number(num.floor()))
                    }),
                    "ceil": js_fn!(|value: JsValue| -> Result<JsValue, rustyscript::Error> {
                        let num = value.as_f64().unwrap_or(0.0);
                        Ok(JsValue::Number(num.ceil()))
                    }),
                    "round": js_fn!(|value: JsValue| -> Result<JsValue, rustyscript::Error> {
                        let num = value.as_f64().unwrap_or(0.0);
                        Ok(JsValue::Number(num.round()))
                    }),
                    "abs": js_fn!(|value: JsValue| -> Result<JsValue, rustyscript::Error> {
                        let num = value.as_f64().unwrap_or(0.0);
                        Ok(JsValue::Number(num.abs()))
                    }),
                    "max": js_fn!(|a: JsValue, b: JsValue| -> Result<JsValue, rustyscript::Error> {
                        let a_num = a.as_f64().unwrap_or(f64::NEG_INFINITY);
                        let b_num = b.as_f64().unwrap_or(f64::NEG_INFINITY);
                        Ok(JsValue::Number(a_num.max(b_num)))
                    }),
                    "min": js_fn!(|a: JsValue, b: JsValue| -> Result<JsValue, rustyscript::Error> {
                        let a_num = a.as_f64().unwrap_or(f64::INFINITY);
                        let b_num = b.as_f64().unwrap_or(f64::INFINITY);
                        Ok(JsValue::Number(a_num.min(b_num)))
                    })
                }),
            )
            .map_err(|e| AgentError::Execution(format!("Failed to add Math: {}", e)))?;

        // Add String functions
        global
            .set(
                "String",
                js_fn!(|value: JsValue| -> Result<JsValue, rustyscript::Error> {
                    Ok(JsValue::String(value.to_string()))
                }),
            )
            .map_err(|e| AgentError::Execution(format!("Failed to add String: {}", e)))?;

        // Add Number functions
        global
            .set(
                "Number",
                js_fn!(|value: JsValue| -> Result<JsValue, rustyscript::Error> {
                    let num = value.as_f64().unwrap_or(0.0);
                    Ok(JsValue::Number(num))
                }),
            )
            .map_err(|e| AgentError::Execution(format!("Failed to add Number: {}", e)))?;

        Ok(())
    }

    /// Add tool functions to the context
    fn add_tool_functions(&self) -> Result<(), AgentError> {
        let global = self.context.global();

        // Add tool functions as JavaScript functions
        for (name, _tool) in &self.tool_registry.tools {
            let tool_name = name.clone();
            let tool_registry = self.tool_registry.clone();

            let js_fn = js_fn!(
                move |args: JsValue| -> Result<JsValue, rustyscript::Error> {
                    let args_str = args.as_string().unwrap_or_default();

                    // For now, return a placeholder response
                    // In a full implementation, you would execute the actual tool here
                    Ok(JsValue::String(format!(
                        "Tool {} called with: {}",
                        tool_name, args_str
                    )))
                }
            );

            global.set(name, js_fn).map_err(|e| {
                AgentError::Execution(format!("Failed to register tool {}: {}", name, e))
            })?;
        }

        Ok(())
    }

    /// Extract output from execution result
    fn extract_output(&self, result: &JsValue) -> Result<String, AgentError> {
        if let Some(obj) = result.as_object() {
            if let Some(output) = obj.get("output") {
                return Ok(output.as_string().unwrap_or_default());
            }
        }
        Ok(result.to_string())
    }

    /// Check if the result indicates a final answer
    fn check_final_answer(&self, result: &JsValue) -> Result<bool, AgentError> {
        if let Some(obj) = result.as_object() {
            if let Some(is_final) = obj.get("is_final_answer") {
                return Ok(is_final.as_bool().unwrap_or(false));
            }
        }
        Ok(false)
    }

    /// Extract logs from execution result
    fn extract_logs(&self, result: &JsValue) -> Result<String, AgentError> {
        if let Some(obj) = result.as_object() {
            if let Some(logs) = obj.get("logs") {
                return Ok(logs.as_string().unwrap_or_default());
            }
        }
        Ok(String::new())
    }

    /// Update variables from execution result
    async fn update_variables(&self, result: &JsValue) -> Result<(), AgentError> {
        if let Some(obj) = result.as_object() {
            if let Some(variables) = obj.get("variables") {
                if let Some(vars_obj) = variables.as_object() {
                    let mut vars = self.variables.write().await;
                    for (key, value) in vars_obj.iter() {
                        let key_str = key.as_string().unwrap_or_default();
                        let value_json: Value =
                            serde_json::from_str(&value.to_string()).unwrap_or(json!(null));
                        vars.insert(key_str, value_json);
                    }
                }
            }
        }
        Ok(())
    }

    /// Set variables that will be available in the next execution
    pub async fn set_variables(&self, variables: HashMap<String, Value>) {
        let mut vars = self.variables.write().await;
        *vars = variables;
    }

    /// Get current variables
    pub async fn get_variables(&self) -> HashMap<String, Value> {
        self.variables.read().await.clone()
    }
}
