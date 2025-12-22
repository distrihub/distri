use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::{MessageRole, ToolCall, ToolResponse};
use crate::hooks::InlineHookRequest;

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "snake_case")]
pub struct AgentEvent {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub thread_id: String,
    pub run_id: String,
    pub event: AgentEventType,
    pub task_id: String,
    pub agent_id: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum AgentEventType {
    // Main run events
    RunStarted {},
    RunFinished {
        success: bool,
        total_steps: usize,
        failed_steps: usize,
    },
    RunError {
        message: String,
        code: Option<String>,
    },
    PlanStarted {
        initial_plan: bool,
    },
    PlanFinished {
        total_steps: usize,
    },
    PlanPruned {
        removed_steps: Vec<String>,
    },
    // Step execution events
    StepStarted {
        step_id: String,
        step_index: usize,
    },
    StepCompleted {
        step_id: String,
        success: bool,
    },

    // Tool execution events
    ToolExecutionStart {
        step_id: String,
        tool_call_id: String,
        tool_call_name: String,
        input: Value,
    },
    ToolExecutionEnd {
        step_id: String,
        tool_call_id: String,
        tool_call_name: String,
        success: bool,
    },

    // Message events for streaming
    TextMessageStart {
        message_id: String,
        step_id: String,
        role: MessageRole,
        is_final: Option<bool>,
    },
    TextMessageContent {
        message_id: String,
        step_id: String,
        delta: String,
        stripped_content: Option<Vec<(usize, String)>>,
    },
    TextMessageEnd {
        message_id: String,
        step_id: String,
    },

    // Tool call events with parent/child relationships
    ToolCalls {
        step_id: String,
        parent_message_id: Option<String>,
        tool_calls: Vec<ToolCall>,
    },
    ToolResults {
        step_id: String,
        parent_message_id: Option<String>,
        results: Vec<ToolResponse>,
    },

    // Agent transfer events
    AgentHandover {
        from_agent: String,
        to_agent: String,
        reason: Option<String>,
    },

    // Workflow events
    WorkflowStarted {
        workflow_name: String,
        total_steps: usize,
    },
    NodeStarted {
        node_id: String,
        node_name: String,
        step_type: String,
    },
    NodeCompleted {
        node_id: String,
        node_name: String,
        success: bool,
        error: Option<String>,
    },
    RunCompleted {
        workflow_name: String,
        success: bool,
        total_steps: usize,
    },
    RunFailed {
        workflow_name: String,
        error: String,
        failed_at_step: Option<String>,
    },

    BrowserScreenshot {
        image: String,
        format: Option<String>,
        filename: Option<String>,
        size: Option<u64>,
        timestamp_ms: Option<i64>,
    },

    InlineHookRequested {
        request: InlineHookRequest,
    },

    // TODO events
    TodosUpdated {
        formatted_todos: String,
        action: String,
        todo_count: usize,
    },
}

impl AgentEvent {
    pub fn new(event: AgentEventType) -> Self {
        Self {
            timestamp: chrono::Utc::now(),
            thread_id: uuid::Uuid::new_v4().to_string(),
            run_id: uuid::Uuid::new_v4().to_string(),
            event,
            task_id: uuid::Uuid::new_v4().to_string(),
            agent_id: "default".to_string(),
        }
    }

    pub fn with_context(
        event: AgentEventType,
        thread_id: String,
        run_id: String,
        task_id: String,
        agent_id: String,
    ) -> Self {
        Self {
            timestamp: chrono::Utc::now(),
            thread_id,
            run_id,
            task_id,
            event,
            agent_id,
        }
    }
}
