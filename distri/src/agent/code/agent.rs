use crate::agent::code::{CodeAnalyzer, CodeExecutor, CodeValidator};
use crate::agent::hooks::AgentHooks;
use crate::agent::standard::StandardAgent;
use crate::agent::types::{AgentDefinition, BaseAgent, ExecutorContext, StepResult};
use crate::error::AgentError;
use crate::llm::{LLMExecutor, LLMResponse};
use crate::tools::{LlmToolsRegistry, Tool, ToolCall};
use crate::types::Message;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

/// A custom agent that can reason and execute code
#[derive(Debug)]
pub struct CodeAgent {
    inner: StandardAgent,
    code_executor: CodeExecutor,
    code_tools: Vec<Box<dyn Tool>>,
    reasoning_mode: ReasoningMode,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ReasoningMode {
    /// Use LLM for reasoning, code for execution
    Hybrid,
    /// Use code for both reasoning and execution
    CodeOnly,
    /// Use LLM for both reasoning and execution
    LLMOnly,
}

impl Default for ReasoningMode {
    fn default() -> Self {
        ReasoningMode::Hybrid
    }
}

impl CodeAgent {
    pub fn new(
        definition: AgentDefinition,
        tools_registry: Arc<LlmToolsRegistry>,
        executor: Arc<crate::agent::executor::AgentExecutor>,
        session_store: Arc<Box<dyn crate::stores::SessionStore>>,
        context: Arc<ExecutorContext>,
    ) -> Self {
        let code_executor = CodeExecutor::new(context.clone());
        
        // Create code-specific tools
        let mut code_tools: Vec<Box<dyn Tool>> = vec![
            Box::new(code_executor.clone()),
            Box::new(CodeValidator),
            Box::new(CodeAnalyzer),
        ];

        // Add any additional tools from the registry
        for tool in tools_registry.tools.values() {
            code_tools.push(tool.clone());
        }

        Self {
            inner: StandardAgent::new(definition, tools_registry, executor, session_store),
            code_executor,
            code_tools,
            reasoning_mode: ReasoningMode::Hybrid,
        }
    }

    pub fn with_reasoning_mode(mut self, mode: ReasoningMode) -> Self {
        self.reasoning_mode = mode;
        self
    }

    pub fn with_code_function(mut self, name: &str, description: &str, parameters: Value) -> Self {
        use crate::agent::code::sandbox::FunctionDefinition;
        
        let function = FunctionDefinition::new(name.to_string())
            .with_description(description.to_string())
            .with_parameters(parameters);
        
        self.code_executor.add_function(function);
        self
    }

    /// Execute code and return the result
    async fn execute_code(&self, code: &str, context: &HashMap<String, Value>) -> Result<Value, AgentError> {
        info!("CodeAgent executing code: {}", code);
        
        match self.code_executor.execute_with_context(code, context).await {
            Ok(result) => {
                debug!("Code execution successful: {:?}", result);
                Ok(result)
            }
            Err(e) => {
                error!("Code execution failed: {}", e);
                Err(AgentError::ToolExecution(format!("Code execution failed: {}", e)))
            }
        }
    }

    /// Generate code using LLM reasoning
    async fn generate_code(&self, prompt: &str, context: &HashMap<String, Value>) -> Result<String, AgentError> {
        let code_generation_prompt = format!(
            "You are a JavaScript/TypeScript expert. Generate code that solves the following problem:\n\n{}\n\nContext: {:?}\n\nGenerate only the code, no explanations. The code should be complete and executable.",
            prompt, context
        );

        let message = Message::user(code_generation_prompt, None);
        let messages = vec![message];

        let llm_executor = LLMExecutor::new(
            self.inner.get_definition().model_settings.clone(),
            Arc::new(LlmToolsRegistry::new()),
            Arc::new(ExecutorContext::default()),
            None,
            None,
        );

        let response = llm_executor.execute(&messages).await?;
        Ok(response.content)
    }

    /// Analyze the task and decide on the best approach
    async fn analyze_task(&self, message: &Message) -> Result<ReasoningMode, AgentError> {
        let analysis_prompt = format!(
            "Analyze this task and determine the best approach:\n\n{}\n\nRespond with one of:\n- 'code': Task requires code execution or computation\n- 'llm': Task requires reasoning, analysis, or text generation\n- 'hybrid': Task requires both code and reasoning",
            message.content
        );

        let message = Message::user(analysis_prompt, None);
        let messages = vec![message];

        let llm_executor = LLMExecutor::new(
            self.inner.get_definition().model_settings.clone(),
            Arc::new(LlmToolsRegistry::new()),
            Arc::new(ExecutorContext::default()),
            None,
            None,
        );

        let response = llm_executor.execute(&messages).await?;
        let response_lower = response.content.to_lowercase();

        if response_lower.contains("code") {
            Ok(ReasoningMode::CodeOnly)
        } else if response_lower.contains("llm") {
            Ok(ReasoningMode::LLMOnly)
        } else {
            Ok(ReasoningMode::Hybrid)
        }
    }

    /// Execute a hybrid approach using both code and LLM reasoning
    async fn execute_hybrid(&self, message: Message, context: Arc<ExecutorContext>) -> Result<String, AgentError> {
        info!("Executing hybrid approach for: {}", message.content);

        // First, analyze the task
        let reasoning_mode = self.analyze_task(&message).await?;
        
        match reasoning_mode {
            ReasoningMode::CodeOnly => {
                // Generate and execute code
                let context_map = HashMap::new();
                let code = self.generate_code(&message.content, &context_map).await?;
                
                // Validate the generated code
                let validation_tool = CodeValidator;
                let validation_call = ToolCall {
                    tool_id: "validate_1".to_string(),
                    tool_name: "validate_code".to_string(),
                    input: serde_json::to_string(&serde_json::json!({
                        "code": code
                    }))?,
                };

                let validation_result = validation_tool.execute(
                    validation_call,
                    crate::tools::BuiltInToolContext {
                        agent_id: self.get_name().to_string(),
                        agent_store: Arc::new(crate::stores::memory::InMemoryAgentStore::new()),
                        context: context.clone(),
                        event_tx: None,
                        coordinator_tx: mpsc::channel(100).0,
                        tool_sessions: None,
                        registry: Arc::new(tokio::sync::RwLock::new(crate::servers::registry::McpServerRegistry::default())),
                    },
                ).await?;

                let validation: Value = serde_json::from_str(&validation_result)?;
                if !validation["valid"].as_bool().unwrap_or(false) {
                    warn!("Generated code validation failed: {:?}", validation);
                    // Fall back to LLM reasoning
                    return self.execute_llm_only(message, context).await;
                }

                // Execute the code
                let context_map = HashMap::new();
                let result = self.execute_code(&code, &context_map).await?;
                
                // Format the result
                let formatted_result = format!(
                    "Generated and executed code:\n```javascript\n{}\n```\n\nResult: {}",
                    code,
                    serde_json::to_string_pretty(&result)?
                );
                
                Ok(formatted_result)
            }
            ReasoningMode::LLMOnly => {
                self.execute_llm_only(message, context).await
            }
            ReasoningMode::Hybrid => {
                // Use LLM to generate a plan, then execute code based on the plan
                let planning_prompt = format!(
                    "Create a step-by-step plan to solve this problem:\n\n{}\n\nFor each step, indicate whether it requires code execution or reasoning. Return the plan as JSON.",
                    message.content
                );

                let plan_message = Message::user(planning_prompt, None);
                let plan_response = self.execute_llm_only(plan_message, context.clone()).await?;
                
                // Try to parse the plan and execute accordingly
                // For now, we'll use a simplified approach
                let context_map = HashMap::new();
                let code = self.generate_code(&message.content, &context_map).await?;
                let result = self.execute_code(&code, &context_map).await?;
                
                let formatted_result = format!(
                    "Plan: {}\n\nExecuted code:\n```javascript\n{}\n```\n\nResult: {}",
                    plan_response,
                    code,
                    serde_json::to_string_pretty(&result)?
                );
                
                Ok(formatted_result)
            }
        }
    }

    /// Execute using only LLM reasoning
    async fn execute_llm_only(&self, message: Message, context: Arc<ExecutorContext>) -> Result<String, AgentError> {
        self.inner.invoke(message, context, None).await
    }

    /// Execute using only code
    async fn execute_code_only(&self, message: Message, context: Arc<ExecutorContext>) -> Result<String, AgentError> {
        let context_map = HashMap::new();
        let code = self.generate_code(&message.content, &context_map).await?;
        let result = self.execute_code(&code, &context_map).await?;
        
        let formatted_result = format!(
            "Generated code:\n```javascript\n{}\n```\n\nResult: {}",
            code,
            serde_json::to_string_pretty(&result)?
        );
        
        Ok(formatted_result)
    }
}

#[async_trait]
impl BaseAgent for CodeAgent {
    async fn invoke(
        &self,
        message: Message,
        context: Arc<ExecutorContext>,
        event_tx: Option<mpsc::Sender<crate::agent::AgentEvent>>,
    ) -> Result<String, AgentError> {
        info!("CodeAgent invoked with reasoning mode: {:?}", self.reasoning_mode);

        match self.reasoning_mode {
            ReasoningMode::Hybrid => {
                self.execute_hybrid(message, context).await
            }
            ReasoningMode::CodeOnly => {
                self.execute_code_only(message, context).await
            }
            ReasoningMode::LLMOnly => {
                self.execute_llm_only(message, context).await
            }
        }
    }

    async fn invoke_stream(
        &self,
        message: Message,
        context: Arc<ExecutorContext>,
        event_tx: mpsc::Sender<crate::agent::AgentEvent>,
    ) -> Result<(), AgentError> {
        // For streaming, we'll use the hybrid approach but stream the results
        let result = self.execute_hybrid(message, context).await?;
        
        // Send the result as a stream
        let event = crate::agent::AgentEvent {
            event: crate::agent::AgentEventType::TextMessageContent {
                delta: result,
            },
            timestamp: chrono::Utc::now(),
        };
        
        event_tx.send(event).await
            .map_err(|e| AgentError::ToolExecution(format!("Failed to send event: {}", e)))?;
        
        Ok(())
    }

    fn clone_box(&self) -> Box<dyn BaseAgent> {
        // Create a new CodeAgent with the same configuration
        let definition = self.inner.get_definition();
        let tools_registry = Arc::new(LlmToolsRegistry::new()); // Simplified for cloning
        let executor = Arc::new(crate::agent::executor::AgentExecutorBuilder::default().build().unwrap());
        let session_store = Arc::new(Box::new(crate::stores::LocalSessionStore::new()) as Box<dyn crate::stores::SessionStore>);
        let context = Arc::new(ExecutorContext::default());
        
        let mut new_agent = CodeAgent::new(definition, tools_registry, executor, session_store, context);
        new_agent.reasoning_mode = self.reasoning_mode.clone();
        
        // Copy the code executor functions
        for function in self.code_executor.get_functions() {
            new_agent.code_executor.add_function(function);
        }
        
        Box::new(new_agent)
    }

    fn get_name(&self) -> &str {
        self.inner.get_name()
    }

    fn get_description(&self) -> &str {
        self.inner.get_description()
    }

    fn get_definition(&self) -> AgentDefinition {
        self.inner.get_definition()
    }

    fn get_tools(&self) -> Vec<&Box<dyn Tool>> {
        self.code_tools.iter().collect()
    }

    fn agent_type(&self) -> crate::agent::types::AgentType {
        crate::agent::types::AgentType::Custom("code_agent".to_string())
    }
}

#[async_trait]
impl AgentHooks for CodeAgent {
    async fn before_invoke(
        &self,
        message: Message,
        context: Arc<ExecutorContext>,
        event_tx: Option<mpsc::Sender<crate::agent::AgentEvent>>,
    ) -> Result<(), AgentError> {
        info!("CodeAgent before_invoke: {}", message.content);
        Ok(())
    }

    async fn after_execute(&self, response: LLMResponse) -> Result<LLMResponse, AgentError> {
        info!("CodeAgent after_execute: {:?}", response);
        Ok(response)
    }
}