mod server;
use std::collections::HashMap;

use serde_json::Value;
pub use server::{build_server, DISTRI_LOCAL_SERVER};
mod local;
pub use local::*;
mod log;
pub use log::*;
pub mod reason;
use crate::{error::AgentError, memory::TaskStep, types::ToolCall};
use tokio::sync::{mpsc, oneshot, Mutex};

// Event types for streaming responses
#[derive(Debug, Clone)]
pub enum AgentEvent {
    RunStarted {
        thread_id: String,
        run_id: String,
    },
    RunFinished {
        thread_id: String,
        run_id: String,
    },
    RunError {
        thread_id: String,
        run_id: String,
        message: String,
        code: Option<String>,
    },
    StepStarted {
        thread_id: String,
        run_id: String,
        step_name: String,
    },
    StepFinished {
        thread_id: String,
        run_id: String,
        step_name: String,
    },
    TextMessageStart {
        thread_id: String,
        run_id: String,
        message_id: String,
        role: String,
    },
    TextMessageContent {
        thread_id: String,
        run_id: String,
        message_id: String,
        delta: String,
    },
    TextMessageEnd {
        thread_id: String,
        run_id: String,
        message_id: String,
    },
    ToolCallStart {
        thread_id: String,
        run_id: String,
        tool_call_id: String,
        tool_call_name: String,
        parent_message_id: Option<String>,
    },
    ToolCallArgs {
        thread_id: String,
        run_id: String,
        tool_call_id: String,
        delta: String,
    },
    ToolCallEnd {
        thread_id: String,
        run_id: String,
        tool_call_id: String,
    },
    StateSnapshot {
        thread_id: String,
        run_id: String,
        snapshot: Value,
    },
    StateDelta {
        thread_id: String,
        run_id: String,
        delta: Value,
    },
    MessagesSnapshot {
        thread_id: String,
        run_id: String,
        messages: Vec<Value>,
    },
}

// Message types for coordinator communication
#[derive(Debug)]
pub enum CoordinatorMessage {
    ExecuteTool {
        agent_id: String,
        tool_call: ToolCall,
        response_tx: oneshot::Sender<String>,
    },
    Execute {
        agent_id: String,
        task: TaskStep,
        params: Option<serde_json::Value>,
        context: std::sync::Arc<CoordinatorContext>,
        event_tx: Option<tokio::sync::mpsc::Sender<AgentEvent>>,
        response_tx: oneshot::Sender<Result<String, AgentError>>,
    },
    ExecuteStream {
        agent_id: String,
        task: TaskStep,
        params: Option<serde_json::Value>,
        event_tx: tokio::sync::mpsc::Sender<AgentEvent>,
        context: std::sync::Arc<CoordinatorContext>,
    },
}

#[derive(Debug, Clone)]
pub struct AgentHandle {
    pub agent_id: String,
    pub coordinator_tx: mpsc::Sender<CoordinatorMessage>,
    pub verbose: bool,
}

#[async_trait::async_trait]
pub trait AgentCoordinator {
    async fn execute(
        &self,
        agent_name: &str,
        task: TaskStep,
        params: Option<serde_json::Value>,
        context: std::sync::Arc<CoordinatorContext>,
        event_tx: Option<tokio::sync::mpsc::Sender<AgentEvent>>,
    ) -> Result<String, AgentError>;
    async fn execute_stream(
        &self,
        agent_name: &str,
        task: TaskStep,
        params: Option<serde_json::Value>,
        event_tx: tokio::sync::mpsc::Sender<AgentEvent>,
        context: std::sync::Arc<CoordinatorContext>,
    ) -> Result<(), AgentError>;
}

impl AgentHandle {
    pub async fn execute_tool(&self, tool_call: ToolCall) -> Result<String, AgentError> {
        let (response_tx, response_rx) = oneshot::channel();
        self.coordinator_tx
            .send(CoordinatorMessage::ExecuteTool {
                agent_id: self.agent_id.clone(),
                tool_call,
                response_tx,
            })
            .await
            .map_err(|e| {
                AgentError::ToolExecution(format!("Failed to send tool execution request: {}", e))
            })?;

        response_rx.await.map_err(|e| {
            AgentError::ToolExecution(format!("Failed to receive tool response: {}", e))
        })
    }

    pub async fn execute(
        &self,
        task: TaskStep,
        params: Option<serde_json::Value>,
        context: std::sync::Arc<CoordinatorContext>,
        event_tx: Option<tokio::sync::mpsc::Sender<AgentEvent>>,
    ) -> Result<String, AgentError> {
        let (response_tx, response_rx) = oneshot::channel();
        self.coordinator_tx
            .send(CoordinatorMessage::Execute {
                agent_id: self.agent_id.clone(),
                task,
                params,
                context,
                event_tx,
                response_tx,
            })
            .await
            .map_err(|e| AgentError::ToolExecution(format!("Failed to execute agent: {}", e)))?;

        response_rx.await.map_err(|e| {
            AgentError::ToolExecution(format!("Failed to receive execution response: {}", e))
        })?
    }

    pub async fn execute_stream(
        &self,
        task: TaskStep,
        params: Option<serde_json::Value>,
        event_tx: tokio::sync::mpsc::Sender<AgentEvent>,
        context: std::sync::Arc<CoordinatorContext>,
    ) -> Result<(), AgentError> {
        self.coordinator_tx
            .send(CoordinatorMessage::ExecuteStream {
                agent_id: self.agent_id.clone(),
                task,
                params,
                event_tx,
                context,
            })
            .await
            .map_err(|e| AgentError::ToolExecution(format!("Failed to execute agent: {}", e)))
    }
}

#[derive(Debug)]
pub struct CoordinatorContext {
    pub thread_id: String,
    pub run_id: Mutex<String>,
    pub verbose: bool,
    pub user_id: Option<String>,
    /// Add additional context for tools to use passed as meta in MCP calls
    pub tools_context: HashMap<String, HashMap<String, Value>>,
}

impl Default for CoordinatorContext {
    fn default() -> Self {
        Self::new(
            uuid::Uuid::new_v4().to_string(),
            uuid::Uuid::new_v4().to_string(),
            true,
            None,
            HashMap::new(),
        )
    }
}

impl CoordinatorContext {
    pub fn new(
        thread_id: String,
        run_id: String,
        verbose: bool,
        user_id: Option<String>,
        tools_context: HashMap<String, HashMap<String, Value>>,
    ) -> Self {
        Self {
            thread_id,
            run_id: Mutex::new(run_id),
            verbose,
            user_id,
            tools_context,
        }
    }

    pub async fn update_run_id(&self, run_id: String) {
        let mut run_id_guard = self.run_id.lock().await;
        *run_id_guard = run_id;
    }
}
