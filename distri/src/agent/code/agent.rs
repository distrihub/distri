use std::sync::Arc;

use tokio::sync::mpsc;

use crate::{
    agent::{
        code::tools::{CodeResponse, ConsoleLogTool, ExecuteCodeTool, FinalAnswerTool},
        hooks::CodeParsingHooks,
        AgentExecutor, AgentHooks, StandardAgent,
    },
    delegate_base_agent,
    tools::Tool,
    AgentDefinition, SessionStore,
};

#[derive(Debug, Clone)]
pub struct CodeAgent {
    base: StandardAgent,
    code_hooks: Arc<CodeParsingHooks>,
}

impl CodeAgent {
    pub fn new(
        definition: AgentDefinition,
        tools: Vec<Arc<dyn Tool>>,
        coordinator: Arc<AgentExecutor>,
        session_store: Arc<Box<dyn SessionStore>>,
    ) -> Self {
        let (tx, rx) = tokio::sync::mpsc::channel(100);
        // Create code parsing hooks
        let code_hooks = Arc::new(CodeParsingHooks::new(tools.clone(), rx));
        let tools = Self::init_tools(tools.clone(), tx.clone());
        // Create StandardAgent
        let base = StandardAgent::new(definition, tools, coordinator, session_store);

        Self { base, code_hooks }
    }

    pub fn init_tools(
        tools: Vec<Arc<dyn Tool>>,
        tx: mpsc::Sender<CodeResponse>,
    ) -> Vec<Arc<dyn Tool>> {
        let mut tools = tools;
        tools.push(Arc::new(FinalAnswerTool(tx.clone())));
        tools.push(Arc::new(ConsoleLogTool(tx.clone())));

        let inner_tools = tools.clone();
        tools.push(Arc::new(ExecuteCodeTool(inner_tools)));
        tools
    }
}

#[async_trait::async_trait]
impl AgentHooks for CodeAgent {
    async fn before_invoke(
        &self,
        message: crate::types::Message,
        context: Arc<crate::agent::ExecutorContext>,
        event_tx: Option<tokio::sync::mpsc::Sender<crate::agent::AgentEvent>>,
    ) -> Result<(), crate::error::AgentError> {
        self.code_hooks
            .before_invoke(message, context, event_tx)
            .await
    }

    async fn llm_messages(
        &self,
        messages: &[crate::types::Message],
    ) -> Result<Vec<crate::types::Message>, crate::error::AgentError> {
        self.code_hooks.llm_messages(messages).await
    }

    async fn after_execute(
        &self,
        response: crate::llm::LLMResponse,
    ) -> Result<crate::llm::LLMResponse, crate::error::AgentError> {
        self.code_hooks.after_execute(response).await
    }

    async fn after_execute_stream(
        &self,
        response: crate::llm::StreamResult,
    ) -> Result<crate::llm::StreamResult, crate::error::AgentError> {
        self.code_hooks.after_execute_stream(response).await
    }

    async fn before_tool_calls(
        &self,
        tool_calls: &[crate::types::ToolCall],
    ) -> Result<Vec<crate::types::ToolCall>, crate::error::AgentError> {
        self.code_hooks.before_tool_calls(tool_calls).await
    }

    async fn after_tool_calls(
        &self,
        tool_responses: &[crate::types::Message],
    ) -> Result<Vec<crate::types::Message>, crate::error::AgentError> {
        self.code_hooks.after_tool_calls(tool_responses).await
    }

    async fn before_step_result(
        &self,
        step_result: crate::agent::StepResult,
    ) -> Result<crate::agent::StepResult, crate::error::AgentError> {
        self.code_hooks.before_step_result(step_result).await
    }
}

delegate_base_agent!(CodeAgent, "code", base);
