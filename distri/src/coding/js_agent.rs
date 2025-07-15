use crate::{
    agent::{AgentEvent, AgentEventType, BaseAgent, ExecutorContext, StepResult},
    coding::{executor::JsExecutor, js_tools::JsToolRegistry},
    error::AgentError,
    llm::LLMExecutor,
    memory::{ActionStep, MemoryStep, TaskStep},
    tools::LlmToolsRegistry,
    types::{AgentDefinition, Message, MessageContent, MessageRole},
    SessionStore,
};
use async_openai::types::Role;
use regex::Regex;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::info;
use uuid::Uuid;

/// JavaScript agent that generates and executes JavaScript code instead of using traditional tool calls
pub struct JsAgent {
    definition: AgentDefinition,
    tools_registry: Arc<LlmToolsRegistry>,
    js_tool_registry: Arc<JsToolRegistry>,
    js_executor: JsExecutor,
    session_store: Arc<Box<dyn SessionStore>>,
    verbose: bool,
}

impl std::fmt::Debug for JsAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JsAgent")
            .field("definition", &self.definition)
            .finish()
    }
}

impl JsAgent {
    pub fn new(
        definition: AgentDefinition,
        tools_registry: Arc<LlmToolsRegistry>,
        session_store: Arc<Box<dyn SessionStore>>,
        context: Arc<ExecutorContext>,
    ) -> Result<Self, AgentError> {
        // Convert tools to JsToolRegistry
        let js_tools = tools_registry.tools.clone();
        let js_tool_registry = Arc::new(JsToolRegistry::new(js_tools));

        // Create JavaScript executor
        let js_executor = JsExecutor::new(js_tool_registry.clone())?;

        Ok(Self {
            definition,
            tools_registry,
            js_tool_registry,
            js_executor,
            session_store,
            verbose: context.verbose,
        })
    }

    /// Generate the system prompt for JavaScript code generation
    fn generate_system_prompt(&self) -> String {
        let tool_descriptions = self.js_tool_registry.get_tool_descriptions();
        let function_schemas = self.js_tool_registry.generate_function_schemas();

        format!(
            r#"You are a JavaScript coding agent. Your task is to write JavaScript code to solve problems.

Available tools and functions:
{}

Function schemas:
{}

IMPORTANT INSTRUCTIONS:
1. Write valid JavaScript code that uses the available functions
2. Use console.log() for debugging and intermediate outputs
3. Use finalAnswer(value) when you have the final result
4. Use setOutput(value) for intermediate outputs
5. Use setVariable(name, value) to store variables for future use
6. Handle errors gracefully with try-catch blocks
7. Return the result object with output, logs, is_final_answer, and variables

Example code structure:
```javascript
try {{
    // Your code here
    const result = someFunction(parameters);
    console.log('Result:', result);
    
    if (isFinalResult) {{
        finalAnswer(result);
    }} else {{
        setOutput(result);
    }}
}} catch (error) {{
    console.error('Error:', error);
    setOutput('Error: ' + error.message);
}}
```

Your response should be valid JavaScript code that can be executed directly."#,
            tool_descriptions, function_schemas
        )
    }

    /// Extract JavaScript code from LLM response
    fn extract_js_code(&self, response: &str) -> String {
        // Look for code blocks
        let code_block_regex = Regex::new(r"```(?:javascript|js)?\s*\n(.*?)\n```").unwrap();
        if let Some(captures) = code_block_regex.captures(response) {
            return captures[1].trim().to_string();
        }

        // If no code blocks, try to extract code after "```javascript" or "```js"
        let js_start_regex = Regex::new(r"```(?:javascript|js)\s*\n").unwrap();
        if let Some(m) = js_start_regex.find(response) {
            let after_start = &response[m.end()..];
            if let Some(end_pos) = after_start.find("```") {
                return after_start[..end_pos].trim().to_string();
            }
        }

        // If still no code found, return the entire response as code
        response.trim().to_string()
    }

    /// Execute a single step with JavaScript code generation
    async fn execute_step(
        &self,
        task: &TaskStep,
        context: Arc<ExecutorContext>,
        event_tx: Option<mpsc::Sender<AgentEvent>>,
    ) -> Result<StepResult, AgentError> {
        // Get conversation history
        let messages = self
            .session_store
            .get_messages(&context.thread_id)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))?;

        // Create LLM executor
        let llm_executor = LLMExecutor::new(
            self.definition.into(),
            Arc::default(),
            context.clone(),
            None,
            Some("js_agent".to_string()),
        );

        // Prepare messages for LLM
        let mut llm_messages = vec![Message {
            role: MessageRole::System,
            name: None,
            content: vec![MessageContent {
                content_type: "text".to_string(),
                text: Some(self.generate_system_prompt()),
                image: None,
            }],
            tool_calls: vec![],
        }];

        // Add conversation history
        llm_messages.extend(messages);

        // Add current task
        llm_messages.push(Message {
            role: MessageRole::User,
            name: None,
            content: vec![MessageContent {
                content_type: "text".to_string(),
                text: Some(format!("Task: {}", task.task)),
                image: None,
            }],
            tool_calls: vec![],
        });

        // Generate JavaScript code
        let response = llm_executor.execute(&llm_messages, None).await?;
        let js_code = self.extract_js_code(&response.content);

        if self.verbose {
            info!("Generated JavaScript code:\n{}", js_code);
        }

        // Execute the JavaScript code
        let code_output = self.js_executor.execute(&js_code).await?;

        if self.verbose {
            info!("Code execution output: {:?}", code_output);
        }

        // Emit events if available
        if let Some(tx) = event_tx {
            let _ = tx
                .send(AgentEvent {
                    thread_id: context.thread_id.clone(),
                    run_id: context.run_id.lock().await.clone(),
                    event: AgentEventType::TextMessageStart {
                        message_id: Uuid::new_v4().to_string(),
                        role: Role::Assistant,
                    },
                })
                .await;

            let _ = tx
                .send(AgentEvent {
                    thread_id: context.thread_id.clone(),
                    run_id: context.run_id.lock().await.clone(),
                    event: AgentEventType::TextMessageContent {
                        message_id: Uuid::new_v4().to_string(),
                        delta: format!(
                            "Generated code:\n```javascript\n{}\n```\n\nOutput:\n{}",
                            js_code, code_output.output
                        ),
                    },
                })
                .await;

            let _ = tx
                .send(AgentEvent {
                    thread_id: context.thread_id.clone(),
                    run_id: context.run_id.lock().await.clone(),
                    event: AgentEventType::TextMessageEnd {
                        message_id: Uuid::new_v4().to_string(),
                    },
                })
                .await;
        }

        // Store the interaction in session
        let assistant_message = Message {
            role: MessageRole::Assistant,
            name: None,
            content: vec![MessageContent {
                content_type: "text".to_string(),
                text: Some(format!(
                    "Generated JavaScript code:\n```javascript\n{}\n```\n\nOutput:\n{}",
                    js_code, code_output.output
                )),
                image: None,
            }],
            tool_calls: vec![],
        };

        self.session_store
            .add_message(&context.thread_id, assistant_message)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))?;

        // Return result based on whether it's a final answer
        if code_output.is_final_answer {
            Ok(StepResult::Finish(code_output.output))
        } else {
            // Add the output as a tool response message
            let tool_response = Message {
                role: MessageRole::ToolResponse,
                name: None,
                content: vec![MessageContent {
                    content_type: "text".to_string(),
                    text: Some(code_output.output.clone()),
                    image: None,
                }],
                tool_calls: vec![],
            };

            self.session_store
                .add_message(&context.thread_id, tool_response)
                .await
                .map_err(|e| AgentError::Session(e.to_string()))?;

            Ok(StepResult::Continue(vec![Message {
                role: MessageRole::Assistant,
                name: None,
                content: vec![MessageContent {
                    content_type: "text".to_string(),
                    text: Some(format!("Code execution result: {}", code_output.output)),
                    image: None,
                }],
                tool_calls: vec![],
            }]))
        }
    }
}

impl BaseAgent for JsAgent {
    fn agent_type(&self) -> crate::agent::AgentType {
        crate::agent::AgentType::Custom("js_agent".to_string())
    }

    fn get_definition(&self) -> AgentDefinition {
        self.definition.clone()
    }

    fn get_description(&self) -> &str {
        &self.definition.description
    }

    fn get_tools(&self) -> Vec<&Box<dyn crate::tools::Tool>> {
        self.tools_registry.tools.values().collect()
    }

    async fn invoke(
        &self,
        task: TaskStep,
        params: Option<Value>,
        context: Arc<ExecutorContext>,
        event_tx: Option<mpsc::Sender<AgentEvent>>,
    ) -> Result<String, AgentError> {
        // Store the task in session
        let user_message = Message {
            role: MessageRole::User,
            name: None,
            content: vec![MessageContent {
                content_type: "text".to_string(),
                text: Some(task.task.clone()),
                image: None,
            }],
            tool_calls: vec![],
        };

        self.session_store
            .add_message(&context.thread_id, user_message)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))?;

        // Execute steps until we get a final answer or reach max iterations
        let max_iterations = self.definition.max_iterations.unwrap_or(10);
        let mut iteration = 0;

        while iteration < max_iterations {
            iteration += 1;

            if self.verbose {
                info!("JsAgent iteration {} of {}", iteration, max_iterations);
            }

            let step_result = self
                .execute_step(&task, context.clone(), event_tx.clone())
                .await?;

            match step_result {
                StepResult::Finish(final_answer) => {
                    return Ok(final_answer);
                }
                StepResult::Continue(_) => {
                    // Continue to next iteration
                    continue;
                }
            }
        }

        Err(AgentError::MaxIterationsReached(
            "Maximum iterations reached without finding final answer".to_string(),
        ))
    }

    fn clone_box(&self) -> Box<dyn BaseAgent> {
        Box::new(Self {
            definition: self.definition.clone(),
            tools_registry: self.tools_registry.clone(),
            js_tool_registry: self.js_tool_registry.clone(),
            js_executor: JsExecutor::new(self.js_tool_registry.clone())
                .expect("Failed to clone JsExecutor"),
            session_store: self.session_store.clone(),
            verbose: self.verbose,
        })
    }

    fn get_name(&self) -> &str {
        &self.definition.name
    }
}
