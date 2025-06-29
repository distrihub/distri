use crate::{
    agent_store::{AgentExecutionContext, CustomAgent},
    error::AgentError,
    memory::{MemoryStep, SystemStep},
    types::{Message, MessageContent, MessageRole},
};

/// StepAgent that demonstrates custom agent behavior using the step() function
#[derive(Debug)]
pub struct StepAgent {
    pub name: String,
    pub execution_log: std::sync::Mutex<Vec<String>>,
}

impl StepAgent {
    pub fn new(name: String) -> Self {
        Self {
            name,
            execution_log: std::sync::Mutex::new(Vec::new()),
        }
    }

    pub fn get_execution_log(&self) -> Vec<String> {
        self.execution_log.lock().unwrap().clone()
    }

    pub fn clear_log(&self) {
        self.execution_log.lock().unwrap().clear();
    }

    fn log(&self, message: String) {
        self.execution_log.lock().unwrap().push(message.clone());
        tracing::info!("[{}] {}", self.name, message);
    }
}

#[async_trait::async_trait]
impl CustomAgent for StepAgent {
    async fn step(
        &self,
        context: &AgentExecutionContext,
    ) -> Result<String, AgentError> {
        self.log(format!("Starting step execution for task: {}", context.task.task));

        // 1. Write system prompt to session if available
        if let Ok(messages) = context.session_writer.get_messages().await {
            self.log(format!("Found {} existing messages in session", messages.len()));
        }

        // 2. Write a custom message to session
        let custom_message = MemoryStep::System(SystemStep {
            system_prompt: format!("Custom preprocessing by {}: Task received - {}", 
                self.name, context.task.task),
        });
        context.session_writer.write_step(custom_message).await?;
        self.log("Wrote custom preprocessing message to session".to_string());

        // 3. Create messages for LLM call
        let mut messages = context.session_writer.get_messages().await?;
        
        // Add the current task as a user message
        let user_message = Message {
            role: MessageRole::User,
            name: Some("user".to_string()),
            content: vec![MessageContent {
                content_type: "text".to_string(),
                text: Some(context.task.task.clone()),
                image: None,
            }],
            tool_calls: vec![],
        };
        messages.push(user_message);

        self.log(format!("Prepared {} messages for LLM call", messages.len()));

        // 4. Call LLM function
        let llm_response = context.llm_executor.llm(messages).await?;
        self.log(format!("LLM responded with: {}", llm_response));

        // 5. Do some custom post-processing
        let processed_response = format!(
            "Processed by {}: {}\n\nOriginal LLM response: {}",
            self.name,
            "Custom post-processing completed successfully",
            llm_response
        );

        // 6. Log parameters if provided
        if let Some(params) = &context.params {
            self.log(format!("Parameters provided: {}", params));
        }

        self.log("Step execution completed".to_string());
        Ok(processed_response)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// A custom agent that simulates API calls
#[derive(Debug)]
pub struct ApiAgent {
    pub name: String,
    pub api_calls: std::sync::Mutex<Vec<String>>,
}

impl ApiAgent {
    pub fn new(name: String) -> Self {
        Self {
            name,
            api_calls: std::sync::Mutex::new(Vec::new()),
        }
    }

    pub fn get_api_calls(&self) -> Vec<String> {
        self.api_calls.lock().unwrap().clone()
    }

    async fn simulate_api_call(&self, endpoint: &str, data: &str) -> Result<String, AgentError> {
        // Simulate network delay
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        
        let call = format!("API_CALL: {} with data: {}", endpoint, data);
        self.api_calls.lock().unwrap().push(call.clone());
        
        Ok(format!("API response from {}: {{\"status\": \"success\", \"data\": \"processed\"}}", endpoint))
    }
}

#[async_trait::async_trait]
impl CustomAgent for ApiAgent {
    async fn step(
        &self,
        context: &AgentExecutionContext,
    ) -> Result<String, AgentError> {
        // 1. Extract relevant data from task
        let task_data = &context.task.task;
        
        // 2. Make API calls based on task content
        let api_response = if task_data.contains("weather") {
            self.simulate_api_call("/api/weather", task_data).await?
        } else if task_data.contains("user") {
            self.simulate_api_call("/api/users", task_data).await?
        } else {
            self.simulate_api_call("/api/general", task_data).await?
        };

        // 3. Write API response to session
        let api_step = MemoryStep::System(SystemStep {
            system_prompt: format!("API Response: {}", api_response),
        });
        context.session_writer.write_step(api_step).await?;

        // 4. Get current messages and add context
        let mut messages = context.session_writer.get_messages().await?;
        
        // Add enriched context from API
        let context_message = Message {
            role: MessageRole::User,
            name: Some("api_context".to_string()),
            content: vec![MessageContent {
                content_type: "text".to_string(),
                text: Some(format!("Based on the API data: {}\n\nUser request: {}", 
                    api_response, context.task.task)),
                image: None,
            }],
            tool_calls: vec![],
        };
        messages.push(context_message);

        // 5. Call LLM with enriched context
        let llm_response = context.llm_executor.llm(messages).await?;

        // 6. Return combined response
        Ok(format!("API-Enhanced Response:\n{}\n\nAPI Calls Made: {:?}", 
            llm_response, self.get_api_calls()))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// A failing custom agent for testing error handling
#[derive(Debug)]
pub struct FailingStepAgent {
    pub should_fail: bool,
}

impl FailingStepAgent {
    pub fn new(should_fail: bool) -> Self {
        Self { should_fail }
    }
}

#[async_trait::async_trait]
impl CustomAgent for FailingStepAgent {
    async fn step(
        &self,
        _context: &AgentExecutionContext,
    ) -> Result<String, AgentError> {
        if self.should_fail {
            return Err(AgentError::ToolExecution("Simulated step failure".to_string()));
        }
        Ok("Success".to_string())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}