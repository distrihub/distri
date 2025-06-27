mod server;
use std::collections::HashMap;

use serde_json::Value;
pub use server::{build_server, DISTRI_LOCAL_SERVER};
mod local;
pub use local::*;
mod log;
pub use log::*;
mod reason;
use crate::{
    error::AgentError,
    memory::TaskStep,
    types::{AgentDefinition, ServerTools, ToolCall},
};
use tokio::sync::{mpsc, oneshot, Mutex};

// AG-UI protocol compliant event types
#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// AGUI: RUN_STARTED
    RunStarted {
        run_id: String,
    },
    /// AGUI: RUN_FINISHED
    RunFinished {
        run_id: String,
    },
    /// AGUI: RUN_ERROR
    RunError {
        run_id: String,
        message: String,
        code: Option<String>,
    },
    /// AGUI: TEXT_MESSAGE_START
    TextMessageStart {
        run_id: String,
        message_id: String,
        role: String,
    },
    /// AGUI: TEXT_MESSAGE_CONTENT
    TextMessageContent {
        run_id: String,
        message_id: String,
        delta: String,
    },
    /// AGUI: TEXT_MESSAGE_END
    TextMessageEnd {
        run_id: String,
        message_id: String,
    },
    /// AGUI: TOOL_CALL_START
    ToolCallStart {
        run_id: String,
        tool_call_id: String,
        tool_name: String,
    },
    /// AGUI: TOOL_CALL_ARGS
    ToolCallArgs {
        run_id: String,
        tool_call_id: String,
        delta: String,
    },
    /// AGUI: TOOL_CALL_END
    ToolCallEnd {
        run_id: String,
        tool_call_id: String,
    },
    /// AGUI: TOOL_CALL_RESULT
    ToolResult {
        run_id: String,
        tool_call_id: String,
        result: String,
    },
    /// AGUI: STATE_SNAPSHOT
    StateSnapshot {
        run_id: String,
        snapshot: Value,
    },
    /// AGUI: STATE_DELTA
    StateDelta {
        run_id: String,
        delta: Value,
    },
    /// AGUI: CUSTOM (for thinking events, use customType: THINKING_START, THINKING_CONTENT, THINKING_END)
    ThinkingStart {
        run_id: String,
        thinking_id: String,
    },
    ThinkingContent {
        run_id: String,
        thinking_id: String,
        delta: String,
    },
    ThinkingEnd {
        run_id: String,
        thinking_id: String,
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
        response_tx: oneshot::Sender<Result<String, AgentError>>,
    },
    ExecuteStream {
        agent_id: String,
        task: TaskStep,
        params: Option<serde_json::Value>,
        event_tx: mpsc::Sender<AgentEvent>,
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
    async fn list_agents(
        &self,
        cursor: Option<String>,
    ) -> Result<(Vec<AgentDefinition>, Option<String>), AgentError>;
    async fn get_agent(&self, agent_name: &str) -> Result<AgentDefinition, AgentError>;
    async fn get_tools(&self, agent_name: &str) -> Result<Vec<ServerTools>, AgentError>;
    async fn execute(
        &self,
        agent_name: &str,
        task: TaskStep,
        params: Option<serde_json::Value>,
    ) -> Result<String, AgentError>;
    async fn execute_stream(
        &self,
        agent_name: &str,
        task: TaskStep,
        params: Option<serde_json::Value>,
        event_tx: mpsc::Sender<AgentEvent>,
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
    ) -> Result<String, AgentError> {
        let (response_tx, response_rx) = oneshot::channel();
        self.coordinator_tx
            .send(CoordinatorMessage::Execute {
                agent_id: self.agent_id.clone(),
                task,
                params,
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
        event_tx: mpsc::Sender<AgentEvent>,
    ) -> Result<(), AgentError> {
        self.coordinator_tx
            .send(CoordinatorMessage::ExecuteStream {
                agent_id: self.agent_id.clone(),
                task,
                params,
                event_tx,
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
