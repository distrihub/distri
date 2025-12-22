use distri_types::configuration::AgentConfig;
use distri_types::{ExecutionResult, PlanStep};
use serde::{Deserialize, Serialize};

use std::sync::Arc;

use crate::{
    tools::Tool,
    types::{Message, ToolCall},
    AgentError,
};
pub use distri_types::{AgentEvent, AgentEventType};

// Re-export from context module
pub use super::context::{ExecutorContext, ExecutorContextMetadata, ForkOptions, ForkType};

// Re-export strategy enums from main types module
pub use crate::types::{ExecutionKind, MemoryKind};

pub const MAX_ITERATIONS: usize = 10;

/// DAG node representation for agent visualization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DagNode {
    /// Unique node ID
    pub id: String,
    /// Display name
    pub name: String,
    /// Node type (agent, tool, workflow_step)
    pub node_type: String,
    /// Dependencies (list of node IDs this node depends on)
    pub dependencies: Vec<String>,
    /// Additional metadata
    pub metadata: serde_json::Value,
}

/// DAG representation of agent execution flow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDag {
    /// List of nodes in the DAG
    pub nodes: Vec<DagNode>,
    /// Agent name
    pub agent_name: String,
    /// Agent description
    pub description: String,
}

// New lifecycle hooks for strategy-based execution
#[async_trait::async_trait]
pub trait AgentHooks: Send + Sync + std::fmt::Debug {
    async fn before_execute(
        &self,
        _message: &mut Message,
        _context: Arc<ExecutorContext>,
    ) -> Result<(), AgentError> {
        Ok(())
    }
    async fn on_plan_start(
        &self,
        _message: &mut Message,
        _context: Arc<ExecutorContext>,
    ) -> Result<(), AgentError> {
        Ok(())
    }

    async fn on_plan_end(
        &self,
        _message: &mut Message,
        _context: Arc<ExecutorContext>,
        _plan: &[PlanStep],
    ) -> Result<(), AgentError> {
        Ok(())
    }

    async fn on_step_start(&self, _step: &PlanStep) -> Result<(), AgentError> {
        Ok(())
    }

    async fn on_step_end(
        &self,
        _context: Arc<ExecutorContext>,
        _step: &PlanStep,
        _result: &ExecutionResult,
    ) -> Result<(), AgentError> {
        Ok(())
    }

    async fn on_halt(&self, _reason: &str) -> Result<(), AgentError> {
        Ok(())
    }
}

/// Result of agent invocation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvokeResult {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Debug)]
pub enum CoordinatorMessage {
    ExecuteStream {
        agent_id: String,
        message: Message,
        context: Arc<ExecutorContext>,
    },
    HandoverAgent {
        from_agent: String,
        to_agent: String,
        reason: Option<String>,
        context: Arc<ExecutorContext>,
    },
}

impl Default for InvokeResult {
    fn default() -> Self {
        Self {
            content: None,
            tool_calls: vec![],
        }
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

    async fn invoke_stream(
        &self,
        _message: Message,
        _context: Arc<ExecutorContext>,
    ) -> Result<InvokeResult, AgentError> {
        return Err(AgentError::NotImplemented(
            "BaseAgent::invoke_stream not implemented".to_string(),
        ));
    }

    /// Clone the agent (required for object safety)
    fn clone_box(&self) -> Box<dyn BaseAgent>;

    /// Get the agent's name/id
    fn get_name(&self) -> &str;

    fn get_description(&self) -> &str;
    fn get_definition(&self) -> AgentConfig;
    fn get_tools(&self) -> Vec<Arc<dyn Tool>>;

    /// Get DAG representation of this agent's execution flow
    fn get_dag(&self) -> AgentDag;
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentType {
    #[default]
    StandardAgent,
    SequentialWorkflowAgent,
    DagWorkflowAgent,
    CustomAgent,
}
