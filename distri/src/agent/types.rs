use serde::{Deserialize, Serialize};

use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use crate::{
    memory::TaskStep,
    tools::Tool,
    types::{Message, ToolCall},
    AgentDefinition, AgentError,
};
use async_openai::types::Role;

use serde_json::Value;
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "snake_case")]
pub struct AgentEvent {
    pub thread_id: String,
    pub run_id: String,
    pub event: AgentEventType,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "snake_case", tag = "type", content = "data")]
pub enum AgentEventType {
    RunStarted {},
    RunFinished {},
    RunError {
        message: String,
        code: Option<String>,
    },
    TextMessageStart {
        message_id: String,
        role: Role,
    },
    TextMessageContent {
        message_id: String,
        delta: String,
    },
    TextMessageEnd {
        message_id: String,
    },
    ToolCallStart {
        tool_call_id: String,
        tool_call_name: String,
    },
    ToolCallArgs {
        tool_call_id: String,
        delta: String,
    },
    ToolCallEnd {
        tool_call_id: String,
    },
    ToolCallResult {
        tool_call_id: String,
        result: String,
    },
    AgentHandover {
        from_agent: String,
        to_agent: String,
        reason: Option<String>,
    },
}

#[derive(Debug)]
pub enum CoordinatorMessage {
    ExecuteTools {
        agent_id: String,
        tool_call: ToolCall,
        event_tx: Option<mpsc::Sender<AgentEvent>>,
        context: Arc<ExecutorContext>,
    },
    Execute {
        agent_id: String,
        task: crate::memory::TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<ExecutorContext>,
        event_tx: Option<mpsc::Sender<AgentEvent>>,
        response_tx: oneshot::Sender<Result<String, crate::error::AgentError>>,
    },
    ExecuteStream {
        agent_id: String,
        task: crate::memory::TaskStep,
        params: Option<serde_json::Value>,
        event_tx: mpsc::Sender<AgentEvent>,
        context: Arc<ExecutorContext>,
    },
    HandoverAgent {
        from_agent: String,
        to_agent: String,
        reason: Option<String>,
        context: Arc<ExecutorContext>,
        event_tx: Option<mpsc::Sender<AgentEvent>>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExecutorContextMetadata {
    /// Add additional context for tools to use passed as meta in tool calls
    pub tools:
        std::collections::HashMap<String, std::collections::HashMap<String, serde_json::Value>>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}
#[derive(Debug, Clone)]
pub struct ExecutorContext {
    pub thread_id: String,
    pub run_id: Arc<tokio::sync::Mutex<String>>,
    pub verbose: bool,
    pub user_id: Option<String>,
    pub metadata: Option<ExecutorContextMetadata>,
    pub req_id: Option<Value>,
}

impl Default for ExecutorContext {
    fn default() -> Self {
        Self::new(Uuid::new_v4().to_string(), None, true, None, None, None)
    }
}

impl ExecutorContext {
    pub fn new(
        thread_id: String,
        run_id: Option<String>,
        verbose: bool,
        user_id: Option<String>,
        metadata: Option<ExecutorContextMetadata>,
        req_id: Option<Value>,
    ) -> Self {
        Self {
            thread_id,
            run_id: Arc::new(tokio::sync::Mutex::new(
                run_id.unwrap_or_else(|| Uuid::new_v4().to_string()),
            )),
            verbose,
            user_id,
            metadata,
            req_id,
        }
    }

    pub async fn new_run(&self) -> String {
        let new_run_id = Uuid::new_v4().to_string();
        *self.run_id.lock().await = new_run_id.clone();
        new_run_id
    }

    pub async fn update_run_id(&self, run_id: String) {
        let mut run_id_guard = self.run_id.lock().await;
        *run_id_guard = run_id;
    }
}

pub const MAX_ITERATIONS: i32 = 10;

/// Trait for agent hooks that can be chained together
#[async_trait::async_trait]
pub trait AgentHooks: Send + Sync {
    async fn after_task_step(
        &self,
        _task: TaskStep,
        _context: Arc<ExecutorContext>,
    ) -> Result<(), AgentError> {
        Ok(())
    }

    async fn before_llm_step(
        &self,
        messages: &[Message],
        _params: &Option<serde_json::Value>,
        _context: Arc<ExecutorContext>,
    ) -> Result<Vec<Message>, AgentError> {
        Ok(messages.to_vec())
    }

    async fn before_tool_calls(
        &self,
        tool_calls: &[ToolCall],
        _context: Arc<ExecutorContext>,
    ) -> Result<Vec<ToolCall>, AgentError> {
        Ok(tool_calls.to_vec())
    }

    async fn after_tool_calls(
        &self,
        _tool_responses: &[String],
        _context: Arc<ExecutorContext>,
    ) -> Result<(), AgentError> {
        Ok(())
    }

    async fn after_finish(
        &self,
        step_result: StepResult,
        _context: Arc<ExecutorContext>,
    ) -> Result<StepResult, AgentError> {
        Ok(step_result)
    }
}

#[async_trait::async_trait]
pub trait BaseAgent: Send + Sync + std::fmt::Debug {
    async fn validate(&self) -> Result<(), AgentError> {
        self.get_definition()
            .validate()
            .map_err(|e| AgentError::Validation(e.to_string()))?;
        Ok(())
    }

    async fn invoke(
        &self,
        task: TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<ExecutorContext>,
        event_tx: Option<mpsc::Sender<AgentEvent>>,
    ) -> Result<String, AgentError>;

    async fn invoke_stream(
        &self,
        _task: TaskStep,
        _params: Option<serde_json::Value>,
        _context: Arc<ExecutorContext>,
        _event_tx: mpsc::Sender<AgentEvent>,
    ) -> Result<(), AgentError> {
        Err(AgentError::NotImplemented(
            "BaseAgent::invoke_stream not implemented".to_string(),
        ))
    }

    /// Clone the agent (required for object safety)
    fn clone_box(&self) -> Box<dyn BaseAgent>;

    /// Get the agent's name/id
    fn get_name(&self) -> &str;

    fn get_description(&self) -> &str;
    fn get_definition(&self) -> AgentDefinition;
    fn get_tools(&self) -> Vec<&Box<dyn Tool>>;

    // Used in deserialization
    fn agent_type(&self) -> AgentType;

    /// Get hooks for this agent (default implementation returns None for standard agents)
    fn get_hooks(&self) -> Option<&dyn AgentHooks> {
        None
    }
}

/// Result of a single step execution
#[derive(Debug)]
pub enum StepResult {
    /// Continue with more iterations, with new messages to add
    Continue(Vec<Message>),
    /// Finish execution with this final response
    Finish(String),
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentType {
    #[default]
    Standard,
    Remote,
    Custom(String),
}
