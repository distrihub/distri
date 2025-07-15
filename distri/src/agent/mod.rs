pub mod agent;
pub mod agents;
pub mod capabilities;
pub mod composable_agent;
pub mod executor;
pub mod factory;
mod hooks;
pub mod log;
pub mod macros;
pub mod reason;
pub mod server;
use crate::types::ToolCall;
pub use agent::{BaseAgent, StandardAgent, StepResult, MAX_ITERATIONS};
pub use agents::{
    create_tool_parser_agent_factory, create_tool_parser_agent_factory_with_format, ToolParserAgent,
};
use async_openai::types::Role;
pub use executor::{AgentExecutor, AgentExecutorBuilder};
pub use factory::AgentFactoryRegistry;
pub use hooks::AgentHooks;
pub use log::{ModelLogger, StepLogger};
use serde::{Deserialize, Serialize};
use serde_json::Value;
pub use server::{build_server, DISTRI_LOCAL_SERVER};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

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
