pub mod agent;
pub mod executor;
pub mod extensible_example;
pub mod log;
pub mod reason;
pub mod server;

pub use agent::{BaseAgent, DefaultAgent, StandardAgent, StepResult, MAX_ITERATIONS};
pub use executor::{AgentExecutor, AgentExecutorBuilder};
pub use extensible_example::{FilteringAgent, LoggingAgent};
pub use log::{ModelLogger, StepLogger};
pub use server::{build_server, DISTRI_LOCAL_SERVER};

use crate::types::ToolCall;
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
    AgentHandover {
        thread_id: String,
        run_id: String,
        from_agent: String,
        to_agent: String,
        reason: Option<String>,
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
    HandoverAgent {
        from_agent: String,
        to_agent: String,
        reason: Option<String>,
        context: Arc<ExecutorContext>,
        event_tx: Option<mpsc::Sender<AgentEvent>>,
    },
}

#[derive(Debug, Clone)]
pub struct ExecutorContext {
    pub thread_id: String,
    pub run_id: Arc<tokio::sync::Mutex<String>>,
    pub verbose: bool,
    pub user_id: Option<String>,
    /// Add additional context for tools to use passed as meta in MCP calls
    pub tools_context:
        std::collections::HashMap<String, std::collections::HashMap<String, serde_json::Value>>,
}

impl Default for ExecutorContext {
    fn default() -> Self {
        Self::new(Uuid::new_v4().to_string(), None, true, None, None)
    }
}

impl ExecutorContext {
    pub fn new(
        thread_id: String,
        run_id: Option<String>,
        verbose: bool,
        user_id: Option<String>,
        tools_context: Option<
            std::collections::HashMap<String, std::collections::HashMap<String, serde_json::Value>>,
        >,
    ) -> Self {
        Self {
            thread_id,
            run_id: Arc::new(tokio::sync::Mutex::new(
                run_id.unwrap_or_else(|| Uuid::new_v4().to_string()),
            )),
            verbose,
            user_id,
            tools_context: tools_context.unwrap_or_default(),
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
