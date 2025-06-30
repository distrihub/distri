pub mod agent;
pub mod executor;
pub mod log;
pub mod reason;
pub mod server;

pub use agent::{
    AgentInvoke, AgentInvokeStream, BaseAgent, DefaultAgent, StepResult, TestCustomAgent,
    MAX_ITERATIONS,
};
pub use executor::{AgentExecutor, CoordinatorMessage, ExecutorContext};
pub use log::StepLogger;
pub use server::DISTRI_LOCAL_SERVER;

use crate::types::{Message, MessageContent, MessageRole, ToolCall};
use async_openai::types::CreateChatCompletionResponse;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "snake_case")]
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
        tool_name: String,
    },
    ToolCallContent {
        thread_id: String,
        run_id: String,
        tool_call_id: String,
        content: String,
    },
    ToolCallEnd {
        thread_id: String,
        run_id: String,
        tool_call_id: String,
    },
}

#[derive(Debug)]
pub enum CoordinatorMessage {
    ExecuteTool {
        agent_id: String,
        tool_call: ToolCall,
        response_tx: oneshot::Sender<String>,
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
}

#[derive(Debug, Clone)]
pub struct ExecutorContext {
    pub thread_id: String,
    pub run_id: Arc<tokio::sync::Mutex<String>>,
    pub verbose: bool,
    pub user_id: Option<String>,
}

impl ExecutorContext {
    pub fn new(thread_id: String, verbose: bool, user_id: Option<String>) -> Self {
        Self {
            thread_id,
            run_id: Arc::new(tokio::sync::Mutex::new(Uuid::new_v4().to_string())),
            verbose,
            user_id,
        }
    }

    pub async fn new_run(&self) -> String {
        let new_run_id = Uuid::new_v4().to_string();
        *self.run_id.lock().await = new_run_id.clone();
        new_run_id
    }
}
