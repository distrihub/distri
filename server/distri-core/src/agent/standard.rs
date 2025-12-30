use crate::{
    agent::{
        memory::DefaultMemoryManager,
        strategy::{
            execution::{AgentExecutor, MemoryStrategy},
            planning::UnifiedPlanner,
        },
        types::{AgentHooks, BaseAgent},
        ExecutorContext, InvokeResult,
    },
    types::{AgentStrategy, Message, StandardDefinition},
    AgentError,
};

use async_trait::async_trait;
use distri_types::{ExecutionResult, PlanStep, Tool};
use serde_json::Value;
use std::sync::Arc;

/// Default hooks implementation
#[derive(Debug)]
pub struct DefaultHooks;

#[async_trait]
impl AgentHooks for DefaultHooks {
    async fn on_plan_start(
        &self,
        _message: &mut crate::types::Message,
        _context: Arc<ExecutorContext>,
    ) -> Result<(), AgentError> {
        Ok(())
    }

    async fn on_plan_end(
        &self,
        _message: &mut crate::types::Message,
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

/// Standard agent implementation using AgentLoop
#[derive(Clone, Debug)]
pub struct StandardAgent {
    pub definition: StandardDefinition,
    pub loop_engine: crate::agent::agent_loop::AgentLoop,
    pub tools: Vec<Arc<dyn Tool>>,
    pub hooks: Arc<dyn AgentHooks>,
}

impl StandardAgent {
    pub async fn new(
        definition: StandardDefinition,
        tools: Vec<Arc<dyn Tool>>,
        external_tool_calls_store: Arc<dyn distri_types::stores::ExternalToolCallsStore>,
        hooks: Arc<dyn AgentHooks>,
    ) -> Result<Self, AgentError> {
        let strategy = definition.strategy.clone().unwrap_or_default();
        // Initialize strategies based on unified configuration
        let planner = Arc::new(UnifiedPlanner::new(definition.clone(), strategy.clone()));

        let executor_strategy =
            Self::create_executor(tools.clone(), &definition, external_tool_calls_store).await?;
        let memory = Self::create_memory(&strategy);

        let loop_engine = crate::agent::agent_loop::AgentLoop::new(
            definition.clone(),
            planner,
            executor_strategy,
            memory,
            hooks.clone(),
        );

        Ok(Self {
            definition,
            loop_engine,
            tools,
            hooks,
        })
    }

    /// Create executor based on configuration
    async fn create_executor(
        tools: Vec<Arc<dyn Tool>>,
        definition: &StandardDefinition,
        external_tool_calls_store: Arc<dyn distri_types::stores::ExternalToolCallsStore>,
    ) -> Result<Arc<dyn crate::agent::strategy::execution::ExecutionStrategy>, AgentError> {
        Ok(Arc::new(AgentExecutor::new(
            tools,
            Some(definition.clone()),
            external_tool_calls_store,
        )))
    }

    /// Create memory strategy based on configuration
    fn create_memory(_strategy: &AgentStrategy) -> Arc<dyn MemoryStrategy> {
        // For now, always use default memory
        Arc::new(DefaultMemoryManager::new())
    }

    pub fn with_loop_engine(mut self, loop_engine: crate::agent::agent_loop::AgentLoop) -> Self {
        self.loop_engine = loop_engine;
        self
    }

    pub fn with_hooks(mut self, hooks: Arc<dyn AgentHooks>) -> Self {
        self.hooks = hooks;
        self
    }
}

#[async_trait::async_trait]
impl BaseAgent for StandardAgent {
    async fn invoke_stream(
        &self,
        mut message: Message,
        context: Arc<ExecutorContext>,
    ) -> Result<InvokeResult, AgentError> {
        self.hooks
            .before_execute(&mut message, context.clone())
            .await?;
        let content = self.loop_engine.run(message, context.clone()).await?;
        let content = match content {
            Some(Value::String(c)) => Some(c),
            Some(v) => Some(v.to_string()),
            None => None,
        };
        Ok(InvokeResult {
            content,
            tool_calls: vec![],
        })
    }

    fn clone_box(&self) -> Box<dyn BaseAgent> {
        Box::new(StandardAgent::clone(self))
    }

    fn get_name(&self) -> &str {
        &self.definition.name
    }

    fn get_description(&self) -> &str {
        &self.definition.description
    }

    fn get_definition(&self) -> distri_types::configuration::AgentConfig {
        distri_types::configuration::AgentConfig::StandardAgent(self.definition.clone())
    }

    fn get_tools(&self) -> Vec<Arc<dyn Tool>> {
        self.tools.clone()
    }

    fn get_dag(&self) -> crate::agent::types::AgentDag {
        // StandardAgent is a single execution block
        let node = crate::agent::types::DagNode {
            id: "agent_execution".to_string(),
            name: self.definition.name.clone(),
            node_type: "standard_agent".to_string(),
            dependencies: vec![], // No dependencies for single block
            metadata: serde_json::json!({
                "agent_type": "standard",
                "max_iterations": self.definition.max_iterations,
                "model": self.definition.model_settings.model,
                "tools_count": self.tools.len()
            }),
        };

        crate::agent::types::AgentDag {
            nodes: vec![node],
            agent_name: self.definition.name.clone(),
            description: self.definition.description.clone(),
        }
    }
}
