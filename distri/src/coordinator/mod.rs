mod server;
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
use tokio::sync::{mpsc, oneshot};
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
}
