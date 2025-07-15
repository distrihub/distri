use crate::{
    agent::{agent::AgentType, AgentExecutorBuilder, AgentHooks, BaseAgent, StandardAgent},
    memory::TaskStep,
    types::{AgentDefinition, ModelSettings},
};
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::info;

#[derive(Clone)]
struct TestHookAgent {
    inner: StandardAgent,
    hook_calls: Arc<tokio::sync::Mutex<Vec<String>>>,
}

impl std::fmt::Debug for TestHookAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestHookAgent").finish()
    }
}

impl TestHookAgent {
    fn new(
        definition: AgentDefinition,
        tools_registry: Arc<crate::tools::LlmToolsRegistry>,
        executor: Arc<crate::agent::AgentExecutor>,
        context: Arc<crate::agent::ExecutorContext>,
        session_store: Arc<Box<dyn crate::SessionStore>>,
    ) -> Self {
        let inner = StandardAgent::new(
            definition,
            tools_registry,
            executor,
            context,
            session_store,
        );
        Self {
            inner,
            hook_calls: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        }
    }

    async fn get_hook_calls(&self) -> Vec<String> {
        self.hook_calls.lock().await.clone()
    }
}

#[async_trait::async_trait]
impl BaseAgent for TestHookAgent {
    fn agent_type(&self) -> AgentType {
        AgentType::Custom("TestHookAgent".to_string())
    }

    fn get_definition(&self) -> crate::types::AgentDefinition {
        self.inner.get_definition()
    }

    fn get_description(&self) -> &str {
        self.inner.get_description()
    }

    fn get_tools(&self) -> Vec<&Box<dyn crate::tools::Tool>> {
        self.inner.get_tools()
    }

    fn get_name(&self) -> &str {
        self.inner.get_name()
    }

    fn clone_box(&self) -> Box<dyn BaseAgent> {
        Box::new(self.clone())
    }

    fn get_hooks(&self) -> Option<&dyn AgentHooks> {
        Some(self)
    }

    async fn invoke(
        &self,
        task: TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<crate::agent::ExecutorContext>,
        event_tx: Option<mpsc::Sender<crate::agent::AgentEvent>>,
    ) -> Result<String, crate::error::AgentError> {
        self.inner.invoke(task, params, context, event_tx).await
    }

    async fn invoke_stream(
        &self,
        task: TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<crate::agent::ExecutorContext>,
        event_tx: mpsc::Sender<crate::agent::AgentEvent>,
    ) -> Result<(), crate::error::AgentError> {
        self.inner
            .invoke_stream(task, params, context, event_tx)
            .await
    }
}

#[async_trait::async_trait]
impl AgentHooks for TestHookAgent {
    async fn after_task_step(
        &self,
        _task: TaskStep,
        _context: Arc<crate::agent::ExecutorContext>,
    ) -> Result<(), crate::error::AgentError> {
        let mut calls = self.hook_calls.lock().await;
        calls.push("after_task_step".to_string());
        info!("🔧 TestHookAgent: after_task_step called");
        Ok(())
    }

    async fn before_llm_step(
        &self,
        _messages: &[crate::types::Message],
        _params: &Option<serde_json::Value>,
        _context: Arc<crate::agent::ExecutorContext>,
    ) -> Result<Vec<crate::types::Message>, crate::error::AgentError> {
        let mut calls = self.hook_calls.lock().await;
        calls.push("before_llm_step".to_string());
        info!("🔧 TestHookAgent: before_llm_step called");
        Ok(vec![])
    }

    async fn after_finish(
        &self,
        step_result: crate::agent::agent::StepResult,
        _context: Arc<crate::agent::ExecutorContext>,
    ) -> Result<crate::agent::agent::StepResult, crate::error::AgentError> {
        let mut calls = self.hook_calls.lock().await;
        calls.push("after_finish".to_string());
        info!("🔧 TestHookAgent: after_finish called");
        Ok(step_result)
    }
}

#[tokio::test]
async fn test_hook_delegation() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    // Create a custom agent factory for testing
    let custom_factory = Arc::new(
        |definition, tools_registry, executor, context, session_store| {
            Box::new(TestHookAgent::new(
                definition,
                tools_registry,
                executor,
                context,
                session_store,
            )) as Box<dyn BaseAgent>
        },
    );

    // Create executor with custom factory
    let stores = crate::types::StoreConfig::default().initialize().await?;
    let mut factory_registry = crate::agent::factory::AgentFactoryRegistry::new();
    factory_registry.register_factory("test_hook".to_string(), custom_factory);

    let executor = AgentExecutorBuilder::default()
        .with_stores(stores)
        .with_agent_factory(Arc::new(tokio::sync::RwLock::new(factory_registry)))
        .build()?;

    let executor = Arc::new(executor);

    // Create agent definition
    let agent_def = AgentDefinition {
        name: "test-hook-agent".to_string(),
        description: "A test agent for hook delegation".to_string(),
        agent_type: Some("test_hook".to_string()),
        system_prompt: Some("You are a helpful assistant.".to_string()),
        mcp_servers: vec![],
        model_settings: ModelSettings::default(),
        history_size: Some(10),
        max_iterations: Some(1), // Limit iterations to avoid API calls
        ..Default::default()
    };

    // Register agent definition
    executor
        .register_agent_definition(agent_def.clone())
        .await?;

    // Test creating agent from definition
    let agent = executor.create_agent_from_definition(agent_def).await?;
    assert_eq!(agent.get_name(), "test-hook-agent");
    assert_eq!(agent.agent_type(), AgentType::Custom("TestHookAgent".to_string()));

    // Verify that the agent has hooks
    assert!(agent.get_hooks().is_some());

    info!("✅ Hook delegation test completed successfully");
    Ok(())
}